import * as fs from "fs";
import * as path from "path";
import * as vscode from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  Trace,
} from "vscode-languageclient/node";

let client: LanguageClient | undefined;

///@notice This function tries to get the path to the compiled lsp server
function resolveServerPath(context: vscode.ExtensionContext): string {
  //First try to read it from the config file(package.json)
  const config = vscode.workspace.getConfiguration("solidityLsp");
  const configuredPath = config.get<string>("serverPath");
  if (configuredPath && configuredPath.trim().length > 0) {
    return configuredPath;
  }

  return path.resolve(//else default  to this
    context.extensionPath,
    "..",
    "lsp_server",
    "target",
    "debug",
    "lsp_server"
  );
}

export async function activate(context: vscode.ExtensionContext) {
  const serverPath = resolveServerPath(context);
  const config = vscode.workspace.getConfiguration("solidityLsp");
  const traceSetting = config.get<"off" | "messages" | "verbose">(
    "trace.server",
    "verbose"
  );
  const outputChannel = vscode.window.createOutputChannel("Solidity LSP");
  const traceOutputChannel = vscode.window.createOutputChannel("Solidity LSP Trace");
  context.subscriptions.push(outputChannel, traceOutputChannel);
  outputChannel.appendLine("Activating Solidity LSP extension...");
  outputChannel.appendLine(`Resolved server path: ${serverPath}`);
  outputChannel.appendLine(`Configured trace.server: ${traceSetting}`);
  outputChannel.show(true);

  if (!fs.existsSync(serverPath)) {
    const msg = `Solidity LSP server not found at ${serverPath}. Build it first (cargo build).`;
    outputChannel.appendLine(msg);
    vscode.window.showErrorMessage(msg);
    return;
  }

  const env = { ...process.env };
  env.RUST_LOG = "lsp_server=debug";

  const serverOptions: ServerOptions = {
    command: serverPath,
    args: [],
    options: { env },
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [
      { scheme: "file", language: "solidity" },
      { scheme: "file", pattern: "**/*.sol" },
    ],
    outputChannel,
    traceOutputChannel,
  };

  client = new LanguageClient(
    "solidityLsp",
    "Solidity LSP",
    serverOptions,
    clientOptions
  );

  try {
    await client.start();
    const traceLevel =
      traceSetting === "verbose"
        ? Trace.Verbose
        : traceSetting === "messages"
        ? Trace.Messages
        : Trace.Off;
    await client.setTrace(traceLevel);
    outputChannel.appendLine(`Solidity LSP started: ${serverPath}`);
    outputChannel.appendLine(`Server env RUST_LOG=${env.RUST_LOG}`);
  } catch (error) {
    const msg = `Failed to start Solidity LSP: ${String(error)}`;
    outputChannel.appendLine(msg);
    vscode.window.showErrorMessage(msg);
  }

  // ── Virtual document provider ─────────────────────────────────────
  // Registers a custom URI scheme so that view commands can display
  // output in read-only virtual files.
  const provider: vscode.TextDocumentContentProvider = {
    provideTextDocumentContent(uri: vscode.Uri): string {
      return virtualDocs.get(uri.toString()) ?? "";
    },
  };
  context.subscriptions.push(
    vscode.workspace.registerTextDocumentContentProvider(VIRTUAL_SCHEME, provider)
  );

  // ── View HIR command ──────────────────────────────────────────────
  // Sends a custom `solidity/viewHir` request with the user's selection
  // range and displays the lowered HIR expression tree in a virtual
  // read-only file.
  context.subscriptions.push(
    vscode.commands.registerCommand("solidityLsp.viewHir", async () => {
      await sendViewRequest(client, "solidity/viewHir", "viewHir", "hir");
    })
  );

  // ── View AST command ──────────────────────────────────────────────
  // Sends a custom `solidity/viewAst` request with the user's selection
  // range and displays the raw tree-sitter named AST tree in a virtual
  // read-only file.
  context.subscriptions.push(
    vscode.commands.registerCommand("solidityLsp.viewAst", async () => {
      await sendViewRequest(client, "solidity/viewAst", "viewAst", "ast");
    })
  );
}

/// URI scheme for virtual documents used by view commands.
/// Documents using this scheme are in-memory only — no file on disk,
/// no "save changes?" prompt on close.
const VIRTUAL_SCHEME = "solidity-lsp";

/// In-memory store of virtual document contents, keyed by URI.
const virtualDocs = new Map<string, string>();

/// Shared helper: reads the active editor's selection, sends a custom
/// LSP request with the range, and displays the returned text in a
/// virtual read-only document beside the editor.
async function sendViewRequest(
  client: LanguageClient | undefined,
  method: string,
  label: string,
  kind: string,
): Promise<void> {
  const editor = vscode.window.activeTextEditor;
  if (!editor) {
    vscode.window.showWarningMessage("No active editor.");
    return;
  }
  if (!client) {
    vscode.window.showErrorMessage("LSP client is not running.");
    return;
  }

  const textDocument = editor.document.uri;
  const start = editor.selection.start;
  const end = editor.selection.end;

  try {
    const result = await client.sendRequest<{ content: string }>(method, {
      textDocument: { uri: textDocument.toString() },
      start: { line: start.line, character: start.character },
      end: { line: end.line, character: end.character },
    });

    const content = result?.content ?? `(no output from ${label})`;

    // Store content and open via the virtual document provider.
    // Using a unique URI per kind so repeated invocations replace
    // the same tab rather than spawning new ones.
    const uri = vscode.Uri.parse(`${VIRTUAL_SCHEME}:${kind}.${kind === "hir" ? "rs" : "txt"}`);
    virtualDocs.set(uri.toString(), content);

    const doc = await vscode.workspace.openTextDocument(uri);
    await vscode.window.showTextDocument(doc, {
      viewColumn: vscode.ViewColumn.Beside,
      preview: false,
    });
  } catch (err) {
    vscode.window.showErrorMessage(`${label} failed: ${String(err)}`);
  }
}

export async function deactivate() {
  if (!client) {
    return;
  }
  await client.stop();
}

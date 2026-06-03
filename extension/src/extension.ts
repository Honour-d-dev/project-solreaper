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
  env.RUST_LOG = "lsp_server=info";

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
}

export async function deactivate() {
  if (!client) {
    return;
  }
  await client.stop();
}

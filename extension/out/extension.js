"use strict";
var __createBinding = (this && this.__createBinding) || (Object.create ? (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    var desc = Object.getOwnPropertyDescriptor(m, k);
    if (!desc || ("get" in desc ? !m.__esModule : desc.writable || desc.configurable)) {
      desc = { enumerable: true, get: function() { return m[k]; } };
    }
    Object.defineProperty(o, k2, desc);
}) : (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    o[k2] = m[k];
}));
var __setModuleDefault = (this && this.__setModuleDefault) || (Object.create ? (function(o, v) {
    Object.defineProperty(o, "default", { enumerable: true, value: v });
}) : function(o, v) {
    o["default"] = v;
});
var __importStar = (this && this.__importStar) || (function () {
    var ownKeys = function(o) {
        ownKeys = Object.getOwnPropertyNames || function (o) {
            var ar = [];
            for (var k in o) if (Object.prototype.hasOwnProperty.call(o, k)) ar[ar.length] = k;
            return ar;
        };
        return ownKeys(o);
    };
    return function (mod) {
        if (mod && mod.__esModule) return mod;
        var result = {};
        if (mod != null) for (var k = ownKeys(mod), i = 0; i < k.length; i++) if (k[i] !== "default") __createBinding(result, mod, k[i]);
        __setModuleDefault(result, mod);
        return result;
    };
})();
Object.defineProperty(exports, "__esModule", { value: true });
exports.activate = activate;
exports.deactivate = deactivate;
const fs = __importStar(require("fs"));
const path = __importStar(require("path"));
const vscode = __importStar(require("vscode"));
const node_1 = require("vscode-languageclient/node");
let client;
///@notice This function tries to get the path to the compiled lsp server
function resolveServerPath(context) {
    //First try to read it from the config file(package.json)
    const config = vscode.workspace.getConfiguration("solidityLsp");
    const configuredPath = config.get("serverPath");
    if (configuredPath && configuredPath.trim().length > 0) {
        return configuredPath;
    }
    return path.resolve(//else default  to this
    context.extensionPath, "..", "lsp_server", "target", "debug", "lsp_server");
}
async function activate(context) {
    const serverPath = resolveServerPath(context);
    const config = vscode.workspace.getConfiguration("solidityLsp");
    const traceSetting = config.get("trace.server", "verbose");
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
    const serverOptions = {
        command: serverPath,
        args: [],
        options: { env },
    };
    const clientOptions = {
        documentSelector: [
            { scheme: "file", language: "solidity" },
            { scheme: "file", pattern: "**/*.sol" },
        ],
        outputChannel,
        traceOutputChannel,
    };
    client = new node_1.LanguageClient("solidityLsp", "Solidity LSP", serverOptions, clientOptions);
    try {
        await client.start();
        const traceLevel = traceSetting === "verbose"
            ? node_1.Trace.Verbose
            : traceSetting === "messages"
                ? node_1.Trace.Messages
                : node_1.Trace.Off;
        await client.setTrace(traceLevel);
        outputChannel.appendLine(`Solidity LSP started: ${serverPath}`);
        outputChannel.appendLine(`Server env RUST_LOG=${env.RUST_LOG}`);
    }
    catch (error) {
        const msg = `Failed to start Solidity LSP: ${String(error)}`;
        outputChannel.appendLine(msg);
        vscode.window.showErrorMessage(msg);
    }
}
async function deactivate() {
    if (!client) {
        return;
    }
    await client.stop();
}
//# sourceMappingURL=extension.js.map
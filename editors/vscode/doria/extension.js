"use strict";

const cp = require("child_process");
const fs = require("fs");
const path = require("path");
const vscode = require("vscode");

let client;

function activate(context) {
  client = new DoriaLanguageClient(context);
  context.subscriptions.push(client);
  client.start();
}

function deactivate() {
  if (client) {
    return client.dispose();
  }
}

class DoriaLanguageClient {
  constructor(context) {
    this.context = context;
    this.nextId = 1;
    this.pending = new Map();
    this.buffer = Buffer.alloc(0);
    this.diagnostics = vscode.languages.createDiagnosticCollection("doria");
    this.process = undefined;
    this.started = false;

    context.subscriptions.push(this.diagnostics);
    context.subscriptions.push(
      vscode.workspace.onDidOpenTextDocument((document) => this.didOpen(document)),
      vscode.workspace.onDidChangeTextDocument((event) => this.didChange(event)),
      vscode.workspace.onDidCloseTextDocument((document) => this.didClose(document)),
      vscode.workspace.onDidSaveTextDocument((document) => this.didSave(document)),
      vscode.languages.registerHoverProvider(
        { language: "doria" },
        {
          provideHover: (document, position) => this.provideHover(document, position)
        }
      ),
      vscode.languages.registerCompletionItemProvider(
        { language: "doria" },
        {
          provideCompletionItems: (document, position) => this.provideCompletionItems(document, position)
        },
        "$",
        ">",
        ":"
      )
    );
  }

  start() {
    if (this.started) {
      return;
    }
    this.started = true;

    const serverPath = resolveServerPath(this.context);
    const child = cp.spawn(serverPath, [], {
      cwd: workspaceRoot(),
      stdio: ["pipe", "pipe", "pipe"]
    });
    this.process = child;

    child.on("error", (error) => {
      vscode.window.showWarningMessage(`Doria language server failed to start: ${error.message}`);
      this.resetServer(child, error);
    });
    child.stderr.on("data", (chunk) => {
      console.error(`[doria-lsp] ${chunk.toString()}`);
    });
    child.stdout.on("data", (chunk) => this.onData(chunk));
    child.on("close", (code, signal) => {
      this.resetServer(
        child,
        new Error(`Doria language server stopped (code ${code ?? "none"}, signal ${signal ?? "none"})`)
      );
    });

    this.sendRequest("initialize", {
      processId: process.pid,
      rootUri: vscode.workspace.workspaceFolders?.[0]?.uri.toString() ?? null,
      capabilities: {}
    }).then(() => {
      this.sendNotification("initialized", {});
      for (const document of vscode.workspace.textDocuments) {
        this.didOpen(document);
      }
    }).catch(() => {
      // The spawn error path rejects the initialize request after surfacing a warning.
    });
  }

  dispose() {
    this.diagnostics.dispose();
    if (!this.process) {
      return Promise.resolve();
    }

    const child = this.process;
    return this.sendRequest("shutdown", {})
      .catch(() => undefined)
      .then(() => {
        this.sendNotification("exit", {});
        setTimeout(() => {
          if (!child.killed) {
            child.kill();
          }
        }, 1000);
      });
  }

  didOpen(document) {
    if (!isDoria(document) || !this.process) {
      return;
    }

    this.sendNotification("textDocument/didOpen", {
      textDocument: {
        uri: document.uri.toString(),
        languageId: "doria",
        version: document.version,
        text: document.getText()
      }
    });
  }

  didChange(event) {
    if (!isDoria(event.document) || !this.process) {
      return;
    }

    this.sendNotification("textDocument/didChange", {
      textDocument: {
        uri: event.document.uri.toString(),
        version: event.document.version
      },
      contentChanges: [
        {
          text: event.document.getText()
        }
      ]
    });
  }

  didSave(document) {
    if (!isDoria(document) || !this.process) {
      return;
    }

    this.sendNotification("textDocument/didSave", {
      textDocument: {
        uri: document.uri.toString()
      }
    });
  }

  didClose(document) {
    if (!isDoria(document) || !this.process) {
      return;
    }

    this.sendNotification("textDocument/didClose", {
      textDocument: {
        uri: document.uri.toString()
      }
    });
    this.diagnostics.delete(document.uri);
  }

  sendRequest(method, params) {
    if (!this.process) {
      return Promise.reject(new Error("Doria language server is not running"));
    }

    const id = this.nextId++;
    this.send({ jsonrpc: "2.0", id, method, params });
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
    });
  }

  sendNotification(method, params) {
    if (!this.process) {
      return;
    }

    this.send({ jsonrpc: "2.0", method, params });
  }

  send(message) {
    if (!this.process) {
      return;
    }

    const body = Buffer.from(JSON.stringify(message), "utf8");
    const header = Buffer.from(`Content-Length: ${body.length}\r\n\r\n`, "ascii");
    this.process.stdin.write(Buffer.concat([header, body]));
  }

  resetServer(child, error) {
    if (child !== this.process) {
      return;
    }

    this.process = undefined;
    this.started = false;
    this.buffer = Buffer.alloc(0);
    this.diagnostics.clear();
    this.rejectPending(error);
  }

  rejectPending(error) {
    for (const pending of this.pending.values()) {
      pending.reject(error);
    }
    this.pending.clear();
  }

  onData(chunk) {
    this.buffer = Buffer.concat([this.buffer, chunk]);

    while (true) {
      const headerEnd = this.buffer.indexOf("\r\n\r\n");
      if (headerEnd === -1) {
        return;
      }

      const header = this.buffer.slice(0, headerEnd).toString("ascii");
      const lengthMatch = header.match(/Content-Length:\s*(\d+)/i);
      if (!lengthMatch) {
        this.buffer = this.buffer.slice(headerEnd + 4);
        continue;
      }

      const length = Number(lengthMatch[1]);
      const messageEnd = headerEnd + 4 + length;
      if (this.buffer.length < messageEnd) {
        return;
      }

      const body = this.buffer.slice(headerEnd + 4, messageEnd).toString("utf8");
      this.buffer = this.buffer.slice(messageEnd);
      this.handleMessage(JSON.parse(body));
    }
  }

  handleMessage(message) {
    if (Object.prototype.hasOwnProperty.call(message, "id")) {
      const pending = this.pending.get(message.id);
      if (pending) {
        this.pending.delete(message.id);
        if (message.error) {
          pending.reject(new Error(message.error.message));
        } else {
          pending.resolve(message.result);
        }
      }
      return;
    }

    if (message.method === "textDocument/publishDiagnostics") {
      this.publishDiagnostics(message.params);
    }
  }

  publishDiagnostics(params) {
    const uri = vscode.Uri.parse(params.uri);
    const diagnostics = (params.diagnostics ?? []).map((diagnostic) => {
      const range = new vscode.Range(
        diagnostic.range.start.line,
        diagnostic.range.start.character,
        diagnostic.range.end.line,
        diagnostic.range.end.character
      );
      const item = new vscode.Diagnostic(
        range,
        diagnostic.message,
        toSeverity(diagnostic.severity)
      );
      item.code = diagnostic.code;
      item.source = diagnostic.source;
      return item;
    });
    this.diagnostics.set(uri, diagnostics);
  }

  provideHover(document, position) {
    if (!isDoria(document) || !this.process) {
      return undefined;
    }

    return this.sendRequest("textDocument/hover", {
      textDocument: {
        uri: document.uri.toString()
      },
      position: toLspPosition(position)
    })
      .then((hover) => {
        if (!hover) {
          return undefined;
        }
        return new vscode.Hover(toHoverContents(hover.contents), hover.range ? toRange(hover.range) : undefined);
      })
      .catch(() => undefined);
  }

  provideCompletionItems(document, position) {
    if (!isDoria(document) || !this.process) {
      return undefined;
    }

    return this.sendRequest("textDocument/completion", {
      textDocument: {
        uri: document.uri.toString()
      },
      position: toLspPosition(position)
    })
      .then((result) => {
        const items = Array.isArray(result) ? result : result?.items ?? [];
        return items.map((item) => {
          const completion = new vscode.CompletionItem(item.label, toCompletionKind(item.kind));
          completion.detail = item.detail;
          completion.documentation = item.documentation;
          completion.insertText = item.insertText;
          return completion;
        });
      })
      .catch(() => undefined);
  }
}

function resolveServerPath(context) {
  const configured = vscode.workspace.getConfiguration("doria").get("languageServer.path");
  if (configured && configured.trim().length > 0) {
    return configured;
  }

  if (process.env.DORIA_LSP_PATH) {
    return process.env.DORIA_LSP_PATH;
  }

  const root = workspaceRoot();
  if (root) {
    const workspaceBinary = process.platform === "win32"
      ? path.join(root, "target", "debug", "doria-lsp.exe")
      : path.join(root, "target", "debug", "doria-lsp");
    if (fs.existsSync(workspaceBinary)) {
      return workspaceBinary;
    }
  }

  const extensionBinary = process.platform === "win32"
    ? path.resolve(context.extensionPath, "..", "..", "..", "target", "debug", "doria-lsp.exe")
    : path.resolve(context.extensionPath, "..", "..", "..", "target", "debug", "doria-lsp");
  if (fs.existsSync(extensionBinary)) {
    return extensionBinary;
  }

  return process.platform === "win32" ? "doria-lsp.exe" : "doria-lsp";
}

function workspaceRoot() {
  return vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
}

function isDoria(document) {
  return document.languageId === "doria" || document.fileName.endsWith(".doria");
}

function toSeverity(severity) {
  switch (severity) {
    case 1:
      return vscode.DiagnosticSeverity.Error;
    case 2:
      return vscode.DiagnosticSeverity.Warning;
    case 3:
      return vscode.DiagnosticSeverity.Information;
    case 4:
      return vscode.DiagnosticSeverity.Hint;
    default:
      return vscode.DiagnosticSeverity.Error;
  }
}

function toLspPosition(position) {
  return {
    line: position.line,
    character: position.character
  };
}

function toRange(range) {
  return new vscode.Range(
    range.start.line,
    range.start.character,
    range.end.line,
    range.end.character
  );
}

function toHoverContents(contents) {
  if (typeof contents === "string") {
    return contents;
  }
  if (contents && contents.kind === "markdown") {
    return new vscode.MarkdownString(contents.value);
  }
  if (contents && contents.value) {
    return contents.value;
  }
  if (Array.isArray(contents)) {
    return contents.map(toHoverContents);
  }
  return "";
}

function toCompletionKind(kind) {
  switch (kind) {
    case 7:
      return vscode.CompletionItemKind.Class;
    case 14:
      return vscode.CompletionItemKind.Keyword;
    case 25:
      return vscode.CompletionItemKind.TypeParameter;
    default:
      return vscode.CompletionItemKind.Text;
  }
}

module.exports = {
  activate,
  deactivate
};

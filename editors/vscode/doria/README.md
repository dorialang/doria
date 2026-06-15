# Doria Language Support

This extension provides `.doria` language registration, TextMate syntax highlighting, editor bracket/comment behavior, and diagnostics from `doria-lsp`.

Syntax colors depend on the active VS Code theme. This extension improves Doria's TextMate scopes for cleaner highlighting, but it does not ship a custom color theme yet.

Before launching the extension, build the language server from the repository root:

```bash
cargo build -p doriac --bin doria-lsp
```

The extension resolves the server from:

```text
1. doria.languageServer.path
2. DORIA_LSP_PATH
3. target/debug/doria-lsp in the open workspace
4. doria-lsp on PATH
```

No npm dependencies are required for the development extension.

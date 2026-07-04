# Doria Language Support

This extension provides `.doria` language registration, TextMate syntax highlighting, editor bracket/comment behavior, and diagnostics from `doria-lsp`.

Syntax colors depend on the active VS Code theme. This extension improves Doria's TextMate scopes for cleaner highlighting, but it does not ship a custom color theme yet.

Double-quoted string interpolation such as `{$this->name}` keeps the string text green while colorizing the variable reference. Single-quoted strings are treated as literal strings.

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

After changing the TextMate grammar, reload the VS Code window or restart the Extension Development Host so VS Code reads the updated grammar.

Keep this TextMate grammar aligned with the IntelliJ / JetBrains highlighter under `editors/intellij/doria`. From the repository root, run:

```bash
python3 scripts/check_editor_highlighting.py
```

Files under `editors/fixtures/` are syntax-highlighting smoke fixtures. The VS Code client keeps them out of `doria-lsp` diagnostics so accepted/planned editor vocabulary can be exercised before compiler implementation lands.

Doria has two distinct `use` contexts: file/namespace-scope `use` imports names from namespaces, while class-body `use` composes traits into a class or trait. The TextMate grammar keeps these scopes separate as import use and trait-composition use.

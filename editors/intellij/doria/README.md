# Doria IntelliJ Language Support

This directory contains first-pass Doria support for IntelliJ-based IDEs.

It provides:

- `.doria` file recognition.
- Basic syntax highlighting for Doria keywords, variables, types, strings, comments, numbers, operators, and punctuation.
- A Doria settings page for configuring the language server path.
- `doria-lsp` integration through the IntelliJ Platform LSP API.

The initial plugin targets IntelliJ Platform `2025.2.1+`, where JetBrains exposes the LSP module as `com.intellij.modules.lsp`.

## Build the language server

From the repository root:

```bash
cargo build -p doriac --bin doria-lsp
```

## Build the plugin

From this directory:

```bash
gradle buildPlugin
```

The packaged plugin will be written under:

```text
build/distributions/
```

## Run in a sandbox IDE

```bash
gradle runIde
```

## Language server path resolution

The plugin looks for `doria-lsp` in this order:

```text
1. Doria settings: Language server path
2. DORIA_LSP_PATH environment variable
3. target/debug/doria-lsp in the open project
4. doria-lsp on PATH
```

On Windows, the executable name is `doria-lsp.exe`.

The settings path also accepts `$PROJECT_DIR$`, for example:

```text
$PROJECT_DIR$/target/debug/doria-lsp
```

## Notes

This plugin intentionally reuses the existing `doria-lsp` binary instead of duplicating compiler diagnostics in IntelliJ. Syntax highlighting is local and lightweight; diagnostics, completion, and hover come from the language server.

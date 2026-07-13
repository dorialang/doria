# Doria IntelliJ Language Support

This directory contains first-pass Doria support for IntelliJ-based IDEs.

Plugin releases track the Doria toolchain CalVer. The target release is `2026.03.1-canary`.

It provides:

- `.doria` file recognition.
- Basic syntax highlighting for Doria keywords, variables, types, attributes, strings, string interpolation, comments, numbers, operators, punctuation, accepted OOP declaration vocabulary, namespace/import/include/directive vocabulary, and rejected strict-comparison/preprocessor spellings.
- A Doria settings page for configuring the language server path.
- `doria-lsp` integration through the IntelliJ Platform LSP API.

The initial plugin targets IntelliJ Platform `2025.2.1+`, where JetBrains exposes the LSP module as `com.intellij.modules.lsp`.

This is first-pass Doria support for IntelliJ / JetBrains IDEs. The local IntelliJ highlighter is syntax highlighting only: it does not provide a semantic PSI tree, formatter, inspections, refactors, or compiler diagnostics. Compiler-backed diagnostics, completion, and hover remain separate and come from `doria-lsp` when the language server is configured and available.

The plugin registers the lower-case `doria` language id so Markdown fenced blocks using the `doria` info string can resolve to the Doria highlighter where the JetBrains Markdown plugin performs language injection. Planned keywords are highlighted for documentation readability only; compiler support still follows the staged plan.

## Build the language server

From the repository root:

```bash
cargo build -p doriac --bin doria-lsp
```

## Build the plugin

From this directory:

```bash
./gradlew buildPlugin
```

On Windows PowerShell or Command Prompt:

```powershell
.\gradlew.bat buildPlugin
```

Use the checked-in Gradle wrapper instead of a system Gradle installation. The wrapper pins the Gradle distribution used by the IntelliJ Platform Gradle Plugin, so local builds and CI do not depend on whichever `gradle` happens to be installed globally.

The packaged plugin will be written under:

```text
build/distributions/
```

## Enable in RustRover or another JetBrains IDE

Install the packaged plugin from disk:

```text
Settings/Preferences -> Plugins -> gear icon -> Install Plugin from Disk...
```

Select the ZIP from `build/distributions/`, then restart the IDE when prompted. After restart, `.doria` files should be associated with the Doria file type and use the Doria syntax highlighter automatically.

If a `.doria` file still opens without highlighting, check:

```text
Settings/Preferences -> Editor -> File Types
```

Make sure `*.doria` is listed under `Doria`, and remove it from `Text` or `Plain Text` if the IDE previously learned that association.

The syntax highlighter, file type registration, comments, and settings page only require the IntelliJ Platform module. `doria-lsp` integration is enabled when the IDE also provides JetBrains' LSP module.

Double-quoted string interpolation such as `{$this->name}` keeps the string text green while colorizing the variable reference. Single-quoted strings are treated as literal strings.

VS Code and IntelliJ / JetBrains highlighting should stay aligned. The shared smoke fixture is:

```text
editors/fixtures/latest-tokens.doria
```

After changing editor highlighting, run this from the repository root:

```bash
php scripts/check_editor_highlighting.php
```

Files under `editors/fixtures/` are syntax-highlighting smoke fixtures. The IntelliJ LSP adapter keeps them out of `doria-lsp` diagnostics so accepted/planned editor vocabulary can be exercised before compiler implementation lands.

Doria uses distinct spellings for imports and trait composition: file/namespace-scope `use` imports names from namespaces, while class-body or trait-body `uses` composes traits. The IntelliJ highlighter keeps these scopes separate as import use and trait-composition uses.

## Run in a sandbox IDE

```bash
./gradlew runIde
```

On Windows PowerShell or Command Prompt:

```powershell
.\gradlew.bat runIde
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

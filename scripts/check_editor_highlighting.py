#!/usr/bin/env python3
"""Check that Doria editor highlighters cover the accepted token vocabulary."""

from __future__ import annotations

import json
import pathlib
import sys
from collections.abc import Iterator
from typing import Any


ROOT = pathlib.Path(__file__).resolve().parents[1]
VSCODE_PACKAGE = ROOT / "editors/vscode/doria/package.json"
VSCODE_GRAMMAR = ROOT / "editors/vscode/doria/syntaxes/doria.tmLanguage.json"
VSCODE_EXTENSION = ROOT / "editors/vscode/doria/extension.js"
INTELLIJ_LEXER = ROOT / "editors/intellij/doria/src/main/kotlin/dev/doria/intellij/highlighting/DoriaLexer.kt"
INTELLIJ_LSP_FILES = ROOT / "editors/intellij/doria/src/main/kotlin/dev/doria/intellij/lsp/DoriaLspFiles.kt"
FIXTURE = ROOT / "editors/fixtures/latest-tokens.doria"

ACCEPTED_KEYWORDS = {
    "class",
    "interface",
    "trait",
    "extends",
    "implements",
    "namespace",
    "use",
    "as",
    "include",
    "declare",
    "break",
    "continue",
    "when",
    "given",
    "finally",
}

PRIMITIVE_TYPES = {
    "void",
    "int",
    "int8",
    "int16",
    "int32",
    "int64",
    "uint8",
    "uint16",
    "uint32",
    "uint64",
    "float",
    "float32",
    "float64",
    "string",
    "bool",
    "mixed",
}

WORD_OPERATORS = {"not", "and", "or", "xor"}
REJECTED_KEYWORDS = {"goto"}
REJECTED_PREPROCESSOR = {
    "include",
    "define",
    "undef",
    "if",
    "ifdef",
    "ifndef",
    "elif",
    "else",
    "endif",
    "warning",
    "error",
}
STRICT_COMPARISON = {"===", "!=="}
NOT_KEYWORDS = {"Option", "Result"}


def fail(message: str) -> None:
    print(f"editor highlighting check failed: {message}", file=sys.stderr)
    raise SystemExit(1)


def require(condition: bool, message: str) -> None:
    if not condition:
        fail(message)


def load_json(path: pathlib.Path) -> Any:
    try:
        with path.open("r", encoding="utf-8") as handle:
            return json.load(handle)
    except json.JSONDecodeError as error:
        fail(f"{path.relative_to(ROOT)} is not valid JSON: {error}")


def walk_patterns(node: Any) -> Iterator[dict[str, Any]]:
    if isinstance(node, dict):
        if "match" in node or "begin" in node or "name" in node:
            yield node
        for value in node.values():
            yield from walk_patterns(value)
    elif isinstance(node, list):
        for item in node:
            yield from walk_patterns(item)


def check_vscode_package() -> None:
    package = load_json(VSCODE_PACKAGE)
    grammars = package.get("contributes", {}).get("grammars", [])
    require(
        any(
            grammar.get("language") == "doria"
            and grammar.get("scopeName") == "source.doria"
            and grammar.get("path") == "./syntaxes/doria.tmLanguage.json"
            for grammar in grammars
        ),
        "VS Code package.json must map doria/source.doria to ./syntaxes/doria.tmLanguage.json",
    )


def check_vscode_grammar() -> None:
    grammar = load_json(VSCODE_GRAMMAR)
    grammar_text = json.dumps(grammar, sort_keys=True)
    patterns = list(walk_patterns(grammar))

    for token in sorted(ACCEPTED_KEYWORDS | PRIMITIVE_TYPES | WORD_OPERATORS):
        require(token in grammar_text, f"VS Code grammar is missing {token!r}")

    for token in sorted(NOT_KEYWORDS):
        require(token not in grammar_text, f"VS Code grammar must not treat {token!r} as a keyword")

    normal_operator_matches = [
        pattern.get("match", "")
        for pattern in patterns
        if pattern.get("name") == "keyword.operator.doria"
    ]
    require(normal_operator_matches, "VS Code grammar must define normal operator highlighting")
    for operator in STRICT_COMPARISON:
        require(
            all(operator not in match for match in normal_operator_matches),
            f"VS Code grammar must not highlight {operator!r} as a normal operator",
        )

    invalid_operator_patterns = [
        pattern.get("match", "")
        for pattern in patterns
        if pattern.get("name") == "invalid.illegal.operator.strict-comparison.doria"
    ]
    for operator in STRICT_COMPARISON:
        require(
            any(operator in match for match in invalid_operator_patterns),
            f"VS Code grammar must mark {operator!r} invalid",
        )

    invalid_preprocessor_patterns = [
        pattern.get("match", "")
        for pattern in patterns
        if pattern.get("name") == "invalid.illegal.preprocessor.doria"
    ]
    require(invalid_preprocessor_patterns, "VS Code grammar must define invalid preprocessor highlighting")
    for directive in sorted(REJECTED_PREPROCESSOR):
        require(
            any(directive in match for match in invalid_preprocessor_patterns),
            f"VS Code grammar must mark #{directive} invalid or unsupported",
        )

    require(
        any(pattern.get("name") == "invalid.illegal.keyword.goto.doria" for pattern in patterns),
        "VS Code grammar must mark goto invalid or unsupported",
    )


def check_intellij_lexer() -> None:
    lexer_text = INTELLIJ_LEXER.read_text(encoding="utf-8")

    for token in sorted(ACCEPTED_KEYWORDS | PRIMITIVE_TYPES | WORD_OPERATORS):
        require(f'"{token}"' in lexer_text, f"IntelliJ lexer is missing {token!r}")

    for token in sorted(NOT_KEYWORDS):
        require(f'"{token}"' not in lexer_text, f"IntelliJ lexer must not treat {token!r} as a keyword")

    for operator in STRICT_COMPARISON:
        require(f'"{operator}"' in lexer_text, f"IntelliJ lexer must recognize {operator!r}")
    require(
        "STRICT_COMPARISON_OPERATORS" in lexer_text and "DoriaTokenTypes.INVALID" in lexer_text,
        "IntelliJ lexer must route strict comparison operators to invalid highlighting",
    )

    require('"goto"' in lexer_text and "INVALID_KEYWORDS" in lexer_text, "IntelliJ lexer must mark goto invalid")
    for directive in sorted(REJECTED_PREPROCESSOR):
        require(f'"{directive}"' in lexer_text, f"IntelliJ lexer must recognize #{directive} as unsupported")


def check_editor_fixture_diagnostics_are_skipped() -> None:
    vscode_text = VSCODE_EXTENSION.read_text(encoding="utf-8")
    intellij_text = INTELLIJ_LSP_FILES.read_text(encoding="utf-8")

    require(
        "/editors/fixtures/" in vscode_text and "isDoriaSource" in vscode_text,
        "VS Code client must keep editor fixtures out of doria-lsp diagnostics",
    )
    require(
        "/editors/fixtures/" in intellij_text and "isDoriaSourceFile" in intellij_text,
        "IntelliJ LSP adapter must keep editor fixtures out of doria-lsp diagnostics",
    )


def check_fixture() -> None:
    fixture_text = FIXTURE.read_text(encoding="utf-8")
    for token in sorted(ACCEPTED_KEYWORDS | WORD_OPERATORS):
        require(token in fixture_text, f"shared editor fixture is missing {token!r}")
    for token in sorted({"int8", "int16", "int32", "int64", "uint8", "uint16", "uint32", "uint64", "float32", "float64"}):
        require(token in fixture_text, f"shared editor fixture is missing {token!r}")


def main() -> int:
    check_vscode_package()
    check_vscode_grammar()
    check_intellij_lexer()
    check_editor_fixture_diagnostics_are_skipped()
    check_fixture()
    print("Doria editor highlighting checks passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

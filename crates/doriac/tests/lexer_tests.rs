use doriac::lexer::{Lexer, StringQuoteKind, TokenKind};
use doriac::source::SourceFile;

fn token_kinds(source: &str) -> Vec<TokenKind> {
    let source = SourceFile::new("test.doria", source);
    Lexer::new(&source)
        .lex()
        .expect("lexing should succeed")
        .into_iter()
        .map(|token| token.kind)
        .collect()
}

#[test]
fn lexes_declarations_and_generics() {
    let kinds = token_kinds(
        r#"let writable $name = "Doria";
Dictionary<string, int> $items = ["apples" => 5];"#,
    );

    assert!(matches!(kinds[0], TokenKind::Let));
    assert!(matches!(kinds[1], TokenKind::Writable));
    assert!(matches!(kinds[2], TokenKind::Variable(ref name) if name == "name"));
    assert!(matches!(kinds[6], TokenKind::Identifier(ref name) if name == "Dictionary"));
    assert!(kinds.iter().any(|kind| matches!(kind, TokenKind::FatArrow)));
}

#[test]
fn lexes_string_quote_kinds() {
    let kinds = token_kinds(r#"'{}' "{}""#);
    assert!(matches!(
        &kinds[0],
        TokenKind::StringLiteral {
            value,
            quote: StringQuoteKind::Single,
        } if value == "{}"
    ));
    assert!(matches!(
        &kinds[1],
        TokenKind::StringLiteral {
            value,
            quote: StringQuoteKind::Double,
        } if value == "{}"
    ));
}

#[test]
fn lexes_basic_control_flow_keywords() {
    let kinds = token_kinds("if else while");
    assert!(matches!(kinds[0], TokenKind::If));
    assert!(matches!(kinds[1], TokenKind::Else));
    assert!(matches!(kinds[2], TokenKind::While));
}

#[test]
fn lexes_loop_control_keywords() {
    let kinds = token_kinds("break continue");
    assert!(matches!(kinds[0], TokenKind::Break));
    assert!(matches!(kinds[1], TokenKind::Continue));
}

#[test]
fn lexes_boolean_word_operators() {
    let kinds = token_kinds("not and or xor");
    assert!(matches!(kinds[0], TokenKind::Not));
    assert!(matches!(kinds[1], TokenKind::And));
    assert!(matches!(kinds[2], TokenKind::Or));
    assert!(matches!(kinds[3], TokenKind::Xor));
}

#[test]
fn rejects_php_strict_equality_tokens() {
    for (source, message) in [
        (
            "echo 1 === 1;",
            "Doria uses typed `==`; `===` is not supported",
        ),
        (
            "echo 1 !== 1;",
            "Doria uses typed `!=`; `!==` is not supported",
        ),
    ] {
        let err = doriac::lex_source("test.doria", source)
            .expect_err("strict equality token should be rejected");
        assert!(
            err.iter()
                .any(|diagnostic| diagnostic.message.contains(message)),
            "expected diagnostic containing {message}, got {err:?}"
        );
    }
}
#[test]
fn lexes_future_reserved_words() {
    let kinds = token_kinds("async await spawn scope");
    assert!(matches!(kinds[0], TokenKind::Reserved(ref word) if word == "async"));
    assert!(matches!(kinds[1], TokenKind::Reserved(ref word) if word == "await"));
    assert!(matches!(kinds[2], TokenKind::Reserved(ref word) if word == "spawn"));
    assert!(matches!(kinds[3], TokenKind::Reserved(ref word) if word == "scope"));
}

#[test]
fn lexes_planned_control_flow_words_as_identifiers() {
    let kinds = token_kinds("when finally");
    assert!(matches!(kinds[0], TokenKind::Identifier(ref word) if word == "when"));
    assert!(matches!(kinds[1], TokenKind::Identifier(ref word) if word == "finally"));
}

#[test]
fn lexes_result_and_option_as_identifiers() {
    let kinds = token_kinds("Result Option");
    assert!(matches!(kinds[0], TokenKind::Identifier(ref word) if word == "Result"));
    assert!(matches!(kinds[1], TokenKind::Identifier(ref word) if word == "Option"));
}

#[test]
fn lexes_internal_keyword() {
    let kinds = token_kinds("internal writable string $name;");
    assert!(matches!(kinds[0], TokenKind::Internal));
    assert!(matches!(kinds[1], TokenKind::Writable));
}

#[test]
fn lexes_non_doria_visibility_words_as_identifiers() {
    let kinds = token_kinds("public protected private");
    assert!(matches!(kinds[0], TokenKind::Identifier(ref word) if word == "public"));
    assert!(matches!(kinds[1], TokenKind::Identifier(ref word) if word == "protected"));
    assert!(matches!(kinds[2], TokenKind::Identifier(ref word) if word == "private"));
}

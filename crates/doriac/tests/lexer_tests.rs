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
fn lexes_stage_9_iteration_tokens() {
    let kinds = token_kinds(
        r#"
for (let writable $i = 0; $i < 10; $i++) {
}

++$i;
--$i;
$i++;
$i--;

foreach (0..10 as $i) {
}

foreach (0..<10 as $i) {
}
"#,
    );

    assert!(kinds.iter().any(|kind| matches!(kind, TokenKind::For)));
    assert!(kinds.iter().any(|kind| matches!(kind, TokenKind::Foreach)));
    assert!(kinds.iter().any(|kind| matches!(kind, TokenKind::As)));
    assert!(kinds.iter().any(|kind| matches!(kind, TokenKind::PlusPlus)));
    assert!(kinds
        .iter()
        .any(|kind| matches!(kind, TokenKind::MinusMinus)));
    assert!(kinds.iter().any(|kind| matches!(kind, TokenKind::DotDot)));
    assert!(kinds
        .iter()
        .any(|kind| matches!(kind, TokenKind::DotDotLess)));
}

#[test]
fn lexes_checked_error_direction_keywords() {
    let kinds = token_kinds("throw throws");
    assert!(matches!(kinds[0], TokenKind::Throw));
    assert!(matches!(kinds[1], TokenKind::Throws));
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
fn lexes_stage_13_primitive_type_spellings() {
    let kinds =
        token_kinds("int int8 int16 int32 int64 uint8 uint16 uint32 uint64 float float32 float64");

    assert_eq!(
        kinds,
        vec![
            TokenKind::IntType,
            TokenKind::Int8Type,
            TokenKind::Int16Type,
            TokenKind::Int32Type,
            TokenKind::Int64Type,
            TokenKind::UInt8Type,
            TokenKind::UInt16Type,
            TokenKind::UInt32Type,
            TokenKind::UInt64Type,
            TokenKind::FloatType,
            TokenKind::Float32Type,
            TokenKind::Float64Type,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn lexes_stage_13_integer_operators_and_compound_assignments() {
    let kinds = token_kinds("<< >> & | ^ ~ *= /= %= <<= >>= &= |= ^=");

    assert_eq!(
        kinds,
        vec![
            TokenKind::ShiftLeft,
            TokenKind::ShiftRight,
            TokenKind::Ampersand,
            TokenKind::Pipe,
            TokenKind::Caret,
            TokenKind::Tilde,
            TokenKind::StarEquals,
            TokenKind::SlashEquals,
            TokenKind::PercentEquals,
            TokenKind::ShiftLeftEquals,
            TokenKind::ShiftRightEquals,
            TokenKind::AmpersandEquals,
            TokenKind::PipeEquals,
            TokenKind::CaretEquals,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn keeps_boolean_and_bitwise_symbol_tokens_distinct() {
    let kinds = token_kinds("& && | || ^ xor");

    assert_eq!(
        kinds,
        vec![
            TokenKind::Ampersand,
            TokenKind::AndAnd,
            TokenKind::Pipe,
            TokenKind::OrOr,
            TokenKind::Caret,
            TokenKind::Xor,
            TokenKind::Eof,
        ]
    );
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

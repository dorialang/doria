use doriac::lexer::{Lexer, TokenKind};
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
fn lexes_future_reserved_words() {
    let kinds = token_kinds("async await spawn scope");
    assert!(matches!(kinds[0], TokenKind::Reserved(ref word) if word == "async"));
    assert!(matches!(kinds[1], TokenKind::Reserved(ref word) if word == "await"));
    assert!(matches!(kinds[2], TokenKind::Reserved(ref word) if word == "spawn"));
    assert!(matches!(kinds[3], TokenKind::Reserved(ref word) if word == "scope"));
}

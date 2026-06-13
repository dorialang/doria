use doriac::ast::{Item, Stmt};

#[test]
fn parses_variable_declarations() {
    let program = doriac::parse_source(
        "test.doria",
        r#"
let $x = 5;
let writable $name = "Doria";
writable int $score = 1;
"#,
    )
    .expect("parse should succeed");

    assert_eq!(program.items.len(), 3);
    assert!(matches!(
        &program.items[0],
        Item::Statement(Stmt::VarDecl(decl)) if !decl.writable && decl.name == "x"
    ));
    assert!(matches!(
        &program.items[1],
        Item::Statement(Stmt::VarDecl(decl)) if decl.writable && decl.name == "name"
    ));
    assert!(matches!(
        &program.items[2],
        Item::Statement(Stmt::VarDecl(decl)) if decl.writable && decl.ty.is_some()
    ));
}

#[test]
fn parses_class_with_writable_method() {
    let program = doriac::parse_source(
        "test.doria",
        r#"
class Person
{
    public writable string $name;

    public writable function rename(string $name): void
    {
        $this->name = $name;
    }
}
"#,
    )
    .expect("parse should succeed");

    assert!(matches!(&program.items[0], Item::Class(class_decl) if class_decl.name == "Person"));
}

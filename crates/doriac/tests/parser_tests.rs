use doriac::ast::{ClassMember, Item, MemberAccess, Stmt};

#[test]
fn parses_variable_declarations() {
    let program = doriac::parse_source(
        "test.doria",
        r#"
let $x = 5;
let writable $name = "Doria";
writable int $score = 1;
null $empty = null;
"#,
    )
    .expect("parse should succeed");

    assert_eq!(program.items.len(), 4);
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
    assert!(matches!(
        &program.items[3],
        Item::Statement(Stmt::VarDecl(decl))
            if !decl.writable
                && decl.name == "empty"
                && matches!(decl.ty.as_ref(), Some(ty) if ty.name == "null")
    ));
}

#[test]
fn parses_class_with_writable_method() {
    let program = doriac::parse_source(
        "test.doria",
        r#"
class Person
{
    writable string $name;

    writable function rename(string $name): void
    {
        $this->name = $name;
    }
}
"#,
    )
    .expect("parse should succeed");

    assert!(matches!(&program.items[0], Item::Class(class_decl) if class_decl.name == "Person"));
}

#[test]
fn parses_default_external_and_internal_members() {
    let program = doriac::parse_source(
        "test.doria",
        r#"
class Parser
{
    string $name;
    internal string $slug;
    internal writable int $position = 0;

    function parse(): Ast
    {
        return $this->parseProgram();
    }

    internal function parseProgram(): Ast
    {
        return new Ast();
    }

    internal writable function advance(): void
    {
        $this->position = $this->position + 1;
    }
}
"#,
    )
    .expect("parse should succeed");

    let Item::Class(class_decl) = &program.items[0] else {
        panic!("expected class");
    };

    assert!(matches!(
        &class_decl.members[0],
        ClassMember::Property(property)
            if property.name == "name"
                && property.access == MemberAccess::External
                && !property.writable
    ));
    assert!(matches!(
        &class_decl.members[1],
        ClassMember::Property(property)
            if property.name == "slug"
                && property.access == MemberAccess::Internal
                && !property.writable
    ));
    assert!(matches!(
        &class_decl.members[2],
        ClassMember::Property(property)
            if property.name == "position"
                && property.access == MemberAccess::Internal
                && property.writable
    ));
    assert!(matches!(
        &class_decl.members[3],
        ClassMember::Method(method)
            if method.name == "parse"
                && method.access == MemberAccess::External
                && !method.writable_this
    ));
    assert!(matches!(
        &class_decl.members[4],
        ClassMember::Method(method)
            if method.name == "parseProgram"
                && method.access == MemberAccess::Internal
                && !method.writable_this
    ));
    assert!(matches!(
        &class_decl.members[5],
        ClassMember::Method(method)
            if method.name == "advance"
                && method.access == MemberAccess::Internal
                && method.writable_this
    ));
}

#[test]
fn rejects_unsupported_visibility_member_syntax() {
    for source in [
        "class Person { public string $name; }",
        "class Person { public function greet(): void {} }",
        "class Person { private string $name; }",
        "class Person { private function greet(): void {} }",
        "class Person { protected string $name; }",
        "class Person { protected function greet(): void {} }",
    ] {
        let err = doriac::parse_source("test.doria", source)
            .expect_err("unsupported visibility syntax should be rejected");

        assert!(
            err.iter().any(|diagnostic| diagnostic.code == "P0001"),
            "expected parse diagnostic for source `{source}`"
        );
    }
}

use doriac::ast::{
    BinaryOp, ClassMember, ElseBranch, Expr, InterpolatedStringPart, Item, MemberAccess, Stmt,
    UnaryOp,
};

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

fn parse_echo_expr(source: &str) -> Expr {
    let program = doriac::parse_source("test.doria", source).expect("parse should succeed");
    let Item::Statement(Stmt::Echo { expr, .. }) = &program.items[0] else {
        panic!("expected echo statement");
    };
    expr.clone()
}

#[test]
fn parses_boolean_word_operators() {
    assert!(matches!(
        parse_echo_expr("echo true and false;"),
        Expr::Binary {
            op: BinaryOp::And,
            ..
        }
    ));
    assert!(matches!(
        parse_echo_expr("echo false or true;"),
        Expr::Binary {
            op: BinaryOp::Or,
            ..
        }
    ));
    assert!(matches!(
        parse_echo_expr("echo true xor false;"),
        Expr::Binary {
            op: BinaryOp::Xor,
            ..
        }
    ));
    assert!(matches!(
        parse_echo_expr("echo not false;"),
        Expr::Unary {
            op: UnaryOp::Not,
            ..
        }
    ));
}

#[test]
fn rejects_ambiguous_xor_expressions() {
    for source in [
        "echo true xor false xor true;",
        "echo true and false xor true;",
        "echo true xor false or true;",
    ] {
        let err = doriac::parse_source("test.doria", source)
            .expect_err("ambiguous xor expression should be rejected");
        assert!(
            err.iter()
                .any(|diagnostic| diagnostic.message.contains("ambiguous `xor`")),
            "expected ambiguous xor diagnostic, got {err:?}"
        );
    }
}

#[test]
fn accepts_parenthesized_xor_expressions() {
    for source in [
        "echo (true xor false) xor true;",
        "echo (true and false) xor true;",
        "echo true xor (false or true);",
    ] {
        doriac::parse_source("test.doria", source)
            .unwrap_or_else(|err| panic!("parenthesized xor expression should parse: {err:?}"));
    }
}

#[test]
fn parses_plain_and_interpolated_strings() {
    assert!(matches!(
        parse_echo_expr("echo '{$name}';"),
        Expr::String { value, .. } if value == "{$name}"
    ));
    assert!(matches!(
        parse_echo_expr("echo \"Hello\";"),
        Expr::String { value, .. } if value == "Hello"
    ));
    assert!(matches!(
        parse_echo_expr("echo \"{}\";"),
        Expr::String { value, .. } if value == "{}"
    ));

    let Expr::InterpolatedString { parts, .. } = parse_echo_expr("echo \"Hello, {$name}\";") else {
        panic!("expected interpolated string");
    };
    assert!(matches!(&parts[0], InterpolatedStringPart::Text(text) if text == "Hello, "));
    assert!(matches!(
        &parts[1],
        InterpolatedStringPart::Expr(Expr::Variable { name, .. }) if name == "name"
    ));

    let Expr::InterpolatedString { parts, .. } = parse_echo_expr("echo \"Hello, {$this->name}\";")
    else {
        panic!("expected interpolated string");
    };
    assert!(matches!(
        &parts[1],
        InterpolatedStringPart::Expr(Expr::PropertyAccess { object, property, .. })
            if matches!(object.as_ref(), Expr::This { .. }) && property == "name"
    ));

    let Expr::InterpolatedString { parts, .. } = parse_echo_expr("echo \"{$first} {$last}\";")
    else {
        panic!("expected interpolated string");
    };
    assert_eq!(parts.len(), 3);
    assert!(matches!(
        &parts[0],
        InterpolatedStringPart::Expr(Expr::Variable { name, .. }) if name == "first"
    ));
    assert!(matches!(&parts[1], InterpolatedStringPart::Text(text) if text == " "));
    assert!(matches!(
        &parts[2],
        InterpolatedStringPart::Expr(Expr::Variable { name, .. }) if name == "last"
    ));
}

#[test]
fn rejects_malformed_or_unsupported_string_interpolation() {
    for (source, message) in [
        (
            "echo \"Hello, {$name\";",
            "unterminated string interpolation",
        ),
        ("echo \"Hello, {$}\";", "empty string interpolation"),
        (
            "echo \"Total: {$a + $b}\";",
            "unsupported string interpolation expression",
        ),
        (
            "echo \"Name: {$user->name()}\";",
            "unsupported string interpolation expression",
        ),
    ] {
        let err = doriac::parse_source("test.doria", source)
            .expect_err("parse should reject unsupported interpolation");
        assert!(
            err.iter()
                .any(|diagnostic| diagnostic.message.contains(message)),
            "expected diagnostic containing {message}, got {err:?}"
        );
    }
}
#[test]
fn parses_if_else_and_while_control_flow() {
    let program = doriac::parse_source(
        "test.doria",
        r#"
if (true) {
    echo "yes";
}

if ($age < 13) {
    echo "child";
} else if ($age < 20) {
    echo "teen";
} else {
    echo "adult";
}

while ($count < 10) {
    $count += 1;
}
"#,
    )
    .expect("parse should succeed");

    let Item::Statement(Stmt::If(simple_if)) = &program.items[0] else {
        panic!("expected if statement");
    };
    assert!(matches!(
        simple_if.condition,
        Expr::Bool { value: true, .. }
    ));
    assert_eq!(simple_if.then_block.statements.len(), 1);
    assert!(simple_if.else_branch.is_none());

    let Item::Statement(Stmt::If(if_stmt)) = &program.items[1] else {
        panic!("expected if statement");
    };
    let Some(ElseBranch::If(else_if)) = &if_stmt.else_branch else {
        panic!("expected else-if branch");
    };
    assert!(matches!(else_if.condition, Expr::Binary { .. }));
    assert!(matches!(else_if.else_branch, Some(ElseBranch::Block(_))));

    let Item::Statement(Stmt::While(while_stmt)) = &program.items[2] else {
        panic!("expected while statement");
    };
    assert!(matches!(while_stmt.condition, Expr::Binary { .. }));
    assert_eq!(while_stmt.body.statements.len(), 1);
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
fn parses_planned_control_flow_words_as_declaration_names() {
    let program = doriac::parse_source(
        "test.doria",
        r#"
function when(): void {}
class finally {}
"#,
    )
    .expect("parse should succeed");

    assert!(matches!(
        &program.items[0],
        Item::Function(function) if function.name == "when"
    ));
    assert!(matches!(
        &program.items[1],
        Item::Class(class_decl) if class_decl.name == "finally"
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

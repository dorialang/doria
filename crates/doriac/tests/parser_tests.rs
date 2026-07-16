use doriac::ast::{
    AssignOp, BinaryOp, ClassMember, ElseBranch, Expr, ForIncrement, ForInitializer, IncrementOp,
    IncrementPosition, InterpolatedStringPart, Item, MemberAccess, Stmt, UnaryOp,
};

#[test]
fn parses_class_workflow_and_qualified_type_syntax_before_semantics_land() {
    let program = doriac::parse_source(
        "test.doria",
        r#"
namespace Vendor\App;

class Child extends Vendor\Base implements Vendor\Contracts\Printable
{
    function convert(Vendor\Input $input): Vendor\Output
    {
        return new Vendor\Output();
    }
}
"#,
    )
    .expect("accepted namespace and inheritance syntax should parse");

    assert_eq!(
        program
            .namespace
            .as_ref()
            .map(|namespace| namespace.name.as_str()),
        Some("Vendor\\App")
    );
    let Item::Class(class) = &program.items[0] else {
        panic!("expected class declaration");
    };
    assert_eq!(class.parent.as_deref(), Some("Vendor\\Base"));
    assert_eq!(class.implements, ["Vendor\\Contracts\\Printable"]);
    let ClassMember::Method(method) = &class.members[0] else {
        panic!("expected method declaration");
    };
    assert_eq!(method.params[0].ty.name, "Vendor\\Input");
    assert_eq!(
        method.return_type.as_ref().map(|ty| ty.name.as_str()),
        Some("Vendor\\Output")
    );
}

#[test]
fn parses_interface_declarations_before_semantics_land() {
    let program = doriac::parse_source(
        "test.doria",
        r#"
interface Printable
{
    function render(): string;
}
"#,
    )
    .expect("accepted interface syntax should parse");

    let Item::Interface(interface) = &program.items[0] else {
        panic!("expected interface declaration");
    };
    assert_eq!(interface.name, "Printable");
}

#[test]
fn parses_variable_declarations() {
    let program = doriac::parse_source(
        "test.doria",
        r#"
let $x = 5;
let writable $name = "Doria";
writable int $score = 1;
null $empty = null;
int[] $numbers = [1, 2, 3];
"#,
    )
    .expect("parse should succeed");

    assert_eq!(program.items.len(), 5);
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
    assert!(matches!(
        &program.items[4],
        Item::Statement(Stmt::VarDecl(decl))
            if !decl.writable
                && decl.name == "numbers"
                && matches!(decl.ty.as_ref(), Some(ty)
                    if ty.name == "[]" && ty.args.len() == 1 && ty.args[0].name == "int")
    ));
}

#[test]
fn parses_stage_13_primitive_type_spellings() {
    let program = doriac::parse_source(
        "test.doria",
        r#"
int8 $int8Value = 0;
int16 $int16Value = 0;
int32 $int32Value = 0;
int64 $int64Value = 0;
uint8 $uint8Value = 0;
uint16 $uint16Value = 0;
uint32 $uint32Value = 0;
uint64 $uint64Value = 0;
float32 $float32Value = 0.0;
float64 $float64Value = 0.0;
"#,
    )
    .expect("parse should succeed");

    let names = program
        .items
        .iter()
        .map(|item| {
            let Item::Statement(Stmt::VarDecl(decl)) = item else {
                panic!("expected variable declaration");
            };
            decl.ty
                .as_ref()
                .expect("expected explicit type")
                .name
                .as_str()
        })
        .collect::<Vec<_>>();

    assert_eq!(
        names,
        [
            "int8", "int16", "int32", "int64", "uint8", "uint16", "uint32", "uint64", "float32",
            "float64",
        ]
    );
}

#[test]
fn parses_adjacent_nested_generic_type_closers_after_shift_tokens_are_added() {
    let program = doriac::parse_source(
        "test.doria",
        "Dictionary<string, List<uint64>> $values = [];",
    )
    .expect("nested generic type should parse without whitespace between closing angles");

    let Item::Statement(Stmt::VarDecl(decl)) = &program.items[0] else {
        panic!("expected variable declaration");
    };
    let ty = decl.ty.as_ref().expect("expected explicit type");
    assert_eq!(ty.name, "Dictionary");
    assert_eq!(ty.args[1].name, "List");
    assert_eq!(ty.args[1].args[0].name, "uint64");
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
fn parses_stage_13_unary_and_binary_operators() {
    for (source, expected) in [
        ("echo -$value;", UnaryOp::Negate),
        ("echo ~$value;", UnaryOp::BitwiseNot),
    ] {
        assert!(matches!(
            parse_echo_expr(source),
            Expr::Unary { op, .. } if op == expected
        ));
    }

    for (source, expected) in [
        ("echo $a / $b;", BinaryOp::Div),
        ("echo $a % $b;", BinaryOp::Mod),
        ("echo $a << $b;", BinaryOp::ShiftLeft),
        ("echo $a >> $b;", BinaryOp::ShiftRight),
        ("echo $a & $b;", BinaryOp::BitwiseAnd),
        ("echo $a ^ $b;", BinaryOp::BitwiseXor),
        ("echo $a | $b;", BinaryOp::BitwiseOr),
    ] {
        assert!(matches!(
            parse_echo_expr(source),
            Expr::Binary { op, .. } if op == expected
        ));
    }
}

#[test]
fn parses_shift_below_additive_precedence() {
    let Expr::Binary {
        left,
        op: BinaryOp::ShiftLeft,
        right,
        ..
    } = parse_echo_expr("echo 1 + 2 << 1;")
    else {
        panic!("expected outer shift-left expression");
    };

    assert!(matches!(
        left.as_ref(),
        Expr::Binary {
            op: BinaryOp::Add,
            ..
        }
    ));
    assert!(matches!(right.as_ref(), Expr::Int { value, .. } if value == "1"));
}

#[test]
fn parses_equality_before_bitwise_and() {
    let Expr::Binary {
        left,
        op: BinaryOp::BitwiseAnd,
        right,
        ..
    } = parse_echo_expr("echo 1 & 2 == 0;")
    else {
        panic!("expected outer bitwise-and expression");
    };

    assert!(matches!(left.as_ref(), Expr::Int { value, .. } if value == "1"));
    assert!(matches!(
        right.as_ref(),
        Expr::Binary {
            op: BinaryOp::Equal,
            ..
        }
    ));
}

#[test]
fn keeps_bitwise_and_boolean_xor_distinct() {
    assert!(matches!(
        parse_echo_expr("echo $a ^ $b;"),
        Expr::Binary {
            op: BinaryOp::BitwiseXor,
            ..
        }
    ));
    assert!(matches!(
        parse_echo_expr("echo $a xor $b;"),
        Expr::Binary {
            op: BinaryOp::Xor,
            ..
        }
    ));
}

#[test]
fn parses_all_stage_13_compound_assignments() {
    let program = doriac::parse_source(
        "test.doria",
        r#"
$value += 1;
$value -= 1;
$value *= 1;
$value /= 1;
$value %= 1;
$value <<= 1;
$value >>= 1;
$value &= 1;
$value |= 1;
$value ^= 1;
"#,
    )
    .expect("compound assignments should parse");

    let expected = [
        AssignOp::AddAssign,
        AssignOp::SubAssign,
        AssignOp::MulAssign,
        AssignOp::DivAssign,
        AssignOp::ModAssign,
        AssignOp::ShiftLeftAssign,
        AssignOp::ShiftRightAssign,
        AssignOp::BitwiseAndAssign,
        AssignOp::BitwiseOrAssign,
        AssignOp::BitwiseXorAssign,
    ];

    for (item, expected) in program.items.iter().zip(expected) {
        assert!(matches!(
            item,
            Item::Statement(Stmt::Assignment(assignment)) if assignment.op == expected
        ));
    }
}

#[test]
fn structural_lowering_preserves_stage_13_operator_variants() {
    let ast = doriac::parse_source(
        "test.doria",
        "$value <<= 1; echo ~-$value | $mask ^ $other & 1;",
    )
    .expect("parse should succeed");
    let hir = doriac::lowering::lower_program(&ast)
        .expect("AST without interface declarations should lower structurally");

    assert!(matches!(
        &hir.items[0],
        doriac::hir::Item::Statement(doriac::hir::Stmt::Assignment(assignment))
            if assignment.op == AssignOp::ShiftLeftAssign
    ));
    assert!(matches!(
        &hir.items[1],
        doriac::hir::Item::Statement(doriac::hir::Stmt::Echo {
            expr: doriac::hir::Expr::Binary {
                op: BinaryOp::BitwiseOr,
                ..
            },
            ..
        })
    ));
}

#[test]
fn direct_lowering_reports_accepted_interface_declarations() {
    let ast = doriac::parse_source("test.doria", "interface Printable {}")
        .expect("accepted interface declaration should parse");
    let diagnostics = doriac::lowering::lower_program(&ast)
        .expect_err("unsupported interface declaration should not reach Doria IR");

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, "E0464");
    assert!(diagnostics[0]
        .message
        .contains("interface declaration `Printable`"));
}

#[test]
fn parses_string_concat_operator() {
    let expr = parse_echo_expr(r#"echo "Hello " . $name . "!";"#);
    let Expr::Binary {
        left,
        op: BinaryOp::Concat,
        right,
        ..
    } = expr
    else {
        panic!("expected outer concat expression");
    };

    assert!(matches!(right.as_ref(), Expr::String { value, .. } if value == "!"));
    let Expr::Binary {
        left: inner_left,
        op: BinaryOp::Concat,
        right: inner_right,
        ..
    } = left.as_ref()
    else {
        panic!("expected left-associative inner concat expression");
    };
    assert!(matches!(inner_left.as_ref(), Expr::String { value, .. } if value == "Hello "));
    assert!(matches!(inner_right.as_ref(), Expr::Variable { name, .. } if name == "name"));
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
        parse_echo_expr("echo \"\\{}\";"),
        Expr::String { value, .. } if value == "{}"
    ));

    let Expr::InterpolatedString { parts, .. } = parse_echo_expr("echo \"Hello, {$name}\";") else {
        panic!("expected interpolated string");
    };
    assert!(matches!(
        &parts[0],
        InterpolatedStringPart::Text { value, span }
            if value == "Hello, " && *span == doriac::source::Span::new(6, 13)
    ));
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
    assert!(matches!(
        &parts[1],
        InterpolatedStringPart::Text { value, .. } if value == " "
    ));
    assert!(matches!(
        &parts[2],
        InterpolatedStringPart::Expr(Expr::Variable { name, .. }) if name == "last"
    ));
}

#[test]
fn parses_full_expressions_inside_interpolated_strings() {
    for (source, expected) in [
        ("echo \"{$a + $b}\";", "binary"),
        ("echo \"{formatValue($value)}\";", "function call"),
        ("echo \"{($a + $b) * 2}\";", "grouped expression"),
        ("echo \"{Counter::next()}\";", "static call"),
        ("echo \"{true}\";", "boolean"),
        ("echo \"{$a < $b}\";", "comparison"),
    ] {
        let Expr::InterpolatedString { parts, .. } = parse_echo_expr(source) else {
            panic!("expected an interpolated string for {expected}");
        };
        assert!(
            matches!(parts.as_slice(), [InterpolatedStringPart::Expr(_)]),
            "expected one {expected} interpolation, got {parts:?}"
        );
    }

    let Expr::InterpolatedString { parts, .. } =
        parse_echo_expr("echo \"{formatValue(\"left\")} {formatValue('right')}\";")
    else {
        panic!("expected interpolated string with nested quoted arguments");
    };
    assert_eq!(parts.len(), 3);

    let Expr::InterpolatedString { parts, .. } =
        parse_echo_expr("echo \"{formatValue(1 /* } */ + 2)}\";")
    else {
        panic!("expected interpolation with an ordinary expression comment");
    };
    assert!(matches!(
        parts.as_slice(),
        [InterpolatedStringPart::Expr(Expr::FunctionCall { .. })]
    ));

    let Expr::InterpolatedString { parts, .. } = parse_echo_expr(r#"echo "{$first}{$second}";"#)
    else {
        panic!("expected adjacent interpolation parts");
    };
    assert_eq!(parts.len(), 2);
}

#[test]
fn applies_the_stage_18_literal_brace_rule() {
    assert!(matches!(
        parse_echo_expr(r#"echo "\{literal}";"#),
        Expr::String { value, .. } if value == "{literal}"
    ));
    assert!(matches!(
        parse_echo_expr(r#"echo "right } and escaped \}";"#),
        Expr::String { value, .. } if value == "right } and escaped }"
    ));

    let Expr::InterpolatedString { parts, .. } = parse_echo_expr(r#"echo "\{{$value}}";"#) else {
        panic!("expected escaped brace adjacent to interpolation");
    };
    assert!(matches!(
        parts.as_slice(),
        [
            InterpolatedStringPart::Text { value: open, .. },
            InterpolatedStringPart::Expr(Expr::Variable { name, .. }),
            InterpolatedStringPart::Text { value: close, .. },
        ] if open == "{" && name == "value" && close == "}"
    ));

    let Expr::InterpolatedString { parts, .. } = parse_echo_expr(r#"echo "{formatValue("\{")}";"#)
    else {
        panic!("expected interpolation containing an escaped brace in an inner string");
    };
    assert!(matches!(
        parts.as_slice(),
        [InterpolatedStringPart::Expr(Expr::FunctionCall { name, .. })]
            if name == "formatValue"
    ));
}

#[test]
fn rejects_malformed_string_interpolation() {
    for (source, message) in [
        (
            "echo \"Hello, {$name\";",
            "unterminated string interpolation",
        ),
        ("echo \"Hello, {}\";", "empty string interpolation"),
        ("echo \"Hello, {$}\";", "expected variable name after `$`"),
        ("echo \"{/* comment */}\";", "expected expression"),
        ("echo \"{// comment\n}\";", "expected expression"),
        ("echo \"A literal {word}\";", "unescaped `{`"),
        ("echo \"{foo-bar}\";", "unescaped `{`"),
        ("echo \"{word . suffix}\";", "unescaped `{`"),
        ("echo \"{{word}}\";", "unescaped `{`"),
    ] {
        let err = doriac::parse_source("test.doria", source)
            .expect_err("parse should reject malformed interpolation");
        assert!(
            err.iter()
                .any(|diagnostic| diagnostic.message.contains(message)),
            "expected diagnostic containing {message}, got {err:?}"
        );
    }
}

#[test]
fn rejects_truncated_collections_without_recursive_eof_parsing() {
    for source in ["[", "f.unction ma { ec ;void { ec ; [\n"] {
        let diagnostics = doriac::parse_source("fuzz-regression.doria", source)
            .expect_err("truncated collection input must produce a diagnostic");
        assert!(!diagnostics.is_empty());
    }
}

#[test]
fn identifier_composite_interpolations_receive_the_literal_brace_fix() {
    for source in ["echo \"{foo-bar}\";", "echo \"{word . suffix}\";"] {
        let diagnostics = doriac::parse_source("test.doria", source)
            .expect_err("bare identifier composites must not become interpolation expressions");
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "P0002")
            .unwrap_or_else(|| panic!("expected P0002 for {source}, got {diagnostics:?}"));
        let fix = diagnostic
            .fix
            .as_ref()
            .expect("P0002 should carry a machine-applicable fix");
        assert_eq!(fix.replacement, "\\{");
        assert_eq!(fix.span.start, source.find('{').expect("opening brace"));
    }
}

#[test]
fn keeps_interpolation_diagnostics_on_original_source_offsets() {
    let source = "echo \"prefix {1 + } suffix\";";
    let diagnostics = doriac::parse_source("test.doria", source)
        .expect_err("malformed interpolation expression should fail");
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.message.contains("expected expression"))
        .expect("ordinary parser diagnostic should be preserved");
    assert_eq!(diagnostic.span.start, source.find('+').expect("operator"));

    let source = "echo \"literal {word}\";";
    let diagnostics = doriac::parse_source("test.doria", source)
        .expect_err("bare literal opening brace should fail");
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "P0002")
        .expect("literal brace diagnostic should be present");
    let fix = diagnostic
        .fix
        .as_ref()
        .expect("diagnostic should carry a fix");
    assert_eq!(fix.span, diagnostic.span);
    assert_eq!(fix.replacement, "\\{");

    let diagnostics = doriac::parse_source("test.doria", "echo \"{1 + }\"; echo \"{2 + }\";")
        .expect_err("each malformed interpolation should be diagnosed");
    assert_eq!(
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.message == "expected expression")
            .count(),
        2
    );
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
fn parses_break_and_continue_statements() {
    let program = doriac::parse_source(
        "test.doria",
        r#"
while (true) {
    break;
}

while (true) {
    continue;
}
"#,
    )
    .expect("parse should succeed");

    let Item::Statement(Stmt::While(break_loop)) = &program.items[0] else {
        panic!("expected break loop");
    };
    assert!(matches!(
        break_loop.body.statements.as_slice(),
        [Stmt::Break { .. }]
    ));

    let Item::Statement(Stmt::While(continue_loop)) = &program.items[1] else {
        panic!("expected continue loop");
    };
    assert!(matches!(
        continue_loop.body.statements.as_slice(),
        [Stmt::Continue { .. }]
    ));
}

#[test]
fn rejects_numeric_or_labeled_loop_control() {
    for (source, message) in [
        (
            "while (true) { break 2; }",
            "`break` does not accept a value or label in this Doria slice",
        ),
        (
            "while (true) { continue 2; }",
            "`continue` does not accept a value or label in this Doria slice",
        ),
        (
            "while (true) { break outer; }",
            "`break` does not accept a value or label in this Doria slice",
        ),
        (
            "while (true) { continue outer; }",
            "`continue` does not accept a value or label in this Doria slice",
        ),
    ] {
        let err = doriac::parse_source("test.doria", source)
            .expect_err("numeric or labeled loop control should be rejected");
        assert!(
            err.iter()
                .any(|diagnostic| diagnostic.message.contains(message)),
            "expected diagnostic containing {message}, got {err:?}"
        );
    }
}

#[test]
fn parses_stage_9_for_loops_and_mutation_statements() {
    let program = doriac::parse_source(
        "test.doria",
        r#"
for (let writable $i = 0; $i < 10; $i++) {
    echo $i;
}

for (let writable $i = 0; $i < 10; ++$i) {
}

for (let writable $i = 10; $i > 0; $i--) {
}

$i++;
++$i;
$i--;
--$i;
"#,
    )
    .expect("parse should succeed");

    let Item::Statement(Stmt::For(first_for)) = &program.items[0] else {
        panic!("expected for loop");
    };
    assert!(matches!(
        &first_for.initializer,
        Some(ForInitializer::VarDecl(decl)) if decl.writable && decl.name == "i"
    ));
    assert!(matches!(first_for.condition, Some(Expr::Binary { .. })));
    assert!(matches!(
        &first_for.increment,
        Some(ForIncrement::Increment(increment))
            if increment.op == IncrementOp::Increment
                && increment.position == IncrementPosition::Post
    ));

    let Item::Statement(Stmt::For(second_for)) = &program.items[1] else {
        panic!("expected for loop");
    };
    assert!(matches!(
        &second_for.increment,
        Some(ForIncrement::Increment(increment))
            if increment.op == IncrementOp::Increment
                && increment.position == IncrementPosition::Pre
    ));

    let Item::Statement(Stmt::For(third_for)) = &program.items[2] else {
        panic!("expected for loop");
    };
    assert!(matches!(
        &third_for.increment,
        Some(ForIncrement::Increment(increment))
            if increment.op == IncrementOp::Decrement
                && increment.position == IncrementPosition::Post
    ));

    assert!(matches!(
        &program.items[3],
        Item::Statement(Stmt::Increment(increment))
            if increment.op == IncrementOp::Increment
                && increment.position == IncrementPosition::Post
    ));
    assert!(matches!(
        &program.items[4],
        Item::Statement(Stmt::Increment(increment))
            if increment.op == IncrementOp::Increment
                && increment.position == IncrementPosition::Pre
    ));
    assert!(matches!(
        &program.items[5],
        Item::Statement(Stmt::Increment(increment))
            if increment.op == IncrementOp::Decrement
                && increment.position == IncrementPosition::Post
    ));
    assert!(matches!(
        &program.items[6],
        Item::Statement(Stmt::Increment(increment))
            if increment.op == IncrementOp::Decrement
                && increment.position == IncrementPosition::Pre
    ));
}

#[test]
fn parses_stage_9_foreach_ranges() {
    let program = doriac::parse_source(
        "test.doria",
        r#"
foreach (0..10 as $i) {
}

foreach (0..<10 as $i) {
}
"#,
    )
    .expect("parse should succeed");

    let Item::Statement(Stmt::Foreach(inclusive)) = &program.items[0] else {
        panic!("expected foreach");
    };
    assert!(matches!(
        inclusive.iterable,
        Expr::Range {
            inclusive: true,
            ..
        }
    ));

    let Item::Statement(Stmt::Foreach(exclusive)) = &program.items[1] else {
        panic!("expected foreach");
    };
    assert!(matches!(
        exclusive.iterable,
        Expr::Range {
            inclusive: false,
            ..
        }
    ));
}

#[test]
fn rejects_value_producing_increment_expressions() {
    for source in ["let $x = $i++;", "let $x = ++$i;"] {
        doriac::parse_source("test.doria", source)
            .expect_err("value-producing increment should not parse in Stage 9");
    }
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

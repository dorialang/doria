use doriac::ast::{Item, Stmt};

fn diagnostics(source: &str) -> Vec<doriac::diagnostics::Diagnostic> {
    doriac::check_source("stage26a.doria", source).expect_err("source should be rejected")
}

fn diagnostic(source: &str, code: &str) -> doriac::diagnostics::Diagnostic {
    diagnostics(source)
        .into_iter()
        .find(|diagnostic| diagnostic.code == code)
        .unwrap_or_else(|| panic!("expected {code}"))
}

#[test]
fn parser_preserves_one_initializer_and_each_binding_span() {
    let source = "function main(): void { let writable $first, $second = 42; }";
    let program = doriac::parse_source("stage26a.doria", source)
        .expect("grouped declaration syntax should parse");
    let Item::Function(function) = &program.items[0] else {
        panic!("expected function");
    };
    let Stmt::VarDecl(declaration) = &function.body.statements[0] else {
        panic!("expected local declaration");
    };
    assert!(declaration.writable);
    assert!(declaration.ty.is_none());
    assert_eq!(declaration.bindings.len(), 2);
    assert_eq!(declaration.bindings[0].name, "first");
    assert_eq!(declaration.bindings[1].name, "second");
    assert_eq!(
        &source[declaration.bindings[0].span.start..declaration.bindings[0].span.end],
        "$first"
    );
    assert_eq!(
        &source[declaration.bindings[1].span.start..declaration.bindings[1].span.end],
        "$second"
    );
}

#[test]
fn all_four_grouped_forms_evaluate_once_and_initialize_independent_locals() {
    let source = r#"
function value(): int { echo "value\n"; return 7; }
function text(): string { echo "text\n"; return "same"; }
function main(): void
{
    let $a, $b = value();
    let writable $c, $d = value();
    int $e, $f = value();
    writable int $g, $h = value();
    let $left, $right = text();
    $c = 8;
    $h = 9;
    echo "{$a}{$b}{$c}{$d}{$e}{$f}{$g}{$h}:{$left}:{$right}\n";
}
"#;
    let mir = doriac::lower_source_to_mir("stage26a.doria", source)
        .expect("grouped Copy declarations should lower to MIR");
    assert!(mir.functions.iter().any(|function| {
        function.blocks.iter().any(|block| {
            block.statements.iter().any(|statement| {
                matches!(
                    statement,
                    doriac::mir::Statement::AssignLocalGroup { targets, .. }
                        if targets.len() == 2
                )
            })
        })
    }));
    let output = doriac::mir_interpreter::interpret(&mir)
        .expect("grouped declarations should execute through the semantic oracle");
    assert_eq!(
        output.stdout,
        b"value\nvalue\nvalue\nvalue\ntext\n77877779:same:same\n"
    );
}

#[test]
fn explicitly_typed_nullable_move_bindings_accept_only_literal_null() {
    doriac::lower_source_to_mir(
        "stage26a-null.doria",
        r#"
class Token { function __construct() {} }
function main(): void
{
    ?Token $left, $right = null;
    ?List<int> $first, $second = null;
    if ($left == null && $right == null && $first == null && $second == null) {
        echo "empty\n";
    }
}
"#,
    )
    .expect("explicit nullable move-type groups initialized with null should lower");

    let untyped = diagnostic(
        "function main(): void { let $left, $right = null; }",
        "E0552",
    );
    assert_eq!(
        untyped.title,
        "Grouped Null Declaration Needs An Explicit Type"
    );

    let non_null = diagnostic(
        r#"
class Token { function __construct() {} }
function main(): void { ?Token $left, $right = new Token(); }
"#,
        "E0551",
    );
    assert_eq!(
        non_null.title,
        "Initializer Cannot Create Multiple Owned Bindings"
    );
}

#[test]
fn every_owned_family_is_rejected_by_the_shared_move_type_rule() {
    for declaration in [
        "let $a, $b = new Token();",
        "let $a, $b = [1, 2];",
        "let $a, $b = Bytes::fromArray([1, 2]);",
        "mixed $a, $b = 1;",
    ] {
        let source = format!(
            r#"
class Token {{ function __construct() {{}} }}
function main(): void {{ {declaration} }}
"#
        );
        let error = diagnostic(&source, "E0551");
        assert_eq!(
            error.title, "Initializer Cannot Create Multiple Owned Bindings",
            "{declaration}"
        );
    }
}

#[test]
fn grouped_names_are_inserted_atomically_and_duplicates_point_at_the_binding() {
    let source =
        "function main(): void { let $existing = 1; let $existing, $later = 2; let $later = 3; }";
    let errors = diagnostics(source);
    let duplicate = errors
        .iter()
        .find(|diagnostic| diagnostic.code == "E0103")
        .expect("existing name should be rejected");
    assert_eq!(
        &source[duplicate.span.start..duplicate.span.end],
        "$existing"
    );
    assert_eq!(
        errors
            .iter()
            .filter(|diagnostic| diagnostic.code == "E0103")
            .count(),
        1,
        "the invalid group must not insert its otherwise-valid later binding"
    );
}

#[test]
fn rejected_per_binding_syntax_has_focused_diagnostics() {
    for (source, code, title) in [
        (
            "function main(): void { let $a, = 1; }",
            "E0556",
            "Grouped Declaration Cannot Have A Trailing Comma",
        ),
        (
            "function main(): void { let $a, writable $b = 1; }",
            "E0554",
            "Grouped Bindings Share One Mutability Mode",
        ),
        (
            "function main(): void { int $a, string $b = 1; }",
            "E0555",
            "Grouped Bindings Share One Declared Type",
        ),
        (
            "function main(): void { let $a, $b = 1, $c = 2; }",
            "E0553",
            "Grouped Declarations Use One Shared Initializer",
        ),
    ] {
        let error = diagnostic(source, code);
        assert_eq!(error.title, title);
    }
}

#[test]
fn traditional_for_initializer_uses_the_same_grouped_local_contract() {
    let mir = doriac::lower_source_to_mir(
        "stage26a-for.doria",
        r#"
function seed(): int { echo "seed\n"; return 0; }
function main(): void
{
    for (let writable $index, $start = seed(); $index < 2; $index++) {
        echo "{$index}:{$start}\n";
    }
}
"#,
    )
    .expect("traditional for should reuse grouped local declarations");
    let output = doriac::mir_interpreter::interpret(&mir)
        .expect("grouped traditional-for initializer should execute");
    assert_eq!(output.stdout, b"seed\n0:0\n1:0\n");
}

use std::process::Command;

use doriac::ast::{ClosureBody, ClosureCaptureMode, ClosureForm, Expr, Item, Stmt};
use doriac::diagnostics::{DiagnosticFormat, DiagnosticKind, RenderOptions};
use doriac::source::Span;
use doriac::types::{FunctionInvocationMode, FunctionTypeParameterMode};

fn span_text(source: &str, span: Span) -> &str {
    &source[span.start..span.end]
}

fn local_initializer(program: &doriac::ast::Program, index: usize) -> &Expr {
    let Item::Statement(Stmt::VarDecl(declaration)) = &program.items[index] else {
        panic!("expected local declaration at item {index}");
    };
    &declaration.initializer
}

fn closure(expr: &Expr) -> &doriac::ast::ClosureExpression {
    let Expr::Closure(closure) = expr else {
        panic!("expected closure expression, got {expr:#?}");
    };
    closure
}

#[test]
fn arrow_closure_ast_preserves_parameters_body_and_exact_spans() {
    let source = "let $double = fn(int $value) => $value * 2;";
    let program = doriac::parse_source("arrow.doria", source).expect("arrow should parse");
    let closure = closure(local_initializer(&program, 0));

    assert_eq!(closure.form, ClosureForm::Arrow);
    assert_eq!(
        span_text(source, closure.span),
        "fn(int $value) => $value * 2"
    );
    assert_eq!(span_text(source, closure.keyword_span), "fn");
    assert_eq!(
        span_text(source, closure.parameter_list_span),
        "(int $value)"
    );
    assert_eq!(closure.parameters.len(), 1);
    assert_eq!(span_text(source, closure.parameters[0].span), "int $value");
    assert_eq!(span_text(source, closure.parameters[0].type_span), "int");
    assert_eq!(span_text(source, closure.parameters[0].name_span), "$value");
    assert!(closure.return_type.is_none());
    assert!(closure.captures.is_none());
    let ClosureBody::Expression {
        arrow_span,
        expression,
    } = &closure.body
    else {
        panic!("expected expression body");
    };
    assert_eq!(span_text(source, *arrow_span), "=>");
    assert_eq!(span_text(source, expression.span()), "$value * 2");
}

#[test]
fn capture_ast_preserves_modes_duplicates_order_and_exact_spans() {
    let source = "let $operation = fn(take Message $input, writable int $count) with ($minimum, writable $total, take $message, $minimum) => process($input);";
    let program = doriac::parse_source("captures.doria", source).expect("captures should parse");
    let closure = closure(local_initializer(&program, 0));

    assert_eq!(closure.parameters.len(), 2);
    assert!(closure.parameters[0].take);
    assert_eq!(
        span_text(source, closure.parameters[0].take_span.unwrap()),
        "take"
    );
    assert!(closure.parameters[1].writable);
    assert_eq!(
        span_text(source, closure.parameters[1].writable_span.unwrap()),
        "writable"
    );

    let captures = closure.captures.as_ref().expect("capture clause");
    assert_eq!(span_text(source, captures.keyword_span), "with");
    assert_eq!(span_text(source, captures.open_span), "(");
    assert_eq!(span_text(source, captures.close_span), ")");
    assert_eq!(
        span_text(source, captures.span),
        "with ($minimum, writable $total, take $message, $minimum)"
    );
    assert_eq!(captures.captures.len(), 4);
    assert_eq!(captures.captures[0].mode, ClosureCaptureMode::Readonly);
    assert_eq!(captures.captures[1].mode, ClosureCaptureMode::Writable);
    assert_eq!(captures.captures[2].mode, ClosureCaptureMode::Take);
    assert_eq!(captures.captures[3].mode, ClosureCaptureMode::Readonly);
    assert_eq!(captures.captures[0].name, "minimum");
    assert_eq!(captures.captures[3].name, "minimum");
    assert_eq!(span_text(source, captures.captures[0].span), "$minimum");
    assert_eq!(
        span_text(source, captures.captures[1].modifier_span.unwrap()),
        "writable"
    );
    assert_eq!(span_text(source, captures.captures[1].name_span), "$total");
    assert_eq!(
        span_text(source, captures.captures[2].modifier_span.unwrap()),
        "take"
    );
    assert_eq!(
        span_text(source, captures.captures[2].span),
        "take $message"
    );
}

#[test]
fn anonymous_block_closure_preserves_return_capture_and_block_spans() {
    let source = "let $positive = function (int $value): bool with ($minimum) { return $value > $minimum; };";
    let program = doriac::parse_source("block.doria", source).expect("block closure should parse");
    let closure = closure(local_initializer(&program, 0));

    assert_eq!(closure.form, ClosureForm::AnonymousBlock);
    assert_eq!(span_text(source, closure.keyword_span), "function");
    let return_type = closure.return_type.as_ref().expect("written return type");
    assert_eq!(span_text(source, return_type.colon_span), ":");
    assert_eq!(span_text(source, return_type.type_span), "bool");
    assert_eq!(span_text(source, return_type.span), ": bool");
    let ClosureBody::Block(block) = &closure.body else {
        panic!("expected block body");
    };
    assert_eq!(
        span_text(source, block.span),
        "{ return $value > $minimum; }"
    );
}

#[test]
fn closures_parse_in_nested_argument_return_and_collection_positions() {
    let source = r#"
let $nested = fn(int $outer) => fn(int $inner) => $outer + $inner;
let $items = [fn(int $value) => $value];
consume(fn(string $label) => $label);

function make(): function(int): int
{
    return fn(int $value) => $value + 1;
}
"#;
    let program = doriac::parse_source("positions.doria", source)
        .expect("closures should parse in every expression position");

    let nested = closure(local_initializer(&program, 0));
    let ClosureBody::Expression { expression, .. } = &nested.body else {
        panic!("expected outer arrow body");
    };
    assert!(matches!(expression.as_ref(), Expr::Closure(_)));
    assert!(matches!(
        local_initializer(&program, 1),
        Expr::Array { elements, .. } if matches!(elements[0].value, Expr::Closure(_))
    ));
    assert!(matches!(
        &program.items[2],
        Item::Statement(Stmt::Expr {
            expr: Expr::FunctionCall { args, .. },
            ..
        }) if matches!(args[0].value, Expr::Closure(_))
    ));
    let Item::Function(make) = &program.items[3] else {
        panic!("expected named function");
    };
    assert_eq!(
        make.return_type.as_ref().unwrap().to_string(),
        "function(int): int"
    );
    assert!(matches!(
        &make.body.statements[0],
        Stmt::Return {
            expr: Some(Expr::Closure(_)),
            ..
        }
    ));
}

#[test]
fn accepted_function_types_preserve_types_and_spans_without_parameter_names() {
    let source = r#"function accept(
    function(int, string): bool $predicate,
    function(): void $done
): function(function(int): string): void
{
}
"#;
    let program = doriac::parse_source("function_types.doria", source)
        .expect("accepted function type spelling should parse");
    let Item::Function(function) = &program.items[0] else {
        panic!("expected function declaration");
    };

    assert_eq!(
        function.params[0].ty.to_string(),
        "function(int, string): bool"
    );
    assert_eq!(function.params[1].ty.to_string(), "function(): void");
    let function_type = function.params[0].ty.function.as_ref().unwrap();
    assert_eq!(span_text(source, function_type.keyword_span), "function");
    assert_eq!(
        span_text(source, function_type.parameter_list_span),
        "(int, string)"
    );
    assert_eq!(span_text(source, function_type.parameters[0].span), "int");
    assert_eq!(span_text(source, function_type.return_type_span), "bool");
    assert_eq!(
        function.return_type.as_ref().unwrap().to_string(),
        "function(function(int): string): void"
    );
}

#[test]
fn stage30a_function_types_preserve_modes_effects_grouping_and_exact_spans() {
    let source = r#"function accept(
    function(int): int $readonlyCallback,
    writable function writable(writable Counter): void $writableCallback,
    take function once(take Payload): Payload $factory,
    function(string): Record throws ParseError, StorageError $parser,
    function((function(): int throws FirstError), string): void $nested
): function(): (function(): int throws InnerError) throws OuterError
{
}
"#;
    let program = doriac::parse_source("stage30a_types.doria", source)
        .expect("Stage 30a function types should parse");
    let Item::Function(function) = &program.items[0] else {
        panic!("expected function declaration");
    };

    let readonly = function.params[0].ty.function.as_ref().unwrap();
    assert_eq!(readonly.invocation_mode, FunctionInvocationMode::Readonly);
    assert!(readonly.invocation_modifier_span.is_none());
    assert_eq!(span_text(source, readonly.keyword_span), "function");
    assert_eq!(span_text(source, readonly.parameter_list_open_span), "(");
    assert_eq!(span_text(source, readonly.parameter_list_close_span), ")");
    assert_eq!(span_text(source, readonly.parameter_list_span), "(int)");
    assert_eq!(span_text(source, readonly.parameters[0].type_span), "int");
    assert_eq!(span_text(source, readonly.colon_span), ":");
    assert_eq!(span_text(source, readonly.return_type_span), "int");

    let writable = function.params[1].ty.function.as_ref().unwrap();
    assert_eq!(writable.invocation_mode, FunctionInvocationMode::Writable);
    assert_eq!(
        span_text(source, writable.invocation_modifier_span.unwrap()),
        "writable"
    );
    assert_eq!(
        writable.parameters[0].ownership_mode,
        FunctionTypeParameterMode::Writable
    );
    assert_eq!(
        span_text(
            source,
            writable.parameters[0].ownership_modifier_span.unwrap()
        ),
        "writable"
    );
    assert_eq!(
        span_text(source, writable.parameters[0].span),
        "writable Counter"
    );

    let once = function.params[2].ty.function.as_ref().unwrap();
    assert_eq!(once.invocation_mode, FunctionInvocationMode::Once);
    assert_eq!(
        once.parameters[0].ownership_mode,
        FunctionTypeParameterMode::Take
    );
    assert_eq!(
        function.params[2].ty.to_string(),
        "function once(take Payload): Payload"
    );

    let parser = function.params[3].ty.function.as_ref().unwrap();
    let effects = parser.throws_clause.as_ref().unwrap();
    assert_eq!(span_text(source, effects.keyword_span), "throws");
    assert_eq!(
        span_text(source, effects.entries[0].type_span),
        "ParseError"
    );
    assert_eq!(
        span_text(source, effects.entries[1].type_span),
        "StorageError"
    );
    assert_eq!(
        span_text(source, effects.span),
        "throws ParseError, StorageError"
    );
    assert_eq!(
        function.params[3].ty.to_string(),
        "function(string): Record throws ParseError, StorageError"
    );

    let nested = function.params[4].ty.function.as_ref().unwrap();
    let grouped = nested.parameters[0].ty.grouped.as_ref().unwrap();
    assert_eq!(span_text(source, grouped.open_span), "(");
    assert_eq!(span_text(source, grouped.close_span), ")");
    assert_eq!(
        span_text(source, grouped.span),
        "(function(): int throws FirstError)"
    );

    let outer = function
        .return_type
        .as_ref()
        .unwrap()
        .function
        .as_ref()
        .unwrap();
    let grouped_return = outer.return_type.grouped.as_ref().unwrap();
    assert!(grouped_return
        .inner
        .function
        .as_ref()
        .unwrap()
        .throws_clause
        .is_some());
    assert_eq!(
        span_text(source, outer.throws_clause.as_ref().unwrap().span),
        "throws OuterError"
    );
    assert_eq!(
        function.return_type.as_ref().unwrap().to_string(),
        "function(): (function(): int throws InnerError) throws OuterError"
    );
}

#[test]
fn stage30a_grouped_types_preserve_authored_parentheses_and_composition() {
    let source = r#"function grouped(
    (int) $integer,
    ((List<string>)) $labels,
    (?Payload) $payload,
    ?(function writable(int): int) $callback,
    ?function(int): int $nullableReadonly,
    ?function once(): Payload $nullableOnce,
    List<function once(): Payload> $factories,
    (function(int): string)[] $arrayCallbacks
): void
{
}
"#;
    let program = doriac::parse_source("grouped_types.doria", source)
        .expect("grouping should compose in existing type positions");
    let Item::Function(function) = &program.items[0] else {
        panic!("expected function declaration");
    };

    assert_eq!(function.params[0].ty.to_string(), "(int)");
    assert_eq!(function.params[1].ty.to_string(), "((List<string>))");
    assert_eq!(function.params[2].ty.to_string(), "(?Payload)");
    assert_eq!(
        function.params[3].ty.to_string(),
        "?(function writable(int): int)"
    );
    assert_eq!(function.params[4].ty.to_string(), "?function(int): int");
    assert_eq!(
        function.params[5].ty.to_string(),
        "?function once(): Payload"
    );
    assert_eq!(
        function.params[6].ty.to_string(),
        "List<function once(): Payload>"
    );
    assert_eq!(
        function.params[7].ty.to_string(),
        "(function(int): string)[]"
    );
}

#[test]
fn grouped_types_preserve_inner_and_outer_nullability_during_semantic_resolution() {
    let source = r#"
class Payload {}

function inspect((?Payload) $inner, ?(Payload) $outer): void
{
}

function main(): void
{
    inspect(null, null);
}
"#;

    doriac::check_source("grouped_nullability.doria", source)
        .expect("grouping must be semantically transparent to nullability");
}

#[test]
fn stage30a_callable_postfix_ast_is_distinct_and_chains_with_exact_spans() {
    let source = r#"function main(): void
{
    $callback(1);
    factory()(2);
    ($factory())(3)[0];
    $callbacks[0](4)->result;
    (fn(int $value) => $value)(5);
    (function (int $value): int { return $value; })(6);
    named(value: 7);
    $object->method(value: 8);
    Type::make(value: 9);
}
"#;
    let program = doriac::parse_source("callable_postfix.doria", source)
        .expect("callable postfix syntax should parse");
    let Item::Function(main) = &program.items[0] else {
        panic!("expected main function");
    };

    let Stmt::Expr {
        expr:
            Expr::CallableCall {
                callee,
                open_span,
                args,
                close_span,
                argument_list_span,
                span,
            },
        ..
    } = &main.body.statements[0]
    else {
        panic!("variable invocation must be a callable call");
    };
    assert_eq!(span_text(source, callee.span()), "$callback");
    assert_eq!(span_text(source, *open_span), "(");
    assert_eq!(span_text(source, args[0].span), "1");
    assert_eq!(span_text(source, *close_span), ")");
    assert_eq!(span_text(source, *argument_list_span), "(1)");
    assert_eq!(span_text(source, *span), "$callback(1)");

    assert!(matches!(
        &main.body.statements[1],
        Stmt::Expr {
            expr: Expr::CallableCall { callee, .. },
            ..
        } if matches!(callee.as_ref(), Expr::FunctionCall { name, .. } if name == "factory")
    ));
    assert!(matches!(
        &main.body.statements[2],
        Stmt::Expr {
            expr: Expr::Index { collection, .. },
            ..
        } if matches!(collection.as_ref(), Expr::CallableCall { .. })
    ));
    assert!(matches!(
        &main.body.statements[3],
        Stmt::Expr {
            expr: Expr::PropertyAccess { object, .. },
            ..
        } if matches!(object.as_ref(), Expr::CallableCall { .. })
    ));
    assert!(matches!(
        &main.body.statements[4],
        Stmt::Expr {
            expr: Expr::CallableCall { callee, .. },
            ..
        } if matches!(callee.as_ref(), Expr::Grouped { expr, .. } if matches!(expr.as_ref(), Expr::Closure(_)))
    ));
    assert!(matches!(
        &main.body.statements[6],
        Stmt::Expr {
            expr: Expr::FunctionCall { name, args, .. },
            ..
        } if name == "named" && args[0].name.as_ref().is_some_and(|name| name.text == "value")
    ));
    assert!(matches!(
        &main.body.statements[7],
        Stmt::Expr {
            expr: Expr::MethodCall { method, args, .. },
            ..
        } if method == "method" && args[0].name.is_some()
    ));
    assert!(matches!(
        &main.body.statements[8],
        Stmt::Expr {
            expr: Expr::StaticCall { method, args, .. },
            ..
        } if method == "make" && args[0].name.is_some()
    ));
}

#[test]
fn stage30a_malformed_forms_have_deliberate_diagnostics_and_recover() {
    let cases = [
        (
            "function take(): Payload",
            "Function Invocation Mode Uses `Once`",
        ),
        (
            "function readonly(int): int",
            "Readonly Function Mode Is Implicit",
        ),
        (
            "function writable once(int): int",
            "Function Invocation Mode Is Duplicated",
        ),
        (
            "function(take take Payload): void",
            "Function Type Parameter Mode Is Duplicated",
        ),
        (
            "function(take writable Payload): void",
            "Function Type Parameter Modes Conflict",
        ),
        (
            "function(readonly Payload): void",
            "Readonly Parameter Mode Is Implicit",
        ),
        (
            "function(writable): void",
            "Function Type Parameter Type Is Missing",
        ),
        ("function(): void throws", "Function Type Effect Is Missing"),
        (
            "function(): void throws FirstError,",
            "Function Type Effect Is Missing",
        ),
        ("(int, string)", "Tuple Type Is Not Supported"),
        ("()", "Type Group Is Empty"),
    ];

    for (ty, title) in cases {
        let source =
            format!("function rejected({ty} $value): void {{}} function later(): void {{}}");
        let diagnostics = doriac::parse_source("malformed_stage30a.doria", source)
            .expect_err("malformed Stage 30a syntax should be rejected");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.title == title),
            "{ty} should report {title:?}, got {diagnostics:#?}"
        );
        assert!(
            diagnostics.len() <= 3,
            "{ty} caused a parser cascade: {diagnostics:#?}"
        );
    }

    for ty in [
        "function(function(): int throws FirstError, string): void",
        "function(): function(): int throws Failure",
    ] {
        let source = format!("function rejected({ty} $value): void {{}}");
        let diagnostics = doriac::parse_source("ambiguous_effects.doria", source)
            .expect_err("ambiguous nested effects require grouping");
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.title == "Nested Function Type Effects Need Grouping"));
    }

    let source = "function main(): void { $callback(value: 42); }";
    let diagnostics = doriac::parse_source("named_callable_arg.doria", source)
        .expect_err("explicit callable-value named arguments are invalid");
    assert_eq!(diagnostics.len(), 1, "unexpected cascade: {diagnostics:#?}");
    assert_eq!(
        diagnostics[0].title,
        "Callable Value Argument Cannot Be Named"
    );
}

#[test]
fn stage30d_lowers_valid_closures_and_callable_invocation() {
    let source = r#"function main(): void
{
    let $callback = fn(int $value) => $value;
    $callback(42);
}
"#;
    doriac::check_source("stage30d.doria", source)
        .expect("valid closures should pass target-neutral checking");
    let hir = doriac::lower_source("stage30d.doria", source)
        .expect("valid closures should lower to closure-aware HIR");
    let hir_dump = format!("{hir:#?}");
    assert!(hir_dump.contains("ClosureExpression"));
    assert!(hir_dump.contains("CallableCall"));

    let mir = doriac::lower_source_to_mir("stage30d.doria", source)
        .expect("valid closures should lower to closure-aware MIR");
    let mir_dump = mir.to_string();
    assert!(mir_dump.contains("function types:"));
    assert!(mir_dump.contains("closure descriptors:"));
    assert!(mir_dump.contains("indirect "));
}

#[test]
fn isolated_callable_invocation_reports_the_undeclared_callee_without_cascades() {
    let source = "function main(): void { $callback(42); }";
    let diagnostics = doriac::check_source("isolated_callable.doria", source)
        .expect_err("an undeclared callable must be rejected precisely");

    assert_eq!(diagnostics.len(), 1, "unexpected cascade: {diagnostics:#?}");
    assert_eq!(diagnostics[0].code, "E0101");
    assert!(diagnostics[0].message.contains("callback"));
}

#[test]
fn stage30a_accepted_fixture_is_parser_only_and_source_preserving() {
    let source = include_str!("fixtures/accepted_syntax/stage30a_callable_grammar.doria");
    let program = doriac::parse_source("stage30a_callable_grammar.doria", source)
        .expect("accepted Stage 30a fixture should parse");
    let ast = format!("{program:#?}");

    assert!(ast.contains("invocation_mode: Once"));
    assert!(ast.contains("FunctionTypeThrowsRef"));
    assert!(ast.contains("GroupedTypeRef"));
    assert!(ast.contains("CallableCall"));
}

#[test]
fn decision_0120_fixture_is_parseable_and_visible_in_ast_output() {
    let source = include_str!("fixtures/accepted_syntax/closures.doria");
    let program = doriac::parse_source("closures.doria", source)
        .expect("accepted closure inventory should have no parser diagnostics");
    let ast = format!("{program:#?}");

    assert!(ast.contains("ClosureExpression"));
    assert!(ast.contains("AnonymousBlock"));
    assert!(ast.contains("Writable"));
    assert!(ast.contains("Take"));
    assert!(ast.contains("FunctionTypeRef"));
}

#[test]
fn semantic_and_ide_paths_are_target_neutral_while_only_php_keeps_its_boundary() {
    let source = include_str!("fixtures/accepted_syntax/closure_boundary.doria");
    doriac::check_source("closure_boundary.doria", source)
        .expect("valid closure syntax should pass target-neutral checking");
    let (_, analysis) = doriac::analyze_source_for_ide("closure_boundary.doria", source)
        .expect("IDE analysis should succeed");
    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
    doriac::lower_source("closure_boundary.doria", source)
        .expect("valid closure syntax should enter HIR lowering");

    doriac::compile_source(
        "closure_boundary.doria",
        source,
        doriac::backend::BackendTarget::Native,
    )
    .expect("native closure execution should be available in Stage 30e");

    let diagnostics = doriac::compile_source(
        "closure_boundary.doria",
        source,
        doriac::backend::BackendTarget::Php,
    )
    .expect_err("PHP closure output remains a Stage 30f boundary");
    assert_eq!(diagnostics.len(), 1, "unexpected cascade: {diagnostics:#?}");
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic.code, "E0641");
    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::UnsupportedDevelopmentSurface
    );
    assert!(diagnostic.development_only);
    assert_eq!(diagnostic.title, "Closure PHP Output Is Not Yet Available");
    assert!(diagnostic.message.contains("Stage 30f"));

    let json = doriac::render_diagnostics_with_options(
        "closure_boundary.doria",
        source,
        &diagnostics,
        RenderOptions {
            format: DiagnosticFormat::Json,
            ..RenderOptions::default()
        },
    );
    let envelope: serde_json::Value = serde_json::from_str(&json).expect("schema-version-1 JSON");
    assert_eq!(envelope["schemaVersion"], 1);
    assert_eq!(envelope["diagnostics"][0]["code"], "E0641");
    assert_eq!(
        envelope["diagnostics"][0]["kind"],
        "unsupportedDevelopmentSurface"
    );
    assert_eq!(envelope["diagnostics"][0]["developmentOnly"], true);
}

#[test]
fn function_type_semantic_use_no_longer_hits_the_execution_boundary() {
    let source = "function accept(function(int): int $callback): void {}";
    doriac::check_source("function_type_boundary.doria", source)
        .expect("type-only structural function syntax has semantic identity in Stage 30b");
}

#[test]
fn closure_signatures_report_permanent_type_errors_before_the_stage_30_boundary() {
    let cases = [
        (
            "arrow_void_parameter",
            "let $closure = fn(void $value) => $value;",
            "E0430",
            "void",
        ),
        (
            "block_object_parameter",
            "let $closure = function (object $value): int { return 1; };",
            "E0401",
            "object",
        ),
        (
            "block_null_return",
            "let $closure = function (): null { return null; };",
            "E0431",
            "null",
        ),
    ];

    for (name, source, type_code, rejected_type) in cases {
        let diagnostics = doriac::check_source(name, source)
            .expect_err("invalid closure signatures must be reported precisely");
        assert_eq!(diagnostics.len(), 1, "unexpected cascade: {diagnostics:#?}");
        assert!(diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "E0641"));
        let type_diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == type_code)
            .expect("permanent type-position diagnostic");
        assert_eq!(span_text(source, type_diagnostic.span), rejected_type);
    }
}

#[test]
fn function_types_recursively_validate_components_without_boundary_cascades() {
    let cases = [
        (
            "function_type_void_parameter",
            "function accept(function(void): int $callback): void {}",
            "E0430",
            "void",
        ),
        (
            "function_type_object_return",
            "function accept(function(int): object $callback): void {}",
            "E0401",
            "object",
        ),
        (
            "nested_function_type_null_parameter",
            "function accept(function(function(null): int): int $callback): void {}",
            "E0431",
            "null",
        ),
    ];

    for (name, source, type_code, rejected_type) in cases {
        let diagnostics = doriac::check_source(name, source)
            .expect_err("invalid callable components must be reported precisely");
        assert_eq!(diagnostics.len(), 1, "unexpected cascade: {diagnostics:#?}");
        assert!(diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "E0641"));
        let type_diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == type_code)
            .expect("component type-position diagnostic");
        assert_eq!(span_text(source, type_diagnostic.span), rejected_type);
    }
}

#[test]
fn malformed_closure_inventory_has_deliberate_diagnostics() {
    let cases = [
        (
            "untyped_arrow_parameter",
            include_str!("fixtures/negative_syntax/closures/untyped_arrow_parameter.doria"),
            "requires a written type",
        ),
        (
            "untyped_block_parameter",
            include_str!("fixtures/negative_syntax/closures/untyped_block_parameter.doria"),
            "requires a written type",
        ),
        (
            "missing_block_return_type",
            include_str!("fixtures/negative_syntax/closures/missing_block_return_type.doria"),
            "requires a written return type",
        ),
        (
            "php_use_capture",
            include_str!("fixtures/negative_syntax/closures/php_use_capture.doria"),
            "use `with`, not PHP closure `use`",
        ),
        (
            "reference_capture",
            include_str!("fixtures/negative_syntax/closures/reference_capture.doria"),
            "do not use PHP reference `&` syntax",
        ),
        (
            "writable_reference_capture",
            include_str!("fixtures/negative_syntax/closures/writable_reference_capture.doria"),
            "do not use PHP reference `&` syntax",
        ),
        (
            "readonly_capture_modifier",
            include_str!("fixtures/negative_syntax/closures/readonly_capture_modifier.doria"),
            "written as a bare variable",
        ),
        (
            "empty_capture_list",
            include_str!("fixtures/negative_syntax/closures/empty_capture_list.doria"),
            "omits the `with` clause",
        ),
        (
            "missing_capture_variable",
            include_str!("fixtures/negative_syntax/closures/missing_capture_variable.doria"),
            "expected captured variable",
        ),
        (
            "missing_capture_comma",
            include_str!("fixtures/negative_syntax/closures/missing_capture_comma.doria"),
            "expected `,` between closure captures",
        ),
        (
            "missing_capture_close",
            include_str!("fixtures/negative_syntax/closures/missing_capture_close.doria"),
            "expected `)` after closure captures",
        ),
        (
            "missing_arrow",
            include_str!("fixtures/negative_syntax/closures/missing_arrow.doria"),
            "expected `=>` before arrow closure body",
        ),
        (
            "missing_arrow_body",
            include_str!("fixtures/negative_syntax/closures/missing_arrow_body.doria"),
            "expected expression",
        ),
        (
            "missing_block",
            include_str!("fixtures/negative_syntax/closures/missing_block.doria"),
            "use a block body",
        ),
        (
            "wrong_capture_position",
            include_str!("fixtures/negative_syntax/closures/wrong_capture_position.doria"),
            "must appear after its parameter list",
        ),
        (
            "multiple_capture_modifiers",
            include_str!("fixtures/negative_syntax/closures/multiple_capture_modifiers.doria"),
            "exactly one ownership mode",
        ),
        (
            "unterminated_nested_closure",
            include_str!("fixtures/negative_syntax/closures/unterminated_nested_closure.doria"),
            "expected `}` after block",
        ),
        (
            "function_type_named_parameter",
            include_str!("fixtures/negative_syntax/closures/function_type_named_parameter.doria"),
            "contain types, not parameter names",
        ),
        (
            "function_type_missing_return",
            include_str!("fixtures/negative_syntax/closures/function_type_missing_return.doria"),
            "expected `:` before function-type return type",
        ),
        (
            "function_type_missing_close",
            include_str!("fixtures/negative_syntax/closures/function_type_missing_close.doria"),
            "expected `)` after function-type parameters",
        ),
    ];

    for (name, source, expected) in cases {
        let diagnostics = doriac::parse_source(format!("{name}.doria"), source)
            .expect_err("malformed closure syntax should be rejected");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains(expected)),
            "{name} should mention {expected:?}, got {diagnostics:#?}"
        );
    }
}

#[test]
fn closure_recovery_does_not_cascade_into_following_syntax() {
    for (name, source) in [
        (
            "empty_capture_list",
            include_str!("fixtures/negative_syntax/closures/empty_capture_list.doria"),
        ),
        (
            "missing_block_return_type",
            include_str!("fixtures/negative_syntax/closures/missing_block_return_type.doria"),
        ),
    ] {
        let diagnostics = doriac::parse_source(format!("{name}.doria"), source)
            .expect_err("malformed closure should be rejected");
        assert_eq!(
            diagnostics.len(),
            1,
            "{name} recovery cascaded: {diagnostics:#?}"
        );
    }
}

#[test]
fn declarations_fat_arrows_generics_and_checked_errors_still_parse() {
    doriac::parse_source(
        "regressions.doria",
        r#"
class Worker
{
    function run(Dictionary<string, int> $values): void throws Error
    {
        let $mapped = ["answer" => 42];
        let $name = match (true) {
            true => "ready",
            default => "waiting"
        };
    }
}

function namedFunction(int $value): bool
{
    return $value > 0;
}
"#,
    )
    .expect("existing declarations, arrows, generics, and checked errors should still parse");
}

#[test]
fn cli_ast_check_and_hir_accept_valid_closures() {
    let fixture = format!(
        "{}/tests/fixtures/accepted_syntax/closure_boundary.doria",
        env!("CARGO_MANIFEST_DIR")
    );
    let ast = Command::new(env!("CARGO_BIN_EXE_doriac"))
        .args(["ast", &fixture])
        .output()
        .expect("doriac ast should run");
    assert!(
        ast.status.success(),
        "{}",
        String::from_utf8_lossy(&ast.stderr)
    );
    assert!(String::from_utf8_lossy(&ast.stdout).contains("ClosureExpression"));

    let check = Command::new(env!("CARGO_BIN_EXE_doriac"))
        .args(["check", &fixture, "--diagnostic-format", "json"])
        .output()
        .expect("doriac check should run");
    assert!(check.status.success());
    assert!(check.stderr.is_empty());
    let envelope: serde_json::Value =
        serde_json::from_slice(&check.stdout).expect("check JSON should be valid");
    assert_eq!(envelope["schemaVersion"], 1);
    assert!(envelope["diagnostics"].as_array().unwrap().is_empty());

    let hir = Command::new(env!("CARGO_BIN_EXE_doriac"))
        .args(["hir", &fixture])
        .output()
        .expect("doriac hir should run");
    assert!(
        hir.status.success(),
        "{}",
        String::from_utf8_lossy(&hir.stderr)
    );
    assert!(String::from_utf8_lossy(&hir.stdout).contains("ClosureExpression"));
}

use doriac::ast::Item;
use doriac::diagnostics::DiagnosticSeverity;
use doriac::semantics::SemanticAnalysis;
use doriac::symbols::{BindingKind, BindingOwnership, BorrowSource, ReturnBorrow};

const DECLARATIONS: &str = r#"
class Child {
    int $value = 42;
    function selfRef(): self { return $this; }
    function make(): function(): int { return fn() with ($this) => $this->value; }
}
class Holder {
    Child $child = new Child();
    function childRef(): Child { return $this->child; }
}
function childFrom(bool $ignored, Holder $owner): Child { return $owner->childRef(); }
"#;

fn analyze_capture(
    parameters: &str,
    result: &str,
    body: &str,
) -> (String, SemanticAnalysis, Option<ReturnBorrow>) {
    let source = format!("{DECLARATIONS}\nfunction capture({parameters}): {result} {{\n{body}\n}}");
    let (program, analysis) =
        doriac::analyze_source_for_ide("return-borrow-alias.doria", source.clone())
            .unwrap_or_else(|diagnostics| panic!("{source}\n{diagnostics:#?}"));
    let capture = program
        .items
        .iter()
        .find_map(|item| match item {
            Item::Function(function) if function.name == "capture" => Some(function),
            _ => None,
        })
        .expect("capture declaration");
    let borrow = analysis.info.return_borrows.get(&capture.span).copied();
    (source, analysis, borrow)
}

fn assert_parameter_borrow(parameters: &str, body: &str) {
    let (source, analysis, borrow) = analyze_capture(parameters, "function(): int", body);
    assert!(
        analysis
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity != DiagnosticSeverity::Error),
        "{source}\n{:#?}",
        analysis.diagnostics
    );
    assert_eq!(
        borrow,
        Some(ReturnBorrow {
            source: BorrowSource::Parameter(0),
            writable: false,
            kind: doriac::types::ReturnBorrowKind::Retained,
        }),
        "{source}"
    );
}

fn assert_error(analysis: &SemanticAnalysis, code: &str, source: &str) {
    assert!(
        analysis.diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == DiagnosticSeverity::Error && diagnostic.code == code
        }),
        "expected {code}: {source}\n{:#?}",
        analysis.diagnostics
    );
}

fn assert_binding_ownership(
    analysis: &SemanticAnalysis,
    name: &str,
    kind: BindingKind,
    ownership: BindingOwnership,
) {
    let declarations: Vec<_> = analysis
        .info
        .binding_resolution
        .declarations_by_id
        .values()
        .filter(|declaration| declaration.name == name && declaration.kind == kind)
        .collect();
    assert_eq!(declarations.len(), 1, "expected one {kind:?} named {name}");
    assert_eq!(declarations[0].ownership, ownership, "binding {name}");
}

#[test]
fn canonical_binding_ownership_distinguishes_callback_loans_and_retained_owners() {
    let source = r#"
class Source {
    int $value = 42;
    function __construct(take function(): int $stored) {}
    function(): int $callback { borrowed get => $this->stored; }
    function(): int $fresh { get => fn() with ($this) => $this->value; }
}
function inspect(Source $source, bool $choose): int {
    let $direct = $source->callback;
    let $grouped = (($source->callback));
    let $alias = $direct;
    let $fresh = $source->fresh;
    let $freshAlias = $fresh;
    let $ownedWhen = when ($choose) {
        return $source->fresh;
    } else {
        return $source->fresh;
    };
    return $direct() + $grouped() + $alias() + $freshAlias() + $ownedWhen();
}
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("callback-binding-ownership.doria", source)
        .unwrap_or_else(|diagnostics| panic!("{source}\n{diagnostics:#?}"));
    // A borrowed existing carrier and a new owned carrier have different
    // cleanup obligations, even when both retain the same receiver.
    assert!(
        analysis
            .diagnostics
            .iter()
            .all(|diagnostic| { diagnostic.severity != DiagnosticSeverity::Error }),
        "{source}\n{:#?}",
        analysis.diagnostics
    );
    for name in ["direct", "grouped", "alias"] {
        assert_binding_ownership(
            &analysis,
            name,
            BindingKind::Local,
            BindingOwnership::ReadonlyBorrow,
        );
    }
    for name in ["fresh", "freshAlias", "ownedWhen"] {
        assert_binding_ownership(&analysis, name, BindingKind::Local, BindingOwnership::Owned);
    }
    assert_binding_ownership(
        &analysis,
        "source",
        BindingKind::FunctionParameter,
        BindingOwnership::ReadonlyBorrow,
    );
    assert_binding_ownership(
        &analysis,
        "this",
        BindingKind::ClosureCapture,
        BindingOwnership::ReadonlyBorrow,
    );
}

#[test]
fn canonical_binding_ownership_preserves_ordinary_move_loans_and_grouped_copy_values() {
    let source = format!(
        r#"{DECLARATIONS}
function inspect(Holder $holder): int {{
    let $child = $holder->childRef();
    let $grouped = (($holder->childRef()));
    let $alias = $child;
    let $forwarded = $alias->selfRef();
    let $owned = new Child();
    let $left, $right = 1;
    return $child->value + $grouped->value + $alias->value + $forwarded->value
        + $owned->value + $left + $right;
}}
"#
    );
    let (_, analysis) =
        doriac::analyze_source_for_ide("move-binding-ownership.doria", source.clone())
            .unwrap_or_else(|diagnostics| panic!("{source}\n{diagnostics:#?}"));
    assert!(
        analysis
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity != DiagnosticSeverity::Error),
        "{source}\n{:#?}",
        analysis.diagnostics
    );
    for name in ["child", "grouped", "alias", "forwarded"] {
        assert_binding_ownership(
            &analysis,
            name,
            BindingKind::Local,
            BindingOwnership::ReadonlyBorrow,
        );
    }
    assert_binding_ownership(
        &analysis,
        "owned",
        BindingKind::Local,
        BindingOwnership::Owned,
    );
    for name in ["left", "right"] {
        assert_binding_ownership(
            &analysis,
            name,
            BindingKind::GroupedLocal,
            BindingOwnership::Owned,
        );
    }
    assert_binding_ownership(
        &analysis,
        "holder",
        BindingKind::FunctionParameter,
        BindingOwnership::ReadonlyBorrow,
    );
}

#[test]
fn borrowed_method_result_alias_preserves_the_parameter_source() {
    assert_parameter_borrow(
        "Holder $holder",
        "let $view = $holder->childRef(); return $view->make();",
    );
}

#[test]
fn borrowed_result_call_chains_preserve_the_original_source() {
    assert_parameter_borrow(
        "Holder $holder",
        r#"
let $view = ($holder->childRef());
let $next = $view->selfRef();
let $last = ($next->selfRef());
return $last->make();
"#,
    );
}

#[test]
fn named_arguments_follow_the_callees_parameter_identity() {
    for acquisition in [
        "childFrom(ignored: true, owner: $holder)",
        "childFrom(owner: $holder, ignored: true)",
    ] {
        assert_parameter_borrow(
            "Holder $holder",
            &format!("let $view = {acquisition}; return $view->make();"),
        );
    }
}

#[test]
fn nested_shadow_does_not_replace_the_outer_alias_source() {
    assert_parameter_borrow(
        "Holder $holder",
        r#"
let $view = $holder->childRef();
{
    let $view = new Child();
    let $observed = $view->value;
}
return $view->make();
"#,
    );
}

#[test]
fn alias_initializer_resolves_the_outer_binding_before_shadowing() {
    assert_parameter_borrow(
        "Holder $holder",
        r#"
{
    let $holder = $holder->childRef();
    return $holder->make();
}
"#,
    );
}

#[test]
fn branch_local_aliases_join_at_the_same_parameter_source() {
    assert_parameter_borrow(
        "Holder $holder, bool $choose",
        r#"
if ($choose) {
    let $view = $holder->childRef();
    return $view->make();
} else {
    let $view = $holder->childRef();
    let $next = $view->selfRef();
    return $next->make();
}
"#,
    );
}

#[test]
fn returning_branch_does_not_leak_its_shadow_into_the_continuation() {
    assert_parameter_borrow(
        "Holder $holder, bool $choose",
        r#"
let $view = $holder->childRef();
if ($choose) {
    let $view = $holder->childRef();
    return $view->make();
}
return $view->make();
"#,
    );
}

#[test]
fn loop_backedges_and_exits_preserve_outer_and_iteration_aliases() {
    assert_parameter_borrow(
        "Holder $holder, bool $repeat, bool $stop",
        r#"
let $view = $holder->childRef();
while ($repeat) {
    let $iteration = $view->selfRef();
    if ($stop) { return $iteration->make(); }
    continue;
}
return $view->make();
"#,
    );
    assert_parameter_borrow(
        "Holder $holder, bool $repeat, bool $stop",
        r#"
let $view = $holder->childRef();
do {
    let $view = $holder->childRef();
    if ($stop) { break; }
    let $observed = $view->value;
} while ($repeat);
return $view->make();
"#,
    );
}

#[test]
fn aliases_of_owned_sources_do_not_acquire_parameter_lifetimes() {
    for (parameters, body) in [
        (
            "",
            "let $holder = new Holder(); let $view = $holder->childRef(); return $view->make();",
        ),
        (
            "take Holder $holder",
            "let $view = $holder->childRef(); return $view->make();",
        ),
        (
            "Holder $holder",
            "{ let $holder = new Holder(); let $view = $holder->childRef(); return $view->make(); }",
        ),
    ] {
        let (source, analysis, borrow) = analyze_capture(parameters, "function(): int", body);
        assert_error(&analysis, "E0658", &source);
        assert_eq!(borrow, None, "{source}");
    }
}

#[test]
fn different_borrowed_parameter_sources_remain_ambiguous() {
    let (source, analysis, borrow) = analyze_capture(
        "Holder $left, Holder $right, bool $choose",
        "Child",
        r#"
if ($choose) {
    let $view = $left->childRef();
    return $view;
}
let $view = $right->childRef();
return $view;
"#,
    );
    assert_error(&analysis, "E0474", &source);
    assert_eq!(borrow, None, "{source}");
}

#[test]
fn owned_and_borrowed_branch_results_do_not_share_an_inferred_lifetime() {
    let (source, analysis, borrow) = analyze_capture(
        "Holder $holder, bool $choose",
        "function(): int",
        r#"
if ($choose) {
    let $view = $holder->childRef();
    return $view->make();
}
let $owner = new Holder();
let $view = $owner->childRef();
return $view->make();
"#,
    );
    assert_error(&analysis, "E0658", &source);
    assert_eq!(borrow, None, "{source}");
}

#[test]
fn borrowed_aliases_do_not_enable_writable_rebinding() {
    let (source, analysis, _) = analyze_capture(
        "Holder $holder",
        "function(): int",
        r#"
let writable $view = $holder->childRef();
$view = new Child();
return $view->make();
"#,
    );
    assert_error(&analysis, "E0478", &source);
}

#[test]
fn direct_owned_property_move_out_remains_rejected() {
    let (source, analysis, _) = analyze_capture(
        "Holder $holder",
        "function(): int",
        "let $view = $holder->child; return $view->make();",
    );
    assert_error(&analysis, "E0472", &source);
}

#[test]
fn anonymous_function_argument_results_preserve_the_original_parameter_source() {
    assert_parameter_borrow(
        "Holder $holder",
        r#"
let $select = function (bool $ignored, Holder $owner): Child {
    let $child = $owner->childRef();
    return $child->selfRef();
};
let $view = $select(true, $holder);
return $view->make();
"#,
    );
}

#[test]
fn arrow_function_argument_results_preserve_the_original_parameter_source() {
    assert_parameter_borrow(
        "Holder $holder",
        r#"
let $select = fn(bool $ignored, Holder $owner) => $owner->childRef();
let $view = $select(true, $holder);
return $view->make();
"#,
    );
}

#[test]
fn indirect_borrowed_results_do_not_extend_temporary_owner_lifetimes() {
    for selector in [
        "function (bool $ignored, Holder $owner): Child { return $owner->childRef(); }",
        "fn(bool $ignored, Holder $owner) => $owner->childRef()",
    ] {
        for (body, code) in [
            ("return $select(true, new Holder())->make();", "E0658"),
            (
                "let $view = $select(true, new Holder()); return $view->make();",
                "E0478",
            ),
        ] {
            let (source, analysis, borrow) = analyze_capture(
                "",
                "function(): int",
                &format!("let $select = {selector};\n{body}"),
            );
            assert_error(&analysis, code, &source);
            assert_eq!(borrow, None, "{source}");
        }
    }
}

fn analyze_nullable_source(body: &str) -> (String, SemanticAnalysis) {
    let source = format!(
        r#"
class Owner {{ int $value = 42; }}
function identity(?Owner $owner): ?Owner {{ return $owner; }}
function captureReadonly(?Owner $owner): function(): bool {{
    return fn() with ($owner) => $owner == null;
}}
function forwardReadonly(?Owner $owner): function(): bool {{ return captureReadonly($owner); }}
function captureWritable(writable ?Owner $owner): function writable(): bool {{
    return function (): bool with (writable $owner) {{ return $owner == null; }};
}}
function result(): function(): bool {{ {body} }}
"#
    );
    let (_, analysis) = doriac::analyze_source_for_ide("nullable-source.doria", source.clone())
        .unwrap_or_else(|diagnostics| panic!("{source}\n{diagnostics:#?}"));
    (source, analysis)
}

#[test]
fn immutable_null_sources_do_not_create_readonly_lifetime_obligations() {
    for body in [
        "return captureReadonly(null);",
        "let $callback = forwardReadonly((null)); return $callback;",
        "let $absent = identity(null); return captureReadonly($absent);",
        "let $absent = identity(null); let $next = $absent; return captureReadonly($next);",
        "let $absent = identity(null); return fn() with ($absent) => $absent == null;",
        "let $identity = fn(?Owner $owner) => $owner; let $absent = $identity(null); return captureReadonly($absent);",
    ] {
        let (source, analysis) = analyze_nullable_source(body);
        assert!(
            analysis
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.severity != DiagnosticSeverity::Error),
            "{source}\n{:#?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn readonly_absence_does_not_relax_temporary_or_writable_source_requirements() {
    for (body, code) in [
        ("return captureReadonly(new Owner());", "E0658"),
        (
            "let $view = identity(new Owner()); return captureReadonly($view);",
            "E0478",
        ),
        (
            "let writable $callback = captureWritable(null); return fn() => true;",
            "E0204",
        ),
        (
            "writable ?Owner $owner = null; return captureReadonly($owner);",
            "E0658",
        ),
        (
            "?Owner $owner = null; return captureReadonly($owner);",
            "E0658",
        ),
        (
            "let $identity = fn(?Owner $owner) => $owner; let $view = $identity(new Owner()); return captureReadonly($view);",
            "E0478",
        ),
    ] {
        let (source, analysis) = analyze_nullable_source(body);
        assert_error(&analysis, code, &source);
    }
}

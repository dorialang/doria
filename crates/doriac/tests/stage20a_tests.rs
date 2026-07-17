use doriac::const_eval::{ConstValue, ParameterDefaultKey};
use doriac::numeric::IntegerType;

const COPY_SCALAR_DEFAULTS: &str =
    include_str!("../../../examples/native/main_stage20a_copy_scalar_defaults.doria");

fn diagnostics(source: &str) -> Vec<doriac::diagnostics::Diagnostic> {
    doriac::check_source("stage20a.doria", source).expect_err("source should be rejected")
}

fn default_diagnostic(source: &str) -> doriac::diagnostics::Diagnostic {
    diagnostics(source)
        .into_iter()
        .find(|diagnostic| diagnostic.code == "E0498")
        .expect("expected Stage 20a default-argument diagnostic")
}

#[test]
fn copy_scalar_defaults_are_folded_by_callable_and_parameter_identity() {
    let hir = doriac::lower_source("copy-scalar-defaults.doria", COPY_SCALAR_DEFAULTS)
        .expect("Copy-scalar defaults should pass semantic analysis");
    let selected = hir
        .items
        .iter()
        .find_map(|item| match item {
            doriac::hir::Item::Function(function) if function.name == "selected" => Some(function),
            _ => None,
        })
        .expect("selected function");
    assert!(matches!(
        hir.semantic_info
            .parameter_defaults
            .get(&ParameterDefaultKey {
                function_start: selected.span.start,
                parameter_index: 0,
            }),
        Some(ConstValue::Integer(value)) if value.signed_value() == 3
    ));
    assert!(hir
        .semantic_info
        .parameter_defaults
        .values()
        .any(|value| matches!(value, ConstValue::Integer(value) if value.ty == IntegerType::Int64 && value.signed_value() == 4)));
}

#[test]
fn writable_copy_scalar_defaults_are_supported() {
    doriac::check_source(
        "writable-copy-default.doria",
        r#"
function choose(writable int $value = 1): int
{
    return $value;
}

function main(): int
{
    return choose();
}
"#,
    )
    .expect("writable Copy-scalar defaults should be accepted");
}

#[test]
fn deferred_default_categories_have_stable_semantic_diagnostics() {
    let writable_string = default_diagnostic(
        r#"
function label(writable string $value = "guest"): string { return $value; }
"#,
    );
    assert_eq!(
        writable_string.message,
        "default values for `writable string` parameters are not yet supported"
    );

    let take_string = default_diagnostic(
        r#"
function label(take string $value = "guest"): string { return $value; }
"#,
    );
    assert_eq!(
        take_string.message,
        "default values for `take string` parameters are not yet supported"
    );

    let move_type = default_diagnostic(
        r#"
class User {}
function identity(User $value = new User()): User { return $value; }
"#,
    );
    assert_eq!(
        move_type.message,
        "default values for move-type or `take` parameters are not yet supported"
    );

    let take = default_diagnostic(
        r#"
class User {}
function identity(take User $value = new User()): User { return $value; }
"#,
    );
    assert_eq!(
        take.message,
        "default values for move-type or `take` parameters are not yet supported"
    );

    let runtime = default_diagnostic(
        r#"
function runtimeValue(): int { return 1; }
function choose(int $value = runtimeValue()): int { return $value; }
"#,
    );
    assert_eq!(
        runtime.message,
        "a default value must be a constant expression"
    );
}

#[test]
fn every_call_kind_splices_defaults_before_mir_execution() {
    let hir = doriac::lower_source("copy-scalar-defaults.doria", COPY_SCALAR_DEFAULTS)
        .expect("Copy-scalar defaults should pass semantic analysis");
    let mir = doriac::mir_lowering::lower_program(&hir)
        .expect("all omitted Copy-scalar arguments should lower");
    let output = doriac::mir_interpreter::interpret(&mir).expect("lowered defaults should execute");
    assert_eq!(output.stdout, b"7:3:6:8:1:0:5:4:true:1.5\n");
    assert!(output.stderr.is_empty());
    assert_eq!(output.exit_status, 0);
}

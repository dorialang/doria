use doriac::const_eval::{ConstValue, ParameterDefaultKey};

const CONST_STRING_DEFAULTS: &str =
    include_str!("../../../examples/native/main_stage20b_const_string_defaults.doria");

fn default_diagnostic(source: &str) -> doriac::diagnostics::Diagnostic {
    doriac::check_source("stage20b.doria", source)
        .expect_err("source should be rejected")
        .into_iter()
        .find(|diagnostic| diagnostic.code == "E0498")
        .expect("expected a default-argument diagnostic")
}

#[test]
fn readonly_string_defaults_are_folded_by_callable_and_parameter_identity() {
    let hir = doriac::lower_source("const-string-defaults.doria", CONST_STRING_DEFAULTS)
        .expect("const string defaults should pass semantic analysis");
    let greeting = hir
        .items
        .iter()
        .find_map(|item| match item {
            doriac::hir::Item::Function(function) if function.name == "greeting" => Some(function),
            _ => None,
        })
        .expect("greeting function");
    let constructor = hir
        .items
        .iter()
        .find_map(|item| match item {
            doriac::hir::Item::Class(class) if class.name == "Greeter" => {
                class.members.iter().find_map(|member| match member {
                    doriac::hir::ClassMember::Method(method) if method.name == "__construct" => {
                        Some(method)
                    }
                    _ => None,
                })
            }
            _ => None,
        })
        .expect("Greeter constructor");

    assert!(matches!(
        hir.semantic_info
            .parameter_defaults
            .get(&ParameterDefaultKey {
                function_start: greeting.span.start,
                parameter_index: 0,
            }),
        Some(ConstValue::String(value)) if value == "hi"
    ));
    assert!(matches!(
        hir.semantic_info
            .parameter_defaults
            .get(&ParameterDefaultKey {
                function_start: constructor.span.start,
                parameter_index: 0,
            }),
        Some(ConstValue::String(value)) if value == "hi"
    ));
    assert!(hir
        .semantic_info
        .parameter_defaults
        .values()
        .any(|value| matches!(value, ConstValue::String(value) if value == "instance")));
}

#[test]
fn every_call_kind_materializes_omitted_string_defaults_before_mir_execution() {
    let hir = doriac::lower_source("const-string-defaults.doria", CONST_STRING_DEFAULTS)
        .expect("const string defaults should pass semantic analysis");
    let mir = doriac::mir_lowering::lower_program(&hir)
        .expect("all omitted readonly string arguments should lower");
    let output = doriac::mir_interpreter::interpret(&mir)
        .expect("materialized readonly string defaults should execute");

    assert_eq!(
        output.stdout,
        b"hi:hello:hi:yo:instance:custom:hi:static:2:items:3:items:3:things:units:2\ndrop:hello\ndrop:hi\n"
    );
    assert!(output.stderr.is_empty());
    assert_eq!(output.exit_status, 0);
}

#[test]
fn deferred_string_ownership_modes_have_distinct_diagnostics() {
    let nullable = default_diagnostic(
        r#"
function label(?string $value = null): ?string { return $value; }
"#,
    );
    assert_eq!(
        nullable.message,
        "default values for nullable string parameters are not yet supported"
    );

    let writable = default_diagnostic(
        r#"
function label(writable string $value = "guest"): string { return $value; }
"#,
    );
    assert_eq!(
        writable.message,
        "default values for `writable string` parameters are not yet supported"
    );

    let take = default_diagnostic(
        r#"
function label(take string $value = "guest"): string { return $value; }
"#,
    );
    assert_eq!(
        take.message,
        "default values for `take string` parameters are not yet supported"
    );
}

#[test]
fn runtime_string_defaults_remain_constant_expression_errors() {
    let diagnostic = default_diagnostic(
        r#"
function runtimeLabel(): string { return "runtime"; }
function label(string $value = runtimeLabel()): string { return $value; }
"#,
    );
    assert_eq!(
        diagnostic.message,
        "a default value must be a constant expression"
    );
}

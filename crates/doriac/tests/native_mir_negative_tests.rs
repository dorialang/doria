use doriac::backend::{Backend, BackendOutput, BackendTarget, NativeBackend};
use doriac::diagnostics::Diagnostic;

#[test]
fn native_compilation_does_not_execute_an_infinite_program() {
    let source = include_str!("../../../examples/compile-only/main_infinite_while.doria");
    let hir = doriac::lower_source("main_infinite_while.doria", source)
        .expect("infinite loop should pass frontend checking");
    let output = NativeBackend
        .emit(&hir)
        .expect("native compilation should not execute user code as a preflight");
    let BackendOutput::Executable { bytes, .. } = output else {
        panic!("native backend should emit an executable");
    };
    assert!(!bytes.is_empty());
}

#[test]
fn native_and_debug_share_remaining_mir_coverage_diagnostics() {
    let source = r#"function main(): void
{
    foreach (0..<2 as int $key => int $item) {
    }
}
"#;
    let native = compile_error(source, BackendTarget::Native);
    let debug = compile_error(source, BackendTarget::Debug);

    assert_eq!(native[0].code, "E0425");
    assert_eq!(debug[0].code, native[0].code);
    assert_eq!(debug[0].message, native[0].message);
    assert!(native[0].message.contains("key bindings"));
}

fn compile_error(source: &str, target: BackendTarget) -> Vec<Diagnostic> {
    doriac::compile_source("test.doria", source, target)
        .expect_err("source should be outside current MIR coverage")
}

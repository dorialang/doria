fn diagnostics(source: &str) -> Vec<doriac::diagnostics::Diagnostic> {
    doriac::check_source("stage23c.doria", source).expect_err("source should be rejected")
}

#[test]
fn sequence_fills_are_contextual_and_execute_in_source_order() {
    let source = include_str!("../../../examples/native/main_stage23c_sequence_fill.doria");
    let mir = doriac::lower_source_to_mir("stage23c-fill.doria", source)
        .expect("sequence fills should lower through shared MIR");
    let output = doriac::mir_interpreter::interpret(&mir)
        .expect("sequence fills should execute through shared MIR");
    assert_eq!(
        output.stdout,
        b"count\nvalue\ncount\ncount\n3:true\n2:42:42\n2:false\n3:7\n2:9\n3:repeat:repeat\n3:true:false\n"
    );
    assert!(mir.functions.iter().any(|function| {
        function.blocks.iter().any(|block| {
            block.statements.iter().any(|statement| {
                matches!(
                    statement,
                    doriac::mir::Statement::AssignLocal {
                        value: doriac::mir::Rvalue::Collection(
                            doriac::mir::CollectionExpression::Fill { .. }
                        ),
                        ..
                    }
                )
            })
        })
    }));
}

#[test]
fn negative_constant_fill_count_is_rejected() {
    let errors = diagnostics(
        r#"
function main(): void
{
    bool[] $flags = [true; -1];
}
"#,
    );
    assert!(errors.iter().any(|diagnostic| {
        diagnostic.code == "E0527"
            && diagnostic
                .message
                .contains("cannot be negative at compile time")
    }));
}

#[test]
fn move_elements_name_the_cloneable_gate() {
    let errors = diagnostics(
        r#"
class Token { function __construct() {} }
function main(): void
{
    List<Token> $tokens = [new Token(); 2];
}
"#,
    );
    assert!(errors.iter().any(|diagnostic| {
        diagnostic.code == "E0528"
            && diagnostic.message.contains("Stage 23c")
            && diagnostic.message.contains("decision 0102")
            && diagnostic.message.contains("Cloneable")
            && diagnostic.message.contains("Stage 35")
    }));
}

#[test]
fn repeat_form_rejects_keyed_and_unique_collection_families() {
    for (declaration, family) in [
        ("Set<int> $values = [1; 2];", "Set"),
        ("Dictionary<string, int> $values = [1; 2];", "Dictionary"),
    ] {
        let source = format!("function main(): void {{ {declaration} }}");
        let errors = diagnostics(&source);
        assert!(errors.iter().any(|diagnostic| {
            diagnostic.code == "E0529" && diagnostic.message.contains(family)
        }));
    }
}

#[test]
fn runtime_negative_fill_count_preserves_canonical_panic() {
    let source = include_str!("../../../examples/native/main_stage23c_negative_fill_panic.doria");
    let mir = doriac::lower_source_to_mir("stage23c-negative.doria", source)
        .expect("dynamic negative count should reach runtime");
    let output = doriac::mir_interpreter::interpret(&mir)
        .expect("runtime negative fill should produce a Doria panic outcome");
    assert_eq!(output.exit_status, 101);
    assert_eq!(
        output.stderr,
        b"Panic: fill count is negative\nStack Trace:\n  at main\n"
    );
}

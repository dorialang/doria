use doriac::{mir, mir_validation};

fn program(source: &str) -> mir::Program {
    let program = doriac::lower_source_to_mir("optimizer.doria", source)
        .unwrap_or_else(|error| panic!("{source}\n{error:?}"));
    mir_validation::validate_program(&program).unwrap();
    program
}

fn function<'a>(program: &'a mir::Program, name: &str) -> &'a mir::Function {
    program
        .functions
        .iter()
        .find(|function| function.name == name)
        .unwrap()
}

#[test]
fn local_class_storage_preserves_nested_and_checked_cleanup() {
    let program = program(include_str!(
        "../../../examples/native/main_stage35a_stack_classes.doria"
    ));
    let facts = mir_validation::optimization_facts(&program);
    let exercise = function(&program, "exercise");
    assert_eq!(facts.functions[exercise.id.0].stack_classes.len(), 3);
    assert_eq!(
        doriac::mir_interpreter::interpret(&program).unwrap().stdout,
        include_bytes!("fixtures/native_io/main_stage35a_stack_classes/expected_stdout")
    );
    assert!(!doriac::codegen_cranelift::lower_mir_to_object(&program)
        .unwrap()
        .is_empty());
    #[cfg(feature = "llvm-backend")]
    {
        let ir = doriac::codegen_llvm::lower_mir_to_llvm_ir(&program).unwrap();
        let body = llvm_body(&ir, exercise);
        assert_eq!(
            body.lines()
                .filter(|line| line.contains("class.stack.") && line.contains("alloca"))
                .count(),
            3,
            "{body}"
        );
        assert!(
            !body.contains("@dr_v1_class_free("),
            "stack roots cannot be freed: {body}"
        );
        assert!(!doriac::codegen_llvm::lower_mir_to_object(&program)
            .unwrap()
            .is_empty());
    }
}

#[test]
fn returned_captured_erased_and_repeated_objects_keep_heap_storage() {
    let program = program(
        r#"
interface Value { function read(): int; }
class Number implements Value {
    function __construct(int $value) {}
    function read(): int { return $this->value; }
}
function returned(): Number { let $value = new Number(1); return $value; }
function captured(): function(): int {
    let $value = new Number(2);
    return fn() with (take $value) => $value->read();
}
function erased(): Value { let $value = new Number(3); return $value; }
function repeated(): void {
    foreach (0..2 as int $i) { let $value = new Number($i); echo $value->read(); }
}
function main(): void {
    echo returned()->read();
    let $callback = captured(); echo $callback();
    echo erased()->read(); repeated();
}
"#,
    );
    let facts = mir_validation::optimization_facts(&program);
    for name in ["returned", "captured", "erased", "repeated"] {
        assert!(
            facts.functions[function(&program, name).id.0]
                .stack_classes
                .is_empty(),
            "{name}"
        );
    }
}

#[test]
fn pointer_contracts_follow_effects_and_actual_abi_not_readonly_spelling() {
    let program = program(
        r#"
class Number { writable int $value = 1; }
class Text { string $value = "value"; }
open class Base { int $value = 1; }
function read(Number $value): int { return $value->value; }
function change(writable Number $value): void { $value->value++; }
function copyText(Text $value): string { return $value->value; }
function nullable(?Number $value): void {}
function dynamic(Base $value): void {}
function main(): void { let writable $number = new Number(); change($number); echo read($number); }
"#,
    );
    let facts = mir_validation::optimization_facts(&program);
    let parameter = |name| facts.functions[function(&program, name).id.0].parameters[0];
    let read = parameter("read");
    assert!(read.readonly && read.nocapture && read.nonnull);
    assert!(!read.noalias);
    assert_eq!((read.dereferenceable, read.alignment), (8, 8));
    let change = parameter("change");
    assert!(!change.readonly && change.nocapture && change.noalias);
    assert!(!parameter("copyText").readonly);
    assert_eq!(
        parameter("nullable"),
        mir_validation::ParameterFacts::default()
    );
    assert_eq!(
        parameter("dynamic"),
        mir_validation::ParameterFacts::default()
    );
    #[cfg(feature = "llvm-backend")]
    {
        let ir = doriac::codegen_llvm::lower_mir_to_llvm_ir(&program).unwrap();
        let header = llvm_body(&ir, function(&program, "read"))
            .lines()
            .next()
            .unwrap();
        for attribute in [
            "nocapture",
            "nonnull",
            "readonly",
            "align 8",
            "dereferenceable(8)",
        ] {
            assert!(header.contains(attribute), "{header}");
        }
        let header = llvm_body(&ir, function(&program, "copyText"))
            .lines()
            .next()
            .unwrap();
        assert!(!header.contains("readonly"), "{header}");
    }
}

#[test]
fn stack_closure_placement_cannot_be_forged_for_an_escaping_environment() {
    for body in [
        "return fn() with (take $value) => $value;",
        "let $callback = fn() with (take $value) => $value; return $callback;",
    ] {
        let source = format!(
            r#"
function make(): function(): int {{ let $value = 7; {body} }}
function main(): void {{ let $callback = make(); echo $callback(); }}
"#
        );
        let mut program = program(&source);
        assert_eq!(
            program.closure_descriptors[0].environment_placement,
            mir::ClosureEnvironmentPlacement::Heap
        );
        program.closure_descriptors[0].environment_placement =
            mir::ClosureEnvironmentPlacement::Stack;
        assert!(mir_validation::validate_program(&program)
            .unwrap_err()
            .message
            .contains("stack closure environment escapes"));
        assert!(doriac::codegen_cranelift::lower_mir_to_object(&program).is_err());
        #[cfg(feature = "llvm-backend")]
        assert!(doriac::codegen_llvm::lower_mir_to_object(&program).is_err());
    }
}

#[test]
fn escaping_stack_environments_are_rejected_through_aggregate_and_call_paths() {
    for source in [
        r#"function make(): List<function(): int> {
            let $n = 7;
            return [fn() with (take $n) => $n];
        }
        function main(): void { let $callbacks = make(); }"#,
        r#"function main(): void {
            let $n = 7;
            writable List<function(): int> $callbacks = [];
            $callbacks->add(fn() with (take $n) => $n);
        }"#,
        r#"class Holder { function __construct(take function(): int $callback) {} }
        function main(): void { let $n = 7; let $holder = new Holder(fn() with (take $n) => $n); }"#,
        r#"function consume(take function(): int $callback): int { return $callback(); }
        function main(): void { let $n = 7; echo consume(fn() with (take $n) => $n); }"#,
        r#"function make(): mixed { let $n = 7; return fn() with (take $n) => $n; }
        function main(): void { let $value = make(); }"#,
        r#"function make(): mixed {
            let $n = 7; mixed $boxed = fn() with (take $n) => $n;
            let $moved = $boxed; return $moved;
        }
        function main(): void { let $value = make(); }"#,
        r#"function make(): ?mixed {
            let $n = 7; ?mixed $boxed = fn() with (take $n) => $n;
            let $moved = $boxed; return $moved;
        }
        function main(): void { let $value = make(); }"#,
        r#"enum Callback { case Ready(function(): int $callback); }
        function make(): Callback {
            let $n = 7; let $boxed = Callback::Ready(fn() with (take $n) => $n);
            let $moved = $boxed; return $moved;
        }
        function main(): void { let $value = make(); }"#,
        r#"enum Callback { case Ready(function(): int $callback); }
        function make(): ?Callback {
            let $n = 7; ?Callback $boxed = Callback::Ready(fn() with (take $n) => $n);
            let $moved = $boxed; return $moved;
        }
        function main(): void { let $value = make(); }"#,
    ] {
        let mut program = program(source);
        let descriptor = program
            .closure_descriptors
            .iter_mut()
            .find(|descriptor| {
                descriptor.environment_placement == mir::ClosureEnvironmentPlacement::Heap
            })
            .expect(source);
        descriptor.environment_placement = mir::ClosureEnvironmentPlacement::Stack;
        assert!(
            mir_validation::validate_program(&program).is_err(),
            "{source}"
        );
        assert!(
            doriac::codegen_cranelift::lower_mir_to_object(&program).is_err(),
            "{source}"
        );
        #[cfg(feature = "llvm-backend")]
        assert!(
            doriac::codegen_llvm::lower_mir_to_object(&program).is_err(),
            "{source}"
        );
    }
}

#[test]
fn stack_class_budget_is_cumulative_and_pointer_metadata_excludes_escaping_values() {
    let properties = (0..300)
        .map(|i| format!("int $p{i} = {i};"))
        .collect::<String>();
    let source = format!(
        r#"
class Large {{ {properties} }}
class Number {{ int $value = 1; }}
function identity(take Number $value): Number {{ return $value; }}
function main(): void {{ let $one = new Large(); let $two = new Large(); echo $one->p0 + $two->p1; }}
"#
    );
    let program = program(&source);
    let facts = mir_validation::optimization_facts(&program);
    assert_eq!(
        facts.functions[function(&program, "main").id.0]
            .stack_classes
            .len(),
        1
    );
    let identity = facts.functions[function(&program, "identity").id.0].parameters[0];
    assert!(!identity.nocapture && !identity.noalias && !identity.readonly);
}

#[test]
fn closure_storage_is_reserved_only_by_functions_that_construct_it() {
    let program = program(
        r#"
function unrelated(): int { return 4; }
function main(): void {
    let $value = unrelated(); let $callback = fn() with ($value) => $value; echo $callback();
}
"#,
    );
    let facts = mir_validation::optimization_facts(&program);
    assert!(facts.functions[function(&program, "unrelated").id.0]
        .constructed_closures
        .is_empty());
    assert_eq!(
        facts.functions[function(&program, "main").id.0]
            .constructed_closures
            .len(),
        1
    );
    #[cfg(feature = "llvm-backend")]
    {
        let ir = doriac::codegen_llvm::lower_mir_to_llvm_ir(&program).unwrap();
        assert!(!llvm_body(&ir, function(&program, "unrelated")).contains("closure.environment."));
        assert!(
            llvm_body(&ir, function(&program, "main")).contains("closure.environment.0 = alloca")
        );
    }
}

#[test]
fn taking_parameters_classify_transferred_environments_as_owned() {
    let program = program(include_str!(
        "../../../examples/native/main_stage35a_transferred_closure.doria"
    ));
    assert_eq!(
        doriac::mir_interpreter::interpret(&program).unwrap().stdout,
        include_bytes!("fixtures/native_io/main_stage35a_transferred_closure/expected_stdout")
    );
    assert!(program
        .closure_descriptors
        .iter()
        .filter(|descriptor| descriptor.environment_layout.is_some())
        .all(
            |descriptor| descriptor.environment_placement == mir::ClosureEnvironmentPlacement::Heap
        ));
}

#[cfg(feature = "llvm-backend")]
#[test]
fn release_devirtualizes_exact_calls_but_retains_unknown_dispatch() {
    let exact = program(include_str!(
        "../../../examples/native/main_stage34_inheritance_devirtualized_exact_call.doria"
    ));
    let ir = doriac::codegen_llvm::lower_mir_to_optimized_llvm_ir(&exact).unwrap();
    assert!(!ir.lines().any(|line| line.contains("call i64 %")), "{ir}");

    let unknown = program(
        r#"
interface Value { function read(): int; }
class First implements Value { function read(): int { return 11; } }
class Second implements Value { function read(): int { return 29; } }
function select(bool $first): Value {
    if ($first) { return new First(); } return new Second();
}
function erased(Value $value): int { return $value->read(); }
function main(List<string> $args): void {
    let $value = select($args->count == 0); echo erased($value);
}
"#,
    );
    let raw = doriac::codegen_llvm::lower_mir_to_llvm_ir(&unknown).unwrap();
    assert!(llvm_body(&raw, function(&unknown, "erased")).contains("%interface.method.entry("));
    let optimized = doriac::codegen_llvm::lower_mir_to_optimized_llvm_ir(&unknown).unwrap();
    assert!(
        optimized
            .lines()
            .any(|line| line.contains("call i8 %interface.method.entry")),
        "{optimized}"
    );
}

#[test]
fn generic_and_trait_calls_remain_static_and_primitive_constraints_do_not_box() {
    for source in [
        include_str!("../../../examples/native/main_stage35_interface_structure.doria"),
        include_str!("../../../examples/native/main_stage35_trait_generic.doria"),
        include_str!("../../../examples/native/main_stage35_interface_primitive_contracts.doria"),
    ] {
        let program = program(source);
        for function in program.functions.iter().filter(|function| {
            [
                "constrained",
                "same",
                "order",
                "hashValue",
                "Numbers::",
                "Words::",
            ]
            .iter()
            .any(|name| function.name.starts_with(name))
        }) {
            assert!(
                !function.blocks.iter().any(|block| matches!(
                    block.terminator,
                    mir::Terminator::IndirectCall {
                        callee: mir::IndirectCallee::InterfaceMethod { .. },
                        ..
                    } | mir::Terminator::CheckedIndirectCall {
                        callee: mir::IndirectCallee::InterfaceMethod { .. },
                        ..
                    }
                )),
                "{function}"
            );
        }
        #[cfg(feature = "llvm-backend")]
        {
            let ir = doriac::codegen_llvm::lower_mir_to_llvm_ir(&program).unwrap();
            assert!(!ir.contains("mixed_box"), "{ir}");
        }
    }
}

#[cfg(feature = "llvm-backend")]
fn llvm_body<'a>(ir: &'a str, function: &mir::Function) -> &'a str {
    let symbol = doriac::native_abi::function_symbol(function);
    ir.split("define ")
        .find(|body| {
            body.lines()
                .next()
                .is_some_and(|line| line.contains(&format!("@{symbol}(")))
        })
        .unwrap()
        .split("\n}")
        .next()
        .unwrap()
}

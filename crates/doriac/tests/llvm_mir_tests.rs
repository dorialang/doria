#![cfg(feature = "llvm-backend")]

use doriac::mir::{
    BasicBlock, BlockId, FloatBinaryOp, FloatExpression, Function, FunctionId, Program, ReturnType,
    Rvalue, ScalarType, Terminator, Type, ValueExpression,
};
use doriac::numeric::{FloatType, FloatValue};

fn assert_object(source: &str) {
    let program =
        doriac::lower_source_to_mir("llvm-test.doria", source).expect("source should lower to MIR");
    let object = doriac::codegen_llvm::lower_mir_to_object(&program)
        .expect("verified MIR should lower to an optimized LLVM object");
    assert!(!object.is_empty());
}

#[test]
fn closure_ir_keeps_descriptors_static_and_environment_allocations_escape_selected() {
    let local = doriac::lower_source_to_mir(
        "llvm-local-closure.doria",
        r#"
function main(): void
{
    let $value = 42;
    let $callback = fn() with ($value) => $value;
    echo "{$callback()}\n";
}
"#,
    )
    .expect("nonescaping closure should lower");
    let local_ir = doriac::codegen_llvm::lower_mir_to_llvm_ir(&local)
        .expect("nonescaping closure should lower to LLVM IR");
    assert!(
        local_ir.contains("@__doria_closure_descriptor_0 = internal constant { ptr, ptr }"),
        "closure descriptor is not immutable two-word data:\n{local_ir}"
    );
    assert!(
        local_ir.contains("@__doria_drop_closure_environment_0"),
        "closure descriptor has no generated drop identity:\n{local_ir}"
    );
    assert!(
        local_ir.contains("closure.environment.0 = alloca"),
        "nonescaping closure has no stack environment:\n{local_ir}"
    );
    assert!(
        !local_ir.contains("dr_v1_closure_environment_allocate"),
        "nonescaping closure unexpectedly uses heap allocation:\n{local_ir}"
    );
    assert!(
        scan_alloca_placement(&local_ir).escaped.is_empty(),
        "closure storage escaped the LLVM entry block:\n{local_ir}"
    );

    let escaping = doriac::lower_source_to_mir(
        "llvm-escaping-closure.doria",
        r#"
function bind(string $value): function(): string
{
    return fn() with (take $value) => $value;
}

function main(): void
{
    let $callback = bind("owned");
    echo $callback() . "\n";
}
"#,
    )
    .expect("escaping closure should lower");
    let escaping_ir = doriac::codegen_llvm::lower_mir_to_llvm_ir(&escaping)
        .expect("escaping closure should lower to LLVM IR");
    for required in [
        "dr_v1_closure_environment_allocate",
        "dr_v1_closure_environment_free",
        "closure.checked.call",
    ] {
        assert!(
            escaping_ir.contains(required),
            "escaping closure IR is missing {required}:\n{escaping_ir}"
        );
    }
    assert!(
        !escaping_ir.contains("%closure.call ="),
        "ambient-capable structural callable used the unchecked ABI:\n{escaping_ir}"
    );
    for forbidden in ["closure_retain", "closure_release", "closure_registry"] {
        assert!(
            !escaping_ir.contains(forbidden),
            "ordinary closure ownership gained {forbidden}:\n{escaping_ir}"
        );
    }
}

#[test]
fn virtual_receivers_use_one_slot_abi_for_open_roots_and_closed_overrides() {
    let program = doriac::lower_source_to_mir(
        "llvm-virtual-receiver.doria",
        include_str!(
            "../../../examples/native/main_stage34_inheritance_writable_shared_reference.doria"
        ),
    )
    .expect("inheritance source should lower");
    let override_function = program
        .functions
        .iter()
        .find(|function| function.name == "AnswerCounter::add" && function.virtual_slot.is_some())
        .expect("closed override implementation");
    let direct_function = program
        .functions
        .iter()
        .find(|function| function.name == "AnswerCounter::add::<direct>")
        .expect("closed exact-call implementation");
    assert!(override_function.uses_virtual_receiver_abi());
    assert!(!direct_function.uses_virtual_receiver_abi());

    let ir = doriac::codegen_llvm::lower_mir_to_llvm_ir(&program)
        .expect("virtual receiver source should lower to LLVM IR");
    let override_symbol = doriac::native_abi::function_symbol(override_function);
    let override_definition = ir
        .lines()
        .find(|line| line.contains(&format!("@{override_symbol}(")))
        .expect("closed override LLVM definition");
    assert!(
        override_definition.contains("{ ptr, ptr }"),
        "vtable implementation does not use the uniform receiver carrier:\n{override_definition}"
    );
    let direct_symbol = doriac::native_abi::function_symbol(direct_function);
    let direct_definition = ir
        .lines()
        .find(|line| line.contains(&format!("@{direct_symbol}(")))
        .expect("closed direct LLVM definition");
    assert!(
        !direct_definition.contains("{ ptr, ptr }"),
        "closed direct implementation lost its compact receiver ABI:\n{direct_definition}"
    );

    for source in [
        include_str!("../../../examples/native/main_stage34_inheritance_checked_virtual.doria"),
        include_str!(
            "../../../examples/native/main_stage34_inheritance_devirtualized_exact_call.doria"
        ),
    ] {
        assert_object(source);
    }
}

#[test]
fn checked_error_ir_uses_status_out_slots_static_metadata_and_entry_scratch() {
    let source = include_str!("../../../examples/native/main_checked_error_catch.doria");
    let program = doriac::lower_source_to_mir("llvm-checked-errors.doria", source)
        .expect("checked-error source should lower to validated MIR");
    let ir = doriac::codegen_llvm::lower_mir_to_llvm_ir(&program)
        .expect("checked-error MIR should lower to LLVM IR");

    assert!(ir.contains("@__doria_error_descriptor_0"), "{ir}");
    assert!(ir.contains("@__doria_error_origin_0"), "{ir}");
    assert!(
        ir.contains("define internal i8"),
        "throwing ABI must return i8:\n{ir}"
    );
    assert!(
        ir.contains("checked.call"),
        "checked call is missing:\n{ir}"
    );
    assert!(
        ir.contains("checked.call.succeeded") && ir.contains("checked.call.failed"),
        "checked status does not have distinct success and error tests:\n{ir}"
    );
    assert!(
        ir.contains("checked.call.invalid-status"),
        "an impossible checked status is not trapped structurally:\n{ir}"
    );
    assert!(
        ir.contains("error.origin.empty") && ir.contains("error.origin.write"),
        "first-throw origin is not set conditionally:\n{ir}"
    );
    assert!(
        ir.contains("error.descriptor") && ir.contains("icmp eq ptr"),
        "exact catch does not compare descriptor identity:\n{ir}"
    );
    for forbidden in [" invoke ", "landingpad", "personality", "resume "] {
        assert!(
            !ir.contains(forbidden),
            "LLVM unwinding leaked into Doria MIR:\n{ir}"
        );
    }

    let placement = scan_alloca_placement(&ir);
    assert!(
        placement.escaped.is_empty(),
        "checked-call scratch escaped the function entry block:\n{}",
        placement.escaped.join("\n")
    );
}

#[test]
fn lowers_complete_stage_14_mir_shapes_to_verified_objects() {
    for source in [
        include_str!("../../../examples/native/main_return_42.doria"),
        include_str!("../../../examples/native/main_void_empty.doria"),
        include_str!("../../../examples/native/main_function_add_42.doria"),
        include_str!("../../../examples/native/main_recursive_fibonacci_55.doria"),
        include_str!("../../../examples/native/main_narrow_recursive_42.doria"),
        include_str!("../../../examples/native/main_fixed_width_arithmetic_42.doria"),
        include_str!("../../../examples/native/main_uint64_boundary_42.doria"),
        include_str!("../../../examples/native/main_add_overflow_panic.doria"),
        include_str!("../../../examples/native/main_divide_by_zero_panic.doria"),
        include_str!("../../../examples/native/main_shift_count_panic.doria"),
        include_str!("../../../examples/native/main_integer_conversion_panic.doria"),
        include_str!("../../../examples/native/main_float32_rounding_42.doria"),
        include_str!("../../../examples/native/main_float64_arithmetic_42.doria"),
        include_str!("../../../examples/native/main_float_nan_comparison_42.doria"),
        include_str!("../../../examples/native/main_float_signed_zero_42.doria"),
        include_str!("../../../examples/native/main_bool_short_circuit_42.doria"),
        include_str!("../../../examples/native/main_bool_xor_42.doria"),
        include_str!("../../../examples/native/main_float_to_int_42.doria"),
        include_str!("../../../examples/native/main_float_to_int_nan_panic.doria"),
        include_str!("../../../examples/native/main_float_to_int_infinity_panic.doria"),
        include_str!("../../../examples/native/main_float_to_int_range_panic.doria"),
        include_str!("../../../examples/native/main_string_concat_hello.doria"),
        include_str!("../../../examples/native/main_invalid_status_panic.doria"),
        include_str!("../../../examples/native/main_release_profile_42.doria"),
    ] {
        assert_object(source);
    }
}

#[test]
fn enum_ir_preserves_inline_tags_backings_nullability_and_mixed_identity() {
    let source = r#"
enum Status { case Draft; case Published; }
enum Priority: int { case Low = 1; case High = 10; }
enum Transport: string { case Road = "road"; case Rail = "rail"; }
function main(): void throws Doria\Std\Io\IoError
{
    Status $status = Status::Draft;
    Priority $priority = Priority::High;
    Transport $transport = Transport::Rail;
    ?Status $nullable = Status::Draft;
    mixed $boxed = Status::Draft;
    echo $status == Status::Draft;
    echo $priority->value;
    echo $transport->value;
    echo $nullable != null;
    echo $boxed is Status;
}

"#;
    let program = doriac::lower_source_to_mir("llvm-enum.doria", source)
        .expect("enum source should lower to validated MIR");
    let ir = doriac::codegen_llvm::lower_mir_to_llvm_ir(&program)
        .expect("enum MIR should lower to LLVM IR");

    assert!(
        ir.contains("icmp eq i32"),
        "enum equality is not a tag comparison:\n{ir}"
    );
    assert!(
        ir.contains("enum.backing.value"),
        "int backing is not selected in O(1):\n{ir}"
    );
    assert!(
        ir.contains("enum.backing.string"),
        "string backing has no static-data selection:\n{ir}"
    );
    assert!(
        ir.contains("__doria_string_"),
        "string backing bytes are not module data:\n{ir}"
    );
    assert!(
        ir.contains("mixed.type.matches"),
        "mixed enum narrowing ignores nominal identity:\n{ir}"
    );
    assert!(
        ir.contains("{ i64, i32 }") || ir.contains("{ i8, i32 }") || ir.contains("{ i1, i32 }"),
        "nullable enum lacks a separate presence field:\n{ir}"
    );
    assert!(
        !ir.contains("dr_v1_enum_"),
        "unit/backed enums gained a runtime allocation API:\n{ir}"
    );
}

#[test]
fn payload_enum_ir_stays_inline_across_mixed_and_collection_storage() {
    let source = include_str!("../../../examples/native/main_payload_enums_mixed.doria");
    let collections = include_str!("../../../examples/native/main_payload_enums_collections.doria");

    for (name, source) in [("mixed", source), ("collections", collections)] {
        let program = doriac::lower_source_to_mir(format!("llvm-payload-{name}.doria"), source)
            .expect("payload enum source should lower to validated MIR");
        let ir = doriac::codegen_llvm::lower_mir_to_llvm_ir(&program)
            .expect("payload enum MIR should lower to LLVM IR");

        assert!(
            ir.contains("payload.enum.construct"),
            "payload construction is not represented as inline storage:\n{ir}"
        );
        assert!(
            !ir.contains("dr_v1_enum_") && !ir.contains("dr_v4_enum_"),
            "ordinary payload enums gained a runtime allocation API:\n{ir}"
        );
        if name == "mixed" {
            assert!(
                ir.contains("dr_v2_mixed_new_aggregate"),
                "mixed payload enums do not use the existing aggregate box:\n{ir}"
            );
        } else {
            assert!(
                ir.contains("dr_v4_collection_new_aggregate"),
                "payload enum collections do not use inline aggregate slots:\n{ir}"
            );
        }
    }
}

#[test]
fn match_ir_uses_inline_dispatch_and_projects_payloads_only_in_selected_arms() {
    let source = r#"
enum Outcome { case Empty; case Number(int $value); case Pair(int $left, int $right); }

function value(Outcome $outcome): int
{
    return match ($outcome) {
        Outcome::Empty => 0,
        Outcome::Number($number) => $number,
        Outcome::Pair($left, $right) => $left + $right,
    };
}

function main(): int { return value(Outcome::Pair(20, 22)); }
"#;
    let program = doriac::lower_source_to_mir("llvm-match.doria", source)
        .expect("match source should lower to validated MIR");
    let ir = doriac::codegen_llvm::lower_mir_to_llvm_ir(&program)
        .expect("match MIR should lower to LLVM IR");

    assert!(
        ir.contains("payload.case.matches"),
        "payload match has no inline tag comparison:\n{ir}"
    );
    assert!(
        ir.contains("payload.binding.load"),
        "selected payload arms do not project their fields:\n{ir}"
    );
    assert!(
        !ir.contains("dr_v1_enum_") && !ir.contains("dr_v4_enum_"),
        "match dispatch introduced a runtime enum allocation API:\n{ir}"
    );

    let lines = ir.lines().collect::<Vec<_>>();
    for binding_line in lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| line.contains("payload.binding.load").then_some(index))
    {
        let block = lines[..binding_line]
            .iter()
            .rev()
            .find_map(|line| {
                (!line.chars().next().is_some_and(char::is_whitespace))
                    .then(|| line.split_once(':').map(|(label, _)| label))
                    .flatten()
            })
            .expect("payload projection should be inside a named basic block");
        let branch_target = format!("label %{block}");
        assert!(
            lines[..binding_line].iter().any(|line| {
                line.contains("br i1 %payload.case.matches") && line.contains(&branch_target)
            }),
            "payload projection block `{block}` is not selected by its case test:\n{ir}"
        );
    }
}

#[test]
fn guarded_consuming_match_ir_keeps_storage_inline_and_scratch_in_entry() {
    let source = include_str!("../../../examples/native/main_match_guarded_take.doria");
    let program = doriac::lower_source_to_mir("llvm-guarded-take.doria", source)
        .expect("guarded consuming match should lower to validated MIR");
    let ir = doriac::codegen_llvm::lower_mir_to_llvm_ir(&program)
        .expect("guarded consuming match MIR should lower to LLVM IR");

    let case_test = ir
        .find("payload.enum.case.matches")
        .expect("guarded match should test the inline enum tag");
    let extraction = ir[case_test..]
        .find("llvm.memset")
        .map(|offset| case_test + offset)
        .expect("selected consuming arm should clear its moved payload slot");
    assert!(
        case_test < extraction,
        "payload extraction preceded its exact case test:\n{ir}"
    );
    assert!(
        ir.matches("payload.enum.case.matches").count() >= 2,
        "guard fallthrough did not retain the later repeated case test:\n{ir}"
    );
    assert!(
        !ir.contains("dr_v1_enum_") && !ir.contains("dr_v4_enum_"),
        "guarded consuming match introduced a runtime match/enum allocation:\n{ir}"
    );

    let placement = scan_alloca_placement(&ir);
    assert!(
        placement.escaped.is_empty(),
        "guarded consuming match allocated scratch outside entry:\n{}",
        placement.escaped.join("\n")
    );
}

#[test]
fn stage28a_control_flow_keeps_one_validated_cfg_without_runtime_objects() {
    let source = r#"
function select(bool $ready): int
{
    return when ($ready): int { return 42; } else { return 0; };
}
function gated(bool $ready): void
{
    let writable $count = 0;
    given {
        let $limit = 2;
        $ready;
    } while ($count < $limit) {
        $count++;
        continue;
    }
}
function repeat(): void
{
    let writable $count = 0;
    do {
        $count++;
        if ($count < 2) { continue; }
    } while ($count < 3);
}
function record(string $message): void
{
    try { echo $message; } catch (Doria\Std\Io\IoError) {}
}
function finalized(bool $ready): int throws Doria\Std\Io\IoError
{
    if ($ready) {
        return 42;
    } finally {
        record("cleanup");
    }
    return 0;
}
function loopFinalizer(): void throws Doria\Std\Io\IoError
{
    let writable $count = 0;
    while ($count < 2) {
        $count++;
        if ($count == 1) { continue; }
        break;
    } finally {
        record("loop cleanup");
    }
}
function main(): void throws Doria\Std\Io\IoError
{
    echo "{select(true)}:{finalized(true)}";
    gated(true);
    repeat();
    loopFinalizer();
}
"#;
    let program = doriac::lower_source_to_mir("llvm-stage28a.doria", source)
        .expect("Stage 28a source should lower to validated MIR");

    let select = program
        .functions
        .iter()
        .find(|function| function.name == "select")
        .expect("select should exist");
    let when = select
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| match statement {
            doriac::mir::Statement::ControlFlowPlan(doriac::mir::ControlFlowPlan::When(plan)) => {
                Some(plan)
            }
            _ => None,
        })
        .expect("when result plan should exist");
    for branch in &when.branches {
        let block = &select.blocks[branch.0];
        assert_eq!(
            block
                .statements
                .iter()
                .filter(|statement| matches!(
                    statement,
                    doriac::mir::Statement::AssignLocal { target, .. } if *target == when.result
                ))
                .count(),
            1,
            "each selected when branch must write one merge result"
        );
        assert!(matches!(block.terminator, Terminator::Jump(target) if target == when.merge));
    }

    let gated = program
        .functions
        .iter()
        .find(|function| function.name == "gated")
        .expect("gated should exist");
    let given = gated
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| match statement {
            doriac::mir::Statement::ControlFlowPlan(doriac::mir::ControlFlowPlan::Given(plan)) => {
                Some(plan)
            }
            _ => None,
        })
        .expect("given while plan should exist");
    assert_eq!(given.attachment, doriac::mir::GivenAttachment::While);
    assert_ne!(given.setup_entry, given.predicates[0].block);
    let Terminator::Branch { else_block, .. } =
        gated.blocks[given.predicates[0].block.0].terminator
    else {
        panic!("given predicate should be a bool branch");
    };
    assert_eq!(Some(else_block), given.gate_failed);
    for source in &given.continue_sources {
        assert!(matches!(
            gated.blocks[source.0].terminator,
            Terminator::Jump(target) if target == given.predicates[0].block
        ));
    }

    let repeat = program
        .functions
        .iter()
        .find(|function| function.name == "repeat")
        .expect("repeat should exist");
    let do_while = repeat
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| match statement {
            doriac::mir::Statement::ControlFlowPlan(doriac::mir::ControlFlowPlan::DoWhile(
                plan,
            )) => Some(plan),
            _ => None,
        })
        .expect("do-while plan should exist");
    assert!(matches!(
        repeat.blocks[do_while.entry.0].terminator,
        Terminator::Jump(target) if target == do_while.body
    ));
    assert!(matches!(
        repeat.blocks[do_while.condition.0].terminator,
        Terminator::Branch { then_block, else_block, .. }
            if then_block == do_while.body && else_block == do_while.exit
    ));
    for source in &do_while.continue_sources {
        assert!(matches!(
            repeat.blocks[source.0].terminator,
            Terminator::Jump(target) if target == do_while.condition
        ));
    }

    let finalized = program
        .functions
        .iter()
        .find(|function| function.name == "finalized")
        .expect("finalized should exist");
    let finalized_plan = finalized
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| match statement {
            doriac::mir::Statement::ControlFlowPlan(doriac::mir::ControlFlowPlan::Finalizer(
                plan,
            )) => Some(plan),
            _ => None,
        })
        .expect("returning if-finalizer plan should exist");
    assert_eq!(
        finalized_plan.attachment,
        doriac::mir::FinalizerAttachment::If
    );
    assert!(finalized_plan.exits.iter().any(|exit| matches!(
        exit.kind,
        doriac::mir::StructuredExitKind::FunctionReturn { value: Some(_) }
    )));

    let looped = program
        .functions
        .iter()
        .find(|function| function.name == "loopFinalizer")
        .expect("loopFinalizer should exist");
    let loop_plan = looped
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| match statement {
            doriac::mir::Statement::ControlFlowPlan(doriac::mir::ControlFlowPlan::Finalizer(
                plan,
            )) => Some(plan),
            _ => None,
        })
        .expect("while-finalizer plan should exist");
    assert_eq!(
        loop_plan.attachment,
        doriac::mir::FinalizerAttachment::While
    );
    assert!(loop_plan
        .exits
        .iter()
        .any(|exit| matches!(exit.kind, doriac::mir::StructuredExitKind::Break)));
    assert!(!loop_plan
        .exits
        .iter()
        .any(|exit| matches!(exit.kind, doriac::mir::StructuredExitKind::Continue)));

    let ir = doriac::codegen_llvm::lower_mir_to_llvm_ir(&program)
        .expect("validated Stage 28a MIR should lower to LLVM IR");
    let placement = scan_alloca_placement(&ir);
    assert!(
        placement.escaped.is_empty(),
        "Stage 28a control flow allocated loop scratch outside entry:\n{}",
        placement.escaped.join("\n")
    );
    assert!(
        ![
            "dr_v1_when",
            "dr_v1_given",
            "dr_v1_do_while",
            "dr_v1_finalizer",
            "dr_v1_cleanup_stack",
        ]
        .iter()
        .any(|name| ir.contains(name)),
        "Stage 28a control flow introduced runtime control-flow objects:\n{ir}"
    );
}

#[test]
fn rejects_malformed_mixed_width_float_mir_before_llvm_emission() {
    let program = Program {
        sources: vec![doriac::mir::SourceUnit::standalone("llvm-test.doria", "")],
        packages: vec![doriac::mir::PackageUnit::standalone()],
        selected_target: doriac::mir::SelectedTarget::standalone("llvm-test.doria"),
        source: doriac::source::SourceFile::new("llvm-test.doria", ""),
        compilation_context: doriac::names::CompilationContext::standalone("llvm-test.doria"),
        namespace: None,
        global_symbols: doriac::names::GlobalSymbolFacts::default(),
        enums: vec![],
        classes: vec![],
        collection_types: vec![],
        statics: vec![],
        error_descriptors: vec![],
        error_origins: vec![],
        function_types: Vec::new(),
        closure_descriptors: Vec::new(),
        closure_environment_layouts: Vec::new(),
        functions: vec![
            Function {
                id: FunctionId(0),
                name: "main".to_string(),
                source_span: Default::default(),
                method: None,
                virtual_slot: None,
                receiver_mode: None,
                params: Vec::new(),
                parameter_modes: Vec::new(),
                return_type: ReturnType::Void,
                return_borrow: None,
                required_checked_effects: Vec::new(),
                ambient_checked_effects: Vec::new(),
                test_assertion_checked_effects: Vec::new(),
                checked_effects: Vec::new(),
                locals: Vec::new(),
                blocks: vec![BasicBlock {
                    id: BlockId(0),
                    statements: Vec::new(),
                    terminator: Terminator::ReturnVoid,
                }],
                entry_block: BlockId(0),
                closure: None,
            },
            Function {
                id: FunctionId(1),
                name: "mixedWidth".to_string(),
                source_span: Default::default(),
                method: None,
                virtual_slot: None,
                receiver_mode: None,
                params: Vec::new(),
                parameter_modes: Vec::new(),
                return_type: ReturnType::Value(Type::Scalar(ScalarType::Float(FloatType::Float64))),
                return_borrow: None,
                required_checked_effects: Vec::new(),
                ambient_checked_effects: Vec::new(),
                test_assertion_checked_effects: Vec::new(),
                checked_effects: Vec::new(),
                locals: Vec::new(),
                blocks: vec![BasicBlock {
                    id: BlockId(0),
                    statements: Vec::new(),
                    terminator: Terminator::Return(Rvalue::Value(ValueExpression::Float(
                        FloatExpression::Binary {
                            ty: FloatType::Float64,
                            op: FloatBinaryOp::Add,
                            left: Box::new(FloatExpression::constant(FloatValue::from_f32(1.0))),
                            right: Box::new(FloatExpression::constant(FloatValue::from_f64(2.0))),
                        },
                    ))),
                }],
                entry_block: BlockId(0),
                closure: None,
            },
        ],
        selected_entry: Some(FunctionId(0)),
        entry: FunctionId(0),
    };

    let error = doriac::codegen_llvm::lower_mir_to_object(&program)
        .expect_err("malformed MIR should be rejected before LLVM construction");
    assert!(error
        .message
        .contains("float binary expression has float32 and float operands"));
}

#[test]
fn lowers_complete_stage17_io_and_format_mir_to_verified_objects() {
    for source in [
        include_str!("../../../examples/native/main_read_line_echo.doria"),
        include_str!("../../../examples/native/main_file_copy.doria"),
        include_str!("../../../examples/native/main_sprintf_matrix.doria"),
        include_str!("../../../examples/native/main_printf_42.doria"),
        include_str!("../../../examples/native/main_write_stderr.doria"),
        include_str!("../../../examples/native/main_missing_file_panic.doria"),
        r#"
function identity(?string $value): ?string { return $value; }
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    let $line = identity(read_line());
    if ($line != null) { echo $line; }
}
"#,
    ] {
        assert_object(source);
    }
}

#[test]
fn lowers_stage_18_expression_interpolation_to_verified_objects() {
    for source in [
        include_str!("../../../examples/native/main_expression_interpolation.doria"),
        include_str!("../../../examples/native/main_expression_interpolation_order.doria"),
    ] {
        assert_object(source);
    }
}

/// Where each `alloca` in a module was emitted.
///
/// `blocks` and `in_entry` exist so a caller can prove the scan actually
/// happened. An earlier version of this walk silently matched nothing, and an
/// empty `escaped` looked identical to a clean module.
#[derive(Default)]
struct AllocaPlacement {
    blocks: usize,
    in_entry: usize,
    escaped: Vec<String>,
}

/// Scans printed LLVM IR for allocations emitted outside their function's
/// entry block.
///
/// The IR is read as text because the property under test is exactly what the
/// printed module says: which basic block each allocation landed in.
fn scan_alloca_placement(ir: &str) -> AllocaPlacement {
    let mut placement = AllocaPlacement::default();
    let mut function = String::new();
    let mut entry = String::new();
    let mut block = String::new();

    for line in ir.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("define ") {
            function = rest
                .split_once('@')
                .map(|(_, name)| name.split('(').next().unwrap_or(name).to_string())
                .unwrap_or_default();
            entry.clear();
            block.clear();
            continue;
        }
        if trimmed == "}" {
            function.clear();
            continue;
        }
        if function.is_empty() || trimmed.is_empty() {
            continue;
        }
        // A block label is unindented and is either bare or followed by a
        // `; preds = ...` comment. Matching on a trailing colon alone misses
        // every block that has a predecessor, which is nearly all of them.
        if !line.starts_with(char::is_whitespace) {
            if let Some((label, rest)) = trimmed.split_once(':') {
                let rest = rest.trim();
                if rest.is_empty() || rest.starts_with(';') {
                    block = label.to_string();
                    placement.blocks += 1;
                    if entry.is_empty() {
                        entry = block.clone();
                    }
                    continue;
                }
            }
        }
        if trimmed.contains(" = alloca ") {
            if block == entry {
                placement.in_entry += 1;
            } else {
                placement
                    .escaped
                    .push(format!("{function}: {trimmed}  (in block '{block}')"));
            }
        }
    }
    placement
}

/// A scratch slot emitted outside the entry block is a dynamic stack
/// allocation. LLVM moves the stack pointer when it executes and does not
/// reclaim it until the function returns, so one emitted inside a loop grows
/// the frame every iteration until the program hits its guard page and dies
/// with no diagnostic. Emitting into the entry block instead makes the slot
/// part of the fixed frame, which costs one prologue instruction regardless of
/// how many times the surrounding code runs.
#[test]
fn allocates_every_scratch_slot_in_the_entry_block() {
    let sources: [&str; 8] = [
        // Dictionary get, set, index, and remove: the shape that first failed.
        r#"
function main(): void throws Doria\Std\Io\IoError
{
    writable Dictionary<string, int> $values = [];
    writable List<string> $keys = [];
    for (let writable $index = 0; $index < 4; $index++) {
        let $key = "key{$index}";
        $values->set($key, $index);
        $keys->add($key);
    }
    let writable $total = 0;
    for (let writable $index = 0; $index < 4; $index++) {
        let $key = $keys[$index];
        $total = $total + ($values->get($key) ?? 0);
        $total = $total + $values[$key];
    }
    for (let writable $index = 0; $index < 4; $index++) {
        $values->remove($keys[$index]);
    }
    echo "{$total}:{$values->count}\n";
}
"#,
        // Aggregate enum collection reads, nullable removals, conversion, and
        // cleanup must also reuse fixed entry-block scratch.
        include_str!("../../../examples/native/main_payload_enums_collections.doria"),
        // Set construction, membership, and removal inside a loop.
        r#"
function main(): void throws Doria\Std\Io\IoError
{
    writable Set<int> $seen = Set::from([]);
    let writable $hits = 0;
    for (let writable $index = 0; $index < 8; $index++) {
        $seen->add($index % 4);
        if ($seen->contains($index % 4)) { $hits = $hits + 1; }
    }
    for (let writable $index = 0; $index < 4; $index++) {
        $seen->remove($index);
    }
    echo "{$hits}:{$seen->count}\n";
}
"#,
        // List access and removal, which drive the collection drop loops.
        r#"
function main(): void throws Doria\Std\Io\IoError
{
    writable List<string> $items = [];
    for (let writable $index = 0; $index < 8; $index++) {
        $items->add("item{$index}");
    }
    let writable $count = 0;
    for (let writable $index = 0; $index < 4; $index++) {
        let $removed = $items->removeAt(0);
        $count = $count + $removed->length;
    }
    echo "{$count}:{$items->count}\n";
}
"#,
        // String search and parse, each of which allocates an out-parameter.
        r#"
function main(): void throws Doria\Std\Io\IoError
{
    let writable $found = 0;
    for (let writable $index = 0; $index < 8; $index++) {
        let $text = "value{$index}";
        if (String::contains($text, "value")) { $found = $found + 1; }
        $found = $found + (Int::parse("{$index}") ?? 0);
    }
    echo "{$found}\n";
}
"#,
        // Sorted collections, which take the ordered runtime paths.
        r#"
function main(): void throws Doria\Std\Io\IoError
{
    writable SortedSet<int> $ordered = SortedSet::from([]);
    writable SortedDictionary<string, int> $indexed = SortedDictionary::from([]);
    for (let writable $index = 0; $index < 8; $index++) {
        $ordered->add($index);
        $indexed->set("key{$index}", $index);
    }
    let writable $total = 0;
    for (let writable $index = 0; $index < 8; $index++) {
        if ($ordered->contains($index)) { $total = $total + 1; }
        $total = $total + ($indexed->get("key{$index}") ?? 0);
    }
    echo "{$total}\n";
}
"#,
        // Collection clear uses the same release loop repeatedly; its index
        // scratch must remain in the fixed entry frame.
        r#"
function main(): void throws Doria\Std\Io\IoError
{
    writable List<string> $values = [];
    for (let writable $index = 0; $index < 8; $index++) {
        $values->add("value{$index}");
        $values->clear();
    }
    echo "{$values->count}\n";
}
"#,
        // Class temporaries allocated in a loop body.
        r#"
class Point
{
    function __construct(int $x, int $y)
    {
    }
}

function main(): void throws Doria\Std\Io\IoError
{
    let writable $total = 0;
    for (let writable $index = 0; $index < 8; $index++) {
        let $point = new Point($index, $index * 2);
        $total = ($total + $point->x + $point->y) % 1000;
    }
    echo "{$total}\n";
}
"#,
    ];

    for source in sources {
        let program = doriac::lower_source_to_mir("llvm-test.doria", source)
            .expect("source should lower to MIR");
        let ir = doriac::codegen_llvm::lower_mir_to_llvm_ir(&program)
            .expect("verified MIR should lower to LLVM IR");
        let placement = scan_alloca_placement(&ir);
        // Prove the scan saw a real module before trusting that it found
        // nothing: a walk that matches no blocks reports a clean result too.
        assert!(
            placement.blocks > 1,
            "expected a multi-block module, saw {} blocks",
            placement.blocks
        );
        assert!(
            placement.in_entry > 0,
            "expected entry-block allocations, saw none"
        );
        assert!(
            placement.escaped.is_empty(),
            "these allocations leak stack on every pass through their block:\n{}",
            placement.escaped.join("\n")
        );
    }
}

#[test]
fn collection_clear_uses_reset_and_type_aware_release_paths() {
    let program = doriac::lower_source_to_mir(
        "llvm-clear.doria",
        r#"
function main(): void
{
    writable List<int> $scalars = [1, 2];
    $scalars->clear();
    writable List<string> $strings = ["one", "two"];
    $strings->clear();
}
"#,
    )
    .expect("collection clear should lower to MIR");
    let ir = doriac::codegen_llvm::lower_mir_to_llvm_ir(&program)
        .expect("collection clear should lower to LLVM IR");
    let first_clear = ir
        .find("%collection.clear = load")
        .expect("fixture must emit the scalar clear receiver load");
    let after_first_clear = &ir[first_clear..];
    let reset = after_first_clear
        .find("dr_v2_collection_reset_after_cleanup")
        .expect("clear must reset the retained collection allocation");
    let free = after_first_clear
        .find("dr_v1_collection_free")
        .expect("later cleanup must free a collection allocation");
    assert!(
        reset < free,
        "clear must reset before the later scope-exit drop frees the allocation"
    );
    assert!(
        ir.contains("collection.drop.body"),
        "owned clear needs release iteration"
    );
    let detach = ir
        .find("call void @dr_v3_collection_detach_for_cleanup")
        .expect("owned clear must detach storage before release iteration");
    let release = ir[detach..]
        .find("call void @dr_v1_string_release")
        .map(|offset| detach + offset)
        .expect("owned clear must release its detached string values");
    let finish = ir[release..]
        .find("call void @dr_v3_collection_finish_detached_cleanup")
        .map(|offset| release + offset)
        .expect("owned clear must finish detached storage after release iteration");
    assert!(
        detach < release && release < finish,
        "owned clear must become empty before drop glue and preserve destructor refills"
    );
    let placement = scan_alloca_placement(&ir);
    assert!(placement.escaped.is_empty(), "{:#?}", placement.escaped);

    let bytes = doriac::lower_source_to_mir(
        "llvm-bytes-drop.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError
{
    Bytes $contents = read_stdin_bytes();
    write_stdout_bytes($contents);
}
"#,
    )
    .expect("Bytes drop fixture should lower to MIR");
    doriac::codegen_llvm::lower_mir_to_llvm_ir(&bytes)
        .expect("ordinary Bytes drops must keep the free path");
}

/// Guards the scanner itself. The fixture carries the `; preds = ...` comments
/// LLVM actually prints after a label, because a scanner that only recognises
/// bare `label:` lines silently treats a whole function as one block and then
/// reports every module clean.
#[test]
fn detects_an_allocation_emitted_outside_the_entry_block() {
    let ir = "\
define internal void @sample(ptr %0) {
prologue:
  %slot = alloca i64, align 8
  br label %body

body:                                             ; preds = %prologue, %body
  %leaked = alloca i64, align 8
  br i1 true, label %body, label %done

done:                                             ; preds = %body
  ret void
}
";
    let placement = scan_alloca_placement(ir);
    assert_eq!(placement.blocks, 3, "expected three blocks");
    assert_eq!(placement.in_entry, 1, "expected one entry allocation");
    assert_eq!(
        placement.escaped.len(),
        1,
        "expected exactly one escaped allocation, got {:?}",
        placement.escaped
    );
    assert!(
        placement.escaped[0].contains("%leaked"),
        "{}",
        placement.escaped[0]
    );
    assert!(
        placement.escaped[0].contains("'body'"),
        "{}",
        placement.escaped[0]
    );
}
/// Iterating a dictionary must read each element at the position being walked,
/// not look it up by its key.
///
/// Reading by key costs a binary search per element on an ordered dictionary and
/// a linear scan per element on an unordered one, for a value already known to
/// sit at that index. Before this was fixed a `foreach` over a 1024-entry
/// SortedDictionary ran 9.5 times an equivalent walk over a SortedSet, and
/// iterating a plain Dictionary was quadratic in the entry count.
#[test]
fn dictionary_iteration_reads_elements_positionally() {
    let sources: [(&str, &str); 4] = [
        (
            "sorted dictionary, values projection",
            r#"
function main(): void throws Doria\Std\Io\IoError
{
    writable SortedDictionary<int, int> $values = SortedDictionary::from([]);
    for (let writable $index = 0; $index < 8; $index++) { $values->set($index, $index); }
    let writable $total = 0;
    foreach ($values->values as int $value) { $total = $total + $value; }
    echo "{$total}\n";
}
"#,
        ),
        (
            "sorted dictionary, key and value bound",
            r#"
function main(): void throws Doria\Std\Io\IoError
{
    writable SortedDictionary<int, int> $values = SortedDictionary::from([]);
    for (let writable $index = 0; $index < 8; $index++) { $values->set($index, $index); }
    let writable $total = 0;
    foreach ($values as int $key => int $value) { $total = $total + $key + $value; }
    echo "{$total}\n";
}
"#,
        ),
        (
            "unordered dictionary, values projection",
            r#"
function main(): void throws Doria\Std\Io\IoError
{
    writable Dictionary<int, int> $values = [];
    for (let writable $index = 0; $index < 8; $index++) { $values->set($index, $index); }
    let writable $total = 0;
    foreach ($values->values as int $value) { $total = $total + $value; }
    echo "{$total}\n";
}
"#,
        ),
        (
            "string-keyed dictionary with string values",
            r#"
function main(): void throws Doria\Std\Io\IoError
{
    writable Dictionary<string, string> $values = [];
    for (let writable $index = 0; $index < 8; $index++) { $values->set("k{$index}", "v{$index}"); }
    let writable $total = 0;
    foreach ($values->values as string $value) { $total = $total + $value->length; }
    echo "{$total}\n";
}
"#,
        ),
    ];

    for (label, source) in sources {
        let program = doriac::lower_source_to_mir("llvm-test.doria", source)
            .unwrap_or_else(|error| panic!("{label} should lower to MIR: {error:?}"));
        let ir = doriac::codegen_llvm::lower_mir_to_llvm_ir(&program)
            .unwrap_or_else(|error| panic!("{label} should lower to LLVM IR: {error:?}"));

        // The keyed lookup is the defect: it searches for an element already
        // known to sit at the index being walked. Ordered collections read
        // positionally through the runtime; an unordered dictionary lowers to an
        // inline bounds-checked load and emits no call at all, so the absence of
        // the keyed lookup is what both shapes have in common.
        assert!(
            !ir.contains("dr_v1_collection_keyed_get"),
            "{label}: iteration still looks values up by key"
        );
    }
}

/// The key is only read when something consumes it. A values-only walk that
/// still called `key_at` per element would pay for a read it never uses.
#[test]
fn a_values_only_walk_does_not_read_keys() {
    let source = r#"
function main(): void throws Doria\Std\Io\IoError
{
    writable SortedDictionary<int, int> $values = SortedDictionary::from([]);
    for (let writable $index = 0; $index < 8; $index++) { $values->set($index, $index); }
    let writable $total = 0;
    foreach ($values->values as int $value) { $total = $total + $value; }
    echo "{$total}\n";
}
"#;
    let program = doriac::lower_source_to_mir("llvm-test.doria", source).expect("lowers to MIR");
    let ir = doriac::codegen_llvm::lower_mir_to_llvm_ir(&program).expect("lowers to LLVM IR");
    assert!(
        !ir.contains("dr_v2_collection_key_at"),
        "a values-only walk read the key it never uses"
    );
}

/// Collection headers and their value buffers are separate runtime
/// allocations. The LLVM fast path records that fact so a value write inside a
/// loop cannot force invariant header fields to be reloaded on every pass.
#[test]
fn collection_fast_path_carries_disjoint_alias_metadata() {
    let source = r#"
function main(): void
{
    writable bool[] $flags = [true; 8];
    let writable $index = 0;
    while ($index < $flags->length) {
        $flags[$index] = false;
        $index++;
    }
    if ($flags[0]) { panic("flag was not cleared"); }
}
"#;
    let program = doriac::lower_source_to_mir("llvm-test.doria", source).expect("lowers to MIR");
    let ir = doriac::codegen_llvm::lower_mir_to_llvm_ir(&program).expect("lowers to LLVM IR");

    let tagged_line = |name: &str| {
        ir.lines()
            .find(|line| line.contains(name) && line.contains("!tbaa !"))
            .unwrap_or_else(|| panic!("{name} was not tagged with TBAA:\n{ir}"))
    };
    let tag = |line: &str| {
        line.split_once("!tbaa !")
            .map(|(_, id)| {
                id.chars()
                    .take_while(char::is_ascii_digit)
                    .collect::<String>()
            })
            .filter(|id| !id.is_empty())
            .expect("tagged line should carry a TBAA id")
    };

    let header_tag = tag(tagged_line("collection.length = load"));
    assert_eq!(
        tag(tagged_line("collection.values = load")),
        header_tag.clone(),
        "all collection-header fields should share the header alias type"
    );
    assert_eq!(
        tag(tagged_line("collection.membership.index = load")),
        header_tag.clone(),
        "the membership index is part of the collection header"
    );

    let values_tag = tag(tagged_line("collection.value = load"));
    let value_store = ir
        .lines()
        .find(|line| line.trim_start().starts_with("store ") && line.contains("!tbaa !"))
        .unwrap_or_else(|| panic!("collection value store was not tagged with TBAA:\n{ir}"));
    assert_eq!(
        tag(value_store),
        values_tag.clone(),
        "collection element reads and writes should share the values alias type"
    );
    assert_ne!(
        header_tag, values_tag,
        "collection headers and value buffers must remain disjoint alias types"
    );
    assert!(ir.contains("Doria collection header"));
    assert!(ir.contains("Doria collection values"));
}

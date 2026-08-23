use doriac::mir::{
    self, ClosureCaptureOperand, FunctionExpression, FunctionInvocationMode, FunctionTypeId,
    Rvalue, Statement, Terminator, Type,
};

const CAPTURING_SOURCE: &str = r#"
function main(): void
{
    let $base = 41;
    let $answer = fn(int $value) with ($base) => $base + $value;
    echo "{$answer(1)}\n";
}
"#;

const CHECKED_SOURCE: &str = r#"
class Failure implements Error
{
    function __construct(string $message)
    {
    }
}

function main(): void
{
    let $fail = function (): int {
        throw new Failure("failed");
    };

    try {
        $fail();
    } catch (Failure $error) {
        echo "caught\n";
    }
}
"#;

fn capturing_program() -> mir::Program {
    doriac::lower_source_to_mir("stage30d-malformed.doria", CAPTURING_SOURCE)
        .expect("valid capturing closure should lower")
}

fn checked_program() -> mir::Program {
    doriac::lower_source_to_mir("stage30d-malformed-checked.doria", CHECKED_SOURCE)
        .expect("valid checked closure should lower")
}

fn no_capture_program() -> mir::Program {
    doriac::lower_source_to_mir(
        "stage30e-malformed-placement.doria",
        "function main(): void { let $callback = fn() => 42; echo \"{$callback()}\\n\"; }",
    )
    .expect("valid no-capture closure should lower")
}

fn assert_malformed(program: &mir::Program, expected: &str) {
    let error = doriac::mir_validation::validate_program(program)
        .expect_err("malformed closure MIR must stop before backend execution");
    assert!(
        error.message.contains(expected),
        "expected {expected:?}, got {:?}",
        error.message
    );
}

fn closure_function_mut(program: &mut mir::Program) -> &mut mir::Function {
    program
        .functions
        .iter_mut()
        .find(|function| function.closure.is_some())
        .expect("fixture should contain a synthetic closure function")
}

fn creation_mut(program: &mut mir::Program) -> &mut FunctionExpression {
    program
        .functions
        .iter_mut()
        .flat_map(|function| &mut function.blocks)
        .flat_map(|block| &mut block.statements)
        .find_map(|statement| match statement {
            Statement::AssignLocal {
                value: Rvalue::Function(value @ FunctionExpression::Create { .. }),
                ..
            } => Some(value),
            _ => None,
        })
        .expect("fixture should construct a closure")
}

fn indirect_call_mut(program: &mut mir::Program) -> &mut Terminator {
    program
        .functions
        .iter_mut()
        .flat_map(|function| &mut function.blocks)
        .map(|block| &mut block.terminator)
        .find(|terminator| {
            matches!(
                terminator,
                Terminator::IndirectCall { .. } | Terminator::CheckedIndirectCall { .. }
            )
        })
        .expect("fixture should invoke a closure")
}

#[test]
fn rejects_malformed_function_type_and_descriptor_tables() {
    let valid = capturing_program();
    doriac::mir_validation::validate_program(&valid).expect("fixture MIR should validate");

    let mut missing_function_type = valid.clone();
    missing_function_type.function_types.clear();
    assert_malformed(&missing_function_type, "function type#0 does not exist");

    let mut wrong_function_type_slot = valid.clone();
    wrong_function_type_slot.function_types[0].id = FunctionTypeId(7);
    assert_malformed(&wrong_function_type_slot, "function type table slot 0");

    let mut missing_descriptor = valid.clone();
    missing_descriptor.closure_descriptors.clear();
    assert_malformed(&missing_descriptor, "closure descriptor#0 does not exist");

    let mut duplicate_descriptor_identity = valid.clone();
    let mut duplicate = duplicate_descriptor_identity.closure_descriptors[0].clone();
    duplicate.id = mir::ClosureDescriptorId(0);
    duplicate_descriptor_identity
        .closure_descriptors
        .push(duplicate);
    assert_malformed(
        &duplicate_descriptor_identity,
        "closure descriptor table slot 1",
    );

    let mut wrong_descriptor_type = valid.clone();
    wrong_descriptor_type.closure_descriptors[0].function_type = FunctionTypeId(99);
    assert_malformed(&wrong_descriptor_type, "function type#99 does not exist");

    let mut wrong_descriptor_mode = valid.clone();
    wrong_descriptor_mode.closure_descriptors[0].invocation_mode = FunctionInvocationMode::Once;
    assert_malformed(
        &wrong_descriptor_mode,
        "invocation mode disagrees with its function type",
    );
}

#[test]
fn rejects_malformed_environment_layouts_and_release_plans() {
    let valid = capturing_program();

    let mut missing_layout = valid.clone();
    missing_layout.closure_environment_layouts.clear();
    assert_malformed(
        &missing_layout,
        "closure environment layout#0 does not exist",
    );

    let mut duplicate_logical_index = valid.clone();
    duplicate_logical_index.closure_environment_layouts[0].fields[0].logical_index = 1;
    assert_malformed(
        &duplicate_logical_index,
        "invalid field identity or ordering",
    );

    let mut wrong_release_order = valid.clone();
    wrong_release_order.closure_environment_layouts[0].logical_release_order = vec![];
    assert_malformed(&wrong_release_order, "reverse logical release order");

    let mut raw_environment_field = valid.clone();
    raw_environment_field.closure_environment_layouts[0].fields[0].ty =
        Type::ClosureEnvironment(None);
    assert_malformed(
        &raw_environment_field,
        "cannot contain a raw environment handle",
    );
}

#[test]
fn rejects_malformed_native_environment_placement() {
    let mut missing_placement = capturing_program();
    missing_placement.closure_descriptors[0].environment_placement =
        mir::ClosureEnvironmentPlacement::None;
    assert_malformed(
        &missing_placement,
        "has an environment without native placement",
    );

    let mut invented_placement = no_capture_program();
    invented_placement.closure_descriptors[0].environment_placement =
        mir::ClosureEnvironmentPlacement::Stack;
    assert_malformed(
        &invented_placement,
        "has native environment placement without a layout",
    );
}

#[test]
fn rejects_malformed_synthetic_closure_functions() {
    let valid = capturing_program();

    let mut missing_hidden_parameter = valid.clone();
    closure_function_mut(&mut missing_hidden_parameter)
        .params
        .remove(0);
    assert_malformed(
        &missing_hidden_parameter,
        "parameter modes do not match its parameters",
    );

    let mut wrong_hidden_type = valid.clone();
    let function = closure_function_mut(&mut wrong_hidden_type);
    let hidden = function.closure.as_ref().unwrap().hidden_environment;
    function.locals[hidden.0].ty = Type::Scalar(mir::ScalarType::Integer(
        doriac::numeric::IntegerType::Int64,
    ));
    assert_malformed(&wrong_hidden_type, "invalid hidden environment parameter");

    let mut missing_capture_binding = valid.clone();
    closure_function_mut(&mut missing_capture_binding)
        .closure
        .as_mut()
        .unwrap()
        .capture_locals
        .clear();
    assert_malformed(
        &missing_capture_binding,
        "capture bindings do not match its environment",
    );

    let mut descriptor_points_to_nonclosure = valid.clone();
    descriptor_points_to_nonclosure.closure_descriptors[0].entry_function =
        descriptor_points_to_nonclosure.entry;
    assert_malformed(
        &descriptor_points_to_nonclosure,
        "closure descriptor entry function lacks closure metadata",
    );
}

#[test]
fn rejects_malformed_closure_construction_plans() {
    let valid = capturing_program();

    let mut wrong_capture_count = valid.clone();
    let FunctionExpression::Create { captures, .. } = creation_mut(&mut wrong_capture_count) else {
        unreachable!()
    };
    captures.clear();
    assert_malformed(
        &wrong_capture_count,
        "capture count does not match its layout",
    );

    let mut writable_claim_from_readonly = valid.clone();
    let FunctionExpression::Create { captures, .. } =
        creation_mut(&mut writable_claim_from_readonly)
    else {
        unreachable!()
    };
    let ClosureCaptureOperand::BorrowLocal { writable, .. } = &mut captures[0] else {
        panic!("fixture should borrow its capture")
    };
    *writable = true;
    assert_malformed(
        &writable_claim_from_readonly,
        "borrow capture has incompatible type or access",
    );

    let mut owned_operand_for_borrow_field = valid.clone();
    let FunctionExpression::Create { captures, .. } =
        creation_mut(&mut owned_operand_for_borrow_field)
    else {
        unreachable!()
    };
    let local = match captures[0] {
        ClosureCaptureOperand::BorrowLocal { local, .. } => local,
        _ => panic!("fixture should borrow its capture"),
    };
    captures[0] = ClosureCaptureOperand::CopyValue(Rvalue::Value(mir::ValueExpression::Integer(
        mir::IntegerExpression::Use {
            operand: mir::Operand::Local(local),
            ty: doriac::numeric::IntegerType::Int64,
        },
    )));
    assert_malformed(
        &owned_operand_for_borrow_field,
        "does not match environment storage",
    );
}

#[test]
fn rejects_malformed_indirect_call_plans() {
    let valid = capturing_program();

    let mut wrong_function_type = valid.clone();
    let Terminator::IndirectCall { function_type, .. } =
        indirect_call_mut(&mut wrong_function_type)
    else {
        unreachable!()
    };
    *function_type = FunctionTypeId(99);
    assert_malformed(&wrong_function_type, "function type#99 does not exist");

    let mut wrong_arity = valid.clone();
    let Terminator::IndirectCall { args, .. } = indirect_call_mut(&mut wrong_arity) else {
        unreachable!()
    };
    args.clear();
    assert_malformed(&wrong_arity, "expects 1 arguments, got 0");

    let mut once_without_consumption = valid.clone();
    once_without_consumption.function_types[0].invocation_mode = FunctionInvocationMode::Once;
    once_without_consumption.closure_descriptors[0].invocation_mode = FunctionInvocationMode::Once;
    let Terminator::IndirectCall {
        callee,
        invocation_mode,
        ..
    } = indirect_call_mut(&mut once_without_consumption)
    else {
        unreachable!()
    };
    *invocation_mode = FunctionInvocationMode::Once;
    let FunctionExpression::Local { transfer, .. } = callee else {
        panic!("fixture should invoke a local carrier")
    };
    *transfer = false;
    assert_malformed(
        &once_without_consumption,
        "once indirect call does not consume its function carrier",
    );

    let mut wrong_result_shape = valid.clone();
    let Terminator::IndirectCall { result, .. } = indirect_call_mut(&mut wrong_result_shape) else {
        unreachable!()
    };
    *result = None;
    assert_malformed(&wrong_result_shape, "wrong result-slot shape");
}

#[test]
fn rejects_checked_and_unchecked_indirect_call_mismatches() {
    let checked = checked_program();
    doriac::mir_validation::validate_program(&checked)
        .expect("checked fixture MIR should validate");

    let mut checked_nonthrowing = checked.clone();
    checked_nonthrowing.function_types[0]
        .checked_effects
        .clear();
    assert_malformed(
        &checked_nonthrowing,
        "checked indirect call uses a nonthrowing function type",
    );

    let mut merged_edges = checked.clone();
    let Terminator::CheckedIndirectCall {
        success, failure, ..
    } = indirect_call_mut(&mut merged_edges)
    else {
        unreachable!()
    };
    *failure = *success;
    assert_malformed(
        &merged_edges,
        "checked indirect call success and error edges are identical",
    );

    let mut wrong_error_slot = checked.clone();
    let error = {
        let Terminator::CheckedIndirectCall { error, .. } =
            indirect_call_mut(&mut wrong_error_slot)
        else {
            unreachable!()
        };
        *error
    };
    let owner = wrong_error_slot
        .functions
        .iter_mut()
        .find(|function| function.locals.get(error.0).is_some())
        .expect("call owner should contain the Error slot");
    owner.locals[error.0].ty = Type::Scalar(mir::ScalarType::Integer(
        doriac::numeric::IntegerType::Int64,
    ));
    assert_malformed(&wrong_error_slot, "incompatible Error slot");
}

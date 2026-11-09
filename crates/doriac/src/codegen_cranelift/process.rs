//! Decision 0110 child processes: every `Child` intrinsic is one call to the
//! shared checked transport `dr_v1_checked_process`, with its operands packed
//! as words in a stack array and the ordinary checked-I/O error outputs.
use super::*;
use crate::compiler_known_stream::ProcessFunction;

#[allow(clippy::too_many_arguments)]
pub(super) fn lower(
    builder: &mut FunctionBuilder,
    operation: &mir::CheckedIoOperation,
    result: Option<mir::LocalId>,
    error: mir::LocalId,
    success: Block,
    failure: Block,
    span: crate::source::Span,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let mir::CheckedIoOperation::Process {
        operation: function,
        process,
        arguments,
        ..
    } = operation
    else {
        unreachable!("process lowering receives a process operation");
    };
    set_active_panic_site(builder, span, resources);
    let pointer = resources.module.target_config().pointer_type();
    let zero = builder.ins().iconst(pointer, 0);
    let child = match process {
        Some(process) => builder.ins().stack_load(
            pointer,
            pointer,
            local_slot(resources.local_slots, *process)?,
            0,
        ),
        None => zero,
    };
    let words = operand_words(builder, *function, arguments, resources)?;
    let operands = builder.create_sized_stack_slot(StackSlotData::new(
        StackSlotKind::ExplicitSlot,
        (words.len().max(1) * 8) as u32,
        3,
    ));
    for (index, word) in words.iter().enumerate() {
        builder
            .ins()
            .stack_store(types::I64, *word, operands, (index * 8) as i32);
    }
    let operands = builder.ins().stack_addr(pointer, operands, 0);
    let count = builder.ins().iconst(pointer, words.len() as i64);
    let mut slots = checked_io_output_slots(builder, pointer);
    slots[1] =
        builder.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 8, 3));
    let word_zero = builder.ins().iconst(types::I64, 0);
    builder
        .ins()
        .stack_store(types::I64, word_zero, slots[1], 0);
    let out = slots
        .iter()
        .map(|slot| builder.ins().stack_addr(pointer, *slot, 0))
        .collect::<Vec<_>>();
    let tag = builder.ins().iconst(types::I8, i64::from(function.tag()));
    let status = runtime_call(
        builder,
        crate::native_abi::CHECKED_PROCESS,
        &[
            pointer,
            types::I8,
            pointer,
            pointer,
            pointer,
            pointer,
            pointer,
            pointer,
            pointer,
            pointer,
            pointer,
        ],
        Some(types::I8),
        &[
            resources.current_frame,
            tag,
            child,
            operands,
            count,
            out[1],
            out[2],
            out[3],
            out[4],
            out[7],
            out[8],
        ],
        resources,
    )?
    .ok_or_else(|| backend_failure("process operation produced no status"))?;
    finish_checked_resource(
        builder, operation, result, error, success, failure, status, slots, resources,
    )
}

/// The operand words in `dr_v1_checked_process` order.
fn operand_words(
    builder: &mut FunctionBuilder,
    function: ProcessFunction,
    arguments: &[mir::LocalId],
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Vec<Value>, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let word = |builder: &mut FunctionBuilder, value: Value| {
        if pointer == types::I64 {
            value
        } else {
            builder.ins().uextend(types::I64, value)
        }
    };
    let mut words = Vec::new();
    for (position, argument) in arguments.iter().enumerate() {
        let slot = local_slot(resources.local_slots, *argument)?;
        let ty = local_definition(resources.program, resources.function_id, *argument)?.ty;
        match ty {
            mir::Type::String | mir::Type::Collection(_) => {
                let value = builder.ins().stack_load(pointer, pointer, slot, 0);
                words.push(word(builder, value));
            }
            mir::Type::NullableString => {
                let present = builder.ins().stack_load(pointer, pointer, slot, 0);
                let value =
                    builder
                        .ins()
                        .stack_load(pointer, pointer, slot, pointer.bytes() as i32);
                let zero = builder.ins().iconst(pointer, 0);
                let value = builder.ins().select(present, value, zero);
                words.push(word(builder, value));
            }
            mir::Type::Scalar(mir::ScalarType::Bool) => {
                let value = builder.ins().stack_load(pointer, types::I8, slot, 0);
                words.push(builder.ins().uextend(types::I64, value));
            }
            mir::Type::Scalar(mir::ScalarType::Enum(_)) => {
                let value = builder.ins().stack_load(pointer, types::I32, slot, 0);
                words.push(builder.ins().uextend(types::I64, value));
            }
            mir::Type::Scalar(mir::ScalarType::Integer(_)) => {
                words.push(builder.ins().stack_load(types::I64, types::I64, slot, 0));
            }
            mir::Type::Scalar(mir::ScalarType::Inline(
                crate::inline_value::InlineValueType::Deadline,
            )) => {
                for offset in [0, 8] {
                    words.push(
                        builder
                            .ins()
                            .stack_load(types::I64, types::I64, slot, offset),
                    );
                }
            }
            _ => {
                return Err(malformed_mir(format!(
                    "process operation {function:?} argument {position} has type {ty}"
                )))
            }
        }
    }
    Ok(words)
}

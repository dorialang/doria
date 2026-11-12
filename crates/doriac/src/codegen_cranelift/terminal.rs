//! The two terminal primitives (Decision 0140) as runtime calls.

use super::*;

pub(super) fn lower_call(
    builder: &mut FunctionBuilder,
    call: mir::TerminalCall,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let result = match call {
        mir::TerminalCall::Operate {
            operation,
            first,
            second,
            text,
        } => {
            let mut words = [operation, first, second].map(|_| None);
            for (word, local) in words.iter_mut().zip([operation, first, second]) {
                *word = Some(builder.ins().stack_load(
                    pointer,
                    types::I64,
                    local_slot(resources.local_slots, local)?,
                    0,
                ));
            }
            let [operation, first, second] = words.map(|word| word.expect("loaded word"));
            let operation = builder.ins().ireduce(types::I32, operation);
            let text = load_lowered_from_stack(
                builder,
                resources.program,
                mir::Type::String,
                local_slot(resources.local_slots, text)?,
                pointer,
            )
            .single()?;
            runtime_call(
                builder,
                crate::native_abi::TERMINAL_OPERATE,
                &[types::I32, types::I64, types::I64, pointer],
                Some(types::I64),
                &[operation, first, second, text],
                resources,
            )?
        }
        mir::TerminalCall::AwaitEvent {
            generation,
            wait,
            cancellation,
        } => {
            let generation = builder.ins().stack_load(
                pointer,
                types::I64,
                local_slot(resources.local_slots, generation)?,
                0,
            );
            let generation = builder.ins().ireduce(types::I32, generation);
            let wait = readiness_set::wait_words(builder, wait, resources)?;
            let cancellation_ty =
                local_definition(resources.program, resources.function_id, cancellation)?.ty;
            let (present, state) = load_lowered_from_stack(
                builder,
                resources.program,
                cancellation_ty,
                local_slot(resources.local_slots, cancellation)?,
                pointer,
            )
            .nullable()?;
            let zero = builder.ins().iconst(pointer, 0);
            let cancellation = builder.ins().select(present, state, zero);
            runtime_call(
                builder,
                crate::native_abi::TERMINAL_AWAIT_EVENT,
                &[types::I32, types::I8, types::I64, types::I64, pointer],
                Some(types::I64),
                &[generation, wait[0], wait[1], wait[2], cancellation],
                resources,
            )?
        }
    };
    result.ok_or_else(|| backend_failure("terminal call produced no result"))
}

//! The two terminal primitives (Decision 0140) as runtime calls.

use super::*;

impl<'ctx> FunctionLowerer<'ctx, '_> {
    pub(super) fn lower_terminal_call(
        &mut self,
        call: mir::TerminalCall,
    ) -> Result<IntValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let byte = self.context.i8_type();
        let selector = self.context.i32_type();
        let integer = self.context.i64_type();
        let result = match call {
            mir::TerminalCall::Operate {
                operation,
                first,
                second,
                text,
            } => {
                let mut words = Vec::with_capacity(3);
                for local in [operation, first, second] {
                    words.push(
                        build(self.builder.build_load(
                            integer,
                            local_slot(&self.local_slots, local)?,
                            "terminal.word",
                        ))?
                        .into_int_value(),
                    );
                }
                let operation = build(self.builder.build_int_truncate(
                    words[0],
                    selector,
                    "terminal.operation",
                ))?;
                let text = build(self.builder.build_load(
                    pointer,
                    local_slot(&self.local_slots, text)?,
                    "terminal.text",
                ))?;
                self.call_runtime(
                    crate::native_abi::TERMINAL_OPERATE,
                    &[
                        selector.into(),
                        integer.into(),
                        integer.into(),
                        pointer.into(),
                    ],
                    Some(integer.into()),
                    &[
                        operation.into(),
                        words[1].into(),
                        words[2].into(),
                        text.into(),
                    ],
                )?
            }
            mir::TerminalCall::AwaitEvent {
                generation,
                wait,
                cancellation,
            } => {
                let generation = build(self.builder.build_load(
                    integer,
                    local_slot(&self.local_slots, generation)?,
                    "terminal.generation",
                ))?
                .into_int_value();
                let generation = build(self.builder.build_int_truncate(
                    generation,
                    selector,
                    "terminal.generation-word",
                ))?;
                let [kind, low, high] = self.wait_words(wait)?;
                let cancellation_ty = local_in(self.function, cancellation)?.ty;
                let (state, _) = self
                    .load_managed_inline_claim(
                        local_slot(&self.local_slots, cancellation)?,
                        cancellation_ty,
                    )?
                    .ok_or_else(|| {
                        malformed_mir("terminal cancellation has no managed inline carrier")
                    })?;
                let cancellation = build(self.builder.build_int_to_ptr(
                    state,
                    pointer,
                    "terminal.cancellation",
                ))?;
                self.call_runtime(
                    crate::native_abi::TERMINAL_AWAIT_EVENT,
                    &[
                        selector.into(),
                        byte.into(),
                        integer.into(),
                        integer.into(),
                        pointer.into(),
                    ],
                    Some(integer.into()),
                    &[
                        generation.into(),
                        kind.into(),
                        low.into(),
                        high.into(),
                        cancellation.into(),
                    ],
                )?
            }
        };
        Ok(result
            .ok_or_else(|| backend_failure("terminal call produced no result"))?
            .into_int_value())
    }
}

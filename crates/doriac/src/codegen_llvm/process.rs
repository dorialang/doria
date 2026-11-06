//! Decision 0110 child processes: every `Child` intrinsic is one call to the
//! shared checked transport `dr_v1_checked_process`, with its operands packed
//! as words in an entry-block array and the ordinary checked-I/O outputs.
use super::*;
use crate::compiler_known_stream::ProcessFunction;

impl<'ctx> FunctionLowerer<'ctx, '_> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn lower_checked_process(
        &mut self,
        operation: &mir::CheckedIoOperation,
        result: Option<mir::LocalId>,
        error: mir::LocalId,
        success: BasicBlock<'ctx>,
        failure: BasicBlock<'ctx>,
        span: crate::source::Span,
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
        self.set_active_panic_site(span)?;
        let pointer = self.context.ptr_type(AddressSpace::default());
        let word = self.context.ptr_sized_int_type(self.target_data, None);
        let byte = self.context.i8_type();
        let int = self.context.i64_type();
        let child = match process {
            Some(process) => build(self.builder.build_load(
                pointer,
                local_slot(&self.local_slots, *process)?,
                "process.child",
            ))?
            .into_pointer_value(),
            None => pointer.const_null(),
        };
        let words = self.process_operand_words(*function, arguments)?;
        let array_type = int.array_type(words.len().max(1) as u32);
        let operands = self.entry_alloca(array_type, "process.operands")?;
        for (index, value) in words.iter().enumerate() {
            let element = unsafe {
                build(self.builder.build_in_bounds_gep(
                    array_type,
                    operands,
                    &[int.const_zero(), int.const_int(index as u64, false)],
                    "process.operand",
                ))?
            };
            build(self.builder.build_store(element, *value))?;
        }
        let [tag_slot, _, message_slot, path_slot, system_slot, valid_slot, invalid_slot, transferred_slot, meta_slot] =
            self.checked_io_output_slots()?;
        let value_slot = self.entry_alloca(int, "process.value")?;
        build(self.builder.build_store(value_slot, int.const_zero()))?;
        let status = self
            .call_runtime(
                crate::native_abi::CHECKED_PROCESS,
                &[
                    pointer.into(),
                    byte.into(),
                    pointer.into(),
                    pointer.into(),
                    word.into(),
                    pointer.into(),
                    pointer.into(),
                    pointer.into(),
                    pointer.into(),
                    pointer.into(),
                    pointer.into(),
                ],
                Some(byte.into()),
                &[
                    self.current_frame.into(),
                    byte.const_int(u64::from(function.tag()), false).into(),
                    child.into(),
                    operands.into(),
                    word.const_int(words.len() as u64, false).into(),
                    value_slot.into(),
                    message_slot.into(),
                    path_slot.into(),
                    system_slot.into(),
                    transferred_slot.into(),
                    meta_slot.into(),
                ],
            )?
            .ok_or_else(|| backend_failure("process operation produced no status"))?
            .into_int_value();
        self.finish_checked_resource(
            operation,
            result,
            error,
            success,
            failure,
            status,
            [
                tag_slot,
                value_slot,
                message_slot,
                path_slot,
                system_slot,
                valid_slot,
                invalid_slot,
                transferred_slot,
                meta_slot,
            ],
        )
    }

    /// The operand words in `dr_v1_checked_process` order.
    fn process_operand_words(
        &mut self,
        function: ProcessFunction,
        arguments: &[mir::LocalId],
    ) -> Result<Vec<IntValue<'ctx>>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let int = self.context.i64_type();
        let mut words = Vec::new();
        for (position, argument) in arguments.iter().enumerate() {
            let slot = local_slot(&self.local_slots, *argument)?;
            let ty = local_in(self.function, *argument)?.ty;
            match ty {
                mir::Type::String | mir::Type::Collection(_) => {
                    let value = build(self.builder.build_load(pointer, slot, "process.pointer"))?
                        .into_pointer_value();
                    words.push(build(self.builder.build_ptr_to_int(
                        value,
                        int,
                        "process.pointer-word",
                    ))?);
                }
                mir::Type::NullableString => {
                    let word = self.context.ptr_sized_int_type(self.target_data, None);
                    let parts = self
                        .context
                        .struct_type(&[word.into(), pointer.into()], false);
                    let present = build(self.builder.build_struct_gep(parts, slot, 0, "present"))?;
                    let present = build(self.builder.build_load(word, present, "process.present"))?
                        .into_int_value();
                    let value = build(self.builder.build_struct_gep(parts, slot, 1, "value"))?;
                    let value = build(self.builder.build_load(pointer, value, "process.string"))?
                        .into_pointer_value();
                    let present = build(self.builder.build_int_compare(
                        IntPredicate::NE,
                        present,
                        word.const_zero(),
                        "process.some",
                    ))?;
                    let value = build(self.builder.build_select(
                        present,
                        value,
                        pointer.const_null(),
                        "process.directory",
                    ))?
                    .into_pointer_value();
                    words.push(build(self.builder.build_ptr_to_int(
                        value,
                        int,
                        "process.directory-word",
                    ))?);
                }
                mir::Type::Scalar(mir::ScalarType::Bool) => {
                    let value = build(self.builder.build_load(
                        self.context.i8_type(),
                        slot,
                        "process.flag",
                    ))?
                    .into_int_value();
                    words.push(build(self.builder.build_int_z_extend(
                        value,
                        int,
                        "process.flag-word",
                    ))?);
                }
                mir::Type::Scalar(mir::ScalarType::Enum(_)) => {
                    let value = build(self.builder.build_load(
                        self.context.i32_type(),
                        slot,
                        "process.cleanup",
                    ))?
                    .into_int_value();
                    words.push(build(self.builder.build_int_z_extend(
                        value,
                        int,
                        "process.cleanup-word",
                    ))?);
                }
                mir::Type::Scalar(mir::ScalarType::Integer(_)) => {
                    words.push(
                        build(self.builder.build_load(int, slot, "process.integer"))?
                            .into_int_value(),
                    );
                }
                mir::Type::Scalar(mir::ScalarType::Inline(
                    crate::inline_value::InlineValueType::Deadline,
                )) => {
                    let parts = self.context.struct_type(&[int.into(), int.into()], false);
                    for index in 0..2 {
                        let part = build(
                            self.builder
                                .build_struct_gep(parts, slot, index, "deadline"),
                        )?;
                        words.push(
                            build(self.builder.build_load(int, part, "process.deadline"))?
                                .into_int_value(),
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
}

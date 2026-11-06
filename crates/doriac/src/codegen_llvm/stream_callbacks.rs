//! C-ABI adapters around ordinary checked Doria interface entries. Context,
//! requests, regions and result homes are borrowed only for the current call.

use super::*;
use crate::compiler_known_stream::StreamOperation as Op;

const OPERATIONS: [Op; 7] = [
    Op::ReadinessSource,
    Op::Read,
    Op::Write,
    Op::PrepareRead,
    Op::PrepareWrite,
    Op::CompleteRead,
    Op::CompleteWrite,
];

pub(super) struct Callback<'ctx> {
    plan: mir::StreamCallbackPlan,
    function: FunctionValue<'ctx>,
}

pub(super) enum CallTarget<'ctx> {
    Static(FunctionValue<'ctx>),
    Dynamic(PointerValue<'ctx>),
}

fn symbol(receiver: mir::Type, operation: Op) -> Result<String, BackendError> {
    let receiver = match receiver {
        mir::Type::Class(id) => format!("class_{}", id.0),
        mir::Type::Scalar(mir::ScalarType::Inline(ty)) => format!(
            "inline_{}",
            ty.nominal_class()
                .ok_or_else(|| malformed_mir("inline stream callback has no nominal identity"))?
                .0,
        ),
        mir::Type::Interface(id) => format!("interface_{}", id.0),
        _ => return Err(malformed_mir("stream callback receiver is not nominal")),
    };
    Ok(format!("__doria_stream_callback_{receiver}_{operation:?}"))
}

fn pending(operation: Op) -> bool {
    matches!(
        operation,
        Op::PrepareRead | Op::PrepareWrite | Op::CompleteRead | Op::CompleteWrite
    )
}

fn argument_count(operation: Op) -> usize {
    if operation == Op::ReadinessSource {
        5
    } else if pending(operation) {
        7
    } else {
        6
    }
}

fn signature<'ctx>(
    context: &'ctx Context,
    target_data: &TargetData,
    operation: Op,
) -> inkwell::types::FunctionType<'ctx> {
    let pointer = context.ptr_type(AddressSpace::default());
    let mut params = vec![pointer.into(); argument_count(operation)];
    if operation != Op::ReadinessSource {
        params[3 + usize::from(pending(operation))] =
            context.ptr_sized_int_type(target_data, None).into();
    }
    context.i8_type().fn_type(&params, false)
}

pub(super) fn declare<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    target_data: &TargetData,
    program: &mir::Program,
) -> Result<Vec<Callback<'ctx>>, BackendError> {
    let mut callbacks: Vec<Callback<'ctx>> = Vec::new();
    for function in &program.functions {
        for block in &function.blocks {
            let mir::Terminator::CheckedIo {
                operation: mir::CheckedIoOperation::RegisterReadinessStream { stream, .. },
                ..
            } = &block.terminator
            else {
                continue;
            };
            let receiver = local_in(function, *stream)?.ty;
            for operation in OPERATIONS {
                let Some(plan) = program.stream_callback(receiver, operation) else {
                    continue;
                };
                if callbacks
                    .iter()
                    .any(|item| item.plan.receiver == receiver && item.plan.operation == operation)
                {
                    continue;
                }
                let function = module.add_function(
                    &symbol(receiver, operation)?,
                    signature(context, target_data, operation),
                    Some(Linkage::Internal),
                );
                let zeroext = inkwell::attributes::Attribute::get_named_enum_kind_id("zeroext");
                function.add_attribute(
                    AttributeLoc::Return,
                    context.create_enum_attribute(zeroext, 0),
                );
                callbacks.push(Callback { plan, function });
            }
        }
    }
    Ok(callbacks)
}

pub(super) fn address<'ctx>(
    module: &Module<'ctx>,
    program: &mir::Program,
    receiver: mir::Type,
    operation: Op,
) -> Result<Option<PointerValue<'ctx>>, BackendError> {
    if !OPERATIONS.contains(&operation) || program.stream_callback(receiver, operation).is_none() {
        return Ok(None);
    }
    module
        .get_function(&symbol(receiver, operation)?)
        .map(|function| Some(function.as_global_value().as_pointer_value()))
        .ok_or_else(|| malformed_mir("registered stream callback is not declared"))
}

/// Used by both transfer thunks and ordinary registration getter calls. Method
/// views are borrowed only; the original owned receiver descriptor is untouched.
#[allow(clippy::too_many_arguments)]
pub(super) fn resolve_target<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    target_data: &TargetData,
    functions: &[FunctionValue<'ctx>],
    vtables: &[GlobalValue<'ctx>],
    plan: mir::StreamCallbackPlan,
    descriptor: PointerValue<'ctx>,
) -> Result<(CallTarget<'ctx>, PointerValue<'ctx>), BackendError> {
    let pointer = context.ptr_type(AddressSpace::default());
    let word = context.ptr_sized_int_type(target_data, None);
    let vtable = match plan.target {
        mir::StreamCallbackTarget::Static { vtable, entry } => {
            let vtable = vtables
                .get(vtable.0)
                .ok_or_else(|| malformed_mir("stream callback vtable is not declared"))?
                .as_pointer_value();
            let function = *functions
                .get(entry.0)
                .ok_or_else(|| malformed_mir("stream callback entry is not declared"))?;
            return Ok((CallTarget::Static(function), vtable));
        }
        mir::StreamCallbackTarget::ClassDescriptor => {
            let views = build(unsafe {
                builder.build_gep(
                    pointer,
                    descriptor,
                    &[word.const_int(
                        u64::from(crate::native_abi::CLASS_DESCRIPTOR_INTERFACE_WORD),
                        false,
                    )],
                    "stream.class.views",
                )
            })?;
            let views = build(builder.build_load(pointer, views, "stream.class.interfaces"))?
                .into_pointer_value();
            let slot = build(unsafe {
                builder.build_gep(
                    pointer,
                    views,
                    &[word.const_int(plan.interface.0 as u64, false)],
                    "stream.interface.slot",
                )
            })?;
            build(builder.build_load(pointer, slot, "stream.interface.vtable"))?
                .into_pointer_value()
        }
        mir::StreamCallbackTarget::InterfaceDescriptor => descriptor,
    };
    let slot = build(unsafe {
        builder.build_gep(
            pointer,
            vtable,
            &[word.const_int(
                (crate::native_abi::INTERFACE_VTABLE_HEADER_WORDS + plan.slot) as u64,
                false,
            )],
            "stream.method.slot",
        )
    })?;
    let entry =
        build(builder.build_load(pointer, slot, "stream.method.entry"))?.into_pointer_value();
    Ok((CallTarget::Dynamic(entry), vtable))
}

pub(super) fn define<'ctx>(
    context: &'ctx Context,
    target_data: &TargetData,
    program: &mir::Program,
    callbacks: &[Callback<'ctx>],
    declarations: &DeclaredProgram<'ctx>,
) -> Result<(), BackendError> {
    for callback in callbacks {
        let builder = context.create_builder();
        let entry = context.append_basic_block(callback.function, "entry");
        builder.position_at_end(entry);
        Emitter {
            context,
            builder: &builder,
            target_data,
            program,
            function: callback.function,
        }
        .emit(callback.plan, declarations)?;
    }
    Ok(())
}

struct Emitter<'a, 'ctx> {
    context: &'ctx Context,
    builder: &'a Builder<'ctx>,
    target_data: &'a TargetData,
    program: &'a mir::Program,
    function: FunctionValue<'ctx>,
}

impl<'ctx> Emitter<'_, 'ctx> {
    fn offset(
        &self,
        address: PointerValue<'ctx>,
        bytes: u64,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        build(unsafe {
            self.builder.build_gep(
                self.context.i8_type(),
                address,
                &[self.context.i64_type().const_int(bytes, false)],
                "stream.field",
            )
        })
    }

    fn load_pointer(
        &self,
        address: PointerValue<'ctx>,
        word: u32,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let offset = u64::from(word) * u64::from(self.target_data.get_pointer_byte_size(None));
        Ok(build(self.builder.build_load(
            self.context.ptr_type(AddressSpace::default()),
            self.offset(address, offset)?,
            "stream.pointer",
        ))?
        .into_pointer_value())
    }

    fn load_word(
        &self,
        address: PointerValue<'ctx>,
        word: u32,
    ) -> Result<IntValue<'ctx>, BackendError> {
        let offset = u64::from(word) * u64::from(self.target_data.get_pointer_byte_size(None));
        Ok(build(self.builder.build_load(
            self.context.ptr_sized_int_type(self.target_data, None),
            self.offset(address, offset)?,
            "stream.word",
        ))?
        .into_int_value())
    }

    fn return_status(&self, status: u64) -> Result<(), BackendError> {
        build(
            self.builder
                .build_return(Some(&self.context.i8_type().const_int(status, false))),
        )?;
        Ok(())
    }

    fn block(&self, name: &str) -> BasicBlock<'ctx> {
        self.context.append_basic_block(self.function, name)
    }

    fn emit(
        &self,
        plan: mir::StreamCallbackPlan,
        declarations: &DeclaredProgram<'ctx>,
    ) -> Result<(), BackendError> {
        let params = self.function.get_params();
        let resolver = plan.operation == Op::ReadinessSource;
        let base = usize::from(pending(plan.operation));
        let frame = params[0].into_pointer_value();
        let context = params[1].into_pointer_value();
        let pointer = self.context.ptr_type(AddressSpace::default());
        let word = self.context.ptr_sized_int_type(self.target_data, None);
        let data = if resolver {
            pointer.const_null()
        } else {
            params[2 + base].into_pointer_value()
        };
        // The C signature spells size_t, which is an integer even though it has
        // the same register width as a pointer in Cranelift.
        let length = if resolver {
            word.const_zero()
        } else {
            params[3 + base].into_int_value()
        };
        let outcome = params[if resolver { 2 } else { 4 + base }].into_pointer_value();
        let error = params[if resolver { 4 } else { 5 + base }].into_pointer_value();
        let bytes = u64::from(self.target_data.get_pointer_byte_size(None));
        build(self.builder.build_store(outcome, word.const_zero()))?;
        if !resolver {
            build(self.builder.build_store(
                self.offset(outcome, bytes)?,
                self.context.i8_type().const_zero(),
            ))?;
            build(self.builder.build_store(
                self.offset(outcome, bytes + 1)?,
                self.context.i8_type().const_zero(),
            ))?;
        }
        let object = self.load_pointer(context, 0)?;
        let mut descriptor = self.load_pointer(context, 1)?;
        if resolver
            && matches!(
                plan.receiver,
                mir::Type::Class(_) | mir::Type::Scalar(mir::ScalarType::Inline(_))
            )
        {
            descriptor = self.load_pointer(
                descriptor,
                crate::native_abi::INTERFACE_VTABLE_CLASS_DESCRIPTOR_WORD,
            )?;
        }
        let contract = function_type_in(self.program, plan.signature)?;
        let abi = native_closure_abi::NativeCallableSignaturePlan::interface_entry(contract);
        let home = build(
            self.builder
                .build_alloca(word.array_type(2), "stream.receiver.home"),
        )?;
        build(self.builder.build_store(home, context))?;
        build(self.builder.build_store(
            self.offset(home, bytes)?,
            word.const_int(
                native_closure_abi::borrow_home_type_key(if resolver {
                    mir::Type::Interface(plan.interface)
                } else {
                    plan.receiver
                }),
                false,
            ),
        ))?;
        let result = match contract.return_type {
            mir::ReturnType::Void => None,
            mir::ReturnType::Value(ty) => {
                let slot = build(self.builder.build_alloca(
                    llvm_type(self.context, self.target_data, self.program, ty),
                    "stream.result",
                ))?;
                let layout = native_closure_abi::value_layout(self.program, ty, bytes as u32);
                slot.as_instruction_value()
                    .ok_or_else(|| backend_failure("stream result alloca has no instruction"))?
                    .set_alignment(layout.align)
                    .map_err(|error| backend_failure(error.to_string()))?;
                Some(slot)
            }
        };
        let mut values = Vec::<BasicMetadataValueEnum<'ctx>>::new();
        for input in &abi.hidden_inputs {
            values.push(match *input {
                native_closure_abi::NativeCallableHiddenInput::CurrentFrame => frame.into(),
                native_closure_abi::NativeCallableHiddenInput::ResultOut => result
                    .ok_or_else(|| malformed_mir("stream callback has no result home"))?
                    .into(),
                native_closure_abi::NativeCallableHiddenInput::ErrorOut => error.into(),
                native_closure_abi::NativeCallableHiddenInput::ParameterBorrowHome(0) => {
                    home.into()
                }
                native_closure_abi::NativeCallableHiddenInput::ResultBorrowHomeOut if resolver => {
                    let slot = build(
                        self.builder
                            .build_alloca(word.array_type(2), "stream.result.home"),
                    )?;
                    build(
                        self.builder
                            .build_store(slot, word.array_type(2).const_zero()),
                    )?;
                    slot.into()
                }
                native_closure_abi::NativeCallableHiddenInput::ParameterBorrowHome(parameter) => {
                    let ty = contract.parameters[parameter].ty;
                    let carrier = match ty {
                        mir::Type::Class(_) if base == 1 && parameter == 1 => {
                            let slot =
                                build(self.builder.build_alloca(word, "stream.request.carrier"))?;
                            build(self.builder.build_store(slot, params[2]))?;
                            slot
                        }
                        mir::Type::ByteRegion(_) => {
                            let slot = build(
                                self.builder
                                    .build_alloca(word.array_type(2), "stream.region.carrier"),
                            )?;
                            build(self.builder.build_store(slot, data))?;
                            build(self.builder.build_store(self.offset(slot, bytes)?, length))?;
                            slot
                        }
                        _ => return Err(malformed_mir("stream callback has no input carrier")),
                    };
                    let descriptor = build(
                        self.builder
                            .build_alloca(word.array_type(2), "stream.input.home"),
                    )?;
                    build(self.builder.build_store(descriptor, carrier))?;
                    build(self.builder.build_store(
                        self.offset(descriptor, bytes)?,
                        word.const_int(native_closure_abi::borrow_home_type_key(ty), false),
                    ))?;
                    descriptor.into()
                }
                _ => {
                    return Err(malformed_mir(
                        "transfer callback has an unexpected borrowed result",
                    ));
                }
            });
        }
        let (target, vtable) = resolve_target(
            self.context,
            self.builder,
            self.target_data,
            &declarations.functions,
            &declarations.interface_vtables,
            plan,
            descriptor,
        )?;
        let receiver = error_carrier_type(self.context).get_undef();
        let receiver =
            build(
                self.builder
                    .build_insert_value(receiver, object, 0, "stream.receiver.object"),
            )?
            .into_struct_value();
        let receiver =
            build(
                self.builder
                    .build_insert_value(receiver, vtable, 1, "stream.receiver.vtable"),
            )?
            .into_struct_value();
        values.push(receiver.into());
        if pending(plan.operation) {
            values.push(params[2].into());
        }
        let region = byte_region_type(self.context, self.target_data).get_undef();
        let region = build(
            self.builder
                .build_insert_value(region, data, 0, "stream.region.data"),
        )?
        .into_struct_value();
        let region =
            build(
                self.builder
                    .build_insert_value(region, length, 1, "stream.region.length"),
            )?
            .into_struct_value();
        if !resolver {
            values.push(region.into());
        }
        let call = match target {
            CallTarget::Static(function) => build(self.builder.build_call(
                function,
                &values,
                if abi.checked { "stream.checked" } else { "" },
            ))?,
            CallTarget::Dynamic(function) => build(self.builder.build_indirect_call(
                indirect_function_type(
                    self.context,
                    self.target_data,
                    self.program,
                    contract,
                    false,
                )?,
                function,
                &values,
                if abi.checked { "stream.checked" } else { "" },
            ))?,
        };
        apply_indirect_call_abi_attributes(self.context, call, contract, false);
        if abi.checked {
            let status = call
                .try_as_basic_value()
                .basic()
                .ok_or_else(|| malformed_mir("checked stream call has no status"))?
                .into_int_value();
            let success = self.block("stream.success");
            let not_success = self.block("stream.not-success");
            let failure = self.block("stream.failure");
            let invalid = self.block("stream.invalid-status");
            let ok = build(self.builder.build_int_compare(
                IntPredicate::EQ,
                status,
                self.context.i8_type().const_zero(),
                "stream.ok",
            ))?;
            build(
                self.builder
                    .build_conditional_branch(ok, success, not_success),
            )?;
            self.builder.position_at_end(not_success);
            let failed = build(self.builder.build_int_compare(
                IntPredicate::EQ,
                status,
                self.context.i8_type().const_int(1, false),
                "stream.failed",
            ))?;
            build(
                self.builder
                    .build_conditional_branch(failed, failure, invalid),
            )?;
            self.builder.position_at_end(invalid);
            self.return_status(3)?;
            self.builder.position_at_end(failure);
            if !resolver {
                self.project_error_progress(error, outcome, length)?;
            }
            self.return_status(1)?;
            self.builder.position_at_end(success);
        }
        if resolver {
            let source = if abi.checked {
                build(self.builder.build_load(
                    pointer,
                    result.ok_or_else(|| malformed_mir("readiness resolver has no result"))?,
                    "stream.source",
                ))?
                .into_pointer_value()
            } else {
                call.try_as_basic_value()
                    .basic()
                    .ok_or_else(|| malformed_mir("readiness resolver returned no value"))?
                    .into_pointer_value()
            };
            build(self.builder.build_store(outcome, source))?;
            let output = params[3].into_pointer_value();
            build(self.builder.build_store(output, object))?;
            build(
                self.builder
                    .build_store(self.offset(output, bytes)?, descriptor),
            )?;
        } else if let Some(result) = result {
            self.project_outcome(contract.return_type, result, outcome)?;
        }
        self.return_status(0)
    }

    fn count_fits(&self, count: IntValue<'ctx>) -> Result<IntValue<'ctx>, BackendError> {
        let nonnegative = build(self.builder.build_int_compare(
            IntPredicate::SGE,
            count,
            self.context.i64_type().const_zero(),
            "stream.nonnegative",
        ))?;
        if self.target_data.get_pointer_byte_size(None) == 8 {
            return Ok(nonnegative);
        }
        let fits = build(
            self.builder.build_int_compare(
                IntPredicate::ULE,
                count,
                self.context
                    .i64_type()
                    .const_int(u64::from(u32::MAX), false),
                "stream.usize-fits",
            ),
        )?;
        build(
            self.builder
                .build_and(nonnegative, fits, "stream.count-fits"),
        )
    }

    fn store_count(
        &self,
        outcome: PointerValue<'ctx>,
        count: IntValue<'ctx>,
    ) -> Result<(), BackendError> {
        let word = self.context.ptr_sized_int_type(self.target_data, None);
        let count = if word.get_bit_width() == 64 {
            count
        } else {
            build(self.builder.build_int_truncate(count, word, "stream.count"))?
        };
        build(self.builder.build_store(outcome, count))?;
        build(self.builder.build_store(
            self.offset(
                outcome,
                u64::from(self.target_data.get_pointer_byte_size(None)) + 1,
            )?,
            self.context.i8_type().const_int(1, false),
        ))?;
        Ok(())
    }

    fn project_outcome(
        &self,
        return_type: mir::ReturnType,
        mut result: PointerValue<'ctx>,
        outcome: PointerValue<'ctx>,
    ) -> Result<(), BackendError> {
        let ty = match return_type {
            mir::ReturnType::Value(mir::Type::PayloadEnum(ty)) => ty,
            mir::ReturnType::Value(mir::Type::NullablePayloadEnum(ty)) => {
                let present = build(self.builder.build_load(
                    self.context.i8_type(),
                    result,
                    "stream.present",
                ))?
                .into_int_value();
                let present = build(self.builder.build_int_compare(
                    IntPredicate::NE,
                    present,
                    self.context.i8_type().const_zero(),
                    "stream.has-outcome",
                ))?;
                let yes = self.block("stream.some");
                let no = self.block("stream.none");
                build(self.builder.build_conditional_branch(present, yes, no))?;
                self.builder.position_at_end(no);
                self.return_status(4)?;
                self.builder.position_at_end(yes);
                result = self.offset(result, u64::from(ty.nullable_payload_offset))?;
                ty
            }
            _ => {
                return Err(malformed_mir(
                    "stream callback result is not an inline outcome",
                ));
            }
        };
        let definition = enum_definition(self.program, ty.id)?;
        let tag_type = match definition.layout.tag_width {
            1 => self.context.i8_type(),
            2 => self.context.i16_type(),
            4 => self.context.i32_type(),
            _ => return Err(malformed_mir("stream outcome tag width is invalid")),
        };
        let tag = build(self.builder.build_load(
            tag_type,
            self.offset(result, u64::from(definition.layout.tag_offset))?,
            "stream.tag",
        ))?
        .into_int_value();
        let mut runtime_tag = self.context.i8_type().const_int(255, false);
        for (index, case) in definition.cases.iter().enumerate() {
            let selected = build(self.builder.build_int_compare(
                IntPredicate::EQ,
                tag,
                tag_type.const_int(u64::from(case.tag), false),
                "stream.case",
            ))?;
            runtime_tag = build(self.builder.build_select(
                selected,
                self.context.i8_type().const_int(index as u64, false),
                runtime_tag,
                "stream.runtime-tag",
            ))?
            .into_int_value();
        }
        build(self.builder.build_store(
            self.offset(
                outcome,
                u64::from(self.target_data.get_pointer_byte_size(None)),
            )?,
            runtime_tag,
        ))?;
        let has_count = build(self.builder.build_int_compare(
            IntPredicate::EQ,
            runtime_tag,
            self.context.i8_type().const_zero(),
            "stream.has-count",
        ))?;
        let data = self.block("stream.data");
        let done = self.block("stream.outcome-done");
        build(self.builder.build_conditional_branch(has_count, data, done))?;
        self.builder.position_at_end(data);
        let count = build(self.builder.build_load(
            self.context.i64_type(),
            self.offset(
                result,
                u64::from(definition.layout.cases[0].fields[0].offset),
            )?,
            "stream.byte-count",
        ))?
        .into_int_value();
        let copy = self.block("stream.copy-count");
        let invalid = self.block("stream.invalid-count");
        build(
            self.builder
                .build_conditional_branch(self.count_fits(count)?, copy, invalid),
        )?;
        self.builder.position_at_end(invalid);
        self.return_status(3)?;
        self.builder.position_at_end(copy);
        self.store_count(outcome, count)?;
        build(self.builder.build_unconditional_branch(done))?;
        self.builder.position_at_end(done);
        Ok(())
    }

    fn project_error_progress(
        &self,
        error: PointerValue<'ctx>,
        outcome: PointerValue<'ctx>,
        length: IntValue<'ctx>,
    ) -> Result<(), BackendError> {
        let descriptor = self.load_pointer(error, 1)?;
        let magic = self.load_word(
            descriptor,
            crate::native_abi::INTERFACE_VTABLE_IO_PROGRESS_MAGIC_WORD,
        )?;
        let is_io = build(
            self.builder.build_int_compare(
                IntPredicate::EQ,
                magic,
                magic
                    .get_type()
                    .const_int(crate::native_abi::IO_ERROR_PROGRESS_DESCRIPTOR_MAGIC, false),
                "stream.io-progress",
            ),
        )?;
        let inspect = self.block("stream.inspect-progress");
        let done = self.block("stream.progress-done");
        build(self.builder.build_conditional_branch(is_io, inspect, done))?;
        self.builder.position_at_end(inspect);
        let object = self.load_pointer(error, 0)?;
        let offset = self.load_word(
            descriptor,
            crate::native_abi::INTERFACE_VTABLE_IO_TRANSFERRED_COUNT_WORD,
        )?;
        let property = build(unsafe {
            self.builder.build_gep(
                self.context.i8_type(),
                object,
                &[offset],
                "stream.progress-property",
            )
        })?;
        let present = self.load_word(property, 0)?;
        let present = build(self.builder.build_int_compare(
            IntPredicate::NE,
            present,
            present.get_type().const_zero(),
            "stream.progress-present",
        ))?;
        let read = self.block("stream.read-progress");
        build(self.builder.build_conditional_branch(present, read, done))?;
        self.builder.position_at_end(read);
        let layout = native_closure_abi::type_layout(
            mir::Type::NullableScalar(mir::ScalarType::Integer(IntegerType::Int64)),
            self.target_data.get_pointer_byte_size(None),
        );
        let count = build(self.builder.build_load(
            self.context.i64_type(),
            self.offset(property, u64::from(layout.align))?,
            "stream.error-count",
        ))?
        .into_int_value();
        let bound = if length.get_type().get_bit_width() == 64 {
            length
        } else {
            build(
                self.builder
                    .build_int_z_extend(length, self.context.i64_type(), "stream.bound"),
            )?
        };
        let bounded = build(self.builder.build_int_compare(
            IntPredicate::ULE,
            count,
            bound,
            "stream.bounded-progress",
        ))?;
        let valid = build(self.builder.build_and(
            self.count_fits(count)?,
            bounded,
            "stream.valid-progress",
        ))?;
        let copy = self.block("stream.copy-progress");
        build(self.builder.build_conditional_branch(valid, copy, done))?;
        self.builder.position_at_end(copy);
        self.store_count(outcome, count)?;
        build(self.builder.build_unconditional_branch(done))?;
        self.builder.position_at_end(done);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inkwell::values::AnyValue;

    fn fixture() -> mir::Program {
        crate::lower_source_to_mir(
            "stream-callback-emitters.doria",
            include_str!("../../tests/fixtures/stream_callback_emitters.doria"),
        )
        .unwrap()
    }

    #[test]
    fn unregistered_types_and_empty_readiness_construction_emit_no_callbacks() {
        let context = Context::create();
        let module = context.create_module("empty-callbacks");
        let target = host_target_machine().unwrap().get_target_data();
        assert!(declare(&context, &module, &target, &fixture())
            .unwrap()
            .is_empty());
        let empty = crate::lower_source_to_mir("empty-callbacks.doria",
            "function main(): void throws Doria\\Std\\Io\\IoError { let $set = new Doria\\Std\\Io\\ReadinessSet(); }").unwrap();
        assert!(declare(&context, &module, &target, &empty)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn all_c_abi_thunks_compile_for_closed_open_and_original_erased_receivers() {
        for checked_getter in [false, true] {
            exercise_callbacks(checked_getter);
        }
    }

    fn exercise_callbacks(checked_getter: bool) {
        let mut program = if checked_getter {
            crate::lower_source_to_mir(
                "checked-source-callbacks.doria",
                include_str!("../../tests/fixtures/stream_callback_emitters.doria")
                    .replace("borrowed get =>", "borrowed get throws IoError =>"),
            )
            .unwrap()
        } else {
            fixture()
        };
        let context = Context::create();
        let machine = host_target_machine().unwrap();
        let target = machine.get_target_data();
        let module = build_module(&context, &machine, &program, None).unwrap();
        let declarations = DeclaredProgram {
            functions: program
                .functions
                .iter()
                .map(|function| module.get_function(&function_symbol(function)).unwrap())
                .collect(),
            interface_vtables: program
                .interface_vtables
                .iter()
                .map(|table| {
                    module
                        .get_global(&format!("__doria_interface_vtable_{}", table.id.0))
                        .unwrap()
                })
                .collect(),
            class_drop_functions: Vec::new(),
            collection_drop_functions: Vec::new(),
            closure_drop_functions: Vec::new(),
            closure_descriptors: Vec::new(),
            class_descriptors: Vec::new(),
            statics: Vec::new(),
            error_origins: Vec::new(),
        };
        let closed = mir::Type::Class(
            program
                .classes
                .iter()
                .find(|class| class.declaration_name == "Closed")
                .unwrap()
                .id,
        );
        let open = mir::Type::Class(
            program
                .classes
                .iter()
                .find(|class| class.declaration_name == "Open")
                .unwrap()
                .id,
        );
        let erased = mir::Type::Interface(
            program
                .interface_types
                .iter()
                .find(|interface| interface.name == "Both")
                .unwrap()
                .id,
        );
        for interface in &mut program.interface_types {
            interface.name = format!("RenamedSurface{}", interface.id.0);
            for (slot, method) in interface.methods.iter_mut().enumerate() {
                method.name = format!("renamedMethod{slot}");
            }
        }
        for receiver in [closed, open, erased] {
            for operation in OPERATIONS {
                let plan = program.stream_callback(receiver, operation).unwrap();
                let sig = signature(&context, &target, operation);
                assert_eq!(sig.count_param_types(), argument_count(operation) as u32);
                if operation != Op::ReadinessSource {
                    assert!(
                        sig.get_param_types()[3 + usize::from(pending(operation))].is_int_type()
                    );
                }
                let function = module.add_function(
                    &symbol(receiver, operation).unwrap(),
                    sig,
                    Some(Linkage::Internal),
                );
                define(
                    &context,
                    &target,
                    &program,
                    &[Callback { plan, function }],
                    &declarations,
                )
                .unwrap();
                assert!(function.verify(true));
                let ir = function.print_to_string().to_string();
                let contract = function_type_in(&program, plan.signature).unwrap();
                let abi =
                    native_closure_abi::NativeCallableSignaturePlan::interface_entry(contract);
                assert_eq!(
                    ir.contains(if abi.checked {
                        "call i8 @"
                    } else {
                        "call ptr @"
                    }),
                    receiver == closed,
                    "{operation:?}: {ir}"
                );
                assert_eq!(ir.matches(" call ").count(), 1, "{ir}");
                assert!(
                    !ir.contains("dr_v2_class_allocate") && !ir.contains("dr_v1_bytes_allocate"),
                    "{ir}"
                );
                if matches!(operation, Op::CompleteRead | Op::CompleteWrite) {
                    assert!(
                        ir.contains("ret i8 4"),
                        "nullable absence must survive: {ir}"
                    );
                }
            }
        }
        module.verify().unwrap();
        assert!(!machine
            .write_to_memory_buffer(&module, FileType::Object)
            .unwrap()
            .as_slice()
            .is_empty());
    }
}

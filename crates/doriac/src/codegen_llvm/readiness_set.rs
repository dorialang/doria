//! Owning membership calls the ordinary borrowed getter before transferring
//! the original complete receiver to the shared runtime.

use super::*;
use crate::compiler_known_stream::StreamOperation as Op;

impl<'ctx> FunctionLowerer<'ctx, '_> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn lower_readiness_membership(
        &mut self,
        operation: &mir::CheckedIoOperation,
        result: Option<mir::LocalId>,
        error: mir::LocalId,
        success: BasicBlock<'ctx>,
        failure: BasicBlock<'ctx>,
        span: crate::source::Span,
    ) -> Result<(), BackendError> {
        if let mir::CheckedIoOperation::ObserveTransferGroup {
            group,
            wait,
            outcome,
        } = operation
        {
            return self.lower_transfer_observation(
                operation, *group, *wait, *outcome, result, error, success, failure, span,
            );
        }
        if let mir::CheckedIoOperation::ResubmitTransfer {
            group,
            key,
            reading,
            offset,
            length,
        } = operation
        {
            self.set_active_panic_site(span)?;
            let pointer = self.context.ptr_type(AddressSpace::default());
            let byte = self.context.i8_type();
            let integer = self.context.i64_type();
            let slots = self.checked_io_output_slots()?;
            let group = build(self.builder.build_load(
                pointer,
                local_slot(&self.local_slots, *group)?,
                "resubmit.group",
            ))?;
            let key = local_slot(&self.local_slots, *key)?;
            let offset = build(self.builder.build_load(
                integer,
                local_slot(&self.local_slots, *offset)?,
                "resubmit.offset",
            ))?;
            let length = build(self.builder.build_load(
                integer,
                local_slot(&self.local_slots, *length)?,
                "resubmit.length",
            ))?;
            let mut args: Vec<BasicMetadataValueEnum<'ctx>> = vec![
                self.current_frame.into(),
                group.into(),
                key.into(),
                byte.const_int(u64::from(*reading), false).into(),
                offset.into(),
                length.into(),
            ];
            args.extend([2, 3, 4, 7, 8].map(|index| BasicMetadataValueEnum::from(slots[index])));
            let status = self
                .call_runtime(
                    "dr_v1_checked_transfer_group_resubmit",
                    &[
                        pointer.into(),
                        pointer.into(),
                        pointer.into(),
                        byte.into(),
                        integer.into(),
                        integer.into(),
                        pointer.into(),
                        pointer.into(),
                        pointer.into(),
                        pointer.into(),
                        pointer.into(),
                    ],
                    Some(byte.into()),
                    &args,
                )?
                .ok_or_else(|| backend_failure("transfer resubmission produced no status"))?
                .into_int_value();
            return self.finish_checked_resource(
                operation, result, error, success, failure, status, slots,
            );
        }
        if let mir::CheckedIoOperation::TransferGroupReadRegion { group, key } = operation {
            self.set_active_panic_site(span)?;
            let pointer = self.context.ptr_type(AddressSpace::default());
            let slots = self.checked_io_output_slots()?;
            let group = build(self.builder.build_load(
                pointer,
                local_slot(&self.local_slots, *group)?,
                "read-region.group",
            ))?;
            let key = local_slot(&self.local_slots, *key)?;
            let mut args: Vec<BasicMetadataValueEnum<'ctx>> =
                vec![self.current_frame.into(), group.into(), key.into()];
            args.extend(
                [1, 5, 0, 2, 3, 4, 7, 8].map(|index| BasicMetadataValueEnum::from(slots[index])),
            );
            let status = self
                .call_runtime(
                    "dr_v1_checked_transfer_group_read_region",
                    &[pointer.into(); 11],
                    Some(self.context.i8_type().into()),
                    &args,
                )?
                .ok_or_else(|| backend_failure("completed read access produced no status"))?
                .into_int_value();
            return self.finish_checked_resource(
                operation, result, error, success, failure, status, slots,
            );
        }
        if let mir::CheckedIoOperation::BeginTransferGroup {
            set,
            plan,
            deadline,
            cancellation,
            ..
        } = operation
        {
            self.set_active_panic_site(span)?;
            let pointer = self.context.ptr_type(AddressSpace::default());
            let byte = self.context.i8_type();
            let slots = self.checked_io_output_slots()?;
            let set = build(self.builder.build_load(
                pointer,
                local_slot(&self.local_slots, *set)?,
                "group.set",
            ))?;
            let plan_slot = local_slot(&self.local_slots, *plan)?;
            let plan = build(self.builder.build_load(pointer, plan_slot, "group.plan"))?;
            build(self.builder.build_store(plan_slot, pointer.const_null()))?;
            let word = self.context.ptr_sized_int_type(self.target_data, None);
            let deadline_ty = local_in(self.function, *deadline)?.ty;
            let deadline_slot = local_slot(&self.local_slots, *deadline)?;
            let present = build(self.builder.build_load(
                word,
                deadline_slot,
                "group.deadline.present",
            ))?
            .into_int_value();
            let present = build(self.builder.build_int_compare(
                IntPredicate::NE,
                present,
                word.const_zero(),
                "group.has-deadline",
            ))?;
            let payload = llvm_byte_offset(
                self.context,
                self.target_data,
                &self.builder,
                deadline_slot,
                native_closure_abi::type_layout(deadline_ty, word.get_bit_width() / 8).align,
                "group.deadline.payload",
            )?;
            let deadline = build(self.builder.build_select(
                present,
                payload,
                pointer.const_null(),
                "group.deadline",
            ))?
            .into_pointer_value();
            let cancellation_ty = local_in(self.function, *cancellation)?.ty;
            let (state, _) = self
                .load_managed_inline_claim(
                    local_slot(&self.local_slots, *cancellation)?,
                    cancellation_ty,
                )?
                .ok_or_else(|| malformed_mir("group cancellation has no managed inline carrier"))?;
            let cancellation = build(self.builder.build_int_to_ptr(
                state,
                pointer,
                "group.cancellation",
            ))?;
            let error_address = local_slot(&self.local_slots, error)?;
            build(
                self.builder
                    .build_store(error_address, error_carrier_type(self.context).const_zero()),
            )?;
            let mut args: Vec<BasicMetadataValueEnum<'ctx>> = vec![
                self.current_frame.into(),
                set.into(),
                plan.into(),
                deadline.into(),
                cancellation.into(),
                slots[1].into(),
                error_address.into(),
            ];
            args.extend([2, 3, 4, 7, 8].map(|index| BasicMetadataValueEnum::from(slots[index])));
            let status = self
                .call_runtime(
                    "dr_v2_checked_transfer_group_begin",
                    &[pointer.into(); 12],
                    Some(byte.into()),
                    &args,
                )?
                .ok_or_else(|| backend_failure("group setup produced no status"))?
                .into_int_value();
            let original_error = build(self.builder.build_int_compare(
                IntPredicate::EQ,
                status,
                byte.const_int(2, false),
                "group.original-error",
            ))?;
            let inspect = self
                .context
                .append_basic_block(current_function(&self.builder)?, "group.status");
            build(
                self.builder
                    .build_conditional_branch(original_error, failure, inspect),
            )?;
            self.builder.position_at_end(inspect);
            return self.finish_checked_resource(
                operation, result, error, success, failure, status, slots,
            );
        }
        self.set_active_panic_site(span)?;
        let pointer = self.context.ptr_type(AddressSpace::default());
        let byte = self.context.i8_type();
        let result = result.ok_or_else(|| malformed_mir("membership call has no result"))?;
        let result_slot = local_slot(&self.local_slots, result)?;
        let slots = self.checked_io_output_slots()?;
        let carrier = self.entry_alloca(error_carrier_type(self.context), "membership.carrier")?;
        let returned_home = self.entry_alloca(pointer, "membership.returned-home")?;
        build(
            self.builder
                .build_store(returned_home, pointer.const_null()),
        )?;
        build(
            self.builder
                .build_store(carrier, error_carrier_type(self.context).const_zero()),
        )?;

        let (set, symbol, mut args): (_, _, Vec<BasicMetadataValueEnum<'ctx>>) = match *operation {
            mir::CheckedIoOperation::RegisterReadinessStream { set, stream, .. } => {
                let ty = local_in(self.function, stream)?.ty;
                let slot = local_slot(&self.local_slots, stream)?;
                let storage = llvm_type(self.context, self.target_data, self.program, ty);
                let original = build(self.builder.build_load(
                    storage,
                    slot,
                    "membership.original",
                ))?;
                let (object, descriptor) = match ty {
                    mir::Type::Scalar(mir::ScalarType::Inline(ty)) => {
                        let class = ty.nominal_class().ok_or_else(|| {
                            malformed_mir("registered inline value has no nominal identity")
                        })?;
                        if ty.storage_bytes() > self.target_data.get_pointer_byte_size(None) {
                            return Err(malformed_mir(
                                "registered inline value exceeds one carrier word",
                            ));
                        }
                        let object = build(self.builder.build_int_to_ptr(
                            original.into_int_value(),
                            pointer,
                            "membership.inline-bits",
                        ))?;
                        (object, self.class_descriptor_address(class)?)
                    }
                    mir::Type::Class(class) => {
                        let (object, descriptor) = self.class_parts(original, class)?;
                        (object, descriptor.unwrap_or_else(|| pointer.const_null()))
                    }
                    mir::Type::Interface(_) => self.error_parts(original.into_struct_value())?,
                    _ => return Err(malformed_mir("registered owner is not nominal")),
                };
                let plan = self
                    .program
                    .stream_callback(ty, Op::ReadinessSource)
                    .ok_or_else(|| malformed_mir("registered owner has no readiness getter"))?;
                let (target, vtable) = stream_callbacks::resolve_target(
                    self.context,
                    &self.builder,
                    self.target_data,
                    self.functions,
                    self.interface_vtables,
                    plan,
                    descriptor,
                )?;
                let source =
                    self.readiness_getter(target, plan, object, vtable, stream, error, failure)?;
                // The method view is ordinary class metadata for concrete/open
                // values, but is the unchanged original vtable for erasure.
                let owner = self.error_value(object, vtable)?;
                build(self.builder.build_store(carrier, owner))?;
                let pending = self.readiness_pending_adapter(ty, object, descriptor)?;
                let resolver =
                    stream_callbacks::address(self.module, self.program, ty, Op::ReadinessSource)?
                        .ok_or_else(|| malformed_mir("registered owner has no source resolver"))?;
                // Keep the original home alive for the getter, then resolve take
                // before runtime admission, which consumes on either status.
                build(self.builder.build_store(slot, storage.const_zero()))?;
                (
                    set,
                    "dr_v1_checked_readiness_set_add_owned",
                    vec![
                        source.into(),
                        resolver.into(),
                        pending.into(),
                        carrier.into(),
                        result_slot.into(),
                    ],
                )
            }
            mir::CheckedIoOperation::UnregisterReadinessStream { set, key, .. } => {
                let key = local_slot(&self.local_slots, key)?;
                (
                    set,
                    "dr_v1_checked_readiness_set_remove_owned",
                    vec![key.into(), carrier.into()],
                )
            }
            mir::CheckedIoOperation::BorrowReadinessStream { set, key, .. } => {
                let key = local_slot(&self.local_slots, key)?;
                (
                    set,
                    "dr_v1_checked_readiness_set_get_owned",
                    vec![key.into(), carrier.into(), returned_home.into()],
                )
            }
            _ => {
                return Err(malformed_mir(
                    "nonmembership operation reached membership lowering",
                ));
            }
        };
        let set = build(self.builder.build_load(
            pointer,
            local_slot(&self.local_slots, set)?,
            "membership.set",
        ))?;
        args.splice(0..0, [self.current_frame.into(), set.into()]);
        args.extend([2, 3, 4, 7, 8].map(|index| BasicMetadataValueEnum::from(slots[index])));
        let status = self
            .call_runtime(
                symbol,
                &vec![pointer.into(); args.len()],
                Some(byte.into()),
                &args,
            )?
            .ok_or_else(|| backend_failure("membership returned no status"))?
            .into_int_value();
        let current = current_function(&self.builder)?;
        let store = self
            .context
            .append_basic_block(current, "membership.success-store");
        let failed = self
            .context
            .append_basic_block(current, "membership.failure-status");
        let ok = build(self.builder.build_int_compare(
            IntPredicate::EQ,
            status,
            byte.const_zero(),
            "membership.succeeded",
        ))?;
        build(self.builder.build_conditional_branch(ok, store, failed))?;
        self.builder.position_at_end(store);
        if let mir::CheckedIoOperation::UnregisterReadinessStream { stream_type, .. }
        | mir::CheckedIoOperation::BorrowReadinessStream { stream_type, .. } = *operation
        {
            if matches!(
                operation,
                mir::CheckedIoOperation::BorrowReadinessStream { .. }
            ) {
                if let Some(plan) = self
                    .program
                    .stream_callback(stream_type, Op::ReadinessSource)
                {
                    if stream_type.has_move_ownership() {
                        let home = BorrowHome {
                            address: build(self.builder.build_load(
                                pointer,
                                returned_home,
                                "membership.home",
                            ))?
                            .into_pointer_value(),
                            source_type: self
                                .borrow_home_type_key(mir::Type::Interface(plan.interface)),
                        };
                        let descriptor =
                            *self.borrow_home_addresses.get(&result).ok_or_else(|| {
                                malformed_mir("readiness loan has no home descriptor")
                            })?;
                        self.store_borrow_home(descriptor, home)?;
                    }
                    self.store_removed_readiness_owner(carrier, result_slot, stream_type, success)?;
                } else {
                    // Unconstrained key declarations remain legal; registration
                    // cannot issue a key for a type without readiness capability.
                    self.trap_invalid_membership_transport()?;
                }
            } else {
                self.store_removed_readiness_owner(carrier, result_slot, stream_type, success)?;
            }
        } else {
            build(self.builder.build_unconditional_branch(success))?;
        }
        self.builder.position_at_end(failed);
        let build_error = self
            .context
            .append_basic_block(current, "membership.build-error");
        let invalid = self
            .context
            .append_basic_block(current, "membership.invalid-status");
        let is_error = build(self.builder.build_int_compare(
            IntPredicate::EQ,
            status,
            byte.const_int(1, false),
            "membership.failed",
        ))?;
        build(
            self.builder
                .build_conditional_branch(is_error, build_error, invalid),
        )?;
        self.builder.position_at_end(build_error);
        self.lower_checked_io_error(
            error, slots[2], slots[3], slots[4], slots[5], slots[6], slots[7], slots[8], failure,
        )?;
        self.builder.position_at_end(invalid);
        self.trap_invalid_membership_transport()?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn readiness_getter(
        &mut self,
        target: stream_callbacks::CallTarget<'ctx>,
        plan: mir::StreamCallbackPlan,
        object: PointerValue<'ctx>,
        vtable: PointerValue<'ctx>,
        stream: mir::LocalId,
        error: mir::LocalId,
        failure: BasicBlock<'ctx>,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let contract = function_type_in(self.program, plan.signature)?.clone();
        let abi = native_closure_abi::NativeCallableSignaturePlan::interface_entry(&contract);
        let output = self.entry_alloca(pointer, "readiness.getter.value")?;
        let home = self.nominal_local_borrow_home(stream)?;
        let returned_home = self.borrow_home_argument(self.absent_borrow_home())?;
        let error = local_slot(&self.local_slots, error)?;
        let mut args = Vec::with_capacity(abi.hidden_inputs.len() + 1);
        for input in &abi.hidden_inputs {
            args.push(match input {
                native_closure_abi::NativeCallableHiddenInput::CurrentFrame => {
                    self.current_frame.into()
                }
                native_closure_abi::NativeCallableHiddenInput::ResultOut => output.into(),
                native_closure_abi::NativeCallableHiddenInput::ErrorOut => error.into(),
                native_closure_abi::NativeCallableHiddenInput::BorrowHome => home.into(),
                native_closure_abi::NativeCallableHiddenInput::ParameterBorrowHome(0) => {
                    home.into()
                }
                native_closure_abi::NativeCallableHiddenInput::ResultBorrowHomeOut => {
                    returned_home.into()
                }
                native_closure_abi::NativeCallableHiddenInput::Environment => {
                    return Err(malformed_mir("getter has a closure environment"));
                }
                native_closure_abi::NativeCallableHiddenInput::ParameterBorrowHome(_) => {
                    return Err(malformed_mir("getter has an unexpected source parameter"));
                }
            });
        }
        args.push(self.error_value(object, vtable)?.into());
        let call = match target {
            stream_callbacks::CallTarget::Static(function) => {
                build(self.builder.build_call(function, &args, "readiness.getter"))?
            }
            stream_callbacks::CallTarget::Dynamic(entry) => {
                build(self.builder.build_indirect_call(
                    indirect_function_type(
                        self.context,
                        self.target_data,
                        self.program,
                        &contract,
                        false,
                    )?,
                    entry,
                    &args,
                    "readiness.getter",
                ))?
            }
        };
        apply_indirect_call_abi_attributes(self.context, call, &contract, false);
        let raw = call
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| malformed_mir("readiness getter returned no value"))?;
        if !abi.checked {
            return Ok(raw.into_pointer_value());
        }
        let status = raw.into_int_value();
        let byte = self.context.i8_type();
        let current = current_function(&self.builder)?;
        let ready = self
            .context
            .append_basic_block(current, "readiness.getter.ready");
        let not_ready = self
            .context
            .append_basic_block(current, "readiness.getter.failure-status");
        let failed = self
            .context
            .append_basic_block(current, "readiness.getter.failed");
        let invalid = self
            .context
            .append_basic_block(current, "readiness.getter.invalid-status");
        let ok = build(self.builder.build_int_compare(
            IntPredicate::EQ,
            status,
            byte.const_zero(),
            "readiness.getter.succeeded",
        ))?;
        build(self.builder.build_conditional_branch(ok, ready, not_ready))?;
        self.builder.position_at_end(not_ready);
        let is_error = build(self.builder.build_int_compare(
            IntPredicate::EQ,
            status,
            byte.const_int(1, false),
            "readiness.getter.is-error",
        ))?;
        build(
            self.builder
                .build_conditional_branch(is_error, failed, invalid),
        )?;
        self.builder.position_at_end(failed);
        let ty = local_in(self.function, stream)?.ty;
        let slot = local_slot(&self.local_slots, stream)?;
        let storage = llvm_type(self.context, self.target_data, self.program, ty);
        let original = build(self.builder.build_load(
            storage,
            slot,
            "readiness.getter.failed-owner",
        ))?;
        build(self.builder.build_store(slot, storage.const_zero()))?;
        // Preserve the getter's original ErrorOut carrier. Resolve take with
        // ordinary drop glue before entering the caller's catch/cleanup block.
        self.drop_stored_value(original, ty)?;
        build(self.builder.build_unconditional_branch(failure))?;
        self.builder.position_at_end(invalid);
        self.trap_invalid_membership_transport()?;
        self.builder.position_at_end(ready);
        Ok(build(
            self.builder
                .build_load(pointer, output, "readiness.source.value"),
        )?
        .into_pointer_value())
    }

    fn readiness_pending_adapter(
        &self,
        ty: mir::Type,
        object: PointerValue<'ctx>,
        descriptor: PointerValue<'ctx>,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let callbacks = [
            Op::PrepareRead,
            Op::PrepareWrite,
            Op::CompleteRead,
            Op::CompleteWrite,
        ]
        .map(|operation| stream_callbacks::address(self.module, self.program, ty, operation))
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
        if callbacks.iter().all(Option::is_none) {
            return Ok(pointer.const_null());
        }
        if callbacks[0].is_some() != callbacks[2].is_some()
            || callbacks[1].is_some() != callbacks[3].is_some()
        {
            return Err(malformed_mir("pending readiness callbacks are not paired"));
        }
        let storage = self.context.struct_type(&[pointer.into(); 6], false);
        let adapter = self.entry_alloca(storage, "membership.pending-adapter")?;
        let mut value = storage.const_zero();
        for (index, field) in [object, descriptor]
            .into_iter()
            .chain(
                callbacks
                    .into_iter()
                    .map(|callback| callback.unwrap_or_else(|| pointer.const_null())),
            )
            .enumerate()
        {
            value = build(self.builder.build_insert_value(
                value,
                field,
                index as u32,
                "membership.pending-field",
            ))?
            .into_struct_value();
        }
        build(self.builder.build_store(adapter, value))?;
        Ok(adapter)
    }

    fn store_removed_readiness_owner(
        &self,
        carrier: PointerValue<'ctx>,
        destination: PointerValue<'ctx>,
        ty: mir::Type,
        success: BasicBlock<'ctx>,
    ) -> Result<(), BackendError> {
        // unregister<T> deliberately has no readiness bound. Non-nominal T
        // declarations compile; no successful registration can issue that key.
        if !matches!(ty, mir::Type::Class(_) | mir::Type::Interface(_))
            && !matches!(ty, mir::Type::Scalar(mir::ScalarType::Inline(value)) if value.nominal_class().is_some() && value.storage_bytes() <= self.target_data.get_pointer_byte_size(None))
        {
            return self.trap_invalid_membership_transport();
        }
        let value = build(self.builder.build_load(
            error_carrier_type(self.context),
            carrier,
            "membership.returned",
        ))?
        .into_struct_value();
        let (object, vtable) = self.error_parts(value)?;
        let restored = match ty {
            mir::Type::Scalar(mir::ScalarType::Inline(_)) => build(self.builder.build_ptr_to_int(
                object,
                self.context.i64_type(),
                "membership.restored-inline",
            ))?
            .into(),
            mir::Type::Interface(_) => value.into(),
            mir::Type::Class(class) => {
                let descriptor = if class_uses_open_carrier(self.program, class) {
                    let field = build(self.builder.build_struct_gep(
                        interface_vtable_header_type(self.context, self.target_data),
                        vtable,
                        crate::native_abi::INTERFACE_VTABLE_CLASS_DESCRIPTOR_WORD,
                        "membership.returned-class-field",
                    ))?;
                    Some(
                        build(self.builder.build_load(
                            self.context.ptr_type(AddressSpace::default()),
                            field,
                            "membership.returned-class",
                        ))?
                        .into_pointer_value(),
                    )
                } else {
                    None
                };
                self.class_value_for_static_type(object, descriptor, class, class)?
            }
            _ => unreachable!(),
        };
        self.store_value_at_address(destination, restored, ty)?;
        build(
            self.builder
                .build_store(carrier, error_carrier_type(self.context).const_zero()),
        )?;
        build(self.builder.build_unconditional_branch(success))?;
        Ok(())
    }

    fn trap_invalid_membership_transport(&self) -> Result<(), BackendError> {
        self.call_runtime("llvm.trap", &[], None, &[])?;
        build(self.builder.build_unreachable())?;
        Ok(())
    }

    /// Same order as checked file/pending I/O: tag, value, message, path,
    /// system code, valid count, invalid count, transferred count, metadata.
    #[allow(clippy::too_many_arguments)]
    fn lower_transfer_observation(
        &mut self,
        operation: &mir::CheckedIoOperation,
        group: mir::LocalId,
        wait: mir::LocalId,
        outcome: mir::PayloadEnumType,
        result: Option<mir::LocalId>,
        error: mir::LocalId,
        success: BasicBlock<'ctx>,
        failure: BasicBlock<'ctx>,
        span: crate::source::Span,
    ) -> Result<(), BackendError> {
        self.set_active_panic_site(span)?;
        let pointer = self.context.ptr_type(AddressSpace::default());
        let byte = self.context.i8_type();
        let integer = self.context.i64_type();
        let word = self.context.ptr_sized_int_type(self.target_data, None);
        let slots = self.checked_io_output_slots()?;
        let group = build(self.builder.build_load(
            pointer,
            local_slot(&self.local_slots, group)?,
            "observe.group",
        ))?;
        let mir::Type::PayloadEnum(wait_type) = local_in(self.function, wait)?.ty else {
            return Err(malformed_mir("observation has no Wait enum"));
        };
        let wait_definition = enum_definition(self.program, wait_type.id)?;
        let wait_address = local_slot(&self.local_slots, wait)?;
        let tag = build(
            self.builder
                .build_load(byte, wait_address, "observe.wait-tag"),
        )?
        .into_int_value();
        let kind_slot = self.entry_alloca(byte, "observe.wait-kind")?;
        let low_slot = self.entry_alloca(integer, "observe.wait-low")?;
        let high_slot = self.entry_alloca(integer, "observe.wait-high")?;
        for slot in [low_slot, high_slot] {
            build(self.builder.build_store(slot, integer.const_zero()))?;
        }
        let function = current_function(&self.builder)?;
        let relative = self
            .context
            .append_basic_block(function, "observe.relative");
        let absolute = self
            .context
            .append_basic_block(function, "observe.absolute");
        let untimed = self.context.append_basic_block(function, "observe.untimed");
        let check_absolute = self
            .context
            .append_basic_block(function, "observe.check-absolute");
        let ready = self
            .context
            .append_basic_block(function, "observe.wait-ready");
        let relative_test = build(self.builder.build_int_compare(
            IntPredicate::EQ,
            tag,
            byte.const_int(1, false),
            "observe.is-relative",
        ))?;
        build(
            self.builder
                .build_conditional_branch(relative_test, relative, check_absolute),
        )?;
        self.builder.position_at_end(check_absolute);
        let absolute_test = build(self.builder.build_int_compare(
            IntPredicate::EQ,
            tag,
            byte.const_int(2, false),
            "observe.is-absolute",
        ))?;
        build(
            self.builder
                .build_conditional_branch(absolute_test, absolute, untimed),
        )?;
        for (block, case, kind) in [(relative, 1usize, 2u64), (absolute, 2, 3)] {
            self.builder.position_at_end(block);
            let offset = wait_definition.layout.cases[case].fields[0].offset;
            let address = self.byte_offset(wait_address, offset, "observe.wait-payload")?;
            let low = build(self.builder.build_load(integer, address, "observe.low"))?;
            build(self.builder.build_store(low_slot, low))?;
            if case == 2 {
                let address =
                    self.byte_offset(wait_address, offset + 8, "observe.wait-high-address")?;
                let high = build(self.builder.build_load(integer, address, "observe.high"))?;
                build(self.builder.build_store(high_slot, high))?;
            }
            build(
                self.builder
                    .build_store(kind_slot, byte.const_int(kind, false)),
            )?;
            build(self.builder.build_unconditional_branch(ready))?;
        }
        self.builder.position_at_end(untimed);
        let indefinite = build(self.builder.build_int_compare(
            IntPredicate::EQ,
            tag,
            byte.const_int(3, false),
            "observe.is-indefinite",
        ))?;
        let kind = build(self.builder.build_select(
            indefinite,
            byte.const_int(1, false),
            byte.const_zero(),
            "observe.kind",
        ))?;
        build(self.builder.build_store(kind_slot, kind))?;
        build(self.builder.build_unconditional_branch(ready))?;
        self.builder.position_at_end(ready);
        let kind = build(self.builder.build_load(byte, kind_slot, "observe.kind"))?;
        let low = build(self.builder.build_load(integer, low_slot, "observe.low"))?;
        let high = build(self.builder.build_load(integer, high_slot, "observe.high"))?;
        let result_tag = self.entry_alloca(byte, "observe.result")?;
        let reading = self.entry_alloca(byte, "observe.reading")?;
        let key = self.entry_alloca(integer.array_type(2), "observe.key")?;
        let outcome_tag = self.entry_alloca(byte, "observe.outcome")?;
        let count = self.entry_alloca(word, "observe.count")?;
        let pointer_bytes = word.get_bit_width() / 8;
        let control = self.entry_alloca(
            integer.array_type(crate::native_abi::transfer_control_size(pointer_bytes) / 8),
            "observe.control",
        )?;
        let error_address = local_slot(&self.local_slots, error)?;
        build(
            self.builder
                .build_store(error_address, error_carrier_type(self.context).const_zero()),
        )?;
        let mut args: Vec<BasicMetadataValueEnum<'ctx>> = vec![
            self.current_frame.into(),
            group.into(),
            kind.into(),
            low.into(),
            high.into(),
            result_tag.into(),
            reading.into(),
            key.into(),
            outcome_tag.into(),
            count.into(),
            error_address.into(),
            control.into(),
        ];
        args.extend([2, 3, 4, 7, 8].map(|index| BasicMetadataValueEnum::from(slots[index])));
        let mut parameter_types = vec![
            pointer.into(),
            pointer.into(),
            byte.into(),
            integer.into(),
            integer.into(),
        ];
        parameter_types.extend([BasicMetadataTypeEnum::from(pointer); 12]);
        let status = self
            .call_runtime(
                "dr_v1_checked_transfer_group_observe",
                &parameter_types,
                Some(byte.into()),
                &args,
            )?
            .ok_or_else(|| backend_failure("completion observation produced no status"))?
            .into_int_value();
        let inspect = self.context.append_basic_block(function, "observe.status");
        let original = build(self.builder.build_int_compare(
            IntPredicate::EQ,
            status,
            byte.const_int(2, false),
            "observe.original-error",
        ))?;
        build(
            self.builder
                .build_conditional_branch(original, failure, inspect),
        )?;
        self.builder.position_at_end(inspect);
        let store = self.context.append_basic_block(function, "observe.store");
        let ordinary_error = self.context.append_basic_block(function, "observe.error");
        let ok = build(self.builder.build_int_compare(
            IntPredicate::EQ,
            status,
            byte.const_zero(),
            "observe.ok",
        ))?;
        build(
            self.builder
                .build_conditional_branch(ok, store, ordinary_error),
        )?;
        self.builder.position_at_end(ordinary_error);
        self.finish_checked_resource(operation, None, error, success, failure, status, slots)?;
        self.builder.position_at_end(store);
        let address = local_slot(
            &self.local_slots,
            result.ok_or_else(|| malformed_mir("completion observation has no result"))?,
        )?;
        self.zero_payload_bytes(address, outcome, false)?;
        let definition = enum_definition(self.program, outcome.id)?;
        let tag = build(
            self.builder
                .build_load(byte, result_tag, "observe.result-tag"),
        )?
        .into_int_value();
        build(self.builder.build_store(address, tag))?;
        let complete = self
            .context
            .append_basic_block(function, "observe.completed");
        let completed = build(self.builder.build_int_compare(
            IntPredicate::EQ,
            tag,
            byte.const_zero(),
            "observe.is-completed",
        ))?;
        build(
            self.builder
                .build_conditional_branch(completed, complete, success),
        )?;
        self.builder.position_at_end(complete);
        let types = crate::compiler_known_stream::TransferObservationTypes::for_result(
            self.program,
            outcome,
        )
        .ok_or_else(|| malformed_mir("completion observation has no typed schema"))?;
        let completion_definition = enum_definition(self.program, types.completion.id)?;
        let completion = self.byte_offset(
            address,
            definition.layout.cases[0].fields[0].offset,
            "observe.completion",
        )?;
        let direction =
            build(self.builder.build_load(byte, reading, "observe.reading"))?.into_int_value();
        let is_read = build(self.builder.build_int_compare(
            IntPredicate::NE,
            direction,
            byte.const_zero(),
            "observe.is-read",
        ))?;
        let blocks = [
            self.context.append_basic_block(function, "observe.read"),
            self.context.append_basic_block(function, "observe.write"),
        ];
        build(
            self.builder
                .build_conditional_branch(is_read, blocks[0], blocks[1]),
        )?;
        for (direction, block) in blocks.into_iter().enumerate() {
            self.builder.position_at_end(block);
            self.store_payload_enum_tag(
                completion,
                completion_definition.layout.tag_width,
                direction as u32,
            )?;
            let fields = &completion_definition.layout.cases[direction].fields;
            let key_address =
                self.byte_offset(completion, fields[0].offset, "observe.key-field")?;
            let value = build(
                self.builder
                    .build_load(integer.array_type(2), key, "observe.key"),
            )?;
            build(self.builder.build_store(key_address, value))?;
            let tag = build(
                self.builder
                    .build_load(byte, outcome_tag, "observe.outcome-tag"),
            )?
            .into_int_value();
            let present = build(self.builder.build_int_compare(
                IntPredicate::NE,
                tag,
                byte.const_zero(),
                "observe.has-outcome",
            ))?;
            let fill = self
                .context
                .append_basic_block(function, "observe.outcome-present");
            build(
                self.builder
                    .build_conditional_branch(present, fill, success),
            )?;
            self.builder.position_at_end(fill);
            let nullable = self.byte_offset(completion, fields[1].offset, "observe.nullable")?;
            build(self.builder.build_store(nullable, byte.const_int(1, false)))?;
            let outcome_type = types.outcomes[direction];
            let inner = self.byte_offset(
                nullable,
                outcome_type.nullable_payload_offset,
                "observe.inner-outcome",
            )?;
            let inner_definition = enum_definition(self.program, outcome_type.id)?;
            let tag = build(self.builder.build_int_sub(
                tag,
                byte.const_int(1, false),
                "observe.source-tag",
            ))?;
            build(self.builder.build_store(inner, tag))?;
            let value =
                build(self.builder.build_load(word, count, "observe.byte-count"))?.into_int_value();
            let value = if word == integer {
                value
            } else {
                build(
                    self.builder
                        .build_int_z_extend(value, integer, "observe.count64"),
                )?
            };
            let count_address = self.byte_offset(
                inner,
                inner_definition.layout.cases[0].fields[0].offset,
                "observe.count-field",
            )?;
            build(self.builder.build_store(count_address, value))?;
            build(self.builder.build_unconditional_branch(success))?;
        }
        Ok(())
    }

    /// Every slot is empty on each call, including counts unused by membership.
    pub(super) fn checked_io_output_slots(&self) -> Result<[PointerValue<'ctx>; 9], BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let word = self.context.ptr_sized_int_type(self.target_data, None);
        let int = self.context.i64_type();
        let mut slots = Vec::with_capacity(9);
        for (name, ty) in [
            ("checked.io.tag", int.into()),
            ("checked.io.value", word.into()),
            ("checked.io.message", pointer.into()),
            ("checked.io.path", pointer.into()),
            ("checked.io.system-code", int.into()),
            ("checked.io.valid-count", word.into()),
            ("checked.io.invalid-count", word.into()),
            ("checked.io.transferred-count", word.into()),
            ("checked.io.meta", int.into()),
        ] {
            let ty: BasicTypeEnum<'ctx> = ty;
            let slot = self.entry_alloca(ty, name)?;
            build(self.builder.build_store(slot, ty.const_zero()))?;
            slots.push(slot);
        }
        Ok(slots.try_into().expect("nine checked I/O output slots"))
    }
}

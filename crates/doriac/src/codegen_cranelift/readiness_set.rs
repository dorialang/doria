//! Owning membership calls the ordinary borrowed getter before transferring
//! the original complete receiver to the shared runtime.
use super::*;
use crate::compiler_known_stream::StreamOperation as Op;

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
    if let mir::CheckedIoOperation::ObserveTransferGroup {
        group,
        wait,
        outcome,
    } = operation
    {
        return observe(
            builder, *group, *wait, *outcome, result, error, success, failure, span, resources,
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
        set_active_panic_site(builder, span, resources);
        let pointer = resources.module.target_config().pointer_type();
        let slots = checked_io_output_slots(builder, pointer);
        let group = builder.ins().stack_load(
            pointer,
            pointer,
            local_slot(resources.local_slots, *group)?,
            0,
        );
        let key = builder
            .ins()
            .stack_addr(pointer, local_slot(resources.local_slots, *key)?, 0);
        let reading = builder.ins().iconst(types::I8, i64::from(*reading));
        let offset = builder.ins().stack_load(
            pointer,
            types::I64,
            local_slot(resources.local_slots, *offset)?,
            0,
        );
        let length = builder.ins().stack_load(
            pointer,
            types::I64,
            local_slot(resources.local_slots, *length)?,
            0,
        );
        let mut args = vec![resources.current_frame, group, key, reading, offset, length];
        args.extend(
            [2, 3, 4, 7, 8].map(|index| builder.ins().stack_addr(pointer, slots[index], 0)),
        );
        let status = runtime_call(
            builder,
            "dr_v1_checked_transfer_group_resubmit",
            &[
                pointer,
                pointer,
                pointer,
                types::I8,
                types::I64,
                types::I64,
                pointer,
                pointer,
                pointer,
                pointer,
                pointer,
            ],
            Some(types::I8),
            &args,
            resources,
        )?
        .ok_or_else(|| backend_failure("transfer resubmission produced no status"))?;
        return finish_checked_resource(
            builder, operation, result, error, success, failure, status, slots, resources,
        );
    }
    if let mir::CheckedIoOperation::TransferGroupReadRegion { group, key } = operation {
        set_active_panic_site(builder, span, resources);
        let pointer = resources.module.target_config().pointer_type();
        let slots = checked_io_output_slots(builder, pointer);
        let group = builder.ins().stack_load(
            pointer,
            pointer,
            local_slot(resources.local_slots, *group)?,
            0,
        );
        let key = builder
            .ins()
            .stack_addr(pointer, local_slot(resources.local_slots, *key)?, 0);
        let mut args = vec![resources.current_frame, group, key];
        args.extend(
            [1, 5, 0, 2, 3, 4, 7, 8]
                .map(|index| builder.ins().stack_addr(pointer, slots[index], 0)),
        );
        let status = runtime_call(
            builder,
            "dr_v1_checked_transfer_group_read_region",
            &[pointer; 11],
            Some(types::I8),
            &args,
            resources,
        )?
        .ok_or_else(|| backend_failure("completed read access produced no status"))?;
        return finish_checked_resource(
            builder, operation, result, error, success, failure, status, slots, resources,
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
        set_active_panic_site(builder, span, resources);
        let pointer = resources.module.target_config().pointer_type();
        let slots = checked_io_output_slots(builder, pointer);
        let set = builder.ins().stack_load(
            pointer,
            pointer,
            local_slot(resources.local_slots, *set)?,
            0,
        );
        let plan_slot = local_slot(resources.local_slots, *plan)?;
        let plan = builder.ins().stack_load(pointer, pointer, plan_slot, 0);
        let zero = builder.ins().iconst(pointer, 0);
        builder.ins().stack_store(pointer, zero, plan_slot, 0);
        let deadline_ty = local_definition(resources.program, resources.function_id, *deadline)?.ty;
        let deadline_slot = local_slot(resources.local_slots, *deadline)?;
        let deadline_present = builder.ins().stack_load(pointer, pointer, deadline_slot, 0);
        let deadline_payload = builder.ins().stack_addr(
            pointer,
            deadline_slot,
            native_closure_abi::type_layout(deadline_ty, pointer.bytes()).align as i32,
        );
        let deadline = builder
            .ins()
            .select(deadline_present, deadline_payload, zero);
        let cancellation_ty =
            local_definition(resources.program, resources.function_id, *cancellation)?.ty;
        let (present, state) = load_lowered_from_stack(
            builder,
            resources.program,
            cancellation_ty,
            local_slot(resources.local_slots, *cancellation)?,
            pointer,
        )
        .nullable()?;
        let cancellation = builder.ins().select(present, state, zero);
        let error_address =
            builder
                .ins()
                .stack_addr(pointer, local_slot(resources.local_slots, error)?, 0);
        zero_inline_bytes(builder, error_address, pointer.bytes() * 2, pointer);
        let mut args = vec![
            resources.current_frame,
            set,
            plan,
            deadline,
            cancellation,
            builder.ins().stack_addr(pointer, slots[1], 0),
            error_address,
        ];
        args.extend(
            [2, 3, 4, 7, 8].map(|index| builder.ins().stack_addr(pointer, slots[index], 0)),
        );
        let status = runtime_call(
            builder,
            "dr_v2_checked_transfer_group_begin",
            &[pointer; 12],
            Some(types::I8),
            &args,
            resources,
        )?
        .ok_or_else(|| backend_failure("group setup produced no status"))?;
        let original_error = builder.ins().icmp_imm_u(IntCC::Equal, status, 2);
        let inspect = builder.create_block();
        builder
            .ins()
            .brif(original_error, failure, &[], inspect, &[]);
        builder.switch_to_block(inspect);
        return finish_checked_resource(
            builder, operation, result, error, success, failure, status, slots, resources,
        );
    }
    set_active_panic_site(builder, span, resources);
    let pointer = resources.module.target_config().pointer_type();
    let result = result.ok_or_else(|| malformed_mir("membership call has no result"))?;
    let result_slot = local_slot(resources.local_slots, result)?;
    let result_address = builder.ins().stack_addr(pointer, result_slot, 0);
    let slots = checked_io_output_slots(builder, pointer);
    let outputs = [2, 3, 4, 7, 8].map(|index| builder.ins().stack_addr(pointer, slots[index], 0));
    let carrier = scratch(builder, pointer, 2);
    let returned_home = scratch(builder, pointer, 1);
    let (set, symbol, mut args) = match operation {
        mir::CheckedIoOperation::RegisterReadinessStream { set, stream, .. } => {
            let ty = local_definition(resources.program, resources.function_id, *stream)?.ty;
            let slot = local_slot(resources.local_slots, *stream)?;
            let value = load_lowered_from_stack(builder, resources.program, ty, slot, pointer);
            let (object, descriptor) = match ty {
                mir::Type::Scalar(mir::ScalarType::Inline(ty)) => {
                    let class = ty.nominal_class().ok_or_else(|| {
                        malformed_mir("registered inline value has no nominal identity")
                    })?;
                    if ty.storage_bytes() > pointer.bytes() {
                        return Err(malformed_mir(
                            "registered inline value exceeds one carrier word",
                        ));
                    }
                    let bits = value.single()?;
                    let object = if pointer == types::I64 {
                        bits
                    } else {
                        builder.ins().ireduce(pointer, bits)
                    };
                    (object, class_descriptor_address(builder, class, resources)?)
                }
                mir::Type::Class(_) => {
                    let (object, descriptor) = value.class_parts()?;
                    (
                        object,
                        descriptor.unwrap_or_else(|| builder.ins().iconst(pointer, 0)),
                    )
                }
                mir::Type::Interface(_) => value.nullable()?,
                _ => return Err(malformed_mir("registered owner is not nominal")),
            };
            let plan = resources
                .program
                .stream_callback(ty, Op::ReadinessSource)
                .ok_or_else(|| malformed_mir("registered owner has no readiness getter"))?;
            let (target, vtable) = stream_callbacks::resolve_target(
                builder,
                resources.module,
                resources.program,
                resources.function_ids,
                resources.static_ids,
                plan,
                descriptor,
            )?;
            let source = call_getter(
                builder, target, plan, object, vtable, *stream, error, failure, resources,
            )?;
            let flags = cranelift_codegen::ir::MachMemFlags::trusted();
            builder.ins().store(flags, object, carrier, 0);
            // Erased receivers keep their original method view. Concrete views
            // use ordinary class drop/descriptor metadata without another owner.
            builder
                .ins()
                .store(flags, vtable, carrier, pointer.bytes() as i32);
            let pending = pending_adapter(builder, ty, object, descriptor, resources)?;
            let resolver = stream_callbacks::address(
                builder,
                resources.module,
                resources.program,
                ty,
                Op::ReadinessSource,
            )?
            .ok_or_else(|| malformed_mir("registered owner has no source resolver"))?;
            let address = builder.ins().stack_addr(pointer, slot, 0);
            let layout = native_closure_abi::value_layout(resources.program, ty, pointer.bytes());
            zero_inline_bytes(builder, address, layout.size, pointer);
            (
                *set,
                "dr_v1_checked_readiness_set_add_owned",
                vec![source, resolver, pending, carrier, result_address],
            )
        }
        mir::CheckedIoOperation::UnregisterReadinessStream { set, key, .. } => {
            let key_slot = local_slot(resources.local_slots, *key)?;
            let key = builder.ins().stack_addr(pointer, key_slot, 0);
            (
                *set,
                "dr_v1_checked_readiness_set_remove_owned",
                vec![key, carrier],
            )
        }
        mir::CheckedIoOperation::BorrowReadinessStream { set, key, .. } => {
            let key_slot = local_slot(resources.local_slots, *key)?;
            let key = builder.ins().stack_addr(pointer, key_slot, 0);
            (
                *set,
                "dr_v1_checked_readiness_set_get_owned",
                vec![key, carrier, returned_home],
            )
        }
        _ => {
            return Err(malformed_mir(
                "nonmembership operation reached membership lowering",
            ))
        }
    };
    let set =
        builder
            .ins()
            .stack_load(pointer, pointer, local_slot(resources.local_slots, set)?, 0);
    args.splice(0..0, [resources.current_frame, set]);
    args.extend(outputs);
    let status = runtime_call(
        builder,
        symbol,
        &vec![pointer; args.len()],
        Some(types::I8),
        &args,
        resources,
    )?
    .ok_or_else(|| backend_failure("membership returned no status"))?;
    let store = builder.create_block();
    let failed = builder.create_block();
    let ok = builder.ins().icmp_imm_u(IntCC::Equal, status, 0);
    builder.ins().brif(ok, store, &[], failed, &[]);
    builder.switch_to_block(store);
    if let mir::CheckedIoOperation::UnregisterReadinessStream { stream_type, .. }
    | mir::CheckedIoOperation::BorrowReadinessStream { stream_type, .. } = operation
    {
        let flags = cranelift_codegen::ir::MachMemFlags::trusted();
        let object = builder.ins().load(pointer, flags, carrier, 0);
        let vtable = builder
            .ins()
            .load(pointer, flags, carrier, pointer.bytes() as i32);
        let issuable = !matches!(
            operation,
            mir::CheckedIoOperation::BorrowReadinessStream { .. }
        ) || resources
            .program
            .stream_callback(*stream_type, Op::ReadinessSource)
            .is_some();
        let value = if issuable {
            match stream_type {
                mir::Type::Scalar(mir::ScalarType::Inline(ty))
                    if ty.nominal_class().is_some() && ty.storage_bytes() <= pointer.bytes() =>
                {
                    Some(LoweredValue::Single(if pointer == types::I64 {
                        object
                    } else {
                        builder.ins().uextend(types::I64, object)
                    }))
                }
                mir::Type::Interface(_) => Some(LoweredValue::Nullable {
                    present: object,
                    payload: vtable,
                }),
                mir::Type::Class(class) if class_uses_open_carrier(resources.program, *class) => {
                    let descriptor = builder.ins().load(
                        pointer,
                        flags,
                        vtable,
                        (pointer.bytes()
                            * crate::native_abi::INTERFACE_VTABLE_CLASS_DESCRIPTOR_WORD)
                            as i32,
                    );
                    Some(LoweredValue::OpenClass { object, descriptor })
                }
                mir::Type::Class(_) => Some(LoweredValue::Single(object)),
                // Unconstrained key types remain legal. No such key can be issued
                // by register; success would therefore be malformed runtime transport.
                _ => None,
            }
        } else {
            None
        };
        if let Some(value) = value {
            store_lowered_to_stack(
                builder,
                resources.program,
                *stream_type,
                result_slot,
                value,
                pointer,
            )?;
            if matches!(
                operation,
                mir::CheckedIoOperation::BorrowReadinessStream { .. }
            ) && stream_type.has_move_ownership()
            {
                let plan = resources
                    .program
                    .stream_callback(*stream_type, Op::ReadinessSource)
                    .ok_or_else(|| malformed_mir("readiness loan has no registered method view"))?;
                let home = BorrowHome {
                    address: builder.ins().load(pointer, flags, returned_home, 0),
                    source_type: builder.ins().iconst(
                        pointer,
                        native_closure_abi::borrow_home_type_key(mir::Type::Interface(
                            plan.interface,
                        )) as i64,
                    ),
                };
                let descriptor = *resources
                    .borrow_home_addresses
                    .get(&result)
                    .ok_or_else(|| malformed_mir("readiness loan has no home descriptor"))?;
                store_borrow_home_descriptor(builder, descriptor, home, pointer);
            }
            builder.ins().jump(success, &[]);
        } else {
            builder
                .ins()
                .trap(TrapCode::unwrap_user(RUNTIME_RETURNED_TRAP));
        }
    } else {
        builder.ins().jump(success, &[]);
    }
    builder.switch_to_block(failed);
    let build_error = builder.create_block();
    let invalid = builder.create_block();
    let is_error = builder.ins().icmp_imm_u(IntCC::Equal, status, 1);
    builder.ins().brif(is_error, build_error, &[], invalid, &[]);
    builder.switch_to_block(build_error);
    lower_checked_io_error(
        builder, error, slots[2], slots[3], slots[4], slots[5], slots[6], slots[7], slots[8],
        failure, resources,
    )?;
    builder.switch_to_block(invalid);
    builder
        .ins()
        .trap(TrapCode::unwrap_user(RUNTIME_RETURNED_TRAP));
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn call_getter(
    builder: &mut FunctionBuilder,
    target: stream_callbacks::CallTarget,
    plan: mir::StreamCallbackPlan,
    object: Value,
    vtable: Value,
    stream: mir::LocalId,
    error: mir::LocalId,
    failure: Block,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let contract = function_type_in(resources.program, plan.signature)?;
    let abi = native_closure_abi::NativeCallableSignaturePlan::interface_entry(contract);
    let output = scratch(builder, pointer, 1);
    let returned_home = scratch(builder, pointer, 2);
    let home = closure_source_home(builder, stream, resources)?;
    let home = borrow_home_descriptor(builder, home, pointer);
    let error_address =
        builder
            .ins()
            .stack_addr(pointer, local_slot(resources.local_slots, error)?, 0);
    let mut args = Vec::with_capacity(abi.hidden_inputs.len() + 2);
    for input in &abi.hidden_inputs {
        args.push(match input {
            native_closure_abi::NativeCallableHiddenInput::CurrentFrame => resources.current_frame,
            native_closure_abi::NativeCallableHiddenInput::ResultOut => output,
            native_closure_abi::NativeCallableHiddenInput::ErrorOut => error_address,
            native_closure_abi::NativeCallableHiddenInput::BorrowHome => home,
            native_closure_abi::NativeCallableHiddenInput::ParameterBorrowHome(0) => home,
            native_closure_abi::NativeCallableHiddenInput::ResultBorrowHomeOut => returned_home,
            native_closure_abi::NativeCallableHiddenInput::Environment => {
                return Err(malformed_mir("getter has a closure environment"))
            }
            native_closure_abi::NativeCallableHiddenInput::ParameterBorrowHome(_) => {
                return Err(malformed_mir("getter has an unexpected source parameter"))
            }
        });
    }
    args.extend([object, vtable]);
    let call = match target {
        stream_callbacks::CallTarget::Static(function) => builder.ins().call(function, &args),
        stream_callbacks::CallTarget::Dynamic(entry) => {
            let signature =
                indirect_function_signature(resources.module, resources.program, contract, false);
            let signature = builder.import_signature(signature);
            builder.ins().call_indirect(signature, entry, &args)
        }
    };
    let raw = builder.inst_results(call)[0];
    if !abi.checked {
        return Ok(raw);
    }
    let ready = builder.create_block();
    let not_ready = builder.create_block();
    let failed = builder.create_block();
    let invalid = builder.create_block();
    let ok = builder.ins().icmp_imm_u(IntCC::Equal, raw, 0);
    builder.ins().brif(ok, ready, &[], not_ready, &[]);
    builder.switch_to_block(not_ready);
    let is_error = builder.ins().icmp_imm_u(IntCC::Equal, raw, 1);
    builder.ins().brif(is_error, failed, &[], invalid, &[]);
    builder.switch_to_block(failed);
    let ty = local_definition(resources.program, resources.function_id, stream)?.ty;
    let slot = local_slot(resources.local_slots, stream)?;
    let value = load_lowered_from_stack(builder, resources.program, ty, slot, pointer);
    let address = builder.ins().stack_addr(pointer, slot, 0);
    let layout = native_closure_abi::value_layout(resources.program, ty, pointer.bytes());
    zero_inline_bytes(builder, address, layout.size, pointer);
    match ty {
        mir::Type::Scalar(mir::ScalarType::Inline(ty)) => {
            release_inline_value(builder, value.single()?, ty, resources)?;
        }
        mir::Type::Class(class) => {
            lower_drop_class_carrier_checked(builder, value, class, resources)?
        }
        mir::Type::Interface(_) => lower_drop_error_value(builder, value, resources)?,
        _ => return Err(malformed_mir("getter receiver is not nominal")),
    }
    builder.ins().jump(failure, &[]);
    builder.switch_to_block(invalid);
    builder
        .ins()
        .trap(TrapCode::unwrap_user(RUNTIME_RETURNED_TRAP));
    builder.switch_to_block(ready);
    Ok(builder.ins().load(
        pointer,
        cranelift_codegen::ir::MachMemFlags::trusted(),
        output,
        0,
    ))
}

fn pending_adapter(
    builder: &mut FunctionBuilder,
    ty: mir::Type,
    object: Value,
    descriptor: Value,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let mut callbacks = Vec::with_capacity(4);
    for operation in [
        Op::PrepareRead,
        Op::PrepareWrite,
        Op::CompleteRead,
        Op::CompleteWrite,
    ] {
        callbacks.push(stream_callbacks::address(
            builder,
            resources.module,
            resources.program,
            ty,
            operation,
        )?);
    }
    if callbacks.iter().all(Option::is_none) {
        return Ok(builder.ins().iconst(pointer, 0));
    }
    if callbacks[0].is_some() != callbacks[2].is_some()
        || callbacks[1].is_some() != callbacks[3].is_some()
    {
        return Err(malformed_mir("pending readiness callbacks are not paired"));
    }
    let adapter = scratch(builder, pointer, 6);
    let flags = cranelift_codegen::ir::MachMemFlags::trusted();
    builder.ins().store(flags, object, adapter, 0);
    builder
        .ins()
        .store(flags, descriptor, adapter, pointer.bytes() as i32);
    for (index, callback) in callbacks.into_iter().enumerate() {
        let value = callback.unwrap_or_else(|| builder.ins().iconst(pointer, 0));
        builder.ins().store(
            flags,
            value,
            adapter,
            ((index + 2) as u32 * pointer.bytes()) as i32,
        );
    }
    Ok(adapter)
}

#[allow(clippy::too_many_arguments)]
fn observe(
    builder: &mut FunctionBuilder,
    group: mir::LocalId,
    wait: mir::LocalId,
    outcome: mir::PayloadEnumType,
    result: Option<mir::LocalId>,
    error: mir::LocalId,
    success: Block,
    failure: Block,
    span: crate::source::Span,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    set_active_panic_site(builder, span, resources);
    let pointer = resources.module.target_config().pointer_type();
    let flags = cranelift_codegen::ir::MachMemFlags::trusted();
    let slots = checked_io_output_slots(builder, pointer);
    let group_id = group;
    let group = builder.ins().stack_load(
        pointer,
        pointer,
        local_slot(resources.local_slots, group)?,
        0,
    );
    let wait_type = local_definition(resources.program, resources.function_id, wait)?.ty;
    let mir::Type::PayloadEnum(wait_type) = wait_type else {
        return Err(malformed_mir("observation has no Wait enum"));
    };
    let wait_definition = enum_definition(resources.program, wait_type.id)?;
    let wait_address =
        builder
            .ins()
            .stack_addr(pointer, local_slot(resources.local_slots, wait)?, 0);
    let tag = builder.ins().load(types::I8, flags, wait_address, 0);
    let wait_values = builder.create_block();
    for ty in [types::I8, types::I64, types::I64] {
        builder.append_block_param(wait_values, ty);
    }
    let relative = builder.create_block();
    let absolute = builder.create_block();
    let untimed = builder.create_block();
    let check_absolute = builder.create_block();
    let is_relative = builder.ins().icmp_imm_u(IntCC::Equal, tag, 1);
    builder
        .ins()
        .brif(is_relative, relative, &[], check_absolute, &[]);
    builder.switch_to_block(check_absolute);
    let is_absolute = builder.ins().icmp_imm_u(IntCC::Equal, tag, 2);
    builder.ins().brif(is_absolute, absolute, &[], untimed, &[]);
    for (block, case, private_tag) in [(relative, 1usize, 2), (absolute, 2, 3)] {
        builder.switch_to_block(block);
        let offset = wait_definition.layout.cases[case].fields[0].offset as i32;
        let low = builder.ins().load(types::I64, flags, wait_address, offset);
        let high = if case == 2 {
            builder
                .ins()
                .load(types::I64, flags, wait_address, offset + 8)
        } else {
            builder.ins().iconst(types::I64, 0)
        };
        let kind = builder.ins().iconst(types::I8, private_tag);
        builder
            .ins()
            .jump(wait_values, &[kind.into(), low.into(), high.into()]);
    }
    builder.switch_to_block(untimed);
    let indefinite = builder.ins().icmp_imm_u(IntCC::Equal, tag, 3);
    let one = builder.ins().iconst(types::I8, 1);
    let zero = builder.ins().iconst(types::I8, 0);
    let kind = builder.ins().select(indefinite, one, zero);
    let zero64 = builder.ins().iconst(types::I64, 0);
    builder
        .ins()
        .jump(wait_values, &[kind.into(), zero64.into(), zero64.into()]);
    builder.switch_to_block(wait_values);
    let values = builder.block_params(wait_values).to_vec();
    let output = [1, 1, 2, 1, 1].map(|words| scratch(builder, pointer, words));
    let control = scratch(
        builder,
        pointer,
        crate::native_abi::transfer_control_size(pointer.bytes()) / pointer.bytes(),
    );
    let error_address =
        builder
            .ins()
            .stack_addr(pointer, local_slot(resources.local_slots, error)?, 0);
    zero_inline_bytes(builder, error_address, pointer.bytes() * 2, pointer);
    let mut args = vec![
        resources.current_frame,
        group,
        values[0],
        values[1],
        values[2],
    ];
    args.extend(output);
    args.extend([error_address, control]);
    args.extend([2, 3, 4, 7, 8].map(|index| builder.ins().stack_addr(pointer, slots[index], 0)));
    let mut parameter_types = vec![pointer, pointer, types::I8, types::I64, types::I64];
    parameter_types.extend([pointer; 12]);
    let status = runtime_call(
        builder,
        "dr_v1_checked_transfer_group_observe",
        &parameter_types,
        Some(types::I8),
        &args,
        resources,
    )?
    .ok_or_else(|| backend_failure("completion observation produced no status"))?;
    let inspect = builder.create_block();
    let original = builder.ins().icmp_imm_u(IntCC::Equal, status, 2);
    builder.ins().brif(original, failure, &[], inspect, &[]);
    builder.switch_to_block(inspect);
    let store = builder.create_block();
    let ordinary_error = builder.create_block();
    let ok = builder.ins().icmp_imm_u(IntCC::Equal, status, 0);
    builder.ins().brif(ok, store, &[], ordinary_error, &[]);
    builder.switch_to_block(ordinary_error);
    // The shared resource finisher constructs the same ordinary IoError and
    // rejects invalid runtime status values; this path cannot succeed.
    finish_checked_resource(
        builder,
        &mir::CheckedIoOperation::ObserveTransferGroup {
            group: group_id,
            wait,
            outcome,
        },
        None,
        error,
        success,
        failure,
        status,
        slots,
        resources,
    )?;
    builder.switch_to_block(store);
    let result = result.ok_or_else(|| malformed_mir("completion observation has no result"))?;
    let address = builder
        .ins()
        .stack_addr(pointer, local_slot(resources.local_slots, result)?, 0);
    zero_inline_bytes(builder, address, outcome.size, pointer);
    let definition = enum_definition(resources.program, outcome.id)?;
    let result_tag = builder.ins().load(types::I8, flags, output[0], 0);
    builder.ins().store(
        flags,
        result_tag,
        address,
        definition.layout.tag_offset as i32,
    );
    let complete = builder.create_block();
    let completed = builder.ins().icmp_imm_u(IntCC::Equal, result_tag, 0);
    builder.ins().brif(completed, complete, &[], success, &[]);
    builder.switch_to_block(complete);
    let types = crate::compiler_known_stream::TransferObservationTypes::for_result(
        resources.program,
        outcome,
    )
    .ok_or_else(|| malformed_mir("completion observation has no typed schema"))?;
    let completion_definition = enum_definition(resources.program, types.completion.id)?;
    let completion = builder.ins().iadd_imm_u(
        address,
        i64::from(definition.layout.cases[0].fields[0].offset),
    );
    let reading = builder.ins().load(types::I8, flags, output[1], 0);
    let is_read = builder.ins().icmp_imm_u(IntCC::NotEqual, reading, 0);
    let blocks = [builder.create_block(), builder.create_block()];
    builder.ins().brif(is_read, blocks[0], &[], blocks[1], &[]);
    for (direction, block) in blocks.into_iter().enumerate() {
        builder.switch_to_block(block);
        store_payload_enum_tag(
            builder,
            completion,
            completion_definition.layout.tag_width,
            direction as u32,
        )?;
        let fields = &completion_definition.layout.cases[direction].fields;
        for word in [0i32, 8] {
            let value = builder.ins().load(types::I64, flags, output[2], word);
            builder
                .ins()
                .store(flags, value, completion, fields[0].offset as i32 + word);
        }
        let outcome_tag = builder.ins().load(types::I8, flags, output[3], 0);
        let present = builder.ins().icmp_imm_u(IntCC::NotEqual, outcome_tag, 0);
        let fill = builder.create_block();
        builder.ins().brif(present, fill, &[], success, &[]);
        builder.switch_to_block(fill);
        let present = builder.ins().iconst(types::I8, 1);
        builder
            .ins()
            .store(flags, present, completion, fields[1].offset as i32);
        let outcome_type = types.outcomes[direction];
        let inner = builder.ins().iadd_imm_u(
            completion,
            i64::from(fields[1].offset + outcome_type.nullable_payload_offset),
        );
        let inner_definition = enum_definition(resources.program, outcome_type.id)?;
        let tag = builder.ins().iadd_imm_u(outcome_tag, -1);
        builder
            .ins()
            .store(flags, tag, inner, inner_definition.layout.tag_offset as i32);
        let count = builder.ins().load(pointer, flags, output[4], 0);
        let count = if pointer == types::I64 {
            count
        } else {
            builder.ins().uextend(types::I64, count)
        };
        builder.ins().store(
            flags,
            count,
            inner,
            inner_definition.layout.cases[0].fields[0].offset as i32,
        );
        builder.ins().jump(success, &[]);
    }
    Ok(())
}

fn scratch(builder: &mut FunctionBuilder, pointer: ClifType, words: u32) -> Value {
    let slot = builder.create_sized_stack_slot(StackSlotData::new(
        StackSlotKind::ExplicitSlot,
        pointer.bytes() * words,
        pointer.bytes().ilog2() as u8,
    ));
    let address = builder.ins().stack_addr(pointer, slot, 0);
    zero_inline_bytes(builder, address, pointer.bytes() * words, pointer);
    address
}

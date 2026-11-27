//! The statements only a companion region entry contains (Decision 0148
//! section 23, entry ABI version 1): a host operation's zero-filled record,
//! a slot moved between a record and its local, and the call through
//! `DrRegionHostV1.call`. Content slots go through the runtime's slot
//! accessors, which own the content rules for both native backends.

use super::*;
use crate::native_abi::{
    RegionSlotForm, REGION_SLOT_LOAD_ARRAY, REGION_SLOT_LOAD_BYTES,
    REGION_SLOT_LOAD_FOREIGN_STRING, REGION_SLOT_LOAD_TEXT, REGION_SLOT_STORE_ARRAY,
    REGION_SLOT_STORE_BYTES, REGION_SLOT_STORE_FOREIGN_STRING, REGION_SLOT_STORE_TEXT,
};

const POINTER: mir::Type = mir::Type::Scalar(mir::ScalarType::Inline(
    crate::inline_value::InlineValueType::Pointer,
));

fn pointer_local(
    builder: &mut FunctionBuilder,
    local: mir::LocalId,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let slot = local_slot(resources.local_slots, local)?;
    load_lowered_from_stack(builder, resources.program, POINTER, slot, pointer).single()
}

/// `HostRecord`: `size` zero-filled bytes of this frame, aligned to `align`.
pub(super) fn host_record(
    builder: &mut FunctionBuilder,
    target: mir::LocalId,
    size: u64,
    align: u64,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let size = u32::try_from(size).map_err(|_| malformed_mir("a host record fits a frame"))?;
    let record = builder.create_sized_stack_slot(StackSlotData::new(
        StackSlotKind::ExplicitSlot,
        size.max(1),
        (align.max(1) as u32).trailing_zeros() as u8,
    ));
    let address = builder.ins().stack_addr(pointer, record, 0);
    zero_inline_bytes(builder, address, size, pointer);
    let slot = local_slot(resources.local_slots, target)?;
    store_lowered_to_stack(
        builder,
        resources.program,
        POINTER,
        slot,
        LoweredValue::Single(address),
        pointer,
    )
}

/// `RegionSlot`: moves `slot` between the record `record` addresses and
/// the slot's local.
pub(super) fn region_slot(
    builder: &mut FunctionBuilder,
    record: mir::LocalId,
    slot: mir::CompanionSlot,
    access: mir::SlotAccess,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let flags = cranelift_codegen::ir::MachMemFlags::trusted();
    let local = slot.value.local;
    let ty = local_definition(resources.program, resources.function_id, local)?.ty;
    let form = RegionSlotForm::of(slot, ty, resources.program)
        .ok_or_else(|| malformed_mir("a companion slot holds a value with a slot form"))?;
    let base = pointer_local(builder, record, resources)?;
    let address = builder.ins().iadd_imm_u(base, slot.offset as i64);
    let stack = local_slot(resources.local_slots, local)?;
    let frame = resources.current_frame;
    match (form, access) {
        (RegionSlotForm::Native, mir::SlotAccess::Store) => {
            let value = load_lowered_from_stack(builder, resources.program, ty, stack, pointer);
            store_lowered_to_address(builder, resources.program, ty, address, value, pointer)?;
        }
        (RegionSlotForm::Native, mir::SlotAccess::Load) => {
            let value = load_lowered_from_address(builder, resources.program, ty, address, pointer);
            store_lowered_to_stack(builder, resources.program, ty, stack, value, pointer)?;
        }
        (RegionSlotForm::Region { nullable }, mir::SlotAccess::Store) => {
            let zero = builder.ins().iconst(pointer, 0);
            let (present, data, length) =
                match load_lowered_from_stack(builder, resources.program, ty, stack, pointer) {
                    LoweredValue::ByteRegion { data, length } if !nullable => {
                        (builder.ins().iconst(types::I8, 1), data, length)
                    }
                    LoweredValue::NullableByteRegion {
                        present,
                        data,
                        length,
                    } if nullable => (
                        builder.ins().icmp_imm_u(IntCC::NotEqual, present, 0),
                        builder.ins().select(present, data, zero),
                        builder.ins().select(present, length, zero),
                    ),
                    _ => return Err(malformed_mir("a region slot's local holds a region")),
                };
            // A present region's data is never null, even when it is empty:
            // an empty one points at its slot.
            let empty = builder.ins().icmp_imm_u(IntCC::Equal, data, 0);
            let substitute = builder.ins().band(present, empty);
            let data = builder.ins().select(substitute, address, data);
            builder.ins().store(flags, data, address, 0);
            builder
                .ins()
                .store(flags, length, address, pointer.bytes() as i32);
        }
        (RegionSlotForm::Region { nullable }, mir::SlotAccess::Load) => {
            let data = builder.ins().load(pointer, flags, address, 0);
            let length = builder
                .ins()
                .load(pointer, flags, address, pointer.bytes() as i32);
            let value = if nullable {
                let present = builder.ins().icmp_imm_u(IntCC::NotEqual, data, 0);
                LoweredValue::NullableByteRegion {
                    present: builder.ins().uextend(pointer, present),
                    data,
                    length,
                }
            } else {
                LoweredValue::ByteRegion { data, length }
            };
            store_lowered_to_stack(builder, resources.program, ty, stack, value, pointer)?;
        }
        (form, access) => {
            let nullable = matches!(form, RegionSlotForm::Text { nullable: true });
            let current = if nullable {
                let (present, payload) =
                    load_lowered_from_stack(builder, resources.program, ty, stack, pointer)
                        .nullable()?;
                let zero = builder.ins().iconst(pointer, 0);
                builder.ins().select(present, payload, zero)
            } else {
                builder.ins().stack_load(pointer, pointer, stack, 0)
            };
            if access == mir::SlotAccess::Store {
                let (symbol, extra) = match form {
                    RegionSlotForm::Text { .. } => (REGION_SLOT_STORE_TEXT, None),
                    RegionSlotForm::Bytes => (REGION_SLOT_STORE_BYTES, None),
                    RegionSlotForm::Array(_) => (REGION_SLOT_STORE_ARRAY, None),
                    RegionSlotForm::ForeignString { unit_bytes } => {
                        (REGION_SLOT_STORE_FOREIGN_STRING, Some(unit_bytes))
                    }
                    RegionSlotForm::Native | RegionSlotForm::Region { .. } => unreachable!(),
                };
                let mut values = vec![address, current];
                values.extend(extra.map(|unit| builder.ins().iconst(pointer, unit as i64)));
                let params = vec![pointer; values.len()];
                runtime_call(builder, symbol, &params, None, &values, resources)?;
                return Ok(());
            }
            let loaded = match form {
                RegionSlotForm::Text { .. } => runtime_call(
                    builder,
                    REGION_SLOT_LOAD_TEXT,
                    &[pointer, pointer],
                    Some(pointer),
                    &[address, current],
                    resources,
                )?,
                RegionSlotForm::Bytes => runtime_call(
                    builder,
                    REGION_SLOT_LOAD_BYTES,
                    &[pointer, pointer, pointer],
                    Some(pointer),
                    &[frame, address, current],
                    resources,
                )?,
                RegionSlotForm::ForeignString { unit_bytes } => {
                    let unit = builder.ins().iconst(pointer, unit_bytes as i64);
                    runtime_call(
                        builder,
                        REGION_SLOT_LOAD_FOREIGN_STRING,
                        &[pointer, pointer, pointer, pointer],
                        Some(pointer),
                        &[frame, address, unit, current],
                        resources,
                    )?
                }
                RegionSlotForm::Array(collection) => {
                    // A fresh empty array of the local's type, filled from
                    // the slot unless it holds none; the replaced array is
                    // dropped as a reassignment drops it.
                    let data = builder.ins().load(pointer, flags, address, 0);
                    let present = builder.ins().icmp_imm_u(IntCC::NotEqual, data, 0);
                    let fill = builder.create_block();
                    let done = builder.create_block();
                    builder.append_block_param(done, pointer);
                    let zero = builder.ins().iconst(pointer, 0);
                    builder
                        .ins()
                        .brif(present, fill, &[], done, &[BlockArg::Value(zero)]);
                    builder.switch_to_block(fill);
                    let array = lower_collection_expression(
                        builder,
                        &mir::CollectionExpression::Literal {
                            collection,
                            entries: Vec::new(),
                        },
                        resources,
                    )?;
                    runtime_call(
                        builder,
                        REGION_SLOT_LOAD_ARRAY,
                        &[pointer, pointer, pointer],
                        None,
                        &[frame, address, array],
                        resources,
                    )?;
                    builder.ins().jump(done, &[BlockArg::Value(array)]);
                    builder.switch_to_block(done);
                    let array = builder.block_params(done)[0];
                    lower_drop_collection_value(builder, current, collection, resources)?;
                    Some(array)
                }
                RegionSlotForm::Native | RegionSlotForm::Region { .. } => unreachable!(),
            }
            .ok_or_else(|| backend_failure("a slot load produced no value"))?;
            if nullable {
                let present = builder.ins().icmp_imm_u(IntCC::NotEqual, loaded, 0);
                let value = LoweredValue::Nullable {
                    present: builder.ins().uextend(pointer, present),
                    payload: loaded,
                };
                store_lowered_to_stack(builder, resources.program, ty, stack, value, pointer)?;
            } else {
                builder.ins().stack_store(pointer, loaded, stack, 0);
            }
        }
    }
    Ok(())
}

/// `CallHost`: `host->call(host, operation, arguments, results)`, an
/// indirect C call through the function pointer at offset 0 of the host.
/// Its status is always 0 in version 1, and the companion does not read it.
pub(super) fn call_host(
    builder: &mut FunctionBuilder,
    host: Option<mir::LocalId>,
    operation: u32,
    arguments: Option<mir::LocalId>,
    results: Option<mir::LocalId>,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let flags = cranelift_codegen::ir::MachMemFlags::trusted();
    let host = match host {
        Some(host) => pointer_local(builder, host, resources)?,
        None => runtime_call(
            builder,
            crate::native_abi::REGION_HOST,
            &[],
            Some(pointer),
            &[],
            resources,
        )?
        .ok_or_else(|| backend_failure("the attached host is a pointer"))?,
    };
    let mut record = |local: Option<mir::LocalId>, builder: &mut FunctionBuilder| match local {
        Some(local) => pointer_local(builder, local, resources),
        None => Ok(builder.ins().iconst(pointer, 0)),
    };
    let arguments = record(arguments, builder)?;
    let results = record(results, builder)?;
    let call = builder.ins().load(pointer, flags, host, 0);
    let mut signature = resources.module.make_signature();
    signature.params = vec![
        AbiParam::new(pointer),
        AbiParam::new(types::I32),
        AbiParam::new(pointer),
        AbiParam::new(pointer),
    ];
    signature.returns = vec![AbiParam::new(types::I32)];
    let signature = builder.import_signature(signature);
    let operation = builder.ins().iconst(types::I32, i64::from(operation));
    builder
        .ins()
        .call_indirect(signature, call, &[host, operation, arguments, results]);
    Ok(())
}

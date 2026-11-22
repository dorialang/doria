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

impl<'ctx> FunctionLowerer<'ctx, '_> {
    fn companion_pointer(
        &mut self,
        local: mir::LocalId,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let address = self.pointer_local(local)?;
        build(self.builder.build_int_to_ptr(
            address,
            self.context.ptr_type(AddressSpace::default()),
            "companion.record",
        ))
    }

    /// `HostRecord`: `size` zero-filled bytes of this frame, aligned to
    /// `align`.
    pub(super) fn lower_host_record(
        &mut self,
        target: mir::LocalId,
        size: u64,
        align: u64,
    ) -> Result<(), BackendError> {
        let bytes = self.context.i8_type().array_type(size.max(1) as u32);
        let record = self.entry_alloca(bytes, "companion.host-record")?;
        let alignment = align.max(1) as u32;
        record
            .as_instruction()
            .ok_or_else(|| backend_failure("a host record is an alloca"))?
            .set_alignment(alignment)
            .map_err(|error| backend_failure(error.to_string()))?;
        self.zero_memory_value(
            record,
            native_closure_abi::NativeLayout {
                size: size as u32,
                align: alignment,
            },
        )?;
        let address = build(self.builder.build_ptr_to_int(
            record,
            self.context.i64_type(),
            "companion.host-record.address",
        ))?;
        build(
            self.builder
                .build_store(local_slot(&self.local_slots, target)?, address),
        )?;
        Ok(())
    }

    /// `RegionSlot`: moves `slot` between the record `record` addresses and
    /// the slot's local.
    pub(super) fn lower_region_slot(
        &mut self,
        record: mir::LocalId,
        slot: mir::CompanionSlot,
        access: mir::SlotAccess,
    ) -> Result<(), BackendError> {
        let local = slot.value.local;
        let ty = local_in(self.function, local)?.ty;
        let form = RegionSlotForm::of(slot, ty, self.program)
            .ok_or_else(|| malformed_mir("a companion slot holds a value with a slot form"))?;
        let base = self.companion_pointer(record)?;
        let address = self.byte_offset(base, slot.offset as u32, "companion.slot")?;
        let storage = local_slot(&self.local_slots, local)?;
        let pointer = self.context.ptr_type(AddressSpace::default());
        let word = self.context.ptr_sized_int_type(self.target_data, None);
        let stored = llvm_type(self.context, self.target_data, self.program, ty);
        match (form, access) {
            (RegionSlotForm::Native, access) => {
                if let Some(layout) = native_closure_abi::memory_value_layout(ty) {
                    return match access {
                        mir::SlotAccess::Store => self.copy_memory_value(address, storage, layout),
                        mir::SlotAccess::Load => self.copy_memory_value(storage, address, layout),
                    };
                }
                let (from, to) = match access {
                    mir::SlotAccess::Store => (storage, address),
                    mir::SlotAccess::Load => (address, storage),
                };
                let value = build(self.builder.build_load(stored, from, "companion.native"))?;
                build(self.builder.build_store(to, value))?;
            }
            (RegionSlotForm::Region { nullable }, mir::SlotAccess::Store) => {
                let value = build(self.builder.build_load(stored, storage, "companion.region"))?
                    .into_struct_value();
                let (present, region) = if nullable {
                    let present = build(self.builder.build_extract_value(value, 0, "present"))?
                        .into_int_value();
                    let present = build(self.builder.build_int_compare(
                        IntPredicate::NE,
                        present,
                        word.const_zero(),
                        "companion.region.present",
                    ))?;
                    let region = build(self.builder.build_extract_value(value, 1, "region"))?
                        .into_struct_value();
                    (present, region)
                } else {
                    (self.context.bool_type().const_int(1, false), value)
                };
                let data = build(self.builder.build_extract_value(region, 0, "data"))?
                    .into_pointer_value();
                let length =
                    build(self.builder.build_extract_value(region, 1, "length"))?.into_int_value();
                // An absent region writes no data and no length; a present
                // one never writes null data, even when it is empty: an
                // empty one points at its slot.
                let null = build(self.builder.build_is_null(data, "companion.region.null"))?;
                let substitute = build(self.builder.build_and(present, null, "substitute"))?;
                let data = build(self.builder.build_select(substitute, address, data, "data"))?;
                let data = build(self.builder.build_select(
                    present,
                    data,
                    pointer.const_null().into(),
                    "data",
                ))?;
                let length =
                    build(
                        self.builder
                            .build_select(present, length, word.const_zero(), "length"),
                    )?;
                build(self.builder.build_store(address, data))?;
                let length_address =
                    self.byte_offset(address, word.get_bit_width() / 8, "companion.length")?;
                build(self.builder.build_store(length_address, length))?;
            }
            (RegionSlotForm::Region { nullable }, mir::SlotAccess::Load) => {
                let data = build(self.builder.build_load(pointer, address, "data"))?;
                let length_address =
                    self.byte_offset(address, word.get_bit_width() / 8, "companion.length")?;
                let length = build(self.builder.build_load(word, length_address, "length"))?;
                let region = byte_region_type(self.context, self.target_data);
                let mut value = region.get_undef();
                value = build(self.builder.build_insert_value(value, data, 0, "data"))?
                    .into_struct_value();
                value = build(self.builder.build_insert_value(value, length, 1, "length"))?
                    .into_struct_value();
                if nullable {
                    let present =
                        build(self.builder.build_is_not_null(
                            data.into_pointer_value(),
                            "companion.region.present",
                        ))?;
                    let present = build(self.builder.build_int_z_extend(present, word, "present"))?;
                    let nullable = stored.into_struct_type().get_undef();
                    let nullable = build(
                        self.builder
                            .build_insert_value(nullable, present, 0, "present"),
                    )?;
                    let nullable = build(
                        self.builder
                            .build_insert_value(nullable, value, 1, "region"),
                    )?;
                    build(self.builder.build_store(storage, nullable))?;
                } else {
                    build(self.builder.build_store(storage, value))?;
                }
            }
            (form, access) => {
                let nullable = matches!(form, RegionSlotForm::Text { nullable: true });
                let current = if nullable {
                    let value = build(self.builder.build_load(stored, storage, "companion.text"))?
                        .into_struct_value();
                    let present = build(self.builder.build_extract_value(value, 0, "present"))?
                        .into_int_value();
                    let present = build(self.builder.build_int_compare(
                        IntPredicate::NE,
                        present,
                        word.const_zero(),
                        "present",
                    ))?;
                    let payload = build(self.builder.build_extract_value(value, 1, "payload"))?;
                    build(self.builder.build_select(
                        present,
                        payload,
                        pointer.const_null().into(),
                        "companion.current",
                    ))?
                    .into_pointer_value()
                } else {
                    build(
                        self.builder
                            .build_load(pointer, storage, "companion.current"),
                    )?
                    .into_pointer_value()
                };
                let frame = self.current_frame;
                if access == mir::SlotAccess::Store {
                    let (symbol, unit) = match form {
                        RegionSlotForm::Text { .. } => (REGION_SLOT_STORE_TEXT, None),
                        RegionSlotForm::Bytes => (REGION_SLOT_STORE_BYTES, None),
                        RegionSlotForm::Array(_) => (REGION_SLOT_STORE_ARRAY, None),
                        RegionSlotForm::ForeignString { unit_bytes } => {
                            (REGION_SLOT_STORE_FOREIGN_STRING, Some(unit_bytes))
                        }
                        RegionSlotForm::Native | RegionSlotForm::Region { .. } => unreachable!(),
                    };
                    let mut params: Vec<BasicMetadataTypeEnum<'ctx>> =
                        vec![pointer.into(), pointer.into()];
                    let mut values: Vec<BasicMetadataValueEnum<'ctx>> =
                        vec![address.into(), current.into()];
                    if let Some(unit) = unit {
                        params.push(word.into());
                        values.push(word.const_int(unit, false).into());
                    }
                    let _ = self.call_runtime(symbol, &params, None, &values)?;
                    return Ok(());
                }
                let loaded = match form {
                    RegionSlotForm::Text { .. } => self.call_runtime(
                        REGION_SLOT_LOAD_TEXT,
                        &[pointer.into(), pointer.into()],
                        Some(pointer.into()),
                        &[address.into(), current.into()],
                    )?,
                    RegionSlotForm::Bytes => self.call_runtime(
                        REGION_SLOT_LOAD_BYTES,
                        &[pointer.into(), pointer.into(), pointer.into()],
                        Some(pointer.into()),
                        &[frame.into(), address.into(), current.into()],
                    )?,
                    RegionSlotForm::ForeignString { unit_bytes } => self.call_runtime(
                        REGION_SLOT_LOAD_FOREIGN_STRING,
                        &[pointer.into(), pointer.into(), word.into(), pointer.into()],
                        Some(pointer.into()),
                        &[
                            frame.into(),
                            address.into(),
                            word.const_int(unit_bytes, false).into(),
                            current.into(),
                        ],
                    )?,
                    RegionSlotForm::Array(collection) => {
                        // A fresh empty array of the local's type, filled
                        // from the slot unless it holds none; the replaced
                        // array is dropped as a reassignment drops it.
                        let data = build(self.builder.build_load(pointer, address, "data"))?
                            .into_pointer_value();
                        let present = build(self.builder.build_is_not_null(data, "present"))?;
                        let function = current_function(&self.builder)?;
                        let before = self
                            .builder
                            .get_insert_block()
                            .ok_or_else(|| backend_failure("a slot load has a block"))?;
                        let fill = self.context.append_basic_block(function, "companion.fill");
                        let done = self
                            .context
                            .append_basic_block(function, "companion.filled");
                        build(self.builder.build_conditional_branch(present, fill, done))?;
                        self.builder.position_at_end(fill);
                        let array = self.lower_collection_expression(
                            &mir::CollectionExpression::Literal {
                                collection,
                                entries: Vec::new(),
                            },
                        )?;
                        let _ = self.call_runtime(
                            REGION_SLOT_LOAD_ARRAY,
                            &[pointer.into(), pointer.into(), pointer.into()],
                            None,
                            &[frame.into(), address.into(), array.into()],
                        )?;
                        let filled = self
                            .builder
                            .get_insert_block()
                            .ok_or_else(|| backend_failure("a slot load has a block"))?;
                        build(self.builder.build_unconditional_branch(done))?;
                        self.builder.position_at_end(done);
                        let phi = build(self.builder.build_phi(pointer, "companion.array"))?;
                        phi.add_incoming(&[(&pointer.const_null(), before), (&array, filled)]);
                        self.drop_collection_value(current, collection)?;
                        Some(phi.as_basic_value())
                    }
                    RegionSlotForm::Native | RegionSlotForm::Region { .. } => unreachable!(),
                }
                .ok_or_else(|| backend_failure("a slot load produced no value"))?
                .into_pointer_value();
                if nullable {
                    let present = build(self.builder.build_is_not_null(loaded, "present"))?;
                    let present = build(self.builder.build_int_z_extend(present, word, "present"))?;
                    let value = stored.into_struct_type().get_undef();
                    let value = build(
                        self.builder
                            .build_insert_value(value, present, 0, "present"),
                    )?;
                    let value =
                        build(self.builder.build_insert_value(value, loaded, 1, "payload"))?;
                    build(self.builder.build_store(storage, value))?;
                } else {
                    build(self.builder.build_store(storage, loaded))?;
                }
            }
        }
        Ok(())
    }

    /// `CallHost`: `host->call(host, operation, arguments, results)`, an
    /// indirect C call through the function pointer at offset 0 of the
    /// host. Its status is always 0 in version 1, and the companion does
    /// not read it.
    pub(super) fn lower_call_host(
        &mut self,
        host: Option<mir::LocalId>,
        operation: u32,
        arguments: Option<mir::LocalId>,
        results: Option<mir::LocalId>,
    ) -> Result<(), BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let int32 = self.context.i32_type();
        let host = match host {
            Some(host) => self.companion_pointer(host)?,
            None => self
                .call_runtime(
                    crate::native_abi::REGION_HOST,
                    &[],
                    Some(pointer.into()),
                    &[],
                )?
                .ok_or_else(|| backend_failure("the attached host is a pointer"))?
                .into_pointer_value(),
        };
        let mut record = |local: Option<mir::LocalId>| match local {
            Some(local) => self.companion_pointer(local),
            None => Ok(pointer.const_null()),
        };
        let arguments = record(arguments)?;
        let results = record(results)?;
        let call = build(
            self.builder
                .build_load(pointer, host, "companion.host.call"),
        )?
        .into_pointer_value();
        let signature = int32.fn_type(
            &[pointer.into(), int32.into(), pointer.into(), pointer.into()],
            false,
        );
        let _ = build(self.builder.build_indirect_call(
            signature,
            call,
            &[
                host.into(),
                int32.const_int(u64::from(operation), false).into(),
                arguments.into(),
                results.into(),
            ],
            "companion.host.status",
        ))?;
        Ok(())
    }
}

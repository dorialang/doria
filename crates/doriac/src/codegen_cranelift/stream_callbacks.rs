//! C-ABI adapters around ordinary checked Doria interface entries. The retained
//! receiver and all callback homes are borrowed; this layer creates no owner.

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

pub(super) struct Callback {
    plan: mir::StreamCallbackPlan,
    function: FuncId,
}

pub(super) enum CallTarget {
    Static(cranelift_codegen::ir::FuncRef),
    Dynamic(Value),
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

fn signature(module: &ObjectModule, operation: Op) -> Signature {
    let pointer = module.target_config().pointer_type();
    let mut signature = module.make_signature();
    // Resolver: frame, retained receiver home, source, context, original error.
    // Transfer: frame, context, [request], data, length, outcome, original error.
    signature.params = vec![AbiParam::new(pointer); argument_count(operation)];
    signature.returns.push(AbiParam::new(types::I8).uext());
    signature
}

/// Called only for the types encountered in validated owning registration MIR.
/// Merely declaring a class or constructing an empty set emits no thunks.
pub(super) fn declare(
    module: &mut ObjectModule,
    program: &mir::Program,
) -> Result<Vec<Callback>, BackendError> {
    let mut callbacks: Vec<Callback> = Vec::new();
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
                let function = module
                    .declare_function(
                        &symbol(receiver, operation)?,
                        Linkage::Local,
                        &signature(module, operation),
                    )
                    .map_err(|error| backend_failure(error.to_string()))?;
                callbacks.push(Callback { plan, function });
            }
        }
    }
    Ok(callbacks)
}

pub(super) fn address(
    builder: &mut FunctionBuilder,
    module: &mut ObjectModule,
    program: &mir::Program,
    receiver: mir::Type,
    operation: Op,
) -> Result<Option<Value>, BackendError> {
    if !OPERATIONS.contains(&operation) || program.stream_callback(receiver, operation).is_none() {
        return Ok(None);
    }
    let function = module
        .declare_function(
            &symbol(receiver, operation)?,
            Linkage::Local,
            &signature(module, operation),
        )
        .map_err(|error| backend_failure(error.to_string()))?;
    let function = module.declare_func_in_func(function, builder.func);
    Ok(Some(builder.ins().func_addr(
        module.target_config().pointer_type(),
        function,
    )))
}

/// Shared with the registration getter call. The selected method view is only
/// borrowed dispatch metadata; it never replaces the retained original carrier.
#[allow(clippy::too_many_arguments)]
pub(super) fn resolve_target(
    builder: &mut FunctionBuilder,
    module: &mut ObjectModule,
    program: &mir::Program,
    functions: &[FuncId],
    statics: &[DataId],
    plan: mir::StreamCallbackPlan,
    descriptor: Value,
) -> Result<(CallTarget, Value), BackendError> {
    let pointer = module.target_config().pointer_type();
    let flags = cranelift_codegen::ir::MachMemFlags::trusted();
    let vtable = match plan.target {
        mir::StreamCallbackTarget::Static { vtable, entry } => {
            let id = *statics
                .get(program.statics.len() + vtable.0)
                .ok_or_else(|| malformed_mir("stream callback vtable is not declared"))?;
            let data = module.declare_data_in_func(id, builder.func);
            let vtable = builder.ins().symbol_value(pointer, data);
            let function = *functions
                .get(entry.0)
                .ok_or_else(|| malformed_mir("stream callback entry is not declared"))?;
            return Ok((
                CallTarget::Static(module.declare_func_in_func(function, builder.func)),
                vtable,
            ));
        }
        mir::StreamCallbackTarget::ClassDescriptor => {
            let views = builder.ins().load(
                pointer,
                flags,
                descriptor,
                (pointer.bytes() * crate::native_abi::CLASS_DESCRIPTOR_INTERFACE_WORD) as i32,
            );
            builder.ins().load(
                pointer,
                flags,
                views,
                word_offset(pointer, plan.interface.0)?,
            )
        }
        mir::StreamCallbackTarget::InterfaceDescriptor => descriptor,
    };
    let entry = builder.ins().load(
        pointer,
        flags,
        vtable,
        word_offset(
            pointer,
            crate::native_abi::INTERFACE_VTABLE_HEADER_WORDS + plan.slot,
        )?,
    );
    Ok((CallTarget::Dynamic(entry), vtable))
}

fn word_offset(pointer: ClifType, word: usize) -> Result<i32, BackendError> {
    word.checked_mul(pointer.bytes() as usize)
        .and_then(|offset| i32::try_from(offset).ok())
        .ok_or_else(|| malformed_mir("stream callback offset exceeds native address range"))
}

pub(super) fn define(
    module: &mut ObjectModule,
    program: &mir::Program,
    callbacks: &[Callback],
    functions: &[FuncId],
    statics: &[DataId],
) -> Result<(), BackendError> {
    for callback in callbacks {
        let mut context = module.make_context();
        context.func.signature = signature(module, callback.plan.operation);
        let mut builder_context = FunctionBuilderContext::new();
        {
            let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_context);
            emit(
                &mut builder,
                module,
                program,
                callback.plan,
                functions,
                statics,
            )?;
            builder.seal_all_blocks();
            builder.finalize(module.target_config());
        }
        module
            .define_function(callback.function, &mut context)
            .map_err(|error| backend_failure(format!("stream callback: {error:?}")))?;
        module.clear_context(&mut context);
    }
    Ok(())
}

fn scratch(builder: &mut FunctionBuilder, pointer: ClifType, size: u32, align: u32) -> Value {
    let slot = builder.create_sized_stack_slot(StackSlotData::new(
        StackSlotKind::ExplicitSlot,
        size,
        align.trailing_zeros() as u8,
    ));
    builder.ins().stack_addr(pointer, slot, 0)
}

#[allow(clippy::too_many_arguments)]
fn emit(
    builder: &mut FunctionBuilder,
    module: &mut ObjectModule,
    program: &mir::Program,
    plan: mir::StreamCallbackPlan,
    functions: &[FuncId],
    statics: &[DataId],
) -> Result<(), BackendError> {
    let entry = builder.create_block();
    builder.append_block_params_for_function_params(entry);
    builder.switch_to_block(entry);
    let args = builder.block_params(entry).to_vec();
    let pointer = module.target_config().pointer_type();
    let flags = cranelift_codegen::ir::MachMemFlags::trusted();
    let resolver = plan.operation == Op::ReadinessSource;
    let is_pending = pending(plan.operation);
    let base = usize::from(is_pending);
    let zero = builder.ins().iconst(pointer, 0);
    let (data, length, outcome, error) = if resolver {
        (zero, zero, args[2], args[4])
    } else {
        (
            args[2 + base],
            args[3 + base],
            args[4 + base],
            args[5 + base],
        )
    };
    let zero_byte = builder.ins().iconst(types::I8, 0);
    builder.ins().store(flags, zero, outcome, 0);
    if !resolver {
        builder
            .ins()
            .store(flags, zero_byte, outcome, pointer.bytes() as i32);
        builder
            .ins()
            .store(flags, zero_byte, outcome, pointer.bytes() as i32 + 1);
    }
    let object = builder.ins().load(pointer, flags, args[1], 0);
    let mut descriptor = builder
        .ins()
        .load(pointer, flags, args[1], pointer.bytes() as i32);
    // Retained owners use the complete interface carrier. Transfer contexts
    // use the concrete descriptor expected by the ordinary dispatch plan.
    if resolver
        && matches!(
            plan.receiver,
            mir::Type::Class(_) | mir::Type::Scalar(mir::ScalarType::Inline(_))
        )
    {
        descriptor = builder.ins().load(
            pointer,
            flags,
            descriptor,
            (pointer.bytes() * crate::native_abi::INTERFACE_VTABLE_CLASS_DESCRIPTOR_WORD) as i32,
        );
    }
    let contract = function_type_in(program, plan.signature)?;
    let abi = native_closure_abi::NativeCallableSignaturePlan::interface_entry(contract);
    let source_type = builder.ins().iconst(
        pointer,
        native_closure_abi::borrow_home_type_key(if resolver {
            mir::Type::Interface(plan.interface)
        } else {
            plan.receiver
        }) as i64,
    );
    let home = super::borrow_home_descriptor(
        builder,
        super::BorrowHome {
            address: args[1],
            source_type,
        },
        pointer,
    );
    let result = match contract.return_type {
        mir::ReturnType::Void => None,
        mir::ReturnType::Value(ty) => {
            let layout = native_closure_abi::value_layout(program, ty, pointer.bytes());
            Some(scratch(builder, pointer, layout.size, layout.align))
        }
    };
    let mut values = Vec::new();
    for input in &abi.hidden_inputs {
        values.push(match *input {
            native_closure_abi::NativeCallableHiddenInput::CurrentFrame => args[0],
            native_closure_abi::NativeCallableHiddenInput::ResultOut => {
                result.ok_or_else(|| malformed_mir("stream callback has no result home"))?
            }
            native_closure_abi::NativeCallableHiddenInput::ErrorOut => error,
            native_closure_abi::NativeCallableHiddenInput::ParameterBorrowHome(0) => home,
            native_closure_abi::NativeCallableHiddenInput::ResultBorrowHomeOut if resolver => {
                let slot = scratch(builder, pointer, pointer.bytes() * 2, pointer.bytes());
                builder.ins().store(flags, zero, slot, 0);
                builder
                    .ins()
                    .store(flags, zero, slot, pointer.bytes() as i32);
                slot
            }
            native_closure_abi::NativeCallableHiddenInput::ParameterBorrowHome(parameter) => {
                let ty = contract.parameters[parameter].ty;
                // The C bridge supplies value carriers, just like evaluated
                // temporary arguments of an ordinary Doria call.
                let carrier = match ty {
                    mir::Type::Class(_) if is_pending && parameter == 1 => {
                        let slot = scratch(builder, pointer, pointer.bytes(), pointer.bytes());
                        builder.ins().store(flags, args[2], slot, 0);
                        slot
                    }
                    mir::Type::ByteRegion(_) => {
                        let slot = scratch(builder, pointer, pointer.bytes() * 2, pointer.bytes());
                        builder.ins().store(flags, data, slot, 0);
                        builder
                            .ins()
                            .store(flags, length, slot, pointer.bytes() as i32);
                        slot
                    }
                    _ => return Err(malformed_mir("stream callback has no input carrier")),
                };
                let source_type = builder
                    .ins()
                    .iconst(pointer, native_closure_abi::borrow_home_type_key(ty) as i64);
                super::borrow_home_descriptor(
                    builder,
                    super::BorrowHome {
                        address: carrier,
                        source_type,
                    },
                    pointer,
                )
            }
            _ => {
                return Err(malformed_mir(
                    "transfer callback has an unexpected borrowed result",
                ))
            }
        });
    }
    let (target, vtable) = resolve_target(
        builder, module, program, functions, statics, plan, descriptor,
    )?;
    values.extend([object, vtable]);
    if is_pending {
        values.push(args[2]);
    }
    if !resolver {
        values.extend([data, length]);
    }
    let call = match target {
        CallTarget::Static(function) => builder.ins().call(function, &values),
        CallTarget::Dynamic(function) => {
            let signature = indirect_function_signature(module, program, contract, false);
            let signature = builder.import_signature(signature);
            builder.ins().call_indirect(signature, function, &values)
        }
    };
    if abi.checked {
        let status = builder.inst_results(call)[0];
        let success = builder.create_block();
        let not_success = builder.create_block();
        let failed = builder.create_block();
        let invalid = builder.create_block();
        let ok = builder.ins().icmp_imm_u(IntCC::Equal, status, 0);
        builder.ins().brif(ok, success, &[], not_success, &[]);
        builder.switch_to_block(not_success);
        let failure = builder.ins().icmp_imm_u(IntCC::Equal, status, 1);
        builder.ins().brif(failure, failed, &[], invalid, &[]);
        builder.switch_to_block(invalid);
        let invalid_status = builder.ins().iconst(types::I8, 3);
        builder.ins().return_(&[invalid_status]);
        builder.switch_to_block(failed);
        if !resolver {
            project_error_progress(builder, pointer, error, outcome, length)?;
        }
        let failed_status = builder.ins().iconst(types::I8, 1);
        builder.ins().return_(&[failed_status]);
        builder.switch_to_block(success);
    }
    if resolver {
        let source = if abi.checked {
            builder.ins().load(
                pointer,
                flags,
                result.ok_or_else(|| malformed_mir("readiness resolver has no result"))?,
                0,
            )
        } else {
            builder.inst_results(call)[0]
        };
        builder.ins().store(flags, source, outcome, 0);
        builder.ins().store(flags, object, args[3], 0);
        builder
            .ins()
            .store(flags, descriptor, args[3], pointer.bytes() as i32);
    } else if let Some(result) = result {
        project_outcome(
            builder,
            program,
            pointer,
            contract.return_type,
            result,
            outcome,
        )?;
    }
    let ok = builder.ins().iconst(types::I8, 0);
    builder.ins().return_(&[ok]);
    Ok(())
}

fn project_outcome(
    builder: &mut FunctionBuilder,
    program: &mir::Program,
    pointer: ClifType,
    return_type: mir::ReturnType,
    mut result: Value,
    outcome: Value,
) -> Result<(), BackendError> {
    let flags = cranelift_codegen::ir::MachMemFlags::trusted();
    let ty = match return_type {
        mir::ReturnType::Value(mir::Type::PayloadEnum(ty)) => ty,
        mir::ReturnType::Value(mir::Type::NullablePayloadEnum(ty)) => {
            let present = builder.ins().load(types::I8, flags, result, 0);
            let yes = builder.create_block();
            let no = builder.create_block();
            let present = builder.ins().icmp_imm_u(IntCC::NotEqual, present, 0);
            builder.ins().brif(present, yes, &[], no, &[]);
            builder.switch_to_block(no);
            let absent = builder.ins().iconst(types::I8, 4);
            builder.ins().return_(&[absent]);
            builder.switch_to_block(yes);
            result = builder
                .ins()
                .iadd_imm_u(result, i64::from(ty.nullable_payload_offset));
            ty
        }
        _ => {
            return Err(malformed_mir(
                "stream callback result is not an inline outcome",
            ))
        }
    };
    let definition = enum_definition(program, ty.id)?;
    let tag_type = match definition.layout.tag_width {
        1 => types::I8,
        2 => types::I16,
        4 => types::I32,
        _ => return Err(malformed_mir("stream outcome tag width is invalid")),
    };
    let tag = builder
        .ins()
        .load(tag_type, flags, result, definition.layout.tag_offset as i32);
    let mut runtime_tag = builder.ins().iconst(types::I8, 255);
    for (index, case) in definition.cases.iter().enumerate() {
        let selected = builder
            .ins()
            .icmp_imm_u(IntCC::Equal, tag, i64::from(case.tag));
        let projected = builder.ins().iconst(types::I8, index as i64);
        runtime_tag = builder.ins().select(selected, projected, runtime_tag);
    }
    builder
        .ins()
        .store(flags, runtime_tag, outcome, pointer.bytes() as i32);
    let data = builder.create_block();
    let done = builder.create_block();
    let has_count = builder.ins().icmp_imm_u(IntCC::Equal, runtime_tag, 0);
    builder.ins().brif(has_count, data, &[], done, &[]);
    builder.switch_to_block(data);
    let count = builder.ins().load(
        types::I64,
        flags,
        result,
        definition.layout.cases[0].fields[0].offset as i32,
    );
    let valid = count_fits(builder, pointer, count);
    let copy = builder.create_block();
    let invalid = builder.create_block();
    builder.ins().brif(valid, copy, &[], invalid, &[]);
    builder.switch_to_block(invalid);
    let invalid_status = builder.ins().iconst(types::I8, 3);
    builder.ins().return_(&[invalid_status]);
    builder.switch_to_block(copy);
    let count = if pointer == types::I64 {
        count
    } else {
        builder.ins().ireduce(pointer, count)
    };
    let one = builder.ins().iconst(types::I8, 1);
    builder.ins().store(flags, count, outcome, 0);
    builder
        .ins()
        .store(flags, one, outcome, pointer.bytes() as i32 + 1);
    builder.ins().jump(done, &[]);
    builder.switch_to_block(done);
    Ok(())
}

fn count_fits(builder: &mut FunctionBuilder, pointer: ClifType, count: Value) -> Value {
    let nonnegative = builder
        .ins()
        .icmp_imm_s(IntCC::SignedGreaterThanOrEqual, count, 0);
    if pointer == types::I64 {
        return nonnegative;
    }
    let fits = builder
        .ins()
        .icmp_imm_u(IntCC::UnsignedLessThanOrEqual, count, i64::from(u32::MAX));
    builder.ins().band(nonnegative, fits)
}

fn project_error_progress(
    builder: &mut FunctionBuilder,
    pointer: ClifType,
    error: Value,
    outcome: Value,
    length: Value,
) -> Result<(), BackendError> {
    let flags = cranelift_codegen::ir::MachMemFlags::trusted();
    let descriptor = builder
        .ins()
        .load(pointer, flags, error, pointer.bytes() as i32);
    let magic = builder.ins().load(
        pointer,
        flags,
        descriptor,
        (pointer.bytes() * crate::native_abi::INTERFACE_VTABLE_IO_PROGRESS_MAGIC_WORD) as i32,
    );
    let is_io = builder.ins().icmp_imm_u(
        IntCC::Equal,
        magic,
        crate::native_abi::IO_ERROR_PROGRESS_DESCRIPTOR_MAGIC as i64,
    );
    let inspect = builder.create_block();
    let done = builder.create_block();
    builder.ins().brif(is_io, inspect, &[], done, &[]);
    builder.switch_to_block(inspect);
    let object = builder.ins().load(pointer, flags, error, 0);
    let offset = builder.ins().load(
        pointer,
        flags,
        descriptor,
        (pointer.bytes() * crate::native_abi::INTERFACE_VTABLE_IO_TRANSFERRED_COUNT_WORD) as i32,
    );
    let property = builder.ins().iadd(object, offset);
    let present = builder.ins().load(pointer, flags, property, 0);
    let present = builder.ins().icmp_imm_u(IntCC::NotEqual, present, 0);
    let read = builder.create_block();
    builder.ins().brif(present, read, &[], done, &[]);
    builder.switch_to_block(read);
    let layout = native_closure_abi::type_layout(
        mir::Type::NullableScalar(mir::ScalarType::Integer(IntegerType::Int64)),
        pointer.bytes(),
    );
    let count = builder
        .ins()
        .load(types::I64, flags, property, layout.align as i32);
    let fits = count_fits(builder, pointer, count);
    let bound = if pointer == types::I64 {
        length
    } else {
        builder.ins().uextend(types::I64, length)
    };
    let bounded = builder
        .ins()
        .icmp(IntCC::UnsignedLessThanOrEqual, count, bound);
    let valid = builder.ins().band(fits, bounded);
    let copy = builder.create_block();
    builder.ins().brif(valid, copy, &[], done, &[]);
    builder.switch_to_block(copy);
    let count = if pointer == types::I64 {
        count
    } else {
        builder.ins().ireduce(pointer, count)
    };
    let one = builder.ins().iconst(types::I8, 1);
    builder.ins().store(flags, count, outcome, 0);
    builder
        .ins()
        .store(flags, one, outcome, pointer.bytes() as i32 + 1);
    builder.ins().jump(done, &[]);
    builder.switch_to_block(done);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> mir::Program {
        crate::lower_source_to_mir(
            "stream-callback-emitters.doria",
            include_str!("../../tests/fixtures/stream_callback_emitters.doria"),
        )
        .unwrap()
    }

    fn module() -> ObjectModule {
        let isa = cranelift_native::builder()
            .unwrap()
            .finish(settings::Flags::new(settings::builder()))
            .unwrap();
        ObjectModule::new(
            ObjectBuilder::new(isa, "callback-tests", default_libcall_names()).unwrap(),
        )
    }

    #[test]
    fn unregistered_types_and_empty_readiness_construction_emit_no_callbacks() {
        let program = fixture();
        assert!(declare(&mut module(), &program).unwrap().is_empty());
        let empty = crate::lower_source_to_mir("empty-callbacks.doria",
            "function main(): void throws Doria\\Std\\Io\\IoError { let $set = new Doria\\Std\\Io\\ReadinessSet(); }").unwrap();
        assert!(declare(&mut module(), &empty).unwrap().is_empty());
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
        let mut module = module();
        let functions = program
            .functions
            .iter()
            .map(|function| {
                let signature = function_signature(&mut module, &program, function).unwrap();
                module
                    .declare_function(&function_symbol(function), Linkage::Local, &signature)
                    .unwrap()
            })
            .collect::<Vec<_>>();
        let mut statics = program
            .statics
            .iter()
            .map(|item| {
                module
                    .declare_data(
                        &format!("test_static_{}", item.id.0),
                        Linkage::Local,
                        false,
                        false,
                    )
                    .unwrap()
            })
            .collect::<Vec<_>>();
        statics.extend(
            program
                .interface_vtables
                .iter()
                .map(|table| declare_interface_vtable(&mut module, table.id).unwrap()),
        );
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
                let sig = signature(&module, operation);
                assert_eq!(sig.params.len(), argument_count(operation));
                let function = module
                    .declare_function(&symbol(receiver, operation).unwrap(), Linkage::Local, &sig)
                    .unwrap();
                let mut context = module.make_context();
                context.func.signature = sig;
                let mut builder_context = FunctionBuilderContext::new();
                {
                    let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_context);
                    emit(
                        &mut builder,
                        &mut module,
                        &program,
                        plan,
                        &functions,
                        &statics,
                    )
                    .unwrap();
                    builder.seal_all_blocks();
                    builder.finalize(module.target_config());
                }
                let ir = context.func.to_string();
                assert_eq!(
                    ir.contains("call_indirect"),
                    receiver != closed,
                    "{operation:?}: {ir}"
                );
                // No per-transfer owner, wrapper or buffer allocation calls.
                assert_eq!(
                    ir.matches(" = call ").count() + ir.matches(" = call_indirect ").count(),
                    1,
                    "{ir}"
                );
                module.define_function(function, &mut context).unwrap();
                module.clear_context(&mut context);
            }
        }
    }
}

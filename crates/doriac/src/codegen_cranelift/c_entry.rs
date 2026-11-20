//! C entries of C-ABI Doria functions (Decision 0148 sections 12 and 13).
//!
//! Each C entry has the function's C signature under the target's C
//! convention, the inverse of an outbound foreign call: it receives every
//! value where the shared classification (`native_abi::c_abi`) places it,
//! rebuilds the Doria values, and calls the function as Doria code does.
//! Before the call, `dr_v1_enter_c_function` checks the thread (P1706) and
//! links the function's frame to the outbound foreign call in progress.

use super::*;
use crate::c_layout::CScalar;
use crate::native_abi::c_abi::{
    self, Extension, PartClass, PartType, PhysicalParameter, PhysicalReturn, PhysicalSignature,
};
use cranelift_codegen::ir::ArgumentPurpose;

/// The symbol of the C entry of `function`.
pub(super) fn symbol(function: mir::FunctionId) -> String {
    format!("__doria_c_entry_{}", function.0)
}

fn entry_frame_symbol(function: mir::FunctionId) -> String {
    format!("__doria_c_entry_frame_{}", function.0)
}

/// The C layout of a C entry's parameters and result, and their placement.
pub(super) fn physical(
    program: &mir::Program,
    function: &mir::Function,
) -> Result<(Vec<mir::Type>, PhysicalSignature), BackendError> {
    let target = crate::c_layout::CTarget::host();
    let c_form = |ty| {
        mir::foreign_c_type(ty, &program.foreign_aggregates, target)
            .ok_or_else(|| malformed_mir("a C entry passes only values with a C form"))
    };
    let types = function
        .params
        .iter()
        .map(|local| local_in(function, *local).map(|local| local.ty))
        .collect::<Result<Vec<_>, _>>()?;
    let parameters = types
        .iter()
        .map(|ty| c_form(*ty))
        .collect::<Result<Vec<_>, _>>()?;
    let result = match function.return_type {
        mir::ReturnType::Value(ty) => Some(c_form(ty)?),
        mir::ReturnType::Void => None,
    };
    let classification = c_abi::classify(target, &parameters, result.as_ref(), None);
    Ok((
        types,
        c_abi::physical_signature(target, &parameters, &classification),
    ))
}

fn clif_scalar(scalar: CScalar, pointer: ClifType) -> ClifType {
    match scalar {
        CScalar::Integer { bytes: 1, .. } | CScalar::Bool => types::I8,
        CScalar::Integer { bytes: 2, .. } => types::I16,
        CScalar::Integer { bytes: 4, .. } => types::I32,
        CScalar::Float { bytes: 4 } => types::F32,
        CScalar::Float { .. } => types::F64,
        CScalar::Integer { .. } | CScalar::Pointer => pointer,
    }
}

fn clif_part(ty: PartType) -> ClifType {
    match ty {
        PartType::Integer64 => types::I64,
        PartType::Float32 => types::F32,
        PartType::Float64 => types::F64,
    }
}

/// The Cranelift signature of a C function the classification placed as
/// `physical`. Outbound foreign calls and C entries share it.
pub(super) fn c_signature(module: &ObjectModule, physical: &PhysicalSignature) -> Signature {
    let pointer = module.target_config().pointer_type();
    let mut signature = module.make_signature();
    for parameter in &physical.parameters {
        signature.params.push(match parameter {
            PhysicalParameter::ResultAddress => {
                AbiParam::special(pointer, ArgumentPurpose::StructReturn)
            }
            PhysicalParameter::Scalar { scalar, .. } => {
                let parameter = AbiParam::new(clif_scalar(*scalar, pointer));
                match c_abi::extension(*scalar) {
                    Some(Extension::Sign) => parameter.sext(),
                    Some(Extension::Zero) => parameter.uext(),
                    None => parameter,
                }
            }
            PhysicalParameter::Part { ty, .. } => AbiParam::new(clif_part(*ty)),
            PhysicalParameter::Padding(PartClass::Integer) => AbiParam::new(types::I64),
            PhysicalParameter::Padding(PartClass::Float) => AbiParam::new(types::F64),
            PhysicalParameter::StackImage { size, .. } => {
                AbiParam::special(pointer, ArgumentPurpose::StructArgument(*size as u32))
            }
            PhysicalParameter::CopyAddress { .. } => AbiParam::new(pointer),
        });
    }
    match &physical.return_value {
        // The bits above a narrow result are not relied on.
        PhysicalReturn::Scalar(scalar) => signature
            .returns
            .push(AbiParam::new(clif_scalar(*scalar, pointer))),
        PhysicalReturn::Parts(parts) => signature
            .returns
            .extend(parts.iter().map(|(_, ty)| AbiParam::new(clif_part(*ty)))),
        PhysicalReturn::Void | PhysicalReturn::Indirect => {}
    }
    signature
}

/// Declares the C entry of every C-ABI function of `program`.
pub(super) fn declare(
    module: &mut ObjectModule,
    program: &mir::Program,
) -> Result<Vec<(mir::FunctionId, FuncId)>, BackendError> {
    program
        .c_entries
        .iter()
        .map(|id| {
            let function = function_in(program, *id)?;
            let (_, physical) = physical(program, function)?;
            let signature = c_signature(module, &physical);
            let entry = module
                .declare_function(&symbol(*id), Linkage::Local, &signature)
                .map_err(|error| backend_failure(error.to_string()))?;
            Ok((*id, entry))
        })
        .collect()
}

/// The address of the C entry of `function`, a C function pointer value.
pub(super) fn address(
    builder: &mut FunctionBuilder,
    function: mir::FunctionId,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let Some(cranelift_module::FuncOrDataId::Func(id)) =
        resources.module.get_name(&symbol(function))
    else {
        return Err(malformed_mir("a C entry address names a declared C entry"));
    };
    let pointer = resources.module.target_config().pointer_type();
    let reference = resources.module.declare_func_in_func(id, builder.func);
    Ok(builder.ins().func_addr(pointer, reference))
}

/// The address of the runtime's record of the outbound foreign call in
/// progress (`dr_v1_foreign_call_frame`).
pub(super) fn foreign_call_record(
    builder: &mut FunctionBuilder,
    module: &mut ObjectModule,
) -> Result<Value, BackendError> {
    let id = module
        .declare_data(
            crate::native_abi::FOREIGN_CALL_FRAME,
            Linkage::Import,
            true,
            false,
        )
        .map_err(|error| backend_failure(error.to_string()))?;
    let global = module.declare_data_in_func(id, builder.func);
    let pointer = module.target_config().pointer_type();
    Ok(builder.ins().symbol_value(pointer, global))
}

/// Defines the frame that describes `function` for a P1706 report: its name
/// and declaration, with no parent. It reuses the name and source data the
/// function's own frame refers to.
fn define_entry_frame(
    module: &mut ObjectModule,
    program: &mir::Program,
    function: &mir::Function,
) -> Result<DataId, BackendError> {
    let width = usize::from(module.target_config().pointer_bytes());
    let callable_name = program.diagnostic_callable_name(function);
    let source = program
        .source_for_span(function.source_span)
        .unwrap_or(&program.source);
    let mut words = vec![0_u64; 11];
    words[2] = callable_name.len() as u64;
    words[4] = source.path.len() as u64;
    words[6] = source.text.len() as u64;
    words[7] = function.source_span.start as u64;
    words[8] = function.source_span.end as u64;
    words[9] = function.source_span.start as u64;
    words[10] = function.source_span.end as u64;
    let mut bytes = Vec::with_capacity(width * words.len());
    for word in words {
        append_target_word(&mut bytes, word, width);
    }
    let mut description = DataDescription::new();
    description.set_align(width as u64);
    description.define(bytes.into_boxed_slice());
    for (word, data) in [
        (1, frame_data_symbols(function.id).name),
        (3, frame_data_symbols(function.id).path),
        (5, frame_data_symbols(function.id).text),
    ] {
        let Some(cranelift_module::FuncOrDataId::Data(id)) = module.get_name(&data) else {
            return Err(malformed_mir(
                "a C entry's function defines its own diagnostic frame data",
            ));
        };
        let reference = module.declare_data_in_data(id, &mut description);
        description.write_data_addr((word * width) as u32, reference, 0);
    }
    let id = module
        .declare_data(
            &entry_frame_symbol(function.id),
            Linkage::Local,
            false,
            false,
        )
        .map_err(|error| backend_failure(error.to_string()))?;
    module
        .define_data(id, &description)
        .map_err(|error| backend_failure(error.to_string()))?;
    Ok(id)
}

/// Defines every declared C entry, after the functions they call.
pub(super) fn define(
    module: &mut ObjectModule,
    program: &mir::Program,
    entries: &[(mir::FunctionId, FuncId)],
    function_ids: &[FuncId],
) -> Result<(), BackendError> {
    for (id, entry) in entries {
        let function = function_in(program, *id)?;
        let entry_frame = define_entry_frame(module, program, function)?;
        let callee = *function_ids
            .get(id.0)
            .ok_or_else(|| malformed_mir("a C entry's function was not declared"))?;
        let mut context = module.make_context();
        context.func.signature = module
            .declarations()
            .get_function_decl(*entry)
            .signature
            .clone();
        let mut builder_context = FunctionBuilderContext::new();
        {
            let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_context);
            emit(&mut builder, module, program, function, entry_frame, callee)?;
            builder.seal_all_blocks();
            builder.finalize(module.target_config());
        }
        module
            .define_function(*entry, &mut context)
            .map_err(|error| backend_failure(format!("{error:?}")))?;
        module.clear_context(&mut context);
    }
    Ok(())
}

fn slot(builder: &mut FunctionBuilder, size: u32, align: u32, pointer: ClifType) -> Value {
    let slot = builder.create_sized_stack_slot(StackSlotData::new(
        StackSlotKind::ExplicitSlot,
        size.max(1),
        align.max(1).trailing_zeros() as u8,
    ));
    builder.ins().stack_addr(pointer, slot, 0)
}

fn emit(
    builder: &mut FunctionBuilder,
    module: &mut ObjectModule,
    program: &mir::Program,
    function: &mir::Function,
    entry_frame: DataId,
    callee: FuncId,
) -> Result<(), BackendError> {
    let pointer = module.target_config().pointer_type();
    let flags = MemFlagsData::trusted();
    let (types, physical) = physical(program, function)?;
    let block = builder.create_block();
    builder.append_block_params_for_function_params(block);
    builder.switch_to_block(block);
    let received = builder.block_params(block).to_vec();

    // The thread check and the frame link (section 13).
    let marker = slot(builder, pointer.bytes() * 11, pointer.bytes(), pointer);
    let entry_frame = module.declare_data_in_func(entry_frame, builder.func);
    let entry_frame = builder.ins().symbol_value(pointer, entry_frame);
    let mut enter = module.make_signature();
    enter.params = vec![AbiParam::new(pointer), AbiParam::new(pointer)];
    enter.returns = vec![AbiParam::new(pointer)];
    let enter = module
        .declare_function(crate::native_abi::ENTER_C_FUNCTION, Linkage::Import, &enter)
        .map_err(|error| backend_failure(error.to_string()))?;
    let enter = module.declare_func_in_func(enter, builder.func);
    let call = builder.ins().call(enter, &[marker, entry_frame]);
    let parent = builder.inst_results(call)[0];

    // Each aggregate argument, rebuilt from the registers or memory C
    // passed it in.
    let mut aggregates: HashMap<usize, Value> = HashMap::new();
    let mut scalars: HashMap<usize, Value> = HashMap::new();
    let mut result_address = None;
    for (parameter, value) in physical.parameters.iter().zip(&received) {
        match parameter {
            PhysicalParameter::ResultAddress => result_address = Some(*value),
            PhysicalParameter::Scalar { argument, .. } => {
                scalars.insert(*argument, *value);
            }
            PhysicalParameter::Part {
                argument, offset, ..
            } => {
                let mir::Type::ForeignAggregate(aggregate) = types[*argument] else {
                    return Err(malformed_mir("register parts carry a foreign aggregate"));
                };
                let size = c_abi::padded_image_size(u64::from(aggregate.size)) as u32;
                let image = *aggregates
                    .entry(*argument)
                    .or_insert_with(|| slot(builder, size, 8, pointer));
                builder.ins().store(flags, *value, image, *offset as i32);
            }
            PhysicalParameter::Padding(_) => {}
            // C passed the address of its own copy.
            PhysicalParameter::StackImage { argument, .. }
            | PhysicalParameter::CopyAddress { argument } => {
                aggregates.insert(*argument, *value);
            }
        }
    }

    let plan = native_closure_abi::NativeCallableSignaturePlan::direct(function);
    let mut values = Vec::new();
    let mut result_slot = None;
    for input in &plan.hidden_inputs {
        values.push(match input {
            native_closure_abi::NativeCallableHiddenInput::CurrentFrame => parent,
            native_closure_abi::NativeCallableHiddenInput::ResultOut => {
                let mir::ReturnType::Value(ty) = function.return_type else {
                    return Err(malformed_mir("a result slot holds a value"));
                };
                let layout = native_closure_abi::value_layout(program, ty, pointer.bytes());
                let address = slot(builder, layout.size, layout.align, pointer);
                result_slot = Some(address);
                address
            }
            _ => {
                return Err(malformed_mir(
                    "a C entry's function takes only its frame and result slot",
                ))
            }
        });
    }
    for (index, ty) in types.iter().enumerate() {
        if let mir::Type::ForeignAggregate(_) = ty {
            values.push(
                *aggregates
                    .get(&index)
                    .ok_or_else(|| malformed_mir("C passed every aggregate argument"))?,
            );
            continue;
        }
        let value = *scalars
            .get(&index)
            .ok_or_else(|| malformed_mir("C passed every scalar argument"))?;
        match ty {
            // C `_Bool` is 0 or 1; any other byte still reads as `true`.
            mir::Type::Scalar(mir::ScalarType::Bool) => {
                let value = builder.ins().icmp_imm_u(IntCC::NotEqual, value, 0);
                values.push(builder.ins().uextend(types::I32, value));
            }
            mir::Type::Scalar(mir::ScalarType::Float(FloatType::Float32)) => {
                values.push(
                    builder
                        .ins()
                        .bitcast(types::I32, MemFlagsData::new(), value),
                );
            }
            // A null `?Ptr<T>` is address zero.
            mir::Type::NullableScalar(_) => {
                let present = builder.ins().icmp_imm_u(IntCC::NotEqual, value, 0);
                values.push(builder.ins().uextend(pointer, present));
                values.push(value);
            }
            _ => values.push(value),
        }
    }

    let callee = module.declare_func_in_func(callee, builder.func);
    let call = builder.ins().call(callee, &values);
    let results = builder.inst_results(call).to_vec();
    let mir::ReturnType::Value(ty) = function.return_type else {
        builder.ins().return_(&[]);
        return Ok(());
    };
    match &physical.return_value {
        PhysicalReturn::Indirect => {
            let mir::Type::ForeignAggregate(aggregate) = ty else {
                return Err(malformed_mir("an indirect C result is a foreign aggregate"));
            };
            let source = result_slot.ok_or_else(|| malformed_mir("an aggregate has a slot"))?;
            let destination =
                result_address.ok_or_else(|| malformed_mir("C passed the result address"))?;
            copy_foreign_aggregate(
                builder,
                destination,
                source,
                aggregate,
                AggregateAccess::Storage,
                pointer,
            );
            builder.ins().return_(&[]);
        }
        PhysicalReturn::Parts(parts) => {
            let mir::Type::ForeignAggregate(aggregate) = ty else {
                return Err(malformed_mir("register parts return a foreign aggregate"));
            };
            let source = result_slot.ok_or_else(|| malformed_mir("an aggregate has a slot"))?;
            let size = c_abi::padded_image_size(u64::from(aggregate.size)) as u32;
            let image = slot(builder, size, 8, pointer);
            let zero = builder.ins().iconst(types::I64, 0);
            for offset in (0..size).step_by(8) {
                builder.ins().store(flags, zero, image, offset as i32);
            }
            copy_foreign_aggregate(
                builder,
                image,
                source,
                aggregate,
                AggregateAccess::Storage,
                pointer,
            );
            let values = parts
                .iter()
                .map(|(offset, part)| {
                    builder
                        .ins()
                        .load(clif_part(*part), flags, image, *offset as i32)
                })
                .collect::<Vec<_>>();
            builder.ins().return_(&values);
        }
        PhysicalReturn::Scalar(scalar) => {
            let value = *results
                .first()
                .ok_or_else(|| malformed_mir("a C entry's function returns its value"))?;
            let value = match (ty, scalar) {
                (mir::Type::Scalar(mir::ScalarType::Bool), _) => {
                    builder.ins().ireduce(types::I8, value)
                }
                (mir::Type::Scalar(mir::ScalarType::Float(FloatType::Float32)), _) => builder
                    .ins()
                    .bitcast(types::F32, MemFlagsData::new(), value),
                // An absent `?Ptr<T>` is address zero.
                (mir::Type::NullableScalar(_), _) => {
                    let payload = *results
                        .get(1)
                        .ok_or_else(|| malformed_mir("a nullable result has its payload"))?;
                    let null = builder.ins().iconst(pointer, 0);
                    builder.ins().select(value, payload, null)
                }
                _ => value,
            };
            builder.ins().return_(&[value]);
        }
        PhysicalReturn::Void => {
            return Err(malformed_mir("a C entry with a result returns it"));
        }
    }
    Ok(())
}

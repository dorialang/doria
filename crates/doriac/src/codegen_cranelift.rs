use std::collections::{HashMap, HashSet};

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::immediates::{Ieee32, Ieee64};
use cranelift_codegen::ir::{
    types, AbiParam, Block, BlockArg, InstBuilder, Signature, StackSlot, StackSlotData,
    StackSlotKind, TrapCode, Type as ClifType, Value,
};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{default_libcall_names, DataDescription, DataId, FuncId, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};

use crate::backend::BackendError;
use crate::format_string::{FormatConversion, FormatPiece};
use crate::mir;
use crate::mir_validation;
use crate::native_abi::{
    function_symbol, CLASS_ALLOCATE, CLASS_FREE, FORMAT_F32, FORMAT_F64, FORMAT_I64, FORMAT_STRING,
    FORMAT_U64, NULLABLE_STRING_EQUAL, READ_FILE, READ_STDIN_LINE, STRING_COMPARE, STRING_CONCAT,
    STRING_DATA, STRING_FROM_BOOL, STRING_FROM_F32, STRING_FROM_F64, STRING_FROM_I64,
    STRING_FROM_U64, STRING_FROM_UTF8, STRING_LENGTH, STRING_RELEASE, STRING_RETAIN,
    STRING_WRITE_STDERR, STRING_WRITE_STDOUT, WRITE_FILE,
};
use crate::numeric::{FloatType, FloatValue, IntegerPanic, IntegerType, IntegerValue};

const RUNTIME_RETURNED_TRAP: u8 = 1;

pub fn lower_mir_to_object(program: &mir::Program) -> Result<Vec<u8>, BackendError> {
    mir_validation::validate_program(program)?;

    let isa_builder =
        cranelift_native::builder().map_err(|error| backend_failure(error.to_string()))?;
    let mut flag_builder = settings::builder();
    flag_builder
        .set("is_pic", "true")
        .map_err(|error| backend_failure(error.to_string()))?;
    let isa = isa_builder
        .finish(settings::Flags::new(flag_builder))
        .map_err(|error| backend_failure(error.to_string()))?;
    let mut module = ObjectModule::new(
        ObjectBuilder::new(isa, "doria_stage_13", default_libcall_names())
            .map_err(|error| backend_failure(error.to_string()))?,
    );

    let mut function_ids = Vec::with_capacity(program.functions.len());
    for function in &program.functions {
        let signature = function_signature(&mut module, function)?;
        let function_id = module
            .declare_function(&function_symbol(function), Linkage::Local, &signature)
            .map_err(|error| backend_failure(error.to_string()))?;
        function_ids.push(function_id);
    }
    let static_ids = define_static_data(&mut module, program)?;

    let mut process_signature = module.make_signature();
    process_signature.returns.push(AbiParam::new(types::I32));
    let process_main_id = module
        .declare_function("main", Linkage::Export, &process_signature)
        .map_err(|error| backend_failure(error.to_string()))?;

    for function in &program.functions {
        define_function(&mut module, program, function, &function_ids, &static_ids)?;
    }
    define_process_main(
        &mut module,
        program,
        process_main_id,
        &process_signature,
        &function_ids,
    )?;

    module
        .finish()
        .emit()
        .map_err(|error| backend_failure(error.to_string()))
}

fn function_signature(
    module: &mut ObjectModule,
    function: &mir::Function,
) -> Result<Signature, BackendError> {
    let mut signature = module.make_signature();
    signature
        .params
        .push(AbiParam::new(module.target_config().pointer_type()));
    for parameter in &function.params {
        append_type_abi_params(
            &mut signature.params,
            local_in(function, *parameter)?.ty,
            module.target_config().pointer_type(),
        );
    }
    if let mir::ReturnType::Value(ty) = function.return_type {
        append_type_abi_params(
            &mut signature.returns,
            ty,
            module.target_config().pointer_type(),
        );
    }
    Ok(signature)
}

fn clif_integer_type(ty: IntegerType) -> ClifType {
    match ty.bit_width() {
        8 => types::I8,
        16 => types::I16,
        32 => types::I32,
        64 => types::I64,
        width => unreachable!("canonical Doria integer has unsupported width {width}"),
    }
}

fn integer_abi_param(ty: IntegerType) -> AbiParam {
    let parameter = AbiParam::new(clif_integer_type(ty));
    if ty.bit_width() == 64 {
        parameter
    } else if ty.is_signed() {
        parameter.sext()
    } else {
        parameter.uext()
    }
}

fn clif_scalar_type(ty: mir::ScalarType) -> ClifType {
    match ty {
        mir::ScalarType::Integer(ty) => clif_integer_type(ty),
        mir::ScalarType::Float(FloatType::Float32) => types::F32,
        mir::ScalarType::Float(FloatType::Float64) => types::F64,
        mir::ScalarType::Bool => types::I8,
    }
}

fn scalar_abi_param(ty: mir::ScalarType) -> AbiParam {
    match ty {
        mir::ScalarType::Integer(ty) => integer_abi_param(ty),
        _ => AbiParam::new(clif_scalar_type(ty)),
    }
}

fn append_type_abi_params(params: &mut Vec<AbiParam>, ty: mir::Type, pointer_type: ClifType) {
    match ty {
        mir::Type::Scalar(ty) => params.push(scalar_abi_param(ty)),
        mir::Type::String | mir::Type::Class(_) | mir::Type::NullableClass(_) => {
            params.push(AbiParam::new(pointer_type));
        }
        mir::Type::NullableScalar(ty) => {
            params.push(AbiParam::new(pointer_type));
            params.push(scalar_abi_param(ty));
        }
        mir::Type::NullableString => {
            params.push(AbiParam::new(pointer_type));
            params.push(AbiParam::new(pointer_type));
        }
    }
}

#[derive(Clone, Copy)]
enum LoweredValue {
    Single(Value),
    Nullable { present: Value, payload: Value },
}

impl LoweredValue {
    fn single(self) -> Result<Value, BackendError> {
        match self {
            Self::Single(value) => Ok(value),
            Self::Nullable { .. } => Err(malformed_mir("expected a single backend value")),
        }
    }

    fn nullable(self) -> Result<(Value, Value), BackendError> {
        match self {
            Self::Nullable { present, payload } => Ok((present, payload)),
            Self::Single(_) => Err(malformed_mir("expected a nullable backend value")),
        }
    }

    fn append_to(self, values: &mut Vec<Value>) {
        match self {
            Self::Single(value) => values.push(value),
            Self::Nullable { present, payload } => {
                values.push(present);
                values.push(payload);
            }
        }
    }
}

fn scalar_storage_bytes(ty: mir::ScalarType) -> u32 {
    match ty {
        mir::ScalarType::Integer(ty) => ty.storage_bytes(),
        mir::ScalarType::Float(ty) => ty.storage_bytes(),
        mir::ScalarType::Bool => 1,
    }
}

fn define_static_data(
    module: &mut ObjectModule,
    program: &mir::Program,
) -> Result<Vec<DataId>, BackendError> {
    let pointer_bytes = usize::from(module.target_config().pointer_bytes());
    let mut ids = Vec::with_capacity(program.statics.len());
    for property in &program.statics {
        let symbol = format!(
            "__doria_static_{}_{}_{}",
            property.class.0, property.id.0, property.name
        );
        let id = module
            .declare_data(&symbol, Linkage::Local, property.writable, false)
            .map_err(|error| backend_failure(error.to_string()))?;
        let mut description = DataDescription::new();
        description.set_align(pointer_bytes as u64);
        match &property.initializer {
            mir::StaticValue::Scalar(value) => {
                let scalar = scalar_data_bytes(*value);
                if matches!(property.ty, mir::Type::NullableScalar(_)) {
                    let mut bytes = Vec::with_capacity(pointer_bytes * 2);
                    append_target_word(&mut bytes, 1, pointer_bytes);
                    bytes.extend_from_slice(&scalar);
                    bytes.resize(pointer_bytes * 2, 0);
                    description.define(bytes.into_boxed_slice());
                } else {
                    description.define(scalar.into_boxed_slice());
                }
            }
            mir::StaticValue::Null => {
                let bytes = if matches!(
                    property.ty,
                    mir::Type::NullableScalar(_) | mir::Type::NullableString
                ) {
                    pointer_bytes * 2
                } else {
                    pointer_bytes
                };
                description.define_zeroinit(bytes);
            }
            mir::StaticValue::String(value) => {
                let object_id = module
                    .declare_data(&format!("{symbol}_string"), Linkage::Local, false, false)
                    .map_err(|error| backend_failure(error.to_string()))?;
                let mut object = DataDescription::new();
                object.set_align(pointer_bytes as u64);
                let mut bytes = Vec::with_capacity(pointer_bytes * 2 + value.len());
                append_target_word(&mut bytes, u64::MAX, pointer_bytes);
                append_target_word(&mut bytes, value.len() as u64, pointer_bytes);
                bytes.extend_from_slice(value.as_bytes());
                object.define(bytes.into_boxed_slice());
                module
                    .define_data(object_id, &object)
                    .map_err(|error| backend_failure(error.to_string()))?;

                // A relocated pointer slot is initialized data, even though its
                // placeholder bytes are zero. Marking it zeroinit places the
                // relocation in Mach-O __bss, which Apple linkers cannot handle.
                let pointer_offset = if matches!(property.ty, mir::Type::NullableString) {
                    let mut bytes = Vec::with_capacity(pointer_bytes * 2);
                    append_target_word(&mut bytes, 1, pointer_bytes);
                    bytes.resize(pointer_bytes * 2, 0);
                    description.define(bytes.into_boxed_slice());
                    pointer_bytes
                } else {
                    description.define(vec![0; pointer_bytes].into_boxed_slice());
                    0
                };
                let object_reference = module.declare_data_in_data(object_id, &mut description);
                description.write_data_addr(pointer_offset as u32, object_reference, 0);
            }
        }
        module
            .define_data(id, &description)
            .map_err(|error| backend_failure(error.to_string()))?;
        ids.push(id);
    }
    Ok(ids)
}

fn append_target_word(bytes: &mut Vec<u8>, value: u64, width: usize) {
    let encoded = value.to_ne_bytes();
    if cfg!(target_endian = "little") {
        bytes.extend_from_slice(&encoded[..width]);
    } else {
        bytes.extend_from_slice(&encoded[encoded.len() - width..]);
    }
}

fn scalar_data_bytes(value: mir::ScalarValue) -> Vec<u8> {
    match value {
        mir::ScalarValue::Integer(value) => match value.ty.bit_width() {
            8 => vec![value.bits as u8],
            16 => (value.bits as u16).to_ne_bytes().to_vec(),
            32 => (value.bits as u32).to_ne_bytes().to_vec(),
            64 => value.bits.to_ne_bytes().to_vec(),
            width => unreachable!("canonical Doria integer has unsupported width {width}"),
        },
        mir::ScalarValue::Float(value) => match value.ty {
            FloatType::Float32 => (value.bits as u32).to_ne_bytes().to_vec(),
            FloatType::Float64 => value.bits.to_ne_bytes().to_vec(),
        },
        mir::ScalarValue::Bool(value) => vec![u8::from(value)],
    }
}

fn define_function(
    module: &mut ObjectModule,
    program: &mir::Program,
    function: &mir::Function,
    function_ids: &[FuncId],
    static_ids: &[DataId],
) -> Result<(), BackendError> {
    let function_id = *function_ids
        .get(function.id.0)
        .ok_or_else(|| malformed_mir(format!("function{} was not declared", function.id.0)))?;
    let signature = function_signature(module, function)?;
    let mut context = module.make_context();
    context.func.signature = signature;
    let mut builder_context = FunctionBuilderContext::new();

    {
        let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let blocks = function
            .blocks
            .iter()
            .map(|_| builder.create_block())
            .collect::<Vec<_>>();
        let entry = block_for(&blocks, function.entry_block)?;
        builder.append_block_params_for_function_params(entry);

        let local_slots = function
            .locals
            .iter()
            .map(|local| match local.ty {
                mir::Type::Scalar(ty) => {
                    let bytes = scalar_storage_bytes(ty);
                    Some(builder.create_sized_stack_slot(StackSlotData::new(
                        StackSlotKind::ExplicitSlot,
                        bytes,
                        bytes.trailing_zeros() as u8,
                    )))
                }
                mir::Type::String | mir::Type::Class(_) | mir::Type::NullableClass(_) => {
                    Some(builder.create_sized_stack_slot(StackSlotData::new(
                        StackSlotKind::ExplicitSlot,
                        u32::from(module.target_config().pointer_bytes()),
                        module.target_config().pointer_bytes().trailing_zeros() as u8,
                    )))
                }
                mir::Type::NullableScalar(_) | mir::Type::NullableString => {
                    let pointer_bytes = u32::from(module.target_config().pointer_bytes());
                    Some(builder.create_sized_stack_slot(StackSlotData::new(
                        StackSlotKind::ExplicitSlot,
                        pointer_bytes * 2,
                        pointer_bytes.trailing_zeros() as u8,
                    )))
                }
            })
            .collect::<Vec<_>>();
        let pointer_type = module.target_config().pointer_type();
        let pointer_bytes = pointer_type.bytes();
        let deferred_class_temporary_slots = (0..mir::class_temporary_capacity(function))
            .map(|_| {
                builder.create_sized_stack_slot(StackSlotData::new(
                    StackSlotKind::ExplicitSlot,
                    pointer_bytes,
                    pointer_bytes.trailing_zeros() as u8,
                ))
            })
            .collect::<Vec<_>>();
        let frame_slot = builder.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot,
            pointer_bytes * 3,
            pointer_bytes.trailing_zeros() as u8,
        ));

        builder.switch_to_block(entry);
        initialize_locals(&mut builder, function, &local_slots, pointer_type)?;
        let zero = builder.ins().iconst(pointer_type, 0);
        for slot in &deferred_class_temporary_slots {
            builder.ins().stack_store(zero, *slot, 0);
        }
        bind_parameters(&mut builder, function, &local_slots, entry)?;
        let parent_frame = builder.block_params(entry)[0];
        let function_name = define_named_data(
            &mut builder,
            function.name.as_bytes(),
            module,
            &format!("__doria_function_name_{}", function.id.0),
        )?;
        builder.ins().stack_store(parent_frame, frame_slot, 0);
        builder
            .ins()
            .stack_store(function_name, frame_slot, pointer_bytes as i32);
        let function_name_length = builder
            .ins()
            .iconst(pointer_type, function.name.len() as i64);
        builder
            .ins()
            .stack_store(function_name_length, frame_slot, (pointer_bytes * 2) as i32);
        let current_frame = builder.ins().stack_addr(pointer_type, frame_slot, 0);

        let mut resources = LoweringResources {
            module,
            program,
            function_ids,
            static_ids,
            local_slots: &local_slots,
            deferred_class_temporary_slots,
            deferred_class_temporary_slot_cursor: 0,
            write_stdout_func_id: None,
            panic_func_id: None,
            runtime_functions: HashMap::new(),
            next_data_id: 0,
            function_id: function.id,
            current_frame,
            defer_class_temporary_drops: false,
            deferred_class_temporary_drops: Vec::new(),
        };
        retain_string_parameters(&mut builder, function, &mut resources)?;
        lower_block(
            &mut builder,
            &function.blocks[function.entry_block.0],
            &blocks,
            &mut resources,
        )?;
        for (block_index, mir_block) in function.blocks.iter().enumerate() {
            if block_index == function.entry_block.0 {
                continue;
            }
            builder.switch_to_block(blocks[block_index]);
            lower_block(&mut builder, mir_block, &blocks, &mut resources)?;
        }

        builder.seal_all_blocks();
        builder.finalize();
    }

    module
        .define_function(function_id, &mut context)
        .map_err(|error| backend_failure(error.to_string()))?;
    module.clear_context(&mut context);
    Ok(())
}

fn initialize_locals(
    builder: &mut FunctionBuilder,
    function: &mir::Function,
    slots: &[Option<StackSlot>],
    pointer_type: ClifType,
) -> Result<(), BackendError> {
    for local in &function.locals {
        let zero = match local.ty {
            mir::Type::Scalar(mir::ScalarType::Integer(ty)) => {
                builder.ins().iconst(clif_integer_type(ty), 0)
            }
            mir::Type::Scalar(mir::ScalarType::Float(FloatType::Float32)) => {
                builder.ins().f32const(Ieee32::with_bits(0))
            }
            mir::Type::Scalar(mir::ScalarType::Float(FloatType::Float64)) => {
                builder.ins().f64const(Ieee64::with_bits(0))
            }
            mir::Type::Scalar(mir::ScalarType::Bool) => builder.ins().iconst(types::I8, 0),
            mir::Type::String | mir::Type::Class(_) | mir::Type::NullableClass(_) => {
                builder.ins().iconst(pointer_type, 0)
            }
            mir::Type::NullableScalar(_) | mir::Type::NullableString => {
                let zero = builder.ins().iconst(pointer_type, 0);
                let slot = local_slot(slots, local.id)?;
                builder
                    .ins()
                    .stack_store(zero, slot, pointer_type.bytes() as i32);
                zero
            }
        };
        builder
            .ins()
            .stack_store(zero, local_slot(slots, local.id)?, 0);
    }
    Ok(())
}

fn bind_parameters(
    builder: &mut FunctionBuilder,
    function: &mir::Function,
    slots: &[Option<StackSlot>],
    entry: Block,
) -> Result<(), BackendError> {
    let params = builder.block_params(entry).to_vec();
    let mut params = params.into_iter().skip(1);
    for parameter in &function.params {
        let slot = local_slot(slots, *parameter)?;
        let ty = local_in(function, *parameter)?.ty;
        let first = params
            .next()
            .ok_or_else(|| malformed_mir("function parameter is missing an ABI value"))?;
        builder.ins().stack_store(first, slot, 0);
        if matches!(ty, mir::Type::NullableScalar(_) | mir::Type::NullableString) {
            let payload = params.next().ok_or_else(|| {
                malformed_mir("nullable function parameter is missing its ABI payload")
            })?;
            let payload_offset = builder.func.dfg.value_type(first).bytes() as i32;
            builder.ins().stack_store(payload, slot, payload_offset);
        }
    }
    Ok(())
}

fn retain_string_parameters(
    builder: &mut FunctionBuilder,
    function: &mir::Function,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    for parameter in &function.params {
        if matches!(
            local_in(function, *parameter)?.ty,
            mir::Type::String | mir::Type::NullableString
        ) {
            let slot = local_slot(resources.local_slots, *parameter)?;
            let offset = if matches!(
                local_in(function, *parameter)?.ty,
                mir::Type::NullableString
            ) {
                pointer.bytes() as i32
            } else {
                0
            };
            let value = builder.ins().stack_load(pointer, slot, offset);
            let retained = retain_string(builder, value, resources)?;
            builder.ins().stack_store(retained, slot, offset);
        }
    }
    Ok(())
}

fn cleanup_string_locals(
    builder: &mut FunctionBuilder,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let function = function_in(resources.program, resources.function_id)?;
    let string_locals = function
        .locals
        .iter()
        .filter(|local| matches!(local.ty, mir::Type::String | mir::Type::NullableString))
        .map(|local| local.id)
        .collect::<Vec<_>>();
    for local in string_locals {
        let definition = local_in(function, local)?;
        let offset = if matches!(definition.ty, mir::Type::NullableString) {
            pointer.bytes() as i32
        } else {
            0
        };
        let value =
            builder
                .ins()
                .stack_load(pointer, local_slot(resources.local_slots, local)?, offset);
        release_string(builder, value, resources)?;
    }
    Ok(())
}

fn cleanup_class_locals(
    builder: &mut FunctionBuilder,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let pointer_type = resources.module.target_config().pointer_type();
    let function = function_in(resources.program, resources.function_id)?;
    let class_locals = function
        .locals
        .iter()
        .rev()
        .filter_map(|local| match (local.owned, local.ty) {
            (true, mir::Type::Class(class) | mir::Type::NullableClass(class)) => {
                Some((local.id, class))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    for (local, class) in class_locals {
        let slot = local_slot(resources.local_slots, local)?;
        let value = builder.ins().stack_load(pointer_type, slot, 0);
        let zero = builder.ins().iconst(pointer_type, 0);
        builder.ins().stack_store(zero, slot, 0);
        lower_drop_class_value_checked(builder, value, class, resources)?;
    }
    Ok(())
}

fn flush_deferred_class_temporary_drops(
    builder: &mut FunctionBuilder,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let drops = std::mem::take(&mut resources.deferred_class_temporary_drops);
    emit_deferred_class_temporary_drops(builder, &drops, resources)
}

fn emit_deferred_class_temporary_drops(
    builder: &mut FunctionBuilder,
    drops: &[(StackSlot, crate::class_layout::ClassId)],
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    for (slot, class) in drops.iter().rev() {
        let value = builder.ins().stack_load(pointer, *slot, 0);
        let zero = builder.ins().iconst(pointer, 0);
        builder.ins().stack_store(zero, *slot, 0);
        lower_drop_class_value_checked(builder, value, *class, resources)?;
    }
    Ok(())
}

fn defer_or_drop_class_temporary(
    builder: &mut FunctionBuilder,
    value: Value,
    class: crate::class_layout::ClassId,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    if !resources.defer_class_temporary_drops {
        return lower_drop_class_value_checked(builder, value, class, resources);
    }
    let slot = *resources
        .deferred_class_temporary_slots
        .get(resources.deferred_class_temporary_slot_cursor)
        .ok_or_else(|| malformed_mir("class temporary stack-slot capacity was exhausted"))?;
    resources.deferred_class_temporary_slot_cursor += 1;
    builder.ins().stack_store(value, slot, 0);
    resources.deferred_class_temporary_drops.push((slot, class));
    Ok(())
}

fn define_process_main(
    module: &mut ObjectModule,
    program: &mir::Program,
    process_main_id: FuncId,
    process_signature: &Signature,
    function_ids: &[FuncId],
) -> Result<(), BackendError> {
    let entry = program
        .functions
        .get(program.entry.0)
        .ok_or_else(|| malformed_mir("entry function does not exist"))?;
    let entry_id = *function_ids
        .get(program.entry.0)
        .ok_or_else(|| malformed_mir("entry function was not declared"))?;

    let mut context = module.make_context();
    context.func.signature = process_signature.clone();
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let block = builder.create_block();
        builder.switch_to_block(block);
        builder.seal_block(block);

        let pointer_type = module.target_config().pointer_type();
        let entry_ref = module.declare_func_in_func(entry_id, builder.func);
        let entry_pointer = builder.ins().func_addr(pointer_type, entry_ref);
        let mut runtime_signature = module.make_signature();
        runtime_signature.params.push(AbiParam::new(pointer_type));
        runtime_signature.returns.push(AbiParam::new(types::I32));
        let runtime_symbol = match entry.return_type {
            mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Integer(
                IntegerType::Int64,
            ))) => "dr_v1_main_int",
            mir::ReturnType::Void => "dr_v1_main_void",
            mir::ReturnType::Value(other) => {
                return Err(malformed_mir(format!(
                    "entry function has unsupported process return type {other}"
                )));
            }
        };
        let runtime_id = module
            .declare_function(runtime_symbol, Linkage::Import, &runtime_signature)
            .map_err(|error| backend_failure(error.to_string()))?;
        let runtime = module.declare_func_in_func(runtime_id, builder.func);
        let call = builder.ins().call(runtime, &[entry_pointer]);
        let status = builder.inst_results(call)[0];
        builder.ins().return_(&[status]);
        builder.finalize();
    }

    module
        .define_function(process_main_id, &mut context)
        .map_err(|error| backend_failure(error.to_string()))?;
    module.clear_context(&mut context);
    Ok(())
}

struct LoweringResources<'module, 'program> {
    module: &'module mut ObjectModule,
    program: &'program mir::Program,
    function_ids: &'program [FuncId],
    static_ids: &'program [DataId],
    local_slots: &'program [Option<StackSlot>],
    deferred_class_temporary_slots: Vec<StackSlot>,
    deferred_class_temporary_slot_cursor: usize,
    write_stdout_func_id: Option<FuncId>,
    panic_func_id: Option<FuncId>,
    runtime_functions: HashMap<&'static str, FuncId>,
    next_data_id: usize,
    function_id: mir::FunctionId,
    current_frame: Value,
    defer_class_temporary_drops: bool,
    deferred_class_temporary_drops: Vec<(StackSlot, crate::class_layout::ClassId)>,
}

impl<'module, 'program> LoweringResources<'module, 'program> {
    fn declare_write_stdout(&mut self) -> Result<FuncId, BackendError> {
        if let Some(id) = self.write_stdout_func_id {
            return Ok(id);
        }
        let pointer_type = self.module.target_config().pointer_type();
        let mut signature = self.module.make_signature();
        signature.params.push(AbiParam::new(pointer_type));
        signature.params.push(AbiParam::new(pointer_type));
        signature.params.push(AbiParam::new(pointer_type));
        let id = self
            .module
            .declare_function("dr_v1_write_stdout", Linkage::Import, &signature)
            .map_err(|error| backend_failure(error.to_string()))?;
        self.write_stdout_func_id = Some(id);
        Ok(id)
    }

    fn declare_panic(&mut self) -> Result<FuncId, BackendError> {
        if let Some(id) = self.panic_func_id {
            return Ok(id);
        }
        let pointer_type = self.module.target_config().pointer_type();
        let mut signature = self.module.make_signature();
        signature.params.push(AbiParam::new(pointer_type));
        signature.params.push(AbiParam::new(pointer_type));
        signature.params.push(AbiParam::new(pointer_type));
        let id = self
            .module
            .declare_function("dr_v1_panic", Linkage::Import, &signature)
            .map_err(|error| backend_failure(error.to_string()))?;
        self.panic_func_id = Some(id);
        Ok(id)
    }

    fn declare_runtime(
        &mut self,
        name: &'static str,
        params: &[ClifType],
        result: Option<ClifType>,
    ) -> Result<FuncId, BackendError> {
        if let Some(id) = self.runtime_functions.get(name) {
            return Ok(*id);
        }
        let mut signature = self.module.make_signature();
        signature
            .params
            .extend(params.iter().copied().map(AbiParam::new));
        if let Some(result) = result {
            signature.returns.push(AbiParam::new(result));
        }
        let id = self
            .module
            .declare_function(name, Linkage::Import, &signature)
            .map_err(|error| backend_failure(error.to_string()))?;
        self.runtime_functions.insert(name, id);
        Ok(id)
    }
}

fn lower_block(
    builder: &mut FunctionBuilder,
    block: &mir::BasicBlock,
    blocks: &[Block],
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    for statement in &block.statements {
        lower_statement(builder, statement, resources)?;
    }
    lower_terminator(builder, &block.terminator, blocks, resources)
}

fn lower_statement(
    builder: &mut FunctionBuilder,
    statement: &mir::Statement,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    debug_assert!(resources.deferred_class_temporary_drops.is_empty());
    resources.defer_class_temporary_drops = true;
    match statement {
        mir::Statement::AssignLocal { target, value } => {
            let definition = local_definition(resources.program, resources.function_id, *target)?;
            let new_value = lower_rvalue(builder, value, resources)?;
            let slot = local_slot(resources.local_slots, *target)?;
            let pointer = resources.module.target_config().pointer_type();
            let old_value = match definition.ty {
                mir::Type::String => Some((
                    load_lowered_from_stack(builder, definition.ty, slot, pointer).single()?,
                    None,
                )),
                mir::Type::NullableString => Some((
                    load_lowered_from_stack(builder, definition.ty, slot, pointer)
                        .nullable()?
                        .1,
                    None,
                )),
                mir::Type::Class(class) | mir::Type::NullableClass(class) if definition.owned => {
                    Some((
                        load_lowered_from_stack(builder, definition.ty, slot, pointer).single()?,
                        Some(class),
                    ))
                }
                _ => None,
            };
            store_lowered_to_stack(builder, definition.ty, slot, new_value, pointer)?;
            if let Some((old, class)) = old_value {
                if let Some(class) = class {
                    lower_drop_class_value_checked(builder, old, class, resources)?;
                } else {
                    release_string(builder, old, resources)?;
                }
            }
        }
        mir::Statement::EchoStringLiteral(value) => {
            lower_echo_bytes(builder, value.as_bytes(), resources)?;
        }
        mir::Statement::EchoString(value) => {
            let string = lower_string_expression(builder, value, resources)?;
            let pointer_type = resources.module.target_config().pointer_type();
            let write_id = resources.declare_runtime(
                STRING_WRITE_STDOUT,
                &[pointer_type, pointer_type],
                None,
            )?;
            let write = resources
                .module
                .declare_func_in_func(write_id, builder.func);
            builder
                .ins()
                .call(write, &[resources.current_frame, string]);
            release_string(builder, string, resources)?;
        }
        mir::Statement::CallVoid { function, args }
        | mir::Statement::CallBorrowed { function, args } => {
            let _ = lower_function_call(builder, *function, args, resources)?;
        }
        mir::Statement::CallNullSafe {
            object,
            function,
            args,
        } => lower_null_safe_statement_call(builder, object, *function, args, resources)?,
        mir::Statement::WriteStderr(value) => {
            let string = lower_string_expression(builder, value, resources)?;
            let pointer = resources.module.target_config().pointer_type();
            let _ = runtime_call(
                builder,
                STRING_WRITE_STDERR,
                &[pointer, pointer],
                None,
                &[resources.current_frame, string],
                resources,
            )?;
            release_string(builder, string, resources)?;
        }
        mir::Statement::Printf(format) => {
            let string = lower_format_expression(builder, format, resources)?;
            let pointer = resources.module.target_config().pointer_type();
            let _ = runtime_call(
                builder,
                STRING_WRITE_STDOUT,
                &[pointer, pointer],
                None,
                &[resources.current_frame, string],
                resources,
            )?;
            release_string(builder, string, resources)?;
        }
        mir::Statement::WriteFile { path, contents } => {
            let path = lower_string_expression(builder, path, resources)?;
            let contents = lower_string_expression(builder, contents, resources)?;
            let pointer = resources.module.target_config().pointer_type();
            let _ = runtime_call(
                builder,
                WRITE_FILE,
                &[pointer, pointer, pointer],
                None,
                &[resources.current_frame, path, contents],
                resources,
            )?;
            release_string(builder, path, resources)?;
            release_string(builder, contents, resources)?;
        }
        mir::Statement::AssignProperty {
            object,
            property,
            value,
        } => {
            let property_definition = property_definition(resources.program, *property)?;
            let value = lower_rvalue(builder, value, resources)?;
            let address = lower_property_address(builder, *object, *property, resources)?;
            let pointer_type = resources.module.target_config().pointer_type();
            let old_value = match property_definition.ty {
                mir::Type::String | mir::Type::Class(_) | mir::Type::NullableClass(_) => Some(
                    load_lowered_from_address(
                        builder,
                        property_definition.ty,
                        address,
                        pointer_type,
                    )
                    .single()?,
                ),
                mir::Type::NullableString => Some(
                    load_lowered_from_address(
                        builder,
                        property_definition.ty,
                        address,
                        pointer_type,
                    )
                    .nullable()?
                    .1,
                ),
                mir::Type::Scalar(_) | mir::Type::NullableScalar(_) => None,
            };
            store_lowered_to_address(
                builder,
                property_definition.ty,
                address,
                value,
                pointer_type,
            )?;
            match (property_definition.ty, old_value) {
                (mir::Type::String | mir::Type::NullableString, Some(old_value)) => {
                    release_string(builder, old_value, resources)?;
                }
                (mir::Type::Class(class) | mir::Type::NullableClass(class), Some(old_value)) => {
                    lower_drop_class_value_checked(builder, old_value, class, resources)?;
                }
                _ => {}
            }
        }
        mir::Statement::AssignStatic { target, value } => {
            let property = static_definition(resources.program, *target)?;
            let new_value = lower_rvalue(builder, value, resources)?;
            let address = lower_static_address(builder, *target, resources)?;
            let pointer = resources.module.target_config().pointer_type();
            let old_value = match property.ty {
                mir::Type::String => Some(
                    load_lowered_from_address(builder, property.ty, address, pointer).single()?,
                ),
                mir::Type::NullableString => Some(
                    load_lowered_from_address(builder, property.ty, address, pointer)
                        .nullable()?
                        .1,
                ),
                _ => None,
            };
            store_lowered_to_address(builder, property.ty, address, new_value, pointer)?;
            if let Some(old_value) = old_value {
                release_string(builder, old_value, resources)?;
            }
        }
        mir::Statement::DropClass { local, .. } => {
            let pointer_type = resources.module.target_config().pointer_type();
            let slot = local_slot(resources.local_slots, *local)?;
            let value = builder.ins().stack_load(pointer_type, slot, 0);
            let zero = builder.ins().iconst(pointer_type, 0);
            builder.ins().stack_store(zero, slot, 0);
            let (mir::Type::Class(class) | mir::Type::NullableClass(class)) =
                local_definition(resources.program, resources.function_id, *local)?.ty
            else {
                return Err(malformed_mir(format!(
                    "drop local{} did not target a class local",
                    local.0
                )));
            };
            lower_drop_class_value_checked(builder, value, class, resources)?;
        }
    }
    resources.defer_class_temporary_drops = false;
    flush_deferred_class_temporary_drops(builder, resources)
}

fn lower_static_address(
    builder: &mut FunctionBuilder,
    id: mir::StaticId,
    resources: &LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let data_id = *resources
        .static_ids
        .get(id.0)
        .ok_or_else(|| malformed_mir(format!("static{} was not declared", id.0)))?;
    let global = resources.module.declare_data_in_func(data_id, builder.func);
    Ok(builder
        .ins()
        .global_value(resources.module.target_config().pointer_type(), global))
}

fn load_lowered_from_stack(
    builder: &mut FunctionBuilder,
    ty: mir::Type,
    slot: StackSlot,
    pointer: ClifType,
) -> LoweredValue {
    match ty {
        mir::Type::NullableScalar(scalar) => LoweredValue::Nullable {
            present: builder.ins().stack_load(pointer, slot, 0),
            payload: builder.ins().stack_load(
                clif_scalar_type(scalar),
                slot,
                pointer.bytes() as i32,
            ),
        },
        mir::Type::NullableString => LoweredValue::Nullable {
            present: builder.ins().stack_load(pointer, slot, 0),
            payload: builder
                .ins()
                .stack_load(pointer, slot, pointer.bytes() as i32),
        },
        mir::Type::Scalar(scalar) => {
            LoweredValue::Single(builder.ins().stack_load(clif_scalar_type(scalar), slot, 0))
        }
        mir::Type::String | mir::Type::Class(_) | mir::Type::NullableClass(_) => {
            LoweredValue::Single(builder.ins().stack_load(pointer, slot, 0))
        }
    }
}

fn store_lowered_to_stack(
    builder: &mut FunctionBuilder,
    ty: mir::Type,
    slot: StackSlot,
    value: LoweredValue,
    pointer: ClifType,
) -> Result<(), BackendError> {
    match ty {
        mir::Type::NullableScalar(_) | mir::Type::NullableString => {
            let (present, payload) = value.nullable()?;
            builder.ins().stack_store(present, slot, 0);
            builder
                .ins()
                .stack_store(payload, slot, pointer.bytes() as i32);
        }
        _ => {
            builder.ins().stack_store(value.single()?, slot, 0);
        }
    }
    Ok(())
}

fn load_lowered_from_address(
    builder: &mut FunctionBuilder,
    ty: mir::Type,
    address: Value,
    pointer: ClifType,
) -> LoweredValue {
    let flags = cranelift_codegen::ir::MachMemFlags::trusted();
    match ty {
        mir::Type::NullableScalar(scalar) => LoweredValue::Nullable {
            present: builder.ins().load(pointer, flags, address, 0),
            payload: builder.ins().load(
                clif_scalar_type(scalar),
                flags,
                address,
                pointer.bytes() as i32,
            ),
        },
        mir::Type::NullableString => LoweredValue::Nullable {
            present: builder.ins().load(pointer, flags, address, 0),
            payload: builder
                .ins()
                .load(pointer, flags, address, pointer.bytes() as i32),
        },
        mir::Type::Scalar(scalar) => LoweredValue::Single(builder.ins().load(
            clif_scalar_type(scalar),
            flags,
            address,
            0,
        )),
        mir::Type::String | mir::Type::Class(_) | mir::Type::NullableClass(_) => {
            LoweredValue::Single(builder.ins().load(pointer, flags, address, 0))
        }
    }
}

fn store_lowered_to_address(
    builder: &mut FunctionBuilder,
    ty: mir::Type,
    address: Value,
    value: LoweredValue,
    pointer: ClifType,
) -> Result<(), BackendError> {
    let flags = cranelift_codegen::ir::MachMemFlags::trusted();
    match ty {
        mir::Type::NullableScalar(_) | mir::Type::NullableString => {
            let (present, payload) = value.nullable()?;
            builder.ins().store(flags, present, address, 0);
            builder
                .ins()
                .store(flags, payload, address, pointer.bytes() as i32);
        }
        _ => {
            builder.ins().store(flags, value.single()?, address, 0);
        }
    }
    Ok(())
}

fn lower_terminator(
    builder: &mut FunctionBuilder,
    terminator: &mir::Terminator,
    blocks: &[Block],
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    match terminator {
        mir::Terminator::Return(expression) => {
            debug_assert!(resources.deferred_class_temporary_drops.is_empty());
            resources.defer_class_temporary_drops = true;
            let value = lower_rvalue(builder, expression, resources)?;
            resources.defer_class_temporary_drops = false;
            flush_deferred_class_temporary_drops(builder, resources)?;
            cleanup_class_locals(builder, resources)?;
            cleanup_string_locals(builder, resources)?;
            let mut values = Vec::with_capacity(2);
            value.append_to(&mut values);
            builder.ins().return_(&values);
        }
        mir::Terminator::ReturnVoid => {
            cleanup_class_locals(builder, resources)?;
            cleanup_string_locals(builder, resources)?;
            builder.ins().return_(&[]);
        }
        mir::Terminator::Panic(message) => {
            debug_assert!(resources.deferred_class_temporary_drops.is_empty());
            resources.defer_class_temporary_drops = true;
            let string = lower_string_expression(builder, message, resources)?;
            resources.defer_class_temporary_drops = false;
            // Abort-only panic never reaches statement-end destruction.
            resources.deferred_class_temporary_drops.clear();
            let pointer_type = resources.module.target_config().pointer_type();
            let data_id =
                resources.declare_runtime(STRING_DATA, &[pointer_type], Some(pointer_type))?;
            let len_id =
                resources.declare_runtime(STRING_LENGTH, &[pointer_type], Some(pointer_type))?;
            let data_ref = resources.module.declare_func_in_func(data_id, builder.func);
            let len_ref = resources.module.declare_func_in_func(len_id, builder.func);
            let data_call = builder.ins().call(data_ref, &[string]);
            let len_call = builder.ins().call(len_ref, &[string]);
            let data = builder.inst_results(data_call)[0];
            let len = builder.inst_results(len_call)[0];
            let panic_id = resources.declare_panic()?;
            let panic = resources
                .module
                .declare_func_in_func(panic_id, builder.func);
            builder
                .ins()
                .call(panic, &[resources.current_frame, data, len]);
            builder
                .ins()
                .trap(TrapCode::unwrap_user(RUNTIME_RETURNED_TRAP));
        }
        mir::Terminator::Unreachable => {
            builder
                .ins()
                .trap(TrapCode::unwrap_user(RUNTIME_RETURNED_TRAP));
        }
        mir::Terminator::Jump(target) => {
            builder.ins().jump(block_for(blocks, *target)?, &[]);
        }
        mir::Terminator::Branch {
            condition,
            then_block,
            else_block,
        } => {
            if mir::bool_class_temporary_capacity(condition) == 0 {
                return lower_condition_to_branch(
                    builder,
                    condition,
                    block_for(blocks, *then_block)?,
                    block_for(blocks, *else_block)?,
                    resources,
                );
            }
            debug_assert!(resources.deferred_class_temporary_drops.is_empty());
            let cleanup_then = builder.create_block();
            let cleanup_else = builder.create_block();
            resources.defer_class_temporary_drops = true;
            lower_condition_to_branch(builder, condition, cleanup_then, cleanup_else, resources)?;
            resources.defer_class_temporary_drops = false;
            let drops = std::mem::take(&mut resources.deferred_class_temporary_drops);

            builder.switch_to_block(cleanup_then);
            emit_deferred_class_temporary_drops(builder, &drops, resources)?;
            builder.ins().jump(block_for(blocks, *then_block)?, &[]);

            builder.switch_to_block(cleanup_else);
            emit_deferred_class_temporary_drops(builder, &drops, resources)?;
            builder.ins().jump(block_for(blocks, *else_block)?, &[]);
        }
    }
    Ok(())
}

fn lower_value_expression(
    builder: &mut FunctionBuilder,
    expression: &mir::ValueExpression,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    match expression {
        mir::ValueExpression::Integer(value) => lower_integer_expression(builder, value, resources),
        mir::ValueExpression::Float(value) => lower_float_expression(builder, value, resources),
        mir::ValueExpression::Bool(value) => lower_condition_value(builder, value, resources),
    }
}

fn lower_rvalue(
    builder: &mut FunctionBuilder,
    expression: &mir::Rvalue,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<LoweredValue, BackendError> {
    match expression {
        mir::Rvalue::Value(value) => {
            lower_value_expression(builder, value, resources).map(LoweredValue::Single)
        }
        mir::Rvalue::String(value) => {
            lower_string_expression(builder, value, resources).map(LoweredValue::Single)
        }
        mir::Rvalue::NullableScalar(value) => {
            lower_nullable_scalar_expression(builder, value, resources)
        }
        mir::Rvalue::NullableString(value) => {
            lower_nullable_string_expression(builder, value, resources)
        }
        mir::Rvalue::Class(value) => {
            lower_class_expression(builder, value, resources).map(LoweredValue::Single)
        }
        mir::Rvalue::NullableClass(value) => {
            lower_nullable_class_expression(builder, value, resources).map(LoweredValue::Single)
        }
    }
}

fn lower_class_expression(
    builder: &mut FunctionBuilder,
    expression: &mir::ClassExpression,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer_type = resources.module.target_config().pointer_type();
    match expression {
        mir::ClassExpression::Local {
            local, transfer, ..
        } => {
            let slot = local_slot(resources.local_slots, *local)?;
            let value = builder.ins().stack_load(pointer_type, slot, 0);
            if *transfer {
                let zero = builder.ins().iconst(pointer_type, 0);
                builder.ins().stack_store(zero, slot, 0);
            }
            Ok(value)
        }
        mir::ClassExpression::NullableLocalAssumeNonNull {
            local, transfer, ..
        } => {
            let slot = local_slot(resources.local_slots, *local)?;
            let value = builder.ins().stack_load(pointer_type, slot, 0);
            if *transfer {
                let zero = builder.ins().iconst(pointer_type, 0);
                builder.ins().stack_store(zero, slot, 0);
            }
            Ok(value)
        }
        mir::ClassExpression::Property {
            object, property, ..
        } => {
            let address = lower_property_address(builder, *object, *property, resources)?;
            Ok(builder.ins().load(
                pointer_type,
                cranelift_codegen::ir::MachMemFlags::trusted(),
                address,
                0,
            ))
        }
        mir::ClassExpression::Call { function, args, .. } => {
            lower_function_call(builder, *function, args, resources)?
                .ok_or_else(|| malformed_mir("class call produced no result"))?
                .single()
        }
        mir::ClassExpression::New {
            class,
            properties,
            constructor,
            args,
        } => {
            // Explicit property initializers precede promoted properties in the
            // canonical construction order, so evaluate their source expressions
            // before any constructor-argument side effects. Arguments are still
            // evaluated exactly once and shared by promotion and the lifecycle call.
            let mut lowered_properties = Vec::with_capacity(properties.len());
            for property in properties {
                lowered_properties.push(match &property.source {
                    mir::PropertyValueSource::Expression(value) => {
                        Some(lower_rvalue(builder, value, resources)?)
                    }
                    mir::PropertyValueSource::ConstructorArgument(_)
                    | mir::PropertyValueSource::ConstructorBody => None,
                });
            }
            let lowered_args = lower_call_args(builder, args, resources)?;
            let class_definition = class_definition(resources.program, *class)?;
            let size = builder
                .ins()
                .iconst(pointer_type, i64::from(class_definition.layout.size));
            let align = builder
                .ins()
                .iconst(pointer_type, i64::from(class_definition.layout.align));
            let object = runtime_call(
                builder,
                CLASS_ALLOCATE,
                &[pointer_type, pointer_type, pointer_type],
                Some(pointer_type),
                &[resources.current_frame, size, align],
                resources,
            )?
            .ok_or_else(|| backend_failure("class allocation produced no result"))?;
            for (property, lowered_property) in properties.iter().zip(lowered_properties) {
                let value = match &property.source {
                    mir::PropertyValueSource::Expression(_) => lowered_property,
                    mir::PropertyValueSource::ConstructorArgument(index) => {
                        Some(*lowered_args.arguments.get(*index).ok_or_else(|| {
                            malformed_mir(format!("constructor argument {index} does not exist"))
                        })?)
                    }
                    mir::PropertyValueSource::ConstructorBody => None,
                };
                let Some(value) = value else {
                    continue;
                };
                let address = lower_property_address_from_value(
                    builder,
                    object,
                    property.property,
                    resources,
                )?;
                let property_definition =
                    property_definition(resources.program, property.property)?;
                store_lowered_to_address(
                    builder,
                    property_definition.ty,
                    address,
                    value,
                    pointer_type,
                )?;
            }
            if let Some(constructor) = constructor {
                let mut constructor_args = vec![resources.current_frame, object];
                constructor_args.extend(lowered_args.abi_values.iter().copied());
                let callee = declared_function(builder, resources, *constructor)?;
                builder.ins().call(callee, &constructor_args);

                let constructor_definition = function_in(resources.program, *constructor)?;
                for (index, argument) in args.iter().enumerate() {
                    let Some(class) = argument.owned_temporary_class() else {
                        continue;
                    };
                    let promoted = properties.iter().any(|property| {
                        matches!(
                            property.source,
                            mir::PropertyValueSource::ConstructorArgument(argument)
                                if argument == index
                        )
                    });
                    let parameter =
                        *constructor_definition
                            .params
                            .get(index + 1)
                            .ok_or_else(|| {
                                malformed_mir(format!(
                                    "constructor function{} is missing parameter {index}",
                                    constructor.0
                                ))
                            })?;
                    if !promoted && !local_in(constructor_definition, parameter)?.owned {
                        let value = lowered_args.arguments[index].single()?;
                        defer_or_drop_class_temporary(builder, value, class, resources)?;
                    }
                }
            }
            for (index, string) in lowered_args.owned_strings {
                let promoted = properties.iter().any(|property| {
                    matches!(
                        property.source,
                        mir::PropertyValueSource::ConstructorArgument(argument)
                            if argument == index
                    )
                });
                if !promoted {
                    release_string(builder, string, resources)?;
                }
            }
            Ok(object)
        }
        mir::ClassExpression::Coalesce { left, right, .. } => {
            let left_owned = left.owned_temporary_class().is_some();
            let right_owned = right.owned_temporary_class().is_some();
            let left = lower_nullable_class_expression(builder, left, resources)?;
            let zero = builder.ins().iconst(pointer_type, 0);
            let present = builder.ins().icmp(IntCC::NotEqual, left, zero);
            let left_block = builder.create_block();
            let right_block = builder.create_block();
            let done = builder.create_block();
            builder.append_block_param(done, pointer_type);
            builder.append_block_param(done, pointer_type);
            builder
                .ins()
                .brif(present, left_block, &[], right_block, &[]);
            builder.switch_to_block(left_block);
            let left_temporary = if left_owned { left } else { zero };
            builder.ins().jump(
                done,
                &[BlockArg::Value(left), BlockArg::Value(left_temporary)],
            );
            builder.switch_to_block(right_block);
            let right = lower_class_expression(builder, right, resources)?;
            let right_temporary = if right_owned { right } else { zero };
            builder.ins().jump(
                done,
                &[BlockArg::Value(right), BlockArg::Value(right_temporary)],
            );
            builder.switch_to_block(done);
            let result = builder.block_params(done)[0];
            if left_owned || right_owned {
                defer_or_drop_class_temporary(
                    builder,
                    builder.block_params(done)[1],
                    expression.class(),
                    resources,
                )?;
            }
            Ok(result)
        }
    }
}

fn presence_word(builder: &mut FunctionBuilder, value: Value, pointer: ClifType) -> Value {
    let zero = builder.ins().iconst(pointer, 0);
    let present = builder.ins().icmp(IntCC::NotEqual, value, zero);
    builder.ins().uextend(pointer, present)
}

fn lower_nullable_class_expression(
    builder: &mut FunctionBuilder,
    expression: &mir::NullableClassExpression,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    match expression {
        mir::NullableClassExpression::Null(_) => Ok(builder.ins().iconst(pointer, 0)),
        mir::NullableClassExpression::Class(value) => {
            lower_class_expression(builder, value, resources)
        }
        mir::NullableClassExpression::Local {
            local, transfer, ..
        } => {
            let slot = local_slot(resources.local_slots, *local)?;
            let value = builder.ins().stack_load(pointer, slot, 0);
            if *transfer {
                let zero = builder.ins().iconst(pointer, 0);
                builder.ins().stack_store(zero, slot, 0);
            }
            Ok(value)
        }
        mir::NullableClassExpression::Property {
            object, property, ..
        } => {
            let address = lower_property_address(builder, *object, *property, resources)?;
            Ok(builder.ins().load(
                pointer,
                cranelift_codegen::ir::MachMemFlags::trusted(),
                address,
                0,
            ))
        }
        mir::NullableClassExpression::Call { function, args, .. } => {
            lower_function_call(builder, *function, args, resources)?
                .ok_or_else(|| malformed_mir("nullable-class call produced no result"))?
                .single()
        }
        mir::NullableClassExpression::NullSafeProperty {
            object, property, ..
        } => {
            let owned_receiver = object.owned_temporary_class();
            let object = lower_nullable_class_expression(builder, object, resources)?;
            lower_null_safe_single(
                builder,
                object,
                pointer,
                owned_receiver,
                resources,
                |builder, resources| {
                    let address =
                        lower_property_address_from_value(builder, object, *property, resources)?;
                    Ok(builder.ins().load(
                        pointer,
                        cranelift_codegen::ir::MachMemFlags::trusted(),
                        address,
                        0,
                    ))
                },
            )
        }
        mir::NullableClassExpression::NullSafeCall {
            object,
            function,
            args,
            ..
        } => {
            let owned_receiver = object.owned_temporary_class();
            let object = lower_nullable_class_expression(builder, object, resources)?;
            lower_null_safe_single(
                builder,
                object,
                pointer,
                owned_receiver,
                resources,
                |builder, resources| {
                    lower_method_call_with_receiver(builder, object, *function, args, resources)?
                        .ok_or_else(|| malformed_mir("null-safe class call produced no result"))?
                        .single()
                },
            )
        }
        mir::NullableClassExpression::Coalesce {
            class, left, right, ..
        } => {
            let left_owned = left.owned_temporary_class().is_some();
            let right_owned = right.owned_temporary_class().is_some();
            let left = lower_nullable_class_expression(builder, left, resources)?;
            let zero = builder.ins().iconst(pointer, 0);
            let present = builder.ins().icmp(IntCC::NotEqual, left, zero);
            let left_block = builder.create_block();
            let right_block = builder.create_block();
            let done = builder.create_block();
            builder.append_block_param(done, pointer);
            builder.append_block_param(done, pointer);
            builder
                .ins()
                .brif(present, left_block, &[], right_block, &[]);
            builder.switch_to_block(left_block);
            let left_temporary = if left_owned { left } else { zero };
            builder.ins().jump(
                done,
                &[BlockArg::Value(left), BlockArg::Value(left_temporary)],
            );
            builder.switch_to_block(right_block);
            let right = lower_nullable_class_expression(builder, right, resources)?;
            let right_temporary = if right_owned { right } else { zero };
            builder.ins().jump(
                done,
                &[BlockArg::Value(right), BlockArg::Value(right_temporary)],
            );
            builder.switch_to_block(done);
            let result = builder.block_params(done)[0];
            if left_owned || right_owned {
                defer_or_drop_class_temporary(
                    builder,
                    builder.block_params(done)[1],
                    *class,
                    resources,
                )?;
            }
            Ok(result)
        }
    }
}

fn lower_null_safe_single(
    builder: &mut FunctionBuilder,
    object: Value,
    result_type: ClifType,
    owned_receiver: Option<crate::class_layout::ClassId>,
    resources: &mut LoweringResources<'_, '_>,
    present_value: impl FnOnce(
        &mut FunctionBuilder,
        &mut LoweringResources<'_, '_>,
    ) -> Result<Value, BackendError>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    if let Some(class) = owned_receiver {
        defer_or_drop_class_temporary(builder, object, class, resources)?;
    }
    let present = presence_word(builder, object, pointer);
    let zero = builder.ins().iconst(pointer, 0);
    let is_present = builder.ins().icmp(IntCC::NotEqual, present, zero);
    let some = builder.create_block();
    let none = builder.create_block();
    let done = builder.create_block();
    builder.append_block_param(done, result_type);
    builder.ins().brif(is_present, some, &[], none, &[]);
    builder.switch_to_block(some);
    let value = present_value(builder, resources)?;
    builder.ins().jump(done, &[BlockArg::Value(value)]);
    builder.switch_to_block(none);
    let zero = builder.ins().iconst(result_type, 0);
    builder.ins().jump(done, &[BlockArg::Value(zero)]);
    builder.switch_to_block(done);
    let result = builder.block_params(done)[0];
    Ok(result)
}

fn lower_drop_class_value_checked(
    builder: &mut FunctionBuilder,
    object: Value,
    class: crate::class_layout::ClassId,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let pointer_type = resources.module.target_config().pointer_type();
    let zero = builder.ins().iconst(pointer_type, 0);
    let has_object = builder.ins().icmp(IntCC::NotEqual, object, zero);
    let drop_block = builder.create_block();
    let continue_block = builder.create_block();
    builder
        .ins()
        .brif(has_object, drop_block, &[], continue_block, &[]);
    builder.switch_to_block(drop_block);
    lower_drop_class_value(builder, object, class, resources)?;
    builder.ins().jump(continue_block, &[]);
    builder.switch_to_block(continue_block);
    Ok(())
}

fn lower_drop_class_value(
    builder: &mut FunctionBuilder,
    object: Value,
    class: crate::class_layout::ClassId,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let pointer_type = resources.module.target_config().pointer_type();
    let class_definition = class_definition(resources.program, class)?;
    if let Some(destructor) = class_definition.destructor {
        let callee = declared_function(builder, resources, destructor)?;
        builder
            .ins()
            .call(callee, &[resources.current_frame, object]);
    }
    for property in class_definition.properties.iter().rev() {
        let address = lower_property_address_from_value(builder, object, property.id, resources)?;
        match property.ty {
            mir::Type::String | mir::Type::NullableString => {
                let value = match property.ty {
                    mir::Type::NullableString => {
                        load_lowered_from_address(builder, property.ty, address, pointer_type)
                            .nullable()?
                            .1
                    }
                    _ => load_lowered_from_address(builder, property.ty, address, pointer_type)
                        .single()?,
                };
                release_string(builder, value, resources)?;
            }
            mir::Type::Class(class) | mir::Type::NullableClass(class) => {
                let value = builder.ins().load(
                    pointer_type,
                    cranelift_codegen::ir::MachMemFlags::trusted(),
                    address,
                    0,
                );
                lower_drop_class_value_checked(builder, value, class, resources)?;
            }
            mir::Type::Scalar(_) | mir::Type::NullableScalar(_) => {}
        }
    }
    let _ = runtime_call(
        builder,
        CLASS_FREE,
        &[pointer_type],
        None,
        &[object],
        resources,
    )?;
    Ok(())
}

fn runtime_call(
    builder: &mut FunctionBuilder,
    name: &'static str,
    params: &[ClifType],
    result: Option<ClifType>,
    values: &[Value],
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Option<Value>, BackendError> {
    let id = resources.declare_runtime(name, params, result)?;
    let reference = resources.module.declare_func_in_func(id, builder.func);
    let call = builder.ins().call(reference, values);
    Ok(builder.inst_results(call).first().copied())
}

fn retain_string(
    builder: &mut FunctionBuilder,
    value: Value,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    runtime_call(
        builder,
        STRING_RETAIN,
        &[pointer],
        Some(pointer),
        &[value],
        resources,
    )?
    .ok_or_else(|| backend_failure("string retain produced no result"))
}

fn release_string(
    builder: &mut FunctionBuilder,
    value: Value,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    runtime_call(
        builder,
        STRING_RELEASE,
        &[pointer],
        None,
        &[value],
        resources,
    )?;
    Ok(())
}

fn lower_string_expression(
    builder: &mut FunctionBuilder,
    expression: &mir::StringExpression,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    match expression {
        mir::StringExpression::Literal(value) => {
            let data = define_data(builder, value.as_bytes(), resources)?;
            let length = builder.ins().iconst(pointer, value.len() as i64);
            runtime_call(
                builder,
                STRING_FROM_UTF8,
                &[pointer, pointer],
                Some(pointer),
                &[data, length],
                resources,
            )?
            .ok_or_else(|| backend_failure("string allocation produced no result"))
        }
        mir::StringExpression::Local(local) => {
            let value =
                builder
                    .ins()
                    .stack_load(pointer, local_slot(resources.local_slots, *local)?, 0);
            retain_string(builder, value, resources)
        }
        mir::StringExpression::Static(id) => {
            let address = lower_static_address(builder, *id, resources)?;
            let value = builder.ins().load(
                pointer,
                cranelift_codegen::ir::MachMemFlags::trusted(),
                address,
                0,
            );
            retain_string(builder, value, resources)
        }
        mir::StringExpression::NullableLocalAssumeNonNull(local) => {
            let pointer = resources.module.target_config().pointer_type();
            let value = builder.ins().stack_load(
                pointer,
                local_slot(resources.local_slots, *local)?,
                pointer.bytes() as i32,
            );
            retain_string(builder, value, resources)
        }
        mir::StringExpression::Property { object, property } => {
            let address = lower_property_address(builder, *object, *property, resources)?;
            let value = builder.ins().load(
                pointer,
                cranelift_codegen::ir::MachMemFlags::trusted(),
                address,
                0,
            );
            retain_string(builder, value, resources)
        }
        mir::StringExpression::Concat(parts) => {
            let mut parts = parts.iter();
            let Some(first) = parts.next() else {
                return lower_string_expression(
                    builder,
                    &mir::StringExpression::Literal(String::new()),
                    resources,
                );
            };
            let mut value = lower_string_expression(builder, first, resources)?;
            for part in parts {
                let right = lower_string_expression(builder, part, resources)?;
                let concatenated = runtime_call(
                    builder,
                    STRING_CONCAT,
                    &[pointer, pointer],
                    Some(pointer),
                    &[value, right],
                    resources,
                )?
                .ok_or_else(|| backend_failure("string concat produced no result"))?;
                release_string(builder, value, resources)?;
                release_string(builder, right, resources)?;
                value = concatenated;
            }
            Ok(value)
        }
        mir::StringExpression::Display(value) => {
            let scalar = lower_value_expression(builder, value, resources)?;
            let (name, parameter_type, argument) = match value.ty() {
                mir::ScalarType::Integer(ty) if ty.is_signed() => {
                    let argument = if ty.bit_width() < 64 {
                        builder.ins().sextend(types::I64, scalar)
                    } else {
                        scalar
                    };
                    (STRING_FROM_I64, types::I64, argument)
                }
                mir::ScalarType::Integer(ty) => {
                    let argument = if ty.bit_width() < 64 {
                        builder.ins().uextend(types::I64, scalar)
                    } else {
                        scalar
                    };
                    (STRING_FROM_U64, types::I64, argument)
                }
                mir::ScalarType::Float(FloatType::Float32) => (STRING_FROM_F32, types::F32, scalar),
                mir::ScalarType::Float(FloatType::Float64) => (STRING_FROM_F64, types::F64, scalar),
                mir::ScalarType::Bool => (STRING_FROM_BOOL, types::I8, scalar),
            };
            runtime_call(
                builder,
                name,
                &[parameter_type],
                Some(pointer),
                &[argument],
                resources,
            )?
            .ok_or_else(|| backend_failure("display conversion produced no result"))
        }
        mir::StringExpression::Call { function, args } => {
            lower_function_call(builder, *function, args, resources)?
                .ok_or_else(|| malformed_mir("string call produced no result"))?
                .single()
        }
        mir::StringExpression::ReadFile(path) => {
            let path = lower_string_expression(builder, path, resources)?;
            let pointer = resources.module.target_config().pointer_type();
            let result = runtime_call(
                builder,
                READ_FILE,
                &[pointer, pointer],
                Some(pointer),
                &[resources.current_frame, path],
                resources,
            )?
            .ok_or_else(|| backend_failure("read_file produced no result"))?;
            release_string(builder, path, resources)?;
            Ok(result)
        }
        mir::StringExpression::Format(format) => {
            lower_format_expression(builder, format, resources)
        }
        mir::StringExpression::Coalesce { left, right } => {
            let left = lower_nullable_string_expression(builder, left, resources)?;
            let (present, payload) = left.nullable()?;
            lower_coalesce_value(
                builder,
                present,
                payload,
                pointer,
                resources,
                |builder, resources| lower_string_expression(builder, right, resources),
            )
        }
    }
}

fn lower_nullable_string_expression(
    builder: &mut FunctionBuilder,
    expression: &mir::NullableStringExpression,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<LoweredValue, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    match expression {
        mir::NullableStringExpression::Null => {
            let zero = builder.ins().iconst(pointer, 0);
            Ok(LoweredValue::Nullable {
                present: zero,
                payload: zero,
            })
        }
        mir::NullableStringExpression::String(value) => {
            let payload = lower_string_expression(builder, value, resources)?;
            let present = builder.ins().iconst(pointer, 1);
            Ok(LoweredValue::Nullable { present, payload })
        }
        mir::NullableStringExpression::Local(local) => {
            let value = load_lowered_from_stack(
                builder,
                mir::Type::NullableString,
                local_slot(resources.local_slots, *local)?,
                pointer,
            );
            let (present, payload) = value.nullable()?;
            let payload = retain_string(builder, payload, resources)?;
            Ok(LoweredValue::Nullable { present, payload })
        }
        mir::NullableStringExpression::Static(id) => {
            let address = lower_static_address(builder, *id, resources)?;
            let (present, payload) =
                load_lowered_from_address(builder, mir::Type::NullableString, address, pointer)
                    .nullable()?;
            let payload = retain_string(builder, payload, resources)?;
            Ok(LoweredValue::Nullable { present, payload })
        }
        mir::NullableStringExpression::Property { object, property } => {
            let address = lower_property_address(builder, *object, *property, resources)?;
            let (present, payload) =
                load_lowered_from_address(builder, mir::Type::NullableString, address, pointer)
                    .nullable()?;
            let payload = retain_string(builder, payload, resources)?;
            Ok(LoweredValue::Nullable { present, payload })
        }
        mir::NullableStringExpression::ReadLine => {
            let payload = runtime_call(
                builder,
                READ_STDIN_LINE,
                &[pointer],
                Some(pointer),
                &[resources.current_frame],
                resources,
            )?
            .ok_or_else(|| backend_failure("read_line produced no result"))?;
            let present = presence_word(builder, payload, pointer);
            Ok(LoweredValue::Nullable { present, payload })
        }
        mir::NullableStringExpression::Call { function, args } => {
            lower_function_call(builder, *function, args, resources)?
                .ok_or_else(|| malformed_mir("nullable-string call produced no result"))
        }
        mir::NullableStringExpression::NullSafeProperty { object, property } => {
            let owned_receiver = object.owned_temporary_class();
            let object = lower_nullable_class_expression(builder, object, resources)?;
            lower_null_safe_nullable(
                builder,
                object,
                pointer,
                owned_receiver,
                resources,
                |builder, resources| {
                    let address =
                        lower_property_address_from_value(builder, object, *property, resources)?;
                    let ty = property_definition(resources.program, *property)?.ty;
                    let value = load_lowered_from_address(builder, ty, address, pointer);
                    match value {
                        LoweredValue::Single(payload) => Ok(LoweredValue::Single(retain_string(
                            builder, payload, resources,
                        )?)),
                        LoweredValue::Nullable { present, payload } => Ok(LoweredValue::Nullable {
                            present,
                            payload: retain_string(builder, payload, resources)?,
                        }),
                    }
                },
            )
        }
        mir::NullableStringExpression::NullSafeCall {
            object,
            function,
            args,
        } => {
            let owned_receiver = object.owned_temporary_class();
            let object = lower_nullable_class_expression(builder, object, resources)?;
            lower_null_safe_nullable(
                builder,
                object,
                pointer,
                owned_receiver,
                resources,
                |builder, resources| {
                    lower_method_call_with_receiver(builder, object, *function, args, resources)?
                        .ok_or_else(|| malformed_mir("null-safe string call produced no result"))
                },
            )
        }
        mir::NullableStringExpression::Coalesce { left, right } => {
            let left = lower_nullable_string_expression(builder, left, resources)?;
            lower_nullable_coalesce(builder, left, pointer, resources, |builder, resources| {
                lower_nullable_string_expression(builder, right, resources)
            })
        }
    }
}

fn lower_nullable_scalar_expression(
    builder: &mut FunctionBuilder,
    expression: &mir::NullableScalarExpression,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<LoweredValue, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let ty = expression.ty();
    match expression {
        mir::NullableScalarExpression::Null(_) => {
            let present = builder.ins().iconst(pointer, 0);
            let payload = scalar_zero(builder, ty);
            Ok(LoweredValue::Nullable { present, payload })
        }
        mir::NullableScalarExpression::Value(value) => {
            let payload = lower_value_expression(builder, value, resources)?;
            let present = builder.ins().iconst(pointer, 1);
            Ok(LoweredValue::Nullable { present, payload })
        }
        mir::NullableScalarExpression::Local { local, .. } => Ok(load_lowered_from_stack(
            builder,
            mir::Type::NullableScalar(ty),
            local_slot(resources.local_slots, *local)?,
            pointer,
        )),
        mir::NullableScalarExpression::Property {
            object, property, ..
        } => {
            let address = lower_property_address(builder, *object, *property, resources)?;
            Ok(load_lowered_from_address(
                builder,
                mir::Type::NullableScalar(ty),
                address,
                pointer,
            ))
        }
        mir::NullableScalarExpression::Static { id, .. } => {
            let address = lower_static_address(builder, *id, resources)?;
            Ok(load_lowered_from_address(
                builder,
                mir::Type::NullableScalar(ty),
                address,
                pointer,
            ))
        }
        mir::NullableScalarExpression::Call { function, args, .. } => {
            lower_function_call(builder, *function, args, resources)?
                .ok_or_else(|| malformed_mir("nullable-scalar call produced no result"))
        }
        mir::NullableScalarExpression::NullSafeProperty {
            object, property, ..
        } => {
            let owned_receiver = object.owned_temporary_class();
            let object = lower_nullable_class_expression(builder, object, resources)?;
            lower_null_safe_nullable(
                builder,
                object,
                clif_scalar_type(ty),
                owned_receiver,
                resources,
                |builder, resources| {
                    let address =
                        lower_property_address_from_value(builder, object, *property, resources)?;
                    let property_ty = property_definition(resources.program, *property)?.ty;
                    Ok(load_lowered_from_address(
                        builder,
                        property_ty,
                        address,
                        pointer,
                    ))
                },
            )
        }
        mir::NullableScalarExpression::NullSafeCall {
            object,
            function,
            args,
            ..
        } => {
            let owned_receiver = object.owned_temporary_class();
            let object = lower_nullable_class_expression(builder, object, resources)?;
            lower_null_safe_nullable(
                builder,
                object,
                clif_scalar_type(ty),
                owned_receiver,
                resources,
                |builder, resources| {
                    lower_method_call_with_receiver(builder, object, *function, args, resources)?
                        .ok_or_else(|| malformed_mir("null-safe scalar call produced no result"))
                },
            )
        }
        mir::NullableScalarExpression::Coalesce { left, right, .. } => {
            let left = lower_nullable_scalar_expression(builder, left, resources)?;
            lower_nullable_coalesce(
                builder,
                left,
                clif_scalar_type(ty),
                resources,
                |builder, resources| lower_nullable_scalar_expression(builder, right, resources),
            )
        }
    }
}

fn scalar_zero(builder: &mut FunctionBuilder, ty: mir::ScalarType) -> Value {
    match ty {
        mir::ScalarType::Integer(ty) => builder.ins().iconst(clif_integer_type(ty), 0),
        mir::ScalarType::Float(FloatType::Float32) => builder.ins().f32const(Ieee32::with_bits(0)),
        mir::ScalarType::Float(FloatType::Float64) => builder.ins().f64const(Ieee64::with_bits(0)),
        mir::ScalarType::Bool => builder.ins().iconst(types::I8, 0),
    }
}

fn lower_null_safe_nullable(
    builder: &mut FunctionBuilder,
    object: Value,
    payload_type: ClifType,
    owned_receiver: Option<crate::class_layout::ClassId>,
    resources: &mut LoweringResources<'_, '_>,
    present_value: impl FnOnce(
        &mut FunctionBuilder,
        &mut LoweringResources<'_, '_>,
    ) -> Result<LoweredValue, BackendError>,
) -> Result<LoweredValue, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    if let Some(class) = owned_receiver {
        defer_or_drop_class_temporary(builder, object, class, resources)?;
    }
    let present = presence_word(builder, object, pointer);
    let zero = builder.ins().iconst(pointer, 0);
    let is_present = builder.ins().icmp(IntCC::NotEqual, present, zero);
    let some = builder.create_block();
    let none = builder.create_block();
    let done = builder.create_block();
    builder.append_block_param(done, pointer);
    builder.append_block_param(done, payload_type);
    builder.ins().brif(is_present, some, &[], none, &[]);
    builder.switch_to_block(some);
    let value = present_value(builder, resources)?;
    let (present, payload) = match value {
        LoweredValue::Single(payload) => (builder.ins().iconst(pointer, 1), payload),
        LoweredValue::Nullable { present, payload } => (present, payload),
    };
    builder
        .ins()
        .jump(done, &[BlockArg::Value(present), BlockArg::Value(payload)]);
    builder.switch_to_block(none);
    let absent = builder.ins().iconst(pointer, 0);
    let payload = if payload_type.is_float() {
        if payload_type == types::F32 {
            builder.ins().f32const(Ieee32::with_bits(0))
        } else {
            builder.ins().f64const(Ieee64::with_bits(0))
        }
    } else {
        builder.ins().iconst(payload_type, 0)
    };
    builder
        .ins()
        .jump(done, &[BlockArg::Value(absent), BlockArg::Value(payload)]);
    builder.switch_to_block(done);
    let result = LoweredValue::Nullable {
        present: builder.block_params(done)[0],
        payload: builder.block_params(done)[1],
    };
    Ok(result)
}

fn lower_null_safe_statement_call(
    builder: &mut FunctionBuilder,
    object: &mir::NullableClassExpression,
    function: mir::FunctionId,
    args: &[mir::Rvalue],
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let receiver = lower_nullable_class_expression(builder, object, resources)?;
    if let Some(class) = object.owned_temporary_class() {
        defer_or_drop_class_temporary(builder, receiver, class, resources)?;
    }
    let pointer = resources.module.target_config().pointer_type();
    let zero = builder.ins().iconst(pointer, 0);
    let present = builder.ins().icmp(IntCC::NotEqual, receiver, zero);
    let call_block = builder.create_block();
    let done = builder.create_block();
    builder.ins().brif(present, call_block, &[], done, &[]);
    builder.switch_to_block(call_block);
    let _ = lower_method_call_with_receiver(builder, receiver, function, args, resources)?;
    builder.ins().jump(done, &[]);
    builder.switch_to_block(done);
    Ok(())
}

fn lower_coalesce_value(
    builder: &mut FunctionBuilder,
    present: Value,
    payload: Value,
    result_type: ClifType,
    resources: &mut LoweringResources<'_, '_>,
    fallback: impl FnOnce(
        &mut FunctionBuilder,
        &mut LoweringResources<'_, '_>,
    ) -> Result<Value, BackendError>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let zero = builder.ins().iconst(pointer, 0);
    let has_value = builder.ins().icmp(IntCC::NotEqual, present, zero);
    let some = builder.create_block();
    let none = builder.create_block();
    let done = builder.create_block();
    builder.append_block_param(done, result_type);
    builder.ins().brif(has_value, some, &[], none, &[]);
    builder.switch_to_block(some);
    builder.ins().jump(done, &[BlockArg::Value(payload)]);
    builder.switch_to_block(none);
    let fallback = fallback(builder, resources)?;
    builder.ins().jump(done, &[BlockArg::Value(fallback)]);
    builder.switch_to_block(done);
    Ok(builder.block_params(done)[0])
}

fn lower_nullable_coalesce(
    builder: &mut FunctionBuilder,
    left: LoweredValue,
    payload_type: ClifType,
    resources: &mut LoweringResources<'_, '_>,
    fallback: impl FnOnce(
        &mut FunctionBuilder,
        &mut LoweringResources<'_, '_>,
    ) -> Result<LoweredValue, BackendError>,
) -> Result<LoweredValue, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let (present, payload) = left.nullable()?;
    let zero = builder.ins().iconst(pointer, 0);
    let has_value = builder.ins().icmp(IntCC::NotEqual, present, zero);
    let some = builder.create_block();
    let none = builder.create_block();
    let done = builder.create_block();
    builder.append_block_param(done, pointer);
    builder.append_block_param(done, payload_type);
    builder.ins().brif(has_value, some, &[], none, &[]);
    builder.switch_to_block(some);
    builder
        .ins()
        .jump(done, &[BlockArg::Value(present), BlockArg::Value(payload)]);
    builder.switch_to_block(none);
    let (fallback_present, fallback_payload) = fallback(builder, resources)?.nullable()?;
    builder.ins().jump(
        done,
        &[
            BlockArg::Value(fallback_present),
            BlockArg::Value(fallback_payload),
        ],
    );
    builder.switch_to_block(done);
    Ok(LoweredValue::Nullable {
        present: builder.block_params(done)[0],
        payload: builder.block_params(done)[1],
    })
}

fn lower_format_expression(
    builder: &mut FunctionBuilder,
    format: &mir::FormatExpression,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let mut result = lower_string_expression(
        builder,
        &mir::StringExpression::Literal(String::new()),
        resources,
    )?;
    for piece in &format.pieces {
        let next = match piece {
            FormatPiece::Literal(value) => lower_string_expression(
                builder,
                &mir::StringExpression::Literal(value.clone()),
                resources,
            )?,
            FormatPiece::Argument { index, spec } => {
                let argument = format
                    .arguments
                    .get(*index as usize)
                    .ok_or_else(|| malformed_mir("format argument index is out of bounds"))?;
                lower_format_argument(builder, argument, *spec, resources)?
            }
        };
        let concatenated = runtime_call(
            builder,
            STRING_CONCAT,
            &[pointer, pointer],
            Some(pointer),
            &[result, next],
            resources,
        )?
        .ok_or_else(|| backend_failure("format concatenation produced no result"))?;
        release_string(builder, result, resources)?;
        release_string(builder, next, resources)?;
        result = concatenated;
    }
    Ok(result)
}

fn lower_format_argument(
    builder: &mut FunctionBuilder,
    argument: &mir::FormatArgument,
    spec: crate::format_string::FormatSpec,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let width = builder
        .ins()
        .iconst(types::I32, i64::from(spec.width.unwrap_or(0)));
    let flags_value = u8::from(spec.left_align) | (u8::from(spec.zero_pad) << 1);
    let flags = builder.ins().iconst(types::I8, i64::from(flags_value));
    if spec.conversion == FormatConversion::Display {
        let string = match argument {
            mir::FormatArgument::String(value) | mir::FormatArgument::ClassDisplay(value) => {
                lower_string_expression(builder, value, resources)?
            }
            mir::FormatArgument::Value(value) => lower_string_expression(
                builder,
                &mir::StringExpression::Display(value.clone()),
                resources,
            )?,
        };
        let formatted = runtime_call(
            builder,
            FORMAT_STRING,
            &[pointer, types::I32, types::I8],
            Some(pointer),
            &[string, width, flags],
            resources,
        )?
        .ok_or_else(|| backend_failure("string formatting produced no result"))?;
        release_string(builder, string, resources)?;
        return Ok(formatted);
    }

    if let mir::FormatArgument::Value(mir::ValueExpression::Float(float)) = argument {
        let value = lower_float_expression(builder, float, resources)?;
        let precision = builder
            .ins()
            .iconst(types::I32, i64::from(spec.precision.unwrap_or(6)));
        let (name, ty) = match float.ty() {
            FloatType::Float32 => (FORMAT_F32, types::F32),
            FloatType::Float64 => (FORMAT_F64, types::F64),
        };
        return runtime_call(
            builder,
            name,
            &[ty, types::I32, types::I32, types::I8],
            Some(pointer),
            &[value, precision, width, flags],
            resources,
        )?
        .ok_or_else(|| backend_failure("float formatting produced no result"));
    }

    let mir::FormatArgument::Value(mir::ValueExpression::Integer(integer)) = argument else {
        return Err(malformed_mir(
            "format conversion and argument type disagree",
        ));
    };
    let ty = integer.ty();
    let mut value = lower_integer_expression(builder, integer, resources)?;
    if ty.bit_width() < 64 {
        value = if ty.is_signed() {
            builder.ins().sextend(types::I64, value)
        } else {
            builder.ins().uextend(types::I64, value)
        };
    }
    let conversion = match spec.conversion {
        FormatConversion::Decimal => 1,
        FormatConversion::HexLower => 2,
        FormatConversion::HexUpper => 3,
        FormatConversion::Octal => 4,
        FormatConversion::Binary => 5,
        _ => {
            return Err(malformed_mir(
                "integer argument has non-integer format conversion",
            ))
        }
    };
    let conversion = builder.ins().iconst(types::I8, conversion);
    let (name, params, values) = if ty.is_signed() {
        let bit_width = builder.ins().iconst(types::I8, i64::from(ty.bit_width()));
        (
            FORMAT_I64,
            vec![types::I64, types::I8, types::I8, types::I32, types::I8],
            vec![value, bit_width, conversion, width, flags],
        )
    } else {
        (
            FORMAT_U64,
            vec![types::I64, types::I8, types::I32, types::I8],
            vec![value, conversion, width, flags],
        )
    };
    runtime_call(builder, name, &params, Some(pointer), &values, resources)?
        .ok_or_else(|| backend_failure("integer formatting produced no result"))
}

fn lower_integer_expression(
    builder: &mut FunctionBuilder,
    expression: &mir::IntegerExpression,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    match expression {
        mir::IntegerExpression::Use { ty, operand } => {
            lower_integer_operand(builder, *ty, operand, resources)
        }
        mir::IntegerExpression::Unary { ty, op, operand } => {
            let operand = lower_integer_expression(builder, operand, resources)?;
            lower_integer_unary(builder, *ty, *op, operand, resources)
        }
        mir::IntegerExpression::Binary {
            ty,
            op,
            left,
            right,
        } => {
            let left = lower_integer_expression(builder, left, resources)?;
            let right = lower_integer_expression(builder, right, resources)?;
            lower_integer_binary(builder, *ty, *op, left, right, resources)
        }
        mir::IntegerExpression::Convert { ty, value } => {
            let source_type = value.ty();
            let value = lower_integer_expression(builder, value, resources)?;
            lower_integer_conversion(builder, source_type, *ty, value, resources)
        }
        mir::IntegerExpression::FloatToInt { value } => {
            let value = lower_float_expression(builder, value, resources)?;
            lower_float_to_int(builder, value, resources)
        }
        mir::IntegerExpression::Call { ty, function, args } => {
            lower_integer_call(builder, *ty, *function, args, resources)
        }
        mir::IntegerExpression::Coalesce { ty, left, right } => {
            let left = lower_nullable_scalar_expression(builder, left, resources)?;
            let (present, payload) = left.nullable()?;
            lower_coalesce_value(
                builder,
                present,
                payload,
                clif_integer_type(*ty),
                resources,
                |builder, resources| lower_integer_expression(builder, right, resources),
            )
        }
    }
}

fn lower_integer_unary(
    builder: &mut FunctionBuilder,
    ty: IntegerType,
    op: mir::IntegerUnaryOp,
    operand: Value,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    match op {
        mir::IntegerUnaryOp::Negate => {
            let zero = builder.ins().iconst(clif_integer_type(ty), 0);
            let (value, overflow) = builder.ins().ssub_overflow(zero, operand);
            lower_panic_if(builder, overflow, IntegerPanic::OverflowNegation, resources)?;
            Ok(value)
        }
        mir::IntegerUnaryOp::BitwiseNot => Ok(builder.ins().bnot(operand)),
    }
}

fn lower_integer_binary(
    builder: &mut FunctionBuilder,
    ty: IntegerType,
    op: mir::IntegerBinaryOp,
    left: Value,
    right: Value,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    match op {
        mir::IntegerBinaryOp::Add
        | mir::IntegerBinaryOp::Subtract
        | mir::IntegerBinaryOp::Multiply => {
            lower_checked_arithmetic(builder, ty, op, left, right, resources)
        }
        mir::IntegerBinaryOp::Divide => lower_integer_division(builder, ty, left, right, resources),
        mir::IntegerBinaryOp::Remainder => {
            lower_integer_remainder(builder, ty, left, right, resources)
        }
        mir::IntegerBinaryOp::ShiftLeft | mir::IntegerBinaryOp::ShiftRight => {
            lower_integer_shift(builder, ty, op, left, right, resources)
        }
        mir::IntegerBinaryOp::BitwiseAnd => Ok(builder.ins().band(left, right)),
        mir::IntegerBinaryOp::BitwiseXor => Ok(builder.ins().bxor(left, right)),
        mir::IntegerBinaryOp::BitwiseOr => Ok(builder.ins().bor(left, right)),
    }
}

fn lower_checked_arithmetic(
    builder: &mut FunctionBuilder,
    ty: IntegerType,
    op: mir::IntegerBinaryOp,
    left: Value,
    right: Value,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let (value, overflow) = match op {
        mir::IntegerBinaryOp::Add if ty.is_signed() => builder.ins().sadd_overflow(left, right),
        mir::IntegerBinaryOp::Add => builder.ins().uadd_overflow(left, right),
        mir::IntegerBinaryOp::Subtract if ty.is_signed() => {
            builder.ins().ssub_overflow(left, right)
        }
        mir::IntegerBinaryOp::Subtract => builder.ins().usub_overflow(left, right),
        mir::IntegerBinaryOp::Multiply if ty.is_signed() => {
            builder.ins().smul_overflow(left, right)
        }
        mir::IntegerBinaryOp::Multiply => builder.ins().umul_overflow(left, right),
        _ => unreachable!("non-arithmetic operator reached checked arithmetic lowering"),
    };
    let panic = match op {
        mir::IntegerBinaryOp::Add => IntegerPanic::OverflowAddition,
        mir::IntegerBinaryOp::Subtract => IntegerPanic::OverflowSubtraction,
        mir::IntegerBinaryOp::Multiply => IntegerPanic::OverflowMultiplication,
        _ => unreachable!("non-arithmetic operator reached checked arithmetic lowering"),
    };
    lower_panic_if(builder, overflow, panic, resources)?;
    Ok(value)
}

fn lower_integer_division(
    builder: &mut FunctionBuilder,
    ty: IntegerType,
    left: Value,
    right: Value,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let zero = builder.ins().iconst(clif_integer_type(ty), 0);
    let divides_by_zero = builder.ins().icmp(IntCC::Equal, right, zero);
    lower_panic_if(
        builder,
        divides_by_zero,
        IntegerPanic::DivisionByZero,
        resources,
    )?;

    if ty.is_signed() {
        let minimum = integer_constant(
            builder,
            IntegerValue::from_bits(ty, 1_u64 << (ty.bit_width() - 1)),
        );
        let negative_one = integer_constant(builder, IntegerValue::from_bits(ty, ty.mask()));
        let is_minimum = builder.ins().icmp(IntCC::Equal, left, minimum);
        let is_negative_one = builder.ins().icmp(IntCC::Equal, right, negative_one);
        let overflows = builder.ins().band(is_minimum, is_negative_one);
        lower_panic_if(
            builder,
            overflows,
            IntegerPanic::DivisionOverflow,
            resources,
        )?;
        Ok(builder.ins().sdiv(left, right))
    } else {
        Ok(builder.ins().udiv(left, right))
    }
}

fn lower_integer_remainder(
    builder: &mut FunctionBuilder,
    ty: IntegerType,
    left: Value,
    right: Value,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let zero = builder.ins().iconst(clif_integer_type(ty), 0);
    let divides_by_zero = builder.ins().icmp(IntCC::Equal, right, zero);
    lower_panic_if(
        builder,
        divides_by_zero,
        IntegerPanic::RemainderByZero,
        resources,
    )?;

    if !ty.is_signed() {
        return Ok(builder.ins().urem(left, right));
    }

    let minimum = integer_constant(
        builder,
        IntegerValue::from_bits(ty, 1_u64 << (ty.bit_width() - 1)),
    );
    let negative_one = integer_constant(builder, IntegerValue::from_bits(ty, ty.mask()));
    let is_minimum = builder.ins().icmp(IntCC::Equal, left, minimum);
    let is_negative_one = builder.ins().icmp(IntCC::Equal, right, negative_one);
    let special_case = builder.ins().band(is_minimum, is_negative_one);
    let zero_block = builder.create_block();
    let remainder_block = builder.create_block();
    let done_block = builder.create_block();
    builder.append_block_param(done_block, clif_integer_type(ty));
    builder
        .ins()
        .brif(special_case, zero_block, &[], remainder_block, &[]);

    builder.switch_to_block(zero_block);
    builder.ins().jump(done_block, &[BlockArg::Value(zero)]);

    builder.switch_to_block(remainder_block);
    let remainder = builder.ins().srem(left, right);
    builder
        .ins()
        .jump(done_block, &[BlockArg::Value(remainder)]);

    builder.switch_to_block(done_block);
    Ok(builder.block_params(done_block)[0])
}

fn lower_integer_shift(
    builder: &mut FunctionBuilder,
    ty: IntegerType,
    op: mir::IntegerBinaryOp,
    left: Value,
    right: Value,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let width = builder
        .ins()
        .iconst(clif_integer_type(ty), ty.bit_width() as i64);
    let too_large = builder
        .ins()
        .icmp(IntCC::UnsignedGreaterThanOrEqual, right, width);
    let invalid = if ty.is_signed() {
        let zero = builder.ins().iconst(clif_integer_type(ty), 0);
        let negative = builder.ins().icmp(IntCC::SignedLessThan, right, zero);
        builder.ins().bor(negative, too_large)
    } else {
        too_large
    };
    lower_panic_if(
        builder,
        invalid,
        IntegerPanic::ShiftCountOutOfRange,
        resources,
    )?;

    match op {
        mir::IntegerBinaryOp::ShiftLeft => Ok(builder.ins().ishl(left, right)),
        mir::IntegerBinaryOp::ShiftRight if ty.is_signed() => Ok(builder.ins().sshr(left, right)),
        mir::IntegerBinaryOp::ShiftRight => Ok(builder.ins().ushr(left, right)),
        _ => unreachable!("non-shift operator reached shift lowering"),
    }
}

fn lower_integer_conversion(
    builder: &mut FunctionBuilder,
    source: IntegerType,
    target: IntegerType,
    value: Value,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    if let Some(out_of_range) = conversion_out_of_range(builder, source, target, value) {
        lower_panic_if(
            builder,
            out_of_range,
            IntegerPanic::ConversionOutOfRange,
            resources,
        )?;
    }

    Ok(match target.bit_width().cmp(&source.bit_width()) {
        std::cmp::Ordering::Equal => value,
        std::cmp::Ordering::Less => builder.ins().ireduce(clif_integer_type(target), value),
        std::cmp::Ordering::Greater if source.is_signed() => {
            builder.ins().sextend(clif_integer_type(target), value)
        }
        std::cmp::Ordering::Greater => builder.ins().uextend(clif_integer_type(target), value),
    })
}

fn conversion_out_of_range(
    builder: &mut FunctionBuilder,
    source: IntegerType,
    target: IntegerType,
    value: Value,
) -> Option<Value> {
    match (source.is_signed(), target.is_signed()) {
        (true, true) if target.bit_width() < source.bit_width() => {
            let minimum = integer_constant(
                builder,
                IntegerValue::from_i128(source, target.min_value())
                    .expect("narrow signed minimum fits source"),
            );
            let maximum = integer_constant(
                builder,
                IntegerValue::from_i128(source, target.max_value())
                    .expect("narrow signed maximum fits source"),
            );
            let below = builder.ins().icmp(IntCC::SignedLessThan, value, minimum);
            let above = builder.ins().icmp(IntCC::SignedGreaterThan, value, maximum);
            Some(builder.ins().bor(below, above))
        }
        (true, false) => {
            let zero = builder.ins().iconst(clif_integer_type(source), 0);
            let negative = builder.ins().icmp(IntCC::SignedLessThan, value, zero);
            if target.bit_width() < source.bit_width() {
                let maximum = integer_constant(
                    builder,
                    IntegerValue::from_u128(source, target.max_value() as u128)
                        .expect("narrow unsigned maximum fits signed source"),
                );
                let above = builder
                    .ins()
                    .icmp(IntCC::UnsignedGreaterThan, value, maximum);
                Some(builder.ins().bor(negative, above))
            } else {
                Some(negative)
            }
        }
        (false, false) if target.bit_width() < source.bit_width() => {
            let maximum = integer_constant(
                builder,
                IntegerValue::from_u128(source, target.max_value() as u128)
                    .expect("narrow unsigned maximum fits source"),
            );
            Some(
                builder
                    .ins()
                    .icmp(IntCC::UnsignedGreaterThan, value, maximum),
            )
        }
        (false, true) if target.bit_width() <= source.bit_width() => {
            let maximum = integer_constant(
                builder,
                IntegerValue::from_u128(source, target.max_value() as u128)
                    .expect("signed maximum fits unsigned source"),
            );
            Some(
                builder
                    .ins()
                    .icmp(IntCC::UnsignedGreaterThan, value, maximum),
            )
        }
        _ => None,
    }
}

fn lower_panic_if(
    builder: &mut FunctionBuilder,
    condition: Value,
    panic: IntegerPanic,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    lower_panic_if_message(builder, condition, panic.message().as_bytes(), resources)
}

fn lower_panic_if_message(
    builder: &mut FunctionBuilder,
    condition: Value,
    message: &[u8],
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let panic_block = builder.create_block();
    let continue_block = builder.create_block();
    builder
        .ins()
        .brif(condition, panic_block, &[], continue_block, &[]);

    builder.switch_to_block(panic_block);
    lower_runtime_panic(builder, message, resources)?;

    builder.switch_to_block(continue_block);
    Ok(())
}

fn integer_constant(builder: &mut FunctionBuilder, value: IntegerValue) -> Value {
    builder
        .ins()
        .iconst(clif_integer_type(value.ty), value.bits as i64)
}

fn lower_integer_operand(
    builder: &mut FunctionBuilder,
    ty: IntegerType,
    operand: &mir::Operand,
    resources: &LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    match operand {
        mir::Operand::Scalar(mir::ScalarValue::Integer(value)) => {
            if value.ty != ty {
                return Err(malformed_mir(format!(
                    "{ty} expression contains {} constant",
                    value.ty
                )));
            }
            Ok(integer_constant(builder, *value))
        }
        mir::Operand::Local(id) => {
            let definition = local_definition(resources.program, resources.function_id, *id)?;
            if definition.ty != mir::Type::Scalar(mir::ScalarType::Integer(ty)) {
                return Err(malformed_mir(format!(
                    "{ty} expression reads local{} with type {}",
                    id.0, definition.ty
                )));
            }
            let slot = local_slot(resources.local_slots, *id)?;
            Ok(builder.ins().stack_load(clif_integer_type(ty), slot, 0))
        }
        mir::Operand::NullablePayload(id) => {
            let definition = local_definition(resources.program, resources.function_id, *id)?;
            if definition.ty != mir::Type::NullableScalar(mir::ScalarType::Integer(ty)) {
                return Err(malformed_mir(format!(
                    "{ty} expression reads nullable payload local{} with type {}",
                    id.0, definition.ty
                )));
            }
            let pointer = resources.module.target_config().pointer_type();
            Ok(builder.ins().stack_load(
                clif_integer_type(ty),
                local_slot(resources.local_slots, *id)?,
                pointer.bytes() as i32,
            ))
        }
        mir::Operand::Static(id) => {
            let address = lower_static_address(builder, *id, resources)?;
            Ok(builder.ins().load(
                clif_integer_type(ty),
                cranelift_codegen::ir::MachMemFlags::trusted(),
                address,
                0,
            ))
        }
        mir::Operand::Property { object, property } => {
            let address = lower_property_address(builder, *object, *property, resources)?;
            Ok(builder.ins().load(
                clif_integer_type(ty),
                cranelift_codegen::ir::MachMemFlags::trusted(),
                address,
                0,
            ))
        }
        mir::Operand::Scalar(_) => Err(malformed_mir(
            "integer expression contains non-integer constant",
        )),
    }
}

fn float_constant(builder: &mut FunctionBuilder, value: FloatValue) -> Value {
    match value.ty {
        FloatType::Float32 => builder.ins().f32const(Ieee32::with_bits(value.bits as u32)),
        FloatType::Float64 => builder.ins().f64const(Ieee64::with_bits(value.bits)),
    }
}

fn lower_float_expression(
    builder: &mut FunctionBuilder,
    expression: &mir::FloatExpression,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    match expression {
        mir::FloatExpression::Use { ty, operand } => match operand {
            mir::Operand::Scalar(mir::ScalarValue::Float(value)) if value.ty == *ty => {
                Ok(float_constant(builder, *value))
            }
            mir::Operand::Local(id) => {
                let expected = mir::Type::Scalar(mir::ScalarType::Float(*ty));
                let definition = local_definition(resources.program, resources.function_id, *id)?;
                if definition.ty != expected {
                    return Err(malformed_mir(format!(
                        "{ty} expression reads local{} with type {}",
                        id.0, definition.ty
                    )));
                }
                Ok(builder.ins().stack_load(
                    clif_scalar_type(mir::ScalarType::Float(*ty)),
                    local_slot(resources.local_slots, *id)?,
                    0,
                ))
            }
            mir::Operand::NullablePayload(id) => {
                let expected = mir::Type::NullableScalar(mir::ScalarType::Float(*ty));
                let definition = local_definition(resources.program, resources.function_id, *id)?;
                if definition.ty != expected {
                    return Err(malformed_mir(format!(
                        "{ty} expression reads nullable payload local{} with type {}",
                        id.0, definition.ty
                    )));
                }
                let pointer = resources.module.target_config().pointer_type();
                Ok(builder.ins().stack_load(
                    clif_scalar_type(mir::ScalarType::Float(*ty)),
                    local_slot(resources.local_slots, *id)?,
                    pointer.bytes() as i32,
                ))
            }
            mir::Operand::Static(id) => {
                let address = lower_static_address(builder, *id, resources)?;
                Ok(builder.ins().load(
                    clif_scalar_type(mir::ScalarType::Float(*ty)),
                    cranelift_codegen::ir::MachMemFlags::trusted(),
                    address,
                    0,
                ))
            }
            mir::Operand::Property { object, property } => {
                let address = lower_property_address(builder, *object, *property, resources)?;
                Ok(builder.ins().load(
                    clif_scalar_type(mir::ScalarType::Float(*ty)),
                    cranelift_codegen::ir::MachMemFlags::trusted(),
                    address,
                    0,
                ))
            }
            _ => Err(malformed_mir(
                "float expression contains non-float constant",
            )),
        },
        mir::FloatExpression::Negate { operand, .. } => {
            let operand = lower_float_expression(builder, operand, resources)?;
            Ok(builder.ins().fneg(operand))
        }
        mir::FloatExpression::Binary {
            op, left, right, ..
        } => {
            let left = lower_float_expression(builder, left, resources)?;
            let right = lower_float_expression(builder, right, resources)?;
            Ok(match op {
                mir::FloatBinaryOp::Add => builder.ins().fadd(left, right),
                mir::FloatBinaryOp::Subtract => builder.ins().fsub(left, right),
                mir::FloatBinaryOp::Multiply => builder.ins().fmul(left, right),
                mir::FloatBinaryOp::Divide => builder.ins().fdiv(left, right),
            })
        }
        mir::FloatExpression::IntToFloat { value } => {
            if value.ty() != IntegerType::Int64 {
                return Err(malformed_mir("Int::toFloat operand is not canonical int"));
            }
            let value = lower_integer_expression(builder, value, resources)?;
            Ok(builder.ins().fcvt_from_sint(types::F64, value))
        }
        mir::FloatExpression::Call { function, args, .. } => {
            lower_scalar_call(builder, *function, args, resources)
        }
        mir::FloatExpression::Coalesce { ty, left, right } => {
            let left = lower_nullable_scalar_expression(builder, left, resources)?;
            let (present, payload) = left.nullable()?;
            lower_coalesce_value(
                builder,
                present,
                payload,
                clif_scalar_type(mir::ScalarType::Float(*ty)),
                resources,
                |builder, resources| lower_float_expression(builder, right, resources),
            )
        }
    }
}

fn lower_float_to_int(
    builder: &mut FunctionBuilder,
    value: Value,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let minimum = builder.ins().f64const(Ieee64::with_bits(
        (-9_223_372_036_854_775_808.0_f64).to_bits(),
    ));
    let maximum = builder.ins().f64const(Ieee64::with_bits(
        (9_223_372_036_854_775_808.0_f64).to_bits(),
    ));
    let unordered = builder.ins().fcmp(FloatCC::Unordered, value, value);
    let below = builder.ins().fcmp(FloatCC::LessThan, value, minimum);
    let above = builder
        .ins()
        .fcmp(FloatCC::GreaterThanOrEqual, value, maximum);
    let invalid_range = builder.ins().bor(below, above);
    let invalid = builder.ins().bor(unordered, invalid_range);
    lower_panic_if_message(
        builder,
        invalid,
        b"float-to-integer conversion out of range",
        resources,
    )?;
    Ok(builder.ins().fcvt_to_sint(types::I64, value))
}

fn lower_integer_call(
    builder: &mut FunctionBuilder,
    ty: IntegerType,
    function: mir::FunctionId,
    args: &[mir::Rvalue],
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    lower_function_call(builder, function, args, resources)?
        .ok_or_else(|| {
            malformed_mir(format!(
                "{ty} call to function{} produced no result",
                function.0,
            ))
        })?
        .single()
}

struct LoweredCallArgs {
    arguments: Vec<LoweredValue>,
    abi_values: Vec<Value>,
    owned_strings: Vec<(usize, Value)>,
}

fn lower_call_args(
    builder: &mut FunctionBuilder,
    args: &[mir::Rvalue],
    resources: &mut LoweringResources<'_, '_>,
) -> Result<LoweredCallArgs, BackendError> {
    let mut arguments = Vec::with_capacity(args.len());
    let mut abi_values = Vec::with_capacity(args.len() * 2);
    let mut owned_strings = Vec::new();
    for (index, argument) in args.iter().enumerate() {
        let value = lower_rvalue(builder, argument, resources)?;
        if matches!(argument.ty(), mir::Type::String | mir::Type::NullableString) {
            let string = match argument.ty() {
                mir::Type::NullableString => value.nullable()?.1,
                _ => value.single()?,
            };
            owned_strings.push((index, string));
        }
        value.append_to(&mut abi_values);
        arguments.push(value);
    }
    Ok(LoweredCallArgs {
        arguments,
        abi_values,
        owned_strings,
    })
}

fn lower_function_call(
    builder: &mut FunctionBuilder,
    function: mir::FunctionId,
    args: &[mir::Rvalue],
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Option<LoweredValue>, BackendError> {
    let lowered = lower_call_args(builder, args, resources)?;
    let mut values = vec![resources.current_frame];
    values.extend(lowered.abi_values.iter().copied());
    let callee = declared_function(builder, resources, function)?;
    let call = builder.ins().call(callee, &values);
    let results = builder.inst_results(call);
    let callee_definition = function_in(resources.program, function)?;
    let result = match callee_definition.return_type {
        mir::ReturnType::Void => None,
        mir::ReturnType::Value(mir::Type::NullableScalar(_) | mir::Type::NullableString) => {
            Some(LoweredValue::Nullable {
                present: *results
                    .first()
                    .ok_or_else(|| malformed_mir("nullable call produced no presence result"))?,
                payload: *results
                    .get(1)
                    .ok_or_else(|| malformed_mir("nullable call produced no payload result"))?,
            })
        }
        mir::ReturnType::Value(_) => {
            Some(LoweredValue::Single(*results.first().ok_or_else(|| {
                malformed_mir("value call produced no result")
            })?))
        }
    };
    for (_, string) in lowered.owned_strings {
        release_string(builder, string, resources)?;
    }
    for (index, argument) in args.iter().enumerate() {
        let Some(class) = argument.owned_temporary_class() else {
            continue;
        };
        let parameter = *callee_definition.params.get(index).ok_or_else(|| {
            malformed_mir(format!(
                "function{} is missing parameter {index}",
                function.0
            ))
        })?;
        if !local_in(callee_definition, parameter)?.owned {
            let value = lowered.arguments[index].single()?;
            defer_or_drop_class_temporary(builder, value, class, resources)?;
        }
    }
    Ok(result)
}

fn lower_method_call_with_receiver(
    builder: &mut FunctionBuilder,
    receiver: Value,
    function: mir::FunctionId,
    args: &[mir::Rvalue],
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Option<LoweredValue>, BackendError> {
    let lowered = lower_call_args(builder, args, resources)?;
    let mut values = vec![resources.current_frame, receiver];
    values.extend(lowered.abi_values.iter().copied());
    let callee = declared_function(builder, resources, function)?;
    let call = builder.ins().call(callee, &values);
    let definition = function_in(resources.program, function)?;
    let results = builder.inst_results(call);
    let result = match definition.return_type {
        mir::ReturnType::Void => None,
        mir::ReturnType::Value(mir::Type::NullableScalar(_) | mir::Type::NullableString) => {
            Some(LoweredValue::Nullable {
                present: *results
                    .first()
                    .ok_or_else(|| malformed_mir("nullable method call has no presence result"))?,
                payload: *results
                    .get(1)
                    .ok_or_else(|| malformed_mir("nullable method call has no payload result"))?,
            })
        }
        mir::ReturnType::Value(_) => Some(LoweredValue::Single(
            *results
                .first()
                .ok_or_else(|| malformed_mir("method call has no result"))?,
        )),
    };
    for (_, string) in lowered.owned_strings {
        release_string(builder, string, resources)?;
    }
    for (index, argument) in args.iter().enumerate() {
        let Some(class) = argument.owned_temporary_class() else {
            continue;
        };
        let parameter = *definition.params.get(index + 1).ok_or_else(|| {
            malformed_mir(format!(
                "method function{} is missing parameter {}",
                function.0,
                index + 1
            ))
        })?;
        if !local_in(definition, parameter)?.owned {
            defer_or_drop_class_temporary(
                builder,
                lowered.arguments[index].single()?,
                class,
                resources,
            )?;
        }
    }
    Ok(result)
}

fn lower_scalar_call(
    builder: &mut FunctionBuilder,
    function: mir::FunctionId,
    args: &[mir::Rvalue],
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    lower_function_call(builder, function, args, resources)?
        .ok_or_else(|| malformed_mir(format!("call to function{} produced no result", function.0)))?
        .single()
}

fn declared_function(
    builder: &mut FunctionBuilder,
    resources: &mut LoweringResources<'_, '_>,
    function: mir::FunctionId,
) -> Result<cranelift_codegen::ir::FuncRef, BackendError> {
    let function_id = *resources
        .function_ids
        .get(function.0)
        .ok_or_else(|| malformed_mir(format!("function{} does not exist", function.0)))?;
    Ok(resources
        .module
        .declare_func_in_func(function_id, builder.func))
}

fn lower_condition_to_branch(
    builder: &mut FunctionBuilder,
    condition: &mir::BoolExpression,
    then_block: Block,
    else_block: Block,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    match condition {
        mir::BoolExpression::Use { operand } => {
            let value = lower_bool_operand(builder, operand, resources)?;
            builder.ins().brif(value, then_block, &[], else_block, &[]);
        }
        mir::BoolExpression::Compare { op, left, right } => {
            let ty = left.ty();
            let left = lower_value_expression(builder, left, resources)?;
            let right = lower_value_expression(builder, right, resources)?;
            let value = match ty {
                mir::ScalarType::Integer(ty) => {
                    builder.ins().icmp(compare_code(*op, ty), left, right)
                }
                mir::ScalarType::Float(_) => {
                    builder.ins().fcmp(float_compare_code(*op), left, right)
                }
                mir::ScalarType::Bool => match op {
                    mir::CompareOp::Equal => builder.ins().icmp(IntCC::Equal, left, right),
                    mir::CompareOp::NotEqual => builder.ins().icmp(IntCC::NotEqual, left, right),
                    _ => return Err(malformed_mir("ordered bool comparison is invalid")),
                },
            };
            builder.ins().brif(value, then_block, &[], else_block, &[]);
        }
        mir::BoolExpression::StringCompare { op, left, right } => {
            let pointer = resources.module.target_config().pointer_type();
            let left = lower_string_expression(builder, left, resources)?;
            let right = lower_string_expression(builder, right, resources)?;
            let compared = runtime_call(
                builder,
                STRING_COMPARE,
                &[pointer, pointer],
                Some(types::I32),
                &[left, right],
                resources,
            )?
            .ok_or_else(|| backend_failure("string comparison produced no result"))?;
            release_string(builder, left, resources)?;
            release_string(builder, right, resources)?;
            let zero = builder.ins().iconst(types::I32, 0);
            let code = match op {
                mir::CompareOp::Equal => IntCC::Equal,
                mir::CompareOp::NotEqual => IntCC::NotEqual,
                mir::CompareOp::Less => IntCC::SignedLessThan,
                mir::CompareOp::LessEqual => IntCC::SignedLessThanOrEqual,
                mir::CompareOp::Greater => IntCC::SignedGreaterThan,
                mir::CompareOp::GreaterEqual => IntCC::SignedGreaterThanOrEqual,
            };
            let value = builder.ins().icmp(code, compared, zero);
            builder.ins().brif(value, then_block, &[], else_block, &[]);
        }
        mir::BoolExpression::NullableStringCompare { op, left, right } => {
            let pointer = resources.module.target_config().pointer_type();
            let left = lower_nullable_string_expression(builder, left, resources)?;
            let right = lower_nullable_string_expression(builder, right, resources)?;
            let (_, left) = left.nullable()?;
            let (_, right) = right.nullable()?;
            let equal = runtime_call(
                builder,
                NULLABLE_STRING_EQUAL,
                &[pointer, pointer],
                Some(types::I8),
                &[left, right],
                resources,
            )?
            .ok_or_else(|| backend_failure("nullable-string comparison produced no result"))?;
            release_string(builder, left, resources)?;
            release_string(builder, right, resources)?;
            let value = match op {
                mir::CompareOp::Equal => equal,
                mir::CompareOp::NotEqual => {
                    let zero = builder.ins().iconst(types::I8, 0);
                    builder.ins().icmp(IntCC::Equal, equal, zero)
                }
                _ => return Err(malformed_mir("ordered nullable comparison is invalid")),
            };
            builder.ins().brif(value, then_block, &[], else_block, &[]);
        }
        mir::BoolExpression::Not(condition) => {
            lower_condition_to_branch(builder, condition, else_block, then_block, resources)?;
        }
        mir::BoolExpression::Binary {
            op: mir::BoolBinaryOp::And,
            left,
            right,
        } => {
            let right_block = builder.create_block();
            lower_condition_to_branch(builder, left, right_block, else_block, resources)?;
            builder.switch_to_block(right_block);
            lower_condition_to_branch(builder, right, then_block, else_block, resources)?;
        }
        mir::BoolExpression::Binary {
            op: mir::BoolBinaryOp::Or,
            left,
            right,
        } => {
            let right_block = builder.create_block();
            lower_condition_to_branch(builder, left, then_block, right_block, resources)?;
            builder.switch_to_block(right_block);
            lower_condition_to_branch(builder, right, then_block, else_block, resources)?;
        }
        mir::BoolExpression::Binary {
            op: mir::BoolBinaryOp::Xor,
            left,
            right,
        } => {
            let left = lower_condition_value(builder, left, resources)?;
            let right = lower_condition_value(builder, right, resources)?;
            let value = builder.ins().icmp(IntCC::NotEqual, left, right);
            builder.ins().brif(value, then_block, &[], else_block, &[]);
        }
        mir::BoolExpression::Call { function, args } => {
            let value = lower_scalar_call(builder, *function, args, resources)?;
            builder.ins().brif(value, then_block, &[], else_block, &[]);
        }
        mir::BoolExpression::NullableScalarIsPresent(value) => {
            let value = lower_nullable_scalar_expression(builder, value, resources)?;
            let (present, _) = value.nullable()?;
            let pointer = resources.module.target_config().pointer_type();
            let zero = builder.ins().iconst(pointer, 0);
            let present = builder.ins().icmp(IntCC::NotEqual, present, zero);
            builder
                .ins()
                .brif(present, then_block, &[], else_block, &[]);
        }
        mir::BoolExpression::NullableClassIsPresent(value) => {
            let owned = value.owned_temporary_class();
            let value = lower_nullable_class_expression(builder, value, resources)?;
            if let Some(class) = owned {
                defer_or_drop_class_temporary(builder, value, class, resources)?;
            }
            let pointer = resources.module.target_config().pointer_type();
            let zero = builder.ins().iconst(pointer, 0);
            let present = builder.ins().icmp(IntCC::NotEqual, value, zero);
            builder
                .ins()
                .brif(present, then_block, &[], else_block, &[]);
        }
        mir::BoolExpression::Coalesce { left, right } => {
            let left = lower_nullable_scalar_expression(builder, left, resources)?;
            let (present, payload) = left.nullable()?;
            let pointer = resources.module.target_config().pointer_type();
            let zero = builder.ins().iconst(pointer, 0);
            let present = builder.ins().icmp(IntCC::NotEqual, present, zero);
            let use_left = builder.create_block();
            let use_right = builder.create_block();
            builder.ins().brif(present, use_left, &[], use_right, &[]);
            builder.switch_to_block(use_left);
            builder
                .ins()
                .brif(payload, then_block, &[], else_block, &[]);
            builder.switch_to_block(use_right);
            lower_condition_to_branch(builder, right, then_block, else_block, resources)?;
        }
    }
    Ok(())
}

fn lower_condition_value(
    builder: &mut FunctionBuilder,
    condition: &mir::BoolExpression,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let true_block = builder.create_block();
    let false_block = builder.create_block();
    let done_block = builder.create_block();
    builder.append_block_param(done_block, types::I8);

    lower_condition_to_branch(builder, condition, true_block, false_block, resources)?;

    builder.switch_to_block(true_block);
    let true_value = builder.ins().iconst(types::I8, 1);
    builder
        .ins()
        .jump(done_block, &[BlockArg::Value(true_value)]);

    builder.switch_to_block(false_block);
    let false_value = builder.ins().iconst(types::I8, 0);
    builder
        .ins()
        .jump(done_block, &[BlockArg::Value(false_value)]);

    builder.switch_to_block(done_block);
    Ok(builder.block_params(done_block)[0])
}

fn compare_code(op: mir::CompareOp, ty: IntegerType) -> IntCC {
    match op {
        mir::CompareOp::Equal => IntCC::Equal,
        mir::CompareOp::NotEqual => IntCC::NotEqual,
        mir::CompareOp::Less if ty.is_signed() => IntCC::SignedLessThan,
        mir::CompareOp::Less => IntCC::UnsignedLessThan,
        mir::CompareOp::LessEqual if ty.is_signed() => IntCC::SignedLessThanOrEqual,
        mir::CompareOp::LessEqual => IntCC::UnsignedLessThanOrEqual,
        mir::CompareOp::Greater if ty.is_signed() => IntCC::SignedGreaterThan,
        mir::CompareOp::Greater => IntCC::UnsignedGreaterThan,
        mir::CompareOp::GreaterEqual if ty.is_signed() => IntCC::SignedGreaterThanOrEqual,
        mir::CompareOp::GreaterEqual => IntCC::UnsignedGreaterThanOrEqual,
    }
}

fn float_compare_code(op: mir::CompareOp) -> FloatCC {
    match op {
        mir::CompareOp::Equal => FloatCC::Equal,
        mir::CompareOp::NotEqual => FloatCC::NotEqual,
        mir::CompareOp::Less => FloatCC::LessThan,
        mir::CompareOp::LessEqual => FloatCC::LessThanOrEqual,
        mir::CompareOp::Greater => FloatCC::GreaterThan,
        mir::CompareOp::GreaterEqual => FloatCC::GreaterThanOrEqual,
    }
}

fn lower_bool_operand(
    builder: &mut FunctionBuilder,
    operand: &mir::Operand,
    resources: &LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    match operand {
        mir::Operand::Scalar(mir::ScalarValue::Bool(value)) => {
            Ok(builder.ins().iconst(types::I8, i64::from(*value)))
        }
        mir::Operand::Local(id) => {
            let definition = local_definition(resources.program, resources.function_id, *id)?;
            if definition.ty != mir::Type::Scalar(mir::ScalarType::Bool) {
                return Err(malformed_mir(format!(
                    "bool expression reads local{} with type {}",
                    id.0, definition.ty
                )));
            }
            Ok(builder
                .ins()
                .stack_load(types::I8, local_slot(resources.local_slots, *id)?, 0))
        }
        mir::Operand::NullablePayload(id) => {
            let definition = local_definition(resources.program, resources.function_id, *id)?;
            if definition.ty != mir::Type::NullableScalar(mir::ScalarType::Bool) {
                return Err(malformed_mir(format!(
                    "bool expression reads nullable payload local{} with type {}",
                    id.0, definition.ty
                )));
            }
            let pointer = resources.module.target_config().pointer_type();
            Ok(builder.ins().stack_load(
                types::I8,
                local_slot(resources.local_slots, *id)?,
                pointer.bytes() as i32,
            ))
        }
        mir::Operand::Static(id) => {
            let address = lower_static_address(builder, *id, resources)?;
            Ok(builder.ins().load(
                types::I8,
                cranelift_codegen::ir::MachMemFlags::trusted(),
                address,
                0,
            ))
        }
        mir::Operand::Property { object, property } => {
            let address = lower_property_address(builder, *object, *property, resources)?;
            Ok(builder.ins().load(
                types::I8,
                cranelift_codegen::ir::MachMemFlags::trusted(),
                address,
                0,
            ))
        }
        _ => Err(malformed_mir("bool expression contains non-bool constant")),
    }
}

#[allow(dead_code)]
fn resolve_string_locals(
    function: &mir::Function,
) -> Result<HashMap<mir::LocalId, Vec<u8>>, BackendError> {
    let mut definitions = HashMap::new();
    for block in &function.blocks {
        for statement in &block.statements {
            let mir::Statement::AssignLocal { target, value } = statement else {
                continue;
            };
            if function.locals[target.0].ty != mir::Type::String {
                continue;
            }
            let mir::Rvalue::String(expression) = value else {
                return Err(malformed_mir(format!(
                    "string local local{} has a non-string initializer",
                    target.0
                )));
            };
            if definitions.insert(*target, expression.clone()).is_some() {
                return Err(malformed_mir(format!(
                    "readonly string local local{} is assigned more than once",
                    target.0
                )));
            }
        }
    }

    let mut values = HashMap::new();
    for local in definitions.keys().copied().collect::<Vec<_>>() {
        resolve_string_local(local, &definitions, &mut values, &mut HashSet::new())?;
    }
    Ok(values)
}

fn resolve_string_local(
    local: mir::LocalId,
    definitions: &HashMap<mir::LocalId, mir::StringExpression>,
    values: &mut HashMap<mir::LocalId, Vec<u8>>,
    visiting: &mut HashSet<mir::LocalId>,
) -> Result<Vec<u8>, BackendError> {
    if let Some(value) = values.get(&local) {
        return Ok(value.clone());
    }
    if !visiting.insert(local) {
        return Err(malformed_mir(format!(
            "cyclic readonly string local local{}",
            local.0
        )));
    }
    let expression = definitions.get(&local).ok_or_else(|| {
        malformed_mir(format!(
            "string local local{} has no compile-time initializer",
            local.0
        ))
    })?;
    let value =
        resolve_string_expression_from_definitions(expression, definitions, values, visiting)?;
    visiting.remove(&local);
    values.insert(local, value.clone());
    Ok(value)
}

fn resolve_string_expression_from_definitions(
    expression: &mir::StringExpression,
    definitions: &HashMap<mir::LocalId, mir::StringExpression>,
    values: &mut HashMap<mir::LocalId, Vec<u8>>,
    visiting: &mut HashSet<mir::LocalId>,
) -> Result<Vec<u8>, BackendError> {
    match expression {
        mir::StringExpression::Literal(value) => Ok(value.as_bytes().to_vec()),
        mir::StringExpression::Local(local) => {
            resolve_string_local(*local, definitions, values, visiting)
        }
        mir::StringExpression::NullableLocalAssumeNonNull(_) => {
            Err(malformed_mir("runtime string expression is not a constant"))
        }
        mir::StringExpression::Static(_) => {
            Err(malformed_mir("runtime string expression is not a constant"))
        }
        mir::StringExpression::Property { .. } => {
            Err(malformed_mir("runtime string expression is not a constant"))
        }
        mir::StringExpression::Concat(parts) => {
            let mut value = Vec::new();
            for part in parts {
                value.extend(resolve_string_expression_from_definitions(
                    part,
                    definitions,
                    values,
                    visiting,
                )?);
            }
            Ok(value)
        }
        mir::StringExpression::Display(_)
        | mir::StringExpression::Call { .. }
        | mir::StringExpression::ReadFile(_)
        | mir::StringExpression::Format(_)
        | mir::StringExpression::Coalesce { .. } => {
            Err(malformed_mir("runtime string expression is not a constant"))
        }
    }
}

#[allow(dead_code)]
fn resolve_string_expression(
    expression: &mir::StringExpression,
    values: &HashMap<mir::LocalId, Vec<u8>>,
) -> Result<Vec<u8>, BackendError> {
    match expression {
        mir::StringExpression::Literal(value) => Ok(value.as_bytes().to_vec()),
        mir::StringExpression::Local(local) => values.get(local).cloned().ok_or_else(|| {
            malformed_mir(format!(
                "string local local{} has no resolved value",
                local.0
            ))
        }),
        mir::StringExpression::NullableLocalAssumeNonNull(_) => {
            Err(malformed_mir("runtime string expression is not a constant"))
        }
        mir::StringExpression::Static(_) => {
            Err(malformed_mir("runtime string expression is not a constant"))
        }
        mir::StringExpression::Property { .. } => {
            Err(malformed_mir("runtime string expression is not a constant"))
        }
        mir::StringExpression::Concat(parts) => {
            let mut value = Vec::new();
            for part in parts {
                value.extend(resolve_string_expression(part, values)?);
            }
            Ok(value)
        }
        mir::StringExpression::Display(_)
        | mir::StringExpression::Call { .. }
        | mir::StringExpression::ReadFile(_)
        | mir::StringExpression::Format(_)
        | mir::StringExpression::Coalesce { .. } => {
            Err(malformed_mir("runtime string expression is not a constant"))
        }
    }
}

fn lower_echo_bytes(
    builder: &mut FunctionBuilder,
    bytes: &[u8],
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    if bytes.is_empty() {
        return Ok(());
    }
    let pointer = define_data(builder, bytes, resources)?;
    let pointer_type = resources.module.target_config().pointer_type();
    let length = builder.ins().iconst(pointer_type, bytes.len() as i64);
    let write_id = resources.declare_write_stdout()?;
    let write = resources
        .module
        .declare_func_in_func(write_id, builder.func);
    builder
        .ins()
        .call(write, &[resources.current_frame, pointer, length]);
    Ok(())
}

fn lower_runtime_panic(
    builder: &mut FunctionBuilder,
    message: &[u8],
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let pointer = define_data(builder, message, resources)?;
    let pointer_type = resources.module.target_config().pointer_type();
    let length = builder.ins().iconst(pointer_type, message.len() as i64);
    let panic_id = resources.declare_panic()?;
    let panic = resources
        .module
        .declare_func_in_func(panic_id, builder.func);
    builder
        .ins()
        .call(panic, &[resources.current_frame, pointer, length]);
    builder
        .ins()
        .trap(TrapCode::unwrap_user(RUNTIME_RETURNED_TRAP));
    Ok(())
}

fn define_named_data(
    builder: &mut FunctionBuilder,
    bytes: &[u8],
    module: &mut ObjectModule,
    name: &str,
) -> Result<Value, BackendError> {
    let data_id = module
        .declare_data(name, Linkage::Local, false, false)
        .map_err(|error| backend_failure(error.to_string()))?;
    let mut description = DataDescription::new();
    description.define(bytes.to_vec().into_boxed_slice());
    module
        .define_data(data_id, &description)
        .map_err(|error| backend_failure(error.to_string()))?;
    let pointer_type = module.target_config().pointer_type();
    let global = module.declare_data_in_func(data_id, builder.func);
    Ok(builder.ins().global_value(pointer_type, global))
}

fn define_data(
    builder: &mut FunctionBuilder,
    bytes: &[u8],
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let name = format!(
        "__doria_data_{}_{}",
        resources.function_id.0, resources.next_data_id
    );
    resources.next_data_id += 1;
    let data_id = resources
        .module
        .declare_data(&name, Linkage::Local, false, false)
        .map_err(|error| backend_failure(error.to_string()))?;
    let mut description = DataDescription::new();
    description.define(bytes.to_vec().into_boxed_slice());
    resources
        .module
        .define_data(data_id, &description)
        .map_err(|error| backend_failure(error.to_string()))?;
    let pointer_type = resources.module.target_config().pointer_type();
    let global = resources.module.declare_data_in_func(data_id, builder.func);
    Ok(builder.ins().global_value(pointer_type, global))
}

fn function_in(
    program: &mir::Program,
    id: mir::FunctionId,
) -> Result<&mir::Function, BackendError> {
    program
        .functions
        .get(id.0)
        .filter(|function| function.id == id)
        .ok_or_else(|| malformed_mir(format!("FunctionId function{} does not exist", id.0)))
}

fn local_in(function: &mir::Function, id: mir::LocalId) -> Result<&mir::Local, BackendError> {
    function
        .locals
        .get(id.0)
        .filter(|local| local.id == id)
        .ok_or_else(|| malformed_mir(format!("LocalId local{} does not exist", id.0)))
}

fn local_definition(
    program: &mir::Program,
    function: mir::FunctionId,
    local: mir::LocalId,
) -> Result<&mir::Local, BackendError> {
    local_in(function_in(program, function)?, local)
}

fn class_definition(
    program: &mir::Program,
    class: crate::class_layout::ClassId,
) -> Result<&mir::Class, BackendError> {
    program
        .classes
        .get(class.0)
        .filter(|definition| definition.id == class)
        .ok_or_else(|| malformed_mir(format!("class#{} does not exist", class.0)))
}

fn property_definition(
    program: &mir::Program,
    property: crate::class_layout::PropertyId,
) -> Result<&mir::Property, BackendError> {
    class_definition(program, property.class)?
        .properties
        .get(property.index)
        .filter(|definition| definition.id == property)
        .ok_or_else(|| malformed_mir(format!("property{} does not exist", property.index)))
}

fn static_definition(
    program: &mir::Program,
    id: mir::StaticId,
) -> Result<&mir::StaticProperty, BackendError> {
    program
        .statics
        .get(id.0)
        .filter(|property| property.id == id)
        .ok_or_else(|| malformed_mir(format!("static{} does not exist", id.0)))
}

fn lower_property_address(
    builder: &mut FunctionBuilder,
    object: mir::LocalId,
    property: crate::class_layout::PropertyId,
    resources: &LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer_type = resources.module.target_config().pointer_type();
    let slot = local_slot(resources.local_slots, object)?;
    let object = builder.ins().stack_load(pointer_type, slot, 0);
    lower_property_address_from_value(builder, object, property, resources)
}

fn lower_property_address_from_value(
    builder: &mut FunctionBuilder,
    object: Value,
    property: crate::class_layout::PropertyId,
    resources: &LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer_type = resources.module.target_config().pointer_type();
    let class = class_definition(resources.program, property.class)?;
    let layout = class
        .layout
        .properties
        .iter()
        .find(|layout| layout.id == property)
        .ok_or_else(|| malformed_mir(format!("property{} has no layout", property.index)))?;
    let offset = builder.ins().iconst(pointer_type, i64::from(layout.offset));
    Ok(builder.ins().iadd(object, offset))
}

fn block_for(blocks: &[Block], id: mir::BlockId) -> Result<Block, BackendError> {
    blocks
        .get(id.0)
        .copied()
        .ok_or_else(|| malformed_mir(format!("BlockId block{} does not exist", id.0)))
}

fn local_slot(slots: &[Option<StackSlot>], id: mir::LocalId) -> Result<StackSlot, BackendError> {
    slots
        .get(id.0)
        .copied()
        .flatten()
        .ok_or_else(|| malformed_mir(format!("LocalId local{} is not a scalar local", id.0)))
}

fn malformed_mir(message: impl Into<String>) -> BackendError {
    BackendError::new(format!(
        "backend emission failure: malformed MIR: {}",
        message.into()
    ))
}

fn backend_failure(message: impl Into<String>) -> BackendError {
    BackendError::new(format!("backend emission failure: {}", message.into()))
}

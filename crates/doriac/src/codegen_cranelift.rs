use std::collections::{HashMap, HashSet};

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::immediates::{Ieee32, Ieee64};
use cranelift_codegen::ir::{
    types, AbiParam, Block, BlockArg, InstBuilder, MemFlagsData, Signature, StackSlot,
    StackSlotData, StackSlotKind, TrapCode, Type as ClifType, Value,
};
use cranelift_codegen::isa::TargetFrontendConfig;
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{default_libcall_names, DataDescription, DataId, FuncId, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};

use crate::backend::BackendError;
use crate::format_string::{FormatConversion, FormatPiece};
use crate::mir;
use crate::mir_validation;
use crate::native_abi::{
    collection_comparator_code, collection_header_size, collection_value_width, function_symbol,
    nullable_collection_access_code, nullable_payload_type, stage26_collection_kind, APPEND_FILE,
    APPEND_FILE_BYTES, BYTES_DATA, BYTES_EQUAL, BYTES_FREE, BYTES_FROM_COLLECTION, BYTES_GET,
    BYTES_LENGTH, BYTES_SET, BYTES_TO_COLLECTION, CHECKED_IO, CHECKED_IO_APPEND_FILE,
    CHECKED_IO_ERROR_INVALID_UTF8, CHECKED_IO_ERROR_IO, CHECKED_IO_META_HAS_INVALID_COUNT_SHIFT,
    CHECKED_IO_META_HAS_SYSTEM_CODE_SHIFT, CHECKED_IO_META_KIND_SHIFT,
    CHECKED_IO_META_OPERATION_SHIFT, CHECKED_IO_META_REASON_SHIFT, CHECKED_IO_META_TARGET_SHIFT,
    CHECKED_IO_READ_FILE_BYTES, CHECKED_IO_READ_FILE_TEXT, CHECKED_IO_READ_LINE,
    CHECKED_IO_READ_STDIN_BYTES, CHECKED_IO_WRITE_FILE, CHECKED_IO_WRITE_STDERR,
    CHECKED_IO_WRITE_STDOUT, CLASS_ALLOCATE, CLASS_FREE, CLOSURE_ENVIRONMENT_ALLOCATE,
    CLOSURE_ENVIRONMENT_FREE, COLLECTION_AGGREGATE_INSERT_SLOT,
    COLLECTION_AGGREGATE_KEYED_SET_SLOT, COLLECTION_AGGREGATE_NEW,
    COLLECTION_AGGREGATE_NULLABLE_ACCESS_INTO, COLLECTION_AGGREGATE_PUSH_FRONT_SLOT,
    COLLECTION_AGGREGATE_PUSH_SLOT, COLLECTION_AGGREGATE_REMOVE_AT_INTO,
    COLLECTION_AGGREGATE_VALUE_AT, COLLECTION_COMPARE_FLOAT32, COLLECTION_COMPARE_FLOAT64,
    COLLECTION_COMPARE_STRING, COLLECTION_COMPARE_WORD, COLLECTION_CONTAINS,
    COLLECTION_DETACH_FOR_CLEANUP, COLLECTION_FILL_STRING, COLLECTION_FILL_WORD,
    COLLECTION_FINISH_DETACHED_CLEANUP, COLLECTION_FREE, COLLECTION_INDEX_OF, COLLECTION_INSERT_AT,
    COLLECTION_INSERT_AT_NULLABLE, COLLECTION_KEYED_GET, COLLECTION_KEYED_GET_NULLABLE,
    COLLECTION_KEYED_HAS, COLLECTION_KEYED_SET, COLLECTION_KEYED_SET_NULLABLE, COLLECTION_KEY_AT,
    COLLECTION_LENGTH, COLLECTION_NEW, COLLECTION_NULLABLE_ACCESS, COLLECTION_PUSH,
    COLLECTION_PUSH_FRONT, COLLECTION_PUSH_FRONT_NULLABLE, COLLECTION_PUSH_NULLABLE,
    COLLECTION_PUSH_UNIQUE, COLLECTION_REMOVE_AT, COLLECTION_REMOVE_VALUE,
    COLLECTION_RESET_AFTER_CLEANUP, COLLECTION_SET_ALGEBRA, COLLECTION_SET_AT,
    COLLECTION_SET_AT_NULLABLE, COLLECTION_STAGE26_FINALIZE, COLLECTION_STAGE26_FROM_COPY,
    COLLECTION_STAGE26_NEW, COLLECTION_VALUE_AT, FLOAT_PARSE, FORMAT_F32, FORMAT_F64, FORMAT_I64,
    FORMAT_STRING, FORMAT_U64, INT_PARSE, MIXED_CLONE_OWNED, MIXED_FREE, MIXED_NEW,
    MIXED_NEW_AGGREGATE, MIXED_NEW_AGGREGATE_BORROWED, MIXED_NEW_BORROWED, MIXED_PAYLOAD,
    MIXED_RELEASE_OWNED, MIXED_TAG, MIXED_TAG_BOOL, MIXED_TAG_CLASS, MIXED_TAG_ENUM,
    MIXED_TAG_ERROR, MIXED_TAG_FLOAT32, MIXED_TAG_FLOAT64, MIXED_TAG_FUNCTION, MIXED_TAG_INT16,
    MIXED_TAG_INT32, MIXED_TAG_INT64, MIXED_TAG_INT8, MIXED_TAG_PAYLOAD_ENUM, MIXED_TAG_STRING,
    MIXED_TAG_UINT16, MIXED_TAG_UINT32, MIXED_TAG_UINT64, MIXED_TAG_UINT8, MIXED_TYPE_ID,
    NULLABLE_STRING_EQUAL, PROCESS_EXIT, READ_FILE, READ_FILE_BYTES, READ_STDIN_BYTES,
    READ_STDIN_LINE_PROMPTED, SHARED_ACQUIRE, SHARED_CREATE, SHARED_CREATE_WEAK, SHARED_PAYLOAD,
    SHARED_RELEASE, SHARED_RELEASE_WEAK, SHARED_RETAIN, STRING_BYTE_LENGTH, STRING_COMPARE,
    STRING_CONCAT, STRING_CONTAINS, STRING_CONTAINS_IGNORE_CASE, STRING_COUNT_OCCURRENCES,
    STRING_DATA, STRING_ENDS_WITH, STRING_ENDS_WITH_IGNORE_CASE, STRING_EQUALS_IGNORE_CASE,
    STRING_FROM_BOOL, STRING_FROM_BYTES, STRING_FROM_F32, STRING_FROM_F64, STRING_FROM_I64,
    STRING_FROM_U64, STRING_FROM_UTF8, STRING_GRAPHEME_LENGTH, STRING_INDEX_OF,
    STRING_INDEX_OF_IGNORE_CASE, STRING_IS_EMPTY, STRING_JOIN, STRING_LAST_INDEX_OF,
    STRING_LAST_INDEX_OF_IGNORE_CASE, STRING_LOWER, STRING_LOWER_FIRST, STRING_PAD_END,
    STRING_PAD_START, STRING_RELEASE, STRING_REPEAT, STRING_REPLACE, STRING_RETAIN, STRING_SLICE,
    STRING_SPLIT, STRING_STARTS_WITH, STRING_STARTS_WITH_IGNORE_CASE, STRING_TO_BYTES, STRING_TRIM,
    STRING_TRIM_END, STRING_TRIM_START, STRING_UPPER, STRING_UPPER_FIRST, STRING_WRITE_STDERR,
    STRING_WRITE_STDOUT, WRITABLE_SHARED_ACQUIRE, WRITABLE_SHARED_ACQUIRE_READONLY_ACCESS,
    WRITABLE_SHARED_ACQUIRE_WRITABLE_ACCESS, WRITABLE_SHARED_CREATE, WRITABLE_SHARED_CREATE_WEAK,
    WRITABLE_SHARED_READONLY_PAYLOAD, WRITABLE_SHARED_RELEASE,
    WRITABLE_SHARED_RELEASE_READONLY_ACCESS, WRITABLE_SHARED_RELEASE_WEAK,
    WRITABLE_SHARED_RELEASE_WRITABLE_ACCESS, WRITABLE_SHARED_RETAIN,
    WRITABLE_SHARED_WRITABLE_PAYLOAD, WRITE_FILE, WRITE_FILE_BYTES, WRITE_STDERR_BYTES,
    WRITE_STDOUT_BYTES,
};
use crate::native_closure_abi;
use crate::numeric::{FloatType, FloatValue, IntegerPanic, IntegerType, IntegerValue};

const RUNTIME_RETURNED_TRAP: u8 = 1;

struct DeclaredNativeItems<'a> {
    function_ids: &'a [FuncId],
    class_drop_function_ids: &'a [FuncId],
    collection_drop_function_ids: &'a [FuncId],
    closure_drop_function_ids: &'a [FuncId],
    closure_descriptor_ids: &'a [DataId],
    static_ids: &'a [DataId],
}

pub fn lower_mir_to_object(program: &mir::Program) -> Result<Vec<u8>, BackendError> {
    mir_validation::validate_program(program)?;
    lower_validated_mir_to_object(program)
}

pub(crate) fn lower_validated_mir_to_object(
    program: &mir::Program,
) -> Result<Vec<u8>, BackendError> {
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
    let class_drop_function_ids = declare_class_drop_functions(&mut module, program)?;
    let collection_drop_function_ids = declare_collection_drop_functions(&mut module, program)?;
    let closure_drop_function_ids = declare_closure_drop_functions(&mut module, program)?;
    let closure_descriptor_ids = define_closure_descriptors(
        &mut module,
        program,
        &function_ids,
        &closure_drop_function_ids,
    )?;
    let static_ids = define_static_data(&mut module, program, &class_drop_function_ids)?;
    let declarations = DeclaredNativeItems {
        function_ids: &function_ids,
        class_drop_function_ids: &class_drop_function_ids,
        collection_drop_function_ids: &collection_drop_function_ids,
        closure_drop_function_ids: &closure_drop_function_ids,
        closure_descriptor_ids: &closure_descriptor_ids,
        static_ids: &static_ids,
    };

    // The process entry point is C `main(int argc, char **argv)`. Both
    // parameters are always declared, even when the Doria entry ignores them,
    // so the platform start-up code always sees the signature it expects.
    let mut process_signature = module.make_signature();
    let process_pointer_type = module.target_config().pointer_type();
    process_signature.params.push(AbiParam::new(types::I32));
    process_signature
        .params
        .push(AbiParam::new(process_pointer_type));
    process_signature.returns.push(AbiParam::new(types::I32));
    let process_main_id = module
        .declare_function("main", Linkage::Export, &process_signature)
        .map_err(|error| backend_failure(error.to_string()))?;

    for function in &program.functions {
        define_function(&mut module, program, function, &declarations)?;
    }
    for class in &program.classes {
        define_class_drop_function(&mut module, program, class.id, &declarations)?;
    }
    for collection in &program.collection_types {
        define_collection_drop_function(&mut module, program, collection.id, &declarations)?;
    }
    for descriptor in &program.closure_descriptors {
        define_closure_drop_function(&mut module, program, descriptor, &declarations)?;
    }
    define_process_main(
        &mut module,
        program,
        process_main_id,
        &process_signature,
        &function_ids,
        &static_ids,
    )?;

    module
        .finish()
        .emit()
        .map_err(|error| backend_failure(error.to_string()))
}

fn declare_class_drop_functions(
    module: &mut ObjectModule,
    program: &mir::Program,
) -> Result<Vec<FuncId>, BackendError> {
    let pointer = module.target_config().pointer_type();
    program
        .classes
        .iter()
        .map(|class| {
            let mut signature = module.make_signature();
            signature.params.push(AbiParam::new(pointer));
            signature.params.push(AbiParam::new(pointer));
            module
                .declare_function(
                    &format!("__doria_drop_class_{}", class.id.0),
                    Linkage::Local,
                    &signature,
                )
                .map_err(|error| backend_failure(error.to_string()))
        })
        .collect()
}

fn declare_collection_drop_functions(
    module: &mut ObjectModule,
    program: &mir::Program,
) -> Result<Vec<FuncId>, BackendError> {
    let pointer = module.target_config().pointer_type();
    program
        .collection_types
        .iter()
        .map(|collection| {
            let mut signature = module.make_signature();
            signature.params.push(AbiParam::new(pointer));
            signature.params.push(AbiParam::new(pointer));
            module
                .declare_function(
                    &format!("__doria_drop_collection_{}", collection.id.0),
                    Linkage::Local,
                    &signature,
                )
                .map_err(|error| backend_failure(error.to_string()))
        })
        .collect()
}

fn declare_closure_drop_functions(
    module: &mut ObjectModule,
    program: &mir::Program,
) -> Result<Vec<FuncId>, BackendError> {
    let pointer = module.target_config().pointer_type();
    program
        .closure_descriptors
        .iter()
        .map(|descriptor| {
            let mut signature = module.make_signature();
            signature.params.push(AbiParam::new(pointer));
            signature.params.push(AbiParam::new(pointer));
            module
                .declare_function(
                    &format!("__doria_drop_closure_environment_{}", descriptor.id.0),
                    Linkage::Local,
                    &signature,
                )
                .map_err(|error| backend_failure(error.to_string()))
        })
        .collect()
}

fn define_closure_descriptors(
    module: &mut ObjectModule,
    program: &mir::Program,
    function_ids: &[FuncId],
    closure_drop_function_ids: &[FuncId],
) -> Result<Vec<DataId>, BackendError> {
    let pointer_bytes = usize::from(module.target_config().pointer_bytes());
    let layout = native_closure_abi::descriptor_layout(pointer_bytes as u32);
    program
        .closure_descriptors
        .iter()
        .map(|descriptor| {
            let entry = *function_ids
                .get(descriptor.entry_function.0)
                .ok_or_else(|| malformed_mir("closure descriptor entry was not declared"))?;
            let drop_environment = *closure_drop_function_ids
                .get(descriptor.id.0)
                .ok_or_else(|| malformed_mir("closure descriptor drop glue was not declared"))?;
            let id = module
                .declare_data(
                    &format!("__doria_closure_descriptor_{}", descriptor.id.0),
                    Linkage::Local,
                    false,
                    false,
                )
                .map_err(|error| backend_failure(error.to_string()))?;
            let mut data = DataDescription::new();
            data.set_align(u64::from(layout.layout.align));
            // Relocated pointer slots must live in initialized data. Mach-O
            // linkers cannot apply function relocations to a BSS section.
            data.define(vec![0; layout.layout.size as usize].into_boxed_slice());
            let entry = module.declare_func_in_data(entry, &mut data);
            data.write_function_addr(layout.entry_offset, entry);
            let drop_environment = module.declare_func_in_data(drop_environment, &mut data);
            data.write_function_addr(layout.drop_environment_offset, drop_environment);
            module
                .define_data(id, &data)
                .map_err(|error| backend_failure(error.to_string()))?;
            Ok(id)
        })
        .collect()
}

fn function_signature(
    module: &mut ObjectModule,
    function: &mir::Function,
) -> Result<Signature, BackendError> {
    let mut signature = module.make_signature();
    let plan = native_closure_abi::NativeCallableSignaturePlan::direct(function);
    for _ in &plan.hidden_inputs {
        signature
            .params
            .push(AbiParam::new(module.target_config().pointer_type()));
    }
    for (index, parameter) in function.params.iter().enumerate() {
        let ty = local_in(function, *parameter)?.ty;
        if function.closure.is_some()
            && !matches!(ty, mir::Type::ClosureEnvironment(_))
            && function.parameter_modes[index] == mir::FunctionParameterMode::Writable
        {
            signature
                .params
                .push(AbiParam::new(module.target_config().pointer_type()));
        } else {
            append_type_abi_params(
                &mut signature.params,
                ty,
                module.target_config().pointer_type(),
            );
        }
    }
    if plan.checked {
        signature.returns.push(AbiParam::new(types::I8));
    } else if let mir::ReturnType::Value(ty) = function.return_type {
        if !matches!(
            ty,
            mir::Type::PayloadEnum(_) | mir::Type::NullablePayloadEnum(_)
        ) {
            append_type_abi_params(
                &mut signature.returns,
                ty,
                module.target_config().pointer_type(),
            );
        }
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
        mir::ScalarType::Enum(_) => types::I32,
    }
}

fn scalar_abi_param(ty: mir::ScalarType) -> AbiParam {
    match ty {
        mir::ScalarType::Integer(ty) => integer_abi_param(ty),
        // The private Doria ABI uses full register slots for narrow values so
        // platform C ABI extension rules cannot change internal calls.
        mir::ScalarType::Float(FloatType::Float32) => AbiParam::new(types::I32),
        mir::ScalarType::Bool => AbiParam::new(types::I32),
        _ => AbiParam::new(clif_scalar_type(ty)),
    }
}

fn append_type_abi_params(params: &mut Vec<AbiParam>, ty: mir::Type, pointer_type: ClifType) {
    match ty {
        mir::Type::Scalar(ty) => params.push(scalar_abi_param(ty)),
        mir::Type::String
        | mir::Type::Mixed
        | mir::Type::Class(_)
        | mir::Type::NullableClass(_)
        | mir::Type::NullableMixed
        | mir::Type::SharedReference(_)
        | mir::Type::WeakReference(_)
        | mir::Type::NullableSharedReference(_)
        | mir::Type::NullableWeakReference(_)
        | mir::Type::WritableSharedReference(_)
        | mir::Type::WritableWeakReference(_)
        | mir::Type::NullableWritableSharedReference(_)
        | mir::Type::NullableWritableWeakReference(_)
        | mir::Type::ReadonlySharedReferenceAccess(_)
        | mir::Type::WritableSharedReferenceAccess(_)
        | mir::Type::NullableReadonlySharedReferenceAccess(_)
        | mir::Type::NullableWritableSharedReferenceAccess(_)
        | mir::Type::Collection(_)
        | mir::Type::NullableCollection(_)
        | mir::Type::PayloadEnum(_)
        | mir::Type::NullablePayloadEnum(_) => {
            params.push(AbiParam::new(pointer_type));
        }
        mir::Type::Error | mir::Type::NullableError => {
            params.push(AbiParam::new(pointer_type));
            params.push(AbiParam::new(pointer_type));
        }
        mir::Type::Function(_) | mir::Type::NullableFunction(_) => {
            params.push(AbiParam::new(pointer_type));
            params.push(AbiParam::new(pointer_type));
        }
        mir::Type::ClosureEnvironment(_) => params.push(AbiParam::new(pointer_type)),
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

fn value_to_doria_abi(
    builder: &mut FunctionBuilder,
    value: LoweredValue,
    ty: mir::Type,
) -> LoweredValue {
    match (value, ty) {
        (
            LoweredValue::Single(value),
            mir::Type::Scalar(mir::ScalarType::Float(FloatType::Float32)),
        ) => LoweredValue::Single(
            builder
                .ins()
                .bitcast(types::I32, MemFlagsData::new(), value),
        ),
        (
            LoweredValue::Nullable { present, payload },
            mir::Type::NullableScalar(mir::ScalarType::Float(FloatType::Float32)),
        ) => LoweredValue::Nullable {
            present,
            payload: builder
                .ins()
                .bitcast(types::I32, MemFlagsData::new(), payload),
        },
        (LoweredValue::Single(value), mir::Type::Scalar(mir::ScalarType::Bool)) => {
            LoweredValue::Single(builder.ins().uextend(types::I32, value))
        }
        (
            LoweredValue::Nullable { present, payload },
            mir::Type::NullableScalar(mir::ScalarType::Bool),
        ) => LoweredValue::Nullable {
            present,
            payload: builder.ins().uextend(types::I32, payload),
        },
        (value, _) => value,
    }
}

fn value_from_doria_abi(builder: &mut FunctionBuilder, value: Value, ty: mir::Type) -> Value {
    match ty {
        mir::Type::Scalar(mir::ScalarType::Float(FloatType::Float32)) => {
            builder
                .ins()
                .bitcast(types::F32, MemFlagsData::new(), value)
        }
        mir::Type::Scalar(mir::ScalarType::Bool) => builder.ins().ireduce(types::I8, value),
        _ => value,
    }
}

fn nullable_payload_from_doria_abi(
    builder: &mut FunctionBuilder,
    payload: Value,
    ty: mir::Type,
) -> Value {
    match ty {
        mir::Type::NullableScalar(mir::ScalarType::Float(FloatType::Float32)) => builder
            .ins()
            .bitcast(types::F32, MemFlagsData::new(), payload),
        mir::Type::NullableScalar(mir::ScalarType::Bool) => {
            builder.ins().ireduce(types::I8, payload)
        }
        _ => payload,
    }
}

fn scalar_storage_bytes(ty: mir::ScalarType) -> u32 {
    match ty {
        mir::ScalarType::Integer(ty) => ty.storage_bytes(),
        mir::ScalarType::Float(ty) => ty.storage_bytes(),
        mir::ScalarType::Bool => 1,
        mir::ScalarType::Enum(_) => 4,
    }
}

fn define_static_data(
    module: &mut ObjectModule,
    program: &mir::Program,
    class_drop_function_ids: &[FuncId],
) -> Result<Vec<DataId>, BackendError> {
    let pointer_bytes = usize::from(module.target_config().pointer_bytes());
    let mut ids = Vec::with_capacity(program.statics.len());
    for property in &program.statics {
        let symbol = format!(
            "__doria_static_{}_{}_{}",
            property.class.0, property.id.0, property.name
        );
        let runtime_initialized = matches!(property.initializer, mir::StaticValue::PayloadEnum(_));
        let id = module
            .declare_data(
                &symbol,
                Linkage::Local,
                property.writable || runtime_initialized,
                false,
            )
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
                    mir::Type::NullableScalar(_)
                        | mir::Type::NullableString
                        | mir::Type::Function(_)
                        | mir::Type::NullableFunction(_)
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
            mir::StaticValue::PayloadEnum(value) => {
                let nullable = matches!(property.ty, mir::Type::NullablePayloadEnum(_));
                description.set_align(value.ty.align.into());
                description.define_zeroinit(value.ty.storage_size(nullable) as usize);
            }
        }
        module
            .define_data(id, &description)
            .map_err(|error| backend_failure(error.to_string()))?;
        ids.push(id);
    }
    for descriptor in &program.error_descriptors {
        let symbol = format!("__doria_error_descriptor_{}", descriptor.id.0);
        let type_name_id = module
            .declare_data(&format!("{symbol}_type_name"), Linkage::Local, false, false)
            .map_err(|error| backend_failure(error.to_string()))?;
        let mut type_name = DataDescription::new();
        type_name.define(descriptor.type_name.as_bytes().to_vec().into_boxed_slice());
        module
            .define_data(type_name_id, &type_name)
            .map_err(|error| backend_failure(error.to_string()))?;

        let class = class_definition(program, descriptor.class)?;
        let message = class
            .layout
            .properties
            .iter()
            .find(|property| property.id == descriptor.message_property)
            .ok_or_else(|| malformed_mir("Error message property has no class layout"))?;
        let drop_id = *class_drop_function_ids
            .get(descriptor.class.0)
            .ok_or_else(|| malformed_mir("Error class drop glue was not declared"))?;
        let id = module
            .declare_data(&symbol, Linkage::Local, false, false)
            .map_err(|error| backend_failure(error.to_string()))?;
        let mut description = DataDescription::new();
        description.set_align(pointer_bytes as u64);
        let mut bytes = Vec::with_capacity(pointer_bytes * 8);
        append_target_word(&mut bytes, 0, pointer_bytes);
        append_target_word(&mut bytes, descriptor.type_name.len() as u64, pointer_bytes);
        append_target_word(&mut bytes, u64::from(message.offset), pointer_bytes);
        append_target_word(&mut bytes, 0, pointer_bytes);
        append_target_word(&mut bytes, u64::from(class.layout.size), pointer_bytes);
        append_target_word(
            &mut bytes,
            u64::from(class.error_origin_offset.ok_or_else(|| {
                malformed_mir("Error descriptor class has no hidden origin slot")
            })?),
            pointer_bytes,
        );
        append_target_word(&mut bytes, 0, pointer_bytes);
        append_target_word(&mut bytes, 0, pointer_bytes);
        description.define(bytes.into_boxed_slice());
        let type_name_reference = module.declare_data_in_data(type_name_id, &mut description);
        description.write_data_addr(0, type_name_reference, 0);
        let drop_reference = module.declare_func_in_data(drop_id, &mut description);
        description.write_function_addr((pointer_bytes * 3) as u32, drop_reference);
        module
            .define_data(id, &description)
            .map_err(|error| backend_failure(error.to_string()))?;
        ids.push(id);
    }
    for origin in &program.error_origins {
        let doria_source = program
            .source_for_span(origin.span)
            .unwrap_or(&program.source);
        let symbol = format!("__doria_error_origin_{}", origin.id.0);
        let id = module
            .declare_data(&symbol, Linkage::Local, false, false)
            .map_err(|error| backend_failure(error.to_string()))?;
        let mut description = DataDescription::new();
        description.set_align(pointer_bytes as u64);
        let path_id = module
            .declare_data(&format!("{symbol}_path"), Linkage::Local, false, false)
            .map_err(|error| backend_failure(error.to_string()))?;
        let mut path = DataDescription::new();
        path.define(doria_source.path.as_bytes().to_vec().into_boxed_slice());
        module
            .define_data(path_id, &path)
            .map_err(|error| backend_failure(error.to_string()))?;
        let source_id = module
            .declare_data(&format!("{symbol}_source"), Linkage::Local, false, false)
            .map_err(|error| backend_failure(error.to_string()))?;
        let mut source = DataDescription::new();
        source.define(doria_source.text.as_bytes().to_vec().into_boxed_slice());
        module
            .define_data(source_id, &source)
            .map_err(|error| backend_failure(error.to_string()))?;
        let callable_id = module
            .declare_data(&format!("{symbol}_callable"), Linkage::Local, false, false)
            .map_err(|error| backend_failure(error.to_string()))?;
        let mut callable = DataDescription::new();
        callable.define(origin.callable.as_bytes().to_vec().into_boxed_slice());
        module
            .define_data(callable_id, &callable)
            .map_err(|error| backend_failure(error.to_string()))?;
        let mut bytes = Vec::with_capacity(pointer_bytes * 8);
        append_target_word(&mut bytes, 0, pointer_bytes);
        append_target_word(&mut bytes, doria_source.path.len() as u64, pointer_bytes);
        append_target_word(&mut bytes, 0, pointer_bytes);
        append_target_word(&mut bytes, doria_source.text.len() as u64, pointer_bytes);
        append_target_word(&mut bytes, origin.span.start as u64, pointer_bytes);
        append_target_word(&mut bytes, origin.span.end as u64, pointer_bytes);
        append_target_word(&mut bytes, 0, pointer_bytes);
        append_target_word(&mut bytes, origin.callable.len() as u64, pointer_bytes);
        description.define(bytes.into_boxed_slice());
        let path_reference = module.declare_data_in_data(path_id, &mut description);
        description.write_data_addr(0, path_reference, 0);
        let source_reference = module.declare_data_in_data(source_id, &mut description);
        description.write_data_addr((pointer_bytes * 2) as u32, source_reference, 0);
        let callable_reference = module.declare_data_in_data(callable_id, &mut description);
        description.write_data_addr((pointer_bytes * 6) as u32, callable_reference, 0);
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
        mir::ScalarValue::Enum(value) => (value.case_id.index as u32).to_ne_bytes().to_vec(),
    }
}

fn define_function(
    module: &mut ObjectModule,
    program: &mir::Program,
    function: &mir::Function,
    declarations: &DeclaredNativeItems<'_>,
) -> Result<(), BackendError> {
    let DeclaredNativeItems {
        function_ids,
        class_drop_function_ids,
        collection_drop_function_ids,
        closure_descriptor_ids,
        static_ids,
        ..
    } = declarations;
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
                mir::Type::String
                | mir::Type::Mixed
                | mir::Type::Class(_)
                | mir::Type::NullableClass(_)
                | mir::Type::NullableMixed
                | mir::Type::SharedReference(_)
                | mir::Type::WeakReference(_)
                | mir::Type::NullableSharedReference(_)
                | mir::Type::NullableWeakReference(_)
                | mir::Type::WritableSharedReference(_)
                | mir::Type::WritableWeakReference(_)
                | mir::Type::NullableWritableSharedReference(_)
                | mir::Type::NullableWritableWeakReference(_)
                | mir::Type::ReadonlySharedReferenceAccess(_)
                | mir::Type::WritableSharedReferenceAccess(_)
                | mir::Type::NullableReadonlySharedReferenceAccess(_)
                | mir::Type::NullableWritableSharedReferenceAccess(_)
                | mir::Type::Collection(_)
                | mir::Type::NullableCollection(_) => {
                    Some(builder.create_sized_stack_slot(StackSlotData::new(
                        StackSlotKind::ExplicitSlot,
                        u32::from(module.target_config().pointer_bytes()),
                        module.target_config().pointer_bytes().trailing_zeros() as u8,
                    )))
                }
                mir::Type::NullableScalar(_)
                | mir::Type::NullableString
                | mir::Type::Error
                | mir::Type::NullableError
                | mir::Type::Function(_)
                | mir::Type::NullableFunction(_) => {
                    let pointer_bytes = u32::from(module.target_config().pointer_bytes());
                    Some(builder.create_sized_stack_slot(StackSlotData::new(
                        StackSlotKind::ExplicitSlot,
                        pointer_bytes * 2,
                        pointer_bytes.trailing_zeros() as u8,
                    )))
                }
                mir::Type::PayloadEnum(ty) | mir::Type::NullablePayloadEnum(ty) => {
                    let nullable = matches!(local.ty, mir::Type::NullablePayloadEnum(_));
                    Some(builder.create_sized_stack_slot(StackSlotData::new(
                        StackSlotKind::ExplicitSlot,
                        ty.storage_size(nullable),
                        ty.align.trailing_zeros() as u8,
                    )))
                }
                mir::Type::ClosureEnvironment(_) => {
                    Some(builder.create_sized_stack_slot(StackSlotData::new(
                        StackSlotKind::ExplicitSlot,
                        u32::from(module.target_config().pointer_bytes()),
                        module.target_config().pointer_bytes().trailing_zeros() as u8,
                    )))
                }
            })
            .collect::<Vec<_>>();
        let closure_environment_slots = program
            .closure_descriptors
            .iter()
            .map(|descriptor| {
                if descriptor.environment_placement != mir::ClosureEnvironmentPlacement::Stack {
                    return Ok(None);
                }
                let logical = descriptor.environment_layout.ok_or_else(|| {
                    malformed_mir("stack closure descriptor has no environment layout")
                })?;
                let layout = native_closure_abi::environment_layout(
                    program,
                    logical,
                    u32::from(module.target_config().pointer_bytes()),
                )?;
                Ok(Some(builder.create_sized_stack_slot(StackSlotData::new(
                    StackSlotKind::ExplicitSlot,
                    layout.layout.size.max(1),
                    layout.layout.align.trailing_zeros() as u8,
                ))))
            })
            .collect::<Result<Vec<_>, BackendError>>()?;
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
            pointer_bytes * 11,
            pointer_bytes.trailing_zeros() as u8,
        ));

        builder.switch_to_block(entry);
        initialize_locals(&mut builder, function, &local_slots, module.target_config())?;
        let zero = builder.ins().iconst(pointer_type, 0);
        for slot in &deferred_class_temporary_slots {
            builder.ins().stack_store(pointer_type, zero, *slot, 0);
        }
        let writable_parameter_addresses =
            bind_parameters(&mut builder, function, &local_slots, entry, pointer_type)?;
        let signature_plan = native_closure_abi::NativeCallableSignaturePlan::direct(function);
        let parent_frame = builder.block_params(entry)[signature_plan
            .index_of(native_closure_abi::NativeCallableHiddenInput::CurrentFrame)
            .expect("native callable plans always include a current frame")];
        let return_address = signature_plan
            .index_of(native_closure_abi::NativeCallableHiddenInput::ResultOut)
            .map(|index| builder.block_params(entry)[index]);
        let checked_error_address = signature_plan
            .index_of(native_closure_abi::NativeCallableHiddenInput::ErrorOut)
            .map(|index| builder.block_params(entry)[index]);
        let borrow_home_addresses = match (
            native_closure_abi::return_borrow_source_parameter(function)?,
            signature_plan.index_of(native_closure_abi::NativeCallableHiddenInput::BorrowHome),
        ) {
            (Some(local), Some(index)) => {
                HashMap::from([(local, builder.block_params(entry)[index])])
            }
            (None, None) => HashMap::new(),
            _ => {
                return Err(malformed_mir(
                    "callable borrow-home ABI plan is inconsistent",
                ))
            }
        };
        let function_name = define_named_data(
            &mut builder,
            function.name.as_bytes(),
            module,
            &format!("__doria_function_name_{}", function.id.0),
        )?;
        builder
            .ins()
            .stack_store(pointer_type, parent_frame, frame_slot, 0);
        builder.ins().stack_store(
            pointer_type,
            function_name,
            frame_slot,
            pointer_bytes as i32,
        );
        let function_name_length = builder
            .ins()
            .iconst(pointer_type, function.name.len() as i64);
        builder.ins().stack_store(
            pointer_type,
            function_name_length,
            frame_slot,
            (pointer_bytes * 2) as i32,
        );
        let doria_source = program
            .source_for_span(function.source_span)
            .unwrap_or(&program.source);
        let source_path = define_named_data(
            &mut builder,
            doria_source.path.as_bytes(),
            module,
            &format!("__doria_source_path_{}", function.id.0),
        )?;
        let source_text = define_named_data(
            &mut builder,
            doria_source.text.as_bytes(),
            module,
            &format!("__doria_source_text_{}", function.id.0),
        )?;
        for (word, value) in [
            (3, source_path),
            (
                4,
                builder
                    .ins()
                    .iconst(pointer_type, doria_source.path.len() as i64),
            ),
            (5, source_text),
            (
                6,
                builder
                    .ins()
                    .iconst(pointer_type, doria_source.text.len() as i64),
            ),
            (
                7,
                builder
                    .ins()
                    .iconst(pointer_type, function.source_span.start as i64),
            ),
            (
                8,
                builder
                    .ins()
                    .iconst(pointer_type, function.source_span.end as i64),
            ),
            (
                9,
                builder
                    .ins()
                    .iconst(pointer_type, function.source_span.start as i64),
            ),
            (
                10,
                builder
                    .ins()
                    .iconst(pointer_type, function.source_span.end as i64),
            ),
        ] {
            builder.ins().stack_store(
                pointer_type,
                value,
                frame_slot,
                (pointer_bytes * word) as i32,
            );
        }
        let current_frame = builder.ins().stack_addr(pointer_type, frame_slot, 0);

        let mut resources = LoweringResources {
            module,
            program,
            function_ids,
            class_drop_function_ids,
            collection_drop_function_ids,
            closure_descriptor_ids,
            closure_environment_slots: &closure_environment_slots,
            closure_bound_fields: HashMap::new(),
            borrow_home_addresses,
            writable_parameter_addresses,
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
            return_address,
            checked_error_address,
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
        builder.finalize(module.target_config());
    }

    module
        .define_function(function_id, &mut context)
        .map_err(|error| backend_failure(format!("{error:?}")))?;
    module.clear_context(&mut context);
    Ok(())
}

fn define_class_drop_function(
    module: &mut ObjectModule,
    program: &mir::Program,
    class: crate::class_layout::ClassId,
    declarations: &DeclaredNativeItems<'_>,
) -> Result<(), BackendError> {
    let DeclaredNativeItems {
        function_ids,
        class_drop_function_ids,
        collection_drop_function_ids,
        closure_descriptor_ids,
        static_ids,
        ..
    } = declarations;
    let function_id = *class_drop_function_ids
        .get(class.0)
        .ok_or_else(|| malformed_mir(format!("class{} drop function was not declared", class.0)))?;
    let pointer = module.target_config().pointer_type();
    let mut context = module.make_context();
    context.func.signature.params.push(AbiParam::new(pointer));
    context.func.signature.params.push(AbiParam::new(pointer));
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let entry = builder.create_block();
        builder.append_block_params_for_function_params(entry);
        builder.switch_to_block(entry);
        let current_frame = builder.block_params(entry)[0];
        let object = builder.block_params(entry)[1];
        let local_slots = [];
        let closure_environment_slots = [];
        let mut resources = LoweringResources {
            module,
            program,
            function_ids,
            class_drop_function_ids,
            collection_drop_function_ids,
            closure_descriptor_ids,
            closure_environment_slots: &closure_environment_slots,
            closure_bound_fields: HashMap::new(),
            borrow_home_addresses: HashMap::new(),
            writable_parameter_addresses: HashMap::new(),
            static_ids,
            local_slots: &local_slots,
            deferred_class_temporary_slots: Vec::new(),
            deferred_class_temporary_slot_cursor: 0,
            write_stdout_func_id: None,
            panic_func_id: None,
            runtime_functions: HashMap::new(),
            next_data_id: 0,
            function_id: program.entry,
            current_frame,
            return_address: None,
            checked_error_address: None,
            defer_class_temporary_drops: false,
            deferred_class_temporary_drops: Vec::new(),
        };
        lower_drop_class_value(&mut builder, object, class, &mut resources)?;
        builder.ins().return_(&[]);
        builder.seal_all_blocks();
        builder.finalize(module.target_config());
    }
    module
        .define_function(function_id, &mut context)
        .map_err(|error| backend_failure(format!("{error:?}")))?;
    module.clear_context(&mut context);
    Ok(())
}

fn define_collection_drop_function(
    module: &mut ObjectModule,
    program: &mir::Program,
    collection: mir::CollectionTypeId,
    declarations: &DeclaredNativeItems<'_>,
) -> Result<(), BackendError> {
    let DeclaredNativeItems {
        function_ids,
        class_drop_function_ids,
        collection_drop_function_ids,
        closure_descriptor_ids,
        static_ids,
        ..
    } = declarations;
    let function_id = *collection_drop_function_ids
        .get(collection.0)
        .ok_or_else(|| malformed_mir("collection drop function was not declared"))?;
    let pointer = module.target_config().pointer_type();
    let mut context = module.make_context();
    context.func.signature.params.push(AbiParam::new(pointer));
    context.func.signature.params.push(AbiParam::new(pointer));
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let entry = builder.create_block();
        builder.append_block_params_for_function_params(entry);
        builder.switch_to_block(entry);
        let current_frame = builder.block_params(entry)[0];
        let value = builder.block_params(entry)[1];
        let local_slots = [];
        let closure_environment_slots = [];
        let mut resources = LoweringResources {
            module,
            program,
            function_ids,
            class_drop_function_ids,
            collection_drop_function_ids,
            closure_descriptor_ids,
            closure_environment_slots: &closure_environment_slots,
            closure_bound_fields: HashMap::new(),
            borrow_home_addresses: HashMap::new(),
            writable_parameter_addresses: HashMap::new(),
            static_ids,
            local_slots: &local_slots,
            deferred_class_temporary_slots: Vec::new(),
            deferred_class_temporary_slot_cursor: 0,
            write_stdout_func_id: None,
            panic_func_id: None,
            runtime_functions: HashMap::new(),
            next_data_id: 0,
            function_id: program.entry,
            current_frame,
            return_address: None,
            checked_error_address: None,
            defer_class_temporary_drops: false,
            deferred_class_temporary_drops: Vec::new(),
        };
        lower_drop_collection_value(&mut builder, value, collection, &mut resources)?;
        builder.ins().return_(&[]);
        builder.seal_all_blocks();
        builder.finalize(module.target_config());
    }
    module
        .define_function(function_id, &mut context)
        .map_err(|error| backend_failure(format!("{error:?}")))?;
    module.clear_context(&mut context);
    Ok(())
}

fn define_closure_drop_function(
    module: &mut ObjectModule,
    program: &mir::Program,
    descriptor: &mir::ClosureDescriptor,
    declarations: &DeclaredNativeItems<'_>,
) -> Result<(), BackendError> {
    let DeclaredNativeItems {
        function_ids,
        class_drop_function_ids,
        collection_drop_function_ids,
        closure_drop_function_ids,
        closure_descriptor_ids,
        static_ids,
    } = declarations;
    let function_id = *closure_drop_function_ids
        .get(descriptor.id.0)
        .ok_or_else(|| malformed_mir("closure environment drop function was not declared"))?;
    let pointer = module.target_config().pointer_type();
    let environment_layout = descriptor
        .environment_layout
        .map(|logical| -> Result<_, BackendError> {
            Ok((
                closure_environment_layout_in(program, logical)?.clone(),
                native_closure_abi::environment_layout(program, logical, pointer.bytes())?,
            ))
        })
        .transpose()?;
    let mut context = module.make_context();
    context.func.signature.params.push(AbiParam::new(pointer));
    context.func.signature.params.push(AbiParam::new(pointer));
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let entry = builder.create_block();
        builder.append_block_params_for_function_params(entry);
        builder.switch_to_block(entry);
        let current_frame = builder.block_params(entry)[0];
        let environment = builder.block_params(entry)[1];
        let local_slots = [];
        let closure_environment_slots = [];
        let mut resources = LoweringResources {
            module,
            program,
            function_ids,
            class_drop_function_ids,
            collection_drop_function_ids,
            closure_descriptor_ids,
            closure_environment_slots: &closure_environment_slots,
            closure_bound_fields: HashMap::new(),
            borrow_home_addresses: HashMap::new(),
            writable_parameter_addresses: HashMap::new(),
            static_ids,
            local_slots: &local_slots,
            deferred_class_temporary_slots: Vec::new(),
            deferred_class_temporary_slot_cursor: 0,
            write_stdout_func_id: None,
            panic_func_id: None,
            runtime_functions: HashMap::new(),
            next_data_id: 0,
            function_id: descriptor.entry_function,
            current_frame,
            return_address: None,
            checked_error_address: None,
            defer_class_temporary_drops: false,
            deferred_class_temporary_drops: Vec::new(),
        };
        if let Some((logical, native)) = &environment_layout {
            for logical_index in &logical.logical_release_order {
                let field = logical
                    .fields
                    .get(*logical_index)
                    .ok_or_else(|| malformed_mir("closure drop field does not exist"))?;
                if field.storage != mir::ClosureEnvironmentStorage::Owned {
                    continue;
                }
                let layout = native
                    .fields
                    .iter()
                    .find(|layout| layout.field == field.id)
                    .ok_or_else(|| malformed_mir("native closure drop field does not exist"))?;
                let Some(bit) = layout.live_bit else {
                    continue;
                };
                let byte_offset = (bit / 8) as i32;
                let mask = 1_i64 << (bit % 8);
                let byte = builder.ins().load(
                    types::I8,
                    cranelift_codegen::ir::MachMemFlags::trusted(),
                    environment,
                    byte_offset,
                );
                let live = builder.ins().band_imm_u(byte, mask);
                let live = builder.ins().icmp_imm_u(IntCC::NotEqual, live, 0);
                let drop_block = builder.create_block();
                let next = builder.create_block();
                builder.ins().brif(live, drop_block, &[], next, &[]);
                builder.switch_to_block(drop_block);
                let address = builder
                    .ins()
                    .iadd_imm_u(environment, i64::from(layout.offset));
                lower_drop_value_at_address(&mut builder, field.ty, address, &mut resources)?;
                set_environment_live_bit(&mut builder, environment, bit, false);
                builder.ins().jump(next, &[]);
                builder.switch_to_block(next);
            }
            if descriptor.environment_placement == mir::ClosureEnvironmentPlacement::Heap {
                runtime_call(
                    &mut builder,
                    CLOSURE_ENVIRONMENT_FREE,
                    &[pointer],
                    None,
                    &[environment],
                    &mut resources,
                )?;
            }
        }
        builder.ins().return_(&[]);
        builder.seal_all_blocks();
        builder.finalize(module.target_config());
    }
    module
        .define_function(function_id, &mut context)
        .map_err(|error| backend_failure(format!("{error:?}")))?;
    module.clear_context(&mut context);
    Ok(())
}

fn initialize_locals(
    builder: &mut FunctionBuilder,
    function: &mir::Function,
    slots: &[Option<StackSlot>],
    frontend_config: TargetFrontendConfig,
) -> Result<(), BackendError> {
    let pointer_type = frontend_config.pointer_type();
    for local in &function.locals {
        if let mir::Type::PayloadEnum(ty) | mir::Type::NullablePayloadEnum(ty) = local.ty {
            let nullable = matches!(local.ty, mir::Type::NullablePayloadEnum(_));
            let slot = local_slot(slots, local.id)?;
            let address = builder.ins().stack_addr(pointer_type, slot, 0);
            let zero = builder.ins().iconst(types::I8, 0);
            let size = builder
                .ins()
                .iconst(pointer_type, i64::from(ty.storage_size(nullable)));
            builder.call_memset(frontend_config, address, zero, size);
            continue;
        }
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
            mir::Type::Scalar(mir::ScalarType::Enum(_)) => builder.ins().iconst(types::I32, 0),
            mir::Type::String
            | mir::Type::Mixed
            | mir::Type::Class(_)
            | mir::Type::NullableClass(_)
            | mir::Type::NullableMixed
            | mir::Type::SharedReference(_)
            | mir::Type::WeakReference(_)
            | mir::Type::NullableSharedReference(_)
            | mir::Type::NullableWeakReference(_)
            | mir::Type::WritableSharedReference(_)
            | mir::Type::WritableWeakReference(_)
            | mir::Type::NullableWritableSharedReference(_)
            | mir::Type::NullableWritableWeakReference(_)
            | mir::Type::ReadonlySharedReferenceAccess(_)
            | mir::Type::WritableSharedReferenceAccess(_)
            | mir::Type::NullableReadonlySharedReferenceAccess(_)
            | mir::Type::NullableWritableSharedReferenceAccess(_)
            | mir::Type::Collection(_)
            | mir::Type::NullableCollection(_)
            | mir::Type::ClosureEnvironment(_) => builder.ins().iconst(pointer_type, 0),
            mir::Type::NullableScalar(_)
            | mir::Type::NullableString
            | mir::Type::Error
            | mir::Type::NullableError
            | mir::Type::Function(_)
            | mir::Type::NullableFunction(_) => {
                let zero = builder.ins().iconst(pointer_type, 0);
                let slot = local_slot(slots, local.id)?;
                builder
                    .ins()
                    .stack_store(pointer_type, zero, slot, pointer_type.bytes() as i32);
                zero
            }
            mir::Type::PayloadEnum(_) | mir::Type::NullablePayloadEnum(_) => unreachable!(),
        };
        builder
            .ins()
            .stack_store(pointer_type, zero, local_slot(slots, local.id)?, 0);
    }
    Ok(())
}

fn bind_parameters(
    builder: &mut FunctionBuilder,
    function: &mir::Function,
    slots: &[Option<StackSlot>],
    entry: Block,
    pointer_type: ClifType,
) -> Result<HashMap<mir::LocalId, Value>, BackendError> {
    let params = builder.block_params(entry).to_vec();
    let plan = native_closure_abi::NativeCallableSignaturePlan::direct(function);
    let mut params = params.into_iter().skip(plan.source_parameter_offset());
    let mut writable_parameter_addresses = HashMap::new();
    for (index, parameter) in function.params.iter().enumerate() {
        let slot = local_slot(slots, *parameter)?;
        let ty = local_in(function, *parameter)?.ty;
        let first = params
            .next()
            .ok_or_else(|| malformed_mir("function parameter is missing an ABI value"))?;
        if function.closure.is_some()
            && !matches!(ty, mir::Type::ClosureEnvironment(_))
            && function.parameter_modes[index] == mir::FunctionParameterMode::Writable
        {
            let value = load_lowered_from_address(builder, ty, first, pointer_type);
            store_lowered_to_stack(builder, ty, slot, value, pointer_type)?;
            writable_parameter_addresses.insert(*parameter, first);
            continue;
        }
        let first = value_from_doria_abi(builder, first, ty);
        if let mir::Type::PayloadEnum(payload) | mir::Type::NullablePayloadEnum(payload) = ty {
            let destination = builder.ins().stack_addr(pointer_type, slot, 0);
            copy_inline_bytes(
                builder,
                destination,
                first,
                payload.storage_size(matches!(ty, mir::Type::NullablePayloadEnum(_))),
                pointer_type,
            );
        } else {
            builder.ins().stack_store(pointer_type, first, slot, 0);
        }
        if matches!(
            ty,
            mir::Type::NullableScalar(_)
                | mir::Type::NullableString
                | mir::Type::Error
                | mir::Type::NullableError
                | mir::Type::Function(_)
                | mir::Type::NullableFunction(_)
        ) {
            let payload = params.next().ok_or_else(|| {
                malformed_mir("nullable function parameter is missing its ABI payload")
            })?;
            let payload = nullable_payload_from_doria_abi(builder, payload, ty);
            let payload_offset = builder.func.dfg.value_type(first).bytes() as i32;
            builder
                .ins()
                .stack_store(pointer_type, payload, slot, payload_offset);
        }
    }
    Ok(writable_parameter_addresses)
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
            let value = builder.ins().stack_load(pointer, pointer, slot, offset);
            let retained = retain_string(builder, value, resources)?;
            builder.ins().stack_store(pointer, retained, slot, offset);
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
        let value = builder.ins().stack_load(
            pointer,
            pointer,
            local_slot(resources.local_slots, local)?,
            offset,
        );
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
        let value = builder
            .ins()
            .stack_load(pointer_type, pointer_type, slot, 0);
        let zero = builder.ins().iconst(pointer_type, 0);
        builder.ins().stack_store(pointer_type, zero, slot, 0);
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
    drops: &[(StackSlot, DeferredOwnedTemporary)],
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    for (slot, temporary) in drops.iter().rev() {
        let value = builder.ins().stack_load(pointer, pointer, *slot, 0);
        let zero = builder.ins().iconst(pointer, 0);
        builder.ins().stack_store(pointer, zero, *slot, 0);
        match temporary {
            DeferredOwnedTemporary::Class(class) => {
                lower_drop_class_value_checked(builder, value, *class, resources)?;
            }
            DeferredOwnedTemporary::Collection(collection) => {
                lower_drop_collection_value(builder, value, *collection, resources)?;
            }
            DeferredOwnedTemporary::Mixed(ownership) => {
                lower_cleanup_mixed_temporary(builder, value, *ownership, resources)?;
            }
            DeferredOwnedTemporary::Shared(weak) => {
                lower_drop_shared_value(builder, value, *weak, resources)?;
            }
            DeferredOwnedTemporary::WritableShared(symbol) => {
                lower_drop_writable_shared_value(builder, value, symbol, resources)?;
            }
        }
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
    let pointer = resources.module.target_config().pointer_type();
    builder.ins().stack_store(pointer, value, slot, 0);
    resources
        .deferred_class_temporary_drops
        .push((slot, DeferredOwnedTemporary::Class(class)));
    Ok(())
}

fn defer_or_drop_collection_temporary(
    builder: &mut FunctionBuilder,
    value: Value,
    collection: mir::CollectionTypeId,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    if !resources.defer_class_temporary_drops {
        return lower_drop_collection_value(builder, value, collection, resources);
    }
    let slot = *resources
        .deferred_class_temporary_slots
        .get(resources.deferred_class_temporary_slot_cursor)
        .ok_or_else(|| malformed_mir("owned temporary stack-slot capacity was exhausted"))?;
    resources.deferred_class_temporary_slot_cursor += 1;
    let pointer = resources.module.target_config().pointer_type();
    builder.ins().stack_store(pointer, value, slot, 0);
    resources
        .deferred_class_temporary_drops
        .push((slot, DeferredOwnedTemporary::Collection(collection)));
    Ok(())
}

fn defer_or_cleanup_mixed_temporary(
    builder: &mut FunctionBuilder,
    value: Value,
    ownership: mir::MixedOwnership,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    if !resources.defer_class_temporary_drops {
        return lower_cleanup_mixed_temporary(builder, value, ownership, resources);
    }
    let slot = *resources
        .deferred_class_temporary_slots
        .get(resources.deferred_class_temporary_slot_cursor)
        .ok_or_else(|| malformed_mir("owned temporary stack-slot capacity was exhausted"))?;
    resources.deferred_class_temporary_slot_cursor += 1;
    let pointer = resources.module.target_config().pointer_type();
    builder.ins().stack_store(pointer, value, slot, 0);
    resources
        .deferred_class_temporary_drops
        .push((slot, DeferredOwnedTemporary::Mixed(ownership)));
    Ok(())
}

fn defer_or_drop_shared_temporary(
    builder: &mut FunctionBuilder,
    value: Value,
    weak: bool,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    if !resources.defer_class_temporary_drops {
        return lower_drop_shared_value(builder, value, weak, resources);
    }
    let slot = *resources
        .deferred_class_temporary_slots
        .get(resources.deferred_class_temporary_slot_cursor)
        .ok_or_else(|| malformed_mir("owned temporary stack-slot capacity was exhausted"))?;
    resources.deferred_class_temporary_slot_cursor += 1;
    let pointer = resources.module.target_config().pointer_type();
    builder.ins().stack_store(pointer, value, slot, 0);
    resources
        .deferred_class_temporary_drops
        .push((slot, DeferredOwnedTemporary::Shared(weak)));
    Ok(())
}

fn defer_or_drop_writable_shared_temporary(
    builder: &mut FunctionBuilder,
    value: Value,
    symbol: &'static str,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    if !resources.defer_class_temporary_drops {
        return lower_drop_writable_shared_value(builder, value, symbol, resources);
    }
    let slot = *resources
        .deferred_class_temporary_slots
        .get(resources.deferred_class_temporary_slot_cursor)
        .ok_or_else(|| malformed_mir("owned temporary stack-slot capacity was exhausted"))?;
    resources.deferred_class_temporary_slot_cursor += 1;
    let pointer = resources.module.target_config().pointer_type();
    builder.ins().stack_store(pointer, value, slot, 0);
    resources
        .deferred_class_temporary_drops
        .push((slot, DeferredOwnedTemporary::WritableShared(symbol)));
    Ok(())
}

fn define_process_main(
    module: &mut ObjectModule,
    program: &mir::Program,
    process_main_id: FuncId,
    process_signature: &Signature,
    function_ids: &[FuncId],
    static_ids: &[DataId],
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
        builder.append_block_params_for_function_params(block);
        builder.switch_to_block(block);
        builder.seal_block(block);
        let argc = builder.block_params(block)[0];
        let argv = builder.block_params(block)[1];

        let pointer_type = module.target_config().pointer_type();
        initialize_payload_statics(&mut builder, module, program, static_ids)?;
        let entry_ref = module.declare_func_in_func(entry_id, builder.func);
        let entry_pointer = builder.ins().func_addr(pointer_type, entry_ref);
        // Decision 0099: an entry that declares the argument list is invoked
        // through the `_args` runtime glue, which builds an owned
        // `List<string>`, lends it to `main`, and releases it afterwards.
        let takes_arguments = !entry.params.is_empty();
        if !takes_arguments {
            let mut validation_signature = module.make_signature();
            validation_signature.params.push(AbiParam::new(types::I32));
            validation_signature
                .params
                .push(AbiParam::new(pointer_type));
            let validation_id = module
                .declare_function(
                    "dr_v1_validate_entry_args",
                    Linkage::Import,
                    &validation_signature,
                )
                .map_err(|error| backend_failure(error.to_string()))?;
            let validation = module.declare_func_in_func(validation_id, builder.func);
            builder.ins().call(validation, &[argc, argv]);
        }
        let mut runtime_signature = module.make_signature();
        runtime_signature.params.push(AbiParam::new(pointer_type));
        if takes_arguments {
            runtime_signature.params.push(AbiParam::new(types::I32));
            runtime_signature.params.push(AbiParam::new(pointer_type));
        }
        let integer_entry = matches!(
            entry.return_type,
            mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Integer(
                IntegerType::Int64
            )))
        );
        if integer_entry {
            for _ in 0..6 {
                runtime_signature.params.push(AbiParam::new(pointer_type));
            }
        }
        runtime_signature.returns.push(AbiParam::new(types::I32));
        let checked_entry = !entry.checked_effects.is_empty();
        let runtime_symbol = match (entry.return_type, takes_arguments, checked_entry) {
            (
                mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Integer(
                    IntegerType::Int64,
                ))),
                false,
                false,
            ) => "dr_v2_main_int",
            (
                mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Integer(
                    IntegerType::Int64,
                ))),
                true,
                false,
            ) => "dr_v2_main_int_args",
            (
                mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Integer(
                    IntegerType::Int64,
                ))),
                false,
                true,
            ) => "dr_v3_main_checked_int",
            (
                mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Integer(
                    IntegerType::Int64,
                ))),
                true,
                true,
            ) => "dr_v3_main_checked_int_args",
            (mir::ReturnType::Void, false, false) => "dr_v2_main_void",
            (mir::ReturnType::Void, true, false) => "dr_v2_main_void_args",
            (mir::ReturnType::Void, false, true) => "dr_v3_main_checked_void",
            (mir::ReturnType::Void, true, true) => "dr_v3_main_checked_void_args",
            (mir::ReturnType::Value(other), _, _) => {
                return Err(malformed_mir(format!(
                    "entry function has unsupported process return type {other}"
                )));
            }
        };
        let runtime_id = module
            .declare_function(runtime_symbol, Linkage::Import, &runtime_signature)
            .map_err(|error| backend_failure(error.to_string()))?;
        let runtime = module.declare_func_in_func(runtime_id, builder.func);
        let mut runtime_args = vec![entry_pointer];
        if takes_arguments {
            runtime_args.extend([argc, argv]);
        }
        if integer_entry {
            let doria_source = program
                .source_for_span(entry.source_span)
                .unwrap_or(&program.source);
            let source_path = define_named_data(
                &mut builder,
                doria_source.path.as_bytes(),
                module,
                "__doria_process_source_path",
            )?;
            let source_text = define_named_data(
                &mut builder,
                doria_source.text.as_bytes(),
                module,
                "__doria_process_source_text",
            )?;
            runtime_args.extend([
                source_path,
                builder
                    .ins()
                    .iconst(pointer_type, doria_source.path.len() as i64),
                source_text,
                builder
                    .ins()
                    .iconst(pointer_type, doria_source.text.len() as i64),
                builder
                    .ins()
                    .iconst(pointer_type, entry.source_span.start as i64),
                builder
                    .ins()
                    .iconst(pointer_type, entry.source_span.end as i64),
            ]);
        }
        let call = builder.ins().call(runtime, &runtime_args);
        let status = builder.inst_results(call)[0];
        if cfg!(windows) {
            let mut exit_signature = module.make_signature();
            exit_signature.params.push(AbiParam::new(types::I32));
            let exit_id = module
                .declare_function(PROCESS_EXIT, Linkage::Import, &exit_signature)
                .map_err(|error| backend_failure(error.to_string()))?;
            let exit = module.declare_func_in_func(exit_id, builder.func);
            builder.ins().call(exit, &[status]);
            builder
                .ins()
                .trap(TrapCode::unwrap_user(RUNTIME_RETURNED_TRAP));
        } else {
            builder.ins().return_(&[status]);
        }
        builder.finalize(module.target_config());
    }

    module
        .define_function(process_main_id, &mut context)
        .map_err(|error| backend_failure(error.to_string()))?;
    module.clear_context(&mut context);
    Ok(())
}

fn initialize_payload_statics(
    builder: &mut FunctionBuilder,
    module: &mut ObjectModule,
    program: &mir::Program,
    static_ids: &[DataId],
) -> Result<(), BackendError> {
    let pointer = module.target_config().pointer_type();
    for property in &program.statics {
        let mir::StaticValue::PayloadEnum(value) = &property.initializer else {
            continue;
        };
        let id = *static_ids
            .get(property.id.0)
            .ok_or_else(|| malformed_mir("payload enum static was not declared"))?;
        let global = module.declare_data_in_func(id, builder.func);
        let address = builder.ins().symbol_value(pointer, global);
        initialize_payload_static_value(
            builder,
            module,
            program,
            &property.initializer,
            property.ty,
            address,
            &format!("__doria_static_init_{}", property.id.0),
        )?;
        debug_assert_eq!(
            value.ty.id,
            match property.ty {
                mir::Type::PayloadEnum(ty) | mir::Type::NullablePayloadEnum(ty) => ty.id,
                _ => unreachable!(),
            }
        );
    }
    Ok(())
}

fn initialize_payload_static_value(
    builder: &mut FunctionBuilder,
    module: &mut ObjectModule,
    program: &mir::Program,
    value: &mir::StaticValue,
    ty: mir::Type,
    mut address: Value,
    symbol: &str,
) -> Result<(), BackendError> {
    let pointer = module.target_config().pointer_type();
    let flags = MemFlagsData::new();
    match (value, ty) {
        (mir::StaticValue::Scalar(value), mir::Type::Scalar(expected))
            if value.ty() == expected =>
        {
            let value = cranelift_scalar_constant(builder, *value);
            builder.ins().store(flags, value, address, 0);
        }
        (mir::StaticValue::Scalar(value), mir::Type::NullableScalar(expected))
            if value.ty() == expected =>
        {
            let present = builder.ins().iconst(pointer, 1);
            builder.ins().store(flags, present, address, 0);
            let value = cranelift_scalar_constant(builder, *value);
            builder
                .ins()
                .store(flags, value, address, pointer.bytes() as i32);
        }
        (mir::StaticValue::String(value), mir::Type::String | mir::Type::NullableString) => {
            let mut bytes = Vec::with_capacity(pointer.bytes() as usize * 2 + value.len());
            append_target_word(&mut bytes, u64::MAX, pointer.bytes() as usize);
            append_target_word(&mut bytes, value.len() as u64, pointer.bytes() as usize);
            bytes.extend_from_slice(value.as_bytes());
            let string = define_named_data(builder, &bytes, module, &format!("{symbol}_string"))?;
            if matches!(ty, mir::Type::NullableString) {
                let present = builder.ins().iconst(pointer, 1);
                builder.ins().store(flags, present, address, 0);
                builder
                    .ins()
                    .store(flags, string, address, pointer.bytes() as i32);
            } else {
                builder.ins().store(flags, string, address, 0);
            }
        }
        (mir::StaticValue::Null, _) => {}
        (
            mir::StaticValue::PayloadEnum(value),
            mir::Type::PayloadEnum(expected) | mir::Type::NullablePayloadEnum(expected),
        ) if value.ty == expected => {
            if matches!(ty, mir::Type::NullablePayloadEnum(_)) {
                let present = builder.ins().iconst(types::I8, 1);
                builder.ins().store(flags, present, address, 0);
                address = builder
                    .ins()
                    .iadd_imm_u(address, i64::from(expected.nullable_payload_offset));
            }
            let definition = enum_definition(program, expected.id)?;
            let case = definition
                .cases
                .get(value.case.index)
                .filter(|case| case.id == value.case)
                .ok_or_else(|| malformed_mir("payload enum static case does not exist"))?;
            let layout = definition
                .layout
                .cases
                .get(value.case.index)
                .filter(|layout| layout.case_id == value.case)
                .ok_or_else(|| malformed_mir("payload enum static case layout does not exist"))?;
            let tag = builder
                .ins()
                .iconst(clif_tag_type(definition.layout.tag_width)?, case.tag as i64);
            builder
                .ins()
                .store(flags, tag, address, definition.layout.tag_offset as i32);
            for (index, ((field, field_definition), field_layout)) in value
                .fields
                .iter()
                .zip(&case.payload)
                .zip(&layout.fields)
                .enumerate()
            {
                let field_address = builder
                    .ins()
                    .iadd_imm_u(address, i64::from(field_layout.offset));
                initialize_payload_static_value(
                    builder,
                    module,
                    program,
                    field,
                    field_definition.ty,
                    field_address,
                    &format!("{symbol}_field_{index}"),
                )?;
            }
        }
        _ => {
            return Err(malformed_mir(
                "payload enum static initializer type mismatch",
            ))
        }
    }
    Ok(())
}

fn cranelift_scalar_constant(builder: &mut FunctionBuilder, value: mir::ScalarValue) -> Value {
    match value {
        mir::ScalarValue::Integer(value) => integer_constant(builder, value),
        mir::ScalarValue::Float(value) => match value.ty {
            FloatType::Float32 => builder.ins().f32const(Ieee32::with_bits(value.bits as u32)),
            FloatType::Float64 => builder.ins().f64const(Ieee64::with_bits(value.bits)),
        },
        mir::ScalarValue::Bool(value) => builder.ins().iconst(types::I8, i64::from(value)),
        mir::ScalarValue::Enum(value) => {
            builder.ins().iconst(types::I32, value.case_id.index as i64)
        }
    }
}

fn clif_tag_type(width: u32) -> Result<ClifType, BackendError> {
    match width {
        1 => Ok(types::I8),
        2 => Ok(types::I16),
        4 => Ok(types::I32),
        _ => Err(malformed_mir("payload enum tag has unsupported width")),
    }
}

struct LoweringResources<'module, 'program> {
    module: &'module mut ObjectModule,
    program: &'program mir::Program,
    function_ids: &'program [FuncId],
    class_drop_function_ids: &'program [FuncId],
    collection_drop_function_ids: &'program [FuncId],
    closure_descriptor_ids: &'program [DataId],
    closure_environment_slots: &'program [Option<StackSlot>],
    closure_bound_fields: HashMap<mir::LocalId, BoundClosureField>,
    borrow_home_addresses: HashMap<mir::LocalId, Value>,
    writable_parameter_addresses: HashMap<mir::LocalId, Value>,
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
    return_address: Option<Value>,
    checked_error_address: Option<Value>,
    defer_class_temporary_drops: bool,
    deferred_class_temporary_drops: Vec<(StackSlot, DeferredOwnedTemporary)>,
}

#[derive(Clone, Copy)]
struct BoundClosureField {
    address: Value,
    storage: mir::ClosureEnvironmentStorage,
    ty: mir::Type,
}

#[derive(Clone, Copy)]
enum DeferredOwnedTemporary {
    Class(crate::class_layout::ClassId),
    Collection(mir::CollectionTypeId),
    Mixed(mir::MixedOwnership),
    Shared(bool),
    WritableShared(&'static str),
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
            .declare_function("dr_v2_write_stdout", Linkage::Import, &signature)
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
            .declare_function("dr_v2_panic", Linkage::Import, &signature)
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
            .extend(params.iter().copied().map(runtime_abi_param));
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

fn runtime_abi_param(ty: ClifType) -> AbiParam {
    let parameter = AbiParam::new(ty);
    if ty == types::I8 || ty == types::I16 {
        parameter.uext()
    } else {
        parameter
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

fn lower_bind_closure_environment(
    builder: &mut FunctionBuilder,
    environment_local: mir::LocalId,
    bindings: &[(mir::ClosureEnvironmentFieldId, mir::LocalId)],
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let descriptor = resources
        .program
        .closure_descriptors
        .iter()
        .find(|descriptor| descriptor.entry_function == resources.function_id)
        .ok_or_else(|| malformed_mir("closure environment binding is outside a closure"))?
        .clone();
    let logical_id = descriptor
        .environment_layout
        .ok_or_else(|| malformed_mir("closure environment binding has no environment layout"))?;
    let logical = closure_environment_layout_in(resources.program, logical_id)?.clone();
    let pointer = resources.module.target_config().pointer_type();
    let native =
        native_closure_abi::environment_layout(resources.program, logical_id, pointer.bytes())?;
    let environment = load_lowered_from_stack(
        builder,
        mir::Type::ClosureEnvironment(Some(logical_id)),
        local_slot(resources.local_slots, environment_local)?,
        pointer,
    )
    .single()?;
    for (field_id, target) in bindings {
        let field = logical
            .fields
            .iter()
            .find(|field| field.id == *field_id)
            .ok_or_else(|| malformed_mir("closure binding field does not exist"))?;
        let layout = native
            .fields
            .iter()
            .find(|layout| layout.field == *field_id)
            .ok_or_else(|| malformed_mir("native closure binding field does not exist"))?;
        let field_address = builder
            .ins()
            .iadd_imm_u(environment, i64::from(layout.offset));
        let place = match field.storage {
            mir::ClosureEnvironmentStorage::ReadonlyBorrow
            | mir::ClosureEnvironmentStorage::WritableBorrow => builder.ins().load(
                pointer,
                cranelift_codegen::ir::MachMemFlags::trusted(),
                field_address,
                0,
            ),
            mir::ClosureEnvironmentStorage::Owned => field_address,
        };
        let value = load_lowered_from_address(builder, field.ty, place, pointer);
        let target_slot = local_slot(resources.local_slots, *target)?;
        store_lowered_to_stack(builder, field.ty, target_slot, value, pointer)?;
        if matches!(field.ty, mir::Type::String | mir::Type::NullableString) {
            let offset = if field.ty == mir::Type::NullableString {
                pointer.bytes() as i32
            } else {
                0
            };
            let string = builder
                .ins()
                .stack_load(pointer, pointer, target_slot, offset);
            let string = retain_string(builder, string, resources)?;
            builder
                .ins()
                .stack_store(pointer, string, target_slot, offset);
        }
        if field.storage == mir::ClosureEnvironmentStorage::WritableBorrow
            && field.ty.transfers_writable_capture_ownership()
        {
            let size = match field.ty {
                mir::Type::PayloadEnum(payload) => payload.storage_size(false),
                mir::Type::NullablePayloadEnum(payload) => payload.storage_size(true),
                mir::Type::Error
                | mir::Type::NullableError
                | mir::Type::Function(_)
                | mir::Type::NullableFunction(_) => pointer.bytes() * 2,
                _ => pointer.bytes(),
            };
            zero_inline_bytes(builder, place, size, pointer);
        }
        let address = if field.storage == mir::ClosureEnvironmentStorage::Owned {
            builder.ins().stack_addr(pointer, target_slot, 0)
        } else {
            place
        };
        resources.closure_bound_fields.insert(
            *target,
            BoundClosureField {
                address,
                storage: field.storage,
                ty: field.ty,
            },
        );
        if field.storage == mir::ClosureEnvironmentStorage::Owned && field.ty.has_move_ownership() {
            zero_inline_bytes(builder, field_address, layout.layout.size, pointer);
            if let Some(bit) = layout.live_bit {
                set_environment_live_bit(builder, environment, bit, false);
            }
        }
    }
    Ok(())
}

fn sync_writable_closure_captures(
    builder: &mut FunctionBuilder,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let bindings = resources
        .closure_bound_fields
        .iter()
        .filter_map(|(local, field)| {
            (field.storage == mir::ClosureEnvironmentStorage::WritableBorrow)
                .then_some((*local, *field))
        })
        .collect::<Vec<_>>();
    let pointer = resources.module.target_config().pointer_type();
    for (local, field) in bindings {
        let slot = local_slot(resources.local_slots, local)?;
        let new = load_lowered_from_stack(builder, field.ty, slot, pointer);
        match field.ty {
            mir::Type::Scalar(_) | mir::Type::NullableScalar(_) => {
                store_lowered_to_address(builder, field.ty, field.address, new, pointer)?;
            }
            mir::Type::String | mir::Type::NullableString => {
                let old = load_lowered_from_address(builder, field.ty, field.address, pointer);
                let old_string = if field.ty == mir::Type::NullableString {
                    old.nullable()?.1
                } else {
                    old.single()?
                };
                let new_string = if field.ty == mir::Type::NullableString {
                    new.nullable()?.1
                } else {
                    new.single()?
                };
                let same = builder.ins().icmp(IntCC::Equal, old_string, new_string);
                let done = builder.create_block();
                let replace = builder.create_block();
                builder.ins().brif(same, done, &[], replace, &[]);
                builder.switch_to_block(replace);
                let retained = retain_string(builder, new_string, resources)?;
                let replacement = if field.ty == mir::Type::NullableString {
                    let (present, _) = new.nullable()?;
                    LoweredValue::Nullable {
                        present,
                        payload: retained,
                    }
                } else {
                    LoweredValue::Single(retained)
                };
                store_lowered_to_address(builder, field.ty, field.address, replacement, pointer)?;
                release_string(builder, old_string, resources)?;
                builder.ins().jump(done, &[]);
                builder.switch_to_block(done);
            }
            mir::Type::Function(_) | mir::Type::NullableFunction(_) => {
                sync_writable_two_word_capture(builder, field, slot, new, pointer)?;
            }
            mir::Type::Error | mir::Type::NullableError => {
                sync_writable_two_word_capture(builder, field, slot, new, pointer)?;
            }
            mir::Type::PayloadEnum(payload) | mir::Type::NullablePayloadEnum(payload) => {
                let nullable = matches!(field.ty, mir::Type::NullablePayloadEnum(_));
                let replacement = new.single()?;
                copy_inline_bytes(
                    builder,
                    field.address,
                    replacement,
                    payload.storage_size(nullable),
                    pointer,
                );
                zero_inline_bytes(
                    builder,
                    replacement,
                    payload.storage_size(nullable),
                    pointer,
                );
            }
            mir::Type::Class(_)
            | mir::Type::NullableClass(_)
            | mir::Type::Collection(_)
            | mir::Type::NullableCollection(_)
            | mir::Type::Mixed
            | mir::Type::NullableMixed
            | mir::Type::SharedReference(_)
            | mir::Type::WeakReference(_)
            | mir::Type::NullableSharedReference(_)
            | mir::Type::NullableWeakReference(_)
            | mir::Type::WritableSharedReference(_)
            | mir::Type::WritableWeakReference(_)
            | mir::Type::NullableWritableSharedReference(_)
            | mir::Type::NullableWritableWeakReference(_)
            | mir::Type::ReadonlySharedReferenceAccess(_)
            | mir::Type::WritableSharedReferenceAccess(_)
            | mir::Type::NullableReadonlySharedReferenceAccess(_)
            | mir::Type::NullableWritableSharedReferenceAccess(_) => {
                sync_writable_pointer_capture(builder, field, slot, new.single()?, pointer)?;
            }
            mir::Type::ClosureEnvironment(_) => {
                return Err(malformed_mir(
                    "closure environment pointer cannot be a captured source value",
                ));
            }
        }
    }
    Ok(())
}

fn sync_writable_two_word_capture(
    builder: &mut FunctionBuilder,
    field: BoundClosureField,
    slot: StackSlot,
    new: LoweredValue,
    pointer: ClifType,
) -> Result<(), BackendError> {
    store_lowered_to_address(builder, field.ty, field.address, new, pointer)?;
    clear_function_carrier_stack(builder, slot, pointer);
    Ok(())
}

fn sync_writable_pointer_capture(
    builder: &mut FunctionBuilder,
    field: BoundClosureField,
    slot: StackSlot,
    new: Value,
    pointer: ClifType,
) -> Result<(), BackendError> {
    store_lowered_to_address(
        builder,
        field.ty,
        field.address,
        LoweredValue::Single(new),
        pointer,
    )?;
    let zero = builder.ins().iconst(pointer, 0);
    builder.ins().stack_store(pointer, zero, slot, 0);
    Ok(())
}

fn lower_statement(
    builder: &mut FunctionBuilder,
    statement: &mir::Statement,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    debug_assert!(resources.deferred_class_temporary_drops.is_empty());
    resources.defer_class_temporary_drops = true;
    match statement {
        mir::Statement::BindClosureEnvironment {
            environment,
            bindings,
        } => lower_bind_closure_environment(builder, *environment, bindings, resources)?,
        mir::Statement::DropFunction { local, .. } => {
            let pointer = resources.module.target_config().pointer_type();
            let slot = local_slot(resources.local_slots, *local)?;
            let carrier = load_lowered_from_stack(
                builder,
                local_definition(resources.program, resources.function_id, *local)?.ty,
                slot,
                pointer,
            );
            lower_drop_function_carrier(builder, carrier, resources)?;
            clear_function_carrier_stack(builder, slot, pointer);
        }
        mir::Statement::BindPayloadEnumFields {
            source,
            ty,
            case,
            nullable,
            mode,
            targets,
        } => {
            let pointer = resources.module.target_config().pointer_type();
            let source_slot = local_slot(resources.local_slots, *source)?;
            let mut source_address = builder.ins().stack_addr(pointer, source_slot, 0);
            if *nullable {
                source_address = builder
                    .ins()
                    .iadd_imm_u(source_address, i64::from(ty.nullable_payload_offset));
            }
            let definition = enum_definition(resources.program, ty.id)?;
            let case_definition = definition
                .cases
                .get(case.index)
                .filter(|definition| definition.id == *case)
                .ok_or_else(|| malformed_mir("payload binding case does not exist"))?;
            let case_layout = definition
                .layout
                .cases
                .get(case.index)
                .filter(|layout| layout.case_id == *case)
                .ok_or_else(|| malformed_mir("payload binding case layout does not exist"))?;
            for ((field, layout), target) in case_definition
                .payload
                .iter()
                .zip(&case_layout.fields)
                .zip(targets)
            {
                let field_address = builder
                    .ins()
                    .iadd_imm_u(source_address, i64::from(layout.offset));
                let value = load_lowered_from_address(builder, field.ty, field_address, pointer);
                let target_slot = local_slot(resources.local_slots, *target)?;
                store_lowered_to_stack(builder, field.ty, target_slot, value, pointer)?;
                if matches!(mode, mir::MatchBindingMode::ConsumedArm)
                    && field.ty.has_move_ownership()
                {
                    let size = match field.ty {
                        mir::Type::PayloadEnum(payload) => payload.storage_size(false),
                        mir::Type::NullablePayloadEnum(payload) => payload.storage_size(true),
                        _ => pointer.bytes(),
                    };
                    zero_inline_bytes(builder, field_address, size, pointer);
                    continue;
                }
                if matches!(mode, mir::MatchBindingMode::GuardView) {
                    continue;
                }
                match field.ty {
                    mir::Type::String => {
                        let value = builder.ins().stack_load(pointer, pointer, target_slot, 0);
                        let retained = retain_string(builder, value, resources)?;
                        builder.ins().stack_store(pointer, retained, target_slot, 0);
                    }
                    mir::Type::NullableString => {
                        let value = builder.ins().stack_load(
                            pointer,
                            pointer,
                            target_slot,
                            pointer.bytes() as i32,
                        );
                        let retained = retain_string(builder, value, resources)?;
                        builder.ins().stack_store(
                            pointer,
                            retained,
                            target_slot,
                            pointer.bytes() as i32,
                        );
                    }
                    mir::Type::PayloadEnum(payload) if payload.capabilities.copy => {
                        let target = builder.ins().stack_addr(pointer, target_slot, 0);
                        retain_payload_enum_at(builder, target, payload, false, resources)?;
                    }
                    mir::Type::NullablePayloadEnum(payload) if payload.capabilities.copy => {
                        let target = builder.ins().stack_addr(pointer, target_slot, 0);
                        retain_payload_enum_at(builder, target, payload, true, resources)?;
                    }
                    _ => {}
                }
            }
        }
        mir::Statement::MatchResultPlan { .. } => {}
        mir::Statement::ControlFlowPlan(_) => {}
        mir::Statement::AssignLocalGroup { targets, value } => {
            let definition = local_definition(
                resources.program,
                resources.function_id,
                *targets
                    .first()
                    .ok_or_else(|| malformed_mir("grouped local assignment has no targets"))?,
            )?;
            let value = lower_rvalue(builder, value, resources)?;
            let pointer = resources.module.target_config().pointer_type();
            for (index, target) in targets.iter().enumerate() {
                let value = if index == 0 {
                    value
                } else {
                    match (definition.ty, value) {
                        (mir::Type::String, LoweredValue::Single(value)) => {
                            LoweredValue::Single(retain_string(builder, value, resources)?)
                        }
                        (
                            mir::Type::NullableString,
                            LoweredValue::Nullable { present, payload },
                        ) => LoweredValue::Nullable {
                            present,
                            payload: retain_string(builder, payload, resources)?,
                        },
                        _ => value,
                    }
                };
                let slot = local_slot(resources.local_slots, *target)?;
                store_lowered_to_stack(builder, definition.ty, slot, value, pointer)?;
            }
        }
        mir::Statement::AssignLocal { target, value } => {
            let definition = local_definition(resources.program, resources.function_id, *target)?;
            let new_value = lower_rvalue(builder, value, resources)?;
            let slot = local_slot(resources.local_slots, *target)?;
            let pointer = resources.module.target_config().pointer_type();
            let owns_replaced_value =
                definition.owned || resources.writable_parameter_addresses.contains_key(target);
            let old_error = (owns_replaced_value
                && matches!(definition.ty, mir::Type::Error | mir::Type::NullableError))
            .then(|| load_lowered_from_stack(builder, definition.ty, slot, pointer));
            let old_function = (owns_replaced_value
                && matches!(
                    definition.ty,
                    mir::Type::Function(_) | mir::Type::NullableFunction(_)
                ))
            .then(|| load_lowered_from_stack(builder, definition.ty, slot, pointer));
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
                mir::Type::Class(class) | mir::Type::NullableClass(class)
                    if owns_replaced_value =>
                {
                    Some((
                        load_lowered_from_stack(builder, definition.ty, slot, pointer).single()?,
                        Some(class),
                    ))
                }
                mir::Type::SharedReference(_) | mir::Type::NullableSharedReference(_)
                    if owns_replaced_value =>
                {
                    Some((
                        load_lowered_from_stack(builder, definition.ty, slot, pointer).single()?,
                        None,
                    ))
                }
                mir::Type::WeakReference(_)
                | mir::Type::NullableWeakReference(_)
                | mir::Type::WritableSharedReference(_)
                | mir::Type::WritableWeakReference(_)
                | mir::Type::NullableWritableSharedReference(_)
                | mir::Type::NullableWritableWeakReference(_)
                | mir::Type::ReadonlySharedReferenceAccess(_)
                | mir::Type::WritableSharedReferenceAccess(_)
                | mir::Type::NullableReadonlySharedReferenceAccess(_)
                | mir::Type::NullableWritableSharedReferenceAccess(_)
                    if owns_replaced_value =>
                {
                    Some((
                        load_lowered_from_stack(builder, definition.ty, slot, pointer).single()?,
                        None,
                    ))
                }
                mir::Type::Mixed | mir::Type::NullableMixed if owns_replaced_value => Some((
                    load_lowered_from_stack(builder, definition.ty, slot, pointer).single()?,
                    None,
                )),
                mir::Type::Collection(_) | mir::Type::NullableCollection(_)
                    if owns_replaced_value =>
                {
                    Some((
                        load_lowered_from_stack(builder, definition.ty, slot, pointer).single()?,
                        None,
                    ))
                }
                mir::Type::PayloadEnum(payload) | mir::Type::NullablePayloadEnum(payload)
                    if owns_replaced_value =>
                {
                    let nullable = matches!(definition.ty, mir::Type::NullablePayloadEnum(_));
                    let old = create_payload_storage(builder, payload, nullable, resources);
                    let address = builder.ins().stack_addr(pointer, slot, 0);
                    copy_inline_bytes(
                        builder,
                        old,
                        address,
                        payload.storage_size(nullable),
                        pointer,
                    );
                    Some((old, None))
                }
                _ => None,
            };
            store_lowered_to_stack(builder, definition.ty, slot, new_value, pointer)?;
            if let Some(address) = resources.writable_parameter_addresses.get(target).copied() {
                let current = load_lowered_from_stack(builder, definition.ty, slot, pointer);
                store_lowered_to_address(builder, definition.ty, address, current, pointer)?;
            }
            if let Some((old, class)) = old_value {
                if let mir::Type::Collection(collection)
                | mir::Type::NullableCollection(collection) = definition.ty
                {
                    lower_drop_collection_value(builder, old, collection, resources)?;
                } else if matches!(definition.ty, mir::Type::Mixed | mir::Type::NullableMixed) {
                    lower_drop_mixed_value(builder, old, resources)?;
                } else if matches!(
                    definition.ty,
                    mir::Type::SharedReference(_) | mir::Type::NullableSharedReference(_)
                ) {
                    lower_drop_shared_value(builder, old, false, resources)?;
                } else if matches!(
                    definition.ty,
                    mir::Type::WeakReference(_) | mir::Type::NullableWeakReference(_)
                ) {
                    lower_drop_shared_value(builder, old, true, resources)?;
                } else if let Some(symbol) = writable_shared_release_symbol(definition.ty) {
                    lower_drop_writable_shared_value(builder, old, symbol, resources)?;
                } else if let Some(class) = class {
                    lower_drop_class_value_checked(builder, old, class, resources)?;
                } else if let mir::Type::PayloadEnum(payload) = definition.ty {
                    lower_drop_payload_enum_at(builder, old, payload, false, resources)?;
                } else if let mir::Type::NullablePayloadEnum(payload) = definition.ty {
                    lower_drop_payload_enum_at(builder, old, payload, true, resources)?;
                } else {
                    release_string(builder, old, resources)?;
                }
            }
            if let Some(old) = old_error {
                lower_drop_error_value(builder, old, resources)?;
            }
            if let Some(old) = old_function {
                lower_drop_function_carrier(builder, old, resources)?;
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
        mir::Statement::CallVoid {
            function,
            args,
            span,
        }
        | mir::Statement::CallBorrowed {
            function,
            args,
            span,
        } => {
            let _ = lower_function_call_at(builder, *function, args, *span, resources)?;
        }
        mir::Statement::CallNullSafe {
            object,
            function,
            args,
            span,
        } => {
            set_active_panic_site(builder, *span, resources);
            lower_null_safe_statement_call(builder, object, *function, args, resources)?
        }
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
        mir::Statement::WriteFile { path, contents }
        | mir::Statement::AppendFile { path, contents } => {
            let path = lower_string_expression(builder, path, resources)?;
            let contents = lower_string_expression(builder, contents, resources)?;
            let pointer = resources.module.target_config().pointer_type();
            let name = if matches!(statement, mir::Statement::AppendFile { .. }) {
                APPEND_FILE
            } else {
                WRITE_FILE
            };
            let _ = runtime_call(
                builder,
                name,
                &[pointer, pointer, pointer],
                None,
                &[resources.current_frame, path, contents],
                resources,
            )?;
            release_string(builder, path, resources)?;
            release_string(builder, contents, resources)?;
        }
        mir::Statement::WriteFileBytes {
            path,
            contents,
            append,
        } => {
            let path = lower_string_expression(builder, path, resources)?;
            let contents = lower_collection_pointer(builder, *contents, resources)?;
            let pointer = resources.module.target_config().pointer_type();
            let _ = runtime_call(
                builder,
                if *append {
                    APPEND_FILE_BYTES
                } else {
                    WRITE_FILE_BYTES
                },
                &[pointer, pointer, pointer],
                None,
                &[resources.current_frame, path, contents],
                resources,
            )?;
            release_string(builder, path, resources)?;
        }
        mir::Statement::WriteStreamBytes { contents, stderr } => {
            let contents = lower_collection_pointer(builder, *contents, resources)?;
            let pointer = resources.module.target_config().pointer_type();
            let _ = runtime_call(
                builder,
                if *stderr {
                    WRITE_STDERR_BYTES
                } else {
                    WRITE_STDOUT_BYTES
                },
                &[pointer, pointer],
                None,
                &[resources.current_frame, contents],
                resources,
            )?;
        }
        mir::Statement::AssignProperty {
            object,
            property,
            value,
            kind,
            ..
        } => {
            let property_definition = property_definition(resources.program, *property)?;
            let value = lower_rvalue(builder, value, resources)?;
            let address = lower_property_address(builder, *object, *property, resources)?;
            let pointer_type = resources.module.target_config().pointer_type();
            let replaces = !matches!(kind, mir::PropertyWriteKind::Initialize);
            let old_error = (replaces
                && matches!(
                    property_definition.ty,
                    mir::Type::Error | mir::Type::NullableError
                ))
            .then(|| {
                load_lowered_from_address(builder, property_definition.ty, address, pointer_type)
            });
            let old_function = (replaces
                && matches!(
                    property_definition.ty,
                    mir::Type::Function(_) | mir::Type::NullableFunction(_)
                ))
            .then(|| {
                load_lowered_from_address(builder, property_definition.ty, address, pointer_type)
            });
            let old_value = if replaces {
                match property_definition.ty {
                    mir::Type::String
                    | mir::Type::Mixed
                    | mir::Type::Class(_)
                    | mir::Type::NullableClass(_)
                    | mir::Type::NullableMixed
                    | mir::Type::SharedReference(_)
                    | mir::Type::WeakReference(_)
                    | mir::Type::NullableSharedReference(_)
                    | mir::Type::NullableWeakReference(_)
                    | mir::Type::WritableSharedReference(_)
                    | mir::Type::WritableWeakReference(_)
                    | mir::Type::NullableWritableSharedReference(_)
                    | mir::Type::NullableWritableWeakReference(_)
                    | mir::Type::ReadonlySharedReferenceAccess(_)
                    | mir::Type::WritableSharedReferenceAccess(_)
                    | mir::Type::NullableReadonlySharedReferenceAccess(_)
                    | mir::Type::NullableWritableSharedReferenceAccess(_)
                    | mir::Type::Collection(_)
                    | mir::Type::NullableCollection(_) => Some(
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
                    mir::Type::PayloadEnum(payload) | mir::Type::NullablePayloadEnum(payload) => {
                        let nullable =
                            matches!(property_definition.ty, mir::Type::NullablePayloadEnum(_));
                        let old = create_payload_storage(builder, payload, nullable, resources);
                        copy_inline_bytes(
                            builder,
                            old,
                            address,
                            payload.storage_size(nullable),
                            pointer_type,
                        );
                        Some(old)
                    }
                    mir::Type::Scalar(_)
                    | mir::Type::NullableScalar(_)
                    | mir::Type::Error
                    | mir::Type::NullableError => None,
                    mir::Type::Function(_) | mir::Type::NullableFunction(_) => None,
                    mir::Type::ClosureEnvironment(_) => {
                        return Err(malformed_mir(
                            "closure environment pointer reached a Doria property",
                        ));
                    }
                }
            } else {
                None
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
                (
                    mir::Type::Collection(collection) | mir::Type::NullableCollection(collection),
                    Some(old_value),
                ) => {
                    lower_drop_collection_value(builder, old_value, collection, resources)?;
                }
                (
                    mir::Type::SharedReference(_) | mir::Type::NullableSharedReference(_),
                    Some(old_value),
                ) => lower_drop_shared_value(builder, old_value, false, resources)?,
                (
                    mir::Type::WeakReference(_) | mir::Type::NullableWeakReference(_),
                    Some(old_value),
                ) => lower_drop_shared_value(builder, old_value, true, resources)?,
                (ty, Some(old_value)) if writable_shared_release_symbol(ty).is_some() => {
                    lower_drop_writable_shared_value(
                        builder,
                        old_value,
                        writable_shared_release_symbol(ty).expect("matched above"),
                        resources,
                    )?
                }
                (mir::Type::Mixed | mir::Type::NullableMixed, Some(old_value)) => {
                    lower_drop_mixed_value(builder, old_value, resources)?;
                }
                (mir::Type::PayloadEnum(payload), Some(old_value)) => {
                    lower_drop_payload_enum_at(builder, old_value, payload, false, resources)?;
                }
                (mir::Type::NullablePayloadEnum(payload), Some(old_value)) => {
                    lower_drop_payload_enum_at(builder, old_value, payload, true, resources)?;
                }
                _ => {}
            }
            if let Some(old_error) = old_error {
                lower_drop_error_value(builder, old_error, resources)?;
            }
            if let Some(old_function) = old_function {
                lower_drop_function_carrier(builder, old_function, resources)?;
            }
        }
        mir::Statement::AssignStatic { target, value } => {
            let property = static_definition(resources.program, *target)?;
            let new_value = lower_rvalue(builder, value, resources)?;
            let address = lower_static_address(builder, *target, resources)?;
            let pointer = resources.module.target_config().pointer_type();
            let old_error = matches!(property.ty, mir::Type::Error | mir::Type::NullableError)
                .then(|| load_lowered_from_address(builder, property.ty, address, pointer));
            let old_value = match property.ty {
                mir::Type::String | mir::Type::Mixed | mir::Type::NullableMixed => Some(
                    load_lowered_from_address(builder, property.ty, address, pointer).single()?,
                ),
                mir::Type::NullableString => Some(
                    load_lowered_from_address(builder, property.ty, address, pointer)
                        .nullable()?
                        .1,
                ),
                mir::Type::PayloadEnum(payload) | mir::Type::NullablePayloadEnum(payload) => {
                    let nullable = matches!(property.ty, mir::Type::NullablePayloadEnum(_));
                    let old = create_payload_storage(builder, payload, nullable, resources);
                    copy_inline_bytes(
                        builder,
                        old,
                        address,
                        payload.storage_size(nullable),
                        pointer,
                    );
                    Some(old)
                }
                _ => None,
            };
            store_lowered_to_address(builder, property.ty, address, new_value, pointer)?;
            if let Some(old_value) = old_value {
                if matches!(property.ty, mir::Type::Mixed | mir::Type::NullableMixed) {
                    lower_drop_mixed_value(builder, old_value, resources)?;
                } else if let mir::Type::PayloadEnum(payload) = property.ty {
                    lower_drop_payload_enum_at(builder, old_value, payload, false, resources)?;
                } else if let mir::Type::NullablePayloadEnum(payload) = property.ty {
                    lower_drop_payload_enum_at(builder, old_value, payload, true, resources)?;
                } else {
                    release_string(builder, old_value, resources)?;
                }
            }
            if let Some(old_error) = old_error {
                lower_drop_error_value(builder, old_error, resources)?;
            }
        }
        mir::Statement::DropClass { local, .. } => {
            let pointer_type = resources.module.target_config().pointer_type();
            let slot = local_slot(resources.local_slots, *local)?;
            let value = builder
                .ins()
                .stack_load(pointer_type, pointer_type, slot, 0);
            let zero = builder.ins().iconst(pointer_type, 0);
            builder.ins().stack_store(pointer_type, zero, slot, 0);
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
        mir::Statement::DropString { local } => {
            let pointer = resources.module.target_config().pointer_type();
            let slot = local_slot(resources.local_slots, *local)?;
            let ty = local_definition(resources.program, resources.function_id, *local)?.ty;
            let offset = if matches!(ty, mir::Type::NullableString) {
                pointer.bytes() as i32
            } else {
                0
            };
            let value = builder.ins().stack_load(pointer, pointer, slot, offset);
            let zero = builder.ins().iconst(pointer, 0);
            builder.ins().stack_store(pointer, zero, slot, offset);
            release_string(builder, value, resources)?;
        }
        mir::Statement::DropMixed { local } => {
            let pointer = resources.module.target_config().pointer_type();
            let slot = local_slot(resources.local_slots, *local)?;
            let value = builder.ins().stack_load(pointer, pointer, slot, 0);
            let zero = builder.ins().iconst(pointer, 0);
            builder.ins().stack_store(pointer, zero, slot, 0);
            lower_drop_mixed_value(builder, value, resources)?;
        }
        mir::Statement::CollectionAdd {
            collection,
            value,
            index,
            op,
        } => {
            lower_collection_add(builder, *collection, value, index.as_ref(), *op, resources)?;
        }
        mir::Statement::CollectionSet {
            collection,
            key,
            value,
        } => {
            lower_collection_set(builder, *collection, key, value, false, resources)?;
        }
        mir::Statement::AssignCollectionIndex {
            positional,
            collection,
            index: key,
            value,
        } => {
            lower_collection_set(builder, *collection, key, value, *positional, resources)?;
        }
        mir::Statement::CollectionClear {
            collection,
            collection_type,
        } => {
            let pointer = resources.module.target_config().pointer_type();
            let slot = local_slot(resources.local_slots, *collection)?;
            let value = builder.ins().stack_load(pointer, pointer, slot, 0);
            lower_clear_collection_value(builder, value, *collection_type, resources)?;
        }
        mir::Statement::DropCollection { local, collection } => {
            let pointer = resources.module.target_config().pointer_type();
            let slot = local_slot(resources.local_slots, *local)?;
            let value = builder.ins().stack_load(pointer, pointer, slot, 0);
            let zero = builder.ins().iconst(pointer, 0);
            builder.ins().stack_store(pointer, zero, slot, 0);
            lower_drop_collection_value(builder, value, *collection, resources)?;
        }
        mir::Statement::DropSharedReference { local, .. } => {
            let pointer = resources.module.target_config().pointer_type();
            let slot = local_slot(resources.local_slots, *local)?;
            let value = builder.ins().stack_load(pointer, pointer, slot, 0);
            let zero = builder.ins().iconst(pointer, 0);
            builder.ins().stack_store(pointer, zero, slot, 0);
            lower_drop_shared_value(builder, value, false, resources)?;
        }
        mir::Statement::DropWeakReference { local, .. } => {
            let pointer = resources.module.target_config().pointer_type();
            let slot = local_slot(resources.local_slots, *local)?;
            let value = builder.ins().stack_load(pointer, pointer, slot, 0);
            let zero = builder.ins().iconst(pointer, 0);
            builder.ins().stack_store(pointer, zero, slot, 0);
            lower_drop_shared_value(builder, value, true, resources)?;
        }
        mir::Statement::DropWritableSharedReference { local, .. } => {
            lower_drop_writable_shared_local(builder, *local, WRITABLE_SHARED_RELEASE, resources)?;
        }
        mir::Statement::DropWritableWeakReference { local, .. } => {
            lower_drop_writable_shared_local(
                builder,
                *local,
                WRITABLE_SHARED_RELEASE_WEAK,
                resources,
            )?;
        }
        mir::Statement::DropSharedReferenceAccess {
            local, writable, ..
        } => {
            lower_drop_writable_shared_local(
                builder,
                *local,
                if *writable {
                    WRITABLE_SHARED_RELEASE_WRITABLE_ACCESS
                } else {
                    WRITABLE_SHARED_RELEASE_READONLY_ACCESS
                },
                resources,
            )?;
        }
        mir::Statement::DropPayloadEnum {
            local,
            ty,
            nullable,
        } => {
            let pointer = resources.module.target_config().pointer_type();
            let address =
                builder
                    .ins()
                    .stack_addr(pointer, local_slot(resources.local_slots, *local)?, 0);
            lower_drop_payload_enum_at(builder, address, *ty, *nullable, resources)?;
            zero_inline_bytes(builder, address, ty.storage_size(*nullable), pointer);
        }
        mir::Statement::EnsureErrorOrigin { error, origin } => {
            let pointer = resources.module.target_config().pointer_type();
            let slot = local_slot(resources.local_slots, *error)?;
            let (object, descriptor) =
                load_lowered_from_stack(builder, mir::Type::Error, slot, pointer).nullable()?;
            let flags = cranelift_codegen::ir::MachMemFlags::trusted();
            let origin_offset =
                builder
                    .ins()
                    .load(pointer, flags, descriptor, (pointer.bytes() * 5) as i32);
            let origin_slot = builder.ins().iadd(object, origin_offset);
            let current = builder.ins().load(pointer, flags, origin_slot, 0);
            let zero = builder.ins().iconst(pointer, 0);
            let empty = builder.ins().icmp(IntCC::Equal, current, zero);
            let write = builder.create_block();
            let done = builder.create_block();
            builder.ins().brif(empty, write, &[], done, &[]);
            builder.switch_to_block(write);
            let origin = lower_error_origin_address(builder, *origin, resources)?;
            builder.ins().store(flags, origin, origin_slot, 0);
            builder.ins().jump(done, &[]);
            builder.switch_to_block(done);
        }
        mir::Statement::ExtractErrorObject { target, error, .. } => {
            let pointer = resources.module.target_config().pointer_type();
            let error_slot = local_slot(resources.local_slots, *error)?;
            let (object, _) =
                load_lowered_from_stack(builder, mir::Type::Error, error_slot, pointer)
                    .nullable()?;
            let target_slot = local_slot(resources.local_slots, *target)?;
            builder.ins().stack_store(pointer, object, target_slot, 0);
            let zero = builder.ins().iconst(pointer, 0);
            builder.ins().stack_store(pointer, zero, error_slot, 0);
            builder
                .ins()
                .stack_store(pointer, zero, error_slot, pointer.bytes() as i32);
        }
        mir::Statement::DropError { local } => {
            let pointer = resources.module.target_config().pointer_type();
            let slot = local_slot(resources.local_slots, *local)?;
            let value = load_lowered_from_stack(builder, mir::Type::Error, slot, pointer);
            let zero = builder.ins().iconst(pointer, 0);
            builder.ins().stack_store(pointer, zero, slot, 0);
            builder
                .ins()
                .stack_store(pointer, zero, slot, pointer.bytes() as i32);
            lower_drop_error_value(builder, value, resources)?;
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
        .symbol_value(resources.module.target_config().pointer_type(), global))
}

fn lower_error_descriptor_address(
    builder: &mut FunctionBuilder,
    id: mir::ErrorDescriptorId,
    resources: &LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let data_id = *resources
        .static_ids
        .get(resources.program.statics.len() + id.0)
        .ok_or_else(|| malformed_mir(format!("Error descriptor{} was not declared", id.0)))?;
    let global = resources.module.declare_data_in_func(data_id, builder.func);
    Ok(builder
        .ins()
        .symbol_value(resources.module.target_config().pointer_type(), global))
}

fn lower_error_origin_address(
    builder: &mut FunctionBuilder,
    id: mir::ErrorOriginId,
    resources: &LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let index = resources.program.statics.len() + resources.program.error_descriptors.len() + id.0;
    let data_id = *resources
        .static_ids
        .get(index)
        .ok_or_else(|| malformed_mir(format!("Error origin{} was not declared", id.0)))?;
    let global = resources.module.declare_data_in_func(data_id, builder.func);
    Ok(builder
        .ins()
        .symbol_value(resources.module.target_config().pointer_type(), global))
}

fn load_lowered_from_stack(
    builder: &mut FunctionBuilder,
    ty: mir::Type,
    slot: StackSlot,
    pointer: ClifType,
) -> LoweredValue {
    match ty {
        mir::Type::NullableScalar(scalar) => LoweredValue::Nullable {
            present: builder.ins().stack_load(pointer, pointer, slot, 0),
            payload: builder.ins().stack_load(
                pointer,
                clif_scalar_type(scalar),
                slot,
                pointer.bytes() as i32,
            ),
        },
        mir::Type::NullableString
        | mir::Type::Error
        | mir::Type::NullableError
        | mir::Type::Function(_)
        | mir::Type::NullableFunction(_) => LoweredValue::Nullable {
            present: builder.ins().stack_load(pointer, pointer, slot, 0),
            payload: builder
                .ins()
                .stack_load(pointer, pointer, slot, pointer.bytes() as i32),
        },
        mir::Type::Scalar(scalar) => LoweredValue::Single(builder.ins().stack_load(
            pointer,
            clif_scalar_type(scalar),
            slot,
            0,
        )),
        mir::Type::PayloadEnum(_) | mir::Type::NullablePayloadEnum(_) => {
            LoweredValue::Single(builder.ins().stack_addr(pointer, slot, 0))
        }
        mir::Type::String
        | mir::Type::Mixed
        | mir::Type::Class(_)
        | mir::Type::NullableClass(_)
        | mir::Type::NullableMixed
        | mir::Type::SharedReference(_)
        | mir::Type::WeakReference(_)
        | mir::Type::NullableSharedReference(_)
        | mir::Type::NullableWeakReference(_)
        | mir::Type::WritableSharedReference(_)
        | mir::Type::WritableWeakReference(_)
        | mir::Type::NullableWritableSharedReference(_)
        | mir::Type::NullableWritableWeakReference(_)
        | mir::Type::ReadonlySharedReferenceAccess(_)
        | mir::Type::WritableSharedReferenceAccess(_)
        | mir::Type::NullableReadonlySharedReferenceAccess(_)
        | mir::Type::NullableWritableSharedReferenceAccess(_)
        | mir::Type::Collection(_)
        | mir::Type::NullableCollection(_)
        | mir::Type::ClosureEnvironment(_) => {
            LoweredValue::Single(builder.ins().stack_load(pointer, pointer, slot, 0))
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
        mir::Type::PayloadEnum(payload) | mir::Type::NullablePayloadEnum(payload) => {
            let nullable = matches!(ty, mir::Type::NullablePayloadEnum(_));
            let destination = builder.ins().stack_addr(pointer, slot, 0);
            copy_inline_bytes(
                builder,
                destination,
                value.single()?,
                payload.storage_size(nullable),
                pointer,
            );
        }
        mir::Type::NullableScalar(_)
        | mir::Type::NullableString
        | mir::Type::Error
        | mir::Type::NullableError
        | mir::Type::Function(_)
        | mir::Type::NullableFunction(_) => {
            let (present, payload) = value.nullable()?;
            builder.ins().stack_store(pointer, present, slot, 0);
            builder
                .ins()
                .stack_store(pointer, payload, slot, pointer.bytes() as i32);
        }
        _ => {
            builder.ins().stack_store(pointer, value.single()?, slot, 0);
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
        mir::Type::NullableString
        | mir::Type::Error
        | mir::Type::NullableError
        | mir::Type::Function(_)
        | mir::Type::NullableFunction(_) => LoweredValue::Nullable {
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
        mir::Type::PayloadEnum(_) | mir::Type::NullablePayloadEnum(_) => {
            LoweredValue::Single(address)
        }
        mir::Type::String
        | mir::Type::Mixed
        | mir::Type::Class(_)
        | mir::Type::NullableClass(_)
        | mir::Type::NullableMixed
        | mir::Type::SharedReference(_)
        | mir::Type::WeakReference(_)
        | mir::Type::NullableSharedReference(_)
        | mir::Type::NullableWeakReference(_)
        | mir::Type::WritableSharedReference(_)
        | mir::Type::WritableWeakReference(_)
        | mir::Type::NullableWritableSharedReference(_)
        | mir::Type::NullableWritableWeakReference(_)
        | mir::Type::ReadonlySharedReferenceAccess(_)
        | mir::Type::WritableSharedReferenceAccess(_)
        | mir::Type::NullableReadonlySharedReferenceAccess(_)
        | mir::Type::NullableWritableSharedReferenceAccess(_)
        | mir::Type::Collection(_)
        | mir::Type::NullableCollection(_)
        | mir::Type::ClosureEnvironment(_) => {
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
        mir::Type::PayloadEnum(payload) | mir::Type::NullablePayloadEnum(payload) => {
            let nullable = matches!(ty, mir::Type::NullablePayloadEnum(_));
            copy_inline_bytes(
                builder,
                address,
                value.single()?,
                payload.storage_size(nullable),
                pointer,
            );
        }
        mir::Type::NullableScalar(_)
        | mir::Type::NullableString
        | mir::Type::Error
        | mir::Type::NullableError
        | mir::Type::Function(_)
        | mir::Type::NullableFunction(_) => {
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

fn copy_inline_bytes(
    builder: &mut FunctionBuilder,
    destination: Value,
    source: Value,
    size: u32,
    pointer: ClifType,
) {
    let flags = cranelift_codegen::ir::MachMemFlags::trusted();
    let word_bytes = pointer.bytes();
    let mut offset = 0_u32;
    while size - offset >= word_bytes {
        let value = builder.ins().load(pointer, flags, source, offset as i32);
        builder
            .ins()
            .store(flags, value, destination, offset as i32);
        offset += word_bytes;
    }
    while offset < size {
        let value = builder.ins().load(types::I8, flags, source, offset as i32);
        builder
            .ins()
            .store(flags, value, destination, offset as i32);
        offset += 1;
    }
}

fn lower_terminator(
    builder: &mut FunctionBuilder,
    terminator: &mir::Terminator,
    blocks: &[Block],
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    match terminator {
        mir::Terminator::IndirectCall {
            callee,
            function_type,
            invocation_mode,
            args,
            result,
            continuation,
            span,
        } => lower_indirect_call(
            builder,
            callee,
            *function_type,
            *invocation_mode,
            args,
            *result,
            block_for(blocks, *continuation)?,
            *span,
            resources,
        )?,
        mir::Terminator::CheckedIndirectCall {
            callee,
            function_type,
            invocation_mode,
            args,
            result,
            error,
            success,
            failure,
            span,
        } => lower_checked_indirect_call(
            builder,
            callee,
            *function_type,
            *invocation_mode,
            args,
            *result,
            *error,
            block_for(blocks, *success)?,
            block_for(blocks, *failure)?,
            *span,
            resources,
        )?,
        mir::Terminator::Return(expression) => {
            debug_assert!(resources.deferred_class_temporary_drops.is_empty());
            resources.defer_class_temporary_drops = true;
            let value = lower_rvalue(builder, expression, resources)?;
            resources.defer_class_temporary_drops = false;
            flush_deferred_class_temporary_drops(builder, resources)?;
            if let Some(destination) = resources.return_address.filter(|_| {
                !function_in(resources.program, resources.function_id)
                    .expect("validated function identity")
                    .checked_effects
                    .is_empty()
            }) {
                store_lowered_to_address(
                    builder,
                    expression.ty(),
                    destination,
                    value,
                    resources.module.target_config().pointer_type(),
                )?;
                sync_writable_closure_captures(builder, resources)?;
                cleanup_class_locals(builder, resources)?;
                cleanup_string_locals(builder, resources)?;
                let success = builder.ins().iconst(types::I8, 0);
                builder.ins().return_(&[success]);
                return Ok(());
            }
            sync_writable_closure_captures(builder, resources)?;
            cleanup_class_locals(builder, resources)?;
            cleanup_string_locals(builder, resources)?;
            if let mir::Type::PayloadEnum(payload) | mir::Type::NullablePayloadEnum(payload) =
                expression.ty()
            {
                let destination = resources.return_address.ok_or_else(|| {
                    malformed_mir("payload enum return has no hidden result address")
                })?;
                copy_inline_bytes(
                    builder,
                    destination,
                    value.single()?,
                    payload
                        .storage_size(matches!(expression.ty(), mir::Type::NullablePayloadEnum(_))),
                    resources.module.target_config().pointer_type(),
                );
                builder.ins().return_(&[]);
                return Ok(());
            }
            let mut values = Vec::with_capacity(2);
            value_to_doria_abi(builder, value, expression.ty()).append_to(&mut values);
            builder.ins().return_(&values);
        }
        mir::Terminator::ReturnVoid => {
            sync_writable_closure_captures(builder, resources)?;
            cleanup_class_locals(builder, resources)?;
            cleanup_string_locals(builder, resources)?;
            if resources.checked_error_address.is_some() {
                let success = builder.ins().iconst(types::I8, 0);
                builder.ins().return_(&[success]);
            } else {
                builder.ins().return_(&[]);
            }
        }
        mir::Terminator::Panic { message, span } => {
            set_active_panic_site(builder, *span, resources);
            debug_assert!(resources.deferred_class_temporary_drops.is_empty());
            resources.defer_class_temporary_drops = true;
            let string = lower_string_expression(builder, message, resources)?;
            resources.defer_class_temporary_drops = false;
            // Message evaluation may call another Doria function and update the
            // active panic site. The explicit panic itself originates here.
            set_active_panic_site(builder, *span, resources);
            // Abort-only panic never reaches statement-end destruction.
            resources.deferred_class_temporary_drops.clear();
            let pointer_type = resources.module.target_config().pointer_type();
            let data_id =
                resources.declare_runtime(STRING_DATA, &[pointer_type], Some(pointer_type))?;
            let len_id = resources.declare_runtime(
                STRING_BYTE_LENGTH,
                &[pointer_type],
                Some(pointer_type),
            )?;
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
        mir::Terminator::CheckedCall {
            function,
            args,
            result,
            error,
            success,
            failure,
            span,
        } => {
            set_active_panic_site(builder, *span, resources);
            let lowered = lower_call_args(builder, args, resources)?;
            let callee_definition = function_in(resources.program, *function)?;
            let pointer = resources.module.target_config().pointer_type();
            let mut values = vec![resources.current_frame];
            if let Some(result) = result {
                let slot = local_slot(resources.local_slots, *result)?;
                values.push(builder.ins().stack_addr(pointer, slot, 0));
            }
            let error_slot = local_slot(resources.local_slots, *error)?;
            values.push(builder.ins().stack_addr(pointer, error_slot, 0));
            if let Some(home) =
                direct_call_borrow_home(builder, callee_definition, args, resources)?
            {
                values.push(home);
            }
            values.extend(lowered.abi_values.iter().copied());
            let callee = declared_function(builder, resources, *function)?;
            let call = builder.ins().call(callee, &values);
            let status = *builder
                .inst_results(call)
                .first()
                .ok_or_else(|| malformed_mir("checked call produced no status"))?;
            cleanup_call_arguments(
                builder,
                *function,
                args,
                &lowered,
                callee_definition,
                resources,
            )?;

            let failure_status = builder.create_block();
            let invalid_status = builder.create_block();
            let succeeded = builder.ins().icmp_imm_u(IntCC::Equal, status, 0);
            builder.ins().brif(
                succeeded,
                block_for(blocks, *success)?,
                &[],
                failure_status,
                &[],
            );
            builder.switch_to_block(failure_status);
            let failed = builder.ins().icmp_imm_u(IntCC::Equal, status, 1);
            builder.ins().brif(
                failed,
                block_for(blocks, *failure)?,
                &[],
                invalid_status,
                &[],
            );
            builder.switch_to_block(invalid_status);
            builder
                .ins()
                .trap(TrapCode::unwrap_user(RUNTIME_RETURNED_TRAP));
        }
        mir::Terminator::CheckedConstruct {
            class,
            properties,
            constructor,
            args,
            result,
            error,
            success,
            failure,
            span,
        } => {
            set_active_panic_site(builder, *span, resources);
            let pointer = resources.module.target_config().pointer_type();
            let (object, lowered) =
                lower_class_allocation(builder, *class, properties, args, resources)?;
            let error_slot = local_slot(resources.local_slots, *error)?;
            let mut values = vec![
                resources.current_frame,
                builder.ins().stack_addr(pointer, error_slot, 0),
                object,
            ];
            values.extend(lowered.abi_values.iter().copied());
            let callee = declared_function(builder, resources, *constructor)?;
            let call = builder.ins().call(callee, &values);
            let status = *builder
                .inst_results(call)
                .first()
                .ok_or_else(|| malformed_mir("checked constructor produced no status"))?;
            cleanup_constructor_arguments(
                builder,
                *constructor,
                properties,
                args,
                &lowered,
                resources,
            )?;

            let succeeded = builder.ins().icmp_imm_u(IntCC::Equal, status, 0);
            let success_store = builder.create_block();
            let failure_status = builder.create_block();
            builder
                .ins()
                .brif(succeeded, success_store, &[], failure_status, &[]);
            builder.switch_to_block(success_store);
            builder.ins().stack_store(
                pointer,
                object,
                local_slot(resources.local_slots, *result)?,
                0,
            );
            builder.ins().jump(block_for(blocks, *success)?, &[]);

            builder.switch_to_block(failure_status);
            let failed = builder.ins().icmp_imm_u(IntCC::Equal, status, 1);
            let failed_cleanup = builder.create_block();
            let invalid_status = builder.create_block();
            builder
                .ins()
                .brif(failed, failed_cleanup, &[], invalid_status, &[]);
            builder.switch_to_block(failed_cleanup);
            lower_drop_failed_class_value(builder, object, *class, resources)?;
            builder.ins().jump(block_for(blocks, *failure)?, &[]);

            builder.switch_to_block(invalid_status);
            builder
                .ins()
                .trap(TrapCode::unwrap_user(RUNTIME_RETURNED_TRAP));
        }
        mir::Terminator::CheckedIo {
            operation,
            result,
            error,
            success,
            failure,
            span,
        } => lower_checked_io_terminator(
            builder,
            operation,
            *result,
            *error,
            block_for(blocks, *success)?,
            block_for(blocks, *failure)?,
            *span,
            resources,
        )?,
        mir::Terminator::ErrorSwitch {
            error,
            cases,
            catch_all,
            fallback,
        } => {
            let pointer = resources.module.target_config().pointer_type();
            let slot = local_slot(resources.local_slots, *error)?;
            let descriptor =
                builder
                    .ins()
                    .stack_load(pointer, pointer, slot, pointer.bytes() as i32);
            for (case, target) in cases {
                let next = builder.create_block();
                let expected = lower_error_descriptor_address(builder, *case, resources)?;
                let matches = builder.ins().icmp(IntCC::Equal, descriptor, expected);
                builder
                    .ins()
                    .brif(matches, block_for(blocks, *target)?, &[], next, &[]);
                builder.switch_to_block(next);
            }
            let target = catch_all.unwrap_or(*fallback);
            builder.ins().jump(block_for(blocks, target)?, &[]);
        }
        mir::Terminator::PropagateError { error } => {
            let destination = resources
                .checked_error_address
                .ok_or_else(|| malformed_mir("checked propagation has no caller Error out slot"))?;
            let pointer = resources.module.target_config().pointer_type();
            let slot = local_slot(resources.local_slots, *error)?;
            let value = load_lowered_from_stack(builder, mir::Type::Error, slot, pointer);
            store_lowered_to_address(builder, mir::Type::Error, destination, value, pointer)?;
            let zero = builder.ins().iconst(pointer, 0);
            builder.ins().stack_store(pointer, zero, slot, 0);
            builder
                .ins()
                .stack_store(pointer, zero, slot, pointer.bytes() as i32);
            sync_writable_closure_captures(builder, resources)?;
            cleanup_class_locals(builder, resources)?;
            cleanup_string_locals(builder, resources)?;
            let failure = builder.ins().iconst(types::I8, 1);
            builder.ins().return_(&[failure]);
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn lower_checked_io_terminator(
    builder: &mut FunctionBuilder,
    operation: &mir::CheckedIoOperation,
    result: Option<mir::LocalId>,
    error: mir::LocalId,
    success: Block,
    failure: Block,
    span: crate::source::Span,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    set_active_panic_site(builder, span, resources);
    debug_assert!(resources.deferred_class_temporary_drops.is_empty());
    resources.defer_class_temporary_drops = true;
    let pointer = resources.module.target_config().pointer_type();
    let zero_pointer = builder.ins().iconst(pointer, 0);
    let mut owned_strings = Vec::new();

    let (operation_code, path_or_prompt, contents_object, contents_is_bytes) = match operation {
        mir::CheckedIoOperation::ReadLine { prompt } => {
            let prompt = lower_string_expression(builder, prompt, resources)?;
            owned_strings.push(prompt);
            (CHECKED_IO_READ_LINE, prompt, zero_pointer, false)
        }
        mir::CheckedIoOperation::ReadFile { path, bytes } => {
            let path = lower_string_expression(builder, path, resources)?;
            owned_strings.push(path);
            (
                if *bytes {
                    CHECKED_IO_READ_FILE_BYTES
                } else {
                    CHECKED_IO_READ_FILE_TEXT
                },
                path,
                zero_pointer,
                false,
            )
        }
        mir::CheckedIoOperation::ReadStdinBytes => (
            CHECKED_IO_READ_STDIN_BYTES,
            zero_pointer,
            zero_pointer,
            false,
        ),
        mir::CheckedIoOperation::WriteFile {
            path,
            contents,
            append,
        } => {
            let path = lower_string_expression(builder, path, resources)?;
            owned_strings.push(path);
            let (contents, bytes) =
                lower_checked_io_contents(builder, contents, &mut owned_strings, resources)?;
            (
                if *append {
                    CHECKED_IO_APPEND_FILE
                } else {
                    CHECKED_IO_WRITE_FILE
                },
                path,
                contents,
                bytes,
            )
        }
        mir::CheckedIoOperation::WriteStream { contents, stderr } => {
            let (contents, bytes) =
                lower_checked_io_contents(builder, contents, &mut owned_strings, resources)?;
            (
                if *stderr {
                    CHECKED_IO_WRITE_STDERR
                } else {
                    CHECKED_IO_WRITE_STDOUT
                },
                zero_pointer,
                contents,
                bytes,
            )
        }
    };

    let (contents_data, contents_length) = if contents_object == zero_pointer {
        (zero_pointer, zero_pointer)
    } else {
        let data_symbol = if contents_is_bytes {
            BYTES_DATA
        } else {
            STRING_DATA
        };
        let length_symbol = if contents_is_bytes {
            BYTES_LENGTH
        } else {
            STRING_BYTE_LENGTH
        };
        let data = runtime_call(
            builder,
            data_symbol,
            &[pointer],
            Some(pointer),
            &[contents_object],
            resources,
        )?
        .ok_or_else(|| backend_failure("checked I/O contents data produced no result"))?;
        let length = runtime_call(
            builder,
            length_symbol,
            &[pointer],
            Some(pointer),
            &[contents_object],
            resources,
        )?
        .ok_or_else(|| backend_failure("checked I/O contents length produced no result"))?;
        (data, length)
    };

    let pointer_slot = || {
        StackSlotData::new(
            StackSlotKind::ExplicitSlot,
            pointer.bytes(),
            pointer.bytes().ilog2() as u8,
        )
    };
    let result_slot = builder.create_sized_stack_slot(pointer_slot());
    let message_slot = builder.create_sized_stack_slot(pointer_slot());
    let path_slot = builder.create_sized_stack_slot(pointer_slot());
    let system_code_slot =
        builder.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 8, 3));
    let valid_count_slot = builder.create_sized_stack_slot(pointer_slot());
    let invalid_count_slot = builder.create_sized_stack_slot(pointer_slot());
    let meta_slot =
        builder.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 8, 3));
    let operation_code = builder.ins().iconst(types::I8, i64::from(operation_code));
    let out = [
        builder.ins().stack_addr(pointer, result_slot, 0),
        builder.ins().stack_addr(pointer, message_slot, 0),
        builder.ins().stack_addr(pointer, path_slot, 0),
        builder.ins().stack_addr(pointer, system_code_slot, 0),
        builder.ins().stack_addr(pointer, valid_count_slot, 0),
        builder.ins().stack_addr(pointer, invalid_count_slot, 0),
        builder.ins().stack_addr(pointer, meta_slot, 0),
    ];
    let status = runtime_call(
        builder,
        CHECKED_IO,
        &[
            pointer,
            types::I8,
            pointer,
            pointer,
            pointer,
            pointer,
            pointer,
            pointer,
            pointer,
            pointer,
            pointer,
            pointer,
        ],
        Some(types::I8),
        &[
            resources.current_frame,
            operation_code,
            path_or_prompt,
            contents_data,
            contents_length,
            out[0],
            out[1],
            out[2],
            out[3],
            out[4],
            out[5],
            out[6],
        ],
        resources,
    )?
    .ok_or_else(|| backend_failure("checked I/O produced no status"))?;
    for string in owned_strings {
        release_string(builder, string, resources)?;
    }
    resources.defer_class_temporary_drops = false;
    flush_deferred_class_temporary_drops(builder, resources)?;

    let succeeded = builder.ins().icmp_imm_u(IntCC::Equal, status, 0);
    let success_store = builder.create_block();
    let failure_status = builder.create_block();
    let invalid_status = builder.create_block();
    builder
        .ins()
        .brif(succeeded, success_store, &[], failure_status, &[]);

    builder.switch_to_block(success_store);
    if let Some(result) = result {
        let raw = builder.ins().stack_load(pointer, pointer, result_slot, 0);
        let definition = local_definition(resources.program, resources.function_id, result)?;
        let slot = local_slot(resources.local_slots, result)?;
        match definition.ty {
            mir::Type::NullableString => {
                let present = presence_word(builder, raw, pointer);
                builder.ins().stack_store(pointer, present, slot, 0);
                builder
                    .ins()
                    .stack_store(pointer, raw, slot, pointer.bytes() as i32);
            }
            mir::Type::String | mir::Type::Collection(_) => {
                builder.ins().stack_store(pointer, raw, slot, 0);
            }
            _ => {
                return Err(malformed_mir(
                    "checked I/O has an unsupported native result type",
                ))
            }
        }
    }
    builder.ins().jump(success, &[]);

    builder.switch_to_block(failure_status);
    let failed = builder.ins().icmp_imm_u(IntCC::Equal, status, 1);
    let build_error = builder.create_block();
    builder
        .ins()
        .brif(failed, build_error, &[], invalid_status, &[]);
    builder.switch_to_block(build_error);
    lower_checked_io_error(
        builder,
        error,
        message_slot,
        path_slot,
        system_code_slot,
        valid_count_slot,
        invalid_count_slot,
        meta_slot,
        failure,
        resources,
    )?;

    builder.switch_to_block(invalid_status);
    builder
        .ins()
        .trap(TrapCode::unwrap_user(RUNTIME_RETURNED_TRAP));
    Ok(())
}

fn lower_checked_io_contents(
    builder: &mut FunctionBuilder,
    contents: &mir::IoContents,
    owned_strings: &mut Vec<Value>,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(Value, bool), BackendError> {
    match contents {
        mir::IoContents::String(value) => {
            let value = lower_string_expression(builder, value, resources)?;
            owned_strings.push(value);
            Ok((value, false))
        }
        mir::IoContents::Format(value) => {
            let value = lower_format_expression(builder, value, resources)?;
            owned_strings.push(value);
            Ok((value, false))
        }
        mir::IoContents::Bytes(local) => {
            Ok((lower_collection_pointer(builder, *local, resources)?, true))
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn lower_indirect_call(
    builder: &mut FunctionBuilder,
    callee: &mir::FunctionExpression,
    function_type: mir::FunctionTypeId,
    invocation_mode: mir::FunctionInvocationMode,
    args: &[mir::Rvalue],
    result: Option<mir::LocalId>,
    continuation: Block,
    span: crate::source::Span,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    set_active_panic_site(builder, span, resources);
    let function_type = function_type_in(resources.program, function_type)?.clone();
    if function_type.has_checked_transport() {
        return Err(malformed_mir(
            "throwing function type reached nonthrowing indirect call",
        ));
    }
    let carrier = lower_function_expression(builder, callee, resources)?;
    let (descriptor, environment) = carrier.nullable()?;
    let lowered =
        lower_call_args_with_parameters(builder, args, &function_type.parameters, resources)?;
    let pointer = resources.module.target_config().pointer_type();
    let mut values = vec![resources.current_frame];
    if matches!(
        function_type.return_type,
        mir::ReturnType::Value(mir::Type::PayloadEnum(_) | mir::Type::NullablePayloadEnum(_))
    ) {
        let target = result.ok_or_else(|| malformed_mir("indirect payload call has no result"))?;
        let slot = local_slot(resources.local_slots, target)?;
        values.push(builder.ins().stack_addr(pointer, slot, 0));
    }
    if let Some(home) = indirect_call_borrow_home(builder, &function_type, args, resources)? {
        values.push(home);
    }
    values.push(environment);
    values.extend(lowered.abi_values.iter().copied());
    let entry = load_closure_entry(builder, descriptor, pointer);
    let signature = indirect_function_signature(resources.module, &function_type);
    let signature = builder.import_signature(signature);
    let call = builder.ins().call_indirect(signature, entry, &values);
    let call_results = builder.inst_results(call).to_vec();
    store_indirect_result(
        builder,
        &function_type.return_type,
        result,
        &call_results,
        resources,
    )?;
    cleanup_indirect_call_arguments(builder, &function_type, args, &lowered, resources)?;
    if invocation_mode == mir::FunctionInvocationMode::Once {
        lower_drop_function_carrier(builder, carrier, resources)?;
    }
    builder.ins().jump(continuation, &[]);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn lower_checked_indirect_call(
    builder: &mut FunctionBuilder,
    callee: &mir::FunctionExpression,
    function_type: mir::FunctionTypeId,
    invocation_mode: mir::FunctionInvocationMode,
    args: &[mir::Rvalue],
    result: Option<mir::LocalId>,
    error: mir::LocalId,
    success: Block,
    failure: Block,
    span: crate::source::Span,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    set_active_panic_site(builder, span, resources);
    let function_type = function_type_in(resources.program, function_type)?.clone();
    if !function_type.has_checked_transport() {
        return Err(malformed_mir(
            "nonthrowing function type reached checked indirect call",
        ));
    }
    let carrier = lower_function_expression(builder, callee, resources)?;
    let (descriptor, environment) = carrier.nullable()?;
    let lowered =
        lower_call_args_with_parameters(builder, args, &function_type.parameters, resources)?;
    let pointer = resources.module.target_config().pointer_type();
    let mut values = vec![resources.current_frame];
    if let Some(result) = result {
        let slot = local_slot(resources.local_slots, result)?;
        values.push(builder.ins().stack_addr(pointer, slot, 0));
    }
    let error_slot = local_slot(resources.local_slots, error)?;
    values.push(builder.ins().stack_addr(pointer, error_slot, 0));
    if let Some(home) = indirect_call_borrow_home(builder, &function_type, args, resources)? {
        values.push(home);
    }
    values.push(environment);
    values.extend(lowered.abi_values.iter().copied());
    let entry = load_closure_entry(builder, descriptor, pointer);
    let signature = indirect_function_signature(resources.module, &function_type);
    let signature = builder.import_signature(signature);
    let call = builder.ins().call_indirect(signature, entry, &values);
    let status = *builder
        .inst_results(call)
        .first()
        .ok_or_else(|| malformed_mir("checked indirect call produced no status"))?;
    cleanup_indirect_call_arguments(builder, &function_type, args, &lowered, resources)?;
    if invocation_mode == mir::FunctionInvocationMode::Once {
        lower_drop_function_carrier(builder, carrier, resources)?;
    }
    let invalid_status = builder.create_block();
    let failed_status = builder.create_block();
    let succeeded = builder.ins().icmp_imm_u(IntCC::Equal, status, 0);
    builder
        .ins()
        .brif(succeeded, success, &[], failed_status, &[]);
    builder.switch_to_block(failed_status);
    let failed = builder.ins().icmp_imm_u(IntCC::Equal, status, 1);
    builder
        .ins()
        .brif(failed, failure, &[], invalid_status, &[]);
    builder.switch_to_block(invalid_status);
    builder
        .ins()
        .trap(TrapCode::unwrap_user(RUNTIME_RETURNED_TRAP));
    Ok(())
}

fn load_closure_entry(
    builder: &mut FunctionBuilder,
    descriptor: Value,
    pointer: ClifType,
) -> Value {
    let layout = native_closure_abi::descriptor_layout(pointer.bytes());
    builder.ins().load(
        pointer,
        cranelift_codegen::ir::MachMemFlags::trusted(),
        descriptor,
        layout.entry_offset as i32,
    )
}

fn indirect_function_signature(
    module: &mut ObjectModule,
    function_type: &mir::FunctionType,
) -> Signature {
    let pointer = module.target_config().pointer_type();
    let mut signature = module.make_signature();
    let plan = native_closure_abi::NativeCallableSignaturePlan::indirect(function_type);
    for _ in &plan.hidden_inputs {
        signature.params.push(AbiParam::new(pointer));
    }
    for parameter in &function_type.parameters {
        if parameter.mode == mir::FunctionParameterMode::Writable {
            signature.params.push(AbiParam::new(pointer));
        } else {
            append_type_abi_params(&mut signature.params, parameter.ty, pointer);
        }
    }
    if plan.checked {
        signature.returns.push(AbiParam::new(types::I8));
    } else if let mir::ReturnType::Value(ty) = function_type.return_type {
        if !matches!(
            ty,
            mir::Type::PayloadEnum(_) | mir::Type::NullablePayloadEnum(_)
        ) {
            append_type_abi_params(&mut signature.returns, ty, pointer);
        }
    }
    signature
}

fn store_indirect_result(
    builder: &mut FunctionBuilder,
    return_type: &mir::ReturnType,
    result: Option<mir::LocalId>,
    results: &[Value],
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let mir::ReturnType::Value(ty) = *return_type else {
        return Ok(());
    };
    if matches!(
        ty,
        mir::Type::PayloadEnum(_) | mir::Type::NullablePayloadEnum(_)
    ) {
        return Ok(());
    }
    let target = result.ok_or_else(|| malformed_mir("value indirect call has no result local"))?;
    let value = if matches!(
        ty,
        mir::Type::NullableScalar(_)
            | mir::Type::NullableString
            | mir::Type::Error
            | mir::Type::NullableError
            | mir::Type::Function(_)
            | mir::Type::NullableFunction(_)
    ) {
        let present = *results
            .first()
            .ok_or_else(|| malformed_mir("indirect call has no first result word"))?;
        let payload = *results
            .get(1)
            .ok_or_else(|| malformed_mir("indirect call has no second result word"))?;
        LoweredValue::Nullable {
            present,
            payload: nullable_payload_from_doria_abi(builder, payload, ty),
        }
    } else {
        let value = *results
            .first()
            .ok_or_else(|| malformed_mir("indirect call has no result"))?;
        LoweredValue::Single(value_from_doria_abi(builder, value, ty))
    };
    let pointer = resources.module.target_config().pointer_type();
    let slot = local_slot(resources.local_slots, target)?;
    store_lowered_to_stack(builder, ty, slot, value, pointer)
}

fn cleanup_indirect_call_arguments(
    builder: &mut FunctionBuilder,
    function_type: &mir::FunctionType,
    args: &[mir::Rvalue],
    lowered: &LoweredCallArgs,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    for (_, string) in &lowered.owned_strings {
        release_string(builder, *string, resources)?;
    }
    for (index, value, ownership) in &lowered.temporary_mixed {
        if args[*index].transferred_owned_local().is_some()
            || function_type.parameters[*index].mode == mir::FunctionParameterMode::Take
        {
            continue;
        }
        lower_cleanup_mixed_temporary(builder, *value, *ownership, resources)?;
    }
    for index in ordered_owned_argument_indices(args) {
        let argument = &args[index];
        if function_type.parameters[index].mode == mir::FunctionParameterMode::Take {
            continue;
        }
        let value = lowered.arguments[index];
        if let Some(class) = argument.owned_temporary_class() {
            defer_or_drop_class_temporary(builder, value.single()?, class, resources)?;
        } else if let Some(collection) = argument.owned_temporary_collection() {
            defer_or_drop_collection_temporary(builder, value.single()?, collection, resources)?;
        } else if let Some(shared) = argument.owned_temporary_shared() {
            defer_or_drop_owned_shared_temporary(builder, value.single()?, shared, resources)?;
        } else if let Some((payload, nullable)) = argument.owned_temporary_payload_enum() {
            lower_drop_payload_enum_at(builder, value.single()?, payload, nullable, resources)?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn lower_checked_io_error(
    builder: &mut FunctionBuilder,
    error: mir::LocalId,
    message_slot: StackSlot,
    path_slot: StackSlot,
    system_code_slot: StackSlot,
    valid_count_slot: StackSlot,
    invalid_count_slot: StackSlot,
    meta_slot: StackSlot,
    failure: Block,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let meta = builder.ins().stack_load(pointer, types::I64, meta_slot, 0);
    let kind = checked_io_meta_byte(builder, meta, CHECKED_IO_META_KIND_SHIFT);
    let invalid_utf8 =
        builder
            .ins()
            .icmp_imm_u(IntCC::Equal, kind, i64::from(CHECKED_IO_ERROR_INVALID_UTF8));
    let invalid = builder.create_block();
    let ordinary = builder.create_block();
    builder
        .ins()
        .brif(invalid_utf8, invalid, &[], ordinary, &[]);

    builder.switch_to_block(ordinary);
    let is_io = builder
        .ins()
        .icmp_imm_u(IntCC::Equal, kind, i64::from(CHECKED_IO_ERROR_IO));
    let valid_io = builder.create_block();
    let malformed = builder.create_block();
    builder.ins().brif(is_io, valid_io, &[], malformed, &[]);
    builder.switch_to_block(valid_io);
    lower_checked_io_error_object(
        builder,
        crate::compiler_known_io::IO_ERROR,
        error,
        message_slot,
        path_slot,
        system_code_slot,
        valid_count_slot,
        invalid_count_slot,
        meta,
        resources,
    )?;
    builder.ins().jump(failure, &[]);

    builder.switch_to_block(invalid);
    lower_checked_io_error_object(
        builder,
        crate::compiler_known_io::INVALID_UTF8_ERROR,
        error,
        message_slot,
        path_slot,
        system_code_slot,
        valid_count_slot,
        invalid_count_slot,
        meta,
        resources,
    )?;
    builder.ins().jump(failure, &[]);

    builder.switch_to_block(malformed);
    builder
        .ins()
        .trap(TrapCode::unwrap_user(RUNTIME_RETURNED_TRAP));
    let _ = pointer;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn lower_checked_io_error_object(
    builder: &mut FunctionBuilder,
    type_name: &str,
    error: mir::LocalId,
    message_slot: StackSlot,
    path_slot: StackSlot,
    system_code_slot: StackSlot,
    valid_count_slot: StackSlot,
    invalid_count_slot: StackSlot,
    meta: Value,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let class = resources
        .program
        .classes
        .iter()
        .find(|class| class.name == type_name)
        .ok_or_else(|| malformed_mir(format!("compiler-known class `{type_name}` is missing")))?
        .clone();
    let descriptor = resources
        .program
        .error_descriptors
        .iter()
        .find(|descriptor| descriptor.type_name == type_name)
        .ok_or_else(|| {
            malformed_mir(format!(
                "compiler-known Error `{type_name}` has no descriptor"
            ))
        })?
        .id;
    let size = builder.ins().iconst(pointer, i64::from(class.layout.size));
    let align = builder.ins().iconst(pointer, i64::from(class.layout.align));
    let object = runtime_call(
        builder,
        CLASS_ALLOCATE,
        &[pointer, pointer, pointer],
        Some(pointer),
        &[resources.current_frame, size, align],
        resources,
    )?
    .ok_or_else(|| backend_failure("checked I/O Error allocation produced no result"))?;
    let message = builder.ins().stack_load(pointer, pointer, message_slot, 0);
    store_known_property(
        builder,
        object,
        &class,
        "message",
        LoweredValue::Single(message),
        resources,
    )?;

    if type_name == crate::compiler_known_io::IO_ERROR {
        let operation = checked_io_meta_byte(builder, meta, CHECKED_IO_META_OPERATION_SHIFT);
        let operation = builder.ins().uextend(types::I32, operation);
        store_known_property(
            builder,
            object,
            &class,
            "operation",
            LoweredValue::Single(operation),
            resources,
        )?;
        let target = checked_io_meta_byte(builder, meta, CHECKED_IO_META_TARGET_SHIFT);
        let path = builder.ins().stack_load(pointer, pointer, path_slot, 0);
        store_checked_io_payload_property(
            builder,
            object,
            &class,
            "target",
            crate::compiler_known_io::IO_TARGET,
            target,
            path,
            resources,
        )?;
        let reason = checked_io_meta_byte(builder, meta, CHECKED_IO_META_REASON_SHIFT);
        let reason = builder.ins().uextend(types::I32, reason);
        store_known_property(
            builder,
            object,
            &class,
            "reason",
            LoweredValue::Single(reason),
            resources,
        )?;
        let present = checked_io_meta_bit(builder, meta, CHECKED_IO_META_HAS_SYSTEM_CODE_SHIFT);
        let system_code = builder
            .ins()
            .stack_load(pointer, types::I64, system_code_slot, 0);
        store_known_property(
            builder,
            object,
            &class,
            "systemCode",
            LoweredValue::Nullable {
                present,
                payload: system_code,
            },
            resources,
        )?;
    } else {
        let source = checked_io_meta_byte(builder, meta, CHECKED_IO_META_TARGET_SHIFT);
        let path = builder.ins().stack_load(pointer, pointer, path_slot, 0);
        store_checked_io_payload_property(
            builder,
            object,
            &class,
            "source",
            crate::compiler_known_io::UTF8_INPUT_SOURCE,
            source,
            path,
            resources,
        )?;
        let valid = builder
            .ins()
            .stack_load(pointer, pointer, valid_count_slot, 0);
        let valid = if pointer == types::I64 {
            valid
        } else {
            builder.ins().uextend(types::I64, valid)
        };
        store_known_property(
            builder,
            object,
            &class,
            "validByteCount",
            LoweredValue::Single(valid),
            resources,
        )?;
        let present = checked_io_meta_bit(builder, meta, CHECKED_IO_META_HAS_INVALID_COUNT_SHIFT);
        let invalid = builder
            .ins()
            .stack_load(pointer, pointer, invalid_count_slot, 0);
        let invalid = if pointer == types::I64 {
            invalid
        } else {
            builder.ins().uextend(types::I64, invalid)
        };
        store_known_property(
            builder,
            object,
            &class,
            "invalidByteCount",
            LoweredValue::Nullable {
                present,
                payload: invalid,
            },
            resources,
        )?;
    }

    let error_slot = local_slot(resources.local_slots, error)?;
    let error_address = builder.ins().stack_addr(pointer, error_slot, 0);
    let descriptor = lower_error_descriptor_address(builder, descriptor, resources)?;
    store_lowered_to_address(
        builder,
        mir::Type::Error,
        error_address,
        LoweredValue::Nullable {
            present: object,
            payload: descriptor,
        },
        pointer,
    )?;
    Ok(())
}

fn store_known_property(
    builder: &mut FunctionBuilder,
    object: Value,
    class: &mir::Class,
    name: &str,
    value: LoweredValue,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let property = class
        .properties
        .iter()
        .find(|property| property.name == name)
        .ok_or_else(|| malformed_mir(format!("class `{}` has no `${name}`", class.name)))?;
    let address = lower_property_address_from_value(builder, object, property.id, resources)?;
    store_lowered_to_address(
        builder,
        property.ty,
        address,
        value,
        resources.module.target_config().pointer_type(),
    )
}

#[allow(clippy::too_many_arguments)]
fn store_checked_io_payload_property(
    builder: &mut FunctionBuilder,
    object: Value,
    class: &mir::Class,
    property_name: &str,
    enum_name: &str,
    tag: Value,
    path: Value,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let property = class
        .properties
        .iter()
        .find(|property| property.name == property_name)
        .ok_or_else(|| {
            malformed_mir(format!("class `{}` has no `${property_name}`", class.name))
        })?;
    let mir::Type::PayloadEnum(payload) = property.ty else {
        return Err(malformed_mir(format!(
            "compiler-known `${property_name}` is not a payload enum"
        )));
    };
    let definition = resources
        .program
        .enums
        .iter()
        .find(|definition| definition.name == enum_name)
        .ok_or_else(|| malformed_mir(format!("compiler-known enum `{enum_name}` is missing")))?
        .clone();
    if definition.id != payload.id {
        return Err(malformed_mir(
            "compiler-known payload enum identity changed",
        ));
    }
    let address = lower_property_address_from_value(builder, object, property.id, resources)?;
    let pointer = resources.module.target_config().pointer_type();
    zero_inline_bytes(builder, address, payload.size, pointer);
    let tag_type = clif_tag_type(definition.layout.tag_width)?;
    let tag = if tag_type == types::I8 {
        tag
    } else {
        builder.ins().uextend(tag_type, tag)
    };
    builder.ins().store(
        cranelift_codegen::ir::MachMemFlags::trusted(),
        tag,
        address,
        definition.layout.tag_offset as i32,
    );
    let file_case = definition
        .cases
        .first()
        .filter(|case| case.name == "File")
        .ok_or_else(|| malformed_mir("compiler-known I/O enum has no leading File case"))?;
    let field = definition
        .layout
        .cases
        .get(file_case.id.index)
        .and_then(|layout| layout.fields.first())
        .ok_or_else(|| malformed_mir("compiler-known File case has no path field"))?;
    builder.ins().store(
        cranelift_codegen::ir::MachMemFlags::trusted(),
        path,
        address,
        field.offset as i32,
    );
    Ok(())
}

fn checked_io_meta_byte(builder: &mut FunctionBuilder, meta: Value, shift: u32) -> Value {
    let shifted = builder.ins().ushr_imm_u(meta, i64::from(shift));
    let masked = builder.ins().band_imm_u(shifted, 0xff);
    builder.ins().ireduce(types::I8, masked)
}

fn checked_io_meta_bit(builder: &mut FunctionBuilder, meta: Value, shift: u32) -> Value {
    let shifted = builder.ins().ushr_imm_u(meta, i64::from(shift));
    let masked = builder.ins().band_imm_u(shifted, 1);
    let bit = builder.ins().ireduce(types::I8, masked);
    builder.ins().uextend(types::I64, bit)
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
        mir::ValueExpression::Enum(value) => lower_enum_expression(builder, value, resources),
    }
}

fn lower_enum_expression(
    builder: &mut FunctionBuilder,
    expression: &mir::EnumExpression,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    match expression {
        mir::EnumExpression::Case(value) => {
            Ok(builder.ins().iconst(types::I32, value.case_id.index as i64))
        }
        mir::EnumExpression::Use { enum_id, operand } => {
            lower_enum_operand(builder, *enum_id, operand, resources)
        }
        mir::EnumExpression::Call { function, args, .. } => {
            lower_function_call(builder, *function, args, resources)?
                .ok_or_else(|| malformed_mir("enum call produced no result"))?
                .single()
        }
        mir::EnumExpression::Coalesce { left, right, .. } => {
            let left = lower_nullable_scalar_expression(builder, left, resources)?;
            let (present, payload) = left.nullable()?;
            lower_coalesce_value(
                builder,
                present,
                payload,
                types::I32,
                resources,
                |builder, resources| lower_enum_expression(builder, right, resources),
            )
        }
    }
}

fn lower_rvalue(
    builder: &mut FunctionBuilder,
    expression: &mir::Rvalue,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<LoweredValue, BackendError> {
    match expression {
        mir::Rvalue::Function(value) => lower_function_expression(builder, value, resources),
        mir::Rvalue::NullableFunction(value) => {
            lower_nullable_function_expression(builder, value, resources)
        }
        mir::Rvalue::Value(value) => {
            lower_value_expression(builder, value, resources).map(LoweredValue::Single)
        }
        mir::Rvalue::String(value) => {
            lower_string_expression(builder, value, resources).map(LoweredValue::Single)
        }
        mir::Rvalue::Mixed(value) => {
            lower_mixed_expression(builder, value, resources).map(LoweredValue::Single)
        }
        mir::Rvalue::NullableScalar(value) => {
            lower_nullable_scalar_expression(builder, value, resources)
        }
        mir::Rvalue::NullableString(value) => {
            lower_nullable_string_expression(builder, value, resources)
        }
        mir::Rvalue::NullableMixed(value) => {
            lower_nullable_mixed_expression(builder, value, resources).map(LoweredValue::Single)
        }
        mir::Rvalue::Error(value) => lower_error_expression(builder, value, resources),
        mir::Rvalue::NullableError(value) => {
            lower_nullable_error_expression(builder, value, resources)
        }
        mir::Rvalue::Class(value) => {
            lower_class_expression(builder, value, resources).map(LoweredValue::Single)
        }
        mir::Rvalue::NullableClass(value) => {
            lower_nullable_class_expression(builder, value, resources).map(LoweredValue::Single)
        }
        mir::Rvalue::SharedReference(value) => {
            lower_shared_reference_expression(builder, value, resources).map(LoweredValue::Single)
        }
        mir::Rvalue::WeakReference(value) => {
            lower_weak_reference_expression(builder, value, resources).map(LoweredValue::Single)
        }
        mir::Rvalue::NullableSharedReference(value) => {
            lower_nullable_shared_reference_expression(builder, value, resources)
                .map(LoweredValue::Single)
        }
        mir::Rvalue::NullableWeakReference(value) => {
            lower_nullable_weak_reference_expression(builder, value, resources)
                .map(LoweredValue::Single)
        }
        mir::Rvalue::WritableSharedReference(value) => {
            lower_writable_shared_reference_expression(builder, value, resources)
                .map(LoweredValue::Single)
        }
        mir::Rvalue::WritableWeakReference(value) => {
            lower_writable_weak_reference_expression(builder, value, resources)
                .map(LoweredValue::Single)
        }
        mir::Rvalue::NullableWritableSharedReference(value) => {
            lower_nullable_writable_shared_reference_expression(builder, value, resources)
                .map(LoweredValue::Single)
        }
        mir::Rvalue::NullableWritableWeakReference(value) => {
            lower_nullable_writable_weak_reference_expression(builder, value, resources)
                .map(LoweredValue::Single)
        }
        mir::Rvalue::SharedReferenceAccess(value) => {
            lower_shared_reference_access_expression(builder, value, resources)
                .map(LoweredValue::Single)
        }
        mir::Rvalue::NullableSharedReferenceAccess(value) => {
            lower_nullable_shared_reference_access_expression(builder, value, resources)
                .map(LoweredValue::Single)
        }
        mir::Rvalue::Collection(value) => {
            lower_collection_expression(builder, value, resources).map(LoweredValue::Single)
        }
        mir::Rvalue::NullableCollection(value) => {
            lower_nullable_collection_expression(builder, value, resources)
                .map(LoweredValue::Single)
        }
        mir::Rvalue::PayloadEnum(value) => {
            lower_payload_enum_expression(builder, value, resources).map(LoweredValue::Single)
        }
        mir::Rvalue::NullablePayloadEnum(value) => {
            lower_nullable_payload_enum_expression(builder, value, resources)
                .map(LoweredValue::Single)
        }
    }
}

fn lower_function_expression(
    builder: &mut FunctionBuilder,
    expression: &mir::FunctionExpression,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<LoweredValue, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    match expression {
        mir::FunctionExpression::Create {
            descriptor,
            captures,
            ..
        } => {
            let descriptor_id = *resources
                .closure_descriptor_ids
                .get(descriptor.0)
                .ok_or_else(|| malformed_mir("closure descriptor was not emitted"))?;
            let descriptor_global = resources
                .module
                .declare_data_in_func(descriptor_id, builder.func);
            let descriptor_pointer = builder.ins().symbol_value(pointer, descriptor_global);
            let environment =
                lower_closure_environment_create(builder, *descriptor, captures, resources)?;
            Ok(LoweredValue::Nullable {
                present: descriptor_pointer,
                payload: environment,
            })
        }
        mir::FunctionExpression::Local {
            local, transfer, ..
        } => {
            let slot = local_slot(resources.local_slots, *local)?;
            let value = load_lowered_from_stack(
                builder,
                mir::Type::Function(expression.function_type()),
                slot,
                pointer,
            );
            if *transfer {
                clear_function_carrier_stack(builder, slot, pointer);
            }
            Ok(value)
        }
        mir::FunctionExpression::Property {
            object, property, ..
        } => {
            let address = lower_property_address(builder, *object, *property, resources)?;
            Ok(load_lowered_from_address(
                builder,
                mir::Type::Function(expression.function_type()),
                address,
                pointer,
            ))
        }
        mir::FunctionExpression::Call { function, args, .. } => {
            lower_function_call(builder, *function, args, resources)?
                .ok_or_else(|| malformed_mir("function-valued call returned void"))
        }
        mir::FunctionExpression::AssumePresent { value, .. } => {
            lower_nullable_function_expression(builder, value, resources)
        }
        mir::FunctionExpression::CollectionIndex {
            collection,
            index,
            positional,
            remove,
            ..
        } => lower_two_word_collection_index(
            builder,
            *collection,
            index,
            *positional,
            *remove,
            resources,
        ),
        mir::FunctionExpression::MixedPayload {
            function_type,
            mixed,
            transfer,
        } => lower_mixed_function_payload(builder, *mixed, *function_type, *transfer, resources),
    }
}

fn lower_closure_environment_create(
    builder: &mut FunctionBuilder,
    descriptor_id: mir::ClosureDescriptorId,
    captures: &[mir::ClosureCaptureOperand],
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let descriptor = closure_descriptor_in(resources.program, descriptor_id)?.clone();
    let Some(logical_id) = descriptor.environment_layout else {
        if !captures.is_empty() {
            return Err(malformed_mir(
                "environment-free closure has capture operands",
            ));
        }
        return Ok(builder.ins().iconst(pointer, 0));
    };
    let logical = closure_environment_layout_in(resources.program, logical_id)?.clone();
    let native =
        native_closure_abi::environment_layout(resources.program, logical_id, pointer.bytes())?;
    if captures.len() != logical.fields.len() {
        return Err(malformed_mir(
            "closure capture count disagrees with its environment layout",
        ));
    }
    let environment = match descriptor.environment_placement {
        mir::ClosureEnvironmentPlacement::None => {
            return Err(malformed_mir(
                "capturing closure uses environment-free placement",
            ))
        }
        mir::ClosureEnvironmentPlacement::Stack => {
            let slot = resources
                .closure_environment_slots
                .get(descriptor_id.0)
                .copied()
                .flatten()
                .ok_or_else(|| malformed_mir("stack closure environment has no stack slot"))?;
            builder.ins().stack_addr(pointer, slot, 0)
        }
        mir::ClosureEnvironmentPlacement::Heap => {
            let size = builder.ins().iconst(pointer, i64::from(native.layout.size));
            let align = builder
                .ins()
                .iconst(pointer, i64::from(native.layout.align));
            runtime_call(
                builder,
                CLOSURE_ENVIRONMENT_ALLOCATE,
                &[pointer, pointer, pointer],
                Some(pointer),
                &[resources.current_frame, size, align],
                resources,
            )?
            .ok_or_else(|| backend_failure("closure environment allocation produced no result"))?
        }
    };
    zero_inline_bytes(builder, environment, native.layout.size, pointer);
    for ((field, field_layout), capture) in logical
        .fields
        .iter()
        .zip(native.fields.iter())
        .zip(captures)
    {
        if field.id != field_layout.field {
            return Err(malformed_mir(
                "native closure field identity disagrees with MIR",
            ));
        }
        let address = builder
            .ins()
            .iadd_imm_u(environment, i64::from(field_layout.offset));
        match capture {
            mir::ClosureCaptureOperand::BorrowLocal { local, writable } => {
                let expected = if *writable {
                    mir::ClosureEnvironmentStorage::WritableBorrow
                } else {
                    mir::ClosureEnvironmentStorage::ReadonlyBorrow
                };
                if field.storage != expected {
                    return Err(malformed_mir(
                        "borrow capture storage disagrees with environment layout",
                    ));
                }
                let source = closure_source_address(builder, *local, resources)?;
                builder.ins().store(
                    cranelift_codegen::ir::MachMemFlags::trusted(),
                    source,
                    address,
                    0,
                );
            }
            mir::ClosureCaptureOperand::CopyValue(value)
            | mir::ClosureCaptureOperand::MoveValue(value) => {
                if field.storage != mir::ClosureEnvironmentStorage::Owned {
                    return Err(malformed_mir(
                        "owned capture uses a borrowed environment field",
                    ));
                }
                let value = lower_rvalue(builder, value, resources)?;
                store_lowered_to_address(builder, field.ty, address, value, pointer)?;
                if let Some(bit) = field_layout.live_bit {
                    set_environment_live_bit(builder, environment, bit, true);
                }
            }
        }
    }
    Ok(environment)
}

fn closure_source_address(
    builder: &mut FunctionBuilder,
    local: mir::LocalId,
    resources: &LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    if let Some(field) = resources.closure_bound_fields.get(&local) {
        return Ok(field.address);
    }
    if let Some(address) = resources.writable_parameter_addresses.get(&local) {
        return Ok(*address);
    }
    if let Some(address) = resources.borrow_home_addresses.get(&local) {
        return Ok(*address);
    }
    let pointer = resources.module.target_config().pointer_type();
    let slot = local_slot(resources.local_slots, local)?;
    Ok(builder.ins().stack_addr(pointer, slot, 0))
}

fn direct_call_borrow_home(
    builder: &mut FunctionBuilder,
    callee: &mir::Function,
    args: &[mir::Rvalue],
    resources: &LoweringResources<'_, '_>,
) -> Result<Option<Value>, BackendError> {
    let Some(return_borrow) = callee
        .return_borrow
        .filter(|_| native_closure_abi::returns_function_value(callee.return_type))
    else {
        return Ok(None);
    };
    let index = native_closure_abi::return_borrow_argument_index(
        return_borrow,
        callee.receiver_mode.is_some(),
    );
    let source = args.get(index).ok_or_else(|| {
        malformed_mir(format!(
            "borrow-returning call to {} has no source argument",
            callee.name
        ))
    })?;
    Ok(Some(rvalue_borrow_home(builder, source, resources)?))
}

fn indirect_call_borrow_home(
    builder: &mut FunctionBuilder,
    function_type: &mir::FunctionType,
    args: &[mir::Rvalue],
    resources: &LoweringResources<'_, '_>,
) -> Result<Option<Value>, BackendError> {
    let Some(return_borrow) = function_type
        .return_borrow
        .filter(|_| native_closure_abi::returns_function_value(function_type.return_type))
    else {
        return Ok(None);
    };
    let source = args
        .get(native_closure_abi::return_borrow_argument_index(
            return_borrow,
            false,
        ))
        .ok_or_else(|| malformed_mir("borrow-returning indirect call has no source argument"))?;
    Ok(Some(rvalue_borrow_home(builder, source, resources)?))
}

fn rvalue_borrow_home(
    builder: &mut FunctionBuilder,
    source: &mir::Rvalue,
    resources: &LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let place = match source {
        mir::Rvalue::Value(mir::ValueExpression::Integer(mir::IntegerExpression::Use {
            operand,
            ..
        }))
        | mir::Rvalue::Value(mir::ValueExpression::Float(mir::FloatExpression::Use {
            operand,
            ..
        }))
        | mir::Rvalue::Value(mir::ValueExpression::Bool(mir::BoolExpression::Use { operand }))
        | mir::Rvalue::Value(mir::ValueExpression::Enum(mir::EnumExpression::Use {
            operand,
            ..
        })) => return operand_borrow_home(builder, operand, resources),
        mir::Rvalue::String(mir::StringExpression::Local(local))
        | mir::Rvalue::String(mir::StringExpression::NullableLocalAssumeNonNull(local))
        | mir::Rvalue::Class(mir::ClassExpression::Local { local, .. })
        | mir::Rvalue::Class(mir::ClassExpression::NullableLocalAssumeNonNull { local, .. })
        | mir::Rvalue::Collection(mir::CollectionExpression::Local { local, .. })
        | mir::Rvalue::Mixed(mir::MixedExpression::Local { local, .. })
        | mir::Rvalue::Error(mir::ErrorExpression::Local { local, .. })
        | mir::Rvalue::Error(mir::ErrorExpression::NullableLocalAssumeNonNull { local, .. })
        | mir::Rvalue::Function(mir::FunctionExpression::Local { local, .. })
        | mir::Rvalue::NullableFunction(mir::NullableFunctionExpression::Local { local, .. }) => {
            Some((*local, None))
        }
        mir::Rvalue::NullableScalar(mir::NullableScalarExpression::Local { local, .. })
        | mir::Rvalue::NullableString(mir::NullableStringExpression::Local(local))
        | mir::Rvalue::NullableClass(mir::NullableClassExpression::Local { local, .. })
        | mir::Rvalue::NullableCollection(mir::NullableCollectionExpression::Local {
            local, ..
        })
        | mir::Rvalue::NullableMixed(mir::NullableMixedExpression::Local { local, .. })
        | mir::Rvalue::NullableError(mir::NullableErrorExpression::Local { local, .. }) => {
            Some((*local, None))
        }
        mir::Rvalue::String(mir::StringExpression::Property {
            object, property, ..
        })
        | mir::Rvalue::Class(mir::ClassExpression::Property {
            object, property, ..
        })
        | mir::Rvalue::Collection(mir::CollectionExpression::Property {
            object, property, ..
        })
        | mir::Rvalue::Mixed(mir::MixedExpression::Property {
            object, property, ..
        })
        | mir::Rvalue::Error(mir::ErrorExpression::Property {
            object, property, ..
        })
        | mir::Rvalue::Function(mir::FunctionExpression::Property {
            object, property, ..
        })
        | mir::Rvalue::NullableScalar(mir::NullableScalarExpression::Property {
            object,
            property,
            ..
        })
        | mir::Rvalue::NullableString(mir::NullableStringExpression::Property {
            object,
            property,
            ..
        })
        | mir::Rvalue::NullableClass(mir::NullableClassExpression::Property {
            object,
            property,
            ..
        })
        | mir::Rvalue::NullableFunction(mir::NullableFunctionExpression::Property {
            object,
            property,
            ..
        }) => Some((*object, Some(*property))),
        _ => None,
    };
    match place {
        Some((local, None)) => closure_source_address(builder, local, resources),
        Some((object, Some(property))) => {
            lower_property_address(builder, object, property, resources)
        }
        None => Err(malformed_mir(
            "borrow-returning call source is not an addressable Doria place",
        )),
    }
}

fn operand_borrow_home(
    builder: &mut FunctionBuilder,
    operand: &mir::Operand,
    resources: &LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    match operand {
        mir::Operand::Local(local) | mir::Operand::NullablePayload(local) => {
            closure_source_address(builder, *local, resources)
        }
        mir::Operand::Property { object, property } => {
            lower_property_address(builder, *object, *property, resources)
        }
        _ => Err(malformed_mir(
            "borrow-returning call source is not an addressable scalar place",
        )),
    }
}

fn set_environment_live_bit(
    builder: &mut FunctionBuilder,
    environment: Value,
    bit: u32,
    live: bool,
) {
    let byte_offset = (bit / 8) as i32;
    let bit_mask = 1_i64 << (bit % 8);
    let flags = cranelift_codegen::ir::MachMemFlags::trusted();
    let current = builder
        .ins()
        .load(types::I8, flags, environment, byte_offset);
    let next = if live {
        builder.ins().bor_imm_u(current, bit_mask)
    } else {
        builder.ins().band_imm_u(current, (!bit_mask) & 0xff)
    };
    builder.ins().store(flags, next, environment, byte_offset);
}

fn lower_nullable_function_expression(
    builder: &mut FunctionBuilder,
    expression: &mir::NullableFunctionExpression,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<LoweredValue, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    match expression {
        mir::NullableFunctionExpression::Null { .. } => {
            let zero = builder.ins().iconst(pointer, 0);
            Ok(LoweredValue::Nullable {
                present: zero,
                payload: zero,
            })
        }
        mir::NullableFunctionExpression::Present(value) => {
            lower_function_expression(builder, value, resources)
        }
        mir::NullableFunctionExpression::Local {
            local, transfer, ..
        } => {
            let slot = local_slot(resources.local_slots, *local)?;
            let value = load_lowered_from_stack(
                builder,
                mir::Type::NullableFunction(expression.function_type()),
                slot,
                pointer,
            );
            if *transfer {
                clear_function_carrier_stack(builder, slot, pointer);
            }
            Ok(value)
        }
        mir::NullableFunctionExpression::Property {
            object, property, ..
        } => {
            let address = lower_property_address(builder, *object, *property, resources)?;
            Ok(load_lowered_from_address(
                builder,
                mir::Type::NullableFunction(expression.function_type()),
                address,
                pointer,
            ))
        }
        mir::NullableFunctionExpression::Call { function, args, .. } => {
            lower_function_call(builder, *function, args, resources)?
                .ok_or_else(|| malformed_mir("nullable function-valued call returned void"))
        }
        mir::NullableFunctionExpression::DictionaryGet {
            collection,
            key,
            access,
            ..
        } => lower_nullable_two_word_collection_get(builder, *collection, key, *access, resources),
        mir::NullableFunctionExpression::CollectionIndex {
            collection,
            index,
            positional,
            remove,
            ..
        } => lower_two_word_collection_index(
            builder,
            *collection,
            index,
            *positional,
            *remove,
            resources,
        ),
    }
}

fn clear_function_carrier_stack(builder: &mut FunctionBuilder, slot: StackSlot, pointer: ClifType) {
    let zero = builder.ins().iconst(pointer, 0);
    builder.ins().stack_store(pointer, zero, slot, 0);
    builder
        .ins()
        .stack_store(pointer, zero, slot, pointer.bytes() as i32);
}

fn create_payload_storage(
    builder: &mut FunctionBuilder,
    ty: mir::PayloadEnumType,
    nullable: bool,
    resources: &LoweringResources<'_, '_>,
) -> Value {
    let slot = builder.create_sized_stack_slot(StackSlotData::new(
        StackSlotKind::ExplicitSlot,
        ty.storage_size(nullable),
        ty.align.trailing_zeros() as u8,
    ));
    builder
        .ins()
        .stack_addr(resources.module.target_config().pointer_type(), slot, 0)
}

fn zero_inline_bytes(builder: &mut FunctionBuilder, address: Value, size: u32, pointer: ClifType) {
    let flags = cranelift_codegen::ir::MachMemFlags::trusted();
    let zero_word = builder.ins().iconst(pointer, 0);
    let zero_byte = builder.ins().iconst(types::I8, 0);
    let word_bytes = pointer.bytes();
    let mut offset = 0_u32;
    while size - offset >= word_bytes {
        builder
            .ins()
            .store(flags, zero_word, address, offset as i32);
        offset += word_bytes;
    }
    while offset < size {
        builder
            .ins()
            .store(flags, zero_byte, address, offset as i32);
        offset += 1;
    }
}

fn lower_error_expression(
    builder: &mut FunctionBuilder,
    expression: &mir::ErrorExpression,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<LoweredValue, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let load_pair = |builder: &mut FunctionBuilder, address: Value| LoweredValue::Nullable {
        present: builder.ins().load(
            pointer,
            cranelift_codegen::ir::MachMemFlags::trusted(),
            address,
            0,
        ),
        payload: builder.ins().load(
            pointer,
            cranelift_codegen::ir::MachMemFlags::trusted(),
            address,
            pointer.bytes() as i32,
        ),
    };
    match expression {
        mir::ErrorExpression::Local { local, transfer } => {
            let slot = local_slot(resources.local_slots, *local)?;
            let value = load_lowered_from_stack(builder, mir::Type::Error, slot, pointer);
            if *transfer {
                let zero = builder.ins().iconst(pointer, 0);
                builder.ins().stack_store(pointer, zero, slot, 0);
                builder
                    .ins()
                    .stack_store(pointer, zero, slot, pointer.bytes() as i32);
            }
            Ok(value)
        }
        mir::ErrorExpression::NullableLocalAssumeNonNull { local, transfer } => {
            let slot = local_slot(resources.local_slots, *local)?;
            let value = load_lowered_from_stack(builder, mir::Type::NullableError, slot, pointer);
            if *transfer {
                let zero = builder.ins().iconst(pointer, 0);
                builder.ins().stack_store(pointer, zero, slot, 0);
                builder
                    .ins()
                    .stack_store(pointer, zero, slot, pointer.bytes() as i32);
            }
            Ok(value)
        }
        mir::ErrorExpression::FromClass { object, descriptor } => Ok(LoweredValue::Nullable {
            present: lower_class_expression(builder, object, resources)?,
            payload: lower_error_descriptor_address(builder, *descriptor, resources)?,
        }),
        mir::ErrorExpression::FromNullableClass { object, descriptor } => {
            Ok(LoweredValue::Nullable {
                present: lower_nullable_class_expression(builder, object, resources)?,
                payload: lower_error_descriptor_address(builder, *descriptor, resources)?,
            })
        }
        mir::ErrorExpression::Property {
            object,
            property,
            transfer,
        } => {
            let address = lower_property_address(builder, *object, *property, resources)?;
            let value = load_pair(builder, address);
            if *transfer {
                let zero = builder.ins().iconst(pointer, 0);
                let flags = cranelift_codegen::ir::MachMemFlags::trusted();
                builder.ins().store(flags, zero, address, 0);
                builder
                    .ins()
                    .store(flags, zero, address, pointer.bytes() as i32);
            }
            Ok(value)
        }
        mir::ErrorExpression::Call { function, args, .. } => {
            lower_function_call(builder, *function, args, resources)?
                .ok_or_else(|| malformed_mir("Error call returned void"))
        }
        mir::ErrorExpression::CollectionIndex {
            collection,
            index,
            positional,
            remove,
        } => lower_two_word_collection_index(
            builder,
            *collection,
            index,
            *positional,
            *remove,
            resources,
        ),
        mir::ErrorExpression::MixedPayload { mixed, transfer } => {
            let slot = local_slot(resources.local_slots, *mixed)?;
            let mixed_value =
                load_lowered_from_stack(builder, mir::Type::Mixed, slot, pointer).single()?;
            let word = runtime_call(
                builder,
                MIXED_PAYLOAD,
                &[pointer],
                Some(types::I64),
                &[mixed_value],
                resources,
            )?
            .ok_or_else(|| backend_failure("mixed Error payload read produced no result"))?;
            let address = if pointer == types::I64 {
                word
            } else {
                builder.ins().ireduce(pointer, word)
            };
            let value = load_pair(builder, address);
            if *transfer {
                let zero = builder.ins().iconst(pointer, 0);
                builder.ins().stack_store(pointer, zero, slot, 0);
                let final_claim = runtime_call(
                    builder,
                    MIXED_RELEASE_OWNED,
                    &[pointer],
                    Some(types::I8),
                    &[mixed_value],
                    resources,
                )?
                .ok_or_else(|| backend_failure("mixed Error move released no ownership claim"))?;
                let no_claim = builder.ins().icmp_imm_u(IntCC::Equal, final_claim, 0);
                lower_panic_if_code_at_active_site(builder, no_claim, "P1321", resources)?;
                runtime_call(
                    builder,
                    MIXED_FREE,
                    &[pointer],
                    None,
                    &[mixed_value],
                    resources,
                )?;
            }
            Ok(value)
        }
    }
}

fn lower_nullable_error_expression(
    builder: &mut FunctionBuilder,
    expression: &mir::NullableErrorExpression,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<LoweredValue, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    match expression {
        mir::NullableErrorExpression::Null => {
            let zero = builder.ins().iconst(pointer, 0);
            Ok(LoweredValue::Nullable {
                present: zero,
                payload: zero,
            })
        }
        mir::NullableErrorExpression::Error(value) => {
            lower_error_expression(builder, value, resources)
        }
        mir::NullableErrorExpression::Local { local, transfer } => {
            let slot = local_slot(resources.local_slots, *local)?;
            let value = load_lowered_from_stack(builder, mir::Type::NullableError, slot, pointer);
            if *transfer {
                let zero = builder.ins().iconst(pointer, 0);
                builder.ins().stack_store(pointer, zero, slot, 0);
                builder
                    .ins()
                    .stack_store(pointer, zero, slot, pointer.bytes() as i32);
            }
            Ok(value)
        }
        mir::NullableErrorExpression::Property {
            object,
            property,
            transfer,
        } => {
            let address = lower_property_address(builder, *object, *property, resources)?;
            let value =
                load_lowered_from_address(builder, mir::Type::NullableError, address, pointer);
            if *transfer {
                let zero = builder.ins().iconst(pointer, 0);
                let flags = cranelift_codegen::ir::MachMemFlags::trusted();
                builder.ins().store(flags, zero, address, 0);
                builder
                    .ins()
                    .store(flags, zero, address, pointer.bytes() as i32);
            }
            Ok(value)
        }
        mir::NullableErrorExpression::Call { function, args, .. } => {
            lower_function_call(builder, *function, args, resources)?
                .ok_or_else(|| malformed_mir("nullable Error call returned void"))
        }
        mir::NullableErrorExpression::DictionaryGet {
            collection,
            key,
            access,
        } => lower_nullable_two_word_collection_get(builder, *collection, key, *access, resources),
        mir::NullableErrorExpression::CollectionIndex {
            collection,
            index,
            positional,
            remove,
        } => lower_two_word_collection_index(
            builder,
            *collection,
            index,
            *positional,
            *remove,
            resources,
        ),
    }
}

fn lower_two_word_collection_index(
    builder: &mut FunctionBuilder,
    collection: mir::LocalId,
    index: &mir::Rvalue,
    positional: bool,
    remove: bool,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<LoweredValue, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let local = local_definition(resources.program, resources.function_id, collection)?;
    let mir::Type::Collection(collection_type) = local.ty else {
        return Err(malformed_mir(
            "aggregate collection place uses a non-collection local",
        ));
    };
    let definition = collection_definition(resources.program, collection_type)?.clone();
    let index_type = match (positional, definition.key) {
        (false, Some(key)) => key,
        _ => mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64)),
    };
    let collection_value = lower_collection_pointer(builder, collection, resources)?;
    let index_value = lower_rvalue(builder, index, resources)?.single()?;
    let index_word = value_to_collection_word(builder, index_value, index_type, pointer)?;
    let address = if remove {
        let slot = builder.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot,
            pointer.bytes() * 2,
            pointer.bytes().trailing_zeros() as u8,
        ));
        let address = builder.ins().stack_addr(pointer, slot, 0);
        let index = if pointer == types::I64 {
            index_word
        } else {
            builder.ins().ireduce(pointer, index_word)
        };
        runtime_call(
            builder,
            COLLECTION_AGGREGATE_REMOVE_AT_INTO,
            &[pointer, pointer, pointer, pointer],
            None,
            &[resources.current_frame, collection_value, index, address],
            resources,
        )?;
        address
    } else {
        let positional_value = builder.ins().iconst(types::I8, i64::from(positional));
        let key_kind = builder
            .ins()
            .iconst(types::I8, collection_compare_kind(index_type)?);
        runtime_call(
            builder,
            COLLECTION_AGGREGATE_VALUE_AT,
            &[pointer, pointer, types::I64, types::I8, types::I8],
            Some(pointer),
            &[
                resources.current_frame,
                collection_value,
                index_word,
                positional_value,
                key_kind,
            ],
            resources,
        )?
        .ok_or_else(|| backend_failure("aggregate collection read produced no slot"))?
    };
    Ok(load_lowered_from_address(
        builder,
        definition.value,
        address,
        pointer,
    ))
}

fn lower_nullable_two_word_collection_get(
    builder: &mut FunctionBuilder,
    collection: mir::LocalId,
    key: &mir::Rvalue,
    access: mir::NullableCollectionAccess,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<LoweredValue, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let local = local_definition(resources.program, resources.function_id, collection)?;
    let mir::Type::Collection(collection_type) = local.ty else {
        return Err(malformed_mir(
            "aggregate nullable access uses a non-collection local",
        ));
    };
    let definition = collection_definition(resources.program, collection_type)?.clone();
    let key_type = match access {
        mir::NullableCollectionAccess::Get
        | mir::NullableCollectionAccess::Index
        | mir::NullableCollectionAccess::Remove => definition
            .key
            .ok_or_else(|| malformed_mir("dictionary aggregate access has no key type"))?,
        _ => mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64)),
    };
    let collection = lower_collection_pointer(builder, collection, resources)?;
    let key_value = lower_rvalue(builder, key, resources)?.single()?;
    let key_word = value_to_collection_word(builder, key_value, key_type, pointer)?;
    let raw_slot = builder.create_sized_stack_slot(StackSlotData::new(
        StackSlotKind::ExplicitSlot,
        pointer.bytes() * 2,
        pointer.bytes().trailing_zeros() as u8,
    ));
    let raw = builder.ins().stack_addr(pointer, raw_slot, 0);
    zero_inline_bytes(builder, raw, pointer.bytes() * 2, pointer);
    let found_slot =
        builder.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 1, 0));
    let found = builder.ins().stack_addr(pointer, found_slot, 0);
    let removed_key_slot =
        builder.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 8, 3));
    let removed_key = builder.ins().stack_addr(pointer, removed_key_slot, 0);
    let key_kind = builder
        .ins()
        .iconst(types::I8, collection_compare_kind(key_type)?);
    let access_code = nullable_collection_access_code(access)
        .ok_or_else(|| malformed_mir("aggregate nullable index must use the direct index path"))?;
    let access_value = builder.ins().iconst(types::I8, i64::from(access_code));
    let stored_nullable = builder.ins().iconst(
        types::I8,
        i64::from(matches!(
            definition.value,
            mir::Type::NullableError | mir::Type::NullableFunction(_)
        )),
    );
    runtime_call(
        builder,
        COLLECTION_AGGREGATE_NULLABLE_ACCESS_INTO,
        &[
            pointer,
            types::I64,
            types::I8,
            types::I8,
            types::I8,
            pointer,
            pointer,
            pointer,
        ],
        None,
        &[
            collection,
            key_word,
            key_kind,
            access_value,
            stored_nullable,
            found,
            removed_key,
            raw,
        ],
        resources,
    )?;
    if key_type == mir::Type::String {
        release_string(builder, key_value, resources)?;
        if access == mir::NullableCollectionAccess::Remove {
            let removed_key = builder
                .ins()
                .stack_load(pointer, types::I64, removed_key_slot, 0);
            let removed_key =
                collection_word_to_value(builder, removed_key, mir::Type::String, pointer)?;
            release_string(builder, removed_key, resources)?;
        }
    }
    let result_type = match definition.value {
        mir::Type::Error | mir::Type::NullableError => mir::Type::NullableError,
        mir::Type::Function(function) | mir::Type::NullableFunction(function) => {
            mir::Type::NullableFunction(function)
        }
        ty => {
            return Err(malformed_mir(format!(
                "type {ty} does not use two-word nullable collection access"
            )))
        }
    };
    Ok(load_lowered_from_address(
        builder,
        result_type,
        raw,
        pointer,
    ))
}

fn lower_payload_enum_expression(
    builder: &mut FunctionBuilder,
    expression: &mir::PayloadEnumExpression,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let ty = expression.ty();
    match expression {
        mir::PayloadEnumExpression::Construct { case, fields, .. } => {
            let definition = enum_definition(resources.program, ty.id)?;
            let case_definition = definition
                .cases
                .get(case.index)
                .filter(|candidate| candidate.id == *case)
                .ok_or_else(|| malformed_mir("payload enum construction case does not exist"))?;
            let case_layout = definition
                .layout
                .cases
                .get(case.index)
                .filter(|candidate| candidate.case_id == *case)
                .ok_or_else(|| malformed_mir("payload enum construction layout is missing"))?;
            let address = create_payload_storage(builder, ty, false, resources);
            let pointer = resources.module.target_config().pointer_type();
            zero_inline_bytes(builder, address, ty.size, pointer);
            store_payload_enum_tag(
                builder,
                address,
                definition.layout.tag_width,
                case_definition.tag,
            )?;
            for ((field, field_definition), field_layout) in fields
                .iter()
                .zip(&case_definition.payload)
                .zip(&case_layout.fields)
            {
                let value = lower_rvalue(builder, field, resources)?;
                let field_address = builder
                    .ins()
                    .iadd_imm_u(address, i64::from(field_layout.offset));
                store_lowered_to_address(
                    builder,
                    field_definition.ty,
                    field_address,
                    value,
                    pointer,
                )?;
            }
            Ok(address)
        }
        mir::PayloadEnumExpression::Use { place, mode, .. } => {
            lower_payload_enum_place(builder, place, ty, false, *mode, resources)
        }
        mir::PayloadEnumExpression::Call { function, args, .. } => {
            lower_function_call(builder, *function, args, resources)?
                .ok_or_else(|| malformed_mir("payload enum call returned void"))?
                .single()
        }
        mir::PayloadEnumExpression::Coalesce { left, right, .. } => {
            let left = lower_nullable_payload_enum_expression(builder, left, resources)?;
            let flags = cranelift_codegen::ir::MachMemFlags::trusted();
            let present = builder.ins().load(types::I8, flags, left, 0);
            let zero = builder.ins().iconst(types::I8, 0);
            let is_present = builder.ins().icmp(IntCC::NotEqual, present, zero);
            let left_block = builder.create_block();
            let right_block = builder.create_block();
            let done = builder.create_block();
            let pointer = resources.module.target_config().pointer_type();
            builder.append_block_param(done, pointer);
            builder
                .ins()
                .brif(is_present, left_block, &[], right_block, &[]);
            builder.switch_to_block(left_block);
            let payload = builder
                .ins()
                .iadd_imm_u(left, i64::from(ty.nullable_payload_offset));
            builder.ins().jump(done, &[BlockArg::Value(payload)]);
            builder.switch_to_block(right_block);
            let right = lower_payload_enum_expression(builder, right, resources)?;
            builder.ins().jump(done, &[BlockArg::Value(right)]);
            builder.switch_to_block(done);
            Ok(builder.block_params(done)[0])
        }
    }
}

fn lower_nullable_payload_enum_expression(
    builder: &mut FunctionBuilder,
    expression: &mir::NullablePayloadEnumExpression,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let ty = expression.ty();
    match expression {
        mir::NullablePayloadEnumExpression::Null(_) => {
            let address = create_payload_storage(builder, ty, true, resources);
            zero_inline_bytes(
                builder,
                address,
                ty.nullable_size,
                resources.module.target_config().pointer_type(),
            );
            Ok(address)
        }
        mir::NullablePayloadEnumExpression::Value(value) => {
            let payload = lower_payload_enum_expression(builder, value, resources)?;
            let address = create_payload_storage(builder, ty, true, resources);
            zero_inline_bytes(
                builder,
                address,
                ty.nullable_size,
                resources.module.target_config().pointer_type(),
            );
            let flags = cranelift_codegen::ir::MachMemFlags::trusted();
            let present = builder.ins().iconst(types::I8, 1);
            builder.ins().store(flags, present, address, 0);
            let destination = builder
                .ins()
                .iadd_imm_u(address, i64::from(ty.nullable_payload_offset));
            copy_inline_bytes(
                builder,
                destination,
                payload,
                ty.size,
                resources.module.target_config().pointer_type(),
            );
            Ok(address)
        }
        mir::NullablePayloadEnumExpression::Use { place, mode, .. } => {
            lower_payload_enum_place(builder, place, ty, true, *mode, resources)
        }
        mir::NullablePayloadEnumExpression::Call { function, args, .. } => {
            lower_function_call(builder, *function, args, resources)?
                .ok_or_else(|| malformed_mir("nullable payload enum call returned void"))?
                .single()
        }
        mir::NullablePayloadEnumExpression::CollectionGet {
            collection,
            key,
            access,
            stored_nullable,
            mode,
            ..
        } => lower_nullable_payload_enum_collection_get(
            builder,
            ty,
            *collection,
            key,
            *access,
            *stored_nullable,
            *mode,
            resources,
        ),
        mir::NullablePayloadEnumExpression::Coalesce { left, right, .. } => {
            let left = lower_nullable_payload_enum_expression(builder, left, resources)?;
            let flags = cranelift_codegen::ir::MachMemFlags::trusted();
            let present = builder.ins().load(types::I8, flags, left, 0);
            let zero = builder.ins().iconst(types::I8, 0);
            let is_present = builder.ins().icmp(IntCC::NotEqual, present, zero);
            let left_block = builder.create_block();
            let right_block = builder.create_block();
            let done = builder.create_block();
            let pointer = resources.module.target_config().pointer_type();
            builder.append_block_param(done, pointer);
            builder
                .ins()
                .brif(is_present, left_block, &[], right_block, &[]);
            builder.switch_to_block(left_block);
            builder.ins().jump(done, &[BlockArg::Value(left)]);
            builder.switch_to_block(right_block);
            let right = lower_nullable_payload_enum_expression(builder, right, resources)?;
            builder.ins().jump(done, &[BlockArg::Value(right)]);
            builder.switch_to_block(done);
            Ok(builder.block_params(done)[0])
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn lower_nullable_payload_enum_collection_get(
    builder: &mut FunctionBuilder,
    ty: mir::PayloadEnumType,
    collection: mir::LocalId,
    key: &mir::Rvalue,
    access: mir::NullableCollectionAccess,
    stored_nullable: bool,
    mode: mir::PayloadEnumUseMode,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let local = local_definition(resources.program, resources.function_id, collection)?;
    let mir::Type::Collection(collection_type) = local.ty else {
        return Err(malformed_mir(
            "payload enum nullable access uses a non-collection local",
        ));
    };
    let definition = collection_definition(resources.program, collection_type)?.clone();
    let key_type = match access {
        mir::NullableCollectionAccess::Get
        | mir::NullableCollectionAccess::Index
        | mir::NullableCollectionAccess::Remove => definition
            .key
            .ok_or_else(|| malformed_mir("dictionary access has no key type"))?,
        _ => mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64)),
    };
    let collection = lower_collection_pointer(builder, collection, resources)?;
    let key_value = lower_rvalue(builder, key, resources)?.single()?;
    let key_word = value_to_collection_word(builder, key_value, key_type, pointer)?;
    let raw = create_payload_storage(builder, ty, stored_nullable, resources);
    zero_inline_bytes(builder, raw, ty.storage_size(stored_nullable), pointer);
    let found_slot =
        builder.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 1, 0));
    let found_pointer = builder.ins().stack_addr(pointer, found_slot, 0);
    let removed_key_slot =
        builder.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 8, 3));
    let removed_key_pointer = builder.ins().stack_addr(pointer, removed_key_slot, 0);
    let key_kind = builder
        .ins()
        .iconst(types::I8, collection_compare_kind(key_type)?);
    let access_value = builder.ins().iconst(
        types::I8,
        i64::from(nullable_collection_access_code(access).ok_or_else(|| {
            malformed_mir("aggregate nullable index must use the direct index path")
        })?),
    );
    let stored_nullable_value = builder.ins().iconst(types::I8, i64::from(stored_nullable));
    let _ = runtime_call(
        builder,
        COLLECTION_AGGREGATE_NULLABLE_ACCESS_INTO,
        &[
            pointer,
            types::I64,
            types::I8,
            types::I8,
            types::I8,
            pointer,
            pointer,
            pointer,
        ],
        None,
        &[
            collection,
            key_word,
            key_kind,
            access_value,
            stored_nullable_value,
            found_pointer,
            removed_key_pointer,
            raw,
        ],
        resources,
    )?;
    if key_type == mir::Type::String {
        release_string(builder, key_value, resources)?;
        if access == mir::NullableCollectionAccess::Remove {
            let removed_key = builder
                .ins()
                .stack_load(pointer, types::I64, removed_key_slot, 0);
            let removed_key =
                collection_word_to_value(builder, removed_key, mir::Type::String, pointer)?;
            release_string(builder, removed_key, resources)?;
        }
    }
    let result = create_payload_storage(builder, ty, true, resources);
    zero_inline_bytes(builder, result, ty.nullable_size, pointer);
    if stored_nullable {
        copy_inline_bytes(builder, result, raw, ty.nullable_size, pointer);
    } else {
        let found = builder.ins().stack_load(pointer, types::I8, found_slot, 0);
        builder.ins().store(
            cranelift_codegen::ir::MachMemFlags::trusted(),
            found,
            result,
            0,
        );
        let destination = builder
            .ins()
            .iadd_imm_u(result, i64::from(ty.nullable_payload_offset));
        copy_inline_bytes(builder, destination, raw, ty.size, pointer);
    }
    let mutating = matches!(
        access,
        mir::NullableCollectionAccess::Remove
            | mir::NullableCollectionAccess::Pop
            | mir::NullableCollectionAccess::PopFront
            | mir::NullableCollectionAccess::PopBack
    );
    if !mutating && matches!(mode, mir::PayloadEnumUseMode::Copy) {
        retain_payload_enum_at(builder, result, ty, true, resources)?;
    }
    Ok(result)
}

fn store_payload_enum_tag(
    builder: &mut FunctionBuilder,
    address: Value,
    width: u32,
    tag: u32,
) -> Result<(), BackendError> {
    let ty = match width {
        1 => types::I8,
        2 => types::I16,
        4 => types::I32,
        _ => return Err(malformed_mir("payload enum tag has unsupported width")),
    };
    let value = builder.ins().iconst(ty, i64::from(tag));
    builder.ins().store(
        cranelift_codegen::ir::MachMemFlags::trusted(),
        value,
        address,
        0,
    );
    Ok(())
}

fn lower_payload_enum_place(
    builder: &mut FunctionBuilder,
    place: &mir::PayloadEnumPlace,
    ty: mir::PayloadEnumType,
    nullable: bool,
    mode: mir::PayloadEnumUseMode,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    if let mir::PayloadEnumPlace::MixedPayload { mixed } = place {
        if nullable {
            return Err(malformed_mir(
                "mixed payload projection cannot produce a nullable aggregate",
            ));
        }
        let mixed_slot = local_slot(resources.local_slots, *mixed)?;
        let mixed_value =
            load_lowered_from_stack(builder, mir::Type::Mixed, mixed_slot, pointer).single()?;
        let payload_word = runtime_call(
            builder,
            MIXED_PAYLOAD,
            &[pointer],
            Some(types::I64),
            &[mixed_value],
            resources,
        )?
        .ok_or_else(|| backend_failure("mixed aggregate payload read produced no result"))?;
        let source = collection_word_to_value(
            builder,
            payload_word,
            mir::Type::Class(crate::class_layout::ClassId(0)),
            pointer,
        )?;
        if matches!(mode, mir::PayloadEnumUseMode::Borrow) {
            return Ok(source);
        }
        let destination = create_payload_storage(builder, ty, false, resources);
        copy_inline_bytes(builder, destination, source, ty.size, pointer);
        if matches!(mode, mir::PayloadEnumUseMode::Copy) {
            retain_payload_enum_at(builder, destination, ty, false, resources)?;
            return Ok(destination);
        }
        let zero = builder.ins().iconst(pointer, 0);
        builder.ins().stack_store(pointer, zero, mixed_slot, 0);
        let final_claim = runtime_call(
            builder,
            MIXED_RELEASE_OWNED,
            &[pointer],
            Some(types::I8),
            &[mixed_value],
            resources,
        )?
        .ok_or_else(|| backend_failure("mixed payload move released no ownership claim"))?;
        let no_claim = builder.ins().icmp_imm_u(IntCC::Equal, final_claim, 0);
        lower_panic_if_code_at_active_site(builder, no_claim, "P1321", resources)?;
        runtime_call(
            builder,
            MIXED_FREE,
            &[pointer],
            None,
            &[mixed_value],
            resources,
        )?;
        return Ok(destination);
    }
    let (source, narrowed_nullable_source) = match place {
        mir::PayloadEnumPlace::Local(local) => (
            builder
                .ins()
                .stack_addr(pointer, local_slot(resources.local_slots, *local)?, 0),
            None,
        ),
        mir::PayloadEnumPlace::NullableLocalAssumeNonNull(local) => {
            let storage =
                builder
                    .ins()
                    .stack_addr(pointer, local_slot(resources.local_slots, *local)?, 0);
            (
                builder
                    .ins()
                    .iadd_imm_u(storage, i64::from(ty.nullable_payload_offset)),
                Some(storage),
            )
        }
        mir::PayloadEnumPlace::Static(id) => (lower_static_address(builder, *id, resources)?, None),
        mir::PayloadEnumPlace::Property { object, property } => (
            lower_property_address(builder, *object, *property, resources)?,
            None,
        ),
        mir::PayloadEnumPlace::CollectionIndex {
            collection,
            index,
            positional,
            remove,
        } => {
            let collection_local =
                local_definition(resources.program, resources.function_id, *collection)?;
            let mir::Type::Collection(collection_type) = collection_local.ty else {
                return Err(malformed_mir(
                    "payload enum collection place uses a non-collection local",
                ));
            };
            let definition = collection_definition(resources.program, collection_type)?;
            let collection_value = lower_collection_pointer(builder, *collection, resources)?;
            let index_value = lower_rvalue(builder, index, resources)?.single()?;
            let index_type = match (*positional, definition.key) {
                (false, Some(key)) => key,
                _ => mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64)),
            };
            let index_word = value_to_collection_word(builder, index_value, index_type, pointer)?;
            if *remove {
                let destination = create_payload_storage(builder, ty, nullable, resources);
                let index = if pointer == types::I64 {
                    index_word
                } else {
                    builder.ins().ireduce(pointer, index_word)
                };
                let _ = runtime_call(
                    builder,
                    COLLECTION_AGGREGATE_REMOVE_AT_INTO,
                    &[pointer, pointer, pointer, pointer],
                    None,
                    &[
                        resources.current_frame,
                        collection_value,
                        index,
                        destination,
                    ],
                    resources,
                )?;
                return Ok(destination);
            }
            if matches!(mode, mir::PayloadEnumUseMode::Move) {
                return Err(malformed_mir(
                    "payload enum collection move requires a removing operation",
                ));
            }
            let positional_value = builder.ins().iconst(types::I8, i64::from(*positional));
            let key_kind = builder
                .ins()
                .iconst(types::I8, collection_compare_kind(index_type)?);
            (
                runtime_call(
                    builder,
                    COLLECTION_AGGREGATE_VALUE_AT,
                    &[pointer, pointer, types::I64, types::I8, types::I8],
                    Some(pointer),
                    &[
                        resources.current_frame,
                        collection_value,
                        index_word,
                        positional_value,
                        key_kind,
                    ],
                    resources,
                )?
                .ok_or_else(|| backend_failure("aggregate collection read produced no slot"))?,
                None,
            )
        }
        mir::PayloadEnumPlace::MixedPayload { .. } => unreachable!(),
    };
    if matches!(mode, mir::PayloadEnumUseMode::Borrow) {
        return Ok(source);
    }
    let destination = create_payload_storage(builder, ty, nullable, resources);
    copy_inline_bytes(
        builder,
        destination,
        source,
        ty.storage_size(nullable),
        pointer,
    );
    if matches!(mode, mir::PayloadEnumUseMode::Copy) {
        retain_payload_enum_at(builder, destination, ty, nullable, resources)?;
    } else if let Some(storage) = narrowed_nullable_source {
        zero_inline_bytes(builder, storage, ty.storage_size(true), pointer);
    } else {
        zero_inline_bytes(builder, source, ty.storage_size(nullable), pointer);
    }
    Ok(destination)
}

fn retain_payload_enum_at(
    builder: &mut FunctionBuilder,
    address: Value,
    ty: mir::PayloadEnumType,
    nullable: bool,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    if nullable {
        let flags = cranelift_codegen::ir::MachMemFlags::trusted();
        let present = builder.ins().load(types::I8, flags, address, 0);
        let zero = builder.ins().iconst(types::I8, 0);
        let is_present = builder.ins().icmp(IntCC::NotEqual, present, zero);
        let some = builder.create_block();
        let done = builder.create_block();
        builder.ins().brif(is_present, some, &[], done, &[]);
        builder.switch_to_block(some);
        let payload = builder
            .ins()
            .iadd_imm_u(address, i64::from(ty.nullable_payload_offset));
        retain_payload_enum_at(builder, payload, ty, false, resources)?;
        builder.ins().jump(done, &[]);
        builder.switch_to_block(done);
        return Ok(());
    }
    if !ty.capabilities.needs_drop {
        return Ok(());
    }
    let definition = enum_definition(resources.program, ty.id)?.clone();
    lower_payload_case_dispatch(
        builder,
        address,
        &definition,
        resources,
        |builder, field, field_address, resources| match field.ty {
            mir::Type::String => {
                let pointer = resources.module.target_config().pointer_type();
                let value = builder.ins().load(
                    pointer,
                    cranelift_codegen::ir::MachMemFlags::trusted(),
                    field_address,
                    0,
                );
                let retained = retain_string(builder, value, resources)?;
                builder.ins().store(
                    cranelift_codegen::ir::MachMemFlags::trusted(),
                    retained,
                    field_address,
                    0,
                );
                Ok(())
            }
            mir::Type::PayloadEnum(nested) => {
                retain_payload_enum_at(builder, field_address, nested, false, resources)
            }
            mir::Type::NullablePayloadEnum(nested) => {
                retain_payload_enum_at(builder, field_address, nested, true, resources)
            }
            _ => Ok(()),
        },
    )
}

fn lower_drop_payload_enum_at(
    builder: &mut FunctionBuilder,
    address: Value,
    ty: mir::PayloadEnumType,
    nullable: bool,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    if nullable {
        let flags = cranelift_codegen::ir::MachMemFlags::trusted();
        let present = builder.ins().load(types::I8, flags, address, 0);
        let zero = builder.ins().iconst(types::I8, 0);
        let is_present = builder.ins().icmp(IntCC::NotEqual, present, zero);
        let some = builder.create_block();
        let done = builder.create_block();
        builder.ins().brif(is_present, some, &[], done, &[]);
        builder.switch_to_block(some);
        let payload = builder
            .ins()
            .iadd_imm_u(address, i64::from(ty.nullable_payload_offset));
        lower_drop_payload_enum_at(builder, payload, ty, false, resources)?;
        builder.ins().jump(done, &[]);
        builder.switch_to_block(done);
        return Ok(());
    }
    if !ty.capabilities.needs_drop {
        return Ok(());
    }
    let definition = enum_definition(resources.program, ty.id)?.clone();
    lower_payload_case_dispatch_reverse(
        builder,
        address,
        &definition,
        resources,
        |builder, field, field_address, resources| {
            lower_drop_value_at_address(builder, field.ty, field_address, resources)
        },
    )
}

fn lower_payload_case_dispatch(
    builder: &mut FunctionBuilder,
    address: Value,
    definition: &mir::EnumDefinition,
    resources: &mut LoweringResources<'_, '_>,
    mut field_action: impl FnMut(
        &mut FunctionBuilder,
        &mir::EnumPayloadDefinition,
        Value,
        &mut LoweringResources<'_, '_>,
    ) -> Result<(), BackendError>,
) -> Result<(), BackendError> {
    lower_payload_case_dispatch_ordered(
        builder,
        address,
        definition,
        resources,
        false,
        &mut field_action,
    )
}

fn lower_payload_case_dispatch_reverse(
    builder: &mut FunctionBuilder,
    address: Value,
    definition: &mir::EnumDefinition,
    resources: &mut LoweringResources<'_, '_>,
    mut field_action: impl FnMut(
        &mut FunctionBuilder,
        &mir::EnumPayloadDefinition,
        Value,
        &mut LoweringResources<'_, '_>,
    ) -> Result<(), BackendError>,
) -> Result<(), BackendError> {
    lower_payload_case_dispatch_ordered(
        builder,
        address,
        definition,
        resources,
        true,
        &mut field_action,
    )
}

fn lower_payload_case_dispatch_ordered(
    builder: &mut FunctionBuilder,
    address: Value,
    definition: &mir::EnumDefinition,
    resources: &mut LoweringResources<'_, '_>,
    reverse_fields: bool,
    field_action: &mut impl FnMut(
        &mut FunctionBuilder,
        &mir::EnumPayloadDefinition,
        Value,
        &mut LoweringResources<'_, '_>,
    ) -> Result<(), BackendError>,
) -> Result<(), BackendError> {
    let tag_type = match definition.layout.tag_width {
        1 => types::I8,
        2 => types::I16,
        4 => types::I32,
        _ => return Err(malformed_mir("payload enum tag has unsupported width")),
    };
    let tag = builder.ins().load(
        tag_type,
        cranelift_codegen::ir::MachMemFlags::trusted(),
        address,
        definition.layout.tag_offset as i32,
    );
    let done = builder.create_block();
    for (case, layout) in definition.cases.iter().zip(&definition.layout.cases) {
        let case_block = builder.create_block();
        let next = builder.create_block();
        let expected = builder.ins().iconst(tag_type, i64::from(case.tag));
        let matches = builder.ins().icmp(IntCC::Equal, tag, expected);
        builder.ins().brif(matches, case_block, &[], next, &[]);
        builder.switch_to_block(case_block);
        if reverse_fields {
            for (field, layout) in case.payload.iter().zip(&layout.fields).rev() {
                let field_address = builder.ins().iadd_imm_u(address, i64::from(layout.offset));
                field_action(builder, field, field_address, resources)?;
            }
        } else {
            for (field, layout) in case.payload.iter().zip(&layout.fields) {
                let field_address = builder.ins().iadd_imm_u(address, i64::from(layout.offset));
                field_action(builder, field, field_address, resources)?;
            }
        }
        builder.ins().jump(done, &[]);
        builder.switch_to_block(next);
    }
    builder
        .ins()
        .trap(TrapCode::unwrap_user(RUNTIME_RETURNED_TRAP));
    builder.switch_to_block(done);
    Ok(())
}

fn lower_payload_enum_equal_value(
    builder: &mut FunctionBuilder,
    left: Value,
    right: Value,
    ty: mir::PayloadEnumType,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let equal = builder.create_block();
    let not_equal = builder.create_block();
    let done = builder.create_block();
    builder.append_block_param(done, types::I8);
    lower_payload_enum_equal_to_branch(builder, left, right, ty, equal, not_equal, resources)?;
    builder.switch_to_block(equal);
    let yes = builder.ins().iconst(types::I8, 1);
    builder.ins().jump(done, &[BlockArg::Value(yes)]);
    builder.switch_to_block(not_equal);
    let no = builder.ins().iconst(types::I8, 0);
    builder.ins().jump(done, &[BlockArg::Value(no)]);
    builder.switch_to_block(done);
    Ok(builder.block_params(done)[0])
}

fn lower_nullable_payload_enum_equal_value(
    builder: &mut FunctionBuilder,
    left: Value,
    right: Value,
    ty: mir::PayloadEnumType,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let flags = cranelift_codegen::ir::MachMemFlags::trusted();
    let left_present = builder.ins().load(types::I8, flags, left, 0);
    let right_present = builder.ins().load(types::I8, flags, right, 0);
    let equal = builder.create_block();
    let not_equal = builder.create_block();
    let both_present = builder.create_block();
    let same_presence = builder.create_block();
    let done = builder.create_block();
    builder.append_block_param(done, types::I8);

    let presence_equal = builder
        .ins()
        .icmp(IntCC::Equal, left_present, right_present);
    builder
        .ins()
        .brif(presence_equal, same_presence, &[], not_equal, &[]);
    builder.switch_to_block(same_presence);
    let zero = builder.ins().iconst(types::I8, 0);
    let present = builder.ins().icmp(IntCC::NotEqual, left_present, zero);
    builder.ins().brif(present, both_present, &[], equal, &[]);
    builder.switch_to_block(both_present);
    let left_payload = builder
        .ins()
        .iadd_imm_u(left, i64::from(ty.nullable_payload_offset));
    let right_payload = builder
        .ins()
        .iadd_imm_u(right, i64::from(ty.nullable_payload_offset));
    lower_payload_enum_equal_to_branch(
        builder,
        left_payload,
        right_payload,
        ty,
        equal,
        not_equal,
        resources,
    )?;

    builder.switch_to_block(equal);
    let yes = builder.ins().iconst(types::I8, 1);
    builder.ins().jump(done, &[BlockArg::Value(yes)]);
    builder.switch_to_block(not_equal);
    let no = builder.ins().iconst(types::I8, 0);
    builder.ins().jump(done, &[BlockArg::Value(no)]);
    builder.switch_to_block(done);
    Ok(builder.block_params(done)[0])
}

fn lower_payload_enum_equal_to_branch(
    builder: &mut FunctionBuilder,
    left: Value,
    right: Value,
    ty: mir::PayloadEnumType,
    equal: Block,
    not_equal: Block,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let definition = enum_definition(resources.program, ty.id)?.clone();
    let tag_type = match definition.layout.tag_width {
        1 => types::I8,
        2 => types::I16,
        4 => types::I32,
        _ => return Err(malformed_mir("payload enum tag has unsupported width")),
    };
    let flags = cranelift_codegen::ir::MachMemFlags::trusted();
    let left_tag = builder
        .ins()
        .load(tag_type, flags, left, definition.layout.tag_offset as i32);
    let right_tag = builder
        .ins()
        .load(tag_type, flags, right, definition.layout.tag_offset as i32);
    let dispatch = builder.create_block();
    let tags_equal = builder.ins().icmp(IntCC::Equal, left_tag, right_tag);
    builder
        .ins()
        .brif(tags_equal, dispatch, &[], not_equal, &[]);
    builder.switch_to_block(dispatch);

    for (case, layout) in definition.cases.iter().zip(&definition.layout.cases) {
        let case_block = builder.create_block();
        let next_case = builder.create_block();
        let expected = builder.ins().iconst(tag_type, i64::from(case.tag));
        let matches = builder.ins().icmp(IntCC::Equal, left_tag, expected);
        builder.ins().brif(matches, case_block, &[], next_case, &[]);
        builder.switch_to_block(case_block);
        if case.payload.is_empty() {
            builder.ins().jump(equal, &[]);
        } else {
            for (index, (field, field_layout)) in
                case.payload.iter().zip(&layout.fields).enumerate()
            {
                let left_field = builder
                    .ins()
                    .iadd_imm_u(left, i64::from(field_layout.offset));
                let right_field = builder
                    .ins()
                    .iadd_imm_u(right, i64::from(field_layout.offset));
                let field_equal = lower_value_at_address_equal(
                    builder,
                    left_field,
                    right_field,
                    field.ty,
                    resources,
                )?;
                if index + 1 == case.payload.len() {
                    builder.ins().brif(field_equal, equal, &[], not_equal, &[]);
                } else {
                    let next_field = builder.create_block();
                    builder
                        .ins()
                        .brif(field_equal, next_field, &[], not_equal, &[]);
                    builder.switch_to_block(next_field);
                }
            }
        }
        builder.switch_to_block(next_case);
    }
    builder
        .ins()
        .trap(TrapCode::unwrap_user(RUNTIME_RETURNED_TRAP));
    Ok(())
}

fn lower_value_at_address_equal(
    builder: &mut FunctionBuilder,
    left: Value,
    right: Value,
    ty: mir::Type,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let flags = cranelift_codegen::ir::MachMemFlags::trusted();
    match ty {
        mir::Type::Scalar(scalar) => {
            let left = builder.ins().load(clif_scalar_type(scalar), flags, left, 0);
            let right = builder
                .ins()
                .load(clif_scalar_type(scalar), flags, right, 0);
            Ok(match scalar {
                mir::ScalarType::Float(_) => builder.ins().fcmp(FloatCC::Equal, left, right),
                _ => builder.ins().icmp(IntCC::Equal, left, right),
            })
        }
        mir::Type::NullableScalar(scalar) => {
            let left = load_lowered_from_address(builder, ty, left, pointer);
            let right = load_lowered_from_address(builder, ty, right, pointer);
            let (left_present, left_value) = left.nullable()?;
            let (right_present, right_value) = right.nullable()?;
            let presence_equal = builder
                .ins()
                .icmp(IntCC::Equal, left_present, right_present);
            let payload_equal = match scalar {
                mir::ScalarType::Float(_) => {
                    builder.ins().fcmp(FloatCC::Equal, left_value, right_value)
                }
                _ => builder.ins().icmp(IntCC::Equal, left_value, right_value),
            };
            let zero = builder.ins().iconst(pointer, 0);
            let absent = builder.ins().icmp(IntCC::Equal, left_present, zero);
            let absent_or_payload_equal = builder.ins().bor(absent, payload_equal);
            Ok(builder.ins().band(presence_equal, absent_or_payload_equal))
        }
        mir::Type::String => {
            let left = builder.ins().load(pointer, flags, left, 0);
            let right = builder.ins().load(pointer, flags, right, 0);
            let compared = runtime_call(
                builder,
                STRING_COMPARE,
                &[pointer, pointer],
                Some(types::I32),
                &[left, right],
                resources,
            )?
            .ok_or_else(|| backend_failure("string comparison produced no result"))?;
            let zero = builder.ins().iconst(types::I32, 0);
            Ok(builder.ins().icmp(IntCC::Equal, compared, zero))
        }
        mir::Type::NullableString => {
            let left = load_lowered_from_address(builder, ty, left, pointer)
                .nullable()?
                .1;
            let right = load_lowered_from_address(builder, ty, right, pointer)
                .nullable()?
                .1;
            runtime_call(
                builder,
                NULLABLE_STRING_EQUAL,
                &[pointer, pointer],
                Some(types::I8),
                &[left, right],
                resources,
            )?
            .ok_or_else(|| backend_failure("nullable-string comparison produced no result"))
        }
        mir::Type::Class(_) | mir::Type::NullableClass(_) => {
            let left = builder.ins().load(pointer, flags, left, 0);
            let right = builder.ins().load(pointer, flags, right, 0);
            Ok(builder.ins().icmp(IntCC::Equal, left, right))
        }
        mir::Type::Collection(collection) => {
            if collection_definition(resources.program, collection)?.kind
                != mir::CollectionKind::Bytes
            {
                return Err(malformed_mir(
                    "payload enum field uses collection equality without Bytes semantics",
                ));
            }
            let left = builder.ins().load(pointer, flags, left, 0);
            let right = builder.ins().load(pointer, flags, right, 0);
            runtime_call(
                builder,
                BYTES_EQUAL,
                &[pointer, pointer],
                Some(types::I8),
                &[left, right],
                resources,
            )?
            .ok_or_else(|| backend_failure("Bytes equality produced no result"))
        }
        mir::Type::NullableCollection(collection) => {
            if collection_definition(resources.program, collection)?.kind
                != mir::CollectionKind::Bytes
            {
                return Err(malformed_mir(
                    "nullable payload enum field uses collection equality without Bytes semantics",
                ));
            }
            let left = builder.ins().load(pointer, flags, left, 0);
            let right = builder.ins().load(pointer, flags, right, 0);
            lower_nullable_bytes_equal_value(builder, left, right, resources)
        }
        mir::Type::PayloadEnum(payload) => {
            lower_payload_enum_equal_value(builder, left, right, payload, resources)
        }
        mir::Type::NullablePayloadEnum(payload) => {
            lower_nullable_payload_enum_equal_value(builder, left, right, payload, resources)
        }
        _ => Err(malformed_mir(format!(
            "payload enum field type {ty} has no equality lowering"
        ))),
    }
}

fn lower_nullable_bytes_equal_value(
    builder: &mut FunctionBuilder,
    left: Value,
    right: Value,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let both_non_null = builder.create_block();
    let compare_identity = builder.create_block();
    let done = builder.create_block();
    builder.append_block_param(done, types::I8);
    let zero = builder.ins().iconst(pointer, 0);
    let left_present = builder.ins().icmp(IntCC::NotEqual, left, zero);
    let right_present = builder.ins().icmp(IntCC::NotEqual, right, zero);
    let both_present = builder.ins().band(left_present, right_present);
    builder
        .ins()
        .brif(both_present, both_non_null, &[], compare_identity, &[]);
    builder.switch_to_block(both_non_null);
    let equal = runtime_call(
        builder,
        BYTES_EQUAL,
        &[pointer, pointer],
        Some(types::I8),
        &[left, right],
        resources,
    )?
    .ok_or_else(|| backend_failure("Bytes equality produced no result"))?;
    builder.ins().jump(done, &[BlockArg::Value(equal)]);
    builder.switch_to_block(compare_identity);
    let equal = builder.ins().icmp(IntCC::Equal, left, right);
    builder.ins().jump(done, &[BlockArg::Value(equal)]);
    builder.switch_to_block(done);
    Ok(builder.block_params(done)[0])
}

fn lower_drop_value_at_address(
    builder: &mut FunctionBuilder,
    ty: mir::Type,
    address: Value,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let flags = cranelift_codegen::ir::MachMemFlags::trusted();
    match ty {
        mir::Type::Error | mir::Type::NullableError => {
            let value = load_lowered_from_address(builder, ty, address, pointer);
            lower_drop_error_value(builder, value, resources)
        }
        mir::Type::String | mir::Type::NullableString => {
            let value = if ty == mir::Type::NullableString {
                load_lowered_from_address(builder, ty, address, pointer)
                    .nullable()?
                    .1
            } else {
                builder.ins().load(pointer, flags, address, 0)
            };
            release_string(builder, value, resources)
        }
        mir::Type::Class(class) | mir::Type::NullableClass(class) => {
            let value = builder.ins().load(pointer, flags, address, 0);
            lower_drop_class_value_checked(builder, value, class, resources)
        }
        mir::Type::Collection(collection) | mir::Type::NullableCollection(collection) => {
            let value = builder.ins().load(pointer, flags, address, 0);
            lower_drop_collection_value(builder, value, collection, resources)
        }
        mir::Type::Mixed | mir::Type::NullableMixed => {
            let value = builder.ins().load(pointer, flags, address, 0);
            lower_drop_mixed_value(builder, value, resources)
        }
        mir::Type::SharedReference(_) | mir::Type::NullableSharedReference(_) => {
            let value = builder.ins().load(pointer, flags, address, 0);
            lower_drop_shared_value(builder, value, false, resources)
        }
        mir::Type::WeakReference(_) | mir::Type::NullableWeakReference(_) => {
            let value = builder.ins().load(pointer, flags, address, 0);
            lower_drop_shared_value(builder, value, true, resources)
        }
        mir::Type::PayloadEnum(payload) => {
            lower_drop_payload_enum_at(builder, address, payload, false, resources)
        }
        mir::Type::NullablePayloadEnum(payload) => {
            lower_drop_payload_enum_at(builder, address, payload, true, resources)
        }
        mir::Type::WritableSharedReference(_)
        | mir::Type::NullableWritableSharedReference(_)
        | mir::Type::WritableWeakReference(_)
        | mir::Type::NullableWritableWeakReference(_)
        | mir::Type::ReadonlySharedReferenceAccess(_)
        | mir::Type::WritableSharedReferenceAccess(_)
        | mir::Type::NullableReadonlySharedReferenceAccess(_)
        | mir::Type::NullableWritableSharedReferenceAccess(_) => {
            let value = builder.ins().load(pointer, flags, address, 0);
            let symbol = match ty {
                mir::Type::WritableSharedReference(_)
                | mir::Type::NullableWritableSharedReference(_) => WRITABLE_SHARED_RELEASE,
                mir::Type::WritableWeakReference(_)
                | mir::Type::NullableWritableWeakReference(_) => WRITABLE_SHARED_RELEASE_WEAK,
                mir::Type::ReadonlySharedReferenceAccess(_)
                | mir::Type::NullableReadonlySharedReferenceAccess(_) => {
                    WRITABLE_SHARED_RELEASE_READONLY_ACCESS
                }
                _ => WRITABLE_SHARED_RELEASE_WRITABLE_ACCESS,
            };
            lower_drop_writable_shared_value(builder, value, symbol, resources)
        }
        mir::Type::Function(_) | mir::Type::NullableFunction(_) => {
            let value = load_lowered_from_address(builder, ty, address, pointer);
            lower_drop_function_carrier(builder, value, resources)
        }
        mir::Type::Scalar(_) | mir::Type::NullableScalar(_) => Ok(()),
        mir::Type::ClosureEnvironment(_) => Err(malformed_mir(
            "closure environment pointer reached ordinary value cleanup",
        )),
    }
}

fn lower_drop_function_carrier(
    builder: &mut FunctionBuilder,
    value: LoweredValue,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let (descriptor, environment) = value.nullable()?;
    let zero = builder.ins().iconst(pointer, 0);
    let present = builder.ins().icmp(IntCC::NotEqual, descriptor, zero);
    let has_environment = builder.ins().icmp(IntCC::NotEqual, environment, zero);
    let should_drop = builder.ins().band(present, has_environment);
    let drop_block = builder.create_block();
    let done = builder.create_block();
    builder.ins().brif(should_drop, drop_block, &[], done, &[]);
    builder.switch_to_block(drop_block);
    let layout = native_closure_abi::descriptor_layout(pointer.bytes());
    let drop_function = builder.ins().load(
        pointer,
        cranelift_codegen::ir::MachMemFlags::trusted(),
        descriptor,
        layout.drop_environment_offset as i32,
    );
    let signature = closure_drop_signature(resources.module);
    let signature = builder.import_signature(signature);
    builder.ins().call_indirect(
        signature,
        drop_function,
        &[resources.current_frame, environment],
    );
    builder.ins().jump(done, &[]);
    builder.switch_to_block(done);
    Ok(())
}

fn closure_drop_signature(module: &mut ObjectModule) -> Signature {
    let pointer = module.target_config().pointer_type();
    let mut signature = module.make_signature();
    signature.params.push(AbiParam::new(pointer));
    signature.params.push(AbiParam::new(pointer));
    signature
}

fn lower_drop_error_value(
    builder: &mut FunctionBuilder,
    value: LoweredValue,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let (object, descriptor) = value.nullable()?;
    let zero = builder.ins().iconst(pointer, 0);
    let present = builder.ins().icmp(IntCC::NotEqual, object, zero);
    let drop_block = builder.create_block();
    let done = builder.create_block();
    builder.ins().brif(present, drop_block, &[], done, &[]);
    builder.switch_to_block(drop_block);
    let flags = cranelift_codegen::ir::MachMemFlags::trusted();
    let drop_function =
        builder
            .ins()
            .load(pointer, flags, descriptor, (pointer.bytes() * 3) as i32);
    let mut signature = resources.module.make_signature();
    signature.params.push(AbiParam::new(pointer));
    signature.params.push(AbiParam::new(pointer));
    let signature = builder.import_signature(signature);
    builder
        .ins()
        .call_indirect(signature, drop_function, &[resources.current_frame, object]);
    builder.ins().jump(done, &[]);
    builder.switch_to_block(done);
    Ok(())
}

fn lower_nullable_collection_parts(
    builder: &mut FunctionBuilder,
    value: &mir::Rvalue,
    ty: mir::Type,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(Value, Value, mir::Type), BackendError> {
    let payload_ty = nullable_payload_type(ty)
        .ok_or_else(|| malformed_mir("collection value is not nullable"))?;
    let pointer = resources.module.target_config().pointer_type();
    let lowered = lower_rvalue(builder, value, resources)?;
    match lowered {
        LoweredValue::Nullable { present, payload } => {
            let zero = builder.ins().iconst(pointer, 0);
            let present = builder.ins().icmp(IntCC::NotEqual, present, zero);
            Ok((present, payload, payload_ty))
        }
        LoweredValue::Single(payload) => {
            let zero = builder.ins().iconst(pointer, 0);
            let present = builder.ins().icmp(IntCC::NotEqual, payload, zero);
            Ok((present, payload, payload_ty))
        }
    }
}

fn collection_definition(
    program: &mir::Program,
    id: mir::CollectionTypeId,
) -> Result<&mir::CollectionType, BackendError> {
    program
        .collection_types
        .get(id.0)
        .filter(|collection| collection.id == id)
        .ok_or_else(|| malformed_mir(format!("collection type#{} does not exist", id.0)))
}

fn collection_compare_kind(ty: mir::Type) -> Result<i64, BackendError> {
    match ty {
        mir::Type::String => Ok(i64::from(COLLECTION_COMPARE_STRING)),
        mir::Type::Scalar(mir::ScalarType::Float(FloatType::Float32)) => {
            Ok(i64::from(COLLECTION_COMPARE_FLOAT32))
        }
        mir::Type::Scalar(mir::ScalarType::Float(FloatType::Float64)) => {
            Ok(i64::from(COLLECTION_COMPARE_FLOAT64))
        }
        mir::Type::Scalar(_)
        | mir::Type::Mixed
        | mir::Type::Class(_)
        | mir::Type::SharedReference(_)
        | mir::Type::WeakReference(_)
        | mir::Type::NullableSharedReference(_)
        | mir::Type::NullableWeakReference(_)
        | mir::Type::WritableSharedReference(_)
        | mir::Type::WritableWeakReference(_)
        | mir::Type::NullableWritableSharedReference(_)
        | mir::Type::NullableWritableWeakReference(_)
        | mir::Type::ReadonlySharedReferenceAccess(_)
        | mir::Type::WritableSharedReferenceAccess(_)
        | mir::Type::NullableReadonlySharedReferenceAccess(_)
        | mir::Type::NullableWritableSharedReferenceAccess(_)
        | mir::Type::Collection(_) => Ok(i64::from(COLLECTION_COMPARE_WORD)),
        mir::Type::Error | mir::Type::PayloadEnum(_) => Err(malformed_mir(
            "payload enum equality requires the aggregate collection path",
        )),
        mir::Type::NullableScalar(_)
        | mir::Type::NullableString
        | mir::Type::NullableMixed
        | mir::Type::NullableClass(_)
        | mir::Type::NullableCollection(_)
        | mir::Type::NullableError
        | mir::Type::NullablePayloadEnum(_) => Err(malformed_mir(
            "nullable collection elements are not supported by Stage 23 Slice 3",
        )),
        mir::Type::Function(_)
        | mir::Type::NullableFunction(_)
        | mir::Type::ClosureEnvironment(_) => Err(malformed_mir(
            "function and closure-environment values do not support collection comparison",
        )),
    }
}

fn value_to_collection_word(
    builder: &mut FunctionBuilder,
    value: Value,
    ty: mir::Type,
    pointer: ClifType,
) -> Result<Value, BackendError> {
    let value_ty = builder.func.dfg.value_type(value);
    Ok(match ty {
        mir::Type::Scalar(mir::ScalarType::Float(FloatType::Float32)) => {
            let bits = builder
                .ins()
                .bitcast(types::I32, MemFlagsData::new(), value);
            builder.ins().uextend(types::I64, bits)
        }
        mir::Type::Scalar(mir::ScalarType::Float(FloatType::Float64)) => {
            builder
                .ins()
                .bitcast(types::I64, MemFlagsData::new(), value)
        }
        mir::Type::Scalar(_) => {
            if value_ty == types::I64 {
                value
            } else {
                builder.ins().uextend(types::I64, value)
            }
        }
        mir::Type::String
        | mir::Type::Mixed
        | mir::Type::Class(_)
        | mir::Type::SharedReference(_)
        | mir::Type::WeakReference(_)
        | mir::Type::NullableSharedReference(_)
        | mir::Type::NullableWeakReference(_)
        | mir::Type::WritableSharedReference(_)
        | mir::Type::WritableWeakReference(_)
        | mir::Type::NullableWritableSharedReference(_)
        | mir::Type::NullableWritableWeakReference(_)
        | mir::Type::ReadonlySharedReferenceAccess(_)
        | mir::Type::WritableSharedReferenceAccess(_)
        | mir::Type::NullableReadonlySharedReferenceAccess(_)
        | mir::Type::NullableWritableSharedReferenceAccess(_)
        | mir::Type::Collection(_) => {
            if pointer == types::I64 {
                value
            } else {
                builder.ins().uextend(types::I64, value)
            }
        }
        mir::Type::NullableScalar(_)
        | mir::Type::NullableString
        | mir::Type::NullableMixed
        | mir::Type::NullableClass(_)
        | mir::Type::NullableCollection(_)
        | mir::Type::Error
        | mir::Type::NullableError
        | mir::Type::PayloadEnum(_)
        | mir::Type::NullablePayloadEnum(_)
        | mir::Type::Function(_)
        | mir::Type::NullableFunction(_)
        | mir::Type::ClosureEnvironment(_) => {
            return Err(malformed_mir(
                "aggregate values cannot use scalar collection word transport",
            ))
        }
    })
}

fn collection_word_to_value(
    builder: &mut FunctionBuilder,
    word: Value,
    ty: mir::Type,
    pointer: ClifType,
) -> Result<Value, BackendError> {
    Ok(match ty {
        mir::Type::Scalar(mir::ScalarType::Integer(integer)) => {
            let target = clif_integer_type(integer);
            if target == types::I64 {
                word
            } else {
                builder.ins().ireduce(target, word)
            }
        }
        mir::Type::Scalar(mir::ScalarType::Bool) => builder.ins().ireduce(types::I8, word),
        mir::Type::Scalar(mir::ScalarType::Enum(_)) => builder.ins().ireduce(types::I32, word),
        mir::Type::Scalar(mir::ScalarType::Float(FloatType::Float32)) => {
            let bits = builder.ins().ireduce(types::I32, word);
            builder.ins().bitcast(types::F32, MemFlagsData::new(), bits)
        }
        mir::Type::Scalar(mir::ScalarType::Float(FloatType::Float64)) => {
            builder.ins().bitcast(types::F64, MemFlagsData::new(), word)
        }
        mir::Type::String
        | mir::Type::Mixed
        | mir::Type::Class(_)
        | mir::Type::SharedReference(_)
        | mir::Type::WeakReference(_)
        | mir::Type::NullableSharedReference(_)
        | mir::Type::NullableWeakReference(_)
        | mir::Type::WritableSharedReference(_)
        | mir::Type::WritableWeakReference(_)
        | mir::Type::NullableWritableSharedReference(_)
        | mir::Type::NullableWritableWeakReference(_)
        | mir::Type::ReadonlySharedReferenceAccess(_)
        | mir::Type::WritableSharedReferenceAccess(_)
        | mir::Type::NullableReadonlySharedReferenceAccess(_)
        | mir::Type::NullableWritableSharedReferenceAccess(_)
        | mir::Type::Collection(_) => {
            if pointer == types::I64 {
                word
            } else {
                builder.ins().ireduce(pointer, word)
            }
        }
        mir::Type::NullableScalar(_)
        | mir::Type::NullableString
        | mir::Type::NullableMixed
        | mir::Type::NullableClass(_)
        | mir::Type::NullableCollection(_)
        | mir::Type::Error
        | mir::Type::NullableError
        | mir::Type::PayloadEnum(_)
        | mir::Type::NullablePayloadEnum(_)
        | mir::Type::Function(_)
        | mir::Type::NullableFunction(_)
        | mir::Type::ClosureEnvironment(_) => {
            return Err(malformed_mir(
                "aggregate values cannot use scalar collection word transport",
            ))
        }
    })
}

const fn payload_enum_storage(ty: mir::Type) -> Option<(mir::PayloadEnumType, bool)> {
    match ty {
        mir::Type::PayloadEnum(ty) => Some((ty, false)),
        mir::Type::NullablePayloadEnum(ty) => Some((ty, true)),
        _ => None,
    }
}

fn payload_enum_rvalue_is_owned(value: &mir::Rvalue) -> bool {
    value.owned_temporary_payload_enum().is_some()
}

fn lower_payload_enum_collection_search(
    builder: &mut FunctionBuilder,
    collection: Value,
    needle: Value,
    ty: mir::PayloadEnumType,
    nullable: bool,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(Value, Value), BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let length = runtime_call(
        builder,
        COLLECTION_LENGTH,
        &[pointer],
        Some(pointer),
        &[collection],
        resources,
    )?
    .ok_or_else(|| backend_failure("aggregate collection length produced no result"))?;
    let header = builder.create_block();
    let body = builder.create_block();
    let found = builder.create_block();
    let missing = builder.create_block();
    let done = builder.create_block();
    builder.append_block_param(header, pointer);
    builder.append_block_param(found, pointer);
    builder.append_block_param(done, types::I8);
    builder.append_block_param(done, pointer);

    let zero = builder.ins().iconst(pointer, 0);
    builder.ins().jump(header, &[BlockArg::Value(zero)]);
    builder.switch_to_block(header);
    let index = builder.block_params(header)[0];
    let in_bounds = builder.ins().icmp(IntCC::UnsignedLessThan, index, length);
    builder.ins().brif(in_bounds, body, &[], missing, &[]);

    builder.switch_to_block(body);
    let index_word = if pointer == types::I64 {
        index
    } else {
        builder.ins().uextend(types::I64, index)
    };
    let positional = builder.ins().iconst(types::I8, 1);
    let key_kind = builder
        .ins()
        .iconst(types::I8, i64::from(COLLECTION_COMPARE_WORD));
    let candidate = runtime_call(
        builder,
        COLLECTION_AGGREGATE_VALUE_AT,
        &[pointer, pointer, types::I64, types::I8, types::I8],
        Some(pointer),
        &[
            resources.current_frame,
            collection,
            index_word,
            positional,
            key_kind,
        ],
        resources,
    )?
    .ok_or_else(|| backend_failure("aggregate collection search produced no slot"))?;
    let equal = if nullable {
        lower_nullable_payload_enum_equal_value(builder, candidate, needle, ty, resources)?
    } else {
        lower_payload_enum_equal_value(builder, candidate, needle, ty, resources)?
    };
    let next = builder.create_block();
    builder
        .ins()
        .brif(equal, found, &[BlockArg::Value(index)], next, &[]);
    builder.switch_to_block(next);
    let one = builder.ins().iconst(pointer, 1);
    let next_index = builder.ins().iadd(index, one);
    builder.ins().jump(header, &[BlockArg::Value(next_index)]);

    builder.switch_to_block(found);
    let found_index = builder.block_params(found)[0];
    let yes = builder.ins().iconst(types::I8, 1);
    builder
        .ins()
        .jump(done, &[BlockArg::Value(yes), BlockArg::Value(found_index)]);
    builder.switch_to_block(missing);
    let no = builder.ins().iconst(types::I8, 0);
    builder
        .ins()
        .jump(done, &[BlockArg::Value(no), BlockArg::Value(zero)]);
    builder.switch_to_block(done);
    Ok((builder.block_params(done)[0], builder.block_params(done)[1]))
}

fn lower_payload_enum_collection_literal(
    builder: &mut FunctionBuilder,
    definition: &mir::CollectionType,
    entries: &[mir::CollectionEntry],
    ty: mir::PayloadEnumType,
    nullable: bool,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let fixed = definition.kind == mir::CollectionKind::TypedArray;
    let length = builder.ins().iconst(pointer, entries.len() as i64);
    let keyed = builder
        .ins()
        .iconst(types::I8, i64::from(definition.key.is_some()));
    let fixed_value = builder.ins().iconst(types::I8, i64::from(fixed));
    let value_size = builder
        .ins()
        .iconst(pointer, i64::from(ty.storage_size(nullable)));
    let value_alignment = builder.ins().iconst(pointer, i64::from(ty.align));
    let kind = builder.ins().iconst(
        types::I8,
        i64::from(stage26_collection_kind(definition.kind).unwrap_or(0)),
    );
    let comparator = builder.ins().iconst(
        types::I8,
        i64::from(
            definition
                .comparator
                .map(collection_comparator_code)
                .unwrap_or(COLLECTION_COMPARE_WORD),
        ),
    );
    let result = runtime_call(
        builder,
        COLLECTION_AGGREGATE_NEW,
        &[
            pointer,
            pointer,
            types::I8,
            types::I8,
            pointer,
            pointer,
            types::I8,
            types::I8,
        ],
        Some(pointer),
        &[
            resources.current_frame,
            length,
            keyed,
            fixed_value,
            value_size,
            value_alignment,
            kind,
            comparator,
        ],
        resources,
    )?
    .ok_or_else(|| backend_failure("aggregate collection allocation produced no result"))?;
    for (index, entry) in entries.iter().enumerate() {
        let source = lower_rvalue(builder, &entry.value, resources)?.single()?;
        let destination = if let (Some(key_ty), Some(key)) = (definition.key, &entry.key) {
            let key = lower_rvalue(builder, key, resources)?.single()?;
            lower_aggregate_dictionary_write_slot(
                builder, result, key, key_ty, ty, nullable, resources,
            )?
        } else if fixed {
            let index = builder.ins().iconst(types::I64, index as i64);
            let positional = builder.ins().iconst(types::I8, 1);
            let key_kind = builder
                .ins()
                .iconst(types::I8, i64::from(COLLECTION_COMPARE_WORD));
            runtime_call(
                builder,
                COLLECTION_AGGREGATE_VALUE_AT,
                &[pointer, pointer, types::I64, types::I8, types::I8],
                Some(pointer),
                &[resources.current_frame, result, index, positional, key_kind],
                resources,
            )?
            .ok_or_else(|| backend_failure("aggregate array initialization produced no slot"))?
        } else {
            runtime_call(
                builder,
                COLLECTION_AGGREGATE_PUSH_SLOT,
                &[pointer],
                Some(pointer),
                &[result],
                resources,
            )?
            .ok_or_else(|| backend_failure("aggregate collection insertion produced no slot"))?
        };
        copy_inline_bytes(
            builder,
            destination,
            source,
            ty.storage_size(nullable),
            pointer,
        );
    }
    if stage26_collection_kind(definition.kind).is_some() {
        let _ = runtime_call(
            builder,
            COLLECTION_STAGE26_FINALIZE,
            &[pointer],
            None,
            &[result],
            resources,
        )?;
    }
    Ok(result)
}

fn lower_two_word_collection_literal(
    builder: &mut FunctionBuilder,
    definition: &mir::CollectionType,
    entries: &[mir::CollectionEntry],
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let fixed = definition.kind == mir::CollectionKind::TypedArray;
    let length = builder.ins().iconst(pointer, entries.len() as i64);
    let keyed = builder
        .ins()
        .iconst(types::I8, i64::from(definition.key.is_some()));
    let fixed_value = builder.ins().iconst(types::I8, i64::from(fixed));
    let value_size = builder
        .ins()
        .iconst(pointer, i64::from(pointer.bytes() * 2));
    let value_alignment = builder.ins().iconst(pointer, i64::from(pointer.bytes()));
    let kind = builder.ins().iconst(
        types::I8,
        i64::from(stage26_collection_kind(definition.kind).unwrap_or(0)),
    );
    let comparator = builder.ins().iconst(
        types::I8,
        i64::from(
            definition
                .comparator
                .map(collection_comparator_code)
                .unwrap_or(COLLECTION_COMPARE_WORD),
        ),
    );
    let result = runtime_call(
        builder,
        COLLECTION_AGGREGATE_NEW,
        &[
            pointer,
            pointer,
            types::I8,
            types::I8,
            pointer,
            pointer,
            types::I8,
            types::I8,
        ],
        Some(pointer),
        &[
            resources.current_frame,
            length,
            keyed,
            fixed_value,
            value_size,
            value_alignment,
            kind,
            comparator,
        ],
        resources,
    )?
    .ok_or_else(|| backend_failure("aggregate collection allocation produced no result"))?;
    for (index, entry) in entries.iter().enumerate() {
        let value = lower_rvalue(builder, &entry.value, resources)?;
        let destination = if let (Some(key_ty), Some(key)) = (definition.key, &entry.key) {
            let key = lower_rvalue(builder, key, resources)?.single()?;
            lower_two_word_dictionary_write_slot(
                builder,
                result,
                key,
                key_ty,
                definition.value,
                resources,
            )?
        } else if fixed {
            let index = builder.ins().iconst(types::I64, index as i64);
            let positional = builder.ins().iconst(types::I8, 1);
            let key_kind = builder
                .ins()
                .iconst(types::I8, i64::from(COLLECTION_COMPARE_WORD));
            runtime_call(
                builder,
                COLLECTION_AGGREGATE_VALUE_AT,
                &[pointer, pointer, types::I64, types::I8, types::I8],
                Some(pointer),
                &[resources.current_frame, result, index, positional, key_kind],
                resources,
            )?
            .ok_or_else(|| backend_failure("aggregate array initialization produced no slot"))?
        } else {
            runtime_call(
                builder,
                COLLECTION_AGGREGATE_PUSH_SLOT,
                &[pointer],
                Some(pointer),
                &[result],
                resources,
            )?
            .ok_or_else(|| backend_failure("aggregate collection insertion produced no slot"))?
        };
        store_lowered_to_address(builder, definition.value, destination, value, pointer)?;
    }
    if stage26_collection_kind(definition.kind).is_some() {
        runtime_call(
            builder,
            COLLECTION_STAGE26_FINALIZE,
            &[pointer],
            None,
            &[result],
            resources,
        )?;
    }
    Ok(result)
}

fn lower_two_word_dictionary_write_slot(
    builder: &mut FunctionBuilder,
    collection: Value,
    key: Value,
    key_type: mir::Type,
    value_type: mir::Type,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let key_word = value_to_collection_word(builder, key, key_type, pointer)?;
    let key_kind = builder
        .ins()
        .iconst(types::I8, collection_compare_kind(key_type)?);
    let replaced_slot =
        builder.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 1, 0));
    let replaced_pointer = builder.ins().stack_addr(pointer, replaced_slot, 0);
    let destination = runtime_call(
        builder,
        COLLECTION_AGGREGATE_KEYED_SET_SLOT,
        &[pointer, types::I64, types::I8, pointer],
        Some(pointer),
        &[collection, key_word, key_kind, replaced_pointer],
        resources,
    )?
    .ok_or_else(|| backend_failure("aggregate dictionary write produced no slot"))?;
    let replaced = builder
        .ins()
        .stack_load(pointer, types::I8, replaced_slot, 0);
    let zero = builder.ins().iconst(types::I8, 0);
    let has_old = builder.ins().icmp(IntCC::NotEqual, replaced, zero);
    let drop_block = builder.create_block();
    let done = builder.create_block();
    builder.ins().brif(has_old, drop_block, &[], done, &[]);
    builder.switch_to_block(drop_block);
    lower_drop_value_at_address(builder, value_type, destination, resources)?;
    lower_drop_stored_value(builder, key, key_type, resources)?;
    builder.ins().jump(done, &[]);
    builder.switch_to_block(done);
    Ok(destination)
}

fn lower_aggregate_dictionary_write_slot(
    builder: &mut FunctionBuilder,
    collection: Value,
    key: Value,
    key_type: mir::Type,
    value_type: mir::PayloadEnumType,
    nullable_value: bool,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let key_word = value_to_collection_word(builder, key, key_type, pointer)?;
    let key_kind = builder
        .ins()
        .iconst(types::I8, collection_compare_kind(key_type)?);
    let replaced_slot =
        builder.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 1, 0));
    let replaced_pointer = builder.ins().stack_addr(pointer, replaced_slot, 0);
    let destination = runtime_call(
        builder,
        COLLECTION_AGGREGATE_KEYED_SET_SLOT,
        &[pointer, types::I64, types::I8, pointer],
        Some(pointer),
        &[collection, key_word, key_kind, replaced_pointer],
        resources,
    )?
    .ok_or_else(|| backend_failure("aggregate dictionary write produced no slot"))?;
    let replaced = builder
        .ins()
        .stack_load(pointer, types::I8, replaced_slot, 0);
    let zero = builder.ins().iconst(types::I8, 0);
    let has_old = builder.ins().icmp(IntCC::NotEqual, replaced, zero);
    let drop_block = builder.create_block();
    let done = builder.create_block();
    builder.ins().brif(has_old, drop_block, &[], done, &[]);
    builder.switch_to_block(drop_block);
    lower_drop_payload_enum_at(builder, destination, value_type, nullable_value, resources)?;
    lower_drop_stored_value(builder, key, key_type, resources)?;
    builder.ins().jump(done, &[]);
    builder.switch_to_block(done);
    Ok(destination)
}

fn lower_collection_expression(
    builder: &mut FunctionBuilder,
    expression: &mir::CollectionExpression,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    match expression {
        mir::CollectionExpression::StringIntrinsic(call) => {
            lower_string_intrinsic_call(builder, call, resources)?.single()
        }
        mir::CollectionExpression::Local {
            local, transfer, ..
        } => {
            let slot = local_slot(resources.local_slots, *local)?;
            let value = builder.ins().stack_load(pointer, pointer, slot, 0);
            if *transfer {
                let zero = builder.ins().iconst(pointer, 0);
                builder.ins().stack_store(pointer, zero, slot, 0);
            }
            Ok(value)
        }
        mir::CollectionExpression::Literal {
            collection,
            entries,
        } => {
            let definition = collection_definition(resources.program, *collection)?.clone();
            if matches!(
                definition.value,
                mir::Type::Error
                    | mir::Type::NullableError
                    | mir::Type::Function(_)
                    | mir::Type::NullableFunction(_)
            ) {
                return lower_two_word_collection_literal(builder, &definition, entries, resources);
            }
            if let Some((ty, nullable)) = payload_enum_storage(definition.value) {
                return lower_payload_enum_collection_literal(
                    builder,
                    &definition,
                    entries,
                    ty,
                    nullable,
                    resources,
                );
            }
            let fixed = definition.kind == mir::CollectionKind::TypedArray;
            let length = builder.ins().iconst(pointer, entries.len() as i64);
            let keyed = builder
                .ins()
                .iconst(types::I8, i64::from(definition.key.is_some()));
            let value_width = builder.ins().iconst(
                types::I8,
                i64::from(
                    collection_value_width(definition.value, pointer.bytes() as u8).ok_or_else(
                        || {
                            malformed_mir(
                                "nullable collection elements are not supported by Stage 23 Slice 3",
                            )
                        },
                    )?,
                ),
            );
            let result = if let Some(kind) = stage26_collection_kind(definition.kind) {
                let kind = builder.ins().iconst(types::I8, i64::from(kind));
                let comparator = builder.ins().iconst(
                    types::I8,
                    i64::from(
                        definition
                            .comparator
                            .map(collection_comparator_code)
                            .unwrap_or(COLLECTION_COMPARE_WORD),
                    ),
                );
                runtime_call(
                    builder,
                    COLLECTION_STAGE26_NEW,
                    &[pointer, types::I8, types::I8, types::I8, types::I8],
                    Some(pointer),
                    &[length, keyed, value_width, kind, comparator],
                    resources,
                )?
            } else {
                let fixed_value = builder.ins().iconst(types::I8, i64::from(fixed));
                runtime_call(
                    builder,
                    COLLECTION_NEW,
                    &[pointer, types::I8, types::I8, types::I8],
                    Some(pointer),
                    &[length, keyed, fixed_value, value_width],
                    resources,
                )?
            }
            .ok_or_else(|| backend_failure("collection allocation produced no result"))?;
            for (index, entry) in entries.iter().enumerate() {
                if let (Some(key_ty), Some(key)) = (definition.key, &entry.key) {
                    let key_value = lower_rvalue(builder, key, resources)?.single()?;
                    if nullable_payload_type(definition.value).is_some() {
                        let (present, value, payload_ty) = lower_nullable_collection_parts(
                            builder,
                            &entry.value,
                            definition.value,
                            resources,
                        )?;
                        lower_dictionary_set_nullable_value(
                            builder, result, key_value, key_ty, present, value, payload_ty,
                            resources,
                        )?;
                    } else {
                        let value = lower_rvalue(builder, &entry.value, resources)?.single()?;
                        lower_dictionary_set_value(
                            builder,
                            result,
                            key_value,
                            key_ty,
                            value,
                            definition.value,
                            resources,
                        )?;
                    }
                    continue;
                }
                if nullable_payload_type(definition.value).is_some() {
                    let (present, value, payload_ty) = lower_nullable_collection_parts(
                        builder,
                        &entry.value,
                        definition.value,
                        resources,
                    )?;
                    let word = value_to_collection_word(builder, value, payload_ty, pointer)?;
                    if matches!(
                        definition.kind,
                        mir::CollectionKind::Set | mir::CollectionKind::SortedSet
                    ) {
                        let kind = builder
                            .ins()
                            .iconst(types::I8, collection_compare_kind(payload_ty)?);
                        let inserted = runtime_call(
                            builder,
                            COLLECTION_PUSH_UNIQUE,
                            &[pointer, types::I64, types::I8, types::I8],
                            Some(types::I8),
                            &[result, word, present, kind],
                            resources,
                        )?
                        .ok_or_else(|| backend_failure("set insertion produced no result"))?;
                        lower_drop_value_unless(
                            builder,
                            inserted,
                            value,
                            definition.value,
                            resources,
                        )?;
                    } else if fixed {
                        let index = builder.ins().iconst(pointer, index as i64);
                        let previous_present_slot = builder.create_sized_stack_slot(
                            StackSlotData::new(StackSlotKind::ExplicitSlot, 1, 0),
                        );
                        let previous_present_pointer =
                            builder.ins().stack_addr(pointer, previous_present_slot, 0);
                        let _ = runtime_call(
                            builder,
                            COLLECTION_SET_AT_NULLABLE,
                            &[pointer, pointer, pointer, types::I8, types::I64, pointer],
                            Some(types::I64),
                            &[
                                resources.current_frame,
                                result,
                                index,
                                present,
                                word,
                                previous_present_pointer,
                            ],
                            resources,
                        )?;
                    } else {
                        let _ = runtime_call(
                            builder,
                            COLLECTION_PUSH_NULLABLE,
                            &[pointer, types::I8, types::I64],
                            None,
                            &[result, present, word],
                            resources,
                        )?;
                    }
                    continue;
                }
                let value = lower_rvalue(builder, &entry.value, resources)?.single()?;
                if fixed {
                    let value =
                        value_to_collection_word(builder, value, definition.value, pointer)?;
                    let index = builder.ins().iconst(pointer, index as i64);
                    let _ = runtime_call(
                        builder,
                        COLLECTION_SET_AT,
                        &[pointer, pointer, pointer, types::I64],
                        Some(types::I64),
                        &[resources.current_frame, result, index, value],
                        resources,
                    )?;
                } else if matches!(
                    definition.kind,
                    mir::CollectionKind::Set | mir::CollectionKind::SortedSet
                ) {
                    let value =
                        value_to_collection_word(builder, value, definition.value, pointer)?;
                    let value_kind = builder
                        .ins()
                        .iconst(types::I8, collection_compare_kind(definition.value)?);
                    let present = builder.ins().iconst(types::I8, 1);
                    let inserted = runtime_call(
                        builder,
                        COLLECTION_PUSH_UNIQUE,
                        &[pointer, types::I64, types::I8, types::I8],
                        Some(types::I8),
                        &[result, value, present, value_kind],
                        resources,
                    )?
                    .ok_or_else(|| backend_failure("set insertion produced no result"))?;
                    let lowered =
                        collection_word_to_value(builder, value, definition.value, pointer)?;
                    lower_drop_value_unless(
                        builder,
                        inserted,
                        lowered,
                        definition.value,
                        resources,
                    )?;
                } else {
                    let value =
                        value_to_collection_word(builder, value, definition.value, pointer)?;
                    let _ = runtime_call(
                        builder,
                        COLLECTION_PUSH,
                        &[pointer, types::I64],
                        None,
                        &[result, value],
                        resources,
                    )?;
                }
            }
            if stage26_collection_kind(definition.kind).is_some() {
                let _ = runtime_call(
                    builder,
                    COLLECTION_STAGE26_FINALIZE,
                    &[pointer],
                    None,
                    &[result],
                    resources,
                )?;
            }
            Ok(result)
        }
        mir::CollectionExpression::Fill {
            collection,
            value,
            count,
            count_span,
        } => {
            let definition = collection_definition(resources.program, *collection)?.clone();
            let fixed = definition.kind == mir::CollectionKind::TypedArray;
            let value = lower_rvalue(builder, value, resources)?.single()?;
            let count = lower_integer_expression(builder, count, resources)?;
            set_active_panic_site(builder, *count_span, resources);
            let fixed = builder.ins().iconst(types::I8, i64::from(fixed));
            let value_width = builder.ins().iconst(
                types::I8,
                i64::from(
                    collection_value_width(definition.value, pointer.bytes() as u8).ok_or_else(
                        || {
                            malformed_mir(
                                "nullable collection elements are not supported by Stage 23 Slice 3",
                            )
                        },
                    )?,
                ),
            );
            let (name, value_type, argument) = if definition.value == mir::Type::String {
                (COLLECTION_FILL_STRING, pointer, value)
            } else {
                (
                    COLLECTION_FILL_WORD,
                    types::I64,
                    value_to_collection_word(builder, value, definition.value, pointer)?,
                )
            };
            let mut parameter_types = vec![pointer, value_type, types::I64, types::I8];
            let mut arguments = vec![resources.current_frame, argument, count, fixed];
            if name == COLLECTION_FILL_WORD {
                parameter_types.push(types::I8);
                arguments.push(value_width);
            }
            let result = runtime_call(
                builder,
                name,
                &parameter_types,
                Some(pointer),
                &arguments,
                resources,
            )?
            .ok_or_else(|| backend_failure("collection fill allocation produced no result"))?;
            lower_drop_stored_value(builder, value, definition.value, resources)?;
            Ok(result)
        }
        mir::CollectionExpression::Index {
            source,
            index,
            index_span,
            transfer,
            positional,
            ..
        } => {
            set_active_panic_site(builder, *index_span, resources);
            lower_collection_index(builder, *source, index, *transfer, *positional, resources)
        }
        mir::CollectionExpression::Property {
            object, property, ..
        } => {
            let address = lower_property_address(builder, *object, *property, resources)?;
            Ok(builder.ins().load(pointer, MemFlagsData::new(), address, 0))
        }
        mir::CollectionExpression::SharedAccessPayload {
            access, writable, ..
        } => lower_shared_access_payload(builder, *access, *writable, resources),
        mir::CollectionExpression::From {
            collection,
            source,
            transfer,
            algebra,
        } => lower_set_from(
            builder,
            *collection,
            *source,
            *transfer,
            *algebra,
            resources,
        ),
        mir::CollectionExpression::FromBytes { source, .. } => {
            let source = lower_collection_pointer(builder, *source, resources)?;
            runtime_call(
                builder,
                BYTES_TO_COLLECTION,
                &[pointer],
                Some(pointer),
                &[source],
                resources,
            )?
            .ok_or_else(|| backend_failure("Bytes::toArray produced no result"))
        }
        mir::CollectionExpression::BytesFromArray { source, .. } => {
            let source = lower_collection_pointer(builder, *source, resources)?;
            runtime_call(
                builder,
                BYTES_FROM_COLLECTION,
                &[pointer],
                Some(pointer),
                &[source],
                resources,
            )?
            .ok_or_else(|| backend_failure("Bytes::fromArray produced no result"))
        }
        mir::CollectionExpression::ReadFileBytes {
            path, path_span, ..
        } => {
            let path = lower_string_expression(builder, path, resources)?;
            set_active_panic_site(builder, *path_span, resources);
            let result = runtime_call(
                builder,
                READ_FILE_BYTES,
                &[pointer, pointer],
                Some(pointer),
                &[resources.current_frame, path],
                resources,
            )?
            .ok_or_else(|| backend_failure("read_file_bytes produced no result"))?;
            release_string(builder, path, resources)?;
            Ok(result)
        }
        mir::CollectionExpression::ReadStdinBytes { .. } => runtime_call(
            builder,
            READ_STDIN_BYTES,
            &[pointer],
            Some(pointer),
            &[resources.current_frame],
            resources,
        )?
        .ok_or_else(|| backend_failure("read_stdin_bytes produced no result")),
        mir::CollectionExpression::Call { function, args, .. } => {
            lower_function_call(builder, *function, args, resources)?
                .ok_or_else(|| malformed_mir("collection call produced no result"))?
                .single()
        }
    }
}

fn lower_nullable_collection_expression(
    builder: &mut FunctionBuilder,
    expression: &mir::NullableCollectionExpression,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    match expression {
        mir::NullableCollectionExpression::Null(_) => Ok(builder.ins().iconst(pointer, 0)),
        mir::NullableCollectionExpression::Collection(value) => {
            lower_collection_expression(builder, value, resources)
        }
        mir::NullableCollectionExpression::Local {
            local, transfer, ..
        } => {
            let slot = local_slot(resources.local_slots, *local)?;
            let value = builder.ins().stack_load(pointer, pointer, slot, 0);
            if *transfer {
                let zero = builder.ins().iconst(pointer, 0);
                builder.ins().stack_store(pointer, zero, slot, 0);
            }
            Ok(value)
        }
        mir::NullableCollectionExpression::Property {
            object, property, ..
        } => {
            let address = lower_property_address(builder, *object, *property, resources)?;
            Ok(builder.ins().load(pointer, MemFlagsData::new(), address, 0))
        }
        mir::NullableCollectionExpression::Call { function, args, .. } => {
            lower_function_call(builder, *function, args, resources)?
                .ok_or_else(|| malformed_mir("nullable collection call produced no result"))?
                .single()
        }
        mir::NullableCollectionExpression::Coalesce {
            left,
            right,
            transfer,
            collection,
        } => {
            let left_owned = left.owned_temporary_collection().is_some();
            let right_owned = right.owned_temporary_collection().is_some();
            let left = lower_nullable_collection_expression(builder, left, resources)?;
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
            let left_temporary = if left_owned && !transfer { left } else { zero };
            builder.ins().jump(
                done,
                &[BlockArg::Value(left), BlockArg::Value(left_temporary)],
            );
            builder.switch_to_block(right_block);
            let right = lower_nullable_collection_expression(builder, right, resources)?;
            let right_temporary = if right_owned && !transfer {
                right
            } else {
                zero
            };
            builder.ins().jump(
                done,
                &[BlockArg::Value(right), BlockArg::Value(right_temporary)],
            );
            builder.switch_to_block(done);
            if !transfer && (left_owned || right_owned) {
                defer_or_drop_collection_temporary(
                    builder,
                    builder.block_params(done)[1],
                    *collection,
                    resources,
                )?;
            }
            Ok(builder.block_params(done)[0])
        }
    }
}

fn lower_collection_pointer(
    builder: &mut FunctionBuilder,
    local: mir::LocalId,
    resources: &LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    Ok(builder.ins().stack_load(
        pointer,
        pointer,
        local_slot(resources.local_slots, local)?,
        0,
    ))
}

fn lower_collection_index(
    builder: &mut FunctionBuilder,
    collection: mir::LocalId,
    index: &mir::Rvalue,
    remove: bool,
    positional: bool,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let local = local_definition(resources.program, resources.function_id, collection)?;
    let mir::Type::Collection(collection_type) = local.ty else {
        return Err(malformed_mir("collection index uses non-collection local"));
    };
    let definition = collection_definition(resources.program, collection_type)?.clone();
    let collection_value = lower_collection_pointer(builder, collection, resources)?;
    let index_type = definition
        .key
        .unwrap_or(mir::Type::Scalar(mir::ScalarType::Integer(
            IntegerType::Int64,
        )));
    let index_value = lower_rvalue(builder, index, resources)?.single()?;
    if definition.kind == mir::CollectionKind::Bytes {
        if remove {
            return Err(malformed_mir("Bytes indexed reads cannot remove elements"));
        }
        return runtime_call(
            builder,
            BYTES_GET,
            &[pointer, pointer, pointer],
            Some(types::I8),
            &[resources.current_frame, collection_value, index_value],
            resources,
        )?
        .ok_or_else(|| backend_failure("Bytes index read produced no result"));
    }
    let word = if definition.key.is_some() && !positional {
        if remove {
            return Err(malformed_mir(
                "dictionary indexed removal must use Dictionary::remove",
            ));
        }
        let index_word = value_to_collection_word(builder, index_value, index_type, pointer)?;
        let found_slot =
            builder.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 1, 0));
        let found_pointer = builder.ins().stack_addr(pointer, found_slot, 0);
        let key_kind = builder
            .ins()
            .iconst(types::I8, collection_compare_kind(index_type)?);
        let word = runtime_call(
            builder,
            COLLECTION_KEYED_GET,
            &[pointer, types::I64, types::I8, pointer],
            Some(types::I64),
            &[collection_value, index_word, key_kind, found_pointer],
            resources,
        )?
        .ok_or_else(|| backend_failure("dictionary lookup produced no result"))?;
        let found = builder.ins().stack_load(pointer, types::I8, found_slot, 0);
        let zero = builder.ins().iconst(types::I8, 0);
        let missing = builder.ins().icmp(IntCC::Equal, found, zero);
        lower_panic_if_code_at_active_site(builder, missing, "P1312", resources)?;
        if index_type == mir::Type::String {
            release_string(builder, index_value, resources)?;
        }
        word
    } else if remove {
        runtime_call(
            builder,
            COLLECTION_REMOVE_AT,
            &[pointer, pointer, pointer],
            Some(types::I64),
            &[resources.current_frame, collection_value, index_value],
            resources,
        )?
        .ok_or_else(|| backend_failure("collection removal produced no result"))?
    } else {
        runtime_call(
            builder,
            COLLECTION_VALUE_AT,
            &[pointer, pointer, pointer],
            Some(types::I64),
            &[resources.current_frame, collection_value, index_value],
            resources,
        )?
        .ok_or_else(|| backend_failure("collection index produced no result"))?
    };
    collection_word_to_value(builder, word, definition.value, pointer)
}

fn lower_collection_key_at(
    builder: &mut FunctionBuilder,
    collection: mir::LocalId,
    offset: &mir::Rvalue,
    expected: mir::Type,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let local = local_definition(resources.program, resources.function_id, collection)?;
    let mir::Type::Collection(collection_type) = local.ty else {
        return Err(malformed_mir(
            "collection key access uses non-collection local",
        ));
    };
    let definition = collection_definition(resources.program, collection_type)?;
    if definition.key != Some(expected) {
        return Err(malformed_mir("collection key access has another type"));
    }
    let collection = lower_collection_pointer(builder, collection, resources)?;
    let offset = lower_rvalue(builder, offset, resources)?.single()?;
    let word = runtime_call(
        builder,
        COLLECTION_KEY_AT,
        &[pointer, pointer, pointer],
        Some(types::I64),
        &[resources.current_frame, collection, offset],
        resources,
    )?
    .ok_or_else(|| backend_failure("collection key read produced no result"))?;
    collection_word_to_value(builder, word, expected, pointer)
}

fn lower_dictionary_get(
    builder: &mut FunctionBuilder,
    collection: mir::LocalId,
    key: &mir::Rvalue,
    expected: mir::Type,
    access: mir::NullableCollectionAccess,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(Value, Value), BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let local = local_definition(resources.program, resources.function_id, collection)?;
    let mir::Type::Collection(collection_type) = local.ty else {
        return Err(malformed_mir("Dictionary::get uses a non-collection local"));
    };
    let definition = collection_definition(resources.program, collection_type)?.clone();
    if definition.value != expected && nullable_payload_type(definition.value) != Some(expected) {
        return Err(malformed_mir("nullable collection access type mismatch"));
    }
    let key_type = match access {
        mir::NullableCollectionAccess::Get
        | mir::NullableCollectionAccess::Index
        | mir::NullableCollectionAccess::Remove => definition
            .key
            .ok_or_else(|| malformed_mir("dictionary access has no key type"))?,
        mir::NullableCollectionAccess::First
        | mir::NullableCollectionAccess::Last
        | mir::NullableCollectionAccess::Pop
        | mir::NullableCollectionAccess::PopFront
        | mir::NullableCollectionAccess::PopBack
        | mir::NullableCollectionAccess::At => {
            mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64))
        }
    };
    let collection = lower_collection_pointer(builder, collection, resources)?;
    let key_value = lower_rvalue(builder, key, resources)?.single()?;
    let key_word = value_to_collection_word(builder, key_value, key_type, pointer)?;
    let found_slot =
        builder.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 1, 0));
    let found_pointer = builder.ins().stack_addr(pointer, found_slot, 0);
    let removed_key_slot =
        builder.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 8, 3));
    let removed_key_pointer = builder.ins().stack_addr(pointer, removed_key_slot, 0);
    let key_kind = builder
        .ins()
        .iconst(types::I8, collection_compare_kind(key_type)?);
    if access == mir::NullableCollectionAccess::Index {
        let present_slot =
            builder.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 1, 0));
        let present_pointer = builder.ins().stack_addr(pointer, present_slot, 0);
        let word = runtime_call(
            builder,
            COLLECTION_KEYED_GET_NULLABLE,
            &[pointer, types::I64, types::I8, pointer, pointer],
            Some(types::I64),
            &[
                collection,
                key_word,
                key_kind,
                found_pointer,
                present_pointer,
            ],
            resources,
        )?
        .ok_or_else(|| backend_failure("nullable dictionary lookup produced no result"))?;
        let found = builder.ins().stack_load(pointer, types::I8, found_slot, 0);
        let zero = builder.ins().iconst(types::I8, 0);
        let missing = builder.ins().icmp(IntCC::Equal, found, zero);
        lower_panic_if_code_at_active_site(builder, missing, "P1312", resources)?;
        if key_type == mir::Type::String {
            release_string(builder, key_value, resources)?;
        }
        let present = builder
            .ins()
            .stack_load(pointer, types::I8, present_slot, 0);
        let present = builder.ins().uextend(pointer, present);
        let payload = collection_word_to_value(builder, word, expected, pointer)?;
        return Ok((present, payload));
    }
    let access_value = builder.ins().iconst(
        types::I8,
        i64::from(
            nullable_collection_access_code(access)
                .ok_or_else(|| malformed_mir("nullable index must use the direct index path"))?,
        ),
    );
    let word = runtime_call(
        builder,
        COLLECTION_NULLABLE_ACCESS,
        &[pointer, types::I64, types::I8, types::I8, pointer, pointer],
        Some(types::I64),
        &[
            collection,
            key_word,
            key_kind,
            access_value,
            found_pointer,
            removed_key_pointer,
        ],
        resources,
    )?
    .ok_or_else(|| backend_failure("nullable collection access produced no result"))?;
    if key_type == mir::Type::String {
        release_string(builder, key_value, resources)?;
        if access == mir::NullableCollectionAccess::Remove {
            let removed_key = builder
                .ins()
                .stack_load(pointer, types::I64, removed_key_slot, 0);
            let removed_key =
                collection_word_to_value(builder, removed_key, mir::Type::String, pointer)?;
            release_string(builder, removed_key, resources)?;
        }
    }
    let found = builder.ins().stack_load(pointer, types::I8, found_slot, 0);
    let present = builder.ins().uextend(pointer, found);
    let payload = collection_word_to_value(builder, word, expected, pointer)?;
    Ok((present, payload))
}

fn lower_collection_add(
    builder: &mut FunctionBuilder,
    collection: mir::LocalId,
    value: &mir::Rvalue,
    index: Option<&mir::Rvalue>,
    op: mir::CollectionMutationOp,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let local = local_definition(resources.program, resources.function_id, collection)?;
    let mir::Type::Collection(collection_type) = local.ty else {
        return Err(malformed_mir("collection add uses non-collection local"));
    };
    let definition = collection_definition(resources.program, collection_type)?.clone();
    let collection_value = lower_collection_pointer(builder, collection, resources)?;
    let index = if op == mir::CollectionMutationOp::InsertAt {
        Some(
            lower_rvalue(
                builder,
                index.ok_or_else(|| malformed_mir("insertAt has no index"))?,
                resources,
            )?
            .single()?,
        )
    } else {
        None
    };
    if matches!(
        definition.value,
        mir::Type::Error
            | mir::Type::NullableError
            | mir::Type::Function(_)
            | mir::Type::NullableFunction(_)
    ) {
        if op == mir::CollectionMutationOp::Remove {
            return Err(malformed_mir(
                "aggregate remove-by-value requires a collection equality capability",
            ));
        }
        let value = lower_rvalue(builder, value, resources)?;
        let destination = if op == mir::CollectionMutationOp::InsertAt {
            runtime_call(
                builder,
                COLLECTION_AGGREGATE_INSERT_SLOT,
                &[pointer, pointer, pointer],
                Some(pointer),
                &[
                    resources.current_frame,
                    collection_value,
                    index.expect("insertAt index was lowered"),
                ],
                resources,
            )?
            .ok_or_else(|| backend_failure("aggregate collection insertion produced no slot"))?
        } else if op == mir::CollectionMutationOp::PushFront {
            runtime_call(
                builder,
                COLLECTION_AGGREGATE_PUSH_FRONT_SLOT,
                &[pointer],
                Some(pointer),
                &[collection_value],
                resources,
            )?
            .ok_or_else(|| {
                backend_failure("aggregate collection front insertion produced no slot")
            })?
        } else {
            runtime_call(
                builder,
                COLLECTION_AGGREGATE_PUSH_SLOT,
                &[pointer],
                Some(pointer),
                &[collection_value],
                resources,
            )?
            .ok_or_else(|| backend_failure("aggregate collection insertion produced no slot"))?
        };
        store_lowered_to_address(builder, definition.value, destination, value, pointer)?;
        return Ok(());
    }
    if let Some((ty, nullable)) = payload_enum_storage(definition.value) {
        if op == mir::CollectionMutationOp::Remove {
            return Err(malformed_mir(
                "payload enum remove-by-value requires generated enum equality",
            ));
        }
        let source = lower_rvalue(builder, value, resources)?.single()?;
        let destination = if op == mir::CollectionMutationOp::InsertAt {
            runtime_call(
                builder,
                COLLECTION_AGGREGATE_INSERT_SLOT,
                &[pointer, pointer, pointer],
                Some(pointer),
                &[
                    resources.current_frame,
                    collection_value,
                    index.expect("insertAt index was lowered"),
                ],
                resources,
            )?
            .ok_or_else(|| backend_failure("aggregate insertion produced no slot"))?
        } else if op == mir::CollectionMutationOp::PushFront {
            runtime_call(
                builder,
                COLLECTION_AGGREGATE_PUSH_FRONT_SLOT,
                &[pointer],
                Some(pointer),
                &[collection_value],
                resources,
            )?
            .ok_or_else(|| backend_failure("aggregate front insertion produced no slot"))?
        } else {
            runtime_call(
                builder,
                COLLECTION_AGGREGATE_PUSH_SLOT,
                &[pointer],
                Some(pointer),
                &[collection_value],
                resources,
            )?
            .ok_or_else(|| backend_failure("aggregate insertion produced no slot"))?
        };
        copy_inline_bytes(
            builder,
            destination,
            source,
            ty.storage_size(nullable),
            pointer,
        );
        return Ok(());
    }
    if nullable_payload_type(definition.value).is_some()
        && matches!(
            op,
            mir::CollectionMutationOp::Add
                | mir::CollectionMutationOp::InsertAt
                | mir::CollectionMutationOp::PushFront
                | mir::CollectionMutationOp::PushBack
                | mir::CollectionMutationOp::Remove
        )
    {
        let (present, value, payload_ty) =
            lower_nullable_collection_parts(builder, value, definition.value, resources)?;
        let word = value_to_collection_word(builder, value, payload_ty, pointer)?;
        if op == mir::CollectionMutationOp::Remove {
            let removed_slot = builder.create_sized_stack_slot(StackSlotData::new(
                StackSlotKind::ExplicitSlot,
                8,
                3,
            ));
            let removed_pointer = builder.ins().stack_addr(pointer, removed_slot, 0);
            let removed_present_slot = builder.create_sized_stack_slot(StackSlotData::new(
                StackSlotKind::ExplicitSlot,
                1,
                0,
            ));
            let removed_present_pointer =
                builder.ins().stack_addr(pointer, removed_present_slot, 0);
            let kind = builder
                .ins()
                .iconst(types::I8, collection_compare_kind(payload_ty)?);
            let removed = runtime_call(
                builder,
                COLLECTION_REMOVE_VALUE,
                &[pointer, types::I64, types::I8, types::I8, pointer, pointer],
                Some(types::I8),
                &[
                    collection_value,
                    word,
                    present,
                    kind,
                    removed_pointer,
                    removed_present_pointer,
                ],
                resources,
            )?
            .ok_or_else(|| backend_failure("collection removal produced no result"))?;
            let removed_word = builder
                .ins()
                .stack_load(pointer, types::I64, removed_slot, 0);
            let removed_value =
                collection_word_to_value(builder, removed_word, payload_ty, pointer)?;
            let removed_present =
                builder
                    .ins()
                    .stack_load(pointer, types::I8, removed_present_slot, 0);
            let should_drop = builder.ins().band(removed, removed_present);
            lower_drop_value_if(
                builder,
                should_drop,
                removed_value,
                definition.value,
                resources,
            )?;
            lower_drop_stored_value(builder, value, definition.value, resources)?;
            return Ok(());
        }
        if op == mir::CollectionMutationOp::Add
            && matches!(
                definition.kind,
                mir::CollectionKind::Set | mir::CollectionKind::SortedSet
            )
        {
            let kind = builder
                .ins()
                .iconst(types::I8, collection_compare_kind(payload_ty)?);
            let inserted = runtime_call(
                builder,
                COLLECTION_PUSH_UNIQUE,
                &[pointer, types::I64, types::I8, types::I8],
                Some(types::I8),
                &[collection_value, word, present, kind],
                resources,
            )?
            .ok_or_else(|| backend_failure("set insertion produced no result"))?;
            lower_drop_value_unless(builder, inserted, value, definition.value, resources)?;
            return Ok(());
        }
        let (name, parameter_types, arguments) = if op == mir::CollectionMutationOp::InsertAt {
            (
                COLLECTION_INSERT_AT_NULLABLE,
                vec![pointer, pointer, pointer, types::I8, types::I64],
                vec![
                    resources.current_frame,
                    collection_value,
                    index.expect("insertAt index was lowered"),
                    present,
                    word,
                ],
            )
        } else if op == mir::CollectionMutationOp::PushFront {
            (
                COLLECTION_PUSH_FRONT_NULLABLE,
                vec![pointer, types::I8, types::I64],
                vec![collection_value, present, word],
            )
        } else {
            (
                COLLECTION_PUSH_NULLABLE,
                vec![pointer, types::I8, types::I64],
                vec![collection_value, present, word],
            )
        };
        let _ = runtime_call(builder, name, &parameter_types, None, &arguments, resources)?;
        return Ok(());
    }
    let value = lower_rvalue(builder, value, resources)?.single()?;
    let word = value_to_collection_word(builder, value, definition.value, pointer)?;
    if op == mir::CollectionMutationOp::InsertAt {
        let _ = runtime_call(
            builder,
            COLLECTION_INSERT_AT,
            &[pointer, pointer, pointer, types::I64],
            None,
            &[
                resources.current_frame,
                collection_value,
                index.expect("insertAt index was lowered"),
                word,
            ],
            resources,
        )?;
    } else if op == mir::CollectionMutationOp::Remove {
        let removed_slot =
            builder.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 8, 3));
        let removed_pointer = builder.ins().stack_addr(pointer, removed_slot, 0);
        let kind = builder
            .ins()
            .iconst(types::I8, collection_compare_kind(definition.value)?);
        let removed_present_slot =
            builder.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 1, 0));
        let removed_present_pointer = builder.ins().stack_addr(pointer, removed_present_slot, 0);
        let present = builder.ins().iconst(types::I8, 1);
        let removed = runtime_call(
            builder,
            COLLECTION_REMOVE_VALUE,
            &[pointer, types::I64, types::I8, types::I8, pointer, pointer],
            Some(types::I8),
            &[
                collection_value,
                word,
                present,
                kind,
                removed_pointer,
                removed_present_pointer,
            ],
            resources,
        )?
        .ok_or_else(|| backend_failure("set removal produced no result"))?;
        let removed_word = builder
            .ins()
            .stack_load(pointer, types::I64, removed_slot, 0);
        let removed_value =
            collection_word_to_value(builder, removed_word, definition.value, pointer)?;
        lower_drop_value_if(builder, removed, removed_value, definition.value, resources)?;
        lower_drop_stored_value(builder, value, definition.value, resources)?;
    } else if matches!(
        definition.kind,
        mir::CollectionKind::Set | mir::CollectionKind::SortedSet
    ) {
        let kind = builder
            .ins()
            .iconst(types::I8, collection_compare_kind(definition.value)?);
        let present = builder.ins().iconst(types::I8, 1);
        let inserted = runtime_call(
            builder,
            COLLECTION_PUSH_UNIQUE,
            &[pointer, types::I64, types::I8, types::I8],
            Some(types::I8),
            &[collection_value, word, present, kind],
            resources,
        )?
        .ok_or_else(|| backend_failure("set insertion produced no result"))?;
        lower_drop_value_unless(builder, inserted, value, definition.value, resources)?;
    } else if op == mir::CollectionMutationOp::PushFront {
        let _ = runtime_call(
            builder,
            COLLECTION_PUSH_FRONT,
            &[pointer, types::I64],
            None,
            &[collection_value, word],
            resources,
        )?;
    } else {
        let _ = runtime_call(
            builder,
            COLLECTION_PUSH,
            &[pointer, types::I64],
            None,
            &[collection_value, word],
            resources,
        )?;
    }
    Ok(())
}

fn lower_collection_set(
    builder: &mut FunctionBuilder,
    collection: mir::LocalId,
    index: &mir::Rvalue,
    value: &mir::Rvalue,
    positional: bool,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let local = local_definition(resources.program, resources.function_id, collection)?;
    let mir::Type::Collection(collection_type) = local.ty else {
        return Err(malformed_mir("collection write uses non-collection local"));
    };
    let definition = collection_definition(resources.program, collection_type)?.clone();
    let collection_value = lower_collection_pointer(builder, collection, resources)?;
    let index = lower_rvalue(builder, index, resources)?.single()?;
    if matches!(
        definition.value,
        mir::Type::Error
            | mir::Type::NullableError
            | mir::Type::Function(_)
            | mir::Type::NullableFunction(_)
    ) {
        let replacement = lower_rvalue(builder, value, resources)?;
        let destination = if let Some(key_type) = definition.key.filter(|_| !positional) {
            lower_two_word_dictionary_write_slot(
                builder,
                collection_value,
                index,
                key_type,
                definition.value,
                resources,
            )?
        } else {
            let positional = builder.ins().iconst(types::I8, 1);
            let key_kind = builder
                .ins()
                .iconst(types::I8, i64::from(COLLECTION_COMPARE_WORD));
            let index_word = if builder.func.dfg.value_type(index) == types::I64 {
                index
            } else {
                builder.ins().uextend(types::I64, index)
            };
            let destination = runtime_call(
                builder,
                COLLECTION_AGGREGATE_VALUE_AT,
                &[pointer, pointer, types::I64, types::I8, types::I8],
                Some(pointer),
                &[
                    resources.current_frame,
                    collection_value,
                    index_word,
                    positional,
                    key_kind,
                ],
                resources,
            )?
            .ok_or_else(|| backend_failure("aggregate collection write produced no slot"))?;
            lower_drop_value_at_address(builder, definition.value, destination, resources)?;
            destination
        };
        store_lowered_to_address(builder, definition.value, destination, replacement, pointer)?;
        return Ok(());
    }
    if let Some((ty, nullable)) = payload_enum_storage(definition.value) {
        let replacement = lower_rvalue(builder, value, resources)?.single()?;
        let destination = if let Some(key_type) = definition.key.filter(|_| !positional) {
            lower_aggregate_dictionary_write_slot(
                builder,
                collection_value,
                index,
                key_type,
                ty,
                nullable,
                resources,
            )?
        } else {
            let positional = builder.ins().iconst(types::I8, 1);
            let key_kind = builder
                .ins()
                .iconst(types::I8, i64::from(COLLECTION_COMPARE_WORD));
            let index_word = if builder.func.dfg.value_type(index) == types::I64 {
                index
            } else {
                builder.ins().uextend(types::I64, index)
            };
            let destination = runtime_call(
                builder,
                COLLECTION_AGGREGATE_VALUE_AT,
                &[pointer, pointer, types::I64, types::I8, types::I8],
                Some(pointer),
                &[
                    resources.current_frame,
                    collection_value,
                    index_word,
                    positional,
                    key_kind,
                ],
                resources,
            )?
            .ok_or_else(|| backend_failure("aggregate collection write produced no slot"))?;
            lower_drop_payload_enum_at(builder, destination, ty, nullable, resources)?;
            destination
        };
        copy_inline_bytes(
            builder,
            destination,
            replacement,
            ty.storage_size(nullable),
            pointer,
        );
        return Ok(());
    }
    if let Some(payload_ty) = nullable_payload_type(definition.value) {
        let (present, value, actual_payload_ty) =
            lower_nullable_collection_parts(builder, value, definition.value, resources)?;
        debug_assert_eq!(payload_ty, actual_payload_ty);
        if let Some(key_type) = definition.key.filter(|_| !positional) {
            lower_dictionary_set_nullable_value(
                builder,
                collection_value,
                index,
                key_type,
                present,
                value,
                payload_ty,
                resources,
            )?;
        } else {
            let value_word = value_to_collection_word(builder, value, payload_ty, pointer)?;
            let previous_present_slot = builder.create_sized_stack_slot(StackSlotData::new(
                StackSlotKind::ExplicitSlot,
                1,
                0,
            ));
            let previous_present_pointer =
                builder.ins().stack_addr(pointer, previous_present_slot, 0);
            let old_word = runtime_call(
                builder,
                COLLECTION_SET_AT_NULLABLE,
                &[pointer, pointer, pointer, types::I8, types::I64, pointer],
                Some(types::I64),
                &[
                    resources.current_frame,
                    collection_value,
                    index,
                    present,
                    value_word,
                    previous_present_pointer,
                ],
                resources,
            )?
            .ok_or_else(|| backend_failure("nullable collection write produced no result"))?;
            let previous_present =
                builder
                    .ins()
                    .stack_load(pointer, types::I8, previous_present_slot, 0);
            let old_value = collection_word_to_value(builder, old_word, payload_ty, pointer)?;
            lower_drop_value_if(builder, previous_present, old_value, payload_ty, resources)?;
        }
        return Ok(());
    }
    let value = lower_rvalue(builder, value, resources)?.single()?;
    if definition.kind == mir::CollectionKind::Bytes {
        let _ = runtime_call(
            builder,
            BYTES_SET,
            &[pointer, pointer, pointer, types::I8],
            None,
            &[resources.current_frame, collection_value, index, value],
            resources,
        )?;
        return Ok(());
    }
    let value_word = value_to_collection_word(builder, value, definition.value, pointer)?;
    if let Some(key_type) = definition.key.filter(|_| !positional) {
        lower_dictionary_set_value(
            builder,
            collection_value,
            index,
            key_type,
            value,
            definition.value,
            resources,
        )?;
    } else {
        let old_word = runtime_call(
            builder,
            COLLECTION_SET_AT,
            &[pointer, pointer, pointer, types::I64],
            Some(types::I64),
            &[resources.current_frame, collection_value, index, value_word],
            resources,
        )?
        .ok_or_else(|| backend_failure("collection write produced no result"))?;
        let old_value = collection_word_to_value(builder, old_word, definition.value, pointer)?;
        lower_drop_stored_value(builder, old_value, definition.value, resources)?;
    }
    Ok(())
}

fn lower_dictionary_set_value(
    builder: &mut FunctionBuilder,
    collection: Value,
    key: Value,
    key_type: mir::Type,
    value: Value,
    value_type: mir::Type,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let key_word = value_to_collection_word(builder, key, key_type, pointer)?;
    let value_word = value_to_collection_word(builder, value, value_type, pointer)?;
    let replaced_slot =
        builder.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 1, 0));
    let replaced_pointer = builder.ins().stack_addr(pointer, replaced_slot, 0);
    let key_kind = builder
        .ins()
        .iconst(types::I8, collection_compare_kind(key_type)?);
    let old_word = runtime_call(
        builder,
        COLLECTION_KEYED_SET,
        &[pointer, types::I64, types::I64, types::I8, pointer],
        Some(types::I64),
        &[collection, key_word, value_word, key_kind, replaced_pointer],
        resources,
    )?
    .ok_or_else(|| backend_failure("dictionary write produced no result"))?;
    let replaced = builder
        .ins()
        .stack_load(pointer, types::I8, replaced_slot, 0);
    let old_value = collection_word_to_value(builder, old_word, value_type, pointer)?;
    lower_drop_value_if(builder, replaced, old_value, value_type, resources)?;
    lower_drop_value_if(builder, replaced, key, key_type, resources)
}

#[allow(clippy::too_many_arguments)]
fn lower_dictionary_set_nullable_value(
    builder: &mut FunctionBuilder,
    collection: Value,
    key: Value,
    key_type: mir::Type,
    present: Value,
    value: Value,
    payload_type: mir::Type,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let key_word = value_to_collection_word(builder, key, key_type, pointer)?;
    let value_word = value_to_collection_word(builder, value, payload_type, pointer)?;
    let replaced_slot =
        builder.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 1, 0));
    let replaced_pointer = builder.ins().stack_addr(pointer, replaced_slot, 0);
    let previous_present_slot =
        builder.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 1, 0));
    let previous_present_pointer = builder.ins().stack_addr(pointer, previous_present_slot, 0);
    let key_kind = builder
        .ins()
        .iconst(types::I8, collection_compare_kind(key_type)?);
    let old_word = runtime_call(
        builder,
        COLLECTION_KEYED_SET_NULLABLE,
        &[
            pointer,
            types::I64,
            types::I64,
            types::I8,
            types::I8,
            pointer,
            pointer,
        ],
        Some(types::I64),
        &[
            collection,
            key_word,
            value_word,
            present,
            key_kind,
            replaced_pointer,
            previous_present_pointer,
        ],
        resources,
    )?
    .ok_or_else(|| backend_failure("nullable dictionary write produced no result"))?;
    let replaced = builder
        .ins()
        .stack_load(pointer, types::I8, replaced_slot, 0);
    let previous_present = builder
        .ins()
        .stack_load(pointer, types::I8, previous_present_slot, 0);
    let drop_previous = builder.ins().band(replaced, previous_present);
    let old_value = collection_word_to_value(builder, old_word, payload_type, pointer)?;
    lower_drop_value_if(builder, drop_previous, old_value, payload_type, resources)?;
    lower_drop_value_if(builder, replaced, key, key_type, resources)
}

fn lower_set_from(
    builder: &mut FunctionBuilder,
    target: mir::CollectionTypeId,
    source: mir::LocalId,
    transfer: bool,
    algebra: Option<(mir::SetAlgebraOp, mir::LocalId)>,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let target_definition = collection_definition(resources.program, target)?.clone();
    let source_local = local_definition(resources.program, resources.function_id, source)?;
    let mir::Type::Collection(source_type) = source_local.ty else {
        return Err(malformed_mir("Set::from source is not a collection"));
    };
    let source_definition = collection_definition(resources.program, source_type)?.clone();
    if target_definition.value != source_definition.value {
        return Err(malformed_mir("Set::from element type mismatch"));
    }
    if !transfer
        && matches!(
            source_definition.value,
            mir::Type::Class(_) | mir::Type::Collection(_)
        )
    {
        return Err(malformed_mir(
            "Set::from cannot copy move-type elements from a borrowed source",
        ));
    }
    let source_value = lower_collection_pointer(builder, source, resources)?;
    if let Some((op, right)) = algebra {
        let right_value = lower_collection_pointer(builder, right, resources)?;
        let operation = builder.ins().iconst(
            types::I8,
            match op {
                mir::SetAlgebraOp::Union => 0,
                mir::SetAlgebraOp::Intersect => 1,
                mir::SetAlgebraOp::Difference => 2,
            },
        );
        let kind = builder
            .ins()
            .iconst(types::I8, collection_compare_kind(target_definition.value)?);
        return runtime_call(
            builder,
            COLLECTION_SET_ALGEBRA,
            &[pointer, pointer, types::I8, types::I8],
            Some(pointer),
            &[source_value, right_value, operation, kind],
            resources,
        )?
        .ok_or_else(|| backend_failure("set algebra produced no result"));
    }
    if let Some((ty, nullable)) = payload_enum_storage(target_definition.value) {
        return lower_payload_enum_collection_from(
            builder,
            &target_definition,
            source_value,
            ty,
            nullable,
            resources,
        );
    }
    if let Some(kind) = stage26_collection_kind(target_definition.kind) {
        if transfer {
            return Err(malformed_mir(
                "Stage 26 collection construction must preserve its source",
            ));
        }
        let kind = builder.ins().iconst(types::I8, i64::from(kind));
        let comparator = builder.ins().iconst(
            types::I8,
            i64::from(
                target_definition
                    .comparator
                    .map(collection_comparator_code)
                    .unwrap_or(COLLECTION_COMPARE_WORD),
            ),
        );
        let keyed = builder
            .ins()
            .iconst(types::I8, i64::from(target_definition.key.is_some()));
        let value_width = builder.ins().iconst(
            types::I8,
            i64::from(
                collection_value_width(target_definition.value, pointer.bytes() as u8)
                    .ok_or_else(|| malformed_mir("collection value has no runtime width"))?,
            ),
        );
        let key_kind = builder.ins().iconst(
            types::I8,
            target_definition
                .key
                .map(collection_compare_kind)
                .transpose()?
                .unwrap_or(i64::from(COLLECTION_COMPARE_WORD)),
        );
        let value_kind = builder.ins().iconst(
            types::I8,
            collection_compare_kind(
                nullable_payload_type(target_definition.value).unwrap_or(target_definition.value),
            )?,
        );
        return runtime_call(
            builder,
            COLLECTION_STAGE26_FROM_COPY,
            &[
                pointer,
                types::I8,
                types::I8,
                types::I8,
                types::I8,
                types::I8,
                types::I8,
            ],
            Some(pointer),
            &[
                source_value,
                kind,
                comparator,
                keyed,
                value_width,
                key_kind,
                value_kind,
            ],
            resources,
        )?
        .ok_or_else(|| backend_failure("Stage 26 collection conversion produced no result"));
    }
    if transfer {
        let zero = builder.ins().iconst(pointer, 0);
        builder
            .ins()
            .stack_store(pointer, zero, local_slot(resources.local_slots, source)?, 0);
    }
    let zero_length = builder.ins().iconst(pointer, 0);
    let false_value = builder.ins().iconst(types::I8, 0);
    let value_width = builder.ins().iconst(
        types::I8,
        i64::from(
            collection_value_width(target_definition.value, pointer.bytes() as u8).ok_or_else(
                || {
                    malformed_mir(
                        "nullable collection elements are not supported by Stage 23 Slice 3",
                    )
                },
            )?,
        ),
    );
    let target_value = runtime_call(
        builder,
        COLLECTION_NEW,
        &[pointer, types::I8, types::I8, types::I8],
        Some(pointer),
        &[zero_length, false_value, false_value, value_width],
        resources,
    )?
    .ok_or_else(|| backend_failure("set allocation produced no result"))?;
    let length = runtime_call(
        builder,
        COLLECTION_LENGTH,
        &[pointer],
        Some(pointer),
        &[source_value],
        resources,
    )?
    .ok_or_else(|| backend_failure("collection length produced no result"))?;
    let header = builder.create_block();
    let body = builder.create_block();
    let done = builder.create_block();
    builder.append_block_param(header, pointer);
    let start = builder.ins().iconst(pointer, 0);
    builder.ins().jump(header, &[BlockArg::Value(start)]);
    builder.switch_to_block(header);
    let index = builder.block_params(header)[0];
    let more = builder.ins().icmp(IntCC::UnsignedLessThan, index, length);
    builder.ins().brif(more, body, &[], done, &[]);
    builder.switch_to_block(body);
    let word = runtime_call(
        builder,
        COLLECTION_VALUE_AT,
        &[pointer, pointer, pointer],
        Some(types::I64),
        &[resources.current_frame, source_value, index],
        resources,
    )?
    .ok_or_else(|| backend_failure("Set::from element read produced no result"))?;
    let mut value = collection_word_to_value(builder, word, source_definition.value, pointer)?;
    if !transfer && source_definition.value == mir::Type::String {
        value = retain_string(builder, value, resources)?;
    }
    let word = value_to_collection_word(builder, value, source_definition.value, pointer)?;
    let kind = builder
        .ins()
        .iconst(types::I8, collection_compare_kind(source_definition.value)?);
    let present = builder.ins().iconst(types::I8, 1);
    let inserted = runtime_call(
        builder,
        COLLECTION_PUSH_UNIQUE,
        &[pointer, types::I64, types::I8, types::I8],
        Some(types::I8),
        &[target_value, word, present, kind],
        resources,
    )?
    .ok_or_else(|| backend_failure("Set::from insertion produced no result"))?;
    lower_drop_value_unless(builder, inserted, value, source_definition.value, resources)?;
    let one = builder.ins().iconst(pointer, 1);
    let next = builder.ins().iadd(index, one);
    builder.ins().jump(header, &[BlockArg::Value(next)]);
    builder.switch_to_block(done);
    if transfer {
        let _ = runtime_call(
            builder,
            COLLECTION_FREE,
            &[pointer],
            None,
            &[source_value],
            resources,
        )?;
    }
    Ok(target_value)
}

fn lower_payload_enum_collection_from(
    builder: &mut FunctionBuilder,
    target: &mir::CollectionType,
    source: Value,
    ty: mir::PayloadEnumType,
    nullable: bool,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let kind = stage26_collection_kind(target.kind)
        .ok_or_else(|| malformed_mir("aggregate collection conversion targets a legacy kind"))?;
    let length = runtime_call(
        builder,
        COLLECTION_LENGTH,
        &[pointer],
        Some(pointer),
        &[source],
        resources,
    )?
    .ok_or_else(|| backend_failure("aggregate collection length produced no result"))?;
    let keyed = builder
        .ins()
        .iconst(types::I8, i64::from(target.key.is_some()));
    let fixed = builder.ins().iconst(types::I8, 0);
    let value_size = builder
        .ins()
        .iconst(pointer, i64::from(ty.storage_size(nullable)));
    let value_alignment = builder.ins().iconst(pointer, i64::from(ty.align));
    let kind_value = builder.ins().iconst(types::I8, i64::from(kind));
    let comparator = builder.ins().iconst(
        types::I8,
        i64::from(
            target
                .comparator
                .map(collection_comparator_code)
                .unwrap_or(COLLECTION_COMPARE_WORD),
        ),
    );
    let result = runtime_call(
        builder,
        COLLECTION_AGGREGATE_NEW,
        &[
            pointer,
            pointer,
            types::I8,
            types::I8,
            pointer,
            pointer,
            types::I8,
            types::I8,
        ],
        Some(pointer),
        &[
            resources.current_frame,
            length,
            keyed,
            fixed,
            value_size,
            value_alignment,
            kind_value,
            comparator,
        ],
        resources,
    )?
    .ok_or_else(|| backend_failure("aggregate collection allocation produced no result"))?;

    let header = builder.create_block();
    let body = builder.create_block();
    let done = builder.create_block();
    builder.append_block_param(header, pointer);
    let zero = builder.ins().iconst(pointer, 0);
    builder.ins().jump(header, &[BlockArg::Value(zero)]);
    builder.switch_to_block(header);
    let index = builder.block_params(header)[0];
    let more = builder.ins().icmp(IntCC::UnsignedLessThan, index, length);
    builder.ins().brif(more, body, &[], done, &[]);
    builder.switch_to_block(body);
    let index_word = if pointer == types::I64 {
        index
    } else {
        builder.ins().uextend(types::I64, index)
    };
    let positional = builder.ins().iconst(types::I8, 1);
    let word_kind = builder
        .ins()
        .iconst(types::I8, i64::from(COLLECTION_COMPARE_WORD));
    let source_slot = runtime_call(
        builder,
        COLLECTION_AGGREGATE_VALUE_AT,
        &[pointer, pointer, types::I64, types::I8, types::I8],
        Some(pointer),
        &[
            resources.current_frame,
            source,
            index_word,
            positional,
            word_kind,
        ],
        resources,
    )?
    .ok_or_else(|| backend_failure("aggregate collection read produced no slot"))?;
    let destination = if let Some(key_ty) = target.key {
        let mut key = runtime_call(
            builder,
            COLLECTION_KEY_AT,
            &[pointer, pointer, pointer],
            Some(types::I64),
            &[resources.current_frame, source, index],
            resources,
        )?
        .ok_or_else(|| backend_failure("aggregate collection key read produced no result"))?;
        if key_ty == mir::Type::String {
            key = retain_string(builder, key, resources)?;
        }
        lower_aggregate_dictionary_write_slot(
            builder, result, key, key_ty, ty, nullable, resources,
        )?
    } else {
        runtime_call(
            builder,
            COLLECTION_AGGREGATE_PUSH_SLOT,
            &[pointer],
            Some(pointer),
            &[result],
            resources,
        )?
        .ok_or_else(|| backend_failure("aggregate collection insertion produced no slot"))?
    };
    copy_inline_bytes(
        builder,
        destination,
        source_slot,
        ty.storage_size(nullable),
        pointer,
    );
    retain_payload_enum_at(builder, destination, ty, nullable, resources)?;
    let one = builder.ins().iconst(pointer, 1);
    let next = builder.ins().iadd(index, one);
    builder.ins().jump(header, &[BlockArg::Value(next)]);
    builder.switch_to_block(done);
    let _ = runtime_call(
        builder,
        COLLECTION_STAGE26_FINALIZE,
        &[pointer],
        None,
        &[result],
        resources,
    )?;
    Ok(result)
}

fn lower_drop_value_if(
    builder: &mut FunctionBuilder,
    condition: Value,
    value: Value,
    ty: mir::Type,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    if !matches!(
        ty,
        mir::Type::String
            | mir::Type::NullableString
            | mir::Type::Class(_)
            | mir::Type::NullableClass(_)
            | mir::Type::SharedReference(_)
            | mir::Type::WeakReference(_)
            | mir::Type::NullableSharedReference(_)
            | mir::Type::NullableWeakReference(_)
            | mir::Type::WritableSharedReference(_)
            | mir::Type::WritableWeakReference(_)
            | mir::Type::NullableWritableSharedReference(_)
            | mir::Type::NullableWritableWeakReference(_)
            | mir::Type::ReadonlySharedReferenceAccess(_)
            | mir::Type::WritableSharedReferenceAccess(_)
            | mir::Type::NullableReadonlySharedReferenceAccess(_)
            | mir::Type::NullableWritableSharedReferenceAccess(_)
            | mir::Type::Collection(_)
            | mir::Type::NullableCollection(_)
            | mir::Type::Mixed
            | mir::Type::NullableMixed
    ) {
        return Ok(());
    }
    let drop_block = builder.create_block();
    let done = builder.create_block();
    builder.ins().brif(condition, drop_block, &[], done, &[]);
    builder.switch_to_block(drop_block);
    lower_drop_stored_value(builder, value, ty, resources)?;
    builder.ins().jump(done, &[]);
    builder.switch_to_block(done);
    Ok(())
}

fn lower_drop_value_unless(
    builder: &mut FunctionBuilder,
    condition: Value,
    value: Value,
    ty: mir::Type,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let zero = builder.ins().iconst(types::I8, 0);
    let should_drop = builder.ins().icmp(IntCC::Equal, condition, zero);
    lower_drop_value_if(builder, should_drop, value, ty, resources)
}

fn lower_drop_stored_value(
    builder: &mut FunctionBuilder,
    value: Value,
    ty: mir::Type,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    match ty {
        mir::Type::Error | mir::Type::NullableError => {
            let pointer = resources.module.target_config().pointer_type();
            let flags = cranelift_codegen::ir::MachMemFlags::trusted();
            let descriptor = builder
                .ins()
                .load(pointer, flags, value, pointer.bytes() as i32);
            let object = builder.ins().load(pointer, flags, value, 0);
            lower_drop_error_value(
                builder,
                LoweredValue::Nullable {
                    present: object,
                    payload: descriptor,
                },
                resources,
            )
        }
        mir::Type::String | mir::Type::NullableString => release_string(builder, value, resources),
        mir::Type::Mixed | mir::Type::NullableMixed => {
            lower_drop_mixed_value(builder, value, resources)
        }
        mir::Type::Class(class) | mir::Type::NullableClass(class) => {
            lower_drop_class_value_checked(builder, value, class, resources)
        }
        mir::Type::SharedReference(_) | mir::Type::NullableSharedReference(_) => {
            lower_drop_shared_value(builder, value, false, resources)
        }
        mir::Type::WeakReference(_) | mir::Type::NullableWeakReference(_) => {
            lower_drop_shared_value(builder, value, true, resources)
        }
        mir::Type::WritableSharedReference(_)
        | mir::Type::WritableWeakReference(_)
        | mir::Type::NullableWritableSharedReference(_)
        | mir::Type::NullableWritableWeakReference(_)
        | mir::Type::ReadonlySharedReferenceAccess(_)
        | mir::Type::WritableSharedReferenceAccess(_)
        | mir::Type::NullableReadonlySharedReferenceAccess(_)
        | mir::Type::NullableWritableSharedReferenceAccess(_) => {
            let symbol = writable_shared_release_symbol(ty)
                .ok_or_else(|| malformed_mir("writable shared release symbol is missing"))?;
            lower_drop_writable_shared_value(builder, value, symbol, resources)
        }
        mir::Type::Collection(collection) | mir::Type::NullableCollection(collection) => {
            lower_drop_collection_value(builder, value, collection, resources)
        }
        mir::Type::PayloadEnum(ty) => {
            lower_drop_payload_enum_at(builder, value, ty, false, resources)
        }
        mir::Type::NullablePayloadEnum(ty) => {
            lower_drop_payload_enum_at(builder, value, ty, true, resources)
        }
        mir::Type::Function(_) | mir::Type::NullableFunction(_) => {
            let pointer = resources.module.target_config().pointer_type();
            let carrier = load_lowered_from_address(builder, ty, value, pointer);
            lower_drop_function_carrier(builder, carrier, resources)
        }
        mir::Type::Scalar(_) | mir::Type::NullableScalar(_) => Ok(()),
        mir::Type::ClosureEnvironment(_) => Err(malformed_mir(
            "closure environment pointer reached stored-value cleanup",
        )),
    }
}

fn lower_drop_collection_value(
    builder: &mut FunctionBuilder,
    collection: Value,
    collection_type: mir::CollectionTypeId,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    lower_finish_collection_value(
        builder,
        collection,
        collection_type,
        CollectionStorageAction::Free,
        resources,
    )
}

fn lower_clear_collection_value(
    builder: &mut FunctionBuilder,
    collection: Value,
    collection_type: mir::CollectionTypeId,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    lower_finish_collection_value(
        builder,
        collection,
        collection_type,
        CollectionStorageAction::Reset,
        resources,
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CollectionStorageAction {
    Free,
    Reset,
}

fn lower_finish_collection_value(
    builder: &mut FunctionBuilder,
    collection: Value,
    collection_type: mir::CollectionTypeId,
    action: CollectionStorageAction,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let definition = collection_definition(resources.program, collection_type)?.clone();
    let zero = builder.ins().iconst(pointer, 0);
    let present = builder.ins().icmp(IntCC::NotEqual, collection, zero);
    let drop_block = builder.create_block();
    let done = builder.create_block();
    builder.ins().brif(present, drop_block, &[], done, &[]);
    builder.switch_to_block(drop_block);
    if definition.kind == mir::CollectionKind::Bytes {
        if action != CollectionStorageAction::Free {
            return Err(malformed_mir(
                "Bytes cannot be cleared as a named collection",
            ));
        }
        let _ = runtime_call(
            builder,
            BYTES_FREE,
            &[pointer],
            None,
            &[collection],
            resources,
        )?;
        builder.ins().jump(done, &[]);
        builder.switch_to_block(done);
        return Ok(());
    }
    let scalar_values = matches!(
        definition.value,
        mir::Type::Scalar(_) | mir::Type::NullableScalar(_)
    );
    let scalar_keys = definition
        .key
        .is_none_or(|key| matches!(key, mir::Type::Scalar(_)));
    if scalar_values && scalar_keys {
        let symbol = match action {
            CollectionStorageAction::Free => COLLECTION_FREE,
            CollectionStorageAction::Reset => COLLECTION_RESET_AFTER_CLEANUP,
        };
        let _ = runtime_call(builder, symbol, &[pointer], None, &[collection], resources)?;
        builder.ins().jump(done, &[]);
        builder.switch_to_block(done);
        return Ok(());
    }
    let cleanup_collection = if action == CollectionStorageAction::Reset {
        let pointer_bytes = pointer.bytes();
        let cleanup_slot = builder.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot,
            collection_header_size(pointer_bytes),
            pointer_bytes.trailing_zeros() as u8,
        ));
        let cleanup_collection = builder.ins().stack_addr(pointer, cleanup_slot, 0);
        let _ = runtime_call(
            builder,
            COLLECTION_DETACH_FOR_CLEANUP,
            &[pointer, pointer, pointer],
            None,
            &[resources.current_frame, collection, cleanup_collection],
            resources,
        )?;
        cleanup_collection
    } else {
        collection
    };
    let length = runtime_call(
        builder,
        COLLECTION_LENGTH,
        &[pointer],
        Some(pointer),
        &[cleanup_collection],
        resources,
    )?
    .ok_or_else(|| backend_failure("collection length produced no result"))?;
    let header = builder.create_block();
    let body = builder.create_block();
    let free = builder.create_block();
    builder.append_block_param(header, pointer);
    builder.ins().jump(header, &[BlockArg::Value(length)]);
    builder.switch_to_block(header);
    let remaining = builder.block_params(header)[0];
    let more = builder.ins().icmp(IntCC::NotEqual, remaining, zero);
    builder.ins().brif(more, body, &[], free, &[]);
    builder.switch_to_block(body);
    let one = builder.ins().iconst(pointer, 1);
    let index = builder.ins().isub(remaining, one);
    if matches!(
        definition.value,
        mir::Type::Error
            | mir::Type::NullableError
            | mir::Type::Function(_)
            | mir::Type::NullableFunction(_)
    ) {
        let index_word = if pointer == types::I64 {
            index
        } else {
            builder.ins().uextend(types::I64, index)
        };
        let positional = builder.ins().iconst(types::I8, 1);
        let key_kind = builder
            .ins()
            .iconst(types::I8, i64::from(COLLECTION_COMPARE_WORD));
        let value = runtime_call(
            builder,
            COLLECTION_AGGREGATE_VALUE_AT,
            &[pointer, pointer, types::I64, types::I8, types::I8],
            Some(pointer),
            &[
                resources.current_frame,
                cleanup_collection,
                index_word,
                positional,
                key_kind,
            ],
            resources,
        )?
        .ok_or_else(|| backend_failure("aggregate collection value read produced no slot"))?;
        lower_drop_value_at_address(builder, definition.value, value, resources)?;
    } else if let Some((ty, nullable)) = payload_enum_storage(definition.value) {
        let index_word = if pointer == types::I64 {
            index
        } else {
            builder.ins().uextend(types::I64, index)
        };
        let positional = builder.ins().iconst(types::I8, 1);
        let key_kind = builder
            .ins()
            .iconst(types::I8, i64::from(COLLECTION_COMPARE_WORD));
        let value = runtime_call(
            builder,
            COLLECTION_AGGREGATE_VALUE_AT,
            &[pointer, pointer, types::I64, types::I8, types::I8],
            Some(pointer),
            &[
                resources.current_frame,
                cleanup_collection,
                index_word,
                positional,
                key_kind,
            ],
            resources,
        )?
        .ok_or_else(|| backend_failure("aggregate collection value read produced no slot"))?;
        lower_drop_payload_enum_at(builder, value, ty, nullable, resources)?;
    } else {
        let value_word = runtime_call(
            builder,
            COLLECTION_VALUE_AT,
            &[pointer, pointer, pointer],
            Some(types::I64),
            &[resources.current_frame, cleanup_collection, index],
            resources,
        )?
        .ok_or_else(|| backend_failure("collection value read produced no result"))?;
        let stored_value_type = nullable_payload_type(definition.value).unwrap_or(definition.value);
        let value = collection_word_to_value(builder, value_word, stored_value_type, pointer)?;
        lower_drop_stored_value(builder, value, stored_value_type, resources)?;
    }
    if let Some(key_type) = definition.key {
        let key_word = runtime_call(
            builder,
            COLLECTION_KEY_AT,
            &[pointer, pointer, pointer],
            Some(types::I64),
            &[resources.current_frame, cleanup_collection, index],
            resources,
        )?
        .ok_or_else(|| backend_failure("collection key read produced no result"))?;
        let key = collection_word_to_value(builder, key_word, key_type, pointer)?;
        lower_drop_stored_value(builder, key, key_type, resources)?;
    }
    builder.ins().jump(header, &[BlockArg::Value(index)]);
    builder.switch_to_block(free);
    match action {
        CollectionStorageAction::Free => {
            let _ = runtime_call(
                builder,
                COLLECTION_FREE,
                &[pointer],
                None,
                &[cleanup_collection],
                resources,
            )?;
        }
        CollectionStorageAction::Reset => {
            let _ = runtime_call(
                builder,
                COLLECTION_FINISH_DETACHED_CLEANUP,
                &[pointer, pointer],
                None,
                &[collection, cleanup_collection],
                resources,
            )?;
        }
    }
    builder.ins().jump(done, &[]);
    builder.switch_to_block(done);
    Ok(())
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
            let value = builder
                .ins()
                .stack_load(pointer_type, pointer_type, slot, 0);
            if *transfer {
                let zero = builder.ins().iconst(pointer_type, 0);
                builder.ins().stack_store(pointer_type, zero, slot, 0);
            }
            Ok(value)
        }
        mir::ClassExpression::NullableLocalAssumeNonNull {
            local, transfer, ..
        } => {
            let slot = local_slot(resources.local_slots, *local)?;
            let value = builder
                .ins()
                .stack_load(pointer_type, pointer_type, slot, 0);
            if *transfer {
                let zero = builder.ins().iconst(pointer_type, 0);
                builder.ins().stack_store(pointer_type, zero, slot, 0);
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
            let (object, lowered_args) =
                lower_class_allocation(builder, *class, properties, args, resources)?;
            if let Some(constructor) = constructor {
                let mut constructor_args = vec![resources.current_frame, object];
                constructor_args.extend(lowered_args.abi_values.iter().copied());
                let callee = declared_function(builder, resources, *constructor)?;
                builder.ins().call(callee, &constructor_args);

                let constructor_definition = function_in(resources.program, *constructor)?;
                for (index, value, ownership) in &lowered_args.temporary_mixed {
                    if args[*index].transferred_owned_local().is_some() {
                        continue;
                    }
                    let promoted = properties.iter().any(|property| {
                        matches!(
                            property.source,
                            mir::PropertyValueSource::ConstructorArgument(argument)
                                if argument == *index
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
                        lower_cleanup_mixed_temporary(builder, *value, *ownership, resources)?;
                    }
                }
                for index in ordered_owned_argument_indices(args) {
                    let argument = &args[index];
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
                        if let Some(class) = argument.owned_temporary_class() {
                            defer_or_drop_class_temporary(builder, value, class, resources)?;
                        } else if let Some(collection) = argument.owned_temporary_collection() {
                            defer_or_drop_collection_temporary(
                                builder, value, collection, resources,
                            )?;
                        } else if let Some(shared) = argument.owned_temporary_shared() {
                            defer_or_drop_owned_shared_temporary(
                                builder, value, shared, resources,
                            )?;
                        } else if let Some((payload, nullable)) =
                            argument.owned_temporary_payload_enum()
                        {
                            lower_drop_payload_enum_at(
                                builder, value, payload, nullable, resources,
                            )?;
                        } else if argument.mixed_ownership().has_shell() {
                            defer_or_cleanup_mixed_temporary(
                                builder,
                                value,
                                argument.mixed_ownership(),
                                resources,
                            )?;
                        }
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
        mir::ClassExpression::Coalesce {
            left,
            right,
            transfer,
            ..
        } => {
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
            if !transfer && (left_owned || right_owned) {
                defer_or_drop_class_temporary(
                    builder,
                    builder.block_params(done)[1],
                    expression.class(),
                    resources,
                )?;
            }
            Ok(result)
        }
        mir::ClassExpression::CollectionIndex {
            collection,
            index,
            transfer,
            positional,
            ..
        } => lower_collection_index(
            builder,
            *collection,
            index,
            *transfer,
            *positional,
            resources,
        ),
        mir::ClassExpression::MixedPayload {
            mixed,
            class,
            transfer,
        } => {
            if *transfer {
                return lower_take_mixed_payload(
                    builder,
                    *mixed,
                    mir::MixedTag::Class(*class),
                    resources,
                );
            }
            lower_mixed_payload(builder, *mixed, mir::MixedTag::Class(*class), resources)
        }
        mir::ClassExpression::SharedPayload { reference, .. } => {
            let owned = reference.owned_temporary().is_some();
            let control = lower_shared_reference_expression(builder, reference, resources)?;
            let payload = runtime_call(
                builder,
                SHARED_PAYLOAD,
                &[pointer_type],
                Some(pointer_type),
                &[control],
                resources,
            )?
            .ok_or_else(|| backend_failure("shared payload projection produced no result"))?;
            if owned {
                defer_or_drop_shared_temporary(builder, control, false, resources)?;
            }
            Ok(payload)
        }
        mir::ClassExpression::SharedAccessPayload {
            access, writable, ..
        } => lower_shared_access_payload(builder, *access, *writable, resources),
    }
}

fn lower_class_allocation(
    builder: &mut FunctionBuilder,
    class: crate::class_layout::ClassId,
    properties: &[mir::PropertyValue],
    args: &[mir::Rvalue],
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(Value, LoweredCallArgs), BackendError> {
    let pointer = resources.module.target_config().pointer_type();
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
    let class_definition = class_definition(resources.program, class)?;
    let size = builder
        .ins()
        .iconst(pointer, i64::from(class_definition.layout.size));
    let align = builder
        .ins()
        .iconst(pointer, i64::from(class_definition.layout.align));
    let object = runtime_call(
        builder,
        CLASS_ALLOCATE,
        &[pointer, pointer, pointer],
        Some(pointer),
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
        let address =
            lower_property_address_from_value(builder, object, property.property, resources)?;
        let property_definition = property_definition(resources.program, property.property)?;
        store_lowered_to_address(builder, property_definition.ty, address, value, pointer)?;
    }
    Ok((object, lowered_args))
}

fn cleanup_constructor_arguments(
    builder: &mut FunctionBuilder,
    constructor: mir::FunctionId,
    properties: &[mir::PropertyValue],
    args: &[mir::Rvalue],
    lowered: &LoweredCallArgs,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let definition = function_in(resources.program, constructor)?;
    let promoted = |index| {
        properties.iter().any(|property| {
            matches!(
                property.source,
                mir::PropertyValueSource::ConstructorArgument(argument) if argument == index
            )
        })
    };
    for (index, value, ownership) in &lowered.temporary_mixed {
        if args[*index].transferred_owned_local().is_some() || promoted(*index) {
            continue;
        }
        let parameter = *definition.params.get(index + 1).ok_or_else(|| {
            malformed_mir(format!(
                "constructor function{} is missing parameter {index}",
                constructor.0
            ))
        })?;
        if !local_in(definition, parameter)?.owned {
            lower_cleanup_mixed_temporary(builder, *value, *ownership, resources)?;
        }
    }
    for index in ordered_owned_argument_indices(args) {
        if promoted(index) {
            continue;
        }
        let argument = &args[index];
        let parameter = *definition.params.get(index + 1).ok_or_else(|| {
            malformed_mir(format!(
                "constructor function{} is missing parameter {index}",
                constructor.0
            ))
        })?;
        if !local_in(definition, parameter)?.owned {
            let value = lowered.arguments[index].single()?;
            if let Some(class) = argument.owned_temporary_class() {
                defer_or_drop_class_temporary(builder, value, class, resources)?;
            } else if let Some(collection) = argument.owned_temporary_collection() {
                defer_or_drop_collection_temporary(builder, value, collection, resources)?;
            } else if let Some(shared) = argument.owned_temporary_shared() {
                defer_or_drop_owned_shared_temporary(builder, value, shared, resources)?;
            } else if let Some((payload, nullable)) = argument.owned_temporary_payload_enum() {
                lower_drop_payload_enum_at(builder, value, payload, nullable, resources)?;
            } else if argument.mixed_ownership().has_shell() {
                defer_or_cleanup_mixed_temporary(
                    builder,
                    value,
                    argument.mixed_ownership(),
                    resources,
                )?;
            }
        }
    }
    for (index, string) in &lowered.owned_strings {
        if !promoted(*index) {
            release_string(builder, *string, resources)?;
        }
    }
    Ok(())
}

fn lower_shared_access_payload(
    builder: &mut FunctionBuilder,
    access: mir::LocalId,
    writable: bool,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let control = lower_pointer_local(builder, access, false, resources)?;
    runtime_call(
        builder,
        if writable {
            WRITABLE_SHARED_WRITABLE_PAYLOAD
        } else {
            WRITABLE_SHARED_READONLY_PAYLOAD
        },
        &[pointer],
        Some(pointer),
        &[control],
        resources,
    )?
    .ok_or_else(|| backend_failure("shared access payload projection produced no result"))
}

fn lower_shared_reference_expression(
    builder: &mut FunctionBuilder,
    expression: &mir::SharedReferenceExpression,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    match expression {
        mir::SharedReferenceExpression::New { class, value } => {
            let payload = lower_class_expression(builder, value, resources)?;
            let drop_id = *resources
                .class_drop_function_ids
                .get(class.0)
                .ok_or_else(|| malformed_mir("shared payload drop glue does not exist"))?;
            let drop_ref = resources.module.declare_func_in_func(drop_id, builder.func);
            let drop_fn = builder.ins().func_addr(pointer, drop_ref);
            runtime_call(
                builder,
                SHARED_CREATE,
                &[pointer, pointer, pointer],
                Some(pointer),
                &[resources.current_frame, payload, drop_fn],
                resources,
            )?
            .ok_or_else(|| backend_failure("shared construction produced no result"))
        }
        mir::SharedReferenceExpression::Local {
            local, transfer, ..
        }
        | mir::SharedReferenceExpression::NullableLocalAssumeNonNull {
            local, transfer, ..
        } => {
            let slot = local_slot(resources.local_slots, *local)?;
            let value = builder.ins().stack_load(pointer, pointer, slot, 0);
            if *transfer {
                let zero = builder.ins().iconst(pointer, 0);
                builder.ins().stack_store(pointer, zero, slot, 0);
            }
            Ok(value)
        }
        mir::SharedReferenceExpression::Property {
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
        mir::SharedReferenceExpression::Call { function, args, .. } => {
            lower_function_call(builder, *function, args, resources)?
                .ok_or_else(|| malformed_mir("shared-reference call produced no result"))?
                .single()
        }
        mir::SharedReferenceExpression::Share { value, .. } => {
            let owned = value.owned_temporary().is_some();
            let value = lower_shared_reference_expression(builder, value, resources)?;
            let shared = runtime_call(
                builder,
                SHARED_RETAIN,
                &[pointer, pointer],
                Some(pointer),
                &[resources.current_frame, value],
                resources,
            )?
            .ok_or_else(|| backend_failure("shared retain produced no result"))?;
            if owned {
                defer_or_drop_shared_temporary(builder, value, false, resources)?;
            }
            Ok(shared)
        }
        mir::SharedReferenceExpression::Coalesce {
            left,
            right,
            transfer,
            ..
        } => {
            let left_owned = left.owned_temporary().is_some();
            let right_owned = right.owned_temporary().is_some();
            let left = lower_nullable_shared_reference_expression(builder, left, resources)?;
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
            let left_temporary = if left_owned && !transfer { left } else { zero };
            builder.ins().jump(
                done,
                &[BlockArg::Value(left), BlockArg::Value(left_temporary)],
            );
            builder.switch_to_block(right_block);
            let right = lower_shared_reference_expression(builder, right, resources)?;
            let right_temporary = if right_owned && !transfer {
                right
            } else {
                zero
            };
            builder.ins().jump(
                done,
                &[BlockArg::Value(right), BlockArg::Value(right_temporary)],
            );
            builder.switch_to_block(done);
            let result = builder.block_params(done)[0];
            if !transfer && (left_owned || right_owned) {
                defer_or_drop_shared_temporary(
                    builder,
                    builder.block_params(done)[1],
                    false,
                    resources,
                )?;
            }
            Ok(result)
        }
        mir::SharedReferenceExpression::CollectionIndex {
            collection,
            index,
            remove,
            positional,
            ..
        } => lower_collection_index(builder, *collection, index, *remove, *positional, resources),
    }
}

fn lower_weak_reference_expression(
    builder: &mut FunctionBuilder,
    expression: &mir::WeakReferenceExpression,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    match expression {
        mir::WeakReferenceExpression::Local {
            local, transfer, ..
        }
        | mir::WeakReferenceExpression::NullableLocalAssumeNonNull {
            local, transfer, ..
        } => {
            let slot = local_slot(resources.local_slots, *local)?;
            let value = builder.ins().stack_load(pointer, pointer, slot, 0);
            if *transfer {
                let zero = builder.ins().iconst(pointer, 0);
                builder.ins().stack_store(pointer, zero, slot, 0);
            }
            Ok(value)
        }
        mir::WeakReferenceExpression::Property {
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
        mir::WeakReferenceExpression::Call { function, args, .. } => {
            lower_function_call(builder, *function, args, resources)?
                .ok_or_else(|| malformed_mir("weak-reference call produced no result"))?
                .single()
        }
        mir::WeakReferenceExpression::Create { value, .. } => {
            let owned = value.owned_temporary().is_some();
            let value = lower_shared_reference_expression(builder, value, resources)?;
            let weak = runtime_call(
                builder,
                SHARED_CREATE_WEAK,
                &[pointer, pointer],
                Some(pointer),
                &[resources.current_frame, value],
                resources,
            )?
            .ok_or_else(|| backend_failure("weak-reference creation produced no result"))?;
            if owned {
                defer_or_drop_shared_temporary(builder, value, false, resources)?;
            }
            Ok(weak)
        }
        mir::WeakReferenceExpression::Coalesce {
            left,
            right,
            transfer,
            ..
        } => {
            let left_owned = left.owned_temporary().is_some();
            let right_owned = right.owned_temporary().is_some();
            let left = lower_nullable_weak_reference_expression(builder, left, resources)?;
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
            let left_temporary = if left_owned && !transfer { left } else { zero };
            builder.ins().jump(
                done,
                &[BlockArg::Value(left), BlockArg::Value(left_temporary)],
            );
            builder.switch_to_block(right_block);
            let right = lower_weak_reference_expression(builder, right, resources)?;
            let right_temporary = if right_owned && !transfer {
                right
            } else {
                zero
            };
            builder.ins().jump(
                done,
                &[BlockArg::Value(right), BlockArg::Value(right_temporary)],
            );
            builder.switch_to_block(done);
            let result = builder.block_params(done)[0];
            if !transfer && (left_owned || right_owned) {
                defer_or_drop_shared_temporary(
                    builder,
                    builder.block_params(done)[1],
                    true,
                    resources,
                )?;
            }
            Ok(result)
        }
        mir::WeakReferenceExpression::CollectionIndex {
            collection,
            index,
            remove,
            positional,
            ..
        } => lower_collection_index(builder, *collection, index, *remove, *positional, resources),
    }
}

fn lower_nullable_shared_reference_expression(
    builder: &mut FunctionBuilder,
    expression: &mir::NullableSharedReferenceExpression,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    match expression {
        mir::NullableSharedReferenceExpression::Null(_) => Ok(builder.ins().iconst(pointer, 0)),
        mir::NullableSharedReferenceExpression::Shared(value) => {
            lower_shared_reference_expression(builder, value, resources)
        }
        mir::NullableSharedReferenceExpression::Local {
            local, transfer, ..
        } => {
            let slot = local_slot(resources.local_slots, *local)?;
            let value = builder.ins().stack_load(pointer, pointer, slot, 0);
            if *transfer {
                let zero = builder.ins().iconst(pointer, 0);
                builder.ins().stack_store(pointer, zero, slot, 0);
            }
            Ok(value)
        }
        mir::NullableSharedReferenceExpression::Property {
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
        mir::NullableSharedReferenceExpression::Call { function, args, .. } => {
            lower_function_call(builder, *function, args, resources)?
                .ok_or_else(|| malformed_mir("nullable shared call produced no result"))?
                .single()
        }
        mir::NullableSharedReferenceExpression::Acquire { value, .. } => {
            let owned = value.owned_temporary().is_some();
            let value = lower_weak_reference_expression(builder, value, resources)?;
            let acquired = runtime_call(
                builder,
                SHARED_ACQUIRE,
                &[pointer, pointer],
                Some(pointer),
                &[resources.current_frame, value],
                resources,
            )?
            .ok_or_else(|| backend_failure("weak acquisition produced no result"))?;
            if owned {
                defer_or_drop_shared_temporary(builder, value, true, resources)?;
            }
            Ok(acquired)
        }
        mir::NullableSharedReferenceExpression::NullSafeShare { value, .. } => {
            let owned = value.owned_temporary().is_some();
            let value = lower_nullable_shared_reference_expression(builder, value, resources)?;
            let result = lower_null_safe_shared_call(
                builder,
                value,
                SHARED_RETAIN,
                "null-safe shared retain",
                true,
                resources,
            )?;
            if owned {
                defer_or_drop_shared_temporary(builder, value, false, resources)?;
            }
            Ok(result)
        }
        mir::NullableSharedReferenceExpression::NullSafeAcquire { value, .. } => {
            let owned = value.owned_temporary().is_some();
            let value = lower_nullable_weak_reference_expression(builder, value, resources)?;
            let result = lower_null_safe_shared_call(
                builder,
                value,
                SHARED_ACQUIRE,
                "null-safe weak acquisition",
                true,
                resources,
            )?;
            if owned {
                defer_or_drop_shared_temporary(builder, value, true, resources)?;
            }
            Ok(result)
        }
        mir::NullableSharedReferenceExpression::Coalesce {
            left,
            right,
            transfer,
            ..
        } => {
            let left_owned = left.owned_temporary().is_some();
            let right_owned = right.owned_temporary().is_some();
            let left = lower_nullable_shared_reference_expression(builder, left, resources)?;
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
            let left_temporary = if left_owned && !transfer { left } else { zero };
            builder.ins().jump(
                done,
                &[BlockArg::Value(left), BlockArg::Value(left_temporary)],
            );
            builder.switch_to_block(right_block);
            let right = lower_nullable_shared_reference_expression(builder, right, resources)?;
            let right_temporary = if right_owned && !transfer {
                right
            } else {
                zero
            };
            builder.ins().jump(
                done,
                &[BlockArg::Value(right), BlockArg::Value(right_temporary)],
            );
            builder.switch_to_block(done);
            let result = builder.block_params(done)[0];
            if !transfer && (left_owned || right_owned) {
                defer_or_drop_shared_temporary(
                    builder,
                    builder.block_params(done)[1],
                    false,
                    resources,
                )?;
            }
            Ok(result)
        }
        mir::NullableSharedReferenceExpression::DictionaryGet {
            class,
            collection,
            key,
            access,
            stored_nullable,
        } => {
            let (_, payload) = lower_dictionary_get(
                builder,
                *collection,
                key,
                if *stored_nullable {
                    mir::Type::NullableSharedReference(*class)
                } else {
                    mir::Type::SharedReference(*class)
                },
                *access,
                resources,
            )?;
            Ok(payload)
        }
        mir::NullableSharedReferenceExpression::CollectionIndex {
            collection,
            index,
            remove,
            positional,
            ..
        } => lower_collection_index(builder, *collection, index, *remove, *positional, resources),
    }
}

fn lower_null_safe_shared_call(
    builder: &mut FunctionBuilder,
    value: Value,
    symbol: &'static str,
    operation: &'static str,
    takes_frame: bool,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let zero = builder.ins().iconst(pointer, 0);
    let present = builder.ins().icmp(IntCC::NotEqual, value, zero);
    let some = builder.create_block();
    let none = builder.create_block();
    let done = builder.create_block();
    builder.append_block_param(done, pointer);
    builder.ins().brif(present, some, &[], none, &[]);
    builder.switch_to_block(some);
    let (params, values): (&[_], &[_]) = if takes_frame {
        (&[pointer, pointer], &[resources.current_frame, value])
    } else {
        (&[pointer], &[value])
    };
    let result = runtime_call(builder, symbol, params, Some(pointer), values, resources)?
        .ok_or_else(|| backend_failure(format!("{operation} produced no result")))?;
    builder.ins().jump(done, &[BlockArg::Value(result)]);
    builder.switch_to_block(none);
    builder.ins().jump(done, &[BlockArg::Value(zero)]);
    builder.switch_to_block(done);
    Ok(builder.block_params(done)[0])
}

fn lower_nullable_weak_reference_expression(
    builder: &mut FunctionBuilder,
    expression: &mir::NullableWeakReferenceExpression,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    match expression {
        mir::NullableWeakReferenceExpression::Null(_) => Ok(builder.ins().iconst(pointer, 0)),
        mir::NullableWeakReferenceExpression::Weak(value) => {
            lower_weak_reference_expression(builder, value, resources)
        }
        mir::NullableWeakReferenceExpression::Local {
            local, transfer, ..
        } => {
            let slot = local_slot(resources.local_slots, *local)?;
            let value = builder.ins().stack_load(pointer, pointer, slot, 0);
            if *transfer {
                let zero = builder.ins().iconst(pointer, 0);
                builder.ins().stack_store(pointer, zero, slot, 0);
            }
            Ok(value)
        }
        mir::NullableWeakReferenceExpression::Property {
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
        mir::NullableWeakReferenceExpression::Call { function, args, .. } => {
            lower_function_call(builder, *function, args, resources)?
                .ok_or_else(|| malformed_mir("nullable weak call produced no result"))?
                .single()
        }
        mir::NullableWeakReferenceExpression::NullSafeCreate { value, .. } => {
            let owned = value.owned_temporary().is_some();
            let value = lower_nullable_shared_reference_expression(builder, value, resources)?;
            let result = lower_null_safe_shared_call(
                builder,
                value,
                SHARED_CREATE_WEAK,
                "null-safe weak creation",
                true,
                resources,
            )?;
            if owned {
                defer_or_drop_shared_temporary(builder, value, false, resources)?;
            }
            Ok(result)
        }
        mir::NullableWeakReferenceExpression::Coalesce {
            left,
            right,
            transfer,
            ..
        } => {
            let left_owned = left.owned_temporary().is_some();
            let right_owned = right.owned_temporary().is_some();
            let left = lower_nullable_weak_reference_expression(builder, left, resources)?;
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
            let left_temporary = if left_owned && !transfer { left } else { zero };
            builder.ins().jump(
                done,
                &[BlockArg::Value(left), BlockArg::Value(left_temporary)],
            );
            builder.switch_to_block(right_block);
            let right = lower_nullable_weak_reference_expression(builder, right, resources)?;
            let right_temporary = if right_owned && !transfer {
                right
            } else {
                zero
            };
            builder.ins().jump(
                done,
                &[BlockArg::Value(right), BlockArg::Value(right_temporary)],
            );
            builder.switch_to_block(done);
            let result = builder.block_params(done)[0];
            if !transfer && (left_owned || right_owned) {
                defer_or_drop_shared_temporary(
                    builder,
                    builder.block_params(done)[1],
                    true,
                    resources,
                )?;
            }
            Ok(result)
        }
        mir::NullableWeakReferenceExpression::DictionaryGet {
            class,
            collection,
            key,
            access,
            stored_nullable,
        } => {
            let (_, payload) = lower_dictionary_get(
                builder,
                *collection,
                key,
                if *stored_nullable {
                    mir::Type::NullableWeakReference(*class)
                } else {
                    mir::Type::WeakReference(*class)
                },
                *access,
                resources,
            )?;
            Ok(payload)
        }
        mir::NullableWeakReferenceExpression::CollectionIndex {
            collection,
            index,
            remove,
            positional,
            ..
        } => lower_collection_index(builder, *collection, index, *remove, *positional, resources),
    }
}

fn lower_writable_shared_reference_expression(
    builder: &mut FunctionBuilder,
    expression: &mir::WritableSharedReferenceExpression,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    match expression {
        mir::WritableSharedReferenceExpression::New { payload, value } => {
            let value = lower_rvalue(builder, value, resources)?.single()?;
            let drop_function = match payload {
                mir::WritableSharedPayload::Class(class) => {
                    let id = *resources
                        .class_drop_function_ids
                        .get(class.0)
                        .ok_or_else(|| malformed_mir("writable shared drop glue does not exist"))?;
                    resources.module.declare_func_in_func(id, builder.func)
                }
                mir::WritableSharedPayload::Collection(_) => {
                    let mir::WritableSharedPayload::Collection(collection) = payload else {
                        unreachable!()
                    };
                    let id = *resources
                        .collection_drop_function_ids
                        .get(collection.0)
                        .ok_or_else(|| {
                            malformed_mir("writable shared collection drop glue does not exist")
                        })?;
                    resources.module.declare_func_in_func(id, builder.func)
                }
            };
            let drop_function = builder.ins().func_addr(pointer, drop_function);
            runtime_call(
                builder,
                WRITABLE_SHARED_CREATE,
                &[pointer, pointer, pointer],
                Some(pointer),
                &[resources.current_frame, value, drop_function],
                resources,
            )?
            .ok_or_else(|| backend_failure("writable shared construction produced no result"))
        }
        mir::WritableSharedReferenceExpression::Local {
            local, transfer, ..
        }
        | mir::WritableSharedReferenceExpression::NullableLocalAssumeNonNull {
            local,
            transfer,
            ..
        } => lower_pointer_local(builder, *local, *transfer, resources),
        mir::WritableSharedReferenceExpression::Property {
            object, property, ..
        } => lower_pointer_property(builder, *object, *property, resources),
        mir::WritableSharedReferenceExpression::Call { function, args, .. } => {
            lower_function_call(builder, *function, args, resources)?
                .ok_or_else(|| malformed_mir("writable shared call produced no result"))?
                .single()
        }
        mir::WritableSharedReferenceExpression::Share { value, .. } => {
            let owned = value.owned_temporary();
            let control = lower_writable_shared_reference_expression(builder, value, resources)?;
            let shared = runtime_call(
                builder,
                WRITABLE_SHARED_RETAIN,
                &[pointer, pointer],
                Some(pointer),
                &[resources.current_frame, control],
                resources,
            )?
            .ok_or_else(|| backend_failure("writable shared retain produced no result"))?;
            if owned {
                defer_or_drop_writable_shared_temporary(
                    builder,
                    control,
                    WRITABLE_SHARED_RELEASE,
                    resources,
                )?;
            }
            Ok(shared)
        }
        mir::WritableSharedReferenceExpression::Coalesce {
            left,
            right,
            transfer,
            ..
        } => lower_writable_shared_coalesce(
            builder,
            left,
            right,
            *transfer,
            WRITABLE_SHARED_RELEASE,
            resources,
        ),
        mir::WritableSharedReferenceExpression::CollectionIndex {
            collection,
            index,
            remove,
            positional,
            ..
        } => lower_collection_index(builder, *collection, index, *remove, *positional, resources),
    }
}

fn lower_writable_weak_reference_expression(
    builder: &mut FunctionBuilder,
    expression: &mir::WritableWeakReferenceExpression,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    match expression {
        mir::WritableWeakReferenceExpression::Local {
            local, transfer, ..
        }
        | mir::WritableWeakReferenceExpression::NullableLocalAssumeNonNull {
            local,
            transfer,
            ..
        } => lower_pointer_local(builder, *local, *transfer, resources),
        mir::WritableWeakReferenceExpression::Property {
            object, property, ..
        } => lower_pointer_property(builder, *object, *property, resources),
        mir::WritableWeakReferenceExpression::Call { function, args, .. } => {
            lower_function_call(builder, *function, args, resources)?
                .ok_or_else(|| malformed_mir("writable weak call produced no result"))?
                .single()
        }
        mir::WritableWeakReferenceExpression::Create { value, .. } => {
            let owned = value.owned_temporary();
            let control = lower_writable_shared_reference_expression(builder, value, resources)?;
            let weak = runtime_call(
                builder,
                WRITABLE_SHARED_CREATE_WEAK,
                &[pointer, pointer],
                Some(pointer),
                &[resources.current_frame, control],
                resources,
            )?
            .ok_or_else(|| backend_failure("writable weak creation produced no result"))?;
            if owned {
                defer_or_drop_writable_shared_temporary(
                    builder,
                    control,
                    WRITABLE_SHARED_RELEASE,
                    resources,
                )?;
            }
            Ok(weak)
        }
        mir::WritableWeakReferenceExpression::Coalesce {
            left,
            right,
            transfer,
            ..
        } => lower_writable_weak_coalesce(
            builder,
            left,
            right,
            *transfer,
            WRITABLE_SHARED_RELEASE_WEAK,
            resources,
        ),
        mir::WritableWeakReferenceExpression::CollectionIndex {
            collection,
            index,
            remove,
            positional,
            ..
        } => lower_collection_index(builder, *collection, index, *remove, *positional, resources),
    }
}

fn lower_nullable_writable_shared_reference_expression(
    builder: &mut FunctionBuilder,
    expression: &mir::NullableWritableSharedReferenceExpression,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    match expression {
        mir::NullableWritableSharedReferenceExpression::Null(_) => {
            Ok(builder.ins().iconst(pointer, 0))
        }
        mir::NullableWritableSharedReferenceExpression::Strong(value) => {
            lower_writable_shared_reference_expression(builder, value, resources)
        }
        mir::NullableWritableSharedReferenceExpression::Local {
            local, transfer, ..
        } => lower_pointer_local(builder, *local, *transfer, resources),
        mir::NullableWritableSharedReferenceExpression::Property {
            object, property, ..
        } => lower_pointer_property(builder, *object, *property, resources),
        mir::NullableWritableSharedReferenceExpression::Call { function, args, .. } => {
            lower_function_call(builder, *function, args, resources)?
                .ok_or_else(|| malformed_mir("nullable writable shared call produced no result"))?
                .single()
        }
        mir::NullableWritableSharedReferenceExpression::Acquire { value, .. } => {
            let owned = value.owned_temporary();
            let control = lower_writable_weak_reference_expression(builder, value, resources)?;
            let acquired = runtime_call(
                builder,
                WRITABLE_SHARED_ACQUIRE,
                &[pointer, pointer],
                Some(pointer),
                &[resources.current_frame, control],
                resources,
            )?
            .ok_or_else(|| backend_failure("writable weak acquisition produced no result"))?;
            if owned {
                defer_or_drop_writable_shared_temporary(
                    builder,
                    control,
                    WRITABLE_SHARED_RELEASE_WEAK,
                    resources,
                )?;
            }
            Ok(acquired)
        }
        mir::NullableWritableSharedReferenceExpression::NullSafeShare { value, .. } => {
            let owned = value.owned_temporary();
            let control =
                lower_nullable_writable_shared_reference_expression(builder, value, resources)?;
            let result = lower_null_safe_shared_call(
                builder,
                control,
                WRITABLE_SHARED_RETAIN,
                "null-safe writable shared retain",
                true,
                resources,
            )?;
            if owned {
                defer_or_drop_writable_shared_temporary(
                    builder,
                    control,
                    WRITABLE_SHARED_RELEASE,
                    resources,
                )?;
            }
            Ok(result)
        }
        mir::NullableWritableSharedReferenceExpression::NullSafeAcquire { value, .. } => {
            let owned = value.owned_temporary();
            let control =
                lower_nullable_writable_weak_reference_expression(builder, value, resources)?;
            let result = lower_null_safe_shared_call(
                builder,
                control,
                WRITABLE_SHARED_ACQUIRE,
                "null-safe writable weak acquisition",
                true,
                resources,
            )?;
            if owned {
                defer_or_drop_writable_shared_temporary(
                    builder,
                    control,
                    WRITABLE_SHARED_RELEASE_WEAK,
                    resources,
                )?;
            }
            Ok(result)
        }
        mir::NullableWritableSharedReferenceExpression::Coalesce {
            left,
            right,
            transfer,
            ..
        } => lower_nullable_writable_shared_coalesce(
            builder,
            left,
            right,
            *transfer,
            WRITABLE_SHARED_RELEASE,
            resources,
        ),
        mir::NullableWritableSharedReferenceExpression::DictionaryGet {
            payload,
            collection,
            key,
            access,
            stored_nullable,
        } => {
            let (_, value) = lower_dictionary_get(
                builder,
                *collection,
                key,
                if *stored_nullable {
                    mir::Type::NullableWritableSharedReference(*payload)
                } else {
                    mir::Type::WritableSharedReference(*payload)
                },
                *access,
                resources,
            )?;
            Ok(value)
        }
    }
}

fn lower_nullable_writable_weak_reference_expression(
    builder: &mut FunctionBuilder,
    expression: &mir::NullableWritableWeakReferenceExpression,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    match expression {
        mir::NullableWritableWeakReferenceExpression::Null(_) => {
            Ok(builder.ins().iconst(pointer, 0))
        }
        mir::NullableWritableWeakReferenceExpression::Weak(value) => {
            lower_writable_weak_reference_expression(builder, value, resources)
        }
        mir::NullableWritableWeakReferenceExpression::Local {
            local, transfer, ..
        } => lower_pointer_local(builder, *local, *transfer, resources),
        mir::NullableWritableWeakReferenceExpression::Property {
            object, property, ..
        } => lower_pointer_property(builder, *object, *property, resources),
        mir::NullableWritableWeakReferenceExpression::Call { function, args, .. } => {
            lower_function_call(builder, *function, args, resources)?
                .ok_or_else(|| malformed_mir("nullable writable weak call produced no result"))?
                .single()
        }
        mir::NullableWritableWeakReferenceExpression::NullSafeCreate { value, .. } => {
            let owned = value.owned_temporary();
            let control =
                lower_nullable_writable_shared_reference_expression(builder, value, resources)?;
            let result = lower_null_safe_shared_call(
                builder,
                control,
                WRITABLE_SHARED_CREATE_WEAK,
                "null-safe writable weak creation",
                true,
                resources,
            )?;
            if owned {
                defer_or_drop_writable_shared_temporary(
                    builder,
                    control,
                    WRITABLE_SHARED_RELEASE,
                    resources,
                )?;
            }
            Ok(result)
        }
        mir::NullableWritableWeakReferenceExpression::Coalesce {
            left,
            right,
            transfer,
            ..
        } => lower_nullable_writable_weak_coalesce(
            builder,
            left,
            right,
            *transfer,
            WRITABLE_SHARED_RELEASE_WEAK,
            resources,
        ),
        mir::NullableWritableWeakReferenceExpression::DictionaryGet {
            payload,
            collection,
            key,
            access,
            stored_nullable,
        } => {
            let (_, value) = lower_dictionary_get(
                builder,
                *collection,
                key,
                if *stored_nullable {
                    mir::Type::NullableWritableWeakReference(*payload)
                } else {
                    mir::Type::WritableWeakReference(*payload)
                },
                *access,
                resources,
            )?;
            Ok(value)
        }
    }
}

fn lower_shared_reference_access_expression(
    builder: &mut FunctionBuilder,
    expression: &mir::SharedReferenceAccessExpression,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    match expression {
        mir::SharedReferenceAccessExpression::Local {
            local, transfer, ..
        }
        | mir::SharedReferenceAccessExpression::NullableLocalAssumeNonNull {
            local,
            transfer,
            ..
        } => lower_pointer_local(builder, *local, *transfer, resources),
        mir::SharedReferenceAccessExpression::Property {
            object, property, ..
        } => lower_pointer_property(builder, *object, *property, resources),
        mir::SharedReferenceAccessExpression::CollectionIndex {
            collection,
            index,
            remove,
            positional,
            ..
        } => lower_collection_index(builder, *collection, index, *remove, *positional, resources),
        mir::SharedReferenceAccessExpression::Call { function, args, .. } => {
            lower_function_call(builder, *function, args, resources)?
                .ok_or_else(|| malformed_mir("shared access call produced no result"))?
                .single()
        }
        mir::SharedReferenceAccessExpression::Acquire {
            value,
            writable,
            span,
            ..
        } => {
            let owned = value.owned_temporary();
            let control = lower_writable_shared_reference_expression(builder, value, resources)?;
            set_active_panic_site(builder, *span, resources);
            let symbol = if *writable {
                WRITABLE_SHARED_ACQUIRE_WRITABLE_ACCESS
            } else {
                WRITABLE_SHARED_ACQUIRE_READONLY_ACCESS
            };
            let access = runtime_call(
                builder,
                symbol,
                &[pointer, pointer],
                Some(pointer),
                &[resources.current_frame, control],
                resources,
            )?
            .ok_or_else(|| backend_failure("shared access acquisition produced no result"))?;
            if owned {
                defer_or_drop_writable_shared_temporary(
                    builder,
                    control,
                    WRITABLE_SHARED_RELEASE,
                    resources,
                )?;
            }
            Ok(access)
        }
    }
}

fn lower_nullable_shared_reference_access_expression(
    builder: &mut FunctionBuilder,
    expression: &mir::NullableSharedReferenceAccessExpression,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    match expression {
        mir::NullableSharedReferenceAccessExpression::Null { .. } => {
            Ok(builder.ins().iconst(pointer, 0))
        }
        mir::NullableSharedReferenceAccessExpression::Access(value) => {
            lower_shared_reference_access_expression(builder, value, resources)
        }
        mir::NullableSharedReferenceAccessExpression::Local {
            local, transfer, ..
        } => lower_pointer_local(builder, *local, *transfer, resources),
        mir::NullableSharedReferenceAccessExpression::Property {
            object, property, ..
        } => lower_pointer_property(builder, *object, *property, resources),
        mir::NullableSharedReferenceAccessExpression::CollectionIndex {
            collection,
            index,
            remove,
            positional,
            ..
        } => lower_collection_index(builder, *collection, index, *remove, *positional, resources),
        mir::NullableSharedReferenceAccessExpression::CollectionGet {
            collection,
            key,
            access,
            stored,
        } => {
            let (_, value) = lower_dictionary_get(
                builder,
                *collection,
                key,
                stored.into_type(),
                *access,
                resources,
            )?;
            Ok(value)
        }
        mir::NullableSharedReferenceAccessExpression::Call { function, args, .. } => {
            lower_function_call(builder, *function, args, resources)?
                .ok_or_else(|| malformed_mir("nullable shared access call produced no result"))?
                .single()
        }
        mir::NullableSharedReferenceAccessExpression::NullSafeAcquire {
            value,
            writable,
            span,
            ..
        } => {
            let owned = value.owned_temporary();
            let control =
                lower_nullable_writable_shared_reference_expression(builder, value, resources)?;
            set_active_panic_site(builder, *span, resources);
            let symbol = if *writable {
                WRITABLE_SHARED_ACQUIRE_WRITABLE_ACCESS
            } else {
                WRITABLE_SHARED_ACQUIRE_READONLY_ACCESS
            };
            let result = lower_null_safe_shared_call(
                builder,
                control,
                symbol,
                "null-safe shared access acquisition",
                true,
                resources,
            )?;
            if owned {
                defer_or_drop_writable_shared_temporary(
                    builder,
                    control,
                    WRITABLE_SHARED_RELEASE,
                    resources,
                )?;
            }
            Ok(result)
        }
    }
}

fn lower_pointer_local(
    builder: &mut FunctionBuilder,
    local: mir::LocalId,
    transfer: bool,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let slot = local_slot(resources.local_slots, local)?;
    let value = builder.ins().stack_load(pointer, pointer, slot, 0);
    if transfer {
        let zero = builder.ins().iconst(pointer, 0);
        builder.ins().stack_store(pointer, zero, slot, 0);
    }
    Ok(value)
}

fn lower_pointer_property(
    builder: &mut FunctionBuilder,
    object: mir::LocalId,
    property: crate::class_layout::PropertyId,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let address = lower_property_address(builder, object, property, resources)?;
    Ok(builder.ins().load(
        pointer,
        cranelift_codegen::ir::MachMemFlags::trusted(),
        address,
        0,
    ))
}

struct WritablePointerCoalesce {
    left: Value,
    left_owned: bool,
    right: Value,
    right_owned: bool,
    transfer: bool,
    release: &'static str,
    left_block: Block,
    right_block: Block,
    done: Block,
}

fn finish_writable_pointer_coalesce(
    builder: &mut FunctionBuilder,
    coalesce: WritablePointerCoalesce,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let WritablePointerCoalesce {
        left,
        left_owned,
        right,
        right_owned,
        transfer,
        release,
        left_block,
        right_block,
        done,
    } = coalesce;
    let pointer = resources.module.target_config().pointer_type();
    let zero = builder.ins().iconst(pointer, 0);
    builder.switch_to_block(left_block);
    let left_temporary = if left_owned && !transfer { left } else { zero };
    builder.ins().jump(
        done,
        &[BlockArg::Value(left), BlockArg::Value(left_temporary)],
    );
    builder.switch_to_block(right_block);
    let right_temporary = if right_owned && !transfer {
        right
    } else {
        zero
    };
    builder.ins().jump(
        done,
        &[BlockArg::Value(right), BlockArg::Value(right_temporary)],
    );
    builder.switch_to_block(done);
    let result = builder.block_params(done)[0];
    if !transfer && (left_owned || right_owned) {
        defer_or_drop_writable_shared_temporary(
            builder,
            builder.block_params(done)[1],
            release,
            resources,
        )?;
    }
    Ok(result)
}

fn begin_writable_pointer_coalesce(
    builder: &mut FunctionBuilder,
    left: Value,
    resources: &mut LoweringResources<'_, '_>,
) -> (Block, Block, Block) {
    let pointer = resources.module.target_config().pointer_type();
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
    (left_block, right_block, done)
}

fn lower_writable_shared_coalesce(
    builder: &mut FunctionBuilder,
    left: &mir::NullableWritableSharedReferenceExpression,
    right: &mir::WritableSharedReferenceExpression,
    transfer: bool,
    release: &'static str,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let left_owned = left.owned_temporary();
    let right_owned = right.owned_temporary();
    let left_value = lower_nullable_writable_shared_reference_expression(builder, left, resources)?;
    let (left_block, right_block, done) =
        begin_writable_pointer_coalesce(builder, left_value, resources);
    builder.switch_to_block(right_block);
    let right_value = lower_writable_shared_reference_expression(builder, right, resources)?;
    finish_writable_pointer_coalesce(
        builder,
        WritablePointerCoalesce {
            left: left_value,
            left_owned,
            right: right_value,
            right_owned,
            transfer,
            release,
            left_block,
            right_block,
            done,
        },
        resources,
    )
}

fn lower_nullable_writable_shared_coalesce(
    builder: &mut FunctionBuilder,
    left: &mir::NullableWritableSharedReferenceExpression,
    right: &mir::NullableWritableSharedReferenceExpression,
    transfer: bool,
    release: &'static str,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let left_owned = left.owned_temporary();
    let right_owned = right.owned_temporary();
    let left_value = lower_nullable_writable_shared_reference_expression(builder, left, resources)?;
    let (left_block, right_block, done) =
        begin_writable_pointer_coalesce(builder, left_value, resources);
    builder.switch_to_block(right_block);
    let right_value =
        lower_nullable_writable_shared_reference_expression(builder, right, resources)?;
    finish_writable_pointer_coalesce(
        builder,
        WritablePointerCoalesce {
            left: left_value,
            left_owned,
            right: right_value,
            right_owned,
            transfer,
            release,
            left_block,
            right_block,
            done,
        },
        resources,
    )
}

fn lower_writable_weak_coalesce(
    builder: &mut FunctionBuilder,
    left: &mir::NullableWritableWeakReferenceExpression,
    right: &mir::WritableWeakReferenceExpression,
    transfer: bool,
    release: &'static str,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let left_owned = left.owned_temporary();
    let right_owned = right.owned_temporary();
    let left_value = lower_nullable_writable_weak_reference_expression(builder, left, resources)?;
    let (left_block, right_block, done) =
        begin_writable_pointer_coalesce(builder, left_value, resources);
    builder.switch_to_block(right_block);
    let right_value = lower_writable_weak_reference_expression(builder, right, resources)?;
    finish_writable_pointer_coalesce(
        builder,
        WritablePointerCoalesce {
            left: left_value,
            left_owned,
            right: right_value,
            right_owned,
            transfer,
            release,
            left_block,
            right_block,
            done,
        },
        resources,
    )
}

fn lower_nullable_writable_weak_coalesce(
    builder: &mut FunctionBuilder,
    left: &mir::NullableWritableWeakReferenceExpression,
    right: &mir::NullableWritableWeakReferenceExpression,
    transfer: bool,
    release: &'static str,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let left_owned = left.owned_temporary();
    let right_owned = right.owned_temporary();
    let left_value = lower_nullable_writable_weak_reference_expression(builder, left, resources)?;
    let (left_block, right_block, done) =
        begin_writable_pointer_coalesce(builder, left_value, resources);
    builder.switch_to_block(right_block);
    let right_value = lower_nullable_writable_weak_reference_expression(builder, right, resources)?;
    finish_writable_pointer_coalesce(
        builder,
        WritablePointerCoalesce {
            left: left_value,
            left_owned,
            right: right_value,
            right_owned,
            transfer,
            release,
            left_block,
            right_block,
            done,
        },
        resources,
    )
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
        mir::NullableClassExpression::SharedPayload { reference, .. } => {
            let owned = reference.owned_temporary().is_some();
            let control =
                lower_nullable_shared_reference_expression(builder, reference, resources)?;
            let payload = lower_null_safe_shared_call(
                builder,
                control,
                SHARED_PAYLOAD,
                "nullable shared payload projection",
                false,
                resources,
            )?;
            if owned {
                defer_or_drop_shared_temporary(builder, control, false, resources)?;
            }
            Ok(payload)
        }
        mir::NullableClassExpression::Local {
            local, transfer, ..
        } => {
            let slot = local_slot(resources.local_slots, *local)?;
            let value = builder.ins().stack_load(pointer, pointer, slot, 0);
            if *transfer {
                let zero = builder.ins().iconst(pointer, 0);
                builder.ins().stack_store(pointer, zero, slot, 0);
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
            class,
            left,
            right,
            transfer,
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
            if !transfer && (left_owned || right_owned) {
                defer_or_drop_class_temporary(
                    builder,
                    builder.block_params(done)[1],
                    *class,
                    resources,
                )?;
            }
            Ok(result)
        }
        mir::NullableClassExpression::DictionaryGet {
            class,
            collection,
            key,
            access,
        } => {
            let (_, payload) = lower_dictionary_get(
                builder,
                *collection,
                key,
                mir::Type::Class(*class),
                *access,
                resources,
            )?;
            Ok(payload)
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
    let drop_function = *resources
        .class_drop_function_ids
        .get(class.0)
        .ok_or_else(|| malformed_mir(format!("class{} drop function does not exist", class.0)))?;
    let drop_function = resources
        .module
        .declare_func_in_func(drop_function, builder.func);
    builder
        .ins()
        .call(drop_function, &[resources.current_frame, object]);
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
    lower_drop_class_value_impl(builder, object, class, true, resources)
}

fn lower_drop_failed_class_value(
    builder: &mut FunctionBuilder,
    object: Value,
    class: crate::class_layout::ClassId,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    lower_drop_class_value_impl(builder, object, class, false, resources)
}

fn lower_drop_class_value_impl(
    builder: &mut FunctionBuilder,
    object: Value,
    class: crate::class_layout::ClassId,
    run_destructor: bool,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let pointer_type = resources.module.target_config().pointer_type();
    let class_definition = class_definition(resources.program, class)?;
    if let Some(destructor) = class_definition.destructor.filter(|_| run_destructor) {
        let callee = declared_function(builder, resources, destructor)?;
        builder
            .ins()
            .call(callee, &[resources.current_frame, object]);
    }
    for property in class_definition.properties.iter().rev() {
        let address = lower_property_address_from_value(builder, object, property.id, resources)?;
        match property.ty {
            mir::Type::Error | mir::Type::NullableError => {
                let value = load_lowered_from_address(builder, property.ty, address, pointer_type);
                lower_drop_error_value(builder, value, resources)?;
            }
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
            mir::Type::Collection(collection) | mir::Type::NullableCollection(collection) => {
                let value = builder.ins().load(
                    pointer_type,
                    cranelift_codegen::ir::MachMemFlags::trusted(),
                    address,
                    0,
                );
                lower_drop_collection_value(builder, value, collection, resources)?;
            }
            mir::Type::Mixed | mir::Type::NullableMixed => {
                let value = builder.ins().load(
                    pointer_type,
                    cranelift_codegen::ir::MachMemFlags::trusted(),
                    address,
                    0,
                );
                lower_drop_mixed_value(builder, value, resources)?;
            }
            mir::Type::SharedReference(_) | mir::Type::NullableSharedReference(_) => {
                let value = builder.ins().load(
                    pointer_type,
                    cranelift_codegen::ir::MachMemFlags::trusted(),
                    address,
                    0,
                );
                lower_drop_shared_value(builder, value, false, resources)?;
            }
            mir::Type::WeakReference(_) | mir::Type::NullableWeakReference(_) => {
                let value = builder.ins().load(
                    pointer_type,
                    cranelift_codegen::ir::MachMemFlags::trusted(),
                    address,
                    0,
                );
                lower_drop_shared_value(builder, value, true, resources)?;
            }
            mir::Type::WritableSharedReference(_)
            | mir::Type::NullableWritableSharedReference(_)
            | mir::Type::WritableWeakReference(_)
            | mir::Type::NullableWritableWeakReference(_)
            | mir::Type::ReadonlySharedReferenceAccess(_)
            | mir::Type::WritableSharedReferenceAccess(_)
            | mir::Type::NullableReadonlySharedReferenceAccess(_)
            | mir::Type::NullableWritableSharedReferenceAccess(_) => {
                let value = builder.ins().load(
                    pointer_type,
                    cranelift_codegen::ir::MachMemFlags::trusted(),
                    address,
                    0,
                );
                let symbol = match property.ty {
                    mir::Type::WritableSharedReference(_)
                    | mir::Type::NullableWritableSharedReference(_) => WRITABLE_SHARED_RELEASE,
                    mir::Type::WritableWeakReference(_)
                    | mir::Type::NullableWritableWeakReference(_) => WRITABLE_SHARED_RELEASE_WEAK,
                    mir::Type::ReadonlySharedReferenceAccess(_)
                    | mir::Type::NullableReadonlySharedReferenceAccess(_) => {
                        WRITABLE_SHARED_RELEASE_READONLY_ACCESS
                    }
                    mir::Type::WritableSharedReferenceAccess(_)
                    | mir::Type::NullableWritableSharedReferenceAccess(_) => {
                        WRITABLE_SHARED_RELEASE_WRITABLE_ACCESS
                    }
                    _ => unreachable!(),
                };
                lower_drop_writable_shared_value(builder, value, symbol, resources)?;
            }
            mir::Type::Scalar(_) | mir::Type::NullableScalar(_) => {}
            mir::Type::PayloadEnum(payload) => {
                lower_drop_payload_enum_at(builder, address, payload, false, resources)?;
            }
            mir::Type::NullablePayloadEnum(payload) => {
                lower_drop_payload_enum_at(builder, address, payload, true, resources)?;
            }
            mir::Type::Function(_) | mir::Type::NullableFunction(_) => {
                let value = load_lowered_from_address(builder, property.ty, address, pointer_type);
                lower_drop_function_carrier(builder, value, resources)?;
            }
            mir::Type::ClosureEnvironment(_) => {
                return Err(malformed_mir(
                    "closure environment pointer reached a Doria property",
                ));
            }
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

fn lower_drop_shared_value(
    builder: &mut FunctionBuilder,
    value: Value,
    weak: bool,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let zero = builder.ins().iconst(pointer, 0);
    let present = builder.ins().icmp(IntCC::NotEqual, value, zero);
    let drop_block = builder.create_block();
    let done = builder.create_block();
    builder.ins().brif(present, drop_block, &[], done, &[]);
    builder.switch_to_block(drop_block);
    if weak {
        runtime_call(
            builder,
            SHARED_RELEASE_WEAK,
            &[pointer],
            None,
            &[value],
            resources,
        )?;
    } else {
        runtime_call(
            builder,
            SHARED_RELEASE,
            &[pointer, pointer],
            None,
            &[resources.current_frame, value],
            resources,
        )?;
    }
    builder.ins().jump(done, &[]);
    builder.switch_to_block(done);
    Ok(())
}

fn lower_drop_writable_shared_local(
    builder: &mut FunctionBuilder,
    local: mir::LocalId,
    symbol: &'static str,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let slot = local_slot(resources.local_slots, local)?;
    let value = builder.ins().stack_load(pointer, pointer, slot, 0);
    let zero = builder.ins().iconst(pointer, 0);
    builder.ins().stack_store(pointer, zero, slot, 0);
    lower_drop_writable_shared_value(builder, value, symbol, resources)
}

fn lower_drop_writable_shared_value(
    builder: &mut FunctionBuilder,
    value: Value,
    symbol: &'static str,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let zero = builder.ins().iconst(pointer, 0);
    let present = builder.ins().icmp(IntCC::NotEqual, value, zero);
    let drop_block = builder.create_block();
    let done = builder.create_block();
    builder.ins().brif(present, drop_block, &[], done, &[]);
    builder.switch_to_block(drop_block);
    if symbol == WRITABLE_SHARED_RELEASE_WEAK {
        runtime_call(builder, symbol, &[pointer], None, &[value], resources)?;
    } else {
        runtime_call(
            builder,
            symbol,
            &[pointer, pointer],
            None,
            &[resources.current_frame, value],
            resources,
        )?;
    }
    builder.ins().jump(done, &[]);
    builder.switch_to_block(done);
    Ok(())
}

fn lower_drop_mixed_value(
    builder: &mut FunctionBuilder,
    value: Value,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let zero = builder.ins().iconst(pointer, 0);
    let has_box = builder.ins().icmp(IntCC::NotEqual, value, zero);
    let drop_block = builder.create_block();
    let done = builder.create_block();
    builder.ins().brif(has_box, drop_block, &[], done, &[]);
    builder.switch_to_block(drop_block);
    let release_payload = runtime_call(
        builder,
        MIXED_RELEASE_OWNED,
        &[pointer],
        Some(types::I8),
        &[value],
        resources,
    )?
    .ok_or_else(|| backend_failure("mixed ownership release produced no result"))?;
    let drop_payload = builder.create_block();
    let free_shell = builder.create_block();
    builder
        .ins()
        .brif(release_payload, drop_payload, &[], free_shell, &[]);
    builder.switch_to_block(drop_payload);

    let tag = runtime_call(
        builder,
        MIXED_TAG,
        &[pointer],
        Some(types::I8),
        &[value],
        resources,
    )?
    .ok_or_else(|| backend_failure("mixed tag read produced no result"))?;
    let payload = runtime_call(
        builder,
        MIXED_PAYLOAD,
        &[pointer],
        Some(types::I64),
        &[value],
        resources,
    )?
    .ok_or_else(|| backend_failure("mixed payload read produced no result"))?;

    let string_block = builder.create_block();
    let after_string = builder.create_block();
    let string_tag = builder.ins().iconst(types::I8, i64::from(MIXED_TAG_STRING));
    let is_string = builder.ins().icmp(IntCC::Equal, tag, string_tag);
    builder
        .ins()
        .brif(is_string, string_block, &[], after_string, &[]);
    builder.switch_to_block(string_block);
    let string = collection_word_to_value(builder, payload, mir::Type::String, pointer)?;
    release_string(builder, string, resources)?;
    builder.ins().jump(after_string, &[]);

    builder.switch_to_block(after_string);
    let function_block = builder.create_block();
    let after_function = builder.create_block();
    let function_tag = builder
        .ins()
        .iconst(types::I8, i64::from(MIXED_TAG_FUNCTION));
    let is_function = builder.ins().icmp(IntCC::Equal, tag, function_tag);
    builder
        .ins()
        .brif(is_function, function_block, &[], after_function, &[]);
    builder.switch_to_block(function_block);
    let carrier_address = collection_word_to_value(
        builder,
        payload,
        mir::Type::Class(crate::class_layout::ClassId(0)),
        pointer,
    )?;
    let carrier = load_lowered_from_address(
        builder,
        mir::Type::Function(mir::FunctionTypeId(0)),
        carrier_address,
        pointer,
    );
    lower_drop_function_carrier(builder, carrier, resources)?;
    builder.ins().jump(after_function, &[]);

    builder.switch_to_block(after_function);
    let class_block = builder.create_block();
    let after_class = builder.create_block();
    let class_tag = builder.ins().iconst(types::I8, i64::from(MIXED_TAG_CLASS));
    let is_class = builder.ins().icmp(IntCC::Equal, tag, class_tag);
    builder
        .ins()
        .brif(is_class, class_block, &[], after_class, &[]);
    builder.switch_to_block(class_block);
    let type_id = runtime_call(
        builder,
        MIXED_TYPE_ID,
        &[pointer],
        Some(types::I32),
        &[value],
        resources,
    )?
    .ok_or_else(|| backend_failure("mixed type-id read produced no result"))?;
    let object = collection_word_to_value(
        builder,
        payload,
        mir::Type::Class(crate::class_layout::ClassId(0)),
        pointer,
    )?;
    let classes = resources
        .program
        .classes
        .iter()
        .map(|class| class.id)
        .collect::<Vec<_>>();
    if classes.is_empty() {
        builder.ins().jump(after_class, &[]);
    } else {
        let checks = classes
            .iter()
            .map(|_| builder.create_block())
            .collect::<Vec<_>>();
        builder.ins().jump(checks[0], &[]);
        for (index, class) in classes.iter().enumerate() {
            let check = checks[index];
            let next = checks.get(index + 1).copied().unwrap_or(after_class);
            builder.switch_to_block(check);
            let expected = builder.ins().iconst(types::I32, class.0 as i64);
            let matches = builder.ins().icmp(IntCC::Equal, type_id, expected);
            let drop_class = builder.create_block();
            builder.ins().brif(matches, drop_class, &[], next, &[]);
            builder.switch_to_block(drop_class);
            lower_drop_class_value_checked(builder, object, *class, resources)?;
            builder.ins().jump(after_class, &[]);
        }
    }

    builder.switch_to_block(after_class);
    let payload_enum_block = builder.create_block();
    let after_payload_enum = builder.create_block();
    let payload_enum_tag = builder
        .ins()
        .iconst(types::I8, i64::from(MIXED_TAG_PAYLOAD_ENUM));
    let is_payload_enum = builder.ins().icmp(IntCC::Equal, tag, payload_enum_tag);
    builder.ins().brif(
        is_payload_enum,
        payload_enum_block,
        &[],
        after_payload_enum,
        &[],
    );
    builder.switch_to_block(payload_enum_block);
    let type_id = runtime_call(
        builder,
        MIXED_TYPE_ID,
        &[pointer],
        Some(types::I32),
        &[value],
        resources,
    )?
    .ok_or_else(|| backend_failure("mixed payload-enum type-id read produced no result"))?;
    let payload_address = collection_word_to_value(
        builder,
        payload,
        mir::Type::Class(crate::class_layout::ClassId(0)),
        pointer,
    )?;
    let payload_enums = resources
        .program
        .enums
        .iter()
        .filter_map(|definition| definition.payload_type())
        .collect::<Vec<_>>();
    if payload_enums.is_empty() {
        builder.ins().jump(after_payload_enum, &[]);
    } else {
        let checks = payload_enums
            .iter()
            .map(|_| builder.create_block())
            .collect::<Vec<_>>();
        builder.ins().jump(checks[0], &[]);
        for (index, payload_ty) in payload_enums.iter().enumerate() {
            let check = checks[index];
            let next = checks.get(index + 1).copied().unwrap_or(after_payload_enum);
            builder.switch_to_block(check);
            let expected = builder.ins().iconst(types::I32, payload_ty.id.0 as i64);
            let matches = builder.ins().icmp(IntCC::Equal, type_id, expected);
            let drop_payload = builder.create_block();
            builder.ins().brif(matches, drop_payload, &[], next, &[]);
            builder.switch_to_block(drop_payload);
            lower_drop_payload_enum_at(builder, payload_address, *payload_ty, false, resources)?;
            builder.ins().jump(after_payload_enum, &[]);
        }
    }
    builder.switch_to_block(after_payload_enum);
    builder.ins().jump(free_shell, &[]);
    builder.switch_to_block(free_shell);
    runtime_call(builder, MIXED_FREE, &[pointer], None, &[value], resources)?;
    builder.ins().jump(done, &[]);
    builder.switch_to_block(done);
    Ok(())
}

fn lower_free_mixed_shell(
    builder: &mut FunctionBuilder,
    value: Value,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    runtime_call(builder, MIXED_FREE, &[pointer], None, &[value], resources)?;
    Ok(())
}

fn lower_cleanup_mixed_temporary(
    builder: &mut FunctionBuilder,
    value: Value,
    ownership: mir::MixedOwnership,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    match ownership {
        mir::MixedOwnership::None => Ok(()),
        mir::MixedOwnership::ShellOnly => lower_free_mixed_shell(builder, value, resources),
        mir::MixedOwnership::Owned => lower_drop_mixed_value(builder, value, resources),
    }
}

fn lower_cleanup_mixed_temporary_if(
    builder: &mut FunctionBuilder,
    condition: Value,
    value: Value,
    ownership: mir::MixedOwnership,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    if !ownership.has_shell() {
        return Ok(());
    }
    let cleanup = builder.create_block();
    let done = builder.create_block();
    builder.ins().brif(condition, cleanup, &[], done, &[]);
    builder.switch_to_block(cleanup);
    lower_cleanup_mixed_temporary(builder, value, ownership, resources)?;
    builder.ins().jump(done, &[]);
    builder.switch_to_block(done);
    Ok(())
}

fn lower_own_mixed_value(
    builder: &mut FunctionBuilder,
    value: Value,
    ownership: mir::MixedOwnership,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    if ownership == mir::MixedOwnership::Owned {
        return Ok(value);
    }
    let pointer = resources.module.target_config().pointer_type();
    let owned = runtime_call(
        builder,
        MIXED_CLONE_OWNED,
        &[pointer],
        Some(pointer),
        &[value],
        resources,
    )?
    .ok_or_else(|| backend_failure("mixed ownership clone produced no result"))?;
    if ownership == mir::MixedOwnership::ShellOnly {
        lower_free_mixed_shell(builder, value, resources)?;
    }
    Ok(owned)
}

fn lower_own_nullable_mixed_value(
    builder: &mut FunctionBuilder,
    value: Value,
    ownership: mir::MixedOwnership,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    if ownership == mir::MixedOwnership::Owned {
        return Ok(value);
    }
    let pointer = resources.module.target_config().pointer_type();
    let null = builder.ins().iconst(pointer, 0);
    let present = builder.ins().icmp(IntCC::NotEqual, value, null);
    let own = builder.create_block();
    let done = builder.create_block();
    let result = builder.append_block_param(done, pointer);
    builder.ins().brif(present, own, &[], done, &[null.into()]);
    builder.switch_to_block(own);
    let owned = lower_own_mixed_value(builder, value, ownership, resources)?;
    builder.ins().jump(done, &[owned.into()]);
    builder.switch_to_block(done);
    Ok(result)
}

fn mixed_tag_value(tag: mir::MixedTag) -> (u8, u32) {
    match tag {
        mir::MixedTag::Bool => (MIXED_TAG_BOOL, 0),
        mir::MixedTag::Integer(IntegerType::Int8) => (MIXED_TAG_INT8, 0),
        mir::MixedTag::Integer(IntegerType::Int16) => (MIXED_TAG_INT16, 0),
        mir::MixedTag::Integer(IntegerType::Int32) => (MIXED_TAG_INT32, 0),
        mir::MixedTag::Integer(IntegerType::Int64) => (MIXED_TAG_INT64, 0),
        mir::MixedTag::Integer(IntegerType::UInt8) => (MIXED_TAG_UINT8, 0),
        mir::MixedTag::Integer(IntegerType::UInt16) => (MIXED_TAG_UINT16, 0),
        mir::MixedTag::Integer(IntegerType::UInt32) => (MIXED_TAG_UINT32, 0),
        mir::MixedTag::Integer(IntegerType::UInt64) => (MIXED_TAG_UINT64, 0),
        mir::MixedTag::Float(FloatType::Float32) => (MIXED_TAG_FLOAT32, 0),
        mir::MixedTag::Float(FloatType::Float64) => (MIXED_TAG_FLOAT64, 0),
        mir::MixedTag::String => (MIXED_TAG_STRING, 0),
        mir::MixedTag::Class(class) => (MIXED_TAG_CLASS, class.0 as u32),
        mir::MixedTag::Error => (MIXED_TAG_ERROR, 0),
        mir::MixedTag::Enum(enum_id) => (MIXED_TAG_ENUM, enum_id.0 as u32),
        mir::MixedTag::PayloadEnum(ty) => (MIXED_TAG_PAYLOAD_ENUM, ty.id.0 as u32),
        mir::MixedTag::Function(ty) => (MIXED_TAG_FUNCTION, ty.0 as u32),
    }
}

fn lower_mixed_box(
    builder: &mut FunctionBuilder,
    tag: mir::MixedTag,
    payload: Value,
    payload_owned: bool,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let payload_ty = tag.ty();
    let (tag_value, type_id) = mixed_tag_value(tag);
    let tag = builder.ins().iconst(types::I8, i64::from(tag_value));
    let type_id = builder.ins().iconst(types::I32, i64::from(type_id));
    let payload = value_to_collection_word(builder, payload, payload_ty, pointer)?;
    runtime_call(
        builder,
        if payload_owned {
            MIXED_NEW
        } else {
            MIXED_NEW_BORROWED
        },
        &[types::I8, types::I32, types::I64],
        Some(pointer),
        &[tag, type_id, payload],
        resources,
    )?
    .ok_or_else(|| backend_failure("mixed allocation produced no result"))
}

fn lower_mixed_expression(
    builder: &mut FunctionBuilder,
    expression: &mir::MixedExpression,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    match expression {
        mir::MixedExpression::Local { local, transfer } => {
            let slot = local_slot(resources.local_slots, *local)?;
            let value =
                load_lowered_from_stack(builder, mir::Type::Mixed, slot, pointer).single()?;
            if *transfer {
                let zero = builder.ins().iconst(pointer, 0);
                builder.ins().stack_store(pointer, zero, slot, 0);
            }
            Ok(value)
        }
        mir::MixedExpression::Property { object, property } => {
            let address = lower_property_address(builder, *object, *property, resources)?;
            Ok(load_lowered_from_address(builder, mir::Type::Mixed, address, pointer).single()?)
        }
        mir::MixedExpression::Call { function, args, .. } => {
            lower_function_call(builder, *function, args, resources)
                .and_then(|value| {
                    value.ok_or_else(|| malformed_mir("mixed call produced no result"))
                })
                .and_then(LoweredValue::single)
        }
        mir::MixedExpression::BoxValue(value) => {
            let tag = match value.ty() {
                mir::ScalarType::Bool => mir::MixedTag::Bool,
                mir::ScalarType::Integer(ty) => mir::MixedTag::Integer(ty),
                mir::ScalarType::Float(ty) => mir::MixedTag::Float(ty),
                mir::ScalarType::Enum(enum_id) => mir::MixedTag::Enum(enum_id),
            };
            let payload = lower_value_expression(builder, value, resources)?;
            lower_mixed_box(builder, tag, payload, false, resources)
        }
        mir::MixedExpression::BoxString {
            value,
            payload_owned,
        } => {
            let payload = lower_string_expression(builder, value, resources)?;
            let mixed = lower_mixed_box(
                builder,
                mir::MixedTag::String,
                payload,
                *payload_owned,
                resources,
            )?;
            if !payload_owned {
                release_string(builder, payload, resources)?;
            }
            Ok(mixed)
        }
        mir::MixedExpression::BoxClass {
            value,
            payload_owned,
        } => {
            let class = value.class();
            let payload = lower_class_expression(builder, value, resources)?;
            lower_mixed_box(
                builder,
                mir::MixedTag::Class(class),
                payload,
                *payload_owned,
                resources,
            )
        }
        mir::MixedExpression::BoxPayloadEnum { value } => {
            let ty = value.ty();
            let source = lower_payload_enum_expression(builder, value, resources)?;
            let (tag_value, type_id) = mixed_tag_value(mir::MixedTag::PayloadEnum(ty));
            let tag = builder.ins().iconst(types::I8, i64::from(tag_value));
            let type_id = builder.ins().iconst(types::I32, i64::from(type_id));
            let size = builder.ins().iconst(pointer, i64::from(ty.size));
            let alignment = builder.ins().iconst(pointer, i64::from(ty.align));
            runtime_call(
                builder,
                MIXED_NEW_AGGREGATE,
                &[types::I8, types::I32, pointer, pointer, pointer],
                Some(pointer),
                &[tag, type_id, source, size, alignment],
                resources,
            )?
            .ok_or_else(|| backend_failure("mixed aggregate allocation produced no result"))
        }
        mir::MixedExpression::BoxError { value } => {
            let value = lower_error_expression(builder, value, resources)?;
            let slot = builder.create_sized_stack_slot(StackSlotData::new(
                StackSlotKind::ExplicitSlot,
                pointer.bytes() * 2,
                pointer.bytes().trailing_zeros() as u8,
            ));
            store_lowered_to_stack(builder, mir::Type::Error, slot, value, pointer)?;
            let source = builder.ins().stack_addr(pointer, slot, 0);
            let tag = builder.ins().iconst(types::I8, i64::from(MIXED_TAG_ERROR));
            let type_id = builder.ins().iconst(types::I32, 0);
            let size = builder
                .ins()
                .iconst(pointer, i64::from(pointer.bytes() * 2));
            let alignment = builder.ins().iconst(pointer, i64::from(pointer.bytes()));
            runtime_call(
                builder,
                MIXED_NEW_AGGREGATE,
                &[types::I8, types::I32, pointer, pointer, pointer],
                Some(pointer),
                &[tag, type_id, source, size, alignment],
                resources,
            )?
            .ok_or_else(|| backend_failure("mixed Error allocation produced no result"))
        }
        mir::MixedExpression::BoxFunction {
            value,
            payload_owned,
        } => {
            let function_type = value.function_type();
            let value = lower_function_expression(builder, value, resources)?;
            let slot = builder.create_sized_stack_slot(StackSlotData::new(
                StackSlotKind::ExplicitSlot,
                pointer.bytes() * 2,
                pointer.bytes().trailing_zeros() as u8,
            ));
            store_lowered_to_stack(
                builder,
                mir::Type::Function(function_type),
                slot,
                value,
                pointer,
            )?;
            let source = builder.ins().stack_addr(pointer, slot, 0);
            let (tag_value, type_id) = mixed_tag_value(mir::MixedTag::Function(function_type));
            let tag = builder.ins().iconst(types::I8, i64::from(tag_value));
            let type_id = builder.ins().iconst(types::I32, i64::from(type_id));
            let size = builder
                .ins()
                .iconst(pointer, i64::from(pointer.bytes() * 2));
            let alignment = builder.ins().iconst(pointer, i64::from(pointer.bytes()));
            runtime_call(
                builder,
                if *payload_owned {
                    MIXED_NEW_AGGREGATE
                } else {
                    MIXED_NEW_AGGREGATE_BORROWED
                },
                &[types::I8, types::I32, pointer, pointer, pointer],
                Some(pointer),
                &[tag, type_id, source, size, alignment],
                resources,
            )?
            .ok_or_else(|| backend_failure("mixed function allocation produced no result"))
        }
        mir::MixedExpression::CollectionIndex {
            positional,
            collection,
            index,
            transfer,
            remove,
        } => {
            let value = lower_collection_index(
                builder,
                *collection,
                index,
                *remove,
                *positional,
                resources,
            )?;
            if *transfer && !*remove {
                // Owning index read (`mixed $x = $items[0]`): clone the collection's box
                // into an owned handle that shares the payload owner with the element that
                // remains in the collection.
                lower_own_mixed_value(builder, value, mir::MixedOwnership::None, resources)
            } else {
                // `removeAt` popped the element out, so the box is already ours and the
                // collection no longer claims the payload; a borrow read keeps the box.
                Ok(value)
            }
        }
    }
}

fn lower_nullable_mixed_expression(
    builder: &mut FunctionBuilder,
    expression: &mir::NullableMixedExpression,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    match expression {
        mir::NullableMixedExpression::Null => Ok(builder.ins().iconst(pointer, 0)),
        mir::NullableMixedExpression::Mixed(value) => {
            lower_mixed_expression(builder, value, resources)
        }
        mir::NullableMixedExpression::BoxNullablePayloadEnum(value) => {
            let ty = value.ty();
            let source = lower_nullable_payload_enum_expression(builder, value, resources)?;
            let present = builder.ins().load(
                types::I8,
                cranelift_codegen::ir::MachMemFlags::trusted(),
                source,
                0,
            );
            let box_value = builder.create_block();
            let absent = builder.create_block();
            let done = builder.create_block();
            builder.append_block_param(done, pointer);
            let zero = builder.ins().iconst(types::I8, 0);
            let present = builder.ins().icmp(IntCC::NotEqual, present, zero);
            builder.ins().brif(present, box_value, &[], absent, &[]);
            builder.switch_to_block(box_value);
            let payload = builder
                .ins()
                .iadd_imm_u(source, i64::from(ty.nullable_payload_offset));
            let (tag_value, type_id) = mixed_tag_value(mir::MixedTag::PayloadEnum(ty));
            let tag = builder.ins().iconst(types::I8, i64::from(tag_value));
            let type_id = builder.ins().iconst(types::I32, i64::from(type_id));
            let size = builder.ins().iconst(pointer, i64::from(ty.size));
            let alignment = builder.ins().iconst(pointer, i64::from(ty.align));
            let boxed = runtime_call(
                builder,
                MIXED_NEW_AGGREGATE,
                &[types::I8, types::I32, pointer, pointer, pointer],
                Some(pointer),
                &[tag, type_id, payload, size, alignment],
                resources,
            )?
            .ok_or_else(|| {
                backend_failure("nullable mixed aggregate allocation produced no result")
            })?;
            builder.ins().jump(done, &[boxed.into()]);
            builder.switch_to_block(absent);
            let null = builder.ins().iconst(pointer, 0);
            builder.ins().jump(done, &[null.into()]);
            builder.switch_to_block(done);
            Ok(builder.block_params(done)[0])
        }
        mir::NullableMixedExpression::Local { local, transfer } => {
            let slot = local_slot(resources.local_slots, *local)?;
            let value = load_lowered_from_stack(builder, mir::Type::NullableMixed, slot, pointer)
                .single()?;
            if *transfer {
                let zero = builder.ins().iconst(pointer, 0);
                builder.ins().stack_store(pointer, zero, slot, 0);
            }
            Ok(value)
        }
        mir::NullableMixedExpression::Property { object, property } => {
            let address = lower_property_address(builder, *object, *property, resources)?;
            Ok(
                load_lowered_from_address(builder, mir::Type::NullableMixed, address, pointer)
                    .single()?,
            )
        }
        mir::NullableMixedExpression::Call { function, args, .. } => {
            lower_function_call(builder, *function, args, resources)
                .and_then(|value| {
                    value.ok_or_else(|| malformed_mir("nullable mixed call produced no result"))
                })
                .and_then(LoweredValue::single)
        }
        mir::NullableMixedExpression::Coalesce {
            left,
            right,
            transfer,
        } => {
            let left_ownership = left.ownership();
            let right_ownership = right.ownership();
            let left = lower_nullable_mixed_expression(builder, left, resources)?;
            let left_block = builder.create_block();
            let fallback_block = builder.create_block();
            let done_block = builder.create_block();
            let result = builder.append_block_param(done_block, pointer);
            let zero = builder.ins().iconst(pointer, 0);
            let present = builder.ins().icmp(IntCC::NotEqual, left, zero);
            builder
                .ins()
                .brif(present, left_block, &[], fallback_block, &[]);
            builder.switch_to_block(left_block);
            let left = if *transfer {
                left
            } else {
                lower_own_mixed_value(builder, left, left_ownership, resources)?
            };
            builder.ins().jump(done_block, &[left.into()]);
            builder.switch_to_block(fallback_block);
            let right = lower_nullable_mixed_expression(builder, right, resources)?;
            let right = if *transfer {
                right
            } else {
                lower_own_nullable_mixed_value(builder, right, right_ownership, resources)?
            };
            builder.ins().jump(done_block, &[right.into()]);
            builder.switch_to_block(done_block);
            Ok(result)
        }
    }
}

fn lower_mixed_payload(
    builder: &mut FunctionBuilder,
    mixed: mir::LocalId,
    tag: mir::MixedTag,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let slot = local_slot(resources.local_slots, mixed)?;
    let mixed = load_lowered_from_stack(builder, mir::Type::Mixed, slot, pointer).single()?;
    let word = runtime_call(
        builder,
        MIXED_PAYLOAD,
        &[pointer],
        Some(types::I64),
        &[mixed],
        resources,
    )?
    .ok_or_else(|| backend_failure("mixed payload read produced no result"))?;
    collection_word_to_value(builder, word, tag.ty(), pointer)
}

fn lower_mixed_function_payload(
    builder: &mut FunctionBuilder,
    mixed: mir::LocalId,
    function_type: mir::FunctionTypeId,
    transfer: bool,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<LoweredValue, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let slot = local_slot(resources.local_slots, mixed)?;
    let mixed_value = load_lowered_from_stack(builder, mir::Type::Mixed, slot, pointer).single()?;
    let payload_word = runtime_call(
        builder,
        MIXED_PAYLOAD,
        &[pointer],
        Some(types::I64),
        &[mixed_value],
        resources,
    )?
    .ok_or_else(|| backend_failure("mixed function payload read produced no result"))?;
    let address = if pointer == types::I64 {
        payload_word
    } else {
        builder.ins().ireduce(pointer, payload_word)
    };
    let value = load_lowered_from_address(
        builder,
        mir::Type::Function(function_type),
        address,
        pointer,
    );
    if transfer {
        let zero = builder.ins().iconst(pointer, 0);
        builder.ins().stack_store(pointer, zero, slot, 0);
        let final_claim = runtime_call(
            builder,
            MIXED_RELEASE_OWNED,
            &[pointer],
            Some(types::I8),
            &[mixed_value],
            resources,
        )?
        .ok_or_else(|| backend_failure("mixed function move released no ownership claim"))?;
        let no_claim = builder.ins().icmp_imm_u(IntCC::Equal, final_claim, 0);
        lower_panic_if_code_at_active_site(builder, no_claim, "P1321", resources)?;
        runtime_call(
            builder,
            MIXED_FREE,
            &[pointer],
            None,
            &[mixed_value],
            resources,
        )?;
    }
    Ok(value)
}

fn lower_take_mixed_payload(
    builder: &mut FunctionBuilder,
    mixed: mir::LocalId,
    tag: mir::MixedTag,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let slot = local_slot(resources.local_slots, mixed)?;
    let mixed_value = load_lowered_from_stack(builder, mir::Type::Mixed, slot, pointer).single()?;
    let word = runtime_call(
        builder,
        MIXED_PAYLOAD,
        &[pointer],
        Some(types::I64),
        &[mixed_value],
        resources,
    )?
    .ok_or_else(|| backend_failure("mixed payload read produced no result"))?;
    let payload = collection_word_to_value(builder, word, tag.ty(), pointer)?;
    let zero = builder.ins().iconst(pointer, 0);
    builder.ins().stack_store(pointer, zero, slot, 0);
    let owns_final = runtime_call(
        builder,
        MIXED_RELEASE_OWNED,
        &[pointer],
        Some(types::I8),
        &[mixed_value],
        resources,
    )?
    .ok_or_else(|| backend_failure("mixed payload take released no ownership claim"))?;
    // A move-type payload may only be moved out when this box holds the final owning
    // claim. If another box still shares the owner (e.g. the collection this value was
    // read from with `mixed $x = $items[0]`), or the box only borrows its payload,
    // `release_owned` reports a non-final claim; moving the payload out anyway would
    // hand ownership to the callee while the other holder later drops the same payload,
    // a double free. Refuse it rather than corrupt memory.
    if matches!(tag, mir::MixedTag::String | mir::MixedTag::Class(_)) {
        let zero_flag = builder.ins().iconst(types::I8, 0);
        let not_final = builder.ins().icmp(IntCC::Equal, owns_final, zero_flag);
        lower_panic_if_code_at_active_site(builder, not_final, "P1321", resources)?;
    }
    runtime_call(
        builder,
        MIXED_FREE,
        &[pointer],
        None,
        &[mixed_value],
        resources,
    )?;
    Ok(payload)
}

fn lower_mixed_is(
    builder: &mut FunctionBuilder,
    mixed: &mir::MixedExpression,
    tag: mir::MixedTag,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let ownership = mixed.ownership();
    let mixed_value = lower_mixed_expression(builder, mixed, resources)?;
    let actual_tag = runtime_call(
        builder,
        MIXED_TAG,
        &[pointer],
        Some(types::I8),
        &[mixed_value],
        resources,
    )?
    .ok_or_else(|| backend_failure("mixed tag read produced no result"))?;
    let (expected_tag, expected_type_id) = mixed_tag_value(tag);
    let expected_tag = builder.ins().iconst(types::I8, i64::from(expected_tag));
    let tag_matches = builder.ins().icmp(IntCC::Equal, actual_tag, expected_tag);
    let result = if tag.has_structural_type_id() {
        let actual_type_id = runtime_call(
            builder,
            MIXED_TYPE_ID,
            &[pointer],
            Some(types::I32),
            &[mixed_value],
            resources,
        )?
        .ok_or_else(|| backend_failure("mixed type-id read produced no result"))?;
        let expected_type_id = builder
            .ins()
            .iconst(types::I32, i64::from(expected_type_id));
        let type_matches = builder
            .ins()
            .icmp(IntCC::Equal, actual_type_id, expected_type_id);
        builder.ins().band(tag_matches, type_matches)
    } else {
        tag_matches
    };
    lower_cleanup_mixed_temporary(builder, mixed_value, ownership, resources)?;
    Ok(result)
}

fn lower_string_expression(
    builder: &mut FunctionBuilder,
    expression: &mir::StringExpression,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    match expression {
        mir::StringExpression::Intrinsic(call) => {
            lower_string_intrinsic_call(builder, call, resources)?.single()
        }
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
            let value = builder.ins().stack_load(
                pointer,
                pointer,
                local_slot(resources.local_slots, *local)?,
                0,
            );
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
        mir::StringExpression::MixedPayload(local) => {
            let value = lower_mixed_payload(builder, *local, mir::MixedTag::String, resources)?;
            retain_string(builder, value, resources)
        }
        mir::StringExpression::NullableLocalAssumeNonNull(local) => {
            let pointer = resources.module.target_config().pointer_type();
            let value = builder.ins().stack_load(
                pointer,
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
        mir::StringExpression::ErrorMessage(error) => {
            let (object, descriptor) =
                lower_error_expression(builder, error, resources)?.nullable()?;
            let offset = builder.ins().load(
                pointer,
                cranelift_codegen::ir::MachMemFlags::trusted(),
                descriptor,
                (pointer.bytes() * 2) as i32,
            );
            let address = builder.ins().iadd(object, offset);
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
                mir::ScalarType::Enum(_) => {
                    return Err(malformed_mir(
                        "enum display requires an explicit projection",
                    ));
                }
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
        mir::StringExpression::ReadFile { path, path_span } => {
            let path = lower_string_expression(builder, path, resources)?;
            set_active_panic_site(builder, *path_span, resources);
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
        mir::StringExpression::CollectionIndex {
            positional,
            collection,
            index,
            remove,
        } => {
            let value = lower_collection_index(
                builder,
                *collection,
                index,
                *remove,
                *positional,
                resources,
            )?;
            if *remove {
                Ok(value)
            } else {
                retain_string(builder, value, resources)
            }
        }
        mir::StringExpression::CollectionKeyAt { collection, offset } => {
            let value = lower_collection_key_at(
                builder,
                *collection,
                offset,
                mir::Type::String,
                resources,
            )?;
            retain_string(builder, value, resources)
        }
        mir::StringExpression::EnumBacking { enum_id, value } => {
            lower_string_enum_backing(builder, *enum_id, value, resources)
        }
    }
}

fn lower_string_enum_backing(
    builder: &mut FunctionBuilder,
    enum_id: crate::enums::EnumId,
    value: &mir::EnumExpression,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let tag = lower_enum_expression(builder, value, resources)?;
    lower_string_enum_backing_from_tag(builder, enum_id, tag, resources)
}

fn lower_string_enum_backing_from_tag(
    builder: &mut FunctionBuilder,
    enum_id: crate::enums::EnumId,
    tag: Value,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let cases = enum_definition(resources.program, enum_id)?
        .cases
        .iter()
        .map(|case| match case.backing_value.as_ref() {
            Some(crate::enums::EnumBackingValue::String(value)) => Ok((case.tag, value.clone())),
            _ => Err(malformed_mir("string-backed enum case has no string value")),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let pointer = resources.module.target_config().pointer_type();
    let done = builder.create_block();
    builder.append_block_param(done, pointer);
    let mut cases = cases.into_iter().peekable();
    while let Some((case_tag, backing)) = cases.next() {
        if cases.peek().is_some() {
            let selected = builder.create_block();
            let next = builder.create_block();
            let expected = builder.ins().iconst(types::I32, i64::from(case_tag));
            let matches = builder.ins().icmp(IntCC::Equal, tag, expected);
            builder.ins().brif(matches, selected, &[], next, &[]);
            builder.switch_to_block(selected);
            let result = lower_string_expression(
                builder,
                &mir::StringExpression::Literal(backing),
                resources,
            )?;
            builder.ins().jump(done, &[result.into()]);
            builder.switch_to_block(next);
        } else {
            let result = lower_string_expression(
                builder,
                &mir::StringExpression::Literal(backing),
                resources,
            )?;
            builder.ins().jump(done, &[result.into()]);
        }
    }
    builder.switch_to_block(done);
    Ok(builder.block_params(done)[0])
}

fn lower_nullable_string_expression(
    builder: &mut FunctionBuilder,
    expression: &mir::NullableStringExpression,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<LoweredValue, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    match expression {
        mir::NullableStringExpression::Intrinsic(call) => {
            lower_string_intrinsic_call(builder, call, resources)
        }
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
        mir::NullableStringExpression::ReadLine {
            prompt,
            prompt_span,
        } => {
            // The prompt is evaluated once here, then borrowed for the duration of
            // the runtime call, which owns the write/flush/read ordering.
            let prompt = lower_string_expression(builder, prompt, resources)?;
            set_active_panic_site(builder, *prompt_span, resources);
            let payload = runtime_call(
                builder,
                READ_STDIN_LINE_PROMPTED,
                &[pointer, pointer],
                Some(pointer),
                &[resources.current_frame, prompt],
                resources,
            )?
            .ok_or_else(|| backend_failure("read_line produced no result"))?;
            release_string(builder, prompt, resources)?;
            let present = presence_word(builder, payload, pointer);
            Ok(LoweredValue::Nullable { present, payload })
        }
        mir::NullableStringExpression::Call { function, args } => {
            lower_function_call(builder, *function, args, resources)?
                .ok_or_else(|| malformed_mir("nullable-string call produced no result"))
        }
        mir::NullableStringExpression::EnumBacking { enum_id, value } => {
            let value = lower_nullable_scalar_expression(builder, value, resources)?;
            lower_nullable_value_map(
                builder,
                value,
                pointer,
                resources,
                |builder, tag, resources| {
                    lower_string_enum_backing_from_tag(builder, *enum_id, tag, resources)
                },
            )
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
        mir::NullableStringExpression::DictionaryGet {
            collection,
            key,
            access,
        } => {
            let (present, payload) = lower_dictionary_get(
                builder,
                *collection,
                key,
                mir::Type::String,
                *access,
                resources,
            )?;
            let payload = if matches!(
                access,
                mir::NullableCollectionAccess::Remove
                    | mir::NullableCollectionAccess::Pop
                    | mir::NullableCollectionAccess::PopFront
                    | mir::NullableCollectionAccess::PopBack
            ) {
                payload
            } else {
                retain_string(builder, payload, resources)?
            };
            Ok(LoweredValue::Nullable { present, payload })
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
        mir::NullableScalarExpression::StringIntrinsic(call) => {
            lower_string_intrinsic_call(builder, call, resources)
        }
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
        mir::NullableScalarExpression::EnumBacking { enum_id, value } => {
            let value = lower_nullable_scalar_expression(builder, value, resources)?;
            lower_nullable_value_map(
                builder,
                value,
                types::I64,
                resources,
                |builder, tag, resources| {
                    lower_integer_enum_backing_from_tag(builder, *enum_id, tag, resources)
                },
            )
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
        mir::NullableScalarExpression::DictionaryGet {
            collection,
            key,
            access,
            ..
        } => {
            let (present, payload) = lower_dictionary_get(
                builder,
                *collection,
                key,
                mir::Type::Scalar(ty),
                *access,
                resources,
            )?;
            Ok(LoweredValue::Nullable { present, payload })
        }
        mir::NullableScalarExpression::CollectionIndexOf { collection, value } => {
            let local = local_definition(resources.program, resources.function_id, *collection)?;
            let mir::Type::Collection(collection_type) = local.ty else {
                return Err(malformed_mir("List::indexOf uses a non-collection local"));
            };
            let definition = collection_definition(resources.program, collection_type)?.clone();
            if let Some((payload, nullable)) = payload_enum_storage(definition.value) {
                let owned = payload_enum_rvalue_is_owned(value);
                let needle = lower_rvalue(builder, value, resources)?.single()?;
                let collection = lower_collection_pointer(builder, *collection, resources)?;
                let (found, index) = lower_payload_enum_collection_search(
                    builder, collection, needle, payload, nullable, resources,
                )?;
                if owned {
                    lower_drop_payload_enum_at(builder, needle, payload, nullable, resources)?;
                }
                let present = builder.ins().uextend(pointer, found);
                let position = if pointer == types::I64 {
                    index
                } else {
                    builder.ins().uextend(types::I64, index)
                };
                return Ok(LoweredValue::Nullable {
                    present,
                    payload: position,
                });
            }
            let (needle_present, needle, needle_type) =
                if nullable_payload_type(definition.value).is_some() {
                    lower_nullable_collection_parts(builder, value, definition.value, resources)?
                } else {
                    let needle = lower_rvalue(builder, value, resources)?.single()?;
                    (builder.ins().iconst(types::I8, 1), needle, definition.value)
                };
            let needle_word = value_to_collection_word(builder, needle, needle_type, pointer)?;
            let collection = lower_collection_pointer(builder, *collection, resources)?;
            let kind = builder
                .ins()
                .iconst(types::I8, collection_compare_kind(needle_type)?);
            let found_slot = builder.create_sized_stack_slot(StackSlotData::new(
                StackSlotKind::ExplicitSlot,
                1,
                0,
            ));
            let found_pointer = builder.ins().stack_addr(pointer, found_slot, 0);
            let position = runtime_call(
                builder,
                COLLECTION_INDEX_OF,
                &[pointer, types::I64, types::I8, types::I8, pointer],
                Some(types::I64),
                &[collection, needle_word, needle_present, kind, found_pointer],
                resources,
            )?
            .ok_or_else(|| backend_failure("List::indexOf produced no result"))?;
            let found = builder.ins().stack_load(pointer, types::I8, found_slot, 0);
            let present = builder.ins().uextend(pointer, found);
            lower_drop_stored_value(builder, needle, definition.value, resources)?;
            Ok(LoweredValue::Nullable {
                present,
                payload: position,
            })
        }
        mir::NullableScalarExpression::Parse { value, .. } => {
            let text = lower_string_expression(builder, value, resources)?;
            let found_slot = builder.create_sized_stack_slot(StackSlotData::new(
                StackSlotKind::ExplicitSlot,
                1,
                0,
            ));
            let found_pointer = builder.ins().stack_addr(pointer, found_slot, 0);
            let symbol = match ty {
                mir::ScalarType::Integer(_) => INT_PARSE,
                mir::ScalarType::Float(_) => FLOAT_PARSE,
                mir::ScalarType::Bool | mir::ScalarType::Enum(_) => {
                    return Err(malformed_mir("parse does not produce a bool value"));
                }
            };
            let word = runtime_call(
                builder,
                symbol,
                &[pointer, pointer],
                Some(types::I64),
                &[text, found_pointer],
                resources,
            )?
            .ok_or_else(|| backend_failure("parse produced no result"))?;
            release_string(builder, text, resources)?;
            let found = builder.ins().stack_load(pointer, types::I8, found_slot, 0);
            let present = builder.ins().uextend(pointer, found);
            let payload = collection_word_to_value(builder, word, mir::Type::Scalar(ty), pointer)?;
            Ok(LoweredValue::Nullable { present, payload })
        }
    }
}

fn scalar_zero(builder: &mut FunctionBuilder, ty: mir::ScalarType) -> Value {
    match ty {
        mir::ScalarType::Integer(ty) => builder.ins().iconst(clif_integer_type(ty), 0),
        mir::ScalarType::Float(FloatType::Float32) => builder.ins().f32const(Ieee32::with_bits(0)),
        mir::ScalarType::Float(FloatType::Float64) => builder.ins().f64const(Ieee64::with_bits(0)),
        mir::ScalarType::Bool => builder.ins().iconst(types::I8, 0),
        mir::ScalarType::Enum(_) => builder.ins().iconst(types::I32, 0),
    }
}

fn lower_nullable_value_map(
    builder: &mut FunctionBuilder,
    value: LoweredValue,
    payload_type: ClifType,
    resources: &mut LoweringResources<'_, '_>,
    present_value: impl FnOnce(
        &mut FunctionBuilder,
        Value,
        &mut LoweringResources<'_, '_>,
    ) -> Result<Value, BackendError>,
) -> Result<LoweredValue, BackendError> {
    let pointer = resources.module.target_config().pointer_type();
    let (present, source_payload) = value.nullable()?;
    let zero = builder.ins().iconst(pointer, 0);
    let is_present = builder.ins().icmp(IntCC::NotEqual, present, zero);
    let some = builder.create_block();
    let none = builder.create_block();
    let done = builder.create_block();
    builder.append_block_param(done, pointer);
    builder.append_block_param(done, payload_type);
    builder.ins().brif(is_present, some, &[], none, &[]);
    builder.switch_to_block(some);
    let payload = present_value(builder, source_payload, resources)?;
    let present = builder.ins().iconst(pointer, 1);
    builder
        .ins()
        .jump(done, &[BlockArg::Value(present), BlockArg::Value(payload)]);
    builder.switch_to_block(none);
    let absent = builder.ins().iconst(pointer, 0);
    let payload = clif_zero(builder, payload_type);
    builder
        .ins()
        .jump(done, &[BlockArg::Value(absent), BlockArg::Value(payload)]);
    builder.switch_to_block(done);
    let params = builder.block_params(done);
    Ok(LoweredValue::Nullable {
        present: params[0],
        payload: params[1],
    })
}

fn clif_zero(builder: &mut FunctionBuilder, ty: ClifType) -> Value {
    if ty == types::F32 {
        builder.ins().f32const(Ieee32::with_bits(0))
    } else if ty == types::F64 {
        builder.ins().f64const(Ieee64::with_bits(0))
    } else {
        builder.ins().iconst(ty, 0)
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
    let payload = clif_zero(builder, payload_type);
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
        mir::IntegerExpression::Unary {
            ty,
            op,
            operand,
            span,
        } => {
            let operand = lower_integer_expression(builder, operand, resources)?;
            lower_integer_unary(builder, *ty, *op, operand, *span, resources)
        }
        mir::IntegerExpression::Binary {
            ty,
            op,
            left,
            right,
            span,
            right_span,
        } => {
            let left = lower_integer_expression(builder, left, resources)?;
            let right = lower_integer_expression(builder, right, resources)?;
            lower_integer_binary(
                builder,
                *ty,
                *op,
                left,
                right,
                (*span, *right_span),
                resources,
            )
        }
        mir::IntegerExpression::Convert {
            ty,
            value,
            value_span,
            ..
        } => {
            let source_type = value.ty();
            let value = lower_integer_expression(builder, value, resources)?;
            lower_integer_conversion(builder, source_type, *ty, value, *value_span, resources)
        }
        mir::IntegerExpression::FloatToInt {
            value, value_span, ..
        } => {
            let value = lower_float_expression(builder, value, resources)?;
            lower_float_to_int(builder, value, *value_span, resources)
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
        mir::IntegerExpression::EnumBacking { enum_id, value } => {
            let tag = lower_enum_expression(builder, value, resources)?;
            lower_integer_enum_backing_from_tag(builder, *enum_id, tag, resources)
        }
    }
}

fn lower_integer_enum_backing_from_tag(
    builder: &mut FunctionBuilder,
    enum_id: crate::enums::EnumId,
    tag: Value,
    resources: &LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let values = enum_definition(resources.program, enum_id)?
        .cases
        .iter()
        .map(|case| match case.backing_value.as_ref() {
            Some(crate::enums::EnumBackingValue::Int(value)) => Ok((case.tag, *value)),
            _ => Err(malformed_mir("int-backed enum case has no integer value")),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut values = values.into_iter();
    let (_first_tag, first) = values
        .next()
        .ok_or_else(|| malformed_mir("enum has no cases"))?;
    let mut result = integer_constant(builder, first);
    for (case_tag, backing) in values {
        let expected = builder.ins().iconst(types::I32, i64::from(case_tag));
        let selected = builder.ins().icmp(IntCC::Equal, tag, expected);
        let backing = integer_constant(builder, backing);
        result = builder.ins().select(selected, backing, result);
    }
    Ok(result)
}

fn lower_integer_unary(
    builder: &mut FunctionBuilder,
    ty: IntegerType,
    op: mir::IntegerUnaryOp,
    operand: Value,
    span: crate::source::Span,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    match op {
        mir::IntegerUnaryOp::Negate => {
            let zero = builder.ins().iconst(clif_integer_type(ty), 0);
            let (value, overflow) = builder.ins().ssub_overflow(zero, operand);
            lower_panic_if(
                builder,
                overflow,
                IntegerPanic::OverflowNegation,
                span,
                resources,
            )?;
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
    spans: (crate::source::Span, crate::source::Span),
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let (span, right_span) = spans;
    match op {
        mir::IntegerBinaryOp::Add
        | mir::IntegerBinaryOp::Subtract
        | mir::IntegerBinaryOp::Multiply => {
            lower_checked_arithmetic(builder, ty, op, left, right, span, resources)
        }
        mir::IntegerBinaryOp::Divide => {
            lower_integer_division(builder, ty, left, right, span, right_span, resources)
        }
        mir::IntegerBinaryOp::Remainder => {
            lower_integer_remainder(builder, ty, left, right, right_span, resources)
        }
        mir::IntegerBinaryOp::ShiftLeft | mir::IntegerBinaryOp::ShiftRight => {
            lower_integer_shift(builder, ty, op, left, right, right_span, resources)
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
    span: crate::source::Span,
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
    lower_panic_if(builder, overflow, panic, span, resources)?;
    Ok(value)
}

fn lower_integer_division(
    builder: &mut FunctionBuilder,
    ty: IntegerType,
    left: Value,
    right: Value,
    span: crate::source::Span,
    right_span: crate::source::Span,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let zero = builder.ins().iconst(clif_integer_type(ty), 0);
    let divides_by_zero = builder.ins().icmp(IntCC::Equal, right, zero);
    lower_panic_if(
        builder,
        divides_by_zero,
        IntegerPanic::DivisionByZero,
        right_span,
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
            span,
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
    right_span: crate::source::Span,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let zero = builder.ins().iconst(clif_integer_type(ty), 0);
    let divides_by_zero = builder.ins().icmp(IntCC::Equal, right, zero);
    lower_panic_if(
        builder,
        divides_by_zero,
        IntegerPanic::RemainderByZero,
        right_span,
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
    right_span: crate::source::Span,
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
        right_span,
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
    span: crate::source::Span,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    if let Some(out_of_range) = conversion_out_of_range(builder, source, target, value) {
        lower_panic_if(
            builder,
            out_of_range,
            IntegerPanic::ConversionOutOfRange,
            span,
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
    span: crate::source::Span,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    lower_panic_if_code(builder, condition, panic.code(), span, resources)
}

fn lower_panic_if_code(
    builder: &mut FunctionBuilder,
    condition: Value,
    code: &'static str,
    span: crate::source::Span,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    lower_panic_if_code_with_site(builder, condition, code, Some(span), resources)
}

fn lower_panic_if_code_at_active_site(
    builder: &mut FunctionBuilder,
    condition: Value,
    code: &'static str,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    lower_panic_if_code_with_site(builder, condition, code, None, resources)
}

fn lower_panic_if_code_with_site(
    builder: &mut FunctionBuilder,
    condition: Value,
    code: &'static str,
    span: Option<crate::source::Span>,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let panic_block = builder.create_block();
    let continue_block = builder.create_block();
    builder
        .ins()
        .brif(condition, panic_block, &[], continue_block, &[]);

    builder.switch_to_block(panic_block);
    if let Some(span) = span {
        set_active_panic_site(builder, span, resources);
    }
    lower_runtime_panic_code(builder, code.as_bytes(), &[], resources)?;

    builder.switch_to_block(continue_block);
    Ok(())
}

fn lower_panic_if_signed_fact(
    builder: &mut FunctionBuilder,
    condition: Value,
    code: &'static str,
    fact_name: &'static str,
    value: Value,
    span: crate::source::Span,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let panic_block = builder.create_block();
    let continue_block = builder.create_block();
    builder
        .ins()
        .brif(condition, panic_block, &[], continue_block, &[]);

    builder.switch_to_block(panic_block);
    set_active_panic_site(builder, span, resources);
    let pointer = resources.module.target_config().pointer_type();
    let code_pointer = define_data(builder, code.as_bytes(), resources)?;
    let code_length = builder.ins().iconst(pointer, code.len() as i64);
    let fact_name_pointer = define_data(builder, fact_name.as_bytes(), resources)?;
    let fact_name_length = builder.ins().iconst(pointer, fact_name.len() as i64);
    let panic_id = resources.declare_runtime(
        "dr_v2_panic_signed_fact",
        &[pointer, pointer, pointer, pointer, pointer, types::I64],
        None,
    )?;
    let panic = resources
        .module
        .declare_func_in_func(panic_id, builder.func);
    builder.ins().call(
        panic,
        &[
            resources.current_frame,
            code_pointer,
            code_length,
            fact_name_pointer,
            fact_name_length,
            value,
        ],
    );
    builder
        .ins()
        .trap(TrapCode::unwrap_user(RUNTIME_RETURNED_TRAP));

    builder.switch_to_block(continue_block);
    Ok(())
}

fn lower_padding_empty_panic_if(
    builder: &mut FunctionBuilder,
    condition: Value,
    pad_start: bool,
    facts: [Value; 4],
    span: crate::source::Span,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let [value, current_length, requested_length, padding_length] = facts;
    let panic_block = builder.create_block();
    let continue_block = builder.create_block();
    builder
        .ins()
        .brif(condition, panic_block, &[], continue_block, &[]);

    builder.switch_to_block(panic_block);
    set_active_panic_site(builder, span, resources);
    let pointer = resources.module.target_config().pointer_type();
    let panic_id = resources.declare_runtime(
        "dr_v2_panic_string_padding_empty",
        &[pointer, types::I8, pointer, pointer, types::I64, pointer],
        None,
    )?;
    let panic = resources
        .module
        .declare_func_in_func(panic_id, builder.func);
    let pad_start = builder.ins().iconst(types::I8, i64::from(pad_start));
    builder.ins().call(
        panic,
        &[
            resources.current_frame,
            pad_start,
            value,
            current_length,
            requested_length,
            padding_length,
        ],
    );
    builder
        .ins()
        .trap(TrapCode::unwrap_user(RUNTIME_RETURNED_TRAP));

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
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    match operand {
        mir::Operand::StringIntrinsic(call) => {
            lower_string_intrinsic_call(builder, call, resources)?.single()
        }
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
            let pointer = resources.module.target_config().pointer_type();
            Ok(builder
                .ins()
                .stack_load(pointer, clif_integer_type(ty), slot, 0))
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
                pointer,
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
        mir::Operand::CollectionLength(collection) if ty == IntegerType::Int64 => {
            let pointer = resources.module.target_config().pointer_type();
            let local = local_definition(resources.program, resources.function_id, *collection)?;
            let mir::Type::Collection(collection_type) = local.ty else {
                return Err(malformed_mir("length uses non-collection local"));
            };
            let is_bytes = collection_definition(resources.program, collection_type)?.kind
                == mir::CollectionKind::Bytes;
            let collection = lower_collection_pointer(builder, *collection, resources)?;
            runtime_call(
                builder,
                if is_bytes {
                    BYTES_LENGTH
                } else {
                    COLLECTION_LENGTH
                },
                &[pointer],
                Some(pointer),
                &[collection],
                resources,
            )?
            .ok_or_else(|| backend_failure("collection length produced no result"))
        }
        mir::Operand::CollectionIndex {
            positional,
            collection,
            index,
            remove,
        } => lower_collection_index(builder, *collection, index, *remove, *positional, resources),
        mir::Operand::CollectionKeyAt { collection, offset } => lower_collection_key_at(
            builder,
            *collection,
            offset,
            mir::Type::Scalar(mir::ScalarType::Integer(ty)),
            resources,
        ),
        mir::Operand::MixedPayload { mixed, tag } => {
            lower_mixed_payload(builder, *mixed, *tag, resources)
        }
        mir::Operand::Scalar(_) => Err(malformed_mir(
            "integer expression contains non-integer constant",
        )),
        mir::Operand::CollectionLength(_) => Err(malformed_mir(
            "collection length expression must have type int64",
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
                let pointer = resources.module.target_config().pointer_type();
                Ok(builder.ins().stack_load(
                    pointer,
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
                    pointer,
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
            mir::Operand::CollectionIndex {
                positional,
                collection,
                index,
                remove,
            } => {
                lower_collection_index(builder, *collection, index, *remove, *positional, resources)
            }
            mir::Operand::CollectionKeyAt { collection, offset } => lower_collection_key_at(
                builder,
                *collection,
                offset,
                mir::Type::Scalar(mir::ScalarType::Float(*ty)),
                resources,
            ),
            mir::Operand::MixedPayload { mixed, tag } => {
                lower_mixed_payload(builder, *mixed, *tag, resources)
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
    span: crate::source::Span,
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
    lower_panic_if_code(builder, invalid, "P1110", span, resources)?;
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
    temporary_mixed: Vec<(usize, Value, mir::MixedOwnership)>,
}

fn ordered_owned_argument_indices(args: &[mir::Rvalue]) -> Vec<usize> {
    let mut indices = args
        .iter()
        .enumerate()
        .filter_map(|(index, argument)| {
            (argument.owned_temporary_class().is_some()
                || argument.owned_temporary_collection().is_some()
                || argument.owned_temporary_shared().is_some()
                || argument.owned_temporary_payload_enum().is_some()
                || (argument.mixed_ownership().has_shell()
                    && argument.transferred_owned_local().is_some()))
            .then_some(index)
        })
        .collect::<Vec<_>>();
    if indices
        .iter()
        .all(|index| args[*index].transferred_owned_local().is_some())
    {
        indices.sort_by_key(|index| {
            args[*index]
                .transferred_owned_local()
                .expect("all reordered owned temporaries have source-order locals")
                .0
        });
    }
    indices
}

fn lower_call_args(
    builder: &mut FunctionBuilder,
    args: &[mir::Rvalue],
    resources: &mut LoweringResources<'_, '_>,
) -> Result<LoweredCallArgs, BackendError> {
    lower_call_args_with_optional_parameters(builder, args, None, resources)
}

fn lower_call_args_with_parameters(
    builder: &mut FunctionBuilder,
    args: &[mir::Rvalue],
    parameters: &[mir::FunctionParameter],
    resources: &mut LoweringResources<'_, '_>,
) -> Result<LoweredCallArgs, BackendError> {
    lower_call_args_with_optional_parameters(builder, args, Some(parameters), resources)
}

fn lower_call_args_with_optional_parameters(
    builder: &mut FunctionBuilder,
    args: &[mir::Rvalue],
    parameters: Option<&[mir::FunctionParameter]>,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<LoweredCallArgs, BackendError> {
    let mut arguments = Vec::with_capacity(args.len());
    let mut abi_values = Vec::with_capacity(args.len() * 2);
    let mut owned_strings = Vec::new();
    let mut temporary_mixed = Vec::new();
    for (index, argument) in args.iter().enumerate() {
        if parameters
            .and_then(|parameters| parameters.get(index))
            .is_some_and(|parameter| parameter.mode == mir::FunctionParameterMode::Writable)
        {
            let local = argument.direct_place_local().ok_or_else(|| {
                malformed_mir("writable indirect-call argument is not a direct local place")
            })?;
            let address = closure_source_address(builder, local, resources)?;
            arguments.push(load_lowered_from_address(
                builder,
                argument.ty(),
                address,
                resources.module.target_config().pointer_type(),
            ));
            abi_values.push(address);
            continue;
        }
        let value = lower_rvalue(builder, argument, resources)?;
        if matches!(argument.ty(), mir::Type::String | mir::Type::NullableString) {
            let string = match argument.ty() {
                mir::Type::NullableString => value.nullable()?.1,
                _ => value.single()?,
            };
            owned_strings.push((index, string));
        }
        let ownership = argument.mixed_ownership();
        if ownership.has_shell() {
            temporary_mixed.push((index, value.single()?, ownership));
        }
        value_to_doria_abi(builder, value, argument.ty()).append_to(&mut abi_values);
        arguments.push(value);
    }
    Ok(LoweredCallArgs {
        arguments,
        abi_values,
        owned_strings,
        temporary_mixed,
    })
}

fn lower_string_intrinsic_call(
    builder: &mut FunctionBuilder,
    call: &mir::StringIntrinsicCall,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<LoweredValue, BackendError> {
    use mir::StringIntrinsicKind as Kind;

    set_active_panic_site(builder, call.span, resources);
    let pointer = resources.module.target_config().pointer_type();
    let lowered = lower_call_args(builder, &call.args, resources)?;
    let argument = |index: usize| -> Result<Value, BackendError> {
        lowered
            .arguments
            .get(index)
            .ok_or_else(|| malformed_mir("String intrinsic argument is missing"))?
            .single()
    };
    let call_runtime = |builder: &mut FunctionBuilder,
                        name: &'static str,
                        params: &[ClifType],
                        result: ClifType,
                        values: &[Value],
                        resources: &mut LoweringResources<'_, '_>| {
        runtime_call(builder, name, params, Some(result), values, resources)?
            .ok_or_else(|| backend_failure("String intrinsic runtime call produced no result"))
    };

    let result = match call.kind {
        Kind::GraphemeLength | Kind::ByteLength => {
            let name = if call.kind == Kind::GraphemeLength {
                STRING_GRAPHEME_LENGTH
            } else {
                STRING_BYTE_LENGTH
            };
            let value = call_runtime(
                builder,
                name,
                &[pointer],
                pointer,
                &[argument(0)?],
                resources,
            )?;
            let value = if pointer == types::I64 {
                value
            } else {
                builder.ins().uextend(types::I64, value)
            };
            LoweredValue::Single(value)
        }
        Kind::IsEmpty => LoweredValue::Single(call_runtime(
            builder,
            STRING_IS_EMPTY,
            &[pointer],
            types::I8,
            &[argument(0)?],
            resources,
        )?),
        Kind::ToBytes => LoweredValue::Single(call_runtime(
            builder,
            STRING_TO_BYTES,
            &[pointer],
            pointer,
            &[argument(0)?],
            resources,
        )?),
        Kind::Trim
        | Kind::TrimStart
        | Kind::TrimEnd
        | Kind::Lower
        | Kind::Upper
        | Kind::LowerFirst
        | Kind::UpperFirst => {
            let name = match call.kind {
                Kind::Trim => STRING_TRIM,
                Kind::TrimStart => STRING_TRIM_START,
                Kind::TrimEnd => STRING_TRIM_END,
                Kind::Lower => STRING_LOWER,
                Kind::Upper => STRING_UPPER,
                Kind::LowerFirst => STRING_LOWER_FIRST,
                Kind::UpperFirst => STRING_UPPER_FIRST,
                _ => unreachable!(),
            };
            LoweredValue::Single(call_runtime(
                builder,
                name,
                &[pointer, pointer],
                pointer,
                &[resources.current_frame, argument(0)?],
                resources,
            )?)
        }
        Kind::ContainsIgnoreCase | Kind::StartsWithIgnoreCase | Kind::EndsWithIgnoreCase => {
            let name = match call.kind {
                Kind::ContainsIgnoreCase => STRING_CONTAINS_IGNORE_CASE,
                Kind::StartsWithIgnoreCase => STRING_STARTS_WITH_IGNORE_CASE,
                Kind::EndsWithIgnoreCase => STRING_ENDS_WITH_IGNORE_CASE,
                _ => unreachable!(),
            };
            LoweredValue::Single(call_runtime(
                builder,
                name,
                &[pointer, pointer, pointer],
                types::I8,
                &[resources.current_frame, argument(0)?, argument(1)?],
                resources,
            )?)
        }
        Kind::Contains | Kind::StartsWith | Kind::EndsWith => {
            let name = match call.kind {
                Kind::Contains => STRING_CONTAINS,
                Kind::StartsWith => STRING_STARTS_WITH,
                Kind::EndsWith => STRING_ENDS_WITH,
                _ => unreachable!(),
            };
            LoweredValue::Single(call_runtime(
                builder,
                name,
                &[pointer, pointer],
                types::I8,
                &[argument(0)?, argument(1)?],
                resources,
            )?)
        }
        Kind::EqualsIgnoreCase => LoweredValue::Single(call_runtime(
            builder,
            STRING_EQUALS_IGNORE_CASE,
            &[pointer, pointer, pointer],
            types::I8,
            &[resources.current_frame, argument(0)?, argument(1)?],
            resources,
        )?),
        Kind::IndexOf
        | Kind::LastIndexOf
        | Kind::IndexOfIgnoreCase
        | Kind::LastIndexOfIgnoreCase => {
            let found_slot = builder.create_sized_stack_slot(StackSlotData::new(
                StackSlotKind::ExplicitSlot,
                1,
                0,
            ));
            let found_pointer = builder.ins().stack_addr(pointer, found_slot, 0);
            let (name, with_frame) = match call.kind {
                Kind::IndexOf => (STRING_INDEX_OF, false),
                Kind::LastIndexOf => (STRING_LAST_INDEX_OF, false),
                Kind::IndexOfIgnoreCase => (STRING_INDEX_OF_IGNORE_CASE, true),
                Kind::LastIndexOfIgnoreCase => (STRING_LAST_INDEX_OF_IGNORE_CASE, true),
                _ => unreachable!(),
            };
            let mut values = Vec::with_capacity(4);
            if with_frame {
                values.push(resources.current_frame);
            }
            values.extend([argument(0)?, argument(1)?, found_pointer]);
            let mut params = Vec::with_capacity(4);
            if with_frame {
                params.push(pointer);
            }
            params.extend([pointer, pointer, pointer]);
            let payload = call_runtime(builder, name, &params, types::I64, &values, resources)?;
            let found = builder.ins().stack_load(pointer, types::I8, found_slot, 0);
            let present = builder.ins().uextend(pointer, found);
            LoweredValue::Nullable { present, payload }
        }
        Kind::CountOccurrences => LoweredValue::Single(call_runtime(
            builder,
            STRING_COUNT_OCCURRENCES,
            &[pointer, pointer, pointer],
            types::I64,
            &[resources.current_frame, argument(0)?, argument(1)?],
            resources,
        )?),
        Kind::Replace => LoweredValue::Single(call_runtime(
            builder,
            STRING_REPLACE,
            &[pointer, pointer, pointer, pointer],
            pointer,
            &[
                resources.current_frame,
                argument(0)?,
                argument(1)?,
                argument(2)?,
            ],
            resources,
        )?),
        Kind::Split => LoweredValue::Single(call_runtime(
            builder,
            STRING_SPLIT,
            &[pointer, pointer, pointer],
            pointer,
            &[resources.current_frame, argument(0)?, argument(1)?],
            resources,
        )?),
        Kind::Join => LoweredValue::Single(call_runtime(
            builder,
            STRING_JOIN,
            &[pointer, pointer, pointer],
            pointer,
            &[resources.current_frame, argument(0)?, argument(1)?],
            resources,
        )?),
        Kind::Slice => {
            let (has_length, length) = lowered
                .arguments
                .get(2)
                .ok_or_else(|| malformed_mir("String::slice length argument is missing"))?
                .nullable()?;
            let has_length = builder.ins().ireduce(types::I8, has_length);
            let has_length_flag = builder.ins().icmp_imm_u(IntCC::NotEqual, has_length, 0);
            let negative = builder.ins().icmp_imm_s(IntCC::SignedLessThan, length, 0);
            let invalid_length = builder.ins().band(has_length_flag, negative);
            lower_panic_if_signed_fact(
                builder,
                invalid_length,
                "P1201",
                doria_diagnostic_catalogue::STRING_SLICE_LENGTH_FACT,
                length,
                call.argument_spans.get(2).copied().unwrap_or(call.span),
                resources,
            )?;
            set_active_panic_site(builder, call.span, resources);
            LoweredValue::Single(call_runtime(
                builder,
                STRING_SLICE,
                &[pointer, pointer, types::I64, types::I64, types::I8],
                pointer,
                &[
                    resources.current_frame,
                    argument(0)?,
                    argument(1)?,
                    length,
                    has_length,
                ],
                resources,
            )?)
        }
        Kind::Repeat => {
            let count = argument(1)?;
            let negative = builder.ins().icmp_imm_s(IntCC::SignedLessThan, count, 0);
            lower_panic_if_signed_fact(
                builder,
                negative,
                "P1204",
                doria_diagnostic_catalogue::STRING_REPETITION_COUNT_FACT,
                count,
                call.argument_spans.get(1).copied().unwrap_or(call.span),
                resources,
            )?;
            set_active_panic_site(builder, call.span, resources);
            LoweredValue::Single(call_runtime(
                builder,
                STRING_REPEAT,
                &[pointer, pointer, types::I64],
                pointer,
                &[resources.current_frame, argument(0)?, count],
                resources,
            )?)
        }
        Kind::PadStart | Kind::PadEnd => {
            let name = if call.kind == Kind::PadStart {
                STRING_PAD_START
            } else {
                STRING_PAD_END
            };
            let target_length = argument(1)?;
            let negative = builder
                .ins()
                .icmp_imm_s(IntCC::SignedLessThan, target_length, 0);
            lower_panic_if_signed_fact(
                builder,
                negative,
                "P1202",
                doria_diagnostic_catalogue::STRING_PADDING_REQUESTED_LENGTH_FACT,
                target_length,
                call.argument_spans.get(1).copied().unwrap_or(call.span),
                resources,
            )?;
            let current_length_word = call_runtime(
                builder,
                STRING_GRAPHEME_LENGTH,
                &[pointer],
                pointer,
                &[argument(0)?],
                resources,
            )?;
            let current_length = if pointer == types::I64 {
                current_length_word
            } else {
                builder.ins().uextend(types::I64, current_length_word)
            };
            let needs_padding =
                builder
                    .ins()
                    .icmp(IntCC::SignedGreaterThan, target_length, current_length);
            let padding_length = call_runtime(
                builder,
                STRING_GRAPHEME_LENGTH,
                &[pointer],
                pointer,
                &[argument(2)?],
                resources,
            )?;
            let padding_empty = builder.ins().icmp_imm_u(IntCC::Equal, padding_length, 0);
            let invalid_padding = builder.ins().band(needs_padding, padding_empty);
            lower_padding_empty_panic_if(
                builder,
                invalid_padding,
                call.kind == Kind::PadStart,
                [
                    argument(0)?,
                    current_length_word,
                    target_length,
                    padding_length,
                ],
                call.argument_spans.get(2).copied().unwrap_or(call.span),
                resources,
            )?;
            set_active_panic_site(builder, call.span, resources);
            LoweredValue::Single(call_runtime(
                builder,
                name,
                &[pointer, pointer, types::I64, pointer],
                pointer,
                &[
                    resources.current_frame,
                    argument(0)?,
                    argument(1)?,
                    argument(2)?,
                ],
                resources,
            )?)
        }
        Kind::FromBytes => {
            let payload = call_runtime(
                builder,
                STRING_FROM_BYTES,
                &[pointer],
                pointer,
                &[argument(0)?],
                resources,
            )?;
            let present = presence_word(builder, payload, pointer);
            LoweredValue::Nullable { present, payload }
        }
    };

    for (_, string) in lowered.owned_strings {
        release_string(builder, string, resources)?;
    }
    for index in ordered_owned_argument_indices(&call.args) {
        if let Some(collection) = call.args[index].owned_temporary_collection() {
            defer_or_drop_collection_temporary(
                builder,
                lowered.arguments[index].single()?,
                collection,
                resources,
            )?;
        }
    }
    Ok(result)
}

fn set_active_panic_site(
    builder: &mut FunctionBuilder,
    span: crate::source::Span,
    resources: &LoweringResources<'_, '_>,
) {
    let pointer = resources.module.target_config().pointer_type();
    let pointer_bytes = pointer.bytes() as i32;
    let flags = MemFlagsData::new();
    let start = builder.ins().iconst(pointer, span.start as i64);
    let end = builder.ins().iconst(pointer, span.end as i64);
    builder
        .ins()
        .store(flags, start, resources.current_frame, pointer_bytes * 9);
    builder
        .ins()
        .store(flags, end, resources.current_frame, pointer_bytes * 10);
}

fn lower_function_call(
    builder: &mut FunctionBuilder,
    function: mir::FunctionId,
    args: &[mir::Rvalue],
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Option<LoweredValue>, BackendError> {
    let span = function_in(resources.program, resources.function_id)?.source_span;
    lower_function_call_at(builder, function, args, span, resources)
}

fn lower_function_call_at(
    builder: &mut FunctionBuilder,
    function: mir::FunctionId,
    args: &[mir::Rvalue],
    span: crate::source::Span,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Option<LoweredValue>, BackendError> {
    set_active_panic_site(builder, span, resources);
    let lowered = lower_call_args(builder, args, resources)?;
    let mut values = vec![resources.current_frame];
    let callee_definition = function_in(resources.program, function)?;
    if !callee_definition.checked_effects.is_empty() {
        return Err(malformed_mir(
            "throwing callable reached ordinary Cranelift call lowering",
        ));
    }
    let payload_result = match callee_definition.return_type {
        mir::ReturnType::Value(
            ty @ (mir::Type::PayloadEnum(_) | mir::Type::NullablePayloadEnum(_)),
        ) => {
            let address = create_payload_result_storage(builder, ty, resources)?;
            values.push(address);
            Some(address)
        }
        _ => None,
    };
    if let Some(home) = direct_call_borrow_home(builder, callee_definition, args, resources)? {
        values.push(home);
    }
    values.extend(lowered.abi_values.iter().copied());
    let callee = declared_function(builder, resources, function)?;
    let call = builder.ins().call(callee, &values);
    let results = builder.inst_results(call);
    let result = match callee_definition.return_type {
        mir::ReturnType::Void => None,
        mir::ReturnType::Value(
            mir::Type::NullableScalar(_)
            | mir::Type::NullableString
            | mir::Type::Error
            | mir::Type::NullableError
            | mir::Type::Function(_)
            | mir::Type::NullableFunction(_),
        ) => {
            let ty = match callee_definition.return_type {
                mir::ReturnType::Value(ty) => ty,
                mir::ReturnType::Void => unreachable!(),
            };
            Some(LoweredValue::Nullable {
                present: *results
                    .first()
                    .ok_or_else(|| malformed_mir("nullable call produced no presence result"))?,
                payload: nullable_payload_from_doria_abi(
                    builder,
                    *results
                        .get(1)
                        .ok_or_else(|| malformed_mir("nullable call produced no payload result"))?,
                    ty,
                ),
            })
        }
        mir::ReturnType::Value(mir::Type::PayloadEnum(_) | mir::Type::NullablePayloadEnum(_)) => {
            Some(LoweredValue::Single(payload_result.ok_or_else(|| {
                malformed_mir("payload enum call has no result storage")
            })?))
        }
        mir::ReturnType::Value(ty) => {
            let value = *results
                .first()
                .ok_or_else(|| malformed_mir("value call produced no result"))?;
            Some(LoweredValue::Single(value_from_doria_abi(
                builder, value, ty,
            )))
        }
    };
    cleanup_call_arguments(
        builder,
        function,
        args,
        &lowered,
        callee_definition,
        resources,
    )?;
    Ok(result)
}

fn cleanup_call_arguments(
    builder: &mut FunctionBuilder,
    function: mir::FunctionId,
    args: &[mir::Rvalue],
    lowered: &LoweredCallArgs,
    callee_definition: &mir::Function,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    for (_, string) in &lowered.owned_strings {
        release_string(builder, *string, resources)?;
    }
    for (index, value, ownership) in &lowered.temporary_mixed {
        if args[*index].transferred_owned_local().is_some() {
            continue;
        }
        let parameter = *callee_definition.params.get(*index).ok_or_else(|| {
            malformed_mir(format!(
                "function{} is missing parameter {index}",
                function.0
            ))
        })?;
        if !local_in(callee_definition, parameter)?.owned {
            lower_cleanup_mixed_temporary(builder, *value, *ownership, resources)?;
        }
    }
    for index in ordered_owned_argument_indices(args) {
        let argument = &args[index];
        let parameter = *callee_definition.params.get(index).ok_or_else(|| {
            malformed_mir(format!(
                "function{} is missing parameter {index}",
                function.0
            ))
        })?;
        if !local_in(callee_definition, parameter)?.owned {
            let value = lowered.arguments[index].single()?;
            if let Some(class) = argument.owned_temporary_class() {
                defer_or_drop_class_temporary(builder, value, class, resources)?;
            } else if let Some(collection) = argument.owned_temporary_collection() {
                defer_or_drop_collection_temporary(builder, value, collection, resources)?;
            } else if let Some(shared) = argument.owned_temporary_shared() {
                defer_or_drop_owned_shared_temporary(builder, value, shared, resources)?;
            } else if let Some((payload, nullable)) = argument.owned_temporary_payload_enum() {
                lower_drop_payload_enum_at(builder, value, payload, nullable, resources)?;
            } else if argument.mixed_ownership().has_shell() {
                defer_or_cleanup_mixed_temporary(
                    builder,
                    value,
                    argument.mixed_ownership(),
                    resources,
                )?;
            }
        }
    }
    Ok(())
}

fn create_payload_result_storage(
    builder: &mut FunctionBuilder,
    ty: mir::Type,
    resources: &LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let (payload, nullable) = match ty {
        mir::Type::PayloadEnum(payload) => (payload, false),
        mir::Type::NullablePayloadEnum(payload) => (payload, true),
        _ => {
            return Err(malformed_mir(
                "payload result storage received another type",
            ))
        }
    };
    let slot = builder.create_sized_stack_slot(StackSlotData::new(
        StackSlotKind::ExplicitSlot,
        payload.storage_size(nullable),
        payload.align.trailing_zeros() as u8,
    ));
    Ok(builder
        .ins()
        .stack_addr(resources.module.target_config().pointer_type(), slot, 0))
}

fn defer_or_drop_owned_shared_temporary(
    builder: &mut FunctionBuilder,
    value: Value,
    ownership: mir::OwnedSharedTemporary,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    match ownership {
        mir::OwnedSharedTemporary::Strong => {
            defer_or_drop_shared_temporary(builder, value, false, resources)
        }
        mir::OwnedSharedTemporary::Weak => {
            defer_or_drop_shared_temporary(builder, value, true, resources)
        }
        mir::OwnedSharedTemporary::WritableStrong => defer_or_drop_writable_shared_temporary(
            builder,
            value,
            WRITABLE_SHARED_RELEASE,
            resources,
        ),
        mir::OwnedSharedTemporary::WritableWeak => defer_or_drop_writable_shared_temporary(
            builder,
            value,
            WRITABLE_SHARED_RELEASE_WEAK,
            resources,
        ),
        mir::OwnedSharedTemporary::ReadonlyAccess => defer_or_drop_writable_shared_temporary(
            builder,
            value,
            WRITABLE_SHARED_RELEASE_READONLY_ACCESS,
            resources,
        ),
        mir::OwnedSharedTemporary::WritableAccess => defer_or_drop_writable_shared_temporary(
            builder,
            value,
            WRITABLE_SHARED_RELEASE_WRITABLE_ACCESS,
            resources,
        ),
    }
}

fn lower_method_call_with_receiver(
    builder: &mut FunctionBuilder,
    receiver: Value,
    function: mir::FunctionId,
    args: &[mir::Rvalue],
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Option<LoweredValue>, BackendError> {
    let lowered = lower_call_args(builder, args, resources)?;
    let definition = function_in(resources.program, function)?;
    let mut values = vec![resources.current_frame];
    let payload_result = match definition.return_type {
        mir::ReturnType::Value(
            ty @ (mir::Type::PayloadEnum(_) | mir::Type::NullablePayloadEnum(_)),
        ) => {
            let address = create_payload_result_storage(builder, ty, resources)?;
            values.push(address);
            Some(address)
        }
        _ => None,
    };
    values.push(receiver);
    values.extend(lowered.abi_values.iter().copied());
    let callee = declared_function(builder, resources, function)?;
    let call = builder.ins().call(callee, &values);
    let results = builder.inst_results(call);
    let result = match definition.return_type {
        mir::ReturnType::Void => None,
        mir::ReturnType::Value(
            mir::Type::NullableScalar(_)
            | mir::Type::NullableString
            | mir::Type::Function(_)
            | mir::Type::NullableFunction(_),
        ) => {
            let ty = match definition.return_type {
                mir::ReturnType::Value(ty) => ty,
                mir::ReturnType::Void => unreachable!(),
            };
            Some(LoweredValue::Nullable {
                present: *results
                    .first()
                    .ok_or_else(|| malformed_mir("nullable method call has no presence result"))?,
                payload: nullable_payload_from_doria_abi(
                    builder,
                    *results.get(1).ok_or_else(|| {
                        malformed_mir("nullable method call has no payload result")
                    })?,
                    ty,
                ),
            })
        }
        mir::ReturnType::Value(mir::Type::PayloadEnum(_) | mir::Type::NullablePayloadEnum(_)) => {
            Some(LoweredValue::Single(payload_result.ok_or_else(|| {
                malformed_mir("payload enum method call has no result storage")
            })?))
        }
        mir::ReturnType::Value(ty) => {
            let value = *results
                .first()
                .ok_or_else(|| malformed_mir("method call has no result"))?;
            Some(LoweredValue::Single(value_from_doria_abi(
                builder, value, ty,
            )))
        }
    };
    for (_, string) in lowered.owned_strings {
        release_string(builder, string, resources)?;
    }
    for (index, value, ownership) in &lowered.temporary_mixed {
        if args[*index].transferred_owned_local().is_some() {
            continue;
        }
        let parameter = *definition.params.get(index + 1).ok_or_else(|| {
            malformed_mir(format!(
                "method function{} is missing parameter {}",
                function.0,
                index + 1
            ))
        })?;
        if !local_in(definition, parameter)?.owned {
            lower_cleanup_mixed_temporary(builder, *value, *ownership, resources)?;
        }
    }
    for index in ordered_owned_argument_indices(args) {
        let argument = &args[index];
        let parameter = *definition.params.get(index + 1).ok_or_else(|| {
            malformed_mir(format!(
                "method function{} is missing parameter {}",
                function.0,
                index + 1
            ))
        })?;
        if !local_in(definition, parameter)?.owned {
            let value = lowered.arguments[index].single()?;
            if let Some(class) = argument.owned_temporary_class() {
                defer_or_drop_class_temporary(builder, value, class, resources)?;
            } else if let Some(collection) = argument.owned_temporary_collection() {
                defer_or_drop_collection_temporary(builder, value, collection, resources)?;
            } else if let Some(shared) = argument.owned_temporary_shared() {
                defer_or_drop_owned_shared_temporary(builder, value, shared, resources)?;
            } else if let Some((payload, nullable)) = argument.owned_temporary_payload_enum() {
                lower_drop_payload_enum_at(builder, value, payload, nullable, resources)?;
            } else if argument.mixed_ownership().has_shell() {
                defer_or_cleanup_mixed_temporary(
                    builder,
                    value,
                    argument.mixed_ownership(),
                    resources,
                )?;
            }
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
        mir::BoolExpression::NullableFunctionIsPresent(value) => {
            let pointer = resources.module.target_config().pointer_type();
            let (descriptor, _) =
                lower_nullable_function_expression(builder, value, resources)?.nullable()?;
            let zero = builder.ins().iconst(pointer, 0);
            let present = builder.ins().icmp(IntCC::NotEqual, descriptor, zero);
            builder
                .ins()
                .brif(present, then_block, &[], else_block, &[]);
        }
        mir::BoolExpression::PayloadEnumIsCase {
            local,
            ty,
            case,
            nullable,
        } => {
            let pointer = resources.module.target_config().pointer_type();
            let source_slot = local_slot(resources.local_slots, *local)?;
            let mut source_address = builder.ins().stack_addr(pointer, source_slot, 0);
            if *nullable {
                let present = builder.ins().stack_load(pointer, types::I8, source_slot, 0);
                let zero = builder.ins().iconst(types::I8, 0);
                let is_present = builder.ins().icmp(IntCC::NotEqual, present, zero);
                let present_block = builder.create_block();
                builder
                    .ins()
                    .brif(is_present, present_block, &[], else_block, &[]);
                builder.switch_to_block(present_block);
                source_address = builder
                    .ins()
                    .iadd_imm_u(source_address, i64::from(ty.nullable_payload_offset));
            }
            let definition = enum_definition(resources.program, ty.id)?;
            let case_definition = definition
                .cases
                .get(case.index)
                .filter(|definition| definition.id == *case)
                .ok_or_else(|| malformed_mir("payload-enum case test references no case"))?;
            let tag_type = clif_tag_type(definition.layout.tag_width)?;
            let tag = builder.ins().load(
                tag_type,
                cranelift_codegen::ir::MachMemFlags::trusted(),
                source_address,
                definition.layout.tag_offset as i32,
            );
            let expected = builder
                .ins()
                .iconst(tag_type, i64::from(case_definition.tag));
            let matches = builder.ins().icmp(IntCC::Equal, tag, expected);
            builder
                .ins()
                .brif(matches, then_block, &[], else_block, &[]);
        }
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
                mir::ScalarType::Bool => builder.ins().icmp(bool_compare_code(*op), left, right),
                mir::ScalarType::Enum(_) => builder.ins().icmp(bool_compare_code(*op), left, right),
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
        mir::BoolExpression::NullablePayloadEnumIsPresent(value) => {
            let owned = value.owned_temporary();
            let ty = value.ty();
            let address = lower_nullable_payload_enum_expression(builder, value, resources)?;
            let present = builder.ins().load(
                types::I8,
                cranelift_codegen::ir::MachMemFlags::trusted(),
                address,
                0,
            );
            if owned {
                lower_drop_payload_enum_at(builder, address, ty, true, resources)?;
            }
            let zero = builder.ins().iconst(types::I8, 0);
            let present = builder.ins().icmp(IntCC::NotEqual, present, zero);
            builder
                .ins()
                .brif(present, then_block, &[], else_block, &[]);
        }
        mir::BoolExpression::PayloadEnumCompare { op, left, right } => {
            let ty = left.ty();
            let left_owned = left.owned_temporary();
            let right_owned = right.owned_temporary();
            let left_address = lower_payload_enum_expression(builder, left, resources)?;
            let right_address = lower_payload_enum_expression(builder, right, resources)?;
            let mut equal = lower_payload_enum_equal_value(
                builder,
                left_address,
                right_address,
                ty,
                resources,
            )?;
            if right_owned {
                lower_drop_payload_enum_at(builder, right_address, ty, false, resources)?;
            }
            if left_owned {
                lower_drop_payload_enum_at(builder, left_address, ty, false, resources)?;
            }
            if matches!(op, mir::CompareOp::NotEqual) {
                let zero = builder.ins().iconst(types::I8, 0);
                equal = builder.ins().icmp(IntCC::Equal, equal, zero);
            }
            builder.ins().brif(equal, then_block, &[], else_block, &[]);
        }
        mir::BoolExpression::NullablePayloadEnumCompare { op, left, right } => {
            let ty = left.ty();
            let left_owned = left.owned_temporary();
            let right_owned = right.owned_temporary();
            let left_address = lower_nullable_payload_enum_expression(builder, left, resources)?;
            let right_address = lower_nullable_payload_enum_expression(builder, right, resources)?;
            let mut equal = lower_nullable_payload_enum_equal_value(
                builder,
                left_address,
                right_address,
                ty,
                resources,
            )?;
            if right_owned {
                lower_drop_payload_enum_at(builder, right_address, ty, true, resources)?;
            }
            if left_owned {
                lower_drop_payload_enum_at(builder, left_address, ty, true, resources)?;
            }
            if matches!(op, mir::CompareOp::NotEqual) {
                let zero = builder.ins().iconst(types::I8, 0);
                equal = builder.ins().icmp(IntCC::Equal, equal, zero);
            }
            builder.ins().brif(equal, then_block, &[], else_block, &[]);
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
        mir::BoolExpression::NullableCollectionIsPresent(value) => {
            let owned = value.owned_temporary_collection();
            let value = lower_nullable_collection_expression(builder, value, resources)?;
            if let Some(collection) = owned {
                defer_or_drop_collection_temporary(builder, value, collection, resources)?;
            }
            let pointer = resources.module.target_config().pointer_type();
            let zero = builder.ins().iconst(pointer, 0);
            let present = builder.ins().icmp(IntCC::NotEqual, value, zero);
            builder
                .ins()
                .brif(present, then_block, &[], else_block, &[]);
        }
        mir::BoolExpression::NullableSharedReferenceIsPresent(value) => {
            let owned = value.owned_temporary().is_some();
            let value = lower_nullable_shared_reference_expression(builder, value, resources)?;
            if owned {
                defer_or_drop_shared_temporary(builder, value, false, resources)?;
            }
            let pointer = resources.module.target_config().pointer_type();
            let zero = builder.ins().iconst(pointer, 0);
            let present = builder.ins().icmp(IntCC::NotEqual, value, zero);
            builder
                .ins()
                .brif(present, then_block, &[], else_block, &[]);
        }
        mir::BoolExpression::NullableWeakReferenceIsPresent(value) => {
            let owned = value.owned_temporary().is_some();
            let value = lower_nullable_weak_reference_expression(builder, value, resources)?;
            if owned {
                defer_or_drop_shared_temporary(builder, value, true, resources)?;
            }
            let pointer = resources.module.target_config().pointer_type();
            let zero = builder.ins().iconst(pointer, 0);
            let present = builder.ins().icmp(IntCC::NotEqual, value, zero);
            builder
                .ins()
                .brif(present, then_block, &[], else_block, &[]);
        }
        mir::BoolExpression::NullableWritableSharedReferenceIsPresent(value) => {
            let owned = value.owned_temporary();
            let value =
                lower_nullable_writable_shared_reference_expression(builder, value, resources)?;
            if owned {
                defer_or_drop_writable_shared_temporary(
                    builder,
                    value,
                    WRITABLE_SHARED_RELEASE,
                    resources,
                )?;
            }
            let pointer = resources.module.target_config().pointer_type();
            let zero = builder.ins().iconst(pointer, 0);
            let present = builder.ins().icmp(IntCC::NotEqual, value, zero);
            builder
                .ins()
                .brif(present, then_block, &[], else_block, &[]);
        }
        mir::BoolExpression::NullableWritableWeakReferenceIsPresent(value) => {
            let owned = value.owned_temporary();
            let value =
                lower_nullable_writable_weak_reference_expression(builder, value, resources)?;
            if owned {
                defer_or_drop_writable_shared_temporary(
                    builder,
                    value,
                    WRITABLE_SHARED_RELEASE_WEAK,
                    resources,
                )?;
            }
            let pointer = resources.module.target_config().pointer_type();
            let zero = builder.ins().iconst(pointer, 0);
            let present = builder.ins().icmp(IntCC::NotEqual, value, zero);
            builder
                .ins()
                .brif(present, then_block, &[], else_block, &[]);
        }
        mir::BoolExpression::NullableSharedReferenceAccessIsPresent(value) => {
            let owned = value.owned_temporary();
            let lowered =
                lower_nullable_shared_reference_access_expression(builder, value, resources)?;
            if owned {
                let symbol = if value.writable() {
                    WRITABLE_SHARED_RELEASE_WRITABLE_ACCESS
                } else {
                    WRITABLE_SHARED_RELEASE_READONLY_ACCESS
                };
                defer_or_drop_writable_shared_temporary(builder, lowered, symbol, resources)?;
            }
            let pointer = resources.module.target_config().pointer_type();
            let zero = builder.ins().iconst(pointer, 0);
            let present = builder.ins().icmp(IntCC::NotEqual, lowered, zero);
            builder
                .ins()
                .brif(present, then_block, &[], else_block, &[]);
        }
        mir::BoolExpression::NullableMixedIsPresent(value) => {
            let ownership = value.ownership();
            let value = lower_nullable_mixed_expression(builder, value, resources)?;
            let pointer = resources.module.target_config().pointer_type();
            let zero = builder.ins().iconst(pointer, 0);
            let present = builder.ins().icmp(IntCC::NotEqual, value, zero);
            lower_cleanup_mixed_temporary(builder, value, ownership, resources)?;
            builder
                .ins()
                .brif(present, then_block, &[], else_block, &[]);
        }
        mir::BoolExpression::NullableErrorIsPresent(value) => {
            let value = lower_nullable_error_expression(builder, value, resources)?;
            let (object, _) = value.nullable()?;
            let pointer = resources.module.target_config().pointer_type();
            let zero = builder.ins().iconst(pointer, 0);
            let present = builder.ins().icmp(IntCC::NotEqual, object, zero);
            builder
                .ins()
                .brif(present, then_block, &[], else_block, &[]);
        }
        mir::BoolExpression::MixedIs { mixed, tag } => {
            let value = lower_mixed_is(builder, mixed, *tag, resources)?;
            builder.ins().brif(value, then_block, &[], else_block, &[]);
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
        mir::BoolExpression::CollectionHas {
            collection,
            value,
            op,
        } => {
            let pointer = resources.module.target_config().pointer_type();
            let local = local_definition(resources.program, resources.function_id, *collection)?;
            let mir::Type::Collection(collection_type) = local.ty else {
                return Err(malformed_mir("collection has uses non-collection local"));
            };
            let definition = collection_definition(resources.program, collection_type)?.clone();
            let stored_needle_type = if *op == mir::CollectionMembershipOp::Contains {
                definition.key.unwrap_or(definition.value)
            } else {
                definition.value
            };
            if let Some((payload, nullable)) = payload_enum_storage(stored_needle_type) {
                if *op == mir::CollectionMembershipOp::Add {
                    return Err(malformed_mir(
                        "payload enum elements cannot use set insertion",
                    ));
                }
                let owned = payload_enum_rvalue_is_owned(value);
                let needle = lower_rvalue(builder, value, resources)?.single()?;
                let collection_value = lower_collection_pointer(builder, *collection, resources)?;
                let (found, index) = lower_payload_enum_collection_search(
                    builder,
                    collection_value,
                    needle,
                    payload,
                    nullable,
                    resources,
                )?;
                if *op == mir::CollectionMembershipOp::Remove {
                    let remove = builder.create_block();
                    let removed = builder.create_block();
                    builder.ins().brif(found, remove, &[], removed, &[]);
                    builder.switch_to_block(remove);
                    let destination = create_payload_storage(builder, payload, nullable, resources);
                    let _ = runtime_call(
                        builder,
                        COLLECTION_AGGREGATE_REMOVE_AT_INTO,
                        &[pointer, pointer, pointer, pointer],
                        None,
                        &[
                            resources.current_frame,
                            collection_value,
                            index,
                            destination,
                        ],
                        resources,
                    )?;
                    lower_drop_payload_enum_at(builder, destination, payload, nullable, resources)?;
                    builder.ins().jump(removed, &[]);
                    builder.switch_to_block(removed);
                }
                if owned {
                    lower_drop_payload_enum_at(builder, needle, payload, nullable, resources)?;
                }
                builder.ins().brif(found, then_block, &[], else_block, &[]);
                return Ok(());
            }
            let mixed_ownership = value.mixed_ownership();
            let (needle_present, needle, needle_type) =
                if nullable_payload_type(stored_needle_type).is_some() {
                    lower_nullable_collection_parts(builder, value, stored_needle_type, resources)?
                } else {
                    (
                        builder.ins().iconst(types::I8, 1),
                        lower_rvalue(builder, value, resources)?.single()?,
                        stored_needle_type,
                    )
                };
            let needle_word = value_to_collection_word(builder, needle, needle_type, pointer)?;
            let collection_value = lower_collection_pointer(builder, *collection, resources)?;
            let kind = builder
                .ins()
                .iconst(types::I8, collection_compare_kind(needle_type)?);
            let result = match op {
                mir::CollectionMembershipOp::Contains
                | mir::CollectionMembershipOp::ContainsValue => {
                    let name = if *op == mir::CollectionMembershipOp::Contains
                        && definition.key.is_some()
                    {
                        COLLECTION_KEYED_HAS
                    } else {
                        COLLECTION_CONTAINS
                    };
                    let (params, args) = if name == COLLECTION_KEYED_HAS {
                        (
                            vec![pointer, types::I64, types::I8],
                            vec![collection_value, needle_word, kind],
                        )
                    } else {
                        (
                            vec![pointer, types::I64, types::I8, types::I8],
                            vec![collection_value, needle_word, needle_present, kind],
                        )
                    };
                    let found =
                        runtime_call(builder, name, &params, Some(types::I8), &args, resources)?
                            .ok_or_else(|| {
                                backend_failure("collection membership produced no result")
                            })?;
                    if matches!(
                        stored_needle_type,
                        mir::Type::Mixed | mir::Type::NullableMixed
                    ) {
                        lower_cleanup_mixed_temporary(builder, needle, mixed_ownership, resources)?;
                    } else {
                        lower_drop_stored_value(builder, needle, stored_needle_type, resources)?;
                    }
                    found
                }
                mir::CollectionMembershipOp::Add => {
                    let inserted = runtime_call(
                        builder,
                        COLLECTION_PUSH_UNIQUE,
                        &[pointer, types::I64, types::I8, types::I8],
                        Some(types::I8),
                        &[collection_value, needle_word, needle_present, kind],
                        resources,
                    )?
                    .ok_or_else(|| backend_failure("set insertion produced no result"))?;
                    if matches!(
                        stored_needle_type,
                        mir::Type::Mixed | mir::Type::NullableMixed
                    ) {
                        let zero = builder.ins().iconst(types::I8, 0);
                        let rejected = builder.ins().icmp(IntCC::Equal, inserted, zero);
                        lower_cleanup_mixed_temporary_if(
                            builder,
                            rejected,
                            needle,
                            mixed_ownership,
                            resources,
                        )?;
                    } else {
                        lower_drop_value_unless(
                            builder,
                            inserted,
                            needle,
                            stored_needle_type,
                            resources,
                        )?;
                    }
                    inserted
                }
                mir::CollectionMembershipOp::Remove => {
                    let removed_slot = builder.create_sized_stack_slot(StackSlotData::new(
                        StackSlotKind::ExplicitSlot,
                        8,
                        3,
                    ));
                    let removed_pointer = builder.ins().stack_addr(pointer, removed_slot, 0);
                    let removed_present_slot = builder.create_sized_stack_slot(StackSlotData::new(
                        StackSlotKind::ExplicitSlot,
                        1,
                        0,
                    ));
                    let removed_present_pointer =
                        builder.ins().stack_addr(pointer, removed_present_slot, 0);
                    let removed = runtime_call(
                        builder,
                        COLLECTION_REMOVE_VALUE,
                        &[pointer, types::I64, types::I8, types::I8, pointer, pointer],
                        Some(types::I8),
                        &[
                            collection_value,
                            needle_word,
                            needle_present,
                            kind,
                            removed_pointer,
                            removed_present_pointer,
                        ],
                        resources,
                    )?
                    .ok_or_else(|| backend_failure("set removal produced no result"))?;
                    let removed_word =
                        builder
                            .ins()
                            .stack_load(pointer, types::I64, removed_slot, 0);
                    let removed_value =
                        collection_word_to_value(builder, removed_word, needle_type, pointer)?;
                    let removed_present =
                        builder
                            .ins()
                            .stack_load(pointer, types::I8, removed_present_slot, 0);
                    let should_drop = builder.ins().band(removed, removed_present);
                    lower_drop_value_if(
                        builder,
                        should_drop,
                        removed_value,
                        stored_needle_type,
                        resources,
                    )?;
                    if matches!(
                        stored_needle_type,
                        mir::Type::Mixed | mir::Type::NullableMixed
                    ) {
                        lower_cleanup_mixed_temporary(builder, needle, mixed_ownership, resources)?;
                    } else {
                        lower_drop_stored_value(builder, needle, stored_needle_type, resources)?;
                    }
                    removed
                }
            };
            builder.ins().brif(result, then_block, &[], else_block, &[]);
        }
        mir::BoolExpression::CollectionIsEmpty { collection } => {
            let pointer = resources.module.target_config().pointer_type();
            let collection = lower_collection_pointer(builder, *collection, resources)?;
            let length = runtime_call(
                builder,
                COLLECTION_LENGTH,
                &[pointer],
                Some(pointer),
                &[collection],
                resources,
            )?
            .ok_or_else(|| backend_failure("collection length produced no result"))?;
            let zero = builder.ins().iconst(pointer, 0);
            let empty = builder.ins().icmp(IntCC::Equal, length, zero);
            builder.ins().brif(empty, then_block, &[], else_block, &[]);
        }
        mir::BoolExpression::CollectionEqual { left, right } => {
            let pointer = resources.module.target_config().pointer_type();
            let left = lower_collection_pointer(builder, *left, resources)?;
            let right = lower_collection_pointer(builder, *right, resources)?;
            let equal = runtime_call(
                builder,
                BYTES_EQUAL,
                &[pointer, pointer],
                Some(types::I8),
                &[left, right],
                resources,
            )?
            .ok_or_else(|| backend_failure("Bytes equality produced no result"))?;
            builder.ins().brif(equal, then_block, &[], else_block, &[]);
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

fn bool_compare_code(op: mir::CompareOp) -> IntCC {
    match op {
        mir::CompareOp::Equal => IntCC::Equal,
        mir::CompareOp::NotEqual => IntCC::NotEqual,
        mir::CompareOp::Less => IntCC::UnsignedLessThan,
        mir::CompareOp::LessEqual => IntCC::UnsignedLessThanOrEqual,
        mir::CompareOp::Greater => IntCC::UnsignedGreaterThan,
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
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    match operand {
        mir::Operand::StringIntrinsic(call) => {
            lower_string_intrinsic_call(builder, call, resources)?.single()
        }
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
            let pointer = resources.module.target_config().pointer_type();
            Ok(builder.ins().stack_load(
                pointer,
                types::I8,
                local_slot(resources.local_slots, *id)?,
                0,
            ))
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
                pointer,
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
        mir::Operand::CollectionIndex {
            positional,
            collection,
            index,
            remove,
        } => lower_collection_index(builder, *collection, index, *remove, *positional, resources),
        mir::Operand::CollectionKeyAt { collection, offset } => lower_collection_key_at(
            builder,
            *collection,
            offset,
            mir::Type::Scalar(mir::ScalarType::Bool),
            resources,
        ),
        mir::Operand::MixedPayload { mixed, tag } => {
            lower_mixed_payload(builder, *mixed, *tag, resources)
        }
        _ => Err(malformed_mir("bool expression contains non-bool constant")),
    }
}

fn lower_enum_operand(
    builder: &mut FunctionBuilder,
    enum_id: crate::enums::EnumId,
    operand: &mir::Operand,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let expected = mir::Type::Scalar(mir::ScalarType::Enum(enum_id));
    match operand {
        mir::Operand::Scalar(mir::ScalarValue::Enum(value)) if value.enum_id == enum_id => {
            Ok(builder.ins().iconst(types::I32, value.case_id.index as i64))
        }
        mir::Operand::Local(id) => {
            let definition = local_definition(resources.program, resources.function_id, *id)?;
            if definition.ty != expected {
                return Err(malformed_mir(
                    "enum expression reads a local of another type",
                ));
            }
            let pointer = resources.module.target_config().pointer_type();
            Ok(builder.ins().stack_load(
                pointer,
                types::I32,
                local_slot(resources.local_slots, *id)?,
                0,
            ))
        }
        mir::Operand::NullablePayload(id) => {
            let definition = local_definition(resources.program, resources.function_id, *id)?;
            if definition.ty != mir::Type::NullableScalar(mir::ScalarType::Enum(enum_id)) {
                return Err(malformed_mir(
                    "enum expression reads a nullable local of another type",
                ));
            }
            let pointer = resources.module.target_config().pointer_type();
            Ok(builder.ins().stack_load(
                pointer,
                types::I32,
                local_slot(resources.local_slots, *id)?,
                pointer.bytes() as i32,
            ))
        }
        mir::Operand::Static(id) => {
            let address = lower_static_address(builder, *id, resources)?;
            Ok(builder.ins().load(
                types::I32,
                cranelift_codegen::ir::MachMemFlags::trusted(),
                address,
                0,
            ))
        }
        mir::Operand::Property { object, property } => {
            let address = lower_property_address(builder, *object, *property, resources)?;
            Ok(builder.ins().load(
                types::I32,
                cranelift_codegen::ir::MachMemFlags::trusted(),
                address,
                0,
            ))
        }
        mir::Operand::CollectionIndex {
            positional,
            collection,
            index,
            remove,
        } => lower_collection_index(builder, *collection, index, *remove, *positional, resources),
        mir::Operand::CollectionKeyAt { collection, offset } => {
            lower_collection_key_at(builder, *collection, offset, expected, resources)
        }
        mir::Operand::MixedPayload { mixed, tag } => {
            lower_mixed_payload(builder, *mixed, *tag, resources)
        }
        _ => Err(malformed_mir(
            "enum expression contains another scalar type",
        )),
    }
}

#[allow(dead_code)]
fn resolve_string_locals(
    function: &mir::Function,
) -> Result<HashMap<mir::LocalId, Vec<u8>>, BackendError> {
    let mut definitions = HashMap::new();
    for block in &function.blocks {
        for statement in &block.statements {
            let (targets, value): (&[mir::LocalId], _) = match statement {
                mir::Statement::AssignLocal { target, value } => {
                    (std::slice::from_ref(target), value)
                }
                mir::Statement::AssignLocalGroup { targets, value } => (targets, value),
                _ => continue,
            };
            let mir::Rvalue::String(expression) = value else {
                if targets
                    .iter()
                    .any(|target| function.locals[target.0].ty == mir::Type::String)
                {
                    return Err(malformed_mir("string local has a non-string initializer"));
                }
                continue;
            };
            for target in targets {
                if function.locals[target.0].ty != mir::Type::String {
                    continue;
                }
                if definitions.insert(*target, expression.clone()).is_some() {
                    return Err(malformed_mir(format!(
                        "readonly string local local{} is assigned more than once",
                        target.0
                    )));
                }
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
        mir::StringExpression::MixedPayload(_) => {
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
        mir::StringExpression::Intrinsic(_)
        | mir::StringExpression::EnumBacking { .. }
        | mir::StringExpression::Display(_)
        | mir::StringExpression::Call { .. }
        | mir::StringExpression::ReadFile { .. }
        | mir::StringExpression::Format(_)
        | mir::StringExpression::Coalesce { .. }
        | mir::StringExpression::CollectionIndex { .. }
        | mir::StringExpression::CollectionKeyAt { .. }
        | mir::StringExpression::ErrorMessage(_) => {
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
        mir::StringExpression::MixedPayload(_) => {
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
        mir::StringExpression::Intrinsic(_)
        | mir::StringExpression::EnumBacking { .. }
        | mir::StringExpression::Display(_)
        | mir::StringExpression::Call { .. }
        | mir::StringExpression::ReadFile { .. }
        | mir::StringExpression::Format(_)
        | mir::StringExpression::Coalesce { .. }
        | mir::StringExpression::CollectionIndex { .. }
        | mir::StringExpression::CollectionKeyAt { .. }
        | mir::StringExpression::ErrorMessage(_) => {
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

fn lower_runtime_panic_code(
    builder: &mut FunctionBuilder,
    code: &[u8],
    message: &[u8],
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let pointer_type = resources.module.target_config().pointer_type();
    let code_pointer = define_data(builder, code, resources)?;
    let code_length = builder.ins().iconst(pointer_type, code.len() as i64);
    let message_pointer = define_data(builder, message, resources)?;
    let message_length = builder.ins().iconst(pointer_type, message.len() as i64);
    let panic_id = resources.declare_runtime(
        "dr_v2_panic_code",
        &[
            pointer_type,
            pointer_type,
            pointer_type,
            pointer_type,
            pointer_type,
        ],
        None,
    )?;
    let panic = resources
        .module
        .declare_func_in_func(panic_id, builder.func);
    builder.ins().call(
        panic,
        &[
            resources.current_frame,
            code_pointer,
            code_length,
            message_pointer,
            message_length,
        ],
    );
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
    Ok(builder.ins().symbol_value(pointer_type, global))
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
    Ok(builder.ins().symbol_value(pointer_type, global))
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

fn function_type_in(
    program: &mir::Program,
    id: mir::FunctionTypeId,
) -> Result<&mir::FunctionType, BackendError> {
    program
        .function_types
        .get(id.0)
        .filter(|definition| definition.id == id)
        .ok_or_else(|| malformed_mir(format!("function type#{} does not exist", id.0)))
}

fn closure_descriptor_in(
    program: &mir::Program,
    id: mir::ClosureDescriptorId,
) -> Result<&mir::ClosureDescriptor, BackendError> {
    program
        .closure_descriptors
        .get(id.0)
        .filter(|definition| definition.id == id)
        .ok_or_else(|| malformed_mir(format!("closure descriptor#{} does not exist", id.0)))
}

fn closure_environment_layout_in(
    program: &mir::Program,
    id: mir::ClosureEnvironmentLayoutId,
) -> Result<&mir::ClosureEnvironmentLayout, BackendError> {
    program
        .closure_environment_layouts
        .get(id.0)
        .filter(|definition| definition.id == id)
        .ok_or_else(|| {
            malformed_mir(format!(
                "closure environment layout#{} does not exist",
                id.0
            ))
        })
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

fn enum_definition(
    program: &mir::Program,
    enum_id: crate::enums::EnumId,
) -> Result<&mir::EnumDefinition, BackendError> {
    program
        .enums
        .get(enum_id.0)
        .filter(|definition| definition.id == enum_id)
        .ok_or_else(|| malformed_mir(format!("enum#{} does not exist", enum_id.0)))
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
    let object = builder
        .ins()
        .stack_load(pointer_type, pointer_type, slot, 0);
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

fn writable_shared_release_symbol(ty: mir::Type) -> Option<&'static str> {
    match ty {
        mir::Type::WritableSharedReference(_) | mir::Type::NullableWritableSharedReference(_) => {
            Some(WRITABLE_SHARED_RELEASE)
        }
        mir::Type::WritableWeakReference(_) | mir::Type::NullableWritableWeakReference(_) => {
            Some(WRITABLE_SHARED_RELEASE_WEAK)
        }
        mir::Type::ReadonlySharedReferenceAccess(_)
        | mir::Type::NullableReadonlySharedReferenceAccess(_) => {
            Some(WRITABLE_SHARED_RELEASE_READONLY_ACCESS)
        }
        mir::Type::WritableSharedReferenceAccess(_)
        | mir::Type::NullableWritableSharedReferenceAccess(_) => {
            Some(WRITABLE_SHARED_RELEASE_WRITABLE_ACCESS)
        }
        _ => None,
    }
}

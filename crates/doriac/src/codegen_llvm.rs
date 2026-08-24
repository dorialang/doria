use std::collections::{HashMap, HashSet};
use std::num::NonZeroU32;
use std::sync::OnceLock;

use inkwell::attributes::AttributeLoc;
use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::passes::PassBuilderOptions;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetData, TargetMachine,
};
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum, IntType, StructType};
use inkwell::values::{
    BasicMetadataValueEnum, BasicValue, BasicValueEnum, FloatValue as LlvmFloatValue,
    FunctionValue, GlobalValue, InstructionValue, IntValue, PointerValue, StructValue,
    UnnamedAddress,
};
use inkwell::{AddressSpace, FloatPredicate, IntPredicate, OptimizationLevel};

use crate::backend::BackendError;
use crate::format_string::{FormatConversion, FormatPiece};
use crate::mir;
use crate::mir_validation;
use crate::native_abi::{
    collection_comparator_code, collection_value_width, function_symbol,
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
    COLLECTION_FINISH_DETACHED_CLEANUP, COLLECTION_FREE, COLLECTION_INDEX_FIELD,
    COLLECTION_INDEX_OF, COLLECTION_INSERT_AT, COLLECTION_INSERT_AT_NULLABLE, COLLECTION_KEYED_GET,
    COLLECTION_KEYED_GET_NULLABLE, COLLECTION_KEYED_HAS, COLLECTION_KEYED_SET,
    COLLECTION_KEYED_SET_NULLABLE, COLLECTION_KEY_AT, COLLECTION_LENGTH, COLLECTION_LENGTH_FIELD,
    COLLECTION_NEW, COLLECTION_NULLABLE_ACCESS, COLLECTION_PUSH, COLLECTION_PUSH_FRONT,
    COLLECTION_PUSH_FRONT_NULLABLE, COLLECTION_PUSH_NULLABLE, COLLECTION_PUSH_UNIQUE,
    COLLECTION_REMOVE_AT, COLLECTION_REMOVE_VALUE, COLLECTION_RESET_AFTER_CLEANUP,
    COLLECTION_SET_ALGEBRA, COLLECTION_SET_AT, COLLECTION_SET_AT_NULLABLE,
    COLLECTION_STAGE26_FINALIZE, COLLECTION_STAGE26_FROM_COPY, COLLECTION_STAGE26_NEW,
    COLLECTION_VALUES_FIELD, COLLECTION_VALUE_AT, FLOAT_PARSE, FORMAT_F32, FORMAT_F64, FORMAT_I64,
    FORMAT_STRING, FORMAT_U64, INT_PARSE, MIXED_CLONE_OWNED, MIXED_FREE, MIXED_NEW,
    MIXED_NEW_AGGREGATE, MIXED_NEW_BORROWED, MIXED_PAYLOAD, MIXED_RELEASE_OWNED, MIXED_TAG,
    MIXED_TAG_BOOL, MIXED_TAG_CLASS, MIXED_TAG_ENUM, MIXED_TAG_ERROR, MIXED_TAG_FLOAT32,
    MIXED_TAG_FLOAT64, MIXED_TAG_INT16, MIXED_TAG_INT32, MIXED_TAG_INT64, MIXED_TAG_INT8,
    MIXED_TAG_PAYLOAD_ENUM, MIXED_TAG_STRING, MIXED_TAG_UINT16, MIXED_TAG_UINT32, MIXED_TAG_UINT64,
    MIXED_TAG_UINT8, MIXED_TYPE_ID, NULLABLE_STRING_EQUAL, PROCESS_EXIT, READ_FILE,
    READ_FILE_BYTES, READ_STDIN_BYTES, READ_STDIN_LINE_PROMPTED, SHARED_ACQUIRE, SHARED_CREATE,
    SHARED_CREATE_WEAK, SHARED_PAYLOAD, SHARED_RELEASE, SHARED_RELEASE_WEAK, SHARED_RETAIN,
    STRING_BYTE_LENGTH, STRING_COMPARE, STRING_CONCAT, STRING_CONTAINS,
    STRING_CONTAINS_IGNORE_CASE, STRING_COUNT_OCCURRENCES, STRING_DATA, STRING_ENDS_WITH,
    STRING_ENDS_WITH_IGNORE_CASE, STRING_EQUALS_IGNORE_CASE, STRING_FROM_BOOL, STRING_FROM_BYTES,
    STRING_FROM_F32, STRING_FROM_F64, STRING_FROM_I64, STRING_FROM_U64, STRING_FROM_UTF8,
    STRING_GRAPHEME_LENGTH, STRING_INDEX_OF, STRING_INDEX_OF_IGNORE_CASE, STRING_IS_EMPTY,
    STRING_JOIN, STRING_LAST_INDEX_OF, STRING_LAST_INDEX_OF_IGNORE_CASE, STRING_LOWER,
    STRING_LOWER_FIRST, STRING_PAD_END, STRING_PAD_START, STRING_RELEASE, STRING_REPEAT,
    STRING_REPLACE, STRING_RETAIN, STRING_SLICE, STRING_SPLIT, STRING_STARTS_WITH,
    STRING_STARTS_WITH_IGNORE_CASE, STRING_TO_BYTES, STRING_TRIM, STRING_TRIM_END,
    STRING_TRIM_START, STRING_UPPER, STRING_UPPER_FIRST, STRING_WRITE_STDERR, STRING_WRITE_STDOUT,
    WRITABLE_SHARED_ACQUIRE, WRITABLE_SHARED_ACQUIRE_READONLY_ACCESS,
    WRITABLE_SHARED_ACQUIRE_WRITABLE_ACCESS, WRITABLE_SHARED_CREATE, WRITABLE_SHARED_CREATE_WEAK,
    WRITABLE_SHARED_READONLY_PAYLOAD, WRITABLE_SHARED_RELEASE,
    WRITABLE_SHARED_RELEASE_READONLY_ACCESS, WRITABLE_SHARED_RELEASE_WEAK,
    WRITABLE_SHARED_RELEASE_WRITABLE_ACCESS, WRITABLE_SHARED_RETAIN,
    WRITABLE_SHARED_WRITABLE_PAYLOAD, WRITE_FILE, WRITE_FILE_BYTES, WRITE_STDERR_BYTES,
    WRITE_STDOUT_BYTES,
};
use crate::native_closure_abi;
use crate::numeric::{FloatType, FloatValue, IntegerPanic, IntegerType, IntegerValue};

pub fn lower_mir_to_object(program: &mir::Program) -> Result<Vec<u8>, BackendError> {
    mir_validation::validate_program(program)?;
    lower_validated_mir_to_object(program)
}

/// Emits the LLVM IR this backend produces for `program`, before any
/// optimization pass has run.
///
/// Tests assert structural properties of the code the compiler itself emits
/// with this. Reading the optimized module instead would measure what LLVM
/// happened to tidy up, which can mask a defect in what we emit.
pub fn lower_mir_to_llvm_ir(program: &mir::Program) -> Result<String, BackendError> {
    mir_validation::validate_program(program)?;
    let target_machine = host_target_machine()?;
    let context = Context::create();
    let module = build_module(&context, &target_machine, program)?;
    Ok(module.print_to_string().to_string())
}

fn host_target_machine() -> Result<TargetMachine, BackendError> {
    initialize_native_target()?;
    let triple = TargetMachine::get_default_triple();
    let target = Target::from_triple(&triple)
        .map_err(|error| backend_failure(format!("failed to select host LLVM target: {error}")))?;
    target
        .create_target_machine(
            &triple,
            "generic",
            "",
            OptimizationLevel::Aggressive,
            RelocMode::PIC,
            CodeModel::Default,
        )
        .ok_or_else(|| backend_failure("failed to create host LLVM target machine"))
}

pub(crate) fn lower_validated_mir_to_object(
    program: &mir::Program,
) -> Result<Vec<u8>, BackendError> {
    let target_machine = host_target_machine()?;
    let context = Context::create();
    let module = build_module(&context, &target_machine, program)?;

    let pass_options = PassBuilderOptions::create();
    pass_options.set_verify_each(true);
    module
        .run_passes("default<O3>", &target_machine, pass_options)
        .map_err(|error| backend_failure(format!("LLVM optimization failed: {error}")))?;
    module
        .verify()
        .map_err(|error| backend_failure(format!("optimized LLVM verification failed: {error}")))?;

    let object = target_machine
        .write_to_memory_buffer(&module, FileType::Object)
        .map_err(|error| backend_failure(format!("LLVM object emission failed: {error}")))?;
    Ok(object.as_slice().to_vec())
}

fn build_module<'ctx>(
    context: &'ctx Context,
    target_machine: &TargetMachine,
    program: &mir::Program,
) -> Result<Module<'ctx>, BackendError> {
    let triple = TargetMachine::get_default_triple();
    let module = context.create_module("doria_stage_15");
    module.set_triple(&triple);
    let target_data = target_machine.get_target_data();
    module.set_data_layout(&target_data.get_data_layout());

    let functions = declare_functions(context, &module, &target_data, program)?;
    let class_drop_functions = declare_class_drop_functions(context, &module, program);
    let collection_drop_functions = declare_collection_drop_functions(context, &module, program);
    let closure_drop_functions = declare_closure_drop_functions(context, &module, program);
    let closure_descriptors = declare_closure_descriptors(
        context,
        &module,
        program,
        &functions,
        &closure_drop_functions,
    )?;
    let statics = declare_statics(context, &module, &target_data, program)?;
    let (error_descriptors, error_origins) = declare_error_metadata(
        context,
        &module,
        &target_data,
        program,
        &class_drop_functions,
    )?;
    let declarations = DeclaredProgram {
        functions,
        class_drop_functions,
        collection_drop_functions,
        closure_drop_functions,
        closure_descriptors,
        statics,
        error_descriptors,
        error_origins,
    };
    for function in &program.functions {
        define_function(
            context,
            &module,
            &target_data,
            program,
            function,
            &declarations,
        )?;
    }
    define_class_drop_functions(context, &module, &target_data, program, &declarations)?;
    define_collection_drop_functions(context, &module, &target_data, program, &declarations)?;
    define_closure_drop_functions(context, &module, &target_data, program, &declarations)?;
    define_process_main(context, &module, &target_data, program, &declarations)?;

    module
        .verify()
        .map_err(|error| backend_failure(format!("LLVM verification failed: {error}")))?;
    Ok(module)
}

fn initialize_native_target() -> Result<(), BackendError> {
    static INITIALIZATION: OnceLock<Result<(), String>> = OnceLock::new();
    INITIALIZATION
        .get_or_init(|| Target::initialize_native(&InitializationConfig::default()))
        .as_ref()
        .map_err(|error| backend_failure(format!("failed to initialize host LLVM target: {error}")))
        .copied()
}

struct DeclaredProgram<'ctx> {
    functions: Vec<FunctionValue<'ctx>>,
    class_drop_functions: Vec<FunctionValue<'ctx>>,
    collection_drop_functions: Vec<FunctionValue<'ctx>>,
    closure_drop_functions: Vec<FunctionValue<'ctx>>,
    closure_descriptors: Vec<GlobalValue<'ctx>>,
    statics: Vec<GlobalValue<'ctx>>,
    error_descriptors: Vec<GlobalValue<'ctx>>,
    error_origins: Vec<GlobalValue<'ctx>>,
}

fn declare_closure_drop_functions<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    program: &mir::Program,
) -> Vec<FunctionValue<'ctx>> {
    let pointer = context.ptr_type(AddressSpace::default());
    let signature = context
        .void_type()
        .fn_type(&[pointer.into(), pointer.into()], false);
    program
        .closure_descriptors
        .iter()
        .map(|descriptor| {
            module.add_function(
                &format!("__doria_drop_closure_environment_{}", descriptor.id.0),
                signature,
                Some(Linkage::Internal),
            )
        })
        .collect()
}

fn declare_closure_descriptors<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    program: &mir::Program,
    functions: &[FunctionValue<'ctx>],
    closure_drop_functions: &[FunctionValue<'ctx>],
) -> Result<Vec<GlobalValue<'ctx>>, BackendError> {
    let ty = closure_descriptor_type(context);
    program
        .closure_descriptors
        .iter()
        .map(|descriptor| {
            let entry = functions
                .get(descriptor.entry_function.0)
                .ok_or_else(|| malformed_mir("closure entry function was not declared"))?
                .as_global_value()
                .as_pointer_value();
            let drop_environment = closure_drop_functions
                .get(descriptor.id.0)
                .ok_or_else(|| malformed_mir("closure drop function was not declared"))?
                .as_global_value()
                .as_pointer_value();
            let initializer = ty.const_named_struct(&[entry.into(), drop_environment.into()]);
            let global = module.add_global(
                ty,
                None,
                &format!("__doria_closure_descriptor_{}", descriptor.id.0),
            );
            global.set_initializer(&initializer);
            global.set_constant(true);
            global.set_linkage(Linkage::Internal);
            Ok(global)
        })
        .collect()
}

fn declare_class_drop_functions<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    program: &mir::Program,
) -> Vec<FunctionValue<'ctx>> {
    let pointer = context.ptr_type(AddressSpace::default());
    let signature = context
        .void_type()
        .fn_type(&[pointer.into(), pointer.into()], false);
    program
        .classes
        .iter()
        .map(|class| {
            module.add_function(
                &format!("__doria_drop_class_{}", class.id.0),
                signature,
                Some(Linkage::Internal),
            )
        })
        .collect()
}

fn declare_collection_drop_functions<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    program: &mir::Program,
) -> Vec<FunctionValue<'ctx>> {
    let pointer = context.ptr_type(AddressSpace::default());
    let signature = context
        .void_type()
        .fn_type(&[pointer.into(), pointer.into()], false);
    program
        .collection_types
        .iter()
        .map(|collection| {
            module.add_function(
                &format!("__doria_drop_collection_{}", collection.id.0),
                signature,
                Some(Linkage::Internal),
            )
        })
        .collect()
}

fn declare_functions<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    target_data: &TargetData,
    program: &mir::Program,
) -> Result<Vec<FunctionValue<'ctx>>, BackendError> {
    let mut functions = Vec::with_capacity(program.functions.len());
    for function in &program.functions {
        let function_type = function_type(context, target_data, function)?;
        let value = module.add_function(
            &function_symbol(function),
            function_type,
            Some(Linkage::Internal),
        );
        apply_function_abi_attributes(context, value, function)?;
        functions.push(value);
    }
    Ok(functions)
}

fn declare_statics<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    target_data: &TargetData,
    program: &mir::Program,
) -> Result<Vec<GlobalValue<'ctx>>, BackendError> {
    let usize_type = context.ptr_sized_int_type(target_data, None);
    let mut globals = Vec::with_capacity(program.statics.len());
    for property in &program.statics {
        let symbol = format!(
            "__doria_static_{}_{}_{}",
            property.class.0, property.id.0, property.name
        );
        let global = match &property.initializer {
            mir::StaticValue::Scalar(value) => {
                let initializer = scalar_constant(context, *value);
                let initializer = if matches!(property.ty, mir::Type::NullableScalar(_)) {
                    let ty = llvm_type(context, target_data, property.ty).into_struct_type();
                    ty.const_named_struct(&[usize_type.const_int(1, false).into(), initializer])
                        .into()
                } else {
                    initializer
                };
                let global = module.add_global(initializer.get_type(), None, &symbol);
                global.set_initializer(&initializer);
                global
            }
            mir::StaticValue::Null => {
                let ty = llvm_type(context, target_data, property.ty);
                let global = module.add_global(ty, None, &symbol);
                global.set_initializer(&ty.const_zero());
                global
            }
            mir::StaticValue::String(value) => {
                let bytes = context.const_string(value.as_bytes(), false);
                let object_type = context.struct_type(
                    &[
                        usize_type.into(),
                        usize_type.into(),
                        bytes.get_type().into(),
                    ],
                    false,
                );
                let object = object_type.const_named_struct(&[
                    usize_type.const_all_ones().into(),
                    usize_type.const_int(value.len() as u64, false).into(),
                    bytes.into(),
                ]);
                let object_global =
                    module.add_global(object_type, None, &format!("{symbol}_string"));
                object_global.set_initializer(&object);
                object_global.set_constant(true);
                object_global.set_linkage(Linkage::Private);
                object_global.set_unnamed_address(UnnamedAddress::Global);

                let initializer: BasicValueEnum<'ctx> =
                    if matches!(property.ty, mir::Type::NullableString) {
                        let ty = llvm_type(context, target_data, property.ty).into_struct_type();
                        ty.const_named_struct(&[
                            usize_type.const_int(1, false).into(),
                            object_global.as_pointer_value().into(),
                        ])
                        .into()
                    } else {
                        object_global.as_pointer_value().into()
                    };
                let global = module.add_global(initializer.get_type(), None, &symbol);
                global.set_initializer(&initializer);
                global
            }
            mir::StaticValue::PayloadEnum(value) => {
                let ty = llvm_type(context, target_data, property.ty);
                let global = module.add_global(ty, None, &symbol);
                global.set_initializer(&ty.const_zero());
                global.set_alignment(value.ty.align);
                global
            }
        };
        global.set_constant(
            !property.writable && !matches!(property.initializer, mir::StaticValue::PayloadEnum(_)),
        );
        global.set_linkage(Linkage::Internal);
        globals.push(global);
    }
    Ok(globals)
}

fn declare_error_metadata<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    target_data: &TargetData,
    program: &mir::Program,
    class_drop_functions: &[FunctionValue<'ctx>],
) -> Result<(Vec<GlobalValue<'ctx>>, Vec<GlobalValue<'ctx>>), BackendError> {
    let pointer = context.ptr_type(AddressSpace::default());
    let word = context.ptr_sized_int_type(target_data, None);
    let descriptor_type = error_descriptor_type(context, target_data);
    let mut descriptors = Vec::with_capacity(program.error_descriptors.len());
    for descriptor in &program.error_descriptors {
        let class = class_definition(program, descriptor.class)?;
        let message = class
            .layout
            .properties
            .iter()
            .find(|property| property.id == descriptor.message_property)
            .ok_or_else(|| malformed_mir("Error message property has no class layout"))?;
        let drop = class_drop_functions
            .get(descriptor.class.0)
            .ok_or_else(|| malformed_mir("Error class drop glue was not declared"))?
            .as_global_value()
            .as_pointer_value();
        let type_name = define_bytes(
            context,
            module,
            descriptor.type_name.as_bytes(),
            &format!("__doria_error_descriptor_{}_type_name", descriptor.id.0),
        );
        let initializer = descriptor_type.const_named_struct(&[
            type_name.into(),
            word.const_int(descriptor.type_name.len() as u64, false)
                .into(),
            word.const_int(u64::from(message.offset), false).into(),
            drop.into(),
            word.const_int(u64::from(class.layout.size), false).into(),
            word.const_int(
                u64::from(class.error_origin_offset.ok_or_else(|| {
                    malformed_mir("Error descriptor class has no hidden origin slot")
                })?),
                false,
            )
            .into(),
            word.const_zero().into(),
            word.const_zero().into(),
        ]);
        let global = module.add_global(
            descriptor_type,
            None,
            &format!("__doria_error_descriptor_{}", descriptor.id.0),
        );
        global.set_initializer(&initializer);
        global.set_constant(true);
        global.set_linkage(Linkage::Internal);
        descriptors.push(global);
    }

    let origin_type = context.struct_type(
        &[
            pointer.into(),
            word.into(),
            pointer.into(),
            word.into(),
            word.into(),
            word.into(),
            pointer.into(),
            word.into(),
        ],
        false,
    );
    let mut origins = Vec::with_capacity(program.error_origins.len());
    for origin in &program.error_origins {
        let prefix = format!("__doria_error_origin_{}", origin.id.0);
        let path = define_bytes(
            context,
            module,
            program.source.path.as_bytes(),
            &format!("{prefix}_path"),
        );
        let source = define_bytes(
            context,
            module,
            program.source.text.as_bytes(),
            &format!("{prefix}_source"),
        );
        let callable = define_bytes(
            context,
            module,
            origin.callable.as_bytes(),
            &format!("{prefix}_callable"),
        );
        let initializer = origin_type.const_named_struct(&[
            path.into(),
            word.const_int(program.source.path.len() as u64, false)
                .into(),
            source.into(),
            word.const_int(program.source.text.len() as u64, false)
                .into(),
            word.const_int(origin.span.start as u64, false).into(),
            word.const_int(origin.span.end as u64, false).into(),
            callable.into(),
            word.const_int(origin.callable.len() as u64, false).into(),
        ]);
        let global = module.add_global(origin_type, None, &prefix);
        global.set_initializer(&initializer);
        global.set_constant(true);
        global.set_linkage(Linkage::Internal);
        origins.push(global);
    }
    Ok((descriptors, origins))
}

fn function_type<'ctx>(
    context: &'ctx Context,
    target_data: &TargetData,
    function: &mir::Function,
) -> Result<inkwell::types::FunctionType<'ctx>, BackendError> {
    let pointer = context.ptr_type(AddressSpace::default());
    let signature_plan = native_closure_abi::NativeCallableSignaturePlan::direct(function);
    let mut parameters = signature_plan
        .hidden_inputs
        .iter()
        .map(|_| pointer.into())
        .collect::<Vec<BasicMetadataTypeEnum<'ctx>>>();
    let checked = !function.checked_effects.is_empty();
    for parameter in &function.params {
        let local = local_in(function, *parameter)?;
        parameters.push(
            if matches!(
                local.ty,
                mir::Type::PayloadEnum(_) | mir::Type::NullablePayloadEnum(_)
            ) {
                context.ptr_type(AddressSpace::default()).into()
            } else {
                llvm_type(context, target_data, local.ty).into()
            },
        );
    }
    if checked {
        return Ok(context.i8_type().fn_type(&parameters, false));
    }
    Ok(match function.return_type {
        mir::ReturnType::Void => context.void_type().fn_type(&parameters, false),
        mir::ReturnType::Value(mir::Type::PayloadEnum(_) | mir::Type::NullablePayloadEnum(_)) => {
            context.void_type().fn_type(&parameters, false)
        }
        mir::ReturnType::Value(ty) => {
            llvm_type(context, target_data, ty).fn_type(&parameters, false)
        }
    })
}

fn indirect_function_type<'ctx>(
    context: &'ctx Context,
    target_data: &TargetData,
    function: &mir::FunctionType,
) -> Result<inkwell::types::FunctionType<'ctx>, BackendError> {
    let pointer = context.ptr_type(AddressSpace::default());
    let signature_plan = native_closure_abi::NativeCallableSignaturePlan::indirect(function);
    let mut parameters = signature_plan
        .hidden_inputs
        .iter()
        .map(|_| pointer.into())
        .collect::<Vec<BasicMetadataTypeEnum<'ctx>>>();
    let checked = !function.checked_effects.is_empty();
    for parameter in &function.parameters {
        parameters.push(
            if matches!(
                parameter.ty,
                mir::Type::PayloadEnum(_) | mir::Type::NullablePayloadEnum(_)
            ) {
                pointer.into()
            } else {
                llvm_type(context, target_data, parameter.ty).into()
            },
        );
    }
    if checked {
        return Ok(context.i8_type().fn_type(&parameters, false));
    }
    Ok(match function.return_type {
        mir::ReturnType::Void => context.void_type().fn_type(&parameters, false),
        mir::ReturnType::Value(mir::Type::PayloadEnum(_) | mir::Type::NullablePayloadEnum(_)) => {
            context.void_type().fn_type(&parameters, false)
        }
        mir::ReturnType::Value(ty) => {
            llvm_type(context, target_data, ty).fn_type(&parameters, false)
        }
    })
}

fn apply_function_abi_attributes(
    context: &Context,
    llvm_function: FunctionValue<'_>,
    function: &mir::Function,
) -> Result<(), BackendError> {
    if function.checked_effects.is_empty() {
        if let mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Integer(ty))) =
            function.return_type
        {
            apply_integer_extension_attribute(context, llvm_function, AttributeLoc::Return, ty);
        }
    }
    let source_parameter_offset = native_closure_abi::NativeCallableSignaturePlan::direct(function)
        .source_parameter_offset() as u32;
    for (index, parameter) in function.params.iter().enumerate() {
        let local = local_in(function, *parameter)?;
        if let mir::Type::Scalar(mir::ScalarType::Integer(ty)) = local.ty {
            apply_integer_extension_attribute(
                context,
                llvm_function,
                AttributeLoc::Param(index as u32 + source_parameter_offset),
                ty,
            );
        }
    }
    Ok(())
}

fn apply_integer_extension_attribute(
    context: &Context,
    function: FunctionValue<'_>,
    location: AttributeLoc,
    ty: IntegerType,
) {
    if ty.bit_width() == 64 {
        return;
    }
    let name = if ty.is_signed() { "signext" } else { "zeroext" };
    let kind = inkwell::attributes::Attribute::get_named_enum_kind_id(name);
    function.add_attribute(location, context.create_enum_attribute(kind, 0));
}

fn define_function<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    target_data: &TargetData,
    program: &mir::Program,
    function: &mir::Function,
    declarations: &DeclaredProgram<'ctx>,
) -> Result<(), BackendError> {
    let llvm_function = *declarations
        .functions
        .get(function.id.0)
        .ok_or_else(|| malformed_mir(format!("function{} was not declared", function.id.0)))?;
    let builder = context.create_builder();
    let prologue = context.append_basic_block(llvm_function, "prologue");
    let blocks = function
        .blocks
        .iter()
        .map(|block| context.append_basic_block(llvm_function, &format!("block{}", block.id.0)))
        .collect::<Vec<_>>();
    builder.position_at_end(prologue);

    let mut local_slots = Vec::with_capacity(function.locals.len());
    for local in &function.locals {
        let ty = llvm_type(context, target_data, local.ty);
        let slot = build(builder.build_alloca(ty, &format!("local{}", local.id.0)))?;
        if let mir::Type::PayloadEnum(payload) | mir::Type::NullablePayloadEnum(payload) = local.ty
        {
            slot.as_instruction_value()
                .ok_or_else(|| backend_failure("payload enum alloca has no instruction"))?
                .set_alignment(payload.align)
                .map_err(|error| {
                    backend_failure(format!("failed to align payload enum local: {error}"))
                })?;
        }
        build(builder.build_store(slot, ty.const_zero()))?;
        local_slots.push(Some(slot));
    }
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
                target_data.get_pointer_byte_size(None),
            )?;
            let slot = build(builder.build_alloca(
                context.i8_type().array_type(layout.layout.size.max(1)),
                &format!("closure.environment.{}", descriptor.id.0),
            ))?;
            slot.as_instruction_value()
                .ok_or_else(|| backend_failure("closure environment alloca has no instruction"))?
                .set_alignment(layout.layout.align)
                .map_err(|error| {
                    backend_failure(format!("failed to align closure environment: {error}"))
                })?;
            Ok(Some(slot))
        })
        .collect::<Result<Vec<_>, BackendError>>()?;
    let mut deferred_class_temporary_slots =
        Vec::with_capacity(mir::class_temporary_capacity(function));
    for index in 0..mir::class_temporary_capacity(function) {
        let slot = build(builder.build_alloca(
            context.ptr_type(AddressSpace::default()),
            &format!("class.temporary.{index}"),
        ))?;
        build(builder.build_store(slot, context.ptr_type(AddressSpace::default()).const_null()))?;
        deferred_class_temporary_slots.push(slot);
    }
    let signature_plan = native_closure_abi::NativeCallableSignaturePlan::direct(function);
    let return_address = signature_plan
        .index_of(native_closure_abi::NativeCallableHiddenInput::ResultOut)
        .and_then(|index| llvm_function.get_nth_param(index as u32))
        .map(BasicValueEnum::into_pointer_value);
    let checked_error_address = signature_plan
        .index_of(native_closure_abi::NativeCallableHiddenInput::ErrorOut)
        .and_then(|index| llvm_function.get_nth_param(index as u32))
        .map(BasicValueEnum::into_pointer_value);
    for (index, parameter) in function.params.iter().enumerate() {
        let value = llvm_function
            .get_nth_param(index as u32 + signature_plan.source_parameter_offset() as u32)
            .ok_or_else(|| malformed_mir("LLVM function is missing a declared parameter"))?;
        let local = local_in(function, *parameter)?;
        let destination = local_slot(&local_slots, *parameter)?;
        if let mir::Type::PayloadEnum(payload) | mir::Type::NullablePayloadEnum(payload) = local.ty
        {
            let nullable = matches!(local.ty, mir::Type::NullablePayloadEnum(_));
            let size = context
                .ptr_sized_int_type(target_data, None)
                .const_int(u64::from(payload.storage_size(nullable)), false);
            build(builder.build_memcpy(
                destination,
                payload.align,
                value.into_pointer_value(),
                payload.align,
                size,
            ))?;
        } else {
            build(builder.build_store(destination, value))?;
        }
    }

    let pointer = context.ptr_type(AddressSpace::default());
    let usize_type = context.ptr_sized_int_type(target_data, None);
    let frame_type = context.struct_type(
        &[
            pointer.into(),
            pointer.into(),
            usize_type.into(),
            pointer.into(),
            usize_type.into(),
            pointer.into(),
            usize_type.into(),
            usize_type.into(),
            usize_type.into(),
            usize_type.into(),
            usize_type.into(),
        ],
        false,
    );
    let frame = build(builder.build_alloca(frame_type, "doria.frame.v2"))?;
    let parent = llvm_function
        .get_nth_param(
            signature_plan
                .index_of(native_closure_abi::NativeCallableHiddenInput::CurrentFrame)
                .expect("native callable plans always include a current frame") as u32,
        )
        .ok_or_else(|| malformed_mir("LLVM function is missing its parent frame"))?
        .into_pointer_value();
    let function_name = define_bytes(
        context,
        module,
        function.name.as_bytes(),
        &format!("__doria_function_name_{}", function.id.0),
    );
    let source_path = define_bytes(
        context,
        module,
        program.source.path.as_bytes(),
        &format!("__doria_source_path_{}", function.id.0),
    );
    let source_text = define_bytes(
        context,
        module,
        program.source.text.as_bytes(),
        &format!("__doria_source_text_{}", function.id.0),
    );
    let frame_values: [BasicValueEnum<'ctx>; 11] = [
        parent.into(),
        function_name.into(),
        usize_type
            .const_int(function.name.len() as u64, false)
            .into(),
        source_path.into(),
        usize_type
            .const_int(program.source.path.len() as u64, false)
            .into(),
        source_text.into(),
        usize_type
            .const_int(program.source.text.len() as u64, false)
            .into(),
        usize_type
            .const_int(function.source_span.start as u64, false)
            .into(),
        usize_type
            .const_int(function.source_span.end as u64, false)
            .into(),
        usize_type
            .const_int(function.source_span.start as u64, false)
            .into(),
        usize_type
            .const_int(function.source_span.end as u64, false)
            .into(),
    ];
    for (index, value) in frame_values.into_iter().enumerate() {
        let field =
            build(builder.build_struct_gep(frame_type, frame, index as u32, "doria.frame.field"))?;
        build(builder.build_store(field, value))?;
    }
    let current_frame = frame;
    let borrow_home_addresses = match (
        native_closure_abi::return_borrow_source_parameter(function)?,
        signature_plan.index_of(native_closure_abi::NativeCallableHiddenInput::BorrowHome),
    ) {
        (Some(local), Some(index)) => HashMap::from([(
            local,
            llvm_function
                .get_nth_param(index as u32)
                .ok_or_else(|| malformed_mir("LLVM function is missing its borrow home"))?
                .into_pointer_value(),
        )]),
        (None, None) => HashMap::new(),
        _ => {
            return Err(malformed_mir(
                "callable borrow-home ABI plan is inconsistent",
            ))
        }
    };
    let mut lowerer = FunctionLowerer {
        context,
        module,
        target_data,
        builder,
        entry_block: prologue,
        program,
        function,
        functions: &declarations.functions,
        class_drop_functions: &declarations.class_drop_functions,
        collection_drop_functions: &declarations.collection_drop_functions,
        closure_descriptors: &declarations.closure_descriptors,
        statics: &declarations.statics,
        error_descriptors: &declarations.error_descriptors,
        error_origins: &declarations.error_origins,
        local_slots,
        closure_environment_slots,
        closure_bound_fields: HashMap::new(),
        borrow_home_addresses,
        blocks,
        current_frame,
        return_address,
        checked_error_address,
        next_data_id: 0,
        defer_class_temporary_drops: false,
        deferred_class_temporary_slots,
        deferred_class_temporary_slot_cursor: 0,
        deferred_class_temporary_drops: Vec::new(),
    };
    lowerer.retain_string_parameters()?;
    build(
        lowerer
            .builder
            .build_unconditional_branch(block_for(&lowerer.blocks, function.entry_block)?),
    )?;
    for block in &function.blocks {
        lowerer
            .builder
            .position_at_end(block_for(&lowerer.blocks, block.id)?);
        lowerer.lower_block(block)?;
    }
    Ok(())
}

fn define_class_drop_functions<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    target_data: &TargetData,
    program: &mir::Program,
    declarations: &DeclaredProgram<'ctx>,
) -> Result<(), BackendError> {
    let function = function_in(program, program.entry)?;
    for class in &program.classes {
        let llvm_function = *declarations
            .class_drop_functions
            .get(class.id.0)
            .ok_or_else(|| {
                malformed_mir(format!(
                    "class{} drop function was not declared",
                    class.id.0
                ))
            })?;
        let builder = context.create_builder();
        let entry = context.append_basic_block(llvm_function, "entry");
        builder.position_at_end(entry);
        let current_frame = context.ptr_type(AddressSpace::default()).const_null();
        let object = llvm_function
            .get_nth_param(1)
            .ok_or_else(|| malformed_mir("class drop function is missing its object"))?
            .into_pointer_value();
        let mut lowerer = FunctionLowerer {
            context,
            module,
            target_data,
            builder,
            entry_block: entry,
            program,
            function,
            functions: &declarations.functions,
            class_drop_functions: &declarations.class_drop_functions,
            collection_drop_functions: &declarations.collection_drop_functions,
            closure_descriptors: &declarations.closure_descriptors,
            statics: &declarations.statics,
            error_descriptors: &declarations.error_descriptors,
            error_origins: &declarations.error_origins,
            local_slots: Vec::new(),
            closure_environment_slots: Vec::new(),
            closure_bound_fields: HashMap::new(),
            borrow_home_addresses: HashMap::new(),
            blocks: Vec::new(),
            current_frame,
            return_address: None,
            checked_error_address: None,
            next_data_id: 0,
            defer_class_temporary_drops: false,
            deferred_class_temporary_slots: Vec::new(),
            deferred_class_temporary_slot_cursor: 0,
            deferred_class_temporary_drops: Vec::new(),
        };
        lowerer.drop_class_value(object, class.id)?;
        build(lowerer.builder.build_return(None))?;
    }
    Ok(())
}

fn define_collection_drop_functions<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    target_data: &TargetData,
    program: &mir::Program,
    declarations: &DeclaredProgram<'ctx>,
) -> Result<(), BackendError> {
    let function = function_in(program, program.entry)?;
    for collection in &program.collection_types {
        let llvm_function = *declarations
            .collection_drop_functions
            .get(collection.id.0)
            .ok_or_else(|| malformed_mir("collection drop function was not declared"))?;
        let builder = context.create_builder();
        let entry = context.append_basic_block(llvm_function, "entry");
        builder.position_at_end(entry);
        let value = llvm_function
            .get_nth_param(1)
            .ok_or_else(|| malformed_mir("collection drop function is missing its value"))?
            .into_pointer_value();
        let mut lowerer = FunctionLowerer {
            context,
            module,
            target_data,
            builder,
            entry_block: entry,
            program,
            function,
            functions: &declarations.functions,
            class_drop_functions: &declarations.class_drop_functions,
            collection_drop_functions: &declarations.collection_drop_functions,
            closure_descriptors: &declarations.closure_descriptors,
            statics: &declarations.statics,
            error_descriptors: &declarations.error_descriptors,
            error_origins: &declarations.error_origins,
            local_slots: Vec::new(),
            closure_environment_slots: Vec::new(),
            closure_bound_fields: HashMap::new(),
            borrow_home_addresses: HashMap::new(),
            blocks: Vec::new(),
            current_frame: context.ptr_type(AddressSpace::default()).const_null(),
            return_address: None,
            checked_error_address: None,
            next_data_id: 0,
            defer_class_temporary_drops: false,
            deferred_class_temporary_slots: Vec::new(),
            deferred_class_temporary_slot_cursor: 0,
            deferred_class_temporary_drops: Vec::new(),
        };
        lowerer.drop_collection_value(value, collection.id)?;
        build(lowerer.builder.build_return(None))?;
    }
    Ok(())
}

fn define_closure_drop_functions<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    target_data: &TargetData,
    program: &mir::Program,
    declarations: &DeclaredProgram<'ctx>,
) -> Result<(), BackendError> {
    for descriptor in &program.closure_descriptors {
        let llvm_function = *declarations
            .closure_drop_functions
            .get(descriptor.id.0)
            .ok_or_else(|| malformed_mir("closure drop function was not declared"))?;
        let builder = context.create_builder();
        let entry = context.append_basic_block(llvm_function, "entry");
        builder.position_at_end(entry);
        let current_frame = llvm_function
            .get_nth_param(0)
            .ok_or_else(|| malformed_mir("closure drop function is missing its frame"))?
            .into_pointer_value();
        let environment = llvm_function
            .get_nth_param(1)
            .ok_or_else(|| malformed_mir("closure drop function is missing its environment"))?
            .into_pointer_value();
        let function = function_in(program, descriptor.entry_function)?;
        let mut lowerer = FunctionLowerer {
            context,
            module,
            target_data,
            builder,
            entry_block: entry,
            program,
            function,
            functions: &declarations.functions,
            class_drop_functions: &declarations.class_drop_functions,
            collection_drop_functions: &declarations.collection_drop_functions,
            closure_descriptors: &declarations.closure_descriptors,
            statics: &declarations.statics,
            error_descriptors: &declarations.error_descriptors,
            error_origins: &declarations.error_origins,
            local_slots: Vec::new(),
            closure_environment_slots: Vec::new(),
            closure_bound_fields: HashMap::new(),
            borrow_home_addresses: HashMap::new(),
            blocks: Vec::new(),
            current_frame,
            return_address: None,
            checked_error_address: None,
            next_data_id: 0,
            defer_class_temporary_drops: false,
            deferred_class_temporary_slots: Vec::new(),
            deferred_class_temporary_slot_cursor: 0,
            deferred_class_temporary_drops: Vec::new(),
        };
        if let Some(logical_id) = descriptor.environment_layout {
            let logical = closure_environment_layout_in(program, logical_id)?.clone();
            let native = native_closure_abi::environment_layout(
                program,
                logical_id,
                target_data.get_pointer_byte_size(None),
            )?;
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
                let byte = build(lowerer.builder.build_load(
                    context.i8_type(),
                    lowerer.byte_offset(environment, bit / 8, "closure.live.byte")?,
                    "closure.live",
                ))?
                .into_int_value();
                let mask = context.i8_type().const_int(1_u64 << (bit % 8), false);
                let live = build(lowerer.builder.build_and(byte, mask, "closure.live.mask"))?;
                let live = build(lowerer.builder.build_int_compare(
                    IntPredicate::NE,
                    live,
                    context.i8_type().const_zero(),
                    "closure.live.test",
                ))?;
                let drop_block = context.append_basic_block(llvm_function, "closure.field.drop");
                let next = context.append_basic_block(llvm_function, "closure.field.next");
                build(
                    lowerer
                        .builder
                        .build_conditional_branch(live, drop_block, next),
                )?;
                lowerer.builder.position_at_end(drop_block);
                let address = lowerer.byte_offset(environment, layout.offset, "closure.field")?;
                lowerer.drop_value_at_address(address, field.ty)?;
                lowerer.set_environment_live_bit(environment, bit, false)?;
                build(lowerer.builder.build_unconditional_branch(next))?;
                lowerer.builder.position_at_end(next);
            }
            if descriptor.environment_placement == mir::ClosureEnvironmentPlacement::Heap {
                let pointer = context.ptr_type(AddressSpace::default());
                let _ = lowerer.call_runtime(
                    CLOSURE_ENVIRONMENT_FREE,
                    &[pointer.into()],
                    None,
                    &[environment.into()],
                )?;
            }
        }
        build(lowerer.builder.build_return(None))?;
    }
    Ok(())
}

fn define_process_main<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    target_data: &TargetData,
    program: &mir::Program,
    declarations: &DeclaredProgram<'ctx>,
) -> Result<(), BackendError> {
    let entry = function_in(program, program.entry)?;
    let entry_function = *declarations
        .functions
        .get(program.entry.0)
        .ok_or_else(|| malformed_mir("entry function was not declared"))?;
    let pointer_type = context.ptr_type(AddressSpace::default());
    // The process entry point is C `main(int argc, char **argv)`. Both
    // parameters are always declared, even when the Doria entry ignores them,
    // so the platform start-up code always sees the signature it expects.
    let main = module.add_function(
        "main",
        context
            .i32_type()
            .fn_type(&[context.i32_type().into(), pointer_type.into()], false),
        Some(Linkage::External),
    );
    let builder = context.create_builder();
    let block = context.append_basic_block(main, "entry");
    builder.position_at_end(block);
    initialize_payload_statics(
        context,
        module,
        target_data,
        &builder,
        program,
        &declarations.statics,
    )?;
    let argc = main
        .get_nth_param(0)
        .ok_or_else(|| backend_failure("process entry is missing its argument count"))?;
    let argv = main
        .get_nth_param(1)
        .ok_or_else(|| backend_failure("process entry is missing its argument vector"))?;
    // Decision 0099: an entry that declares the argument list is invoked
    // through the `_args` runtime glue, which builds an owned `List<string>`,
    // lends it to `main`, and releases it afterwards.
    let takes_arguments = !entry.params.is_empty();
    if !takes_arguments {
        let validation = module
            .get_function("dr_v1_validate_entry_args")
            .unwrap_or_else(|| {
                module.add_function(
                    "dr_v1_validate_entry_args",
                    context
                        .void_type()
                        .fn_type(&[context.i32_type().into(), pointer_type.into()], false),
                    Some(Linkage::External),
                )
            });
        build(builder.build_call(
            validation,
            &[argc.into(), argv.into()],
            "process.args.validated",
        ))?;
    }
    let checked_entry = !entry.checked_effects.is_empty();
    let runtime_name = match (entry.return_type, takes_arguments, checked_entry) {
        (
            mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64))),
            false,
            false,
        ) => "dr_v2_main_int",
        (
            mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64))),
            true,
            false,
        ) => "dr_v2_main_int_args",
        (
            mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64))),
            false,
            true,
        ) => "dr_v3_main_checked_int",
        (
            mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64))),
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
            )))
        }
    };
    let integer_entry = matches!(
        entry.return_type,
        mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Integer(
            IntegerType::Int64
        )))
    );
    let usize_type = context.ptr_sized_int_type(target_data, None);
    let runtime = module.get_function(runtime_name).unwrap_or_else(|| {
        let mut parameters: Vec<BasicMetadataTypeEnum<'ctx>> = vec![pointer_type.into()];
        if takes_arguments {
            parameters.push(context.i32_type().into());
            parameters.push(pointer_type.into());
        }
        if integer_entry {
            parameters.push(pointer_type.into());
            parameters.push(usize_type.into());
            parameters.push(pointer_type.into());
            parameters.push(usize_type.into());
            parameters.push(usize_type.into());
            parameters.push(usize_type.into());
        }
        let signature = context.i32_type().fn_type(&parameters, false);
        module.add_function(runtime_name, signature, Some(Linkage::External))
    });
    let entry_pointer = entry_function.as_global_value().as_pointer_value();
    let mut runtime_args: Vec<BasicMetadataValueEnum<'ctx>> = vec![entry_pointer.into()];
    if takes_arguments {
        runtime_args.push(argc.into());
        runtime_args.push(argv.into());
    }
    if integer_entry {
        let source_path = define_bytes(
            context,
            module,
            program.source.path.as_bytes(),
            "__doria_process_source_path",
        );
        let source_text = define_bytes(
            context,
            module,
            program.source.text.as_bytes(),
            "__doria_process_source_text",
        );
        runtime_args.push(source_path.into());
        runtime_args.push(
            usize_type
                .const_int(program.source.path.len() as u64, false)
                .into(),
        );
        runtime_args.push(source_text.into());
        runtime_args.push(
            usize_type
                .const_int(program.source.text.len() as u64, false)
                .into(),
        );
        runtime_args.push(
            usize_type
                .const_int(entry.source_span.start as u64, false)
                .into(),
        );
        runtime_args.push(
            usize_type
                .const_int(entry.source_span.end as u64, false)
                .into(),
        );
    }
    let call = build(builder.build_call(runtime, &runtime_args, "process.status"))?;
    let status = call
        .try_as_basic_value()
        .basic()
        .ok_or_else(|| backend_failure("doria-rt process entry returned no status"))?;
    if cfg!(windows) {
        let exit = module.get_function(PROCESS_EXIT).unwrap_or_else(|| {
            module.add_function(
                PROCESS_EXIT,
                context
                    .void_type()
                    .fn_type(&[context.i32_type().into()], false),
                Some(Linkage::External),
            )
        });
        for name in ["cold", "noreturn"] {
            let kind = inkwell::attributes::Attribute::get_named_enum_kind_id(name);
            exit.add_attribute(
                AttributeLoc::Function,
                context.create_enum_attribute(kind, 0),
            );
        }
        build(builder.build_call(exit, &[status.into()], "process.exit"))?;
        build(builder.build_unreachable())?;
    } else {
        build(builder.build_return(Some(&status)))?;
    }
    Ok(())
}

fn initialize_payload_statics<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    target_data: &TargetData,
    builder: &Builder<'ctx>,
    program: &mir::Program,
    statics: &[GlobalValue<'ctx>],
) -> Result<(), BackendError> {
    for property in &program.statics {
        if !matches!(property.initializer, mir::StaticValue::PayloadEnum(_)) {
            continue;
        }
        let address = statics
            .get(property.id.0)
            .ok_or_else(|| malformed_mir("payload enum static was not declared"))?
            .as_pointer_value();
        initialize_payload_static_value_llvm(
            context,
            module,
            target_data,
            builder,
            program,
            &property.initializer,
            property.ty,
            address,
            &format!("__doria_static_init_{}", property.id.0),
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn initialize_payload_static_value_llvm<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    target_data: &TargetData,
    builder: &Builder<'ctx>,
    program: &mir::Program,
    value: &mir::StaticValue,
    ty: mir::Type,
    mut address: PointerValue<'ctx>,
    symbol: &str,
) -> Result<(), BackendError> {
    match (value, ty) {
        (mir::StaticValue::Scalar(value), mir::Type::Scalar(expected))
            if value.ty() == expected =>
        {
            build(builder.build_store(address, scalar_constant(context, *value)))?;
        }
        (mir::StaticValue::Scalar(value), mir::Type::NullableScalar(expected))
            if value.ty() == expected =>
        {
            let word = context.ptr_sized_int_type(target_data, None);
            build(builder.build_store(address, word.const_int(1, false)))?;
            let payload = llvm_byte_offset(
                context,
                target_data,
                builder,
                address,
                target_data.get_pointer_byte_size(None),
                "static.nullable.payload",
            )?;
            build(builder.build_store(payload, scalar_constant(context, *value)))?;
        }
        (mir::StaticValue::String(value), mir::Type::String | mir::Type::NullableString) => {
            let string = define_immortal_string(context, module, target_data, value, symbol);
            if matches!(ty, mir::Type::NullableString) {
                let word = context.ptr_sized_int_type(target_data, None);
                build(builder.build_store(address, word.const_int(1, false)))?;
                address = llvm_byte_offset(
                    context,
                    target_data,
                    builder,
                    address,
                    target_data.get_pointer_byte_size(None),
                    "static.nullable.string",
                )?;
            }
            build(builder.build_store(address, string))?;
        }
        (mir::StaticValue::Null, _) => {}
        (
            mir::StaticValue::PayloadEnum(value),
            mir::Type::PayloadEnum(expected) | mir::Type::NullablePayloadEnum(expected),
        ) if value.ty == expected => {
            if matches!(ty, mir::Type::NullablePayloadEnum(_)) {
                build(builder.build_store(address, context.i8_type().const_int(1, false)))?;
                address = llvm_byte_offset(
                    context,
                    target_data,
                    builder,
                    address,
                    expected.nullable_payload_offset,
                    "static.nullable.enum",
                )?;
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
                .ok_or_else(|| malformed_mir("payload enum static layout does not exist"))?;
            let tag_type = context
                .custom_width_int_type(
                    std::num::NonZeroU32::new(definition.layout.tag_width * 8)
                        .expect("enum tag width is nonzero"),
                )
                .map_err(|_| malformed_mir("payload enum tag width is unsupported"))?;
            let tag_address = llvm_byte_offset(
                context,
                target_data,
                builder,
                address,
                definition.layout.tag_offset,
                "static.enum.tag",
            )?;
            build(builder.build_store(tag_address, tag_type.const_int(case.tag.into(), false)))?;
            for (index, ((field, field_definition), field_layout)) in value
                .fields
                .iter()
                .zip(&case.payload)
                .zip(&layout.fields)
                .enumerate()
            {
                let field_address = llvm_byte_offset(
                    context,
                    target_data,
                    builder,
                    address,
                    field_layout.offset,
                    "static.enum.field",
                )?;
                initialize_payload_static_value_llvm(
                    context,
                    module,
                    target_data,
                    builder,
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

fn llvm_byte_offset<'ctx>(
    context: &'ctx Context,
    target_data: &TargetData,
    builder: &Builder<'ctx>,
    address: PointerValue<'ctx>,
    offset: u32,
    name: &str,
) -> Result<PointerValue<'ctx>, BackendError> {
    let offset = context
        .ptr_sized_int_type(target_data, None)
        .const_int(offset.into(), false);
    unsafe { build(builder.build_in_bounds_gep(context.i8_type(), address, &[offset], name)) }
}

fn define_immortal_string<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    target_data: &TargetData,
    value: &str,
    symbol: &str,
) -> PointerValue<'ctx> {
    let usize_type = context.ptr_sized_int_type(target_data, None);
    let bytes = context.const_string(value.as_bytes(), false);
    let object_type = context.struct_type(
        &[
            usize_type.into(),
            usize_type.into(),
            bytes.get_type().into(),
        ],
        false,
    );
    let object = object_type.const_named_struct(&[
        usize_type.const_all_ones().into(),
        usize_type.const_int(value.len() as u64, false).into(),
        bytes.into(),
    ]);
    let global = module.add_global(object_type, None, &format!("{symbol}_string"));
    global.set_initializer(&object);
    global.set_constant(true);
    global.set_linkage(Linkage::Private);
    global.set_unnamed_address(UnnamedAddress::Global);
    global.as_pointer_value()
}

struct FunctionLowerer<'ctx, 'program> {
    context: &'ctx Context,
    module: &'program Module<'ctx>,
    target_data: &'program TargetData,
    builder: Builder<'ctx>,
    /// The block every scratch slot is allocated in. See [`FunctionLowerer::entry_alloca`].
    entry_block: BasicBlock<'ctx>,
    program: &'program mir::Program,
    function: &'program mir::Function,
    functions: &'program [FunctionValue<'ctx>],
    class_drop_functions: &'program [FunctionValue<'ctx>],
    collection_drop_functions: &'program [FunctionValue<'ctx>],
    closure_descriptors: &'program [GlobalValue<'ctx>],
    statics: &'program [GlobalValue<'ctx>],
    error_descriptors: &'program [GlobalValue<'ctx>],
    error_origins: &'program [GlobalValue<'ctx>],
    local_slots: Vec<Option<PointerValue<'ctx>>>,
    closure_environment_slots: Vec<Option<PointerValue<'ctx>>>,
    closure_bound_fields: HashMap<mir::LocalId, BoundClosureField<'ctx>>,
    borrow_home_addresses: HashMap<mir::LocalId, PointerValue<'ctx>>,
    blocks: Vec<BasicBlock<'ctx>>,
    current_frame: PointerValue<'ctx>,
    return_address: Option<PointerValue<'ctx>>,
    checked_error_address: Option<PointerValue<'ctx>>,
    next_data_id: usize,
    defer_class_temporary_drops: bool,
    deferred_class_temporary_slots: Vec<PointerValue<'ctx>>,
    deferred_class_temporary_slot_cursor: usize,
    deferred_class_temporary_drops: Vec<(PointerValue<'ctx>, DeferredOwnedTemporary)>,
}

#[derive(Clone, Copy)]
struct BoundClosureField<'ctx> {
    address: PointerValue<'ctx>,
    storage: mir::ClosureEnvironmentStorage,
    ty: mir::Type,
}

struct LoweredCallArguments<'ctx> {
    values: Vec<BasicMetadataValueEnum<'ctx>>,
    lowered: Vec<BasicValueEnum<'ctx>>,
    owned_strings: Vec<PointerValue<'ctx>>,
    temporary_mixed: Vec<(usize, PointerValue<'ctx>, mir::MixedOwnership)>,
}

#[derive(Clone, Copy)]
enum CollectionMemoryRegion {
    Header,
    Values,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CollectionStorageAction {
    Free,
    Reset,
}

#[derive(Clone, Copy)]
enum DeferredOwnedTemporary {
    Class(crate::class_layout::ClassId),
    Collection(mir::CollectionTypeId),
    Mixed(mir::MixedOwnership),
    Shared(bool),
    WritableShared(&'static str),
}

impl<'ctx> FunctionLowerer<'ctx, '_> {
    /// Marks accesses using the runtime invariant that a collection header and
    /// its value buffer are separate allocations. Header fields remain one
    /// conservative alias class, as do all elements within the value buffer.
    fn tag_collection_memory_access(
        &self,
        instruction: InstructionValue<'ctx>,
        region: CollectionMemoryRegion,
    ) -> Result<(), BackendError> {
        let zero = self.context.i64_type().const_zero();
        let root = self.context.metadata_node(&[self
            .context
            .metadata_string("Doria collection memory")
            .into()]);
        let name = match region {
            CollectionMemoryRegion::Header => "Doria collection header",
            CollectionMemoryRegion::Values => "Doria collection values",
        };
        let scalar = self.context.metadata_node(&[
            self.context.metadata_string(name).into(),
            root.into(),
            zero.into(),
        ]);
        let access = self
            .context
            .metadata_node(&[scalar.into(), scalar.into(), zero.into()]);
        instruction
            .set_metadata(access, self.context.get_kind_id("tbaa"))
            .map_err(|error| {
                backend_failure(format!(
                    "failed to attach collection alias metadata: {error}"
                ))
            })
    }

    fn load_collection_memory<T: BasicType<'ctx>>(
        &self,
        ty: T,
        address: PointerValue<'ctx>,
        name: &str,
        region: CollectionMemoryRegion,
    ) -> Result<BasicValueEnum<'ctx>, BackendError> {
        let value = build(self.builder.build_load(ty, address, name))?;
        let instruction = value
            .as_instruction_value()
            .ok_or_else(|| backend_failure("collection load did not produce an instruction"))?;
        self.tag_collection_memory_access(instruction, region)?;
        Ok(value)
    }

    fn store_collection_memory(
        &self,
        address: PointerValue<'ctx>,
        value: BasicValueEnum<'ctx>,
        region: CollectionMemoryRegion,
    ) -> Result<(), BackendError> {
        let instruction = build(self.builder.build_store(address, value))?;
        self.tag_collection_memory_access(instruction, region)
    }

    /// Allocates a scratch slot in the function's entry block.
    ///
    /// LLVM only treats an `alloca` as part of the fixed frame when it sits in
    /// the entry block. One emitted at the current insertion point instead
    /// becomes a dynamic allocation that moves the stack pointer when it
    /// executes and is not reclaimed until the function returns, so a slot
    /// allocated inside a loop grows the frame on every iteration until the
    /// guard page is reached. Only the allocation moves; callers keep their
    /// stores at the current insertion point, so each pass over the
    /// surrounding code still initialises the slot before reading it.
    fn entry_alloca<T: BasicType<'ctx>>(
        &self,
        ty: T,
        name: &str,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let builder = self.context.create_builder();
        match self.entry_block.get_first_instruction() {
            Some(instruction) => builder.position_before(&instruction),
            None => builder.position_at_end(self.entry_block),
        }
        build(builder.build_alloca(ty, name))
    }

    fn error_parts(
        &self,
        value: StructValue<'ctx>,
    ) -> Result<(PointerValue<'ctx>, PointerValue<'ctx>), BackendError> {
        let object =
            build(self.builder.build_extract_value(value, 0, "error.object"))?.into_pointer_value();
        let descriptor = build(
            self.builder
                .build_extract_value(value, 1, "error.descriptor"),
        )?
        .into_pointer_value();
        Ok((object, descriptor))
    }

    fn error_value(
        &self,
        object: PointerValue<'ctx>,
        descriptor: PointerValue<'ctx>,
    ) -> Result<StructValue<'ctx>, BackendError> {
        let value = error_carrier_type(self.context).const_zero();
        let value = build(
            self.builder
                .build_insert_value(value, object, 0, "error.with-object"),
        )?
        .into_struct_value();
        Ok(build(
            self.builder
                .build_insert_value(value, descriptor, 1, "error.with-descriptor"),
        )?
        .into_struct_value())
    }

    fn error_descriptor_address(
        &self,
        id: mir::ErrorDescriptorId,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        self.error_descriptors
            .get(id.0)
            .filter(|_| {
                self.program
                    .error_descriptors
                    .get(id.0)
                    .is_some_and(|d| d.id == id)
            })
            .map(|global| (*global).as_pointer_value())
            .ok_or_else(|| malformed_mir(format!("Error descriptor{} was not declared", id.0)))
    }

    fn error_origin_address(
        &self,
        id: mir::ErrorOriginId,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        self.error_origins
            .get(id.0)
            .filter(|_| {
                self.program
                    .error_origins
                    .get(id.0)
                    .is_some_and(|o| o.id == id)
            })
            .map(|global| (*global).as_pointer_value())
            .ok_or_else(|| malformed_mir(format!("Error origin{} was not declared", id.0)))
    }

    fn drop_error_value(&mut self, value: StructValue<'ctx>) -> Result<(), BackendError> {
        let (object, descriptor) = self.error_parts(value)?;
        let present = build(self.builder.build_is_not_null(object, "error.drop.present"))?;
        let function = current_function(&self.builder)?;
        let drop_block = self.context.append_basic_block(function, "error.drop");
        let done = self.context.append_basic_block(function, "error.drop.done");
        build(
            self.builder
                .build_conditional_branch(present, drop_block, done),
        )?;
        self.builder.position_at_end(drop_block);
        let descriptor_type = error_descriptor_type(self.context, self.target_data);
        let drop_field = build(self.builder.build_struct_gep(
            descriptor_type,
            descriptor,
            3,
            "error.drop.field",
        ))?;
        let pointer = self.context.ptr_type(AddressSpace::default());
        let drop_function = build(self.builder.build_load(
            pointer,
            drop_field,
            "error.drop.function",
        ))?
        .into_pointer_value();
        let signature = self
            .context
            .void_type()
            .fn_type(&[pointer.into(), pointer.into()], false);
        let _ = build(self.builder.build_indirect_call(
            signature,
            drop_function,
            &[self.current_frame.into(), object.into()],
            "error.drop.call",
        ))?;
        build(self.builder.build_unconditional_branch(done))?;
        self.builder.position_at_end(done);
        Ok(())
    }

    fn entry_payload_alloca(
        &self,
        ty: mir::PayloadEnumType,
        nullable: bool,
        name: &str,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let slot = self.entry_alloca(
            self.context.i8_type().array_type(ty.storage_size(nullable)),
            name,
        )?;
        slot.as_instruction_value()
            .ok_or_else(|| backend_failure("payload enum alloca has no instruction"))?
            .set_alignment(ty.align)
            .map_err(|error| {
                backend_failure(format!("failed to align payload enum slot: {error}"))
            })?;
        Ok(slot)
    }

    fn copy_payload_bytes(
        &self,
        destination: PointerValue<'ctx>,
        source: PointerValue<'ctx>,
        ty: mir::PayloadEnumType,
        nullable: bool,
    ) -> Result<(), BackendError> {
        let size = self
            .context
            .ptr_sized_int_type(self.target_data, None)
            .const_int(u64::from(ty.storage_size(nullable)), false);
        build(
            self.builder
                .build_memcpy(destination, ty.align, source, ty.align, size),
        )?;
        Ok(())
    }

    fn zero_payload_bytes(
        &self,
        destination: PointerValue<'ctx>,
        ty: mir::PayloadEnumType,
        nullable: bool,
    ) -> Result<(), BackendError> {
        let size = self
            .context
            .ptr_sized_int_type(self.target_data, None)
            .const_int(u64::from(ty.storage_size(nullable)), false);
        build(self.builder.build_memset(
            destination,
            ty.align,
            self.context.i8_type().const_zero(),
            size,
        ))?;
        Ok(())
    }

    fn byte_offset(
        &self,
        address: PointerValue<'ctx>,
        offset: u32,
        name: &str,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let offset = self
            .context
            .ptr_sized_int_type(self.target_data, None)
            .const_int(u64::from(offset), false);
        unsafe {
            build(self.builder.build_in_bounds_gep(
                self.context.i8_type(),
                address,
                &[offset],
                name,
            ))
        }
    }

    fn nullable_type(&self, payload: BasicTypeEnum<'ctx>) -> inkwell::types::StructType<'ctx> {
        nullable_type(self.context, self.target_data, payload)
    }

    fn nullable_value(
        &self,
        present: IntValue<'ctx>,
        payload: BasicValueEnum<'ctx>,
    ) -> Result<StructValue<'ctx>, BackendError> {
        let value = self.nullable_type(payload.get_type()).get_undef();
        let value = build(
            self.builder
                .build_insert_value(value, present, 0, "nullable.present"),
        )?
        .into_struct_value();
        Ok(build(
            self.builder
                .build_insert_value(value, payload, 1, "nullable.payload"),
        )?
        .into_struct_value())
    }

    fn nullable_parts(
        &self,
        value: StructValue<'ctx>,
    ) -> Result<(IntValue<'ctx>, BasicValueEnum<'ctx>), BackendError> {
        let present = build(
            self.builder
                .build_extract_value(value, 0, "nullable.present"),
        )?
        .into_int_value();
        let payload = build(
            self.builder
                .build_extract_value(value, 1, "nullable.payload"),
        )?;
        Ok((present, payload))
    }

    fn closure_value(
        &self,
        descriptor: PointerValue<'ctx>,
        environment: PointerValue<'ctx>,
    ) -> Result<StructValue<'ctx>, BackendError> {
        let value = closure_carrier_type(self.context).get_undef();
        let value =
            build(
                self.builder
                    .build_insert_value(value, descriptor, 0, "closure.descriptor"),
            )?
            .into_struct_value();
        Ok(build(
            self.builder
                .build_insert_value(value, environment, 1, "closure.environment"),
        )?
        .into_struct_value())
    }

    fn closure_parts(
        &self,
        value: StructValue<'ctx>,
    ) -> Result<(PointerValue<'ctx>, PointerValue<'ctx>), BackendError> {
        let descriptor = build(
            self.builder
                .build_extract_value(value, 0, "closure.descriptor"),
        )?
        .into_pointer_value();
        let environment = build(
            self.builder
                .build_extract_value(value, 1, "closure.environment"),
        )?
        .into_pointer_value();
        Ok((descriptor, environment))
    }

    fn clear_function_slot(&self, slot: PointerValue<'ctx>) -> Result<(), BackendError> {
        build(
            self.builder
                .build_store(slot, closure_carrier_type(self.context).const_zero()),
        )?;
        Ok(())
    }

    fn set_environment_live_bit(
        &self,
        environment: PointerValue<'ctx>,
        bit: u32,
        live: bool,
    ) -> Result<(), BackendError> {
        let address = self.byte_offset(environment, bit / 8, "closure.live.byte")?;
        let current = build(self.builder.build_load(
            self.context.i8_type(),
            address,
            "closure.live",
        ))?
        .into_int_value();
        let mask = 1_u64 << (bit % 8);
        let next = if live {
            build(self.builder.build_or(
                current,
                self.context.i8_type().const_int(mask, false),
                "closure.live.set",
            ))?
        } else {
            build(self.builder.build_and(
                current,
                self.context.i8_type().const_int((!mask) & 0xff, false),
                "closure.live.clear",
            ))?
        };
        build(self.builder.build_store(address, next))?;
        Ok(())
    }

    fn drop_function_carrier(&mut self, value: StructValue<'ctx>) -> Result<(), BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let (descriptor, environment) = self.closure_parts(value)?;
        let present = build(
            self.builder
                .build_is_not_null(descriptor, "closure.drop.present"),
        )?;
        let has_environment = build(
            self.builder
                .build_is_not_null(environment, "closure.drop.environment-present"),
        )?;
        let should_drop = build(self.builder.build_and(
            present,
            has_environment,
            "closure.drop.required",
        ))?;
        let function = current_function(&self.builder)?;
        let drop_block = self.context.append_basic_block(function, "closure.drop");
        let done = self
            .context
            .append_basic_block(function, "closure.drop.done");
        build(
            self.builder
                .build_conditional_branch(should_drop, drop_block, done),
        )?;
        self.builder.position_at_end(drop_block);
        let descriptor_ty = closure_descriptor_type(self.context);
        let drop_field = build(self.builder.build_struct_gep(
            descriptor_ty,
            descriptor,
            1,
            "closure.drop.field",
        ))?;
        let drop_function = build(self.builder.build_load(
            pointer,
            drop_field,
            "closure.drop.function",
        ))?
        .into_pointer_value();
        let signature = self
            .context
            .void_type()
            .fn_type(&[pointer.into(), pointer.into()], false);
        let _ = build(self.builder.build_indirect_call(
            signature,
            drop_function,
            &[self.current_frame.into(), environment.into()],
            "closure.drop.call",
        ))?;
        build(self.builder.build_unconditional_branch(done))?;
        self.builder.position_at_end(done);
        Ok(())
    }

    fn bind_closure_environment(
        &mut self,
        environment_local: mir::LocalId,
        bindings: &[(mir::ClosureEnvironmentFieldId, mir::LocalId)],
    ) -> Result<(), BackendError> {
        let descriptor = self
            .program
            .closure_descriptors
            .iter()
            .find(|descriptor| descriptor.entry_function == self.function.id)
            .ok_or_else(|| malformed_mir("closure environment binding is outside a closure"))?
            .clone();
        let logical_id = descriptor
            .environment_layout
            .ok_or_else(|| malformed_mir("closure environment binding has no layout"))?;
        let logical = closure_environment_layout_in(self.program, logical_id)?.clone();
        let native = native_closure_abi::environment_layout(
            self.program,
            logical_id,
            self.target_data.get_pointer_byte_size(None),
        )?;
        let pointer = self.context.ptr_type(AddressSpace::default());
        let environment = build(self.builder.build_load(
            pointer,
            local_slot(&self.local_slots, environment_local)?,
            "closure.environment",
        ))?
        .into_pointer_value();
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
            let field_address =
                self.byte_offset(environment, layout.offset, "closure.binding.field")?;
            let place = match field.storage {
                mir::ClosureEnvironmentStorage::ReadonlyBorrow
                | mir::ClosureEnvironmentStorage::WritableBorrow => build(
                    self.builder
                        .build_load(pointer, field_address, "closure.binding.place"),
                )?
                .into_pointer_value(),
                mir::ClosureEnvironmentStorage::Owned => field_address,
            };
            let target_slot = local_slot(&self.local_slots, *target)?;
            if let mir::Type::PayloadEnum(payload) | mir::Type::NullablePayloadEnum(payload) =
                field.ty
            {
                self.copy_payload_bytes(
                    target_slot,
                    place,
                    payload,
                    matches!(field.ty, mir::Type::NullablePayloadEnum(_)),
                )?;
            } else {
                let value = build(self.builder.build_load(
                    llvm_type(self.context, self.target_data, field.ty),
                    place,
                    "closure.binding.value",
                ))?;
                build(self.builder.build_store(target_slot, value))?;
            }
            if matches!(field.ty, mir::Type::String | mir::Type::NullableString) {
                let stored = build(self.builder.build_load(
                    llvm_type(self.context, self.target_data, field.ty),
                    target_slot,
                    "closure.binding.string",
                ))?;
                let string = if field.ty == mir::Type::NullableString {
                    self.nullable_parts(stored.into_struct_value())?
                        .1
                        .into_pointer_value()
                } else {
                    stored.into_pointer_value()
                };
                let retained = self.retain_string(string)?;
                if field.ty == mir::Type::NullableString {
                    let (present, _) = self.nullable_parts(stored.into_struct_value())?;
                    let replacement = self.nullable_value(present, retained.into())?;
                    build(self.builder.build_store(target_slot, replacement))?;
                } else {
                    build(self.builder.build_store(target_slot, retained))?;
                }
            }
            if field.storage == mir::ClosureEnvironmentStorage::WritableBorrow
                && field.ty.transfers_writable_capture_ownership()
            {
                match field.ty {
                    mir::Type::PayloadEnum(payload) => {
                        self.zero_payload_bytes(place, payload, false)?;
                    }
                    mir::Type::NullablePayloadEnum(payload) => {
                        self.zero_payload_bytes(place, payload, true)?;
                    }
                    _ => {
                        build(self.builder.build_store(
                            place,
                            llvm_type(self.context, self.target_data, field.ty).const_zero(),
                        ))?;
                    }
                }
            }
            let address = if field.storage == mir::ClosureEnvironmentStorage::Owned {
                target_slot
            } else {
                place
            };
            self.closure_bound_fields.insert(
                *target,
                BoundClosureField {
                    address,
                    storage: field.storage,
                    ty: field.ty,
                },
            );
            if field.storage == mir::ClosureEnvironmentStorage::Owned
                && field.ty.has_move_ownership()
            {
                let zero = self.context.i8_type().const_zero();
                let size = self
                    .context
                    .ptr_sized_int_type(self.target_data, None)
                    .const_int(u64::from(layout.layout.size), false);
                build(
                    self.builder
                        .build_memset(field_address, layout.layout.align, zero, size),
                )?;
                if let Some(bit) = layout.live_bit {
                    self.set_environment_live_bit(environment, bit, false)?;
                }
            }
        }
        Ok(())
    }

    fn closure_source_address(
        &self,
        local: mir::LocalId,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        if let Some(field) = self.closure_bound_fields.get(&local) {
            return Ok(field.address);
        }
        if let Some(address) = self.borrow_home_addresses.get(&local) {
            return Ok(*address);
        }
        local_slot(&self.local_slots, local)
    }

    fn direct_call_borrow_home(
        &self,
        callee: &mir::Function,
        args: &[mir::Rvalue],
        separately_lowered_receiver: Option<PointerValue<'ctx>>,
    ) -> Result<Option<PointerValue<'ctx>>, BackendError> {
        let Some(return_borrow) = callee
            .return_borrow
            .filter(|_| native_closure_abi::returns_function_value(callee.return_type))
        else {
            return Ok(None);
        };
        let source = match (return_borrow.source, separately_lowered_receiver) {
            (mir::BorrowSource::Receiver, Some(_)) => {
                return Err(malformed_mir(
                    "borrow-returning null-safe method call has no stable receiver place",
                ));
            }
            (mir::BorrowSource::Parameter(index), Some(_)) => args.get(index),
            (_, None) => args.get(native_closure_abi::return_borrow_argument_index(
                return_borrow,
                callee.receiver_mode.is_some(),
            )),
        }
        .ok_or_else(|| {
            malformed_mir(format!(
                "borrow-returning call to {} has no source argument",
                callee.name
            ))
        })?;
        self.rvalue_borrow_home(source).map(Some)
    }

    fn indirect_call_borrow_home(
        &self,
        function_type: &mir::FunctionType,
        args: &[mir::Rvalue],
    ) -> Result<Option<PointerValue<'ctx>>, BackendError> {
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
            .ok_or_else(|| {
                malformed_mir("borrow-returning indirect call has no source argument")
            })?;
        self.rvalue_borrow_home(source).map(Some)
    }

    fn rvalue_borrow_home(&self, source: &mir::Rvalue) -> Result<PointerValue<'ctx>, BackendError> {
        let place = match source {
            mir::Rvalue::Value(mir::ValueExpression::Integer(mir::IntegerExpression::Use {
                operand,
                ..
            }))
            | mir::Rvalue::Value(mir::ValueExpression::Float(mir::FloatExpression::Use {
                operand,
                ..
            }))
            | mir::Rvalue::Value(mir::ValueExpression::Bool(mir::BoolExpression::Use {
                operand,
            }))
            | mir::Rvalue::Value(mir::ValueExpression::Enum(mir::EnumExpression::Use {
                operand,
                ..
            })) => return self.operand_borrow_home(operand),
            mir::Rvalue::String(mir::StringExpression::Local(local))
            | mir::Rvalue::String(mir::StringExpression::NullableLocalAssumeNonNull(local))
            | mir::Rvalue::Class(mir::ClassExpression::Local { local, .. })
            | mir::Rvalue::Class(mir::ClassExpression::NullableLocalAssumeNonNull {
                local, ..
            })
            | mir::Rvalue::Collection(mir::CollectionExpression::Local { local, .. })
            | mir::Rvalue::Mixed(mir::MixedExpression::Local { local, .. })
            | mir::Rvalue::Error(mir::ErrorExpression::Local { local, .. })
            | mir::Rvalue::Error(mir::ErrorExpression::NullableLocalAssumeNonNull {
                local, ..
            })
            | mir::Rvalue::Function(mir::FunctionExpression::Local { local, .. })
            | mir::Rvalue::NullableFunction(mir::NullableFunctionExpression::Local {
                local, ..
            })
            | mir::Rvalue::NullableScalar(mir::NullableScalarExpression::Local { local, .. })
            | mir::Rvalue::NullableString(mir::NullableStringExpression::Local(local))
            | mir::Rvalue::NullableClass(mir::NullableClassExpression::Local { local, .. })
            | mir::Rvalue::NullableCollection(mir::NullableCollectionExpression::Local {
                local,
                ..
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
                object,
                property,
                ..
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
            Some((local, None)) => self.closure_source_address(local),
            Some((object, Some(property))) => self.lower_property_address(object, property),
            None => Err(malformed_mir(
                "borrow-returning call source is not an addressable Doria place",
            )),
        }
    }

    fn operand_borrow_home(
        &self,
        operand: &mir::Operand,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        match operand {
            mir::Operand::Local(local) | mir::Operand::NullablePayload(local) => {
                self.closure_source_address(*local)
            }
            mir::Operand::Property { object, property } => {
                self.lower_property_address(*object, *property)
            }
            _ => Err(malformed_mir(
                "borrow-returning call source is not an addressable scalar place",
            )),
        }
    }

    fn sync_writable_closure_captures(&mut self) -> Result<(), BackendError> {
        let bindings = self
            .closure_bound_fields
            .iter()
            .filter_map(|(local, field)| {
                (field.storage == mir::ClosureEnvironmentStorage::WritableBorrow)
                    .then_some((*local, *field))
            })
            .collect::<Vec<_>>();
        for (local, field) in bindings {
            let slot = local_slot(&self.local_slots, local)?;
            match field.ty {
                mir::Type::Scalar(_) | mir::Type::NullableScalar(_) => {
                    let value = build(self.builder.build_load(
                        llvm_type(self.context, self.target_data, field.ty),
                        slot,
                        "closure.capture.new",
                    ))?;
                    build(self.builder.build_store(field.address, value))?;
                }
                mir::Type::String | mir::Type::NullableString => {
                    let ty = llvm_type(self.context, self.target_data, field.ty);
                    let new = build(self.builder.build_load(ty, slot, "closure.capture.new"))?;
                    let old = build(self.builder.build_load(
                        ty,
                        field.address,
                        "closure.capture.old",
                    ))?;
                    let new_string = if field.ty == mir::Type::NullableString {
                        self.nullable_parts(new.into_struct_value())?
                            .1
                            .into_pointer_value()
                    } else {
                        new.into_pointer_value()
                    };
                    let old_string = if field.ty == mir::Type::NullableString {
                        self.nullable_parts(old.into_struct_value())?
                            .1
                            .into_pointer_value()
                    } else {
                        old.into_pointer_value()
                    };
                    let retained = self.retain_string(new_string)?;
                    let replacement: BasicValueEnum<'ctx> = if field.ty == mir::Type::NullableString
                    {
                        let (present, _) = self.nullable_parts(new.into_struct_value())?;
                        self.nullable_value(present, retained.into())?.into()
                    } else {
                        retained.into()
                    };
                    build(self.builder.build_store(field.address, replacement))?;
                    self.release_string(old_string)?;
                }
                mir::Type::PayloadEnum(payload) | mir::Type::NullablePayloadEnum(payload) => {
                    let nullable = matches!(field.ty, mir::Type::NullablePayloadEnum(_));
                    self.copy_payload_bytes(field.address, slot, payload, nullable)?;
                    self.zero_payload_bytes(slot, payload, nullable)?;
                }
                mir::Type::Function(_) | mir::Type::NullableFunction(_) => {
                    let ty = closure_carrier_type(self.context);
                    let new = build(self.builder.build_load(ty, slot, "closure.capture.new"))?;
                    build(self.builder.build_store(field.address, new))?;
                    self.clear_function_slot(slot)?;
                }
                mir::Type::Error | mir::Type::NullableError => {
                    let ty = error_carrier_type(self.context);
                    let new = build(self.builder.build_load(ty, slot, "closure.capture.new"))?;
                    build(self.builder.build_store(field.address, new))?;
                    build(self.builder.build_store(slot, ty.const_zero()))?;
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
                    let pointer = self.context.ptr_type(AddressSpace::default());
                    let new = build(
                        self.builder
                            .build_load(pointer, slot, "closure.capture.new"),
                    )?;
                    build(self.builder.build_store(field.address, new))?;
                    build(self.builder.build_store(slot, pointer.const_null()))?;
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

    fn present_word(&self, present: bool) -> IntValue<'ctx> {
        self.context
            .ptr_sized_int_type(self.target_data, None)
            .const_int(u64::from(present), false)
    }

    fn lower_block(&mut self, block: &mir::BasicBlock) -> Result<(), BackendError> {
        for statement in &block.statements {
            self.lower_statement(statement)?;
        }
        self.lower_terminator(&block.terminator)
    }

    #[allow(clippy::too_many_arguments)]
    fn lower_indirect_call(
        &mut self,
        callee: &mir::FunctionExpression,
        function_type_id: mir::FunctionTypeId,
        invocation_mode: mir::FunctionInvocationMode,
        args: &[mir::Rvalue],
        result: Option<mir::LocalId>,
        continuation: mir::BlockId,
        span: crate::source::Span,
    ) -> Result<(), BackendError> {
        self.set_active_panic_site(span)?;
        let function_type = function_type_in(self.program, function_type_id)?.clone();
        if !function_type.checked_effects.is_empty() {
            return Err(malformed_mir(
                "throwing function type reached nonthrowing indirect call",
            ));
        }
        let carrier = self.lower_function_expression(callee)?;
        let (descriptor, environment) = self.closure_parts(carrier)?;
        let lowered = self.lower_call_arguments(args)?;
        let mut values = vec![self.current_frame.into()];
        let aggregate_return = matches!(
            function_type.return_type,
            mir::ReturnType::Value(mir::Type::PayloadEnum(_) | mir::Type::NullablePayloadEnum(_))
        );
        if aggregate_return {
            let result =
                result.ok_or_else(|| malformed_mir("indirect payload call has no result"))?;
            values.push(local_slot(&self.local_slots, result)?.into());
        }
        if let Some(home) = self.indirect_call_borrow_home(&function_type, args)? {
            values.push(home.into());
        }
        values.push(environment.into());
        values.extend(lowered.values.iter().copied());
        let entry = self.load_closure_entry(descriptor)?;
        let call = build(self.builder.build_indirect_call(
            indirect_function_type(self.context, self.target_data, &function_type)?,
            entry,
            &values,
            "closure.call",
        ))?;
        if let mir::ReturnType::Value(ty) = function_type.return_type {
            if !aggregate_return {
                let value = call
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| malformed_mir("indirect call produced no result"))?;
                let result = result
                    .ok_or_else(|| malformed_mir("value indirect call has no result local"))?;
                self.store_value_at_address(local_slot(&self.local_slots, result)?, value, ty)?;
            }
        }
        self.cleanup_indirect_call_arguments(&function_type, args, &lowered)?;
        if invocation_mode == mir::FunctionInvocationMode::Once {
            self.drop_function_carrier(carrier)?;
        }
        build(
            self.builder
                .build_unconditional_branch(block_for(&self.blocks, continuation)?),
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn lower_checked_indirect_call(
        &mut self,
        callee: &mir::FunctionExpression,
        function_type_id: mir::FunctionTypeId,
        invocation_mode: mir::FunctionInvocationMode,
        args: &[mir::Rvalue],
        result: Option<mir::LocalId>,
        error: mir::LocalId,
        success: mir::BlockId,
        failure: mir::BlockId,
        span: crate::source::Span,
    ) -> Result<(), BackendError> {
        self.set_active_panic_site(span)?;
        let function_type = function_type_in(self.program, function_type_id)?.clone();
        if function_type.checked_effects.is_empty() {
            return Err(malformed_mir(
                "nonthrowing function type reached checked indirect call",
            ));
        }
        let carrier = self.lower_function_expression(callee)?;
        let (descriptor, environment) = self.closure_parts(carrier)?;
        let lowered = self.lower_call_arguments(args)?;
        let mut values = vec![self.current_frame.into()];
        if let Some(result) = result {
            values.push(local_slot(&self.local_slots, result)?.into());
        }
        values.push(local_slot(&self.local_slots, error)?.into());
        if let Some(home) = self.indirect_call_borrow_home(&function_type, args)? {
            values.push(home.into());
        }
        values.push(environment.into());
        values.extend(lowered.values.iter().copied());
        let entry = self.load_closure_entry(descriptor)?;
        let call = build(self.builder.build_indirect_call(
            indirect_function_type(self.context, self.target_data, &function_type)?,
            entry,
            &values,
            "closure.checked.call",
        ))?;
        let status = call
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| malformed_mir("checked indirect call produced no status"))?
            .into_int_value();
        self.cleanup_indirect_call_arguments(&function_type, args, &lowered)?;
        if invocation_mode == mir::FunctionInvocationMode::Once {
            self.drop_function_carrier(carrier)?;
        }
        let current = current_function(&self.builder)?;
        let failed_status = self
            .context
            .append_basic_block(current, "closure.checked.failed-status");
        let invalid_status = self
            .context
            .append_basic_block(current, "closure.checked.invalid-status");
        let succeeded = build(self.builder.build_int_compare(
            IntPredicate::EQ,
            status,
            self.context.i8_type().const_zero(),
            "closure.checked.succeeded",
        ))?;
        build(self.builder.build_conditional_branch(
            succeeded,
            block_for(&self.blocks, success)?,
            failed_status,
        ))?;
        self.builder.position_at_end(failed_status);
        let failed = build(self.builder.build_int_compare(
            IntPredicate::EQ,
            status,
            self.context.i8_type().const_int(1, false),
            "closure.checked.failed",
        ))?;
        build(self.builder.build_conditional_branch(
            failed,
            block_for(&self.blocks, failure)?,
            invalid_status,
        ))?;
        self.builder.position_at_end(invalid_status);
        build(self.builder.build_unreachable())?;
        Ok(())
    }

    fn load_closure_entry(
        &self,
        descriptor: PointerValue<'ctx>,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let field = build(self.builder.build_struct_gep(
            closure_descriptor_type(self.context),
            descriptor,
            0,
            "closure.entry.field",
        ))?;
        Ok(build(self.builder.build_load(pointer, field, "closure.entry"))?.into_pointer_value())
    }

    fn cleanup_indirect_call_arguments(
        &mut self,
        function_type: &mir::FunctionType,
        args: &[mir::Rvalue],
        lowered: &LoweredCallArguments<'ctx>,
    ) -> Result<(), BackendError> {
        for string in &lowered.owned_strings {
            self.release_string(*string)?;
        }
        for index in ordered_owned_argument_indices(args) {
            let parameter = function_type
                .parameters
                .get(index)
                .ok_or_else(|| malformed_mir("indirect call parameter is missing"))?;
            if parameter.mode != mir::FunctionParameterMode::Take {
                let value = lowered.lowered[index];
                if let Some(class) = args[index].owned_temporary_class() {
                    self.defer_or_drop_class_temporary(value.into_pointer_value(), class)?;
                } else if let Some(collection) = args[index].owned_temporary_collection() {
                    self.defer_or_drop_collection_temporary(
                        value.into_pointer_value(),
                        collection,
                    )?;
                } else if let Some(shared) = args[index].owned_temporary_shared() {
                    self.defer_or_drop_owned_shared_temporary(value.into_pointer_value(), shared)?;
                } else if let Some((payload, nullable)) = args[index].owned_temporary_payload_enum()
                {
                    self.drop_payload_enum_at(value.into_pointer_value(), payload, nullable)?;
                }
            }
        }
        Ok(())
    }

    fn lower_statement(&mut self, statement: &mir::Statement) -> Result<(), BackendError> {
        debug_assert!(self.deferred_class_temporary_drops.is_empty());
        self.defer_class_temporary_drops = true;
        match statement {
            mir::Statement::BindClosureEnvironment {
                environment,
                bindings,
            } => self.bind_closure_environment(*environment, bindings)?,
            mir::Statement::DropFunction { local, .. } => {
                let slot = local_slot(&self.local_slots, *local)?;
                let carrier = build(self.builder.build_load(
                    closure_carrier_type(self.context),
                    slot,
                    "closure.drop.local",
                ))?
                .into_struct_value();
                self.drop_function_carrier(carrier)?;
                self.clear_function_slot(slot)?;
            }
            mir::Statement::BindPayloadEnumFields {
                source,
                ty,
                case,
                nullable,
                mode,
                targets,
            } => {
                let source = local_slot(&self.local_slots, *source)?;
                let source = if *nullable {
                    self.byte_offset(source, ty.nullable_payload_offset, "payload.binding.value")?
                } else {
                    source
                };
                let definition = enum_definition(self.program, ty.id)?.clone();
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
                    let source =
                        self.byte_offset(source, layout.offset, "payload.binding.field")?;
                    let target = local_slot(&self.local_slots, *target)?;
                    match field.ty {
                        mir::Type::PayloadEnum(payload) => {
                            self.copy_payload_bytes(target, source, payload, false)?;
                        }
                        mir::Type::NullablePayloadEnum(payload) => {
                            self.copy_payload_bytes(target, source, payload, true)?;
                        }
                        ty => {
                            let value = build(self.builder.build_load(
                                llvm_type(self.context, self.target_data, ty),
                                source,
                                "payload.binding.load",
                            ))?;
                            build(self.builder.build_store(target, value))?;
                        }
                    }
                    if matches!(mode, mir::MatchBindingMode::ConsumedArm)
                        && field.ty.has_move_ownership()
                    {
                        let size = match field.ty {
                            mir::Type::PayloadEnum(payload) => payload.storage_size(false),
                            mir::Type::NullablePayloadEnum(payload) => payload.storage_size(true),
                            _ => self.target_data.get_pointer_byte_size(None),
                        };
                        build(self.builder.build_memset(
                            source,
                            1,
                            self.context.i8_type().const_zero(),
                            self.context.i64_type().const_int(u64::from(size), false),
                        ))?;
                        continue;
                    }
                    if matches!(mode, mir::MatchBindingMode::GuardView) {
                        continue;
                    }
                    match field.ty {
                        mir::Type::String => {
                            let value = build(self.builder.build_load(
                                self.context.ptr_type(AddressSpace::default()),
                                target,
                                "payload.binding.string",
                            ))?
                            .into_pointer_value();
                            let value = self.retain_string(value)?;
                            build(self.builder.build_store(target, value))?;
                        }
                        mir::Type::NullableString => {
                            let value = build(self.builder.build_load(
                                llvm_type(self.context, self.target_data, field.ty),
                                target,
                                "payload.binding.nullable-string",
                            ))?
                            .into_struct_value();
                            let present = build(self.builder.build_extract_value(
                                value,
                                0,
                                "payload.binding.present",
                            ))?;
                            let payload = build(self.builder.build_extract_value(
                                value,
                                1,
                                "payload.binding.string-value",
                            ))?
                            .into_pointer_value();
                            let payload = self.retain_string(payload)?;
                            let value = build(self.builder.build_insert_value(
                                value,
                                present,
                                0,
                                "payload.binding.copy-present",
                            ))?
                            .into_struct_value();
                            let value = build(self.builder.build_insert_value(
                                value,
                                payload,
                                1,
                                "payload.binding.copy-string",
                            ))?;
                            build(self.builder.build_store(target, value))?;
                        }
                        mir::Type::PayloadEnum(payload) if payload.capabilities.copy => {
                            self.retain_payload_enum_at(target, payload, false)?;
                        }
                        mir::Type::NullablePayloadEnum(payload) if payload.capabilities.copy => {
                            self.retain_payload_enum_at(target, payload, true)?;
                        }
                        _ => {}
                    }
                }
            }
            mir::Statement::MatchResultPlan { .. } => {}
            mir::Statement::ControlFlowPlan(_) => {}
            mir::Statement::AssignLocalGroup { targets, value } => {
                let first = *targets
                    .first()
                    .ok_or_else(|| malformed_mir("grouped local assignment has no targets"))?;
                let ty = local_in(self.function, first)?.ty;
                let value = self.lower_rvalue(value)?;
                for (index, target) in targets.iter().enumerate() {
                    let value = if index == 0 {
                        value
                    } else {
                        match ty {
                            mir::Type::String => {
                                self.retain_string(value.into_pointer_value())?.into()
                            }
                            mir::Type::NullableString => {
                                let (present, payload) =
                                    self.nullable_parts(value.into_struct_value())?;
                                let retained = self.retain_string(payload.into_pointer_value())?;
                                self.nullable_value(present, retained.into())?.into()
                            }
                            _ => value,
                        }
                    };
                    let destination = local_slot(&self.local_slots, *target)?;
                    if let mir::Type::PayloadEnum(payload)
                    | mir::Type::NullablePayloadEnum(payload) = ty
                    {
                        let nullable = matches!(ty, mir::Type::NullablePayloadEnum(_));
                        self.copy_payload_bytes(
                            destination,
                            value.into_pointer_value(),
                            payload,
                            nullable,
                        )?;
                        if index > 0 {
                            self.retain_payload_enum_at(destination, payload, nullable)?;
                        }
                    } else {
                        build(self.builder.build_store(destination, value))?;
                    }
                }
            }
            mir::Statement::AssignLocal { target, value } => {
                let local = local_in(self.function, *target)?;
                let slot = local_slot(&self.local_slots, *target)?;
                let old_function = (local.owned
                    && matches!(
                        local.ty,
                        mir::Type::Function(_) | mir::Type::NullableFunction(_)
                    ))
                .then(|| {
                    build(self.builder.build_load(
                        closure_carrier_type(self.context),
                        slot,
                        "closure.old",
                    ))
                    .map(BasicValueEnum::into_struct_value)
                })
                .transpose()?;
                let old = match local.ty {
                    mir::Type::String => Some((
                        build(self.builder.build_load(
                            self.context.ptr_type(AddressSpace::default()),
                            slot,
                            "string.old",
                        ))?
                        .into_pointer_value(),
                        None,
                    )),
                    mir::Type::NullableString => {
                        let old = build(self.builder.build_load(
                            llvm_type(self.context, self.target_data, local.ty),
                            slot,
                            "nullable-string.old",
                        ))?
                        .into_struct_value();
                        Some((self.nullable_parts(old)?.1.into_pointer_value(), None))
                    }
                    mir::Type::Class(class) | mir::Type::NullableClass(class) if local.owned => {
                        Some((
                            build(self.builder.build_load(
                                self.context.ptr_type(AddressSpace::default()),
                                slot,
                                "class.old",
                            ))?
                            .into_pointer_value(),
                            Some(class),
                        ))
                    }
                    mir::Type::Collection(_) | mir::Type::NullableCollection(_) if local.owned => {
                        Some((
                            build(self.builder.build_load(
                                self.context.ptr_type(AddressSpace::default()),
                                slot,
                                "collection.old",
                            ))?
                            .into_pointer_value(),
                            None,
                        ))
                    }
                    mir::Type::Mixed | mir::Type::NullableMixed if local.owned => Some((
                        build(self.builder.build_load(
                            self.context.ptr_type(AddressSpace::default()),
                            slot,
                            "mixed.old",
                        ))?
                        .into_pointer_value(),
                        None,
                    )),
                    mir::Type::SharedReference(_)
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
                        if local.owned =>
                    {
                        Some((
                            build(self.builder.build_load(
                                self.context.ptr_type(AddressSpace::default()),
                                slot,
                                "shared.old",
                            ))?
                            .into_pointer_value(),
                            None,
                        ))
                    }
                    mir::Type::PayloadEnum(payload) | mir::Type::NullablePayloadEnum(payload)
                        if local.owned =>
                    {
                        let nullable = matches!(local.ty, mir::Type::NullablePayloadEnum(_));
                        let old =
                            self.entry_payload_alloca(payload, nullable, "local.payload.old")?;
                        self.copy_payload_bytes(old, slot, payload, nullable)?;
                        Some((old, None))
                    }
                    _ => None,
                };
                let value = self.lower_rvalue(value)?;
                if let mir::Type::PayloadEnum(payload) | mir::Type::NullablePayloadEnum(payload) =
                    local.ty
                {
                    self.copy_payload_bytes(
                        slot,
                        value.into_pointer_value(),
                        payload,
                        matches!(local.ty, mir::Type::NullablePayloadEnum(_)),
                    )?;
                } else {
                    build(self.builder.build_store(slot, value))?;
                }
                if let Some((old, class)) = old {
                    if let mir::Type::Collection(collection)
                    | mir::Type::NullableCollection(collection) = local.ty
                    {
                        self.drop_collection_value(old, collection)?;
                    } else if matches!(local.ty, mir::Type::Mixed | mir::Type::NullableMixed) {
                        self.drop_mixed_value(old)?;
                    } else if matches!(
                        local.ty,
                        mir::Type::WeakReference(_) | mir::Type::NullableWeakReference(_)
                    ) {
                        self.drop_shared_value(old, true)?;
                    } else if let Some(symbol) = writable_shared_release_symbol(local.ty) {
                        self.drop_writable_shared_value(old, symbol)?;
                    } else if matches!(
                        local.ty,
                        mir::Type::SharedReference(_) | mir::Type::NullableSharedReference(_)
                    ) {
                        self.drop_shared_value(old, false)?;
                    } else if let Some(class) = class {
                        self.drop_class_value_checked(old, class)?;
                    } else if let mir::Type::PayloadEnum(payload) = local.ty {
                        self.drop_payload_enum_at(old, payload, false)?;
                    } else if let mir::Type::NullablePayloadEnum(payload) = local.ty {
                        self.drop_payload_enum_at(old, payload, true)?;
                    } else {
                        self.release_string(old)?;
                    }
                }
                if let Some(old) = old_function {
                    self.drop_function_carrier(old)?;
                }
            }
            mir::Statement::EchoStringLiteral(value) => self.lower_echo(value.as_bytes())?,
            mir::Statement::EchoString(value) => {
                let value = self.lower_string_expression(value)?;
                let pointer = self.context.ptr_type(AddressSpace::default());
                let _ = self.call_runtime(
                    STRING_WRITE_STDOUT,
                    &[pointer.into(), pointer.into()],
                    None,
                    &[self.current_frame.into(), value.into()],
                )?;
                self.release_string(value)?;
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
                let _ = self.lower_call_at(*function, args, false, *span)?;
            }
            mir::Statement::CallNullSafe {
                object,
                function,
                args,
                span,
            } => {
                self.set_active_panic_site(*span)?;
                self.lower_null_safe_statement_call(object, *function, args)?
            }
            mir::Statement::WriteStderr(value) => {
                let value = self.lower_string_expression(value)?;
                let pointer = self.context.ptr_type(AddressSpace::default());
                let _ = self.call_runtime(
                    STRING_WRITE_STDERR,
                    &[pointer.into(), pointer.into()],
                    None,
                    &[self.current_frame.into(), value.into()],
                )?;
                self.release_string(value)?;
            }
            mir::Statement::Printf(format) => {
                let value = self.lower_format_expression(format)?;
                let pointer = self.context.ptr_type(AddressSpace::default());
                let _ = self.call_runtime(
                    STRING_WRITE_STDOUT,
                    &[pointer.into(), pointer.into()],
                    None,
                    &[self.current_frame.into(), value.into()],
                )?;
                self.release_string(value)?;
            }
            mir::Statement::WriteFile { path, contents }
            | mir::Statement::AppendFile { path, contents } => {
                let path = self.lower_string_expression(path)?;
                let contents = self.lower_string_expression(contents)?;
                let pointer = self.context.ptr_type(AddressSpace::default());
                let _ = self.call_runtime(
                    if matches!(statement, mir::Statement::AppendFile { .. }) {
                        APPEND_FILE
                    } else {
                        WRITE_FILE
                    },
                    &[pointer.into(), pointer.into(), pointer.into()],
                    None,
                    &[self.current_frame.into(), path.into(), contents.into()],
                )?;
                self.release_string(path)?;
                self.release_string(contents)?;
            }
            mir::Statement::WriteFileBytes {
                path,
                contents,
                append,
            } => {
                let path = self.lower_string_expression(path)?;
                let contents = self.collection_pointer(*contents)?;
                let pointer = self.context.ptr_type(AddressSpace::default());
                let _ = self.call_runtime(
                    if *append {
                        APPEND_FILE_BYTES
                    } else {
                        WRITE_FILE_BYTES
                    },
                    &[pointer.into(), pointer.into(), pointer.into()],
                    None,
                    &[self.current_frame.into(), path.into(), contents.into()],
                )?;
                self.release_string(path)?;
            }
            mir::Statement::WriteStreamBytes { contents, stderr } => {
                let contents = self.collection_pointer(*contents)?;
                let pointer = self.context.ptr_type(AddressSpace::default());
                let _ = self.call_runtime(
                    if *stderr {
                        WRITE_STDERR_BYTES
                    } else {
                        WRITE_STDOUT_BYTES
                    },
                    &[pointer.into(), pointer.into()],
                    None,
                    &[self.current_frame.into(), contents.into()],
                )?;
            }
            mir::Statement::AssignProperty {
                object,
                property,
                value,
                kind,
                ..
            } => {
                let property_ty = property_definition(self.program, *property)?.ty;
                let value = self.lower_rvalue(value)?;
                let address = self.lower_property_address(*object, *property)?;
                let replaces = !matches!(kind, mir::PropertyWriteKind::Initialize);
                let old_error = (replaces
                    && matches!(property_ty, mir::Type::Error | mir::Type::NullableError))
                .then(|| {
                    build(self.builder.build_load(
                        error_carrier_type(self.context),
                        address,
                        "property.old.error",
                    ))
                    .map(BasicValueEnum::into_struct_value)
                })
                .transpose()?;
                let old_function = (replaces
                    && matches!(
                        property_ty,
                        mir::Type::Function(_) | mir::Type::NullableFunction(_)
                    ))
                .then(|| {
                    build(self.builder.build_load(
                        closure_carrier_type(self.context),
                        address,
                        "property.old.function",
                    ))
                    .map(BasicValueEnum::into_struct_value)
                })
                .transpose()?;
                let old = if replaces {
                    match property_ty {
                        mir::Type::String
                        | mir::Type::Class(_)
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
                        | mir::Type::NullableWritableSharedReferenceAccess(_) => Some(
                            build(self.builder.build_load(
                                self.context.ptr_type(AddressSpace::default()),
                                address,
                                "property.old",
                            ))?
                            .into_pointer_value(),
                        ),
                        mir::Type::NullableString => {
                            let value = build(self.builder.build_load(
                                llvm_type(self.context, self.target_data, property_ty),
                                address,
                                "property.old",
                            ))?
                            .into_struct_value();
                            Some(self.nullable_parts(value)?.1.into_pointer_value())
                        }
                        mir::Type::PayloadEnum(payload)
                        | mir::Type::NullablePayloadEnum(payload) => {
                            let nullable = matches!(property_ty, mir::Type::NullablePayloadEnum(_));
                            let old = self.entry_payload_alloca(
                                payload,
                                nullable,
                                "property.payload.old",
                            )?;
                            self.copy_payload_bytes(old, address, payload, nullable)?;
                            Some(old)
                        }
                        mir::Type::Scalar(_)
                        | mir::Type::NullableScalar(_)
                        | mir::Type::Error
                        | mir::Type::NullableError => None,
                        mir::Type::Function(_) | mir::Type::NullableFunction(_) => None,
                        mir::Type::ClosureEnvironment(_) => {
                            return Err(malformed_mir(
                                "closure environment pointer is not a property value",
                            ));
                        }
                    }
                } else {
                    None
                };
                if let mir::Type::PayloadEnum(payload) | mir::Type::NullablePayloadEnum(payload) =
                    property_ty
                {
                    self.copy_payload_bytes(
                        address,
                        value.into_pointer_value(),
                        payload,
                        matches!(property_ty, mir::Type::NullablePayloadEnum(_)),
                    )?;
                } else {
                    build(self.builder.build_store(address, value))?;
                }
                match (property_ty, old) {
                    (mir::Type::String | mir::Type::NullableString, Some(value)) => {
                        self.release_string(value)?;
                    }
                    (mir::Type::Class(class) | mir::Type::NullableClass(class), Some(value)) => {
                        self.drop_class_value_checked(value, class)?;
                    }
                    (
                        mir::Type::Collection(collection)
                        | mir::Type::NullableCollection(collection),
                        Some(value),
                    ) => {
                        self.drop_collection_value(value, collection)?;
                    }
                    (mir::Type::Mixed | mir::Type::NullableMixed, Some(value)) => {
                        self.drop_mixed_value(value)?;
                    }
                    (
                        mir::Type::SharedReference(_) | mir::Type::NullableSharedReference(_),
                        Some(value),
                    ) => self.drop_shared_value(value, false)?,
                    (
                        mir::Type::WeakReference(_) | mir::Type::NullableWeakReference(_),
                        Some(value),
                    ) => self.drop_shared_value(value, true)?,
                    (ty, Some(value)) if writable_shared_release_symbol(ty).is_some() => self
                        .drop_writable_shared_value(
                            value,
                            writable_shared_release_symbol(ty).expect("matched above"),
                        )?,
                    (mir::Type::PayloadEnum(payload), Some(value)) => {
                        self.drop_payload_enum_at(value, payload, false)?;
                    }
                    (mir::Type::NullablePayloadEnum(payload), Some(value)) => {
                        self.drop_payload_enum_at(value, payload, true)?;
                    }
                    _ => {}
                }
                if let Some(old) = old_function {
                    self.drop_function_carrier(old)?;
                }
                if let Some(old_error) = old_error {
                    self.drop_error_value(old_error)?;
                }
            }
            mir::Statement::AssignStatic { target, value } => {
                let property = static_definition(self.program, *target)?;
                let value = self.lower_rvalue(value)?;
                let address = self.static_address(*target)?;
                let old_error = matches!(property.ty, mir::Type::Error | mir::Type::NullableError)
                    .then(|| {
                        build(self.builder.build_load(
                            error_carrier_type(self.context),
                            address,
                            "static.old.error",
                        ))
                        .map(BasicValueEnum::into_struct_value)
                    })
                    .transpose()?;
                let old_function = matches!(
                    property.ty,
                    mir::Type::Function(_) | mir::Type::NullableFunction(_)
                )
                .then(|| {
                    build(self.builder.build_load(
                        closure_carrier_type(self.context),
                        address,
                        "static.old.function",
                    ))
                    .map(BasicValueEnum::into_struct_value)
                })
                .transpose()?;
                let old = match property.ty {
                    mir::Type::String | mir::Type::Mixed | mir::Type::NullableMixed => Some(
                        build(self.builder.build_load(
                            self.context.ptr_type(AddressSpace::default()),
                            address,
                            "static.old",
                        ))
                        .map(BasicValueEnum::into_pointer_value)?,
                    ),
                    mir::Type::NullableString => {
                        let value = build(self.builder.build_load(
                            llvm_type(self.context, self.target_data, property.ty),
                            address,
                            "static.old",
                        ))?
                        .into_struct_value();
                        Some(self.nullable_parts(value)?.1.into_pointer_value())
                    }
                    _ => None,
                };
                build(self.builder.build_store(address, value))?;
                if let Some(old) = old {
                    if matches!(property.ty, mir::Type::Mixed | mir::Type::NullableMixed) {
                        self.drop_mixed_value(old)?;
                    } else {
                        self.release_string(old)?;
                    }
                }
                if let Some(old_error) = old_error {
                    self.drop_error_value(old_error)?;
                }
                if let Some(old_function) = old_function {
                    self.drop_function_carrier(old_function)?;
                }
            }
            mir::Statement::DropClass { local, .. } => {
                let (mir::Type::Class(class) | mir::Type::NullableClass(class)) =
                    local_in(self.function, *local)?.ty
                else {
                    return Err(malformed_mir(format!(
                        "drop local{} did not target a class local",
                        local.0
                    )));
                };
                let pointer = self.context.ptr_type(AddressSpace::default());
                let slot = local_slot(&self.local_slots, *local)?;
                let value = build(self.builder.build_load(pointer, slot, "class.drop"))?
                    .into_pointer_value();
                build(self.builder.build_store(slot, pointer.const_null()))?;
                self.drop_class_value_checked(value, class)?;
            }
            mir::Statement::DropString { local } => {
                let slot = local_slot(&self.local_slots, *local)?;
                let ty = local_in(self.function, *local)?.ty;
                let value = build(self.builder.build_load(
                    llvm_type(self.context, self.target_data, ty),
                    slot,
                    "string.drop",
                ))?;
                let value = if matches!(ty, mir::Type::NullableString) {
                    self.nullable_parts(value.into_struct_value())?
                        .1
                        .into_pointer_value()
                } else {
                    value.into_pointer_value()
                };
                build(self.builder.build_store(
                    slot,
                    llvm_type(self.context, self.target_data, ty).const_zero(),
                ))?;
                self.release_string(value)?;
            }
            mir::Statement::CollectionAdd {
                collection,
                value,
                index,
                op,
            } => {
                self.lower_collection_add(*collection, value, index.as_ref(), *op)?;
            }
            mir::Statement::CollectionSet {
                collection,
                key,
                value,
            } => self.lower_collection_set(*collection, key, value, false)?,
            mir::Statement::AssignCollectionIndex {
                positional,
                collection,
                index: key,
                value,
            } => self.lower_collection_set(*collection, key, value, *positional)?,
            mir::Statement::CollectionClear {
                collection,
                collection_type,
            } => {
                let pointer = self.context.ptr_type(AddressSpace::default());
                let slot = local_slot(&self.local_slots, *collection)?;
                let value = build(self.builder.build_load(pointer, slot, "collection.clear"))?
                    .into_pointer_value();
                self.clear_collection_value(value, *collection_type)?;
            }
            mir::Statement::DropCollection { local, collection } => {
                let pointer = self.context.ptr_type(AddressSpace::default());
                let slot = local_slot(&self.local_slots, *local)?;
                let value = build(self.builder.build_load(pointer, slot, "collection.drop"))?
                    .into_pointer_value();
                build(self.builder.build_store(slot, pointer.const_null()))?;
                self.drop_collection_value(value, *collection)?;
            }
            mir::Statement::DropMixed { local } => {
                let pointer = self.context.ptr_type(AddressSpace::default());
                let slot = local_slot(&self.local_slots, *local)?;
                let value = build(self.builder.build_load(pointer, slot, "mixed.drop"))?
                    .into_pointer_value();
                build(self.builder.build_store(slot, pointer.const_null()))?;
                self.drop_mixed_value(value)?;
            }
            mir::Statement::EnsureErrorOrigin { error, origin } => {
                let slot = local_slot(&self.local_slots, *error)?;
                let value = build(self.builder.build_load(
                    error_carrier_type(self.context),
                    slot,
                    "error.origin.value",
                ))?
                .into_struct_value();
                let (object, descriptor) = self.error_parts(value)?;
                let descriptor_type = error_descriptor_type(self.context, self.target_data);
                let offset_field = build(self.builder.build_struct_gep(
                    descriptor_type,
                    descriptor,
                    5,
                    "error.origin.offset-field",
                ))?;
                let word = self.context.ptr_sized_int_type(self.target_data, None);
                let offset = build(self.builder.build_load(
                    word,
                    offset_field,
                    "error.origin.offset",
                ))?
                .into_int_value();
                let origin_slot = unsafe {
                    build(self.builder.build_in_bounds_gep(
                        self.context.i8_type(),
                        object,
                        &[offset],
                        "error.origin.slot",
                    ))?
                };
                let pointer = self.context.ptr_type(AddressSpace::default());
                let current = build(self.builder.build_load(
                    pointer,
                    origin_slot,
                    "error.origin.current",
                ))?
                .into_pointer_value();
                let empty = build(self.builder.build_is_null(current, "error.origin.empty"))?;
                let function = current_function(&self.builder)?;
                let write = self
                    .context
                    .append_basic_block(function, "error.origin.write");
                let done = self
                    .context
                    .append_basic_block(function, "error.origin.done");
                build(self.builder.build_conditional_branch(empty, write, done))?;
                self.builder.position_at_end(write);
                build(
                    self.builder
                        .build_store(origin_slot, self.error_origin_address(*origin)?),
                )?;
                build(self.builder.build_unconditional_branch(done))?;
                self.builder.position_at_end(done);
            }
            mir::Statement::ExtractErrorObject { target, error, .. } => {
                let error_slot = local_slot(&self.local_slots, *error)?;
                let value = build(self.builder.build_load(
                    error_carrier_type(self.context),
                    error_slot,
                    "error.extract.value",
                ))?
                .into_struct_value();
                let (object, _) = self.error_parts(value)?;
                build(
                    self.builder
                        .build_store(local_slot(&self.local_slots, *target)?, object),
                )?;
                build(
                    self.builder
                        .build_store(error_slot, error_carrier_type(self.context).const_zero()),
                )?;
            }
            mir::Statement::DropError { local } => {
                let slot = local_slot(&self.local_slots, *local)?;
                let value = build(self.builder.build_load(
                    error_carrier_type(self.context),
                    slot,
                    "error.drop.value",
                ))?
                .into_struct_value();
                build(
                    self.builder
                        .build_store(slot, error_carrier_type(self.context).const_zero()),
                )?;
                self.drop_error_value(value)?;
            }
            mir::Statement::DropSharedReference { local, .. } => {
                let pointer = self.context.ptr_type(AddressSpace::default());
                let slot = local_slot(&self.local_slots, *local)?;
                let value = build(self.builder.build_load(pointer, slot, "shared.drop"))?
                    .into_pointer_value();
                build(self.builder.build_store(slot, pointer.const_null()))?;
                self.drop_shared_value(value, false)?;
            }
            mir::Statement::DropWeakReference { local, .. } => {
                let pointer = self.context.ptr_type(AddressSpace::default());
                let slot = local_slot(&self.local_slots, *local)?;
                let value = build(self.builder.build_load(pointer, slot, "weak.drop"))?
                    .into_pointer_value();
                build(self.builder.build_store(slot, pointer.const_null()))?;
                self.drop_shared_value(value, true)?;
            }
            mir::Statement::DropWritableSharedReference { local, .. } => {
                self.drop_writable_shared_local(*local, WRITABLE_SHARED_RELEASE)?;
            }
            mir::Statement::DropWritableWeakReference { local, .. } => {
                self.drop_writable_shared_local(*local, WRITABLE_SHARED_RELEASE_WEAK)?;
            }
            mir::Statement::DropSharedReferenceAccess {
                local, writable, ..
            } => {
                self.drop_writable_shared_local(
                    *local,
                    if *writable {
                        WRITABLE_SHARED_RELEASE_WRITABLE_ACCESS
                    } else {
                        WRITABLE_SHARED_RELEASE_READONLY_ACCESS
                    },
                )?;
            }
            mir::Statement::DropPayloadEnum {
                local,
                ty,
                nullable,
            } => {
                let address = local_slot(&self.local_slots, *local)?;
                self.drop_payload_enum_at(address, *ty, *nullable)?;
                self.zero_payload_bytes(address, *ty, *nullable)?;
            }
        }
        self.defer_class_temporary_drops = false;
        self.flush_deferred_class_temporary_drops()
    }

    fn lower_terminator(&mut self, terminator: &mir::Terminator) -> Result<(), BackendError> {
        match terminator {
            mir::Terminator::IndirectCall {
                callee,
                function_type,
                invocation_mode,
                args,
                result,
                continuation,
                span,
            } => self.lower_indirect_call(
                callee,
                *function_type,
                *invocation_mode,
                args,
                *result,
                *continuation,
                *span,
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
            } => self.lower_checked_indirect_call(
                callee,
                *function_type,
                *invocation_mode,
                args,
                *result,
                *error,
                *success,
                *failure,
                *span,
            )?,
            mir::Terminator::Return(expression) => {
                debug_assert!(self.deferred_class_temporary_drops.is_empty());
                self.defer_class_temporary_drops = true;
                let value = self.lower_rvalue(expression)?;
                self.defer_class_temporary_drops = false;
                self.flush_deferred_class_temporary_drops()?;
                if let Some(destination) = self
                    .return_address
                    .filter(|_| !self.function.checked_effects.is_empty())
                {
                    if let mir::Type::PayloadEnum(payload)
                    | mir::Type::NullablePayloadEnum(payload) = expression.ty()
                    {
                        self.copy_payload_bytes(
                            destination,
                            value.into_pointer_value(),
                            payload,
                            matches!(expression.ty(), mir::Type::NullablePayloadEnum(_)),
                        )?;
                    } else {
                        build(self.builder.build_store(destination, value))?;
                    }
                    self.sync_writable_closure_captures()?;
                    self.cleanup_mixed_locals()?;
                    self.cleanup_class_locals()?;
                    self.cleanup_string_locals()?;
                    build(
                        self.builder
                            .build_return(Some(&self.context.i8_type().const_zero())),
                    )?;
                    return Ok(());
                }
                self.sync_writable_closure_captures()?;
                self.cleanup_mixed_locals()?;
                self.cleanup_class_locals()?;
                self.cleanup_string_locals()?;
                if let mir::Type::PayloadEnum(payload) | mir::Type::NullablePayloadEnum(payload) =
                    expression.ty()
                {
                    let destination = self.return_address.ok_or_else(|| {
                        malformed_mir("payload enum return has no hidden result address")
                    })?;
                    self.copy_payload_bytes(
                        destination,
                        value.into_pointer_value(),
                        payload,
                        matches!(expression.ty(), mir::Type::NullablePayloadEnum(_)),
                    )?;
                    build(self.builder.build_return(None))?;
                } else {
                    build(self.builder.build_return(Some(&value)))?;
                }
            }
            mir::Terminator::ReturnVoid => {
                self.sync_writable_closure_captures()?;
                self.cleanup_mixed_locals()?;
                self.cleanup_class_locals()?;
                self.cleanup_string_locals()?;
                if self.checked_error_address.is_some() {
                    build(
                        self.builder
                            .build_return(Some(&self.context.i8_type().const_zero())),
                    )?;
                } else {
                    build(self.builder.build_return(None))?;
                }
            }
            mir::Terminator::Panic { message, span } => {
                self.set_active_panic_site(*span)?;
                debug_assert!(self.deferred_class_temporary_drops.is_empty());
                self.defer_class_temporary_drops = true;
                let string = self.lower_string_expression(message)?;
                self.defer_class_temporary_drops = false;
                // Message evaluation may call another Doria function and update
                // the active panic site. The explicit panic itself originates here.
                self.set_active_panic_site(*span)?;
                // Abort-only panic never reaches statement-end destruction.
                self.deferred_class_temporary_drops.clear();
                let pointer = self.context.ptr_type(AddressSpace::default());
                let data = self
                    .call_runtime(
                        STRING_DATA,
                        &[pointer.into()],
                        Some(pointer.into()),
                        &[string.into()],
                    )?
                    .ok_or_else(|| backend_failure("string data produced no result"))?;
                let usize_type = self.context.ptr_sized_int_type(self.target_data, None);
                let length = self
                    .call_runtime(
                        STRING_BYTE_LENGTH,
                        &[pointer.into()],
                        Some(usize_type.into()),
                        &[string.into()],
                    )?
                    .ok_or_else(|| backend_failure("string length produced no result"))?;
                let panic = self.runtime_function(
                    "dr_v2_panic",
                    &[pointer.into(), pointer.into(), usize_type.into()],
                    None,
                );
                let values = [self.current_frame.into(), data.into(), length.into()];
                let _ = build(self.builder.build_call(panic, &values, "panic"))?;
                build(self.builder.build_unreachable())?;
            }
            mir::Terminator::Unreachable => {
                build(self.builder.build_unreachable())?;
            }
            mir::Terminator::Jump(target) => {
                build(
                    self.builder
                        .build_unconditional_branch(block_for(&self.blocks, *target)?),
                )?;
            }
            mir::Terminator::Branch {
                condition,
                then_block,
                else_block,
            } => {
                if mir::bool_class_temporary_capacity(condition) == 0 {
                    return self.lower_condition_to_branch(
                        condition,
                        block_for(&self.blocks, *then_block)?,
                        block_for(&self.blocks, *else_block)?,
                    );
                }
                debug_assert!(self.deferred_class_temporary_drops.is_empty());
                let function = current_function(&self.builder)?;
                let cleanup_then = self
                    .context
                    .append_basic_block(function, "condition.cleanup.then");
                let cleanup_else = self
                    .context
                    .append_basic_block(function, "condition.cleanup.else");
                self.defer_class_temporary_drops = true;
                self.lower_condition_to_branch(condition, cleanup_then, cleanup_else)?;
                self.defer_class_temporary_drops = false;
                let drops = std::mem::take(&mut self.deferred_class_temporary_drops);

                self.builder.position_at_end(cleanup_then);
                self.emit_deferred_class_temporary_drops(&drops)?;
                build(
                    self.builder
                        .build_unconditional_branch(block_for(&self.blocks, *then_block)?),
                )?;

                self.builder.position_at_end(cleanup_else);
                self.emit_deferred_class_temporary_drops(&drops)?;
                build(
                    self.builder
                        .build_unconditional_branch(block_for(&self.blocks, *else_block)?),
                )?;
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
                self.set_active_panic_site(*span)?;
                let callee = *self.functions.get(function.0).ok_or_else(|| {
                    malformed_mir(format!("function{} does not exist", function.0))
                })?;
                let callee_definition = function_in(self.program, *function)?;
                let lowered = self.lower_call_arguments(args)?;
                let mut values = Vec::with_capacity(lowered.values.len() + 3);
                values.push(self.current_frame.into());
                if let Some(result) = result {
                    values.push(local_slot(&self.local_slots, *result)?.into());
                }
                values.push(local_slot(&self.local_slots, *error)?.into());
                if let Some(home) = self.direct_call_borrow_home(callee_definition, args, None)? {
                    values.push(home.into());
                }
                values.extend(lowered.values.iter().copied());
                let call = build(self.builder.build_call(callee, &values, "checked.call"))?;
                apply_call_abi_attributes(self.context, call, callee_definition)?;
                let status = call
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| malformed_mir("checked call produced no status"))?
                    .into_int_value();
                self.cleanup_call_arguments(*function, args, false, &lowered)?;

                let current = current_function(&self.builder)?;
                let failure_status = self
                    .context
                    .append_basic_block(current, "checked.call.failure-status");
                let invalid_status = self
                    .context
                    .append_basic_block(current, "checked.call.invalid-status");
                let succeeded = build(self.builder.build_int_compare(
                    IntPredicate::EQ,
                    status,
                    self.context.i8_type().const_zero(),
                    "checked.call.succeeded",
                ))?;
                build(self.builder.build_conditional_branch(
                    succeeded,
                    block_for(&self.blocks, *success)?,
                    failure_status,
                ))?;
                self.builder.position_at_end(failure_status);
                let failed = build(self.builder.build_int_compare(
                    IntPredicate::EQ,
                    status,
                    self.context.i8_type().const_int(1, false),
                    "checked.call.failed",
                ))?;
                build(self.builder.build_conditional_branch(
                    failed,
                    block_for(&self.blocks, *failure)?,
                    invalid_status,
                ))?;
                self.builder.position_at_end(invalid_status);
                build(self.builder.build_unreachable())?;
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
                self.set_active_panic_site(*span)?;
                let (object, lowered) = self.lower_class_allocation(*class, properties, args)?;
                let callee = *self.functions.get(constructor.0).ok_or_else(|| {
                    malformed_mir(format!("function{} does not exist", constructor.0))
                })?;
                let callee_definition = function_in(self.program, *constructor)?;
                let mut values = Vec::with_capacity(lowered.values.len() + 3);
                values.push(self.current_frame.into());
                values.push(local_slot(&self.local_slots, *error)?.into());
                values.push(object.into());
                values.extend(lowered.values.iter().copied());
                let call = build(
                    self.builder
                        .build_call(callee, &values, "checked.construct"),
                )?;
                apply_call_abi_attributes(self.context, call, callee_definition)?;
                let status = call
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| malformed_mir("checked constructor produced no status"))?
                    .into_int_value();
                self.cleanup_constructor_arguments(*constructor, properties, args, &lowered)?;

                let current = current_function(&self.builder)?;
                let success_store = self
                    .context
                    .append_basic_block(current, "checked.construct.success-store");
                let failure_status = self
                    .context
                    .append_basic_block(current, "checked.construct.failure-status");
                let failed_cleanup = self
                    .context
                    .append_basic_block(current, "checked.construct.failed-cleanup");
                let invalid_status = self
                    .context
                    .append_basic_block(current, "checked.construct.invalid-status");
                let succeeded = build(self.builder.build_int_compare(
                    IntPredicate::EQ,
                    status,
                    self.context.i8_type().const_zero(),
                    "checked.construct.succeeded",
                ))?;
                build(self.builder.build_conditional_branch(
                    succeeded,
                    success_store,
                    failure_status,
                ))?;

                self.builder.position_at_end(success_store);
                build(
                    self.builder
                        .build_store(local_slot(&self.local_slots, *result)?, object),
                )?;
                build(
                    self.builder
                        .build_unconditional_branch(block_for(&self.blocks, *success)?),
                )?;

                self.builder.position_at_end(failure_status);
                let failed = build(self.builder.build_int_compare(
                    IntPredicate::EQ,
                    status,
                    self.context.i8_type().const_int(1, false),
                    "checked.construct.failed",
                ))?;
                build(self.builder.build_conditional_branch(
                    failed,
                    failed_cleanup,
                    invalid_status,
                ))?;

                self.builder.position_at_end(failed_cleanup);
                self.drop_failed_class_value(object, *class)?;
                build(
                    self.builder
                        .build_unconditional_branch(block_for(&self.blocks, *failure)?),
                )?;

                self.builder.position_at_end(invalid_status);
                build(self.builder.build_unreachable())?;
            }
            mir::Terminator::CheckedIo {
                operation,
                result,
                error,
                success,
                failure,
                span,
            } => self.lower_checked_io_terminator(
                operation,
                *result,
                *error,
                block_for(&self.blocks, *success)?,
                block_for(&self.blocks, *failure)?,
                *span,
            )?,
            mir::Terminator::ErrorSwitch {
                error,
                cases,
                catch_all,
                fallback,
            } => {
                let slot = local_slot(&self.local_slots, *error)?;
                let value = build(self.builder.build_load(
                    error_carrier_type(self.context),
                    slot,
                    "error.switch.value",
                ))?
                .into_struct_value();
                let (_, descriptor) = self.error_parts(value)?;
                let current = current_function(&self.builder)?;
                for (expected, target) in cases {
                    let next = self
                        .context
                        .append_basic_block(current, "error.switch.next");
                    let matches = build(self.builder.build_int_compare(
                        IntPredicate::EQ,
                        descriptor,
                        self.error_descriptor_address(*expected)?,
                        "error.switch.matches",
                    ))?;
                    build(self.builder.build_conditional_branch(
                        matches,
                        block_for(&self.blocks, *target)?,
                        next,
                    ))?;
                    self.builder.position_at_end(next);
                }
                build(self.builder.build_unconditional_branch(block_for(
                    &self.blocks,
                    catch_all.unwrap_or(*fallback),
                )?))?;
            }
            mir::Terminator::PropagateError { error } => {
                let destination = self.checked_error_address.ok_or_else(|| {
                    malformed_mir("checked propagation has no caller Error out slot")
                })?;
                let slot = local_slot(&self.local_slots, *error)?;
                let value = build(self.builder.build_load(
                    error_carrier_type(self.context),
                    slot,
                    "error.propagate.value",
                ))?;
                build(self.builder.build_store(destination, value))?;
                build(
                    self.builder
                        .build_store(slot, error_carrier_type(self.context).const_zero()),
                )?;
                self.sync_writable_closure_captures()?;
                self.cleanup_mixed_locals()?;
                self.cleanup_class_locals()?;
                self.cleanup_string_locals()?;
                build(
                    self.builder
                        .build_return(Some(&self.context.i8_type().const_int(1, false))),
                )?;
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn lower_checked_io_terminator(
        &mut self,
        operation: &mir::CheckedIoOperation,
        result: Option<mir::LocalId>,
        error: mir::LocalId,
        success: BasicBlock<'ctx>,
        failure: BasicBlock<'ctx>,
        span: crate::source::Span,
    ) -> Result<(), BackendError> {
        self.set_active_panic_site(span)?;
        debug_assert!(self.deferred_class_temporary_drops.is_empty());
        self.defer_class_temporary_drops = true;
        let pointer = self.context.ptr_type(AddressSpace::default());
        let word = self.context.ptr_sized_int_type(self.target_data, None);
        let mut owned_strings = Vec::new();

        let (operation_code, path_or_prompt, contents) = match operation {
            mir::CheckedIoOperation::ReadLine { prompt } => {
                let prompt = self.lower_string_expression(prompt)?;
                owned_strings.push(prompt);
                (CHECKED_IO_READ_LINE, prompt, None)
            }
            mir::CheckedIoOperation::ReadFile { path, bytes } => {
                let path = self.lower_string_expression(path)?;
                owned_strings.push(path);
                (
                    if *bytes {
                        CHECKED_IO_READ_FILE_BYTES
                    } else {
                        CHECKED_IO_READ_FILE_TEXT
                    },
                    path,
                    None,
                )
            }
            mir::CheckedIoOperation::ReadStdinBytes => {
                (CHECKED_IO_READ_STDIN_BYTES, pointer.const_null(), None)
            }
            mir::CheckedIoOperation::WriteFile {
                path,
                contents,
                append,
            } => {
                let path = self.lower_string_expression(path)?;
                owned_strings.push(path);
                let contents = self.lower_checked_io_contents(contents, &mut owned_strings)?;
                (
                    if *append {
                        CHECKED_IO_APPEND_FILE
                    } else {
                        CHECKED_IO_WRITE_FILE
                    },
                    path,
                    Some(contents),
                )
            }
            mir::CheckedIoOperation::WriteStream { contents, stderr } => {
                let contents = self.lower_checked_io_contents(contents, &mut owned_strings)?;
                (
                    if *stderr {
                        CHECKED_IO_WRITE_STDERR
                    } else {
                        CHECKED_IO_WRITE_STDOUT
                    },
                    pointer.const_null(),
                    Some(contents),
                )
            }
        };

        let (contents_data, contents_length) = if let Some((contents, bytes)) = contents {
            let data = self
                .call_runtime(
                    if bytes { BYTES_DATA } else { STRING_DATA },
                    &[pointer.into()],
                    Some(pointer.into()),
                    &[contents.into()],
                )?
                .ok_or_else(|| backend_failure("checked I/O contents data produced no result"))?
                .into_pointer_value();
            let length = self
                .call_runtime(
                    if bytes {
                        BYTES_LENGTH
                    } else {
                        STRING_BYTE_LENGTH
                    },
                    &[pointer.into()],
                    Some(word.into()),
                    &[contents.into()],
                )?
                .ok_or_else(|| backend_failure("checked I/O contents length produced no result"))?
                .into_int_value();
            (data, length)
        } else {
            (pointer.const_null(), word.const_zero())
        };

        let result_slot = self.entry_alloca(pointer, "checked.io.result")?;
        let message_slot = self.entry_alloca(pointer, "checked.io.message")?;
        let path_slot = self.entry_alloca(pointer, "checked.io.path")?;
        let system_code_slot =
            self.entry_alloca(self.context.i64_type(), "checked.io.system-code")?;
        let valid_count_slot = self.entry_alloca(word, "checked.io.valid-count")?;
        let invalid_count_slot = self.entry_alloca(word, "checked.io.invalid-count")?;
        let meta_slot = self.entry_alloca(self.context.i64_type(), "checked.io.meta")?;
        let status = self
            .call_runtime(
                CHECKED_IO,
                &[
                    pointer.into(),
                    self.context.i8_type().into(),
                    pointer.into(),
                    pointer.into(),
                    word.into(),
                    pointer.into(),
                    pointer.into(),
                    pointer.into(),
                    pointer.into(),
                    pointer.into(),
                    pointer.into(),
                    pointer.into(),
                ],
                Some(self.context.i8_type().into()),
                &[
                    self.current_frame.into(),
                    self.context
                        .i8_type()
                        .const_int(u64::from(operation_code), false)
                        .into(),
                    path_or_prompt.into(),
                    contents_data.into(),
                    contents_length.into(),
                    result_slot.into(),
                    message_slot.into(),
                    path_slot.into(),
                    system_code_slot.into(),
                    valid_count_slot.into(),
                    invalid_count_slot.into(),
                    meta_slot.into(),
                ],
            )?
            .ok_or_else(|| backend_failure("checked I/O produced no status"))?
            .into_int_value();
        for string in owned_strings {
            self.release_string(string)?;
        }
        self.defer_class_temporary_drops = false;
        self.flush_deferred_class_temporary_drops()?;

        let current = current_function(&self.builder)?;
        let success_store = self
            .context
            .append_basic_block(current, "checked.io.success-store");
        let failure_status = self
            .context
            .append_basic_block(current, "checked.io.failure-status");
        let build_error = self
            .context
            .append_basic_block(current, "checked.io.build-error");
        let invalid_status = self
            .context
            .append_basic_block(current, "checked.io.invalid-status");
        let succeeded = build(self.builder.build_int_compare(
            IntPredicate::EQ,
            status,
            self.context.i8_type().const_zero(),
            "checked.io.succeeded",
        ))?;
        build(
            self.builder
                .build_conditional_branch(succeeded, success_store, failure_status),
        )?;

        self.builder.position_at_end(success_store);
        if let Some(result) = result {
            let raw = build(
                self.builder
                    .build_load(pointer, result_slot, "checked.io.value"),
            )?
            .into_pointer_value();
            let local = local_in(self.function, result)?;
            let value: BasicValueEnum<'ctx> = match local.ty {
                mir::Type::NullableString => {
                    let present = build(
                        self.builder
                            .build_is_not_null(raw, "checked.io.value.present"),
                    )?;
                    let present = build(self.builder.build_int_z_extend(
                        present,
                        word,
                        "checked.io.value.presence",
                    ))?;
                    self.nullable_value(present, raw.into())?.into()
                }
                mir::Type::String | mir::Type::Collection(_) => raw.into(),
                _ => {
                    return Err(malformed_mir(
                        "checked I/O has an unsupported LLVM result type",
                    ))
                }
            };
            self.store_value_at_address(local_slot(&self.local_slots, result)?, value, local.ty)?;
        }
        build(self.builder.build_unconditional_branch(success))?;

        self.builder.position_at_end(failure_status);
        let failed = build(self.builder.build_int_compare(
            IntPredicate::EQ,
            status,
            self.context.i8_type().const_int(1, false),
            "checked.io.failed",
        ))?;
        build(
            self.builder
                .build_conditional_branch(failed, build_error, invalid_status),
        )?;

        self.builder.position_at_end(build_error);
        self.lower_checked_io_error(
            error,
            message_slot,
            path_slot,
            system_code_slot,
            valid_count_slot,
            invalid_count_slot,
            meta_slot,
            failure,
        )?;

        self.builder.position_at_end(invalid_status);
        build(self.builder.build_unreachable())?;
        Ok(())
    }

    fn lower_checked_io_contents(
        &mut self,
        contents: &mir::IoContents,
        owned_strings: &mut Vec<PointerValue<'ctx>>,
    ) -> Result<(PointerValue<'ctx>, bool), BackendError> {
        match contents {
            mir::IoContents::String(value) => {
                let value = self.lower_string_expression(value)?;
                owned_strings.push(value);
                Ok((value, false))
            }
            mir::IoContents::Format(value) => {
                let value = self.lower_format_expression(value)?;
                owned_strings.push(value);
                Ok((value, false))
            }
            mir::IoContents::Bytes(local) => Ok((self.collection_pointer(*local)?, true)),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn lower_checked_io_error(
        &mut self,
        error: mir::LocalId,
        message_slot: PointerValue<'ctx>,
        path_slot: PointerValue<'ctx>,
        system_code_slot: PointerValue<'ctx>,
        valid_count_slot: PointerValue<'ctx>,
        invalid_count_slot: PointerValue<'ctx>,
        meta_slot: PointerValue<'ctx>,
        failure: BasicBlock<'ctx>,
    ) -> Result<(), BackendError> {
        let meta = build(self.builder.build_load(
            self.context.i64_type(),
            meta_slot,
            "checked.io.meta",
        ))?
        .into_int_value();
        let kind = self.checked_io_meta_byte(meta, CHECKED_IO_META_KIND_SHIFT)?;
        let current = current_function(&self.builder)?;
        let invalid_utf8 = self
            .context
            .append_basic_block(current, "checked.io.invalid-utf8");
        let ordinary = self
            .context
            .append_basic_block(current, "checked.io.ordinary");
        let malformed = self
            .context
            .append_basic_block(current, "checked.io.malformed-kind");
        let is_invalid_utf8 = build(
            self.builder.build_int_compare(
                IntPredicate::EQ,
                kind,
                self.context
                    .i8_type()
                    .const_int(u64::from(CHECKED_IO_ERROR_INVALID_UTF8), false),
                "checked.io.is-invalid-utf8",
            ),
        )?;
        build(
            self.builder
                .build_conditional_branch(is_invalid_utf8, invalid_utf8, ordinary),
        )?;

        self.builder.position_at_end(ordinary);
        let is_io = build(
            self.builder.build_int_compare(
                IntPredicate::EQ,
                kind,
                self.context
                    .i8_type()
                    .const_int(u64::from(CHECKED_IO_ERROR_IO), false),
                "checked.io.is-io",
            ),
        )?;
        let valid_io = self
            .context
            .append_basic_block(current, "checked.io.valid-io");
        build(
            self.builder
                .build_conditional_branch(is_io, valid_io, malformed),
        )?;
        self.builder.position_at_end(valid_io);
        self.lower_checked_io_error_object(
            crate::compiler_known_io::IO_ERROR,
            error,
            message_slot,
            path_slot,
            system_code_slot,
            valid_count_slot,
            invalid_count_slot,
            meta,
        )?;
        build(self.builder.build_unconditional_branch(failure))?;

        self.builder.position_at_end(invalid_utf8);
        self.lower_checked_io_error_object(
            crate::compiler_known_io::INVALID_UTF8_ERROR,
            error,
            message_slot,
            path_slot,
            system_code_slot,
            valid_count_slot,
            invalid_count_slot,
            meta,
        )?;
        build(self.builder.build_unconditional_branch(failure))?;

        self.builder.position_at_end(malformed);
        build(self.builder.build_unreachable())?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn lower_checked_io_error_object(
        &mut self,
        type_name: &str,
        error: mir::LocalId,
        message_slot: PointerValue<'ctx>,
        path_slot: PointerValue<'ctx>,
        system_code_slot: PointerValue<'ctx>,
        valid_count_slot: PointerValue<'ctx>,
        invalid_count_slot: PointerValue<'ctx>,
        meta: IntValue<'ctx>,
    ) -> Result<(), BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let word = self.context.ptr_sized_int_type(self.target_data, None);
        let class = self
            .program
            .classes
            .iter()
            .find(|class| class.name == type_name)
            .ok_or_else(|| malformed_mir(format!("compiler-known class `{type_name}` is missing")))?
            .clone();
        let descriptor = self
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
        let object = self
            .call_runtime(
                CLASS_ALLOCATE,
                &[pointer.into(), word.into(), word.into()],
                Some(pointer.into()),
                &[
                    self.current_frame.into(),
                    word.const_int(u64::from(class.layout.size), false).into(),
                    word.const_int(u64::from(class.layout.align), false).into(),
                ],
            )?
            .ok_or_else(|| backend_failure("checked I/O Error allocation produced no result"))?
            .into_pointer_value();
        let message = build(
            self.builder
                .build_load(pointer, message_slot, "checked.io.message"),
        )?;
        self.store_known_property(object, &class, "message", message)?;

        if type_name == crate::compiler_known_io::IO_ERROR {
            let operation = self.checked_io_meta_byte(meta, CHECKED_IO_META_OPERATION_SHIFT)?;
            let operation = build(self.builder.build_int_z_extend(
                operation,
                self.context.i32_type(),
                "checked.io.operation",
            ))?;
            self.store_known_property(object, &class, "operation", operation.into())?;
            let target = self.checked_io_meta_byte(meta, CHECKED_IO_META_TARGET_SHIFT)?;
            let path = build(
                self.builder
                    .build_load(pointer, path_slot, "checked.io.path"),
            )?
            .into_pointer_value();
            self.store_checked_io_payload_property(
                object,
                &class,
                "target",
                crate::compiler_known_io::IO_TARGET,
                target,
                path,
            )?;
            let reason = self.checked_io_meta_byte(meta, CHECKED_IO_META_REASON_SHIFT)?;
            let reason = build(self.builder.build_int_z_extend(
                reason,
                self.context.i32_type(),
                "checked.io.reason",
            ))?;
            self.store_known_property(object, &class, "reason", reason.into())?;
            let present = self.checked_io_meta_bit(meta, CHECKED_IO_META_HAS_SYSTEM_CODE_SHIFT)?;
            let system_code = build(self.builder.build_load(
                self.context.i64_type(),
                system_code_slot,
                "checked.io.system-code",
            ))?;
            let nullable = self.nullable_value(present, system_code)?;
            self.store_known_property(object, &class, "systemCode", nullable.into())?;
        } else {
            let source = self.checked_io_meta_byte(meta, CHECKED_IO_META_TARGET_SHIFT)?;
            let path = build(
                self.builder
                    .build_load(pointer, path_slot, "checked.io.path"),
            )?
            .into_pointer_value();
            self.store_checked_io_payload_property(
                object,
                &class,
                "source",
                crate::compiler_known_io::UTF8_INPUT_SOURCE,
                source,
                path,
            )?;
            let valid = build(self.builder.build_load(
                word,
                valid_count_slot,
                "checked.io.valid-count",
            ))?
            .into_int_value();
            let valid = if word.get_bit_width() == 64 {
                valid
            } else {
                build(self.builder.build_int_z_extend(
                    valid,
                    self.context.i64_type(),
                    "checked.io.valid-count.i64",
                ))?
            };
            self.store_known_property(object, &class, "validByteCount", valid.into())?;
            let present =
                self.checked_io_meta_bit(meta, CHECKED_IO_META_HAS_INVALID_COUNT_SHIFT)?;
            let invalid = build(self.builder.build_load(
                word,
                invalid_count_slot,
                "checked.io.invalid-count",
            ))?
            .into_int_value();
            let invalid = if word.get_bit_width() == 64 {
                invalid
            } else {
                build(self.builder.build_int_z_extend(
                    invalid,
                    self.context.i64_type(),
                    "checked.io.invalid-count.i64",
                ))?
            };
            let nullable = self.nullable_value(present, invalid.into())?;
            self.store_known_property(object, &class, "invalidByteCount", nullable.into())?;
        }

        let carrier = self.error_value(object, self.error_descriptor_address(descriptor)?)?;
        build(
            self.builder
                .build_store(local_slot(&self.local_slots, error)?, carrier),
        )?;
        Ok(())
    }

    fn store_known_property(
        &self,
        object: PointerValue<'ctx>,
        class: &mir::Class,
        name: &str,
        value: BasicValueEnum<'ctx>,
    ) -> Result<(), BackendError> {
        let property = class
            .properties
            .iter()
            .find(|property| property.name == name)
            .ok_or_else(|| malformed_mir(format!("class `{}` has no `${name}`", class.name)))?;
        self.store_value_at_address(
            self.lower_property_address_from_value(object, property.id)?,
            value,
            property.ty,
        )
    }

    fn store_checked_io_payload_property(
        &self,
        object: PointerValue<'ctx>,
        class: &mir::Class,
        property_name: &str,
        enum_name: &str,
        tag: IntValue<'ctx>,
        path: PointerValue<'ctx>,
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
        let definition = self
            .program
            .enums
            .iter()
            .find(|definition| definition.name == enum_name)
            .ok_or_else(|| {
                malformed_mir(format!("compiler-known enum `{enum_name}` is missing"))
            })?;
        if definition.id != payload.id {
            return Err(malformed_mir(
                "compiler-known payload enum identity changed",
            ));
        }
        let address = self.lower_property_address_from_value(object, property.id)?;
        self.zero_payload_bytes(address, payload, false)?;
        let tag_type = match definition.layout.tag_width {
            1 => self.context.i8_type(),
            2 => self.context.i16_type(),
            4 => self.context.i32_type(),
            _ => return Err(malformed_mir("payload enum tag has unsupported width")),
        };
        let tag = if tag.get_type() == tag_type {
            tag
        } else {
            build(
                self.builder
                    .build_int_z_extend(tag, tag_type, "checked.io.payload.tag"),
            )?
        };
        let tag_address = self.byte_offset(
            address,
            definition.layout.tag_offset,
            "checked.io.payload.tag-address",
        )?;
        build(self.builder.build_store(tag_address, tag))?;
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
        build(self.builder.build_store(
            self.byte_offset(address, field.offset, "checked.io.payload.path")?,
            path,
        ))?;
        Ok(())
    }

    fn checked_io_meta_byte(
        &self,
        meta: IntValue<'ctx>,
        shift: u32,
    ) -> Result<IntValue<'ctx>, BackendError> {
        let shifted = build(self.builder.build_right_shift(
            meta,
            self.context.i64_type().const_int(u64::from(shift), false),
            false,
            "checked.io.meta.shift",
        ))?;
        let masked = build(self.builder.build_and(
            shifted,
            self.context.i64_type().const_int(0xff, false),
            "checked.io.meta.byte",
        ))?;
        build(
            self.builder
                .build_int_truncate(masked, self.context.i8_type(), "checked.io.meta.i8"),
        )
    }

    fn checked_io_meta_bit(
        &self,
        meta: IntValue<'ctx>,
        shift: u32,
    ) -> Result<IntValue<'ctx>, BackendError> {
        let byte = self.checked_io_meta_byte(meta, shift)?;
        let bit = build(self.builder.build_and(
            byte,
            self.context.i8_type().const_int(1, false),
            "checked.io.meta.bit",
        ))?;
        build(self.builder.build_int_z_extend(
            bit,
            self.context.ptr_sized_int_type(self.target_data, None),
            "checked.io.meta.presence",
        ))
    }

    fn lower_value_expression(
        &mut self,
        expression: &mir::ValueExpression,
    ) -> Result<BasicValueEnum<'ctx>, BackendError> {
        match expression {
            mir::ValueExpression::Integer(value) => {
                Ok(self.lower_integer_expression(value)?.into())
            }
            mir::ValueExpression::Float(value) => Ok(self.lower_float_expression(value)?.into()),
            mir::ValueExpression::Bool(value) => Ok(self.lower_condition_value(value)?.into()),
            mir::ValueExpression::Enum(value) => Ok(self.lower_enum_expression(value)?.into()),
        }
    }

    fn lower_enum_expression(
        &mut self,
        expression: &mir::EnumExpression,
    ) -> Result<IntValue<'ctx>, BackendError> {
        match expression {
            mir::EnumExpression::Case(value) => Ok(self
                .context
                .i32_type()
                .const_int(value.case_id.index as u64, false)),
            mir::EnumExpression::Use { enum_id, operand } => {
                self.lower_enum_operand(*enum_id, operand)
            }
            mir::EnumExpression::Call { function, args, .. } => Ok(self
                .lower_call(*function, args, true)?
                .ok_or_else(|| malformed_mir("enum call produced no result"))?
                .into_int_value()),
            mir::EnumExpression::Coalesce { left, right, .. } => {
                let left = self.lower_nullable_scalar_expression(left)?;
                Ok(self
                    .lower_coalesce_payload(left, |lowerer| {
                        Ok(lowerer.lower_enum_expression(right)?.into())
                    })?
                    .into_int_value())
            }
        }
    }

    fn lower_payload_enum_expression(
        &mut self,
        expression: &mir::PayloadEnumExpression,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let ty = expression.ty();
        match expression {
            mir::PayloadEnumExpression::Construct { case, fields, .. } => {
                let definition = enum_definition(self.program, ty.id)?.clone();
                let case_definition = definition
                    .cases
                    .get(case.index)
                    .filter(|candidate| candidate.id == *case)
                    .ok_or_else(|| {
                        malformed_mir("payload enum construction case does not exist")
                    })?;
                let case_layout = definition
                    .layout
                    .cases
                    .get(case.index)
                    .filter(|candidate| candidate.case_id == *case)
                    .ok_or_else(|| malformed_mir("payload enum case layout is missing"))?;
                let address = self.entry_payload_alloca(ty, false, "payload.enum.construct")?;
                self.zero_payload_bytes(address, ty, false)?;
                self.store_payload_enum_tag(
                    address,
                    definition.layout.tag_width,
                    case_definition.tag,
                )?;
                for ((field, field_definition), field_layout) in fields
                    .iter()
                    .zip(&case_definition.payload)
                    .zip(&case_layout.fields)
                {
                    let value = self.lower_rvalue(field)?;
                    let field_address =
                        self.byte_offset(address, field_layout.offset, "payload.enum.field")?;
                    self.store_value_at_address(field_address, value, field_definition.ty)?;
                }
                Ok(address)
            }
            mir::PayloadEnumExpression::Use { place, mode, .. } => {
                self.lower_payload_enum_place(place, ty, false, *mode)
            }
            mir::PayloadEnumExpression::Call { function, args, .. } => Ok(self
                .lower_call(*function, args, true)?
                .ok_or_else(|| malformed_mir("payload enum call returned void"))?
                .into_pointer_value()),
            mir::PayloadEnumExpression::Coalesce {
                left, right, mode, ..
            } => {
                let left = self.lower_nullable_payload_enum_expression(left)?;
                let present = build(self.builder.build_load(
                    self.context.i8_type(),
                    left,
                    "payload.enum.coalesce.present",
                ))?
                .into_int_value();
                let function = current_function(&self.builder)?;
                let use_left = self
                    .context
                    .append_basic_block(function, "payload.coalesce.left");
                let use_right = self
                    .context
                    .append_basic_block(function, "payload.coalesce.right");
                let done = self
                    .context
                    .append_basic_block(function, "payload.coalesce.done");
                let is_present = build(self.builder.build_int_compare(
                    IntPredicate::NE,
                    present,
                    self.context.i8_type().const_zero(),
                    "payload.coalesce.is-present",
                ))?;
                build(
                    self.builder
                        .build_conditional_branch(is_present, use_left, use_right),
                )?;
                self.builder.position_at_end(use_left);
                let left_payload =
                    self.byte_offset(left, ty.nullable_payload_offset, "payload.coalesce.value")?;
                build(self.builder.build_unconditional_branch(done))?;
                let left_end = self.builder.get_insert_block().expect("payload left block");
                self.builder.position_at_end(use_right);
                let right = self.lower_payload_enum_expression(right)?;
                build(self.builder.build_unconditional_branch(done))?;
                let right_end = self
                    .builder
                    .get_insert_block()
                    .expect("payload right block");
                self.builder.position_at_end(done);
                let pointer = self.context.ptr_type(AddressSpace::default());
                let phi = build(self.builder.build_phi(pointer, "payload.coalesce.result"))?;
                phi.add_incoming(&[(&left_payload, left_end), (&right, right_end)]);
                let result = phi.as_basic_value().into_pointer_value();
                if matches!(mode, mir::PayloadEnumUseMode::Borrow) {
                    Ok(result)
                } else {
                    let owned = self.entry_payload_alloca(ty, false, "payload.coalesce.owned")?;
                    self.copy_payload_bytes(owned, result, ty, false)?;
                    if matches!(mode, mir::PayloadEnumUseMode::Copy) {
                        self.retain_payload_enum_at(owned, ty, false)?;
                    }
                    Ok(owned)
                }
            }
        }
    }

    fn lower_nullable_payload_enum_expression(
        &mut self,
        expression: &mir::NullablePayloadEnumExpression,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let ty = expression.ty();
        match expression {
            mir::NullablePayloadEnumExpression::Null(_) => {
                let address = self.entry_payload_alloca(ty, true, "nullable.payload.none")?;
                self.zero_payload_bytes(address, ty, true)?;
                Ok(address)
            }
            mir::NullablePayloadEnumExpression::Value(value) => {
                let payload = self.lower_payload_enum_expression(value)?;
                let address = self.entry_payload_alloca(ty, true, "nullable.payload.some")?;
                self.zero_payload_bytes(address, ty, true)?;
                build(
                    self.builder
                        .build_store(address, self.context.i8_type().const_int(1, false)),
                )?;
                let destination = self.byte_offset(
                    address,
                    ty.nullable_payload_offset,
                    "nullable.payload.value",
                )?;
                let size = self
                    .context
                    .ptr_sized_int_type(self.target_data, None)
                    .const_int(u64::from(ty.size), false);
                build(
                    self.builder
                        .build_memcpy(destination, ty.align, payload, ty.align, size),
                )?;
                Ok(address)
            }
            mir::NullablePayloadEnumExpression::Use { place, mode, .. } => {
                self.lower_payload_enum_place(place, ty, true, *mode)
            }
            mir::NullablePayloadEnumExpression::Call { function, args, .. } => Ok(self
                .lower_call(*function, args, true)?
                .ok_or_else(|| malformed_mir("nullable payload enum call returned void"))?
                .into_pointer_value()),
            mir::NullablePayloadEnumExpression::CollectionGet {
                collection,
                key,
                access,
                stored_nullable,
                mode,
                ..
            } => self.lower_nullable_payload_enum_collection_get(
                ty,
                *collection,
                key,
                *access,
                *stored_nullable,
                *mode,
            ),
            mir::NullablePayloadEnumExpression::Coalesce {
                left, right, mode, ..
            } => {
                let left = self.lower_nullable_payload_enum_expression(left)?;
                let present = build(self.builder.build_load(
                    self.context.i8_type(),
                    left,
                    "nullable.payload.coalesce.present",
                ))?
                .into_int_value();
                let function = current_function(&self.builder)?;
                let use_left = self
                    .context
                    .append_basic_block(function, "nullable.payload.coalesce.left");
                let use_right = self
                    .context
                    .append_basic_block(function, "nullable.payload.coalesce.right");
                let done = self
                    .context
                    .append_basic_block(function, "nullable.payload.coalesce.done");
                let is_present = build(self.builder.build_int_compare(
                    IntPredicate::NE,
                    present,
                    self.context.i8_type().const_zero(),
                    "nullable.payload.coalesce.is-present",
                ))?;
                build(
                    self.builder
                        .build_conditional_branch(is_present, use_left, use_right),
                )?;
                self.builder.position_at_end(use_left);
                build(self.builder.build_unconditional_branch(done))?;
                let left_end = self
                    .builder
                    .get_insert_block()
                    .expect("nullable left block");
                self.builder.position_at_end(use_right);
                let right = self.lower_nullable_payload_enum_expression(right)?;
                build(self.builder.build_unconditional_branch(done))?;
                let right_end = self
                    .builder
                    .get_insert_block()
                    .expect("nullable right block");
                self.builder.position_at_end(done);
                let pointer = self.context.ptr_type(AddressSpace::default());
                let phi = build(
                    self.builder
                        .build_phi(pointer, "nullable.payload.coalesce.result"),
                )?;
                phi.add_incoming(&[(&left, left_end), (&right, right_end)]);
                let result = phi.as_basic_value().into_pointer_value();
                if matches!(mode, mir::PayloadEnumUseMode::Borrow) {
                    Ok(result)
                } else {
                    let owned = self.entry_payload_alloca(ty, true, "nullable.payload.owned")?;
                    self.copy_payload_bytes(owned, result, ty, true)?;
                    if matches!(mode, mir::PayloadEnumUseMode::Copy) {
                        self.retain_payload_enum_at(owned, ty, true)?;
                    }
                    Ok(owned)
                }
            }
        }
    }

    fn lower_nullable_payload_enum_collection_get(
        &mut self,
        ty: mir::PayloadEnumType,
        collection: mir::LocalId,
        key: &mir::Rvalue,
        access: mir::NullableCollectionAccess,
        stored_nullable: bool,
        mode: mir::PayloadEnumUseMode,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let local = local_in(self.function, collection)?;
        let mir::Type::Collection(collection_type) = local.ty else {
            return Err(malformed_mir(
                "payload enum nullable access uses a non-collection local",
            ));
        };
        let definition = self.collection_definition(collection_type)?.clone();
        let key_type = match access {
            mir::NullableCollectionAccess::Get
            | mir::NullableCollectionAccess::Index
            | mir::NullableCollectionAccess::Remove => definition
                .key
                .ok_or_else(|| malformed_mir("dictionary access has no key type"))?,
            _ => mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64)),
        };
        let collection = self.collection_pointer(collection)?;
        let key_value = self.lower_rvalue(key)?;
        let key_word = self.value_to_collection_word(key_value, key_type)?;
        let raw = self.entry_payload_alloca(ty, stored_nullable, "aggregate.nullable.raw")?;
        self.zero_payload_bytes(raw, ty, stored_nullable)?;
        let found = self.entry_alloca(self.context.i8_type(), "aggregate.nullable.found")?;
        let removed_key = self.entry_alloca(self.context.i64_type(), "aggregate.removed.key")?;
        let byte = self.context.i8_type();
        let _ = self.call_runtime(
            COLLECTION_AGGREGATE_NULLABLE_ACCESS_INTO,
            &[
                pointer.into(),
                self.context.i64_type().into(),
                byte.into(),
                byte.into(),
                byte.into(),
                pointer.into(),
                pointer.into(),
                pointer.into(),
            ],
            None,
            &[
                collection.into(),
                key_word.into(),
                self.collection_compare_kind(key_type)?.into(),
                byte.const_int(
                    u64::from(nullable_collection_access_code(access).ok_or_else(|| {
                        malformed_mir("aggregate nullable index must use the direct index path")
                    })?),
                    false,
                )
                .into(),
                byte.const_int(u64::from(stored_nullable), false).into(),
                found.into(),
                removed_key.into(),
                raw.into(),
            ],
        )?;
        if key_type == mir::Type::String {
            self.release_string(key_value.into_pointer_value())?;
            if access == mir::NullableCollectionAccess::Remove {
                let removed_key = build(self.builder.build_load(
                    self.context.i64_type(),
                    removed_key,
                    "aggregate.removed.key.value",
                ))?
                .into_int_value();
                self.release_string(
                    self.collection_word_to_value(removed_key, mir::Type::String)?
                        .into_pointer_value(),
                )?;
            }
        }
        let result = self.entry_payload_alloca(ty, true, "aggregate.nullable.result")?;
        self.zero_payload_bytes(result, ty, true)?;
        if stored_nullable {
            self.copy_payload_bytes(result, raw, ty, true)?;
        } else {
            let present = build(self.builder.build_load(
                self.context.i8_type(),
                found,
                "aggregate.nullable.present",
            ))?;
            build(self.builder.build_store(result, present))?;
            let destination = self.byte_offset(
                result,
                ty.nullable_payload_offset,
                "aggregate.nullable.payload",
            )?;
            self.copy_payload_bytes(destination, raw, ty, false)?;
        }
        let mutating = matches!(
            access,
            mir::NullableCollectionAccess::Remove
                | mir::NullableCollectionAccess::Pop
                | mir::NullableCollectionAccess::PopFront
                | mir::NullableCollectionAccess::PopBack
        );
        if !mutating && matches!(mode, mir::PayloadEnumUseMode::Copy) {
            self.retain_payload_enum_at(result, ty, true)?;
        }
        Ok(result)
    }

    fn lower_payload_enum_place(
        &mut self,
        place: &mir::PayloadEnumPlace,
        ty: mir::PayloadEnumType,
        nullable: bool,
        mode: mir::PayloadEnumUseMode,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        if let mir::PayloadEnumPlace::MixedPayload { mixed } = place {
            if nullable {
                return Err(malformed_mir(
                    "mixed payload projection cannot produce a nullable aggregate",
                ));
            }
            let pointer = self.context.ptr_type(AddressSpace::default());
            let i64_type = self.context.i64_type();
            let mixed_slot = local_slot(&self.local_slots, *mixed)?;
            let mixed_value = build(self.builder.build_load(
                pointer,
                mixed_slot,
                "mixed.aggregate.local",
            ))?
            .into_pointer_value();
            let payload_word = self
                .call_runtime(
                    MIXED_PAYLOAD,
                    &[pointer.into()],
                    Some(i64_type.into()),
                    &[mixed_value.into()],
                )?
                .ok_or_else(|| backend_failure("mixed aggregate payload read produced no result"))?
                .into_int_value();
            let source = build(self.builder.build_int_to_ptr(
                payload_word,
                pointer,
                "mixed.aggregate.payload",
            ))?;
            if matches!(mode, mir::PayloadEnumUseMode::Borrow) {
                return Ok(source);
            }
            let destination = self.entry_payload_alloca(ty, false, "mixed.aggregate.use")?;
            self.copy_payload_bytes(destination, source, ty, false)?;
            if matches!(mode, mir::PayloadEnumUseMode::Copy) {
                self.retain_payload_enum_at(destination, ty, false)?;
                return Ok(destination);
            }
            build(self.builder.build_store(mixed_slot, pointer.const_null()))?;
            let final_claim = self
                .call_runtime(
                    MIXED_RELEASE_OWNED,
                    &[pointer.into()],
                    Some(self.context.i8_type().into()),
                    &[mixed_value.into()],
                )?
                .ok_or_else(|| backend_failure("mixed payload move released no ownership claim"))?
                .into_int_value();
            let no_claim = build(self.builder.build_int_compare(
                IntPredicate::EQ,
                final_claim,
                self.context.i8_type().const_zero(),
                "mixed.aggregate.move.shared",
            ))?;
            self.lower_panic_if_code_at_active_site(no_claim, "P1321")?;
            let _ =
                self.call_runtime(MIXED_FREE, &[pointer.into()], None, &[mixed_value.into()])?;
            return Ok(destination);
        }
        let (source, narrowed_nullable_source) = match place {
            mir::PayloadEnumPlace::Local(local) => (local_slot(&self.local_slots, *local)?, None),
            mir::PayloadEnumPlace::NullableLocalAssumeNonNull(local) => {
                let storage = local_slot(&self.local_slots, *local)?;
                (
                    self.byte_offset(
                        storage,
                        ty.nullable_payload_offset,
                        "nonnull.payload.enum.value",
                    )?,
                    Some(storage),
                )
            }
            mir::PayloadEnumPlace::Static(id) => (self.static_address(*id)?, None),
            mir::PayloadEnumPlace::Property { object, property } => {
                (self.lower_property_address(*object, *property)?, None)
            }
            mir::PayloadEnumPlace::CollectionIndex {
                collection,
                index,
                positional,
                remove,
            } => {
                let local = local_in(self.function, *collection)?;
                let mir::Type::Collection(collection_type) = local.ty else {
                    return Err(malformed_mir(
                        "payload enum collection place uses a non-collection local",
                    ));
                };
                let definition = self.collection_definition(collection_type)?.clone();
                let collection_value = self.collection_pointer(*collection)?;
                let index_value = self.lower_rvalue(index)?;
                let index_type = match (*positional, definition.key) {
                    (false, Some(key)) => key,
                    _ => mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64)),
                };
                let index_word = self.value_to_collection_word(index_value, index_type)?;
                let pointer = self.context.ptr_type(AddressSpace::default());
                let byte = self.context.i8_type();
                if *remove {
                    let destination =
                        self.entry_payload_alloca(ty, nullable, "aggregate.collection.remove")?;
                    let usize_type = self.context.ptr_sized_int_type(self.target_data, None);
                    let index = if usize_type.get_bit_width() == 64 {
                        index_word
                    } else {
                        build(self.builder.build_int_truncate(
                            index_word,
                            usize_type,
                            "aggregate.collection.remove.index",
                        ))?
                    };
                    let _ = self.call_runtime(
                        COLLECTION_AGGREGATE_REMOVE_AT_INTO,
                        &[
                            pointer.into(),
                            pointer.into(),
                            usize_type.into(),
                            pointer.into(),
                        ],
                        None,
                        &[
                            self.current_frame.into(),
                            collection_value.into(),
                            index.into(),
                            destination.into(),
                        ],
                    )?;
                    return Ok(destination);
                }
                if matches!(mode, mir::PayloadEnumUseMode::Move) {
                    return Err(malformed_mir(
                        "payload enum collection move requires a removing operation",
                    ));
                }
                (
                    self.call_runtime(
                        COLLECTION_AGGREGATE_VALUE_AT,
                        &[
                            pointer.into(),
                            pointer.into(),
                            self.context.i64_type().into(),
                            byte.into(),
                            byte.into(),
                        ],
                        Some(pointer.into()),
                        &[
                            self.current_frame.into(),
                            collection_value.into(),
                            index_word.into(),
                            byte.const_int(u64::from(*positional), false).into(),
                            self.collection_compare_kind(index_type)?.into(),
                        ],
                    )?
                    .ok_or_else(|| backend_failure("aggregate collection read produced no slot"))?
                    .into_pointer_value(),
                    None,
                )
            }
            mir::PayloadEnumPlace::MixedPayload { .. } => unreachable!(),
        };
        if matches!(mode, mir::PayloadEnumUseMode::Borrow) {
            return Ok(source);
        }
        let destination = self.entry_payload_alloca(ty, nullable, "payload.enum.use")?;
        self.copy_payload_bytes(destination, source, ty, nullable)?;
        if matches!(mode, mir::PayloadEnumUseMode::Copy) {
            self.retain_payload_enum_at(destination, ty, nullable)?;
        } else if let Some(storage) = narrowed_nullable_source {
            self.zero_payload_bytes(storage, ty, true)?;
        } else {
            self.zero_payload_bytes(source, ty, nullable)?;
        }
        Ok(destination)
    }

    fn store_payload_enum_tag(
        &self,
        address: PointerValue<'ctx>,
        width: u32,
        tag: u32,
    ) -> Result<(), BackendError> {
        let ty = match width {
            1 => self.context.i8_type(),
            2 => self.context.i16_type(),
            4 => self.context.i32_type(),
            _ => return Err(malformed_mir("payload enum tag has unsupported width")),
        };
        build(
            self.builder
                .build_store(address, ty.const_int(u64::from(tag), false)),
        )?;
        Ok(())
    }

    fn store_value_at_address(
        &self,
        address: PointerValue<'ctx>,
        value: BasicValueEnum<'ctx>,
        ty: mir::Type,
    ) -> Result<(), BackendError> {
        if let mir::Type::PayloadEnum(payload) | mir::Type::NullablePayloadEnum(payload) = ty {
            return self.copy_payload_bytes(
                address,
                value.into_pointer_value(),
                payload,
                matches!(ty, mir::Type::NullablePayloadEnum(_)),
            );
        }
        build(self.builder.build_store(address, value))?;
        Ok(())
    }

    fn retain_payload_enum_at(
        &mut self,
        address: PointerValue<'ctx>,
        ty: mir::PayloadEnumType,
        nullable: bool,
    ) -> Result<(), BackendError> {
        if nullable {
            let present = build(self.builder.build_load(
                self.context.i8_type(),
                address,
                "payload.retain.present",
            ))?
            .into_int_value();
            let condition = build(self.builder.build_int_compare(
                IntPredicate::NE,
                present,
                self.context.i8_type().const_zero(),
                "payload.retain.is-present",
            ))?;
            let function = current_function(&self.builder)?;
            let retain = self
                .context
                .append_basic_block(function, "payload.retain.some");
            let done = self
                .context
                .append_basic_block(function, "payload.retain.done");
            build(
                self.builder
                    .build_conditional_branch(condition, retain, done),
            )?;
            self.builder.position_at_end(retain);
            let payload =
                self.byte_offset(address, ty.nullable_payload_offset, "payload.retain.value")?;
            self.retain_payload_enum_at(payload, ty, false)?;
            build(self.builder.build_unconditional_branch(done))?;
            self.builder.position_at_end(done);
            return Ok(());
        }
        if !ty.capabilities.needs_drop {
            return Ok(());
        }
        let definition = enum_definition(self.program, ty.id)?.clone();
        self.dispatch_payload_enum_fields(address, &definition, false, |lowerer, field, address| {
            match field.ty {
                mir::Type::String => {
                    let value = build(lowerer.builder.build_load(
                        lowerer.context.ptr_type(AddressSpace::default()),
                        address,
                        "payload.string.copy",
                    ))?
                    .into_pointer_value();
                    let retained = lowerer.retain_string(value)?;
                    build(lowerer.builder.build_store(address, retained))?;
                    Ok(())
                }
                mir::Type::PayloadEnum(nested) => {
                    lowerer.retain_payload_enum_at(address, nested, false)
                }
                mir::Type::NullablePayloadEnum(nested) => {
                    lowerer.retain_payload_enum_at(address, nested, true)
                }
                _ => Ok(()),
            }
        })
    }

    fn drop_payload_enum_at(
        &mut self,
        address: PointerValue<'ctx>,
        ty: mir::PayloadEnumType,
        nullable: bool,
    ) -> Result<(), BackendError> {
        if nullable {
            let present = build(self.builder.build_load(
                self.context.i8_type(),
                address,
                "payload.drop.present",
            ))?
            .into_int_value();
            let condition = build(self.builder.build_int_compare(
                IntPredicate::NE,
                present,
                self.context.i8_type().const_zero(),
                "payload.drop.is-present",
            ))?;
            let function = current_function(&self.builder)?;
            let drop_block = self
                .context
                .append_basic_block(function, "payload.drop.some");
            let done = self
                .context
                .append_basic_block(function, "payload.drop.done");
            build(
                self.builder
                    .build_conditional_branch(condition, drop_block, done),
            )?;
            self.builder.position_at_end(drop_block);
            let payload =
                self.byte_offset(address, ty.nullable_payload_offset, "payload.drop.value")?;
            self.drop_payload_enum_at(payload, ty, false)?;
            build(self.builder.build_unconditional_branch(done))?;
            self.builder.position_at_end(done);
            return Ok(());
        }
        if !ty.capabilities.needs_drop {
            return Ok(());
        }
        let definition = enum_definition(self.program, ty.id)?.clone();
        self.dispatch_payload_enum_fields(address, &definition, true, |lowerer, field, address| {
            lowerer.drop_value_at_address(address, field.ty)
        })
    }

    fn dispatch_payload_enum_fields(
        &mut self,
        address: PointerValue<'ctx>,
        definition: &mir::EnumDefinition,
        reverse: bool,
        mut action: impl FnMut(
            &mut FunctionLowerer<'ctx, '_>,
            &mir::EnumPayloadDefinition,
            PointerValue<'ctx>,
        ) -> Result<(), BackendError>,
    ) -> Result<(), BackendError> {
        let tag_type = match definition.layout.tag_width {
            1 => self.context.i8_type(),
            2 => self.context.i16_type(),
            4 => self.context.i32_type(),
            _ => return Err(malformed_mir("payload enum tag has unsupported width")),
        };
        let tag = build(
            self.builder
                .build_load(tag_type, address, "payload.enum.tag"),
        )?
        .into_int_value();
        let function = current_function(&self.builder)?;
        let done = self
            .context
            .append_basic_block(function, "payload.enum.dispatch.done");
        for (case, layout) in definition.cases.iter().zip(&definition.layout.cases) {
            let selected = self
                .context
                .append_basic_block(function, "payload.enum.case");
            let next = self
                .context
                .append_basic_block(function, "payload.enum.case.next");
            let matches = build(self.builder.build_int_compare(
                IntPredicate::EQ,
                tag,
                tag_type.const_int(u64::from(case.tag), false),
                "payload.enum.case.matches",
            ))?;
            build(
                self.builder
                    .build_conditional_branch(matches, selected, next),
            )?;
            self.builder.position_at_end(selected);
            if reverse {
                for (field, layout) in case.payload.iter().zip(&layout.fields).rev() {
                    let field_address =
                        self.byte_offset(address, layout.offset, "payload.enum.field")?;
                    action(self, field, field_address)?;
                }
            } else {
                for (field, layout) in case.payload.iter().zip(&layout.fields) {
                    let field_address =
                        self.byte_offset(address, layout.offset, "payload.enum.field")?;
                    action(self, field, field_address)?;
                }
            }
            build(self.builder.build_unconditional_branch(done))?;
            self.builder.position_at_end(next);
        }
        build(self.builder.build_unreachable())?;
        self.builder.position_at_end(done);
        Ok(())
    }

    fn drop_value_at_address(
        &mut self,
        address: PointerValue<'ctx>,
        ty: mir::Type,
    ) -> Result<(), BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        match ty {
            mir::Type::Function(_) | mir::Type::NullableFunction(_) => {
                let value = build(self.builder.build_load(
                    closure_carrier_type(self.context),
                    address,
                    "payload.function",
                ))?
                .into_struct_value();
                self.drop_function_carrier(value)
            }
            mir::Type::ClosureEnvironment(_) => Err(malformed_mir(
                "closure environment pointer reached ordinary value cleanup",
            )),
            mir::Type::Error | mir::Type::NullableError => {
                let value = build(self.builder.build_load(
                    error_carrier_type(self.context),
                    address,
                    "payload.error",
                ))?
                .into_struct_value();
                self.drop_error_value(value)
            }
            mir::Type::String | mir::Type::NullableString => {
                let value = if ty == mir::Type::NullableString {
                    let stored = build(self.builder.build_load(
                        llvm_type(self.context, self.target_data, ty),
                        address,
                        "payload.nullable-string",
                    ))?
                    .into_struct_value();
                    self.nullable_parts(stored)?.1.into_pointer_value()
                } else {
                    build(self.builder.build_load(pointer, address, "payload.string"))?
                        .into_pointer_value()
                };
                self.release_string(value)
            }
            mir::Type::Class(class) | mir::Type::NullableClass(class) => {
                let value = build(self.builder.build_load(pointer, address, "payload.class"))?
                    .into_pointer_value();
                self.drop_class_value_checked(value, class)
            }
            mir::Type::Collection(collection) | mir::Type::NullableCollection(collection) => {
                let value = build(
                    self.builder
                        .build_load(pointer, address, "payload.collection"),
                )?
                .into_pointer_value();
                self.drop_collection_value(value, collection)
            }
            mir::Type::Mixed | mir::Type::NullableMixed => {
                let value = build(self.builder.build_load(pointer, address, "payload.mixed"))?
                    .into_pointer_value();
                self.drop_mixed_value(value)
            }
            mir::Type::SharedReference(_) | mir::Type::NullableSharedReference(_) => {
                let value = build(self.builder.build_load(pointer, address, "payload.shared"))?
                    .into_pointer_value();
                self.drop_shared_value(value, false)
            }
            mir::Type::WeakReference(_) | mir::Type::NullableWeakReference(_) => {
                let value = build(self.builder.build_load(pointer, address, "payload.weak"))?
                    .into_pointer_value();
                self.drop_shared_value(value, true)
            }
            mir::Type::PayloadEnum(nested) => self.drop_payload_enum_at(address, nested, false),
            mir::Type::NullablePayloadEnum(nested) => {
                self.drop_payload_enum_at(address, nested, true)
            }
            mir::Type::WritableSharedReference(_)
            | mir::Type::WritableWeakReference(_)
            | mir::Type::NullableWritableSharedReference(_)
            | mir::Type::NullableWritableWeakReference(_)
            | mir::Type::ReadonlySharedReferenceAccess(_)
            | mir::Type::WritableSharedReferenceAccess(_)
            | mir::Type::NullableReadonlySharedReferenceAccess(_)
            | mir::Type::NullableWritableSharedReferenceAccess(_) => {
                let value = build(self.builder.build_load(
                    pointer,
                    address,
                    "payload.writable-shared",
                ))?
                .into_pointer_value();
                let symbol = writable_shared_release_symbol(ty)
                    .ok_or_else(|| malformed_mir("writable shared release symbol is missing"))?;
                self.drop_writable_shared_value(value, symbol)
            }
            mir::Type::Scalar(_) | mir::Type::NullableScalar(_) => Ok(()),
        }
    }

    fn payload_enum_equal_value(
        &mut self,
        left: PointerValue<'ctx>,
        right: PointerValue<'ctx>,
        ty: mir::PayloadEnumType,
    ) -> Result<IntValue<'ctx>, BackendError> {
        let function = current_function(&self.builder)?;
        let equal = self.context.append_basic_block(function, "payload.equal");
        let not_equal = self
            .context
            .append_basic_block(function, "payload.not-equal");
        let done = self
            .context
            .append_basic_block(function, "payload.equal.done");
        self.payload_enum_equal_to_branch(left, right, ty, equal, not_equal)?;
        self.builder.position_at_end(equal);
        build(self.builder.build_unconditional_branch(done))?;
        let equal_end = self
            .builder
            .get_insert_block()
            .expect("payload equal block");
        self.builder.position_at_end(not_equal);
        build(self.builder.build_unconditional_branch(done))?;
        let not_equal_end = self
            .builder
            .get_insert_block()
            .expect("payload not-equal block");
        self.builder.position_at_end(done);
        let phi = build(
            self.builder
                .build_phi(self.context.i8_type(), "payload.equal.value"),
        )?;
        let yes = self.context.i8_type().const_int(1, false);
        let no = self.context.i8_type().const_zero();
        phi.add_incoming(&[(&yes, equal_end), (&no, not_equal_end)]);
        Ok(phi.as_basic_value().into_int_value())
    }

    fn nullable_payload_enum_equal_value(
        &mut self,
        left: PointerValue<'ctx>,
        right: PointerValue<'ctx>,
        ty: mir::PayloadEnumType,
    ) -> Result<IntValue<'ctx>, BackendError> {
        let left_present = build(self.builder.build_load(
            self.context.i8_type(),
            left,
            "nullable.payload.left.present",
        ))?
        .into_int_value();
        let right_present = build(self.builder.build_load(
            self.context.i8_type(),
            right,
            "nullable.payload.right.present",
        ))?
        .into_int_value();
        let function = current_function(&self.builder)?;
        let same_presence = self
            .context
            .append_basic_block(function, "nullable.payload.same-presence");
        let both_present = self
            .context
            .append_basic_block(function, "nullable.payload.both-present");
        let equal = self
            .context
            .append_basic_block(function, "nullable.payload.equal");
        let not_equal = self
            .context
            .append_basic_block(function, "nullable.payload.not-equal");
        let done = self
            .context
            .append_basic_block(function, "nullable.payload.equal.done");
        let presence_equal = build(self.builder.build_int_compare(
            IntPredicate::EQ,
            left_present,
            right_present,
            "nullable.payload.presence.equal",
        ))?;
        build(
            self.builder
                .build_conditional_branch(presence_equal, same_presence, not_equal),
        )?;
        self.builder.position_at_end(same_presence);
        let present = build(self.builder.build_int_compare(
            IntPredicate::NE,
            left_present,
            self.context.i8_type().const_zero(),
            "nullable.payload.present",
        ))?;
        build(
            self.builder
                .build_conditional_branch(present, both_present, equal),
        )?;
        self.builder.position_at_end(both_present);
        let left_payload = self.byte_offset(
            left,
            ty.nullable_payload_offset,
            "nullable.payload.left.value",
        )?;
        let right_payload = self.byte_offset(
            right,
            ty.nullable_payload_offset,
            "nullable.payload.right.value",
        )?;
        self.payload_enum_equal_to_branch(left_payload, right_payload, ty, equal, not_equal)?;
        self.builder.position_at_end(equal);
        build(self.builder.build_unconditional_branch(done))?;
        let equal_end = self
            .builder
            .get_insert_block()
            .expect("nullable equal block");
        self.builder.position_at_end(not_equal);
        build(self.builder.build_unconditional_branch(done))?;
        let not_equal_end = self
            .builder
            .get_insert_block()
            .expect("nullable not-equal block");
        self.builder.position_at_end(done);
        let phi = build(
            self.builder
                .build_phi(self.context.i8_type(), "nullable.payload.equal.value"),
        )?;
        let yes = self.context.i8_type().const_int(1, false);
        let no = self.context.i8_type().const_zero();
        phi.add_incoming(&[(&yes, equal_end), (&no, not_equal_end)]);
        Ok(phi.as_basic_value().into_int_value())
    }

    fn payload_enum_equal_to_branch(
        &mut self,
        left: PointerValue<'ctx>,
        right: PointerValue<'ctx>,
        ty: mir::PayloadEnumType,
        equal: BasicBlock<'ctx>,
        not_equal: BasicBlock<'ctx>,
    ) -> Result<(), BackendError> {
        let definition = enum_definition(self.program, ty.id)?.clone();
        let tag_type = match definition.layout.tag_width {
            1 => self.context.i8_type(),
            2 => self.context.i16_type(),
            4 => self.context.i32_type(),
            _ => return Err(malformed_mir("payload enum tag has unsupported width")),
        };
        let left_tag =
            build(self.builder.build_load(tag_type, left, "payload.left.tag"))?.into_int_value();
        let right_tag = build(
            self.builder
                .build_load(tag_type, right, "payload.right.tag"),
        )?
        .into_int_value();
        let function = current_function(&self.builder)?;
        let dispatch = self
            .context
            .append_basic_block(function, "payload.equal.dispatch");
        let tags_equal = build(self.builder.build_int_compare(
            IntPredicate::EQ,
            left_tag,
            right_tag,
            "payload.tags.equal",
        ))?;
        build(
            self.builder
                .build_conditional_branch(tags_equal, dispatch, not_equal),
        )?;
        self.builder.position_at_end(dispatch);
        for (case, layout) in definition.cases.iter().zip(&definition.layout.cases) {
            let selected = self
                .context
                .append_basic_block(function, "payload.equal.case");
            let next_case = self
                .context
                .append_basic_block(function, "payload.equal.next-case");
            let selected_case = build(self.builder.build_int_compare(
                IntPredicate::EQ,
                left_tag,
                tag_type.const_int(u64::from(case.tag), false),
                "payload.equal.case.matches",
            ))?;
            build(
                self.builder
                    .build_conditional_branch(selected_case, selected, next_case),
            )?;
            self.builder.position_at_end(selected);
            if case.payload.is_empty() {
                build(self.builder.build_unconditional_branch(equal))?;
            } else {
                for (index, (field, field_layout)) in
                    case.payload.iter().zip(&layout.fields).enumerate()
                {
                    let left_field =
                        self.byte_offset(left, field_layout.offset, "payload.left.field")?;
                    let right_field =
                        self.byte_offset(right, field_layout.offset, "payload.right.field")?;
                    let field_equal =
                        self.value_at_address_equal(left_field, right_field, field.ty)?;
                    if index + 1 == case.payload.len() {
                        build(self.builder.build_conditional_branch(
                            field_equal,
                            equal,
                            not_equal,
                        ))?;
                    } else {
                        let next_field = self
                            .context
                            .append_basic_block(function, "payload.equal.next-field");
                        build(self.builder.build_conditional_branch(
                            field_equal,
                            next_field,
                            not_equal,
                        ))?;
                        self.builder.position_at_end(next_field);
                    }
                }
            }
            self.builder.position_at_end(next_case);
        }
        build(self.builder.build_unreachable())?;
        Ok(())
    }

    fn value_at_address_equal(
        &mut self,
        left: PointerValue<'ctx>,
        right: PointerValue<'ctx>,
        ty: mir::Type,
    ) -> Result<IntValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        match ty {
            mir::Type::Scalar(scalar) => {
                let llvm_ty = scalar_type(self.context, scalar);
                let left = build(self.builder.build_load(llvm_ty, left, "payload.left.value"))?;
                let right = build(
                    self.builder
                        .build_load(llvm_ty, right, "payload.right.value"),
                )?;
                Ok(match scalar {
                    mir::ScalarType::Float(_) => build(self.builder.build_float_compare(
                        FloatPredicate::OEQ,
                        left.into_float_value(),
                        right.into_float_value(),
                        "payload.float.equal",
                    ))?,
                    _ => build(self.builder.build_int_compare(
                        IntPredicate::EQ,
                        left.into_int_value(),
                        right.into_int_value(),
                        "payload.scalar.equal",
                    ))?,
                })
            }
            mir::Type::NullableScalar(scalar) => {
                let llvm_ty = llvm_type(self.context, self.target_data, ty);
                let left = build(
                    self.builder
                        .build_load(llvm_ty, left, "payload.left.nullable"),
                )?
                .into_struct_value();
                let right = build(self.builder.build_load(
                    llvm_ty,
                    right,
                    "payload.right.nullable",
                ))?
                .into_struct_value();
                let (left_present, left_value) = self.nullable_parts(left)?;
                let (right_present, right_value) = self.nullable_parts(right)?;
                let presence_equal = build(self.builder.build_int_compare(
                    IntPredicate::EQ,
                    left_present,
                    right_present,
                    "payload.nullable.presence.equal",
                ))?;
                let payload_equal = match scalar {
                    mir::ScalarType::Float(_) => build(self.builder.build_float_compare(
                        FloatPredicate::OEQ,
                        left_value.into_float_value(),
                        right_value.into_float_value(),
                        "payload.nullable.float.equal",
                    ))?,
                    _ => build(self.builder.build_int_compare(
                        IntPredicate::EQ,
                        left_value.into_int_value(),
                        right_value.into_int_value(),
                        "payload.nullable.scalar.equal",
                    ))?,
                };
                let absent = build(self.builder.build_int_compare(
                    IntPredicate::EQ,
                    left_present,
                    self.present_word(false),
                    "payload.nullable.absent",
                ))?;
                let absent_or_equal = build(self.builder.build_or(
                    absent,
                    payload_equal,
                    "payload.nullable.value.equal",
                ))?;
                build(self.builder.build_and(
                    presence_equal,
                    absent_or_equal,
                    "payload.nullable.equal",
                ))
            }
            mir::Type::Error | mir::Type::NullableError => {
                let carrier = error_carrier_type(self.context);
                let left = build(self.builder.build_load(carrier, left, "payload.left.error"))?
                    .into_struct_value();
                let right = build(
                    self.builder
                        .build_load(carrier, right, "payload.right.error"),
                )?
                .into_struct_value();
                let (left_object, left_descriptor) = self.error_parts(left)?;
                let (right_object, right_descriptor) = self.error_parts(right)?;
                let object_equal = build(self.builder.build_int_compare(
                    IntPredicate::EQ,
                    left_object,
                    right_object,
                    "payload.error.object.equal",
                ))?;
                let descriptor_equal = build(self.builder.build_int_compare(
                    IntPredicate::EQ,
                    left_descriptor,
                    right_descriptor,
                    "payload.error.descriptor.equal",
                ))?;
                build(
                    self.builder
                        .build_and(object_equal, descriptor_equal, "payload.error.equal"),
                )
            }
            mir::Type::String | mir::Type::NullableString => {
                let (left, right, symbol) = if ty == mir::Type::NullableString {
                    let llvm_ty = llvm_type(self.context, self.target_data, ty);
                    let left = build(self.builder.build_load(
                        llvm_ty,
                        left,
                        "payload.left.string",
                    ))?
                    .into_struct_value();
                    let right = build(self.builder.build_load(
                        llvm_ty,
                        right,
                        "payload.right.string",
                    ))?
                    .into_struct_value();
                    (
                        self.nullable_parts(left)?.1.into_pointer_value(),
                        self.nullable_parts(right)?.1.into_pointer_value(),
                        NULLABLE_STRING_EQUAL,
                    )
                } else {
                    (
                        build(
                            self.builder
                                .build_load(pointer, left, "payload.left.string"),
                        )?
                        .into_pointer_value(),
                        build(
                            self.builder
                                .build_load(pointer, right, "payload.right.string"),
                        )?
                        .into_pointer_value(),
                        STRING_COMPARE,
                    )
                };
                let result_ty: BasicTypeEnum<'ctx> = if symbol == STRING_COMPARE {
                    self.context.i32_type().into()
                } else {
                    self.context.i8_type().into()
                };
                let compared = self
                    .call_runtime(
                        symbol,
                        &[pointer.into(), pointer.into()],
                        Some(result_ty),
                        &[left.into(), right.into()],
                    )?
                    .ok_or_else(|| backend_failure("payload string comparison produced no result"))?
                    .into_int_value();
                Ok(if symbol == STRING_COMPARE {
                    build(self.builder.build_int_compare(
                        IntPredicate::EQ,
                        compared,
                        self.context.i32_type().const_zero(),
                        "payload.string.equal",
                    ))?
                } else {
                    build(self.builder.build_int_compare(
                        IntPredicate::NE,
                        compared,
                        self.context.i8_type().const_zero(),
                        "payload.nullable-string.equal",
                    ))?
                })
            }
            mir::Type::Class(_) | mir::Type::NullableClass(_) => {
                let left = build(self.builder.build_load(pointer, left, "payload.left.class"))?
                    .into_pointer_value();
                let right = build(
                    self.builder
                        .build_load(pointer, right, "payload.right.class"),
                )?
                .into_pointer_value();
                build(self.builder.build_int_compare(
                    IntPredicate::EQ,
                    left,
                    right,
                    "payload.class.equal",
                ))
            }
            mir::Type::Collection(collection) => {
                if self.collection_definition(collection)?.kind != mir::CollectionKind::Bytes {
                    return Err(malformed_mir(
                        "payload enum field uses collection equality without Bytes semantics",
                    ));
                }
                let left = build(self.builder.build_load(pointer, left, "payload.left.bytes"))?
                    .into_pointer_value();
                let right = build(
                    self.builder
                        .build_load(pointer, right, "payload.right.bytes"),
                )?
                .into_pointer_value();
                self.call_runtime(
                    BYTES_EQUAL,
                    &[pointer.into(), pointer.into()],
                    Some(self.context.i8_type().into()),
                    &[left.into(), right.into()],
                )?
                .ok_or_else(|| backend_failure("Bytes equality produced no result"))
                .map(BasicValueEnum::into_int_value)
            }
            mir::Type::PayloadEnum(payload) => self.payload_enum_equal_value(left, right, payload),
            mir::Type::NullablePayloadEnum(payload) => {
                self.nullable_payload_enum_equal_value(left, right, payload)
            }
            _ => Err(malformed_mir(format!(
                "payload enum field type {ty} has no equality lowering"
            ))),
        }
    }

    fn lower_function_expression(
        &mut self,
        expression: &mir::FunctionExpression,
    ) -> Result<StructValue<'ctx>, BackendError> {
        match expression {
            mir::FunctionExpression::Create {
                descriptor,
                captures,
                ..
            } => {
                let descriptor_pointer = self
                    .closure_descriptors
                    .get(descriptor.0)
                    .ok_or_else(|| malformed_mir("closure descriptor was not emitted"))?
                    .as_pointer_value();
                let environment = self.create_closure_environment(*descriptor, captures)?;
                self.closure_value(descriptor_pointer, environment)
            }
            mir::FunctionExpression::Local {
                local, transfer, ..
            } => {
                let slot = local_slot(&self.local_slots, *local)?;
                let value = build(self.builder.build_load(
                    closure_carrier_type(self.context),
                    slot,
                    "closure.local",
                ))?
                .into_struct_value();
                if *transfer {
                    self.clear_function_slot(slot)?;
                }
                Ok(value)
            }
            mir::FunctionExpression::Property {
                object, property, ..
            } => Ok(build(self.builder.build_load(
                closure_carrier_type(self.context),
                self.lower_property_address(*object, *property)?,
                "closure.property",
            ))?
            .into_struct_value()),
            mir::FunctionExpression::Call { function, args, .. } => Ok(self
                .lower_call(*function, args, true)?
                .ok_or_else(|| malformed_mir("function-valued call returned void"))?
                .into_struct_value()),
            mir::FunctionExpression::AssumePresent { value, .. } => {
                self.lower_nullable_function_expression(value)
            }
            mir::FunctionExpression::CollectionIndex {
                collection,
                index,
                positional,
                remove,
                ..
            } => self.lower_function_collection_index(*collection, index, *positional, *remove),
        }
    }

    fn create_closure_environment(
        &mut self,
        descriptor_id: mir::ClosureDescriptorId,
        captures: &[mir::ClosureCaptureOperand],
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let descriptor = closure_descriptor_in(self.program, descriptor_id)?.clone();
        let Some(logical_id) = descriptor.environment_layout else {
            if !captures.is_empty() {
                return Err(malformed_mir(
                    "environment-free closure has capture operands",
                ));
            }
            return Ok(pointer.const_null());
        };
        let logical = closure_environment_layout_in(self.program, logical_id)?.clone();
        let native = native_closure_abi::environment_layout(
            self.program,
            logical_id,
            self.target_data.get_pointer_byte_size(None),
        )?;
        if captures.len() != logical.fields.len() {
            return Err(malformed_mir(
                "closure capture count disagrees with its environment layout",
            ));
        }
        let environment = match descriptor.environment_placement {
            mir::ClosureEnvironmentPlacement::None => {
                return Err(malformed_mir(
                    "capturing closure uses environment-free placement",
                ));
            }
            mir::ClosureEnvironmentPlacement::Stack => self
                .closure_environment_slots
                .get(descriptor_id.0)
                .copied()
                .flatten()
                .ok_or_else(|| malformed_mir("stack closure environment has no entry alloca"))?,
            mir::ClosureEnvironmentPlacement::Heap => {
                let word = self.context.ptr_sized_int_type(self.target_data, None);
                self.call_runtime(
                    CLOSURE_ENVIRONMENT_ALLOCATE,
                    &[pointer.into(), word.into(), word.into()],
                    Some(pointer.into()),
                    &[
                        self.current_frame.into(),
                        word.const_int(u64::from(native.layout.size), false).into(),
                        word.const_int(u64::from(native.layout.align), false).into(),
                    ],
                )?
                .ok_or_else(|| {
                    backend_failure("closure environment allocation produced no result")
                })?
                .into_pointer_value()
            }
        };
        let size = self
            .context
            .ptr_sized_int_type(self.target_data, None)
            .const_int(u64::from(native.layout.size), false);
        build(self.builder.build_memset(
            environment,
            native.layout.align,
            self.context.i8_type().const_zero(),
            size,
        ))?;
        for ((field, layout), capture) in logical
            .fields
            .iter()
            .zip(native.fields.iter())
            .zip(captures)
        {
            if field.id != layout.field {
                return Err(malformed_mir(
                    "native closure field identity disagrees with MIR",
                ));
            }
            let address = self.byte_offset(environment, layout.offset, "closure.capture.field")?;
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
                    let source = self.closure_source_address(*local)?;
                    build(self.builder.build_store(address, source))?;
                }
                mir::ClosureCaptureOperand::CopyValue(value)
                | mir::ClosureCaptureOperand::MoveValue(value) => {
                    if field.storage != mir::ClosureEnvironmentStorage::Owned {
                        return Err(malformed_mir(
                            "owned capture uses a borrowed environment field",
                        ));
                    }
                    let value = self.lower_rvalue(value)?;
                    self.store_value_at_address(address, value, field.ty)?;
                    if let Some(bit) = layout.live_bit {
                        self.set_environment_live_bit(environment, bit, true)?;
                    }
                }
            }
        }
        Ok(environment)
    }

    fn lower_nullable_function_expression(
        &mut self,
        expression: &mir::NullableFunctionExpression,
    ) -> Result<StructValue<'ctx>, BackendError> {
        match expression {
            mir::NullableFunctionExpression::Null { .. } => {
                Ok(closure_carrier_type(self.context).const_zero())
            }
            mir::NullableFunctionExpression::Present(value) => {
                self.lower_function_expression(value)
            }
            mir::NullableFunctionExpression::Local {
                local, transfer, ..
            } => {
                let slot = local_slot(&self.local_slots, *local)?;
                let value = build(self.builder.build_load(
                    closure_carrier_type(self.context),
                    slot,
                    "nullable-closure.local",
                ))?
                .into_struct_value();
                if *transfer {
                    self.clear_function_slot(slot)?;
                }
                Ok(value)
            }
            mir::NullableFunctionExpression::Property {
                object, property, ..
            } => Ok(build(self.builder.build_load(
                closure_carrier_type(self.context),
                self.lower_property_address(*object, *property)?,
                "nullable-closure.property",
            ))?
            .into_struct_value()),
            mir::NullableFunctionExpression::Call { function, args, .. } => Ok(self
                .lower_call(*function, args, true)?
                .ok_or_else(|| malformed_mir("nullable function call returned void"))?
                .into_struct_value()),
            mir::NullableFunctionExpression::DictionaryGet {
                collection,
                key,
                access,
                ..
            } => self.lower_nullable_function_collection_get(*collection, key, *access),
            mir::NullableFunctionExpression::CollectionIndex {
                collection,
                index,
                positional,
                remove,
                ..
            } => self.lower_function_collection_index(*collection, index, *positional, *remove),
        }
    }

    fn lower_rvalue(
        &mut self,
        expression: &mir::Rvalue,
    ) -> Result<BasicValueEnum<'ctx>, BackendError> {
        match expression {
            mir::Rvalue::Function(value) => Ok(self.lower_function_expression(value)?.into()),
            mir::Rvalue::NullableFunction(value) => {
                Ok(self.lower_nullable_function_expression(value)?.into())
            }
            mir::Rvalue::Value(value) => self.lower_value_expression(value),
            mir::Rvalue::String(value) => Ok(self.lower_string_expression(value)?.into()),
            mir::Rvalue::NullableScalar(value) => {
                Ok(self.lower_nullable_scalar_expression(value)?.into())
            }
            mir::Rvalue::NullableString(value) => {
                Ok(self.lower_nullable_string_expression(value)?.into())
            }
            mir::Rvalue::Class(value) => Ok(self.lower_class_expression(value)?.into()),
            mir::Rvalue::NullableClass(value) => {
                Ok(self.lower_nullable_class_expression(value)?.into())
            }
            mir::Rvalue::Collection(value) => Ok(self.lower_collection_expression(value)?.into()),
            mir::Rvalue::NullableCollection(value) => {
                Ok(self.lower_nullable_collection_expression(value)?.into())
            }
            mir::Rvalue::Mixed(value) => Ok(self.lower_mixed_expression(value)?.into()),
            mir::Rvalue::NullableMixed(value) => {
                Ok(self.lower_nullable_mixed_expression(value)?.into())
            }
            mir::Rvalue::Error(value) => Ok(self.lower_error_expression(value)?.into()),
            mir::Rvalue::NullableError(value) => {
                Ok(self.lower_nullable_error_expression(value)?.into())
            }
            mir::Rvalue::SharedReference(value) => {
                Ok(self.lower_shared_reference_expression(value)?.into())
            }
            mir::Rvalue::WeakReference(value) => {
                Ok(self.lower_weak_reference_expression(value)?.into())
            }
            mir::Rvalue::NullableSharedReference(value) => Ok(self
                .lower_nullable_shared_reference_expression(value)?
                .into()),
            mir::Rvalue::NullableWeakReference(value) => {
                Ok(self.lower_nullable_weak_reference_expression(value)?.into())
            }
            mir::Rvalue::WritableSharedReference(value) => Ok(self
                .lower_writable_shared_reference_expression(value)?
                .into()),
            mir::Rvalue::WritableWeakReference(value) => {
                Ok(self.lower_writable_weak_reference_expression(value)?.into())
            }
            mir::Rvalue::NullableWritableSharedReference(value) => Ok(self
                .lower_nullable_writable_shared_reference_expression(value)?
                .into()),
            mir::Rvalue::NullableWritableWeakReference(value) => Ok(self
                .lower_nullable_writable_weak_reference_expression(value)?
                .into()),
            mir::Rvalue::SharedReferenceAccess(value) => {
                Ok(self.lower_shared_reference_access_expression(value)?.into())
            }
            mir::Rvalue::NullableSharedReferenceAccess(value) => Ok(self
                .lower_nullable_shared_reference_access_expression(value)?
                .into()),
            mir::Rvalue::PayloadEnum(value) => {
                Ok(self.lower_payload_enum_expression(value)?.into())
            }
            mir::Rvalue::NullablePayloadEnum(value) => {
                Ok(self.lower_nullable_payload_enum_expression(value)?.into())
            }
        }
    }

    fn lower_error_expression(
        &mut self,
        expression: &mir::ErrorExpression,
    ) -> Result<StructValue<'ctx>, BackendError> {
        match expression {
            mir::ErrorExpression::Local { local, transfer } => {
                let slot = local_slot(&self.local_slots, *local)?;
                let value = build(self.builder.build_load(
                    error_carrier_type(self.context),
                    slot,
                    "error.local",
                ))?
                .into_struct_value();
                if *transfer {
                    build(
                        self.builder
                            .build_store(slot, error_carrier_type(self.context).const_zero()),
                    )?;
                }
                Ok(value)
            }
            mir::ErrorExpression::NullableLocalAssumeNonNull { local, transfer } => {
                let slot = local_slot(&self.local_slots, *local)?;
                let value = build(self.builder.build_load(
                    error_carrier_type(self.context),
                    slot,
                    "error.nullable.local.nonnull",
                ))?
                .into_struct_value();
                if *transfer {
                    build(
                        self.builder
                            .build_store(slot, error_carrier_type(self.context).const_zero()),
                    )?;
                }
                Ok(value)
            }
            mir::ErrorExpression::FromClass { object, descriptor } => {
                let object = self.lower_class_expression(object)?;
                self.error_value(object, self.error_descriptor_address(*descriptor)?)
            }
            mir::ErrorExpression::FromNullableClass { object, descriptor } => {
                let object = self.lower_nullable_class_expression(object)?;
                let present = build(self.builder.build_is_not_null(object, "error.present"))?;
                let pointer = self.context.ptr_type(AddressSpace::default());
                let descriptor = build(self.builder.build_select(
                    present,
                    self.error_descriptor_address(*descriptor)?,
                    pointer.const_null(),
                    "error.nullable.descriptor",
                ))?
                .into_pointer_value();
                self.error_value(object, descriptor)
            }
            mir::ErrorExpression::Property {
                object,
                property,
                transfer,
            } => {
                let address = self.lower_property_address(*object, *property)?;
                let value = build(self.builder.build_load(
                    error_carrier_type(self.context),
                    address,
                    "error.property",
                ))?
                .into_struct_value();
                if *transfer {
                    build(
                        self.builder
                            .build_store(address, error_carrier_type(self.context).const_zero()),
                    )?;
                }
                Ok(value)
            }
            mir::ErrorExpression::Call { function, args, .. } => Ok(self
                .lower_call(*function, args, true)?
                .ok_or_else(|| malformed_mir("Error call returned void"))?
                .into_struct_value()),
            mir::ErrorExpression::CollectionIndex {
                collection,
                index,
                positional,
                remove,
            } => self.lower_error_collection_index(*collection, index, *positional, *remove),
            mir::ErrorExpression::MixedPayload { mixed, transfer } => {
                let pointer = self.context.ptr_type(AddressSpace::default());
                let slot = local_slot(&self.local_slots, *mixed)?;
                let mixed = build(self.builder.build_load(pointer, slot, "mixed.error.local"))?
                    .into_pointer_value();
                let payload = self
                    .call_runtime(
                        MIXED_PAYLOAD,
                        &[pointer.into()],
                        Some(self.context.i64_type().into()),
                        &[mixed.into()],
                    )?
                    .ok_or_else(|| backend_failure("mixed Error payload read produced no result"))?
                    .into_int_value();
                let address = build(self.builder.build_int_to_ptr(
                    payload,
                    pointer,
                    "mixed.error.payload",
                ))?;
                let value = build(self.builder.build_load(
                    error_carrier_type(self.context),
                    address,
                    "mixed.error.value",
                ))?
                .into_struct_value();
                if *transfer {
                    build(self.builder.build_store(slot, pointer.const_null()))?;
                    let final_claim = self
                        .call_runtime(
                            MIXED_RELEASE_OWNED,
                            &[pointer.into()],
                            Some(self.context.i8_type().into()),
                            &[mixed.into()],
                        )?
                        .ok_or_else(|| {
                            backend_failure("mixed Error move released no ownership claim")
                        })?
                        .into_int_value();
                    let shared = build(self.builder.build_int_compare(
                        IntPredicate::EQ,
                        final_claim,
                        self.context.i8_type().const_zero(),
                        "mixed.error.move.shared",
                    ))?;
                    self.lower_panic_if_code_at_active_site(shared, "P1321")?;
                    let _ =
                        self.call_runtime(MIXED_FREE, &[pointer.into()], None, &[mixed.into()])?;
                }
                Ok(value)
            }
        }
    }

    fn lower_nullable_error_expression(
        &mut self,
        expression: &mir::NullableErrorExpression,
    ) -> Result<StructValue<'ctx>, BackendError> {
        match expression {
            mir::NullableErrorExpression::Null => Ok(error_carrier_type(self.context).const_zero()),
            mir::NullableErrorExpression::Error(value) => self.lower_error_expression(value),
            mir::NullableErrorExpression::Local { local, transfer } => {
                self.lower_error_expression(&mir::ErrorExpression::Local {
                    local: *local,
                    transfer: *transfer,
                })
            }
            mir::NullableErrorExpression::Property {
                object,
                property,
                transfer,
            } => self.lower_error_expression(&mir::ErrorExpression::Property {
                object: *object,
                property: *property,
                transfer: *transfer,
            }),
            mir::NullableErrorExpression::Call { function, args, .. } => Ok(self
                .lower_call(*function, args, true)?
                .ok_or_else(|| malformed_mir("nullable Error call returned void"))?
                .into_struct_value()),
            mir::NullableErrorExpression::CollectionIndex {
                collection,
                index,
                positional,
                remove,
            } => self.lower_error_collection_index(*collection, index, *positional, *remove),
            mir::NullableErrorExpression::DictionaryGet {
                collection,
                key,
                access,
            } => self.lower_nullable_error_collection_get(*collection, key, *access),
        }
    }

    fn lower_error_collection_index(
        &mut self,
        collection: mir::LocalId,
        index: &mir::Rvalue,
        positional: bool,
        remove: bool,
    ) -> Result<StructValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let local = local_in(self.function, collection)?;
        let mir::Type::Collection(collection_type) = local.ty else {
            return Err(malformed_mir(
                "Error collection place uses a non-collection local",
            ));
        };
        let definition = self.collection_definition(collection_type)?.clone();
        let index_type = match (positional, definition.key) {
            (false, Some(key)) => key,
            _ => mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64)),
        };
        let collection_value = self.collection_pointer(collection)?;
        let index_value = self.lower_rvalue(index)?;
        let index_word = self.value_to_collection_word(index_value, index_type)?;
        let address = if remove {
            let slot =
                self.entry_alloca(error_carrier_type(self.context), "error.collection.remove")?;
            let usize_type = self.context.ptr_sized_int_type(self.target_data, None);
            let index = if usize_type.get_bit_width() == 64 {
                index_word
            } else {
                build(self.builder.build_int_truncate(
                    index_word,
                    usize_type,
                    "error.collection.remove.index",
                ))?
            };
            let _ = self.call_runtime(
                COLLECTION_AGGREGATE_REMOVE_AT_INTO,
                &[
                    pointer.into(),
                    pointer.into(),
                    usize_type.into(),
                    pointer.into(),
                ],
                None,
                &[
                    self.current_frame.into(),
                    collection_value.into(),
                    index.into(),
                    slot.into(),
                ],
            )?;
            slot
        } else {
            self.call_runtime(
                COLLECTION_AGGREGATE_VALUE_AT,
                &[
                    pointer.into(),
                    pointer.into(),
                    self.context.i64_type().into(),
                    self.context.i8_type().into(),
                    self.context.i8_type().into(),
                ],
                Some(pointer.into()),
                &[
                    self.current_frame.into(),
                    collection_value.into(),
                    index_word.into(),
                    self.context
                        .i8_type()
                        .const_int(u64::from(positional), false)
                        .into(),
                    self.collection_compare_kind(index_type)?.into(),
                ],
            )?
            .ok_or_else(|| backend_failure("Error collection read produced no slot"))?
            .into_pointer_value()
        };
        if index_type == mir::Type::String {
            self.release_string(index_value.into_pointer_value())?;
        }
        Ok(build(self.builder.build_load(
            error_carrier_type(self.context),
            address,
            "error.collection.value",
        ))?
        .into_struct_value())
    }

    fn lower_function_collection_index(
        &mut self,
        collection: mir::LocalId,
        index: &mir::Rvalue,
        positional: bool,
        remove: bool,
    ) -> Result<StructValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let local = local_in(self.function, collection)?;
        let mir::Type::Collection(collection_type) = local.ty else {
            return Err(malformed_mir(
                "function collection place uses a non-collection local",
            ));
        };
        let definition = self.collection_definition(collection_type)?.clone();
        let index_type = match (positional, definition.key) {
            (false, Some(key)) => key,
            _ => mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64)),
        };
        let collection_value = self.collection_pointer(collection)?;
        let index_value = self.lower_rvalue(index)?;
        let index_word = self.value_to_collection_word(index_value, index_type)?;
        let address = if remove {
            let slot = self.entry_alloca(
                closure_carrier_type(self.context),
                "function.collection.remove",
            )?;
            let usize_type = self.context.ptr_sized_int_type(self.target_data, None);
            let index = if usize_type.get_bit_width() == 64 {
                index_word
            } else {
                build(self.builder.build_int_truncate(
                    index_word,
                    usize_type,
                    "function.collection.remove.index",
                ))?
            };
            let _ = self.call_runtime(
                COLLECTION_AGGREGATE_REMOVE_AT_INTO,
                &[
                    pointer.into(),
                    pointer.into(),
                    usize_type.into(),
                    pointer.into(),
                ],
                None,
                &[
                    self.current_frame.into(),
                    collection_value.into(),
                    index.into(),
                    slot.into(),
                ],
            )?;
            slot
        } else {
            self.call_runtime(
                COLLECTION_AGGREGATE_VALUE_AT,
                &[
                    pointer.into(),
                    pointer.into(),
                    self.context.i64_type().into(),
                    self.context.i8_type().into(),
                    self.context.i8_type().into(),
                ],
                Some(pointer.into()),
                &[
                    self.current_frame.into(),
                    collection_value.into(),
                    index_word.into(),
                    self.context
                        .i8_type()
                        .const_int(u64::from(positional), false)
                        .into(),
                    self.collection_compare_kind(index_type)?.into(),
                ],
            )?
            .ok_or_else(|| backend_failure("function collection read produced no slot"))?
            .into_pointer_value()
        };
        if index_type == mir::Type::String {
            self.release_string(index_value.into_pointer_value())?;
        }
        Ok(build(self.builder.build_load(
            closure_carrier_type(self.context),
            address,
            "function.collection.value",
        ))?
        .into_struct_value())
    }

    fn lower_nullable_error_collection_get(
        &mut self,
        collection: mir::LocalId,
        key: &mir::Rvalue,
        access: mir::NullableCollectionAccess,
    ) -> Result<StructValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let local = local_in(self.function, collection)?;
        let mir::Type::Collection(collection_type) = local.ty else {
            return Err(malformed_mir(
                "nullable Error access uses a non-collection local",
            ));
        };
        let definition = self.collection_definition(collection_type)?.clone();
        let key_type = match access {
            mir::NullableCollectionAccess::Get
            | mir::NullableCollectionAccess::Index
            | mir::NullableCollectionAccess::Remove => definition
                .key
                .ok_or_else(|| malformed_mir("dictionary access has no key type"))?,
            _ => mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64)),
        };
        let collection_value = self.collection_pointer(collection)?;
        let key_value = self.lower_rvalue(key)?;
        let key_word = self.value_to_collection_word(key_value, key_type)?;
        let result = self.entry_alloca(
            error_carrier_type(self.context),
            "error.collection.optional",
        )?;
        build(
            self.builder
                .build_store(result, error_carrier_type(self.context).const_zero()),
        )?;
        let found = self.entry_alloca(self.context.i8_type(), "error.collection.found")?;
        let removed_key =
            self.entry_alloca(self.context.i64_type(), "error.collection.removed-key")?;
        let stored_nullable = definition.value == mir::Type::NullableError;
        let _ = self.call_runtime(
            COLLECTION_AGGREGATE_NULLABLE_ACCESS_INTO,
            &[
                pointer.into(),
                self.context.i64_type().into(),
                self.context.i8_type().into(),
                self.context.i8_type().into(),
                self.context.i8_type().into(),
                pointer.into(),
                pointer.into(),
                pointer.into(),
            ],
            None,
            &[
                collection_value.into(),
                key_word.into(),
                self.collection_compare_kind(key_type)?.into(),
                self.context
                    .i8_type()
                    .const_int(
                        u64::from(nullable_collection_access_code(access).ok_or_else(|| {
                            malformed_mir("Error nullable index requires a direct access code")
                        })?),
                        false,
                    )
                    .into(),
                self.context
                    .i8_type()
                    .const_int(u64::from(stored_nullable), false)
                    .into(),
                found.into(),
                removed_key.into(),
                result.into(),
            ],
        )?;
        if key_type == mir::Type::String {
            self.release_string(key_value.into_pointer_value())?;
            if access == mir::NullableCollectionAccess::Remove {
                let removed = build(self.builder.build_load(
                    self.context.i64_type(),
                    removed_key,
                    "error.collection.removed-key.value",
                ))?
                .into_int_value();
                self.release_string(
                    self.collection_word_to_value(removed, mir::Type::String)?
                        .into_pointer_value(),
                )?;
            }
        }
        Ok(build(self.builder.build_load(
            error_carrier_type(self.context),
            result,
            "error.collection.optional.value",
        ))?
        .into_struct_value())
    }

    fn lower_nullable_function_collection_get(
        &mut self,
        collection: mir::LocalId,
        key: &mir::Rvalue,
        access: mir::NullableCollectionAccess,
    ) -> Result<StructValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let local = local_in(self.function, collection)?;
        let mir::Type::Collection(collection_type) = local.ty else {
            return Err(malformed_mir(
                "nullable function access uses a non-collection local",
            ));
        };
        let definition = self.collection_definition(collection_type)?.clone();
        let key_type = match access {
            mir::NullableCollectionAccess::Get
            | mir::NullableCollectionAccess::Index
            | mir::NullableCollectionAccess::Remove => definition
                .key
                .ok_or_else(|| malformed_mir("dictionary access has no key type"))?,
            _ => mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64)),
        };
        let collection_value = self.collection_pointer(collection)?;
        let key_value = self.lower_rvalue(key)?;
        let key_word = self.value_to_collection_word(key_value, key_type)?;
        let result = self.entry_alloca(
            closure_carrier_type(self.context),
            "function.collection.optional",
        )?;
        build(
            self.builder
                .build_store(result, closure_carrier_type(self.context).const_zero()),
        )?;
        let found = self.entry_alloca(self.context.i8_type(), "function.collection.found")?;
        let removed_key =
            self.entry_alloca(self.context.i64_type(), "function.collection.removed-key")?;
        let stored_nullable = matches!(definition.value, mir::Type::NullableFunction(_));
        let _ = self.call_runtime(
            COLLECTION_AGGREGATE_NULLABLE_ACCESS_INTO,
            &[
                pointer.into(),
                self.context.i64_type().into(),
                self.context.i8_type().into(),
                self.context.i8_type().into(),
                self.context.i8_type().into(),
                pointer.into(),
                pointer.into(),
                pointer.into(),
            ],
            None,
            &[
                collection_value.into(),
                key_word.into(),
                self.collection_compare_kind(key_type)?.into(),
                self.context
                    .i8_type()
                    .const_int(
                        u64::from(nullable_collection_access_code(access).ok_or_else(|| {
                            malformed_mir("function nullable index has no direct access code")
                        })?),
                        false,
                    )
                    .into(),
                self.context
                    .i8_type()
                    .const_int(u64::from(stored_nullable), false)
                    .into(),
                found.into(),
                removed_key.into(),
                result.into(),
            ],
        )?;
        if key_type == mir::Type::String {
            self.release_string(key_value.into_pointer_value())?;
            if access == mir::NullableCollectionAccess::Remove {
                let removed = build(self.builder.build_load(
                    self.context.i64_type(),
                    removed_key,
                    "function.collection.removed-key.value",
                ))?
                .into_int_value();
                self.release_string(
                    self.collection_word_to_value(removed, mir::Type::String)?
                        .into_pointer_value(),
                )?;
            }
        }
        Ok(build(self.builder.build_load(
            closure_carrier_type(self.context),
            result,
            "function.collection.optional.value",
        ))?
        .into_struct_value())
    }

    fn lower_nullable_collection_parts(
        &mut self,
        value: &mir::Rvalue,
        ty: mir::Type,
    ) -> Result<(IntValue<'ctx>, BasicValueEnum<'ctx>, mir::Type), BackendError> {
        let payload_ty = nullable_payload_type(ty)
            .ok_or_else(|| malformed_mir("collection value is not nullable"))?;
        let value = self.lower_rvalue(value)?;
        if matches!(ty, mir::Type::NullableScalar(_) | mir::Type::NullableString) {
            let (present, payload) = self.nullable_parts(value.into_struct_value())?;
            let present = build(self.builder.build_int_truncate(
                present,
                self.context.i8_type(),
                "collection.nullable.present",
            ))?;
            return Ok((present, payload, payload_ty));
        }
        let payload = value.into_pointer_value();
        let present = build(
            self.builder
                .build_is_not_null(payload, "collection.nullable.present"),
        )?;
        let present = build(self.builder.build_int_z_extend(
            present,
            self.context.i8_type(),
            "collection.nullable.present.i8",
        ))?;
        Ok((present, payload.into(), payload_ty))
    }

    fn collection_definition(
        &self,
        id: mir::CollectionTypeId,
    ) -> Result<&mir::CollectionType, BackendError> {
        self.program
            .collection_types
            .get(id.0)
            .filter(|collection| collection.id == id)
            .ok_or_else(|| malformed_mir(format!("collection type#{} does not exist", id.0)))
    }

    fn collection_compare_kind(&self, ty: mir::Type) -> Result<IntValue<'ctx>, BackendError> {
        let kind =
            match ty {
                mir::Type::String => COLLECTION_COMPARE_STRING,
                mir::Type::Scalar(mir::ScalarType::Float(FloatType::Float32)) => {
                    COLLECTION_COMPARE_FLOAT32
                }
                mir::Type::Scalar(mir::ScalarType::Float(FloatType::Float64)) => {
                    COLLECTION_COMPARE_FLOAT64
                }
                mir::Type::Scalar(_)
                | mir::Type::Class(_)
                | mir::Type::Collection(_)
                | mir::Type::Mixed
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
                | mir::Type::NullableWritableSharedReferenceAccess(_) => COLLECTION_COMPARE_WORD,
                mir::Type::NullableScalar(_)
                | mir::Type::NullableString
                | mir::Type::NullableClass(_)
                | mir::Type::NullableCollection(_)
                | mir::Type::NullableMixed => {
                    return Err(malformed_mir(
                        "nullable collection elements are not supported by Stage 23 Slice 1",
                    ))
                }
                mir::Type::PayloadEnum(_) | mir::Type::NullablePayloadEnum(_) => {
                    return Err(malformed_mir(
                        "payload enum collection values require aggregate comparison",
                    ))
                }
                mir::Type::Error | mir::Type::NullableError => {
                    return Err(malformed_mir(
                        "Error collection values require aggregate identity comparison",
                    ))
                }
                mir::Type::Function(_)
                | mir::Type::NullableFunction(_)
                | mir::Type::ClosureEnvironment(_) => return Err(malformed_mir(
                    "function and closure-environment values do not support collection comparison",
                )),
            };
        Ok(self.context.i8_type().const_int(u64::from(kind), false))
    }

    fn value_to_collection_word(
        &self,
        value: BasicValueEnum<'ctx>,
        ty: mir::Type,
    ) -> Result<IntValue<'ctx>, BackendError> {
        let i64_type = self.context.i64_type();
        match ty {
            mir::Type::Scalar(mir::ScalarType::Float(FloatType::Float32)) => {
                let bits = build(self.builder.build_bit_cast(
                    value,
                    self.context.i32_type(),
                    "collection.f32.bits",
                ))?
                .into_int_value();
                Ok(build(self.builder.build_int_z_extend(
                    bits,
                    i64_type,
                    "collection.f32.word",
                ))?)
            }
            mir::Type::Scalar(mir::ScalarType::Float(FloatType::Float64)) => Ok(build(
                self.builder
                    .build_bit_cast(value, i64_type, "collection.f64.word"),
            )?
            .into_int_value()),
            mir::Type::Scalar(_) => {
                let value = value.into_int_value();
                Ok(if value.get_type().get_bit_width() == 64 {
                    value
                } else {
                    build(self.builder.build_int_z_extend(
                        value,
                        i64_type,
                        "collection.scalar.word",
                    ))?
                })
            }
            mir::Type::String
            | mir::Type::Class(_)
            | mir::Type::Collection(_)
            | mir::Type::Mixed
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
                Ok(build(self.builder.build_ptr_to_int(
                    value.into_pointer_value(),
                    i64_type,
                    "collection.pointer.word",
                ))?)
            }
            mir::Type::NullableScalar(_)
            | mir::Type::NullableString
            | mir::Type::NullableClass(_)
            | mir::Type::NullableCollection(_)
            | mir::Type::NullableMixed => Err(malformed_mir(
                "nullable collection elements are not supported by Stage 23 Slice 1",
            )),
            mir::Type::PayloadEnum(_) | mir::Type::NullablePayloadEnum(_) => Err(malformed_mir(
                "payload enum collection values require aggregate transport",
            )),
            mir::Type::Error | mir::Type::NullableError => Err(malformed_mir(
                "Error collection values require aggregate transport",
            )),
            mir::Type::Function(_)
            | mir::Type::NullableFunction(_)
            | mir::Type::ClosureEnvironment(_) => Err(malformed_mir(
                "function carriers require aggregate collection transport",
            )),
        }
    }

    fn collection_word_to_value(
        &self,
        word: IntValue<'ctx>,
        ty: mir::Type,
    ) -> Result<BasicValueEnum<'ctx>, BackendError> {
        Ok(match ty {
            mir::Type::Scalar(mir::ScalarType::Integer(integer)) => {
                let target = integer_type(self.context, integer);
                if integer.bit_width() == 64 {
                    word.into()
                } else {
                    build(self.builder.build_int_truncate(
                        word,
                        target,
                        "collection.integer.value",
                    ))?
                    .into()
                }
            }
            mir::Type::Scalar(mir::ScalarType::Bool) => build(self.builder.build_int_truncate(
                word,
                self.context.i8_type(),
                "collection.bool.value",
            ))?
            .into(),
            mir::Type::Scalar(mir::ScalarType::Enum(_)) => build(self.builder.build_int_truncate(
                word,
                self.context.i32_type(),
                "collection.enum.value",
            ))?
            .into(),
            mir::Type::Scalar(mir::ScalarType::Float(FloatType::Float32)) => {
                let bits = build(self.builder.build_int_truncate(
                    word,
                    self.context.i32_type(),
                    "collection.f32.bits",
                ))?;
                build(self.builder.build_bit_cast(
                    bits,
                    self.context.f32_type(),
                    "collection.f32.value",
                ))?
            }
            mir::Type::Scalar(mir::ScalarType::Float(FloatType::Float64)) => build(
                self.builder
                    .build_bit_cast(word, self.context.f64_type(), "collection.f64.value"),
            )?,
            mir::Type::String
            | mir::Type::Class(_)
            | mir::Type::Collection(_)
            | mir::Type::Mixed
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
                build(self.builder.build_int_to_ptr(
                    word,
                    self.context.ptr_type(AddressSpace::default()),
                    "collection.pointer.value",
                ))?
                .into()
            }
            mir::Type::NullableScalar(_)
            | mir::Type::NullableString
            | mir::Type::NullableClass(_)
            | mir::Type::NullableCollection(_)
            | mir::Type::NullableMixed => {
                return Err(malformed_mir(
                    "nullable collection elements are not supported by Stage 23 Slice 1",
                ))
            }
            mir::Type::PayloadEnum(_) | mir::Type::NullablePayloadEnum(_) => {
                return Err(malformed_mir(
                    "payload enum collection values require aggregate transport",
                ))
            }
            mir::Type::Error | mir::Type::NullableError => {
                return Err(malformed_mir(
                    "Error collection values require aggregate transport",
                ))
            }
            mir::Type::Function(_)
            | mir::Type::NullableFunction(_)
            | mir::Type::ClosureEnvironment(_) => {
                return Err(malformed_mir(
                    "function carriers require aggregate collection transport",
                ))
            }
        })
    }

    fn collection_pointer(&self, local: mir::LocalId) -> Result<PointerValue<'ctx>, BackendError> {
        Ok(build(self.builder.build_load(
            self.context.ptr_type(AddressSpace::default()),
            local_slot(&self.local_slots, local)?,
            "collection.local",
        ))?
        .into_pointer_value())
    }

    fn payload_enum_storage(ty: mir::Type) -> Option<(mir::PayloadEnumType, bool)> {
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
        &mut self,
        collection: PointerValue<'ctx>,
        needle: PointerValue<'ctx>,
        ty: mir::PayloadEnumType,
        nullable: bool,
    ) -> Result<(IntValue<'ctx>, IntValue<'ctx>), BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let usize_type = self.context.ptr_sized_int_type(self.target_data, None);
        let length = self
            .call_runtime(
                COLLECTION_LENGTH,
                &[pointer.into()],
                Some(usize_type.into()),
                &[collection.into()],
            )?
            .ok_or_else(|| backend_failure("aggregate collection length produced no result"))?
            .into_int_value();
        let function = current_function(&self.builder)?;
        let index_slot = self.entry_alloca(usize_type, "aggregate.search.index")?;
        build(
            self.builder
                .build_store(index_slot, usize_type.const_zero()),
        )?;
        let header = self
            .context
            .append_basic_block(function, "aggregate.search.header");
        let body = self
            .context
            .append_basic_block(function, "aggregate.search.body");
        let found = self
            .context
            .append_basic_block(function, "aggregate.search.found");
        let next = self
            .context
            .append_basic_block(function, "aggregate.search.next");
        let missing = self
            .context
            .append_basic_block(function, "aggregate.search.missing");
        let done = self
            .context
            .append_basic_block(function, "aggregate.search.done");
        build(self.builder.build_unconditional_branch(header))?;

        self.builder.position_at_end(header);
        let index = build(self.builder.build_load(
            usize_type,
            index_slot,
            "aggregate.search.index.value",
        ))?
        .into_int_value();
        let in_bounds = build(self.builder.build_int_compare(
            IntPredicate::ULT,
            index,
            length,
            "aggregate.search.in-bounds",
        ))?;
        build(
            self.builder
                .build_conditional_branch(in_bounds, body, missing),
        )?;

        self.builder.position_at_end(body);
        let index_word = if usize_type.get_bit_width() == 64 {
            index
        } else {
            build(self.builder.build_int_z_extend(
                index,
                self.context.i64_type(),
                "aggregate.search.index.i64",
            ))?
        };
        let candidate = self
            .call_runtime(
                COLLECTION_AGGREGATE_VALUE_AT,
                &[
                    pointer.into(),
                    pointer.into(),
                    self.context.i64_type().into(),
                    self.context.i8_type().into(),
                    self.context.i8_type().into(),
                ],
                Some(pointer.into()),
                &[
                    self.current_frame.into(),
                    collection.into(),
                    index_word.into(),
                    self.context.i8_type().const_int(1, false).into(),
                    self.context
                        .i8_type()
                        .const_int(u64::from(COLLECTION_COMPARE_WORD), false)
                        .into(),
                ],
            )?
            .ok_or_else(|| backend_failure("aggregate collection search produced no slot"))?
            .into_pointer_value();
        let equal = if nullable {
            self.nullable_payload_enum_equal_value(candidate, needle, ty)?
        } else {
            self.payload_enum_equal_value(candidate, needle, ty)?
        };
        let equal = build(self.builder.build_int_compare(
            IntPredicate::NE,
            equal,
            self.context.i8_type().const_zero(),
            "aggregate.search.equal",
        ))?;
        build(self.builder.build_conditional_branch(equal, found, next))?;

        self.builder.position_at_end(next);
        let next_index = build(self.builder.build_int_add(
            index,
            usize_type.const_int(1, false),
            "aggregate.search.next-index",
        ))?;
        build(self.builder.build_store(index_slot, next_index))?;
        build(self.builder.build_unconditional_branch(header))?;

        self.builder.position_at_end(found);
        build(self.builder.build_unconditional_branch(done))?;
        self.builder.position_at_end(missing);
        build(self.builder.build_unconditional_branch(done))?;
        self.builder.position_at_end(done);
        let found_value = build(
            self.builder
                .build_phi(self.context.i8_type(), "aggregate.search.result"),
        )?;
        let yes = self.context.i8_type().const_int(1, false);
        let no = self.context.i8_type().const_zero();
        found_value.add_incoming(&[(&yes, found), (&no, missing)]);
        let found_index = build(
            self.builder
                .build_phi(usize_type, "aggregate.search.result-index"),
        )?;
        found_index.add_incoming(&[(&index, found), (&usize_type.const_zero(), missing)]);
        Ok((
            found_value.as_basic_value().into_int_value(),
            found_index.as_basic_value().into_int_value(),
        ))
    }

    fn lower_payload_enum_collection_literal(
        &mut self,
        definition: &mir::CollectionType,
        entries: &[mir::CollectionEntry],
        ty: mir::PayloadEnumType,
        nullable: bool,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let usize_type = self.context.ptr_sized_int_type(self.target_data, None);
        let byte = self.context.i8_type();
        let fixed = definition.kind == mir::CollectionKind::TypedArray;
        let result = self
            .call_runtime(
                COLLECTION_AGGREGATE_NEW,
                &[
                    pointer.into(),
                    usize_type.into(),
                    byte.into(),
                    byte.into(),
                    usize_type.into(),
                    usize_type.into(),
                    byte.into(),
                    byte.into(),
                ],
                Some(pointer.into()),
                &[
                    self.current_frame.into(),
                    usize_type.const_int(entries.len() as u64, false).into(),
                    byte.const_int(u64::from(definition.key.is_some()), false)
                        .into(),
                    byte.const_int(u64::from(fixed), false).into(),
                    usize_type
                        .const_int(u64::from(ty.storage_size(nullable)), false)
                        .into(),
                    usize_type.const_int(u64::from(ty.align), false).into(),
                    byte.const_int(
                        u64::from(stage26_collection_kind(definition.kind).unwrap_or(0)),
                        false,
                    )
                    .into(),
                    byte.const_int(
                        u64::from(
                            definition
                                .comparator
                                .map(collection_comparator_code)
                                .unwrap_or(COLLECTION_COMPARE_WORD),
                        ),
                        false,
                    )
                    .into(),
                ],
            )?
            .ok_or_else(|| backend_failure("aggregate collection allocation produced no result"))?
            .into_pointer_value();
        for (index, entry) in entries.iter().enumerate() {
            let source = self.lower_rvalue(&entry.value)?.into_pointer_value();
            let destination = if let (Some(key_type), Some(key)) = (definition.key, &entry.key) {
                let key = self.lower_rvalue(key)?;
                self.lower_aggregate_dictionary_write_slot(result, key, key_type, ty, nullable)?
            } else if fixed {
                self.call_runtime(
                    COLLECTION_AGGREGATE_VALUE_AT,
                    &[
                        pointer.into(),
                        pointer.into(),
                        self.context.i64_type().into(),
                        byte.into(),
                        byte.into(),
                    ],
                    Some(pointer.into()),
                    &[
                        self.current_frame.into(),
                        result.into(),
                        self.context
                            .i64_type()
                            .const_int(index as u64, false)
                            .into(),
                        byte.const_int(1, false).into(),
                        byte.const_int(u64::from(COLLECTION_COMPARE_WORD), false)
                            .into(),
                    ],
                )?
                .ok_or_else(|| backend_failure("aggregate array initialization produced no slot"))?
                .into_pointer_value()
            } else {
                self.call_runtime(
                    COLLECTION_AGGREGATE_PUSH_SLOT,
                    &[pointer.into()],
                    Some(pointer.into()),
                    &[result.into()],
                )?
                .ok_or_else(|| backend_failure("aggregate insertion produced no slot"))?
                .into_pointer_value()
            };
            self.copy_payload_bytes(destination, source, ty, nullable)?;
        }
        if stage26_collection_kind(definition.kind).is_some() {
            let _ = self.call_runtime(
                COLLECTION_STAGE26_FINALIZE,
                &[pointer.into()],
                None,
                &[result.into()],
            )?;
        }
        Ok(result)
    }

    fn lower_error_collection_literal(
        &mut self,
        definition: &mir::CollectionType,
        entries: &[mir::CollectionEntry],
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let usize_type = self.context.ptr_sized_int_type(self.target_data, None);
        let byte = self.context.i8_type();
        let fixed = definition.kind == mir::CollectionKind::TypedArray;
        let carrier = error_carrier_type(self.context);
        let result = self
            .call_runtime(
                COLLECTION_AGGREGATE_NEW,
                &[
                    pointer.into(),
                    usize_type.into(),
                    byte.into(),
                    byte.into(),
                    usize_type.into(),
                    usize_type.into(),
                    byte.into(),
                    byte.into(),
                ],
                Some(pointer.into()),
                &[
                    self.current_frame.into(),
                    usize_type.const_int(entries.len() as u64, false).into(),
                    byte.const_int(u64::from(definition.key.is_some()), false)
                        .into(),
                    byte.const_int(u64::from(fixed), false).into(),
                    usize_type
                        .const_int(self.target_data.get_store_size(&carrier), false)
                        .into(),
                    usize_type
                        .const_int(
                            u64::from(self.target_data.get_abi_alignment(&carrier)),
                            false,
                        )
                        .into(),
                    byte.const_int(
                        u64::from(stage26_collection_kind(definition.kind).unwrap_or(0)),
                        false,
                    )
                    .into(),
                    byte.const_int(
                        u64::from(
                            definition
                                .comparator
                                .map(collection_comparator_code)
                                .unwrap_or(COLLECTION_COMPARE_WORD),
                        ),
                        false,
                    )
                    .into(),
                ],
            )?
            .ok_or_else(|| backend_failure("Error collection allocation produced no result"))?
            .into_pointer_value();
        for (index, entry) in entries.iter().enumerate() {
            let value = self.lower_rvalue(&entry.value)?;
            let destination = if let (Some(key_type), Some(key)) = (definition.key, &entry.key) {
                let key = self.lower_rvalue(key)?;
                self.lower_error_dictionary_write_slot(result, key, key_type, definition.value)?
            } else if fixed {
                self.call_runtime(
                    COLLECTION_AGGREGATE_VALUE_AT,
                    &[
                        pointer.into(),
                        pointer.into(),
                        self.context.i64_type().into(),
                        byte.into(),
                        byte.into(),
                    ],
                    Some(pointer.into()),
                    &[
                        self.current_frame.into(),
                        result.into(),
                        self.context
                            .i64_type()
                            .const_int(index as u64, false)
                            .into(),
                        byte.const_int(1, false).into(),
                        byte.const_int(u64::from(COLLECTION_COMPARE_WORD), false)
                            .into(),
                    ],
                )?
                .ok_or_else(|| backend_failure("Error array initialization produced no slot"))?
                .into_pointer_value()
            } else {
                self.call_runtime(
                    COLLECTION_AGGREGATE_PUSH_SLOT,
                    &[pointer.into()],
                    Some(pointer.into()),
                    &[result.into()],
                )?
                .ok_or_else(|| backend_failure("Error collection insertion produced no slot"))?
                .into_pointer_value()
            };
            self.store_value_at_address(destination, value, definition.value)?;
        }
        if stage26_collection_kind(definition.kind).is_some() {
            let _ = self.call_runtime(
                COLLECTION_STAGE26_FINALIZE,
                &[pointer.into()],
                None,
                &[result.into()],
            )?;
        }
        Ok(result)
    }

    fn lower_function_collection_literal(
        &mut self,
        definition: &mir::CollectionType,
        entries: &[mir::CollectionEntry],
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let usize_type = self.context.ptr_sized_int_type(self.target_data, None);
        let byte = self.context.i8_type();
        let fixed = definition.kind == mir::CollectionKind::TypedArray;
        let carrier = closure_carrier_type(self.context);
        let result = self
            .call_runtime(
                COLLECTION_AGGREGATE_NEW,
                &[
                    pointer.into(),
                    usize_type.into(),
                    byte.into(),
                    byte.into(),
                    usize_type.into(),
                    usize_type.into(),
                    byte.into(),
                    byte.into(),
                ],
                Some(pointer.into()),
                &[
                    self.current_frame.into(),
                    usize_type.const_int(entries.len() as u64, false).into(),
                    byte.const_int(u64::from(definition.key.is_some()), false)
                        .into(),
                    byte.const_int(u64::from(fixed), false).into(),
                    usize_type
                        .const_int(self.target_data.get_store_size(&carrier), false)
                        .into(),
                    usize_type
                        .const_int(
                            u64::from(self.target_data.get_abi_alignment(&carrier)),
                            false,
                        )
                        .into(),
                    byte.const_int(
                        u64::from(stage26_collection_kind(definition.kind).unwrap_or(0)),
                        false,
                    )
                    .into(),
                    byte.const_int(
                        u64::from(
                            definition
                                .comparator
                                .map(collection_comparator_code)
                                .unwrap_or(COLLECTION_COMPARE_WORD),
                        ),
                        false,
                    )
                    .into(),
                ],
            )?
            .ok_or_else(|| backend_failure("function collection allocation produced no result"))?
            .into_pointer_value();
        for (index, entry) in entries.iter().enumerate() {
            let value = self.lower_rvalue(&entry.value)?;
            let destination = if let (Some(key_type), Some(key)) = (definition.key, &entry.key) {
                let key = self.lower_rvalue(key)?;
                self.lower_error_dictionary_write_slot(result, key, key_type, definition.value)?
            } else if fixed {
                self.call_runtime(
                    COLLECTION_AGGREGATE_VALUE_AT,
                    &[
                        pointer.into(),
                        pointer.into(),
                        self.context.i64_type().into(),
                        byte.into(),
                        byte.into(),
                    ],
                    Some(pointer.into()),
                    &[
                        self.current_frame.into(),
                        result.into(),
                        self.context
                            .i64_type()
                            .const_int(index as u64, false)
                            .into(),
                        byte.const_int(1, false).into(),
                        byte.const_int(u64::from(COLLECTION_COMPARE_WORD), false)
                            .into(),
                    ],
                )?
                .ok_or_else(|| backend_failure("function array initialization produced no slot"))?
                .into_pointer_value()
            } else {
                self.call_runtime(
                    COLLECTION_AGGREGATE_PUSH_SLOT,
                    &[pointer.into()],
                    Some(pointer.into()),
                    &[result.into()],
                )?
                .ok_or_else(|| backend_failure("function collection insertion produced no slot"))?
                .into_pointer_value()
            };
            self.store_value_at_address(destination, value, definition.value)?;
        }
        if stage26_collection_kind(definition.kind).is_some() {
            let _ = self.call_runtime(
                COLLECTION_STAGE26_FINALIZE,
                &[pointer.into()],
                None,
                &[result.into()],
            )?;
        }
        Ok(result)
    }

    fn lower_error_dictionary_write_slot(
        &mut self,
        collection: PointerValue<'ctx>,
        key: BasicValueEnum<'ctx>,
        key_type: mir::Type,
        value_type: mir::Type,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let byte = self.context.i8_type();
        let key_word = self.value_to_collection_word(key, key_type)?;
        let replaced = self.entry_alloca(byte, "error.dictionary.replaced")?;
        let destination = self
            .call_runtime(
                COLLECTION_AGGREGATE_KEYED_SET_SLOT,
                &[
                    pointer.into(),
                    self.context.i64_type().into(),
                    byte.into(),
                    pointer.into(),
                ],
                Some(pointer.into()),
                &[
                    collection.into(),
                    key_word.into(),
                    self.collection_compare_kind(key_type)?.into(),
                    replaced.into(),
                ],
            )?
            .ok_or_else(|| backend_failure("Error dictionary write produced no slot"))?
            .into_pointer_value();
        let replaced = build(self.builder.build_load(
            byte,
            replaced,
            "error.dictionary.replaced.value",
        ))?
        .into_int_value();
        let function = current_function(&self.builder)?;
        let drop = self
            .context
            .append_basic_block(function, "error.dictionary.replace.drop");
        let done = self
            .context
            .append_basic_block(function, "error.dictionary.replace.done");
        let has_old = build(self.builder.build_int_compare(
            IntPredicate::NE,
            replaced,
            byte.const_zero(),
            "error.dictionary.replaced",
        ))?;
        build(self.builder.build_conditional_branch(has_old, drop, done))?;
        self.builder.position_at_end(drop);
        let carrier = match value_type {
            mir::Type::Error | mir::Type::NullableError => error_carrier_type(self.context),
            mir::Type::Function(_) | mir::Type::NullableFunction(_) => {
                closure_carrier_type(self.context)
            }
            _ => {
                return Err(malformed_mir(
                    "two-word dictionary slot has a non-carrier value type",
                ));
            }
        };
        let old = build(
            self.builder
                .build_load(carrier, destination, "aggregate.dictionary.old"),
        )?
        .into_struct_value();
        if matches!(value_type, mir::Type::Error | mir::Type::NullableError) {
            self.drop_error_value(old)?;
        } else {
            self.drop_function_carrier(old)?;
        }
        self.drop_stored_value(key, key_type)?;
        build(self.builder.build_unconditional_branch(done))?;
        self.builder.position_at_end(done);
        Ok(destination)
    }

    fn lower_aggregate_dictionary_write_slot(
        &mut self,
        collection: PointerValue<'ctx>,
        key: BasicValueEnum<'ctx>,
        key_type: mir::Type,
        value_type: mir::PayloadEnumType,
        nullable_value: bool,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let byte = self.context.i8_type();
        let key_word = self.value_to_collection_word(key, key_type)?;
        let replaced = self.entry_alloca(byte, "aggregate.dictionary.replaced")?;
        let destination = self
            .call_runtime(
                COLLECTION_AGGREGATE_KEYED_SET_SLOT,
                &[
                    pointer.into(),
                    self.context.i64_type().into(),
                    byte.into(),
                    pointer.into(),
                ],
                Some(pointer.into()),
                &[
                    collection.into(),
                    key_word.into(),
                    self.collection_compare_kind(key_type)?.into(),
                    replaced.into(),
                ],
            )?
            .ok_or_else(|| backend_failure("aggregate dictionary write produced no slot"))?
            .into_pointer_value();
        let replaced = build(self.builder.build_load(
            byte,
            replaced,
            "aggregate.dictionary.replaced.value",
        ))?
        .into_int_value();
        let function = current_function(&self.builder)?;
        let drop = self
            .context
            .append_basic_block(function, "aggregate.dictionary.replace.drop");
        let done = self
            .context
            .append_basic_block(function, "aggregate.dictionary.replace.done");
        let has_old = build(self.builder.build_int_compare(
            IntPredicate::NE,
            replaced,
            byte.const_zero(),
            "aggregate.dictionary.replaced",
        ))?;
        build(self.builder.build_conditional_branch(has_old, drop, done))?;
        self.builder.position_at_end(drop);
        self.drop_payload_enum_at(destination, value_type, nullable_value)?;
        self.drop_stored_value(key, key_type)?;
        build(self.builder.build_unconditional_branch(done))?;
        self.builder.position_at_end(done);
        Ok(destination)
    }

    fn checked_collection_value_address(
        &mut self,
        collection: PointerValue<'ctx>,
        index: IntValue<'ctx>,
        value_type: mir::Type,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let header = collection_header_type(self.context, self.target_data);
        let usize_type = self.context.ptr_sized_int_type(self.target_data, None);
        let length_address = build(self.builder.build_struct_gep(
            header,
            collection,
            COLLECTION_LENGTH_FIELD,
            "collection.length.address",
        ))?;
        let length = self
            .load_collection_memory(
                usize_type,
                length_address,
                "collection.length",
                CollectionMemoryRegion::Header,
            )?
            .into_int_value();
        let (comparable_index, comparable_length) = match index
            .get_type()
            .get_bit_width()
            .cmp(&usize_type.get_bit_width())
        {
            std::cmp::Ordering::Less => (
                build(self.builder.build_int_z_extend(
                    index,
                    usize_type,
                    "collection.index.usize",
                ))?,
                length,
            ),
            std::cmp::Ordering::Equal => (index, length),
            std::cmp::Ordering::Greater => (
                index,
                build(self.builder.build_int_z_extend(
                    length,
                    index.get_type(),
                    "collection.length.index-width",
                ))?,
            ),
        };
        let out_of_bounds = build(self.builder.build_int_compare(
            IntPredicate::UGE,
            comparable_index,
            comparable_length,
            "collection.index.out-of-bounds",
        ))?;
        // Keep the active source site established by the containing MIR operation,
        // matching collection accesses that go through the shared runtime.
        self.lower_index_bounds_panic_if(out_of_bounds, "P1310", index, length)?;

        let element_index = match index
            .get_type()
            .get_bit_width()
            .cmp(&usize_type.get_bit_width())
        {
            std::cmp::Ordering::Less => build(self.builder.build_int_z_extend(
                index,
                usize_type,
                "collection.index.usize",
            ))?,
            std::cmp::Ordering::Equal => index,
            std::cmp::Ordering::Greater => build(self.builder.build_int_truncate(
                index,
                usize_type,
                "collection.index.usize",
            ))?,
        };
        let values_address = build(self.builder.build_struct_gep(
            header,
            collection,
            COLLECTION_VALUES_FIELD,
            "collection.values.address",
        ))?;
        let values = self
            .load_collection_memory(
                self.context.ptr_type(AddressSpace::default()),
                values_address,
                "collection.values",
                CollectionMemoryRegion::Header,
            )?
            .into_pointer_value();
        build(unsafe {
            self.builder.build_in_bounds_gep(
                collection_storage_type(self.context, self.target_data, value_type)?,
                values,
                &[element_index],
                "collection.value.address",
            )
        })
    }

    fn lower_collection_expression(
        &mut self,
        expression: &mir::CollectionExpression,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let usize_type = self.context.ptr_sized_int_type(self.target_data, None);
        match expression {
            mir::CollectionExpression::StringIntrinsic(call) => {
                Ok(self.lower_string_intrinsic_call(call)?.into_pointer_value())
            }
            mir::CollectionExpression::Local {
                local, transfer, ..
            } => {
                let value = self.collection_pointer(*local)?;
                if *transfer {
                    build(self.builder.build_store(
                        local_slot(&self.local_slots, *local)?,
                        pointer.const_null(),
                    ))?;
                }
                Ok(value)
            }
            mir::CollectionExpression::Literal {
                collection,
                entries,
            } => {
                let definition = self.collection_definition(*collection)?.clone();
                if let Some((ty, nullable)) = Self::payload_enum_storage(definition.value) {
                    return self.lower_payload_enum_collection_literal(
                        &definition,
                        entries,
                        ty,
                        nullable,
                    );
                }
                if matches!(
                    definition.value,
                    mir::Type::Error | mir::Type::NullableError
                ) {
                    return self.lower_error_collection_literal(&definition, entries);
                }
                if matches!(
                    definition.value,
                    mir::Type::Function(_) | mir::Type::NullableFunction(_)
                ) {
                    return self.lower_function_collection_literal(&definition, entries);
                }
                let fixed = definition.kind == mir::CollectionKind::TypedArray;
                let value_width = collection_value_width(
                    definition.value,
                    self.target_data.get_pointer_byte_size(None) as u8,
                )
                .ok_or_else(|| {
                    malformed_mir(
                        "nullable collection elements are not supported by Stage 23 Slice 3",
                    )
                })?;
                let result = if let Some(kind) = stage26_collection_kind(definition.kind) {
                    self.call_runtime(
                        COLLECTION_STAGE26_NEW,
                        &[
                            usize_type.into(),
                            self.context.i8_type().into(),
                            self.context.i8_type().into(),
                            self.context.i8_type().into(),
                            self.context.i8_type().into(),
                        ],
                        Some(pointer.into()),
                        &[
                            usize_type.const_int(entries.len() as u64, false).into(),
                            self.context
                                .i8_type()
                                .const_int(u64::from(definition.key.is_some()), false)
                                .into(),
                            self.context
                                .i8_type()
                                .const_int(u64::from(value_width), false)
                                .into(),
                            self.context
                                .i8_type()
                                .const_int(u64::from(kind), false)
                                .into(),
                            self.context
                                .i8_type()
                                .const_int(
                                    u64::from(
                                        definition
                                            .comparator
                                            .map(collection_comparator_code)
                                            .unwrap_or(COLLECTION_COMPARE_WORD),
                                    ),
                                    false,
                                )
                                .into(),
                        ],
                    )?
                } else {
                    self.call_runtime(
                        COLLECTION_NEW,
                        &[
                            usize_type.into(),
                            self.context.i8_type().into(),
                            self.context.i8_type().into(),
                            self.context.i8_type().into(),
                        ],
                        Some(pointer.into()),
                        &[
                            usize_type.const_int(entries.len() as u64, false).into(),
                            self.context
                                .i8_type()
                                .const_int(u64::from(definition.key.is_some()), false)
                                .into(),
                            self.context
                                .i8_type()
                                .const_int(u64::from(fixed), false)
                                .into(),
                            self.context
                                .i8_type()
                                .const_int(u64::from(value_width), false)
                                .into(),
                        ],
                    )?
                }
                .ok_or_else(|| backend_failure("collection allocation produced no result"))?
                .into_pointer_value();
                for (index, entry) in entries.iter().enumerate() {
                    if let (Some(key_type), Some(key)) = (definition.key, &entry.key) {
                        let key = self.lower_rvalue(key)?;
                        if nullable_payload_type(definition.value).is_some() {
                            let (present, value, payload_ty) = self
                                .lower_nullable_collection_parts(&entry.value, definition.value)?;
                            self.lower_dictionary_set_nullable_value(
                                result, key, key_type, present, value, payload_ty,
                            )?;
                        } else {
                            let value = self.lower_rvalue(&entry.value)?;
                            self.lower_dictionary_set_value(
                                result,
                                key,
                                key_type,
                                value,
                                definition.value,
                            )?;
                        }
                        continue;
                    }
                    if nullable_payload_type(definition.value).is_some() {
                        let (present, value, payload_ty) =
                            self.lower_nullable_collection_parts(&entry.value, definition.value)?;
                        let value_word = self.value_to_collection_word(value, payload_ty)?;
                        if matches!(
                            definition.kind,
                            mir::CollectionKind::Set | mir::CollectionKind::SortedSet
                        ) {
                            let inserted = self
                                .call_runtime(
                                    COLLECTION_PUSH_UNIQUE,
                                    &[
                                        pointer.into(),
                                        self.context.i64_type().into(),
                                        self.context.i8_type().into(),
                                        self.context.i8_type().into(),
                                    ],
                                    Some(self.context.i8_type().into()),
                                    &[
                                        result.into(),
                                        value_word.into(),
                                        present.into(),
                                        self.collection_compare_kind(payload_ty)?.into(),
                                    ],
                                )?
                                .ok_or_else(|| backend_failure("set insertion produced no result"))?
                                .into_int_value();
                            self.drop_value_unless(inserted, value, definition.value)?;
                        } else if fixed {
                            let previous_present = self.entry_alloca(
                                self.context.i8_type(),
                                "collection.previous.present",
                            )?;
                            let _ = self.call_runtime(
                                COLLECTION_SET_AT_NULLABLE,
                                &[
                                    pointer.into(),
                                    pointer.into(),
                                    usize_type.into(),
                                    self.context.i8_type().into(),
                                    self.context.i64_type().into(),
                                    pointer.into(),
                                ],
                                Some(self.context.i64_type().into()),
                                &[
                                    self.current_frame.into(),
                                    result.into(),
                                    usize_type.const_int(index as u64, false).into(),
                                    present.into(),
                                    value_word.into(),
                                    previous_present.into(),
                                ],
                            )?;
                        } else {
                            let _ = self.call_runtime(
                                COLLECTION_PUSH_NULLABLE,
                                &[
                                    pointer.into(),
                                    self.context.i8_type().into(),
                                    self.context.i64_type().into(),
                                ],
                                None,
                                &[result.into(), present.into(), value_word.into()],
                            )?;
                        }
                        continue;
                    }
                    let value = self.lower_rvalue(&entry.value)?;
                    if fixed {
                        let value_word = self.value_to_collection_word(value, definition.value)?;
                        let _ = self.call_runtime(
                            COLLECTION_SET_AT,
                            &[
                                pointer.into(),
                                pointer.into(),
                                usize_type.into(),
                                self.context.i64_type().into(),
                            ],
                            Some(self.context.i64_type().into()),
                            &[
                                self.current_frame.into(),
                                result.into(),
                                usize_type.const_int(index as u64, false).into(),
                                value_word.into(),
                            ],
                        )?;
                    } else if matches!(
                        definition.kind,
                        mir::CollectionKind::Set | mir::CollectionKind::SortedSet
                    ) {
                        let value_word = self.value_to_collection_word(value, definition.value)?;
                        let inserted = self
                            .call_runtime(
                                COLLECTION_PUSH_UNIQUE,
                                &[
                                    pointer.into(),
                                    self.context.i64_type().into(),
                                    self.context.i8_type().into(),
                                    self.context.i8_type().into(),
                                ],
                                Some(self.context.i8_type().into()),
                                &[
                                    result.into(),
                                    value_word.into(),
                                    self.context.i8_type().const_int(1, false).into(),
                                    self.collection_compare_kind(definition.value)?.into(),
                                ],
                            )?
                            .ok_or_else(|| backend_failure("set insertion produced no result"))?
                            .into_int_value();
                        self.drop_value_unless(inserted, value, definition.value)?;
                    } else {
                        let value_word = self.value_to_collection_word(value, definition.value)?;
                        let _ = self.call_runtime(
                            COLLECTION_PUSH,
                            &[pointer.into(), self.context.i64_type().into()],
                            None,
                            &[result.into(), value_word.into()],
                        )?;
                    }
                }
                if stage26_collection_kind(definition.kind).is_some() {
                    let _ = self.call_runtime(
                        COLLECTION_STAGE26_FINALIZE,
                        &[pointer.into()],
                        None,
                        &[result.into()],
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
                let definition = self.collection_definition(*collection)?.clone();
                let fixed = definition.kind == mir::CollectionKind::TypedArray;
                let value = self.lower_rvalue(value)?;
                let count = self.lower_integer_expression(count)?;
                self.set_active_panic_site(*count_span)?;
                let fixed = self.context.i8_type().const_int(u64::from(fixed), false);
                let value_width = self.context.i8_type().const_int(
                    u64::from(
                        collection_value_width(
                            definition.value,
                            self.target_data.get_pointer_byte_size(None) as u8,
                        )
                        .ok_or_else(|| {
                            malformed_mir(
                                "nullable collection elements are not supported by Stage 23 Slice 3",
                            )
                        })?,
                    ),
                    false,
                );
                let (name, value_type, argument) = if definition.value == mir::Type::String {
                    (COLLECTION_FILL_STRING, pointer.into(), value)
                } else {
                    let word = self.value_to_collection_word(value, definition.value)?;
                    (
                        COLLECTION_FILL_WORD,
                        self.context.i64_type().into(),
                        word.into(),
                    )
                };
                let mut parameter_types = vec![
                    pointer.into(),
                    value_type,
                    self.context.i64_type().into(),
                    self.context.i8_type().into(),
                ];
                let mut arguments = vec![
                    self.current_frame.into(),
                    argument.into(),
                    count.into(),
                    fixed.into(),
                ];
                if name == COLLECTION_FILL_WORD {
                    parameter_types.push(self.context.i8_type().into());
                    arguments.push(value_width.into());
                }
                let result = self
                    .call_runtime(name, &parameter_types, Some(pointer.into()), &arguments)?
                    .ok_or_else(|| {
                        backend_failure("collection fill allocation produced no result")
                    })?
                    .into_pointer_value();
                self.drop_stored_value(value, definition.value)?;
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
                self.set_active_panic_site(*index_span)?;
                Ok(self
                    .lower_collection_index(*source, index, *transfer, *positional)?
                    .into_pointer_value())
            }
            mir::CollectionExpression::Property {
                object, property, ..
            } => Ok(build(self.builder.build_load(
                pointer,
                self.lower_property_address(*object, *property)?,
                "collection.property",
            ))?
            .into_pointer_value()),
            mir::CollectionExpression::SharedAccessPayload {
                access, writable, ..
            } => self.lower_shared_access_payload(*access, *writable),
            mir::CollectionExpression::From {
                collection,
                source,
                transfer,
                algebra,
            } => self.lower_set_from(*collection, *source, *transfer, *algebra),
            mir::CollectionExpression::FromBytes { source, .. } => {
                let source = self.collection_pointer(*source)?;
                Ok(self
                    .call_runtime(
                        BYTES_TO_COLLECTION,
                        &[pointer.into()],
                        Some(pointer.into()),
                        &[source.into()],
                    )?
                    .ok_or_else(|| backend_failure("Bytes::toArray produced no result"))?
                    .into_pointer_value())
            }
            mir::CollectionExpression::BytesFromArray { source, .. } => {
                let source = self.collection_pointer(*source)?;
                Ok(self
                    .call_runtime(
                        BYTES_FROM_COLLECTION,
                        &[pointer.into()],
                        Some(pointer.into()),
                        &[source.into()],
                    )?
                    .ok_or_else(|| backend_failure("Bytes::fromArray produced no result"))?
                    .into_pointer_value())
            }
            mir::CollectionExpression::ReadFileBytes {
                path, path_span, ..
            } => {
                let path = self.lower_string_expression(path)?;
                self.set_active_panic_site(*path_span)?;
                let result = self
                    .call_runtime(
                        READ_FILE_BYTES,
                        &[pointer.into(), pointer.into()],
                        Some(pointer.into()),
                        &[self.current_frame.into(), path.into()],
                    )?
                    .ok_or_else(|| backend_failure("read_file_bytes produced no result"))?
                    .into_pointer_value();
                self.release_string(path)?;
                Ok(result)
            }
            mir::CollectionExpression::ReadStdinBytes { .. } => Ok(self
                .call_runtime(
                    READ_STDIN_BYTES,
                    &[pointer.into()],
                    Some(pointer.into()),
                    &[self.current_frame.into()],
                )?
                .ok_or_else(|| backend_failure("read_stdin_bytes produced no result"))?
                .into_pointer_value()),
            mir::CollectionExpression::Call { function, args, .. } => Ok(self
                .lower_call(*function, args, true)?
                .ok_or_else(|| malformed_mir("collection call produced no result"))?
                .into_pointer_value()),
        }
    }

    fn lower_nullable_collection_expression(
        &mut self,
        expression: &mir::NullableCollectionExpression,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        match expression {
            mir::NullableCollectionExpression::Null(_) => Ok(pointer.const_null()),
            mir::NullableCollectionExpression::Collection(value) => {
                self.lower_collection_expression(value)
            }
            mir::NullableCollectionExpression::Local {
                local, transfer, ..
            } => {
                let slot = local_slot(&self.local_slots, *local)?;
                let value = build(self.builder.build_load(
                    pointer,
                    slot,
                    "nullable-collection.local",
                ))?
                .into_pointer_value();
                if *transfer {
                    build(self.builder.build_store(slot, pointer.const_null()))?;
                }
                Ok(value)
            }
            mir::NullableCollectionExpression::Property {
                object, property, ..
            } => Ok(build(self.builder.build_load(
                pointer,
                self.lower_property_address(*object, *property)?,
                "nullable-collection.property",
            ))?
            .into_pointer_value()),
            mir::NullableCollectionExpression::Call { function, args, .. } => Ok(self
                .lower_call(*function, args, true)?
                .ok_or_else(|| malformed_mir("nullable collection call produced no result"))?
                .into_pointer_value()),
            mir::NullableCollectionExpression::Coalesce {
                collection,
                left,
                right,
                transfer,
            } => {
                let left_owned = left.owned_temporary_collection().is_some();
                let right_owned = right.owned_temporary_collection().is_some();
                let left = self.lower_nullable_collection_expression(left)?;
                let function = current_function(&self.builder)?;
                let some = self
                    .context
                    .append_basic_block(function, "nullable-collection.coalesce.some");
                let none = self
                    .context
                    .append_basic_block(function, "nullable-collection.coalesce.none");
                let done = self
                    .context
                    .append_basic_block(function, "nullable-collection.coalesce.done");
                let present = build(
                    self.builder
                        .build_is_not_null(left, "nullable-collection.coalesce.present"),
                )?;
                build(self.builder.build_conditional_branch(present, some, none))?;
                self.builder.position_at_end(some);
                build(self.builder.build_unconditional_branch(done))?;
                let some_end = self
                    .builder
                    .get_insert_block()
                    .expect("nullable collection coalesce some block");
                self.builder.position_at_end(none);
                let right = self.lower_nullable_collection_expression(right)?;
                build(self.builder.build_unconditional_branch(done))?;
                let none_end = self
                    .builder
                    .get_insert_block()
                    .expect("nullable collection coalesce none block");
                self.builder.position_at_end(done);
                let result = build(
                    self.builder
                        .build_phi(pointer, "nullable-collection.coalesce"),
                )?;
                result.add_incoming(&[(&left, some_end), (&right, none_end)]);
                if !transfer && (left_owned || right_owned) {
                    let temporary = build(
                        self.builder
                            .build_phi(pointer, "nullable-collection.coalesce.temporary"),
                    )?;
                    let null = pointer.const_null();
                    let left_temporary = if left_owned { left } else { null };
                    let right_temporary = if right_owned { right } else { null };
                    temporary
                        .add_incoming(&[(&left_temporary, some_end), (&right_temporary, none_end)]);
                    self.defer_or_drop_collection_temporary(
                        temporary.as_basic_value().into_pointer_value(),
                        *collection,
                    )?;
                }
                Ok(result.as_basic_value().into_pointer_value())
            }
        }
    }

    fn lower_collection_index(
        &mut self,
        collection: mir::LocalId,
        index: &mir::Rvalue,
        remove: bool,
        positional: bool,
    ) -> Result<BasicValueEnum<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let usize_type = self.context.ptr_sized_int_type(self.target_data, None);
        let local = local_in(self.function, collection)?;
        let mir::Type::Collection(collection_type) = local.ty else {
            return Err(malformed_mir("collection index uses non-collection local"));
        };
        let definition = self.collection_definition(collection_type)?.clone();
        let collection_value = self.collection_pointer(collection)?;
        if definition.kind == mir::CollectionKind::Bytes {
            if remove {
                return Err(malformed_mir("byte indexing cannot remove a value"));
            }
            let index = self.lower_rvalue(index)?.into_int_value();
            return self
                .call_runtime(
                    BYTES_GET,
                    &[pointer.into(), pointer.into(), usize_type.into()],
                    Some(self.context.i8_type().into()),
                    &[
                        self.current_frame.into(),
                        collection_value.into(),
                        index.into(),
                    ],
                )?
                .ok_or_else(|| backend_failure("byte index produced no result"));
        }
        let index_type = definition
            .key
            .unwrap_or(mir::Type::Scalar(mir::ScalarType::Integer(
                IntegerType::Int64,
            )));
        let index_value = self.lower_rvalue(index)?;
        let word = if definition.key.is_some() && !positional {
            if remove {
                return Err(malformed_mir(
                    "dictionary indexed removal must use Dictionary::remove",
                ));
            }
            let index_word = self.value_to_collection_word(index_value, index_type)?;
            let found = self.entry_alloca(self.context.i8_type(), "dictionary.found")?;
            let word = self
                .call_runtime(
                    COLLECTION_KEYED_GET,
                    &[
                        pointer.into(),
                        self.context.i64_type().into(),
                        self.context.i8_type().into(),
                        pointer.into(),
                    ],
                    Some(self.context.i64_type().into()),
                    &[
                        collection_value.into(),
                        index_word.into(),
                        self.collection_compare_kind(index_type)?.into(),
                        found.into(),
                    ],
                )?
                .ok_or_else(|| backend_failure("dictionary lookup produced no result"))?
                .into_int_value();
            let found = build(self.builder.build_load(
                self.context.i8_type(),
                found,
                "dictionary.found",
            ))?
            .into_int_value();
            let missing = build(self.builder.build_int_compare(
                IntPredicate::EQ,
                found,
                self.context.i8_type().const_zero(),
                "dictionary.missing",
            ))?;
            self.lower_panic_if_code_at_active_site(missing, "P1312")?;
            if index_type == mir::Type::String {
                self.release_string(index_value.into_pointer_value())?;
            }
            word
        } else if remove {
            self.call_runtime(
                COLLECTION_REMOVE_AT,
                &[pointer.into(), pointer.into(), usize_type.into()],
                Some(self.context.i64_type().into()),
                &[
                    self.current_frame.into(),
                    collection_value.into(),
                    index_value.into(),
                ],
            )?
            .ok_or_else(|| backend_failure("collection removal produced no result"))?
            .into_int_value()
        } else if stage26_collection_kind(definition.kind).is_some() {
            self.call_runtime(
                COLLECTION_VALUE_AT,
                &[pointer.into(), pointer.into(), usize_type.into()],
                Some(self.context.i64_type().into()),
                &[
                    self.current_frame.into(),
                    collection_value.into(),
                    index_value.into(),
                ],
            )?
            .ok_or_else(|| backend_failure("collection index produced no result"))?
            .into_int_value()
        } else {
            let storage_type =
                collection_storage_type(self.context, self.target_data, definition.value)?;
            let address = self.checked_collection_value_address(
                collection_value,
                index_value.into_int_value(),
                definition.value,
            )?;
            let value = self.load_collection_memory(
                storage_type,
                address,
                "collection.value",
                CollectionMemoryRegion::Values,
            )?;
            if matches!(definition.value, mir::Type::Scalar(_)) {
                return Ok(value);
            }
            collection_storage_to_word(&self.builder, self.context, value.into_int_value())?
        };
        self.collection_word_to_value(word, definition.value)
    }

    fn lower_collection_key_at(
        &mut self,
        collection: mir::LocalId,
        offset: &mir::Rvalue,
        expected: mir::Type,
    ) -> Result<BasicValueEnum<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let usize_type = self.context.ptr_sized_int_type(self.target_data, None);
        let local = local_in(self.function, collection)?;
        let mir::Type::Collection(collection_type) = local.ty else {
            return Err(malformed_mir(
                "collection key access uses non-collection local",
            ));
        };
        if self.collection_definition(collection_type)?.key != Some(expected) {
            return Err(malformed_mir("collection key access has another type"));
        }
        let collection = self.collection_pointer(collection)?;
        let offset = self.lower_rvalue(offset)?;
        let word = self
            .call_runtime(
                COLLECTION_KEY_AT,
                &[pointer.into(), pointer.into(), usize_type.into()],
                Some(self.context.i64_type().into()),
                &[
                    self.current_frame.into(),
                    collection.into(),
                    offset.into_int_value().into(),
                ],
            )?
            .ok_or_else(|| backend_failure("collection key read produced no result"))?
            .into_int_value();
        self.collection_word_to_value(word, expected)
    }

    fn lower_dictionary_get(
        &mut self,
        collection: mir::LocalId,
        key: &mir::Rvalue,
        expected: mir::Type,
        access: mir::NullableCollectionAccess,
    ) -> Result<(IntValue<'ctx>, BasicValueEnum<'ctx>), BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let local = local_in(self.function, collection)?;
        let mir::Type::Collection(collection_type) = local.ty else {
            return Err(malformed_mir("Dictionary::get uses a non-collection local"));
        };
        let definition = self.collection_definition(collection_type)?.clone();
        if definition.value != expected && nullable_payload_type(definition.value) != Some(expected)
        {
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
        let collection = self.collection_pointer(collection)?;
        let key_value = self.lower_rvalue(key)?;
        let key_word = self.value_to_collection_word(key_value, key_type)?;
        let found = self.entry_alloca(self.context.i8_type(), "dictionary.get.found")?;
        let removed_key = self.entry_alloca(self.context.i64_type(), "dictionary.removed.key")?;
        if access == mir::NullableCollectionAccess::Index {
            let present = self.entry_alloca(self.context.i8_type(), "dictionary.index.present")?;
            let word = self
                .call_runtime(
                    COLLECTION_KEYED_GET_NULLABLE,
                    &[
                        pointer.into(),
                        self.context.i64_type().into(),
                        self.context.i8_type().into(),
                        pointer.into(),
                        pointer.into(),
                    ],
                    Some(self.context.i64_type().into()),
                    &[
                        collection.into(),
                        key_word.into(),
                        self.collection_compare_kind(key_type)?.into(),
                        found.into(),
                        present.into(),
                    ],
                )?
                .ok_or_else(|| backend_failure("nullable dictionary lookup produced no result"))?
                .into_int_value();
            let found_value = build(self.builder.build_load(
                self.context.i8_type(),
                found,
                "dictionary.index.found",
            ))?
            .into_int_value();
            let missing = build(self.builder.build_int_compare(
                IntPredicate::EQ,
                found_value,
                self.context.i8_type().const_zero(),
                "dictionary.index.missing",
            ))?;
            self.lower_panic_if_code_at_active_site(missing, "P1312")?;
            if key_type == mir::Type::String {
                self.release_string(key_value.into_pointer_value())?;
            }
            let present = build(self.builder.build_load(
                self.context.i8_type(),
                present,
                "dictionary.index.present.value",
            ))?
            .into_int_value();
            let present = build(self.builder.build_int_z_extend(
                present,
                self.context.ptr_sized_int_type(self.target_data, None),
                "dictionary.index.present.extended",
            ))?;
            return Ok((present, self.collection_word_to_value(word, expected)?));
        }
        let access_value =
            self.context.i8_type().const_int(
                u64::from(nullable_collection_access_code(access).ok_or_else(|| {
                    malformed_mir("nullable index must use the direct index path")
                })?),
                false,
            );
        let word = self
            .call_runtime(
                COLLECTION_NULLABLE_ACCESS,
                &[
                    pointer.into(),
                    self.context.i64_type().into(),
                    self.context.i8_type().into(),
                    self.context.i8_type().into(),
                    pointer.into(),
                    pointer.into(),
                ],
                Some(self.context.i64_type().into()),
                &[
                    collection.into(),
                    key_word.into(),
                    self.collection_compare_kind(key_type)?.into(),
                    access_value.into(),
                    found.into(),
                    removed_key.into(),
                ],
            )?
            .ok_or_else(|| backend_failure("nullable collection access produced no result"))?
            .into_int_value();
        if key_type == mir::Type::String {
            self.release_string(key_value.into_pointer_value())?;
            if access == mir::NullableCollectionAccess::Remove {
                let removed_key = build(self.builder.build_load(
                    self.context.i64_type(),
                    removed_key,
                    "dictionary.removed.key.value",
                ))?
                .into_int_value();
                self.release_string(
                    self.collection_word_to_value(removed_key, mir::Type::String)?
                        .into_pointer_value(),
                )?;
            }
        }
        let found = build(self.builder.build_load(
            self.context.i8_type(),
            found,
            "dictionary.get.found.value",
        ))?
        .into_int_value();
        let present = build(self.builder.build_int_z_extend(
            found,
            self.context.ptr_sized_int_type(self.target_data, None),
            "dictionary.get.present",
        ))?;
        Ok((present, self.collection_word_to_value(word, expected)?))
    }

    fn lower_collection_add(
        &mut self,
        collection: mir::LocalId,
        value: &mir::Rvalue,
        index: Option<&mir::Rvalue>,
        op: mir::CollectionMutationOp,
    ) -> Result<(), BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let local = local_in(self.function, collection)?;
        let mir::Type::Collection(collection_type) = local.ty else {
            return Err(malformed_mir("collection add uses non-collection local"));
        };
        let definition = self.collection_definition(collection_type)?.clone();
        let collection_value = self.collection_pointer(collection)?;
        let index = if op == mir::CollectionMutationOp::InsertAt {
            Some(
                self.lower_rvalue(index.ok_or_else(|| malformed_mir("insertAt has no index"))?)?
                    .into_int_value(),
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
            let value = self.lower_rvalue(value)?;
            let destination = if op == mir::CollectionMutationOp::InsertAt {
                self.call_runtime(
                    COLLECTION_AGGREGATE_INSERT_SLOT,
                    &[
                        pointer.into(),
                        pointer.into(),
                        self.context
                            .ptr_sized_int_type(self.target_data, None)
                            .into(),
                    ],
                    Some(pointer.into()),
                    &[
                        self.current_frame.into(),
                        collection_value.into(),
                        index.expect("insertAt index was lowered").into(),
                    ],
                )?
                .ok_or_else(|| backend_failure("Error collection insertion produced no slot"))?
                .into_pointer_value()
            } else {
                let symbol = if op == mir::CollectionMutationOp::PushFront {
                    COLLECTION_AGGREGATE_PUSH_FRONT_SLOT
                } else {
                    COLLECTION_AGGREGATE_PUSH_SLOT
                };
                self.call_runtime(
                    symbol,
                    &[pointer.into()],
                    Some(pointer.into()),
                    &[collection_value.into()],
                )?
                .ok_or_else(|| backend_failure("Error collection insertion produced no slot"))?
                .into_pointer_value()
            };
            self.store_value_at_address(destination, value, definition.value)?;
            return Ok(());
        }
        if let Some((ty, nullable)) = Self::payload_enum_storage(definition.value) {
            if op == mir::CollectionMutationOp::Remove {
                return Err(malformed_mir(
                    "payload enum remove-by-value requires generated enum equality",
                ));
            }
            let source = self.lower_rvalue(value)?.into_pointer_value();
            let destination = if op == mir::CollectionMutationOp::InsertAt {
                self.call_runtime(
                    COLLECTION_AGGREGATE_INSERT_SLOT,
                    &[
                        pointer.into(),
                        pointer.into(),
                        self.context
                            .ptr_sized_int_type(self.target_data, None)
                            .into(),
                    ],
                    Some(pointer.into()),
                    &[
                        self.current_frame.into(),
                        collection_value.into(),
                        index.expect("insertAt index was lowered").into(),
                    ],
                )?
                .ok_or_else(|| backend_failure("aggregate insertion produced no slot"))?
                .into_pointer_value()
            } else {
                let symbol = if op == mir::CollectionMutationOp::PushFront {
                    COLLECTION_AGGREGATE_PUSH_FRONT_SLOT
                } else {
                    COLLECTION_AGGREGATE_PUSH_SLOT
                };
                self.call_runtime(
                    symbol,
                    &[pointer.into()],
                    Some(pointer.into()),
                    &[collection_value.into()],
                )?
                .ok_or_else(|| backend_failure("aggregate insertion produced no slot"))?
                .into_pointer_value()
            };
            self.copy_payload_bytes(destination, source, ty, nullable)?;
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
                self.lower_nullable_collection_parts(value, definition.value)?;
            let word = self.value_to_collection_word(value, payload_ty)?;
            if op == mir::CollectionMutationOp::Remove {
                let removed_slot =
                    self.entry_alloca(self.context.i64_type(), "collection.removed.value")?;
                let removed_present_slot =
                    self.entry_alloca(self.context.i8_type(), "collection.removed.present")?;
                let removed = self
                    .call_runtime(
                        COLLECTION_REMOVE_VALUE,
                        &[
                            pointer.into(),
                            self.context.i64_type().into(),
                            self.context.i8_type().into(),
                            self.context.i8_type().into(),
                            pointer.into(),
                            pointer.into(),
                        ],
                        Some(self.context.i8_type().into()),
                        &[
                            collection_value.into(),
                            word.into(),
                            present.into(),
                            self.collection_compare_kind(payload_ty)?.into(),
                            removed_slot.into(),
                            removed_present_slot.into(),
                        ],
                    )?
                    .ok_or_else(|| backend_failure("collection removal produced no result"))?
                    .into_int_value();
                let removed_word = build(self.builder.build_load(
                    self.context.i64_type(),
                    removed_slot,
                    "collection.removed.word",
                ))?
                .into_int_value();
                let removed_value = self.collection_word_to_value(removed_word, payload_ty)?;
                let removed_present = build(self.builder.build_load(
                    self.context.i8_type(),
                    removed_present_slot,
                    "collection.removed.present.value",
                ))?
                .into_int_value();
                let should_drop = build(self.builder.build_and(
                    removed,
                    removed_present,
                    "collection.removed.should-drop",
                ))?;
                self.drop_value_if(should_drop, removed_value, definition.value)?;
                self.drop_stored_value(value, definition.value)?;
            } else if op == mir::CollectionMutationOp::Add
                && matches!(
                    definition.kind,
                    mir::CollectionKind::Set | mir::CollectionKind::SortedSet
                )
            {
                let inserted = self
                    .call_runtime(
                        COLLECTION_PUSH_UNIQUE,
                        &[
                            pointer.into(),
                            self.context.i64_type().into(),
                            self.context.i8_type().into(),
                            self.context.i8_type().into(),
                        ],
                        Some(self.context.i8_type().into()),
                        &[
                            collection_value.into(),
                            word.into(),
                            present.into(),
                            self.collection_compare_kind(payload_ty)?.into(),
                        ],
                    )?
                    .ok_or_else(|| backend_failure("set insertion produced no result"))?
                    .into_int_value();
                self.drop_value_unless(inserted, value, definition.value)?;
            } else if op == mir::CollectionMutationOp::InsertAt {
                let _ = self.call_runtime(
                    COLLECTION_INSERT_AT_NULLABLE,
                    &[
                        pointer.into(),
                        pointer.into(),
                        self.context
                            .ptr_sized_int_type(self.target_data, None)
                            .into(),
                        self.context.i8_type().into(),
                        self.context.i64_type().into(),
                    ],
                    None,
                    &[
                        self.current_frame.into(),
                        collection_value.into(),
                        index.expect("insertAt index was lowered").into(),
                        present.into(),
                        word.into(),
                    ],
                )?;
            } else {
                let name = if op == mir::CollectionMutationOp::PushFront {
                    COLLECTION_PUSH_FRONT_NULLABLE
                } else {
                    COLLECTION_PUSH_NULLABLE
                };
                let _ = self.call_runtime(
                    name,
                    &[
                        pointer.into(),
                        self.context.i8_type().into(),
                        self.context.i64_type().into(),
                    ],
                    None,
                    &[collection_value.into(), present.into(), word.into()],
                )?;
            }
            return Ok(());
        }
        let value = self.lower_rvalue(value)?;
        let word = self.value_to_collection_word(value, definition.value)?;
        if op == mir::CollectionMutationOp::InsertAt {
            let _ = self.call_runtime(
                COLLECTION_INSERT_AT,
                &[
                    pointer.into(),
                    pointer.into(),
                    self.context
                        .ptr_sized_int_type(self.target_data, None)
                        .into(),
                    self.context.i64_type().into(),
                ],
                None,
                &[
                    self.current_frame.into(),
                    collection_value.into(),
                    index.expect("insertAt index was lowered").into(),
                    word.into(),
                ],
            )?;
        } else if op == mir::CollectionMutationOp::Remove {
            let removed_slot = self.entry_alloca(self.context.i64_type(), "set.removed.value")?;
            let removed = self
                .call_runtime(
                    COLLECTION_REMOVE_VALUE,
                    &[
                        pointer.into(),
                        self.context.i64_type().into(),
                        self.context.i8_type().into(),
                        self.context.i8_type().into(),
                        pointer.into(),
                        pointer.into(),
                    ],
                    Some(self.context.i8_type().into()),
                    &[
                        collection_value.into(),
                        word.into(),
                        self.context.i8_type().const_int(1, false).into(),
                        self.collection_compare_kind(definition.value)?.into(),
                        removed_slot.into(),
                        self.entry_alloca(self.context.i8_type(), "collection.removed.present")?
                            .into(),
                    ],
                )?
                .ok_or_else(|| backend_failure("set removal produced no result"))?
                .into_int_value();
            let removed_word = build(self.builder.build_load(
                self.context.i64_type(),
                removed_slot,
                "set.removed.word",
            ))?
            .into_int_value();
            let removed_value = self.collection_word_to_value(removed_word, definition.value)?;
            self.drop_value_if(removed, removed_value, definition.value)?;
            self.drop_stored_value(value, definition.value)?;
        } else if matches!(
            definition.kind,
            mir::CollectionKind::Set | mir::CollectionKind::SortedSet
        ) {
            let inserted = self
                .call_runtime(
                    COLLECTION_PUSH_UNIQUE,
                    &[
                        pointer.into(),
                        self.context.i64_type().into(),
                        self.context.i8_type().into(),
                        self.context.i8_type().into(),
                    ],
                    Some(self.context.i8_type().into()),
                    &[
                        collection_value.into(),
                        word.into(),
                        self.context.i8_type().const_int(1, false).into(),
                        self.collection_compare_kind(definition.value)?.into(),
                    ],
                )?
                .ok_or_else(|| backend_failure("set insertion produced no result"))?
                .into_int_value();
            self.drop_value_unless(inserted, value, definition.value)?;
        } else if op == mir::CollectionMutationOp::PushFront {
            let _ = self.call_runtime(
                COLLECTION_PUSH_FRONT,
                &[pointer.into(), self.context.i64_type().into()],
                None,
                &[collection_value.into(), word.into()],
            )?;
        } else {
            let _ = self.call_runtime(
                COLLECTION_PUSH,
                &[pointer.into(), self.context.i64_type().into()],
                None,
                &[collection_value.into(), word.into()],
            )?;
        }
        Ok(())
    }

    fn lower_collection_set(
        &mut self,
        collection: mir::LocalId,
        index: &mir::Rvalue,
        value: &mir::Rvalue,
        positional: bool,
    ) -> Result<(), BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let usize_type = self.context.ptr_sized_int_type(self.target_data, None);
        let local = local_in(self.function, collection)?;
        let mir::Type::Collection(collection_type) = local.ty else {
            return Err(malformed_mir("collection write uses non-collection local"));
        };
        let definition = self.collection_definition(collection_type)?.clone();
        let collection_value = self.collection_pointer(collection)?;
        let index = self.lower_rvalue(index)?;
        if matches!(
            definition.value,
            mir::Type::Error
                | mir::Type::NullableError
                | mir::Type::Function(_)
                | mir::Type::NullableFunction(_)
        ) {
            let replacement = self.lower_rvalue(value)?;
            let destination = if let Some(key_type) = definition.key.filter(|_| !positional) {
                self.lower_error_dictionary_write_slot(
                    collection_value,
                    index,
                    key_type,
                    definition.value,
                )?
            } else {
                let index = self.value_to_collection_word(
                    index,
                    mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64)),
                )?;
                let destination = self
                    .call_runtime(
                        COLLECTION_AGGREGATE_VALUE_AT,
                        &[
                            pointer.into(),
                            pointer.into(),
                            self.context.i64_type().into(),
                            self.context.i8_type().into(),
                            self.context.i8_type().into(),
                        ],
                        Some(pointer.into()),
                        &[
                            self.current_frame.into(),
                            collection_value.into(),
                            index.into(),
                            self.context.i8_type().const_int(1, false).into(),
                            self.context
                                .i8_type()
                                .const_int(u64::from(COLLECTION_COMPARE_WORD), false)
                                .into(),
                        ],
                    )?
                    .ok_or_else(|| backend_failure("Error collection write produced no slot"))?
                    .into_pointer_value();
                let carrier = if matches!(
                    definition.value,
                    mir::Type::Error | mir::Type::NullableError
                ) {
                    error_carrier_type(self.context)
                } else {
                    closure_carrier_type(self.context)
                };
                let old = build(self.builder.build_load(
                    carrier,
                    destination,
                    "aggregate.collection.old",
                ))?
                .into_struct_value();
                if matches!(
                    definition.value,
                    mir::Type::Error | mir::Type::NullableError
                ) {
                    self.drop_error_value(old)?;
                } else {
                    self.drop_function_carrier(old)?;
                }
                destination
            };
            self.store_value_at_address(destination, replacement, definition.value)?;
            return Ok(());
        }
        if let Some((ty, nullable)) = Self::payload_enum_storage(definition.value) {
            let replacement = self.lower_rvalue(value)?.into_pointer_value();
            let destination = if let Some(key_type) = definition.key.filter(|_| !positional) {
                self.lower_aggregate_dictionary_write_slot(
                    collection_value,
                    index,
                    key_type,
                    ty,
                    nullable,
                )?
            } else {
                let index = self.value_to_collection_word(
                    index,
                    mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64)),
                )?;
                let destination = self
                    .call_runtime(
                        COLLECTION_AGGREGATE_VALUE_AT,
                        &[
                            pointer.into(),
                            pointer.into(),
                            self.context.i64_type().into(),
                            self.context.i8_type().into(),
                            self.context.i8_type().into(),
                        ],
                        Some(pointer.into()),
                        &[
                            self.current_frame.into(),
                            collection_value.into(),
                            index.into(),
                            self.context.i8_type().const_int(1, false).into(),
                            self.context
                                .i8_type()
                                .const_int(u64::from(COLLECTION_COMPARE_WORD), false)
                                .into(),
                        ],
                    )?
                    .ok_or_else(|| backend_failure("aggregate collection write produced no slot"))?
                    .into_pointer_value();
                self.drop_payload_enum_at(destination, ty, nullable)?;
                destination
            };
            self.copy_payload_bytes(destination, replacement, ty, nullable)?;
            return Ok(());
        }
        if let Some(payload_ty) = nullable_payload_type(definition.value) {
            let (present, value, actual_payload_ty) =
                self.lower_nullable_collection_parts(value, definition.value)?;
            debug_assert_eq!(payload_ty, actual_payload_ty);
            if let Some(key_type) = definition.key.filter(|_| !positional) {
                self.lower_dictionary_set_nullable_value(
                    collection_value,
                    index,
                    key_type,
                    present,
                    value,
                    payload_ty,
                )?;
            } else {
                let previous_present_slot =
                    self.entry_alloca(self.context.i8_type(), "collection.previous.present")?;
                let value_word = self.value_to_collection_word(value, payload_ty)?;
                let old_word = self
                    .call_runtime(
                        COLLECTION_SET_AT_NULLABLE,
                        &[
                            pointer.into(),
                            pointer.into(),
                            usize_type.into(),
                            self.context.i8_type().into(),
                            self.context.i64_type().into(),
                            pointer.into(),
                        ],
                        Some(self.context.i64_type().into()),
                        &[
                            self.current_frame.into(),
                            collection_value.into(),
                            index.into_int_value().into(),
                            present.into(),
                            value_word.into(),
                            previous_present_slot.into(),
                        ],
                    )?
                    .ok_or_else(|| backend_failure("nullable collection write produced no result"))?
                    .into_int_value();
                let previous_present = build(self.builder.build_load(
                    self.context.i8_type(),
                    previous_present_slot,
                    "collection.previous.present.value",
                ))?
                .into_int_value();
                let old_value = self.collection_word_to_value(old_word, payload_ty)?;
                self.drop_value_if(previous_present, old_value, payload_ty)?;
            }
            return Ok(());
        }
        let value = self.lower_rvalue(value)?;
        if definition.kind == mir::CollectionKind::Bytes {
            let _ = self.call_runtime(
                BYTES_SET,
                &[
                    pointer.into(),
                    pointer.into(),
                    usize_type.into(),
                    self.context.i8_type().into(),
                ],
                None,
                &[
                    self.current_frame.into(),
                    collection_value.into(),
                    index.into_int_value().into(),
                    value.into_int_value().into(),
                ],
            )?;
            return Ok(());
        }
        if let Some(key_type) = definition.key.filter(|_| !positional) {
            self.lower_dictionary_set_value(
                collection_value,
                index,
                key_type,
                value,
                definition.value,
            )?;
        } else if matches!(definition.value, mir::Type::Scalar(_))
            && stage26_collection_kind(definition.kind).is_none()
        {
            self.lower_positional_scalar_store(
                collection_value,
                index.into_int_value(),
                value,
                definition.value,
            )?;
        } else {
            let value_word = self.value_to_collection_word(value, definition.value)?;
            let old_word = self
                .call_runtime(
                    COLLECTION_SET_AT,
                    &[
                        pointer.into(),
                        pointer.into(),
                        usize_type.into(),
                        self.context.i64_type().into(),
                    ],
                    Some(self.context.i64_type().into()),
                    &[
                        self.current_frame.into(),
                        collection_value.into(),
                        index.into_int_value().into(),
                        value_word.into(),
                    ],
                )?
                .ok_or_else(|| backend_failure("collection write produced no result"))?
                .into_int_value();
            let old_value = self.collection_word_to_value(old_word, definition.value)?;
            self.drop_stored_value(old_value, definition.value)?;
        }
        Ok(())
    }

    /// Stores a scalar at a position without calling the runtime.
    ///
    /// `dr_v2_collection_set_at` exists to do three things a general element
    /// write needs: drop the value being replaced, dispatch on the element
    /// width recorded in the header, and invalidate any membership index. A
    /// scalar has no drop, and the width is a static property of the element
    /// type here, so the first two are dead. Only the index has to be dealt
    /// with, and it is null unless something asked this collection about
    /// membership — so the write becomes a bounds check and a store, matching
    /// the read path in `lower_collection_index`.
    ///
    /// The indexed case still goes through the runtime rather than open-coding
    /// the discard, which keeps the one path that has to stay in step with the
    /// index's internals inside the runtime that owns them. It runs at most
    /// once per index, because the call leaves the index null.
    ///
    /// Stage 26 kinds are excluded: `Deque` addresses elements through `head`,
    /// which `checked_collection_value_address` does not apply.
    fn lower_positional_scalar_store(
        &mut self,
        collection: PointerValue<'ctx>,
        index: IntValue<'ctx>,
        value: BasicValueEnum<'ctx>,
        value_type: mir::Type,
    ) -> Result<(), BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let usize_type = self.context.ptr_sized_int_type(self.target_data, None);
        let header = collection_header_type(self.context, self.target_data);
        let function = current_function(&self.builder)?;
        let indexed_block = self
            .context
            .append_basic_block(function, "collection.store.indexed");
        let direct_block = self
            .context
            .append_basic_block(function, "collection.store.direct");
        let done_block = self
            .context
            .append_basic_block(function, "collection.store.done");

        let index_address = build(self.builder.build_struct_gep(
            header,
            collection,
            COLLECTION_INDEX_FIELD,
            "collection.index.address",
        ))?;
        let membership_index = self
            .load_collection_memory(
                pointer,
                index_address,
                "collection.membership.index",
                CollectionMemoryRegion::Header,
            )?
            .into_pointer_value();
        let indexed = build(
            self.builder
                .build_is_not_null(membership_index, "collection.indexed"),
        )?;
        build(
            self.builder
                .build_conditional_branch(indexed, indexed_block, direct_block),
        )?;

        self.builder.position_at_end(indexed_block);
        let value_word = self.value_to_collection_word(value, value_type)?;
        let _ = self.call_runtime(
            COLLECTION_SET_AT,
            &[
                pointer.into(),
                pointer.into(),
                usize_type.into(),
                self.context.i64_type().into(),
            ],
            Some(self.context.i64_type().into()),
            &[
                self.current_frame.into(),
                collection.into(),
                index.into(),
                value_word.into(),
            ],
        )?;
        build(self.builder.build_unconditional_branch(done_block))?;

        self.builder.position_at_end(direct_block);
        let address = self.checked_collection_value_address(collection, index, value_type)?;
        self.store_collection_memory(address, value, CollectionMemoryRegion::Values)?;
        build(self.builder.build_unconditional_branch(done_block))?;

        self.builder.position_at_end(done_block);
        Ok(())
    }

    fn lower_dictionary_set_value(
        &mut self,
        collection: PointerValue<'ctx>,
        key: BasicValueEnum<'ctx>,
        key_type: mir::Type,
        value: BasicValueEnum<'ctx>,
        value_type: mir::Type,
    ) -> Result<(), BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let key_word = self.value_to_collection_word(key, key_type)?;
        let value_word = self.value_to_collection_word(value, value_type)?;
        let replaced_slot = self.entry_alloca(self.context.i8_type(), "dictionary.replaced")?;
        let old_word = self
            .call_runtime(
                COLLECTION_KEYED_SET,
                &[
                    pointer.into(),
                    self.context.i64_type().into(),
                    self.context.i64_type().into(),
                    self.context.i8_type().into(),
                    pointer.into(),
                ],
                Some(self.context.i64_type().into()),
                &[
                    collection.into(),
                    key_word.into(),
                    value_word.into(),
                    self.collection_compare_kind(key_type)?.into(),
                    replaced_slot.into(),
                ],
            )?
            .ok_or_else(|| backend_failure("dictionary write produced no result"))?
            .into_int_value();
        let replaced = build(self.builder.build_load(
            self.context.i8_type(),
            replaced_slot,
            "dictionary.replaced",
        ))?
        .into_int_value();
        let old_value = self.collection_word_to_value(old_word, value_type)?;
        self.drop_value_if(replaced, old_value, value_type)?;
        self.drop_value_if(replaced, key, key_type)
    }

    #[allow(clippy::too_many_arguments)]
    fn lower_dictionary_set_nullable_value(
        &mut self,
        collection: PointerValue<'ctx>,
        key: BasicValueEnum<'ctx>,
        key_type: mir::Type,
        present: IntValue<'ctx>,
        value: BasicValueEnum<'ctx>,
        payload_type: mir::Type,
    ) -> Result<(), BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let key_word = self.value_to_collection_word(key, key_type)?;
        let value_word = self.value_to_collection_word(value, payload_type)?;
        let replaced_slot = self.entry_alloca(self.context.i8_type(), "dictionary.replaced")?;
        let previous_present_slot =
            self.entry_alloca(self.context.i8_type(), "dictionary.previous.present")?;
        let old_word = self
            .call_runtime(
                COLLECTION_KEYED_SET_NULLABLE,
                &[
                    pointer.into(),
                    self.context.i64_type().into(),
                    self.context.i64_type().into(),
                    self.context.i8_type().into(),
                    self.context.i8_type().into(),
                    pointer.into(),
                    pointer.into(),
                ],
                Some(self.context.i64_type().into()),
                &[
                    collection.into(),
                    key_word.into(),
                    value_word.into(),
                    present.into(),
                    self.collection_compare_kind(key_type)?.into(),
                    replaced_slot.into(),
                    previous_present_slot.into(),
                ],
            )?
            .ok_or_else(|| backend_failure("nullable dictionary write produced no result"))?
            .into_int_value();
        let replaced = build(self.builder.build_load(
            self.context.i8_type(),
            replaced_slot,
            "dictionary.replaced.value",
        ))?
        .into_int_value();
        let previous_present = build(self.builder.build_load(
            self.context.i8_type(),
            previous_present_slot,
            "dictionary.previous.present.value",
        ))?
        .into_int_value();
        let drop_previous = build(self.builder.build_and(
            replaced,
            previous_present,
            "dictionary.drop.previous",
        ))?;
        let old_value = self.collection_word_to_value(old_word, payload_type)?;
        self.drop_value_if(drop_previous, old_value, payload_type)?;
        self.drop_value_if(replaced, key, key_type)
    }

    fn lower_set_from(
        &mut self,
        target: mir::CollectionTypeId,
        source: mir::LocalId,
        transfer: bool,
        algebra: Option<(mir::SetAlgebraOp, mir::LocalId)>,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let usize_type = self.context.ptr_sized_int_type(self.target_data, None);
        let target_definition = self.collection_definition(target)?.clone();
        let source_local = local_in(self.function, source)?;
        let mir::Type::Collection(source_type) = source_local.ty else {
            return Err(malformed_mir("Set::from source is not a collection"));
        };
        let source_definition = self.collection_definition(source_type)?.clone();
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
        let source_value = self.collection_pointer(source)?;
        if let Some((op, right)) = algebra {
            let right = self.collection_pointer(right)?;
            let operation = self.context.i8_type().const_int(
                match op {
                    mir::SetAlgebraOp::Union => 0,
                    mir::SetAlgebraOp::Intersect => 1,
                    mir::SetAlgebraOp::Difference => 2,
                },
                false,
            );
            return Ok(self
                .call_runtime(
                    COLLECTION_SET_ALGEBRA,
                    &[
                        pointer.into(),
                        pointer.into(),
                        self.context.i8_type().into(),
                        self.context.i8_type().into(),
                    ],
                    Some(pointer.into()),
                    &[
                        source_value.into(),
                        right.into(),
                        operation.into(),
                        self.collection_compare_kind(target_definition.value)?
                            .into(),
                    ],
                )?
                .ok_or_else(|| backend_failure("set algebra produced no result"))?
                .into_pointer_value());
        }
        if let Some((ty, nullable)) = Self::payload_enum_storage(target_definition.value) {
            return self.lower_payload_enum_collection_from(
                &target_definition,
                source_value,
                ty,
                nullable,
            );
        }
        if let Some(kind) = stage26_collection_kind(target_definition.kind) {
            if transfer {
                return Err(malformed_mir(
                    "Stage 26 collection construction must preserve its source",
                ));
            }
            let comparator = target_definition
                .comparator
                .map(collection_comparator_code)
                .unwrap_or(COLLECTION_COMPARE_WORD);
            let key_kind = target_definition
                .key
                .map(|key| self.collection_compare_kind(key))
                .transpose()?
                .map(|value| value.get_zero_extended_constant().unwrap_or(0) as u8)
                .unwrap_or(COLLECTION_COMPARE_WORD);
            let value_kind = self
                .collection_compare_kind(
                    nullable_payload_type(target_definition.value)
                        .unwrap_or(target_definition.value),
                )?
                .get_zero_extended_constant()
                .unwrap_or(0) as u8;
            return Ok(self
                .call_runtime(
                    COLLECTION_STAGE26_FROM_COPY,
                    &[
                        pointer.into(),
                        self.context.i8_type().into(),
                        self.context.i8_type().into(),
                        self.context.i8_type().into(),
                        self.context.i8_type().into(),
                        self.context.i8_type().into(),
                        self.context.i8_type().into(),
                    ],
                    Some(pointer.into()),
                    &[
                        source_value.into(),
                        self.context
                            .i8_type()
                            .const_int(u64::from(kind), false)
                            .into(),
                        self.context
                            .i8_type()
                            .const_int(u64::from(comparator), false)
                            .into(),
                        self.context
                            .i8_type()
                            .const_int(u64::from(target_definition.key.is_some()), false)
                            .into(),
                        self.context
                            .i8_type()
                            .const_int(
                                u64::from(
                                    collection_value_width(
                                        target_definition.value,
                                        self.target_data.get_pointer_byte_size(None) as u8,
                                    )
                                    .ok_or_else(|| {
                                        malformed_mir("collection value has no runtime width")
                                    })?,
                                ),
                                false,
                            )
                            .into(),
                        self.context
                            .i8_type()
                            .const_int(u64::from(key_kind), false)
                            .into(),
                        self.context
                            .i8_type()
                            .const_int(u64::from(value_kind), false)
                            .into(),
                    ],
                )?
                .ok_or_else(|| {
                    backend_failure("Stage 26 collection conversion produced no result")
                })?
                .into_pointer_value());
        }
        if transfer {
            build(
                self.builder
                    .build_store(local_slot(&self.local_slots, source)?, pointer.const_null()),
            )?;
        }
        let result = self
            .call_runtime(
                COLLECTION_NEW,
                &[
                    usize_type.into(),
                    self.context.i8_type().into(),
                    self.context.i8_type().into(),
                    self.context.i8_type().into(),
                ],
                Some(pointer.into()),
                &[
                    usize_type.const_zero().into(),
                    self.context.i8_type().const_zero().into(),
                    self.context.i8_type().const_zero().into(),
                    self.context
                        .i8_type()
                        .const_int(
                            u64::from(
                                collection_value_width(
                                    target_definition.value,
                                    self.target_data.get_pointer_byte_size(None) as u8,
                                )
                                .ok_or_else(|| {
                                    malformed_mir(
                                        "nullable collection elements are not supported by Stage 23 Slice 3",
                                    )
                                })?,
                            ),
                            false,
                        )
                        .into(),
                ],
            )?
            .ok_or_else(|| backend_failure("set allocation produced no result"))?
            .into_pointer_value();
        let length = self
            .call_runtime(
                COLLECTION_LENGTH,
                &[pointer.into()],
                Some(usize_type.into()),
                &[source_value.into()],
            )?
            .ok_or_else(|| backend_failure("collection length produced no result"))?
            .into_int_value();
        let index_slot = self.entry_alloca(usize_type, "set.from.index")?;
        build(
            self.builder
                .build_store(index_slot, usize_type.const_zero()),
        )?;
        let function = current_function(&self.builder)?;
        let header = self.context.append_basic_block(function, "set.from.header");
        let body = self.context.append_basic_block(function, "set.from.body");
        let done = self.context.append_basic_block(function, "set.from.done");
        build(self.builder.build_unconditional_branch(header))?;
        self.builder.position_at_end(header);
        let index = build(
            self.builder
                .build_load(usize_type, index_slot, "set.from.index"),
        )?
        .into_int_value();
        let more = build(self.builder.build_int_compare(
            IntPredicate::ULT,
            index,
            length,
            "set.from.more",
        ))?;
        build(self.builder.build_conditional_branch(more, body, done))?;
        self.builder.position_at_end(body);
        let word = self
            .call_runtime(
                COLLECTION_VALUE_AT,
                &[pointer.into(), pointer.into(), usize_type.into()],
                Some(self.context.i64_type().into()),
                &[self.current_frame.into(), source_value.into(), index.into()],
            )?
            .ok_or_else(|| backend_failure("Set::from element read produced no result"))?
            .into_int_value();
        let mut value = self.collection_word_to_value(word, source_definition.value)?;
        if !transfer && source_definition.value == mir::Type::String {
            value = self.retain_string(value.into_pointer_value())?.into();
        }
        let word = self.value_to_collection_word(value, source_definition.value)?;
        let inserted = self
            .call_runtime(
                COLLECTION_PUSH_UNIQUE,
                &[
                    pointer.into(),
                    self.context.i64_type().into(),
                    self.context.i8_type().into(),
                    self.context.i8_type().into(),
                ],
                Some(self.context.i8_type().into()),
                &[
                    result.into(),
                    word.into(),
                    self.context.i8_type().const_int(1, false).into(),
                    self.collection_compare_kind(source_definition.value)?
                        .into(),
                ],
            )?
            .ok_or_else(|| backend_failure("Set::from insertion produced no result"))?
            .into_int_value();
        self.drop_value_unless(inserted, value, source_definition.value)?;
        let next = build(self.builder.build_int_add(
            index,
            usize_type.const_int(1, false),
            "set.from.next",
        ))?;
        build(self.builder.build_store(index_slot, next))?;
        build(self.builder.build_unconditional_branch(header))?;
        self.builder.position_at_end(done);
        if transfer {
            let _ = self.call_runtime(
                COLLECTION_FREE,
                &[pointer.into()],
                None,
                &[source_value.into()],
            )?;
        }
        Ok(result)
    }

    fn lower_payload_enum_collection_from(
        &mut self,
        target: &mir::CollectionType,
        source: PointerValue<'ctx>,
        ty: mir::PayloadEnumType,
        nullable: bool,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let usize_type = self.context.ptr_sized_int_type(self.target_data, None);
        let byte = self.context.i8_type();
        let kind = stage26_collection_kind(target.kind).ok_or_else(|| {
            malformed_mir("aggregate collection conversion targets a legacy kind")
        })?;
        let length = self
            .call_runtime(
                COLLECTION_LENGTH,
                &[pointer.into()],
                Some(usize_type.into()),
                &[source.into()],
            )?
            .ok_or_else(|| backend_failure("aggregate collection length produced no result"))?
            .into_int_value();
        let result = self
            .call_runtime(
                COLLECTION_AGGREGATE_NEW,
                &[
                    pointer.into(),
                    usize_type.into(),
                    byte.into(),
                    byte.into(),
                    usize_type.into(),
                    usize_type.into(),
                    byte.into(),
                    byte.into(),
                ],
                Some(pointer.into()),
                &[
                    self.current_frame.into(),
                    length.into(),
                    byte.const_int(u64::from(target.key.is_some()), false)
                        .into(),
                    byte.const_zero().into(),
                    usize_type
                        .const_int(u64::from(ty.storage_size(nullable)), false)
                        .into(),
                    usize_type.const_int(u64::from(ty.align), false).into(),
                    byte.const_int(u64::from(kind), false).into(),
                    byte.const_int(
                        u64::from(
                            target
                                .comparator
                                .map(collection_comparator_code)
                                .unwrap_or(COLLECTION_COMPARE_WORD),
                        ),
                        false,
                    )
                    .into(),
                ],
            )?
            .ok_or_else(|| backend_failure("aggregate collection allocation produced no result"))?
            .into_pointer_value();
        let index_slot = self.entry_alloca(usize_type, "aggregate.from.index")?;
        build(
            self.builder
                .build_store(index_slot, usize_type.const_zero()),
        )?;
        let function = current_function(&self.builder)?;
        let header = self
            .context
            .append_basic_block(function, "aggregate.from.header");
        let body = self
            .context
            .append_basic_block(function, "aggregate.from.body");
        let done = self
            .context
            .append_basic_block(function, "aggregate.from.done");
        build(self.builder.build_unconditional_branch(header))?;
        self.builder.position_at_end(header);
        let index = build(self.builder.build_load(
            usize_type,
            index_slot,
            "aggregate.from.index.value",
        ))?
        .into_int_value();
        let more = build(self.builder.build_int_compare(
            IntPredicate::ULT,
            index,
            length,
            "aggregate.from.more",
        ))?;
        build(self.builder.build_conditional_branch(more, body, done))?;
        self.builder.position_at_end(body);
        let index_word = if usize_type.get_bit_width() == 64 {
            index
        } else {
            build(self.builder.build_int_z_extend(
                index,
                self.context.i64_type(),
                "aggregate.from.index.i64",
            ))?
        };
        let source_slot = self
            .call_runtime(
                COLLECTION_AGGREGATE_VALUE_AT,
                &[
                    pointer.into(),
                    pointer.into(),
                    self.context.i64_type().into(),
                    byte.into(),
                    byte.into(),
                ],
                Some(pointer.into()),
                &[
                    self.current_frame.into(),
                    source.into(),
                    index_word.into(),
                    byte.const_int(1, false).into(),
                    byte.const_int(u64::from(COLLECTION_COMPARE_WORD), false)
                        .into(),
                ],
            )?
            .ok_or_else(|| backend_failure("aggregate collection read produced no slot"))?
            .into_pointer_value();
        let destination = if let Some(key_ty) = target.key {
            let key_word = self
                .call_runtime(
                    COLLECTION_KEY_AT,
                    &[pointer.into(), pointer.into(), usize_type.into()],
                    Some(self.context.i64_type().into()),
                    &[self.current_frame.into(), source.into(), index.into()],
                )?
                .ok_or_else(|| backend_failure("aggregate collection key read produced no result"))?
                .into_int_value();
            let key = if key_ty == mir::Type::String {
                let key_pointer = build(self.builder.build_int_to_ptr(
                    key_word,
                    pointer,
                    "aggregate.from.string-key",
                ))?;
                self.retain_string(key_pointer)?;
                key_pointer.into()
            } else {
                key_word.into()
            };
            self.lower_aggregate_dictionary_write_slot(result, key, key_ty, ty, nullable)?
        } else {
            self.call_runtime(
                COLLECTION_AGGREGATE_PUSH_SLOT,
                &[pointer.into()],
                Some(pointer.into()),
                &[result.into()],
            )?
            .ok_or_else(|| backend_failure("aggregate collection insertion produced no slot"))?
            .into_pointer_value()
        };
        self.copy_payload_bytes(destination, source_slot, ty, nullable)?;
        self.retain_payload_enum_at(destination, ty, nullable)?;
        let next = build(self.builder.build_int_add(
            index,
            usize_type.const_int(1, false),
            "aggregate.from.next",
        ))?;
        build(self.builder.build_store(index_slot, next))?;
        build(self.builder.build_unconditional_branch(header))?;
        self.builder.position_at_end(done);
        let _ = self.call_runtime(
            COLLECTION_STAGE26_FINALIZE,
            &[pointer.into()],
            None,
            &[result.into()],
        )?;
        Ok(result)
    }

    fn drop_value_if(
        &mut self,
        condition: IntValue<'ctx>,
        value: BasicValueEnum<'ctx>,
        ty: mir::Type,
    ) -> Result<(), BackendError> {
        if !matches!(
            ty,
            mir::Type::Error
                | mir::Type::NullableError
                | mir::Type::String
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
        let condition = build(self.builder.build_int_compare(
            IntPredicate::NE,
            condition,
            condition.get_type().const_zero(),
            "collection.should_drop",
        ))?;
        let function = current_function(&self.builder)?;
        let drop_block = self
            .context
            .append_basic_block(function, "collection.drop.value");
        let done = self
            .context
            .append_basic_block(function, "collection.drop.value.done");
        build(
            self.builder
                .build_conditional_branch(condition, drop_block, done),
        )?;
        self.builder.position_at_end(drop_block);
        self.drop_stored_value(value, ty)?;
        build(self.builder.build_unconditional_branch(done))?;
        self.builder.position_at_end(done);
        Ok(())
    }

    fn drop_value_unless(
        &mut self,
        condition: IntValue<'ctx>,
        value: BasicValueEnum<'ctx>,
        ty: mir::Type,
    ) -> Result<(), BackendError> {
        let should_drop = build(self.builder.build_int_compare(
            IntPredicate::EQ,
            condition,
            condition.get_type().const_zero(),
            "collection.not_inserted",
        ))?;
        self.drop_value_if(
            build(self.builder.build_int_z_extend(
                should_drop,
                self.context.i8_type(),
                "collection.not_inserted.i8",
            ))?,
            value,
            ty,
        )
    }

    fn drop_stored_value(
        &mut self,
        value: BasicValueEnum<'ctx>,
        ty: mir::Type,
    ) -> Result<(), BackendError> {
        match ty {
            mir::Type::Error | mir::Type::NullableError => {
                self.drop_error_value(value.into_struct_value())
            }
            mir::Type::String => self.release_string(value.into_pointer_value()),
            mir::Type::Class(class) => {
                self.drop_class_value_checked(value.into_pointer_value(), class)
            }
            mir::Type::Collection(collection) | mir::Type::NullableCollection(collection) => {
                self.drop_collection_value(value.into_pointer_value(), collection)
            }
            mir::Type::Mixed | mir::Type::NullableMixed => {
                self.drop_mixed_value(value.into_pointer_value())
            }
            mir::Type::SharedReference(_) | mir::Type::NullableSharedReference(_) => {
                self.drop_shared_value(value.into_pointer_value(), false)
            }
            mir::Type::WeakReference(_) | mir::Type::NullableWeakReference(_) => {
                self.drop_shared_value(value.into_pointer_value(), true)
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
                self.drop_writable_shared_value(value.into_pointer_value(), symbol)
            }
            mir::Type::NullableString => self.release_string(value.into_pointer_value()),
            mir::Type::NullableClass(class) => {
                self.drop_class_value_checked(value.into_pointer_value(), class)
            }
            mir::Type::PayloadEnum(payload) => {
                self.drop_payload_enum_at(value.into_pointer_value(), payload, false)
            }
            mir::Type::NullablePayloadEnum(payload) => {
                self.drop_payload_enum_at(value.into_pointer_value(), payload, true)
            }
            mir::Type::Scalar(_) | mir::Type::NullableScalar(_) => Ok(()),
            mir::Type::Function(_) | mir::Type::NullableFunction(_) => {
                self.drop_function_carrier(value.into_struct_value())
            }
            mir::Type::ClosureEnvironment(_) => Err(malformed_mir(
                "closure environment pointer reached ordinary value cleanup",
            )),
        }
    }

    fn drop_collection_value(
        &mut self,
        collection: PointerValue<'ctx>,
        collection_type: mir::CollectionTypeId,
    ) -> Result<(), BackendError> {
        self.finish_collection_value(collection, collection_type, CollectionStorageAction::Free)
    }

    fn clear_collection_value(
        &mut self,
        collection: PointerValue<'ctx>,
        collection_type: mir::CollectionTypeId,
    ) -> Result<(), BackendError> {
        self.finish_collection_value(collection, collection_type, CollectionStorageAction::Reset)
    }

    fn finish_collection_value(
        &mut self,
        collection: PointerValue<'ctx>,
        collection_type: mir::CollectionTypeId,
        action: CollectionStorageAction,
    ) -> Result<(), BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let usize_type = self.context.ptr_sized_int_type(self.target_data, None);
        let definition = self.collection_definition(collection_type)?.clone();
        let function = current_function(&self.builder)?;
        let drop_block = self.context.append_basic_block(function, "collection.drop");
        let done = self
            .context
            .append_basic_block(function, "collection.drop.done");
        let present = build(
            self.builder
                .build_is_not_null(collection, "collection.present"),
        )?;
        build(
            self.builder
                .build_conditional_branch(present, drop_block, done),
        )?;
        self.builder.position_at_end(drop_block);
        if definition.kind == mir::CollectionKind::Bytes {
            if action != CollectionStorageAction::Free {
                return Err(malformed_mir(
                    "Bytes cannot be cleared as a named collection",
                ));
            }
            let _ = self.call_runtime(BYTES_FREE, &[pointer.into()], None, &[collection.into()])?;
            build(self.builder.build_unconditional_branch(done))?;
            self.builder.position_at_end(done);
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
            let _ = self.call_runtime(symbol, &[pointer.into()], None, &[collection.into()])?;
            build(self.builder.build_unconditional_branch(done))?;
            self.builder.position_at_end(done);
            return Ok(());
        }
        let cleanup_collection = if action == CollectionStorageAction::Reset {
            let cleanup_collection = self.entry_alloca(
                collection_header_type(self.context, self.target_data),
                "collection.cleanup",
            )?;
            let _ = self.call_runtime(
                COLLECTION_DETACH_FOR_CLEANUP,
                &[pointer.into(), pointer.into(), pointer.into()],
                None,
                &[
                    self.current_frame.into(),
                    collection.into(),
                    cleanup_collection.into(),
                ],
            )?;
            cleanup_collection
        } else {
            collection
        };
        let length = self
            .call_runtime(
                COLLECTION_LENGTH,
                &[pointer.into()],
                Some(usize_type.into()),
                &[cleanup_collection.into()],
            )?
            .ok_or_else(|| backend_failure("collection length produced no result"))?
            .into_int_value();
        let index_slot = self.entry_alloca(usize_type, "collection.drop.index")?;
        build(self.builder.build_store(index_slot, length))?;
        let header = self
            .context
            .append_basic_block(function, "collection.drop.header");
        let body = self
            .context
            .append_basic_block(function, "collection.drop.body");
        let free = self
            .context
            .append_basic_block(function, "collection.drop.free");
        build(self.builder.build_unconditional_branch(header))?;
        self.builder.position_at_end(header);
        let index = build(self.builder.build_load(
            usize_type,
            index_slot,
            "collection.drop.index",
        ))?
        .into_int_value();
        let more = build(self.builder.build_int_compare(
            IntPredicate::NE,
            index,
            usize_type.const_zero(),
            "collection.drop.more",
        ))?;
        build(self.builder.build_conditional_branch(more, body, free))?;
        self.builder.position_at_end(body);
        let current = build(self.builder.build_int_sub(
            index,
            usize_type.const_int(1, false),
            "collection.drop.current",
        ))?;
        if matches!(
            definition.value,
            mir::Type::Error | mir::Type::NullableError
        ) {
            let index = if usize_type.get_bit_width() == 64 {
                current
            } else {
                build(self.builder.build_int_z_extend(
                    current,
                    self.context.i64_type(),
                    "error.collection.drop.index",
                ))?
            };
            let value = self
                .call_runtime(
                    COLLECTION_AGGREGATE_VALUE_AT,
                    &[
                        pointer.into(),
                        pointer.into(),
                        self.context.i64_type().into(),
                        self.context.i8_type().into(),
                        self.context.i8_type().into(),
                    ],
                    Some(pointer.into()),
                    &[
                        self.current_frame.into(),
                        cleanup_collection.into(),
                        index.into(),
                        self.context.i8_type().const_int(1, false).into(),
                        self.context
                            .i8_type()
                            .const_int(u64::from(COLLECTION_COMPARE_WORD), false)
                            .into(),
                    ],
                )?
                .ok_or_else(|| backend_failure("Error collection value read produced no slot"))?
                .into_pointer_value();
            let value = build(self.builder.build_load(
                error_carrier_type(self.context),
                value,
                "error.collection.drop.value",
            ))?
            .into_struct_value();
            self.drop_error_value(value)?;
        } else if matches!(
            definition.value,
            mir::Type::Function(_) | mir::Type::NullableFunction(_)
        ) {
            let index = if usize_type.get_bit_width() == 64 {
                current
            } else {
                build(self.builder.build_int_z_extend(
                    current,
                    self.context.i64_type(),
                    "function.collection.drop.index",
                ))?
            };
            let value = self
                .call_runtime(
                    COLLECTION_AGGREGATE_VALUE_AT,
                    &[
                        pointer.into(),
                        pointer.into(),
                        self.context.i64_type().into(),
                        self.context.i8_type().into(),
                        self.context.i8_type().into(),
                    ],
                    Some(pointer.into()),
                    &[
                        self.current_frame.into(),
                        cleanup_collection.into(),
                        index.into(),
                        self.context.i8_type().const_int(1, false).into(),
                        self.context
                            .i8_type()
                            .const_int(u64::from(COLLECTION_COMPARE_WORD), false)
                            .into(),
                    ],
                )?
                .ok_or_else(|| backend_failure("function collection value read produced no slot"))?
                .into_pointer_value();
            let value = build(self.builder.build_load(
                closure_carrier_type(self.context),
                value,
                "function.collection.drop.value",
            ))?
            .into_struct_value();
            self.drop_function_carrier(value)?;
        } else if let Some((ty, nullable)) = Self::payload_enum_storage(definition.value) {
            let index = if usize_type.get_bit_width() == 64 {
                current
            } else {
                build(self.builder.build_int_z_extend(
                    current,
                    self.context.i64_type(),
                    "aggregate.collection.drop.index",
                ))?
            };
            let value = self
                .call_runtime(
                    COLLECTION_AGGREGATE_VALUE_AT,
                    &[
                        pointer.into(),
                        pointer.into(),
                        self.context.i64_type().into(),
                        self.context.i8_type().into(),
                        self.context.i8_type().into(),
                    ],
                    Some(pointer.into()),
                    &[
                        self.current_frame.into(),
                        cleanup_collection.into(),
                        index.into(),
                        self.context.i8_type().const_int(1, false).into(),
                        self.context
                            .i8_type()
                            .const_int(u64::from(COLLECTION_COMPARE_WORD), false)
                            .into(),
                    ],
                )?
                .ok_or_else(|| backend_failure("aggregate collection value read produced no slot"))?
                .into_pointer_value();
            self.drop_payload_enum_at(value, ty, nullable)?;
        } else {
            let value_word = self
                .call_runtime(
                    COLLECTION_VALUE_AT,
                    &[pointer.into(), pointer.into(), usize_type.into()],
                    Some(self.context.i64_type().into()),
                    &[
                        self.current_frame.into(),
                        cleanup_collection.into(),
                        current.into(),
                    ],
                )?
                .ok_or_else(|| backend_failure("collection value read produced no result"))?
                .into_int_value();
            let stored_value_type =
                nullable_payload_type(definition.value).unwrap_or(definition.value);
            let value = self.collection_word_to_value(value_word, stored_value_type)?;
            self.drop_stored_value(value, stored_value_type)?;
        }
        if let Some(key_type) = definition.key {
            let key_word = self
                .call_runtime(
                    COLLECTION_KEY_AT,
                    &[pointer.into(), pointer.into(), usize_type.into()],
                    Some(self.context.i64_type().into()),
                    &[
                        self.current_frame.into(),
                        cleanup_collection.into(),
                        current.into(),
                    ],
                )?
                .ok_or_else(|| backend_failure("collection key read produced no result"))?
                .into_int_value();
            let key = self.collection_word_to_value(key_word, key_type)?;
            self.drop_stored_value(key, key_type)?;
        }
        build(self.builder.build_store(index_slot, current))?;
        build(self.builder.build_unconditional_branch(header))?;
        self.builder.position_at_end(free);
        match action {
            CollectionStorageAction::Free => {
                let _ = self.call_runtime(
                    COLLECTION_FREE,
                    &[pointer.into()],
                    None,
                    &[cleanup_collection.into()],
                )?;
            }
            CollectionStorageAction::Reset => {
                let _ = self.call_runtime(
                    COLLECTION_FINISH_DETACHED_CLEANUP,
                    &[pointer.into(), pointer.into()],
                    None,
                    &[collection.into(), cleanup_collection.into()],
                )?;
            }
        }
        build(self.builder.build_unconditional_branch(done))?;
        self.builder.position_at_end(done);
        Ok(())
    }

    fn lower_class_expression(
        &mut self,
        expression: &mir::ClassExpression,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        match expression {
            mir::ClassExpression::Local {
                local, transfer, ..
            } => {
                let slot = local_slot(&self.local_slots, *local)?;
                let value = build(self.builder.build_load(pointer, slot, "class.local"))?
                    .into_pointer_value();
                if *transfer {
                    build(self.builder.build_store(slot, pointer.const_null()))?;
                }
                Ok(value)
            }
            mir::ClassExpression::NullableLocalAssumeNonNull {
                local, transfer, ..
            } => {
                let slot = local_slot(&self.local_slots, *local)?;
                let value = build(self.builder.build_load(pointer, slot, "class.local"))?
                    .into_pointer_value();
                if *transfer {
                    build(self.builder.build_store(slot, pointer.const_null()))?;
                }
                Ok(value)
            }
            mir::ClassExpression::Property {
                object, property, ..
            } => Ok(build(self.builder.build_load(
                pointer,
                self.lower_property_address(*object, *property)?,
                "class.property",
            ))?
            .into_pointer_value()),
            mir::ClassExpression::SharedPayload { reference, .. } => {
                let owned = reference.owned_temporary().is_some();
                let control = self.lower_shared_reference_expression(reference)?;
                let payload = self
                    .call_runtime(
                        SHARED_PAYLOAD,
                        &[pointer.into()],
                        Some(pointer.into()),
                        &[control.into()],
                    )?
                    .ok_or_else(|| backend_failure("shared payload projection produced no result"))?
                    .into_pointer_value();
                if owned {
                    self.defer_or_drop_shared_temporary(control, false)?;
                }
                Ok(payload)
            }
            mir::ClassExpression::SharedAccessPayload {
                access, writable, ..
            } => self.lower_shared_access_payload(*access, *writable),
            mir::ClassExpression::Call { function, args, .. } => Ok(self
                .lower_call(*function, args, true)?
                .ok_or_else(|| malformed_mir("class call produced no result"))?
                .into_pointer_value()),
            mir::ClassExpression::New {
                class,
                properties,
                constructor,
                args,
            } => {
                let (object, lowered) = self.lower_class_allocation(*class, properties, args)?;
                if let Some(constructor) = constructor {
                    let callee = *self.functions.get(constructor.0).ok_or_else(|| {
                        malformed_mir(format!("function{} does not exist", constructor.0))
                    })?;
                    let mut constructor_args = Vec::<BasicMetadataValueEnum<'ctx>>::with_capacity(
                        lowered.values.len() + 2,
                    );
                    constructor_args.push(self.current_frame.into());
                    constructor_args.push(object.into());
                    constructor_args.extend(lowered.values.iter().copied());
                    let call = build(self.builder.build_call(
                        callee,
                        &constructor_args,
                        "constructor.call",
                    ))?;
                    apply_call_abi_attributes(
                        self.context,
                        call,
                        function_in(self.program, *constructor)?,
                    )?;
                    self.cleanup_constructor_arguments(*constructor, properties, args, &lowered)?;
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
                let left = self.lower_nullable_class_expression(left)?;
                let function = current_function(&self.builder)?;
                let some = self
                    .context
                    .append_basic_block(function, "class.coalesce.some");
                let none = self
                    .context
                    .append_basic_block(function, "class.coalesce.none");
                let done = self
                    .context
                    .append_basic_block(function, "class.coalesce.done");
                let present = build(
                    self.builder
                        .build_is_not_null(left, "class.coalesce.present"),
                )?;
                build(self.builder.build_conditional_branch(present, some, none))?;
                self.builder.position_at_end(some);
                build(self.builder.build_unconditional_branch(done))?;
                let some_end = self
                    .builder
                    .get_insert_block()
                    .expect("coalesce some block");
                self.builder.position_at_end(none);
                let right = self.lower_class_expression(right)?;
                build(self.builder.build_unconditional_branch(done))?;
                let none_end = self
                    .builder
                    .get_insert_block()
                    .expect("coalesce none block");
                self.builder.position_at_end(done);
                let phi = build(self.builder.build_phi(pointer, "class.coalesce"))?;
                phi.add_incoming(&[(&left, some_end), (&right, none_end)]);
                let result = phi.as_basic_value().into_pointer_value();
                if !transfer && (left_owned || right_owned) {
                    let temporary =
                        build(self.builder.build_phi(pointer, "class.coalesce.temporary"))?;
                    let null = pointer.const_null();
                    let left_temporary = if left_owned { left } else { null };
                    let right_temporary = if right_owned { right } else { null };
                    temporary
                        .add_incoming(&[(&left_temporary, some_end), (&right_temporary, none_end)]);
                    self.defer_or_drop_class_temporary(
                        temporary.as_basic_value().into_pointer_value(),
                        expression.class(),
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
            } => Ok(self
                .lower_collection_index(*collection, index, *transfer, *positional)?
                .into_pointer_value()),
            mir::ClassExpression::MixedPayload {
                mixed,
                class,
                transfer,
            } => {
                if *transfer {
                    return Ok(self
                        .lower_take_mixed_payload(*mixed, mir::MixedTag::Class(*class))?
                        .into_pointer_value());
                }
                Ok(self
                    .lower_mixed_payload(*mixed, mir::MixedTag::Class(*class))?
                    .into_pointer_value())
            }
        }
    }

    fn lower_shared_reference_expression(
        &mut self,
        expression: &mir::SharedReferenceExpression,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        match expression {
            mir::SharedReferenceExpression::New { class, value } => {
                let payload = self.lower_class_expression(value)?;
                let drop_function = *self
                    .class_drop_functions
                    .get(class.0)
                    .ok_or_else(|| malformed_mir("shared payload drop glue does not exist"))?;
                let drop_function = drop_function.as_global_value().as_pointer_value();
                Ok(self
                    .call_runtime(
                        SHARED_CREATE,
                        &[pointer.into(), pointer.into(), pointer.into()],
                        Some(pointer.into()),
                        &[
                            self.current_frame.into(),
                            payload.into(),
                            drop_function.into(),
                        ],
                    )?
                    .ok_or_else(|| backend_failure("shared construction produced no result"))?
                    .into_pointer_value())
            }
            mir::SharedReferenceExpression::Local {
                local, transfer, ..
            }
            | mir::SharedReferenceExpression::NullableLocalAssumeNonNull {
                local, transfer, ..
            } => {
                let slot = local_slot(&self.local_slots, *local)?;
                let value = build(self.builder.build_load(pointer, slot, "shared.local"))?
                    .into_pointer_value();
                if *transfer {
                    build(self.builder.build_store(slot, pointer.const_null()))?;
                }
                Ok(value)
            }
            mir::SharedReferenceExpression::Property {
                object, property, ..
            } => Ok(build(self.builder.build_load(
                pointer,
                self.lower_property_address(*object, *property)?,
                "shared.property",
            ))?
            .into_pointer_value()),
            mir::SharedReferenceExpression::Call { function, args, .. } => Ok(self
                .lower_call(*function, args, true)?
                .ok_or_else(|| malformed_mir("shared-reference call produced no result"))?
                .into_pointer_value()),
            mir::SharedReferenceExpression::Share { value, .. } => {
                let owned = value.owned_temporary().is_some();
                let value = self.lower_shared_reference_expression(value)?;
                let shared = self
                    .call_runtime(
                        SHARED_RETAIN,
                        &[pointer.into(), pointer.into()],
                        Some(pointer.into()),
                        &[self.current_frame.into(), value.into()],
                    )?
                    .ok_or_else(|| backend_failure("shared retain produced no result"))?
                    .into_pointer_value();
                if owned {
                    self.defer_or_drop_shared_temporary(value, false)?;
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
                let left = self.lower_nullable_shared_reference_expression(left)?;
                let function = current_function(&self.builder)?;
                let some = self
                    .context
                    .append_basic_block(function, "shared.coalesce.some");
                let none = self
                    .context
                    .append_basic_block(function, "shared.coalesce.none");
                let done = self
                    .context
                    .append_basic_block(function, "shared.coalesce.done");
                let present = build(
                    self.builder
                        .build_is_not_null(left, "shared.coalesce.present"),
                )?;
                build(self.builder.build_conditional_branch(present, some, none))?;
                self.builder.position_at_end(some);
                build(self.builder.build_unconditional_branch(done))?;
                let some_end = self
                    .builder
                    .get_insert_block()
                    .expect("shared coalesce some block");
                self.builder.position_at_end(none);
                let right = self.lower_shared_reference_expression(right)?;
                build(self.builder.build_unconditional_branch(done))?;
                let none_end = self
                    .builder
                    .get_insert_block()
                    .expect("shared coalesce none block");
                self.builder.position_at_end(done);
                let phi = build(self.builder.build_phi(pointer, "shared.coalesce"))?;
                phi.add_incoming(&[(&left, some_end), (&right, none_end)]);
                let result = phi.as_basic_value().into_pointer_value();
                if !transfer && (left_owned || right_owned) {
                    let temporary =
                        build(self.builder.build_phi(pointer, "shared.coalesce.temporary"))?;
                    let null = pointer.const_null();
                    let left_temporary = if left_owned { left } else { null };
                    let right_temporary = if right_owned { right } else { null };
                    temporary
                        .add_incoming(&[(&left_temporary, some_end), (&right_temporary, none_end)]);
                    self.defer_or_drop_shared_temporary(
                        temporary.as_basic_value().into_pointer_value(),
                        false,
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
            } => Ok(self
                .lower_collection_index(*collection, index, *remove, *positional)?
                .into_pointer_value()),
        }
    }

    fn lower_weak_reference_expression(
        &mut self,
        expression: &mir::WeakReferenceExpression,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        match expression {
            mir::WeakReferenceExpression::Local {
                local, transfer, ..
            }
            | mir::WeakReferenceExpression::NullableLocalAssumeNonNull {
                local, transfer, ..
            } => {
                let slot = local_slot(&self.local_slots, *local)?;
                let value = build(self.builder.build_load(pointer, slot, "weak.local"))?
                    .into_pointer_value();
                if *transfer {
                    build(self.builder.build_store(slot, pointer.const_null()))?;
                }
                Ok(value)
            }
            mir::WeakReferenceExpression::Property {
                object, property, ..
            } => Ok(build(self.builder.build_load(
                pointer,
                self.lower_property_address(*object, *property)?,
                "weak.property",
            ))?
            .into_pointer_value()),
            mir::WeakReferenceExpression::Call { function, args, .. } => Ok(self
                .lower_call(*function, args, true)?
                .ok_or_else(|| malformed_mir("weak-reference call produced no result"))?
                .into_pointer_value()),
            mir::WeakReferenceExpression::Create { value, .. } => {
                let owned = value.owned_temporary().is_some();
                let value = self.lower_shared_reference_expression(value)?;
                let weak = self
                    .call_runtime(
                        SHARED_CREATE_WEAK,
                        &[pointer.into(), pointer.into()],
                        Some(pointer.into()),
                        &[self.current_frame.into(), value.into()],
                    )?
                    .ok_or_else(|| backend_failure("weak-reference creation produced no result"))?
                    .into_pointer_value();
                if owned {
                    self.defer_or_drop_shared_temporary(value, false)?;
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
                let left = self.lower_nullable_weak_reference_expression(left)?;
                let function = current_function(&self.builder)?;
                let some = self
                    .context
                    .append_basic_block(function, "weak.coalesce.some");
                let none = self
                    .context
                    .append_basic_block(function, "weak.coalesce.none");
                let done = self
                    .context
                    .append_basic_block(function, "weak.coalesce.done");
                let present = build(
                    self.builder
                        .build_is_not_null(left, "weak.coalesce.present"),
                )?;
                build(self.builder.build_conditional_branch(present, some, none))?;
                self.builder.position_at_end(some);
                build(self.builder.build_unconditional_branch(done))?;
                let some_end = self
                    .builder
                    .get_insert_block()
                    .expect("weak coalesce some block");
                self.builder.position_at_end(none);
                let right = self.lower_weak_reference_expression(right)?;
                build(self.builder.build_unconditional_branch(done))?;
                let none_end = self
                    .builder
                    .get_insert_block()
                    .expect("weak coalesce none block");
                self.builder.position_at_end(done);
                let phi = build(self.builder.build_phi(pointer, "weak.coalesce"))?;
                phi.add_incoming(&[(&left, some_end), (&right, none_end)]);
                let result = phi.as_basic_value().into_pointer_value();
                if !transfer && (left_owned || right_owned) {
                    let temporary =
                        build(self.builder.build_phi(pointer, "weak.coalesce.temporary"))?;
                    let null = pointer.const_null();
                    let left_temporary = if left_owned { left } else { null };
                    let right_temporary = if right_owned { right } else { null };
                    temporary
                        .add_incoming(&[(&left_temporary, some_end), (&right_temporary, none_end)]);
                    self.defer_or_drop_shared_temporary(
                        temporary.as_basic_value().into_pointer_value(),
                        true,
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
            } => Ok(self
                .lower_collection_index(*collection, index, *remove, *positional)?
                .into_pointer_value()),
        }
    }

    fn lower_nullable_shared_reference_expression(
        &mut self,
        expression: &mir::NullableSharedReferenceExpression,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        match expression {
            mir::NullableSharedReferenceExpression::Null(_) => Ok(pointer.const_null()),
            mir::NullableSharedReferenceExpression::Shared(value) => {
                self.lower_shared_reference_expression(value)
            }
            mir::NullableSharedReferenceExpression::Local {
                local, transfer, ..
            } => {
                let slot = local_slot(&self.local_slots, *local)?;
                let value = build(
                    self.builder
                        .build_load(pointer, slot, "nullable-shared.local"),
                )?
                .into_pointer_value();
                if *transfer {
                    build(self.builder.build_store(slot, pointer.const_null()))?;
                }
                Ok(value)
            }
            mir::NullableSharedReferenceExpression::Property {
                object, property, ..
            } => Ok(build(self.builder.build_load(
                pointer,
                self.lower_property_address(*object, *property)?,
                "nullable-shared.property",
            ))?
            .into_pointer_value()),
            mir::NullableSharedReferenceExpression::Call { function, args, .. } => Ok(self
                .lower_call(*function, args, true)?
                .ok_or_else(|| malformed_mir("nullable shared call produced no result"))?
                .into_pointer_value()),
            mir::NullableSharedReferenceExpression::Acquire { value, .. } => {
                let owned = value.owned_temporary().is_some();
                let value = self.lower_weak_reference_expression(value)?;
                let acquired = self
                    .call_runtime(
                        SHARED_ACQUIRE,
                        &[pointer.into(), pointer.into()],
                        Some(pointer.into()),
                        &[self.current_frame.into(), value.into()],
                    )?
                    .ok_or_else(|| backend_failure("weak acquisition produced no result"))?
                    .into_pointer_value();
                if owned {
                    self.defer_or_drop_shared_temporary(value, true)?;
                }
                Ok(acquired)
            }
            mir::NullableSharedReferenceExpression::NullSafeShare { value, .. } => {
                let owned = value.owned_temporary().is_some();
                let value = self.lower_nullable_shared_reference_expression(value)?;
                let result =
                    self.lower_null_safe_shared_call(value, SHARED_RETAIN, "shared retain", true)?;
                if owned {
                    self.defer_or_drop_shared_temporary(value, false)?;
                }
                Ok(result)
            }
            mir::NullableSharedReferenceExpression::NullSafeAcquire { value, .. } => {
                let owned = value.owned_temporary().is_some();
                let value = self.lower_nullable_weak_reference_expression(value)?;
                let result = self.lower_null_safe_shared_call(
                    value,
                    SHARED_ACQUIRE,
                    "weak acquisition",
                    true,
                )?;
                if owned {
                    self.defer_or_drop_shared_temporary(value, true)?;
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
                let left = self.lower_nullable_shared_reference_expression(left)?;
                let function = current_function(&self.builder)?;
                let some = self
                    .context
                    .append_basic_block(function, "nullable.shared.coalesce.some");
                let none = self
                    .context
                    .append_basic_block(function, "nullable.shared.coalesce.none");
                let done = self
                    .context
                    .append_basic_block(function, "nullable.shared.coalesce.done");
                let present = build(
                    self.builder
                        .build_is_not_null(left, "nullable.shared.coalesce.present"),
                )?;
                build(self.builder.build_conditional_branch(present, some, none))?;
                self.builder.position_at_end(some);
                build(self.builder.build_unconditional_branch(done))?;
                let some_end = self
                    .builder
                    .get_insert_block()
                    .expect("nullable shared coalesce some block");
                self.builder.position_at_end(none);
                let right = self.lower_nullable_shared_reference_expression(right)?;
                build(self.builder.build_unconditional_branch(done))?;
                let none_end = self
                    .builder
                    .get_insert_block()
                    .expect("nullable shared coalesce none block");
                self.builder.position_at_end(done);
                let phi = build(self.builder.build_phi(pointer, "nullable.shared.coalesce"))?;
                phi.add_incoming(&[(&left, some_end), (&right, none_end)]);
                let result = phi.as_basic_value().into_pointer_value();
                if !transfer && (left_owned || right_owned) {
                    let temporary = build(
                        self.builder
                            .build_phi(pointer, "nullable.shared.coalesce.temporary"),
                    )?;
                    let null = pointer.const_null();
                    let left_temporary = if left_owned { left } else { null };
                    let right_temporary = if right_owned { right } else { null };
                    temporary
                        .add_incoming(&[(&left_temporary, some_end), (&right_temporary, none_end)]);
                    self.defer_or_drop_shared_temporary(
                        temporary.as_basic_value().into_pointer_value(),
                        false,
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
            } => Ok(self
                .lower_dictionary_get(
                    *collection,
                    key,
                    if *stored_nullable {
                        mir::Type::NullableSharedReference(*class)
                    } else {
                        mir::Type::SharedReference(*class)
                    },
                    *access,
                )?
                .1
                .into_pointer_value()),
            mir::NullableSharedReferenceExpression::CollectionIndex {
                collection,
                index,
                remove,
                positional,
                ..
            } => Ok(self
                .lower_collection_index(*collection, index, *remove, *positional)?
                .into_pointer_value()),
        }
    }

    fn lower_null_safe_shared_call(
        &mut self,
        value: PointerValue<'ctx>,
        symbol: &'static str,
        operation: &'static str,
        takes_frame: bool,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let function = current_function(&self.builder)?;
        let some = self
            .context
            .append_basic_block(function, "shared.null_safe.some");
        let none = self
            .context
            .append_basic_block(function, "shared.null_safe.none");
        let done = self
            .context
            .append_basic_block(function, "shared.null_safe.done");
        let present = build(
            self.builder
                .build_is_not_null(value, "shared.null_safe.present"),
        )?;
        build(self.builder.build_conditional_branch(present, some, none))?;
        self.builder.position_at_end(some);
        let (params, values): (&[_], &[_]) = if takes_frame {
            (
                &[pointer.into(), pointer.into()],
                &[self.current_frame.into(), value.into()],
            )
        } else {
            (&[pointer.into()], &[value.into()])
        };
        let result = self
            .call_runtime(symbol, params, Some(pointer.into()), values)?
            .ok_or_else(|| backend_failure(format!("null-safe {operation} produced no result")))?
            .into_pointer_value();
        build(self.builder.build_unconditional_branch(done))?;
        let some_end = self
            .builder
            .get_insert_block()
            .expect("null-safe shared some block");
        self.builder.position_at_end(none);
        build(self.builder.build_unconditional_branch(done))?;
        let none_end = self
            .builder
            .get_insert_block()
            .expect("null-safe shared none block");
        self.builder.position_at_end(done);
        let phi = build(self.builder.build_phi(pointer, "shared.null_safe"))?;
        let null = pointer.const_null();
        phi.add_incoming(&[(&result, some_end), (&null, none_end)]);
        Ok(phi.as_basic_value().into_pointer_value())
    }

    fn lower_nullable_weak_reference_expression(
        &mut self,
        expression: &mir::NullableWeakReferenceExpression,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        match expression {
            mir::NullableWeakReferenceExpression::Null(_) => Ok(pointer.const_null()),
            mir::NullableWeakReferenceExpression::Weak(value) => {
                self.lower_weak_reference_expression(value)
            }
            mir::NullableWeakReferenceExpression::Local {
                local, transfer, ..
            } => {
                let slot = local_slot(&self.local_slots, *local)?;
                let value = build(
                    self.builder
                        .build_load(pointer, slot, "nullable-weak.local"),
                )?
                .into_pointer_value();
                if *transfer {
                    build(self.builder.build_store(slot, pointer.const_null()))?;
                }
                Ok(value)
            }
            mir::NullableWeakReferenceExpression::Property {
                object, property, ..
            } => Ok(build(self.builder.build_load(
                pointer,
                self.lower_property_address(*object, *property)?,
                "nullable-weak.property",
            ))?
            .into_pointer_value()),
            mir::NullableWeakReferenceExpression::Call { function, args, .. } => Ok(self
                .lower_call(*function, args, true)?
                .ok_or_else(|| malformed_mir("nullable weak call produced no result"))?
                .into_pointer_value()),
            mir::NullableWeakReferenceExpression::NullSafeCreate { value, .. } => {
                let owned = value.owned_temporary().is_some();
                let value = self.lower_nullable_shared_reference_expression(value)?;
                let result = self.lower_null_safe_shared_call(
                    value,
                    SHARED_CREATE_WEAK,
                    "weak creation",
                    true,
                )?;
                if owned {
                    self.defer_or_drop_shared_temporary(value, false)?;
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
                let left = self.lower_nullable_weak_reference_expression(left)?;
                let function = current_function(&self.builder)?;
                let some = self
                    .context
                    .append_basic_block(function, "nullable.weak.coalesce.some");
                let none = self
                    .context
                    .append_basic_block(function, "nullable.weak.coalesce.none");
                let done = self
                    .context
                    .append_basic_block(function, "nullable.weak.coalesce.done");
                let present = build(
                    self.builder
                        .build_is_not_null(left, "nullable.weak.coalesce.present"),
                )?;
                build(self.builder.build_conditional_branch(present, some, none))?;
                self.builder.position_at_end(some);
                build(self.builder.build_unconditional_branch(done))?;
                let some_end = self
                    .builder
                    .get_insert_block()
                    .expect("nullable weak coalesce some block");
                self.builder.position_at_end(none);
                let right = self.lower_nullable_weak_reference_expression(right)?;
                build(self.builder.build_unconditional_branch(done))?;
                let none_end = self
                    .builder
                    .get_insert_block()
                    .expect("nullable weak coalesce none block");
                self.builder.position_at_end(done);
                let phi = build(self.builder.build_phi(pointer, "nullable.weak.coalesce"))?;
                phi.add_incoming(&[(&left, some_end), (&right, none_end)]);
                let result = phi.as_basic_value().into_pointer_value();
                if !transfer && (left_owned || right_owned) {
                    let temporary = build(
                        self.builder
                            .build_phi(pointer, "nullable.weak.coalesce.temporary"),
                    )?;
                    let null = pointer.const_null();
                    let left_temporary = if left_owned { left } else { null };
                    let right_temporary = if right_owned { right } else { null };
                    temporary
                        .add_incoming(&[(&left_temporary, some_end), (&right_temporary, none_end)]);
                    self.defer_or_drop_shared_temporary(
                        temporary.as_basic_value().into_pointer_value(),
                        true,
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
            } => Ok(self
                .lower_dictionary_get(
                    *collection,
                    key,
                    if *stored_nullable {
                        mir::Type::NullableWeakReference(*class)
                    } else {
                        mir::Type::WeakReference(*class)
                    },
                    *access,
                )?
                .1
                .into_pointer_value()),
            mir::NullableWeakReferenceExpression::CollectionIndex {
                collection,
                index,
                remove,
                positional,
                ..
            } => Ok(self
                .lower_collection_index(*collection, index, *remove, *positional)?
                .into_pointer_value()),
        }
    }

    fn lower_pointer_local(
        &mut self,
        local: mir::LocalId,
        transfer: bool,
        name: &str,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let slot = local_slot(&self.local_slots, local)?;
        let value = build(self.builder.build_load(pointer, slot, name))?.into_pointer_value();
        if transfer {
            build(self.builder.build_store(slot, pointer.const_null()))?;
        }
        Ok(value)
    }

    fn lower_pointer_property(
        &mut self,
        object: mir::LocalId,
        property: crate::class_layout::PropertyId,
        name: &str,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        Ok(build(self.builder.build_load(
            pointer,
            self.lower_property_address(object, property)?,
            name,
        ))?
        .into_pointer_value())
    }

    fn lower_shared_access_payload(
        &mut self,
        access: mir::LocalId,
        writable: bool,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let control = self.lower_pointer_local(access, false, "shared.access")?;
        Ok(self
            .call_runtime(
                if writable {
                    WRITABLE_SHARED_WRITABLE_PAYLOAD
                } else {
                    WRITABLE_SHARED_READONLY_PAYLOAD
                },
                &[pointer.into()],
                Some(pointer.into()),
                &[control.into()],
            )?
            .ok_or_else(|| backend_failure("shared access payload projection produced no result"))?
            .into_pointer_value())
    }

    fn lower_writable_shared_reference_expression(
        &mut self,
        expression: &mir::WritableSharedReferenceExpression,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        match expression {
            mir::WritableSharedReferenceExpression::New { payload, value } => {
                let value = self.lower_rvalue(value)?.into_pointer_value();
                let drop_function = match payload {
                    mir::WritableSharedPayload::Class(class) => *self
                        .class_drop_functions
                        .get(class.0)
                        .ok_or_else(|| malformed_mir("writable shared drop glue does not exist"))?,
                    mir::WritableSharedPayload::Collection(collection) => *self
                        .collection_drop_functions
                        .get(collection.0)
                        .ok_or_else(|| {
                            malformed_mir("writable shared collection drop glue does not exist")
                        })?,
                }
                .as_global_value()
                .as_pointer_value();
                Ok(self
                    .call_runtime(
                        WRITABLE_SHARED_CREATE,
                        &[pointer.into(), pointer.into(), pointer.into()],
                        Some(pointer.into()),
                        &[
                            self.current_frame.into(),
                            value.into(),
                            drop_function.into(),
                        ],
                    )?
                    .ok_or_else(|| {
                        backend_failure("writable shared construction produced no result")
                    })?
                    .into_pointer_value())
            }
            mir::WritableSharedReferenceExpression::Local {
                local, transfer, ..
            }
            | mir::WritableSharedReferenceExpression::NullableLocalAssumeNonNull {
                local,
                transfer,
                ..
            } => self.lower_pointer_local(*local, *transfer, "writable.shared.local"),
            mir::WritableSharedReferenceExpression::Property {
                object, property, ..
            } => self.lower_pointer_property(*object, *property, "writable.shared.property"),
            mir::WritableSharedReferenceExpression::Call { function, args, .. } => Ok(self
                .lower_call(*function, args, true)?
                .ok_or_else(|| malformed_mir("writable shared call produced no result"))?
                .into_pointer_value()),
            mir::WritableSharedReferenceExpression::Share { value, .. } => {
                let owned = value.owned_temporary();
                let control = self.lower_writable_shared_reference_expression(value)?;
                let result = self
                    .call_runtime(
                        WRITABLE_SHARED_RETAIN,
                        &[pointer.into(), pointer.into()],
                        Some(pointer.into()),
                        &[self.current_frame.into(), control.into()],
                    )?
                    .ok_or_else(|| backend_failure("writable shared retain produced no result"))?
                    .into_pointer_value();
                if owned {
                    self.defer_or_drop_writable_shared_temporary(control, WRITABLE_SHARED_RELEASE)?;
                }
                Ok(result)
            }
            mir::WritableSharedReferenceExpression::Coalesce {
                left,
                right,
                transfer,
                ..
            } => {
                self.lower_writable_shared_coalesce(left, right, *transfer, WRITABLE_SHARED_RELEASE)
            }
            mir::WritableSharedReferenceExpression::CollectionIndex {
                collection,
                index,
                remove,
                positional,
                ..
            } => Ok(self
                .lower_collection_index(*collection, index, *remove, *positional)?
                .into_pointer_value()),
        }
    }

    fn lower_writable_weak_reference_expression(
        &mut self,
        expression: &mir::WritableWeakReferenceExpression,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        match expression {
            mir::WritableWeakReferenceExpression::Local {
                local, transfer, ..
            }
            | mir::WritableWeakReferenceExpression::NullableLocalAssumeNonNull {
                local,
                transfer,
                ..
            } => self.lower_pointer_local(*local, *transfer, "writable.weak.local"),
            mir::WritableWeakReferenceExpression::Property {
                object, property, ..
            } => self.lower_pointer_property(*object, *property, "writable.weak.property"),
            mir::WritableWeakReferenceExpression::Call { function, args, .. } => Ok(self
                .lower_call(*function, args, true)?
                .ok_or_else(|| malformed_mir("writable weak call produced no result"))?
                .into_pointer_value()),
            mir::WritableWeakReferenceExpression::Create { value, .. } => {
                let owned = value.owned_temporary();
                let control = self.lower_writable_shared_reference_expression(value)?;
                let result = self
                    .call_runtime(
                        WRITABLE_SHARED_CREATE_WEAK,
                        &[pointer.into(), pointer.into()],
                        Some(pointer.into()),
                        &[self.current_frame.into(), control.into()],
                    )?
                    .ok_or_else(|| backend_failure("writable weak creation produced no result"))?
                    .into_pointer_value();
                if owned {
                    self.defer_or_drop_writable_shared_temporary(control, WRITABLE_SHARED_RELEASE)?;
                }
                Ok(result)
            }
            mir::WritableWeakReferenceExpression::Coalesce {
                left,
                right,
                transfer,
                ..
            } => self.lower_writable_weak_coalesce(
                left,
                right,
                *transfer,
                WRITABLE_SHARED_RELEASE_WEAK,
            ),
            mir::WritableWeakReferenceExpression::CollectionIndex {
                collection,
                index,
                remove,
                positional,
                ..
            } => Ok(self
                .lower_collection_index(*collection, index, *remove, *positional)?
                .into_pointer_value()),
        }
    }

    fn lower_nullable_writable_shared_reference_expression(
        &mut self,
        expression: &mir::NullableWritableSharedReferenceExpression,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        match expression {
            mir::NullableWritableSharedReferenceExpression::Null(_) => Ok(pointer.const_null()),
            mir::NullableWritableSharedReferenceExpression::Strong(value) => {
                self.lower_writable_shared_reference_expression(value)
            }
            mir::NullableWritableSharedReferenceExpression::Local {
                local, transfer, ..
            } => self.lower_pointer_local(*local, *transfer, "nullable.writable.shared.local"),
            mir::NullableWritableSharedReferenceExpression::Property {
                object, property, ..
            } => {
                self.lower_pointer_property(*object, *property, "nullable.writable.shared.property")
            }
            mir::NullableWritableSharedReferenceExpression::Call { function, args, .. } => Ok(self
                .lower_call(*function, args, true)?
                .ok_or_else(|| malformed_mir("nullable writable shared call produced no result"))?
                .into_pointer_value()),
            mir::NullableWritableSharedReferenceExpression::Acquire { value, .. } => {
                let owned = value.owned_temporary();
                let control = self.lower_writable_weak_reference_expression(value)?;
                let result = self
                    .call_runtime(
                        WRITABLE_SHARED_ACQUIRE,
                        &[pointer.into(), pointer.into()],
                        Some(pointer.into()),
                        &[self.current_frame.into(), control.into()],
                    )?
                    .ok_or_else(|| backend_failure("writable weak acquisition produced no result"))?
                    .into_pointer_value();
                if owned {
                    self.defer_or_drop_writable_shared_temporary(
                        control,
                        WRITABLE_SHARED_RELEASE_WEAK,
                    )?;
                }
                Ok(result)
            }
            mir::NullableWritableSharedReferenceExpression::NullSafeShare { value, .. } => {
                let owned = value.owned_temporary();
                let control = self.lower_nullable_writable_shared_reference_expression(value)?;
                let result = self.lower_null_safe_shared_call(
                    control,
                    WRITABLE_SHARED_RETAIN,
                    "writable shared retain",
                    true,
                )?;
                if owned {
                    self.defer_or_drop_writable_shared_temporary(control, WRITABLE_SHARED_RELEASE)?;
                }
                Ok(result)
            }
            mir::NullableWritableSharedReferenceExpression::NullSafeAcquire { value, .. } => {
                let owned = value.owned_temporary();
                let control = self.lower_nullable_writable_weak_reference_expression(value)?;
                let result = self.lower_null_safe_shared_call(
                    control,
                    WRITABLE_SHARED_ACQUIRE,
                    "writable weak acquisition",
                    true,
                )?;
                if owned {
                    self.defer_or_drop_writable_shared_temporary(
                        control,
                        WRITABLE_SHARED_RELEASE_WEAK,
                    )?;
                }
                Ok(result)
            }
            mir::NullableWritableSharedReferenceExpression::Coalesce {
                left,
                right,
                transfer,
                ..
            } => self.lower_nullable_writable_shared_coalesce(
                left,
                right,
                *transfer,
                WRITABLE_SHARED_RELEASE,
            ),
            mir::NullableWritableSharedReferenceExpression::DictionaryGet {
                payload,
                collection,
                key,
                access,
                stored_nullable,
            } => Ok(self
                .lower_dictionary_get(
                    *collection,
                    key,
                    if *stored_nullable {
                        mir::Type::NullableWritableSharedReference(*payload)
                    } else {
                        mir::Type::WritableSharedReference(*payload)
                    },
                    *access,
                )?
                .1
                .into_pointer_value()),
        }
    }

    fn lower_nullable_writable_weak_reference_expression(
        &mut self,
        expression: &mir::NullableWritableWeakReferenceExpression,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        match expression {
            mir::NullableWritableWeakReferenceExpression::Null(_) => Ok(pointer.const_null()),
            mir::NullableWritableWeakReferenceExpression::Weak(value) => {
                self.lower_writable_weak_reference_expression(value)
            }
            mir::NullableWritableWeakReferenceExpression::Local {
                local, transfer, ..
            } => self.lower_pointer_local(*local, *transfer, "nullable.writable.weak.local"),
            mir::NullableWritableWeakReferenceExpression::Property {
                object, property, ..
            } => self.lower_pointer_property(*object, *property, "nullable.writable.weak.property"),
            mir::NullableWritableWeakReferenceExpression::Call { function, args, .. } => Ok(self
                .lower_call(*function, args, true)?
                .ok_or_else(|| malformed_mir("nullable writable weak call produced no result"))?
                .into_pointer_value()),
            mir::NullableWritableWeakReferenceExpression::NullSafeCreate { value, .. } => {
                let owned = value.owned_temporary();
                let control = self.lower_nullable_writable_shared_reference_expression(value)?;
                let result = self.lower_null_safe_shared_call(
                    control,
                    WRITABLE_SHARED_CREATE_WEAK,
                    "writable weak creation",
                    true,
                )?;
                if owned {
                    self.defer_or_drop_writable_shared_temporary(control, WRITABLE_SHARED_RELEASE)?;
                }
                Ok(result)
            }
            mir::NullableWritableWeakReferenceExpression::Coalesce {
                left,
                right,
                transfer,
                ..
            } => self.lower_nullable_writable_weak_coalesce(
                left,
                right,
                *transfer,
                WRITABLE_SHARED_RELEASE_WEAK,
            ),
            mir::NullableWritableWeakReferenceExpression::DictionaryGet {
                payload,
                collection,
                key,
                access,
                stored_nullable,
            } => Ok(self
                .lower_dictionary_get(
                    *collection,
                    key,
                    if *stored_nullable {
                        mir::Type::NullableWritableWeakReference(*payload)
                    } else {
                        mir::Type::WritableWeakReference(*payload)
                    },
                    *access,
                )?
                .1
                .into_pointer_value()),
        }
    }

    fn lower_shared_reference_access_expression(
        &mut self,
        expression: &mir::SharedReferenceAccessExpression,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        match expression {
            mir::SharedReferenceAccessExpression::Local {
                local, transfer, ..
            }
            | mir::SharedReferenceAccessExpression::NullableLocalAssumeNonNull {
                local,
                transfer,
                ..
            } => self.lower_pointer_local(*local, *transfer, "shared.access.local"),
            mir::SharedReferenceAccessExpression::Property {
                object, property, ..
            } => self.lower_pointer_property(*object, *property, "shared.access.property"),
            mir::SharedReferenceAccessExpression::CollectionIndex {
                collection,
                index,
                remove,
                positional,
                ..
            } => Ok(self
                .lower_collection_index(*collection, index, *remove, *positional)?
                .into_pointer_value()),
            mir::SharedReferenceAccessExpression::Call { function, args, .. } => Ok(self
                .lower_call(*function, args, true)?
                .ok_or_else(|| malformed_mir("shared access call produced no result"))?
                .into_pointer_value()),
            mir::SharedReferenceAccessExpression::Acquire {
                value,
                writable,
                span,
                ..
            } => {
                let owned = value.owned_temporary();
                let control = self.lower_writable_shared_reference_expression(value)?;
                self.set_active_panic_site(*span)?;
                let result = self
                    .call_runtime(
                        if *writable {
                            WRITABLE_SHARED_ACQUIRE_WRITABLE_ACCESS
                        } else {
                            WRITABLE_SHARED_ACQUIRE_READONLY_ACCESS
                        },
                        &[pointer.into(), pointer.into()],
                        Some(pointer.into()),
                        &[self.current_frame.into(), control.into()],
                    )?
                    .ok_or_else(|| backend_failure("shared access acquisition produced no result"))?
                    .into_pointer_value();
                if owned {
                    self.defer_or_drop_writable_shared_temporary(control, WRITABLE_SHARED_RELEASE)?;
                }
                Ok(result)
            }
        }
    }

    fn lower_nullable_shared_reference_access_expression(
        &mut self,
        expression: &mir::NullableSharedReferenceAccessExpression,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        match expression {
            mir::NullableSharedReferenceAccessExpression::Null { .. } => Ok(pointer.const_null()),
            mir::NullableSharedReferenceAccessExpression::Access(value) => {
                self.lower_shared_reference_access_expression(value)
            }
            mir::NullableSharedReferenceAccessExpression::Local {
                local, transfer, ..
            } => self.lower_pointer_local(*local, *transfer, "nullable.shared.access.local"),
            mir::NullableSharedReferenceAccessExpression::Property {
                object, property, ..
            } => self.lower_pointer_property(*object, *property, "nullable.shared.access.property"),
            mir::NullableSharedReferenceAccessExpression::CollectionIndex {
                collection,
                index,
                remove,
                positional,
                ..
            } => Ok(self
                .lower_collection_index(*collection, index, *remove, *positional)?
                .into_pointer_value()),
            mir::NullableSharedReferenceAccessExpression::CollectionGet {
                collection,
                key,
                access,
                stored,
            } => Ok(self
                .lower_dictionary_get(*collection, key, stored.into_type(), *access)?
                .1
                .into_pointer_value()),
            mir::NullableSharedReferenceAccessExpression::Call { function, args, .. } => Ok(self
                .lower_call(*function, args, true)?
                .ok_or_else(|| malformed_mir("nullable shared access call produced no result"))?
                .into_pointer_value()),
            mir::NullableSharedReferenceAccessExpression::NullSafeAcquire {
                value,
                writable,
                span,
                ..
            } => {
                let owned = value.owned_temporary();
                let control = self.lower_nullable_writable_shared_reference_expression(value)?;
                self.set_active_panic_site(*span)?;
                let result = self.lower_null_safe_shared_call(
                    control,
                    if *writable {
                        WRITABLE_SHARED_ACQUIRE_WRITABLE_ACCESS
                    } else {
                        WRITABLE_SHARED_ACQUIRE_READONLY_ACCESS
                    },
                    "shared access acquisition",
                    true,
                )?;
                if owned {
                    self.defer_or_drop_writable_shared_temporary(control, WRITABLE_SHARED_RELEASE)?;
                }
                Ok(result)
            }
        }
    }

    fn lower_writable_pointer_coalesce(
        &mut self,
        left: PointerValue<'ctx>,
        left_owned: bool,
        right_owned: bool,
        transfer: bool,
        release: &'static str,
        lower_right: impl FnOnce(&mut Self) -> Result<PointerValue<'ctx>, BackendError>,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let function = current_function(&self.builder)?;
        let some = self
            .context
            .append_basic_block(function, "writable.coalesce.some");
        let none = self
            .context
            .append_basic_block(function, "writable.coalesce.none");
        let done = self
            .context
            .append_basic_block(function, "writable.coalesce.done");
        let present = build(
            self.builder
                .build_is_not_null(left, "writable.coalesce.present"),
        )?;
        build(self.builder.build_conditional_branch(present, some, none))?;
        self.builder.position_at_end(some);
        build(self.builder.build_unconditional_branch(done))?;
        let some_end = self
            .builder
            .get_insert_block()
            .expect("writable coalesce some block");
        self.builder.position_at_end(none);
        let right = lower_right(self)?;
        build(self.builder.build_unconditional_branch(done))?;
        let none_end = self
            .builder
            .get_insert_block()
            .expect("writable coalesce none block");
        self.builder.position_at_end(done);
        let result = build(self.builder.build_phi(pointer, "writable.coalesce"))?;
        result.add_incoming(&[(&left, some_end), (&right, none_end)]);
        if !transfer && (left_owned || right_owned) {
            let temporary = build(
                self.builder
                    .build_phi(pointer, "writable.coalesce.temporary"),
            )?;
            let null = pointer.const_null();
            temporary.add_incoming(&[
                (&if left_owned { left } else { null }, some_end),
                (&if right_owned { right } else { null }, none_end),
            ]);
            self.defer_or_drop_writable_shared_temporary(
                temporary.as_basic_value().into_pointer_value(),
                release,
            )?;
        }
        Ok(result.as_basic_value().into_pointer_value())
    }

    fn lower_writable_shared_coalesce(
        &mut self,
        left: &mir::NullableWritableSharedReferenceExpression,
        right: &mir::WritableSharedReferenceExpression,
        transfer: bool,
        release: &'static str,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let left_owned = left.owned_temporary();
        let right_owned = right.owned_temporary();
        let left = self.lower_nullable_writable_shared_reference_expression(left)?;
        self.lower_writable_pointer_coalesce(
            left,
            left_owned,
            right_owned,
            transfer,
            release,
            |lowerer| lowerer.lower_writable_shared_reference_expression(right),
        )
    }

    fn lower_nullable_writable_shared_coalesce(
        &mut self,
        left: &mir::NullableWritableSharedReferenceExpression,
        right: &mir::NullableWritableSharedReferenceExpression,
        transfer: bool,
        release: &'static str,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let left_owned = left.owned_temporary();
        let right_owned = right.owned_temporary();
        let left = self.lower_nullable_writable_shared_reference_expression(left)?;
        self.lower_writable_pointer_coalesce(
            left,
            left_owned,
            right_owned,
            transfer,
            release,
            |lowerer| lowerer.lower_nullable_writable_shared_reference_expression(right),
        )
    }

    fn lower_writable_weak_coalesce(
        &mut self,
        left: &mir::NullableWritableWeakReferenceExpression,
        right: &mir::WritableWeakReferenceExpression,
        transfer: bool,
        release: &'static str,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let left_owned = left.owned_temporary();
        let right_owned = right.owned_temporary();
        let left = self.lower_nullable_writable_weak_reference_expression(left)?;
        self.lower_writable_pointer_coalesce(
            left,
            left_owned,
            right_owned,
            transfer,
            release,
            |lowerer| lowerer.lower_writable_weak_reference_expression(right),
        )
    }

    fn lower_nullable_writable_weak_coalesce(
        &mut self,
        left: &mir::NullableWritableWeakReferenceExpression,
        right: &mir::NullableWritableWeakReferenceExpression,
        transfer: bool,
        release: &'static str,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let left_owned = left.owned_temporary();
        let right_owned = right.owned_temporary();
        let left = self.lower_nullable_writable_weak_reference_expression(left)?;
        self.lower_writable_pointer_coalesce(
            left,
            left_owned,
            right_owned,
            transfer,
            release,
            |lowerer| lowerer.lower_nullable_writable_weak_reference_expression(right),
        )
    }

    fn lower_nullable_class_expression(
        &mut self,
        expression: &mir::NullableClassExpression,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        match expression {
            mir::NullableClassExpression::Null(_) => Ok(pointer.const_null()),
            mir::NullableClassExpression::Class(value) => self.lower_class_expression(value),
            mir::NullableClassExpression::SharedPayload { reference, .. } => {
                let owned = reference.owned_temporary().is_some();
                let control = self.lower_nullable_shared_reference_expression(reference)?;
                let payload = self.lower_null_safe_shared_call(
                    control,
                    SHARED_PAYLOAD,
                    "nullable shared payload projection",
                    false,
                )?;
                if owned {
                    self.defer_or_drop_shared_temporary(control, false)?;
                }
                Ok(payload)
            }
            mir::NullableClassExpression::Local {
                local, transfer, ..
            } => {
                let slot = local_slot(&self.local_slots, *local)?;
                let value = build(
                    self.builder
                        .build_load(pointer, slot, "nullable-class.local"),
                )?
                .into_pointer_value();
                if *transfer {
                    build(self.builder.build_store(slot, pointer.const_null()))?;
                }
                Ok(value)
            }
            mir::NullableClassExpression::Property {
                object, property, ..
            } => Ok(build(self.builder.build_load(
                pointer,
                self.lower_property_address(*object, *property)?,
                "nullable-class.property",
            ))?
            .into_pointer_value()),
            mir::NullableClassExpression::Call { function, args, .. } => Ok(self
                .lower_call(*function, args, true)?
                .ok_or_else(|| malformed_mir("nullable-class call produced no result"))?
                .into_pointer_value()),
            mir::NullableClassExpression::NullSafeProperty {
                object, property, ..
            } => {
                let owned_receiver = object.owned_temporary_class();
                let object = self.lower_nullable_class_expression(object)?;
                self.lower_null_safe_pointer(object, owned_receiver, |lowerer| {
                    Ok(build(lowerer.builder.build_load(
                        pointer,
                        lowerer.lower_property_address_from_value(object, *property)?,
                        "null-safe.class.property",
                    ))?
                    .into_pointer_value())
                })
            }
            mir::NullableClassExpression::NullSafeCall {
                object,
                function,
                args,
                ..
            } => {
                let owned_receiver = object.owned_temporary_class();
                let object = self.lower_nullable_class_expression(object)?;
                self.lower_null_safe_pointer(object, owned_receiver, |lowerer| {
                    Ok(lowerer
                        .lower_method_call(object, *function, args, true)?
                        .ok_or_else(|| malformed_mir("null-safe class call produced no result"))?
                        .into_pointer_value())
                })
            }
            mir::NullableClassExpression::Coalesce {
                class,
                left,
                right,
                transfer,
            } => {
                let left_owned = left.owned_temporary_class().is_some();
                let right_owned = right.owned_temporary_class().is_some();
                let left = self.lower_nullable_class_expression(left)?;
                let function = current_function(&self.builder)?;
                let some = self
                    .context
                    .append_basic_block(function, "nullable-class.coalesce.some");
                let none = self
                    .context
                    .append_basic_block(function, "nullable-class.coalesce.none");
                let done = self
                    .context
                    .append_basic_block(function, "nullable-class.coalesce.done");
                let present = build(
                    self.builder
                        .build_is_not_null(left, "nullable-class.coalesce.present"),
                )?;
                build(self.builder.build_conditional_branch(present, some, none))?;
                self.builder.position_at_end(some);
                build(self.builder.build_unconditional_branch(done))?;
                let some_end = self
                    .builder
                    .get_insert_block()
                    .expect("nullable coalesce some block");
                self.builder.position_at_end(none);
                let right = self.lower_nullable_class_expression(right)?;
                build(self.builder.build_unconditional_branch(done))?;
                let none_end = self
                    .builder
                    .get_insert_block()
                    .expect("nullable coalesce none block");
                self.builder.position_at_end(done);
                let result = build(self.builder.build_phi(pointer, "nullable-class.coalesce"))?;
                result.add_incoming(&[(&left, some_end), (&right, none_end)]);
                if !transfer && (left_owned || right_owned) {
                    let temporary = build(
                        self.builder
                            .build_phi(pointer, "nullable-class.coalesce.temporary"),
                    )?;
                    let null = pointer.const_null();
                    let left_temporary = if left_owned { left } else { null };
                    let right_temporary = if right_owned { right } else { null };
                    temporary
                        .add_incoming(&[(&left_temporary, some_end), (&right_temporary, none_end)]);
                    self.defer_or_drop_class_temporary(
                        temporary.as_basic_value().into_pointer_value(),
                        *class,
                    )?;
                }
                Ok(result.as_basic_value().into_pointer_value())
            }
            mir::NullableClassExpression::DictionaryGet {
                class,
                collection,
                key,
                access,
            } => Ok(self
                .lower_dictionary_get(*collection, key, mir::Type::Class(*class), *access)?
                .1
                .into_pointer_value()),
        }
    }

    fn lower_null_safe_pointer(
        &mut self,
        object: PointerValue<'ctx>,
        owned_receiver: Option<crate::class_layout::ClassId>,
        present_value: impl FnOnce(&mut Self) -> Result<PointerValue<'ctx>, BackendError>,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        if let Some(class) = owned_receiver {
            self.defer_or_drop_class_temporary(object, class)?;
        }
        let function = current_function(&self.builder)?;
        let some = self.context.append_basic_block(function, "null-safe.some");
        let none = self.context.append_basic_block(function, "null-safe.none");
        let done = self.context.append_basic_block(function, "null-safe.done");
        let present = build(self.builder.build_is_not_null(object, "null-safe.present"))?;
        build(self.builder.build_conditional_branch(present, some, none))?;
        self.builder.position_at_end(some);
        let value = present_value(self)?;
        build(self.builder.build_unconditional_branch(done))?;
        let some_end = self
            .builder
            .get_insert_block()
            .expect("null-safe some block");
        self.builder.position_at_end(none);
        build(self.builder.build_unconditional_branch(done))?;
        let none_end = self
            .builder
            .get_insert_block()
            .expect("null-safe none block");
        self.builder.position_at_end(done);
        let pointer = self.context.ptr_type(AddressSpace::default());
        let null = pointer.const_null();
        let phi = build(self.builder.build_phi(pointer, "null-safe.pointer"))?;
        phi.add_incoming(&[(&value, some_end), (&null, none_end)]);
        let result = phi.as_basic_value().into_pointer_value();
        Ok(result)
    }

    fn lower_property_address(
        &self,
        object: mir::LocalId,
        property: crate::class_layout::PropertyId,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let object = build(self.builder.build_load(
            pointer,
            local_slot(&self.local_slots, object)?,
            "property.object",
        ))?
        .into_pointer_value();
        self.lower_property_address_from_value(object, property)
    }

    fn lower_property_address_from_value(
        &self,
        object: PointerValue<'ctx>,
        property: crate::class_layout::PropertyId,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let class = class_definition(self.program, property.class)?;
        let layout = class
            .layout
            .properties
            .iter()
            .find(|layout| layout.id == property)
            .ok_or_else(|| malformed_mir(format!("property{} has no layout", property.index)))?;
        let offset = self
            .context
            .ptr_sized_int_type(self.target_data, None)
            .const_int(u64::from(layout.offset), false);
        unsafe {
            build(self.builder.build_in_bounds_gep(
                self.context.i8_type(),
                object,
                &[offset],
                "property.address",
            ))
        }
    }

    fn static_address(&self, id: mir::StaticId) -> Result<PointerValue<'ctx>, BackendError> {
        static_definition(self.program, id)?;
        self.statics
            .get(id.0)
            .map(|global| global.as_pointer_value())
            .ok_or_else(|| malformed_mir(format!("static{} was not declared", id.0)))
    }

    fn lower_nullable_string_expression(
        &mut self,
        expression: &mir::NullableStringExpression,
    ) -> Result<StructValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        match expression {
            mir::NullableStringExpression::Intrinsic(call) => {
                Ok(self.lower_string_intrinsic_call(call)?.into_struct_value())
            }
            mir::NullableStringExpression::Null => {
                self.nullable_value(self.present_word(false), pointer.const_null().into())
            }
            mir::NullableStringExpression::String(value) => {
                let payload = self.lower_string_expression(value)?;
                self.nullable_value(self.present_word(true), payload.into())
            }
            mir::NullableStringExpression::Local(local) => {
                let value = build(self.builder.build_load(
                    llvm_type(self.context, self.target_data, mir::Type::NullableString),
                    local_slot(&self.local_slots, *local)?,
                    "nullable-string.local",
                ))?
                .into_struct_value();
                let (present, payload) = self.nullable_parts(value)?;
                self.nullable_value(
                    present,
                    self.retain_string(payload.into_pointer_value())?.into(),
                )
            }
            mir::NullableStringExpression::Static(id) => {
                let value = build(self.builder.build_load(
                    llvm_type(self.context, self.target_data, mir::Type::NullableString),
                    self.static_address(*id)?,
                    "nullable-string.static",
                ))?
                .into_struct_value();
                let (present, payload) = self.nullable_parts(value)?;
                self.nullable_value(
                    present,
                    self.retain_string(payload.into_pointer_value())?.into(),
                )
            }
            mir::NullableStringExpression::Property { object, property } => {
                let address = self.lower_property_address(*object, *property)?;
                let value = build(self.builder.build_load(
                    llvm_type(self.context, self.target_data, mir::Type::NullableString),
                    address,
                    "nullable-string.property",
                ))?
                .into_struct_value();
                let (present, payload) = self.nullable_parts(value)?;
                self.nullable_value(
                    present,
                    self.retain_string(payload.into_pointer_value())?.into(),
                )
            }
            mir::NullableStringExpression::ReadLine {
                prompt,
                prompt_span,
            } => {
                // Same validated MIR and same runtime ABI as Cranelift: the prompt is
                // evaluated once, borrowed across the call, and released afterwards.
                let prompt = self.lower_string_expression(prompt)?;
                self.set_active_panic_site(*prompt_span)?;
                let payload = self
                    .call_runtime(
                        READ_STDIN_LINE_PROMPTED,
                        &[pointer.into(), pointer.into()],
                        Some(pointer.into()),
                        &[self.current_frame.into(), prompt.into()],
                    )?
                    .ok_or_else(|| backend_failure("read_line produced no result"))?
                    .into_pointer_value();
                self.release_string(prompt)?;
                let present = build(self.builder.build_is_not_null(payload, "read-line.present"))?;
                let present = build(self.builder.build_int_z_extend(
                    present,
                    self.context.ptr_sized_int_type(self.target_data, None),
                    "read-line.present.word",
                ))?;
                self.nullable_value(present, payload.into())
            }
            mir::NullableStringExpression::Call { function, args } => Ok(self
                .lower_call(*function, args, true)?
                .ok_or_else(|| malformed_mir("nullable-string call produced no result"))?
                .into_struct_value()),
            mir::NullableStringExpression::EnumBacking { enum_id, value } => {
                let value = self.lower_nullable_scalar_expression(value)?;
                self.lower_nullable_value_map(pointer.into(), value, |lowerer, tag| {
                    Ok(lowerer
                        .lower_string_enum_backing_from_tag(*enum_id, tag.into_int_value())?
                        .into())
                })
            }
            mir::NullableStringExpression::NullSafeProperty { object, property } => {
                let owned_receiver = object.owned_temporary_class();
                let object = self.lower_nullable_class_expression(object)?;
                self.lower_null_safe_nullable(pointer.into(), object, owned_receiver, |lowerer| {
                    let property = property_definition(lowerer.program, *property)?;
                    let value = build(lowerer.builder.build_load(
                        llvm_type(lowerer.context, lowerer.target_data, property.ty),
                        lowerer.lower_property_address_from_value(object, property.id)?,
                        "null-safe.string.property",
                    ))?;
                    if property.ty == mir::Type::NullableString {
                        let (present, payload) =
                            lowerer.nullable_parts(value.into_struct_value())?;
                        Ok(lowerer
                            .nullable_value(
                                present,
                                lowerer.retain_string(payload.into_pointer_value())?.into(),
                            )?
                            .into())
                    } else {
                        Ok(lowerer.retain_string(value.into_pointer_value())?.into())
                    }
                })
            }
            mir::NullableStringExpression::NullSafeCall {
                object,
                function,
                args,
            } => {
                let owned_receiver = object.owned_temporary_class();
                let object = self.lower_nullable_class_expression(object)?;
                self.lower_null_safe_nullable(pointer.into(), object, owned_receiver, |lowerer| {
                    lowerer
                        .lower_method_call(object, *function, args, true)?
                        .ok_or_else(|| malformed_mir("null-safe string call produced no result"))
                })
            }
            mir::NullableStringExpression::Coalesce { left, right } => {
                let left = self.lower_nullable_string_expression(left)?;
                self.lower_nullable_coalesce(left, |lowerer| {
                    lowerer
                        .lower_nullable_string_expression(right)
                        .map(BasicValueEnum::from)
                })
            }
            mir::NullableStringExpression::DictionaryGet {
                collection,
                key,
                access,
            } => {
                let (present, payload) =
                    self.lower_dictionary_get(*collection, key, mir::Type::String, *access)?;
                let payload = payload.into_pointer_value();
                let payload = if matches!(
                    access,
                    mir::NullableCollectionAccess::Remove
                        | mir::NullableCollectionAccess::Pop
                        | mir::NullableCollectionAccess::PopFront
                        | mir::NullableCollectionAccess::PopBack
                ) {
                    payload
                } else {
                    self.retain_string(payload)?
                };
                self.nullable_value(present, payload.into())
            }
        }
    }

    fn lower_nullable_scalar_expression(
        &mut self,
        expression: &mir::NullableScalarExpression,
    ) -> Result<StructValue<'ctx>, BackendError> {
        let ty = expression.ty();
        let payload_type = scalar_type(self.context, ty);
        match expression {
            mir::NullableScalarExpression::StringIntrinsic(call) => {
                Ok(self.lower_string_intrinsic_call(call)?.into_struct_value())
            }
            mir::NullableScalarExpression::Null(_) => {
                self.nullable_value(self.present_word(false), payload_type.const_zero())
            }
            mir::NullableScalarExpression::Value(value) => {
                let payload = self.lower_value_expression(value)?;
                self.nullable_value(self.present_word(true), payload)
            }
            mir::NullableScalarExpression::Local { local, .. } => {
                Ok(build(self.builder.build_load(
                    llvm_type(
                        self.context,
                        self.target_data,
                        mir::Type::NullableScalar(ty),
                    ),
                    local_slot(&self.local_slots, *local)?,
                    "nullable-scalar.local",
                ))?
                .into_struct_value())
            }
            mir::NullableScalarExpression::Property {
                object, property, ..
            } => Ok(build(self.builder.build_load(
                llvm_type(
                    self.context,
                    self.target_data,
                    mir::Type::NullableScalar(ty),
                ),
                self.lower_property_address(*object, *property)?,
                "nullable-scalar.property",
            ))?
            .into_struct_value()),
            mir::NullableScalarExpression::Static { id, .. } => {
                Ok(build(self.builder.build_load(
                    llvm_type(
                        self.context,
                        self.target_data,
                        mir::Type::NullableScalar(ty),
                    ),
                    self.static_address(*id)?,
                    "nullable-scalar.static",
                ))?
                .into_struct_value())
            }
            mir::NullableScalarExpression::Call { function, args, .. } => Ok(self
                .lower_call(*function, args, true)?
                .ok_or_else(|| malformed_mir("nullable-scalar call produced no result"))?
                .into_struct_value()),
            mir::NullableScalarExpression::EnumBacking { enum_id, value } => {
                let value = self.lower_nullable_scalar_expression(value)?;
                self.lower_nullable_value_map(
                    self.context.i64_type().into(),
                    value,
                    |lowerer, tag| {
                        Ok(lowerer
                            .lower_integer_enum_backing_from_tag(*enum_id, tag.into_int_value())?
                            .into())
                    },
                )
            }
            mir::NullableScalarExpression::NullSafeProperty {
                object, property, ..
            } => {
                let owned_receiver = object.owned_temporary_class();
                let object = self.lower_nullable_class_expression(object)?;
                self.lower_null_safe_nullable(payload_type, object, owned_receiver, |lowerer| {
                    let property = property_definition(lowerer.program, *property)?;
                    build(lowerer.builder.build_load(
                        llvm_type(lowerer.context, lowerer.target_data, property.ty),
                        lowerer.lower_property_address_from_value(object, property.id)?,
                        "null-safe.scalar.property",
                    ))
                })
            }
            mir::NullableScalarExpression::NullSafeCall {
                object,
                function,
                args,
                ..
            } => {
                let owned_receiver = object.owned_temporary_class();
                let object = self.lower_nullable_class_expression(object)?;
                self.lower_null_safe_nullable(payload_type, object, owned_receiver, |lowerer| {
                    lowerer
                        .lower_method_call(object, *function, args, true)?
                        .ok_or_else(|| malformed_mir("null-safe scalar call produced no result"))
                })
            }
            mir::NullableScalarExpression::Coalesce { left, right, .. } => {
                let left = self.lower_nullable_scalar_expression(left)?;
                self.lower_nullable_coalesce(left, |lowerer| {
                    lowerer
                        .lower_nullable_scalar_expression(right)
                        .map(BasicValueEnum::from)
                })
            }
            mir::NullableScalarExpression::DictionaryGet {
                collection,
                key,
                access,
                ..
            } => {
                let (present, payload) =
                    self.lower_dictionary_get(*collection, key, mir::Type::Scalar(ty), *access)?;
                self.nullable_value(present, payload)
            }
            mir::NullableScalarExpression::CollectionIndexOf { collection, value } => {
                let pointer = self.context.ptr_type(AddressSpace::default());
                let local = local_in(self.function, *collection)?;
                let mir::Type::Collection(collection_type) = local.ty else {
                    return Err(malformed_mir("List::indexOf uses a non-collection local"));
                };
                let definition = self.collection_definition(collection_type)?.clone();
                if let Some((payload, nullable)) = Self::payload_enum_storage(definition.value) {
                    let owned = Self::payload_enum_rvalue_is_owned(value);
                    let needle = self.lower_rvalue(value)?.into_pointer_value();
                    let collection = self.collection_pointer(*collection)?;
                    let (found, index) = self.lower_payload_enum_collection_search(
                        collection, needle, payload, nullable,
                    )?;
                    if owned {
                        self.drop_payload_enum_at(needle, payload, nullable)?;
                    }
                    let present = build(self.builder.build_int_z_extend(
                        found,
                        self.context.ptr_sized_int_type(self.target_data, None),
                        "aggregate.index-of.present",
                    ))?;
                    let position = if index.get_type().get_bit_width() == 64 {
                        index
                    } else {
                        build(self.builder.build_int_z_extend(
                            index,
                            self.context.i64_type(),
                            "aggregate.index-of.position",
                        ))?
                    };
                    return self.nullable_value(present, position.into());
                }
                let (needle_present, needle, needle_type) =
                    if nullable_payload_type(definition.value).is_some() {
                        self.lower_nullable_collection_parts(value, definition.value)?
                    } else {
                        (
                            self.context.i8_type().const_int(1, false),
                            self.lower_rvalue(value)?,
                            definition.value,
                        )
                    };
                let needle_word = self.value_to_collection_word(needle, needle_type)?;
                let collection = self.collection_pointer(*collection)?;
                let kind = self.collection_compare_kind(needle_type)?;
                let found_slot = self.entry_alloca(self.context.i8_type(), "index-of.found")?;
                let position = self
                    .call_runtime(
                        COLLECTION_INDEX_OF,
                        &[
                            pointer.into(),
                            self.context.i64_type().into(),
                            self.context.i8_type().into(),
                            self.context.i8_type().into(),
                            pointer.into(),
                        ],
                        Some(self.context.i64_type().into()),
                        &[
                            collection.into(),
                            needle_word.into(),
                            needle_present.into(),
                            kind.into(),
                            found_slot.into(),
                        ],
                    )?
                    .ok_or_else(|| backend_failure("List::indexOf produced no result"))?
                    .into_int_value();
                let found = build(self.builder.build_load(
                    self.context.i8_type(),
                    found_slot,
                    "index-of.found.value",
                ))?
                .into_int_value();
                let present = build(self.builder.build_int_z_extend(
                    found,
                    self.context.ptr_sized_int_type(self.target_data, None),
                    "index-of.present",
                ))?;
                self.drop_stored_value(needle, definition.value)?;
                self.nullable_value(present, position.into())
            }
            mir::NullableScalarExpression::Parse { value, .. } => {
                let pointer = self.context.ptr_type(AddressSpace::default());
                let text = self.lower_string_expression(value)?;
                let found_slot = self.entry_alloca(self.context.i8_type(), "parse.found")?;
                let symbol = match ty {
                    mir::ScalarType::Integer(_) => INT_PARSE,
                    mir::ScalarType::Float(_) => FLOAT_PARSE,
                    mir::ScalarType::Bool | mir::ScalarType::Enum(_) => {
                        return Err(malformed_mir("parse does not produce a bool value"));
                    }
                };
                let word = self
                    .call_runtime(
                        symbol,
                        &[pointer.into(), pointer.into()],
                        Some(self.context.i64_type().into()),
                        &[text.into(), found_slot.into()],
                    )?
                    .ok_or_else(|| backend_failure("parse produced no result"))?
                    .into_int_value();
                self.release_string(text)?;
                let found = build(self.builder.build_load(
                    self.context.i8_type(),
                    found_slot,
                    "parse.found.value",
                ))?
                .into_int_value();
                let present = build(self.builder.build_int_z_extend(
                    found,
                    self.context.ptr_sized_int_type(self.target_data, None),
                    "parse.present",
                ))?;
                let payload = self.collection_word_to_value(word, mir::Type::Scalar(ty))?;
                self.nullable_value(present, payload)
            }
        }
    }

    fn lower_null_safe_nullable(
        &mut self,
        payload_type: BasicTypeEnum<'ctx>,
        object: PointerValue<'ctx>,
        owned_receiver: Option<crate::class_layout::ClassId>,
        present_value: impl FnOnce(&mut Self) -> Result<BasicValueEnum<'ctx>, BackendError>,
    ) -> Result<StructValue<'ctx>, BackendError> {
        if let Some(class) = owned_receiver {
            self.defer_or_drop_class_temporary(object, class)?;
        }
        let function = current_function(&self.builder)?;
        let some = self.context.append_basic_block(function, "null-safe.some");
        let none = self.context.append_basic_block(function, "null-safe.none");
        let done = self.context.append_basic_block(function, "null-safe.done");
        let present = build(self.builder.build_is_not_null(object, "null-safe.present"))?;
        build(self.builder.build_conditional_branch(present, some, none))?;
        self.builder.position_at_end(some);
        let payload = present_value(self)?;
        let value = match payload {
            BasicValueEnum::StructValue(value)
                if value.get_type() == self.nullable_type(payload_type) =>
            {
                value
            }
            payload => self.nullable_value(self.present_word(true), payload)?,
        };
        build(self.builder.build_unconditional_branch(done))?;
        let some_end = self
            .builder
            .get_insert_block()
            .expect("null-safe some block");
        self.builder.position_at_end(none);
        let absent = self.nullable_value(self.present_word(false), payload_type.const_zero())?;
        build(self.builder.build_unconditional_branch(done))?;
        let none_end = self
            .builder
            .get_insert_block()
            .expect("null-safe none block");
        self.builder.position_at_end(done);
        let phi = build(
            self.builder
                .build_phi(self.nullable_type(payload_type), "null-safe.nullable"),
        )?;
        phi.add_incoming(&[(&value, some_end), (&absent, none_end)]);
        let result = phi.as_basic_value().into_struct_value();
        Ok(result)
    }

    fn lower_nullable_value_map(
        &mut self,
        payload_type: BasicTypeEnum<'ctx>,
        value: StructValue<'ctx>,
        present_value: impl FnOnce(
            &mut Self,
            BasicValueEnum<'ctx>,
        ) -> Result<BasicValueEnum<'ctx>, BackendError>,
    ) -> Result<StructValue<'ctx>, BackendError> {
        let (present, source_payload) = self.nullable_parts(value)?;
        let function = current_function(&self.builder)?;
        let some = self
            .context
            .append_basic_block(function, "nullable.map.some");
        let none = self
            .context
            .append_basic_block(function, "nullable.map.none");
        let done = self
            .context
            .append_basic_block(function, "nullable.map.done");
        let is_present = build(self.builder.build_int_compare(
            IntPredicate::NE,
            present,
            present.get_type().const_zero(),
            "nullable.map.present",
        ))?;
        build(
            self.builder
                .build_conditional_branch(is_present, some, none),
        )?;
        self.builder.position_at_end(some);
        let payload = present_value(self, source_payload)?;
        let value = self.nullable_value(self.present_word(true), payload)?;
        build(self.builder.build_unconditional_branch(done))?;
        let some_end = self
            .builder
            .get_insert_block()
            .expect("nullable map some block");
        self.builder.position_at_end(none);
        let absent = self.nullable_value(self.present_word(false), payload_type.const_zero())?;
        build(self.builder.build_unconditional_branch(done))?;
        let none_end = self
            .builder
            .get_insert_block()
            .expect("nullable map none block");
        self.builder.position_at_end(done);
        let phi = build(
            self.builder
                .build_phi(self.nullable_type(payload_type), "nullable.map.value"),
        )?;
        phi.add_incoming(&[(&value, some_end), (&absent, none_end)]);
        Ok(phi.as_basic_value().into_struct_value())
    }

    fn lower_null_safe_statement_call(
        &mut self,
        object: &mir::NullableClassExpression,
        function: mir::FunctionId,
        args: &[mir::Rvalue],
    ) -> Result<(), BackendError> {
        let receiver = self.lower_nullable_class_expression(object)?;
        if let Some(class) = object.owned_temporary_class() {
            self.defer_or_drop_class_temporary(receiver, class)?;
        }
        let current = current_function(&self.builder)?;
        let call_block = self.context.append_basic_block(current, "null-safe.call");
        let done = self.context.append_basic_block(current, "null-safe.done");
        let present = build(
            self.builder
                .build_is_not_null(receiver, "null-safe.present"),
        )?;
        build(
            self.builder
                .build_conditional_branch(present, call_block, done),
        )?;
        self.builder.position_at_end(call_block);
        let expects_result = !matches!(
            function_in(self.program, function)?.return_type,
            mir::ReturnType::Void
        );
        let _ = self.lower_method_call(receiver, function, args, expects_result)?;
        build(self.builder.build_unconditional_branch(done))?;
        self.builder.position_at_end(done);
        Ok(())
    }

    fn runtime_function(
        &self,
        name: &str,
        params: &[BasicMetadataTypeEnum<'ctx>],
        result: Option<BasicTypeEnum<'ctx>>,
    ) -> FunctionValue<'ctx> {
        let function = self.module.get_function(name).unwrap_or_else(|| {
            let ty = match result {
                Some(result) => result.fn_type(params, false),
                None => self.context.void_type().fn_type(params, false),
            };
            self.module.add_function(name, ty, Some(Linkage::External))
        });
        if name == "dr_v2_panic" {
            for name in ["cold", "noreturn"] {
                let kind = inkwell::attributes::Attribute::get_named_enum_kind_id(name);
                function.add_attribute(
                    AttributeLoc::Function,
                    self.context.create_enum_attribute(kind, 0),
                );
            }
        }
        function
    }

    fn call_runtime(
        &self,
        name: &str,
        params: &[BasicMetadataTypeEnum<'ctx>],
        result: Option<BasicTypeEnum<'ctx>>,
        values: &[BasicMetadataValueEnum<'ctx>],
    ) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let function = self.runtime_function(name, params, result);
        let call = build(self.builder.build_call(function, values, name))?;
        Ok(call.try_as_basic_value().basic())
    }

    fn release_string(&self, value: PointerValue<'ctx>) -> Result<(), BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let _ = self.call_runtime(STRING_RELEASE, &[pointer.into()], None, &[value.into()])?;
        Ok(())
    }

    fn retain_string(&self, value: PointerValue<'ctx>) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        Ok(self
            .call_runtime(
                STRING_RETAIN,
                &[pointer.into()],
                Some(pointer.into()),
                &[value.into()],
            )?
            .ok_or_else(|| backend_failure("string retain produced no result"))?
            .into_pointer_value())
    }

    fn lower_string_intrinsic_call(
        &mut self,
        call: &mir::StringIntrinsicCall,
    ) -> Result<BasicValueEnum<'ctx>, BackendError> {
        use mir::StringIntrinsicKind as Kind;

        self.set_active_panic_site(call.span)?;
        let pointer = self.context.ptr_type(AddressSpace::default());
        let usize_type = self.context.ptr_sized_int_type(self.target_data, None);
        let i8_type = self.context.i8_type();
        let i64_type = self.context.i64_type();
        let mut arguments = Vec::with_capacity(call.args.len());
        let mut owned_strings = Vec::new();
        for argument in &call.args {
            let value = self.lower_rvalue(argument)?;
            match argument.ty() {
                mir::Type::String => owned_strings.push(value.into_pointer_value()),
                mir::Type::NullableString => owned_strings.push(
                    self.nullable_parts(value.into_struct_value())?
                        .1
                        .into_pointer_value(),
                ),
                _ => {}
            }
            arguments.push(value);
        }
        let argument = |index: usize| -> Result<BasicValueEnum<'ctx>, BackendError> {
            arguments
                .get(index)
                .copied()
                .ok_or_else(|| malformed_mir("String intrinsic argument is missing"))
        };

        let result: BasicValueEnum<'ctx> = match call.kind {
            Kind::GraphemeLength | Kind::ByteLength => {
                let name = if call.kind == Kind::GraphemeLength {
                    STRING_GRAPHEME_LENGTH
                } else {
                    STRING_BYTE_LENGTH
                };
                let value = self
                    .call_runtime(
                        name,
                        &[pointer.into()],
                        Some(usize_type.into()),
                        &[argument(0)?.into()],
                    )?
                    .ok_or_else(|| {
                        backend_failure("String length runtime call produced no result")
                    })?
                    .into_int_value();
                if usize_type.get_bit_width() == 64 {
                    value.into()
                } else {
                    build(
                        self.builder
                            .build_int_z_extend(value, i64_type, "string.length.i64"),
                    )?
                    .into()
                }
            }
            Kind::IsEmpty => self
                .call_runtime(
                    STRING_IS_EMPTY,
                    &[pointer.into()],
                    Some(i8_type.into()),
                    &[argument(0)?.into()],
                )?
                .ok_or_else(|| backend_failure("String empty test produced no result"))?,
            Kind::ToBytes => self
                .call_runtime(
                    STRING_TO_BYTES,
                    &[pointer.into()],
                    Some(pointer.into()),
                    &[argument(0)?.into()],
                )?
                .ok_or_else(|| backend_failure("String bytes conversion produced no result"))?,
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
                self.call_runtime(
                    name,
                    &[pointer.into(), pointer.into()],
                    Some(pointer.into()),
                    &[self.current_frame.into(), argument(0)?.into()],
                )?
                .ok_or_else(|| backend_failure("String transform produced no result"))?
            }
            Kind::ContainsIgnoreCase | Kind::StartsWithIgnoreCase | Kind::EndsWithIgnoreCase => {
                let name = match call.kind {
                    Kind::ContainsIgnoreCase => STRING_CONTAINS_IGNORE_CASE,
                    Kind::StartsWithIgnoreCase => STRING_STARTS_WITH_IGNORE_CASE,
                    Kind::EndsWithIgnoreCase => STRING_ENDS_WITH_IGNORE_CASE,
                    _ => unreachable!(),
                };
                self.call_runtime(
                    name,
                    &[pointer.into(), pointer.into(), pointer.into()],
                    Some(i8_type.into()),
                    &[
                        self.current_frame.into(),
                        argument(0)?.into(),
                        argument(1)?.into(),
                    ],
                )?
                .ok_or_else(|| {
                    backend_failure("String case-insensitive predicate produced no result")
                })?
            }
            Kind::Contains | Kind::StartsWith | Kind::EndsWith => {
                let name = match call.kind {
                    Kind::Contains => STRING_CONTAINS,
                    Kind::StartsWith => STRING_STARTS_WITH,
                    Kind::EndsWith => STRING_ENDS_WITH,
                    _ => unreachable!(),
                };
                self.call_runtime(
                    name,
                    &[pointer.into(), pointer.into()],
                    Some(i8_type.into()),
                    &[argument(0)?.into(), argument(1)?.into()],
                )?
                .ok_or_else(|| backend_failure("String predicate produced no result"))?
            }
            Kind::EqualsIgnoreCase => self
                .call_runtime(
                    STRING_EQUALS_IGNORE_CASE,
                    &[pointer.into(), pointer.into(), pointer.into()],
                    Some(i8_type.into()),
                    &[
                        self.current_frame.into(),
                        argument(0)?.into(),
                        argument(1)?.into(),
                    ],
                )?
                .ok_or_else(|| backend_failure("String case-fold comparison produced no result"))?,
            Kind::IndexOf
            | Kind::LastIndexOf
            | Kind::IndexOfIgnoreCase
            | Kind::LastIndexOfIgnoreCase => {
                let found_slot = self.entry_alloca(i8_type, "string.search.found")?;
                let (name, with_frame) = match call.kind {
                    Kind::IndexOf => (STRING_INDEX_OF, false),
                    Kind::LastIndexOf => (STRING_LAST_INDEX_OF, false),
                    Kind::IndexOfIgnoreCase => (STRING_INDEX_OF_IGNORE_CASE, true),
                    Kind::LastIndexOfIgnoreCase => (STRING_LAST_INDEX_OF_IGNORE_CASE, true),
                    _ => unreachable!(),
                };
                let mut params: Vec<BasicMetadataTypeEnum<'ctx>> = Vec::with_capacity(4);
                let mut values = Vec::with_capacity(4);
                if with_frame {
                    params.push(pointer.into());
                    values.push(self.current_frame.into());
                }
                params.push(pointer.into());
                params.push(pointer.into());
                params.push(pointer.into());
                values.push(argument(0)?.into());
                values.push(argument(1)?.into());
                values.push(found_slot.into());
                let payload = self
                    .call_runtime(name, &params, Some(i64_type.into()), &values)?
                    .ok_or_else(|| backend_failure("String search produced no result"))?;
                let found = build(self.builder.build_load(
                    i8_type,
                    found_slot,
                    "string.search.found.value",
                ))?
                .into_int_value();
                let present = build(self.builder.build_int_z_extend(
                    found,
                    usize_type,
                    "string.search.present",
                ))?;
                self.nullable_value(present, payload)?.into()
            }
            Kind::CountOccurrences => self
                .call_runtime(
                    STRING_COUNT_OCCURRENCES,
                    &[pointer.into(), pointer.into(), pointer.into()],
                    Some(i64_type.into()),
                    &[
                        self.current_frame.into(),
                        argument(0)?.into(),
                        argument(1)?.into(),
                    ],
                )?
                .ok_or_else(|| backend_failure("String occurrence count produced no result"))?,
            Kind::Replace => self
                .call_runtime(
                    STRING_REPLACE,
                    &[
                        pointer.into(),
                        pointer.into(),
                        pointer.into(),
                        pointer.into(),
                    ],
                    Some(pointer.into()),
                    &[
                        self.current_frame.into(),
                        argument(0)?.into(),
                        argument(1)?.into(),
                        argument(2)?.into(),
                    ],
                )?
                .ok_or_else(|| backend_failure("String replacement produced no result"))?,
            Kind::Split => self
                .call_runtime(
                    STRING_SPLIT,
                    &[pointer.into(), pointer.into(), pointer.into()],
                    Some(pointer.into()),
                    &[
                        self.current_frame.into(),
                        argument(0)?.into(),
                        argument(1)?.into(),
                    ],
                )?
                .ok_or_else(|| backend_failure("String split produced no result"))?,
            Kind::Join => self
                .call_runtime(
                    STRING_JOIN,
                    &[pointer.into(), pointer.into(), pointer.into()],
                    Some(pointer.into()),
                    &[
                        self.current_frame.into(),
                        argument(0)?.into(),
                        argument(1)?.into(),
                    ],
                )?
                .ok_or_else(|| backend_failure("String join produced no result"))?,
            Kind::Slice => {
                let (has_length, length) = self.nullable_parts(argument(2)?.into_struct_value())?;
                let length = length.into_int_value();
                let has_length = build(self.builder.build_int_truncate(
                    has_length,
                    i8_type,
                    "string.slice.has-length",
                ))?;
                let has_length_flag = build(self.builder.build_int_compare(
                    IntPredicate::NE,
                    has_length,
                    i8_type.const_zero(),
                    "string.slice.has-length.flag",
                ))?;
                let negative = build(self.builder.build_int_compare(
                    IntPredicate::SLT,
                    length,
                    i64_type.const_zero(),
                    "string.slice.negative",
                ))?;
                let invalid_length = build(self.builder.build_and(
                    has_length_flag,
                    negative,
                    "string.slice.invalid-length",
                ))?;
                self.lower_panic_if_signed_fact(
                    invalid_length,
                    "P1201",
                    doria_diagnostic_catalogue::STRING_SLICE_LENGTH_FACT,
                    length,
                    call.argument_spans.get(2).copied().unwrap_or(call.span),
                )?;
                self.set_active_panic_site(call.span)?;
                self.call_runtime(
                    STRING_SLICE,
                    &[
                        pointer.into(),
                        pointer.into(),
                        i64_type.into(),
                        i64_type.into(),
                        i8_type.into(),
                    ],
                    Some(pointer.into()),
                    &[
                        self.current_frame.into(),
                        argument(0)?.into(),
                        argument(1)?.into(),
                        length.into(),
                        has_length.into(),
                    ],
                )?
                .ok_or_else(|| backend_failure("String slice produced no result"))?
            }
            Kind::Repeat => {
                let count = argument(1)?.into_int_value();
                let negative = build(self.builder.build_int_compare(
                    IntPredicate::SLT,
                    count,
                    i64_type.const_zero(),
                    "string.repeat.negative",
                ))?;
                self.lower_panic_if_signed_fact(
                    negative,
                    "P1204",
                    doria_diagnostic_catalogue::STRING_REPETITION_COUNT_FACT,
                    count,
                    call.argument_spans.get(1).copied().unwrap_or(call.span),
                )?;
                self.set_active_panic_site(call.span)?;
                self.call_runtime(
                    STRING_REPEAT,
                    &[pointer.into(), pointer.into(), i64_type.into()],
                    Some(pointer.into()),
                    &[self.current_frame.into(), argument(0)?.into(), count.into()],
                )?
                .ok_or_else(|| backend_failure("String repetition produced no result"))?
            }
            Kind::PadStart | Kind::PadEnd => {
                let name = if call.kind == Kind::PadStart {
                    STRING_PAD_START
                } else {
                    STRING_PAD_END
                };
                let target_length = argument(1)?.into_int_value();
                let negative = build(self.builder.build_int_compare(
                    IntPredicate::SLT,
                    target_length,
                    i64_type.const_zero(),
                    "string.padding.negative",
                ))?;
                self.lower_panic_if_signed_fact(
                    negative,
                    "P1202",
                    doria_diagnostic_catalogue::STRING_PADDING_REQUESTED_LENGTH_FACT,
                    target_length,
                    call.argument_spans.get(1).copied().unwrap_or(call.span),
                )?;
                let current_length_word = self
                    .call_runtime(
                        STRING_GRAPHEME_LENGTH,
                        &[pointer.into()],
                        Some(usize_type.into()),
                        &[argument(0)?.into()],
                    )?
                    .ok_or_else(|| backend_failure("String length produced no result"))?
                    .into_int_value();
                let current_length = build(self.builder.build_int_z_extend_or_bit_cast(
                    current_length_word,
                    i64_type,
                    "string.padding.current-length",
                ))?;
                let needs_padding = build(self.builder.build_int_compare(
                    IntPredicate::SGT,
                    target_length,
                    current_length,
                    "string.padding.required",
                ))?;
                let padding_length = self
                    .call_runtime(
                        STRING_GRAPHEME_LENGTH,
                        &[pointer.into()],
                        Some(usize_type.into()),
                        &[argument(2)?.into()],
                    )?
                    .ok_or_else(|| backend_failure("String length produced no result"))?
                    .into_int_value();
                let padding_empty = build(self.builder.build_int_compare(
                    IntPredicate::EQ,
                    padding_length,
                    usize_type.const_zero(),
                    "string.padding.empty",
                ))?;
                let invalid_padding = build(self.builder.build_and(
                    needs_padding,
                    padding_empty,
                    "string.padding.invalid",
                ))?;
                self.lower_padding_empty_panic_if(
                    invalid_padding,
                    call.kind == Kind::PadStart,
                    (
                        argument(0)?.into_pointer_value(),
                        current_length_word,
                        target_length,
                        padding_length,
                    ),
                    call.argument_spans.get(2).copied().unwrap_or(call.span),
                )?;
                self.set_active_panic_site(call.span)?;
                self.call_runtime(
                    name,
                    &[
                        pointer.into(),
                        pointer.into(),
                        i64_type.into(),
                        pointer.into(),
                    ],
                    Some(pointer.into()),
                    &[
                        self.current_frame.into(),
                        argument(0)?.into(),
                        argument(1)?.into(),
                        argument(2)?.into(),
                    ],
                )?
                .ok_or_else(|| backend_failure("String padding produced no result"))?
            }
            Kind::FromBytes => {
                let payload = self
                    .call_runtime(
                        STRING_FROM_BYTES,
                        &[pointer.into()],
                        Some(pointer.into()),
                        &[argument(0)?.into()],
                    )?
                    .ok_or_else(|| backend_failure("String UTF-8 validation produced no result"))?
                    .into_pointer_value();
                let present = build(
                    self.builder
                        .build_is_not_null(payload, "string.from-bytes.present"),
                )?;
                let present = build(self.builder.build_int_z_extend(
                    present,
                    usize_type,
                    "string.from-bytes.present.word",
                ))?;
                self.nullable_value(present, payload.into())?.into()
            }
        };

        for string in owned_strings {
            self.release_string(string)?;
        }
        for index in ordered_owned_argument_indices(&call.args) {
            if let Some(collection) = call.args[index].owned_temporary_collection() {
                self.defer_or_drop_collection_temporary(
                    arguments[index].into_pointer_value(),
                    collection,
                )?;
            }
        }
        Ok(result)
    }

    fn drop_shared_value(
        &mut self,
        value: PointerValue<'ctx>,
        weak: bool,
    ) -> Result<(), BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let function = current_function(&self.builder)?;
        let drop_block = self
            .context
            .append_basic_block(function, "shared.drop.present");
        let done = self
            .context
            .append_basic_block(function, "shared.drop.done");
        let present = build(
            self.builder
                .build_is_not_null(value, "shared.drop.has_value"),
        )?;
        build(
            self.builder
                .build_conditional_branch(present, drop_block, done),
        )?;
        self.builder.position_at_end(drop_block);
        if weak {
            let _ = self.call_runtime(
                SHARED_RELEASE_WEAK,
                &[pointer.into()],
                None,
                &[value.into()],
            )?;
        } else {
            let _ = self.call_runtime(
                SHARED_RELEASE,
                &[pointer.into(), pointer.into()],
                None,
                &[self.current_frame.into(), value.into()],
            )?;
        }
        build(self.builder.build_unconditional_branch(done))?;
        self.builder.position_at_end(done);
        Ok(())
    }

    fn drop_writable_shared_local(
        &mut self,
        local: mir::LocalId,
        symbol: &'static str,
    ) -> Result<(), BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let slot = local_slot(&self.local_slots, local)?;
        let value = build(
            self.builder
                .build_load(pointer, slot, "writable.shared.drop"),
        )?
        .into_pointer_value();
        build(self.builder.build_store(slot, pointer.const_null()))?;
        self.drop_writable_shared_value(value, symbol)
    }

    fn drop_writable_shared_value(
        &mut self,
        value: PointerValue<'ctx>,
        symbol: &'static str,
    ) -> Result<(), BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let function = current_function(&self.builder)?;
        let drop_block = self
            .context
            .append_basic_block(function, "writable.shared.drop.present");
        let done = self
            .context
            .append_basic_block(function, "writable.shared.drop.done");
        let present = build(
            self.builder
                .build_is_not_null(value, "writable.shared.drop.has_value"),
        )?;
        build(
            self.builder
                .build_conditional_branch(present, drop_block, done),
        )?;
        self.builder.position_at_end(drop_block);
        if symbol == WRITABLE_SHARED_RELEASE_WEAK {
            let _ = self.call_runtime(symbol, &[pointer.into()], None, &[value.into()])?;
        } else {
            let _ = self.call_runtime(
                symbol,
                &[pointer.into(), pointer.into()],
                None,
                &[self.current_frame.into(), value.into()],
            )?;
        }
        build(self.builder.build_unconditional_branch(done))?;
        self.builder.position_at_end(done);
        Ok(())
    }

    fn mixed_tag_value(&self, tag: mir::MixedTag) -> (u8, u32) {
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
        }
    }

    fn lower_mixed_box(
        &mut self,
        tag: mir::MixedTag,
        payload: BasicValueEnum<'ctx>,
        payload_owned: bool,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let i8_type = self.context.i8_type();
        let i32_type = self.context.i32_type();
        let i64_type = self.context.i64_type();
        let payload = self.value_to_collection_word(payload, tag.ty())?;
        let (tag, type_id) = self.mixed_tag_value(tag);
        Ok(self
            .call_runtime(
                if payload_owned {
                    MIXED_NEW
                } else {
                    MIXED_NEW_BORROWED
                },
                &[i8_type.into(), i32_type.into(), i64_type.into()],
                Some(pointer.into()),
                &[
                    i8_type.const_int(u64::from(tag), false).into(),
                    i32_type.const_int(u64::from(type_id), false).into(),
                    payload.into(),
                ],
            )?
            .ok_or_else(|| backend_failure("mixed allocation produced no result"))?
            .into_pointer_value())
    }

    fn lower_mixed_expression(
        &mut self,
        expression: &mir::MixedExpression,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        match expression {
            mir::MixedExpression::Local { local, transfer } => {
                let slot = local_slot(&self.local_slots, *local)?;
                let value = build(self.builder.build_load(pointer, slot, "mixed.local"))?
                    .into_pointer_value();
                if *transfer {
                    build(self.builder.build_store(slot, pointer.const_null()))?;
                }
                Ok(value)
            }
            mir::MixedExpression::Property { object, property } => {
                let address = self.lower_property_address(*object, *property)?;
                Ok(
                    build(self.builder.build_load(pointer, address, "mixed.property"))?
                        .into_pointer_value(),
                )
            }
            mir::MixedExpression::Call { function, args, .. } => Ok(self
                .lower_call(*function, args, true)?
                .ok_or_else(|| malformed_mir("mixed call produced no result"))?
                .into_pointer_value()),
            mir::MixedExpression::BoxValue(value) => {
                let tag = match value.ty() {
                    mir::ScalarType::Bool => mir::MixedTag::Bool,
                    mir::ScalarType::Integer(ty) => mir::MixedTag::Integer(ty),
                    mir::ScalarType::Float(ty) => mir::MixedTag::Float(ty),
                    mir::ScalarType::Enum(enum_id) => mir::MixedTag::Enum(enum_id),
                };
                let payload = self.lower_value_expression(value)?;
                self.lower_mixed_box(tag, payload, false)
            }
            mir::MixedExpression::BoxString {
                value,
                payload_owned,
            } => {
                let payload = self.lower_string_expression(value)?;
                let mixed =
                    self.lower_mixed_box(mir::MixedTag::String, payload.into(), *payload_owned)?;
                if !payload_owned {
                    self.release_string(payload)?;
                }
                Ok(mixed)
            }
            mir::MixedExpression::BoxClass {
                value,
                payload_owned,
            } => {
                let class = value.class();
                let payload = self.lower_class_expression(value)?;
                self.lower_mixed_box(mir::MixedTag::Class(class), payload.into(), *payload_owned)
            }
            mir::MixedExpression::BoxError { value } => {
                let value = self.lower_error_expression(value)?;
                let source = self.entry_alloca(error_carrier_type(self.context), "mixed.error")?;
                build(self.builder.build_store(source, value))?;
                let (tag, type_id) = self.mixed_tag_value(mir::MixedTag::Error);
                let usize_type = self.context.ptr_sized_int_type(self.target_data, None);
                Ok(self
                    .call_runtime(
                        MIXED_NEW_AGGREGATE,
                        &[
                            self.context.i8_type().into(),
                            self.context.i32_type().into(),
                            pointer.into(),
                            usize_type.into(),
                            usize_type.into(),
                        ],
                        Some(pointer.into()),
                        &[
                            self.context
                                .i8_type()
                                .const_int(u64::from(tag), false)
                                .into(),
                            self.context
                                .i32_type()
                                .const_int(u64::from(type_id), false)
                                .into(),
                            source.into(),
                            usize_type
                                .const_int(
                                    self.target_data
                                        .get_store_size(&error_carrier_type(self.context)),
                                    false,
                                )
                                .into(),
                            usize_type
                                .const_int(
                                    u64::from(
                                        self.target_data
                                            .get_abi_alignment(&error_carrier_type(self.context)),
                                    ),
                                    false,
                                )
                                .into(),
                        ],
                    )?
                    .ok_or_else(|| backend_failure("mixed Error allocation produced no result"))?
                    .into_pointer_value())
            }
            mir::MixedExpression::BoxPayloadEnum { value } => {
                let ty = value.ty();
                let source = self.lower_payload_enum_expression(value)?;
                let pointer = self.context.ptr_type(AddressSpace::default());
                let i8_type = self.context.i8_type();
                let i32_type = self.context.i32_type();
                let usize_type = self.context.ptr_sized_int_type(self.target_data, None);
                let (tag, type_id) = self.mixed_tag_value(mir::MixedTag::PayloadEnum(ty));
                Ok(self
                    .call_runtime(
                        MIXED_NEW_AGGREGATE,
                        &[
                            i8_type.into(),
                            i32_type.into(),
                            pointer.into(),
                            usize_type.into(),
                            usize_type.into(),
                        ],
                        Some(pointer.into()),
                        &[
                            i8_type.const_int(u64::from(tag), false).into(),
                            i32_type.const_int(u64::from(type_id), false).into(),
                            source.into(),
                            usize_type.const_int(u64::from(ty.size), false).into(),
                            usize_type.const_int(u64::from(ty.align), false).into(),
                        ],
                    )?
                    .ok_or_else(|| {
                        backend_failure("mixed aggregate allocation produced no result")
                    })?
                    .into_pointer_value())
            }
            mir::MixedExpression::CollectionIndex {
                positional,
                collection,
                index,
                transfer,
                remove,
            } => {
                let value = self
                    .lower_collection_index(*collection, index, *remove, *positional)?
                    .into_pointer_value();
                if *transfer && !*remove {
                    // Owning index read (`mixed $x = $items[0]`): clone the collection's box
                    // into an owned handle that shares the payload owner with the element
                    // that remains in the collection.
                    self.own_mixed_value(value, mir::MixedOwnership::None)
                } else {
                    // `removeAt` popped the element out, so the box is already ours and the
                    // collection no longer claims the payload; a borrow read keeps the box.
                    Ok(value)
                }
            }
        }
    }

    fn lower_nullable_mixed_expression(
        &mut self,
        expression: &mir::NullableMixedExpression,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        match expression {
            mir::NullableMixedExpression::Null => Ok(pointer.const_null()),
            mir::NullableMixedExpression::Mixed(value) => self.lower_mixed_expression(value),
            mir::NullableMixedExpression::BoxNullablePayloadEnum(value) => {
                let ty = value.ty();
                let source = self.lower_nullable_payload_enum_expression(value)?;
                let present = build(self.builder.build_load(
                    self.context.i8_type(),
                    source,
                    "nullable-mixed.payload-enum.present",
                ))?
                .into_int_value();
                let function = current_function(&self.builder)?;
                let box_value = self
                    .context
                    .append_basic_block(function, "nullable-mixed.payload-enum.box");
                let absent = self
                    .context
                    .append_basic_block(function, "nullable-mixed.payload-enum.absent");
                let done = self
                    .context
                    .append_basic_block(function, "nullable-mixed.payload-enum.done");
                let is_present = build(self.builder.build_int_compare(
                    IntPredicate::NE,
                    present,
                    self.context.i8_type().const_zero(),
                    "nullable-mixed.payload-enum.is-present",
                ))?;
                build(
                    self.builder
                        .build_conditional_branch(is_present, box_value, absent),
                )?;
                self.builder.position_at_end(box_value);
                let payload = self.byte_offset(
                    source,
                    ty.nullable_payload_offset,
                    "nullable-mixed.payload-enum.value",
                )?;
                let i8_type = self.context.i8_type();
                let i32_type = self.context.i32_type();
                let usize_type = self.context.ptr_sized_int_type(self.target_data, None);
                let (tag, type_id) = self.mixed_tag_value(mir::MixedTag::PayloadEnum(ty));
                let boxed = self
                    .call_runtime(
                        MIXED_NEW_AGGREGATE,
                        &[
                            i8_type.into(),
                            i32_type.into(),
                            pointer.into(),
                            usize_type.into(),
                            usize_type.into(),
                        ],
                        Some(pointer.into()),
                        &[
                            i8_type.const_int(u64::from(tag), false).into(),
                            i32_type.const_int(u64::from(type_id), false).into(),
                            payload.into(),
                            usize_type.const_int(u64::from(ty.size), false).into(),
                            usize_type.const_int(u64::from(ty.align), false).into(),
                        ],
                    )?
                    .ok_or_else(|| {
                        backend_failure("nullable mixed aggregate allocation produced no result")
                    })?
                    .into_pointer_value();
                build(self.builder.build_unconditional_branch(done))?;
                let boxed_block = self
                    .builder
                    .get_insert_block()
                    .ok_or_else(|| backend_failure("nullable mixed aggregate block is missing"))?;
                self.builder.position_at_end(absent);
                build(self.builder.build_unconditional_branch(done))?;
                self.builder.position_at_end(done);
                let result = build(
                    self.builder
                        .build_phi(pointer, "nullable-mixed.payload-enum"),
                )?;
                result.add_incoming(&[(&boxed, boxed_block), (&pointer.const_null(), absent)]);
                Ok(result.as_basic_value().into_pointer_value())
            }
            mir::NullableMixedExpression::Local { local, transfer } => {
                let slot = local_slot(&self.local_slots, *local)?;
                let value = build(
                    self.builder
                        .build_load(pointer, slot, "nullable-mixed.local"),
                )?
                .into_pointer_value();
                if *transfer {
                    build(self.builder.build_store(slot, pointer.const_null()))?;
                }
                Ok(value)
            }
            mir::NullableMixedExpression::Property { object, property } => {
                let address = self.lower_property_address(*object, *property)?;
                Ok(build(
                    self.builder
                        .build_load(pointer, address, "nullable-mixed.property"),
                )?
                .into_pointer_value())
            }
            mir::NullableMixedExpression::Call { function, args, .. } => Ok(self
                .lower_call(*function, args, true)?
                .ok_or_else(|| malformed_mir("nullable-mixed call produced no result"))?
                .into_pointer_value()),
            mir::NullableMixedExpression::Coalesce {
                left,
                right,
                transfer,
            } => {
                let left_ownership = left.ownership();
                let right_ownership = right.ownership();
                let left = self.lower_nullable_mixed_expression(left)?;
                self.lower_nullable_mixed_coalesce(
                    left,
                    !transfer,
                    left_ownership,
                    right_ownership,
                    |lowerer| Ok(lowerer.lower_nullable_mixed_expression(right)?.into()),
                )
            }
        }
    }

    fn lower_nullable_mixed_coalesce(
        &mut self,
        left: PointerValue<'ctx>,
        normalize_ownership: bool,
        left_ownership: mir::MixedOwnership,
        right_ownership: mir::MixedOwnership,
        fallback: impl FnOnce(&mut Self) -> Result<BasicValueEnum<'ctx>, BackendError>,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let function = current_function(&self.builder)?;
        let some = self
            .context
            .append_basic_block(function, "nullable-mixed.coalesce.some");
        let none = self
            .context
            .append_basic_block(function, "nullable-mixed.coalesce.none");
        let done = self
            .context
            .append_basic_block(function, "nullable-mixed.coalesce.done");
        let present = build(
            self.builder
                .build_is_not_null(left, "nullable-mixed.present"),
        )?;
        build(self.builder.build_conditional_branch(present, some, none))?;
        self.builder.position_at_end(some);
        let left = if normalize_ownership {
            self.own_mixed_value(left, left_ownership)?
        } else {
            left
        };
        build(self.builder.build_unconditional_branch(done))?;
        let some_end = self
            .builder
            .get_insert_block()
            .expect("nullable-mixed coalesce some block");
        self.builder.position_at_end(none);
        let fallback = fallback(self)?.into_pointer_value();
        let fallback = if normalize_ownership {
            self.own_nullable_mixed_value(fallback, right_ownership)?
        } else {
            fallback
        };
        build(self.builder.build_unconditional_branch(done))?;
        let none_end = self
            .builder
            .get_insert_block()
            .expect("nullable-mixed coalesce none block");
        self.builder.position_at_end(done);
        let phi = build(self.builder.build_phi(pointer, "nullable-mixed.coalesce"))?;
        phi.add_incoming(&[(&left, some_end), (&fallback, none_end)]);
        Ok(phi.as_basic_value().into_pointer_value())
    }

    fn lower_mixed_payload(
        &mut self,
        local: mir::LocalId,
        tag: mir::MixedTag,
    ) -> Result<BasicValueEnum<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let i64_type = self.context.i64_type();
        let mixed = build(self.builder.build_load(
            pointer,
            local_slot(&self.local_slots, local)?,
            "mixed.payload.local",
        ))?
        .into_pointer_value();
        let payload = self
            .call_runtime(
                MIXED_PAYLOAD,
                &[pointer.into()],
                Some(i64_type.into()),
                &[mixed.into()],
            )?
            .ok_or_else(|| backend_failure("mixed payload read produced no result"))?
            .into_int_value();
        self.collection_word_to_value(payload, tag.ty())
    }

    fn lower_take_mixed_payload(
        &mut self,
        local: mir::LocalId,
        tag: mir::MixedTag,
    ) -> Result<BasicValueEnum<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let i64_type = self.context.i64_type();
        let slot = local_slot(&self.local_slots, local)?;
        let mixed =
            build(self.builder.build_load(pointer, slot, "mixed.take.local"))?.into_pointer_value();
        let payload = self
            .call_runtime(
                MIXED_PAYLOAD,
                &[pointer.into()],
                Some(i64_type.into()),
                &[mixed.into()],
            )?
            .ok_or_else(|| backend_failure("mixed payload read produced no result"))?
            .into_int_value();
        let payload = self.collection_word_to_value(payload, tag.ty())?;
        build(self.builder.build_store(slot, pointer.const_null()))?;
        let owns_final = self
            .call_runtime(
                MIXED_RELEASE_OWNED,
                &[pointer.into()],
                Some(self.context.i8_type().into()),
                &[mixed.into()],
            )?
            .ok_or_else(|| backend_failure("mixed payload take released no ownership claim"))?
            .into_int_value();
        // A move-type payload may only be moved out when this box holds the final owning
        // claim. If another box still shares the owner (e.g. read from a collection with
        // `mixed $x = $items[0]`) or the box only borrows its payload, `release_owned`
        // reports a non-final claim; transferring the payload anyway would double-free
        // when the other holder later drops it. Refuse it rather than corrupt memory.
        if matches!(tag, mir::MixedTag::String | mir::MixedTag::Class(_)) {
            let not_final = build(self.builder.build_int_compare(
                IntPredicate::EQ,
                owns_final,
                self.context.i8_type().const_zero(),
                "mixed.take.shared",
            ))?;
            self.lower_panic_if_code_at_active_site(not_final, "P1321")?;
        }
        let _ = self.call_runtime(MIXED_FREE, &[pointer.into()], None, &[mixed.into()])?;
        Ok(payload)
    }

    fn lower_mixed_is(
        &mut self,
        mixed: &mir::MixedExpression,
        tag: mir::MixedTag,
    ) -> Result<IntValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let i8_type = self.context.i8_type();
        let i32_type = self.context.i32_type();
        let ownership = mixed.ownership();
        let mixed_value = self.lower_mixed_expression(mixed)?;
        let actual_tag = self
            .call_runtime(
                MIXED_TAG,
                &[pointer.into()],
                Some(i8_type.into()),
                &[mixed_value.into()],
            )?
            .ok_or_else(|| backend_failure("mixed tag read produced no result"))?
            .into_int_value();
        let (expected_tag, expected_type_id) = self.mixed_tag_value(tag);
        let tag_matches = build(self.builder.build_int_compare(
            IntPredicate::EQ,
            actual_tag,
            i8_type.const_int(u64::from(expected_tag), false),
            "mixed.tag.matches",
        ))?;
        let result = if matches!(
            tag,
            mir::MixedTag::Class(_) | mir::MixedTag::Enum(_) | mir::MixedTag::PayloadEnum(_)
        ) {
            let actual_type_id = self
                .call_runtime(
                    MIXED_TYPE_ID,
                    &[pointer.into()],
                    Some(i32_type.into()),
                    &[mixed_value.into()],
                )?
                .ok_or_else(|| backend_failure("mixed type-id read produced no result"))?
                .into_int_value();
            let type_matches = build(self.builder.build_int_compare(
                IntPredicate::EQ,
                actual_type_id,
                i32_type.const_int(u64::from(expected_type_id), false),
                "mixed.type.matches",
            ))?;
            build(
                self.builder
                    .build_and(tag_matches, type_matches, "mixed.matches"),
            )?
        } else {
            tag_matches
        };
        self.cleanup_mixed_temporary(mixed_value, ownership)?;
        Ok(result)
    }

    fn drop_mixed_value(&mut self, value: PointerValue<'ctx>) -> Result<(), BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let i8_type = self.context.i8_type();
        let i32_type = self.context.i32_type();
        let i64_type = self.context.i64_type();
        let function = current_function(&self.builder)?;
        let drop_block = self.context.append_basic_block(function, "mixed.drop");
        let done = self
            .context
            .append_basic_block(function, "mixed.drop.continue");
        let has_box = build(self.builder.build_is_not_null(value, "mixed.has_box"))?;
        build(
            self.builder
                .build_conditional_branch(has_box, drop_block, done),
        )?;
        self.builder.position_at_end(drop_block);
        let release_payload = self
            .call_runtime(
                MIXED_RELEASE_OWNED,
                &[pointer.into()],
                Some(i8_type.into()),
                &[value.into()],
            )?
            .ok_or_else(|| backend_failure("mixed ownership release produced no result"))?
            .into_int_value();
        let drop_payload = self
            .context
            .append_basic_block(function, "mixed.drop.payload");
        let free_shell = self
            .context
            .append_basic_block(function, "mixed.drop.shell");
        let release_payload = build(self.builder.build_int_compare(
            IntPredicate::NE,
            release_payload,
            i8_type.const_zero(),
            "mixed.drop.final-owner",
        ))?;
        build(
            self.builder
                .build_conditional_branch(release_payload, drop_payload, free_shell),
        )?;
        self.builder.position_at_end(drop_payload);
        let tag = self
            .call_runtime(
                MIXED_TAG,
                &[pointer.into()],
                Some(i8_type.into()),
                &[value.into()],
            )?
            .ok_or_else(|| backend_failure("mixed tag read produced no result"))?
            .into_int_value();
        let payload = self
            .call_runtime(
                MIXED_PAYLOAD,
                &[pointer.into()],
                Some(i64_type.into()),
                &[value.into()],
            )?
            .ok_or_else(|| backend_failure("mixed payload read produced no result"))?
            .into_int_value();

        let string_block = self
            .context
            .append_basic_block(function, "mixed.drop.string");
        let after_string = self
            .context
            .append_basic_block(function, "mixed.drop.after.string");
        let is_string = build(self.builder.build_int_compare(
            IntPredicate::EQ,
            tag,
            i8_type.const_int(u64::from(MIXED_TAG_STRING), false),
            "mixed.drop.is_string",
        ))?;
        build(
            self.builder
                .build_conditional_branch(is_string, string_block, after_string),
        )?;
        self.builder.position_at_end(string_block);
        let string = build(
            self.builder
                .build_int_to_ptr(payload, pointer, "mixed.string"),
        )?;
        self.release_string(string)?;
        build(self.builder.build_unconditional_branch(after_string))?;

        self.builder.position_at_end(after_string);
        let error_block = self
            .context
            .append_basic_block(function, "mixed.drop.error");
        let after_error = self
            .context
            .append_basic_block(function, "mixed.drop.after.error");
        let is_error = build(self.builder.build_int_compare(
            IntPredicate::EQ,
            tag,
            i8_type.const_int(u64::from(MIXED_TAG_ERROR), false),
            "mixed.drop.is_error",
        ))?;
        build(
            self.builder
                .build_conditional_branch(is_error, error_block, after_error),
        )?;
        self.builder.position_at_end(error_block);
        let error_address = build(self.builder.build_int_to_ptr(
            payload,
            pointer,
            "mixed.error.address",
        ))?;
        let error = build(self.builder.build_load(
            error_carrier_type(self.context),
            error_address,
            "mixed.error.value",
        ))?
        .into_struct_value();
        self.drop_error_value(error)?;
        build(self.builder.build_unconditional_branch(after_error))?;

        self.builder.position_at_end(after_error);
        let class_block = self
            .context
            .append_basic_block(function, "mixed.drop.class");
        let after_class = self
            .context
            .append_basic_block(function, "mixed.drop.after.class");
        let is_class = build(self.builder.build_int_compare(
            IntPredicate::EQ,
            tag,
            i8_type.const_int(u64::from(MIXED_TAG_CLASS), false),
            "mixed.drop.is_class",
        ))?;
        build(
            self.builder
                .build_conditional_branch(is_class, class_block, after_class),
        )?;
        self.builder.position_at_end(class_block);
        let type_id = self
            .call_runtime(
                MIXED_TYPE_ID,
                &[pointer.into()],
                Some(i32_type.into()),
                &[value.into()],
            )?
            .ok_or_else(|| backend_failure("mixed type-id read produced no result"))?
            .into_int_value();
        let object = build(
            self.builder
                .build_int_to_ptr(payload, pointer, "mixed.class"),
        )?;
        let classes = self
            .program
            .classes
            .iter()
            .map(|class| class.id)
            .collect::<Vec<_>>();
        if classes.is_empty() {
            build(self.builder.build_unconditional_branch(after_class))?;
        } else {
            let checks = classes
                .iter()
                .map(|_| {
                    self.context
                        .append_basic_block(function, "mixed.drop.class.check")
                })
                .collect::<Vec<_>>();
            build(self.builder.build_unconditional_branch(checks[0]))?;
            for (index, class) in classes.iter().enumerate() {
                let check = checks[index];
                let next = checks.get(index + 1).copied().unwrap_or(after_class);
                self.builder.position_at_end(check);
                let matched = build(self.builder.build_int_compare(
                    IntPredicate::EQ,
                    type_id,
                    i32_type.const_int(class.0 as u64, false),
                    "mixed.drop.class.matches",
                ))?;
                let drop_class = self
                    .context
                    .append_basic_block(function, "mixed.drop.class.payload");
                build(
                    self.builder
                        .build_conditional_branch(matched, drop_class, next),
                )?;
                self.builder.position_at_end(drop_class);
                self.drop_class_value_checked(object, *class)?;
                build(self.builder.build_unconditional_branch(after_class))?;
            }
        }
        self.builder.position_at_end(after_class);
        let payload_enum_block = self
            .context
            .append_basic_block(function, "mixed.drop.payload-enum");
        let after_payload_enum = self
            .context
            .append_basic_block(function, "mixed.drop.after.payload-enum");
        let is_payload_enum = build(self.builder.build_int_compare(
            IntPredicate::EQ,
            tag,
            i8_type.const_int(u64::from(MIXED_TAG_PAYLOAD_ENUM), false),
            "mixed.drop.is_payload_enum",
        ))?;
        build(self.builder.build_conditional_branch(
            is_payload_enum,
            payload_enum_block,
            after_payload_enum,
        ))?;
        self.builder.position_at_end(payload_enum_block);
        let type_id = self
            .call_runtime(
                MIXED_TYPE_ID,
                &[pointer.into()],
                Some(i32_type.into()),
                &[value.into()],
            )?
            .ok_or_else(|| backend_failure("mixed payload-enum type-id read produced no result"))?
            .into_int_value();
        let payload_address = build(self.builder.build_int_to_ptr(
            payload,
            pointer,
            "mixed.payload-enum.address",
        ))?;
        let payload_enums = self
            .program
            .enums
            .iter()
            .filter_map(|definition| definition.payload_type())
            .collect::<Vec<_>>();
        if payload_enums.is_empty() {
            build(self.builder.build_unconditional_branch(after_payload_enum))?;
        } else {
            let checks = payload_enums
                .iter()
                .map(|_| {
                    self.context
                        .append_basic_block(function, "mixed.drop.payload-enum.check")
                })
                .collect::<Vec<_>>();
            build(self.builder.build_unconditional_branch(checks[0]))?;
            for (index, payload_ty) in payload_enums.iter().enumerate() {
                let check = checks[index];
                let next = checks.get(index + 1).copied().unwrap_or(after_payload_enum);
                self.builder.position_at_end(check);
                let matched = build(self.builder.build_int_compare(
                    IntPredicate::EQ,
                    type_id,
                    i32_type.const_int(payload_ty.id.0 as u64, false),
                    "mixed.drop.payload-enum.matches",
                ))?;
                let drop_payload = self
                    .context
                    .append_basic_block(function, "mixed.drop.payload-enum.value");
                build(
                    self.builder
                        .build_conditional_branch(matched, drop_payload, next),
                )?;
                self.builder.position_at_end(drop_payload);
                self.drop_payload_enum_at(payload_address, *payload_ty, false)?;
                build(self.builder.build_unconditional_branch(after_payload_enum))?;
            }
        }
        self.builder.position_at_end(after_payload_enum);
        build(self.builder.build_unconditional_branch(free_shell))?;
        self.builder.position_at_end(free_shell);
        let _ = self.call_runtime(MIXED_FREE, &[pointer.into()], None, &[value.into()])?;
        build(self.builder.build_unconditional_branch(done))?;
        self.builder.position_at_end(done);
        Ok(())
    }

    fn free_mixed_shell(&mut self, value: PointerValue<'ctx>) -> Result<(), BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let _ = self.call_runtime(MIXED_FREE, &[pointer.into()], None, &[value.into()])?;
        Ok(())
    }

    fn cleanup_mixed_temporary(
        &mut self,
        value: PointerValue<'ctx>,
        ownership: mir::MixedOwnership,
    ) -> Result<(), BackendError> {
        match ownership {
            mir::MixedOwnership::None => Ok(()),
            mir::MixedOwnership::ShellOnly => self.free_mixed_shell(value),
            mir::MixedOwnership::Owned => self.drop_mixed_value(value),
        }
    }

    fn cleanup_mixed_temporary_if(
        &mut self,
        condition: IntValue<'ctx>,
        value: PointerValue<'ctx>,
        ownership: mir::MixedOwnership,
    ) -> Result<(), BackendError> {
        if !ownership.has_shell() {
            return Ok(());
        }
        let function = current_function(&self.builder)?;
        let cleanup = self
            .context
            .append_basic_block(function, "mixed.temporary.cleanup");
        let done = self
            .context
            .append_basic_block(function, "mixed.temporary.continue");
        build(
            self.builder
                .build_conditional_branch(condition, cleanup, done),
        )?;
        self.builder.position_at_end(cleanup);
        self.cleanup_mixed_temporary(value, ownership)?;
        build(self.builder.build_unconditional_branch(done))?;
        self.builder.position_at_end(done);
        Ok(())
    }

    fn own_mixed_value(
        &mut self,
        value: PointerValue<'ctx>,
        ownership: mir::MixedOwnership,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        if ownership == mir::MixedOwnership::Owned {
            return Ok(value);
        }
        let pointer = self.context.ptr_type(AddressSpace::default());
        let owned = self
            .call_runtime(
                MIXED_CLONE_OWNED,
                &[pointer.into()],
                Some(pointer.into()),
                &[value.into()],
            )?
            .ok_or_else(|| backend_failure("mixed ownership clone produced no result"))?
            .into_pointer_value();
        if ownership == mir::MixedOwnership::ShellOnly {
            self.free_mixed_shell(value)?;
        }
        Ok(owned)
    }

    fn own_nullable_mixed_value(
        &mut self,
        value: PointerValue<'ctx>,
        ownership: mir::MixedOwnership,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        if ownership == mir::MixedOwnership::Owned {
            return Ok(value);
        }
        let pointer = self.context.ptr_type(AddressSpace::default());
        let function = current_function(&self.builder)?;
        let own = self
            .context
            .append_basic_block(function, "nullable-mixed.own");
        let done = self
            .context
            .append_basic_block(function, "nullable-mixed.own.done");
        let current = self
            .builder
            .get_insert_block()
            .ok_or_else(|| backend_failure("nullable mixed ownership has no block"))?;
        let present = build(
            self.builder
                .build_is_not_null(value, "nullable-mixed.own.present"),
        )?;
        build(self.builder.build_conditional_branch(present, own, done))?;
        self.builder.position_at_end(own);
        let owned = self.own_mixed_value(value, ownership)?;
        build(self.builder.build_unconditional_branch(done))?;
        let owned_block = self
            .builder
            .get_insert_block()
            .ok_or_else(|| backend_failure("nullable mixed owned value has no block"))?;
        self.builder.position_at_end(done);
        let result = build(self.builder.build_phi(pointer, "nullable-mixed.owned"))?;
        result.add_incoming(&[(&value, current), (&owned, owned_block)]);
        Ok(result.as_basic_value().into_pointer_value())
    }

    fn cleanup_mixed_locals(&mut self) -> Result<(), BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let mixed_locals = self
            .function
            .locals
            .iter()
            .rev()
            .filter_map(|local| {
                (local.owned && matches!(local.ty, mir::Type::Mixed | mir::Type::NullableMixed))
                    .then_some(local.id)
            })
            .collect::<Vec<_>>();
        for local in mixed_locals {
            let slot = local_slot(&self.local_slots, local)?;
            let value = build(self.builder.build_load(pointer, slot, "mixed.cleanup"))?
                .into_pointer_value();
            build(self.builder.build_store(slot, pointer.const_null()))?;
            self.drop_mixed_value(value)?;
        }
        Ok(())
    }

    fn cleanup_string_locals(&self) -> Result<(), BackendError> {
        for local in &self.function.locals {
            if matches!(local.ty, mir::Type::String | mir::Type::NullableString) {
                let value = build(self.builder.build_load(
                    llvm_type(self.context, self.target_data, local.ty),
                    local_slot(&self.local_slots, local.id)?,
                    "string.cleanup",
                ))?;
                let value = if matches!(local.ty, mir::Type::NullableString) {
                    self.nullable_parts(value.into_struct_value())?
                        .1
                        .into_pointer_value()
                } else {
                    value.into_pointer_value()
                };
                self.release_string(value)?;
            }
        }
        Ok(())
    }

    fn cleanup_class_locals(&mut self) -> Result<(), BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let class_locals = self
            .function
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
            let slot = local_slot(&self.local_slots, local)?;
            let value = build(self.builder.build_load(pointer, slot, "class.cleanup"))?
                .into_pointer_value();
            build(self.builder.build_store(slot, pointer.const_null()))?;
            self.drop_class_value_checked(value, class)?;
        }
        Ok(())
    }

    fn flush_deferred_class_temporary_drops(&mut self) -> Result<(), BackendError> {
        let drops = std::mem::take(&mut self.deferred_class_temporary_drops);
        self.emit_deferred_class_temporary_drops(&drops)
    }

    fn emit_deferred_class_temporary_drops(
        &mut self,
        drops: &[(PointerValue<'ctx>, DeferredOwnedTemporary)],
    ) -> Result<(), BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        for (slot, temporary) in drops.iter().rev() {
            let value = build(
                self.builder
                    .build_load(pointer, *slot, "class.temporary.drop"),
            )?
            .into_pointer_value();
            build(self.builder.build_store(*slot, pointer.const_null()))?;
            match temporary {
                DeferredOwnedTemporary::Class(class) => {
                    self.drop_class_value_checked(value, *class)?;
                }
                DeferredOwnedTemporary::Collection(collection) => {
                    self.drop_collection_value(value, *collection)?;
                }
                DeferredOwnedTemporary::Mixed(ownership) => {
                    self.cleanup_mixed_temporary(value, *ownership)?;
                }
                DeferredOwnedTemporary::Shared(weak) => {
                    self.drop_shared_value(value, *weak)?;
                }
                DeferredOwnedTemporary::WritableShared(symbol) => {
                    self.drop_writable_shared_value(value, symbol)?;
                }
            }
        }
        Ok(())
    }

    fn defer_or_drop_class_temporary(
        &mut self,
        value: PointerValue<'ctx>,
        class: crate::class_layout::ClassId,
    ) -> Result<(), BackendError> {
        if !self.defer_class_temporary_drops {
            return self.drop_class_value_checked(value, class);
        }
        let slot = *self
            .deferred_class_temporary_slots
            .get(self.deferred_class_temporary_slot_cursor)
            .ok_or_else(|| malformed_mir("class temporary stack-slot capacity was exhausted"))?;
        self.deferred_class_temporary_slot_cursor += 1;
        build(self.builder.build_store(slot, value))?;
        self.deferred_class_temporary_drops
            .push((slot, DeferredOwnedTemporary::Class(class)));
        Ok(())
    }

    fn defer_or_drop_collection_temporary(
        &mut self,
        value: PointerValue<'ctx>,
        collection: mir::CollectionTypeId,
    ) -> Result<(), BackendError> {
        if !self.defer_class_temporary_drops {
            return self.drop_collection_value(value, collection);
        }
        let slot = *self
            .deferred_class_temporary_slots
            .get(self.deferred_class_temporary_slot_cursor)
            .ok_or_else(|| malformed_mir("owned temporary stack-slot capacity was exhausted"))?;
        self.deferred_class_temporary_slot_cursor += 1;
        build(self.builder.build_store(slot, value))?;
        self.deferred_class_temporary_drops
            .push((slot, DeferredOwnedTemporary::Collection(collection)));
        Ok(())
    }

    fn defer_or_drop_writable_shared_temporary(
        &mut self,
        value: PointerValue<'ctx>,
        symbol: &'static str,
    ) -> Result<(), BackendError> {
        if !self.defer_class_temporary_drops {
            return self.drop_writable_shared_value(value, symbol);
        }
        let slot = *self
            .deferred_class_temporary_slots
            .get(self.deferred_class_temporary_slot_cursor)
            .ok_or_else(|| malformed_mir("owned temporary stack-slot capacity was exhausted"))?;
        self.deferred_class_temporary_slot_cursor += 1;
        build(self.builder.build_store(slot, value))?;
        self.deferred_class_temporary_drops
            .push((slot, DeferredOwnedTemporary::WritableShared(symbol)));
        Ok(())
    }

    fn defer_or_drop_owned_shared_temporary(
        &mut self,
        value: PointerValue<'ctx>,
        ownership: mir::OwnedSharedTemporary,
    ) -> Result<(), BackendError> {
        match ownership {
            mir::OwnedSharedTemporary::Strong => self.defer_or_drop_shared_temporary(value, false),
            mir::OwnedSharedTemporary::Weak => self.defer_or_drop_shared_temporary(value, true),
            mir::OwnedSharedTemporary::WritableStrong => {
                self.defer_or_drop_writable_shared_temporary(value, WRITABLE_SHARED_RELEASE)
            }
            mir::OwnedSharedTemporary::WritableWeak => {
                self.defer_or_drop_writable_shared_temporary(value, WRITABLE_SHARED_RELEASE_WEAK)
            }
            mir::OwnedSharedTemporary::ReadonlyAccess => self
                .defer_or_drop_writable_shared_temporary(
                    value,
                    WRITABLE_SHARED_RELEASE_READONLY_ACCESS,
                ),
            mir::OwnedSharedTemporary::WritableAccess => self
                .defer_or_drop_writable_shared_temporary(
                    value,
                    WRITABLE_SHARED_RELEASE_WRITABLE_ACCESS,
                ),
        }
    }

    fn defer_or_cleanup_mixed_temporary(
        &mut self,
        value: PointerValue<'ctx>,
        ownership: mir::MixedOwnership,
    ) -> Result<(), BackendError> {
        if !self.defer_class_temporary_drops {
            return self.cleanup_mixed_temporary(value, ownership);
        }
        let slot = *self
            .deferred_class_temporary_slots
            .get(self.deferred_class_temporary_slot_cursor)
            .ok_or_else(|| malformed_mir("owned temporary stack-slot capacity was exhausted"))?;
        self.deferred_class_temporary_slot_cursor += 1;
        build(self.builder.build_store(slot, value))?;
        self.deferred_class_temporary_drops
            .push((slot, DeferredOwnedTemporary::Mixed(ownership)));
        Ok(())
    }

    fn defer_or_drop_shared_temporary(
        &mut self,
        value: PointerValue<'ctx>,
        weak: bool,
    ) -> Result<(), BackendError> {
        if !self.defer_class_temporary_drops {
            return self.drop_shared_value(value, weak);
        }
        let slot = *self
            .deferred_class_temporary_slots
            .get(self.deferred_class_temporary_slot_cursor)
            .ok_or_else(|| malformed_mir("owned temporary stack-slot capacity was exhausted"))?;
        self.deferred_class_temporary_slot_cursor += 1;
        build(self.builder.build_store(slot, value))?;
        self.deferred_class_temporary_drops
            .push((slot, DeferredOwnedTemporary::Shared(weak)));
        Ok(())
    }

    fn drop_class_value_checked(
        &mut self,
        object: PointerValue<'ctx>,
        class: crate::class_layout::ClassId,
    ) -> Result<(), BackendError> {
        let function = current_function(&self.builder)?;
        let drop_block = self.context.append_basic_block(function, "class.drop");
        let continue_block = self
            .context
            .append_basic_block(function, "class.drop.continue");
        let condition = build(self.builder.build_is_not_null(object, "class.has_object"))?;
        build(
            self.builder
                .build_conditional_branch(condition, drop_block, continue_block),
        )?;
        self.builder.position_at_end(drop_block);
        let drop_function = *self.class_drop_functions.get(class.0).ok_or_else(|| {
            malformed_mir(format!("class{} drop function does not exist", class.0))
        })?;
        build(self.builder.build_call(
            drop_function,
            &[self.current_frame.into(), object.into()],
            "class.drop.call",
        ))?;
        build(self.builder.build_unconditional_branch(continue_block))?;
        self.builder.position_at_end(continue_block);
        Ok(())
    }

    fn drop_class_value(
        &mut self,
        object: PointerValue<'ctx>,
        class: crate::class_layout::ClassId,
    ) -> Result<(), BackendError> {
        self.drop_class_value_impl(object, class, true)
    }

    fn drop_failed_class_value(
        &mut self,
        object: PointerValue<'ctx>,
        class: crate::class_layout::ClassId,
    ) -> Result<(), BackendError> {
        self.drop_class_value_impl(object, class, false)
    }

    fn drop_class_value_impl(
        &mut self,
        object: PointerValue<'ctx>,
        class: crate::class_layout::ClassId,
        run_destructor: bool,
    ) -> Result<(), BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let class_definition = class_definition(self.program, class)?;
        let destructor = class_definition.destructor;
        let properties = class_definition.properties.clone();
        if let Some(destructor) = destructor.filter(|_| run_destructor) {
            let callee = *self
                .functions
                .get(destructor.0)
                .ok_or_else(|| malformed_mir(format!("function{} does not exist", destructor.0)))?;
            let call = build(self.builder.build_call(
                callee,
                &[self.current_frame.into(), object.into()],
                "class.destruct",
            ))?;
            apply_call_abi_attributes(self.context, call, function_in(self.program, destructor)?)?;
        }
        for property in properties.iter().rev() {
            let address = self.lower_property_address_from_value(object, property.id)?;
            match property.ty {
                mir::Type::Error | mir::Type::NullableError => {
                    let value = build(self.builder.build_load(
                        error_carrier_type(self.context),
                        address,
                        "property.error",
                    ))?
                    .into_struct_value();
                    self.drop_error_value(value)?;
                }
                mir::Type::String | mir::Type::NullableString => {
                    let value = build(self.builder.build_load(
                        llvm_type(self.context, self.target_data, property.ty),
                        address,
                        "property.string",
                    ))?;
                    let value = if matches!(property.ty, mir::Type::NullableString) {
                        self.nullable_parts(value.into_struct_value())?
                            .1
                            .into_pointer_value()
                    } else {
                        value.into_pointer_value()
                    };
                    self.release_string(value)?;
                }
                mir::Type::Class(class) | mir::Type::NullableClass(class) => {
                    let value = build(self.builder.build_load(pointer, address, "property.class"))?
                        .into_pointer_value();
                    self.drop_class_value_checked(value, class)?;
                }
                mir::Type::Collection(collection) | mir::Type::NullableCollection(collection) => {
                    let value = build(self.builder.build_load(
                        pointer,
                        address,
                        "property.collection",
                    ))?
                    .into_pointer_value();
                    self.drop_collection_value(value, collection)?;
                }
                mir::Type::Mixed | mir::Type::NullableMixed => {
                    let value = build(self.builder.build_load(pointer, address, "property.mixed"))?
                        .into_pointer_value();
                    self.drop_mixed_value(value)?;
                }
                mir::Type::SharedReference(_) | mir::Type::NullableSharedReference(_) => {
                    let value =
                        build(self.builder.build_load(pointer, address, "property.shared"))?
                            .into_pointer_value();
                    self.drop_shared_value(value, false)?;
                }
                mir::Type::WeakReference(_) | mir::Type::NullableWeakReference(_) => {
                    let value = build(self.builder.build_load(pointer, address, "property.weak"))?
                        .into_pointer_value();
                    self.drop_shared_value(value, true)?;
                }
                mir::Type::WritableSharedReference(_)
                | mir::Type::WritableWeakReference(_)
                | mir::Type::NullableWritableSharedReference(_)
                | mir::Type::NullableWritableWeakReference(_)
                | mir::Type::ReadonlySharedReferenceAccess(_)
                | mir::Type::WritableSharedReferenceAccess(_)
                | mir::Type::NullableReadonlySharedReferenceAccess(_)
                | mir::Type::NullableWritableSharedReferenceAccess(_) => {
                    let value = build(self.builder.build_load(
                        pointer,
                        address,
                        "property.writable.shared",
                    ))?
                    .into_pointer_value();
                    let symbol = writable_shared_release_symbol(property.ty).ok_or_else(|| {
                        malformed_mir("writable shared release symbol is missing")
                    })?;
                    self.drop_writable_shared_value(value, symbol)?;
                }
                mir::Type::PayloadEnum(payload) => {
                    self.drop_payload_enum_at(address, payload, false)?;
                }
                mir::Type::NullablePayloadEnum(payload) => {
                    self.drop_payload_enum_at(address, payload, true)?;
                }
                mir::Type::Scalar(_) | mir::Type::NullableScalar(_) => {}
                mir::Type::Function(_) | mir::Type::NullableFunction(_) => {
                    let value = build(self.builder.build_load(
                        closure_carrier_type(self.context),
                        address,
                        "property.function",
                    ))?
                    .into_struct_value();
                    self.drop_function_carrier(value)?;
                }
                mir::Type::ClosureEnvironment(_) => {
                    return Err(malformed_mir(
                        "closure environment pointer is not a class property",
                    ));
                }
            }
        }
        let _ = self.call_runtime(CLASS_FREE, &[pointer.into()], None, &[object.into()])?;
        Ok(())
    }

    fn retain_string_parameters(&self) -> Result<(), BackendError> {
        for parameter in &self.function.params {
            if matches!(
                local_in(self.function, *parameter)?.ty,
                mir::Type::String | mir::Type::NullableString
            ) {
                let slot = local_slot(&self.local_slots, *parameter)?;
                let ty = local_in(self.function, *parameter)?.ty;
                let value = build(self.builder.build_load(
                    llvm_type(self.context, self.target_data, ty),
                    slot,
                    "string.parameter",
                ))?;
                let (present, value) = if matches!(ty, mir::Type::NullableString) {
                    let (present, payload) = self.nullable_parts(value.into_struct_value())?;
                    (Some(present), payload.into_pointer_value())
                } else {
                    (None, value.into_pointer_value())
                };
                let retained = self.retain_string(value)?;
                let retained: BasicValueEnum<'ctx> = if let Some(present) = present {
                    self.nullable_value(present, retained.into())?.into()
                } else {
                    retained.into()
                };
                build(self.builder.build_store(slot, retained))?;
            }
        }
        Ok(())
    }

    fn lower_string_expression(
        &mut self,
        expression: &mir::StringExpression,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let usize_type = self.context.ptr_sized_int_type(self.target_data, None);
        match expression {
            mir::StringExpression::Intrinsic(call) => {
                Ok(self.lower_string_intrinsic_call(call)?.into_pointer_value())
            }
            mir::StringExpression::Literal(value) => {
                let data = define_bytes(
                    self.context,
                    self.module,
                    value.as_bytes(),
                    &format!(
                        "__doria_string_{}_{}",
                        self.function.id.0, self.next_data_id
                    ),
                );
                self.next_data_id += 1;
                Ok(self
                    .call_runtime(
                        STRING_FROM_UTF8,
                        &[pointer.into(), usize_type.into()],
                        Some(pointer.into()),
                        &[
                            data.into(),
                            usize_type.const_int(value.len() as u64, false).into(),
                        ],
                    )?
                    .ok_or_else(|| backend_failure("string allocation produced no result"))?
                    .into_pointer_value())
            }
            mir::StringExpression::Local(local) => {
                let value = build(self.builder.build_load(
                    pointer,
                    local_slot(&self.local_slots, *local)?,
                    "string.local",
                ))?
                .into_pointer_value();
                self.retain_string(value)
            }
            mir::StringExpression::NullableLocalAssumeNonNull(local) => {
                let value = build(self.builder.build_load(
                    llvm_type(self.context, self.target_data, mir::Type::NullableString),
                    local_slot(&self.local_slots, *local)?,
                    "nullable-string.local",
                ))?
                .into_struct_value();
                let payload = self.nullable_parts(value)?.1.into_pointer_value();
                self.retain_string(payload)
            }
            mir::StringExpression::Static(id) => {
                let value = build(self.builder.build_load(
                    pointer,
                    self.static_address(*id)?,
                    "string.static",
                ))?
                .into_pointer_value();
                self.retain_string(value)
            }
            mir::StringExpression::MixedPayload(local) => {
                let value = self
                    .lower_mixed_payload(*local, mir::MixedTag::String)?
                    .into_pointer_value();
                self.retain_string(value)
            }
            mir::StringExpression::Property { object, property } => {
                let address = self.lower_property_address(*object, *property)?;
                let value = build(self.builder.build_load(pointer, address, "string.property"))?
                    .into_pointer_value();
                self.retain_string(value)
            }
            mir::StringExpression::ErrorMessage(error) => {
                let carrier = self.lower_error_expression(error)?;
                let (object, descriptor) = self.error_parts(carrier)?;
                let descriptor_type = error_descriptor_type(self.context, self.target_data);
                let offset_field = build(self.builder.build_struct_gep(
                    descriptor_type,
                    descriptor,
                    2,
                    "error.message.offset-field",
                ))?;
                let word = self.context.ptr_sized_int_type(self.target_data, None);
                let offset = build(self.builder.build_load(
                    word,
                    offset_field,
                    "error.message.offset",
                ))?
                .into_int_value();
                let address = unsafe {
                    build(self.builder.build_in_bounds_gep(
                        self.context.i8_type(),
                        object,
                        &[offset],
                        "error.message.address",
                    ))?
                };
                let pointer = self.context.ptr_type(AddressSpace::default());
                let value = build(self.builder.build_load(pointer, address, "error.message"))?
                    .into_pointer_value();
                self.retain_string(value)
            }
            mir::StringExpression::Concat(parts) => {
                let mut parts = parts.iter();
                let Some(first) = parts.next() else {
                    return self
                        .lower_string_expression(&mir::StringExpression::Literal(String::new()));
                };
                let mut value = self.lower_string_expression(first)?;
                for part in parts {
                    let right = self.lower_string_expression(part)?;
                    let concatenated = self
                        .call_runtime(
                            STRING_CONCAT,
                            &[pointer.into(), pointer.into()],
                            Some(pointer.into()),
                            &[value.into(), right.into()],
                        )?
                        .ok_or_else(|| backend_failure("string concat produced no result"))?
                        .into_pointer_value();
                    self.release_string(value)?;
                    self.release_string(right)?;
                    value = concatenated;
                }
                Ok(value)
            }
            mir::StringExpression::Display(value) => {
                let scalar = self.lower_value_expression(value)?;
                let (name, parameter, argument): (
                    &str,
                    BasicMetadataTypeEnum<'ctx>,
                    BasicMetadataValueEnum<'ctx>,
                ) = match value.ty() {
                    mir::ScalarType::Integer(ty) if ty.is_signed() => {
                        let integer = scalar.into_int_value();
                        let value = if ty.bit_width() < 64 {
                            build(self.builder.build_int_s_extend(
                                integer,
                                self.context.i64_type(),
                                "display.sext",
                            ))?
                        } else {
                            integer
                        };
                        (
                            STRING_FROM_I64,
                            self.context.i64_type().into(),
                            value.into(),
                        )
                    }
                    mir::ScalarType::Integer(ty) => {
                        let integer = scalar.into_int_value();
                        let value = if ty.bit_width() < 64 {
                            build(self.builder.build_int_z_extend(
                                integer,
                                self.context.i64_type(),
                                "display.zext",
                            ))?
                        } else {
                            integer
                        };
                        (
                            STRING_FROM_U64,
                            self.context.i64_type().into(),
                            value.into(),
                        )
                    }
                    mir::ScalarType::Float(FloatType::Float32) => (
                        STRING_FROM_F32,
                        self.context.f32_type().into(),
                        scalar.into(),
                    ),
                    mir::ScalarType::Float(FloatType::Float64) => (
                        STRING_FROM_F64,
                        self.context.f64_type().into(),
                        scalar.into(),
                    ),
                    mir::ScalarType::Bool => (
                        STRING_FROM_BOOL,
                        self.context.i8_type().into(),
                        scalar.into(),
                    ),
                    mir::ScalarType::Enum(_) => {
                        return Err(malformed_mir(
                            "enum display requires an explicit projection",
                        ));
                    }
                };
                Ok(self
                    .call_runtime(name, &[parameter], Some(pointer.into()), &[argument])?
                    .ok_or_else(|| backend_failure("display conversion produced no result"))?
                    .into_pointer_value())
            }
            mir::StringExpression::Call { function, args } => Ok(self
                .lower_call(*function, args, true)?
                .ok_or_else(|| malformed_mir("string call produced no result"))?
                .into_pointer_value()),
            mir::StringExpression::ReadFile { path, path_span } => {
                let path = self.lower_string_expression(path)?;
                self.set_active_panic_site(*path_span)?;
                let result = self
                    .call_runtime(
                        READ_FILE,
                        &[pointer.into(), pointer.into()],
                        Some(pointer.into()),
                        &[self.current_frame.into(), path.into()],
                    )?
                    .ok_or_else(|| backend_failure("read_file produced no result"))?
                    .into_pointer_value();
                self.release_string(path)?;
                Ok(result)
            }
            mir::StringExpression::Format(format) => self.lower_format_expression(format),
            mir::StringExpression::Coalesce { left, right } => {
                let left = self.lower_nullable_string_expression(left)?;
                Ok(self
                    .lower_coalesce_payload(left, |lowerer| {
                        Ok(lowerer.lower_string_expression(right)?.into())
                    })?
                    .into_pointer_value())
            }
            mir::StringExpression::CollectionIndex {
                positional,
                collection,
                index,
                remove,
            } => {
                let value = self
                    .lower_collection_index(*collection, index, *remove, *positional)?
                    .into_pointer_value();
                if *remove {
                    Ok(value)
                } else {
                    self.retain_string(value)
                }
            }
            mir::StringExpression::CollectionKeyAt { collection, offset } => {
                let value = self
                    .lower_collection_key_at(*collection, offset, mir::Type::String)?
                    .into_pointer_value();
                self.retain_string(value)
            }
            mir::StringExpression::EnumBacking { enum_id, value } => {
                self.lower_string_enum_backing(*enum_id, value)
            }
        }
    }

    fn lower_string_enum_backing(
        &mut self,
        enum_id: crate::enums::EnumId,
        value: &mir::EnumExpression,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let tag = self.lower_enum_expression(value)?;
        self.lower_string_enum_backing_from_tag(enum_id, tag)
    }

    fn lower_string_enum_backing_from_tag(
        &mut self,
        enum_id: crate::enums::EnumId,
        tag: IntValue<'ctx>,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let cases = enum_definition(self.program, enum_id)?
            .cases
            .iter()
            .map(|case| match case.backing_value.as_ref() {
                Some(crate::enums::EnumBackingValue::String(value)) => {
                    Ok((case.tag, value.clone()))
                }
                _ => Err(malformed_mir("string-backed enum case has no string value")),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let function = current_function(&self.builder)?;
        let done = self
            .context
            .append_basic_block(function, "enum.backing.done");
        let mut incoming = Vec::with_capacity(cases.len());
        let mut cases = cases.into_iter().peekable();
        while let Some((case_tag, backing)) = cases.next() {
            if cases.peek().is_some() {
                let selected = self
                    .context
                    .append_basic_block(function, "enum.backing.case");
                let next = self
                    .context
                    .append_basic_block(function, "enum.backing.next");
                let matches = build(
                    self.builder.build_int_compare(
                        IntPredicate::EQ,
                        tag,
                        self.context
                            .i32_type()
                            .const_int(u64::from(case_tag), false),
                        "enum.backing.matches",
                    ),
                )?;
                build(
                    self.builder
                        .build_conditional_branch(matches, selected, next),
                )?;
                self.builder.position_at_end(selected);
                let result =
                    self.lower_string_expression(&mir::StringExpression::Literal(backing))?;
                build(self.builder.build_unconditional_branch(done))?;
                incoming.push((result, selected));
                self.builder.position_at_end(next);
            } else {
                let block = self
                    .builder
                    .get_insert_block()
                    .ok_or_else(|| malformed_mir("enum backing has no active block"))?;
                let result =
                    self.lower_string_expression(&mir::StringExpression::Literal(backing))?;
                build(self.builder.build_unconditional_branch(done))?;
                incoming.push((result, block));
            }
        }
        self.builder.position_at_end(done);
        let phi = build(self.builder.build_phi(
            self.context.ptr_type(AddressSpace::default()),
            "enum.backing.string",
        ))?;
        let incoming_refs = incoming
            .iter()
            .map(|(value, block)| (value as &dyn inkwell::values::BasicValue<'ctx>, *block))
            .collect::<Vec<_>>();
        phi.add_incoming(&incoming_refs);
        Ok(phi.as_basic_value().into_pointer_value())
    }

    fn lower_coalesce_payload(
        &mut self,
        nullable: StructValue<'ctx>,
        fallback: impl FnOnce(&mut Self) -> Result<BasicValueEnum<'ctx>, BackendError>,
    ) -> Result<BasicValueEnum<'ctx>, BackendError> {
        let (present, payload) = self.nullable_parts(nullable)?;
        let function = current_function(&self.builder)?;
        let some = self.context.append_basic_block(function, "coalesce.some");
        let none = self.context.append_basic_block(function, "coalesce.none");
        let done = self.context.append_basic_block(function, "coalesce.done");
        let present = build(self.builder.build_int_compare(
            IntPredicate::NE,
            present,
            self.present_word(false),
            "coalesce.present",
        ))?;
        build(self.builder.build_conditional_branch(present, some, none))?;
        self.builder.position_at_end(some);
        build(self.builder.build_unconditional_branch(done))?;
        let some_end = self
            .builder
            .get_insert_block()
            .expect("coalesce some block");
        self.builder.position_at_end(none);
        let fallback = fallback(self)?;
        build(self.builder.build_unconditional_branch(done))?;
        let none_end = self
            .builder
            .get_insert_block()
            .expect("coalesce none block");
        self.builder.position_at_end(done);
        let phi = build(self.builder.build_phi(payload.get_type(), "coalesce.value"))?;
        phi.add_incoming(&[(&payload, some_end), (&fallback, none_end)]);
        Ok(phi.as_basic_value())
    }

    fn lower_nullable_coalesce(
        &mut self,
        left: StructValue<'ctx>,
        fallback: impl FnOnce(&mut Self) -> Result<BasicValueEnum<'ctx>, BackendError>,
    ) -> Result<StructValue<'ctx>, BackendError> {
        let (present, _) = self.nullable_parts(left)?;
        let function = current_function(&self.builder)?;
        let some = self
            .context
            .append_basic_block(function, "nullable.coalesce.some");
        let none = self
            .context
            .append_basic_block(function, "nullable.coalesce.none");
        let done = self
            .context
            .append_basic_block(function, "nullable.coalesce.done");
        let present = build(self.builder.build_int_compare(
            IntPredicate::NE,
            present,
            self.present_word(false),
            "nullable.coalesce.present",
        ))?;
        build(self.builder.build_conditional_branch(present, some, none))?;
        self.builder.position_at_end(some);
        build(self.builder.build_unconditional_branch(done))?;
        let some_end = self
            .builder
            .get_insert_block()
            .expect("nullable coalesce some block");
        self.builder.position_at_end(none);
        let fallback = fallback(self)?.into_struct_value();
        if fallback.get_type() != left.get_type() {
            return Err(malformed_mir(
                "nullable coalesce operands have different MIR types",
            ));
        }
        build(self.builder.build_unconditional_branch(done))?;
        let none_end = self
            .builder
            .get_insert_block()
            .expect("nullable coalesce none block");
        self.builder.position_at_end(done);
        let phi = build(self.builder.build_phi(left.get_type(), "nullable.coalesce"))?;
        phi.add_incoming(&[(&left, some_end), (&fallback, none_end)]);
        Ok(phi.as_basic_value().into_struct_value())
    }

    fn lower_format_expression(
        &mut self,
        format: &mir::FormatExpression,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let mut result =
            self.lower_string_expression(&mir::StringExpression::Literal(String::new()))?;
        for piece in &format.pieces {
            let next =
                match piece {
                    FormatPiece::Literal(value) => self
                        .lower_string_expression(&mir::StringExpression::Literal(value.clone()))?,
                    FormatPiece::Argument { index, spec } => {
                        let argument = format.arguments.get(*index as usize).ok_or_else(|| {
                            malformed_mir("format argument index is out of bounds")
                        })?;
                        self.lower_format_argument(argument, *spec)?
                    }
                };
            let concatenated = self
                .call_runtime(
                    STRING_CONCAT,
                    &[pointer.into(), pointer.into()],
                    Some(pointer.into()),
                    &[result.into(), next.into()],
                )?
                .ok_or_else(|| backend_failure("format concatenation produced no result"))?
                .into_pointer_value();
            self.release_string(result)?;
            self.release_string(next)?;
            result = concatenated;
        }
        Ok(result)
    }

    fn lower_format_argument(
        &mut self,
        argument: &mir::FormatArgument,
        spec: crate::format_string::FormatSpec,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let i8_type = self.context.i8_type();
        let i32_type = self.context.i32_type();
        let i64_type = self.context.i64_type();
        let width = i32_type.const_int(u64::from(spec.width.unwrap_or(0)), false);
        let flags_value = u8::from(spec.left_align) | (u8::from(spec.zero_pad) << 1);
        let flags = i8_type.const_int(u64::from(flags_value), false);

        if spec.conversion == FormatConversion::Display {
            let string = match argument {
                mir::FormatArgument::String(value) | mir::FormatArgument::ClassDisplay(value) => {
                    self.lower_string_expression(value)?
                }
                mir::FormatArgument::Value(value) => {
                    self.lower_string_expression(&mir::StringExpression::Display(value.clone()))?
                }
            };
            let formatted = self
                .call_runtime(
                    FORMAT_STRING,
                    &[pointer.into(), i32_type.into(), i8_type.into()],
                    Some(pointer.into()),
                    &[string.into(), width.into(), flags.into()],
                )?
                .ok_or_else(|| backend_failure("string formatting produced no result"))?
                .into_pointer_value();
            self.release_string(string)?;
            return Ok(formatted);
        }

        if let mir::FormatArgument::Value(mir::ValueExpression::Float(float)) = argument {
            let value = self.lower_float_expression(float)?;
            let precision = i32_type.const_int(u64::from(spec.precision.unwrap_or(6)), false);
            let (name, ty): (&str, BasicMetadataTypeEnum<'ctx>) = match float.ty() {
                FloatType::Float32 => (FORMAT_F32, self.context.f32_type().into()),
                FloatType::Float64 => (FORMAT_F64, self.context.f64_type().into()),
            };
            return Ok(self
                .call_runtime(
                    name,
                    &[ty, i32_type.into(), i32_type.into(), i8_type.into()],
                    Some(pointer.into()),
                    &[value.into(), precision.into(), width.into(), flags.into()],
                )?
                .ok_or_else(|| backend_failure("float formatting produced no result"))?
                .into_pointer_value());
        }

        let mir::FormatArgument::Value(mir::ValueExpression::Integer(integer)) = argument else {
            return Err(malformed_mir(
                "format conversion and argument type disagree",
            ));
        };
        let ty = integer.ty();
        let mut value = self.lower_integer_expression(integer)?;
        if ty.bit_width() < 64 {
            value = if ty.is_signed() {
                build(
                    self.builder
                        .build_int_s_extend(value, i64_type, "format.sext"),
                )?
            } else {
                build(
                    self.builder
                        .build_int_z_extend(value, i64_type, "format.zext"),
                )?
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
        let conversion = i8_type.const_int(conversion, false);
        let result = if ty.is_signed() {
            let bit_width = i8_type.const_int(u64::from(ty.bit_width()), false);
            self.call_runtime(
                FORMAT_I64,
                &[
                    i64_type.into(),
                    i8_type.into(),
                    i8_type.into(),
                    i32_type.into(),
                    i8_type.into(),
                ],
                Some(pointer.into()),
                &[
                    value.into(),
                    bit_width.into(),
                    conversion.into(),
                    width.into(),
                    flags.into(),
                ],
            )?
        } else {
            self.call_runtime(
                FORMAT_U64,
                &[
                    i64_type.into(),
                    i8_type.into(),
                    i32_type.into(),
                    i8_type.into(),
                ],
                Some(pointer.into()),
                &[value.into(), conversion.into(), width.into(), flags.into()],
            )?
        };
        Ok(result
            .ok_or_else(|| backend_failure("integer formatting produced no result"))?
            .into_pointer_value())
    }

    fn lower_integer_expression(
        &mut self,
        expression: &mir::IntegerExpression,
    ) -> Result<IntValue<'ctx>, BackendError> {
        match expression {
            mir::IntegerExpression::Use { ty, operand } => self.lower_integer_operand(*ty, operand),
            mir::IntegerExpression::Unary {
                ty,
                op,
                operand,
                span,
            } => {
                let operand = self.lower_integer_expression(operand)?;
                self.lower_integer_unary(*ty, *op, operand, *span)
            }
            mir::IntegerExpression::Binary {
                ty,
                op,
                left,
                right,
                span,
                right_span,
            } => {
                let left = self.lower_integer_expression(left)?;
                let right = self.lower_integer_expression(right)?;
                self.lower_integer_binary(*ty, *op, left, right, *span, *right_span)
            }
            mir::IntegerExpression::Convert {
                ty,
                value,
                value_span,
                ..
            } => {
                let source = value.ty();
                let value = self.lower_integer_expression(value)?;
                self.lower_integer_conversion(source, *ty, value, *value_span)
            }
            mir::IntegerExpression::FloatToInt {
                value, value_span, ..
            } => {
                let value = self.lower_float_expression(value)?;
                self.lower_float_to_int(value, *value_span)
            }
            mir::IntegerExpression::Call { function, args, .. } => {
                let result = self
                    .lower_call(*function, args, true)?
                    .ok_or_else(|| malformed_mir("scalar call produced no result"))?;
                Ok(result.into_int_value())
            }
            mir::IntegerExpression::Coalesce { left, right, .. } => {
                let left = self.lower_nullable_scalar_expression(left)?;
                Ok(self
                    .lower_coalesce_payload(left, |lowerer| {
                        Ok(lowerer.lower_integer_expression(right)?.into())
                    })?
                    .into_int_value())
            }
            mir::IntegerExpression::EnumBacking { enum_id, value } => {
                let tag = self.lower_enum_expression(value)?;
                self.lower_integer_enum_backing_from_tag(*enum_id, tag)
            }
        }
    }

    fn lower_integer_enum_backing_from_tag(
        &mut self,
        enum_id: crate::enums::EnumId,
        tag: IntValue<'ctx>,
    ) -> Result<IntValue<'ctx>, BackendError> {
        let values = enum_definition(self.program, enum_id)?
            .cases
            .iter()
            .map(|case| match case.backing_value.as_ref() {
                Some(crate::enums::EnumBackingValue::Int(value)) => Ok((case.tag, *value)),
                _ => Err(malformed_mir("int-backed enum case has no integer value")),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut values = values.into_iter();
        let (_, first) = values
            .next()
            .ok_or_else(|| malformed_mir("enum has no cases"))?;
        let mut result = integer_constant(self.context, first);
        for (case_tag, backing) in values {
            let selected = build(
                self.builder.build_int_compare(
                    IntPredicate::EQ,
                    tag,
                    self.context
                        .i32_type()
                        .const_int(u64::from(case_tag), false),
                    "enum.backing.case",
                ),
            )?;
            result = build(self.builder.build_select(
                selected,
                integer_constant(self.context, backing),
                result,
                "enum.backing.value",
            ))?
            .into_int_value();
        }
        Ok(result)
    }

    fn lower_integer_unary(
        &mut self,
        ty: IntegerType,
        op: mir::IntegerUnaryOp,
        operand: IntValue<'ctx>,
        span: crate::source::Span,
    ) -> Result<IntValue<'ctx>, BackendError> {
        match op {
            mir::IntegerUnaryOp::Negate => {
                let zero = integer_type(self.context, ty).const_zero();
                let minimum = integer_constant(
                    self.context,
                    IntegerValue::from_bits(ty, 1_u64 << (ty.bit_width() - 1)),
                );
                let overflow = build(self.builder.build_int_compare(
                    IntPredicate::EQ,
                    operand,
                    minimum,
                    "neg.overflow",
                ))?;
                self.lower_panic_if_code(overflow, IntegerPanic::OverflowNegation.code(), span)?;
                build(self.builder.build_int_sub(zero, operand, "negated"))
            }
            mir::IntegerUnaryOp::BitwiseNot => build(self.builder.build_not(operand, "not")),
        }
    }

    fn lower_integer_binary(
        &mut self,
        ty: IntegerType,
        op: mir::IntegerBinaryOp,
        left: IntValue<'ctx>,
        right: IntValue<'ctx>,
        span: crate::source::Span,
        right_span: crate::source::Span,
    ) -> Result<IntValue<'ctx>, BackendError> {
        match op {
            mir::IntegerBinaryOp::Add
            | mir::IntegerBinaryOp::Subtract
            | mir::IntegerBinaryOp::Multiply => {
                self.lower_checked_arithmetic(ty, op, left, right, span)
            }
            mir::IntegerBinaryOp::Divide => {
                self.lower_integer_division(ty, left, right, span, right_span)
            }
            mir::IntegerBinaryOp::Remainder => {
                self.lower_integer_remainder(ty, left, right, right_span)
            }
            mir::IntegerBinaryOp::ShiftLeft | mir::IntegerBinaryOp::ShiftRight => {
                self.lower_integer_shift(ty, op, left, right, right_span)
            }
            mir::IntegerBinaryOp::BitwiseAnd => build(self.builder.build_and(left, right, "and")),
            mir::IntegerBinaryOp::BitwiseXor => build(self.builder.build_xor(left, right, "xor")),
            mir::IntegerBinaryOp::BitwiseOr => build(self.builder.build_or(left, right, "or")),
        }
    }
}

impl<'ctx> FunctionLowerer<'ctx, '_> {
    fn lower_checked_arithmetic(
        &mut self,
        ty: IntegerType,
        op: mir::IntegerBinaryOp,
        left: IntValue<'ctx>,
        right: IntValue<'ctx>,
        span: crate::source::Span,
    ) -> Result<IntValue<'ctx>, BackendError> {
        let operation = match op {
            mir::IntegerBinaryOp::Add => "add",
            mir::IntegerBinaryOp::Subtract => "sub",
            mir::IntegerBinaryOp::Multiply => "mul",
            _ => unreachable!("non-arithmetic operator reached checked arithmetic lowering"),
        };
        let integer = integer_type(self.context, ty);
        let result_type = self
            .context
            .struct_type(&[integer.into(), self.context.bool_type().into()], false);
        let signedness = if ty.is_signed() { "s" } else { "u" };
        let intrinsic_name = format!(
            "llvm.{signedness}{operation}.with.overflow.i{}",
            ty.bit_width()
        );
        let intrinsic = self
            .module
            .get_function(&intrinsic_name)
            .unwrap_or_else(|| {
                self.module.add_function(
                    &intrinsic_name,
                    result_type.fn_type(&[integer.into(), integer.into()], false),
                    None,
                )
            });
        let result = build(self.builder.build_call(
            intrinsic,
            &[left.into(), right.into()],
            "checked.arithmetic",
        ))?
        .try_as_basic_value()
        .basic()
        .ok_or_else(|| backend_failure("checked arithmetic intrinsic produced no result"))?
        .into_struct_value();
        let value = build(
            self.builder
                .build_extract_value(result, 0, "checked.result"),
        )?
        .into_int_value();
        let overflow = build(
            self.builder
                .build_extract_value(result, 1, "arithmetic.overflow"),
        )?
        .into_int_value();
        let panic = match op {
            mir::IntegerBinaryOp::Add => IntegerPanic::OverflowAddition,
            mir::IntegerBinaryOp::Subtract => IntegerPanic::OverflowSubtraction,
            mir::IntegerBinaryOp::Multiply => IntegerPanic::OverflowMultiplication,
            _ => unreachable!("non-arithmetic operator reached checked arithmetic lowering"),
        };
        self.lower_panic_if_code(overflow, panic.code(), span)?;
        Ok(value)
    }

    fn lower_integer_division(
        &mut self,
        ty: IntegerType,
        left: IntValue<'ctx>,
        right: IntValue<'ctx>,
        span: crate::source::Span,
        right_span: crate::source::Span,
    ) -> Result<IntValue<'ctx>, BackendError> {
        let zero = integer_type(self.context, ty).const_zero();
        let divides_by_zero = build(self.builder.build_int_compare(
            IntPredicate::EQ,
            right,
            zero,
            "division.by_zero",
        ))?;
        self.lower_panic_if_code(
            divides_by_zero,
            IntegerPanic::DivisionByZero.code(),
            right_span,
        )?;
        if ty.is_signed() {
            let minimum = integer_constant(
                self.context,
                IntegerValue::from_bits(ty, 1_u64 << (ty.bit_width() - 1)),
            );
            let negative_one =
                integer_constant(self.context, IntegerValue::from_bits(ty, ty.mask()));
            let is_minimum = build(self.builder.build_int_compare(
                IntPredicate::EQ,
                left,
                minimum,
                "division.is_minimum",
            ))?;
            let is_negative_one = build(self.builder.build_int_compare(
                IntPredicate::EQ,
                right,
                negative_one,
                "division.is_negative_one",
            ))?;
            let overflow = build(self.builder.build_and(
                is_minimum,
                is_negative_one,
                "division.overflow",
            ))?;
            self.lower_panic_if_code(overflow, IntegerPanic::DivisionOverflow.code(), span)?;
            build(self.builder.build_int_signed_div(left, right, "quotient"))
        } else {
            build(self.builder.build_int_unsigned_div(left, right, "quotient"))
        }
    }

    fn lower_integer_remainder(
        &mut self,
        ty: IntegerType,
        left: IntValue<'ctx>,
        right: IntValue<'ctx>,
        right_span: crate::source::Span,
    ) -> Result<IntValue<'ctx>, BackendError> {
        let integer_type = integer_type(self.context, ty);
        let zero = integer_type.const_zero();
        let divides_by_zero = build(self.builder.build_int_compare(
            IntPredicate::EQ,
            right,
            zero,
            "remainder.by_zero",
        ))?;
        self.lower_panic_if_code(
            divides_by_zero,
            IntegerPanic::RemainderByZero.code(),
            right_span,
        )?;
        if !ty.is_signed() {
            return build(
                self.builder
                    .build_int_unsigned_rem(left, right, "remainder"),
            );
        }

        let minimum = integer_constant(
            self.context,
            IntegerValue::from_bits(ty, 1_u64 << (ty.bit_width() - 1)),
        );
        let negative_one = integer_constant(self.context, IntegerValue::from_bits(ty, ty.mask()));
        let is_minimum = build(self.builder.build_int_compare(
            IntPredicate::EQ,
            left,
            minimum,
            "remainder.is_minimum",
        ))?;
        let is_negative_one = build(self.builder.build_int_compare(
            IntPredicate::EQ,
            right,
            negative_one,
            "remainder.is_negative_one",
        ))?;
        let special_case = build(self.builder.build_and(
            is_minimum,
            is_negative_one,
            "remainder.special",
        ))?;
        let function = current_function(&self.builder)?;
        let zero_block = self.context.append_basic_block(function, "remainder.zero");
        let remainder_block = self
            .context
            .append_basic_block(function, "remainder.normal");
        let done_block = self.context.append_basic_block(function, "remainder.done");
        build(
            self.builder
                .build_conditional_branch(special_case, zero_block, remainder_block),
        )?;

        self.builder.position_at_end(zero_block);
        build(self.builder.build_unconditional_branch(done_block))?;

        self.builder.position_at_end(remainder_block);
        let remainder = build(self.builder.build_int_signed_rem(left, right, "remainder"))?;
        build(self.builder.build_unconditional_branch(done_block))?;

        self.builder.position_at_end(done_block);
        let phi = build(self.builder.build_phi(integer_type, "remainder.result"))?;
        phi.add_incoming(&[(&zero, zero_block), (&remainder, remainder_block)]);
        Ok(phi.as_basic_value().into_int_value())
    }

    fn lower_integer_shift(
        &mut self,
        ty: IntegerType,
        op: mir::IntegerBinaryOp,
        left: IntValue<'ctx>,
        right: IntValue<'ctx>,
        right_span: crate::source::Span,
    ) -> Result<IntValue<'ctx>, BackendError> {
        let integer_type = integer_type(self.context, ty);
        let width = integer_type.const_int(ty.bit_width() as u64, false);
        let too_large = build(self.builder.build_int_compare(
            IntPredicate::UGE,
            right,
            width,
            "shift.too_large",
        ))?;
        let invalid = if ty.is_signed() {
            let negative = build(self.builder.build_int_compare(
                IntPredicate::SLT,
                right,
                integer_type.const_zero(),
                "shift.negative",
            ))?;
            build(self.builder.build_or(negative, too_large, "shift.invalid"))?
        } else {
            too_large
        };
        self.lower_panic_if_code(
            invalid,
            IntegerPanic::ShiftCountOutOfRange.code(),
            right_span,
        )?;
        match op {
            mir::IntegerBinaryOp::ShiftLeft => {
                build(self.builder.build_left_shift(left, right, "shift.left"))
            }
            mir::IntegerBinaryOp::ShiftRight => {
                build(
                    self.builder
                        .build_right_shift(left, right, ty.is_signed(), "shift.right"),
                )
            }
            _ => unreachable!("non-shift operator reached shift lowering"),
        }
    }

    fn lower_integer_conversion(
        &mut self,
        source: IntegerType,
        target: IntegerType,
        value: IntValue<'ctx>,
        span: crate::source::Span,
    ) -> Result<IntValue<'ctx>, BackendError> {
        if let Some(out_of_range) = self.conversion_out_of_range(source, target, value)? {
            self.lower_panic_if_code(
                out_of_range,
                IntegerPanic::ConversionOutOfRange.code(),
                span,
            )?;
        }
        let source_width = source.bit_width();
        let target_width = target.bit_width();
        let target_type = integer_type(self.context, target);
        match target_width.cmp(&source_width) {
            std::cmp::Ordering::Equal => Ok(value),
            std::cmp::Ordering::Less => build(self.builder.build_int_truncate(
                value,
                target_type,
                "convert.truncate",
            )),
            std::cmp::Ordering::Greater if source.is_signed() => build(
                self.builder
                    .build_int_s_extend(value, target_type, "convert.extend"),
            ),
            std::cmp::Ordering::Greater => build(self.builder.build_int_z_extend(
                value,
                target_type,
                "convert.extend",
            )),
        }
    }

    fn conversion_out_of_range(
        &self,
        source: IntegerType,
        target: IntegerType,
        value: IntValue<'ctx>,
    ) -> Result<Option<IntValue<'ctx>>, BackendError> {
        let source_type = integer_type(self.context, source);
        match (source.is_signed(), target.is_signed()) {
            (true, true) if target.bit_width() < source.bit_width() => {
                let minimum = integer_constant(
                    self.context,
                    IntegerValue::from_i128(source, target.min_value())
                        .expect("narrow signed minimum fits source"),
                );
                let maximum = integer_constant(
                    self.context,
                    IntegerValue::from_i128(source, target.max_value())
                        .expect("narrow signed maximum fits source"),
                );
                let below = build(self.builder.build_int_compare(
                    IntPredicate::SLT,
                    value,
                    minimum,
                    "convert.below",
                ))?;
                let above = build(self.builder.build_int_compare(
                    IntPredicate::SGT,
                    value,
                    maximum,
                    "convert.above",
                ))?;
                Ok(Some(build(self.builder.build_or(
                    below,
                    above,
                    "convert.invalid",
                ))?))
            }
            (true, false) => {
                let negative = build(self.builder.build_int_compare(
                    IntPredicate::SLT,
                    value,
                    source_type.const_zero(),
                    "convert.negative",
                ))?;
                if target.bit_width() < source.bit_width() {
                    let maximum = integer_constant(
                        self.context,
                        IntegerValue::from_u128(source, target.max_value() as u128)
                            .expect("narrow unsigned maximum fits signed source"),
                    );
                    let above = build(self.builder.build_int_compare(
                        IntPredicate::UGT,
                        value,
                        maximum,
                        "convert.above",
                    ))?;
                    Ok(Some(build(self.builder.build_or(
                        negative,
                        above,
                        "convert.invalid",
                    ))?))
                } else {
                    Ok(Some(negative))
                }
            }
            (false, false) if target.bit_width() < source.bit_width() => {
                let maximum = integer_constant(
                    self.context,
                    IntegerValue::from_u128(source, target.max_value() as u128)
                        .expect("narrow unsigned maximum fits source"),
                );
                Ok(Some(build(self.builder.build_int_compare(
                    IntPredicate::UGT,
                    value,
                    maximum,
                    "convert.above",
                ))?))
            }
            (false, true) if target.bit_width() <= source.bit_width() => {
                let maximum = integer_constant(
                    self.context,
                    IntegerValue::from_u128(source, target.max_value() as u128)
                        .expect("signed maximum fits unsigned source"),
                );
                Ok(Some(build(self.builder.build_int_compare(
                    IntPredicate::UGT,
                    value,
                    maximum,
                    "convert.above",
                ))?))
            }
            _ => Ok(None),
        }
    }

    fn lower_integer_operand(
        &mut self,
        ty: IntegerType,
        operand: &mir::Operand,
    ) -> Result<IntValue<'ctx>, BackendError> {
        match operand {
            mir::Operand::StringIntrinsic(call) => {
                Ok(self.lower_string_intrinsic_call(call)?.into_int_value())
            }
            mir::Operand::Scalar(mir::ScalarValue::Integer(value)) if value.ty == ty => {
                Ok(integer_constant(self.context, *value))
            }
            mir::Operand::Local(local) => Ok(build(self.builder.build_load(
                integer_type(self.context, ty),
                local_slot(&self.local_slots, *local)?,
                "integer.local",
            ))?
            .into_int_value()),
            mir::Operand::NullablePayload(local) => {
                let value = build(self.builder.build_load(
                    llvm_type(
                        self.context,
                        self.target_data,
                        mir::Type::NullableScalar(mir::ScalarType::Integer(ty)),
                    ),
                    local_slot(&self.local_slots, *local)?,
                    "integer.nullable.local",
                ))?
                .into_struct_value();
                Ok(self.nullable_parts(value)?.1.into_int_value())
            }
            mir::Operand::MixedPayload { mixed, tag } => {
                let mir::MixedTag::Integer(integer) = tag else {
                    return Err(malformed_mir("integer mixed payload uses non-integer tag"));
                };
                if *integer != ty {
                    return Err(malformed_mir(
                        "integer mixed payload uses another integer type",
                    ));
                }
                Ok(self.lower_mixed_payload(*mixed, *tag)?.into_int_value())
            }
            mir::Operand::Static(id) => Ok(build(self.builder.build_load(
                integer_type(self.context, ty),
                self.static_address(*id)?,
                "integer.static",
            ))?
            .into_int_value()),
            mir::Operand::Property { object, property } => Ok(build(self.builder.build_load(
                integer_type(self.context, ty),
                self.lower_property_address(*object, *property)?,
                "integer.property",
            ))?
            .into_int_value()),
            mir::Operand::CollectionLength(collection) if ty == IntegerType::Int64 => {
                let pointer = self.context.ptr_type(AddressSpace::default());
                let usize_type = self.context.ptr_sized_int_type(self.target_data, None);
                let local = local_in(self.function, *collection)?;
                let mir::Type::Collection(collection_type) = local.ty else {
                    return Err(malformed_mir("collection length uses non-collection local"));
                };
                let definition = self.collection_definition(collection_type)?;
                let collection = self.collection_pointer(*collection)?;
                Ok(self
                    .call_runtime(
                        if definition.kind == mir::CollectionKind::Bytes {
                            BYTES_LENGTH
                        } else {
                            COLLECTION_LENGTH
                        },
                        &[pointer.into()],
                        Some(usize_type.into()),
                        &[collection.into()],
                    )?
                    .ok_or_else(|| backend_failure("collection length produced no result"))?
                    .into_int_value())
            }
            mir::Operand::CollectionIndex {
                positional,
                collection,
                index,
                remove,
            } => Ok(self
                .lower_collection_index(*collection, index, *remove, *positional)?
                .into_int_value()),
            mir::Operand::CollectionKeyAt { collection, offset } => Ok(self
                .lower_collection_key_at(
                    *collection,
                    offset,
                    mir::Type::Scalar(mir::ScalarType::Integer(ty)),
                )?
                .into_int_value()),
            _ => Err(malformed_mir(
                "integer expression has an incompatible operand",
            )),
        }
    }

    fn lower_float_expression(
        &mut self,
        expression: &mir::FloatExpression,
    ) -> Result<LlvmFloatValue<'ctx>, BackendError> {
        match expression {
            mir::FloatExpression::Use { ty, operand } => match operand {
                mir::Operand::Scalar(mir::ScalarValue::Float(value)) if value.ty == *ty => {
                    Ok(float_constant(self.context, *value))
                }
                mir::Operand::Local(local) => Ok(build(self.builder.build_load(
                    match ty {
                        FloatType::Float32 => self.context.f32_type(),
                        FloatType::Float64 => self.context.f64_type(),
                    },
                    local_slot(&self.local_slots, *local)?,
                    "float.local",
                ))?
                .into_float_value()),
                mir::Operand::NullablePayload(local) => {
                    let value = build(self.builder.build_load(
                        llvm_type(
                            self.context,
                            self.target_data,
                            mir::Type::NullableScalar(mir::ScalarType::Float(*ty)),
                        ),
                        local_slot(&self.local_slots, *local)?,
                        "float.nullable.local",
                    ))?
                    .into_struct_value();
                    Ok(self.nullable_parts(value)?.1.into_float_value())
                }
                mir::Operand::MixedPayload { mixed, tag } => {
                    let mir::MixedTag::Float(float) = tag else {
                        return Err(malformed_mir("float mixed payload uses non-float tag"));
                    };
                    if float != ty {
                        return Err(malformed_mir("float mixed payload uses another float type"));
                    }
                    Ok(self.lower_mixed_payload(*mixed, *tag)?.into_float_value())
                }
                mir::Operand::Static(id) => Ok(build(self.builder.build_load(
                    match ty {
                        FloatType::Float32 => self.context.f32_type(),
                        FloatType::Float64 => self.context.f64_type(),
                    },
                    self.static_address(*id)?,
                    "float.static",
                ))?
                .into_float_value()),
                mir::Operand::Property { object, property } => Ok(build(self.builder.build_load(
                    match ty {
                        FloatType::Float32 => self.context.f32_type(),
                        FloatType::Float64 => self.context.f64_type(),
                    },
                    self.lower_property_address(*object, *property)?,
                    "float.property",
                ))?
                .into_float_value()),
                mir::Operand::CollectionIndex {
                    positional,
                    collection,
                    index,
                    remove,
                } => Ok(self
                    .lower_collection_index(*collection, index, *remove, *positional)?
                    .into_float_value()),
                mir::Operand::CollectionKeyAt { collection, offset } => Ok(self
                    .lower_collection_key_at(
                        *collection,
                        offset,
                        mir::Type::Scalar(mir::ScalarType::Float(*ty)),
                    )?
                    .into_float_value()),
                _ => Err(malformed_mir(
                    "float expression has an incompatible operand",
                )),
            },
            mir::FloatExpression::Negate { operand, .. } => {
                let operand = self.lower_float_expression(operand)?;
                build(self.builder.build_float_neg(operand, "float.negate"))
            }
            mir::FloatExpression::Binary {
                op, left, right, ..
            } => {
                let left = self.lower_float_expression(left)?;
                let right = self.lower_float_expression(right)?;
                match op {
                    mir::FloatBinaryOp::Add => {
                        build(self.builder.build_float_add(left, right, "float.add"))
                    }
                    mir::FloatBinaryOp::Subtract => {
                        build(self.builder.build_float_sub(left, right, "float.sub"))
                    }
                    mir::FloatBinaryOp::Multiply => {
                        build(self.builder.build_float_mul(left, right, "float.mul"))
                    }
                    mir::FloatBinaryOp::Divide => {
                        build(self.builder.build_float_div(left, right, "float.div"))
                    }
                }
            }
            mir::FloatExpression::IntToFloat { value } => {
                let value = self.lower_integer_expression(value)?;
                build(self.builder.build_signed_int_to_float(
                    value,
                    self.context.f64_type(),
                    "int.to_float",
                ))
            }
            mir::FloatExpression::Call { function, args, .. } => {
                let result = self
                    .lower_call(*function, args, true)?
                    .ok_or_else(|| malformed_mir("float call produced no result"))?;
                Ok(result.into_float_value())
            }
            mir::FloatExpression::Coalesce { left, right, .. } => {
                let left = self.lower_nullable_scalar_expression(left)?;
                Ok(self
                    .lower_coalesce_payload(left, |lowerer| {
                        Ok(lowerer.lower_float_expression(right)?.into())
                    })?
                    .into_float_value())
            }
        }
    }

    fn lower_float_to_int(
        &mut self,
        value: LlvmFloatValue<'ctx>,
        span: crate::source::Span,
    ) -> Result<IntValue<'ctx>, BackendError> {
        let float_type = self.context.f64_type();
        let minimum = float_type.const_float(-9_223_372_036_854_775_808.0);
        let maximum = float_type.const_float(9_223_372_036_854_775_808.0);
        let unordered = build(self.builder.build_float_compare(
            FloatPredicate::UNO,
            value,
            value,
            "float_to_int.nan",
        ))?;
        let below = build(self.builder.build_float_compare(
            FloatPredicate::OLT,
            value,
            minimum,
            "float_to_int.below",
        ))?;
        let above = build(self.builder.build_float_compare(
            FloatPredicate::OGE,
            value,
            maximum,
            "float_to_int.above",
        ))?;
        let invalid_range = build(self.builder.build_or(below, above, "float_to_int.range"))?;
        let invalid = build(self.builder.build_or(
            unordered,
            invalid_range,
            "float_to_int.invalid",
        ))?;
        self.lower_panic_if_code(invalid, "P1110", span)?;
        build(self.builder.build_float_to_signed_int(
            value,
            self.context.i64_type(),
            "float.to_int",
        ))
    }

    fn lower_call(
        &mut self,
        function: mir::FunctionId,
        args: &[mir::Rvalue],
        expects_result: bool,
    ) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        self.lower_call_at(function, args, expects_result, self.function.source_span)
    }

    fn lower_call_at(
        &mut self,
        function: mir::FunctionId,
        args: &[mir::Rvalue],
        expects_result: bool,
        span: crate::source::Span,
    ) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        self.set_active_panic_site(span)?;
        self.lower_call_with_receiver(function, args, expects_result, None)
    }

    fn lower_call_arguments(
        &mut self,
        args: &[mir::Rvalue],
    ) -> Result<LoweredCallArguments<'ctx>, BackendError> {
        let mut values = Vec::with_capacity(args.len());
        let mut lowered = Vec::with_capacity(args.len());
        let mut owned_strings = Vec::new();
        let mut temporary_mixed = Vec::new();
        for (index, argument) in args.iter().enumerate() {
            let value = self.lower_rvalue(argument)?;
            match argument.ty() {
                mir::Type::String => owned_strings.push(value.into_pointer_value()),
                mir::Type::NullableString => owned_strings.push(
                    self.nullable_parts(value.into_struct_value())?
                        .1
                        .into_pointer_value(),
                ),
                _ => {}
            }
            let ownership = argument.mixed_ownership();
            if ownership.has_shell() {
                temporary_mixed.push((index, value.into_pointer_value(), ownership));
            }
            values.push(value.into());
            lowered.push(value);
        }
        Ok(LoweredCallArguments {
            values,
            lowered,
            owned_strings,
            temporary_mixed,
        })
    }

    fn cleanup_call_arguments(
        &mut self,
        function: mir::FunctionId,
        args: &[mir::Rvalue],
        receiver_present: bool,
        lowered: &LoweredCallArguments<'ctx>,
    ) -> Result<(), BackendError> {
        let callee = function_in(self.program, function)?;
        for string in &lowered.owned_strings {
            self.release_string(*string)?;
        }
        for (index, value, ownership) in &lowered.temporary_mixed {
            if args[*index].transferred_owned_local().is_some() {
                continue;
            }
            let parameter_index = *index + usize::from(receiver_present);
            let parameter = *callee.params.get(parameter_index).ok_or_else(|| {
                malformed_mir(format!(
                    "function{} is missing parameter {parameter_index}",
                    function.0
                ))
            })?;
            if !local_in(callee, parameter)?.owned {
                self.cleanup_mixed_temporary(*value, *ownership)?;
            }
        }
        for index in ordered_owned_argument_indices(args) {
            let argument = &args[index];
            let parameter_index = index + usize::from(receiver_present);
            let parameter = *callee.params.get(parameter_index).ok_or_else(|| {
                malformed_mir(format!(
                    "function{} is missing parameter {parameter_index}",
                    function.0
                ))
            })?;
            if !local_in(callee, parameter)?.owned {
                let value = lowered.lowered[index].into_pointer_value();
                if let Some(class) = argument.owned_temporary_class() {
                    self.defer_or_drop_class_temporary(value, class)?;
                } else if let Some(collection) = argument.owned_temporary_collection() {
                    self.defer_or_drop_collection_temporary(value, collection)?;
                } else if let Some(shared) = argument.owned_temporary_shared() {
                    self.defer_or_drop_owned_shared_temporary(value, shared)?;
                } else if let Some((payload, nullable)) = argument.owned_temporary_payload_enum() {
                    self.drop_payload_enum_at(value, payload, nullable)?;
                } else if argument.mixed_ownership().has_shell() {
                    self.defer_or_cleanup_mixed_temporary(value, argument.mixed_ownership())?;
                }
            }
        }
        Ok(())
    }

    fn cleanup_constructor_arguments(
        &mut self,
        constructor: mir::FunctionId,
        properties: &[mir::PropertyValue],
        args: &[mir::Rvalue],
        lowered: &LoweredCallArguments<'ctx>,
    ) -> Result<(), BackendError> {
        let definition = function_in(self.program, constructor)?;
        let promoted = |index| {
            properties.iter().any(|property| {
                matches!(
                    property.source,
                    mir::PropertyValueSource::ConstructorArgument(argument)
                        if argument == index
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
                self.cleanup_mixed_temporary(*value, *ownership)?;
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
                let value = lowered.lowered[index].into_pointer_value();
                if let Some(class) = argument.owned_temporary_class() {
                    self.defer_or_drop_class_temporary(value, class)?;
                } else if let Some(collection) = argument.owned_temporary_collection() {
                    self.defer_or_drop_collection_temporary(value, collection)?;
                } else if let Some(shared) = argument.owned_temporary_shared() {
                    self.defer_or_drop_owned_shared_temporary(value, shared)?;
                } else if let Some((payload, nullable)) = argument.owned_temporary_payload_enum() {
                    self.drop_payload_enum_at(value, payload, nullable)?;
                } else if argument.mixed_ownership().has_shell() {
                    self.defer_or_cleanup_mixed_temporary(value, argument.mixed_ownership())?;
                }
            }
        }
        let mut strings = lowered.owned_strings.iter();
        for (index, argument) in args.iter().enumerate() {
            if matches!(argument.ty(), mir::Type::String | mir::Type::NullableString) {
                let string = *strings
                    .next()
                    .expect("lowered string arguments preserve source order");
                if !promoted(index) {
                    self.release_string(string)?;
                }
            }
        }
        Ok(())
    }

    fn lower_class_allocation(
        &mut self,
        class: crate::class_layout::ClassId,
        properties: &[mir::PropertyValue],
        args: &[mir::Rvalue],
    ) -> Result<(PointerValue<'ctx>, LoweredCallArguments<'ctx>), BackendError> {
        // Property initializers precede constructor arguments in Doria source order.
        let mut lowered_properties = Vec::with_capacity(properties.len());
        for property in properties {
            lowered_properties.push(match &property.source {
                mir::PropertyValueSource::Expression(value) => Some(self.lower_rvalue(value)?),
                mir::PropertyValueSource::ConstructorArgument(_)
                | mir::PropertyValueSource::ConstructorBody => None,
            });
        }
        let lowered = self.lower_call_arguments(args)?;
        let pointer = self.context.ptr_type(AddressSpace::default());
        let usize_type = self.context.ptr_sized_int_type(self.target_data, None);
        let class_definition = class_definition(self.program, class)?;
        let object = self
            .call_runtime(
                CLASS_ALLOCATE,
                &[pointer.into(), usize_type.into(), usize_type.into()],
                Some(pointer.into()),
                &[
                    self.current_frame.into(),
                    usize_type
                        .const_int(u64::from(class_definition.layout.size), false)
                        .into(),
                    usize_type
                        .const_int(u64::from(class_definition.layout.align), false)
                        .into(),
                ],
            )?
            .ok_or_else(|| backend_failure("class allocation produced no result"))?
            .into_pointer_value();
        for (property, lowered_property) in properties.iter().zip(lowered_properties) {
            let value = match &property.source {
                mir::PropertyValueSource::Expression(_) => lowered_property,
                mir::PropertyValueSource::ConstructorArgument(index) => {
                    Some(*lowered.lowered.get(*index).ok_or_else(|| {
                        malformed_mir(format!("constructor argument {index} does not exist"))
                    })?)
                }
                mir::PropertyValueSource::ConstructorBody => None,
            };
            let Some(value) = value else {
                continue;
            };
            let address = self.lower_property_address_from_value(object, property.property)?;
            let property_ty = property_definition(self.program, property.property)?.ty;
            self.store_value_at_address(address, value, property_ty)?;
        }
        Ok((object, lowered))
    }

    fn lower_method_call(
        &mut self,
        receiver: PointerValue<'ctx>,
        function: mir::FunctionId,
        args: &[mir::Rvalue],
        expects_result: bool,
    ) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        self.lower_call_with_receiver(function, args, expects_result, Some(receiver))
    }

    fn lower_call_with_receiver(
        &mut self,
        function: mir::FunctionId,
        args: &[mir::Rvalue],
        expects_result: bool,
        receiver: Option<PointerValue<'ctx>>,
    ) -> Result<Option<BasicValueEnum<'ctx>>, BackendError> {
        let callee = *self
            .functions
            .get(function.0)
            .ok_or_else(|| malformed_mir(format!("function{} does not exist", function.0)))?;
        let callee_definition = function_in(self.program, function)?;
        let aggregate_result = match callee_definition.return_type {
            mir::ReturnType::Value(mir::Type::PayloadEnum(ty)) => Some((
                self.entry_payload_alloca(ty, false, "call.payload.result")?,
                ty,
                false,
            )),
            mir::ReturnType::Value(mir::Type::NullablePayloadEnum(ty)) => Some((
                self.entry_payload_alloca(ty, true, "call.payload.result")?,
                ty,
                true,
            )),
            mir::ReturnType::Value(_) | mir::ReturnType::Void => None,
        };
        let mut values = Vec::<BasicMetadataValueEnum<'ctx>>::with_capacity(args.len() + 2);
        values.push(self.current_frame.into());
        if let Some((result, _, _)) = aggregate_result {
            values.push(result.into());
        }
        if let Some(home) = self.direct_call_borrow_home(callee_definition, args, receiver)? {
            values.push(home.into());
        }
        if let Some(receiver) = receiver {
            values.push(receiver.into());
        }
        let lowered = self.lower_call_arguments(args)?;
        values.extend(lowered.values.iter().copied());
        let call = build(self.builder.build_call(callee, &values, "call"))?;
        apply_call_abi_attributes(self.context, call, function_in(self.program, function)?)?;
        let result = if expects_result {
            if let Some((result, _, _)) = aggregate_result {
                Some(result.into())
            } else {
                Some(call.try_as_basic_value().basic().ok_or_else(|| {
                    malformed_mir(format!("call to function{} produced no result", function.0))
                })?)
            }
        } else {
            None
        };
        self.cleanup_call_arguments(function, args, receiver.is_some(), &lowered)?;
        Ok(result)
    }
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

fn apply_call_abi_attributes(
    context: &Context,
    call: inkwell::values::CallSiteValue<'_>,
    function: &mir::Function,
) -> Result<(), BackendError> {
    if function.checked_effects.is_empty() {
        if let mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Integer(ty))) =
            function.return_type
        {
            apply_call_integer_extension_attribute(context, call, AttributeLoc::Return, ty);
        }
    }
    let source_parameter_offset = native_closure_abi::NativeCallableSignaturePlan::direct(function)
        .source_parameter_offset() as u32;
    for (index, parameter) in function.params.iter().enumerate() {
        let local = local_in(function, *parameter)?;
        if let mir::Type::Scalar(mir::ScalarType::Integer(ty)) = local.ty {
            apply_call_integer_extension_attribute(
                context,
                call,
                AttributeLoc::Param(index as u32 + source_parameter_offset),
                ty,
            );
        }
    }
    Ok(())
}

fn apply_call_integer_extension_attribute(
    context: &Context,
    call: inkwell::values::CallSiteValue<'_>,
    location: AttributeLoc,
    ty: IntegerType,
) {
    if ty.bit_width() == 64 {
        return;
    }
    let name = if ty.is_signed() { "signext" } else { "zeroext" };
    let kind = inkwell::attributes::Attribute::get_named_enum_kind_id(name);
    call.add_attribute(location, context.create_enum_attribute(kind, 0));
}

impl<'ctx> FunctionLowerer<'ctx, '_> {
    fn lower_condition_to_branch(
        &mut self,
        condition: &mir::BoolExpression,
        then_block: BasicBlock<'ctx>,
        else_block: BasicBlock<'ctx>,
    ) -> Result<(), BackendError> {
        match condition {
            mir::BoolExpression::NullableFunctionIsPresent(value) => {
                let value = self.lower_nullable_function_expression(value)?;
                let (descriptor, _) = self.closure_parts(value)?;
                let present = build(self.builder.build_int_compare(
                    IntPredicate::NE,
                    descriptor,
                    descriptor.get_type().const_null(),
                    "nullable.closure.present",
                ))?;
                build(
                    self.builder
                        .build_conditional_branch(present, then_block, else_block),
                )?;
            }
            mir::BoolExpression::PayloadEnumIsCase {
                local,
                ty,
                case,
                nullable,
            } => {
                let source = local_slot(&self.local_slots, *local)?;
                let source = if *nullable {
                    let present = build(self.builder.build_load(
                        self.context.i8_type(),
                        source,
                        "payload.case.present",
                    ))?
                    .into_int_value();
                    let present = build(self.builder.build_int_compare(
                        IntPredicate::NE,
                        present,
                        self.context.i8_type().const_zero(),
                        "payload.case.is-present",
                    ))?;
                    let present_block = self
                        .context
                        .append_basic_block(current_function(&self.builder)?, "payload.case.some");
                    build(self.builder.build_conditional_branch(
                        present,
                        present_block,
                        else_block,
                    ))?;
                    self.builder.position_at_end(present_block);
                    self.byte_offset(source, ty.nullable_payload_offset, "payload.case.value")?
                } else {
                    source
                };
                let definition = enum_definition(self.program, ty.id)?;
                let case_definition = definition
                    .cases
                    .get(case.index)
                    .filter(|definition| definition.id == *case)
                    .ok_or_else(|| malformed_mir("payload-enum case test references no case"))?;
                let tag_type = match definition.layout.tag_width {
                    1 => self.context.i8_type(),
                    2 => self.context.i16_type(),
                    4 => self.context.i32_type(),
                    _ => return Err(malformed_mir("payload enum tag has unsupported width")),
                };
                let tag_address = self.byte_offset(
                    source,
                    definition.layout.tag_offset,
                    "payload.case.tag-address",
                )?;
                let tag = build(self.builder.build_load(
                    tag_type,
                    tag_address,
                    "payload.case.tag",
                ))?
                .into_int_value();
                let matches = build(self.builder.build_int_compare(
                    IntPredicate::EQ,
                    tag,
                    tag_type.const_int(u64::from(case_definition.tag), false),
                    "payload.case.matches",
                ))?;
                build(
                    self.builder
                        .build_conditional_branch(matches, then_block, else_block),
                )?;
            }
            mir::BoolExpression::Use { operand } => {
                let value = self.lower_bool_operand(operand)?;
                let condition = build(self.builder.build_int_compare(
                    IntPredicate::NE,
                    value,
                    self.context.i8_type().const_zero(),
                    "bool.condition",
                ))?;
                build(
                    self.builder
                        .build_conditional_branch(condition, then_block, else_block),
                )?;
            }
            mir::BoolExpression::Compare { op, left, right } => {
                let ty = left.ty();
                let left = self.lower_value_expression(left)?;
                let right = self.lower_value_expression(right)?;
                let condition = match ty {
                    mir::ScalarType::Integer(ty) => build(self.builder.build_int_compare(
                        integer_compare_predicate(*op, ty),
                        left.into_int_value(),
                        right.into_int_value(),
                        "integer.compare",
                    ))?,
                    mir::ScalarType::Float(_) => build(self.builder.build_float_compare(
                        float_compare_predicate(*op),
                        left.into_float_value(),
                        right.into_float_value(),
                        "float.compare",
                    ))?,
                    mir::ScalarType::Bool => build(self.builder.build_int_compare(
                        bool_compare_predicate(*op),
                        left.into_int_value(),
                        right.into_int_value(),
                        "bool.compare",
                    ))?,
                    mir::ScalarType::Enum(_) => build(self.builder.build_int_compare(
                        bool_compare_predicate(*op),
                        left.into_int_value(),
                        right.into_int_value(),
                        "enum.compare",
                    ))?,
                };
                build(
                    self.builder
                        .build_conditional_branch(condition, then_block, else_block),
                )?;
            }
            mir::BoolExpression::StringCompare { op, left, right } => {
                let pointer = self.context.ptr_type(AddressSpace::default());
                let left = self.lower_string_expression(left)?;
                let right = self.lower_string_expression(right)?;
                let compared = self
                    .call_runtime(
                        STRING_COMPARE,
                        &[pointer.into(), pointer.into()],
                        Some(self.context.i32_type().into()),
                        &[left.into(), right.into()],
                    )?
                    .ok_or_else(|| backend_failure("string comparison produced no result"))?
                    .into_int_value();
                self.release_string(left)?;
                self.release_string(right)?;
                let predicate = match op {
                    mir::CompareOp::Equal => IntPredicate::EQ,
                    mir::CompareOp::NotEqual => IntPredicate::NE,
                    mir::CompareOp::Less => IntPredicate::SLT,
                    mir::CompareOp::LessEqual => IntPredicate::SLE,
                    mir::CompareOp::Greater => IntPredicate::SGT,
                    mir::CompareOp::GreaterEqual => IntPredicate::SGE,
                };
                let condition = build(self.builder.build_int_compare(
                    predicate,
                    compared,
                    self.context.i32_type().const_zero(),
                    "string.compare",
                ))?;
                build(
                    self.builder
                        .build_conditional_branch(condition, then_block, else_block),
                )?;
            }
            mir::BoolExpression::NullableStringCompare { op, left, right } => {
                let pointer = self.context.ptr_type(AddressSpace::default());
                let left = self.lower_nullable_string_expression(left)?;
                let right = self.lower_nullable_string_expression(right)?;
                let left = self.nullable_parts(left)?.1.into_pointer_value();
                let right = self.nullable_parts(right)?.1.into_pointer_value();
                let equal = self
                    .call_runtime(
                        NULLABLE_STRING_EQUAL,
                        &[pointer.into(), pointer.into()],
                        Some(self.context.i8_type().into()),
                        &[left.into(), right.into()],
                    )?
                    .ok_or_else(|| {
                        backend_failure("nullable-string comparison produced no result")
                    })?
                    .into_int_value();
                self.release_string(left)?;
                self.release_string(right)?;
                let condition = match op {
                    mir::CompareOp::Equal => build(self.builder.build_int_compare(
                        IntPredicate::NE,
                        equal,
                        self.context.i8_type().const_zero(),
                        "nullable-string.equal",
                    ))?,
                    mir::CompareOp::NotEqual => build(self.builder.build_int_compare(
                        IntPredicate::EQ,
                        equal,
                        self.context.i8_type().const_zero(),
                        "nullable-string.not-equal",
                    ))?,
                    _ => return Err(malformed_mir("ordered nullable comparison is invalid")),
                };
                build(
                    self.builder
                        .build_conditional_branch(condition, then_block, else_block),
                )?;
            }
            mir::BoolExpression::NullablePayloadEnumIsPresent(value) => {
                let owned = value.owned_temporary();
                let ty = value.ty();
                let address = self.lower_nullable_payload_enum_expression(value)?;
                let present = build(self.builder.build_load(
                    self.context.i8_type(),
                    address,
                    "nullable.payload.present",
                ))?
                .into_int_value();
                if owned {
                    self.drop_payload_enum_at(address, ty, true)?;
                }
                let condition = build(self.builder.build_int_compare(
                    IntPredicate::NE,
                    present,
                    self.context.i8_type().const_zero(),
                    "nullable.payload.is-present",
                ))?;
                build(
                    self.builder
                        .build_conditional_branch(condition, then_block, else_block),
                )?;
            }
            mir::BoolExpression::PayloadEnumCompare { op, left, right } => {
                let ty = left.ty();
                let left_owned = left.owned_temporary();
                let right_owned = right.owned_temporary();
                let left_address = self.lower_payload_enum_expression(left)?;
                let right_address = self.lower_payload_enum_expression(right)?;
                let equal = self.payload_enum_equal_value(left_address, right_address, ty)?;
                if right_owned {
                    self.drop_payload_enum_at(right_address, ty, false)?;
                }
                if left_owned {
                    self.drop_payload_enum_at(left_address, ty, false)?;
                }
                let condition = build(self.builder.build_int_compare(
                    if matches!(op, mir::CompareOp::Equal) {
                        IntPredicate::NE
                    } else {
                        IntPredicate::EQ
                    },
                    equal,
                    self.context.i8_type().const_zero(),
                    "payload.compare.result",
                ))?;
                build(
                    self.builder
                        .build_conditional_branch(condition, then_block, else_block),
                )?;
            }
            mir::BoolExpression::NullablePayloadEnumCompare { op, left, right } => {
                let ty = left.ty();
                let left_owned = left.owned_temporary();
                let right_owned = right.owned_temporary();
                let left_address = self.lower_nullable_payload_enum_expression(left)?;
                let right_address = self.lower_nullable_payload_enum_expression(right)?;
                let equal =
                    self.nullable_payload_enum_equal_value(left_address, right_address, ty)?;
                if right_owned {
                    self.drop_payload_enum_at(right_address, ty, true)?;
                }
                if left_owned {
                    self.drop_payload_enum_at(left_address, ty, true)?;
                }
                let condition = build(self.builder.build_int_compare(
                    if matches!(op, mir::CompareOp::Equal) {
                        IntPredicate::NE
                    } else {
                        IntPredicate::EQ
                    },
                    equal,
                    self.context.i8_type().const_zero(),
                    "nullable.payload.compare.result",
                ))?;
                build(
                    self.builder
                        .build_conditional_branch(condition, then_block, else_block),
                )?;
            }
            mir::BoolExpression::Not(condition) => {
                self.lower_condition_to_branch(condition, else_block, then_block)?;
            }
            mir::BoolExpression::Binary {
                op: mir::BoolBinaryOp::And,
                left,
                right,
            } => {
                let right_block = self
                    .context
                    .append_basic_block(current_function(&self.builder)?, "and.right");
                self.lower_condition_to_branch(left, right_block, else_block)?;
                self.builder.position_at_end(right_block);
                self.lower_condition_to_branch(right, then_block, else_block)?;
            }
            mir::BoolExpression::Binary {
                op: mir::BoolBinaryOp::Or,
                left,
                right,
            } => {
                let right_block = self
                    .context
                    .append_basic_block(current_function(&self.builder)?, "or.right");
                self.lower_condition_to_branch(left, then_block, right_block)?;
                self.builder.position_at_end(right_block);
                self.lower_condition_to_branch(right, then_block, else_block)?;
            }
            mir::BoolExpression::Binary {
                op: mir::BoolBinaryOp::Xor,
                left,
                right,
            } => {
                let left = self.lower_condition_value(left)?;
                let right = self.lower_condition_value(right)?;
                let value = build(self.builder.build_xor(left, right, "bool.xor"))?;
                let condition = build(self.builder.build_int_compare(
                    IntPredicate::NE,
                    value,
                    self.context.i8_type().const_zero(),
                    "bool.xor.condition",
                ))?;
                build(
                    self.builder
                        .build_conditional_branch(condition, then_block, else_block),
                )?;
            }
            mir::BoolExpression::Call { function, args } => {
                let value = self
                    .lower_call(*function, args, true)?
                    .ok_or_else(|| malformed_mir("bool call produced no result"))?
                    .into_int_value();
                let condition = build(self.builder.build_int_compare(
                    IntPredicate::NE,
                    value,
                    self.context.i8_type().const_zero(),
                    "bool.call.condition",
                ))?;
                build(
                    self.builder
                        .build_conditional_branch(condition, then_block, else_block),
                )?;
            }
            mir::BoolExpression::NullableScalarIsPresent(value) => {
                let value = self.lower_nullable_scalar_expression(value)?;
                let (present, _) = self.nullable_parts(value)?;
                let condition = build(self.builder.build_int_compare(
                    IntPredicate::NE,
                    present,
                    self.present_word(false),
                    "nullable-scalar.present",
                ))?;
                build(
                    self.builder
                        .build_conditional_branch(condition, then_block, else_block),
                )?;
            }
            mir::BoolExpression::NullableClassIsPresent(value) => {
                let owned = value.owned_temporary_class();
                let value = self.lower_nullable_class_expression(value)?;
                if let Some(class) = owned {
                    self.defer_or_drop_class_temporary(value, class)?;
                }
                let condition = build(
                    self.builder
                        .build_is_not_null(value, "nullable-class.present"),
                )?;
                build(
                    self.builder
                        .build_conditional_branch(condition, then_block, else_block),
                )?;
            }
            mir::BoolExpression::NullableCollectionIsPresent(value) => {
                let owned = value.owned_temporary_collection();
                let value = self.lower_nullable_collection_expression(value)?;
                if let Some(collection) = owned {
                    self.defer_or_drop_collection_temporary(value, collection)?;
                }
                let condition = build(
                    self.builder
                        .build_is_not_null(value, "nullable-collection.present"),
                )?;
                build(
                    self.builder
                        .build_conditional_branch(condition, then_block, else_block),
                )?;
            }
            mir::BoolExpression::NullableSharedReferenceIsPresent(value) => {
                let owned = value.owned_temporary().is_some();
                let value = self.lower_nullable_shared_reference_expression(value)?;
                if owned {
                    self.defer_or_drop_shared_temporary(value, false)?;
                }
                let condition = build(
                    self.builder
                        .build_is_not_null(value, "nullable-shared.present"),
                )?;
                build(
                    self.builder
                        .build_conditional_branch(condition, then_block, else_block),
                )?;
            }
            mir::BoolExpression::NullableWeakReferenceIsPresent(value) => {
                let owned = value.owned_temporary().is_some();
                let value = self.lower_nullable_weak_reference_expression(value)?;
                if owned {
                    self.defer_or_drop_shared_temporary(value, true)?;
                }
                let condition = build(
                    self.builder
                        .build_is_not_null(value, "nullable-weak.present"),
                )?;
                build(
                    self.builder
                        .build_conditional_branch(condition, then_block, else_block),
                )?;
            }
            mir::BoolExpression::NullableWritableSharedReferenceIsPresent(value) => {
                let owned = value.owned_temporary();
                let value = self.lower_nullable_writable_shared_reference_expression(value)?;
                if owned {
                    self.defer_or_drop_writable_shared_temporary(value, WRITABLE_SHARED_RELEASE)?;
                }
                let condition = build(
                    self.builder
                        .build_is_not_null(value, "nullable-writable-shared.present"),
                )?;
                build(
                    self.builder
                        .build_conditional_branch(condition, then_block, else_block),
                )?;
            }
            mir::BoolExpression::NullableWritableWeakReferenceIsPresent(value) => {
                let owned = value.owned_temporary();
                let value = self.lower_nullable_writable_weak_reference_expression(value)?;
                if owned {
                    self.defer_or_drop_writable_shared_temporary(
                        value,
                        WRITABLE_SHARED_RELEASE_WEAK,
                    )?;
                }
                let condition = build(
                    self.builder
                        .build_is_not_null(value, "nullable-writable-weak.present"),
                )?;
                build(
                    self.builder
                        .build_conditional_branch(condition, then_block, else_block),
                )?;
            }
            mir::BoolExpression::NullableSharedReferenceAccessIsPresent(value) => {
                let owned = value.owned_temporary();
                let lowered = self.lower_nullable_shared_reference_access_expression(value)?;
                if owned {
                    self.defer_or_drop_writable_shared_temporary(
                        lowered,
                        if value.writable() {
                            WRITABLE_SHARED_RELEASE_WRITABLE_ACCESS
                        } else {
                            WRITABLE_SHARED_RELEASE_READONLY_ACCESS
                        },
                    )?;
                }
                let condition = build(
                    self.builder
                        .build_is_not_null(lowered, "nullable-shared-access.present"),
                )?;
                build(
                    self.builder
                        .build_conditional_branch(condition, then_block, else_block),
                )?;
            }
            mir::BoolExpression::NullableMixedIsPresent(value) => {
                let ownership = value.ownership();
                let value = self.lower_nullable_mixed_expression(value)?;
                let condition = build(
                    self.builder
                        .build_is_not_null(value, "nullable-mixed.present"),
                )?;
                self.cleanup_mixed_temporary(value, ownership)?;
                build(
                    self.builder
                        .build_conditional_branch(condition, then_block, else_block),
                )?;
            }
            mir::BoolExpression::NullableErrorIsPresent(value) => {
                let value = self.lower_nullable_error_expression(value)?;
                let (object, _) = self.error_parts(value)?;
                let condition = build(
                    self.builder
                        .build_is_not_null(object, "nullable-error.present"),
                )?;
                build(
                    self.builder
                        .build_conditional_branch(condition, then_block, else_block),
                )?;
            }
            mir::BoolExpression::MixedIs { mixed, tag } => {
                let condition = self.lower_mixed_is(mixed, *tag)?;
                build(
                    self.builder
                        .build_conditional_branch(condition, then_block, else_block),
                )?;
            }
            mir::BoolExpression::Coalesce { left, right } => {
                let left = self.lower_nullable_scalar_expression(left)?;
                let (present, payload) = self.nullable_parts(left)?;
                let function = current_function(&self.builder)?;
                let use_left = self
                    .context
                    .append_basic_block(function, "bool.coalesce.left");
                let use_right = self
                    .context
                    .append_basic_block(function, "bool.coalesce.right");
                let present = build(self.builder.build_int_compare(
                    IntPredicate::NE,
                    present,
                    self.present_word(false),
                    "bool.coalesce.present",
                ))?;
                build(
                    self.builder
                        .build_conditional_branch(present, use_left, use_right),
                )?;
                self.builder.position_at_end(use_left);
                let payload = payload.into_int_value();
                let payload = build(self.builder.build_int_compare(
                    IntPredicate::NE,
                    payload,
                    self.context.i8_type().const_zero(),
                    "bool.coalesce.value",
                ))?;
                build(
                    self.builder
                        .build_conditional_branch(payload, then_block, else_block),
                )?;
                self.builder.position_at_end(use_right);
                self.lower_condition_to_branch(right, then_block, else_block)?;
            }
            mir::BoolExpression::CollectionHas {
                collection,
                value,
                op,
            } => {
                let pointer = self.context.ptr_type(AddressSpace::default());
                let local = local_in(self.function, *collection)?;
                let mir::Type::Collection(collection_type) = local.ty else {
                    return Err(malformed_mir("collection has uses non-collection local"));
                };
                let definition = self.collection_definition(collection_type)?.clone();
                let stored_needle_type = if *op == mir::CollectionMembershipOp::Contains {
                    definition.key.unwrap_or(definition.value)
                } else {
                    definition.value
                };
                if let Some((payload, nullable)) = Self::payload_enum_storage(stored_needle_type) {
                    if *op == mir::CollectionMembershipOp::Add {
                        return Err(malformed_mir(
                            "payload enum elements cannot use set insertion",
                        ));
                    }
                    let owned = Self::payload_enum_rvalue_is_owned(value);
                    let needle = self.lower_rvalue(value)?.into_pointer_value();
                    let collection_value = self.collection_pointer(*collection)?;
                    let (found, index) = self.lower_payload_enum_collection_search(
                        collection_value,
                        needle,
                        payload,
                        nullable,
                    )?;
                    if *op == mir::CollectionMembershipOp::Remove {
                        let function = current_function(&self.builder)?;
                        let remove = self
                            .context
                            .append_basic_block(function, "aggregate.remove.found");
                        let removed = self
                            .context
                            .append_basic_block(function, "aggregate.remove.done");
                        let condition = build(self.builder.build_int_compare(
                            IntPredicate::NE,
                            found,
                            self.context.i8_type().const_zero(),
                            "aggregate.remove.present",
                        ))?;
                        build(
                            self.builder
                                .build_conditional_branch(condition, remove, removed),
                        )?;
                        self.builder.position_at_end(remove);
                        let destination =
                            self.entry_payload_alloca(payload, nullable, "aggregate.remove.value")?;
                        let _ = self.call_runtime(
                            COLLECTION_AGGREGATE_REMOVE_AT_INTO,
                            &[
                                pointer.into(),
                                pointer.into(),
                                self.context
                                    .ptr_sized_int_type(self.target_data, None)
                                    .into(),
                                pointer.into(),
                            ],
                            None,
                            &[
                                self.current_frame.into(),
                                collection_value.into(),
                                index.into(),
                                destination.into(),
                            ],
                        )?;
                        self.drop_payload_enum_at(destination, payload, nullable)?;
                        build(self.builder.build_unconditional_branch(removed))?;
                        self.builder.position_at_end(removed);
                    }
                    if owned {
                        self.drop_payload_enum_at(needle, payload, nullable)?;
                    }
                    let condition = build(self.builder.build_int_compare(
                        IntPredicate::NE,
                        found,
                        self.context.i8_type().const_zero(),
                        "aggregate.collection.found",
                    ))?;
                    build(
                        self.builder
                            .build_conditional_branch(condition, then_block, else_block),
                    )?;
                    return Ok(());
                }
                let mixed_ownership = value.mixed_ownership();
                let (needle_present, needle, needle_type) =
                    if nullable_payload_type(stored_needle_type).is_some() {
                        self.lower_nullable_collection_parts(value, stored_needle_type)?
                    } else {
                        (
                            self.context.i8_type().const_int(1, false),
                            self.lower_rvalue(value)?,
                            stored_needle_type,
                        )
                    };
                let needle_word = self.value_to_collection_word(needle, needle_type)?;
                let collection_value = self.collection_pointer(*collection)?;
                let kind = self.collection_compare_kind(needle_type)?;
                let found = match op {
                    mir::CollectionMembershipOp::Contains
                    | mir::CollectionMembershipOp::ContainsValue => {
                        let name = if *op == mir::CollectionMembershipOp::Contains
                            && definition.key.is_some()
                        {
                            COLLECTION_KEYED_HAS
                        } else {
                            COLLECTION_CONTAINS
                        };
                        let (parameter_types, arguments) = if name == COLLECTION_KEYED_HAS {
                            (
                                vec![
                                    pointer.into(),
                                    self.context.i64_type().into(),
                                    self.context.i8_type().into(),
                                ],
                                vec![collection_value.into(), needle_word.into(), kind.into()],
                            )
                        } else {
                            (
                                vec![
                                    pointer.into(),
                                    self.context.i64_type().into(),
                                    self.context.i8_type().into(),
                                    self.context.i8_type().into(),
                                ],
                                vec![
                                    collection_value.into(),
                                    needle_word.into(),
                                    needle_present.into(),
                                    kind.into(),
                                ],
                            )
                        };
                        let found = self
                            .call_runtime(
                                name,
                                &parameter_types,
                                Some(self.context.i8_type().into()),
                                &arguments,
                            )?
                            .ok_or_else(|| {
                                backend_failure("collection membership produced no result")
                            })?
                            .into_int_value();
                        if matches!(
                            stored_needle_type,
                            mir::Type::Mixed | mir::Type::NullableMixed
                        ) {
                            self.cleanup_mixed_temporary(
                                needle.into_pointer_value(),
                                mixed_ownership,
                            )?;
                        } else {
                            self.drop_stored_value(needle, stored_needle_type)?;
                        }
                        found
                    }
                    mir::CollectionMembershipOp::Add => {
                        let inserted = self
                            .call_runtime(
                                COLLECTION_PUSH_UNIQUE,
                                &[
                                    pointer.into(),
                                    self.context.i64_type().into(),
                                    self.context.i8_type().into(),
                                    self.context.i8_type().into(),
                                ],
                                Some(self.context.i8_type().into()),
                                &[
                                    collection_value.into(),
                                    needle_word.into(),
                                    needle_present.into(),
                                    kind.into(),
                                ],
                            )?
                            .ok_or_else(|| backend_failure("set insertion produced no result"))?
                            .into_int_value();
                        if matches!(
                            stored_needle_type,
                            mir::Type::Mixed | mir::Type::NullableMixed
                        ) {
                            let rejected = build(self.builder.build_int_compare(
                                IntPredicate::EQ,
                                inserted,
                                self.context.i8_type().const_zero(),
                                "mixed.set.rejected",
                            ))?;
                            self.cleanup_mixed_temporary_if(
                                rejected,
                                needle.into_pointer_value(),
                                mixed_ownership,
                            )?;
                        } else {
                            self.drop_value_unless(inserted, needle, stored_needle_type)?;
                        }
                        inserted
                    }
                    mir::CollectionMembershipOp::Remove => {
                        let removed_slot =
                            self.entry_alloca(self.context.i64_type(), "set.removed.value")?;
                        let removed_present_slot = self
                            .entry_alloca(self.context.i8_type(), "collection.removed.present")?;
                        let removed = self
                            .call_runtime(
                                COLLECTION_REMOVE_VALUE,
                                &[
                                    pointer.into(),
                                    self.context.i64_type().into(),
                                    self.context.i8_type().into(),
                                    self.context.i8_type().into(),
                                    pointer.into(),
                                    pointer.into(),
                                ],
                                Some(self.context.i8_type().into()),
                                &[
                                    collection_value.into(),
                                    needle_word.into(),
                                    needle_present.into(),
                                    kind.into(),
                                    removed_slot.into(),
                                    removed_present_slot.into(),
                                ],
                            )?
                            .ok_or_else(|| backend_failure("set removal produced no result"))?
                            .into_int_value();
                        let removed_word = build(self.builder.build_load(
                            self.context.i64_type(),
                            removed_slot,
                            "set.removed.word",
                        ))?
                        .into_int_value();
                        let removed_value =
                            self.collection_word_to_value(removed_word, needle_type)?;
                        let removed_present = build(self.builder.build_load(
                            self.context.i8_type(),
                            removed_present_slot,
                            "collection.removed.present.value",
                        ))?
                        .into_int_value();
                        let should_drop = build(self.builder.build_and(
                            removed,
                            removed_present,
                            "collection.removed.should-drop",
                        ))?;
                        self.drop_value_if(should_drop, removed_value, stored_needle_type)?;
                        if matches!(
                            stored_needle_type,
                            mir::Type::Mixed | mir::Type::NullableMixed
                        ) {
                            self.cleanup_mixed_temporary(
                                needle.into_pointer_value(),
                                mixed_ownership,
                            )?;
                        } else {
                            self.drop_stored_value(needle, stored_needle_type)?;
                        }
                        removed
                    }
                };
                let found = build(self.builder.build_int_compare(
                    IntPredicate::NE,
                    found,
                    self.context.i8_type().const_zero(),
                    "collection.found",
                ))?;
                build(
                    self.builder
                        .build_conditional_branch(found, then_block, else_block),
                )?;
            }
            mir::BoolExpression::CollectionIsEmpty { collection } => {
                let pointer = self.context.ptr_type(AddressSpace::default());
                let usize_type = self.context.ptr_sized_int_type(self.target_data, None);
                let collection = self.collection_pointer(*collection)?;
                let length = self
                    .call_runtime(
                        COLLECTION_LENGTH,
                        &[pointer.into()],
                        Some(usize_type.into()),
                        &[collection.into()],
                    )?
                    .ok_or_else(|| backend_failure("collection length produced no result"))?
                    .into_int_value();
                let empty = build(self.builder.build_int_compare(
                    IntPredicate::EQ,
                    length,
                    usize_type.const_zero(),
                    "collection.empty",
                ))?;
                build(
                    self.builder
                        .build_conditional_branch(empty, then_block, else_block),
                )?;
            }
            mir::BoolExpression::CollectionEqual { left, right } => {
                let pointer = self.context.ptr_type(AddressSpace::default());
                let left = self.collection_pointer(*left)?;
                let right = self.collection_pointer(*right)?;
                let equal = self
                    .call_runtime(
                        BYTES_EQUAL,
                        &[pointer.into(), pointer.into()],
                        Some(self.context.i8_type().into()),
                        &[left.into(), right.into()],
                    )?
                    .ok_or_else(|| backend_failure("Bytes equality produced no result"))?
                    .into_int_value();
                let equal = build(self.builder.build_int_compare(
                    IntPredicate::NE,
                    equal,
                    self.context.i8_type().const_zero(),
                    "bytes.equal",
                ))?;
                build(
                    self.builder
                        .build_conditional_branch(equal, then_block, else_block),
                )?;
            }
        }
        Ok(())
    }

    fn lower_condition_value(
        &mut self,
        condition: &mir::BoolExpression,
    ) -> Result<IntValue<'ctx>, BackendError> {
        let function = current_function(&self.builder)?;
        let true_block = self.context.append_basic_block(function, "bool.true");
        let false_block = self.context.append_basic_block(function, "bool.false");
        let done_block = self.context.append_basic_block(function, "bool.done");
        self.lower_condition_to_branch(condition, true_block, false_block)?;

        self.builder.position_at_end(true_block);
        build(self.builder.build_unconditional_branch(done_block))?;

        self.builder.position_at_end(false_block);
        build(self.builder.build_unconditional_branch(done_block))?;

        self.builder.position_at_end(done_block);
        let phi = build(self.builder.build_phi(self.context.i8_type(), "bool.value"))?;
        let true_value = self.context.i8_type().const_int(1, false);
        let false_value = self.context.i8_type().const_zero();
        phi.add_incoming(&[(&true_value, true_block), (&false_value, false_block)]);
        Ok(phi.as_basic_value().into_int_value())
    }

    fn lower_bool_operand(
        &mut self,
        operand: &mir::Operand,
    ) -> Result<IntValue<'ctx>, BackendError> {
        match operand {
            mir::Operand::StringIntrinsic(call) => {
                Ok(self.lower_string_intrinsic_call(call)?.into_int_value())
            }
            mir::Operand::Scalar(mir::ScalarValue::Bool(value)) => {
                Ok(self.context.i8_type().const_int(u64::from(*value), false))
            }
            mir::Operand::Local(local) => Ok(build(self.builder.build_load(
                self.context.i8_type(),
                local_slot(&self.local_slots, *local)?,
                "bool.local",
            ))?
            .into_int_value()),
            mir::Operand::NullablePayload(local) => {
                let value = build(self.builder.build_load(
                    llvm_type(
                        self.context,
                        self.target_data,
                        mir::Type::NullableScalar(mir::ScalarType::Bool),
                    ),
                    local_slot(&self.local_slots, *local)?,
                    "bool.nullable.local",
                ))?
                .into_struct_value();
                Ok(self.nullable_parts(value)?.1.into_int_value())
            }
            mir::Operand::MixedPayload { mixed, tag } => {
                if !matches!(tag, mir::MixedTag::Bool) {
                    return Err(malformed_mir("bool mixed payload uses non-bool tag"));
                }
                Ok(self.lower_mixed_payload(*mixed, *tag)?.into_int_value())
            }
            mir::Operand::Static(id) => Ok(build(self.builder.build_load(
                self.context.i8_type(),
                self.static_address(*id)?,
                "bool.static",
            ))?
            .into_int_value()),
            mir::Operand::Property { object, property } => Ok(build(self.builder.build_load(
                self.context.i8_type(),
                self.lower_property_address(*object, *property)?,
                "bool.property",
            ))?
            .into_int_value()),
            mir::Operand::CollectionIndex {
                positional,
                collection,
                index,
                remove,
            } => Ok(self
                .lower_collection_index(*collection, index, *remove, *positional)?
                .into_int_value()),
            mir::Operand::CollectionKeyAt { collection, offset } => Ok(self
                .lower_collection_key_at(
                    *collection,
                    offset,
                    mir::Type::Scalar(mir::ScalarType::Bool),
                )?
                .into_int_value()),
            _ => Err(malformed_mir("bool expression has an incompatible operand")),
        }
    }

    fn lower_enum_operand(
        &mut self,
        enum_id: crate::enums::EnumId,
        operand: &mir::Operand,
    ) -> Result<IntValue<'ctx>, BackendError> {
        let ty = self.context.i32_type();
        match operand {
            mir::Operand::Scalar(mir::ScalarValue::Enum(value)) if value.enum_id == enum_id => {
                Ok(ty.const_int(value.case_id.index as u64, false))
            }
            mir::Operand::Local(local) => Ok(build(self.builder.build_load(
                ty,
                local_slot(&self.local_slots, *local)?,
                "enum.local",
            ))?
            .into_int_value()),
            mir::Operand::NullablePayload(local) => {
                let value = build(self.builder.build_load(
                    llvm_type(
                        self.context,
                        self.target_data,
                        mir::Type::NullableScalar(mir::ScalarType::Enum(enum_id)),
                    ),
                    local_slot(&self.local_slots, *local)?,
                    "enum.nullable.local",
                ))?
                .into_struct_value();
                Ok(self.nullable_parts(value)?.1.into_int_value())
            }
            mir::Operand::MixedPayload { mixed, tag } => {
                if *tag != mir::MixedTag::Enum(enum_id) {
                    return Err(malformed_mir("enum mixed payload uses another enum type"));
                }
                Ok(self.lower_mixed_payload(*mixed, *tag)?.into_int_value())
            }
            mir::Operand::Static(id) => Ok(build(self.builder.build_load(
                ty,
                self.static_address(*id)?,
                "enum.static",
            ))?
            .into_int_value()),
            mir::Operand::Property { object, property } => Ok(build(self.builder.build_load(
                ty,
                self.lower_property_address(*object, *property)?,
                "enum.property",
            ))?
            .into_int_value()),
            mir::Operand::CollectionIndex {
                positional,
                collection,
                index,
                remove,
            } => Ok(self
                .lower_collection_index(*collection, index, *remove, *positional)?
                .into_int_value()),
            mir::Operand::CollectionKeyAt { collection, offset } => Ok(self
                .lower_collection_key_at(
                    *collection,
                    offset,
                    mir::Type::Scalar(mir::ScalarType::Enum(enum_id)),
                )?
                .into_int_value()),
            _ => Err(malformed_mir("enum expression has an incompatible operand")),
        }
    }

    fn lower_panic_if_code(
        &mut self,
        condition: IntValue<'ctx>,
        code: &'static str,
        span: crate::source::Span,
    ) -> Result<(), BackendError> {
        self.lower_panic_if_code_with_site(condition, code, Some(span))
    }

    fn lower_panic_if_code_at_active_site(
        &mut self,
        condition: IntValue<'ctx>,
        code: &'static str,
    ) -> Result<(), BackendError> {
        self.lower_panic_if_code_with_site(condition, code, None)
    }

    fn lower_panic_if_code_with_site(
        &mut self,
        condition: IntValue<'ctx>,
        code: &'static str,
        span: Option<crate::source::Span>,
    ) -> Result<(), BackendError> {
        let function = current_function(&self.builder)?;
        let panic_block = self
            .context
            .append_basic_block(function, "panic.catalogued");
        let continue_block = self
            .context
            .append_basic_block(function, "panic.catalogued.continue");
        build(
            self.builder
                .build_conditional_branch(condition, panic_block, continue_block),
        )?;
        self.builder.position_at_end(panic_block);
        if let Some(span) = span {
            self.set_active_panic_site(span)?;
        }
        self.lower_runtime_panic_code(code.as_bytes(), &[])?;
        self.builder.position_at_end(continue_block);
        Ok(())
    }

    fn lower_panic_if_signed_fact(
        &mut self,
        condition: IntValue<'ctx>,
        code: &'static str,
        fact_name: &'static str,
        value: IntValue<'ctx>,
        span: crate::source::Span,
    ) -> Result<(), BackendError> {
        let function = current_function(&self.builder)?;
        let panic_block = self
            .context
            .append_basic_block(function, "panic.catalogued.fact");
        let continue_block = self
            .context
            .append_basic_block(function, "panic.catalogued.fact.continue");
        build(
            self.builder
                .build_conditional_branch(condition, panic_block, continue_block),
        )?;
        self.builder.position_at_end(panic_block);
        self.set_active_panic_site(span)?;
        let pointer = self.context.ptr_type(AddressSpace::default());
        let usize_type = self.context.ptr_sized_int_type(self.target_data, None);
        let runtime = self.runtime_function(
            "dr_v2_panic_signed_fact",
            &[
                pointer.into(),
                pointer.into(),
                usize_type.into(),
                pointer.into(),
                usize_type.into(),
                self.context.i64_type().into(),
            ],
            None,
        );
        let code_pointer = self.define_data(code.as_bytes(), "panic.code");
        let fact_name_pointer = self.define_data(fact_name.as_bytes(), "panic.fact-name");
        build(self.builder.build_call(
            runtime,
            &[
                self.current_frame.into(),
                code_pointer.into(),
                usize_type.const_int(code.len() as u64, false).into(),
                fact_name_pointer.into(),
                usize_type.const_int(fact_name.len() as u64, false).into(),
                value.into(),
            ],
            "panic.catalogued.fact",
        ))?;
        build(self.builder.build_unreachable())?;
        self.builder.position_at_end(continue_block);
        Ok(())
    }

    fn lower_index_bounds_panic_if(
        &mut self,
        condition: IntValue<'ctx>,
        code: &'static str,
        index: IntValue<'ctx>,
        length: IntValue<'ctx>,
    ) -> Result<(), BackendError> {
        let function = current_function(&self.builder)?;
        let panic_block = self
            .context
            .append_basic_block(function, "panic.index-bounds");
        let continue_block = self
            .context
            .append_basic_block(function, "panic.index-bounds.continue");
        build(
            self.builder
                .build_conditional_branch(condition, panic_block, continue_block),
        )?;
        self.builder.position_at_end(panic_block);
        let pointer = self.context.ptr_type(AddressSpace::default());
        let usize_type = self.context.ptr_sized_int_type(self.target_data, None);
        let i64_type = self.context.i64_type();
        let index = match index.get_type().get_bit_width().cmp(&64) {
            std::cmp::Ordering::Less => build(self.builder.build_int_s_extend(
                index,
                i64_type,
                "panic.index.i64",
            ))?,
            std::cmp::Ordering::Equal => index,
            std::cmp::Ordering::Greater => build(self.builder.build_int_truncate(
                index,
                i64_type,
                "panic.index.i64",
            ))?,
        };
        let runtime = self.runtime_function(
            "dr_v2_panic_index_out_of_bounds",
            &[
                pointer.into(),
                pointer.into(),
                usize_type.into(),
                i64_type.into(),
                usize_type.into(),
            ],
            None,
        );
        let code_pointer = self.define_data(code.as_bytes(), "panic.code");
        build(self.builder.build_call(
            runtime,
            &[
                self.current_frame.into(),
                code_pointer.into(),
                usize_type.const_int(code.len() as u64, false).into(),
                index.into(),
                length.into(),
            ],
            "panic.index-bounds",
        ))?;
        build(self.builder.build_unreachable())?;
        self.builder.position_at_end(continue_block);
        Ok(())
    }

    fn lower_padding_empty_panic_if(
        &mut self,
        condition: IntValue<'ctx>,
        pad_start: bool,
        facts: (
            PointerValue<'ctx>,
            IntValue<'ctx>,
            IntValue<'ctx>,
            IntValue<'ctx>,
        ),
        span: crate::source::Span,
    ) -> Result<(), BackendError> {
        let (value, current_length, requested_length, padding_length) = facts;
        let function = current_function(&self.builder)?;
        let panic_block = self
            .context
            .append_basic_block(function, "panic.padding-empty");
        let continue_block = self
            .context
            .append_basic_block(function, "panic.padding-empty.continue");
        build(
            self.builder
                .build_conditional_branch(condition, panic_block, continue_block),
        )?;
        self.builder.position_at_end(panic_block);
        self.set_active_panic_site(span)?;
        let pointer = self.context.ptr_type(AddressSpace::default());
        let usize_type = self.context.ptr_sized_int_type(self.target_data, None);
        let i8_type = self.context.i8_type();
        let runtime = self.runtime_function(
            "dr_v2_panic_string_padding_empty",
            &[
                pointer.into(),
                i8_type.into(),
                pointer.into(),
                usize_type.into(),
                self.context.i64_type().into(),
                usize_type.into(),
            ],
            None,
        );
        build(self.builder.build_call(
            runtime,
            &[
                self.current_frame.into(),
                i8_type.const_int(u64::from(pad_start), false).into(),
                value.into(),
                current_length.into(),
                requested_length.into(),
                padding_length.into(),
            ],
            "panic.padding-empty",
        ))?;
        build(self.builder.build_unreachable())?;
        self.builder.position_at_end(continue_block);
        Ok(())
    }

    fn lower_echo(&mut self, bytes: &[u8]) -> Result<(), BackendError> {
        if bytes.is_empty() {
            return Ok(());
        }
        let pointer = self.define_data(bytes, "echo");
        let usize_type = self.context.ptr_sized_int_type(self.target_data, None);
        let runtime = self
            .module
            .get_function("dr_v2_write_stdout")
            .unwrap_or_else(|| {
                self.module.add_function(
                    "dr_v2_write_stdout",
                    self.context.void_type().fn_type(
                        &[
                            self.context.ptr_type(AddressSpace::default()).into(),
                            self.context.ptr_type(AddressSpace::default()).into(),
                            usize_type.into(),
                        ],
                        false,
                    ),
                    Some(Linkage::External),
                )
            });
        build(self.builder.build_call(
            runtime,
            &[
                self.current_frame.into(),
                pointer.into(),
                usize_type.const_int(bytes.len() as u64, false).into(),
            ],
            "write.stdout",
        ))?;
        Ok(())
    }

    fn lower_runtime_panic_code(
        &mut self,
        code: &[u8],
        message: &[u8],
    ) -> Result<(), BackendError> {
        let code_pointer = self.define_data(code, "panic.code");
        let message_pointer = self.define_data(message, "panic.message");
        let pointer = self.context.ptr_type(AddressSpace::default());
        let usize_type = self.context.ptr_sized_int_type(self.target_data, None);
        let runtime = self.runtime_function(
            "dr_v2_panic_code",
            &[
                pointer.into(),
                pointer.into(),
                usize_type.into(),
                pointer.into(),
                usize_type.into(),
            ],
            None,
        );
        build(self.builder.build_call(
            runtime,
            &[
                self.current_frame.into(),
                code_pointer.into(),
                usize_type.const_int(code.len() as u64, false).into(),
                message_pointer.into(),
                usize_type.const_int(message.len() as u64, false).into(),
            ],
            "panic.catalogued",
        ))?;
        build(self.builder.build_unreachable())?;
        Ok(())
    }

    fn define_data(&mut self, bytes: &[u8], role: &str) -> PointerValue<'ctx> {
        let name = format!(
            "__doria_data_{}_{}_{}",
            self.function.id.0, self.next_data_id, role
        );
        self.next_data_id += 1;
        define_bytes(self.context, self.module, bytes, &name)
    }

    fn set_active_panic_site(&self, span: crate::source::Span) -> Result<(), BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let usize_type = self.context.ptr_sized_int_type(self.target_data, None);
        let frame_type = self.context.struct_type(
            &[
                pointer.into(),
                pointer.into(),
                usize_type.into(),
                pointer.into(),
                usize_type.into(),
                pointer.into(),
                usize_type.into(),
                usize_type.into(),
                usize_type.into(),
                usize_type.into(),
                usize_type.into(),
            ],
            false,
        );
        for (index, offset) in [(9, span.start), (10, span.end)] {
            let field = build(self.builder.build_struct_gep(
                frame_type,
                self.current_frame,
                index,
                "doria.frame.active",
            ))?;
            build(
                self.builder
                    .build_store(field, usize_type.const_int(offset as u64, false)),
            )?;
        }
        Ok(())
    }
}

fn integer_compare_predicate(op: mir::CompareOp, ty: IntegerType) -> IntPredicate {
    match op {
        mir::CompareOp::Equal => IntPredicate::EQ,
        mir::CompareOp::NotEqual => IntPredicate::NE,
        mir::CompareOp::Less if ty.is_signed() => IntPredicate::SLT,
        mir::CompareOp::Less => IntPredicate::ULT,
        mir::CompareOp::LessEqual if ty.is_signed() => IntPredicate::SLE,
        mir::CompareOp::LessEqual => IntPredicate::ULE,
        mir::CompareOp::Greater if ty.is_signed() => IntPredicate::SGT,
        mir::CompareOp::Greater => IntPredicate::UGT,
        mir::CompareOp::GreaterEqual if ty.is_signed() => IntPredicate::SGE,
        mir::CompareOp::GreaterEqual => IntPredicate::UGE,
    }
}

fn bool_compare_predicate(op: mir::CompareOp) -> IntPredicate {
    match op {
        mir::CompareOp::Equal => IntPredicate::EQ,
        mir::CompareOp::NotEqual => IntPredicate::NE,
        mir::CompareOp::Less => IntPredicate::ULT,
        mir::CompareOp::LessEqual => IntPredicate::ULE,
        mir::CompareOp::Greater => IntPredicate::UGT,
        mir::CompareOp::GreaterEqual => IntPredicate::UGE,
    }
}

fn float_compare_predicate(op: mir::CompareOp) -> FloatPredicate {
    match op {
        mir::CompareOp::Equal => FloatPredicate::OEQ,
        mir::CompareOp::NotEqual => FloatPredicate::UNE,
        mir::CompareOp::Less => FloatPredicate::OLT,
        mir::CompareOp::LessEqual => FloatPredicate::OLE,
        mir::CompareOp::Greater => FloatPredicate::OGT,
        mir::CompareOp::GreaterEqual => FloatPredicate::OGE,
    }
}

fn scalar_type(context: &Context, ty: mir::ScalarType) -> BasicTypeEnum<'_> {
    match ty {
        mir::ScalarType::Integer(ty) => integer_type(context, ty).into(),
        mir::ScalarType::Float(FloatType::Float32) => context.f32_type().into(),
        mir::ScalarType::Float(FloatType::Float64) => context.f64_type().into(),
        mir::ScalarType::Bool => context.i8_type().into(),
        mir::ScalarType::Enum(_) => context.i32_type().into(),
    }
}

fn collection_storage_type<'ctx>(
    context: &'ctx Context,
    target_data: &TargetData,
    ty: mir::Type,
) -> Result<BasicTypeEnum<'ctx>, BackendError> {
    Ok(match ty {
        mir::Type::Scalar(ty) => scalar_type(context, ty),
        mir::Type::Error | mir::Type::NullableError => llvm_type(context, target_data, ty),
        mir::Type::String
        | mir::Type::Mixed
        | mir::Type::Class(_)
        | mir::Type::Collection(_)
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
            context.ptr_sized_int_type(target_data, None).into()
        }
        mir::Type::NullableScalar(_)
        | mir::Type::NullableString
        | mir::Type::NullableMixed
        | mir::Type::NullableClass(_)
        | mir::Type::NullableCollection(_) => {
            return Err(malformed_mir(
                "nullable collection elements are not supported by Stage 23 Slice 3",
            ))
        }
        mir::Type::PayloadEnum(_) | mir::Type::NullablePayloadEnum(_) => {
            return Err(malformed_mir(
                "payload enum collection values require aggregate storage",
            ))
        }
        mir::Type::Function(_)
        | mir::Type::NullableFunction(_)
        | mir::Type::ClosureEnvironment(_) => {
            return Err(malformed_mir(
                "function carriers require aggregate collection storage",
            ))
        }
    })
}

fn collection_storage_to_word<'ctx>(
    builder: &Builder<'ctx>,
    context: &'ctx Context,
    value: IntValue<'ctx>,
) -> Result<IntValue<'ctx>, BackendError> {
    Ok(if value.get_type().get_bit_width() == 64 {
        value
    } else {
        build(builder.build_int_z_extend(value, context.i64_type(), "collection.value.word"))?
    })
}

fn nullable_type<'ctx>(
    context: &'ctx Context,
    target_data: &TargetData,
    payload: BasicTypeEnum<'ctx>,
) -> inkwell::types::StructType<'ctx> {
    let word = context.ptr_sized_int_type(target_data, None);
    context.struct_type(&[word.into(), payload], false)
}

fn error_carrier_type(context: &Context) -> StructType<'_> {
    let pointer = context.ptr_type(AddressSpace::default());
    context.struct_type(&[pointer.into(), pointer.into()], false)
}

fn closure_carrier_type(context: &Context) -> StructType<'_> {
    let pointer = context.ptr_type(AddressSpace::default());
    context.struct_type(&[pointer.into(), pointer.into()], false)
}

fn closure_descriptor_type(context: &Context) -> StructType<'_> {
    let pointer = context.ptr_type(AddressSpace::default());
    context.struct_type(&[pointer.into(), pointer.into()], false)
}

fn error_descriptor_type<'ctx>(
    context: &'ctx Context,
    target_data: &TargetData,
) -> StructType<'ctx> {
    let pointer = context.ptr_type(AddressSpace::default());
    let word = context.ptr_sized_int_type(target_data, None);
    context.struct_type(
        &[
            pointer.into(),
            word.into(),
            word.into(),
            pointer.into(),
            word.into(),
            word.into(),
            word.into(),
            word.into(),
        ],
        false,
    )
}

fn collection_header_type<'ctx>(
    context: &'ctx Context,
    target_data: &TargetData,
) -> StructType<'ctx> {
    let word = context.ptr_sized_int_type(target_data, None);
    let pointer = context.ptr_type(AddressSpace::default());
    let byte = context.i8_type();
    // Mirrors the complete `#[repr(C)] DrCollectionV1`. Most fields stay
    // runtime-only, but clear uses this type as fixed entry-block scratch while
    // generated drop glue runs, so its size and alignment are part of the
    // private compiler/runtime ABI as well as the offsets codegen reads.
    context.struct_type(
        &[
            word.into(),    // length
            word.into(),    // capacity
            pointer.into(), // keys
            pointer.into(), // values
            byte.into(),    // keyed
            byte.into(),    // fixed
            byte.into(),    // value_width
            byte.into(),    // kind
            byte.into(),    // comparator
            byte.into(),    // finalized
            byte.into(),    // value_nullable
            word.into(),    // head
            pointer.into(), // index
            word.into(),    // index_slots
            byte.into(),    // index_kind
            byte.into(),    // index_keyed
            word.into(),    // value_size
            word.into(),    // value_stride
            word.into(),    // value_alignment
            byte.into(),    // aggregate
        ],
        false,
    )
}

fn llvm_type<'ctx>(
    context: &'ctx Context,
    target_data: &TargetData,
    ty: mir::Type,
) -> BasicTypeEnum<'ctx> {
    match ty {
        mir::Type::Scalar(ty) => scalar_type(context, ty),
        mir::Type::NullableScalar(ty) => {
            nullable_type(context, target_data, scalar_type(context, ty)).into()
        }
        mir::Type::NullableString => nullable_type(
            context,
            target_data,
            context.ptr_type(AddressSpace::default()).into(),
        )
        .into(),
        mir::Type::Error | mir::Type::NullableError => error_carrier_type(context).into(),
        mir::Type::Function(_) | mir::Type::NullableFunction(_) => {
            closure_carrier_type(context).into()
        }
        mir::Type::ClosureEnvironment(_) => context.ptr_type(AddressSpace::default()).into(),
        mir::Type::String
        | mir::Type::Class(_)
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
            context.ptr_type(AddressSpace::default()).into()
        }
        mir::Type::PayloadEnum(payload) => context.i8_type().array_type(payload.size).into(),
        mir::Type::NullablePayloadEnum(payload) => {
            context.i8_type().array_type(payload.nullable_size).into()
        }
    }
}

fn scalar_constant(context: &Context, value: mir::ScalarValue) -> BasicValueEnum<'_> {
    match value {
        mir::ScalarValue::Integer(value) => integer_constant(context, value).into(),
        mir::ScalarValue::Float(value) => float_constant(context, value).into(),
        mir::ScalarValue::Bool(value) => {
            context.i8_type().const_int(u64::from(value), false).into()
        }
        mir::ScalarValue::Enum(value) => context
            .i32_type()
            .const_int(value.case_id.index as u64, false)
            .into(),
    }
}

fn integer_type(context: &Context, ty: IntegerType) -> IntType<'_> {
    context
        .custom_width_int_type(
            NonZeroU32::new(ty.bit_width()).expect("Doria integer widths are nonzero"),
        )
        .expect("Doria integer width is supported by LLVM")
}

fn integer_constant<'ctx>(context: &'ctx Context, value: IntegerValue) -> IntValue<'ctx> {
    integer_type(context, value.ty).const_int(value.bits, false)
}

fn float_constant<'ctx>(context: &'ctx Context, value: FloatValue) -> LlvmFloatValue<'ctx> {
    match value.ty {
        FloatType::Float32 => context
            .f32_type()
            .const_float(f64::from(f32::from_bits(value.bits as u32))),
        FloatType::Float64 => context.f64_type().const_float(f64::from_bits(value.bits)),
    }
}

fn define_bytes<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    bytes: &[u8],
    name: &str,
) -> PointerValue<'ctx> {
    let value = context.const_string(bytes, false);
    let global = module.add_global(value.get_type(), None, name);
    global.set_initializer(&value);
    global.set_constant(true);
    global.set_linkage(Linkage::Private);
    global.set_unnamed_address(UnnamedAddress::Global);
    global.as_pointer_value()
}

fn current_function<'ctx>(builder: &Builder<'ctx>) -> Result<FunctionValue<'ctx>, BackendError> {
    builder
        .get_insert_block()
        .and_then(BasicBlock::get_parent)
        .ok_or_else(|| backend_failure("LLVM builder is not positioned in a function"))
}

fn local_in(function: &mir::Function, id: mir::LocalId) -> Result<&mir::Local, BackendError> {
    function
        .locals
        .get(id.0)
        .filter(|local| local.id == id)
        .ok_or_else(|| malformed_mir(format!("LocalId local{} does not exist", id.0)))
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
        .filter(|function| function.id == id)
        .ok_or_else(|| malformed_mir(format!("function type#{} does not exist", id.0)))
}

fn closure_descriptor_in(
    program: &mir::Program,
    id: mir::ClosureDescriptorId,
) -> Result<&mir::ClosureDescriptor, BackendError> {
    program
        .closure_descriptors
        .get(id.0)
        .filter(|descriptor| descriptor.id == id)
        .ok_or_else(|| malformed_mir(format!("closure descriptor#{} does not exist", id.0)))
}

fn closure_environment_layout_in(
    program: &mir::Program,
    id: mir::ClosureEnvironmentLayoutId,
) -> Result<&mir::ClosureEnvironmentLayout, BackendError> {
    program
        .closure_environment_layouts
        .get(id.0)
        .filter(|layout| layout.id == id)
        .ok_or_else(|| malformed_mir(format!("closure environment#{} does not exist", id.0)))
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

fn local_slot<'ctx>(
    slots: &[Option<PointerValue<'ctx>>],
    id: mir::LocalId,
) -> Result<PointerValue<'ctx>, BackendError> {
    slots
        .get(id.0)
        .copied()
        .flatten()
        .ok_or_else(|| malformed_mir(format!("LocalId local{} is not a scalar local", id.0)))
}

fn block_for<'ctx>(
    blocks: &[BasicBlock<'ctx>],
    id: mir::BlockId,
) -> Result<BasicBlock<'ctx>, BackendError> {
    blocks
        .get(id.0)
        .copied()
        .ok_or_else(|| malformed_mir(format!("BlockId block{} does not exist", id.0)))
}

fn build<T, E: std::fmt::Display>(result: Result<T, E>) -> Result<T, BackendError> {
    result.map_err(|error| backend_failure(format!("LLVM builder failure: {error}")))
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
        | mir::StringExpression::Property { .. }
        | mir::StringExpression::Static(_)
        | mir::StringExpression::MixedPayload(_)
        | mir::StringExpression::NullableLocalAssumeNonNull(_)
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
        | mir::StringExpression::Property { .. }
        | mir::StringExpression::Static(_)
        | mir::StringExpression::MixedPayload(_)
        | mir::StringExpression::NullableLocalAssumeNonNull(_)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_header_layout_matches_the_runtime_abi() {
        initialize_native_target().expect("native LLVM target should initialize");
        let triple = TargetMachine::get_default_triple();
        let target = Target::from_triple(&triple).expect("host LLVM target should exist");
        let machine = target
            .create_target_machine(
                &triple,
                "generic",
                "",
                OptimizationLevel::None,
                RelocMode::PIC,
                CodeModel::Default,
            )
            .expect("host LLVM target machine should exist");
        let target_data = machine.get_target_data();
        let context = Context::create();
        let header = collection_header_type(&context, &target_data);
        for (field, expected) in [
            (
                crate::native_abi::COLLECTION_LENGTH_FIELD,
                doria_rt::DR_COLLECTION_LENGTH_OFFSET,
            ),
            (
                crate::native_abi::COLLECTION_CAPACITY_FIELD,
                doria_rt::DR_COLLECTION_CAPACITY_OFFSET,
            ),
            (
                crate::native_abi::COLLECTION_KEYS_FIELD,
                doria_rt::DR_COLLECTION_KEYS_OFFSET,
            ),
            (
                crate::native_abi::COLLECTION_VALUES_FIELD,
                doria_rt::DR_COLLECTION_VALUES_OFFSET,
            ),
            (
                crate::native_abi::COLLECTION_KEYED_FIELD,
                doria_rt::DR_COLLECTION_KEYED_OFFSET,
            ),
            (
                crate::native_abi::COLLECTION_FIXED_FIELD,
                doria_rt::DR_COLLECTION_FIXED_OFFSET,
            ),
            (
                crate::native_abi::COLLECTION_VALUE_WIDTH_FIELD,
                doria_rt::DR_COLLECTION_VALUE_WIDTH_OFFSET,
            ),
            (
                crate::native_abi::COLLECTION_KIND_FIELD,
                doria_rt::DR_COLLECTION_KIND_OFFSET,
            ),
            // `head` follows four bytes of tail flags, so it is the field that
            // catches a padding disagreement between LLVM and repr(C).
            (
                crate::native_abi::COLLECTION_HEAD_FIELD,
                doria_rt::DR_COLLECTION_HEAD_OFFSET,
            ),
            (
                crate::native_abi::COLLECTION_INDEX_FIELD,
                doria_rt::DR_COLLECTION_INDEX_OFFSET,
            ),
            (
                crate::native_abi::COLLECTION_VALUE_SIZE_FIELD,
                doria_rt::DR_COLLECTION_VALUE_SIZE_OFFSET,
            ),
            (
                crate::native_abi::COLLECTION_VALUE_STRIDE_FIELD,
                doria_rt::DR_COLLECTION_VALUE_STRIDE_OFFSET,
            ),
            (
                crate::native_abi::COLLECTION_VALUE_ALIGNMENT_FIELD,
                doria_rt::DR_COLLECTION_VALUE_ALIGNMENT_OFFSET,
            ),
            (
                crate::native_abi::COLLECTION_AGGREGATE_FIELD,
                doria_rt::DR_COLLECTION_AGGREGATE_OFFSET,
            ),
        ] {
            assert_eq!(
                target_data.offset_of_element(&header, field),
                Some(expected as u64)
            );
        }
        assert_eq!(
            target_data.get_store_size(&header),
            doria_rt::DR_COLLECTION_SIZE as u64
        );
        assert_eq!(
            target_data.get_abi_alignment(&header),
            doria_rt::DR_COLLECTION_ALIGN as u32
        );
    }
}

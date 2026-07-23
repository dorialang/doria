use std::collections::{HashMap, HashSet};
use std::num::NonZeroU32;

use inkwell::attributes::AttributeLoc;
use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::passes::PassBuilderOptions;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetData, TargetMachine,
};
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum, IntType};
use inkwell::values::{
    BasicMetadataValueEnum, BasicValueEnum, FloatValue as LlvmFloatValue, FunctionValue,
    GlobalValue, IntValue, PointerValue, StructValue, UnnamedAddress,
};
use inkwell::{AddressSpace, FloatPredicate, IntPredicate, OptimizationLevel};

use crate::backend::BackendError;
use crate::format_string::{FormatConversion, FormatPiece};
use crate::mir;
use crate::mir_validation;
use crate::native_abi::{
    function_symbol, APPEND_FILE, APPEND_FILE_BYTES, BYTES_EQUAL, BYTES_FREE,
    BYTES_FROM_COLLECTION, BYTES_GET, BYTES_LENGTH, BYTES_SET, BYTES_TO_COLLECTION, CLASS_ALLOCATE,
    CLASS_FREE, COLLECTION_COMPARE_FLOAT32, COLLECTION_COMPARE_FLOAT64, COLLECTION_COMPARE_STRING,
    COLLECTION_COMPARE_WORD, COLLECTION_CONTAINS, COLLECTION_FREE, COLLECTION_INSERT_AT,
    COLLECTION_KEYED_GET, COLLECTION_KEYED_HAS, COLLECTION_KEYED_SET, COLLECTION_KEY_AT,
    COLLECTION_LENGTH, COLLECTION_NEW, COLLECTION_NULLABLE_ACCESS, COLLECTION_PUSH,
    COLLECTION_PUSH_UNIQUE, COLLECTION_REMOVE_AT, COLLECTION_REMOVE_VALUE, COLLECTION_SET_ALGEBRA,
    COLLECTION_SET_AT, COLLECTION_VALUE_AT, FORMAT_F32, FORMAT_F64, FORMAT_I64, FORMAT_STRING,
    FORMAT_U64, NULLABLE_STRING_EQUAL, READ_FILE, READ_FILE_BYTES, READ_STDIN_BYTES,
    READ_STDIN_LINE, STRING_COMPARE, STRING_CONCAT, STRING_DATA, STRING_FROM_BOOL, STRING_FROM_F32,
    STRING_FROM_F64, STRING_FROM_I64, STRING_FROM_U64, STRING_FROM_UTF8, STRING_LENGTH,
    STRING_RELEASE, STRING_RETAIN, STRING_WRITE_STDERR, STRING_WRITE_STDOUT, WRITE_FILE,
    WRITE_FILE_BYTES, WRITE_STDERR_BYTES, WRITE_STDOUT_BYTES,
};
use crate::numeric::{FloatType, FloatValue, IntegerPanic, IntegerType, IntegerValue};

pub fn lower_mir_to_object(program: &mir::Program) -> Result<Vec<u8>, BackendError> {
    mir_validation::validate_program(program)?;
    Target::initialize_native(&InitializationConfig::default()).map_err(|error| {
        backend_failure(format!("failed to initialize host LLVM target: {error}"))
    })?;

    let triple = TargetMachine::get_default_triple();
    let target = Target::from_triple(&triple)
        .map_err(|error| backend_failure(format!("failed to select host LLVM target: {error}")))?;
    let target_machine = target
        .create_target_machine(
            &triple,
            "generic",
            "",
            OptimizationLevel::Aggressive,
            RelocMode::PIC,
            CodeModel::Default,
        )
        .ok_or_else(|| backend_failure("failed to create host LLVM target machine"))?;

    let context = Context::create();
    let module = context.create_module("doria_stage_15");
    module.set_triple(&triple);
    let target_data = target_machine.get_target_data();
    module.set_data_layout(&target_data.get_data_layout());

    let functions = declare_functions(&context, &module, &target_data, program)?;
    let statics = declare_statics(&context, &module, &target_data, program)?;
    for function in &program.functions {
        define_function(
            &context,
            &module,
            &target_data,
            program,
            function,
            &functions,
            &statics,
        )?;
    }
    define_process_main(&context, &module, program, &functions)?;

    module
        .verify()
        .map_err(|error| backend_failure(format!("LLVM verification failed: {error}")))?;
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
        };
        global.set_constant(!property.writable);
        global.set_linkage(Linkage::Internal);
        globals.push(global);
    }
    Ok(globals)
}

fn function_type<'ctx>(
    context: &'ctx Context,
    target_data: &TargetData,
    function: &mir::Function,
) -> Result<inkwell::types::FunctionType<'ctx>, BackendError> {
    let mut parameters = vec![context.ptr_type(AddressSpace::default()).into()];
    for parameter in &function.params {
        let local = local_in(function, *parameter)?;
        parameters.push(llvm_type(context, target_data, local.ty).into());
    }
    Ok(match function.return_type {
        mir::ReturnType::Void => context.void_type().fn_type(&parameters, false),
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
    if let mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Integer(ty))) =
        function.return_type
    {
        apply_integer_extension_attribute(context, llvm_function, AttributeLoc::Return, ty);
    }
    for (index, parameter) in function.params.iter().enumerate() {
        let local = local_in(function, *parameter)?;
        if let mir::Type::Scalar(mir::ScalarType::Integer(ty)) = local.ty {
            apply_integer_extension_attribute(
                context,
                llvm_function,
                AttributeLoc::Param((index + 1) as u32),
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
    functions: &[FunctionValue<'ctx>],
    statics: &[GlobalValue<'ctx>],
) -> Result<(), BackendError> {
    let llvm_function = *functions
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
        build(builder.build_store(slot, ty.const_zero()))?;
        local_slots.push(Some(slot));
    }
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
    for (index, parameter) in function.params.iter().enumerate() {
        let value = llvm_function
            .get_nth_param((index + 1) as u32)
            .ok_or_else(|| malformed_mir("LLVM function is missing a declared parameter"))?;
        build(builder.build_store(local_slot(&local_slots, *parameter)?, value))?;
    }

    let pointer_type = context.ptr_type(AddressSpace::default());
    let usize_type = context.ptr_sized_int_type(target_data, None);
    let frame_type = context.struct_type(
        &[pointer_type.into(), pointer_type.into(), usize_type.into()],
        false,
    );
    let frame = build(builder.build_alloca(frame_type, "doria.frame"))?;
    let parent = llvm_function
        .get_first_param()
        .ok_or_else(|| malformed_mir("LLVM function is missing its parent frame"))?
        .into_pointer_value();
    let function_name = define_bytes(
        context,
        module,
        function.name.as_bytes(),
        &format!("__doria_function_name_{}", function.id.0),
    );
    let parent_slot = build(builder.build_struct_gep(frame_type, frame, 0, "frame.parent"))?;
    let name_slot = build(builder.build_struct_gep(frame_type, frame, 1, "frame.name"))?;
    let length_slot = build(builder.build_struct_gep(frame_type, frame, 2, "frame.name_length"))?;
    build(builder.build_store(parent_slot, parent))?;
    build(builder.build_store(name_slot, function_name))?;
    build(builder.build_store(
        length_slot,
        usize_type.const_int(function.name.len() as u64, false),
    ))?;
    let mut lowerer = FunctionLowerer {
        context,
        module,
        target_data,
        builder,
        program,
        function,
        functions,
        statics,
        local_slots,
        blocks,
        current_frame: frame,
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

fn define_process_main<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    program: &mir::Program,
    functions: &[FunctionValue<'ctx>],
) -> Result<(), BackendError> {
    let entry = function_in(program, program.entry)?;
    let entry_function = *functions
        .get(program.entry.0)
        .ok_or_else(|| malformed_mir("entry function was not declared"))?;
    let main = module.add_function(
        "main",
        context.i32_type().fn_type(&[], false),
        Some(Linkage::External),
    );
    let builder = context.create_builder();
    let block = context.append_basic_block(main, "entry");
    builder.position_at_end(block);
    let pointer_type = context.ptr_type(AddressSpace::default());
    let runtime_name = match entry.return_type {
        mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64))) => {
            "dr_v1_main_int"
        }
        mir::ReturnType::Void => "dr_v1_main_void",
        mir::ReturnType::Value(other) => {
            return Err(malformed_mir(format!(
                "entry function has unsupported process return type {other}"
            )))
        }
    };
    let runtime = module.get_function(runtime_name).unwrap_or_else(|| {
        module.add_function(
            runtime_name,
            context.i32_type().fn_type(&[pointer_type.into()], false),
            Some(Linkage::External),
        )
    });
    let entry_pointer = entry_function.as_global_value().as_pointer_value();
    let call = build(builder.build_call(runtime, &[entry_pointer.into()], "process.status"))?;
    let status = call
        .try_as_basic_value()
        .basic()
        .ok_or_else(|| backend_failure("doria-rt process entry returned no status"))?;
    build(builder.build_return(Some(&status)))?;
    Ok(())
}

struct FunctionLowerer<'ctx, 'program> {
    context: &'ctx Context,
    module: &'program Module<'ctx>,
    target_data: &'program TargetData,
    builder: Builder<'ctx>,
    program: &'program mir::Program,
    function: &'program mir::Function,
    functions: &'program [FunctionValue<'ctx>],
    statics: &'program [GlobalValue<'ctx>],
    local_slots: Vec<Option<PointerValue<'ctx>>>,
    blocks: Vec<BasicBlock<'ctx>>,
    current_frame: PointerValue<'ctx>,
    next_data_id: usize,
    defer_class_temporary_drops: bool,
    deferred_class_temporary_slots: Vec<PointerValue<'ctx>>,
    deferred_class_temporary_slot_cursor: usize,
    deferred_class_temporary_drops: Vec<(PointerValue<'ctx>, crate::class_layout::ClassId)>,
}

impl<'ctx> FunctionLowerer<'ctx, '_> {
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

    fn lower_statement(&mut self, statement: &mir::Statement) -> Result<(), BackendError> {
        debug_assert!(self.deferred_class_temporary_drops.is_empty());
        self.defer_class_temporary_drops = true;
        match statement {
            mir::Statement::AssignLocal { target, value } => {
                let local = local_in(self.function, *target)?;
                let slot = local_slot(&self.local_slots, *target)?;
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
                    mir::Type::Collection(_) if local.owned => Some((
                        build(self.builder.build_load(
                            self.context.ptr_type(AddressSpace::default()),
                            slot,
                            "collection.old",
                        ))?
                        .into_pointer_value(),
                        None,
                    )),
                    _ => None,
                };
                let value = self.lower_rvalue(value)?;
                build(self.builder.build_store(slot, value))?;
                if let Some((old, class)) = old {
                    if let mir::Type::Collection(collection) = local.ty {
                        self.drop_collection_value(old, collection)?;
                    } else if let Some(class) = class {
                        self.drop_class_value_checked(old, class)?;
                    } else {
                        self.release_string(old)?;
                    }
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
            mir::Statement::CallVoid { function, args }
            | mir::Statement::CallBorrowed { function, args } => {
                let _ = self.lower_call(*function, args, false)?;
            }
            mir::Statement::CallNullSafe {
                object,
                function,
                args,
            } => self.lower_null_safe_statement_call(object, *function, args)?,
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
            } => {
                let property_ty = property_definition(self.program, *property)?.ty;
                let value = self.lower_rvalue(value)?;
                let address = self.lower_property_address(*object, *property)?;
                let old = match property_ty {
                    mir::Type::String
                    | mir::Type::Class(_)
                    | mir::Type::NullableClass(_)
                    | mir::Type::Collection(_) => Some(
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
                    mir::Type::Scalar(_) | mir::Type::NullableScalar(_) => None,
                };
                build(self.builder.build_store(address, value))?;
                match (property_ty, old) {
                    (mir::Type::String | mir::Type::NullableString, Some(value)) => {
                        self.release_string(value)?;
                    }
                    (mir::Type::Class(class) | mir::Type::NullableClass(class), Some(value)) => {
                        self.drop_class_value_checked(value, class)?;
                    }
                    (mir::Type::Collection(collection), Some(value)) => {
                        self.drop_collection_value(value, collection)?;
                    }
                    _ => {}
                }
            }
            mir::Statement::AssignStatic { target, value } => {
                let property = static_definition(self.program, *target)?;
                let value = self.lower_rvalue(value)?;
                let address = self.static_address(*target)?;
                let old = match property.ty {
                    mir::Type::String => Some(
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
                    self.release_string(old)?;
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
            }
            | mir::Statement::AssignCollectionIndex {
                collection,
                index: key,
                value,
            } => self.lower_collection_set(*collection, key, value)?,
            mir::Statement::DropCollection { local, collection } => {
                let pointer = self.context.ptr_type(AddressSpace::default());
                let slot = local_slot(&self.local_slots, *local)?;
                let value = build(self.builder.build_load(pointer, slot, "collection.drop"))?
                    .into_pointer_value();
                build(self.builder.build_store(slot, pointer.const_null()))?;
                self.drop_collection_value(value, *collection)?;
            }
        }
        self.defer_class_temporary_drops = false;
        self.flush_deferred_class_temporary_drops()
    }

    fn lower_terminator(&mut self, terminator: &mir::Terminator) -> Result<(), BackendError> {
        match terminator {
            mir::Terminator::Return(expression) => {
                debug_assert!(self.deferred_class_temporary_drops.is_empty());
                self.defer_class_temporary_drops = true;
                let value = self.lower_rvalue(expression)?;
                self.defer_class_temporary_drops = false;
                self.flush_deferred_class_temporary_drops()?;
                self.cleanup_class_locals()?;
                self.cleanup_string_locals()?;
                build(self.builder.build_return(Some(&value)))?;
            }
            mir::Terminator::ReturnVoid => {
                self.cleanup_class_locals()?;
                self.cleanup_string_locals()?;
                build(self.builder.build_return(None))?;
            }
            mir::Terminator::Panic(message) => {
                debug_assert!(self.deferred_class_temporary_drops.is_empty());
                self.defer_class_temporary_drops = true;
                let string = self.lower_string_expression(message)?;
                self.defer_class_temporary_drops = false;
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
                        STRING_LENGTH,
                        &[pointer.into()],
                        Some(usize_type.into()),
                        &[string.into()],
                    )?
                    .ok_or_else(|| backend_failure("string length produced no result"))?;
                let panic = self.runtime_function(
                    "dr_v1_panic",
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
        }
        Ok(())
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
        }
    }

    fn lower_rvalue(
        &mut self,
        expression: &mir::Rvalue,
    ) -> Result<BasicValueEnum<'ctx>, BackendError> {
        match expression {
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
        }
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
        let kind = match ty {
            mir::Type::String => COLLECTION_COMPARE_STRING,
            mir::Type::Scalar(mir::ScalarType::Float(FloatType::Float32)) => {
                COLLECTION_COMPARE_FLOAT32
            }
            mir::Type::Scalar(mir::ScalarType::Float(FloatType::Float64)) => {
                COLLECTION_COMPARE_FLOAT64
            }
            mir::Type::Scalar(_) | mir::Type::Class(_) | mir::Type::Collection(_) => {
                COLLECTION_COMPARE_WORD
            }
            mir::Type::NullableScalar(_)
            | mir::Type::NullableString
            | mir::Type::NullableClass(_) => {
                return Err(malformed_mir(
                    "nullable collection elements are not supported by Stage 23 Slice 1",
                ))
            }
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
            mir::Type::String | mir::Type::Class(_) | mir::Type::Collection(_) => {
                Ok(build(self.builder.build_ptr_to_int(
                    value.into_pointer_value(),
                    i64_type,
                    "collection.pointer.word",
                ))?)
            }
            mir::Type::NullableScalar(_)
            | mir::Type::NullableString
            | mir::Type::NullableClass(_) => Err(malformed_mir(
                "nullable collection elements are not supported by Stage 23 Slice 1",
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
            mir::Type::String | mir::Type::Class(_) | mir::Type::Collection(_) => {
                build(self.builder.build_int_to_ptr(
                    word,
                    self.context.ptr_type(AddressSpace::default()),
                    "collection.pointer.value",
                ))?
                .into()
            }
            mir::Type::NullableScalar(_)
            | mir::Type::NullableString
            | mir::Type::NullableClass(_) => {
                return Err(malformed_mir(
                    "nullable collection elements are not supported by Stage 23 Slice 1",
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

    fn lower_collection_expression(
        &mut self,
        expression: &mir::CollectionExpression,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let usize_type = self.context.ptr_sized_int_type(self.target_data, None);
        match expression {
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
                let fixed = definition.kind == mir::CollectionKind::TypedArray;
                let result = self
                    .call_runtime(
                        COLLECTION_NEW,
                        &[
                            usize_type.into(),
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
                        ],
                    )?
                    .ok_or_else(|| backend_failure("collection allocation produced no result"))?
                    .into_pointer_value();
                for (index, entry) in entries.iter().enumerate() {
                    if let (Some(key_type), Some(key)) = (definition.key, &entry.key) {
                        let key = self.lower_rvalue(key)?;
                        let value = self.lower_rvalue(&entry.value)?;
                        self.lower_dictionary_set_value(
                            result,
                            key,
                            key_type,
                            value,
                            definition.value,
                        )?;
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
                    } else if definition.kind == mir::CollectionKind::Set {
                        let value_word = self.value_to_collection_word(value, definition.value)?;
                        let inserted = self
                            .call_runtime(
                                COLLECTION_PUSH_UNIQUE,
                                &[
                                    pointer.into(),
                                    self.context.i64_type().into(),
                                    self.context.i8_type().into(),
                                ],
                                Some(self.context.i8_type().into()),
                                &[
                                    result.into(),
                                    value_word.into(),
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
                Ok(result)
            }
            mir::CollectionExpression::Index {
                source,
                index,
                transfer,
                ..
            } => Ok(self
                .lower_collection_index(*source, index, *transfer)?
                .into_pointer_value()),
            mir::CollectionExpression::Property {
                object, property, ..
            } => Ok(build(self.builder.build_load(
                pointer,
                self.lower_property_address(*object, *property)?,
                "collection.property",
            ))?
            .into_pointer_value()),
            mir::CollectionExpression::SetFrom {
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
            mir::CollectionExpression::ReadFileBytes { path, .. } => {
                let path = self.lower_string_expression(path)?;
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

    fn lower_collection_index(
        &mut self,
        collection: mir::LocalId,
        index: &mir::Rvalue,
        remove: bool,
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
        let word = if definition.key.is_some() {
            if remove {
                return Err(malformed_mir(
                    "dictionary indexed removal must use Dictionary::remove",
                ));
            }
            let index_word = self.value_to_collection_word(index_value, index_type)?;
            let found = build(
                self.builder
                    .build_alloca(self.context.i8_type(), "dictionary.found"),
            )?;
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
            self.lower_panic_if(missing, b"dictionary key not found")?;
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
        } else {
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
        if definition.value != expected {
            return Err(malformed_mir("nullable collection access type mismatch"));
        }
        let key_type = match access {
            mir::NullableCollectionAccess::Get | mir::NullableCollectionAccess::Remove => {
                definition
                    .key
                    .ok_or_else(|| malformed_mir("dictionary access has no key type"))?
            }
            mir::NullableCollectionAccess::First
            | mir::NullableCollectionAccess::Last
            | mir::NullableCollectionAccess::Pop => {
                mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64))
            }
        };
        let collection = self.collection_pointer(collection)?;
        let key_value = self.lower_rvalue(key)?;
        let key_word = self.value_to_collection_word(key_value, key_type)?;
        let found = build(
            self.builder
                .build_alloca(self.context.i8_type(), "dictionary.get.found"),
        )?;
        let removed_key = build(
            self.builder
                .build_alloca(self.context.i64_type(), "dictionary.removed.key"),
        )?;
        let access_value = self.context.i8_type().const_int(
            match access {
                mir::NullableCollectionAccess::Get => 0,
                mir::NullableCollectionAccess::Remove => 1,
                mir::NullableCollectionAccess::First => 2,
                mir::NullableCollectionAccess::Last => 3,
                mir::NullableCollectionAccess::Pop => 4,
            },
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
            let removed_slot = build(
                self.builder
                    .build_alloca(self.context.i64_type(), "set.removed.value"),
            )?;
            let removed = self
                .call_runtime(
                    COLLECTION_REMOVE_VALUE,
                    &[
                        pointer.into(),
                        self.context.i64_type().into(),
                        self.context.i8_type().into(),
                        pointer.into(),
                    ],
                    Some(self.context.i8_type().into()),
                    &[
                        collection_value.into(),
                        word.into(),
                        self.collection_compare_kind(definition.value)?.into(),
                        removed_slot.into(),
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
        } else if definition.kind == mir::CollectionKind::Set {
            let inserted = self
                .call_runtime(
                    COLLECTION_PUSH_UNIQUE,
                    &[
                        pointer.into(),
                        self.context.i64_type().into(),
                        self.context.i8_type().into(),
                    ],
                    Some(self.context.i8_type().into()),
                    &[
                        collection_value.into(),
                        word.into(),
                        self.collection_compare_kind(definition.value)?.into(),
                    ],
                )?
                .ok_or_else(|| backend_failure("set insertion produced no result"))?
                .into_int_value();
            self.drop_value_unless(inserted, value, definition.value)?;
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
        let value_word = self.value_to_collection_word(value, definition.value)?;
        if let Some(key_type) = definition.key {
            self.lower_dictionary_set_value(
                collection_value,
                index,
                key_type,
                value,
                definition.value,
            )?;
        } else {
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
        let replaced_slot = build(
            self.builder
                .build_alloca(self.context.i8_type(), "dictionary.replaced"),
        )?;
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
                ],
                Some(pointer.into()),
                &[
                    usize_type.const_zero().into(),
                    self.context.i8_type().const_zero().into(),
                    self.context.i8_type().const_zero().into(),
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
        let index_slot = build(self.builder.build_alloca(usize_type, "set.from.index"))?;
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
                ],
                Some(self.context.i8_type().into()),
                &[
                    result.into(),
                    word.into(),
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

    fn drop_value_if(
        &mut self,
        condition: IntValue<'ctx>,
        value: BasicValueEnum<'ctx>,
        ty: mir::Type,
    ) -> Result<(), BackendError> {
        if !matches!(
            ty,
            mir::Type::String | mir::Type::Class(_) | mir::Type::Collection(_)
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
            mir::Type::String => self.release_string(value.into_pointer_value()),
            mir::Type::Class(class) => {
                self.drop_class_value_checked(value.into_pointer_value(), class)
            }
            mir::Type::Collection(collection) => {
                self.drop_collection_value(value.into_pointer_value(), collection)
            }
            mir::Type::Scalar(_) => Ok(()),
            mir::Type::NullableScalar(_)
            | mir::Type::NullableString
            | mir::Type::NullableClass(_) => Err(malformed_mir(
                "nullable collection elements are not supported by Stage 23 Slice 1",
            )),
        }
    }

    fn drop_collection_value(
        &mut self,
        collection: PointerValue<'ctx>,
        collection_type: mir::CollectionTypeId,
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
            let _ = self.call_runtime(BYTES_FREE, &[pointer.into()], None, &[collection.into()])?;
            build(self.builder.build_unconditional_branch(done))?;
            self.builder.position_at_end(done);
            return Ok(());
        }
        let length = self
            .call_runtime(
                COLLECTION_LENGTH,
                &[pointer.into()],
                Some(usize_type.into()),
                &[collection.into()],
            )?
            .ok_or_else(|| backend_failure("collection length produced no result"))?
            .into_int_value();
        let index_slot = build(
            self.builder
                .build_alloca(usize_type, "collection.drop.index"),
        )?;
        build(
            self.builder
                .build_store(index_slot, usize_type.const_zero()),
        )?;
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
            IntPredicate::ULT,
            index,
            length,
            "collection.drop.more",
        ))?;
        build(self.builder.build_conditional_branch(more, body, free))?;
        self.builder.position_at_end(body);
        if let Some(key_type) = definition.key {
            let key_word = self
                .call_runtime(
                    COLLECTION_KEY_AT,
                    &[pointer.into(), pointer.into(), usize_type.into()],
                    Some(self.context.i64_type().into()),
                    &[self.current_frame.into(), collection.into(), index.into()],
                )?
                .ok_or_else(|| backend_failure("collection key read produced no result"))?
                .into_int_value();
            let key = self.collection_word_to_value(key_word, key_type)?;
            self.drop_stored_value(key, key_type)?;
        }
        let value_word = self
            .call_runtime(
                COLLECTION_VALUE_AT,
                &[pointer.into(), pointer.into(), usize_type.into()],
                Some(self.context.i64_type().into()),
                &[self.current_frame.into(), collection.into(), index.into()],
            )?
            .ok_or_else(|| backend_failure("collection value read produced no result"))?
            .into_int_value();
        let value = self.collection_word_to_value(value_word, definition.value)?;
        self.drop_stored_value(value, definition.value)?;
        let next = build(self.builder.build_int_add(
            index,
            usize_type.const_int(1, false),
            "collection.drop.next",
        ))?;
        build(self.builder.build_store(index_slot, next))?;
        build(self.builder.build_unconditional_branch(header))?;
        self.builder.position_at_end(free);
        let _ = self.call_runtime(
            COLLECTION_FREE,
            &[pointer.into()],
            None,
            &[collection.into()],
        )?;
        build(self.builder.build_unconditional_branch(done))?;
        self.builder.position_at_end(done);
        Ok(())
    }

    fn lower_class_expression(
        &mut self,
        expression: &mir::ClassExpression,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let usize_type = self.context.ptr_sized_int_type(self.target_data, None);
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
                // Explicit property initializers precede promoted properties in
                // the canonical construction order, so evaluate them before any
                // constructor-argument side effects. Arguments remain exactly-once.
                let mut lowered_properties = Vec::with_capacity(properties.len());
                for property in properties {
                    lowered_properties.push(match &property.source {
                        mir::PropertyValueSource::Expression(value) => {
                            Some(self.lower_rvalue(value)?)
                        }
                        mir::PropertyValueSource::ConstructorArgument(_)
                        | mir::PropertyValueSource::ConstructorBody => None,
                    });
                }
                let mut lowered_args = Vec::with_capacity(args.len());
                let mut owned_strings = Vec::new();
                for (index, argument) in args.iter().enumerate() {
                    let value = self.lower_rvalue(argument)?;
                    match argument.ty() {
                        mir::Type::String => {
                            owned_strings.push((index, value.into_pointer_value()));
                        }
                        mir::Type::NullableString => owned_strings.push((
                            index,
                            self.nullable_parts(value.into_struct_value())?
                                .1
                                .into_pointer_value(),
                        )),
                        _ => {}
                    }
                    lowered_args.push(value);
                }
                let class_definition = class_definition(self.program, *class)?;
                let size = class_definition.layout.size;
                let align = class_definition.layout.align;
                let object = self
                    .call_runtime(
                        CLASS_ALLOCATE,
                        &[pointer.into(), usize_type.into(), usize_type.into()],
                        Some(pointer.into()),
                        &[
                            self.current_frame.into(),
                            usize_type.const_int(u64::from(size), false).into(),
                            usize_type.const_int(u64::from(align), false).into(),
                        ],
                    )?
                    .ok_or_else(|| backend_failure("class allocation produced no result"))?
                    .into_pointer_value();
                for (property, lowered_property) in properties.iter().zip(lowered_properties) {
                    let value = match &property.source {
                        mir::PropertyValueSource::Expression(_) => lowered_property,
                        mir::PropertyValueSource::ConstructorArgument(index) => {
                            Some(*lowered_args.get(*index).ok_or_else(|| {
                                malformed_mir(format!(
                                    "constructor argument {index} does not exist"
                                ))
                            })?)
                        }
                        mir::PropertyValueSource::ConstructorBody => None,
                    };
                    let Some(value) = value else {
                        continue;
                    };
                    let address =
                        self.lower_property_address_from_value(object, property.property)?;
                    build(self.builder.build_store(address, value))?;
                }
                if let Some(constructor) = constructor {
                    let callee = *self.functions.get(constructor.0).ok_or_else(|| {
                        malformed_mir(format!("function{} does not exist", constructor.0))
                    })?;
                    let mut constructor_args =
                        Vec::<BasicMetadataValueEnum<'ctx>>::with_capacity(lowered_args.len() + 2);
                    constructor_args.push(self.current_frame.into());
                    constructor_args.push(object.into());
                    constructor_args.extend(
                        lowered_args
                            .iter()
                            .copied()
                            .map(BasicMetadataValueEnum::from),
                    );
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

                    let constructor_definition = function_in(self.program, *constructor)?;
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
                            let value = lowered_args[index].into_pointer_value();
                            self.defer_or_drop_class_temporary(value, class)?;
                        }
                    }
                }
                for (index, string) in owned_strings {
                    let promoted = properties.iter().any(|property| {
                        matches!(
                            property.source,
                            mir::PropertyValueSource::ConstructorArgument(argument)
                                if argument == index
                        )
                    });
                    if !promoted {
                        self.release_string(string)?;
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
                ..
            } => Ok(self
                .lower_collection_index(*collection, index, *transfer)?
                .into_pointer_value()),
        }
    }

    fn lower_nullable_class_expression(
        &mut self,
        expression: &mir::NullableClassExpression,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        match expression {
            mir::NullableClassExpression::Null(_) => Ok(pointer.const_null()),
            mir::NullableClassExpression::Class(value) => self.lower_class_expression(value),
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
            mir::NullableStringExpression::ReadLine => {
                let payload = self
                    .call_runtime(
                        READ_STDIN_LINE,
                        &[pointer.into()],
                        Some(pointer.into()),
                        &[self.current_frame.into()],
                    )?
                    .ok_or_else(|| backend_failure("read_line produced no result"))?
                    .into_pointer_value();
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
                    mir::NullableCollectionAccess::Remove | mir::NullableCollectionAccess::Pop
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
        self.module.get_function(name).unwrap_or_else(|| {
            let ty = match result {
                Some(result) => result.fn_type(params, false),
                None => self.context.void_type().fn_type(params, false),
            };
            self.module.add_function(name, ty, Some(Linkage::External))
        })
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
        drops: &[(PointerValue<'ctx>, crate::class_layout::ClassId)],
    ) -> Result<(), BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        for (slot, class) in drops.iter().rev() {
            let value = build(
                self.builder
                    .build_load(pointer, *slot, "class.temporary.drop"),
            )?
            .into_pointer_value();
            build(self.builder.build_store(*slot, pointer.const_null()))?;
            self.drop_class_value_checked(value, *class)?;
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
        self.deferred_class_temporary_drops.push((slot, class));
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
        self.drop_class_value(object, class)?;
        build(self.builder.build_unconditional_branch(continue_block))?;
        self.builder.position_at_end(continue_block);
        Ok(())
    }

    fn drop_class_value(
        &mut self,
        object: PointerValue<'ctx>,
        class: crate::class_layout::ClassId,
    ) -> Result<(), BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let class_definition = class_definition(self.program, class)?;
        let destructor = class_definition.destructor;
        let properties = class_definition.properties.clone();
        if let Some(destructor) = destructor {
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
                mir::Type::Collection(collection) => {
                    let value = build(self.builder.build_load(
                        pointer,
                        address,
                        "property.collection",
                    ))?
                    .into_pointer_value();
                    self.drop_collection_value(value, collection)?;
                }
                mir::Type::Scalar(_) | mir::Type::NullableScalar(_) => {}
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
            mir::StringExpression::Property { object, property } => {
                let address = self.lower_property_address(*object, *property)?;
                let value = build(self.builder.build_load(pointer, address, "string.property"))?
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
            mir::StringExpression::ReadFile(path) => {
                let path = self.lower_string_expression(path)?;
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
                collection,
                index,
                remove,
            } => {
                let value = self
                    .lower_collection_index(*collection, index, *remove)?
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
        }
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
            mir::IntegerExpression::Unary { ty, op, operand } => {
                let operand = self.lower_integer_expression(operand)?;
                self.lower_integer_unary(*ty, *op, operand)
            }
            mir::IntegerExpression::Binary {
                ty,
                op,
                left,
                right,
            } => {
                let left = self.lower_integer_expression(left)?;
                let right = self.lower_integer_expression(right)?;
                self.lower_integer_binary(*ty, *op, left, right)
            }
            mir::IntegerExpression::Convert { ty, value } => {
                let source = value.ty();
                let value = self.lower_integer_expression(value)?;
                self.lower_integer_conversion(source, *ty, value)
            }
            mir::IntegerExpression::FloatToInt { value } => {
                let value = self.lower_float_expression(value)?;
                self.lower_float_to_int(value)
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
        }
    }

    fn lower_integer_unary(
        &mut self,
        ty: IntegerType,
        op: mir::IntegerUnaryOp,
        operand: IntValue<'ctx>,
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
                self.lower_panic_if(
                    overflow,
                    IntegerPanic::OverflowNegation.message().as_bytes(),
                )?;
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
    ) -> Result<IntValue<'ctx>, BackendError> {
        match op {
            mir::IntegerBinaryOp::Add
            | mir::IntegerBinaryOp::Subtract
            | mir::IntegerBinaryOp::Multiply => self.lower_checked_arithmetic(ty, op, left, right),
            mir::IntegerBinaryOp::Divide => self.lower_integer_division(ty, left, right),
            mir::IntegerBinaryOp::Remainder => self.lower_integer_remainder(ty, left, right),
            mir::IntegerBinaryOp::ShiftLeft | mir::IntegerBinaryOp::ShiftRight => {
                self.lower_integer_shift(ty, op, left, right)
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
    ) -> Result<IntValue<'ctx>, BackendError> {
        let wide_type = self
            .context
            .custom_width_int_type(
                NonZeroU32::new(ty.bit_width().saturating_mul(2))
                    .expect("Doria integer widths are nonzero"),
            )
            .expect("Doria widened integer type is supported by LLVM");
        let left = if ty.is_signed() {
            build(
                self.builder
                    .build_int_s_extend(left, wide_type, "left.wide"),
            )?
        } else {
            build(
                self.builder
                    .build_int_z_extend(left, wide_type, "left.wide"),
            )?
        };
        let right = if ty.is_signed() {
            build(
                self.builder
                    .build_int_s_extend(right, wide_type, "right.wide"),
            )?
        } else {
            build(
                self.builder
                    .build_int_z_extend(right, wide_type, "right.wide"),
            )?
        };
        let result = match op {
            mir::IntegerBinaryOp::Add => {
                build(self.builder.build_int_add(left, right, "checked.add"))?
            }
            mir::IntegerBinaryOp::Subtract => {
                build(self.builder.build_int_sub(left, right, "checked.sub"))?
            }
            mir::IntegerBinaryOp::Multiply => {
                build(self.builder.build_int_mul(left, right, "checked.mul"))?
            }
            _ => unreachable!("non-arithmetic operator reached checked arithmetic lowering"),
        };
        let minimum = wide_integer_constant(wide_type, ty.min_value());
        let maximum = wide_integer_constant(wide_type, ty.max_value());
        let below = build(self.builder.build_int_compare(
            if ty.is_signed() {
                IntPredicate::SLT
            } else {
                IntPredicate::ULT
            },
            result,
            minimum,
            "below.minimum",
        ))?;
        let above = build(self.builder.build_int_compare(
            if ty.is_signed() {
                IntPredicate::SGT
            } else {
                IntPredicate::UGT
            },
            result,
            maximum,
            "above.maximum",
        ))?;
        let overflow = build(self.builder.build_or(below, above, "arithmetic.overflow"))?;
        let panic = match op {
            mir::IntegerBinaryOp::Add => IntegerPanic::OverflowAddition,
            mir::IntegerBinaryOp::Subtract => IntegerPanic::OverflowSubtraction,
            mir::IntegerBinaryOp::Multiply => IntegerPanic::OverflowMultiplication,
            _ => unreachable!("non-arithmetic operator reached checked arithmetic lowering"),
        };
        self.lower_panic_if(overflow, panic.message().as_bytes())?;
        build(self.builder.build_int_truncate(
            result,
            integer_type(self.context, ty),
            "checked.result",
        ))
    }

    fn lower_integer_division(
        &mut self,
        ty: IntegerType,
        left: IntValue<'ctx>,
        right: IntValue<'ctx>,
    ) -> Result<IntValue<'ctx>, BackendError> {
        let zero = integer_type(self.context, ty).const_zero();
        let divides_by_zero = build(self.builder.build_int_compare(
            IntPredicate::EQ,
            right,
            zero,
            "division.by_zero",
        ))?;
        self.lower_panic_if(
            divides_by_zero,
            IntegerPanic::DivisionByZero.message().as_bytes(),
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
            self.lower_panic_if(
                overflow,
                IntegerPanic::DivisionOverflow.message().as_bytes(),
            )?;
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
    ) -> Result<IntValue<'ctx>, BackendError> {
        let integer_type = integer_type(self.context, ty);
        let zero = integer_type.const_zero();
        let divides_by_zero = build(self.builder.build_int_compare(
            IntPredicate::EQ,
            right,
            zero,
            "remainder.by_zero",
        ))?;
        self.lower_panic_if(
            divides_by_zero,
            IntegerPanic::RemainderByZero.message().as_bytes(),
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
        self.lower_panic_if(
            invalid,
            IntegerPanic::ShiftCountOutOfRange.message().as_bytes(),
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
    ) -> Result<IntValue<'ctx>, BackendError> {
        if let Some(out_of_range) = self.conversion_out_of_range(source, target, value)? {
            self.lower_panic_if(
                out_of_range,
                IntegerPanic::ConversionOutOfRange.message().as_bytes(),
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
                collection,
                index,
                remove,
            } => Ok(self
                .lower_collection_index(*collection, index, *remove)?
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
                    collection,
                    index,
                    remove,
                } => Ok(self
                    .lower_collection_index(*collection, index, *remove)?
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
        self.lower_panic_if(invalid, b"float-to-integer conversion out of range")?;
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
        self.lower_call_with_receiver(function, args, expects_result, None)
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
        let mut values = Vec::<BasicMetadataValueEnum<'ctx>>::with_capacity(args.len() + 1);
        values.push(self.current_frame.into());
        if let Some(receiver) = receiver {
            values.push(receiver.into());
        }
        let mut lowered_args = Vec::with_capacity(args.len());
        let mut owned_strings = Vec::new();
        for argument in args {
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
            values.push(value.into());
            lowered_args.push(value);
        }
        let call = build(self.builder.build_call(callee, &values, "call"))?;
        apply_call_abi_attributes(self.context, call, function_in(self.program, function)?)?;
        let result = if expects_result {
            Some(call.try_as_basic_value().basic().ok_or_else(|| {
                malformed_mir(format!("call to function{} produced no result", function.0))
            })?)
        } else {
            None
        };
        for string in owned_strings {
            self.release_string(string)?;
        }
        let callee_definition = function_in(self.program, function)?;
        for (index, argument) in args.iter().enumerate() {
            let Some(class) = argument.owned_temporary_class() else {
                continue;
            };
            let parameter_index = index + usize::from(receiver.is_some());
            let parameter = *callee_definition
                .params
                .get(parameter_index)
                .ok_or_else(|| {
                    malformed_mir(format!(
                        "function{} is missing parameter {parameter_index}",
                        function.0
                    ))
                })?;
            if !local_in(callee_definition, parameter)?.owned {
                let value = lowered_args[index].into_pointer_value();
                self.defer_or_drop_class_temporary(value, class)?;
            }
        }
        Ok(result)
    }
}

fn apply_call_abi_attributes(
    context: &Context,
    call: inkwell::values::CallSiteValue<'_>,
    function: &mir::Function,
) -> Result<(), BackendError> {
    if let mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Integer(ty))) =
        function.return_type
    {
        apply_call_integer_extension_attribute(context, call, AttributeLoc::Return, ty);
    }
    for (index, parameter) in function.params.iter().enumerate() {
        let local = local_in(function, *parameter)?;
        if let mir::Type::Scalar(mir::ScalarType::Integer(ty)) = local.ty {
            apply_call_integer_extension_attribute(
                context,
                call,
                AttributeLoc::Param((index + 1) as u32),
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
                    mir::ScalarType::Bool => match op {
                        mir::CompareOp::Equal | mir::CompareOp::NotEqual => {
                            build(self.builder.build_int_compare(
                                if matches!(op, mir::CompareOp::Equal) {
                                    IntPredicate::EQ
                                } else {
                                    IntPredicate::NE
                                },
                                left.into_int_value(),
                                right.into_int_value(),
                                "bool.compare",
                            ))?
                        }
                        _ => return Err(malformed_mir("ordered bool comparison is invalid")),
                    },
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
                let needle_type = definition.key.unwrap_or(definition.value);
                let needle = self.lower_rvalue(value)?;
                let needle_word = self.value_to_collection_word(needle, needle_type)?;
                let collection_value = self.collection_pointer(*collection)?;
                let kind = self.collection_compare_kind(needle_type)?;
                let found = match op {
                    mir::CollectionMembershipOp::Contains => {
                        let name = if definition.key.is_some() {
                            COLLECTION_KEYED_HAS
                        } else {
                            COLLECTION_CONTAINS
                        };
                        let found = self
                            .call_runtime(
                                name,
                                &[
                                    pointer.into(),
                                    self.context.i64_type().into(),
                                    self.context.i8_type().into(),
                                ],
                                Some(self.context.i8_type().into()),
                                &[collection_value.into(), needle_word.into(), kind.into()],
                            )?
                            .ok_or_else(|| {
                                backend_failure("collection membership produced no result")
                            })?
                            .into_int_value();
                        self.drop_stored_value(needle, needle_type)?;
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
                                ],
                                Some(self.context.i8_type().into()),
                                &[collection_value.into(), needle_word.into(), kind.into()],
                            )?
                            .ok_or_else(|| backend_failure("set insertion produced no result"))?
                            .into_int_value();
                        self.drop_value_unless(inserted, needle, needle_type)?;
                        inserted
                    }
                    mir::CollectionMembershipOp::Remove => {
                        let removed_slot = build(
                            self.builder
                                .build_alloca(self.context.i64_type(), "set.removed.value"),
                        )?;
                        let removed = self
                            .call_runtime(
                                COLLECTION_REMOVE_VALUE,
                                &[
                                    pointer.into(),
                                    self.context.i64_type().into(),
                                    self.context.i8_type().into(),
                                    pointer.into(),
                                ],
                                Some(self.context.i8_type().into()),
                                &[
                                    collection_value.into(),
                                    needle_word.into(),
                                    kind.into(),
                                    removed_slot.into(),
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
                        self.drop_value_if(removed, removed_value, needle_type)?;
                        self.drop_stored_value(needle, needle_type)?;
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
                collection,
                index,
                remove,
            } => Ok(self
                .lower_collection_index(*collection, index, *remove)?
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

    fn lower_panic_if(
        &mut self,
        condition: IntValue<'ctx>,
        message: &[u8],
    ) -> Result<(), BackendError> {
        let function = current_function(&self.builder)?;
        let panic_block = self.context.append_basic_block(function, "panic");
        let continue_block = self.context.append_basic_block(function, "panic.continue");
        build(
            self.builder
                .build_conditional_branch(condition, panic_block, continue_block),
        )?;
        self.builder.position_at_end(panic_block);
        self.lower_runtime_panic(message)?;
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
            .get_function("dr_v1_write_stdout")
            .unwrap_or_else(|| {
                self.module.add_function(
                    "dr_v1_write_stdout",
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

    fn lower_runtime_panic(&mut self, message: &[u8]) -> Result<(), BackendError> {
        let pointer = self.define_data(message, "panic.message");
        let usize_type = self.context.ptr_sized_int_type(self.target_data, None);
        let runtime = self.module.get_function("dr_v1_panic").unwrap_or_else(|| {
            self.module.add_function(
                "dr_v1_panic",
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
                usize_type.const_int(message.len() as u64, false).into(),
            ],
            "panic",
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
    }
}

fn nullable_type<'ctx>(
    context: &'ctx Context,
    target_data: &TargetData,
    payload: BasicTypeEnum<'ctx>,
) -> inkwell::types::StructType<'ctx> {
    let word = context.ptr_sized_int_type(target_data, None);
    context.struct_type(&[word.into(), payload], false)
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
        mir::Type::String
        | mir::Type::Class(_)
        | mir::Type::NullableClass(_)
        | mir::Type::Collection(_) => context.ptr_type(AddressSpace::default()).into(),
    }
}

fn scalar_constant(context: &Context, value: mir::ScalarValue) -> BasicValueEnum<'_> {
    match value {
        mir::ScalarValue::Integer(value) => integer_constant(context, value).into(),
        mir::ScalarValue::Float(value) => float_constant(context, value).into(),
        mir::ScalarValue::Bool(value) => {
            context.i8_type().const_int(u64::from(value), false).into()
        }
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

fn wide_integer_constant(integer_type: IntType<'_>, value: i128) -> IntValue<'_> {
    let bits = value as u128;
    let words = [bits as u64, (bits >> 64) as u64];
    let word_count = integer_type.get_bit_width().div_ceil(64) as usize;
    integer_type.const_int_arbitrary_precision(&words[..word_count])
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
        | mir::StringExpression::Property { .. }
        | mir::StringExpression::Static(_)
        | mir::StringExpression::NullableLocalAssumeNonNull(_)
        | mir::StringExpression::ReadFile(_)
        | mir::StringExpression::Format(_)
        | mir::StringExpression::Coalesce { .. }
        | mir::StringExpression::CollectionIndex { .. }
        | mir::StringExpression::CollectionKeyAt { .. } => {
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
        mir::StringExpression::Display(_)
        | mir::StringExpression::Call { .. }
        | mir::StringExpression::Property { .. }
        | mir::StringExpression::Static(_)
        | mir::StringExpression::NullableLocalAssumeNonNull(_)
        | mir::StringExpression::ReadFile(_)
        | mir::StringExpression::Format(_)
        | mir::StringExpression::Coalesce { .. }
        | mir::StringExpression::CollectionIndex { .. }
        | mir::StringExpression::CollectionKeyAt { .. } => {
            Err(malformed_mir("runtime string expression is not a constant"))
        }
    }
}

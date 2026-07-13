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
    BasicMetadataValueEnum, BasicValueEnum, FloatValue as LlvmFloatValue, FunctionValue, IntValue,
    PointerValue, UnnamedAddress,
};
use inkwell::{AddressSpace, FloatPredicate, IntPredicate, OptimizationLevel};

use crate::backend::BackendError;
use crate::format_string::{FormatConversion, FormatPiece};
use crate::mir;
use crate::mir_validation;
use crate::native_abi::{
    function_symbol, FORMAT_F32, FORMAT_F64, FORMAT_I64, FORMAT_STRING, FORMAT_U64,
    NULLABLE_STRING_EQUAL, READ_FILE, READ_STDIN_LINE, STRING_COMPARE, STRING_CONCAT, STRING_DATA,
    STRING_FROM_BOOL, STRING_FROM_F32, STRING_FROM_F64, STRING_FROM_I64, STRING_FROM_U64,
    STRING_FROM_UTF8, STRING_LENGTH, STRING_RELEASE, STRING_RETAIN, STRING_WRITE_STDERR,
    STRING_WRITE_STDOUT, WRITE_FILE,
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

    let functions = declare_functions(&context, &module, program)?;
    for function in &program.functions {
        define_function(
            &context,
            &module,
            &target_data,
            program,
            function,
            &functions,
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
    program: &mir::Program,
) -> Result<Vec<FunctionValue<'ctx>>, BackendError> {
    let mut functions = Vec::with_capacity(program.functions.len());
    for function in &program.functions {
        let function_type = function_type(context, function)?;
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

fn function_type<'ctx>(
    context: &'ctx Context,
    function: &mir::Function,
) -> Result<inkwell::types::FunctionType<'ctx>, BackendError> {
    let mut parameters = vec![context.ptr_type(AddressSpace::default()).into()];
    for parameter in &function.params {
        let local = local_in(function, *parameter)?;
        parameters.push(llvm_type(context, local.ty).into());
    }
    Ok(match function.return_type {
        mir::ReturnType::Void => context.void_type().fn_type(&parameters, false),
        mir::ReturnType::Value(ty) => llvm_type(context, ty).fn_type(&parameters, false),
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
        match local.ty {
            mir::Type::Scalar(ty) => {
                let slot = build(
                    builder.build_alloca(scalar_type(context, ty), &format!("local{}", local.id.0)),
                )?;
                build(builder.build_store(slot, scalar_zero(context, ty)))?;
                local_slots.push(Some(slot));
            }
            mir::Type::String | mir::Type::NullableString => {
                let slot = build(builder.build_alloca(
                    context.ptr_type(AddressSpace::default()),
                    &format!("local{}", local.id.0),
                ))?;
                build(
                    builder
                        .build_store(slot, context.ptr_type(AddressSpace::default()).const_null()),
                )?;
                local_slots.push(Some(slot));
            }
        }
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
        local_slots,
        blocks,
        current_frame: frame,
        next_data_id: 0,
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
    local_slots: Vec<Option<PointerValue<'ctx>>>,
    blocks: Vec<BasicBlock<'ctx>>,
    current_frame: PointerValue<'ctx>,
    next_data_id: usize,
}

impl<'ctx> FunctionLowerer<'ctx, '_> {
    fn lower_block(&mut self, block: &mir::BasicBlock) -> Result<(), BackendError> {
        for statement in &block.statements {
            self.lower_statement(statement)?;
        }
        self.lower_terminator(&block.terminator)
    }

    fn lower_statement(&mut self, statement: &mir::Statement) -> Result<(), BackendError> {
        match statement {
            mir::Statement::AssignLocal { target, value } => {
                let local = local_in(self.function, *target)?;
                match local.ty {
                    mir::Type::Scalar(_) => {
                        let mir::Rvalue::Value(expression) = value else {
                            return Err(malformed_mir(format!(
                                "scalar local local{} has a non-scalar assignment",
                                target.0
                            )));
                        };
                        let value = self.lower_value_expression(expression)?;
                        build(
                            self.builder
                                .build_store(local_slot(&self.local_slots, *target)?, value),
                        )?;
                    }
                    mir::Type::String => {
                        let mir::Rvalue::String(expression) = value else {
                            return Err(malformed_mir(format!(
                                "string local local{} has a non-string assignment",
                                target.0
                            )));
                        };
                        let value = self.lower_string_expression(expression)?;
                        let slot = local_slot(&self.local_slots, *target)?;
                        let old = build(self.builder.build_load(
                            self.context.ptr_type(AddressSpace::default()),
                            slot,
                            "string.old",
                        ))?
                        .into_pointer_value();
                        self.release_string(old)?;
                        build(self.builder.build_store(slot, value))?;
                    }
                    mir::Type::NullableString => {
                        let mir::Rvalue::NullableString(expression) = value else {
                            return Err(malformed_mir(format!(
                                "nullable-string local local{} has another assignment type",
                                target.0
                            )));
                        };
                        let value = self.lower_nullable_string_expression(expression)?;
                        let slot = local_slot(&self.local_slots, *target)?;
                        let old = build(self.builder.build_load(
                            self.context.ptr_type(AddressSpace::default()),
                            slot,
                            "nullable-string.old",
                        ))?
                        .into_pointer_value();
                        self.release_string(old)?;
                        build(self.builder.build_store(slot, value))?;
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
            mir::Statement::CallVoid { function, args } => {
                let _ = self.lower_call(*function, args, false)?;
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
            mir::Statement::WriteFile { path, contents } => {
                let path = self.lower_string_expression(path)?;
                let contents = self.lower_string_expression(contents)?;
                let pointer = self.context.ptr_type(AddressSpace::default());
                let _ = self.call_runtime(
                    WRITE_FILE,
                    &[pointer.into(), pointer.into(), pointer.into()],
                    None,
                    &[self.current_frame.into(), path.into(), contents.into()],
                )?;
                self.release_string(path)?;
                self.release_string(contents)?;
            }
        }
        Ok(())
    }

    fn lower_terminator(&mut self, terminator: &mir::Terminator) -> Result<(), BackendError> {
        match terminator {
            mir::Terminator::Return(expression) => {
                let value = self.lower_rvalue(expression)?;
                self.cleanup_string_locals()?;
                build(self.builder.build_return(Some(&value)))?;
            }
            mir::Terminator::ReturnVoid => {
                self.cleanup_string_locals()?;
                build(self.builder.build_return(None))?;
            }
            mir::Terminator::Panic(message) => {
                let string = self.lower_string_expression(message)?;
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
            } => self.lower_condition_to_branch(
                condition,
                block_for(&self.blocks, *then_block)?,
                block_for(&self.blocks, *else_block)?,
            )?,
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
            mir::Rvalue::NullableString(value) => {
                Ok(self.lower_nullable_string_expression(value)?.into())
            }
        }
    }

    fn lower_nullable_string_expression(
        &mut self,
        expression: &mir::NullableStringExpression,
    ) -> Result<PointerValue<'ctx>, BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        match expression {
            mir::NullableStringExpression::Null => Ok(pointer.const_null()),
            mir::NullableStringExpression::String(value) => self.lower_string_expression(value),
            mir::NullableStringExpression::Local(local) => {
                let value = build(self.builder.build_load(
                    pointer,
                    local_slot(&self.local_slots, *local)?,
                    "nullable-string.local",
                ))?
                .into_pointer_value();
                self.retain_string(value)
            }
            mir::NullableStringExpression::ReadLine => Ok(self
                .call_runtime(
                    READ_STDIN_LINE,
                    &[pointer.into()],
                    Some(pointer.into()),
                    &[self.current_frame.into()],
                )?
                .ok_or_else(|| backend_failure("read_line produced no result"))?
                .into_pointer_value()),
            mir::NullableStringExpression::Call { function, args } => Ok(self
                .lower_call(*function, args, true)?
                .ok_or_else(|| malformed_mir("nullable-string call produced no result"))?
                .into_pointer_value()),
        }
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
        let pointer = self.context.ptr_type(AddressSpace::default());
        for local in &self.function.locals {
            if matches!(local.ty, mir::Type::String | mir::Type::NullableString) {
                let value = build(self.builder.build_load(
                    pointer,
                    local_slot(&self.local_slots, local.id)?,
                    "string.cleanup",
                ))?
                .into_pointer_value();
                self.release_string(value)?;
            }
        }
        Ok(())
    }

    fn retain_string_parameters(&self) -> Result<(), BackendError> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        for parameter in &self.function.params {
            if matches!(
                local_in(self.function, *parameter)?.ty,
                mir::Type::String | mir::Type::NullableString
            ) {
                let slot = local_slot(&self.local_slots, *parameter)?;
                let value = build(self.builder.build_load(pointer, slot, "string.parameter"))?
                    .into_pointer_value();
                let retained = self.retain_string(value)?;
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
            mir::StringExpression::Local(local)
            | mir::StringExpression::NullableLocalAssumeNonNull(local) => {
                let value = build(self.builder.build_load(
                    pointer,
                    local_slot(&self.local_slots, *local)?,
                    "string.local",
                ))?
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
        }
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
                mir::FormatArgument::String(value) => self.lower_string_expression(value)?,
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
        &self,
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
        let callee = *self
            .functions
            .get(function.0)
            .ok_or_else(|| malformed_mir(format!("function{} does not exist", function.0)))?;
        let mut values = Vec::<BasicMetadataValueEnum<'ctx>>::with_capacity(args.len() + 1);
        values.push(self.current_frame.into());
        let mut owned_strings = Vec::new();
        for argument in args {
            let value = self.lower_rvalue(argument)?;
            if matches!(argument.ty(), mir::Type::String | mir::Type::NullableString) {
                owned_strings.push(value.into_pointer_value());
            }
            values.push(value.into());
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

    fn lower_bool_operand(&self, operand: &mir::Operand) -> Result<IntValue<'ctx>, BackendError> {
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

fn llvm_type(context: &Context, ty: mir::Type) -> BasicTypeEnum<'_> {
    match ty {
        mir::Type::Scalar(ty) => scalar_type(context, ty),
        mir::Type::String | mir::Type::NullableString => {
            context.ptr_type(AddressSpace::default()).into()
        }
    }
}

fn scalar_zero(context: &Context, ty: mir::ScalarType) -> BasicValueEnum<'_> {
    scalar_type(context, ty).const_zero()
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
        | mir::StringExpression::NullableLocalAssumeNonNull(_)
        | mir::StringExpression::ReadFile(_)
        | mir::StringExpression::Format(_) => {
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
        | mir::StringExpression::NullableLocalAssumeNonNull(_)
        | mir::StringExpression::ReadFile(_)
        | mir::StringExpression::Format(_) => {
            Err(malformed_mir("runtime string expression is not a constant"))
        }
    }
}

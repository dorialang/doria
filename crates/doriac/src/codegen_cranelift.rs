use std::collections::{HashMap, HashSet};

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::immediates::{Ieee32, Ieee64};
use cranelift_codegen::ir::{
    types, AbiParam, Block, BlockArg, InstBuilder, Signature, StackSlot, StackSlotData,
    StackSlotKind, TrapCode, Type as ClifType, Value,
};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{default_libcall_names, DataDescription, FuncId, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};

use crate::backend::BackendError;
use crate::mir;
use crate::mir_validation;
use crate::native_abi::function_symbol;
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

    let mut process_signature = module.make_signature();
    process_signature.returns.push(AbiParam::new(types::I32));
    let process_main_id = module
        .declare_function("main", Linkage::Export, &process_signature)
        .map_err(|error| backend_failure(error.to_string()))?;

    for function in &program.functions {
        define_function(&mut module, program, function, &function_ids)?;
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
        signature
            .params
            .push(scalar_abi_param(scalar_local_type(function, *parameter)?));
    }
    if let mir::ReturnType::Value(ty) = function.return_type {
        signature.returns.push(scalar_abi_param(ty));
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

fn scalar_storage_bytes(ty: mir::ScalarType) -> u32 {
    match ty {
        mir::ScalarType::Integer(ty) => ty.storage_bytes(),
        mir::ScalarType::Float(ty) => ty.storage_bytes(),
        mir::ScalarType::Bool => 1,
    }
}

fn define_function(
    module: &mut ObjectModule,
    program: &mir::Program,
    function: &mir::Function,
    function_ids: &[FuncId],
) -> Result<(), BackendError> {
    let function_id = *function_ids
        .get(function.id.0)
        .ok_or_else(|| malformed_mir(format!("function{} was not declared", function.id.0)))?;
    let signature = function_signature(module, function)?;
    let string_values = resolve_string_locals(function)?;
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
                mir::Type::String => None,
            })
            .collect::<Vec<_>>();
        let pointer_type = module.target_config().pointer_type();
        let pointer_bytes = pointer_type.bytes();
        let frame_slot = builder.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot,
            pointer_bytes * 3,
            pointer_bytes.trailing_zeros() as u8,
        ));

        builder.switch_to_block(entry);
        initialize_scalar_locals(&mut builder, function, &local_slots)?;
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

        let mut resources = LoweringResources::new(
            module,
            program,
            function_ids,
            &local_slots,
            &string_values,
            function.id,
            current_frame,
        );
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

fn initialize_scalar_locals(
    builder: &mut FunctionBuilder,
    function: &mir::Function,
    slots: &[Option<StackSlot>],
) -> Result<(), BackendError> {
    for local in &function.locals {
        let mir::Type::Scalar(ty) = local.ty else {
            continue;
        };
        let zero = match ty {
            mir::ScalarType::Integer(ty) => builder.ins().iconst(clif_integer_type(ty), 0),
            mir::ScalarType::Float(FloatType::Float32) => {
                builder.ins().f32const(Ieee32::with_bits(0))
            }
            mir::ScalarType::Float(FloatType::Float64) => {
                builder.ins().f64const(Ieee64::with_bits(0))
            }
            mir::ScalarType::Bool => builder.ins().iconst(types::I8, 0),
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
    for (parameter, value) in function.params.iter().zip(params.into_iter().skip(1)) {
        let slot = local_slot(slots, *parameter)?;
        builder.ins().stack_store(value, slot, 0);
    }
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
            mir::ReturnType::Value(mir::ScalarType::Integer(IntegerType::Int64)) => {
                "dr_v1_main_int"
            }
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
    local_slots: &'program [Option<StackSlot>],
    string_values: &'program HashMap<mir::LocalId, Vec<u8>>,
    write_stdout_func_id: Option<FuncId>,
    panic_func_id: Option<FuncId>,
    next_data_id: usize,
    function_id: mir::FunctionId,
    current_frame: Value,
}

impl<'module, 'program> LoweringResources<'module, 'program> {
    fn new(
        module: &'module mut ObjectModule,
        program: &'program mir::Program,
        function_ids: &'program [FuncId],
        local_slots: &'program [Option<StackSlot>],
        string_values: &'program HashMap<mir::LocalId, Vec<u8>>,
        function_id: mir::FunctionId,
        current_frame: Value,
    ) -> Self {
        Self {
            module,
            program,
            function_ids,
            local_slots,
            string_values,
            write_stdout_func_id: None,
            panic_func_id: None,
            next_data_id: 0,
            function_id,
            current_frame,
        }
    }

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
    match statement {
        mir::Statement::AssignLocal { target, value } => {
            let definition = local_definition(resources.program, resources.function_id, *target)?;
            match definition.ty {
                mir::Type::Scalar(_) => {
                    let mir::Rvalue::Value(expression) = value else {
                        return Err(malformed_mir(format!(
                            "scalar local local{} has a non-scalar assignment",
                            target.0
                        )));
                    };
                    let value = lower_value_expression(builder, expression, resources)?;
                    let slot = local_slot(resources.local_slots, *target)?;
                    builder.ins().stack_store(value, slot, 0);
                }
                mir::Type::String => {
                    if !matches!(value, mir::Rvalue::String(_)) {
                        return Err(malformed_mir(format!(
                            "string local local{} has a non-string assignment",
                            target.0
                        )));
                    }
                }
            }
        }
        mir::Statement::EchoStringLiteral(value) => {
            lower_echo_bytes(builder, value.as_bytes(), resources)?;
        }
        mir::Statement::EchoString(value) => {
            let bytes = resolve_string_expression(value, resources.string_values)?;
            lower_echo_bytes(builder, &bytes, resources)?;
        }
        mir::Statement::CallVoid { function, args } => {
            let mut values = vec![resources.current_frame];
            values.extend(lower_call_args(builder, args, resources)?);
            let callee = declared_function(builder, resources, *function)?;
            builder.ins().call(callee, &values);
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
            let value = lower_value_expression(builder, expression, resources)?;
            builder.ins().return_(&[value]);
        }
        mir::Terminator::ReturnVoid => {
            builder.ins().return_(&[]);
        }
        mir::Terminator::Panic(message) => {
            let bytes = resolve_string_expression(message, resources.string_values)?;
            lower_runtime_panic(builder, &bytes, resources)?;
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
        } => lower_condition_to_branch(
            builder,
            condition,
            block_for(blocks, *then_block)?,
            block_for(blocks, *else_block)?,
            resources,
        )?,
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
    args: &[mir::ValueExpression],
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let mut values = vec![resources.current_frame];
    values.extend(lower_call_args(builder, args, resources)?);
    let callee = declared_function(builder, resources, function)?;
    let call = builder.ins().call(callee, &values);
    builder.inst_results(call).first().copied().ok_or_else(|| {
        malformed_mir(format!(
            "{ty} call to function{} produced no result",
            function.0,
        ))
    })
}

fn lower_call_args(
    builder: &mut FunctionBuilder,
    args: &[mir::ValueExpression],
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Vec<Value>, BackendError> {
    args.iter()
        .map(|argument| lower_value_expression(builder, argument, resources))
        .collect()
}

fn lower_scalar_call(
    builder: &mut FunctionBuilder,
    function: mir::FunctionId,
    args: &[mir::ValueExpression],
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let mut values = vec![resources.current_frame];
    values.extend(lower_call_args(builder, args, resources)?);
    let callee = declared_function(builder, resources, function)?;
    let call = builder.ins().call(callee, &values);
    builder
        .inst_results(call)
        .first()
        .copied()
        .ok_or_else(|| malformed_mir(format!("call to function{} produced no result", function.0)))
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
        _ => Err(malformed_mir("bool expression contains non-bool constant")),
    }
}

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
    }
}

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

fn scalar_local_type(
    function: &mir::Function,
    id: mir::LocalId,
) -> Result<mir::ScalarType, BackendError> {
    let local = local_in(function, id)?;
    let mir::Type::Scalar(ty) = local.ty else {
        return Err(malformed_mir(format!(
            "LocalId local{} is not a scalar local",
            id.0
        )));
    };
    Ok(ty)
}

fn local_definition(
    program: &mir::Program,
    function: mir::FunctionId,
    local: mir::LocalId,
) -> Result<&mir::Local, BackendError> {
    local_in(function_in(program, function)?, local)
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

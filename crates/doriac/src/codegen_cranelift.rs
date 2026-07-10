use std::collections::{HashMap, HashSet};

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{
    types, AbiParam, Block, BlockArg, InstBuilder, Signature, StackSlot, StackSlotData,
    StackSlotKind, TrapCode, Value,
};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{default_libcall_names, DataDescription, FuncId, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};

use crate::backend::BackendError;
use crate::mir;

const ARITHMETIC_OVERFLOW_TRAP: u8 = 1;
const PROCESS_STATUS_TRAP: u8 = 2;
const STDOUT_TRAP: u8 = 3;

pub fn lower_mir_to_object(program: &mir::Program) -> Result<Vec<u8>, BackendError> {
    validate_program(program)?;

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
        ObjectBuilder::new(isa, "doria_stage_11", default_libcall_names())
            .map_err(|error| backend_failure(error.to_string()))?,
    );

    let mut function_ids = Vec::with_capacity(program.functions.len());
    for function in &program.functions {
        let signature = function_signature(&mut module, function);
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

fn function_signature(module: &mut ObjectModule, function: &mir::Function) -> Signature {
    let mut signature = module.make_signature();
    for _ in &function.params {
        signature.params.push(AbiParam::new(types::I64));
    }
    if function.return_type == mir::ReturnType::Int {
        signature.returns.push(AbiParam::new(types::I64));
    }
    signature
}

fn function_symbol(function: &mir::Function) -> String {
    let sanitized = function
        .name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!("__doria_fn_{}_{}", function.id.0, sanitized)
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
    let signature = function_signature(module, function);
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
                mir::Type::Int => Some(builder.create_sized_stack_slot(StackSlotData::new(
                    StackSlotKind::ExplicitSlot,
                    8,
                    3,
                ))),
                mir::Type::String => None,
            })
            .collect::<Vec<_>>();

        let mut resources = LoweringResources::new(
            module,
            program,
            function_ids,
            &local_slots,
            &string_values,
            function.id,
        );

        let mut order = Vec::with_capacity(function.blocks.len());
        order.push(function.entry_block.0);
        order.extend((0..function.blocks.len()).filter(|index| *index != function.entry_block.0));

        for block_index in order {
            let mir_block = &function.blocks[block_index];
            let block = blocks[block_index];
            builder.switch_to_block(block);

            if mir_block.id == function.entry_block {
                initialize_integer_locals(&mut builder, &local_slots);
                bind_parameters(&mut builder, function, &local_slots, block)?;
            }

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

fn initialize_integer_locals(builder: &mut FunctionBuilder, slots: &[Option<StackSlot>]) {
    let zero = builder.ins().iconst(types::I64, 0);
    for slot in slots.iter().flatten() {
        builder.ins().stack_store(zero, *slot, 0);
    }
}

fn bind_parameters(
    builder: &mut FunctionBuilder,
    function: &mir::Function,
    slots: &[Option<StackSlot>],
    entry: Block,
) -> Result<(), BackendError> {
    let params = builder.block_params(entry).to_vec();
    for (parameter, value) in function.params.iter().zip(params) {
        let slot = int_slot(slots, *parameter)?;
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

        let callee = module.declare_func_in_func(entry_id, builder.func);
        let call = builder.ins().call(callee, &[]);
        match entry.return_type {
            mir::ReturnType::Int => {
                let value = builder.inst_results(call)[0];
                let negative = builder.ins().icmp_imm(IntCC::SignedLessThan, value, 0);
                builder
                    .ins()
                    .trapnz(negative, TrapCode::unwrap_user(PROCESS_STATUS_TRAP));
                let too_large = builder.ins().icmp_imm(IntCC::SignedGreaterThan, value, 125);
                builder
                    .ins()
                    .trapnz(too_large, TrapCode::unwrap_user(PROCESS_STATUS_TRAP));
                let status = builder.ins().ireduce(types::I32, value);
                builder.ins().return_(&[status]);
            }
            mir::ReturnType::Void => {
                let success = builder.ins().iconst(types::I32, 0);
                builder.ins().return_(&[success]);
            }
        }
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
    write_func_id: Option<FuncId>,
    get_std_handle_func_id: Option<FuncId>,
    write_file_func_id: Option<FuncId>,
    next_data_id: usize,
    function_id: mir::FunctionId,
}

impl<'module, 'program> LoweringResources<'module, 'program> {
    fn new(
        module: &'module mut ObjectModule,
        program: &'program mir::Program,
        function_ids: &'program [FuncId],
        local_slots: &'program [Option<StackSlot>],
        string_values: &'program HashMap<mir::LocalId, Vec<u8>>,
        function_id: mir::FunctionId,
    ) -> Self {
        Self {
            module,
            program,
            function_ids,
            local_slots,
            string_values,
            write_func_id: None,
            get_std_handle_func_id: None,
            write_file_func_id: None,
            next_data_id: 0,
            function_id,
        }
    }

    fn declare_write(&mut self) -> Result<FuncId, BackendError> {
        if let Some(id) = self.write_func_id {
            return Ok(id);
        }
        let pointer_type = self.module.target_config().pointer_type();
        let mut signature = self.module.make_signature();
        signature.params.push(AbiParam::new(types::I32));
        signature.params.push(AbiParam::new(pointer_type));
        signature.params.push(AbiParam::new(pointer_type));
        signature.returns.push(AbiParam::new(pointer_type));
        let id = self
            .module
            .declare_function("write", Linkage::Import, &signature)
            .map_err(|error| backend_failure(error.to_string()))?;
        self.write_func_id = Some(id);
        Ok(id)
    }

    fn declare_get_std_handle(&mut self) -> Result<FuncId, BackendError> {
        if let Some(id) = self.get_std_handle_func_id {
            return Ok(id);
        }
        let pointer_type = self.module.target_config().pointer_type();
        let mut signature = self.module.make_signature();
        signature.params.push(AbiParam::new(types::I32));
        signature.returns.push(AbiParam::new(pointer_type));
        let id = self
            .module
            .declare_function("GetStdHandle", Linkage::Import, &signature)
            .map_err(|error| backend_failure(error.to_string()))?;
        self.get_std_handle_func_id = Some(id);
        Ok(id)
    }

    fn declare_write_file(&mut self) -> Result<FuncId, BackendError> {
        if let Some(id) = self.write_file_func_id {
            return Ok(id);
        }
        let pointer_type = self.module.target_config().pointer_type();
        let mut signature = self.module.make_signature();
        signature.params.push(AbiParam::new(pointer_type));
        signature.params.push(AbiParam::new(pointer_type));
        signature.params.push(AbiParam::new(types::I32));
        signature.params.push(AbiParam::new(pointer_type));
        signature.params.push(AbiParam::new(pointer_type));
        signature.returns.push(AbiParam::new(types::I32));
        let id = self
            .module
            .declare_function("WriteFile", Linkage::Import, &signature)
            .map_err(|error| backend_failure(error.to_string()))?;
        self.write_file_func_id = Some(id);
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
                mir::Type::Int => {
                    let value = lower_int_rvalue(builder, value, resources)?;
                    let slot = int_slot(resources.local_slots, *target)?;
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
            let values = lower_call_args(builder, args, resources)?;
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
        mir::Terminator::Return(operand) => {
            let value = lower_operand(builder, operand, resources)?;
            builder.ins().return_(&[value]);
        }
        mir::Terminator::ReturnVoid => {
            builder.ins().return_(&[]);
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

fn lower_int_rvalue(
    builder: &mut FunctionBuilder,
    value: &mir::Rvalue,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    match value {
        mir::Rvalue::Use(operand) => lower_operand(builder, operand, resources),
        mir::Rvalue::Binary { op, left, right } => {
            let left = lower_operand(builder, left, resources)?;
            let right = lower_operand(builder, right, resources)?;
            Ok(lower_checked_binary(builder, *op, left, right))
        }
        mir::Rvalue::Call { function, args } => lower_int_call(builder, *function, args, resources),
        mir::Rvalue::String(_) => Err(malformed_mir(
            "string rvalue reached integer Cranelift lowering",
        )),
    }
}

fn lower_int_expression(
    builder: &mut FunctionBuilder,
    expression: &mir::IntExpression,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    match expression {
        mir::IntExpression::Use(operand) => lower_operand(builder, operand, resources),
        mir::IntExpression::Binary { op, left, right } => {
            let left = lower_int_expression(builder, left, resources)?;
            let right = lower_int_expression(builder, right, resources)?;
            Ok(lower_checked_binary(builder, *op, left, right))
        }
        mir::IntExpression::Call { function, args } => {
            lower_int_call(builder, *function, args, resources)
        }
    }
}

fn lower_checked_binary(
    builder: &mut FunctionBuilder,
    op: mir::BinaryOp,
    left: Value,
    right: Value,
) -> Value {
    let (value, overflow) = match op {
        mir::BinaryOp::Add => builder.ins().sadd_overflow(left, right),
        mir::BinaryOp::Subtract => builder.ins().ssub_overflow(left, right),
        mir::BinaryOp::Multiply => builder.ins().smul_overflow(left, right),
    };
    builder
        .ins()
        .trapnz(overflow, TrapCode::unwrap_user(ARITHMETIC_OVERFLOW_TRAP));
    value
}

fn lower_operand(
    builder: &mut FunctionBuilder,
    operand: &mir::Operand,
    resources: &LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    match operand {
        mir::Operand::Int(value) => Ok(builder.ins().iconst(types::I64, *value)),
        mir::Operand::Local(id) => {
            let slot = int_slot(resources.local_slots, *id)?;
            Ok(builder.ins().stack_load(types::I64, slot, 0))
        }
    }
}

fn lower_int_call(
    builder: &mut FunctionBuilder,
    function: mir::FunctionId,
    args: &[mir::IntExpression],
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Value, BackendError> {
    let values = lower_call_args(builder, args, resources)?;
    let callee = declared_function(builder, resources, function)?;
    let call = builder.ins().call(callee, &values);
    builder.inst_results(call).first().copied().ok_or_else(|| {
        malformed_mir(format!(
            "int call to function{} produced no result",
            function.0
        ))
    })
}

fn lower_call_args(
    builder: &mut FunctionBuilder,
    args: &[mir::IntExpression],
    resources: &mut LoweringResources<'_, '_>,
) -> Result<Vec<Value>, BackendError> {
    args.iter()
        .map(|argument| lower_int_expression(builder, argument, resources))
        .collect()
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
    condition: &mir::Condition,
    then_block: Block,
    else_block: Block,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    match condition {
        mir::Condition::Bool(true) => {
            builder.ins().jump(then_block, &[]);
        }
        mir::Condition::Bool(false) => {
            builder.ins().jump(else_block, &[]);
        }
        mir::Condition::Compare { op, left, right } => {
            let left = lower_int_expression(builder, left, resources)?;
            let right = lower_int_expression(builder, right, resources)?;
            let value = builder.ins().icmp(compare_code(*op), left, right);
            builder.ins().brif(value, then_block, &[], else_block, &[]);
        }
        mir::Condition::Not(condition) => {
            lower_condition_to_branch(builder, condition, else_block, then_block, resources)?;
        }
        mir::Condition::Binary {
            op: mir::ConditionBinaryOp::And,
            left,
            right,
        } => {
            let right_block = builder.create_block();
            lower_condition_to_branch(builder, left, right_block, else_block, resources)?;
            builder.switch_to_block(right_block);
            lower_condition_to_branch(builder, right, then_block, else_block, resources)?;
        }
        mir::Condition::Binary {
            op: mir::ConditionBinaryOp::Or,
            left,
            right,
        } => {
            let right_block = builder.create_block();
            lower_condition_to_branch(builder, left, then_block, right_block, resources)?;
            builder.switch_to_block(right_block);
            lower_condition_to_branch(builder, right, then_block, else_block, resources)?;
        }
        mir::Condition::Binary {
            op: mir::ConditionBinaryOp::Xor,
            left,
            right,
        } => {
            let left = lower_condition_value(builder, left, resources)?;
            let right = lower_condition_value(builder, right, resources)?;
            let value = builder.ins().icmp(IntCC::NotEqual, left, right);
            builder.ins().brif(value, then_block, &[], else_block, &[]);
        }
    }
    Ok(())
}

fn lower_condition_value(
    builder: &mut FunctionBuilder,
    condition: &mir::Condition,
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

fn compare_code(op: mir::CompareOp) -> IntCC {
    match op {
        mir::CompareOp::Equal => IntCC::Equal,
        mir::CompareOp::NotEqual => IntCC::NotEqual,
        mir::CompareOp::Less => IntCC::SignedLessThan,
        mir::CompareOp::LessEqual => IntCC::SignedLessThanOrEqual,
        mir::CompareOp::Greater => IntCC::SignedGreaterThan,
        mir::CompareOp::GreaterEqual => IntCC::SignedGreaterThanOrEqual,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StdoutPlatform {
    Unix,
    Windows,
    Unsupported,
}

fn stdout_platform(windows: bool, unix: bool) -> StdoutPlatform {
    if windows {
        StdoutPlatform::Windows
    } else if unix {
        StdoutPlatform::Unix
    } else {
        StdoutPlatform::Unsupported
    }
}

fn host_stdout_platform() -> StdoutPlatform {
    stdout_platform(cfg!(windows), cfg!(unix))
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
    match host_stdout_platform() {
        StdoutPlatform::Unix => lower_unix_write(builder, bytes.len(), pointer, resources),
        StdoutPlatform::Windows => lower_windows_write(builder, bytes.len(), pointer, resources),
        StdoutPlatform::Unsupported => Err(BackendError::new(
            "native exact-byte stdout is supported only on Unix-like and Windows targets",
        )),
    }
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

fn lower_unix_write(
    builder: &mut FunctionBuilder,
    byte_count: usize,
    data_pointer: Value,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let write_id = resources.declare_write()?;
    let write = resources
        .module
        .declare_func_in_func(write_id, builder.func);
    let pointer_type = resources.module.target_config().pointer_type();
    let fd = builder.ins().iconst(types::I32, 1);
    let loop_block = builder.create_block();
    let write_block = builder.create_block();
    let advance_block = builder.create_block();
    let error_block = builder.create_block();
    let done_block = builder.create_block();
    builder.append_block_param(loop_block, pointer_type);
    builder.append_block_param(loop_block, pointer_type);

    let zero = builder.ins().iconst(pointer_type, 0);
    let count = builder.ins().iconst(pointer_type, byte_count as i64);
    builder
        .ins()
        .jump(loop_block, &[BlockArg::Value(zero), BlockArg::Value(count)]);

    builder.switch_to_block(loop_block);
    let offset = builder.block_params(loop_block)[0];
    let remaining = builder.block_params(loop_block)[1];
    let complete = builder.ins().icmp_imm(IntCC::Equal, remaining, 0);
    builder
        .ins()
        .brif(complete, done_block, &[], write_block, &[]);

    builder.switch_to_block(write_block);
    let current = builder.ins().iadd(data_pointer, offset);
    let call = builder.ins().call(write, &[fd, current, remaining]);
    let written = builder.inst_results(call)[0];
    let progressed = builder.ins().icmp_imm(IntCC::SignedGreaterThan, written, 0);
    builder
        .ins()
        .brif(progressed, advance_block, &[], error_block, &[]);

    builder.switch_to_block(advance_block);
    let next_offset = builder.ins().iadd(offset, written);
    let next_remaining = builder.ins().isub(remaining, written);
    builder.ins().jump(
        loop_block,
        &[
            BlockArg::Value(next_offset),
            BlockArg::Value(next_remaining),
        ],
    );

    builder.switch_to_block(error_block);
    builder.ins().trap(TrapCode::unwrap_user(STDOUT_TRAP));

    builder.switch_to_block(done_block);
    Ok(())
}

fn lower_windows_write(
    builder: &mut FunctionBuilder,
    byte_count: usize,
    data_pointer: Value,
    resources: &mut LoweringResources<'_, '_>,
) -> Result<(), BackendError> {
    let byte_count = i32::try_from(byte_count).map_err(|_| {
        BackendError::new("Windows exact-byte stdout supports writes up to 2147483647 bytes")
    })?;
    let pointer_type = resources.module.target_config().pointer_type();
    let get_std_handle_id = resources.declare_get_std_handle()?;
    let get_std_handle = resources
        .module
        .declare_func_in_func(get_std_handle_id, builder.func);
    let stdout = builder.ins().iconst(types::I32, -11);
    let handle_call = builder.ins().call(get_std_handle, &[stdout]);
    let handle = builder.inst_results(handle_call)[0];

    let written_slot =
        builder.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 4, 2));
    let zero = builder.ins().iconst(types::I32, 0);
    let written_pointer = builder.ins().stack_addr(pointer_type, written_slot, 0);
    let overlapped = builder.ins().iconst(pointer_type, 0);
    let write_file_id = resources.declare_write_file()?;
    let write_file = resources
        .module
        .declare_func_in_func(write_file_id, builder.func);

    let loop_block = builder.create_block();
    let write_block = builder.create_block();
    let check_block = builder.create_block();
    let advance_block = builder.create_block();
    let error_block = builder.create_block();
    let done_block = builder.create_block();
    builder.append_block_param(loop_block, pointer_type);
    builder.append_block_param(loop_block, types::I32);
    let offset = builder.ins().iconst(pointer_type, 0);
    let remaining = builder.ins().iconst(types::I32, i64::from(byte_count));
    builder.ins().jump(
        loop_block,
        &[BlockArg::Value(offset), BlockArg::Value(remaining)],
    );

    builder.switch_to_block(loop_block);
    let offset = builder.block_params(loop_block)[0];
    let remaining = builder.block_params(loop_block)[1];
    let complete = builder.ins().icmp_imm(IntCC::Equal, remaining, 0);
    builder
        .ins()
        .brif(complete, done_block, &[], write_block, &[]);

    builder.switch_to_block(write_block);
    builder.ins().stack_store(zero, written_slot, 0);
    let current = builder.ins().iadd(data_pointer, offset);
    let call = builder.ins().call(
        write_file,
        &[handle, current, remaining, written_pointer, overlapped],
    );
    let write_ok = builder.inst_results(call)[0];
    let succeeded = builder.ins().icmp_imm(IntCC::NotEqual, write_ok, 0);
    builder
        .ins()
        .brif(succeeded, check_block, &[], error_block, &[]);

    builder.switch_to_block(check_block);
    let written = builder.ins().stack_load(types::I32, written_slot, 0);
    let progressed = builder.ins().icmp_imm(IntCC::NotEqual, written, 0);
    builder
        .ins()
        .brif(progressed, advance_block, &[], error_block, &[]);

    builder.switch_to_block(advance_block);
    let written_offset = builder.ins().uextend(pointer_type, written);
    let next_offset = builder.ins().iadd(offset, written_offset);
    let next_remaining = builder.ins().isub(remaining, written);
    builder.ins().jump(
        loop_block,
        &[
            BlockArg::Value(next_offset),
            BlockArg::Value(next_remaining),
        ],
    );

    builder.switch_to_block(error_block);
    builder.ins().trap(TrapCode::unwrap_user(STDOUT_TRAP));

    builder.switch_to_block(done_block);
    Ok(())
}

fn validate_program(program: &mir::Program) -> Result<(), BackendError> {
    let entry = program
        .functions
        .get(program.entry.0)
        .ok_or_else(|| malformed_mir("entry function does not exist"))?;
    if entry.id != program.entry {
        return Err(malformed_mir(
            "entry FunctionId does not match its table slot",
        ));
    }
    if !entry.params.is_empty() {
        return Err(malformed_mir("entry function declares parameters"));
    }

    for (index, function) in program.functions.iter().enumerate() {
        if function.id != mir::FunctionId(index) {
            return Err(malformed_mir(format!(
                "function table slot {index} contains function{}",
                function.id.0
            )));
        }
        validate_function(program, function)?;
    }
    validate_acyclic_calls(program)
}

fn validate_function(program: &mir::Program, function: &mir::Function) -> Result<(), BackendError> {
    for (index, local) in function.locals.iter().enumerate() {
        if local.id != mir::LocalId(index) {
            return Err(malformed_mir(format!(
                "function {} local slot {index} contains local{}",
                function.name, local.id.0
            )));
        }
    }
    for parameter in &function.params {
        let local = local_in(function, *parameter)?;
        if local.ty != mir::Type::Int {
            return Err(malformed_mir(format!(
                "function {} parameter local{} is not int",
                function.name, parameter.0
            )));
        }
    }
    block_in(function, function.entry_block)?;
    for (index, block) in function.blocks.iter().enumerate() {
        if block.id != mir::BlockId(index) {
            return Err(malformed_mir(format!(
                "function {} block slot {index} contains block{}",
                function.name, block.id.0
            )));
        }
        for statement in &block.statements {
            validate_statement(program, function, statement)?;
        }
        validate_terminator(program, function, &block.terminator)?;
    }
    Ok(())
}

fn validate_statement(
    program: &mir::Program,
    function: &mir::Function,
    statement: &mir::Statement,
) -> Result<(), BackendError> {
    match statement {
        mir::Statement::AssignLocal { target, value } => {
            let local = local_in(function, *target)?;
            match (local.ty, value) {
                (mir::Type::String, mir::Rvalue::String(expression)) => {
                    validate_string_expression(function, expression)
                }
                (mir::Type::String, _) => Err(malformed_mir(format!(
                    "string local local{} receives an int rvalue",
                    target.0
                ))),
                (mir::Type::Int, mir::Rvalue::String(_)) => Err(malformed_mir(format!(
                    "int local local{} receives a string rvalue",
                    target.0
                ))),
                (mir::Type::Int, value) => validate_rvalue(program, function, value),
            }
        }
        mir::Statement::EchoStringLiteral(_) => Ok(()),
        mir::Statement::EchoString(expression) => validate_string_expression(function, expression),
        mir::Statement::CallVoid {
            function: callee,
            args,
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type != mir::ReturnType::Void {
                return Err(malformed_mir(format!(
                    "void call targets int function {}",
                    callee.name
                )));
            }
            validate_call_args(program, function, callee, args)
        }
    }
}

fn validate_terminator(
    program: &mir::Program,
    function: &mir::Function,
    terminator: &mir::Terminator,
) -> Result<(), BackendError> {
    match terminator {
        mir::Terminator::Return(operand) => {
            if function.return_type != mir::ReturnType::Int {
                return Err(malformed_mir(format!(
                    "void function {} has an int return",
                    function.name
                )));
            }
            validate_operand(function, operand)
        }
        mir::Terminator::ReturnVoid => {
            if function.return_type != mir::ReturnType::Void {
                return Err(malformed_mir(format!(
                    "int function {} has a void return",
                    function.name
                )));
            }
            Ok(())
        }
        mir::Terminator::Jump(target) => block_in(function, *target).map(|_| ()),
        mir::Terminator::Branch {
            condition,
            then_block,
            else_block,
        } => {
            block_in(function, *then_block)?;
            block_in(function, *else_block)?;
            validate_condition(program, function, condition)
        }
    }
}

fn validate_rvalue(
    program: &mir::Program,
    function: &mir::Function,
    value: &mir::Rvalue,
) -> Result<(), BackendError> {
    match value {
        mir::Rvalue::Use(operand) => validate_operand(function, operand),
        mir::Rvalue::Binary { left, right, .. } => {
            validate_operand(function, left)?;
            validate_operand(function, right)
        }
        mir::Rvalue::Call {
            function: callee,
            args,
        } => validate_int_call(program, function, *callee, args),
        mir::Rvalue::String(expression) => validate_string_expression(function, expression),
    }
}

fn validate_int_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::IntExpression,
) -> Result<(), BackendError> {
    match expression {
        mir::IntExpression::Use(operand) => validate_operand(function, operand),
        mir::IntExpression::Binary { left, right, .. } => {
            validate_int_expression(program, function, left)?;
            validate_int_expression(program, function, right)
        }
        mir::IntExpression::Call {
            function: callee,
            args,
        } => validate_int_call(program, function, *callee, args),
    }
}

fn validate_int_call(
    program: &mir::Program,
    caller: &mir::Function,
    callee: mir::FunctionId,
    args: &[mir::IntExpression],
) -> Result<(), BackendError> {
    let callee = function_in(program, callee)?;
    if callee.return_type != mir::ReturnType::Int {
        return Err(malformed_mir(format!(
            "int call targets void function {}",
            callee.name
        )));
    }
    validate_call_args(program, caller, callee, args)
}

fn validate_call_args(
    program: &mir::Program,
    caller: &mir::Function,
    callee: &mir::Function,
    args: &[mir::IntExpression],
) -> Result<(), BackendError> {
    if args.len() != callee.params.len() {
        return Err(malformed_mir(format!(
            "call to {} expects {} arguments, got {}",
            callee.name,
            callee.params.len(),
            args.len()
        )));
    }
    for argument in args {
        validate_int_expression(program, caller, argument)?;
    }
    Ok(())
}

fn validate_condition(
    program: &mir::Program,
    function: &mir::Function,
    condition: &mir::Condition,
) -> Result<(), BackendError> {
    match condition {
        mir::Condition::Bool(_) => Ok(()),
        mir::Condition::Compare { left, right, .. } => {
            validate_int_expression(program, function, left)?;
            validate_int_expression(program, function, right)
        }
        mir::Condition::Not(condition) => validate_condition(program, function, condition),
        mir::Condition::Binary { left, right, .. } => {
            validate_condition(program, function, left)?;
            validate_condition(program, function, right)
        }
    }
}

fn validate_operand(function: &mir::Function, operand: &mir::Operand) -> Result<(), BackendError> {
    if let mir::Operand::Local(local) = operand {
        let definition = local_in(function, *local)?;
        if definition.ty != mir::Type::Int {
            return Err(malformed_mir(format!(
                "string local local{} is used as an int operand",
                local.0
            )));
        }
    }
    Ok(())
}

fn validate_string_expression(
    function: &mir::Function,
    expression: &mir::StringExpression,
) -> Result<(), BackendError> {
    match expression {
        mir::StringExpression::Literal(_) => Ok(()),
        mir::StringExpression::Local(local) => {
            let definition = local_in(function, *local)?;
            if definition.ty != mir::Type::String {
                return Err(malformed_mir(format!(
                    "int local local{} is used as a string operand",
                    local.0
                )));
            }
            Ok(())
        }
        mir::StringExpression::Concat(parts) => {
            for part in parts {
                validate_string_expression(function, part)?;
            }
            Ok(())
        }
    }
}

fn validate_acyclic_calls(program: &mir::Program) -> Result<(), BackendError> {
    let mut edges = vec![Vec::new(); program.functions.len()];
    for function in &program.functions {
        collect_function_calls(function, &mut edges[function.id.0]);
    }
    let mut states = vec![0_u8; program.functions.len()];
    for function in 0..program.functions.len() {
        visit_call_graph(function, &edges, &mut states)?;
    }
    Ok(())
}

fn collect_function_calls(function: &mir::Function, calls: &mut Vec<mir::FunctionId>) {
    for block in &function.blocks {
        for statement in &block.statements {
            match statement {
                mir::Statement::AssignLocal { value, .. } => collect_rvalue_calls(value, calls),
                mir::Statement::CallVoid { function, args } => {
                    calls.push(*function);
                    for argument in args {
                        collect_int_expression_calls(argument, calls);
                    }
                }
                mir::Statement::EchoStringLiteral(_) | mir::Statement::EchoString(_) => {}
            }
        }
        if let mir::Terminator::Branch { condition, .. } = &block.terminator {
            collect_condition_calls(condition, calls);
        }
    }
}

fn collect_rvalue_calls(value: &mir::Rvalue, calls: &mut Vec<mir::FunctionId>) {
    if let mir::Rvalue::Call { function, args } = value {
        calls.push(*function);
        for argument in args {
            collect_int_expression_calls(argument, calls);
        }
    }
}

fn collect_int_expression_calls(expression: &mir::IntExpression, calls: &mut Vec<mir::FunctionId>) {
    match expression {
        mir::IntExpression::Use(_) => {}
        mir::IntExpression::Binary { left, right, .. } => {
            collect_int_expression_calls(left, calls);
            collect_int_expression_calls(right, calls);
        }
        mir::IntExpression::Call { function, args } => {
            calls.push(*function);
            for argument in args {
                collect_int_expression_calls(argument, calls);
            }
        }
    }
}

fn collect_condition_calls(condition: &mir::Condition, calls: &mut Vec<mir::FunctionId>) {
    match condition {
        mir::Condition::Bool(_) => {}
        mir::Condition::Compare { left, right, .. } => {
            collect_int_expression_calls(left, calls);
            collect_int_expression_calls(right, calls);
        }
        mir::Condition::Not(condition) => collect_condition_calls(condition, calls),
        mir::Condition::Binary { left, right, .. } => {
            collect_condition_calls(left, calls);
            collect_condition_calls(right, calls);
        }
    }
}

fn visit_call_graph(
    function: usize,
    edges: &[Vec<mir::FunctionId>],
    states: &mut [u8],
) -> Result<(), BackendError> {
    match states[function] {
        1 => return Err(malformed_mir("recursive MIR call graph reached Cranelift")),
        2 => return Ok(()),
        _ => {}
    }
    states[function] = 1;
    for callee in &edges[function] {
        visit_call_graph(callee.0, edges, states)?;
    }
    states[function] = 2;
    Ok(())
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

fn block_in(function: &mir::Function, id: mir::BlockId) -> Result<&mir::BasicBlock, BackendError> {
    function
        .blocks
        .get(id.0)
        .filter(|block| block.id == id)
        .ok_or_else(|| malformed_mir(format!("BlockId block{} does not exist", id.0)))
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

fn int_slot(slots: &[Option<StackSlot>], id: mir::LocalId) -> Result<StackSlot, BackendError> {
    slots
        .get(id.0)
        .copied()
        .flatten()
        .ok_or_else(|| malformed_mir(format!("LocalId local{} is not an int local", id.0)))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stdout_platform_selection_supports_unix_and_windows() {
        assert_eq!(stdout_platform(false, true), StdoutPlatform::Unix);
        assert_eq!(stdout_platform(true, false), StdoutPlatform::Windows);
        assert_eq!(stdout_platform(false, false), StdoutPlatform::Unsupported);
    }
}

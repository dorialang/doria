use std::{collections::HashSet, fmt};

use crate::mir;

const MAX_EXECUTED_BLOCKS: usize = 100_000;
const MAX_CALL_DEPTH: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterpreterOutput {
    pub stdout: Vec<u8>,
    pub exit_status: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterpreterError {
    pub message: String,
}

impl InterpreterError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for InterpreterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for InterpreterError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FunctionOutcome {
    Int(i64),
    Void,
}

struct Interpreter<'program> {
    program: &'program mir::Program,
    stdout: Vec<u8>,
    executed_blocks: usize,
}

pub fn interpret(program: &mir::Program) -> Result<InterpreterOutput, InterpreterError> {
    let entry = program
        .functions
        .get(program.entry.0)
        .ok_or_else(|| InterpreterError::new("MIR entry function does not exist"))?;
    if entry.id != program.entry {
        return Err(InterpreterError::new(format!(
            "MIR entry table points at function{}, but that slot contains function{}",
            program.entry.0, entry.id.0
        )));
    }
    if !entry.params.is_empty() {
        return Err(InterpreterError::new(
            "MIR entry function must not declare parameters",
        ));
    }

    let entry_return_type = entry.return_type;
    let mut interpreter = Interpreter {
        program,
        stdout: Vec::new(),
        executed_blocks: 0,
    };
    let outcome = interpreter.execute_function(program.entry, &[], 0)?;
    let exit_status = match (entry_return_type, outcome) {
        (mir::ReturnType::Int, FunctionOutcome::Int(value)) => validate_process_status(value)?,
        (mir::ReturnType::Void, FunctionOutcome::Void) => 0,
        (mir::ReturnType::Int, FunctionOutcome::Void) => {
            return Err(InterpreterError::new(
                "MIR int entry function returned void",
            ));
        }
        (mir::ReturnType::Void, FunctionOutcome::Int(_)) => {
            return Err(InterpreterError::new(
                "MIR void entry function returned an int value",
            ));
        }
    };

    Ok(InterpreterOutput {
        stdout: interpreter.stdout,
        exit_status,
    })
}

impl Interpreter<'_> {
    fn execute_function(
        &mut self,
        function_id: mir::FunctionId,
        args: &[i64],
        depth: usize,
    ) -> Result<FunctionOutcome, InterpreterError> {
        if depth >= MAX_CALL_DEPTH {
            return Err(InterpreterError::new(format!(
                "MIR interpreter exceeded its defensive call-depth limit of {MAX_CALL_DEPTH} frames"
            )));
        }

        let function = self
            .program
            .functions
            .get(function_id.0)
            .cloned()
            .ok_or_else(|| {
                InterpreterError::new(format!(
                    "MIR function function{} does not exist",
                    function_id.0
                ))
            })?;
        if function.id != function_id {
            return Err(InterpreterError::new(format!(
                "MIR function table entry {} contains function{}",
                function_id.0, function.id.0
            )));
        }
        if args.len() != function.params.len() {
            return Err(InterpreterError::new(format!(
                "MIR function {} expected {} argument(s), got {}",
                function.name,
                function.params.len(),
                args.len()
            )));
        }

        let mut locals = vec![None; function.locals.len()];
        for local in &function.locals {
            if local.id.0 >= locals.len() {
                return Err(InterpreterError::new(format!(
                    "MIR local local{} is outside the local slot table for function {}",
                    local.id.0, function.name
                )));
            }
        }
        for (parameter, value) in function.params.iter().zip(args.iter().copied()) {
            assign_local(&mut locals, *parameter, value)?;
        }

        let mut current_block = function.entry_block;
        let mut seen_states = HashSet::new();

        loop {
            if self.executed_blocks >= MAX_EXECUTED_BLOCKS {
                return Err(InterpreterError::new(format!(
                    "MIR interpreter exhausted its bounded execution fuel after {MAX_EXECUTED_BLOCKS} basic blocks"
                )));
            }
            self.executed_blocks += 1;

            if !seen_states.insert((current_block, locals.clone())) {
                return Err(InterpreterError::new(
                    "MIR interpreter detected a non-terminating control-flow cycle",
                ));
            }

            let block = function.blocks.get(current_block.0).ok_or_else(|| {
                InterpreterError::new(format!(
                    "MIR block block{} does not exist in function {}",
                    current_block.0, function.name
                ))
            })?;
            if block.id != current_block {
                return Err(InterpreterError::new(format!(
                    "MIR block table entry {} contains block{} in function {}",
                    current_block.0, block.id.0, function.name
                )));
            }

            for statement in &block.statements {
                match statement {
                    mir::Statement::AssignLocal { target, value } => {
                        let value = self.eval_rvalue(value, &locals, depth)?;
                        assign_local(&mut locals, *target, value)?;
                    }
                    mir::Statement::EchoStringLiteral(value) => {
                        self.stdout.extend_from_slice(value.as_bytes());
                    }
                    mir::Statement::CallVoid { function, args } => {
                        let args = self.eval_call_args(args, &locals, depth)?;
                        match self.execute_function(*function, &args, depth + 1)? {
                            FunctionOutcome::Void => {}
                            FunctionOutcome::Int(_) => {
                                return Err(InterpreterError::new(format!(
                                    "MIR void call to function{} returned an int value",
                                    function.0
                                )));
                            }
                        }
                    }
                }
            }

            match &block.terminator {
                mir::Terminator::Return(operand) => {
                    if function.return_type != mir::ReturnType::Int {
                        return Err(InterpreterError::new(format!(
                            "MIR void function {} returned an int value",
                            function.name
                        )));
                    }
                    return Ok(FunctionOutcome::Int(eval_operand(operand, &locals)?));
                }
                mir::Terminator::ReturnVoid => {
                    if function.return_type != mir::ReturnType::Void {
                        return Err(InterpreterError::new(format!(
                            "MIR int function {} returned void",
                            function.name
                        )));
                    }
                    return Ok(FunctionOutcome::Void);
                }
                mir::Terminator::Jump(target) => current_block = *target,
                mir::Terminator::Branch {
                    condition,
                    then_block,
                    else_block,
                } => {
                    current_block = if self.eval_condition(condition, &locals, depth)? {
                        *then_block
                    } else {
                        *else_block
                    };
                }
            }
        }
    }

    fn eval_rvalue(
        &mut self,
        value: &mir::Rvalue,
        locals: &[Option<i64>],
        depth: usize,
    ) -> Result<i64, InterpreterError> {
        match value {
            mir::Rvalue::Use(operand) => eval_operand(operand, locals),
            mir::Rvalue::Binary { op, left, right } => {
                let left = eval_operand(left, locals)?;
                let right = eval_operand(right, locals)?;
                eval_binary(*op, left, right)
            }
            mir::Rvalue::Call { function, args } => {
                self.eval_int_call(*function, args, locals, depth)
            }
        }
    }

    fn eval_int_expression(
        &mut self,
        expression: &mir::IntExpression,
        locals: &[Option<i64>],
        depth: usize,
    ) -> Result<i64, InterpreterError> {
        match expression {
            mir::IntExpression::Use(operand) => eval_operand(operand, locals),
            mir::IntExpression::Binary { op, left, right } => {
                let left = self.eval_int_expression(left, locals, depth)?;
                let right = self.eval_int_expression(right, locals, depth)?;
                eval_binary(*op, left, right)
            }
            mir::IntExpression::Call { function, args } => {
                self.eval_int_call(*function, args, locals, depth)
            }
        }
    }

    fn eval_int_call(
        &mut self,
        function: mir::FunctionId,
        args: &[mir::IntExpression],
        locals: &[Option<i64>],
        depth: usize,
    ) -> Result<i64, InterpreterError> {
        let args = self.eval_call_args(args, locals, depth)?;
        match self.execute_function(function, &args, depth + 1)? {
            FunctionOutcome::Int(value) => Ok(value),
            FunctionOutcome::Void => Err(InterpreterError::new(format!(
                "MIR int call to function{} returned void",
                function.0
            ))),
        }
    }

    fn eval_call_args(
        &mut self,
        args: &[mir::IntExpression],
        locals: &[Option<i64>],
        depth: usize,
    ) -> Result<Vec<i64>, InterpreterError> {
        args.iter()
            .map(|arg| self.eval_int_expression(arg, locals, depth))
            .collect()
    }

    fn eval_condition(
        &mut self,
        condition: &mir::Condition,
        locals: &[Option<i64>],
        depth: usize,
    ) -> Result<bool, InterpreterError> {
        match condition {
            mir::Condition::Bool(value) => Ok(*value),
            mir::Condition::Compare { op, left, right } => {
                let left = self.eval_int_expression(left, locals, depth)?;
                let right = self.eval_int_expression(right, locals, depth)?;
                Ok(eval_compare(*op, left, right))
            }
            mir::Condition::Not(condition) => Ok(!self.eval_condition(condition, locals, depth)?),
            mir::Condition::Binary { op, left, right } => match op {
                mir::ConditionBinaryOp::And => {
                    if !self.eval_condition(left, locals, depth)? {
                        Ok(false)
                    } else {
                        self.eval_condition(right, locals, depth)
                    }
                }
                mir::ConditionBinaryOp::Or => {
                    if self.eval_condition(left, locals, depth)? {
                        Ok(true)
                    } else {
                        self.eval_condition(right, locals, depth)
                    }
                }
                mir::ConditionBinaryOp::Xor => {
                    let left = self.eval_condition(left, locals, depth)?;
                    let right = self.eval_condition(right, locals, depth)?;
                    Ok(left ^ right)
                }
            },
        }
    }
}

pub fn render_debug_output(output: &InterpreterOutput) -> String {
    if output.stdout.is_empty() {
        format!("exit_status: {}\nstdout:\n", output.exit_status)
    } else {
        format!(
            "exit_status: {}\nstdout: {}\n",
            output.exit_status,
            String::from_utf8_lossy(&output.stdout)
        )
    }
}

fn eval_compare(op: mir::CompareOp, left: i64, right: i64) -> bool {
    match op {
        mir::CompareOp::Equal => left == right,
        mir::CompareOp::NotEqual => left != right,
        mir::CompareOp::Less => left < right,
        mir::CompareOp::LessEqual => left <= right,
        mir::CompareOp::Greater => left > right,
        mir::CompareOp::GreaterEqual => left >= right,
    }
}

fn eval_operand(operand: &mir::Operand, locals: &[Option<i64>]) -> Result<i64, InterpreterError> {
    match operand {
        mir::Operand::Int(value) => Ok(*value),
        mir::Operand::Local(id) => read_local(locals, *id),
    }
}

fn eval_binary(op: mir::BinaryOp, left: i64, right: i64) -> Result<i64, InterpreterError> {
    match op {
        mir::BinaryOp::Add => left.checked_add(right).ok_or_else(|| {
            InterpreterError::new("MIR interpreter integer overflow during addition")
        }),
        mir::BinaryOp::Subtract => left.checked_sub(right).ok_or_else(|| {
            InterpreterError::new("MIR interpreter integer overflow during subtraction")
        }),
        mir::BinaryOp::Multiply => left.checked_mul(right).ok_or_else(|| {
            InterpreterError::new("MIR interpreter integer overflow during multiplication")
        }),
    }
}

fn read_local(locals: &[Option<i64>], id: mir::LocalId) -> Result<i64, InterpreterError> {
    let slot = locals
        .get(id.0)
        .ok_or_else(|| InterpreterError::new(format!("MIR local local{} does not exist", id.0)))?;
    slot.ok_or_else(|| {
        InterpreterError::new(format!(
            "MIR local local{} was read before assignment",
            id.0
        ))
    })
}

fn assign_local(
    locals: &mut [Option<i64>],
    id: mir::LocalId,
    value: i64,
) -> Result<(), InterpreterError> {
    let slot = locals
        .get_mut(id.0)
        .ok_or_else(|| InterpreterError::new(format!("MIR local local{} does not exist", id.0)))?;
    *slot = Some(value);
    Ok(())
}

fn validate_process_status(value: i64) -> Result<i32, InterpreterError> {
    if (0..=125).contains(&value) {
        Ok(value as i32)
    } else {
        Err(InterpreterError::new(format!(
            "MIR interpreter process exit status must be in the range 0..125, got {value}"
        )))
    }
}

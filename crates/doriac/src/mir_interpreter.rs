use std::fmt;

use crate::mir;

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

pub fn interpret(program: &mir::Program) -> Result<InterpreterOutput, InterpreterError> {
    let function = program
        .functions
        .get(program.entry.0)
        .ok_or_else(|| InterpreterError::new("MIR entry function does not exist"))?;
    let block = function
        .blocks
        .get(function.entry_block.0)
        .ok_or_else(|| InterpreterError::new("MIR entry block does not exist"))?;

    let mut stdout = Vec::new();
    let mut locals = vec![None; function.locals.len()];

    for local in &function.locals {
        if local.id.0 >= locals.len() {
            return Err(InterpreterError::new(format!(
                "MIR local local{} is outside the local slot table",
                local.id.0
            )));
        }
    }

    for statement in &block.statements {
        match statement {
            mir::Statement::AssignLocal { target, value } => {
                let value = eval_rvalue(value, &locals)?;
                assign_local(&mut locals, *target, value)?;
            }
            mir::Statement::EchoStringLiteral(value) => stdout.extend_from_slice(value.as_bytes()),
        }
    }

    let exit_status = match &block.terminator {
        mir::Terminator::Return(operand) => {
            validate_process_status(eval_operand(operand, &locals)?)?
        }
        mir::Terminator::ReturnVoid => 0,
    };

    Ok(InterpreterOutput {
        stdout,
        exit_status,
    })
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

fn eval_rvalue(value: &mir::Rvalue, locals: &[Option<i64>]) -> Result<i64, InterpreterError> {
    match value {
        mir::Rvalue::Use(operand) => eval_operand(operand, locals),
        mir::Rvalue::Binary { op, left, right } => {
            let left = eval_operand(left, locals)?;
            let right = eval_operand(right, locals)?;
            eval_binary(*op, left, right)
        }
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

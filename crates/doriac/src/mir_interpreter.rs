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
    for statement in &block.statements {
        match statement {
            mir::Statement::EchoStringLiteral(value) => stdout.extend_from_slice(value.as_bytes()),
        }
    }

    let exit_status = match block.terminator {
        mir::Terminator::ReturnInt(value) => validate_process_status(value)?,
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

fn validate_process_status(value: i64) -> Result<i32, InterpreterError> {
    if (0..=125).contains(&value) {
        Ok(value as i32)
    } else {
        Err(InterpreterError::new(format!(
            "MIR interpreter process exit status must be in the range 0..125, got {value}"
        )))
    }
}

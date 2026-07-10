use std::fmt;

use crate::mir;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterpreterOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_status: i32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InterpreterLimits {
    pub max_executed_blocks: Option<usize>,
    pub max_call_frames: Option<usize>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum LocalValue {
    Int(i64),
    String(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EvaluationValue {
    Int(i64),
    Bool(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReturnExpectation {
    Int,
    Void,
}

#[derive(Debug, Clone)]
enum EvaluationTask {
    Int(mir::IntExpression),
    Binary(mir::BinaryOp),
    Condition(mir::Condition),
    Compare(mir::CompareOp),
    Not,
    AfterAnd(mir::Condition),
    AfterOr(mir::Condition),
    Xor,
    Invoke {
        function: mir::FunctionId,
        argument_count: usize,
        expectation: ReturnExpectation,
    },
    Assign(mir::LocalId),
    Branch {
        then_block: mir::BlockId,
        else_block: mir::BlockId,
    },
}

struct CallFrame {
    function: mir::FunctionId,
    block: mir::BlockId,
    statement_index: usize,
    entered_block: bool,
    locals: Vec<Option<LocalValue>>,
    tasks: Vec<EvaluationTask>,
    values: Vec<EvaluationValue>,
    caller_expectation: Option<ReturnExpectation>,
}

struct Interpreter<'program> {
    program: &'program mir::Program,
    stdout: Vec<u8>,
    frames: Vec<CallFrame>,
    limits: InterpreterLimits,
    executed_blocks: usize,
}

enum StepOutcome {
    Continue,
    EntryReturned(FunctionOutcome),
    Panic(String),
}

pub fn interpret(program: &mir::Program) -> Result<InterpreterOutput, InterpreterError> {
    interpret_internal(program, InterpreterLimits::default())
}

pub fn interpret_with_limits(
    program: &mir::Program,
    limits: InterpreterLimits,
) -> Result<InterpreterOutput, InterpreterError> {
    interpret_internal(program, limits)
}

fn interpret_internal(
    program: &mir::Program,
    limits: InterpreterLimits,
) -> Result<InterpreterOutput, InterpreterError> {
    let entry = function_in(program, program.entry)?;
    if !entry.params.is_empty() {
        return Err(InterpreterError::new(
            "MIR entry function must not declare parameters",
        ));
    }

    let mut interpreter = Interpreter {
        program,
        stdout: Vec::new(),
        frames: Vec::new(),
        limits,
        executed_blocks: 0,
    };
    interpreter.push_frame(program.entry, &[], None)?;

    loop {
        match interpreter.step()? {
            StepOutcome::Continue => {}
            StepOutcome::Panic(message) => return Ok(interpreter.panic_output(&message)),
            StepOutcome::EntryReturned(outcome) => {
                return interpreter.finish_entry(entry, outcome);
            }
        }
    }
}

impl Interpreter<'_> {
    fn step(&mut self) -> Result<StepOutcome, InterpreterError> {
        let task = self.frames.last_mut().and_then(|frame| frame.tasks.pop());
        if let Some(task) = task {
            return self.execute_task(task);
        }

        self.enter_current_block()?;
        let (function_id, block_id, statement_index) = {
            let frame = self.current_frame()?;
            (frame.function, frame.block, frame.statement_index)
        };
        let function = function_in(self.program, function_id)?;
        let block = block_in(function, block_id)?;

        if let Some(statement) = block.statements.get(statement_index).cloned() {
            self.current_frame_mut()?.statement_index += 1;
            return self.execute_statement(function, statement);
        }

        self.execute_terminator(function, block.terminator.clone())
    }

    fn enter_current_block(&mut self) -> Result<(), InterpreterError> {
        if self.current_frame()?.entered_block {
            return Ok(());
        }
        if let Some(limit) = self.limits.max_executed_blocks {
            if self.executed_blocks >= limit {
                return Err(InterpreterError::new(format!(
                    "MIR interpreter reached the explicit test execution limit of {limit} basic blocks"
                )));
            }
        }
        self.executed_blocks += 1;
        self.current_frame_mut()?.entered_block = true;
        Ok(())
    }

    fn execute_statement(
        &mut self,
        function: &mir::Function,
        statement: mir::Statement,
    ) -> Result<StepOutcome, InterpreterError> {
        match statement {
            mir::Statement::AssignLocal { target, value } => {
                let definition = local_in(function, target)?;
                match (definition.ty, value) {
                    (mir::Type::String, mir::Rvalue::String(expression)) => {
                        let value =
                            eval_string_expression(&expression, &self.current_frame()?.locals)?;
                        assign_local(
                            &function.locals,
                            &mut self.current_frame_mut()?.locals,
                            target,
                            LocalValue::String(value),
                        )?;
                    }
                    (mir::Type::String, _) => {
                        return Err(InterpreterError::new(format!(
                            "MIR string local local{} received a non-string value",
                            target.0
                        )));
                    }
                    (mir::Type::Int, value) => self.queue_int_assignment(target, value)?,
                }
            }
            mir::Statement::EchoStringLiteral(value) => {
                self.stdout.extend_from_slice(value.as_bytes());
            }
            mir::Statement::EchoString(expression) => {
                let value = eval_string_expression(&expression, &self.current_frame()?.locals)?;
                self.stdout.extend_from_slice(value.as_bytes());
            }
            mir::Statement::CallVoid { function, args } => {
                self.queue_call(function, args, ReturnExpectation::Void)?;
            }
        }
        Ok(StepOutcome::Continue)
    }

    fn execute_terminator(
        &mut self,
        function: &mir::Function,
        terminator: mir::Terminator,
    ) -> Result<StepOutcome, InterpreterError> {
        match terminator {
            mir::Terminator::Return(operand) => {
                if function.return_type != mir::ReturnType::Int {
                    return Err(InterpreterError::new(format!(
                        "MIR void function {} returned an int value",
                        function.name
                    )));
                }
                let value = eval_operand(&operand, &self.current_frame()?.locals)?;
                self.complete_frame(FunctionOutcome::Int(value))
            }
            mir::Terminator::ReturnVoid => {
                if function.return_type != mir::ReturnType::Void {
                    return Err(InterpreterError::new(format!(
                        "MIR int function {} returned void",
                        function.name
                    )));
                }
                self.complete_frame(FunctionOutcome::Void)
            }
            mir::Terminator::Panic(message) => {
                let message = eval_string_expression(&message, &self.current_frame()?.locals)?;
                Ok(StepOutcome::Panic(message))
            }
            mir::Terminator::Unreachable => Err(InterpreterError::new(format!(
                "MIR reached an unreachable block in function {}",
                function.name
            ))),
            mir::Terminator::Jump(target) => {
                self.move_to_block(function, target)?;
                Ok(StepOutcome::Continue)
            }
            mir::Terminator::Branch {
                condition,
                then_block,
                else_block,
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::Branch {
                    then_block,
                    else_block,
                });
                frame.tasks.push(EvaluationTask::Condition(condition));
                Ok(StepOutcome::Continue)
            }
        }
    }

    fn execute_task(&mut self, task: EvaluationTask) -> Result<StepOutcome, InterpreterError> {
        match task {
            EvaluationTask::Int(expression) => self.expand_int_expression(expression)?,
            EvaluationTask::Binary(op) => {
                let right = self.pop_int()?;
                let left = self.pop_int()?;
                let value = match eval_binary(op, left, right) {
                    Some(value) => value,
                    None => return Ok(StepOutcome::Panic(overflow_message(op).to_string())),
                };
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Int(value));
            }
            EvaluationTask::Condition(condition) => self.expand_condition(condition)?,
            EvaluationTask::Compare(op) => {
                let right = self.pop_int()?;
                let left = self.pop_int()?;
                let value = eval_compare(op, left, right);
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Bool(value));
            }
            EvaluationTask::Not => {
                let value = !self.pop_bool()?;
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Bool(value));
            }
            EvaluationTask::AfterAnd(right) => {
                if self.pop_bool()? {
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::Condition(right));
                } else {
                    self.current_frame_mut()?
                        .values
                        .push(EvaluationValue::Bool(false));
                }
            }
            EvaluationTask::AfterOr(right) => {
                if self.pop_bool()? {
                    self.current_frame_mut()?
                        .values
                        .push(EvaluationValue::Bool(true));
                } else {
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::Condition(right));
                }
            }
            EvaluationTask::Xor => {
                let right = self.pop_bool()?;
                let left = self.pop_bool()?;
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Bool(left ^ right));
            }
            EvaluationTask::Invoke {
                function,
                argument_count,
                expectation,
            } => {
                let args = self.take_call_arguments(argument_count)?;
                self.push_frame(function, &args, Some(expectation))?;
            }
            EvaluationTask::Assign(target) => {
                let value = self.pop_int()?;
                let function = function_in(self.program, self.current_frame()?.function)?;
                assign_local(
                    &function.locals,
                    &mut self.current_frame_mut()?.locals,
                    target,
                    LocalValue::Int(value),
                )?;
            }
            EvaluationTask::Branch {
                then_block,
                else_block,
            } => {
                let target = if self.pop_bool()? {
                    then_block
                } else {
                    else_block
                };
                let function = function_in(self.program, self.current_frame()?.function)?;
                self.move_to_block(function, target)?;
            }
        }
        Ok(StepOutcome::Continue)
    }

    fn expand_int_expression(
        &mut self,
        expression: mir::IntExpression,
    ) -> Result<(), InterpreterError> {
        match expression {
            mir::IntExpression::Use(operand) => {
                let value = eval_operand(&operand, &self.current_frame()?.locals)?;
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Int(value));
            }
            mir::IntExpression::Binary { op, left, right } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::Binary(op));
                frame.tasks.push(EvaluationTask::Int(*right));
                frame.tasks.push(EvaluationTask::Int(*left));
            }
            mir::IntExpression::Call { function, args } => {
                self.queue_call(function, args, ReturnExpectation::Int)?;
            }
        }
        Ok(())
    }

    fn expand_condition(&mut self, condition: mir::Condition) -> Result<(), InterpreterError> {
        match condition {
            mir::Condition::Bool(value) => self
                .current_frame_mut()?
                .values
                .push(EvaluationValue::Bool(value)),
            mir::Condition::Compare { op, left, right } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::Compare(op));
                frame.tasks.push(EvaluationTask::Int(right));
                frame.tasks.push(EvaluationTask::Int(left));
            }
            mir::Condition::Not(condition) => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::Not);
                frame.tasks.push(EvaluationTask::Condition(*condition));
            }
            mir::Condition::Binary {
                op: mir::ConditionBinaryOp::And,
                left,
                right,
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::AfterAnd(*right));
                frame.tasks.push(EvaluationTask::Condition(*left));
            }
            mir::Condition::Binary {
                op: mir::ConditionBinaryOp::Or,
                left,
                right,
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::AfterOr(*right));
                frame.tasks.push(EvaluationTask::Condition(*left));
            }
            mir::Condition::Binary {
                op: mir::ConditionBinaryOp::Xor,
                left,
                right,
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::Xor);
                frame.tasks.push(EvaluationTask::Condition(*right));
                frame.tasks.push(EvaluationTask::Condition(*left));
            }
        }
        Ok(())
    }

    fn queue_int_assignment(
        &mut self,
        target: mir::LocalId,
        value: mir::Rvalue,
    ) -> Result<(), InterpreterError> {
        self.current_frame_mut()?
            .tasks
            .push(EvaluationTask::Assign(target));
        match value {
            mir::Rvalue::Use(operand) => self
                .current_frame_mut()?
                .tasks
                .push(EvaluationTask::Int(mir::IntExpression::Use(operand))),
            mir::Rvalue::Binary { op, left, right } => self.current_frame_mut()?.tasks.push(
                EvaluationTask::Int(mir::IntExpression::Binary {
                    op,
                    left: Box::new(mir::IntExpression::Use(left)),
                    right: Box::new(mir::IntExpression::Use(right)),
                }),
            ),
            mir::Rvalue::Call { function, args } => {
                self.queue_call(function, args, ReturnExpectation::Int)?;
            }
            mir::Rvalue::String(_) => {
                return Err(InterpreterError::new(
                    "MIR string rvalue reached integer evaluation",
                ));
            }
        }
        Ok(())
    }

    fn queue_call(
        &mut self,
        function: mir::FunctionId,
        args: Vec<mir::IntExpression>,
        expectation: ReturnExpectation,
    ) -> Result<(), InterpreterError> {
        let frame = self.current_frame_mut()?;
        frame.tasks.push(EvaluationTask::Invoke {
            function,
            argument_count: args.len(),
            expectation,
        });
        for argument in args.into_iter().rev() {
            frame.tasks.push(EvaluationTask::Int(argument));
        }
        Ok(())
    }

    fn push_frame(
        &mut self,
        function_id: mir::FunctionId,
        args: &[i64],
        caller_expectation: Option<ReturnExpectation>,
    ) -> Result<(), InterpreterError> {
        if let Some(limit) = self.limits.max_call_frames {
            if self.frames.len() >= limit {
                return Err(InterpreterError::new(format!(
                    "MIR interpreter reached the explicit test call-frame limit of {limit}"
                )));
            }
        }

        let function = function_in(self.program, function_id)?;
        if args.len() != function.params.len() {
            return Err(InterpreterError::new(format!(
                "MIR function {} expected {} argument(s), got {}",
                function.name,
                function.params.len(),
                args.len()
            )));
        }
        let mut locals = vec![None; function.locals.len()];
        for (index, local) in function.locals.iter().enumerate() {
            if local.id != mir::LocalId(index) {
                return Err(InterpreterError::new(format!(
                    "MIR function {} local slot {index} contains local{}",
                    function.name, local.id.0
                )));
            }
        }
        for (parameter, value) in function.params.iter().zip(args.iter().copied()) {
            assign_local(
                &function.locals,
                &mut locals,
                *parameter,
                LocalValue::Int(value),
            )?;
        }
        block_in(function, function.entry_block)?;
        self.frames.push(CallFrame {
            function: function_id,
            block: function.entry_block,
            statement_index: 0,
            entered_block: false,
            locals,
            tasks: Vec::new(),
            values: Vec::new(),
            caller_expectation,
        });
        Ok(())
    }

    fn complete_frame(
        &mut self,
        outcome: FunctionOutcome,
    ) -> Result<StepOutcome, InterpreterError> {
        let frame = self
            .frames
            .pop()
            .ok_or_else(|| InterpreterError::new("MIR interpreter has no call frame to return"))?;
        let Some(expectation) = frame.caller_expectation else {
            return Ok(StepOutcome::EntryReturned(outcome));
        };
        match (expectation, outcome) {
            (ReturnExpectation::Int, FunctionOutcome::Int(value)) => self
                .current_frame_mut()?
                .values
                .push(EvaluationValue::Int(value)),
            (ReturnExpectation::Void, FunctionOutcome::Void) => {}
            (ReturnExpectation::Int, FunctionOutcome::Void) => {
                return Err(InterpreterError::new("MIR int call returned a void value"));
            }
            (ReturnExpectation::Void, FunctionOutcome::Int(_)) => {
                return Err(InterpreterError::new("MIR void call returned an int value"));
            }
        }
        Ok(StepOutcome::Continue)
    }

    fn move_to_block(
        &mut self,
        function: &mir::Function,
        target: mir::BlockId,
    ) -> Result<(), InterpreterError> {
        block_in(function, target)?;
        let frame = self.current_frame_mut()?;
        frame.block = target;
        frame.statement_index = 0;
        frame.entered_block = false;
        Ok(())
    }

    fn take_call_arguments(&mut self, count: usize) -> Result<Vec<i64>, InterpreterError> {
        let frame = self.current_frame_mut()?;
        if frame.values.len() < count {
            return Err(InterpreterError::new(
                "MIR call argument evaluation produced too few values",
            ));
        }
        let start = frame.values.len() - count;
        frame
            .values
            .drain(start..)
            .map(|value| match value {
                EvaluationValue::Int(value) => Ok(value),
                EvaluationValue::Bool(_) => Err(InterpreterError::new(
                    "MIR call argument evaluation produced a bool value",
                )),
            })
            .collect()
    }

    fn pop_int(&mut self) -> Result<i64, InterpreterError> {
        match self.current_frame_mut()?.values.pop() {
            Some(EvaluationValue::Int(value)) => Ok(value),
            Some(EvaluationValue::Bool(_)) => Err(InterpreterError::new(
                "MIR integer evaluation produced a bool value",
            )),
            None => Err(InterpreterError::new(
                "MIR integer evaluation produced no value",
            )),
        }
    }

    fn pop_bool(&mut self) -> Result<bool, InterpreterError> {
        match self.current_frame_mut()?.values.pop() {
            Some(EvaluationValue::Bool(value)) => Ok(value),
            Some(EvaluationValue::Int(_)) => Err(InterpreterError::new(
                "MIR condition evaluation produced an int value",
            )),
            None => Err(InterpreterError::new(
                "MIR condition evaluation produced no value",
            )),
        }
    }

    fn current_frame(&self) -> Result<&CallFrame, InterpreterError> {
        self.frames
            .last()
            .ok_or_else(|| InterpreterError::new("MIR interpreter has no active call frame"))
    }

    fn current_frame_mut(&mut self) -> Result<&mut CallFrame, InterpreterError> {
        self.frames
            .last_mut()
            .ok_or_else(|| InterpreterError::new("MIR interpreter has no active call frame"))
    }

    fn finish_entry(
        &self,
        entry: &mir::Function,
        outcome: FunctionOutcome,
    ) -> Result<InterpreterOutput, InterpreterError> {
        match (entry.return_type, outcome) {
            (mir::ReturnType::Int, FunctionOutcome::Int(value)) => {
                if (0..=125).contains(&value) {
                    Ok(InterpreterOutput {
                        stdout: self.stdout.clone(),
                        stderr: Vec::new(),
                        exit_status: value as i32,
                    })
                } else {
                    Ok(self.panic_output_with_trace(
                        "main returned process status outside 0..125",
                        &[entry.name.as_str()],
                    ))
                }
            }
            (mir::ReturnType::Void, FunctionOutcome::Void) => Ok(InterpreterOutput {
                stdout: self.stdout.clone(),
                stderr: Vec::new(),
                exit_status: 0,
            }),
            (mir::ReturnType::Int, FunctionOutcome::Void) => Err(InterpreterError::new(
                "MIR int entry function returned void",
            )),
            (mir::ReturnType::Void, FunctionOutcome::Int(_)) => Err(InterpreterError::new(
                "MIR void entry function returned an int value",
            )),
        }
    }

    fn panic_output(&self, message: &str) -> InterpreterOutput {
        let trace = self
            .frames
            .iter()
            .rev()
            .filter_map(|frame| {
                self.program
                    .functions
                    .get(frame.function.0)
                    .map(|function| function.name.as_str())
            })
            .collect::<Vec<_>>();
        self.panic_output_with_trace(message, &trace)
    }

    fn panic_output_with_trace(&self, message: &str, trace: &[&str]) -> InterpreterOutput {
        let mut stderr = Vec::new();
        stderr.extend_from_slice(b"panic: ");
        stderr.extend_from_slice(message.as_bytes());
        stderr.extend_from_slice(b"\nstack trace:\n");
        for function in trace {
            stderr.extend_from_slice(b"  at ");
            stderr.extend_from_slice(function.as_bytes());
            stderr.push(b'\n');
        }
        InterpreterOutput {
            stdout: self.stdout.clone(),
            stderr,
            exit_status: 101,
        }
    }
}

pub fn render_debug_output(output: &InterpreterOutput) -> String {
    let stdout = if output.stdout.is_empty() {
        "stdout:\n".to_string()
    } else {
        format!("stdout: {}\n", String::from_utf8_lossy(&output.stdout))
    };
    if output.stderr.is_empty() {
        format!("exit_status: {}\n{stdout}", output.exit_status)
    } else {
        format!(
            "exit_status: {}\n{stdout}stderr: {}",
            output.exit_status,
            String::from_utf8_lossy(&output.stderr)
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

fn eval_binary(op: mir::BinaryOp, left: i64, right: i64) -> Option<i64> {
    match op {
        mir::BinaryOp::Add => left.checked_add(right),
        mir::BinaryOp::Subtract => left.checked_sub(right),
        mir::BinaryOp::Multiply => left.checked_mul(right),
    }
}

fn overflow_message(op: mir::BinaryOp) -> &'static str {
    match op {
        mir::BinaryOp::Add => "integer overflow during addition",
        mir::BinaryOp::Subtract => "integer overflow during subtraction",
        mir::BinaryOp::Multiply => "integer overflow during multiplication",
    }
}

fn eval_operand(
    operand: &mir::Operand,
    locals: &[Option<LocalValue>],
) -> Result<i64, InterpreterError> {
    match operand {
        mir::Operand::Int(value) => Ok(*value),
        mir::Operand::Local(id) => match read_local(locals, *id)? {
            LocalValue::Int(value) => Ok(*value),
            LocalValue::String(_) => Err(InterpreterError::new(format!(
                "MIR string local local{} was used as an int value",
                id.0
            ))),
        },
    }
}

fn eval_string_expression(
    expression: &mir::StringExpression,
    locals: &[Option<LocalValue>],
) -> Result<String, InterpreterError> {
    match expression {
        mir::StringExpression::Literal(value) => Ok(value.clone()),
        mir::StringExpression::Local(id) => match read_local(locals, *id)? {
            LocalValue::String(value) => Ok(value.clone()),
            LocalValue::Int(_) => Err(InterpreterError::new(format!(
                "MIR int local local{} was used as a string value",
                id.0
            ))),
        },
        mir::StringExpression::Concat(parts) => {
            let mut value = String::new();
            for part in parts {
                value.push_str(&eval_string_expression(part, locals)?);
            }
            Ok(value)
        }
    }
}

fn read_local(
    locals: &[Option<LocalValue>],
    id: mir::LocalId,
) -> Result<&LocalValue, InterpreterError> {
    locals
        .get(id.0)
        .ok_or_else(|| InterpreterError::new(format!("MIR local local{} does not exist", id.0)))?
        .as_ref()
        .ok_or_else(|| {
            InterpreterError::new(format!(
                "MIR local local{} was read before assignment",
                id.0
            ))
        })
}

fn assign_local(
    definitions: &[mir::Local],
    locals: &mut [Option<LocalValue>],
    id: mir::LocalId,
    value: LocalValue,
) -> Result<(), InterpreterError> {
    let definition = definitions
        .get(id.0)
        .filter(|local| local.id == id)
        .ok_or_else(|| InterpreterError::new(format!("MIR local local{} does not exist", id.0)))?;
    let compatible = matches!(
        (definition.ty, &value),
        (mir::Type::Int, LocalValue::Int(_)) | (mir::Type::String, LocalValue::String(_))
    );
    if !compatible {
        let actual = match value {
            LocalValue::Int(_) => "int",
            LocalValue::String(_) => "string",
        };
        return Err(InterpreterError::new(format!(
            "MIR local local{} has type {}, but assignment produced {actual}",
            id.0, definition.ty
        )));
    }
    let slot = locals
        .get_mut(id.0)
        .ok_or_else(|| InterpreterError::new(format!("MIR local local{} does not exist", id.0)))?;
    *slot = Some(value);
    Ok(())
}

fn function_in(
    program: &mir::Program,
    id: mir::FunctionId,
) -> Result<&mir::Function, InterpreterError> {
    program
        .functions
        .get(id.0)
        .filter(|function| function.id == id)
        .ok_or_else(|| {
            InterpreterError::new(format!("MIR FunctionId function{} does not exist", id.0))
        })
}

fn local_in(function: &mir::Function, id: mir::LocalId) -> Result<&mir::Local, InterpreterError> {
    function
        .locals
        .get(id.0)
        .filter(|local| local.id == id)
        .ok_or_else(|| InterpreterError::new(format!("MIR LocalId local{} does not exist", id.0)))
}

fn block_in(
    function: &mir::Function,
    id: mir::BlockId,
) -> Result<&mir::BasicBlock, InterpreterError> {
    function
        .blocks
        .get(id.0)
        .filter(|block| block.id == id)
        .ok_or_else(|| InterpreterError::new(format!("MIR BlockId block{} does not exist", id.0)))
}

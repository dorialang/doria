use std::cell::{Ref, RefCell, RefMut};
use std::collections::BTreeMap;
use std::fmt;
use std::rc::Rc;

use crate::mir;
use crate::numeric::{FloatType, FloatValue, IntegerPanic, IntegerType, IntegerValue};

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

#[derive(Debug, Clone, PartialEq, Eq)]
enum FunctionOutcome {
    Value(LocalValue),
    Void,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LocalValue {
    Scalar(mir::ScalarValue),
    String(String),
    Mixed(MixedValue),
    NullableScalar {
        ty: mir::ScalarType,
        value: Option<mir::ScalarValue>,
    },
    NullableString(Option<String>),
    NullableMixed(Option<MixedValue>),
    Class {
        object: usize,
        class: crate::class_layout::ClassId,
    },
    NullableClass {
        object: Option<usize>,
        class: crate::class_layout::ClassId,
    },
    Collection(CollectionValue),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum EvaluationValue {
    Scalar(mir::ScalarValue),
    String(String),
    Mixed(MixedValue),
    NullableScalar {
        ty: mir::ScalarType,
        value: Option<mir::ScalarValue>,
    },
    NullableString(Option<String>),
    NullableMixed(Option<MixedValue>),
    Class {
        object: usize,
        class: crate::class_layout::ClassId,
    },
    NullableClass {
        object: Option<usize>,
        class: crate::class_layout::ClassId,
    },
    Collection(CollectionValue),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MixedValue {
    Scalar(mir::ScalarValue),
    String(String),
    Class {
        object: usize,
        class: crate::class_layout::ClassId,
    },
}

impl MixedValue {
    fn tag(&self) -> mir::MixedTag {
        match self {
            Self::Scalar(mir::ScalarValue::Bool(_)) => mir::MixedTag::Bool,
            Self::Scalar(mir::ScalarValue::Integer(value)) => mir::MixedTag::Integer(value.ty),
            Self::Scalar(mir::ScalarValue::Float(value)) => mir::MixedTag::Float(value.ty),
            Self::String(_) => mir::MixedTag::String,
            Self::Class { class, .. } => mir::MixedTag::Class(*class),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CollectionValue {
    ty: mir::CollectionTypeId,
    entries: SharedCollectionEntries,
}

type CollectionEntries = Vec<(Option<LocalValue>, LocalValue)>;
type SharedCollectionEntries = Rc<RefCell<CollectionEntries>>;

impl CollectionValue {
    fn new(ty: mir::CollectionTypeId, entries: CollectionEntries) -> Self {
        Self {
            ty,
            entries: Rc::new(RefCell::new(entries)),
        }
    }

    fn entries(&self) -> Ref<'_, CollectionEntries> {
        self.entries.borrow()
    }

    fn entries_mut(&self) -> RefMut<'_, CollectionEntries> {
        self.entries.borrow_mut()
    }
}

#[derive(Debug, Clone)]
struct ObjectValue {
    class: crate::class_layout::ClassId,
    properties: Vec<Option<LocalValue>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReturnExpectation {
    Value(mir::Type),
    Discard(mir::Type),
    Void,
}

#[derive(Debug, Clone)]
enum EvaluationTask {
    Rvalue(mir::Rvalue),
    Value(mir::ValueExpression),
    String(mir::StringExpression),
    Mixed(mir::MixedExpression),
    NullableScalar(mir::NullableScalarExpression),
    NullableString(mir::NullableStringExpression),
    NullableMixed(mir::NullableMixedExpression),
    Class(mir::ClassExpression),
    NullableClass(mir::NullableClassExpression),
    Collection(mir::CollectionExpression),
    BuildCollection {
        collection: mir::CollectionTypeId,
        keyed: Vec<bool>,
    },
    LoadCollectionValue {
        collection: mir::LocalId,
        transfer: bool,
    },
    CollectionAdd {
        collection: mir::LocalId,
        op: mir::CollectionMutationOp,
        has_index: bool,
    },
    CollectionSet(mir::LocalId),
    AssignCollectionIndex(mir::LocalId),
    CollectionHas {
        collection: mir::LocalId,
        op: mir::CollectionMembershipOp,
    },
    CollectionIsEmpty(mir::LocalId),
    CollectionLength(mir::LocalId),
    CollectionIndexScalar(mir::LocalId),
    CollectionKeyScalar(mir::LocalId),
    CollectionKeyString(mir::LocalId),
    DictionaryGet {
        collection: mir::LocalId,
        expected: mir::Type,
        access: mir::NullableCollectionAccess,
    },
    CollectionIndexClass {
        collection: mir::LocalId,
        class: crate::class_layout::ClassId,
        transfer: bool,
    },
    BuildClassNew {
        class: crate::class_layout::ClassId,
        properties: Vec<mir::PropertyValue>,
        constructor: Option<mir::FunctionId>,
        argument_count: usize,
        property_expression_count: usize,
        temporary_class_args: Vec<Option<crate::class_layout::ClassId>>,
        temporary_mixed_args: Vec<bool>,
    },
    FinishClassNew {
        object: usize,
        class: crate::class_layout::ClassId,
    },
    BuildNullableSome,
    BuildNullableScalarSome(mir::ScalarType),
    BuildNullableClassSome(crate::class_layout::ClassId),
    BuildMixedValue,
    BuildMixedString,
    BuildMixedClass,
    BuildNullableMixedSome,
    WrapNullable(mir::Type),
    AfterNullableMixedCoalesce {
        right: mir::NullableMixedExpression,
    },
    NullableScalarIsPresent,
    NullableClassIsPresent(Option<crate::class_layout::ClassId>),
    AfterIntegerCoalesce(mir::IntegerExpression),
    AfterFloatCoalesce(mir::FloatExpression),
    AfterBoolCoalesce(mir::BoolExpression),
    AfterStringCoalesce(mir::StringExpression),
    AfterNullableScalarCoalesce(mir::NullableScalarExpression),
    AfterNullableStringCoalesce(mir::NullableStringExpression),
    AfterClassCoalesce {
        right: mir::ClassExpression,
        left_owned: bool,
        transfer: bool,
    },
    FinishClassCoalesceRight(Option<crate::class_layout::ClassId>),
    AfterNullableClassCoalesce {
        right: mir::NullableClassExpression,
        left_owned: bool,
        transfer: bool,
    },
    FinishNullableClassCoalesceRight(Option<crate::class_layout::ClassId>),
    AfterNullSafeProperty {
        property: crate::class_layout::PropertyId,
        result: mir::Type,
        owned_receiver: Option<crate::class_layout::ClassId>,
    },
    AfterNullSafeCall {
        function: mir::FunctionId,
        args: Vec<mir::Rvalue>,
        result: mir::Type,
        owned_receiver: Option<crate::class_layout::ClassId>,
    },
    AfterNullSafeStatementCall {
        function: mir::FunctionId,
        args: Vec<mir::Rvalue>,
        owned_receiver: Option<crate::class_layout::ClassId>,
    },
    NullableStringCompare(mir::CompareOp),
    Format(mir::FormatExpression),
    BuildFormat(mir::FormatExpression),
    ReadFile,
    WriteFile,
    AppendFile,
    ReadFileBytes(mir::CollectionTypeId),
    WriteFileBytes {
        contents: mir::LocalId,
        append: bool,
    },
    WriteStreamBytes {
        contents: mir::LocalId,
        stderr: bool,
    },
    WriteStderr,
    StringConcat(usize),
    StringDisplay,
    StringCompare(mir::CompareOp),
    Echo,
    PanicString,
    Integer(mir::IntegerExpression),
    IntegerUnary(mir::IntegerUnaryOp),
    IntegerBinary(mir::IntegerBinaryOp),
    IntegerConvert(IntegerType),
    FloatToInt,
    Float(mir::FloatExpression),
    FloatNegate,
    FloatBinary(mir::FloatBinaryOp),
    IntToFloat,
    Bool(mir::BoolExpression),
    Compare(mir::CompareOp),
    Not,
    AfterAnd(mir::BoolExpression),
    AfterOr(mir::BoolExpression),
    Xor,
    Invoke {
        function: mir::FunctionId,
        argument_count: usize,
        expectation: ReturnExpectation,
        temporary_class_args: Vec<bool>,
        temporary_mixed_args: Vec<bool>,
    },
    FinishStatement,
    DropTemporaryClasses(Vec<(usize, crate::class_layout::ClassId)>),
    Assign(mir::LocalId),
    AssignStatic(mir::StaticId),
    AssignProperty {
        object: mir::LocalId,
        property: crate::class_layout::PropertyId,
    },
    DropClass(mir::LocalId),
    DropCollection(mir::LocalId),
    DropObject {
        object: usize,
        class: crate::class_layout::ClassId,
    },
    DropObjectProperties {
        object: usize,
        class: crate::class_layout::ClassId,
    },
    FreeObject {
        object: usize,
        class: crate::class_layout::ClassId,
    },
    CleanupFrame,
    ReturnValue(mir::Type),
    ReturnVoid,
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
    statement_temporary_drops: Vec<(usize, crate::class_layout::ClassId)>,
    caller_expectation: Option<ReturnExpectation>,
}

struct Interpreter<'program> {
    program: &'program mir::Program,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    stdin: Vec<u8>,
    stdin_cursor: usize,
    files: BTreeMap<String, Vec<u8>>,
    heap: BTreeMap<usize, ObjectValue>,
    statics: Vec<LocalValue>,
    next_object: usize,
    frames: Vec<CallFrame>,
    limits: InterpreterLimits,
    executed_blocks: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MirIo {
    pub stdin: Vec<u8>,
    pub files: BTreeMap<String, Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterpreterIoOutput {
    pub output: InterpreterOutput,
    pub files: BTreeMap<String, Vec<u8>>,
}

enum StepOutcome {
    Continue,
    EntryReturned(FunctionOutcome),
    Panic(String),
}

pub fn interpret(program: &mir::Program) -> Result<InterpreterOutput, InterpreterError> {
    Ok(interpret_with_io(program, MirIo::default())?.output)
}

pub fn interpret_with_io(
    program: &mir::Program,
    io: MirIo,
) -> Result<InterpreterIoOutput, InterpreterError> {
    interpret_internal(program, InterpreterLimits::default(), io)
}

pub fn interpret_with_limits(
    program: &mir::Program,
    limits: InterpreterLimits,
) -> Result<InterpreterOutput, InterpreterError> {
    Ok(interpret_internal(program, limits, MirIo::default())?.output)
}

fn interpret_internal(
    program: &mir::Program,
    limits: InterpreterLimits,
    io: MirIo,
) -> Result<InterpreterIoOutput, InterpreterError> {
    let entry = function_in(program, program.entry)?;
    if !entry.params.is_empty() {
        return Err(InterpreterError::new(
            "MIR entry function must not declare parameters",
        ));
    }

    let mut interpreter = Interpreter {
        program,
        stdout: Vec::new(),
        stderr: Vec::new(),
        stdin: io.stdin,
        stdin_cursor: 0,
        files: io.files,
        heap: BTreeMap::new(),
        statics: program
            .statics
            .iter()
            .map(|property| match &property.initializer {
                mir::StaticValue::Scalar(value)
                    if property.ty == mir::Type::NullableScalar(value.ty()) =>
                {
                    LocalValue::NullableScalar {
                        ty: value.ty(),
                        value: Some(*value),
                    }
                }
                mir::StaticValue::Scalar(value) => LocalValue::Scalar(*value),
                mir::StaticValue::String(value) if property.ty == mir::Type::String => {
                    LocalValue::String(value.clone())
                }
                mir::StaticValue::String(value) => LocalValue::NullableString(Some(value.clone())),
                mir::StaticValue::Null => match property.ty {
                    mir::Type::NullableScalar(ty) => LocalValue::NullableScalar { ty, value: None },
                    mir::Type::NullableString => LocalValue::NullableString(None),
                    mir::Type::NullableClass(class) => LocalValue::NullableClass {
                        object: None,
                        class,
                    },
                    _ => LocalValue::NullableString(None),
                },
            })
            .collect(),
        next_object: 1,
        frames: Vec::new(),
        limits,
        executed_blocks: 0,
    };
    interpreter.push_frame(program.entry, &[], None)?;

    loop {
        match interpreter.step()? {
            StepOutcome::Continue => {}
            StepOutcome::Panic(message) => {
                let output = interpreter.panic_output(&message);
                return Ok(InterpreterIoOutput {
                    output,
                    files: interpreter.files,
                });
            }
            StepOutcome::EntryReturned(outcome) => {
                let output = interpreter.finish_entry(entry, outcome)?;
                return Ok(InterpreterIoOutput {
                    output,
                    files: interpreter.files,
                });
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
            self.current_frame_mut()?
                .tasks
                .push(EvaluationTask::FinishStatement);
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
                        let frame = self.current_frame_mut()?;
                        frame.tasks.push(EvaluationTask::Assign(target));
                        frame.tasks.push(EvaluationTask::String(expression));
                    }
                    (mir::Type::String, _) => {
                        return Err(InterpreterError::new(format!(
                            "MIR string local local{} received a non-string value",
                            target.0
                        )));
                    }
                    (mir::Type::NullableString, mir::Rvalue::NullableString(expression)) => {
                        let frame = self.current_frame_mut()?;
                        frame.tasks.push(EvaluationTask::Assign(target));
                        frame.tasks.push(EvaluationTask::NullableString(expression));
                    }
                    (mir::Type::NullableString, _) => {
                        return Err(InterpreterError::new(format!(
                            "MIR nullable-string local local{} received another value type",
                            target.0
                        )));
                    }
                    (mir::Type::Mixed, mir::Rvalue::Mixed(expression)) => {
                        let frame = self.current_frame_mut()?;
                        frame.tasks.push(EvaluationTask::Assign(target));
                        frame.tasks.push(EvaluationTask::Mixed(expression));
                    }
                    (mir::Type::Mixed, _) => {
                        return Err(InterpreterError::new(format!(
                            "MIR mixed local local{} received another value type",
                            target.0
                        )));
                    }
                    (mir::Type::NullableMixed, mir::Rvalue::NullableMixed(expression)) => {
                        let frame = self.current_frame_mut()?;
                        frame.tasks.push(EvaluationTask::Assign(target));
                        frame.tasks.push(EvaluationTask::NullableMixed(expression));
                    }
                    (mir::Type::NullableMixed, _) => {
                        return Err(InterpreterError::new(format!(
                            "MIR nullable-mixed local local{} received another value type",
                            target.0
                        )));
                    }
                    (
                        mir::Type::NullableScalar(expected),
                        mir::Rvalue::NullableScalar(expression),
                    ) if expression.ty() == expected => {
                        let frame = self.current_frame_mut()?;
                        frame.tasks.push(EvaluationTask::Assign(target));
                        frame.tasks.push(EvaluationTask::NullableScalar(expression));
                    }
                    (mir::Type::NullableScalar(_), _) => {
                        return Err(InterpreterError::new(format!(
                            "MIR nullable scalar local local{} received another value type",
                            target.0
                        )));
                    }
                    (mir::Type::Scalar(expected), mir::Rvalue::Value(expression)) => {
                        if expression.ty() != expected {
                            return Err(InterpreterError::new(format!(
                                "MIR scalar local local{} has type {expected}, but its rvalue has type {}",
                                target.0,
                                expression.ty()
                            )));
                        }
                        self.queue_value_assignment(target, expression)?;
                    }
                    (mir::Type::Scalar(_), _) => {
                        return Err(InterpreterError::new(format!(
                            "MIR scalar local local{} received a string value",
                            target.0
                        )));
                    }
                    (mir::Type::Class(expected), mir::Rvalue::Class(expression))
                        if expression.class() == expected =>
                    {
                        let frame = self.current_frame_mut()?;
                        frame.tasks.push(EvaluationTask::Assign(target));
                        frame.tasks.push(EvaluationTask::Class(expression));
                    }
                    (
                        mir::Type::NullableClass(expected),
                        mir::Rvalue::NullableClass(expression),
                    ) if expression.class() == expected => {
                        let frame = self.current_frame_mut()?;
                        frame.tasks.push(EvaluationTask::Assign(target));
                        frame.tasks.push(EvaluationTask::NullableClass(expression));
                    }
                    (mir::Type::NullableClass(_), _) => {
                        return Err(InterpreterError::new(format!(
                            "MIR nullable class local local{} received another value type",
                            target.0
                        )));
                    }
                    (mir::Type::Class(_), _) => {
                        return Err(InterpreterError::new(
                            "MIR class local received a non-class value",
                        ));
                    }
                    (mir::Type::Collection(expected), mir::Rvalue::Collection(expression))
                        if expression.collection() == expected =>
                    {
                        let frame = self.current_frame_mut()?;
                        frame.tasks.push(EvaluationTask::Assign(target));
                        frame.tasks.push(EvaluationTask::Collection(expression));
                    }
                    (mir::Type::Collection(_), _) => {
                        return Err(InterpreterError::new(
                            "MIR collection local received a non-collection value",
                        ));
                    }
                }
            }
            mir::Statement::EchoStringLiteral(value) => {
                self.stdout.extend_from_slice(value.as_bytes());
            }
            mir::Statement::EchoString(expression) => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::Echo);
                frame.tasks.push(EvaluationTask::String(expression));
            }
            mir::Statement::CallVoid { function, args } => {
                self.queue_call(function, args, ReturnExpectation::Void)?;
            }
            mir::Statement::CallBorrowed { function, args } => {
                let callee = function_in(self.program, function)?;
                let mir::ReturnType::Value(return_type) = callee.return_type else {
                    return Err(InterpreterError::new(
                        "MIR borrowed call targeted a void function",
                    ));
                };
                self.queue_call(function, args, ReturnExpectation::Discard(return_type))?;
            }
            mir::Statement::CallNullSafe {
                object,
                function,
                args,
            } => {
                let owned_receiver = object.owned_temporary_class();
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::AfterNullSafeStatementCall {
                        function,
                        args,
                        owned_receiver,
                    });
                frame.tasks.push(EvaluationTask::NullableClass(object));
            }
            mir::Statement::Printf(format) => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::Echo);
                frame.tasks.push(EvaluationTask::Format(format));
            }
            mir::Statement::WriteFile { path, contents } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::WriteFile);
                frame.tasks.push(EvaluationTask::String(contents));
                frame.tasks.push(EvaluationTask::String(path));
            }
            mir::Statement::AppendFile { path, contents } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::AppendFile);
                frame.tasks.push(EvaluationTask::String(contents));
                frame.tasks.push(EvaluationTask::String(path));
            }
            mir::Statement::WriteFileBytes {
                path,
                contents,
                append,
            } => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::WriteFileBytes { contents, append });
                frame.tasks.push(EvaluationTask::String(path));
            }
            mir::Statement::WriteStreamBytes { contents, stderr } => {
                self.current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::WriteStreamBytes { contents, stderr });
            }
            mir::Statement::WriteStderr(value) => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::WriteStderr);
                frame.tasks.push(EvaluationTask::String(value));
            }
            mir::Statement::AssignProperty {
                object,
                property,
                value,
            } => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::AssignProperty { object, property });
                frame.tasks.push(EvaluationTask::Rvalue(value));
            }
            mir::Statement::AssignStatic { target, value } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::AssignStatic(target));
                frame.tasks.push(EvaluationTask::Rvalue(value));
            }
            mir::Statement::DropClass { local, .. } => {
                self.current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::DropClass(local));
            }
            mir::Statement::DropString { local } => {
                let value = self
                    .current_frame_mut()?
                    .locals
                    .get_mut(local.0)
                    .and_then(Option::take)
                    .ok_or_else(|| {
                        InterpreterError::new("string temporary was dropped before initialization")
                    })?;
                if !matches!(value, LocalValue::String(_)) {
                    return Err(InterpreterError::new(
                        "string drop references a non-string local",
                    ));
                }
            }
            mir::Statement::DropMixed { local } => {
                self.drop_mixed_local(local)?;
            }
            mir::Statement::CollectionAdd {
                collection,
                value,
                index,
                op,
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::CollectionAdd {
                    collection,
                    op,
                    has_index: index.is_some(),
                });
                frame.tasks.push(EvaluationTask::Rvalue(value));
                if let Some(index) = index {
                    frame.tasks.push(EvaluationTask::Rvalue(index));
                }
            }
            mir::Statement::CollectionSet {
                collection,
                key,
                value,
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::CollectionSet(collection));
                frame.tasks.push(EvaluationTask::Rvalue(value));
                frame.tasks.push(EvaluationTask::Rvalue(key));
            }
            mir::Statement::AssignCollectionIndex {
                collection,
                index,
                value,
            } => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::AssignCollectionIndex(collection));
                frame.tasks.push(EvaluationTask::Rvalue(value));
                frame.tasks.push(EvaluationTask::Rvalue(index));
            }
            mir::Statement::DropCollection { local, .. } => {
                self.current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::DropCollection(local));
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
                let mir::ReturnType::Value(expected) = function.return_type else {
                    return Err(InterpreterError::new(format!(
                        "MIR void function {} returned a scalar value",
                        function.name
                    )));
                };
                if operand.ty() != expected {
                    return Err(InterpreterError::new(format!(
                        "MIR function {} returns {expected}, but its return expression has type {}",
                        function.name,
                        operand.ty()
                    )));
                }
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::ReturnValue(expected));
                frame.tasks.push(EvaluationTask::CleanupFrame);
                frame.tasks.push(EvaluationTask::FinishStatement);
                frame.tasks.push(EvaluationTask::Rvalue(operand));
                Ok(StepOutcome::Continue)
            }
            mir::Terminator::ReturnVoid => {
                if function.return_type != mir::ReturnType::Void {
                    return Err(InterpreterError::new(format!(
                        "MIR int function {} returned void",
                        function.name
                    )));
                }
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::ReturnVoid);
                frame.tasks.push(EvaluationTask::CleanupFrame);
                Ok(StepOutcome::Continue)
            }
            mir::Terminator::Panic(message) => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::PanicString);
                frame.tasks.push(EvaluationTask::String(message));
                Ok(StepOutcome::Continue)
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
                frame.tasks.push(EvaluationTask::FinishStatement);
                frame.tasks.push(EvaluationTask::Bool(condition));
                Ok(StepOutcome::Continue)
            }
        }
    }

    fn execute_task(&mut self, task: EvaluationTask) -> Result<StepOutcome, InterpreterError> {
        match task {
            EvaluationTask::Rvalue(expression) => match expression {
                mir::Rvalue::Value(value) => self
                    .current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::Value(value)),
                mir::Rvalue::String(value) => self
                    .current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::String(value)),
                mir::Rvalue::Mixed(value) => self
                    .current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::Mixed(value)),
                mir::Rvalue::NullableScalar(value) => self
                    .current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::NullableScalar(value)),
                mir::Rvalue::NullableString(value) => self
                    .current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::NullableString(value)),
                mir::Rvalue::NullableMixed(value) => self
                    .current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::NullableMixed(value)),
                mir::Rvalue::Class(value) => self
                    .current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::Class(value)),
                mir::Rvalue::NullableClass(value) => self
                    .current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::NullableClass(value)),
                mir::Rvalue::Collection(value) => self
                    .current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::Collection(value)),
            },
            EvaluationTask::Value(expression) => match expression {
                mir::ValueExpression::Integer(value) => {
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::Integer(value));
                }
                mir::ValueExpression::Float(value) => {
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::Float(value));
                }
                mir::ValueExpression::Bool(value) => {
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::Bool(value));
                }
            },
            EvaluationTask::String(expression) => self.expand_string_expression(expression)?,
            EvaluationTask::Mixed(expression) => self.expand_mixed_expression(expression)?,
            EvaluationTask::NullableScalar(expression) => {
                self.expand_nullable_scalar_expression(expression)?
            }
            EvaluationTask::NullableString(expression) => {
                self.expand_nullable_string_expression(expression)?
            }
            EvaluationTask::NullableMixed(expression) => {
                self.expand_nullable_mixed_expression(expression)?
            }
            EvaluationTask::Class(expression) => self.expand_class_expression(expression)?,
            EvaluationTask::NullableClass(expression) => {
                self.expand_nullable_class_expression(expression)?
            }
            EvaluationTask::Collection(expression) => {
                self.expand_collection_expression(expression)?
            }
            EvaluationTask::BuildCollection { collection, keyed } => {
                let definition = self
                    .program
                    .collection_types
                    .get(collection.0)
                    .ok_or_else(|| InterpreterError::new("collection type does not exist"))?;
                let key_type = definition.key;
                let value_type = definition.value;
                let value_count = keyed.len() + keyed.iter().filter(|keyed| **keyed).count();
                let values = self.take_call_arguments(value_count)?;
                let mut values = values.into_iter();
                let mut entries: Vec<(Option<LocalValue>, LocalValue)> =
                    Vec::with_capacity(keyed.len());
                let unique =
                    self.program.collection_types[collection.0].kind == mir::CollectionKind::Set;
                let mut drops = Vec::new();
                for keyed in keyed {
                    let key = keyed.then(|| values.next()).flatten();
                    let value = values.next().ok_or_else(|| {
                        InterpreterError::new("MIR collection literal produced too few values")
                    })?;
                    if unique
                        && entries.iter().any(|(_, current)| {
                            collection_values_equal(value_type, current, &value)
                        })
                    {
                        if let Some(key) = key {
                            collect_owned_objects_from_value(key, &mut drops);
                        }
                        collect_owned_objects_from_value(value, &mut drops);
                    } else if let Some(position) = key.as_ref().and_then(|key| {
                        entries.iter().position(|(current, _)| {
                            current.as_ref().is_some_and(|current| {
                                key_type.is_some_and(|ty| collection_values_equal(ty, current, key))
                            })
                        })
                    }) {
                        let (old_key, old_value) =
                            std::mem::replace(&mut entries[position], (key, value));
                        if let Some(old_key) = old_key {
                            collect_owned_objects_from_value(old_key, &mut drops);
                        }
                        collect_owned_objects_from_value(old_value, &mut drops);
                    } else {
                        entries.push((key, value));
                    }
                }
                for (object, class) in drops {
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::DropObject { object, class });
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Collection(CollectionValue::new(
                        collection, entries,
                    )));
            }
            EvaluationTask::LoadCollectionValue {
                collection,
                transfer,
            } => {
                let index = self.pop_local_value()?;
                let value = match self.collection_value_at(collection, &index, transfer) {
                    Ok(value) => value,
                    Err(message) => return Ok(StepOutcome::Panic(message)),
                };
                self.push_local_value(value)?;
            }
            EvaluationTask::CollectionAdd {
                collection,
                op,
                has_index,
            } => {
                let value = self.pop_local_value()?;
                let index = has_index
                    .then(|| self.pop_collection_offset())
                    .transpose()?;
                let collection_kind = {
                    let collection = self.collection_local(collection)?;
                    self.program.collection_types[collection.ty.0].kind
                };
                match op {
                    mir::CollectionMutationOp::Add => {
                        let collection = self.collection_local(collection)?;
                        if collection
                            .entries()
                            .iter()
                            .any(|(_, current)| current == &value)
                            && collection_kind == mir::CollectionKind::Set
                        {
                            self.queue_value_drops(value)?;
                            return Ok(StepOutcome::Continue);
                        }
                        collection.entries_mut().push((None, value));
                    }
                    mir::CollectionMutationOp::InsertAt => {
                        let index = index.expect("insertAt task carries an index");
                        let collection = self.collection_local(collection)?;
                        if index > collection.entries().len() {
                            return Ok(StepOutcome::Panic(
                                "collection index out of bounds".to_string(),
                            ));
                        }
                        collection.entries_mut().insert(index, (None, value));
                    }
                    mir::CollectionMutationOp::Remove => {
                        let position = self
                            .collection_local(collection)?
                            .entries()
                            .iter()
                            .position(|(_, current)| current == &value);
                        if let Some(position) = position {
                            let (_, removed) = self
                                .collection_local(collection)?
                                .entries_mut()
                                .remove(position);
                            self.queue_value_drops(removed)?;
                        }
                        self.queue_value_drops(value)?;
                    }
                }
            }
            EvaluationTask::CollectionSet(collection) => {
                let value = self.pop_local_value()?;
                let key = self.pop_local_value()?;
                let position = self
                    .collection_local(collection)?
                    .entries()
                    .iter()
                    .position(|(current, _)| current.as_ref() == Some(&key));
                if let Some(position) = position {
                    let current = self.collection_local(collection)?;
                    let old = std::mem::replace(&mut current.entries_mut()[position].1, value);
                    self.queue_value_drops(old)?;
                    self.queue_value_drops(key)?;
                } else {
                    self.collection_local(collection)?
                        .entries_mut()
                        .push((Some(key), value));
                }
            }
            EvaluationTask::AssignCollectionIndex(collection) => {
                let value = self.pop_local_value()?;
                let index = self.pop_local_value()?;
                let keyed = {
                    let collection = self.collection_local(collection)?;
                    self.program.collection_types[collection.ty.0].key.is_some()
                };
                match self.collection_position(collection, &index) {
                    Ok(position) => {
                        let current = self.collection_local(collection)?;
                        let old = std::mem::replace(&mut current.entries_mut()[position].1, value);
                        self.queue_value_drops(old)?;
                    }
                    Err(_) if keyed => {
                        self.collection_local(collection)?
                            .entries_mut()
                            .push((Some(index), value));
                    }
                    Err(message) => return Ok(StepOutcome::Panic(message)),
                }
            }
            EvaluationTask::CollectionHas { collection, op } => {
                let needle = self.pop_local_value()?;
                let (found, needle_type) = {
                    let collection = self.collection_local(collection)?;
                    let definition = &self.program.collection_types[collection.ty.0];
                    let needle_type = definition.key.unwrap_or(definition.value);
                    if definition.key.is_some() {
                        (
                            collection.entries().iter().any(|(key, _)| {
                                key.as_ref().is_some_and(|key| {
                                    collection_values_equal(needle_type, key, &needle)
                                })
                            }),
                            needle_type,
                        )
                    } else {
                        (
                            collection.entries().iter().any(|(_, value)| {
                                collection_values_equal(needle_type, value, &needle)
                            }),
                            needle_type,
                        )
                    }
                };
                if op == mir::CollectionMembershipOp::Add {
                    if found {
                        self.queue_value_drops(needle)?;
                    } else {
                        self.collection_local(collection)?
                            .entries_mut()
                            .push((None, needle));
                    }
                    self.push_scalar(mir::ScalarValue::Bool(!found))?;
                    return Ok(StepOutcome::Continue);
                }
                let result = match op {
                    mir::CollectionMembershipOp::Contains => found,
                    mir::CollectionMembershipOp::Remove => {
                        let position = {
                            let collection = self.collection_local(collection)?;
                            collection.entries().iter().position(|(_, value)| {
                                collection_values_equal(needle_type, value, &needle)
                            })
                        };
                        if let Some(position) = position {
                            let (_, removed) = self
                                .collection_local(collection)?
                                .entries_mut()
                                .remove(position);
                            self.queue_value_drops(removed)?;
                            true
                        } else {
                            false
                        }
                    }
                    mir::CollectionMembershipOp::Add => unreachable!("handled above"),
                };
                self.queue_value_drops(needle)?;
                self.push_scalar(mir::ScalarValue::Bool(result))?;
            }
            EvaluationTask::CollectionIsEmpty(collection) => {
                let empty = self.collection_local(collection)?.entries().is_empty();
                self.push_scalar(mir::ScalarValue::Bool(empty))?;
            }
            EvaluationTask::CollectionLength(collection) => {
                let length = self.collection_local(collection)?.entries().len();
                self.push_scalar(mir::ScalarValue::Integer(
                    IntegerValue::from_i128(IntegerType::Int64, length as i128)
                        .expect("collection length fits interpreter address space"),
                ))?;
            }
            EvaluationTask::CollectionIndexScalar(collection) => {
                let index = self.pop_local_value()?;
                let value = match self.collection_value_at(collection, &index, false) {
                    Ok(value) => value,
                    Err(message) => return Ok(StepOutcome::Panic(message)),
                };
                let LocalValue::Scalar(value) = value else {
                    return Err(InterpreterError::new(
                        "MIR indexed scalar produced another value type",
                    ));
                };
                self.push_scalar(value)?;
            }
            EvaluationTask::CollectionKeyScalar(collection) => {
                let offset = self.pop_collection_offset()?;
                let value = self.collection_key_at(collection, offset)?;
                let LocalValue::Scalar(value) = value else {
                    return Err(InterpreterError::new(
                        "MIR collection key produced another value type",
                    ));
                };
                self.push_scalar(value)?;
            }
            EvaluationTask::CollectionKeyString(collection) => {
                let offset = self.pop_collection_offset()?;
                let value = self.collection_key_at(collection, offset)?;
                let LocalValue::String(value) = value else {
                    return Err(InterpreterError::new(
                        "MIR collection key produced another value type",
                    ));
                };
                self.push_string(value)?;
            }
            EvaluationTask::DictionaryGet {
                collection,
                expected,
                access,
            } => {
                let key = self.pop_local_value()?;
                let value = match access {
                    mir::NullableCollectionAccess::Get => self
                        .collection_local(collection)?
                        .entries()
                        .iter()
                        .find(|(current, _)| current.as_ref() == Some(&key))
                        .map(|(_, value)| value.clone()),
                    mir::NullableCollectionAccess::Remove => {
                        let position = self
                            .collection_local(collection)?
                            .entries()
                            .iter()
                            .position(|(current, _)| current.as_ref() == Some(&key));
                        if let Some(position) = position {
                            let (removed_key, value) = self
                                .collection_local(collection)?
                                .entries_mut()
                                .remove(position);
                            if let Some(removed_key) = removed_key {
                                self.queue_value_drops(removed_key)?;
                            }
                            Some(value)
                        } else {
                            None
                        }
                    }
                    mir::NullableCollectionAccess::First => self
                        .collection_local(collection)?
                        .entries()
                        .first()
                        .map(|(_, value)| value.clone()),
                    mir::NullableCollectionAccess::Last => self
                        .collection_local(collection)?
                        .entries()
                        .last()
                        .map(|(_, value)| value.clone()),
                    mir::NullableCollectionAccess::Pop => self
                        .collection_local(collection)?
                        .entries_mut()
                        .pop()
                        .map(|(_, value)| value),
                };
                match (expected, value) {
                    (mir::Type::Scalar(ty), Some(LocalValue::Scalar(value)))
                        if value.ty() == ty =>
                    {
                        self.push_nullable_scalar(ty, Some(value))?;
                    }
                    (mir::Type::Scalar(ty), None) => {
                        self.push_nullable_scalar(ty, None)?;
                    }
                    (mir::Type::String, Some(LocalValue::String(value))) => {
                        self.push_nullable_string(Some(value))?;
                    }
                    (mir::Type::String, None) => {
                        self.push_nullable_string(None)?;
                    }
                    (
                        mir::Type::Class(class),
                        Some(LocalValue::Class {
                            object,
                            class: actual,
                        }),
                    ) if class == actual => {
                        self.push_nullable_class(class, Some(object))?;
                    }
                    (mir::Type::Class(class), None) => {
                        self.push_nullable_class(class, None)?;
                    }
                    _ => {
                        return Err(InterpreterError::new(
                            "Dictionary::get produced another value type",
                        ))
                    }
                }
            }
            EvaluationTask::CollectionIndexClass {
                collection,
                class,
                transfer,
            } => {
                let index = self.pop_local_value()?;
                let value = match self.collection_value_at(collection, &index, transfer) {
                    Ok(value) => value,
                    Err(message) => return Ok(StepOutcome::Panic(message)),
                };
                let LocalValue::Class {
                    object,
                    class: actual,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "MIR indexed class produced another value type",
                    ));
                };
                if actual != class {
                    return Err(InterpreterError::new(
                        "MIR indexed class has another class type",
                    ));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Class { object, class });
            }
            EvaluationTask::BuildClassNew {
                class,
                properties,
                constructor,
                argument_count,
                property_expression_count,
                temporary_class_args,
                temporary_mixed_args,
            } => {
                let arguments = self.take_call_arguments(argument_count)?;
                let property_expressions = self.take_call_arguments(property_expression_count)?;
                let object_id = self.next_object;
                self.next_object += 1;
                let class_definition = self.program.classes.get(class.0).ok_or_else(|| {
                    InterpreterError::new(format!("MIR class#{} does not exist", class.0))
                })?;
                let mut slots = vec![None; class_definition.properties.len()];
                let mut expression_values = property_expressions.into_iter();
                for property in &properties {
                    let value = match &property.source {
                        mir::PropertyValueSource::Expression(_) => {
                            expression_values.next().ok_or_else(|| {
                                InterpreterError::new(
                                    "MIR class construction produced too few property values",
                                )
                            })?
                        }
                        mir::PropertyValueSource::ConstructorArgument(index) => {
                            arguments.get(*index).cloned().ok_or_else(|| {
                                InterpreterError::new(format!(
                                    "MIR constructor argument {index} does not exist"
                                ))
                            })?
                        }
                        mir::PropertyValueSource::ConstructorBody => continue,
                    };
                    let slot = slots.get_mut(property.property.index).ok_or_else(|| {
                        InterpreterError::new(format!(
                            "MIR property{} does not exist",
                            property.property.index
                        ))
                    })?;
                    *slot = Some(value);
                }
                self.heap.insert(
                    object_id,
                    ObjectValue {
                        class,
                        properties: slots,
                    },
                );
                if let Some(constructor) = constructor {
                    let constructor_definition = function_in(self.program, constructor)?;
                    let mut temporary_drops = Vec::new();
                    for (index, temporary_class) in temporary_class_args.iter().enumerate() {
                        let Some(class) = temporary_class else {
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
                                    InterpreterError::new(format!(
                                        "MIR constructor function{} is missing parameter {index}",
                                        constructor.0
                                    ))
                                })?;
                        if promoted || local_in(constructor_definition, parameter)?.owned {
                            continue;
                        }
                        let LocalValue::Class {
                            object,
                            class: actual,
                        } = &arguments[index]
                        else {
                            return Err(InterpreterError::new(
                                "MIR temporary constructor argument produced another value type",
                            ));
                        };
                        if actual != class {
                            return Err(InterpreterError::new(
                                "MIR temporary constructor argument produced the wrong class",
                            ));
                        }
                        temporary_drops.push((*object, *class));
                    }
                    for (index, temporary_mixed) in temporary_mixed_args.iter().enumerate() {
                        if !temporary_mixed {
                            continue;
                        }
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
                                    InterpreterError::new(format!(
                                        "MIR constructor function{} is missing parameter {index}",
                                        constructor.0
                                    ))
                                })?;
                        if promoted || local_in(constructor_definition, parameter)?.owned {
                            continue;
                        }
                        collect_owned_objects_from_value(
                            arguments[index].clone(),
                            &mut temporary_drops,
                        );
                    }
                    let mut constructor_arguments = Vec::with_capacity(arguments.len() + 1);
                    constructor_arguments.push(LocalValue::Class {
                        object: object_id,
                        class,
                    });
                    constructor_arguments.extend(arguments);
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::FinishClassNew {
                            object: object_id,
                            class,
                        });
                    if !temporary_drops.is_empty() {
                        self.current_frame_mut()?
                            .statement_temporary_drops
                            .extend(temporary_drops);
                    }
                    self.push_frame(
                        constructor,
                        &constructor_arguments,
                        Some(ReturnExpectation::Void),
                    )?;
                } else {
                    self.current_frame_mut()?
                        .values
                        .push(EvaluationValue::Class {
                            object: object_id,
                            class,
                        });
                }
            }
            EvaluationTask::FinishClassNew { object, class } => {
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Class { object, class });
            }
            EvaluationTask::BuildNullableSome => {
                let value = self.pop_string()?;
                self.push_nullable_string(Some(value))?;
            }
            EvaluationTask::BuildNullableScalarSome(ty) => {
                let value = self.pop_scalar()?;
                if value.ty() != ty {
                    return Err(InterpreterError::new(
                        "nullable scalar payload type mismatch",
                    ));
                }
                self.push_nullable_scalar(ty, Some(value))?;
            }
            EvaluationTask::BuildNullableClassSome(class) => {
                let LocalValue::Class {
                    object,
                    class: actual,
                } = self.pop_local_value()?
                else {
                    return Err(InterpreterError::new(
                        "nullable class payload is not a class",
                    ));
                };
                if actual != class {
                    return Err(InterpreterError::new(
                        "nullable class payload type mismatch",
                    ));
                }
                self.push_nullable_class(class, Some(object))?;
            }
            EvaluationTask::BuildMixedValue => {
                let value = self.pop_scalar()?;
                self.push_mixed(MixedValue::Scalar(value))?;
            }
            EvaluationTask::BuildMixedString => {
                let value = self.pop_string()?;
                self.push_mixed(MixedValue::String(value))?;
            }
            EvaluationTask::BuildMixedClass => {
                let LocalValue::Class { object, class } = self.pop_local_value()? else {
                    return Err(InterpreterError::new("mixed class payload is not a class"));
                };
                self.push_mixed(MixedValue::Class { object, class })?;
            }
            EvaluationTask::BuildNullableMixedSome => {
                let LocalValue::Mixed(value) = self.pop_local_value()? else {
                    return Err(InterpreterError::new("nullable mixed payload is not mixed"));
                };
                self.push_nullable_mixed(Some(value))?;
            }
            EvaluationTask::WrapNullable(ty) => {
                let value = self.pop_local_value()?;
                self.push_nullable_from_value(ty, value)?;
            }
            EvaluationTask::NullableScalarIsPresent => {
                let (_, value) = self.pop_nullable_scalar()?;
                self.push_scalar(mir::ScalarValue::Bool(value.is_some()))?;
            }
            EvaluationTask::NullableClassIsPresent(owned) => {
                let (class, object) = self.pop_nullable_class()?;
                if let (Some(object), Some(_)) = (object, owned) {
                    self.current_frame_mut()?
                        .statement_temporary_drops
                        .push((object, class));
                }
                self.push_scalar(mir::ScalarValue::Bool(object.is_some()))?;
            }
            EvaluationTask::AfterIntegerCoalesce(right) => {
                let (_, value) = self.pop_nullable_scalar()?;
                if let Some(mir::ScalarValue::Integer(value)) = value {
                    self.push_scalar(mir::ScalarValue::Integer(value))?;
                } else {
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::Integer(right));
                }
            }
            EvaluationTask::AfterFloatCoalesce(right) => {
                let (_, value) = self.pop_nullable_scalar()?;
                if let Some(mir::ScalarValue::Float(value)) = value {
                    self.push_scalar(mir::ScalarValue::Float(value))?;
                } else {
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::Float(right));
                }
            }
            EvaluationTask::AfterBoolCoalesce(right) => {
                let (_, value) = self.pop_nullable_scalar()?;
                if let Some(mir::ScalarValue::Bool(value)) = value {
                    self.push_scalar(mir::ScalarValue::Bool(value))?;
                } else {
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::Bool(right));
                }
            }
            EvaluationTask::AfterStringCoalesce(right) => {
                if let Some(value) = self.pop_nullable_string()? {
                    self.push_string(value)?;
                } else {
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::String(right));
                }
            }
            EvaluationTask::AfterNullableScalarCoalesce(right) => {
                let (ty, value) = self.pop_nullable_scalar()?;
                if value.is_some() {
                    self.push_nullable_scalar(ty, value)?;
                } else {
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::NullableScalar(right));
                }
            }
            EvaluationTask::AfterNullableStringCoalesce(right) => {
                if let Some(value) = self.pop_nullable_string()? {
                    self.push_nullable_string(Some(value))?;
                } else {
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::NullableString(right));
                }
            }
            EvaluationTask::AfterClassCoalesce {
                right,
                left_owned,
                transfer,
            } => {
                let (class, object) = self.pop_nullable_class()?;
                if let Some(object) = object {
                    if left_owned && !transfer {
                        self.current_frame_mut()?
                            .statement_temporary_drops
                            .push((object, class));
                    }
                    self.current_frame_mut()?
                        .values
                        .push(EvaluationValue::Class { object, class });
                } else {
                    let owned = (!transfer).then(|| right.owned_temporary_class()).flatten();
                    let frame = self.current_frame_mut()?;
                    frame
                        .tasks
                        .push(EvaluationTask::FinishClassCoalesceRight(owned));
                    frame.tasks.push(EvaluationTask::Class(right));
                }
            }
            EvaluationTask::FinishClassCoalesceRight(owned) => {
                let LocalValue::Class { object, class } = self.pop_local_value()? else {
                    return Err(InterpreterError::new(
                        "class coalesce fallback produced another value type",
                    ));
                };
                if owned.is_some() {
                    self.current_frame_mut()?
                        .statement_temporary_drops
                        .push((object, class));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Class { object, class });
            }
            EvaluationTask::AfterNullableClassCoalesce {
                right,
                left_owned,
                transfer,
            } => {
                let (class, object) = self.pop_nullable_class()?;
                if let Some(object) = object {
                    if left_owned && !transfer {
                        self.current_frame_mut()?
                            .statement_temporary_drops
                            .push((object, class));
                    }
                    self.push_nullable_class(class, Some(object))?;
                } else {
                    let owned = (!transfer).then(|| right.owned_temporary_class()).flatten();
                    let frame = self.current_frame_mut()?;
                    frame
                        .tasks
                        .push(EvaluationTask::FinishNullableClassCoalesceRight(owned));
                    frame.tasks.push(EvaluationTask::NullableClass(right));
                }
            }
            EvaluationTask::AfterNullableMixedCoalesce { right } => {
                let value = self.pop_nullable_mixed()?;
                if let Some(value) = value {
                    self.push_nullable_mixed(Some(value))?;
                } else {
                    let frame = self.current_frame_mut()?;
                    frame.tasks.push(EvaluationTask::NullableMixed(right));
                }
            }
            EvaluationTask::FinishNullableClassCoalesceRight(owned) => {
                let (class, object) = self.pop_nullable_class()?;
                if let (Some(object), Some(_)) = (object, owned) {
                    self.current_frame_mut()?
                        .statement_temporary_drops
                        .push((object, class));
                }
                self.push_nullable_class(class, object)?;
            }
            EvaluationTask::AfterNullSafeProperty {
                property,
                result,
                owned_receiver,
            } => {
                let (class, object) = self.pop_nullable_class()?;
                if let Some(object) = object {
                    if owned_receiver.is_some() {
                        self.current_frame_mut()?
                            .statement_temporary_drops
                            .push((object, class));
                    }
                    let value = self.read_object_property(object, property)?;
                    self.push_nullable_from_value(result, value)?;
                } else {
                    self.push_null(result)?;
                }
            }
            EvaluationTask::AfterNullSafeCall {
                function,
                args,
                result,
                owned_receiver,
            } => {
                let (class, object) = self.pop_nullable_class()?;
                if let Some(object) = object {
                    if owned_receiver.is_some() {
                        self.current_frame_mut()?
                            .statement_temporary_drops
                            .push((object, class));
                    }
                    self.queue_null_safe_call(object, class, function, args, result)?;
                } else {
                    self.push_null(result)?;
                }
            }
            EvaluationTask::AfterNullSafeStatementCall {
                function,
                args,
                owned_receiver,
            } => {
                let (class, object) = self.pop_nullable_class()?;
                if let Some(object) = object {
                    if owned_receiver.is_some() {
                        self.current_frame_mut()?
                            .statement_temporary_drops
                            .push((object, class));
                    }
                    self.queue_null_safe_statement_call(object, class, function, args)?;
                }
            }
            EvaluationTask::NullableStringCompare(op) => {
                let right = self.pop_nullable_string()?;
                let left = self.pop_nullable_string()?;
                let result = match op {
                    mir::CompareOp::Equal => left == right,
                    mir::CompareOp::NotEqual => left != right,
                    _ => {
                        return Err(InterpreterError::new(
                            "MIR ordered nullable-string comparison is invalid",
                        ))
                    }
                };
                self.push_scalar(mir::ScalarValue::Bool(result))?;
            }
            EvaluationTask::Format(format) => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::BuildFormat(format.clone()));
                for argument in format.arguments.into_iter().rev() {
                    match argument {
                        mir::FormatArgument::Value(value) => {
                            frame.tasks.push(EvaluationTask::Value(value));
                        }
                        mir::FormatArgument::String(value)
                        | mir::FormatArgument::ClassDisplay(value) => {
                            frame.tasks.push(EvaluationTask::String(value));
                        }
                    }
                }
            }
            EvaluationTask::BuildFormat(format) => {
                let values = self.take_evaluation_values(format.arguments.len())?;
                self.push_string(render_format(&format, &values)?)?;
            }
            EvaluationTask::ReadFile => {
                let path = self.pop_string()?;
                if path.as_bytes().contains(&0) {
                    return Ok(StepOutcome::Panic(
                        "file path contained an embedded NUL".to_string(),
                    ));
                }
                let Some(bytes) = self.files.get(&path) else {
                    return Ok(StepOutcome::Panic("failed to read file".to_string()));
                };
                let Ok(value) = String::from_utf8(bytes.clone()) else {
                    return Ok(StepOutcome::Panic(
                        "file contained invalid UTF-8".to_string(),
                    ));
                };
                self.push_string(value)?;
            }
            EvaluationTask::WriteFile => {
                let contents = self.pop_string()?;
                let path = self.pop_string()?;
                if path.as_bytes().contains(&0) {
                    return Ok(StepOutcome::Panic(
                        "file path contained an embedded NUL".to_string(),
                    ));
                }
                self.files.insert(path, contents.into_bytes());
            }
            EvaluationTask::AppendFile => {
                let contents = self.pop_string()?;
                let path = self.pop_string()?;
                if path.as_bytes().contains(&0) {
                    return Ok(StepOutcome::Panic(
                        "file path contained an embedded NUL".to_string(),
                    ));
                }
                self.files
                    .entry(path)
                    .or_default()
                    .extend_from_slice(contents.as_bytes());
            }
            EvaluationTask::ReadFileBytes(collection) => {
                let path = self.pop_string()?;
                if path.as_bytes().contains(&0) {
                    return Ok(StepOutcome::Panic(
                        "file path contained an embedded NUL".to_string(),
                    ));
                }
                let Some(contents) = self.files.get(&path).cloned() else {
                    return Ok(StepOutcome::Panic("failed to read file".to_string()));
                };
                self.push_byte_collection(collection, &contents)?;
            }
            EvaluationTask::WriteFileBytes { contents, append } => {
                let path = self.pop_string()?;
                if path.as_bytes().contains(&0) {
                    return Ok(StepOutcome::Panic(
                        "file path contained an embedded NUL".to_string(),
                    ));
                }
                let bytes = self.byte_collection(contents)?;
                if append {
                    self.files
                        .entry(path)
                        .or_default()
                        .extend_from_slice(&bytes);
                } else {
                    self.files.insert(path, bytes);
                }
            }
            EvaluationTask::WriteStreamBytes { contents, stderr } => {
                let bytes = self.byte_collection(contents)?;
                if stderr {
                    self.stderr.extend_from_slice(&bytes);
                } else {
                    self.stdout.extend_from_slice(&bytes);
                }
            }
            EvaluationTask::WriteStderr => {
                let value = self.pop_string()?;
                self.stderr.extend_from_slice(value.as_bytes());
            }
            EvaluationTask::StringConcat(count) => {
                let mut parts = Vec::with_capacity(count);
                for _ in 0..count {
                    parts.push(self.pop_string()?);
                }
                parts.reverse();
                self.push_string(parts.concat())?;
            }
            EvaluationTask::StringDisplay => {
                let value = self.pop_scalar()?;
                self.push_string(display_scalar(value))?;
            }
            EvaluationTask::StringCompare(op) => {
                let right = self.pop_string()?;
                let left = self.pop_string()?;
                let ordering = left.as_bytes().cmp(right.as_bytes());
                let result = match op {
                    mir::CompareOp::Equal => ordering.is_eq(),
                    mir::CompareOp::NotEqual => !ordering.is_eq(),
                    mir::CompareOp::Less => ordering.is_lt(),
                    mir::CompareOp::LessEqual => !ordering.is_gt(),
                    mir::CompareOp::Greater => ordering.is_gt(),
                    mir::CompareOp::GreaterEqual => !ordering.is_lt(),
                };
                self.push_scalar(mir::ScalarValue::Bool(result))?;
            }
            EvaluationTask::Echo => {
                let value = self.pop_string()?;
                self.stdout.extend_from_slice(value.as_bytes());
            }
            EvaluationTask::PanicString => {
                return Ok(StepOutcome::Panic(self.pop_string()?));
            }
            EvaluationTask::Integer(expression) => self.expand_integer_expression(expression)?,
            EvaluationTask::IntegerUnary(op) => {
                let operand = self.pop_integer()?;
                let value = match eval_unary(op, operand) {
                    Ok(value) => value,
                    Err(panic) => return Ok(StepOutcome::Panic(panic.message().to_string())),
                };
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Scalar(mir::ScalarValue::Integer(value)));
            }
            EvaluationTask::IntegerBinary(op) => {
                let right = self.pop_integer()?;
                let left = self.pop_integer()?;
                let value = match eval_binary(op, left, right) {
                    Ok(value) => value,
                    Err(panic) => return Ok(StepOutcome::Panic(panic.message().to_string())),
                };
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Scalar(mir::ScalarValue::Integer(value)));
            }
            EvaluationTask::IntegerConvert(target) => {
                let value = match self.pop_integer()?.convert(target) {
                    Ok(value) => value,
                    Err(panic) => return Ok(StepOutcome::Panic(panic.message().to_string())),
                };
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Scalar(mir::ScalarValue::Integer(value)));
            }
            EvaluationTask::FloatToInt => {
                let value = self.pop_float()?;
                let Some(value) = value.to_i64_checked() else {
                    return Ok(StepOutcome::Panic(
                        "float-to-integer conversion out of range".to_string(),
                    ));
                };
                self.push_scalar(mir::ScalarValue::Integer(
                    IntegerValue::from_i128(IntegerType::Int64, value as i128)
                        .expect("i64 always fits canonical int"),
                ))?;
            }
            EvaluationTask::Float(expression) => self.expand_float_expression(expression)?,
            EvaluationTask::FloatNegate => {
                let value = self.pop_float()?.negate();
                self.push_scalar(mir::ScalarValue::Float(value))?;
            }
            EvaluationTask::FloatBinary(op) => {
                let right = self.pop_float()?;
                let left = self.pop_float()?;
                let value = match op {
                    mir::FloatBinaryOp::Add => left.add(right),
                    mir::FloatBinaryOp::Subtract => left.subtract(right),
                    mir::FloatBinaryOp::Multiply => left.multiply(right),
                    mir::FloatBinaryOp::Divide => left.divide(right),
                };
                self.push_scalar(mir::ScalarValue::Float(value))?;
            }
            EvaluationTask::IntToFloat => {
                let value = self.pop_integer()?;
                if value.ty != IntegerType::Int64 {
                    return Err(InterpreterError::new(
                        "MIR Int::toFloat operand is not canonical int",
                    ));
                }
                self.push_scalar(mir::ScalarValue::Float(FloatValue::from_f64(
                    value.signed_value() as f64,
                )))?;
            }
            EvaluationTask::Bool(condition) => self.expand_bool_expression(condition)?,
            EvaluationTask::Compare(op) => {
                let right = self.pop_scalar()?;
                let left = self.pop_scalar()?;
                let value = eval_compare(op, left, right)?;
                self.push_scalar(mir::ScalarValue::Bool(value))?;
            }
            EvaluationTask::Not => {
                let value = !self.pop_bool()?;
                self.push_scalar(mir::ScalarValue::Bool(value))?;
            }
            EvaluationTask::AfterAnd(right) => {
                if self.pop_bool()? {
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::Bool(right));
                } else {
                    self.push_scalar(mir::ScalarValue::Bool(false))?;
                }
            }
            EvaluationTask::AfterOr(right) => {
                if self.pop_bool()? {
                    self.push_scalar(mir::ScalarValue::Bool(true))?;
                } else {
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::Bool(right));
                }
            }
            EvaluationTask::Xor => {
                let right = self.pop_bool()?;
                let left = self.pop_bool()?;
                self.push_scalar(mir::ScalarValue::Bool(left ^ right))?;
            }
            EvaluationTask::Invoke {
                function,
                argument_count,
                expectation,
                temporary_class_args,
                temporary_mixed_args,
            } => {
                let args = self.take_call_arguments(argument_count)?;
                let mut drops = Vec::new();
                for (argument, temporary) in args.iter().zip(temporary_class_args) {
                    if !temporary {
                        continue;
                    }
                    match argument {
                        LocalValue::Class { object, class } => drops.push((*object, *class)),
                        LocalValue::NullableClass {
                            object: Some(object),
                            class,
                        } => drops.push((*object, *class)),
                        LocalValue::NullableClass { object: None, .. } => {}
                        _ => {
                            return Err(InterpreterError::new(
                                "MIR temporary-class call argument produced another value type",
                            ))
                        }
                    }
                }
                for (argument, temporary) in args.iter().zip(temporary_mixed_args) {
                    if temporary {
                        collect_owned_objects_from_value(argument.clone(), &mut drops);
                    }
                }
                if !drops.is_empty() {
                    self.current_frame_mut()?
                        .statement_temporary_drops
                        .extend(drops);
                }
                self.push_frame(function, &args, Some(expectation))?;
            }
            EvaluationTask::FinishStatement => {
                let drops =
                    std::mem::take(&mut self.current_frame_mut()?.statement_temporary_drops);
                if !drops.is_empty() {
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::DropTemporaryClasses(drops));
                }
            }
            EvaluationTask::DropTemporaryClasses(drops) => {
                let frame = self.current_frame_mut()?;
                for (object, class) in drops {
                    frame
                        .tasks
                        .push(EvaluationTask::DropObject { object, class });
                }
            }
            EvaluationTask::Assign(target) => {
                let value = self.pop_local_value()?;
                let function = function_in(self.program, self.current_frame()?.function)?;
                let old = assign_local(
                    &function.locals,
                    &mut self.current_frame_mut()?.locals,
                    target,
                    value,
                )?;
                if let Some(value) = old {
                    if let Some((object, class)) = owned_object(&value) {
                        self.current_frame_mut()?
                            .tasks
                            .push(EvaluationTask::DropObject { object, class });
                    } else if let LocalValue::Collection(collection) = value {
                        let mut drops = Vec::new();
                        collect_owned_objects_from_collection(collection, &mut drops);
                        for (object, class) in drops {
                            self.current_frame_mut()?
                                .tasks
                                .push(EvaluationTask::DropObject { object, class });
                        }
                    }
                }
            }
            EvaluationTask::AssignStatic(target) => {
                let value = self.pop_local_value()?;
                let slot = self.statics.get_mut(target.0).ok_or_else(|| {
                    InterpreterError::new(format!("MIR static{} does not exist", target.0))
                })?;
                *slot = value;
            }
            EvaluationTask::AssignProperty { object, property } => {
                let value = self.pop_local_value()?;
                if let Some(old) = self.assign_property(object, property, value)? {
                    if let Some((object, class)) = owned_object(&old) {
                        self.current_frame_mut()?
                            .tasks
                            .push(EvaluationTask::DropObject { object, class });
                    } else if let LocalValue::Collection(collection) = old {
                        let mut drops = Vec::new();
                        collect_owned_objects_from_collection(collection, &mut drops);
                        for (object, class) in drops {
                            self.current_frame_mut()?
                                .tasks
                                .push(EvaluationTask::DropObject { object, class });
                        }
                    }
                }
            }
            EvaluationTask::DropClass(local) => {
                self.drop_class_local(local)?;
            }
            EvaluationTask::DropCollection(local) => {
                self.drop_collection_local(local)?;
            }
            EvaluationTask::DropObject { object, class } => {
                self.queue_object_drop(object, class)?;
            }
            EvaluationTask::DropObjectProperties { object, class } => {
                self.queue_object_property_drops(object, class)?;
            }
            EvaluationTask::FreeObject { object, class } => {
                self.free_object(object, class)?;
            }
            EvaluationTask::CleanupFrame => {
                self.cleanup_current_frame()?;
            }
            EvaluationTask::ReturnValue(expected) => {
                let value = self.pop_local_value()?;
                if local_value_type(&value) != expected {
                    return Err(InterpreterError::new(format!(
                        "MIR return evaluation produced {}, expected {expected}",
                        local_value_type(&value)
                    )));
                }
                return self.complete_frame(FunctionOutcome::Value(value));
            }
            EvaluationTask::ReturnVoid => {
                return self.complete_frame(FunctionOutcome::Void);
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

    fn expand_integer_expression(
        &mut self,
        expression: mir::IntegerExpression,
    ) -> Result<(), InterpreterError> {
        match expression {
            mir::IntegerExpression::Use { ty, operand } => {
                if self.queue_collection_scalar_operand(&operand)? {
                    return Ok(());
                }
                let value = self.eval_operand(&operand)?;
                let mir::ScalarValue::Integer(value) = value else {
                    return Err(InterpreterError::new(
                        "MIR integer operand produced another scalar type",
                    ));
                };
                if value.ty != ty {
                    return Err(InterpreterError::new(format!(
                        "MIR operand evaluation produced {}, expression declares {ty}",
                        value.ty
                    )));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Scalar(mir::ScalarValue::Integer(value)));
            }
            mir::IntegerExpression::Unary { ty, op, operand } => {
                if operand.ty() != ty {
                    return Err(InterpreterError::new(format!(
                        "MIR {ty} unary expression has operand type {}",
                        operand.ty()
                    )));
                }
                if op == mir::IntegerUnaryOp::Negate && !ty.is_signed() {
                    return Err(InterpreterError::new(format!(
                        "MIR unary negation requires a signed integer, got {ty}"
                    )));
                }
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::IntegerUnary(op));
                frame.tasks.push(EvaluationTask::Integer(*operand));
            }
            mir::IntegerExpression::Binary {
                ty,
                op,
                left,
                right,
            } => {
                if left.ty() != ty || right.ty() != ty {
                    return Err(InterpreterError::new(format!(
                        "MIR {ty} binary expression has operand types {} and {}",
                        left.ty(),
                        right.ty()
                    )));
                }
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::IntegerBinary(op));
                frame.tasks.push(EvaluationTask::Integer(*right));
                frame.tasks.push(EvaluationTask::Integer(*left));
            }
            mir::IntegerExpression::Convert { ty, value } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::IntegerConvert(ty));
                frame.tasks.push(EvaluationTask::Integer(*value));
            }
            mir::IntegerExpression::FloatToInt { value } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::FloatToInt);
                frame.tasks.push(EvaluationTask::Float(*value));
            }
            mir::IntegerExpression::Call { ty, function, args } => {
                self.queue_call(
                    function,
                    args,
                    ReturnExpectation::Value(mir::Type::Scalar(mir::ScalarType::Integer(ty))),
                )?;
            }
            mir::IntegerExpression::Coalesce { left, right, .. } => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::AfterIntegerCoalesce(*right));
                frame.tasks.push(EvaluationTask::NullableScalar(*left));
            }
        }
        Ok(())
    }

    fn expand_float_expression(
        &mut self,
        expression: mir::FloatExpression,
    ) -> Result<(), InterpreterError> {
        match expression {
            mir::FloatExpression::Use { ty, operand } => {
                if self.queue_collection_scalar_operand(&operand)? {
                    return Ok(());
                }
                let value = self.eval_operand(&operand)?;
                let mir::ScalarValue::Float(value) = value else {
                    return Err(InterpreterError::new(
                        "MIR float operand produced another scalar type",
                    ));
                };
                if value.ty != ty {
                    return Err(InterpreterError::new(format!(
                        "MIR float operand produced {}, expected {ty}",
                        value.ty
                    )));
                }
                self.push_scalar(mir::ScalarValue::Float(value))?;
            }
            mir::FloatExpression::Negate { operand, .. } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::FloatNegate);
                frame.tasks.push(EvaluationTask::Float(*operand));
            }
            mir::FloatExpression::Binary {
                op, left, right, ..
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::FloatBinary(op));
                frame.tasks.push(EvaluationTask::Float(*right));
                frame.tasks.push(EvaluationTask::Float(*left));
            }
            mir::FloatExpression::IntToFloat { value } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::IntToFloat);
                frame.tasks.push(EvaluationTask::Integer(*value));
            }
            mir::FloatExpression::Call { ty, function, args } => {
                self.queue_call(
                    function,
                    args,
                    ReturnExpectation::Value(mir::Type::Scalar(mir::ScalarType::Float(ty))),
                )?;
            }
            mir::FloatExpression::Coalesce { left, right, .. } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::AfterFloatCoalesce(*right));
                frame.tasks.push(EvaluationTask::NullableScalar(*left));
            }
        }
        Ok(())
    }

    fn expand_bool_expression(
        &mut self,
        condition: mir::BoolExpression,
    ) -> Result<(), InterpreterError> {
        match condition {
            mir::BoolExpression::Use { operand } => {
                if self.queue_collection_scalar_operand(&operand)? {
                    return Ok(());
                }
                let value = self.eval_operand(&operand)?;
                if !matches!(value, mir::ScalarValue::Bool(_)) {
                    return Err(InterpreterError::new(
                        "MIR bool operand produced another scalar type",
                    ));
                }
                self.push_scalar(value)?;
            }
            mir::BoolExpression::Compare { op, left, right } => {
                if left.ty() != right.ty() {
                    return Err(InterpreterError::new(format!(
                        "MIR comparison has operand types {} and {}",
                        left.ty(),
                        right.ty()
                    )));
                }
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::Compare(op));
                frame.tasks.push(EvaluationTask::Value(*right));
                frame.tasks.push(EvaluationTask::Value(*left));
            }
            mir::BoolExpression::StringCompare { op, left, right } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::StringCompare(op));
                frame.tasks.push(EvaluationTask::String(*right));
                frame.tasks.push(EvaluationTask::String(*left));
            }
            mir::BoolExpression::NullableStringCompare { op, left, right } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::NullableStringCompare(op));
                frame.tasks.push(EvaluationTask::NullableString(*right));
                frame.tasks.push(EvaluationTask::NullableString(*left));
            }
            mir::BoolExpression::CollectionEqual { left, right } => {
                let equal = self.byte_collection(left)? == self.byte_collection(right)?;
                self.push_scalar(mir::ScalarValue::Bool(equal))?;
            }
            mir::BoolExpression::NullableScalarIsPresent(value) => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::NullableScalarIsPresent);
                frame.tasks.push(EvaluationTask::NullableScalar(*value));
            }
            mir::BoolExpression::NullableClassIsPresent(value) => {
                let owned = value.owned_temporary_class();
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::NullableClassIsPresent(owned));
                frame.tasks.push(EvaluationTask::NullableClass(*value));
            }
            mir::BoolExpression::NullableMixedIsPresent(value) => {
                self.expand_nullable_mixed_expression(*value)?;
                let present = self.pop_nullable_mixed()?.is_some();
                self.push_scalar(mir::ScalarValue::Bool(present))?;
            }
            mir::BoolExpression::MixedIs { mixed, tag } => {
                self.expand_mixed_expression(*mixed)?;
                let LocalValue::Mixed(value) = self.pop_local_value()? else {
                    return Err(InterpreterError::new(
                        "MIR mixed is expression produced another value type",
                    ));
                };
                self.push_scalar(mir::ScalarValue::Bool(value.tag() == tag))?;
            }
            mir::BoolExpression::Not(condition) => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::Not);
                frame.tasks.push(EvaluationTask::Bool(*condition));
            }
            mir::BoolExpression::Binary {
                op: mir::BoolBinaryOp::And,
                left,
                right,
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::AfterAnd(*right));
                frame.tasks.push(EvaluationTask::Bool(*left));
            }
            mir::BoolExpression::Binary {
                op: mir::BoolBinaryOp::Or,
                left,
                right,
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::AfterOr(*right));
                frame.tasks.push(EvaluationTask::Bool(*left));
            }
            mir::BoolExpression::Binary {
                op: mir::BoolBinaryOp::Xor,
                left,
                right,
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::Xor);
                frame.tasks.push(EvaluationTask::Bool(*right));
                frame.tasks.push(EvaluationTask::Bool(*left));
            }
            mir::BoolExpression::Call { function, args } => {
                self.queue_call(
                    function,
                    args,
                    ReturnExpectation::Value(mir::Type::Scalar(mir::ScalarType::Bool)),
                )?;
            }
            mir::BoolExpression::Coalesce { left, right } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::AfterBoolCoalesce(*right));
                frame.tasks.push(EvaluationTask::NullableScalar(*left));
            }
            mir::BoolExpression::CollectionHas {
                collection,
                value,
                op,
            } => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::CollectionHas { collection, op });
                frame.tasks.push(EvaluationTask::Rvalue(*value));
            }
            mir::BoolExpression::CollectionIsEmpty { collection } => {
                self.current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::CollectionIsEmpty(collection));
            }
        }
        Ok(())
    }

    fn expand_string_expression(
        &mut self,
        expression: mir::StringExpression,
    ) -> Result<(), InterpreterError> {
        match expression {
            mir::StringExpression::Literal(value) => self.push_string(value)?,
            mir::StringExpression::Local(id) => {
                match read_local(&self.current_frame()?.locals, id)? {
                    LocalValue::String(value) => self.push_string(value.clone())?,
                    LocalValue::Scalar(_) => {
                        return Err(InterpreterError::new(format!(
                            "MIR scalar local local{} was used as a string value",
                            id.0
                        )))
                    }
                    LocalValue::NullableString(_) => {
                        return Err(InterpreterError::new(format!(
                            "MIR nullable-string local local{} was used as a string value",
                            id.0
                        )))
                    }
                    LocalValue::Mixed(_) | LocalValue::NullableMixed(_) => {
                        return Err(InterpreterError::new(format!(
                            "MIR mixed local local{} was used as a string value",
                            id.0
                        )))
                    }
                    LocalValue::Class { .. } => {
                        return Err(InterpreterError::new(format!(
                            "MIR class local local{} was used as a string value",
                            id.0
                        )))
                    }
                    LocalValue::NullableScalar { .. } | LocalValue::NullableClass { .. } => {
                        return Err(InterpreterError::new(format!(
                            "MIR nullable local local{} was used as a string value",
                            id.0
                        )))
                    }
                    LocalValue::Collection(_) => {
                        return Err(InterpreterError::new(format!(
                            "MIR collection local local{} was used as a string value",
                            id.0
                        )))
                    }
                }
            }
            mir::StringExpression::Static(id) => match self.statics.get(id.0) {
                Some(LocalValue::String(value)) => self.push_string(value.clone())?,
                _ => {
                    return Err(InterpreterError::new(format!(
                        "MIR static{} was used as string",
                        id.0
                    )))
                }
            },
            mir::StringExpression::MixedPayload(local) => {
                let value =
                    mixed_value_from_local(read_local(&self.current_frame()?.locals, local)?)
                        .ok_or_else(|| {
                            InterpreterError::new(
                                "MIR mixed string payload references another local type",
                            )
                        })?;
                let MixedValue::String(value) = value else {
                    return Err(InterpreterError::new(
                        "MIR mixed string payload observed another tag",
                    ));
                };
                self.push_string(value.clone())?;
            }
            mir::StringExpression::NullableLocalAssumeNonNull(id) => {
                match read_local(&self.current_frame()?.locals, id)? {
                    LocalValue::NullableString(Some(value)) => self.push_string(value.clone())?,
                    LocalValue::NullableString(None) => {
                        return Err(InterpreterError::new(
                            "MIR nonnull string expression observed null",
                        ))
                    }
                    _ => {
                        return Err(InterpreterError::new(
                            "MIR nonnull string expression references another local type",
                        ))
                    }
                }
            }
            mir::StringExpression::Property { object, property } => {
                match self.read_property(object, property)? {
                    LocalValue::String(value) => self.push_string(value)?,
                    _ => {
                        return Err(InterpreterError::new(format!(
                            "MIR property{} was used as a string value",
                            property.index
                        )))
                    }
                }
            }
            mir::StringExpression::Concat(parts) => {
                let count = parts.len();
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::StringConcat(count));
                for part in parts.into_iter().rev() {
                    frame.tasks.push(EvaluationTask::String(part));
                }
            }
            mir::StringExpression::Display(value) => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::StringDisplay);
                frame.tasks.push(EvaluationTask::Value(value));
            }
            mir::StringExpression::Call { function, args } => {
                self.queue_call(function, args, ReturnExpectation::Value(mir::Type::String))?;
            }
            mir::StringExpression::ReadFile(path) => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::ReadFile);
                frame.tasks.push(EvaluationTask::String(*path));
            }
            mir::StringExpression::Format(format) => {
                self.current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::Format(*format));
            }
            mir::StringExpression::Coalesce { left, right } => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::AfterStringCoalesce(*right));
                frame.tasks.push(EvaluationTask::NullableString(*left));
            }
            mir::StringExpression::CollectionIndex {
                collection,
                index,
                remove,
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::LoadCollectionValue {
                    collection,
                    transfer: remove,
                });
                frame.tasks.push(EvaluationTask::Rvalue(*index));
            }
            mir::StringExpression::CollectionKeyAt { collection, offset } => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::CollectionKeyString(collection));
                frame.tasks.push(EvaluationTask::Rvalue(*offset));
            }
        }
        Ok(())
    }

    fn expand_nullable_scalar_expression(
        &mut self,
        expression: mir::NullableScalarExpression,
    ) -> Result<(), InterpreterError> {
        let ty = expression.ty();
        match expression {
            mir::NullableScalarExpression::Null(_) => self.push_nullable_scalar(ty, None)?,
            mir::NullableScalarExpression::Value(value) => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::BuildNullableScalarSome(ty));
                frame.tasks.push(EvaluationTask::Value(value));
            }
            mir::NullableScalarExpression::Local { local, .. } => {
                let LocalValue::NullableScalar { ty, value } =
                    read_local(&self.current_frame()?.locals, local)?.clone()
                else {
                    return Err(InterpreterError::new(
                        "nullable scalar references another local type",
                    ));
                };
                self.push_nullable_scalar(ty, value)?;
            }
            mir::NullableScalarExpression::Property {
                object, property, ..
            } => {
                let LocalValue::NullableScalar { ty, value } =
                    self.read_property(object, property)?
                else {
                    return Err(InterpreterError::new(
                        "nullable scalar property has another type",
                    ));
                };
                self.push_nullable_scalar(ty, value)?;
            }
            mir::NullableScalarExpression::Static { id, .. } => {
                let Some(LocalValue::NullableScalar { ty, value }) =
                    self.statics.get(id.0).cloned()
                else {
                    return Err(InterpreterError::new(
                        "nullable scalar static has another type",
                    ));
                };
                self.push_nullable_scalar(ty, value)?;
            }
            mir::NullableScalarExpression::Call { function, args, .. } => {
                self.queue_call(
                    function,
                    args,
                    ReturnExpectation::Value(mir::Type::NullableScalar(ty)),
                )?;
            }
            mir::NullableScalarExpression::NullSafeProperty {
                object, property, ..
            } => {
                let owned_receiver = object.owned_temporary_class();
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::AfterNullSafeProperty {
                    property,
                    result: mir::Type::NullableScalar(ty),
                    owned_receiver,
                });
                frame.tasks.push(EvaluationTask::NullableClass(*object));
            }
            mir::NullableScalarExpression::NullSafeCall {
                object,
                function,
                args,
                ..
            } => {
                let owned_receiver = object.owned_temporary_class();
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::AfterNullSafeCall {
                    function,
                    args,
                    result: mir::Type::NullableScalar(ty),
                    owned_receiver,
                });
                frame.tasks.push(EvaluationTask::NullableClass(*object));
            }
            mir::NullableScalarExpression::Coalesce { left, right, .. } => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::AfterNullableScalarCoalesce(*right));
                frame.tasks.push(EvaluationTask::NullableScalar(*left));
            }
            mir::NullableScalarExpression::DictionaryGet {
                ty,
                collection,
                key,
                access,
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::DictionaryGet {
                    collection,
                    expected: mir::Type::Scalar(ty),
                    access,
                });
                frame.tasks.push(EvaluationTask::Rvalue(*key));
            }
        }
        Ok(())
    }

    fn expand_mixed_expression(
        &mut self,
        expression: mir::MixedExpression,
    ) -> Result<(), InterpreterError> {
        match expression {
            mir::MixedExpression::Local { local, transfer } => {
                let value = if transfer {
                    self.current_frame_mut()?
                        .locals
                        .get_mut(local.0)
                        .and_then(Option::take)
                        .ok_or_else(|| {
                            InterpreterError::new(format!(
                                "MIR mixed local local{} was moved before use",
                                local.0
                            ))
                        })?
                } else {
                    read_local(&self.current_frame()?.locals, local)?.clone()
                };
                let value = mixed_value_from_local(&value)
                    .ok_or_else(|| {
                        InterpreterError::new("MIR mixed expression used another local type")
                    })?
                    .clone();
                self.push_mixed(value)?;
            }
            mir::MixedExpression::Property { object, property } => {
                let LocalValue::Mixed(value) = self.read_property(object, property)? else {
                    return Err(InterpreterError::new(
                        "MIR mixed property contains another value type",
                    ));
                };
                self.push_mixed(value)?;
            }
            mir::MixedExpression::Call { function, args } => {
                self.queue_call(function, args, ReturnExpectation::Value(mir::Type::Mixed))?;
            }
            mir::MixedExpression::BoxValue(value) => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::BuildMixedValue);
                frame.tasks.push(EvaluationTask::Value(value));
            }
            mir::MixedExpression::BoxString(value) => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::BuildMixedString);
                frame.tasks.push(EvaluationTask::String(value));
            }
            mir::MixedExpression::BoxClass(value) => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::BuildMixedClass);
                frame.tasks.push(EvaluationTask::Class(value));
            }
            mir::MixedExpression::CollectionIndex {
                collection,
                index,
                transfer,
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::LoadCollectionValue {
                    collection,
                    transfer,
                });
                frame.tasks.push(EvaluationTask::Rvalue(*index));
            }
        }
        Ok(())
    }

    fn expand_nullable_mixed_expression(
        &mut self,
        expression: mir::NullableMixedExpression,
    ) -> Result<(), InterpreterError> {
        match expression {
            mir::NullableMixedExpression::Null => self.push_nullable_mixed(None)?,
            mir::NullableMixedExpression::Mixed(value) => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::BuildNullableMixedSome);
                frame.tasks.push(EvaluationTask::Mixed(value));
            }
            mir::NullableMixedExpression::Local { local, transfer } => {
                let value = if transfer {
                    self.current_frame_mut()?
                        .locals
                        .get_mut(local.0)
                        .and_then(Option::take)
                        .ok_or_else(|| {
                            InterpreterError::new(format!(
                                "MIR nullable mixed local local{} was moved before use",
                                local.0
                            ))
                        })?
                } else {
                    read_local(&self.current_frame()?.locals, local)?.clone()
                };
                let LocalValue::NullableMixed(value) = value else {
                    return Err(InterpreterError::new(
                        "MIR nullable mixed expression used another local type",
                    ));
                };
                self.push_nullable_mixed(value)?;
            }
            mir::NullableMixedExpression::Property { object, property } => {
                let LocalValue::NullableMixed(value) = self.read_property(object, property)? else {
                    return Err(InterpreterError::new(
                        "MIR nullable mixed property contains another value type",
                    ));
                };
                self.push_nullable_mixed(value)?;
            }
            mir::NullableMixedExpression::Call { function, args } => {
                self.queue_call(
                    function,
                    args,
                    ReturnExpectation::Value(mir::Type::NullableMixed),
                )?;
            }
            mir::NullableMixedExpression::Coalesce {
                left,
                right,
                transfer: _,
            } => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::AfterNullableMixedCoalesce { right: *right });
                frame.tasks.push(EvaluationTask::NullableMixed(*left));
            }
        }
        Ok(())
    }

    fn expand_nullable_string_expression(
        &mut self,
        expression: mir::NullableStringExpression,
    ) -> Result<(), InterpreterError> {
        match expression {
            mir::NullableStringExpression::Null => self.push_nullable_string(None)?,
            mir::NullableStringExpression::String(value) => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::BuildNullableSome);
                frame.tasks.push(EvaluationTask::String(value));
            }
            mir::NullableStringExpression::Local(local) => {
                match read_local(&self.current_frame()?.locals, local)? {
                    LocalValue::NullableString(value) => {
                        self.push_nullable_string(value.clone())?;
                    }
                    _ => {
                        return Err(InterpreterError::new(format!(
                            "MIR non-nullable local local{} used as ?string",
                            local.0
                        )))
                    }
                }
            }
            mir::NullableStringExpression::Static(id) => match self.statics.get(id.0) {
                Some(LocalValue::NullableString(value)) => {
                    self.push_nullable_string(value.clone())?;
                }
                _ => {
                    return Err(InterpreterError::new(format!(
                        "MIR static{} was used as ?string",
                        id.0
                    )))
                }
            },
            mir::NullableStringExpression::Property { object, property } => {
                match self.read_property(object, property)? {
                    LocalValue::NullableString(value) => self.push_nullable_string(value)?,
                    _ => {
                        return Err(InterpreterError::new(format!(
                            "MIR property{} was used as a nullable string value",
                            property.index
                        )))
                    }
                }
            }
            mir::NullableStringExpression::ReadLine => {
                if self.stdin_cursor == self.stdin.len() {
                    self.push_nullable_string(None)?;
                } else {
                    let remaining = &self.stdin[self.stdin_cursor..];
                    let newline = remaining.iter().position(|byte| *byte == b'\n');
                    let consumed = newline.map_or(remaining.len(), |index| index + 1);
                    let mut line_length = newline.unwrap_or(remaining.len());
                    if line_length != 0 && remaining[line_length - 1] == b'\r' {
                        line_length -= 1;
                    }
                    let line = core::str::from_utf8(&remaining[..line_length])
                        .map_err(|_| InterpreterError::new("stdin contained invalid UTF-8"))?
                        .to_string();
                    self.stdin_cursor += consumed;
                    self.push_nullable_string(Some(line))?;
                }
            }
            mir::NullableStringExpression::Call { function, args } => {
                self.queue_call(
                    function,
                    args,
                    ReturnExpectation::Value(mir::Type::NullableString),
                )?;
            }
            mir::NullableStringExpression::NullSafeProperty { object, property } => {
                let owned_receiver = object.owned_temporary_class();
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::AfterNullSafeProperty {
                    property,
                    result: mir::Type::NullableString,
                    owned_receiver,
                });
                frame.tasks.push(EvaluationTask::NullableClass(*object));
            }
            mir::NullableStringExpression::NullSafeCall {
                object,
                function,
                args,
            } => {
                let owned_receiver = object.owned_temporary_class();
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::AfterNullSafeCall {
                    function,
                    args,
                    result: mir::Type::NullableString,
                    owned_receiver,
                });
                frame.tasks.push(EvaluationTask::NullableClass(*object));
            }
            mir::NullableStringExpression::Coalesce { left, right } => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::AfterNullableStringCoalesce(*right));
                frame.tasks.push(EvaluationTask::NullableString(*left));
            }
            mir::NullableStringExpression::DictionaryGet {
                collection,
                key,
                access,
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::DictionaryGet {
                    collection,
                    expected: mir::Type::String,
                    access,
                });
                frame.tasks.push(EvaluationTask::Rvalue(*key));
            }
        }
        Ok(())
    }

    fn expand_class_expression(
        &mut self,
        expression: mir::ClassExpression,
    ) -> Result<(), InterpreterError> {
        match expression {
            mir::ClassExpression::Local {
                class,
                local,
                transfer,
            } => {
                let value = if transfer {
                    self.current_frame_mut()?
                        .locals
                        .get_mut(local.0)
                        .ok_or_else(|| {
                            InterpreterError::new(format!(
                                "MIR local local{} does not exist",
                                local.0
                            ))
                        })?
                        .take()
                        .ok_or_else(|| {
                            InterpreterError::new(format!(
                                "MIR local local{} was moved before use",
                                local.0
                            ))
                        })?
                } else {
                    read_local(&self.current_frame()?.locals, local)?.clone()
                };
                if local_value_type(&value) != mir::Type::Class(class) {
                    return Err(InterpreterError::new(format!(
                        "MIR class expression expected class#{}, got {}",
                        class.0,
                        local_value_type(&value)
                    )));
                }
                let LocalValue::Class { object, class } = value else {
                    unreachable!("checked class local value")
                };
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Class { object, class });
            }
            mir::ClassExpression::Property {
                class,
                object,
                property,
            } => {
                let value = self.read_property(object, property)?;
                let LocalValue::Class {
                    object,
                    class: actual,
                } = value
                else {
                    return Err(InterpreterError::new(format!(
                        "MIR property{} was used as a class value",
                        property.index
                    )));
                };
                if actual != class {
                    return Err(InterpreterError::new(format!(
                        "MIR class property produced class#{}, expected class#{}",
                        actual.0, class.0
                    )));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Class {
                        object,
                        class: actual,
                    });
            }
            mir::ClassExpression::Call {
                class,
                function,
                args,
                ..
            } => {
                self.queue_call(
                    function,
                    args,
                    ReturnExpectation::Value(mir::Type::Class(class)),
                )?;
            }
            mir::ClassExpression::New {
                class,
                properties,
                constructor,
                args,
            } => {
                let temporary_class_args = args
                    .iter()
                    .map(mir::Rvalue::owned_temporary_class)
                    .collect();
                let temporary_mixed_args = args
                    .iter()
                    .map(mir::Rvalue::owned_temporary_mixed)
                    .collect();
                let property_expression_count = properties
                    .iter()
                    .filter(|property| {
                        matches!(property.source, mir::PropertyValueSource::Expression(_))
                    })
                    .count();
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::BuildClassNew {
                    class,
                    properties: properties.clone(),
                    constructor,
                    argument_count: args.len(),
                    property_expression_count,
                    temporary_class_args,
                    temporary_mixed_args,
                });
                for argument in args.into_iter().rev() {
                    frame.tasks.push(EvaluationTask::Rvalue(argument));
                }
                for property in properties.into_iter().rev() {
                    if let mir::PropertyValueSource::Expression(value) = property.source {
                        frame.tasks.push(EvaluationTask::Rvalue(value));
                    }
                }
            }
            mir::ClassExpression::NullableLocalAssumeNonNull {
                class,
                local,
                transfer,
            } => {
                let value = if transfer {
                    self.current_frame_mut()?
                        .locals
                        .get_mut(local.0)
                        .and_then(Option::take)
                        .ok_or_else(|| {
                            InterpreterError::new("nullable class was moved before use")
                        })?
                } else {
                    read_local(&self.current_frame()?.locals, local)?.clone()
                };
                let LocalValue::NullableClass {
                    object: Some(object),
                    class: actual,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "nonnull class expression observed null",
                    ));
                };
                if actual != class {
                    return Err(InterpreterError::new(
                        "nonnull class expression has another class",
                    ));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Class { object, class });
            }
            mir::ClassExpression::Coalesce {
                left,
                right,
                transfer,
                ..
            } => {
                let left_owned = left.owned_temporary_class().is_some();
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::AfterClassCoalesce {
                    right: *right,
                    left_owned,
                    transfer,
                });
                frame.tasks.push(EvaluationTask::NullableClass(*left));
            }
            mir::ClassExpression::CollectionIndex {
                class,
                collection,
                index,
                transfer,
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::CollectionIndexClass {
                    collection,
                    class,
                    transfer,
                });
                frame.tasks.push(EvaluationTask::Rvalue(*index));
            }
            mir::ClassExpression::MixedPayload {
                class,
                mixed,
                transfer,
            } => {
                let value = if transfer {
                    match mixed_value_from_local(read_local(&self.current_frame()?.locals, mixed)?)
                    {
                        Some(MixedValue::Class { class: actual, .. }) if *actual == class => {}
                        Some(MixedValue::Class { .. }) => {
                            return Err(InterpreterError::new(
                                "MIR mixed class payload observed another class",
                            ));
                        }
                        Some(_) => {
                            return Err(InterpreterError::new(
                                "MIR mixed class payload observed another tag",
                            ));
                        }
                        None => {
                            return Err(InterpreterError::new(
                                "MIR mixed class payload references another local type",
                            ));
                        }
                    }
                    let slot = self
                        .current_frame_mut()?
                        .locals
                        .get_mut(mixed.0)
                        .ok_or_else(|| InterpreterError::new("MIR mixed local does not exist"))?;
                    match slot.take() {
                        Some(LocalValue::Mixed(value)) => value,
                        Some(LocalValue::NullableMixed(Some(value))) => value,
                        Some(value) => {
                            *slot = Some(value);
                            return Err(InterpreterError::new(
                                "MIR mixed class payload references another local type",
                            ));
                        }
                        None => {
                            return Err(InterpreterError::new(
                                "MIR mixed class payload was read before assignment",
                            ));
                        }
                    }
                } else {
                    mixed_value_from_local(read_local(&self.current_frame()?.locals, mixed)?)
                        .ok_or_else(|| {
                            InterpreterError::new(
                                "MIR mixed class payload references another local type",
                            )
                        })?
                        .clone()
                };
                let MixedValue::Class {
                    object,
                    class: actual,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "MIR mixed class payload observed another tag",
                    ));
                };
                if actual != class {
                    return Err(InterpreterError::new(
                        "MIR mixed class payload observed another class",
                    ));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Class { object, class });
            }
        }
        Ok(())
    }

    fn expand_collection_expression(
        &mut self,
        expression: mir::CollectionExpression,
    ) -> Result<(), InterpreterError> {
        match expression {
            mir::CollectionExpression::Local {
                collection,
                local,
                transfer,
            } => {
                let value = if transfer {
                    self.current_frame_mut()?
                        .locals
                        .get_mut(local.0)
                        .and_then(Option::take)
                        .ok_or_else(|| {
                            InterpreterError::new(format!(
                                "MIR collection local local{} was moved before use",
                                local.0
                            ))
                        })?
                } else {
                    read_local(&self.current_frame()?.locals, local)?.clone()
                };
                let LocalValue::Collection(value) = value else {
                    return Err(InterpreterError::new(
                        "MIR collection expression used another local type",
                    ));
                };
                if value.ty != collection {
                    return Err(InterpreterError::new(
                        "MIR collection expression has another collection type",
                    ));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Collection(value));
            }
            mir::CollectionExpression::Literal {
                collection,
                entries,
            } => {
                let keyed = entries
                    .iter()
                    .map(|entry| entry.key.is_some())
                    .collect::<Vec<_>>();
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::BuildCollection { collection, keyed });
                for entry in entries.into_iter().rev() {
                    frame.tasks.push(EvaluationTask::Rvalue(entry.value));
                    if let Some(key) = entry.key {
                        frame.tasks.push(EvaluationTask::Rvalue(key));
                    }
                }
            }
            mir::CollectionExpression::Index {
                source,
                index,
                transfer,
                ..
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::LoadCollectionValue {
                    collection: source,
                    transfer,
                });
                frame.tasks.push(EvaluationTask::Rvalue(*index));
            }
            mir::CollectionExpression::Property {
                collection,
                object,
                property,
            } => {
                let LocalValue::Collection(value) = self.read_property(object, property)? else {
                    return Err(InterpreterError::new(
                        "MIR collection property contains another value type",
                    ));
                };
                if value.ty != collection {
                    return Err(InterpreterError::new(
                        "MIR collection property has another collection type",
                    ));
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Collection(value));
            }
            mir::CollectionExpression::SetFrom {
                collection,
                source,
                transfer,
                algebra,
            } => {
                if let Some((op, right)) = algebra {
                    let left = self.collection_local(source)?.clone();
                    let right = self.collection_local(right)?.clone();
                    let value_type = self.program.collection_types[left.ty.0].value;
                    let left_entries = left.entries();
                    let right_entries = right.entries();
                    let mut entries = Vec::new();
                    for (_, value) in left_entries.iter() {
                        let include = match op {
                            mir::SetAlgebraOp::Union => true,
                            mir::SetAlgebraOp::Intersect => {
                                right_entries.iter().any(|(_, candidate)| {
                                    collection_values_equal(value_type, candidate, value)
                                })
                            }
                            mir::SetAlgebraOp::Difference => {
                                !right_entries.iter().any(|(_, candidate)| {
                                    collection_values_equal(value_type, candidate, value)
                                })
                            }
                        };
                        if include {
                            entries.push((None, value.clone()));
                        }
                    }
                    if op == mir::SetAlgebraOp::Union {
                        for (_, value) in right_entries.iter() {
                            if !entries.iter().any(
                                |(_, candidate): &(Option<LocalValue>, LocalValue)| {
                                    collection_values_equal(value_type, candidate, value)
                                },
                            ) {
                                entries.push((None, value.clone()));
                            }
                        }
                    }
                    self.current_frame_mut()?
                        .values
                        .push(EvaluationValue::Collection(CollectionValue::new(
                            collection, entries,
                        )));
                    return Ok(());
                }
                let source = if transfer {
                    self.current_frame_mut()?
                        .locals
                        .get_mut(source.0)
                        .and_then(Option::take)
                        .ok_or_else(|| {
                            InterpreterError::new("Set::from source was moved before use")
                        })?
                } else {
                    read_local(&self.current_frame()?.locals, source)?.clone()
                };
                let LocalValue::Collection(source) = source else {
                    return Err(InterpreterError::new(
                        "Set::from source is not a collection",
                    ));
                };
                let mut entries = Vec::new();
                let mut drops = Vec::new();
                let source_entries = source.entries().clone();
                for (_, value) in source_entries {
                    if !entries
                        .iter()
                        .any(|(_, current): &(Option<LocalValue>, LocalValue)| current == &value)
                    {
                        entries.push((None, value));
                    } else {
                        collect_owned_objects_from_value(value, &mut drops);
                    }
                }
                for (object, class) in drops {
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::DropObject { object, class });
                }
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Collection(CollectionValue::new(
                        collection, entries,
                    )));
            }
            mir::CollectionExpression::FromBytes { collection, source }
            | mir::CollectionExpression::BytesFromArray { collection, source } => {
                let entries = self.collection_local(source)?.entries().clone();
                self.current_frame_mut()?
                    .values
                    .push(EvaluationValue::Collection(CollectionValue::new(
                        collection, entries,
                    )));
            }
            mir::CollectionExpression::ReadFileBytes { collection, path } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::ReadFileBytes(collection));
                frame.tasks.push(EvaluationTask::String(*path));
            }
            mir::CollectionExpression::ReadStdinBytes { collection } => {
                let remaining = self.stdin[self.stdin_cursor..].to_vec();
                self.stdin_cursor = self.stdin.len();
                self.push_byte_collection(collection, &remaining)?;
            }
            mir::CollectionExpression::Call {
                collection,
                function,
                args,
            } => {
                self.queue_call(
                    function,
                    args,
                    ReturnExpectation::Value(mir::Type::Collection(collection)),
                )?;
            }
        }
        Ok(())
    }

    fn expand_nullable_class_expression(
        &mut self,
        expression: mir::NullableClassExpression,
    ) -> Result<(), InterpreterError> {
        let class = expression.class();
        match expression {
            mir::NullableClassExpression::Null(_) => self.push_nullable_class(class, None)?,
            mir::NullableClassExpression::Class(value) => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::BuildNullableClassSome(class));
                frame.tasks.push(EvaluationTask::Class(value));
            }
            mir::NullableClassExpression::Local {
                local, transfer, ..
            } => {
                let value = if transfer {
                    self.current_frame_mut()?
                        .locals
                        .get_mut(local.0)
                        .and_then(Option::take)
                        .ok_or_else(|| {
                            InterpreterError::new("nullable class was moved before use")
                        })?
                } else {
                    read_local(&self.current_frame()?.locals, local)?.clone()
                };
                let LocalValue::NullableClass {
                    object,
                    class: actual,
                } = value
                else {
                    return Err(InterpreterError::new(
                        "nullable class local has another type",
                    ));
                };
                if actual != class {
                    return Err(InterpreterError::new(
                        "nullable class local has another class",
                    ));
                }
                self.push_nullable_class(class, object)?;
            }
            mir::NullableClassExpression::Property {
                object, property, ..
            } => {
                let LocalValue::NullableClass {
                    object,
                    class: actual,
                } = self.read_property(object, property)?
                else {
                    return Err(InterpreterError::new(
                        "nullable class property has another type",
                    ));
                };
                if actual != class {
                    return Err(InterpreterError::new(
                        "nullable class property has another class",
                    ));
                }
                self.push_nullable_class(class, object)?;
            }
            mir::NullableClassExpression::Call { function, args, .. } => {
                self.queue_call(
                    function,
                    args,
                    ReturnExpectation::Value(mir::Type::NullableClass(class)),
                )?;
            }
            mir::NullableClassExpression::NullSafeProperty {
                object, property, ..
            } => {
                let owned_receiver = object.owned_temporary_class();
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::AfterNullSafeProperty {
                    property,
                    result: mir::Type::NullableClass(class),
                    owned_receiver,
                });
                frame.tasks.push(EvaluationTask::NullableClass(*object));
            }
            mir::NullableClassExpression::NullSafeCall {
                object,
                function,
                args,
                ..
            } => {
                let owned_receiver = object.owned_temporary_class();
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::AfterNullSafeCall {
                    function,
                    args,
                    result: mir::Type::NullableClass(class),
                    owned_receiver,
                });
                frame.tasks.push(EvaluationTask::NullableClass(*object));
            }
            mir::NullableClassExpression::Coalesce {
                left,
                right,
                transfer,
                ..
            } => {
                let left_owned = left.owned_temporary_class().is_some();
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::AfterNullableClassCoalesce {
                        right: *right,
                        left_owned,
                        transfer,
                    });
                frame.tasks.push(EvaluationTask::NullableClass(*left));
            }
            mir::NullableClassExpression::DictionaryGet {
                class,
                collection,
                key,
                access,
            } => {
                let frame = self.current_frame_mut()?;
                frame.tasks.push(EvaluationTask::DictionaryGet {
                    collection,
                    expected: mir::Type::Class(class),
                    access,
                });
                frame.tasks.push(EvaluationTask::Rvalue(*key));
            }
        }
        Ok(())
    }

    fn queue_value_assignment(
        &mut self,
        target: mir::LocalId,
        value: mir::ValueExpression,
    ) -> Result<(), InterpreterError> {
        let frame = self.current_frame_mut()?;
        frame.tasks.push(EvaluationTask::Assign(target));
        frame.tasks.push(EvaluationTask::Value(value));
        Ok(())
    }

    fn queue_call(
        &mut self,
        function: mir::FunctionId,
        args: Vec<mir::Rvalue>,
        expectation: ReturnExpectation,
    ) -> Result<(), InterpreterError> {
        let callee = function_in(self.program, function)?;
        let temporary_class_args = args
            .iter()
            .zip(&callee.params)
            .map(|(argument, parameter)| {
                argument.owned_temporary_class().is_some()
                    && !local_in(callee, *parameter).is_ok_and(|local| local.owned)
            })
            .collect();
        let temporary_mixed_args = args
            .iter()
            .zip(&callee.params)
            .map(|(argument, parameter)| {
                argument.owned_temporary_mixed()
                    && !local_in(callee, *parameter).is_ok_and(|local| local.owned)
            })
            .collect();
        let frame = self.current_frame_mut()?;
        frame.tasks.push(EvaluationTask::Invoke {
            function,
            argument_count: args.len(),
            expectation,
            temporary_class_args,
            temporary_mixed_args,
        });
        for argument in args.into_iter().rev() {
            frame.tasks.push(EvaluationTask::Rvalue(argument));
        }
        Ok(())
    }

    fn queue_null_safe_call(
        &mut self,
        object: usize,
        class: crate::class_layout::ClassId,
        function: mir::FunctionId,
        args: Vec<mir::Rvalue>,
        nullable_result: mir::Type,
    ) -> Result<(), InterpreterError> {
        let callee = function_in(self.program, function)?;
        let non_nullable_result = non_nullable_type(nullable_result)
            .ok_or_else(|| InterpreterError::new("null-safe call result is not nullable"))?;
        let mir::ReturnType::Value(result) = callee.return_type else {
            return Err(InterpreterError::new(
                "null-safe value call targeted a void method",
            ));
        };
        if result != non_nullable_result && result != nullable_result {
            return Err(InterpreterError::new(
                "null-safe call result does not match the requested nullable type",
            ));
        }
        let mut temporary_class_args = Vec::with_capacity(args.len() + 1);
        temporary_class_args.push(false);
        temporary_class_args.extend(args.iter().zip(callee.params.iter().skip(1)).map(
            |(argument, parameter)| {
                argument.owned_temporary_class().is_some()
                    && !local_in(callee, *parameter).is_ok_and(|local| local.owned)
            },
        ));
        let mut temporary_mixed_args = Vec::with_capacity(args.len() + 1);
        temporary_mixed_args.push(false);
        temporary_mixed_args.extend(args.iter().zip(callee.params.iter().skip(1)).map(
            |(argument, parameter)| {
                argument.owned_temporary_mixed()
                    && !local_in(callee, *parameter).is_ok_and(|local| local.owned)
            },
        ));
        let frame = self.current_frame_mut()?;
        frame.values.push(EvaluationValue::Class { object, class });
        if result == non_nullable_result {
            frame
                .tasks
                .push(EvaluationTask::WrapNullable(nullable_result));
        }
        frame.tasks.push(EvaluationTask::Invoke {
            function,
            argument_count: args.len() + 1,
            expectation: ReturnExpectation::Value(result),
            temporary_class_args,
            temporary_mixed_args,
        });
        for argument in args.into_iter().rev() {
            frame.tasks.push(EvaluationTask::Rvalue(argument));
        }
        Ok(())
    }

    fn queue_null_safe_statement_call(
        &mut self,
        object: usize,
        class: crate::class_layout::ClassId,
        function: mir::FunctionId,
        args: Vec<mir::Rvalue>,
    ) -> Result<(), InterpreterError> {
        let callee = function_in(self.program, function)?;
        let expectation = match callee.return_type {
            mir::ReturnType::Void => ReturnExpectation::Void,
            mir::ReturnType::Value(ty) => ReturnExpectation::Discard(ty),
        };
        let mut temporary_class_args = Vec::with_capacity(args.len() + 1);
        temporary_class_args.push(false);
        temporary_class_args.extend(args.iter().zip(callee.params.iter().skip(1)).map(
            |(argument, parameter)| {
                argument.owned_temporary_class().is_some()
                    && !local_in(callee, *parameter).is_ok_and(|local| local.owned)
            },
        ));
        let mut temporary_mixed_args = Vec::with_capacity(args.len() + 1);
        temporary_mixed_args.push(false);
        temporary_mixed_args.extend(args.iter().zip(callee.params.iter().skip(1)).map(
            |(argument, parameter)| {
                argument.owned_temporary_mixed()
                    && !local_in(callee, *parameter).is_ok_and(|local| local.owned)
            },
        ));
        let frame = self.current_frame_mut()?;
        frame.values.push(EvaluationValue::Class { object, class });
        frame.tasks.push(EvaluationTask::Invoke {
            function,
            argument_count: args.len() + 1,
            expectation,
            temporary_class_args,
            temporary_mixed_args,
        });
        for argument in args.into_iter().rev() {
            frame.tasks.push(EvaluationTask::Rvalue(argument));
        }
        Ok(())
    }

    fn push_frame(
        &mut self,
        function_id: mir::FunctionId,
        args: &[LocalValue],
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
        for (parameter, value) in function.params.iter().zip(args.iter().cloned()) {
            let definition = local_in(function, *parameter)?;
            if local_value_type(&value) != definition.ty {
                return Err(InterpreterError::new(format!(
                    "MIR function {} parameter local{} expects {}, got {}",
                    function.name,
                    parameter.0,
                    definition.ty,
                    local_value_type(&value)
                )));
            }
            let _ = assign_local(&function.locals, &mut locals, *parameter, value)?;
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
            statement_temporary_drops: Vec::new(),
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
            (ReturnExpectation::Value(expected), FunctionOutcome::Value(value)) => {
                if local_value_type(&value) != expected {
                    return Err(InterpreterError::new(format!(
                        "MIR scalar call expected {expected}, returned {}",
                        local_value_type(&value)
                    )));
                }
                self.current_frame_mut()?.values.push(match value {
                    LocalValue::Scalar(value) => EvaluationValue::Scalar(value),
                    LocalValue::String(value) => EvaluationValue::String(value),
                    LocalValue::Mixed(value) => EvaluationValue::Mixed(value),
                    LocalValue::NullableScalar { ty, value } => {
                        EvaluationValue::NullableScalar { ty, value }
                    }
                    LocalValue::NullableString(value) => EvaluationValue::NullableString(value),
                    LocalValue::NullableMixed(value) => EvaluationValue::NullableMixed(value),
                    LocalValue::Class { object, class } => EvaluationValue::Class { object, class },
                    LocalValue::NullableClass { object, class } => {
                        EvaluationValue::NullableClass { object, class }
                    }
                    LocalValue::Collection(value) => EvaluationValue::Collection(value),
                });
            }
            (ReturnExpectation::Discard(expected), FunctionOutcome::Value(value)) => {
                if local_value_type(&value) != expected {
                    return Err(InterpreterError::new(format!(
                        "MIR discarded call expected {expected}, returned {}",
                        local_value_type(&value)
                    )));
                }
            }
            (ReturnExpectation::Void, FunctionOutcome::Void) => {}
            (
                ReturnExpectation::Value(_) | ReturnExpectation::Discard(_),
                FunctionOutcome::Void,
            ) => {
                return Err(InterpreterError::new(
                    "MIR scalar call returned a void value",
                ));
            }
            (ReturnExpectation::Void, FunctionOutcome::Value(_)) => {
                return Err(InterpreterError::new(
                    "MIR void call returned a scalar value",
                ));
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

    fn take_call_arguments(&mut self, count: usize) -> Result<Vec<LocalValue>, InterpreterError> {
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
                EvaluationValue::Scalar(value) => Ok(LocalValue::Scalar(value)),
                EvaluationValue::String(value) => Ok(LocalValue::String(value)),
                EvaluationValue::Mixed(value) => Ok(LocalValue::Mixed(value)),
                EvaluationValue::NullableScalar { ty, value } => {
                    Ok(LocalValue::NullableScalar { ty, value })
                }
                EvaluationValue::NullableString(value) => Ok(LocalValue::NullableString(value)),
                EvaluationValue::NullableMixed(value) => Ok(LocalValue::NullableMixed(value)),
                EvaluationValue::Class { object, class } => Ok(LocalValue::Class { object, class }),
                EvaluationValue::NullableClass { object, class } => {
                    Ok(LocalValue::NullableClass { object, class })
                }
                EvaluationValue::Collection(value) => Ok(LocalValue::Collection(value)),
            })
            .collect()
    }

    fn push_scalar(&mut self, value: mir::ScalarValue) -> Result<(), InterpreterError> {
        self.current_frame_mut()?
            .values
            .push(EvaluationValue::Scalar(value));
        Ok(())
    }

    fn pop_scalar(&mut self) -> Result<mir::ScalarValue, InterpreterError> {
        match self.current_frame_mut()?.values.pop() {
            Some(EvaluationValue::Scalar(value)) => Ok(value),
            Some(EvaluationValue::String(_)) => Err(InterpreterError::new(
                "MIR scalar evaluation produced a string",
            )),
            Some(EvaluationValue::Mixed(_)) | Some(EvaluationValue::NullableMixed(_)) => Err(
                InterpreterError::new("MIR scalar evaluation produced a mixed value"),
            ),
            Some(EvaluationValue::NullableString(_)) => Err(InterpreterError::new(
                "MIR scalar evaluation produced a nullable string",
            )),
            Some(EvaluationValue::NullableScalar { .. })
            | Some(EvaluationValue::NullableClass { .. }) => Err(InterpreterError::new(
                "MIR scalar evaluation produced a nullable value",
            )),
            Some(EvaluationValue::Class { .. }) => Err(InterpreterError::new(
                "MIR scalar evaluation produced a class",
            )),
            Some(EvaluationValue::Collection(_)) => Err(InterpreterError::new(
                "MIR scalar evaluation produced a collection",
            )),
            None => Err(InterpreterError::new(
                "MIR scalar evaluation produced no value",
            )),
        }
    }

    fn push_string(&mut self, value: String) -> Result<(), InterpreterError> {
        self.current_frame_mut()?
            .values
            .push(EvaluationValue::String(value));
        Ok(())
    }

    fn pop_string(&mut self) -> Result<String, InterpreterError> {
        match self.current_frame_mut()?.values.pop() {
            Some(EvaluationValue::String(value)) => Ok(value),
            Some(EvaluationValue::Scalar(_)) => Err(InterpreterError::new(
                "MIR string evaluation produced a scalar",
            )),
            Some(EvaluationValue::Mixed(_)) | Some(EvaluationValue::NullableMixed(_)) => Err(
                InterpreterError::new("MIR string evaluation produced a mixed value"),
            ),
            Some(EvaluationValue::NullableString(_)) => Err(InterpreterError::new(
                "MIR string evaluation produced a nullable string",
            )),
            Some(EvaluationValue::NullableScalar { .. })
            | Some(EvaluationValue::NullableClass { .. }) => Err(InterpreterError::new(
                "MIR string evaluation produced a nullable value",
            )),
            Some(EvaluationValue::Class { .. }) => Err(InterpreterError::new(
                "MIR string evaluation produced a class",
            )),
            Some(EvaluationValue::Collection(_)) => Err(InterpreterError::new(
                "MIR string evaluation produced a collection",
            )),
            None => Err(InterpreterError::new(
                "MIR string evaluation produced no value",
            )),
        }
    }

    fn push_nullable_string(&mut self, value: Option<String>) -> Result<(), InterpreterError> {
        self.current_frame_mut()?
            .values
            .push(EvaluationValue::NullableString(value));
        Ok(())
    }

    fn push_mixed(&mut self, value: MixedValue) -> Result<(), InterpreterError> {
        self.current_frame_mut()?
            .values
            .push(EvaluationValue::Mixed(value));
        Ok(())
    }

    fn push_nullable_mixed(&mut self, value: Option<MixedValue>) -> Result<(), InterpreterError> {
        self.current_frame_mut()?
            .values
            .push(EvaluationValue::NullableMixed(value));
        Ok(())
    }

    fn pop_nullable_mixed(&mut self) -> Result<Option<MixedValue>, InterpreterError> {
        match self.current_frame_mut()?.values.pop() {
            Some(EvaluationValue::NullableMixed(value)) => Ok(value),
            Some(_) => Err(InterpreterError::new(
                "MIR nullable-mixed evaluation produced another value type",
            )),
            None => Err(InterpreterError::new(
                "MIR nullable-mixed evaluation produced no value",
            )),
        }
    }

    fn pop_nullable_string(&mut self) -> Result<Option<String>, InterpreterError> {
        match self.current_frame_mut()?.values.pop() {
            Some(EvaluationValue::NullableString(value)) => Ok(value),
            Some(_) => Err(InterpreterError::new(
                "MIR nullable-string evaluation produced another value type",
            )),
            None => Err(InterpreterError::new(
                "MIR nullable-string evaluation produced no value",
            )),
        }
    }

    fn push_nullable_scalar(
        &mut self,
        ty: mir::ScalarType,
        value: Option<mir::ScalarValue>,
    ) -> Result<(), InterpreterError> {
        if value.is_some_and(|value| value.ty() != ty) {
            return Err(InterpreterError::new(
                "nullable scalar payload type mismatch",
            ));
        }
        self.current_frame_mut()?
            .values
            .push(EvaluationValue::NullableScalar { ty, value });
        Ok(())
    }

    fn pop_nullable_scalar(
        &mut self,
    ) -> Result<(mir::ScalarType, Option<mir::ScalarValue>), InterpreterError> {
        match self.current_frame_mut()?.values.pop() {
            Some(EvaluationValue::NullableScalar { ty, value }) => Ok((ty, value)),
            Some(_) => Err(InterpreterError::new(
                "nullable scalar produced another value type",
            )),
            None => Err(InterpreterError::new("nullable scalar produced no value")),
        }
    }

    fn push_nullable_class(
        &mut self,
        class: crate::class_layout::ClassId,
        object: Option<usize>,
    ) -> Result<(), InterpreterError> {
        self.current_frame_mut()?
            .values
            .push(EvaluationValue::NullableClass { object, class });
        Ok(())
    }

    fn pop_nullable_class(
        &mut self,
    ) -> Result<(crate::class_layout::ClassId, Option<usize>), InterpreterError> {
        match self.current_frame_mut()?.values.pop() {
            Some(EvaluationValue::NullableClass { object, class }) => Ok((class, object)),
            Some(_) => Err(InterpreterError::new(
                "nullable class produced another value type",
            )),
            None => Err(InterpreterError::new("nullable class produced no value")),
        }
    }

    fn push_null(&mut self, ty: mir::Type) -> Result<(), InterpreterError> {
        match ty {
            mir::Type::NullableScalar(ty) => self.push_nullable_scalar(ty, None),
            mir::Type::NullableString => self.push_nullable_string(None),
            mir::Type::NullableMixed => self.push_nullable_mixed(None),
            mir::Type::NullableClass(class) => self.push_nullable_class(class, None),
            _ => Err(InterpreterError::new(
                "null result does not have nullable type",
            )),
        }
    }

    fn push_nullable_from_value(
        &mut self,
        nullable: mir::Type,
        value: LocalValue,
    ) -> Result<(), InterpreterError> {
        match (nullable, value) {
            (mir::Type::NullableScalar(ty), LocalValue::Scalar(value)) if value.ty() == ty => {
                self.push_nullable_scalar(ty, Some(value))
            }
            (mir::Type::NullableScalar(expected), LocalValue::NullableScalar { ty, value })
                if expected == ty =>
            {
                self.push_nullable_scalar(ty, value)
            }
            (mir::Type::NullableString, LocalValue::String(value)) => {
                self.push_nullable_string(Some(value))
            }
            (mir::Type::NullableString, LocalValue::NullableString(value)) => {
                self.push_nullable_string(value)
            }
            (mir::Type::NullableMixed, LocalValue::Mixed(value)) => {
                self.push_nullable_mixed(Some(value))
            }
            (mir::Type::NullableMixed, LocalValue::NullableMixed(value)) => {
                self.push_nullable_mixed(value)
            }
            (mir::Type::NullableClass(expected), LocalValue::Class { object, class })
                if expected == class =>
            {
                self.push_nullable_class(class, Some(object))
            }
            (mir::Type::NullableClass(expected), LocalValue::NullableClass { object, class })
                if expected == class =>
            {
                self.push_nullable_class(class, object)
            }
            _ => Err(InterpreterError::new(
                "cannot wrap value in requested nullable type",
            )),
        }
    }

    fn take_evaluation_values(
        &mut self,
        count: usize,
    ) -> Result<Vec<EvaluationValue>, InterpreterError> {
        let frame = self.current_frame_mut()?;
        if frame.values.len() < count {
            return Err(InterpreterError::new(
                "MIR format evaluation produced too few values",
            ));
        }
        Ok(frame.values.drain(frame.values.len() - count..).collect())
    }

    fn pop_local_value(&mut self) -> Result<LocalValue, InterpreterError> {
        match self.current_frame_mut()?.values.pop() {
            Some(EvaluationValue::Scalar(value)) => Ok(LocalValue::Scalar(value)),
            Some(EvaluationValue::String(value)) => Ok(LocalValue::String(value)),
            Some(EvaluationValue::Mixed(value)) => Ok(LocalValue::Mixed(value)),
            Some(EvaluationValue::NullableScalar { ty, value }) => {
                Ok(LocalValue::NullableScalar { ty, value })
            }
            Some(EvaluationValue::NullableString(value)) => Ok(LocalValue::NullableString(value)),
            Some(EvaluationValue::NullableMixed(value)) => Ok(LocalValue::NullableMixed(value)),
            Some(EvaluationValue::Class { object, class }) => {
                Ok(LocalValue::Class { object, class })
            }
            Some(EvaluationValue::NullableClass { object, class }) => {
                Ok(LocalValue::NullableClass { object, class })
            }
            Some(EvaluationValue::Collection(value)) => Ok(LocalValue::Collection(value)),
            None => Err(InterpreterError::new("MIR evaluation produced no value")),
        }
    }

    fn push_local_value(&mut self, value: LocalValue) -> Result<(), InterpreterError> {
        let value = match value {
            LocalValue::Scalar(value) => EvaluationValue::Scalar(value),
            LocalValue::String(value) => EvaluationValue::String(value),
            LocalValue::Mixed(value) => EvaluationValue::Mixed(value),
            LocalValue::NullableScalar { ty, value } => {
                EvaluationValue::NullableScalar { ty, value }
            }
            LocalValue::NullableString(value) => EvaluationValue::NullableString(value),
            LocalValue::NullableMixed(value) => EvaluationValue::NullableMixed(value),
            LocalValue::Class { object, class } => EvaluationValue::Class { object, class },
            LocalValue::NullableClass { object, class } => {
                EvaluationValue::NullableClass { object, class }
            }
            LocalValue::Collection(value) => EvaluationValue::Collection(value),
        };
        self.current_frame_mut()?.values.push(value);
        Ok(())
    }

    fn queue_collection_scalar_operand(
        &mut self,
        operand: &mir::Operand,
    ) -> Result<bool, InterpreterError> {
        match operand {
            mir::Operand::CollectionLength(collection) => {
                self.current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::CollectionLength(*collection));
                Ok(true)
            }
            mir::Operand::CollectionIndex {
                collection,
                index,
                remove,
            } => {
                let frame = self.current_frame_mut()?;
                if *remove {
                    frame.tasks.push(EvaluationTask::LoadCollectionValue {
                        collection: *collection,
                        transfer: true,
                    });
                } else {
                    frame
                        .tasks
                        .push(EvaluationTask::CollectionIndexScalar(*collection));
                }
                frame.tasks.push(EvaluationTask::Rvalue((**index).clone()));
                Ok(true)
            }
            mir::Operand::CollectionKeyAt { collection, offset } => {
                let frame = self.current_frame_mut()?;
                frame
                    .tasks
                    .push(EvaluationTask::CollectionKeyScalar(*collection));
                frame.tasks.push(EvaluationTask::Rvalue((**offset).clone()));
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn pop_integer(&mut self) -> Result<IntegerValue, InterpreterError> {
        match self.pop_scalar()? {
            mir::ScalarValue::Integer(value) => Ok(value),
            _ => Err(InterpreterError::new(
                "MIR integer evaluation produced another scalar type",
            )),
        }
    }

    fn pop_float(&mut self) -> Result<FloatValue, InterpreterError> {
        match self.pop_scalar()? {
            mir::ScalarValue::Float(value) => Ok(value),
            _ => Err(InterpreterError::new(
                "MIR float evaluation produced another scalar type",
            )),
        }
    }

    fn pop_bool(&mut self) -> Result<bool, InterpreterError> {
        match self.pop_scalar()? {
            mir::ScalarValue::Bool(value) => Ok(value),
            _ => Err(InterpreterError::new(
                "MIR bool evaluation produced another scalar type",
            )),
        }
    }

    fn eval_operand(&self, operand: &mir::Operand) -> Result<mir::ScalarValue, InterpreterError> {
        match operand {
            mir::Operand::Scalar(value) => Ok(*value),
            mir::Operand::Local(id) => match read_local(&self.current_frame()?.locals, *id)? {
                LocalValue::Scalar(value) => Ok(*value),
                LocalValue::String(_) => Err(InterpreterError::new(format!(
                    "MIR string local local{} was used as a scalar value",
                    id.0
                ))),
                LocalValue::Mixed(_) | LocalValue::NullableMixed(_) => Err(InterpreterError::new(
                    format!("MIR mixed local local{} was used as a scalar value", id.0),
                )),
                LocalValue::NullableString(_) => Err(InterpreterError::new(format!(
                    "MIR nullable-string local local{} was used as a scalar value",
                    id.0
                ))),
                LocalValue::NullableScalar { .. } | LocalValue::NullableClass { .. } => {
                    Err(InterpreterError::new(format!(
                        "MIR nullable local local{} was used as a scalar value",
                        id.0
                    )))
                }
                LocalValue::Class { .. } => Err(InterpreterError::new(format!(
                    "MIR class local local{} was used as a scalar value",
                    id.0
                ))),
                LocalValue::Collection(_) => Err(InterpreterError::new(format!(
                    "MIR collection local local{} was used as a scalar value",
                    id.0
                ))),
            },
            mir::Operand::NullablePayload(id) => {
                match read_local(&self.current_frame()?.locals, *id)? {
                    LocalValue::NullableScalar {
                        value: Some(value), ..
                    } => Ok(*value),
                    LocalValue::NullableScalar { value: None, .. } => Err(InterpreterError::new(
                        "MIR nullable payload was read while null",
                    )),
                    _ => Err(InterpreterError::new(
                        "MIR nullable payload has another type",
                    )),
                }
            }
            mir::Operand::Static(id) => match self.statics.get(id.0) {
                Some(LocalValue::Scalar(value)) => Ok(*value),
                _ => Err(InterpreterError::new(format!(
                    "MIR static{} was used as scalar",
                    id.0
                ))),
            },
            mir::Operand::Property { object, property } => {
                match self.read_property(*object, *property)? {
                    LocalValue::Scalar(value) => Ok(value),
                    _ => Err(InterpreterError::new(format!(
                        "MIR property{} was used as a scalar value",
                        property.index
                    ))),
                }
            }
            mir::Operand::CollectionLength(_)
            | mir::Operand::CollectionIndex { .. }
            | mir::Operand::CollectionKeyAt { .. } => Err(InterpreterError::new(
                "MIR collection operand requires queued evaluation",
            )),
            mir::Operand::MixedPayload { mixed, tag } => {
                let value =
                    mixed_value_from_local(read_local(&self.current_frame()?.locals, *mixed)?)
                        .ok_or_else(|| {
                            InterpreterError::new(
                                "MIR mixed scalar payload references another local type",
                            )
                        })?;
                if value.tag() != *tag {
                    return Err(InterpreterError::new(
                        "MIR mixed scalar payload observed another tag",
                    ));
                }
                let MixedValue::Scalar(value) = value else {
                    return Err(InterpreterError::new(
                        "MIR mixed scalar payload observed non-scalar payload",
                    ));
                };
                Ok(*value)
            }
        }
    }

    fn read_property(
        &self,
        object: mir::LocalId,
        property: crate::class_layout::PropertyId,
    ) -> Result<LocalValue, InterpreterError> {
        let object_id = match read_local(&self.current_frame()?.locals, object)? {
            LocalValue::Class { object, .. } => *object,
            LocalValue::NullableClass {
                object: Some(object),
                ..
            } => *object,
            _ => {
                return Err(InterpreterError::new(format!(
                    "MIR property access uses non-class local local{}",
                    object.0
                )))
            }
        };
        self.read_object_property(object_id, property)
    }

    fn read_object_property(
        &self,
        object_id: usize,
        property: crate::class_layout::PropertyId,
    ) -> Result<LocalValue, InterpreterError> {
        let object_value = self.heap.get(&object_id).ok_or_else(|| {
            InterpreterError::new(format!("MIR object {object_id} is not allocated"))
        })?;
        if object_value.class != property.class {
            return Err(InterpreterError::new(format!(
                "MIR property access expected class#{} but object has class#{}",
                property.class.0, object_value.class.0
            )));
        }
        object_value
            .properties
            .get(property.index)
            .and_then(|value| value.clone())
            .ok_or_else(|| {
                InterpreterError::new(format!(
                    "MIR property{} was read before assignment",
                    property.index
                ))
            })
    }

    fn assign_property(
        &mut self,
        object: mir::LocalId,
        property: crate::class_layout::PropertyId,
        value: LocalValue,
    ) -> Result<Option<LocalValue>, InterpreterError> {
        let object_id = match read_local(&self.current_frame()?.locals, object)? {
            LocalValue::Class { object, .. } => *object,
            LocalValue::NullableClass {
                object: Some(object),
                ..
            } => *object,
            _ => {
                return Err(InterpreterError::new(format!(
                    "MIR property assignment uses non-class local local{}",
                    object.0
                )))
            }
        };
        let object_value = self.heap.get_mut(&object_id).ok_or_else(|| {
            InterpreterError::new(format!("MIR object {object_id} is not allocated"))
        })?;
        if object_value.class != property.class {
            return Err(InterpreterError::new(format!(
                "MIR property assignment expected class#{} but object has class#{}",
                property.class.0, object_value.class.0
            )));
        }
        let slot = object_value
            .properties
            .get_mut(property.index)
            .ok_or_else(|| {
                InterpreterError::new(format!("MIR property{} does not exist", property.index))
            })?;
        Ok(slot.replace(value))
    }

    fn drop_class_local(&mut self, local: mir::LocalId) -> Result<(), InterpreterError> {
        let Some(value) = self
            .current_frame_mut()?
            .locals
            .get_mut(local.0)
            .ok_or_else(|| {
                InterpreterError::new(format!("MIR local local{} does not exist", local.0))
            })?
            .take()
        else {
            return Ok(());
        };
        match value {
            LocalValue::Class { object, class }
            | LocalValue::NullableClass {
                object: Some(object),
                class,
            } => self
                .current_frame_mut()?
                .tasks
                .push(EvaluationTask::DropObject { object, class }),
            LocalValue::NullableClass { object: None, .. } => {}
            _ => {
                return Err(InterpreterError::new(format!(
                    "MIR drop local{} did not contain a class value",
                    local.0
                )))
            }
        }
        Ok(())
    }

    fn queue_object_drop(
        &mut self,
        object: usize,
        class: crate::class_layout::ClassId,
    ) -> Result<(), InterpreterError> {
        let value = self.heap.get(&object).ok_or_else(|| {
            InterpreterError::new(format!("MIR object {object} is not allocated"))
        })?;
        if value.class != class {
            return Err(InterpreterError::new(format!(
                "MIR drop expected class#{} but object has class#{}",
                class.0, value.class.0
            )));
        }
        let destructor = class_in(self.program, class)?.destructor;
        let frame = self.current_frame_mut()?;
        frame
            .tasks
            .push(EvaluationTask::FreeObject { object, class });
        frame
            .tasks
            .push(EvaluationTask::DropObjectProperties { object, class });
        if let Some(function) = destructor {
            self.push_frame(
                function,
                &[LocalValue::Class { object, class }],
                Some(ReturnExpectation::Void),
            )?;
        }
        Ok(())
    }

    fn queue_object_property_drops(
        &mut self,
        object: usize,
        class: crate::class_layout::ClassId,
    ) -> Result<(), InterpreterError> {
        let object_value = self.heap.get_mut(&object).ok_or_else(|| {
            InterpreterError::new(format!("MIR object {object} is not allocated"))
        })?;
        if object_value.class != class {
            return Err(InterpreterError::new(format!(
                "MIR property drop expected class#{} but object has class#{}",
                class.0, object_value.class.0
            )));
        }
        let mut drops = Vec::new();
        for property in object_value.properties.iter_mut().rev() {
            if let Some(value) = property.take() {
                collect_owned_objects_from_value(value, &mut drops);
            }
        }
        let frame = self.current_frame_mut()?;
        for (object, class) in drops.into_iter().rev() {
            frame
                .tasks
                .push(EvaluationTask::DropObject { object, class });
        }
        Ok(())
    }

    fn free_object(
        &mut self,
        object: usize,
        class: crate::class_layout::ClassId,
    ) -> Result<(), InterpreterError> {
        let Some(value) = self.heap.remove(&object) else {
            return Ok(());
        };
        if value.class != class {
            return Err(InterpreterError::new(format!(
                "MIR free expected class#{} but object has class#{}",
                class.0, value.class.0
            )));
        }
        Ok(())
    }

    fn cleanup_current_frame(&mut self) -> Result<(), InterpreterError> {
        let function = function_in(self.program, self.current_frame()?.function)?;
        let owned_classes = function
            .locals
            .iter()
            .filter_map(|local| match (local.owned, local.ty) {
                (true, mir::Type::Class(_)) => Some(local.id),
                _ => None,
            })
            .collect::<Vec<_>>();
        let owned_collections = function
            .locals
            .iter()
            .filter_map(|local| match (local.owned, local.ty) {
                (true, mir::Type::Collection(_)) => Some(local.id),
                _ => None,
            })
            .collect::<Vec<_>>();
        let owned_mixed = function
            .locals
            .iter()
            .filter_map(|local| match (local.owned, local.ty) {
                (true, mir::Type::Mixed | mir::Type::NullableMixed) => Some(local.id),
                _ => None,
            })
            .collect::<Vec<_>>();
        for local in owned_mixed {
            self.drop_mixed_local(local)?;
        }
        for local in owned_classes {
            self.drop_class_local(local)?;
        }
        for local in owned_collections {
            self.drop_collection_local(local)?;
        }
        Ok(())
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

    fn collection_local(&self, local: mir::LocalId) -> Result<&CollectionValue, InterpreterError> {
        match read_local(&self.current_frame()?.locals, local)? {
            LocalValue::Collection(value) => Ok(value),
            _ => Err(InterpreterError::new(format!(
                "MIR local local{} is not a collection",
                local.0
            ))),
        }
    }

    fn byte_collection(&self, local: mir::LocalId) -> Result<Vec<u8>, InterpreterError> {
        let collection = self.collection_local(local)?;
        let definition = self
            .program
            .collection_types
            .get(collection.ty.0)
            .ok_or_else(|| InterpreterError::new("Bytes type does not exist"))?;
        if definition.kind != mir::CollectionKind::Bytes {
            return Err(InterpreterError::new(
                "MIR Bytes operation used another collection",
            ));
        }
        collection
            .entries()
            .iter()
            .map(|(_, value)| match value {
                LocalValue::Scalar(mir::ScalarValue::Integer(value))
                    if value.ty == IntegerType::UInt8 =>
                {
                    Ok(value.unsigned_value() as u8)
                }
                _ => Err(InterpreterError::new(
                    "MIR Bytes contains a non-uint8 value",
                )),
            })
            .collect()
    }

    fn push_byte_collection(
        &mut self,
        collection: mir::CollectionTypeId,
        bytes: &[u8],
    ) -> Result<(), InterpreterError> {
        let entries = bytes
            .iter()
            .map(|byte| {
                (
                    None,
                    LocalValue::Scalar(mir::ScalarValue::Integer(
                        IntegerValue::from_u128(IntegerType::UInt8, u128::from(*byte))
                            .expect("u8 always fits uint8"),
                    )),
                )
            })
            .collect();
        self.current_frame_mut()?
            .values
            .push(EvaluationValue::Collection(CollectionValue::new(
                collection, entries,
            )));
        Ok(())
    }

    fn collection_position(
        &self,
        local: mir::LocalId,
        index: &LocalValue,
    ) -> Result<usize, String> {
        let collection = self
            .collection_local(local)
            .map_err(|error| error.message)?;
        let definition = self
            .program
            .collection_types
            .get(collection.ty.0)
            .ok_or_else(|| "collection type does not exist".to_string())?;
        if definition.key.is_some() {
            collection
                .entries()
                .iter()
                .position(|(key, _)| key.as_ref() == Some(index))
                .ok_or_else(|| "dictionary key not found".to_string())
        } else {
            let LocalValue::Scalar(mir::ScalarValue::Integer(index)) = index else {
                return Err("collection index is not an integer".to_string());
            };
            let Some(index) = usize::try_from(index.signed_value()).ok() else {
                return Err(if definition.kind == mir::CollectionKind::Bytes {
                    "byte index out of bounds".to_string()
                } else {
                    "collection index out of bounds".to_string()
                });
            };
            (index < collection.entries().len())
                .then_some(index)
                .ok_or_else(|| {
                    if definition.kind == mir::CollectionKind::Bytes {
                        "byte index out of bounds".to_string()
                    } else {
                        "collection index out of bounds".to_string()
                    }
                })
        }
    }

    fn collection_value_at(
        &mut self,
        local: mir::LocalId,
        index: &LocalValue,
        transfer: bool,
    ) -> Result<LocalValue, String> {
        let position = self.collection_position(local, index)?;
        if transfer {
            self.collection_local(local)
                .map(|collection| collection.entries_mut().remove(position).1)
                .map_err(|error| error.message)
        } else {
            self.collection_local(local)
                .map(|collection| collection.entries()[position].1.clone())
                .map_err(|error| error.message)
        }
    }

    fn pop_collection_offset(&mut self) -> Result<usize, InterpreterError> {
        let value = self.pop_integer()?;
        usize::try_from(value.signed_value())
            .map_err(|_| InterpreterError::new("MIR collection offset is negative"))
    }

    fn collection_key_at(
        &self,
        local: mir::LocalId,
        offset: usize,
    ) -> Result<LocalValue, InterpreterError> {
        self.collection_local(local)?
            .entries()
            .get(offset)
            .and_then(|(key, _)| key.clone())
            .ok_or_else(|| InterpreterError::new("MIR dictionary key offset is out of bounds"))
    }

    fn drop_collection_local(&mut self, local: mir::LocalId) -> Result<(), InterpreterError> {
        let value = self
            .current_frame_mut()?
            .locals
            .get_mut(local.0)
            .ok_or_else(|| InterpreterError::new("collection local does not exist"))?
            .take();
        if let Some(LocalValue::Collection(collection)) = value {
            let mut drops = Vec::new();
            collect_owned_objects_from_collection(collection, &mut drops);
            for (object, class) in drops {
                self.current_frame_mut()?
                    .tasks
                    .push(EvaluationTask::DropObject { object, class });
            }
        }
        Ok(())
    }

    fn drop_mixed_local(&mut self, local: mir::LocalId) -> Result<(), InterpreterError> {
        let value = self
            .current_frame_mut()?
            .locals
            .get_mut(local.0)
            .ok_or_else(|| InterpreterError::new("mixed local does not exist"))?
            .take();
        if let Some(value @ (LocalValue::Mixed(_) | LocalValue::NullableMixed(_))) = value {
            self.queue_value_drops(value)?;
        }
        Ok(())
    }

    fn queue_value_drops(&mut self, value: LocalValue) -> Result<(), InterpreterError> {
        let mut drops = Vec::new();
        collect_owned_objects_from_value(value, &mut drops);
        for (object, class) in drops {
            self.current_frame_mut()?
                .tasks
                .push(EvaluationTask::DropObject { object, class });
        }
        Ok(())
    }

    fn finish_entry(
        &self,
        entry: &mir::Function,
        outcome: FunctionOutcome,
    ) -> Result<InterpreterOutput, InterpreterError> {
        match (entry.return_type, outcome) {
            (
                mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Integer(
                    IntegerType::Int64,
                ))),
                FunctionOutcome::Value(LocalValue::Scalar(mir::ScalarValue::Integer(value))),
            ) if value.ty == IntegerType::Int64 => {
                let value = value.signed_value();
                if (0..=125).contains(&value) {
                    Ok(InterpreterOutput {
                        stdout: self.stdout.clone(),
                        stderr: self.stderr.clone(),
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
                stderr: self.stderr.clone(),
                exit_status: 0,
            }),
            (mir::ReturnType::Value(_), FunctionOutcome::Void) => Err(InterpreterError::new(
                "MIR scalar entry function returned void",
            )),
            (mir::ReturnType::Void, FunctionOutcome::Value(_)) => Err(InterpreterError::new(
                "MIR void entry function returned a scalar value",
            )),
            (mir::ReturnType::Value(ty), FunctionOutcome::Value(value)) => {
                Err(InterpreterError::new(format!(
                    "MIR entry must return int, but signature/value were {ty}/{}",
                    local_value_type(&value)
                )))
            }
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
        stderr.extend_from_slice(&self.stderr);
        stderr.extend_from_slice(b"Panic: ");
        stderr.extend_from_slice(message.as_bytes());
        stderr.extend_from_slice(b"\nStack Trace:\n");
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

fn eval_compare(
    op: mir::CompareOp,
    left: mir::ScalarValue,
    right: mir::ScalarValue,
) -> Result<bool, InterpreterError> {
    let result = match (left, right) {
        (mir::ScalarValue::Integer(left), mir::ScalarValue::Integer(right))
            if left.ty == right.ty =>
        {
            let ordering = left.compare(right);
            match op {
                mir::CompareOp::Equal => ordering.is_eq(),
                mir::CompareOp::NotEqual => !ordering.is_eq(),
                mir::CompareOp::Less => ordering.is_lt(),
                mir::CompareOp::LessEqual => !ordering.is_gt(),
                mir::CompareOp::Greater => ordering.is_gt(),
                mir::CompareOp::GreaterEqual => !ordering.is_lt(),
            }
        }
        (mir::ScalarValue::Float(left), mir::ScalarValue::Float(right)) if left.ty == right.ty => {
            match op {
                mir::CompareOp::Equal => left.compare_equal(right),
                mir::CompareOp::NotEqual => left.compare_not_equal(right),
                mir::CompareOp::Less => left.compare_less(right),
                mir::CompareOp::LessEqual => left.compare_less_equal(right),
                mir::CompareOp::Greater => left.compare_greater(right),
                mir::CompareOp::GreaterEqual => left.compare_greater_equal(right),
            }
        }
        (mir::ScalarValue::Bool(left), mir::ScalarValue::Bool(right)) => match op {
            mir::CompareOp::Equal => left == right,
            mir::CompareOp::NotEqual => left != right,
            _ => {
                return Err(InterpreterError::new(
                    "MIR ordered bool comparison is invalid",
                ))
            }
        },
        _ => {
            return Err(InterpreterError::new(
                "MIR comparison operands have different scalar types",
            ))
        }
    };
    Ok(result)
}

fn eval_unary(
    op: mir::IntegerUnaryOp,
    operand: IntegerValue,
) -> Result<IntegerValue, IntegerPanic> {
    match op {
        mir::IntegerUnaryOp::Negate => operand.checked_neg(),
        mir::IntegerUnaryOp::BitwiseNot => Ok(operand.bitwise_not()),
    }
}

fn eval_binary(
    op: mir::IntegerBinaryOp,
    left: IntegerValue,
    right: IntegerValue,
) -> Result<IntegerValue, IntegerPanic> {
    match op {
        mir::IntegerBinaryOp::Add => left.checked_add(right),
        mir::IntegerBinaryOp::Subtract => left.checked_sub(right),
        mir::IntegerBinaryOp::Multiply => left.checked_mul(right),
        mir::IntegerBinaryOp::Divide => left.divide(right),
        mir::IntegerBinaryOp::Remainder => left.remainder(right),
        mir::IntegerBinaryOp::ShiftLeft => left.shift_left(right),
        mir::IntegerBinaryOp::ShiftRight => left.shift_right(right),
        mir::IntegerBinaryOp::BitwiseAnd => Ok(left.bitwise_and(right)),
        mir::IntegerBinaryOp::BitwiseXor => Ok(left.bitwise_xor(right)),
        mir::IntegerBinaryOp::BitwiseOr => Ok(left.bitwise_or(right)),
    }
}

fn local_value_type(value: &LocalValue) -> mir::Type {
    match value {
        LocalValue::Scalar(value) => mir::Type::Scalar(value.ty()),
        LocalValue::String(_) => mir::Type::String,
        LocalValue::Mixed(_) => mir::Type::Mixed,
        LocalValue::NullableScalar { ty, .. } => mir::Type::NullableScalar(*ty),
        LocalValue::NullableString(_) => mir::Type::NullableString,
        LocalValue::NullableMixed(_) => mir::Type::NullableMixed,
        LocalValue::Class { class, .. } => mir::Type::Class(*class),
        LocalValue::NullableClass { class, .. } => mir::Type::NullableClass(*class),
        LocalValue::Collection(value) => mir::Type::Collection(value.ty),
    }
}

fn non_nullable_type(ty: mir::Type) -> Option<mir::Type> {
    match ty {
        mir::Type::NullableScalar(ty) => Some(mir::Type::Scalar(ty)),
        mir::Type::NullableString => Some(mir::Type::String),
        mir::Type::NullableMixed => Some(mir::Type::Mixed),
        mir::Type::NullableClass(class) => Some(mir::Type::Class(class)),
        mir::Type::Collection(_) => None,
        _ => None,
    }
}

fn owned_object(value: &LocalValue) -> Option<(usize, crate::class_layout::ClassId)> {
    match value {
        LocalValue::Class { object, class }
        | LocalValue::NullableClass {
            object: Some(object),
            class,
        } => Some((*object, *class)),
        LocalValue::Mixed(MixedValue::Class { object, class })
        | LocalValue::NullableMixed(Some(MixedValue::Class { object, class })) => {
            Some((*object, *class))
        }
        _ => None,
    }
}

fn collect_owned_objects_from_collection(
    collection: CollectionValue,
    drops: &mut Vec<(usize, crate::class_layout::ClassId)>,
) {
    for (key, value) in collection.entries().iter().cloned() {
        if let Some(key) = key {
            collect_owned_objects_from_value(key, drops);
        }
        collect_owned_objects_from_value(value, drops);
    }
}

fn collect_owned_objects_from_value(
    value: LocalValue,
    drops: &mut Vec<(usize, crate::class_layout::ClassId)>,
) {
    if let Some(object) = owned_object(&value) {
        drops.push(object);
    } else if let LocalValue::Collection(collection) = value {
        collect_owned_objects_from_collection(collection, drops);
    }
}

fn collection_values_equal(ty: mir::Type, left: &LocalValue, right: &LocalValue) -> bool {
    match (ty, left, right) {
        (
            mir::Type::Scalar(mir::ScalarType::Float(FloatType::Float32)),
            LocalValue::Scalar(mir::ScalarValue::Float(left)),
            LocalValue::Scalar(mir::ScalarValue::Float(right)),
        ) => left.as_f32() == right.as_f32(),
        (
            mir::Type::Scalar(mir::ScalarType::Float(FloatType::Float64)),
            LocalValue::Scalar(mir::ScalarValue::Float(left)),
            LocalValue::Scalar(mir::ScalarValue::Float(right)),
        ) => left.as_f64() == right.as_f64(),
        _ => left == right,
    }
}

fn display_scalar(value: mir::ScalarValue) -> String {
    match value {
        mir::ScalarValue::Integer(value) => value.display(),
        mir::ScalarValue::Bool(value) => value.to_string(),
        mir::ScalarValue::Float(value) => value.display(),
    }
}

fn render_format(
    format: &mir::FormatExpression,
    values: &[EvaluationValue],
) -> Result<String, InterpreterError> {
    use crate::format_string::{FormatConversion, FormatPiece};

    let mut output = String::new();
    for piece in &format.pieces {
        match piece {
            FormatPiece::Literal(value) => output.push_str(value),
            FormatPiece::Argument { index, spec } => {
                let value = values.get(*index as usize).ok_or_else(|| {
                    InterpreterError::new("MIR format argument index is out of bounds")
                })?;
                let rendered = match (spec.conversion, value) {
                    (FormatConversion::Display, EvaluationValue::Scalar(value)) => {
                        display_scalar(*value)
                    }
                    (FormatConversion::Display, EvaluationValue::String(value)) => value.clone(),
                    (
                        FormatConversion::Decimal,
                        EvaluationValue::Scalar(mir::ScalarValue::Integer(value)),
                    ) => {
                        if value.ty.is_signed() {
                            value.signed_value().to_string()
                        } else {
                            value.unsigned_value().to_string()
                        }
                    }
                    (
                        FormatConversion::HexLower
                        | FormatConversion::HexUpper
                        | FormatConversion::Octal
                        | FormatConversion::Binary,
                        EvaluationValue::Scalar(mir::ScalarValue::Integer(value)),
                    ) => format_integer_base(*value, spec.conversion),
                    (
                        FormatConversion::Float,
                        EvaluationValue::Scalar(mir::ScalarValue::Float(value)),
                    ) => format_fixed_float(*value, spec.precision.unwrap_or(6)),
                    _ => {
                        return Err(InterpreterError::new(
                            "MIR format conversion and argument type disagree",
                        ))
                    }
                };
                output.push_str(&apply_width(rendered, *spec));
            }
        }
    }
    Ok(output)
}

fn format_integer_base(
    value: IntegerValue,
    conversion: crate::format_string::FormatConversion,
) -> String {
    use crate::format_string::FormatConversion;
    let bits = value.unsigned_value();
    match conversion {
        FormatConversion::HexLower => format!("{bits:x}"),
        FormatConversion::HexUpper => format!("{bits:X}"),
        FormatConversion::Octal => format!("{bits:o}"),
        FormatConversion::Binary => format!("{bits:b}"),
        _ => unreachable!("only integer-base conversions reach this helper"),
    }
}

fn format_fixed_float(value: FloatValue, precision: u32) -> String {
    let precision = precision as usize;
    match value.ty {
        crate::numeric::FloatType::Float32 => {
            let value = value.as_f32();
            if value.is_nan() {
                "NaN".to_string()
            } else if value == f32::INFINITY {
                "Infinity".to_string()
            } else if value == f32::NEG_INFINITY {
                "-Infinity".to_string()
            } else {
                format!("{value:.precision$}")
            }
        }
        crate::numeric::FloatType::Float64 => {
            let value = value.as_f64();
            if value.is_nan() {
                "NaN".to_string()
            } else if value == f64::INFINITY {
                "Infinity".to_string()
            } else if value == f64::NEG_INFINITY {
                "-Infinity".to_string()
            } else {
                format!("{value:.precision$}")
            }
        }
    }
}

fn apply_width(mut value: String, spec: crate::format_string::FormatSpec) -> String {
    let width = spec.width.unwrap_or(0) as usize;
    if value.len() >= width {
        return value;
    }
    let padding = width - value.len();
    if spec.left_align {
        value.extend(core::iter::repeat_n(' ', padding));
        return value;
    }
    let fill = if spec.zero_pad { '0' } else { ' ' };
    if fill == '0' && value.starts_with('-') {
        let tail = value.split_off(1);
        let mut padded = String::with_capacity(width);
        padded.push('-');
        padded.extend(core::iter::repeat_n('0', padding));
        padded.push_str(&tail);
        padded
    } else {
        let mut padded = String::with_capacity(width);
        padded.extend(core::iter::repeat_n(fill, padding));
        padded.push_str(&value);
        padded
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
) -> Result<Option<LocalValue>, InterpreterError> {
    let definition = definitions
        .get(id.0)
        .filter(|local| local.id == id)
        .ok_or_else(|| InterpreterError::new(format!("MIR local local{} does not exist", id.0)))?;
    let compatible = matches!(
        (definition.ty, &value),
        (mir::Type::Scalar(expected), LocalValue::Scalar(actual)) if expected == actual.ty()
    ) || matches!(
        (definition.ty, &value),
        (mir::Type::String, LocalValue::String(_))
            | (mir::Type::NullableString, LocalValue::NullableString(_))
            | (mir::Type::Mixed, LocalValue::Mixed(_))
            | (mir::Type::NullableMixed, LocalValue::NullableMixed(_))
    ) || matches!(
        (definition.ty, &value),
        (mir::Type::NullableScalar(expected), LocalValue::NullableScalar { ty, .. }) if expected == *ty
    ) || matches!(
        (definition.ty, &value),
        (mir::Type::Class(expected), LocalValue::Class { class, .. }) if expected == *class
    ) || matches!(
        (definition.ty, &value),
        (mir::Type::NullableClass(expected), LocalValue::NullableClass { class, .. }) if expected == *class
    ) || matches!(
        (definition.ty, &value),
        (mir::Type::Collection(expected), LocalValue::Collection(collection)) if expected == collection.ty
    );
    if !compatible {
        let actual = match &value {
            LocalValue::Scalar(value) => match value.ty() {
                mir::ScalarType::Integer(value) => value.source_name(),
                mir::ScalarType::Float(value) => value.source_name(),
                mir::ScalarType::Bool => "bool",
            },
            LocalValue::String(_) => "string",
            LocalValue::Mixed(_) => "mixed",
            LocalValue::NullableScalar { .. } => "nullable scalar",
            LocalValue::NullableString(_) => "?string",
            LocalValue::NullableMixed(_) => "?mixed",
            LocalValue::Class { .. } => "class",
            LocalValue::NullableClass { .. } => "nullable class",
            LocalValue::Collection(_) => "collection",
        };
        return Err(InterpreterError::new(format!(
            "MIR local local{} has type {}, but assignment produced {actual}",
            id.0, definition.ty
        )));
    }
    let slot = locals
        .get_mut(id.0)
        .ok_or_else(|| InterpreterError::new(format!("MIR local local{} does not exist", id.0)))?;
    Ok(slot.replace(value))
}

fn mixed_value_from_local(value: &LocalValue) -> Option<&MixedValue> {
    match value {
        LocalValue::Mixed(value) => Some(value),
        LocalValue::NullableMixed(Some(value)) => Some(value),
        LocalValue::NullableMixed(None) => None,
        _ => None,
    }
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

fn class_in(
    program: &mir::Program,
    id: crate::class_layout::ClassId,
) -> Result<&mir::Class, InterpreterError> {
    program
        .classes
        .get(id.0)
        .filter(|class| class.id == id)
        .ok_or_else(|| InterpreterError::new(format!("MIR ClassId class{} does not exist", id.0)))
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

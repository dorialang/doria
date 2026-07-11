//! Unstable native-oriented MIR.
//!
//! MIR is the compiler-internal, backend-independent representation that Stage
//! 11 grows into the debug/interpreter oracle and future native lowering input.
//! The text dump is deterministic but not a stable public format.

use std::fmt;

use crate::numeric::{FloatType, FloatValue, IntegerType, IntegerValue};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FunctionId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LocalId(pub usize);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub functions: Vec<Function>,
    pub entry: FunctionId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    pub id: FunctionId,
    pub name: String,
    pub params: Vec<LocalId>,
    pub return_type: ReturnType,
    pub locals: Vec<Local>,
    pub blocks: Vec<BasicBlock>,
    pub entry_block: BlockId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReturnType {
    Value(ScalarType),
    Void,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalarType {
    Integer(IntegerType),
    Float(FloatType),
    Bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScalarValue {
    Integer(IntegerValue),
    Float(FloatValue),
    Bool(bool),
}

impl ScalarValue {
    pub const fn ty(self) -> ScalarType {
        match self {
            Self::Integer(value) => ScalarType::Integer(value.ty),
            Self::Float(value) => ScalarType::Float(value.ty),
            Self::Bool(_) => ScalarType::Bool,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Local {
    pub id: LocalId,
    pub name: String,
    pub ty: Type,
    pub writable: bool,
    pub synthetic: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    Scalar(ScalarType),
    String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasicBlock {
    pub id: BlockId,
    pub statements: Vec<Statement>,
    pub terminator: Terminator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operand {
    Scalar(ScalarValue),
    Local(LocalId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rvalue {
    Value(ValueExpression),
    String(StringExpression),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueExpression {
    Integer(IntegerExpression),
    Float(FloatExpression),
    Bool(BoolExpression),
}

impl ValueExpression {
    pub const fn ty(&self) -> ScalarType {
        match self {
            Self::Integer(value) => ScalarType::Integer(value.ty()),
            Self::Float(value) => ScalarType::Float(value.ty()),
            Self::Bool(_) => ScalarType::Bool,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegerUnaryOp {
    Negate,
    BitwiseNot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegerBinaryOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    ShiftLeft,
    ShiftRight,
    BitwiseAnd,
    BitwiseXor,
    BitwiseOr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntegerExpression {
    Use {
        ty: IntegerType,
        operand: Operand,
    },
    Unary {
        ty: IntegerType,
        op: IntegerUnaryOp,
        operand: Box<IntegerExpression>,
    },
    Binary {
        ty: IntegerType,
        op: IntegerBinaryOp,
        left: Box<IntegerExpression>,
        right: Box<IntegerExpression>,
    },
    Convert {
        ty: IntegerType,
        value: Box<IntegerExpression>,
    },
    FloatToInt {
        value: Box<FloatExpression>,
    },
    Call {
        ty: IntegerType,
        function: FunctionId,
        args: Vec<ValueExpression>,
    },
}

impl IntegerExpression {
    pub const fn ty(&self) -> IntegerType {
        match self {
            Self::Use { ty, .. }
            | Self::Unary { ty, .. }
            | Self::Binary { ty, .. }
            | Self::Convert { ty, .. }
            | Self::Call { ty, .. } => *ty,
            Self::FloatToInt { .. } => IntegerType::Int64,
        }
    }

    pub const fn use_operand(ty: IntegerType, operand: Operand) -> Self {
        Self::Use { ty, operand }
    }

    pub const fn constant(value: IntegerValue) -> Self {
        Self::Use {
            ty: value.ty,
            operand: Operand::Scalar(ScalarValue::Integer(value)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloatBinaryOp {
    Add,
    Subtract,
    Multiply,
    Divide,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FloatExpression {
    Use {
        ty: FloatType,
        operand: Operand,
    },
    Negate {
        ty: FloatType,
        operand: Box<FloatExpression>,
    },
    Binary {
        ty: FloatType,
        op: FloatBinaryOp,
        left: Box<FloatExpression>,
        right: Box<FloatExpression>,
    },
    IntToFloat {
        value: Box<IntegerExpression>,
    },
    Call {
        ty: FloatType,
        function: FunctionId,
        args: Vec<ValueExpression>,
    },
}

impl FloatExpression {
    pub const fn ty(&self) -> FloatType {
        match self {
            Self::Use { ty, .. }
            | Self::Negate { ty, .. }
            | Self::Binary { ty, .. }
            | Self::Call { ty, .. } => *ty,
            Self::IntToFloat { .. } => FloatType::Float64,
        }
    }

    pub const fn constant(value: FloatValue) -> Self {
        Self::Use {
            ty: value.ty,
            operand: Operand::Scalar(ScalarValue::Float(value)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StringExpression {
    Literal(String),
    Local(LocalId),
    Concat(Vec<StringExpression>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoolExpression {
    Use {
        operand: Operand,
    },
    Compare {
        op: CompareOp,
        left: Box<ValueExpression>,
        right: Box<ValueExpression>,
    },
    Not(Box<BoolExpression>),
    Binary {
        op: BoolBinaryOp,
        left: Box<BoolExpression>,
        right: Box<BoolExpression>,
    },
    Call {
        function: FunctionId,
        args: Vec<ValueExpression>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareOp {
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoolBinaryOp {
    And,
    Or,
    Xor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Statement {
    AssignLocal {
        target: LocalId,
        value: Rvalue,
    },
    EchoStringLiteral(String),
    EchoString(StringExpression),
    CallVoid {
        function: FunctionId,
        args: Vec<ValueExpression>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Terminator {
    Return(ValueExpression),
    ReturnVoid,
    Panic(StringExpression),
    Unreachable,
    Jump(BlockId),
    Branch {
        condition: BoolExpression,
        then_block: BlockId,
        else_block: BlockId,
    },
}

impl fmt::Display for Program {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, function) in self.functions.iter().enumerate() {
            if index > 0 {
                writeln!(formatter)?;
            }
            write!(formatter, "{function}")?;
        }
        Ok(())
    }
}

impl fmt::Display for Function {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "function {}(", self.name)?;
        for (index, parameter) in self.params.iter().enumerate() {
            if index > 0 {
                write!(formatter, ", ")?;
            }
            if let Some(local) = self
                .locals
                .get(parameter.0)
                .filter(|local| local.id == *parameter)
            {
                write!(formatter, "${}: {}", local.name, local.ty)?;
            } else {
                write!(formatter, "local{}", parameter.0)?;
            }
        }
        writeln!(formatter, "): {} {{", self.return_type)?;
        if !self.locals.is_empty() {
            writeln!(formatter, "locals:")?;
            for local in &self.locals {
                writeln!(formatter, "    {local}")?;
            }
        }
        for block in &self.blocks {
            write!(formatter, "{block}")?;
        }
        writeln!(formatter, "}}")
    }
}

impl fmt::Display for ReturnType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReturnType::Value(ty) => write!(formatter, "{ty}"),
            ReturnType::Void => write!(formatter, "void"),
        }
    }
}

impl fmt::Display for Local {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let role = if self.synthetic {
            "temp"
        } else if self.writable {
            "writable"
        } else {
            "readonly"
        };
        let name = if self.synthetic {
            self.name.clone()
        } else {
            format!("${}", self.name)
        };
        write!(
            formatter,
            "local{} {} {}: {}",
            self.id.0, role, name, self.ty
        )
    }
}

impl fmt::Display for Type {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Scalar(ty) => write!(formatter, "{ty}"),
            Type::String => write!(formatter, "string"),
        }
    }
}

impl fmt::Display for BasicBlock {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(formatter, "block{}:", self.id.0)?;
        for statement in &self.statements {
            writeln!(formatter, "    {statement}")?;
        }
        writeln!(formatter, "    {}", self.terminator)
    }
}

impl fmt::Display for Operand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Operand::Scalar(value) => write!(formatter, "{value}"),
            Operand::Local(id) => write!(formatter, "local{}", id.0),
        }
    }
}

impl fmt::Display for Rvalue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Rvalue::Value(expression) => write!(formatter, "{expression}"),
            Rvalue::String(value) => write!(formatter, "{value}"),
        }
    }
}

impl fmt::Display for ScalarType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Integer(ty) => write!(formatter, "{ty}"),
            Self::Float(ty) => write!(formatter, "{ty}"),
            Self::Bool => formatter.write_str("bool"),
        }
    }
}

impl fmt::Display for ScalarValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Integer(value) => write!(formatter, "{value}: {}", value.ty),
            Self::Float(value) => match value.ty {
                FloatType::Float32 => write!(formatter, "0x{:08x}: float32", value.bits),
                FloatType::Float64 => write!(formatter, "0x{:016x}: float", value.bits),
            },
            Self::Bool(value) => write!(formatter, "{value}: bool"),
        }
    }
}

impl fmt::Display for ValueExpression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Integer(value) => write!(formatter, "{value}"),
            Self::Float(value) => write!(formatter, "{value}"),
            Self::Bool(value) => write!(formatter, "{value}"),
        }
    }
}

impl fmt::Display for IntegerUnaryOp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IntegerUnaryOp::Negate => write!(formatter, "-"),
            IntegerUnaryOp::BitwiseNot => write!(formatter, "~"),
        }
    }
}

impl fmt::Display for IntegerBinaryOp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IntegerBinaryOp::Add => write!(formatter, "+"),
            IntegerBinaryOp::Subtract => write!(formatter, "-"),
            IntegerBinaryOp::Multiply => write!(formatter, "*"),
            IntegerBinaryOp::Divide => write!(formatter, "/"),
            IntegerBinaryOp::Remainder => write!(formatter, "%"),
            IntegerBinaryOp::ShiftLeft => write!(formatter, "<<"),
            IntegerBinaryOp::ShiftRight => write!(formatter, ">>"),
            IntegerBinaryOp::BitwiseAnd => write!(formatter, "&"),
            IntegerBinaryOp::BitwiseXor => write!(formatter, "^"),
            IntegerBinaryOp::BitwiseOr => write!(formatter, "|"),
        }
    }
}

impl fmt::Display for IntegerExpression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IntegerExpression::Use { ty, operand } => match operand {
                Operand::Scalar(ScalarValue::Integer(value)) => write!(formatter, "{value}: {ty}"),
                Operand::Local(id) => write!(formatter, "local{}: {ty}", id.0),
                Operand::Scalar(_) => write!(formatter, "<malformed scalar>: {ty}"),
            },
            IntegerExpression::Unary { ty, op, operand } => {
                write!(formatter, "({op}{operand}): {ty}")
            }
            IntegerExpression::Binary {
                ty,
                op,
                left,
                right,
            } => write!(formatter, "({left} {op} {right}): {ty}"),
            IntegerExpression::Convert { ty, value } => {
                write!(formatter, "convert<{ty}>({value}): {ty}")
            }
            IntegerExpression::FloatToInt { value } => {
                write!(formatter, "Float::toInt({value}): int")
            }
            IntegerExpression::Call { ty, function, args } => {
                write_call(formatter, *function, args)?;
                write!(formatter, ": {ty}")
            }
        }
    }
}

impl fmt::Display for FloatBinaryOp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Add => "+",
            Self::Subtract => "-",
            Self::Multiply => "*",
            Self::Divide => "/",
        })
    }
}

impl fmt::Display for FloatExpression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Use { ty, operand } => match operand {
                Operand::Scalar(ScalarValue::Float(value)) => write!(formatter, "{value}: {ty}"),
                Operand::Local(id) => write!(formatter, "local{}: {ty}", id.0),
                Operand::Scalar(_) => write!(formatter, "<malformed scalar>: {ty}"),
            },
            Self::Negate { ty, operand } => write!(formatter, "(-{operand}): {ty}"),
            Self::Binary {
                ty,
                op,
                left,
                right,
            } => write!(formatter, "({left} {op} {right}): {ty}"),
            Self::IntToFloat { value } => write!(formatter, "Int::toFloat({value}): float"),
            Self::Call { ty, function, args } => {
                write_call(formatter, *function, args)?;
                write!(formatter, ": {ty}")
            }
        }
    }
}

impl fmt::Display for StringExpression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StringExpression::Literal(value) => {
                write!(formatter, "\"{}\"", escape_debug_string(value))
            }
            StringExpression::Local(id) => write!(formatter, "local{}", id.0),
            StringExpression::Concat(parts) => {
                write!(formatter, "(")?;
                for (index, part) in parts.iter().enumerate() {
                    if index > 0 {
                        write!(formatter, " . ")?;
                    }
                    write!(formatter, "{part}")?;
                }
                write!(formatter, ")")
            }
        }
    }
}

impl fmt::Display for BoolExpression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Use { operand } => match operand {
                Operand::Scalar(ScalarValue::Bool(value)) => write!(formatter, "{value}: bool"),
                Operand::Local(id) => write!(formatter, "local{}: bool", id.0),
                Operand::Scalar(_) => formatter.write_str("<malformed scalar>: bool"),
            },
            Self::Compare { op, left, right } => write!(formatter, "{left} {op} {right}"),
            Self::Not(condition) => write!(formatter, "!({condition})"),
            Self::Binary { op, left, right } => {
                write!(formatter, "({left}) {op} ({right})")
            }
            Self::Call { function, args } => write_call(formatter, *function, args),
        }
    }
}

impl fmt::Display for CompareOp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompareOp::Equal => write!(formatter, "=="),
            CompareOp::NotEqual => write!(formatter, "!="),
            CompareOp::Less => write!(formatter, "<"),
            CompareOp::LessEqual => write!(formatter, "<="),
            CompareOp::Greater => write!(formatter, ">"),
            CompareOp::GreaterEqual => write!(formatter, ">="),
        }
    }
}

impl fmt::Display for BoolBinaryOp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BoolBinaryOp::And => write!(formatter, "&&"),
            BoolBinaryOp::Or => write!(formatter, "||"),
            BoolBinaryOp::Xor => write!(formatter, "xor"),
        }
    }
}

impl fmt::Display for Statement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Statement::AssignLocal { target, value } => {
                write!(formatter, "local{} = {value}", target.0)
            }
            Statement::EchoStringLiteral(value) => {
                write!(formatter, "echo \"{}\"", escape_debug_string(value))
            }
            Statement::EchoString(value) => write!(formatter, "echo {value}"),
            Statement::CallVoid { function, args } => write_call(formatter, *function, args),
        }
    }
}

impl fmt::Display for Terminator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Terminator::Return(operand) => write!(formatter, "return {operand}"),
            Terminator::ReturnVoid => write!(formatter, "return"),
            Terminator::Panic(message) => write!(formatter, "panic {message}"),
            Terminator::Unreachable => write!(formatter, "unreachable"),
            Terminator::Jump(target) => write!(formatter, "jump block{}", target.0),
            Terminator::Branch {
                condition,
                then_block,
                else_block,
            } => write!(
                formatter,
                "branch {condition} -> block{}, block{}",
                then_block.0, else_block.0
            ),
        }
    }
}

fn escape_debug_string(value: &str) -> String {
    value.escape_default().collect()
}

fn write_call<T: fmt::Display>(
    formatter: &mut fmt::Formatter<'_>,
    function: FunctionId,
    args: &[T],
) -> fmt::Result {
    write!(formatter, "call function{}(", function.0)?;
    for (index, arg) in args.iter().enumerate() {
        if index > 0 {
            write!(formatter, ", ")?;
        }
        write!(formatter, "{arg}")?;
    }
    write!(formatter, ")")
}

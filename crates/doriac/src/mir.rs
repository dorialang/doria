//! Unstable native-oriented MIR.
//!
//! MIR is the compiler-internal, backend-independent representation that Stage
//! 11 grows into the debug/interpreter oracle and future native lowering input.
//! The text dump is deterministic but not a stable public format.

use std::fmt;

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
    Int,
    Void,
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
    Int,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasicBlock {
    pub id: BlockId,
    pub statements: Vec<Statement>,
    pub terminator: Terminator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operand {
    Int(i64),
    Local(LocalId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rvalue {
    Use(Operand),
    Binary {
        op: BinaryOp,
        left: Operand,
        right: Operand,
    },
    Call {
        function: FunctionId,
        args: Vec<IntExpression>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Subtract,
    Multiply,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntExpression {
    Use(Operand),
    Binary {
        op: BinaryOp,
        left: Box<IntExpression>,
        right: Box<IntExpression>,
    },
    Call {
        function: FunctionId,
        args: Vec<IntExpression>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Condition {
    Bool(bool),
    Compare {
        op: CompareOp,
        left: IntExpression,
        right: IntExpression,
    },
    Not(Box<Condition>),
    Binary {
        op: ConditionBinaryOp,
        left: Box<Condition>,
        right: Box<Condition>,
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
pub enum ConditionBinaryOp {
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
    CallVoid {
        function: FunctionId,
        args: Vec<IntExpression>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Terminator {
    Return(Operand),
    ReturnVoid,
    Jump(BlockId),
    Branch {
        condition: Condition,
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
            ReturnType::Int => write!(formatter, "int"),
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
            Type::Int => write!(formatter, "int"),
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
            Operand::Int(value) => write!(formatter, "{value}"),
            Operand::Local(id) => write!(formatter, "local{}", id.0),
        }
    }
}

impl fmt::Display for Rvalue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Rvalue::Use(operand) => write!(formatter, "{operand}"),
            Rvalue::Binary { op, left, right } => write!(formatter, "{left} {op} {right}"),
            Rvalue::Call { function, args } => write_call(formatter, *function, args),
        }
    }
}

impl fmt::Display for BinaryOp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BinaryOp::Add => write!(formatter, "+"),
            BinaryOp::Subtract => write!(formatter, "-"),
            BinaryOp::Multiply => write!(formatter, "*"),
        }
    }
}

impl fmt::Display for IntExpression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IntExpression::Use(operand) => write!(formatter, "{operand}"),
            IntExpression::Binary { op, left, right } => {
                write!(formatter, "({left} {op} {right})")
            }
            IntExpression::Call { function, args } => write_call(formatter, *function, args),
        }
    }
}

impl fmt::Display for Condition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Condition::Bool(value) => write!(formatter, "{value}"),
            Condition::Compare { op, left, right } => write!(formatter, "{left} {op} {right}"),
            Condition::Not(condition) => write!(formatter, "!({condition})"),
            Condition::Binary { op, left, right } => {
                write!(formatter, "({left}) {op} ({right})")
            }
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

impl fmt::Display for ConditionBinaryOp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConditionBinaryOp::And => write!(formatter, "&&"),
            ConditionBinaryOp::Or => write!(formatter, "||"),
            ConditionBinaryOp::Xor => write!(formatter, "xor"),
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
            Statement::CallVoid { function, args } => write_call(formatter, *function, args),
        }
    }
}

impl fmt::Display for Terminator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Terminator::Return(operand) => write!(formatter, "return {operand}"),
            Terminator::ReturnVoid => write!(formatter, "return"),
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

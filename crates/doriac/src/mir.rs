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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub functions: Vec<Function>,
    pub entry: FunctionId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    pub id: FunctionId,
    pub name: String,
    pub return_type: ReturnType,
    pub blocks: Vec<BasicBlock>,
    pub entry_block: BlockId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReturnType {
    Int,
    Void,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasicBlock {
    pub id: BlockId,
    pub statements: Vec<Statement>,
    pub terminator: Terminator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Statement {
    EchoStringLiteral(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Terminator {
    ReturnInt(i64),
    ReturnVoid,
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
        writeln!(
            formatter,
            "function {}(): {} {{",
            self.name, self.return_type
        )?;
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

impl fmt::Display for BasicBlock {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(formatter, "block{}:", self.id.0)?;
        for statement in &self.statements {
            writeln!(formatter, "    {statement}")?;
        }
        writeln!(formatter, "    {}", self.terminator)
    }
}

impl fmt::Display for Statement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Statement::EchoStringLiteral(value) => {
                write!(formatter, "echo \"{}\"", escape_debug_string(value))
            }
        }
    }
}

impl fmt::Display for Terminator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Terminator::ReturnInt(value) => write!(formatter, "return {value}"),
            Terminator::ReturnVoid => write!(formatter, "return"),
        }
    }
}

fn escape_debug_string(value: &str) -> String {
    value.escape_default().collect()
}

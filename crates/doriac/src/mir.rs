//! Native-oriented IR design note.
//!
//! This module is intentionally a placeholder today. The current compiler
//! lowers the checked AST into Doria IR. A later native-oriented IR should
//! become the simpler, control-flow-oriented representation used by native,
//! WebAssembly, and interpreter/debug backends.
//!
//! Expected native-oriented IR responsibilities:
//!
//! - Explicit basic blocks and terminators.
//! - Local slots and temporaries instead of source variables.
//! - Resolved type IDs rather than parsed type references.
//! - Lowered method/function calls and constructor initialization.
//! - A stable boundary for ownership/borrow checking over MIR.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub functions: Vec<Function>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    pub name: String,
}

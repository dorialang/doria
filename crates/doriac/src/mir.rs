//! Mid-level IR design note.
//!
//! MIR is intentionally a placeholder today. The current compiler lowers the
//! checked AST into HIR, which is still source-shaped. MIR should become the
//! simpler, control-flow-oriented representation used by native, WebAssembly,
//! and interpreter/debug backends.
//!
//! Expected MIR responsibilities:
//!
//! - Explicit basic blocks and terminators.
//! - Local slots and temporaries instead of source variables.
//! - Resolved type IDs rather than parsed type references.
//! - Lowered method/function calls and constructor initialization.
//! - A stable boundary for borrow/lifetime analysis.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub functions: Vec<Function>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    pub name: String,
}

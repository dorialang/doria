//! Physical homes selected by an already checked argument expression.

use super::*;
use crate::hir::{Block, ElseBranch, IfStmt, Stmt};

pub(crate) fn argument_place_bindings(program: &hir::Program, expr: &Expr) -> Vec<BindingId> {
    let mut bindings = Vec::new();
    visit_value(expr, &mut |value| {
        if let Some(binding) = place_root_binding(program, value) {
            if !bindings.contains(&binding) {
                bindings.push(binding);
            }
        }
    });
    bindings
}

fn visit_value<'a>(expr: &'a Expr, visit: &mut impl FnMut(&'a Expr)) {
    match expr {
        Expr::Grouped { expr, .. } => visit_value(expr, visit),
        Expr::Binary {
            left,
            op: hir::BinaryOp::Coalesce,
            right,
            ..
        } => {
            visit_value(left, visit);
            visit_value(right, visit);
        }
        Expr::Match { arms, .. } => {
            for arm in arms {
                visit_value(&arm.value, visit);
            }
        }
        Expr::When(when) => {
            if let Some(given) = &when.given {
                visit_returns(&given.block, visit);
            }
            for branch in &when.branches {
                visit_returns(&branch.block, visit);
            }
        }
        _ => visit(expr),
    }
}

fn visit_if<'a>(statement: &'a IfStmt, visit: &mut impl FnMut(&'a Expr)) {
    if let Some(given) = &statement.given {
        visit_returns(&given.block, visit);
    }
    visit_returns(&statement.then_block, visit);
    match &statement.else_branch {
        Some(ElseBranch::If(statement)) => visit_if(statement, visit),
        Some(ElseBranch::Block(block)) => visit_returns(block, visit),
        None => {}
    }
    if let Some(finally) = &statement.finally {
        visit_returns(&finally.block, visit);
    }
}

// Do not enter expression statements or initializers: closures and nested
// when expressions there have their own return target.
fn visit_returns<'a>(block: &'a Block, visit: &mut impl FnMut(&'a Expr)) {
    for statement in &block.statements {
        match statement {
            Stmt::Return {
                expr: Some(expr), ..
            } => visit_value(expr, visit),
            Stmt::Block(block) => visit_returns(block, visit),
            Stmt::If(statement) => visit_if(statement, visit),
            Stmt::While(statement) => {
                if let Some(given) = &statement.given {
                    visit_returns(&given.block, visit);
                }
                visit_returns(&statement.body, visit);
                if let Some(finally) = &statement.finally {
                    visit_returns(&finally.block, visit);
                }
            }
            Stmt::DoWhile(statement) => {
                visit_returns(&statement.body, visit);
                if let Some(finally) = &statement.finally {
                    visit_returns(&finally.block, visit);
                }
            }
            Stmt::For(statement) => visit_returns(&statement.body, visit),
            Stmt::Foreach(statement) => visit_returns(&statement.body, visit),
            Stmt::Try(statement) => {
                visit_returns(&statement.body, visit);
                for catch in &statement.catches {
                    visit_returns(&catch.body, visit);
                }
                if let Some(finally) = &statement.finally {
                    visit_returns(&finally.body, visit);
                }
            }
            Stmt::Return { expr: None, .. }
            | Stmt::VarDecl(_)
            | Stmt::Assignment(_)
            | Stmt::Expr { .. }
            | Stmt::Echo { .. }
            | Stmt::Throw(_)
            | Stmt::Increment(_)
            | Stmt::Break { .. }
            | Stmt::Continue { .. } => {}
        }
    }
}

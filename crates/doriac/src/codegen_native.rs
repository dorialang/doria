use std::collections::HashMap;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{types, AbiParam, Block, BlockArg, InstBuilder, Value};
use cranelift_codegen::settings;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{default_libcall_names, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};

use crate::backend::BackendError;
use crate::hir::{self, AssignOp, BinaryOp, ElseBranch, Expr, Item, Stmt, UnaryOp};

const STAGE_6A_LOOP_VERIFICATION_CAP: u64 = 10_000;

pub fn generate_executable(program: &hir::Program) -> Result<Vec<u8>, BackendError> {
    let native_main = validate_stage_6a(program)?;
    let object_bytes = generate_object(&native_main)?;
    link_object(&object_bytes)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeMain {
    body: NativeBlock,
    evaluated_exit_code: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeBlock {
    statements: Vec<NativeStmt>,
    terminator: NativeTerminator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum NativeStmt {
    Local(NativeLocal),
    Assign(NativeAssign),
    While(NativeWhile),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeLocal {
    name: String,
    writable: bool,
    expr: NativeExpr,
    evaluated_value: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeAssign {
    target: String,
    op: NativeAssignOp,
    expr: NativeExpr,
    evaluated_value: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeWhile {
    condition: NativeCondition,
    body: Vec<NativeLoopAssign>,
    final_values: Vec<(String, i64)>,
    evaluated_iterations: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeLoopAssign {
    target: String,
    op: NativeAssignOp,
    expr: NativeExpr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeAssignOp {
    Assign,
    AddAssign,
    SubAssign,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum NativeExpr {
    Int(i64),
    Local(String),
    Binary {
        op: NativeBinaryOp,
        left: Box<NativeExpr>,
        right: Box<NativeExpr>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeBinaryOp {
    Add,
    Subtract,
    Multiply,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum NativeTerminator {
    Return {
        expr: NativeExpr,
        evaluated_exit_code: i32,
    },
    IfElse {
        condition: NativeCondition,
        evaluated_condition: bool,
        then_block: Box<NativeBlock>,
        else_block: Box<NativeBlock>,
    },
    Guard {
        condition: NativeCondition,
        evaluated_condition: bool,
        then_block: Box<NativeBlock>,
        fallback: Box<NativeBlock>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum NativeCondition {
    Bool(bool),
    Compare {
        op: NativeCompareOp,
        left: NativeExpr,
        right: NativeExpr,
    },
    Not(Box<NativeCondition>),
    And {
        left: Box<NativeCondition>,
        right: Box<NativeCondition>,
    },
    Or {
        left: Box<NativeCondition>,
        right: Box<NativeCondition>,
    },
    Xor {
        left: Box<NativeCondition>,
        right: Box<NativeCondition>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeCompareOp {
    Equal,
    NotEqual,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ValidatedNativeExpr {
    expr: NativeExpr,
    value: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ValidatedNativeCondition {
    condition: NativeCondition,
    value: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeLocalState {
    writable: bool,
    value: i64,
}

// Historical helper retained for existing callers; it validates against the
// current native smoke backend.
pub fn validate_stage_2d(program: &hir::Program) -> Result<i32, BackendError> {
    Ok(validate_stage_6a(program)?.evaluated_exit_code)
}

fn validate_stage_6a(program: &hir::Program) -> Result<NativeMain, BackendError> {
    let mut main_functions = Vec::new();

    for item in &program.items {
        match item {
            Item::Function(function) if function.name == "main" => {
                main_functions.push(function);
            }
            Item::Function(function) => {
                return Err(BackendError::new(format!(
                    "unsupported top-level item for current native smoke backend: extra top-level function `{}`",
                    function.name
                )));
            }
            Item::Class(class_decl) => {
                return Err(BackendError::new(format!(
                    "unsupported top-level item for current native smoke backend: class `{}`",
                    class_decl.name
                )));
            }
            Item::Statement(statement) => {
                return Err(BackendError::new(format!(
                    "unsupported top-level item for current native smoke backend: {}",
                    describe_statement(statement)
                )));
            }
        }
    }

    let [main] = main_functions.as_slice() else {
        return Err(match main_functions.len() {
            0 => BackendError::new(
                "no native entrypoint found; native Stage 6a output requires exactly one top-level `function main(): int`",
            ),
            _ => BackendError::new(
                "multiple native entrypoints found; native Stage 6a output requires exactly one top-level `function main(): int`",
            ),
        });
    };

    if !main.params.is_empty() {
        return Err(BackendError::new(
            "wrong main signature for native Stage 6a: `main` must not declare parameters",
        ));
    }

    if !matches!(
        main.return_type.as_ref(),
        Some(return_type) if return_type.name == "int" && return_type.args.is_empty()
    ) {
        return Err(BackendError::new(
            "wrong main signature for native Stage 6a: expected `function main(): int`",
        ));
    }

    let body = validate_stage_6a_block(&main.body.statements, &HashMap::new())?;
    let evaluated_exit_code = evaluate_native_block_exit_code(&body);
    Ok(NativeMain {
        body,
        evaluated_exit_code,
    })
}

fn validate_stage_6a_block(
    statements: &[Stmt],
    local_states: &HashMap<String, NativeLocalState>,
) -> Result<NativeBlock, BackendError> {
    let mut block_states = local_states.clone();
    let mut native_statements = Vec::new();
    let mut terminal_index = 0;

    while let Some(statement) = statements.get(terminal_index) {
        match statement {
            Stmt::VarDecl(decl) => {
                let local = validate_stage_6a_local(decl, &block_states)?;
                block_states.insert(
                    local.name.clone(),
                    NativeLocalState {
                        writable: local.writable,
                        value: local.evaluated_value,
                    },
                );
                native_statements.push(NativeStmt::Local(local));
                terminal_index += 1;
            }
            Stmt::Assignment(assignment) => {
                let assignment = validate_stage_6a_assignment(assignment, &block_states)?;
                let Some(state) = block_states.get_mut(&assignment.target) else {
                    return Err(BackendError::new(
                        "backend validation failure: validated native assignment target was not declared",
                    ));
                };
                state.value = assignment.evaluated_value;
                native_statements.push(NativeStmt::Assign(assignment));
                terminal_index += 1;
            }
            Stmt::While(while_stmt) => {
                let native_while = validate_stage_6a_while(while_stmt, &block_states)?;
                for (name, value) in &native_while.final_values {
                    let Some(state) = block_states.get_mut(name) else {
                        return Err(BackendError::new(
                            "backend validation failure: validated native while target was not declared",
                        ));
                    };
                    state.value = *value;
                }
                native_statements.push(NativeStmt::While(native_while));
                terminal_index += 1;
            }
            _ => break,
        }
    }

    let terminator =
        validate_stage_6a_statement_sequence(&statements[terminal_index..], &block_states)?;

    Ok(NativeBlock {
        statements: native_statements,
        terminator,
    })
}

fn validate_stage_6a_statement_sequence(
    statements: &[Stmt],
    local_states: &HashMap<String, NativeLocalState>,
) -> Result<NativeTerminator, BackendError> {
    match statements {
        [] => Err(BackendError::new(
            "unsupported native block for Stage 6a: expected supported local declarations or assignments followed by a return, terminal if/else, or guard if with fallback",
        )),
        [statement] => validate_stage_6a_terminator(statement, local_states),
        [Stmt::If(if_stmt), rest @ ..] if if_stmt.else_branch.is_none() => {
            validate_stage_6a_guard(if_stmt, rest, local_states)
        }
        [Stmt::If(if_stmt), _] if if_stmt.else_branch.is_some() => {
            Err(BackendError::new(
                "unsupported statement after native terminator for Stage 6a: no statements may follow a terminal if/else",
            ))
        }
        [Stmt::Return { .. }, ..] => Err(BackendError::new(
            "unsupported statement after native terminator for Stage 6a: no statements may follow a final return",
        )),
        [first, ..] => Err(BackendError::new(format!(
            "unsupported native statement for Stage 6a: expected supported block local declaration, block assignment, final return, terminal if/else, or guard if followed by fallback block, found {}",
            describe_statement(first)
        ))),
    }
}

fn validate_stage_6a_guard(
    if_stmt: &hir::IfStmt,
    fallback_statements: &[Stmt],
    local_states: &HashMap<String, NativeLocalState>,
) -> Result<NativeTerminator, BackendError> {
    if fallback_statements.is_empty() {
        return Err(BackendError::new(
            "unsupported native branch fallthrough for Stage 6a: guard `if` without `else` requires a supported fallback block",
        ));
    }

    let condition = validate_stage_6a_condition(&if_stmt.condition, local_states)?;
    let then_block = validate_stage_6a_branch(&if_stmt.then_block.statements, local_states)?;
    let fallback = validate_stage_6a_block(fallback_statements, local_states)?;

    Ok(NativeTerminator::Guard {
        condition: condition.condition,
        evaluated_condition: condition.value,
        then_block: Box::new(then_block),
        fallback: Box::new(fallback),
    })
}

fn validate_stage_6a_local(
    decl: &hir::VarDecl,
    local_states: &HashMap<String, NativeLocalState>,
) -> Result<NativeLocal, BackendError> {
    if let Some(ty) = &decl.ty {
        if ty.name != "int" || !ty.args.is_empty() {
            return Err(unsupported_current_native_local());
        }
    }

    let initializer =
        validate_stage_6a_int_expr(&decl.initializer, local_states).map_err(|error| {
            if should_preserve_native_expression_error(&error.message) {
                error
            } else {
                unsupported_current_native_local()
            }
        })?;
    Ok(NativeLocal {
        name: decl.name.clone(),
        writable: decl.writable,
        expr: initializer.expr,
        evaluated_value: initializer.value,
    })
}

fn unsupported_current_native_local() -> BackendError {
    BackendError::new(
        "unsupported native local for current native smoke backend: expected readonly or writable `int` local initialized from integer literals, supported integer locals, or supported integer arithmetic",
    )
}

fn validate_stage_6a_assignment(
    assignment: &hir::Assignment,
    local_states: &HashMap<String, NativeLocalState>,
) -> Result<NativeAssign, BackendError> {
    let Expr::Variable { name, .. } = &assignment.target else {
        return Err(BackendError::new(
            "unsupported native assignment target for Stage 6a: expected writable `int` local",
        ));
    };

    let Some(target) = local_states.get(name) else {
        return Err(BackendError::new(format!(
            "unsupported native assignment target for Stage 6a: undeclared local `${name}`"
        )));
    };

    if !target.writable {
        return Err(BackendError::new(format!(
            "unsupported native assignment to readonly local for Stage 6a: `${name}`"
        )));
    }

    let value = validate_stage_6a_int_expr(&assignment.value, local_states)?;
    let (op, evaluated_value) = match assignment.op {
        AssignOp::Assign => (NativeAssignOp::Assign, value.value),
        AssignOp::AddAssign => (
            NativeAssignOp::AddAssign,
            checked_native_arithmetic(target.value, NativeBinaryOp::Add, value.value).ok_or_else(
                || BackendError::new("integer arithmetic overflows the Doria `int` range"),
            )?,
        ),
        AssignOp::SubAssign => (
            NativeAssignOp::SubAssign,
            checked_native_arithmetic(target.value, NativeBinaryOp::Subtract, value.value)
                .ok_or_else(|| {
                    BackendError::new("integer arithmetic overflows the Doria `int` range")
                })?,
        ),
    };

    Ok(NativeAssign {
        target: name.clone(),
        op,
        expr: value.expr,
        evaluated_value,
    })
}

fn validate_stage_6a_while(
    while_stmt: &hir::WhileStmt,
    local_states: &HashMap<String, NativeLocalState>,
) -> Result<NativeWhile, BackendError> {
    let condition =
        validate_stage_6a_condition(&while_stmt.condition, local_states).map_err(|error| {
            if should_preserve_native_expression_error(&error.message) {
                error
            } else {
                BackendError::new(
                    "unsupported native while condition for Stage 6a: expected supported boolean condition",
                )
            }
        })?;

    let body = validate_stage_6a_while_body(&while_stmt.body.statements, local_states)?;
    let mut simulated_states = local_states.clone();
    let mut iterations = 0;

    loop {
        let values = native_state_values(&simulated_states);
        let Some(condition_value) = evaluate_native_condition(&condition.condition, &values) else {
            return Err(BackendError::new(
                "backend validation failure: validated native while condition could not be re-evaluated",
            ));
        };

        if !condition_value {
            break;
        }

        if iterations == STAGE_6A_LOOP_VERIFICATION_CAP {
            return Err(stage_6a_loop_cap_error());
        }

        for assignment in &body {
            let values = native_state_values(&simulated_states);
            let Some(target) = simulated_states.get(&assignment.target) else {
                return Err(BackendError::new(
                    "backend validation failure: validated native while target was not declared",
                ));
            };
            let evaluated_value = evaluate_native_assignment_value(
                assignment.op,
                target.value,
                &assignment.expr,
                &values,
            )?;
            let Some(target) = simulated_states.get_mut(&assignment.target) else {
                return Err(BackendError::new(
                    "backend validation failure: validated native while target was not declared",
                ));
            };
            target.value = evaluated_value;
        }

        iterations += 1;
    }

    let mut assigned_targets = Vec::new();
    for assignment in &body {
        if !assigned_targets.contains(&assignment.target) {
            assigned_targets.push(assignment.target.clone());
        }
    }

    let mut final_values = Vec::new();
    for target in assigned_targets {
        let Some(state) = simulated_states.get(&target) else {
            return Err(BackendError::new(
                "backend validation failure: validated native while target was not declared",
            ));
        };
        final_values.push((target, state.value));
    }

    Ok(NativeWhile {
        condition: condition.condition,
        body,
        final_values,
        evaluated_iterations: iterations,
    })
}

fn validate_stage_6a_while_body(
    statements: &[Stmt],
    local_states: &HashMap<String, NativeLocalState>,
) -> Result<Vec<NativeLoopAssign>, BackendError> {
    if statements.is_empty() {
        return Err(BackendError::new(
            "unsupported native while body for Stage 6a: expected one or more supported assignments",
        ));
    }

    let mut assignments = Vec::new();
    for statement in statements {
        match statement {
            Stmt::Assignment(assignment) => {
                assignments.push(validate_stage_6a_loop_assignment(assignment, local_states)?);
            }
            Stmt::VarDecl(_) => {
                return Err(BackendError::new(
                    "unsupported native while body for Stage 6a: declarations inside while bodies are future native work",
                ));
            }
            other => {
                return Err(BackendError::new(format!(
                    "unsupported native while body statement for Stage 6a: expected assignment, found {}",
                    describe_statement(other)
                )));
            }
        }
    }

    Ok(assignments)
}

fn validate_stage_6a_loop_assignment(
    assignment: &hir::Assignment,
    local_states: &HashMap<String, NativeLocalState>,
) -> Result<NativeLoopAssign, BackendError> {
    let Expr::Variable { name, .. } = &assignment.target else {
        return Err(BackendError::new(
            "unsupported native while assignment target for Stage 6a: expected writable `int` local",
        ));
    };

    let Some(target) = local_states.get(name) else {
        return Err(BackendError::new(format!(
            "unsupported native while assignment target for Stage 6a: undeclared local `${name}`"
        )));
    };

    if !target.writable {
        return Err(BackendError::new(format!(
            "unsupported native while assignment target for Stage 6a: readonly local `${name}`"
        )));
    }

    let value = validate_stage_6a_int_expr(&assignment.value, local_states)?;
    Ok(NativeLoopAssign {
        target: name.clone(),
        op: match assignment.op {
            AssignOp::Assign => NativeAssignOp::Assign,
            AssignOp::AddAssign => NativeAssignOp::AddAssign,
            AssignOp::SubAssign => NativeAssignOp::SubAssign,
        },
        expr: value.expr,
    })
}

fn native_state_values(local_states: &HashMap<String, NativeLocalState>) -> HashMap<String, i64> {
    local_states
        .iter()
        .map(|(name, state)| (name.clone(), state.value))
        .collect()
}

fn evaluate_native_assignment_value(
    op: NativeAssignOp,
    current_value: i64,
    expr: &NativeExpr,
    local_values: &HashMap<String, i64>,
) -> Result<i64, BackendError> {
    let value = evaluate_native_expr(expr, local_values).ok_or_else(|| {
        BackendError::new(
            "backend validation failure: validated native while assignment could not be re-evaluated",
        )
    })?;

    match op {
        NativeAssignOp::Assign => Ok(value),
        NativeAssignOp::AddAssign => {
            checked_native_arithmetic(current_value, NativeBinaryOp::Add, value).ok_or_else(|| {
                BackendError::new("integer arithmetic overflows the Doria `int` range")
            })
        }
        NativeAssignOp::SubAssign => {
            checked_native_arithmetic(current_value, NativeBinaryOp::Subtract, value).ok_or_else(
                || BackendError::new("integer arithmetic overflows the Doria `int` range"),
            )
        }
    }
}

fn stage_6a_loop_cap_error() -> BackendError {
    BackendError::new(
        "unsupported native while loop for Stage 6a: loop could not be proven to terminate within the current native smoke verification cap",
    )
}

fn validate_stage_6a_terminator(
    statement: &Stmt,
    local_states: &HashMap<String, NativeLocalState>,
) -> Result<NativeTerminator, BackendError> {
    match statement {
        Stmt::Return { expr: Some(expr), .. } => {
            let (expr, evaluated_exit_code) = validate_stage_6a_return_expr(expr, local_states)?;
            Ok(NativeTerminator::Return {
                expr,
                evaluated_exit_code,
            })
        }
        Stmt::Return { expr: None, .. } => Err(BackendError::new(
            "unsupported native terminal statement for Stage 6a: expected `return <portable integer expression>;`, found bare `return;`",
        )),
        Stmt::If(if_stmt) => {
            let condition = validate_stage_6a_condition(&if_stmt.condition, local_states)?;
            let then_block = validate_stage_6a_branch(&if_stmt.then_block.statements, local_states)?;

            let Some(else_branch) = &if_stmt.else_branch else {
                return Err(BackendError::new(
                    "unsupported native terminal if for Stage 6a: terminal if requires else; guard if without else is supported only when followed by a fallback return",
                ));
            };

            let else_block = match else_branch {
                ElseBranch::Block(else_block) => {
                    validate_stage_6a_branch(&else_block.statements, local_states)?
                }
                ElseBranch::If(else_if) => validate_stage_6a_if_as_block(else_if, local_states)?,
            };

            Ok(NativeTerminator::IfElse {
                condition: condition.condition,
                evaluated_condition: condition.value,
                then_block: Box::new(then_block),
                else_block: Box::new(else_block),
            })
        }
        other => Err(BackendError::new(format!(
            "unsupported native terminal statement for Stage 6a: expected final return or terminal if/else, found {}",
            describe_statement(other)
        ))),
    }
}

fn validate_stage_6a_branch(
    statements: &[Stmt],
    local_states: &HashMap<String, NativeLocalState>,
) -> Result<NativeBlock, BackendError> {
    validate_stage_6a_block(statements, local_states).map_err(|error| {
        if should_preserve_native_block_error(&error.message) {
            error
        } else {
            BackendError::new(
                "unsupported native branch body shape for Stage 6a: expected supported local declarations or assignments followed by a supported native terminator",
            )
        }
    })
}

fn validate_stage_6a_if_as_block(
    if_stmt: &hir::IfStmt,
    local_states: &HashMap<String, NativeLocalState>,
) -> Result<NativeBlock, BackendError> {
    let statement = Stmt::If(if_stmt.clone());
    let terminator = validate_stage_6a_terminator(&statement, local_states)?;
    Ok(NativeBlock {
        statements: Vec::new(),
        terminator,
    })
}

fn should_preserve_native_block_error(message: &str) -> bool {
    should_preserve_native_expression_error(message)
        || message.contains("exit code")
        || message.contains("unsupported native assignment")
        || message.contains("readonly local")
        || message.contains("branch fallthrough")
        || message.contains("unsupported native branch body shape")
        || message.contains("unsupported native while")
}

fn validate_stage_6a_return_expr(
    expr: &Expr,
    local_states: &HashMap<String, NativeLocalState>,
) -> Result<(NativeExpr, i32), BackendError> {
    let return_expr = validate_stage_6a_int_expr(expr, local_states)?;
    let evaluated_exit_code = parse_stage_6a_exit_code(return_expr.value)?;
    Ok((return_expr.expr, evaluated_exit_code))
}

fn validate_stage_6a_int_expr(
    expr: &Expr,
    local_states: &HashMap<String, NativeLocalState>,
) -> Result<ValidatedNativeExpr, BackendError> {
    match expr {
        Expr::Int { value, .. } => {
            let value = parse_doria_int_literal(value)?;
            Ok(ValidatedNativeExpr {
                expr: NativeExpr::Int(value),
                value,
            })
        }
        Expr::Variable { name, .. } => {
            let value = local_states.get(name).map(|state| state.value).ok_or_else(|| {
                BackendError::new(
                    "unsupported native expression for Stage 6a: expected integer literal, supported integer local, or supported integer arithmetic",
                )
            })?;
            Ok(ValidatedNativeExpr {
                expr: NativeExpr::Local(name.clone()),
                value,
            })
        }
        Expr::Grouped { expr, .. } => validate_stage_6a_int_expr(expr, local_states),
        Expr::Binary {
            left, op, right, ..
        } if native_binary_op(op).is_some() => {
            let native_op = native_binary_op(op).expect("checked by guard");
            let left = validate_stage_6a_int_expr(left, local_states)?;
            let right = validate_stage_6a_int_expr(right, local_states)?;
            let value = checked_native_arithmetic(left.value, native_op, right.value).ok_or_else(|| {
                BackendError::new("integer arithmetic overflows the Doria `int` range")
            })?;
            Ok(ValidatedNativeExpr {
                expr: NativeExpr::Binary {
                    op: native_op,
                    left: Box::new(left.expr),
                    right: Box::new(right.expr),
                },
                value,
            })
        }
        Expr::Binary {
            op: BinaryOp::Div | BinaryOp::Mod,
            ..
        } => {
            Err(BackendError::new(
                "unsupported native arithmetic operator for Stage 6a",
            ))
        }
        other => Err(BackendError::new(format!(
            "unsupported native expression for Stage 6a: expected integer literal, supported integer local, or supported integer arithmetic, found `{}`",
            describe_expression(other)
        ))),
    }
}

fn validate_stage_6a_condition(
    expr: &Expr,
    local_states: &HashMap<String, NativeLocalState>,
) -> Result<ValidatedNativeCondition, BackendError> {
    match expr {
        Expr::Bool { value, .. } => Ok(ValidatedNativeCondition {
            condition: NativeCondition::Bool(*value),
            value: *value,
        }),
        Expr::Grouped { expr, .. } => validate_stage_6a_condition(expr, local_states),
        Expr::Unary {
            op: UnaryOp::Not,
            expr,
            ..
        } => {
            let condition = validate_stage_6a_condition(expr, local_states)?;
            Ok(ValidatedNativeCondition {
                condition: NativeCondition::Not(Box::new(condition.condition)),
                value: !condition.value,
            })
        }
        Expr::Binary {
            left, op, right, ..
        } if native_compare_op(op).is_some() => {
            let native_op = native_compare_op(op).expect("checked by guard");
            let left = validate_stage_6a_comparison_operand(left, local_states)?;
            let right = validate_stage_6a_comparison_operand(right, local_states)?;
            Ok(ValidatedNativeCondition {
                condition: NativeCondition::Compare {
                    op: native_op,
                    left: left.expr,
                    right: right.expr,
                },
                value: evaluate_native_compare(left.value, native_op, right.value),
            })
        }
        Expr::Binary {
            left,
            op: BinaryOp::And,
            right,
            ..
        } => {
            let left = validate_stage_6a_condition(left, local_states)?;
            let right = validate_stage_6a_condition(right, local_states)?;
            let value = left.value && right.value;
            Ok(ValidatedNativeCondition {
                condition: NativeCondition::And {
                    left: Box::new(left.condition),
                    right: Box::new(right.condition),
                },
                value,
            })
        }
        Expr::Binary {
            left,
            op: BinaryOp::Or,
            right,
            ..
        } => {
            let left = validate_stage_6a_condition(left, local_states)?;
            let right = validate_stage_6a_condition(right, local_states)?;
            let value = left.value || right.value;
            Ok(ValidatedNativeCondition {
                condition: NativeCondition::Or {
                    left: Box::new(left.condition),
                    right: Box::new(right.condition),
                },
                value,
            })
        }
        Expr::Binary {
            left,
            op: BinaryOp::Xor,
            right,
            ..
        } => {
            let left = validate_stage_6a_condition(left, local_states)?;
            let right = validate_stage_6a_condition(right, local_states)?;
            let value = left.value ^ right.value;
            Ok(ValidatedNativeCondition {
                condition: NativeCondition::Xor {
                    left: Box::new(left.condition),
                    right: Box::new(right.condition),
                },
                value,
            })
        }

        _ => Err(BackendError::new(
            "unsupported native condition for Stage 6a: expected bool literal, supported integer comparison, or supported boolean condition",
        )),
    }
}

fn validate_stage_6a_comparison_operand(
    expr: &Expr,
    local_states: &HashMap<String, NativeLocalState>,
) -> Result<ValidatedNativeExpr, BackendError> {
    validate_stage_6a_int_expr(expr, local_states).map_err(|error| {
        if should_preserve_native_expression_error(&error.message) {
            error
        } else {
            BackendError::new(
                "unsupported native comparison for Stage 6a: expected supported integer expressions",
            )
        }
    })
}

fn should_preserve_native_expression_error(message: &str) -> bool {
    message.contains("unsupported native arithmetic operator")
        || message.contains("integer arithmetic overflows")
        || message.contains("integer literal is outside")
}

fn native_binary_op(op: &BinaryOp) -> Option<NativeBinaryOp> {
    match op {
        BinaryOp::Add => Some(NativeBinaryOp::Add),
        BinaryOp::Sub => Some(NativeBinaryOp::Subtract),
        BinaryOp::Mul => Some(NativeBinaryOp::Multiply),
        _ => None,
    }
}

fn native_compare_op(op: &BinaryOp) -> Option<NativeCompareOp> {
    match op {
        BinaryOp::Equal => Some(NativeCompareOp::Equal),
        BinaryOp::NotEqual => Some(NativeCompareOp::NotEqual),
        BinaryOp::Less => Some(NativeCompareOp::LessThan),
        BinaryOp::LessEqual => Some(NativeCompareOp::LessThanOrEqual),
        BinaryOp::Greater => Some(NativeCompareOp::GreaterThan),
        BinaryOp::GreaterEqual => Some(NativeCompareOp::GreaterThanOrEqual),
        _ => None,
    }
}

fn checked_native_arithmetic(left: i64, op: NativeBinaryOp, right: i64) -> Option<i64> {
    match op {
        NativeBinaryOp::Add => left.checked_add(right),
        NativeBinaryOp::Subtract => left.checked_sub(right),
        NativeBinaryOp::Multiply => left.checked_mul(right),
    }
}

fn evaluate_native_compare(left: i64, op: NativeCompareOp, right: i64) -> bool {
    match op {
        NativeCompareOp::Equal => left == right,
        NativeCompareOp::NotEqual => left != right,
        NativeCompareOp::LessThan => left < right,
        NativeCompareOp::LessThanOrEqual => left <= right,
        NativeCompareOp::GreaterThan => left > right,
        NativeCompareOp::GreaterThanOrEqual => left >= right,
    }
}

fn evaluate_native_block_exit_code(block: &NativeBlock) -> i32 {
    evaluate_native_terminator_exit_code(&block.terminator)
}

fn evaluate_native_terminator_exit_code(terminator: &NativeTerminator) -> i32 {
    match terminator {
        NativeTerminator::Return {
            evaluated_exit_code,
            ..
        } => *evaluated_exit_code,
        NativeTerminator::IfElse {
            evaluated_condition,
            then_block,
            else_block,
            ..
        } => {
            if *evaluated_condition {
                evaluate_native_block_exit_code(then_block)
            } else {
                evaluate_native_block_exit_code(else_block)
            }
        }
        NativeTerminator::Guard {
            evaluated_condition,
            then_block,
            fallback,
            ..
        } => {
            if *evaluated_condition {
                evaluate_native_block_exit_code(then_block)
            } else {
                evaluate_native_block_exit_code(fallback)
            }
        }
    }
}

fn parse_doria_int_literal(value: &str) -> Result<i64, BackendError> {
    value
        .parse::<i64>()
        .map_err(|_| BackendError::new("integer literal is outside the Doria `int` range"))
}

fn parse_stage_6a_exit_code(value: i64) -> Result<i32, BackendError> {
    if !(0..=125).contains(&value) {
        return Err(BackendError::new(
            "native Stage 6a exit code must be in the range 0..125",
        ));
    }

    Ok(value as i32)
}

fn generate_object(native_main: &NativeMain) -> Result<Vec<u8>, BackendError> {
    let isa_builder = cranelift_native::builder()
        .map_err(|error| BackendError::new(format!("backend emission failure: {error}")))?;
    let isa = isa_builder
        .finish(settings::Flags::new(settings::builder()))
        .map_err(|error| BackendError::new(format!("backend emission failure: {error}")))?;
    let mut module = ObjectModule::new(
        ObjectBuilder::new(isa, "doria_stage_6a", default_libcall_names())
            .map_err(|error| BackendError::new(format!("backend emission failure: {error}")))?,
    );

    let mut signature = module.make_signature();
    signature.returns.push(AbiParam::new(types::I32));

    let function_id = module
        .declare_function("main", Linkage::Export, &signature)
        .map_err(|error| BackendError::new(format!("backend emission failure: {error}")))?;

    let mut context = module.make_context();
    context.func.signature = signature;
    let mut function_context = FunctionBuilderContext::new();
    {
        let mut builder = FunctionBuilder::new(&mut context.func, &mut function_context);
        let entry_block = builder.create_block();
        builder.switch_to_block(entry_block);
        builder.seal_block(entry_block);
        let mut lowered_local_values = HashMap::new();
        let mut evaluated_local_values = HashMap::new();
        lower_native_block(
            &mut builder,
            &native_main.body,
            &mut lowered_local_values,
            &mut evaluated_local_values,
        )?;
        builder.finalize();
    }

    module
        .define_function(function_id, &mut context)
        .map_err(|error| BackendError::new(format!("backend emission failure: {error}")))?;
    module.clear_context(&mut context);

    module
        .finish()
        .emit()
        .map_err(|error| BackendError::new(format!("backend emission failure: {error}")))
}

fn lower_native_block(
    builder: &mut FunctionBuilder,
    block: &NativeBlock,
    lowered_local_values: &mut HashMap<String, Value>,
    evaluated_local_values: &mut HashMap<String, i64>,
) -> Result<(), BackendError> {
    for statement in &block.statements {
        lower_native_statement(
            builder,
            statement,
            lowered_local_values,
            evaluated_local_values,
        )?;
    }

    lower_native_terminator(
        builder,
        &block.terminator,
        lowered_local_values,
        evaluated_local_values,
    )
}

fn lower_native_terminator(
    builder: &mut FunctionBuilder,
    terminator: &NativeTerminator,
    lowered_local_values: &HashMap<String, Value>,
    evaluated_local_values: &HashMap<String, i64>,
) -> Result<(), BackendError> {
    match terminator {
        NativeTerminator::Return {
            expr,
            evaluated_exit_code,
        } => lower_native_return(
            builder,
            expr,
            *evaluated_exit_code,
            lowered_local_values,
            evaluated_local_values,
        ),
        NativeTerminator::IfElse {
            condition,
            evaluated_condition,
            then_block,
            else_block,
        } => {
            let Some(evaluated_condition_value) =
                evaluate_native_condition(condition, evaluated_local_values)
            else {
                return Err(BackendError::new(
                    "backend emission failure: validated native condition could not be re-evaluated",
                ));
            };
            debug_assert_eq!(evaluated_condition_value, *evaluated_condition);

            let then_ir_block = builder.create_block();
            let else_ir_block = builder.create_block();
            lower_native_condition_branch(
                builder,
                condition,
                then_ir_block,
                else_ir_block,
                lowered_local_values,
            )?;

            builder.switch_to_block(then_ir_block);
            builder.seal_block(then_ir_block);
            let mut then_lowered_local_values = lowered_local_values.clone();
            let mut then_evaluated_local_values = evaluated_local_values.clone();
            lower_native_block(
                builder,
                then_block,
                &mut then_lowered_local_values,
                &mut then_evaluated_local_values,
            )?;

            builder.switch_to_block(else_ir_block);
            builder.seal_block(else_ir_block);
            let mut else_lowered_local_values = lowered_local_values.clone();
            let mut else_evaluated_local_values = evaluated_local_values.clone();
            lower_native_block(
                builder,
                else_block,
                &mut else_lowered_local_values,
                &mut else_evaluated_local_values,
            )
        }
        NativeTerminator::Guard {
            condition,
            evaluated_condition,
            then_block,
            fallback,
        } => {
            let Some(evaluated_condition_value) =
                evaluate_native_condition(condition, evaluated_local_values)
            else {
                return Err(BackendError::new(
                    "backend emission failure: validated native condition could not be re-evaluated",
                ));
            };
            debug_assert_eq!(evaluated_condition_value, *evaluated_condition);

            let then_ir_block = builder.create_block();
            let fallback_ir_block = builder.create_block();
            lower_native_condition_branch(
                builder,
                condition,
                then_ir_block,
                fallback_ir_block,
                lowered_local_values,
            )?;

            builder.switch_to_block(then_ir_block);
            builder.seal_block(then_ir_block);
            let mut then_lowered_local_values = lowered_local_values.clone();
            let mut then_evaluated_local_values = evaluated_local_values.clone();
            lower_native_block(
                builder,
                then_block,
                &mut then_lowered_local_values,
                &mut then_evaluated_local_values,
            )?;

            builder.switch_to_block(fallback_ir_block);
            builder.seal_block(fallback_ir_block);
            let mut fallback_lowered_local_values = lowered_local_values.clone();
            let mut fallback_evaluated_local_values = evaluated_local_values.clone();
            lower_native_block(
                builder,
                fallback,
                &mut fallback_lowered_local_values,
                &mut fallback_evaluated_local_values,
            )
        }
    }
}

fn lower_native_statement(
    builder: &mut FunctionBuilder,
    statement: &NativeStmt,
    lowered_local_values: &mut HashMap<String, Value>,
    evaluated_local_values: &mut HashMap<String, i64>,
) -> Result<(), BackendError> {
    match statement {
        NativeStmt::Local(local) => {
            let value = lower_native_expr(builder, &local.expr, lowered_local_values)?;
            lowered_local_values.insert(local.name.clone(), value);
            evaluated_local_values.insert(local.name.clone(), local.evaluated_value);
            Ok(())
        }
        NativeStmt::Assign(assignment) => {
            let value = lower_native_assignment(builder, assignment, lowered_local_values)?;
            lowered_local_values.insert(assignment.target.clone(), value);
            evaluated_local_values.insert(assignment.target.clone(), assignment.evaluated_value);
            Ok(())
        }
        NativeStmt::While(native_while) => lower_native_while(
            builder,
            native_while,
            lowered_local_values,
            evaluated_local_values,
        ),
    }
}

fn lower_native_while(
    builder: &mut FunctionBuilder,
    native_while: &NativeWhile,
    lowered_local_values: &mut HashMap<String, Value>,
    evaluated_local_values: &mut HashMap<String, i64>,
) -> Result<(), BackendError> {
    debug_assert!(native_while.evaluated_iterations <= STAGE_6A_LOOP_VERIFICATION_CAP);

    let loop_header = builder.create_block();
    let loop_body = builder.create_block();
    let loop_after = builder.create_block();

    for _ in &native_while.final_values {
        builder.append_block_param(loop_header, types::I64);
    }

    let initial_args = native_while
        .final_values
        .iter()
        .map(|(name, _)| {
            lowered_local_values
                .get(name)
                .copied()
                .map(BlockArg::Value)
                .ok_or_else(|| {
                    BackendError::new(format!(
                        "backend emission failure: validated native while target `{name}` was not lowered"
                    ))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    builder.ins().jump(loop_header, &initial_args);

    builder.switch_to_block(loop_header);
    let mut header_local_values = lowered_local_values.clone();
    for (index, (name, _)) in native_while.final_values.iter().enumerate() {
        header_local_values.insert(name.clone(), builder.block_params(loop_header)[index]);
    }
    lower_native_condition_branch(
        builder,
        &native_while.condition,
        loop_body,
        loop_after,
        &header_local_values,
    )?;

    builder.switch_to_block(loop_body);
    builder.seal_block(loop_body);
    let mut body_local_values = header_local_values.clone();
    for assignment in &native_while.body {
        let value = lower_native_loop_assignment(builder, assignment, &body_local_values)?;
        body_local_values.insert(assignment.target.clone(), value);
    }
    let next_args = native_while
        .final_values
        .iter()
        .map(|(name, _)| {
            body_local_values
                .get(name)
                .copied()
                .map(BlockArg::Value)
                .ok_or_else(|| {
                    BackendError::new(format!(
                        "backend emission failure: validated native while target `{name}` was not lowered"
                    ))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    builder.ins().jump(loop_header, &next_args);
    builder.seal_block(loop_header);

    builder.switch_to_block(loop_after);
    builder.seal_block(loop_after);
    for (name, value) in &native_while.final_values {
        let Some(lowered_value) = header_local_values.get(name).copied() else {
            return Err(BackendError::new(format!(
                "backend emission failure: validated native while target `{name}` was not lowered"
            )));
        };
        lowered_local_values.insert(name.clone(), lowered_value);
        evaluated_local_values.insert(name.clone(), *value);
    }

    Ok(())
}

fn lower_native_assignment(
    builder: &mut FunctionBuilder,
    assignment: &NativeAssign,
    local_values: &HashMap<String, Value>,
) -> Result<Value, BackendError> {
    let right = lower_native_expr(builder, &assignment.expr, local_values)?;
    match assignment.op {
        NativeAssignOp::Assign => Ok(right),
        NativeAssignOp::AddAssign | NativeAssignOp::SubAssign => {
            let left = local_values.get(&assignment.target).copied().ok_or_else(|| {
                BackendError::new(format!(
                    "backend emission failure: validated native assignment target `{}` was not lowered",
                    assignment.target
                ))
            })?;
            Ok(match assignment.op {
                NativeAssignOp::Assign => unreachable!("handled above"),
                NativeAssignOp::AddAssign => builder.ins().iadd(left, right),
                NativeAssignOp::SubAssign => builder.ins().isub(left, right),
            })
        }
    }
}

fn lower_native_loop_assignment(
    builder: &mut FunctionBuilder,
    assignment: &NativeLoopAssign,
    local_values: &HashMap<String, Value>,
) -> Result<Value, BackendError> {
    let right = lower_native_expr(builder, &assignment.expr, local_values)?;
    match assignment.op {
        NativeAssignOp::Assign => Ok(right),
        NativeAssignOp::AddAssign | NativeAssignOp::SubAssign => {
            let left = local_values
                .get(&assignment.target)
                .copied()
                .ok_or_else(|| {
                    BackendError::new(format!(
                    "backend emission failure: validated native while target `{}` was not lowered",
                    assignment.target
                ))
                })?;
            Ok(match assignment.op {
                NativeAssignOp::Assign => unreachable!("handled above"),
                NativeAssignOp::AddAssign => builder.ins().iadd(left, right),
                NativeAssignOp::SubAssign => builder.ins().isub(left, right),
            })
        }
    }
}

fn lower_native_return(
    builder: &mut FunctionBuilder,
    expr: &NativeExpr,
    evaluated_exit_code: i32,
    lowered_local_values: &HashMap<String, Value>,
    evaluated_local_values: &HashMap<String, i64>,
) -> Result<(), BackendError> {
    let return_value = lower_native_expr(builder, expr, lowered_local_values)?;
    let Some(evaluated_return) = evaluate_native_expr(expr, evaluated_local_values) else {
        return Err(BackendError::new(
            "backend emission failure: validated native expression could not be re-evaluated",
        ));
    };
    debug_assert_eq!(evaluated_return, i64::from(evaluated_exit_code));

    let exit_value = builder.ins().ireduce(types::I32, return_value);
    builder.ins().return_(&[exit_value]);
    Ok(())
}

fn lower_native_expr(
    builder: &mut FunctionBuilder,
    expr: &NativeExpr,
    local_values: &HashMap<String, Value>,
) -> Result<Value, BackendError> {
    match expr {
        NativeExpr::Int(value) => Ok(builder.ins().iconst(types::I64, *value)),
        NativeExpr::Local(name) => local_values.get(name).copied().ok_or_else(|| {
            BackendError::new(format!(
                "backend emission failure: validated native local `{name}` was not lowered"
            ))
        }),
        NativeExpr::Binary { op, left, right } => {
            let left = lower_native_expr(builder, left, local_values)?;
            let right = lower_native_expr(builder, right, local_values)?;
            Ok(match op {
                NativeBinaryOp::Add => builder.ins().iadd(left, right),
                NativeBinaryOp::Subtract => builder.ins().isub(left, right),
                NativeBinaryOp::Multiply => builder.ins().imul(left, right),
            })
        }
    }
}

fn lower_native_condition_branch(
    builder: &mut FunctionBuilder,
    condition: &NativeCondition,
    then_block: Block,
    else_block: Block,
    local_values: &HashMap<String, Value>,
) -> Result<(), BackendError> {
    match condition {
        NativeCondition::Bool(value) => {
            let value = builder.ins().iconst(types::I8, i64::from(*value));
            builder.ins().brif(value, then_block, &[], else_block, &[]);
            Ok(())
        }
        NativeCondition::Compare { op, left, right } => {
            let left = lower_native_expr(builder, left, local_values)?;
            let right = lower_native_expr(builder, right, local_values)?;
            let condition = builder.ins().icmp(native_compare_intcc(*op), left, right);
            builder
                .ins()
                .brif(condition, then_block, &[], else_block, &[]);
            Ok(())
        }
        NativeCondition::Not(condition) => {
            lower_native_condition_branch(builder, condition, else_block, then_block, local_values)
        }
        NativeCondition::And { left, right } => {
            let right_block = builder.create_block();
            lower_native_condition_branch(builder, left, right_block, else_block, local_values)?;

            builder.switch_to_block(right_block);
            builder.seal_block(right_block);
            lower_native_condition_branch(builder, right, then_block, else_block, local_values)
        }
        NativeCondition::Or { left, right } => {
            let right_block = builder.create_block();
            lower_native_condition_branch(builder, left, then_block, right_block, local_values)?;

            builder.switch_to_block(right_block);
            builder.seal_block(right_block);
            lower_native_condition_branch(builder, right, then_block, else_block, local_values)
        }
        NativeCondition::Xor { left, right } => {
            let left = lower_native_condition_value(builder, left, local_values)?;
            let right = lower_native_condition_value(builder, right, local_values)?;
            let condition = builder.ins().icmp(IntCC::NotEqual, left, right);
            builder
                .ins()
                .brif(condition, then_block, &[], else_block, &[]);
            Ok(())
        }
    }
}

fn lower_native_condition_value(
    builder: &mut FunctionBuilder,
    condition: &NativeCondition,
    local_values: &HashMap<String, Value>,
) -> Result<Value, BackendError> {
    let true_block = builder.create_block();
    let false_block = builder.create_block();
    let done_block = builder.create_block();
    builder.append_block_param(done_block, types::I8);

    lower_native_condition_branch(builder, condition, true_block, false_block, local_values)?;

    builder.switch_to_block(true_block);
    builder.seal_block(true_block);
    let true_value = builder.ins().iconst(types::I8, 1);
    builder
        .ins()
        .jump(done_block, &[BlockArg::Value(true_value)]);

    builder.switch_to_block(false_block);
    builder.seal_block(false_block);
    let false_value = builder.ins().iconst(types::I8, 0);
    builder
        .ins()
        .jump(done_block, &[BlockArg::Value(false_value)]);

    builder.switch_to_block(done_block);
    builder.seal_block(done_block);
    Ok(builder.block_params(done_block)[0])
}

fn native_compare_intcc(op: NativeCompareOp) -> IntCC {
    match op {
        NativeCompareOp::Equal => IntCC::Equal,
        NativeCompareOp::NotEqual => IntCC::NotEqual,
        NativeCompareOp::LessThan => IntCC::SignedLessThan,
        NativeCompareOp::LessThanOrEqual => IntCC::SignedLessThanOrEqual,
        NativeCompareOp::GreaterThan => IntCC::SignedGreaterThan,
        NativeCompareOp::GreaterThanOrEqual => IntCC::SignedGreaterThanOrEqual,
    }
}

fn evaluate_native_expr(expr: &NativeExpr, local_values: &HashMap<String, i64>) -> Option<i64> {
    match expr {
        NativeExpr::Int(value) => Some(*value),
        NativeExpr::Local(name) => local_values.get(name).copied(),
        NativeExpr::Binary { op, left, right } => checked_native_arithmetic(
            evaluate_native_expr(left, local_values)?,
            *op,
            evaluate_native_expr(right, local_values)?,
        ),
    }
}

fn evaluate_native_condition(
    condition: &NativeCondition,
    local_values: &HashMap<String, i64>,
) -> Option<bool> {
    match condition {
        NativeCondition::Bool(value) => Some(*value),
        NativeCondition::Compare { op, left, right } => Some(evaluate_native_compare(
            evaluate_native_expr(left, local_values)?,
            *op,
            evaluate_native_expr(right, local_values)?,
        )),
        NativeCondition::Not(condition) => {
            Some(!evaluate_native_condition(condition, local_values)?)
        }
        NativeCondition::And { left, right } => {
            if !evaluate_native_condition(left, local_values)? {
                Some(false)
            } else {
                evaluate_native_condition(right, local_values)
            }
        }
        NativeCondition::Or { left, right } => {
            if evaluate_native_condition(left, local_values)? {
                Some(true)
            } else {
                evaluate_native_condition(right, local_values)
            }
        }
        NativeCondition::Xor { left, right } => Some(
            evaluate_native_condition(left, local_values)?
                ^ evaluate_native_condition(right, local_values)?,
        ),
    }
}

fn link_object(object_bytes: &[u8]) -> Result<Vec<u8>, BackendError> {
    let temp_stem = unique_temp_stem();
    let object_path = temp_stem.with_extension(object_extension());
    let executable_path = temp_stem.with_extension(executable_extension());

    fs::write(&object_path, object_bytes)
        .map_err(|error| BackendError::new(format!("backend emission failure: {error}")))?;

    let link_result = invoke_linker(&object_path, &executable_path);
    let executable_bytes = match link_result {
        Ok(()) => fs::read(&executable_path)
            .map_err(|error| BackendError::new(format!("backend emission failure: {error}")))?,
        Err(error) => {
            cleanup_temp_artifacts(&object_path, &executable_path);
            return Err(error);
        }
    };

    cleanup_temp_artifacts(&object_path, &executable_path);
    Ok(executable_bytes)
}

fn invoke_linker(object_path: &Path, executable_path: &Path) -> Result<(), BackendError> {
    // Stage 6a emits a Cranelift object file and asks the host toolchain to
    // link it. This is not a C backend: Doria never generates C source or uses
    // C semantics as an oracle here.
    let cc_is_set = env::var_os("CC").is_some();
    let linker = env::var("CC").unwrap_or_else(|_| default_linker().to_string());
    let mut command = Command::new(&linker);
    command.args(linker_arguments(
        &linker,
        cc_is_set,
        cfg!(windows),
        object_path,
        executable_path,
    ));

    let output = command.output().map_err(|error| {
        BackendError::new(format!(
            "linker/toolchain failure: failed to run `{linker}`: {error}"
        ))
    })?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let details = [stderr.trim(), stdout.trim()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    if details.is_empty() {
        Err(BackendError::new(format!(
            "linker/toolchain failure: `{linker}` exited with status {}",
            output.status
        )))
    } else {
        Err(BackendError::new(format!(
            "linker/toolchain failure: `{linker}` exited with status {}\n{}",
            output.status, details
        )))
    }
}

fn cleanup_temp_artifacts(object_path: &Path, executable_path: &Path) {
    let _ = fs::remove_file(object_path);
    let _ = fs::remove_file(executable_path);
}

fn unique_temp_stem() -> PathBuf {
    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

    let sequence = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    env::temp_dir().join(format!(
        "doriac-native-{}-{nanos}-{sequence}",
        std::process::id()
    ))
}

fn object_extension() -> &'static str {
    if cfg!(windows) {
        "obj"
    } else {
        "o"
    }
}

fn executable_extension() -> &'static str {
    if cfg!(windows) {
        "exe"
    } else {
        "out"
    }
}

fn default_linker() -> &'static str {
    if cfg!(windows) {
        "cl.exe"
    } else {
        "cc"
    }
}

fn linker_arguments(
    linker: &str,
    cc_is_set: bool,
    windows: bool,
    object_path: &Path,
    executable_path: &Path,
) -> Vec<OsString> {
    if windows && (!cc_is_set || is_msvc_style_compiler_driver(linker)) {
        // Cranelift-generated objects do not carry MSVC /DEFAULTLIB directives.
        // For Stage 6a's tiny main, make Doria's main the executable entrypoint
        // instead of relying on CRT startup to discover and call it.
        return vec![
            OsString::from("/nologo"),
            object_path.as_os_str().to_os_string(),
            OsString::from(format!("/Fe:{}", executable_path.display())),
            OsString::from("/link"),
            OsString::from("/ENTRY:main"),
            OsString::from("/SUBSYSTEM:CONSOLE"),
        ];
    }

    vec![
        object_path.as_os_str().to_os_string(),
        OsString::from("-o"),
        executable_path.as_os_str().to_os_string(),
    ]
}

fn is_msvc_style_compiler_driver(linker: &str) -> bool {
    let Some(name) = Path::new(linker).file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    matches!(
        name.to_ascii_lowercase().as_str(),
        "cl" | "cl.exe" | "clang-cl" | "clang-cl.exe"
    )
}

fn describe_statement(statement: &Stmt) -> &'static str {
    match statement {
        Stmt::VarDecl(_) => "local variable declaration",
        Stmt::Assignment(_) => "assignment",
        Stmt::Echo { .. } => "echo statement",
        Stmt::Return { .. } => "return statement",
        Stmt::If(_) => "if statement",
        Stmt::While(_) => "while statement",
        Stmt::Foreach(_) => "foreach statement",
        Stmt::Expr { .. } => "expression statement",
    }
}

fn describe_expression(expr: &Expr) -> &'static str {
    match expr {
        Expr::Variable { .. } => "variable",
        Expr::This { .. } => "$this",
        Expr::Identifier { .. } => "identifier",
        Expr::String { .. } => "string literal",
        Expr::InterpolatedString { .. } => "interpolated string",
        Expr::Int { .. } => "integer literal",
        Expr::Float { .. } => "float literal",
        Expr::Bool { .. } => "bool literal",
        Expr::Null { .. } => "null literal",
        Expr::Array { .. } => "collection literal",
        Expr::PropertyAccess { .. } => "property access",
        Expr::MethodCall { .. } => "method call",
        Expr::FunctionCall { .. } => "function call",
        Expr::StaticCall { .. } => "static call",
        Expr::New { .. } => "object construction",
        Expr::Grouped { .. } => "grouped expression",
        Expr::Unary { .. } => "unary expression",
        Expr::Binary { .. } => "binary expression",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_6a_validation_preserves_return_expression_structure_for_codegen() {
        let program = crate::lower_source(
            "stage3a.doria",
            r#"
function main(): int
{
    let $left = 20;
    let $right = 22;
    return $left + $right;
}
"#,
        )
        .expect("source should lower to HIR");

        let native_main = validate_stage_6a(&program).expect("source should validate for Stage 6a");

        assert_eq!(native_main.evaluated_exit_code, 42);
        assert_eq!(
            native_main.body.statements,
            vec![
                NativeStmt::Local(NativeLocal {
                    name: "left".to_string(),
                    writable: false,
                    expr: NativeExpr::Int(20),
                    evaluated_value: 20,
                }),
                NativeStmt::Local(NativeLocal {
                    name: "right".to_string(),
                    writable: false,
                    expr: NativeExpr::Int(22),
                    evaluated_value: 22,
                }),
            ]
        );

        assert_eq!(
            native_main.body.terminator,
            NativeTerminator::Return {
                expr: NativeExpr::Binary {
                    op: NativeBinaryOp::Add,
                    left: Box::new(NativeExpr::Local("left".to_string())),
                    right: Box::new(NativeExpr::Local("right".to_string())),
                },
                evaluated_exit_code: 42,
            }
        );
    }

    #[test]
    fn stage_6a_validation_preserves_if_else_structure_for_codegen() {
        let program = crate::lower_source(
            "stage4a.doria",
            r#"
function main(): int
{
    let $left = 20;
    let $right = 22;

    if ($left + $right == 42) {
        return 42;
    } else {
        return 0;
    }
}
"#,
        )
        .expect("source should lower to HIR");

        let native_main = validate_stage_6a(&program).expect("source should validate for Stage 6a");

        assert_eq!(native_main.evaluated_exit_code, 42);
        assert_eq!(native_main.body.statements.len(), 2);
        assert_eq!(
            native_main.body.terminator,
            NativeTerminator::IfElse {
                condition: NativeCondition::Compare {
                    op: NativeCompareOp::Equal,
                    left: NativeExpr::Binary {
                        op: NativeBinaryOp::Add,
                        left: Box::new(NativeExpr::Local("left".to_string())),
                        right: Box::new(NativeExpr::Local("right".to_string())),
                    },
                    right: NativeExpr::Int(42),
                },
                evaluated_condition: true,
                then_block: Box::new(NativeBlock {
                    statements: Vec::new(),
                    terminator: NativeTerminator::Return {
                        expr: NativeExpr::Int(42),
                        evaluated_exit_code: 42,
                    },
                }),
                else_block: Box::new(NativeBlock {
                    statements: Vec::new(),
                    terminator: NativeTerminator::Return {
                        expr: NativeExpr::Int(0),
                        evaluated_exit_code: 0,
                    },
                }),
            }
        );
    }

    #[test]
    fn stage_6a_validation_preserves_guard_if_structure_for_codegen() {
        let program = crate::lower_source(
            "stage4a_guard.doria",
            r#"
function main(): int
{
    let $left = 20;
    let $right = 22;

    if ($left + $right == 42) {
        return 42;
    }

    return 0;
}
"#,
        )
        .expect("source should lower to HIR");

        let native_main = validate_stage_6a(&program).expect("source should validate for Stage 6a");

        assert_eq!(native_main.evaluated_exit_code, 42);
        assert_eq!(native_main.body.statements.len(), 2);
        assert_eq!(
            native_main.body.terminator,
            NativeTerminator::Guard {
                condition: NativeCondition::Compare {
                    op: NativeCompareOp::Equal,
                    left: NativeExpr::Binary {
                        op: NativeBinaryOp::Add,
                        left: Box::new(NativeExpr::Local("left".to_string())),
                        right: Box::new(NativeExpr::Local("right".to_string())),
                    },
                    right: NativeExpr::Int(42),
                },
                evaluated_condition: true,
                then_block: Box::new(NativeBlock {
                    statements: Vec::new(),
                    terminator: NativeTerminator::Return {
                        expr: NativeExpr::Int(42),
                        evaluated_exit_code: 42,
                    },
                }),
                fallback: Box::new(NativeBlock {
                    statements: Vec::new(),
                    terminator: NativeTerminator::Return {
                        expr: NativeExpr::Int(0),
                        evaluated_exit_code: 0,
                    },
                }),
            }
        );
    }

    #[test]
    fn stage_6a_validation_preserves_boolean_condition_structure_for_codegen() {
        let program = crate::lower_source(
            "stage4b_boolean_condition.doria",
            r#"
function main(): int
{
    let $left = 20;
    let $right = 22;

    if (($left + $right == 42) and not false) {
        return 42;
    }

    return 0;
}
"#,
        )
        .expect("source should lower to HIR");

        let native_main = validate_stage_6a(&program).expect("source should validate for Stage 6a");

        assert_eq!(native_main.evaluated_exit_code, 42);
        assert_eq!(
            native_main.body.terminator,
            NativeTerminator::Guard {
                condition: NativeCondition::And {
                    left: Box::new(NativeCondition::Compare {
                        op: NativeCompareOp::Equal,
                        left: NativeExpr::Binary {
                            op: NativeBinaryOp::Add,
                            left: Box::new(NativeExpr::Local("left".to_string())),
                            right: Box::new(NativeExpr::Local("right".to_string())),
                        },
                        right: NativeExpr::Int(42),
                    }),
                    right: Box::new(NativeCondition::Not(Box::new(NativeCondition::Bool(false)))),
                },
                evaluated_condition: true,
                then_block: Box::new(NativeBlock {
                    statements: Vec::new(),
                    terminator: NativeTerminator::Return {
                        expr: NativeExpr::Int(42),
                        evaluated_exit_code: 42,
                    },
                }),
                fallback: Box::new(NativeBlock {
                    statements: Vec::new(),
                    terminator: NativeTerminator::Return {
                        expr: NativeExpr::Int(0),
                        evaluated_exit_code: 0,
                    },
                }),
            }
        );
    }

    #[test]
    fn stage_6a_validation_preserves_writable_assignment_order_for_codegen() {
        let program = crate::lower_source(
            "stage5a_writable_assignment.doria",
            r#"
function main(): int
{
    let writable $code = 40;
    $code += 2;

    return $code;
}
"#,
        )
        .expect("source should lower to HIR");

        let native_main = validate_stage_6a(&program).expect("source should validate for Stage 6a");

        assert_eq!(native_main.evaluated_exit_code, 42);
        assert_eq!(
            native_main.body.statements,
            vec![
                NativeStmt::Local(NativeLocal {
                    name: "code".to_string(),
                    writable: true,
                    expr: NativeExpr::Int(40),
                    evaluated_value: 40,
                }),
                NativeStmt::Assign(NativeAssign {
                    target: "code".to_string(),
                    op: NativeAssignOp::AddAssign,
                    expr: NativeExpr::Int(2),
                    evaluated_value: 42,
                }),
            ]
        );
        assert_eq!(
            native_main.body.terminator,
            NativeTerminator::Return {
                expr: NativeExpr::Local("code".to_string()),
                evaluated_exit_code: 42,
            }
        );
    }

    #[test]
    fn windows_default_uses_msvc_compiler_driver_arguments() {
        let args = linker_arguments(
            "cl.exe",
            false,
            true,
            Path::new("main.obj"),
            Path::new("main.exe"),
        );

        assert_eq!(
            args,
            vec![
                OsString::from("/nologo"),
                OsString::from("main.obj"),
                OsString::from("/Fe:main.exe"),
                OsString::from("/link"),
                OsString::from("/ENTRY:main"),
                OsString::from("/SUBSYSTEM:CONSOLE"),
            ]
        );
    }

    #[test]
    fn windows_clang_cl_uses_msvc_compiler_driver_arguments() {
        let args = linker_arguments(
            "clang-cl.exe",
            true,
            true,
            Path::new("main.obj"),
            Path::new("main.exe"),
        );

        assert_eq!(
            args,
            vec![
                OsString::from("/nologo"),
                OsString::from("main.obj"),
                OsString::from("/Fe:main.exe"),
                OsString::from("/link"),
                OsString::from("/ENTRY:main"),
                OsString::from("/SUBSYSTEM:CONSOLE"),
            ]
        );
    }

    #[test]
    fn unix_style_compiler_driver_uses_dash_o() {
        let args = linker_arguments(
            "clang",
            true,
            true,
            Path::new("main.obj"),
            Path::new("main.exe"),
        );

        assert_eq!(
            args,
            vec![
                OsString::from("main.obj"),
                OsString::from("-o"),
                OsString::from("main.exe"),
            ]
        );
    }
}

use std::collections::HashMap;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{types, AbiParam, InstBuilder, Value};
use cranelift_codegen::settings;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{default_libcall_names, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};

use crate::backend::BackendError;
use crate::hir::{self, BinaryOp, ElseBranch, Expr, Item, Stmt};

pub fn generate_executable(program: &hir::Program) -> Result<Vec<u8>, BackendError> {
    let native_main = validate_stage_4a(program)?;
    let object_bytes = generate_object(&native_main)?;
    link_object(&object_bytes)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeMain {
    locals: Vec<NativeLocal>,
    terminator: NativeTerminator,
    evaluated_exit_code: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeLocal {
    name: String,
    expr: NativeExpr,
    evaluated_value: i64,
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
        then_expr: NativeExpr,
        then_exit_code: i32,
        else_expr: NativeExpr,
        else_exit_code: i32,
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

pub fn validate_stage_2d(program: &hir::Program) -> Result<i32, BackendError> {
    Ok(validate_stage_4a(program)?.evaluated_exit_code)
}

fn validate_stage_4a(program: &hir::Program) -> Result<NativeMain, BackendError> {
    let mut main_functions = Vec::new();

    for item in &program.items {
        match item {
            Item::Function(function) if function.name == "main" => {
                main_functions.push(function);
            }
            Item::Function(function) => {
                return Err(BackendError::new(format!(
                    "unsupported top-level item for native Stage 2d: extra top-level function `{}`",
                    function.name
                )));
            }
            Item::Class(class_decl) => {
                return Err(BackendError::new(format!(
                    "unsupported top-level item for native Stage 2d: class `{}`",
                    class_decl.name
                )));
            }
            Item::Statement(statement) => {
                return Err(BackendError::new(format!(
                    "unsupported top-level item for native Stage 2d: {}",
                    describe_statement(statement)
                )));
            }
        }
    }

    let [main] = main_functions.as_slice() else {
        return Err(match main_functions.len() {
            0 => BackendError::new(
                "no native entrypoint found; Stage 2d native output requires exactly one top-level `function main(): int`",
            ),
            _ => BackendError::new(
                "multiple native entrypoints found; Stage 2d native output requires exactly one top-level `function main(): int`",
            ),
        });
    };

    if !main.params.is_empty() {
        return Err(BackendError::new(
            "wrong main signature for native Stage 2d: `main` must not declare parameters",
        ));
    }

    if !matches!(
        main.return_type.as_ref(),
        Some(return_type) if return_type.name == "int" && return_type.args.is_empty()
    ) {
        return Err(BackendError::new(
            "wrong main signature for native Stage 2d: expected `function main(): int`",
        ));
    }

    let mut local_values = HashMap::new();
    let mut locals = Vec::new();

    let mut terminal_index = 0;
    while let Some(Stmt::VarDecl(decl)) = main.body.statements.get(terminal_index) {
        let local = validate_stage_3a_local(decl, &local_values)?;
        local_values.insert(local.name.clone(), local.evaluated_value);
        locals.push(local);
        terminal_index += 1;
    }

    let terminator = validate_stage_4a_statement_sequence(
        &main.body.statements[terminal_index..],
        &local_values,
    )?;
    let evaluated_exit_code = evaluate_native_terminator_exit_code(&terminator);
    Ok(NativeMain {
        locals,
        terminator,
        evaluated_exit_code,
    })
}

fn validate_stage_4a_statement_sequence(
    statements: &[Stmt],
    local_values: &HashMap<String, i64>,
) -> Result<NativeTerminator, BackendError> {
    match statements {
        [] => Err(BackendError::new(
            "unsupported native terminal statement for Stage 4a: expected final return, terminal if/else, or guard if followed by fallback return",
        )),
        [statement] => validate_stage_4a_terminator(statement, local_values),
        [
            Stmt::If(if_stmt),
            Stmt::Return {
                expr: Some(fallback_expr),
                ..
            },
        ] if if_stmt.else_branch.is_none() => {
            validate_stage_4a_guard_return(if_stmt, fallback_expr, local_values)
        }
        [Stmt::If(if_stmt), _] if if_stmt.else_branch.is_some() => {
            Err(BackendError::new(
                "unsupported native statement for Stage 4a: no statements may follow a terminal if/else",
            ))
        }
        [Stmt::Return { .. }, ..] => Err(BackendError::new(
            "unsupported native statement for Stage 4a: no statements may follow a final return",
        )),
        [first, ..] => Err(BackendError::new(format!(
            "unsupported native statement for Stage 4a: expected readonly `int` local declaration, final return, terminal if/else, or guard if followed by fallback return, found {}",
            describe_statement(first)
        ))),
    }
}

fn validate_stage_4a_guard_return(
    if_stmt: &hir::IfStmt,
    fallback_expr: &Expr,
    local_values: &HashMap<String, i64>,
) -> Result<NativeTerminator, BackendError> {
    let condition = validate_stage_4a_condition(&if_stmt.condition, local_values)?;
    let (then_expr, then_exit_code) =
        validate_stage_4a_branch(&if_stmt.then_block.statements, local_values)?;
    let (else_expr, else_exit_code) = validate_stage_4a_return_expr(fallback_expr, local_values)?;

    Ok(NativeTerminator::IfElse {
        condition: condition.condition,
        evaluated_condition: condition.value,
        then_expr,
        then_exit_code,
        else_expr,
        else_exit_code,
    })
}

fn validate_stage_3a_local(
    decl: &hir::VarDecl,
    local_values: &HashMap<String, i64>,
) -> Result<NativeLocal, BackendError> {
    if decl.writable {
        return Err(unsupported_stage_2d_local());
    }

    if let Some(ty) = &decl.ty {
        if ty.name != "int" || !ty.args.is_empty() {
            return Err(unsupported_stage_2d_local());
        }
    }

    let initializer = validate_stage_3a_int_expr(&decl.initializer, local_values)?;
    Ok(NativeLocal {
        name: decl.name.clone(),
        expr: initializer.expr,
        evaluated_value: initializer.value,
    })
}

fn unsupported_stage_2d_local() -> BackendError {
    BackendError::new(
        "unsupported native local for Stage 2d: expected readonly `int` local initialized from integer literals, readonly integer locals, or supported integer arithmetic",
    )
}

fn validate_stage_4a_terminator(
    statement: &Stmt,
    local_values: &HashMap<String, i64>,
) -> Result<NativeTerminator, BackendError> {
    match statement {
        Stmt::Return { expr: Some(expr), .. } => {
            let (expr, evaluated_exit_code) = validate_stage_4a_return_expr(expr, local_values)?;
            Ok(NativeTerminator::Return {
                expr,
                evaluated_exit_code,
            })
        }
        Stmt::Return { expr: None, .. } => Err(BackendError::new(
            "unsupported native terminal statement for Stage 4a: expected `return <portable integer expression>;`, found bare `return;`",
        )),
        Stmt::If(if_stmt) => {
            let condition = validate_stage_4a_condition(&if_stmt.condition, local_values)?;
            let (then_expr, then_exit_code) =
                validate_stage_4a_branch(&if_stmt.then_block.statements, local_values)?;

            let Some(else_branch) = &if_stmt.else_branch else {
                return Err(BackendError::new(
                    "unsupported native terminal if for Stage 4a: terminal if requires else; guard if without else is supported only when followed by a fallback return",
                ));
            };

            let ElseBranch::Block(else_block) = else_branch else {
                return Err(BackendError::new(
                    "unsupported native terminal statement for Stage 4a: else-if is not supported",
                ));
            };

            let (else_expr, else_exit_code) =
                validate_stage_4a_branch(&else_block.statements, local_values)?;

            Ok(NativeTerminator::IfElse {
                condition: condition.condition,
                evaluated_condition: condition.value,
                then_expr,
                then_exit_code,
                else_expr,
                else_exit_code,
            })
        }
        other => Err(BackendError::new(format!(
            "unsupported native terminal statement for Stage 4a: expected final return or terminal if/else, found {}",
            describe_statement(other)
        ))),
    }
}

fn validate_stage_4a_branch(
    statements: &[Stmt],
    local_values: &HashMap<String, i64>,
) -> Result<(NativeExpr, i32), BackendError> {
    let [Stmt::Return {
        expr: Some(expr), ..
    }] = statements
    else {
        return Err(BackendError::new(
            "unsupported native branch for Stage 4a: expected exactly one return statement",
        ));
    };

    validate_stage_4a_return_expr(expr, local_values)
}

fn validate_stage_4a_return_expr(
    expr: &Expr,
    local_values: &HashMap<String, i64>,
) -> Result<(NativeExpr, i32), BackendError> {
    let return_expr = validate_stage_3a_int_expr(expr, local_values)?;
    let evaluated_exit_code = parse_stage_4a_exit_code(return_expr.value)?;
    Ok((return_expr.expr, evaluated_exit_code))
}

fn validate_stage_3a_int_expr(
    expr: &Expr,
    local_values: &HashMap<String, i64>,
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
            let value = local_values.get(name).copied().ok_or_else(|| {
                BackendError::new(
                    "unsupported native expression for Stage 2d: expected integer literal, readonly integer local, or supported integer arithmetic",
                )
            })?;
            Ok(ValidatedNativeExpr {
                expr: NativeExpr::Local(name.clone()),
                value,
            })
        }
        Expr::Binary {
            left, op, right, ..
        } if native_binary_op(op).is_some() => {
            let native_op = native_binary_op(op).expect("checked by guard");
            let left = validate_stage_3a_int_expr(left, local_values)?;
            let right = validate_stage_3a_int_expr(right, local_values)?;
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
                "unsupported native arithmetic operator for Stage 2d",
            ))
        }
        other => Err(BackendError::new(format!(
            "unsupported native expression for Stage 2d: expected integer literal, readonly integer local, or supported integer arithmetic, found `{}`",
            describe_expression(other)
        ))),
    }
}

fn validate_stage_4a_condition(
    expr: &Expr,
    local_values: &HashMap<String, i64>,
) -> Result<ValidatedNativeCondition, BackendError> {
    match expr {
        Expr::Bool { value, .. } => Ok(ValidatedNativeCondition {
            condition: NativeCondition::Bool(*value),
            value: *value,
        }),
        Expr::Binary {
            left, op, right, ..
        } if native_compare_op(op).is_some() => {
            let native_op = native_compare_op(op).expect("checked by guard");
            let left = validate_stage_4a_comparison_operand(left, local_values)?;
            let right = validate_stage_4a_comparison_operand(right, local_values)?;
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
            op: BinaryOp::StrictEqual | BinaryOp::NotStrictEqual,
            ..
        } => Err(BackendError::new(
            "unsupported native comparison operator for Stage 4a",
        )),
        _ => Err(BackendError::new(
            "unsupported native condition for Stage 4a: expected bool literal or supported integer comparison",
        )),
    }
}

fn validate_stage_4a_comparison_operand(
    expr: &Expr,
    local_values: &HashMap<String, i64>,
) -> Result<ValidatedNativeExpr, BackendError> {
    validate_stage_3a_int_expr(expr, local_values).map_err(|error| {
        if should_preserve_comparison_operand_error(&error.message) {
            error
        } else {
            BackendError::new(
                "unsupported native comparison for Stage 4a: expected supported integer expressions",
            )
        }
    })
}

fn should_preserve_comparison_operand_error(message: &str) -> bool {
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

fn evaluate_native_terminator_exit_code(terminator: &NativeTerminator) -> i32 {
    match terminator {
        NativeTerminator::Return {
            evaluated_exit_code,
            ..
        } => *evaluated_exit_code,
        NativeTerminator::IfElse {
            evaluated_condition,
            then_exit_code,
            else_exit_code,
            ..
        } => {
            if *evaluated_condition {
                *then_exit_code
            } else {
                *else_exit_code
            }
        }
    }
}

fn parse_doria_int_literal(value: &str) -> Result<i64, BackendError> {
    value
        .parse::<i64>()
        .map_err(|_| BackendError::new("integer literal is outside the Doria `int` range"))
}

fn parse_stage_4a_exit_code(value: i64) -> Result<i32, BackendError> {
    if !(0..=125).contains(&value) {
        return Err(BackendError::new(
            "native Stage 4a exit code must be in the range 0..125",
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
        ObjectBuilder::new(isa, "doria_stage_4a", default_libcall_names())
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
        for local in &native_main.locals {
            let value = lower_native_expr(&mut builder, &local.expr, &lowered_local_values)?;
            lowered_local_values.insert(local.name.clone(), value);
            evaluated_local_values.insert(local.name.clone(), local.evaluated_value);
        }

        lower_native_terminator(
            &mut builder,
            &native_main.terminator,
            &lowered_local_values,
            &evaluated_local_values,
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
            then_expr,
            then_exit_code,
            else_expr,
            else_exit_code,
        } => {
            let condition_value = lower_native_condition(builder, condition, lowered_local_values)?;
            let Some(evaluated_condition_value) =
                evaluate_native_condition(condition, evaluated_local_values)
            else {
                return Err(BackendError::new(
                    "backend emission failure: validated native condition could not be re-evaluated",
                ));
            };
            debug_assert_eq!(evaluated_condition_value, *evaluated_condition);

            let then_block = builder.create_block();
            let else_block = builder.create_block();
            builder
                .ins()
                .brif(condition_value, then_block, &[], else_block, &[]);

            builder.switch_to_block(then_block);
            builder.seal_block(then_block);
            lower_native_return(
                builder,
                then_expr,
                *then_exit_code,
                lowered_local_values,
                evaluated_local_values,
            )?;

            builder.switch_to_block(else_block);
            builder.seal_block(else_block);
            lower_native_return(
                builder,
                else_expr,
                *else_exit_code,
                lowered_local_values,
                evaluated_local_values,
            )
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

fn lower_native_condition(
    builder: &mut FunctionBuilder,
    condition: &NativeCondition,
    local_values: &HashMap<String, Value>,
) -> Result<Value, BackendError> {
    match condition {
        NativeCondition::Bool(value) => Ok(builder.ins().iconst(types::I8, i64::from(*value))),
        NativeCondition::Compare { op, left, right } => {
            let left = lower_native_expr(builder, left, local_values)?;
            let right = lower_native_expr(builder, right, local_values)?;
            Ok(builder.ins().icmp(native_compare_intcc(*op), left, right))
        }
    }
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
    // Stage 4a emits a Cranelift object file and asks the host toolchain to
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
        // For Stage 4a's tiny main, make Doria's main the executable entrypoint
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
        Expr::Binary { .. } => "binary expression",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_4a_validation_preserves_return_expression_structure_for_codegen() {
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

        let native_main = validate_stage_4a(&program).expect("source should validate for Stage 4a");

        assert_eq!(native_main.evaluated_exit_code, 42);
        assert_eq!(native_main.locals.len(), 2);
        assert_eq!(native_main.locals[0].name, "left");
        assert_eq!(native_main.locals[0].expr, NativeExpr::Int(20));
        assert_eq!(native_main.locals[0].evaluated_value, 20);
        assert_eq!(native_main.locals[1].name, "right");
        assert_eq!(native_main.locals[1].expr, NativeExpr::Int(22));
        assert_eq!(native_main.locals[1].evaluated_value, 22);

        assert_eq!(
            native_main.terminator,
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
    fn stage_4a_validation_preserves_if_else_structure_for_codegen() {
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

        let native_main = validate_stage_4a(&program).expect("source should validate for Stage 4a");

        assert_eq!(native_main.evaluated_exit_code, 42);
        assert_eq!(native_main.locals.len(), 2);
        assert_eq!(
            native_main.terminator,
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
                then_expr: NativeExpr::Int(42),
                then_exit_code: 42,
                else_expr: NativeExpr::Int(0),
                else_exit_code: 0,
            }
        );
    }

    #[test]
    fn stage_4a_validation_preserves_guard_if_structure_for_codegen() {
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

        let native_main = validate_stage_4a(&program).expect("source should validate for Stage 4a");

        assert_eq!(native_main.evaluated_exit_code, 42);
        assert_eq!(native_main.locals.len(), 2);
        assert_eq!(
            native_main.terminator,
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
                then_expr: NativeExpr::Int(42),
                then_exit_code: 42,
                else_expr: NativeExpr::Int(0),
                else_exit_code: 0,
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

use std::collections::{HashMap, HashSet};

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{
    types, AbiParam, Block, BlockArg, InstBuilder, StackSlotData, StackSlotKind, Value,
};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::DataDescription;
use cranelift_module::{default_libcall_names, FuncId, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};

use crate::backend::BackendError;
use crate::hir::{self, AssignOp, BinaryOp, ElseBranch, Expr, Item, Stmt, UnaryOp};

const STAGE_7B_LOOP_VERIFICATION_CAP: u64 = 10_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeSmokeModule {
    body: NativeSmokeBlock,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeSmokeBlock {
    statements: Vec<NativeSmokeStmt>,
    terminator: NativeSmokeTerminator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum NativeSmokeStmt {
    Local(NativeSmokeLocal),
    Assign(NativeSmokeAssign),
    EchoStringLiteral { value: String },
    While(NativeSmokeWhile),
    If(NativeSmokeIf),
    Break,
    Continue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeSmokeLocal {
    name: String,
    writable: bool,
    expr: NativeSmokeExpr,
    evaluated_value: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeSmokeAssign {
    target: String,
    op: NativeSmokeAssignOp,
    expr: NativeSmokeExpr,
    evaluated_value: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeSmokeWhile {
    condition: NativeSmokeCondition,
    body: NativeSmokeFallthroughBlock,
    final_values: Vec<(String, i64)>,
    evaluated_iterations: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeSmokeIf {
    condition: NativeSmokeCondition,
    evaluated_condition: bool,
    then_block: NativeSmokeFallthroughBlock,
    else_block: Option<NativeSmokeFallthroughBlock>,
    merged_values: Vec<(String, i64)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeSmokeFallthroughBlock {
    statements: Vec<NativeSmokeStmt>,
    final_states: HashMap<String, NativeSmokeLocalState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeSmokeLoopControl {
    Fallthrough,
    Break,
    Continue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeSmokeLoopEvaluation {
    visible_states: HashMap<String, NativeSmokeLocalState>,
    control: NativeSmokeLoopControl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeSmokeLoweringFlow {
    Fallthrough,
    Diverged,
}

#[derive(Debug, Clone, Copy)]
struct NativeSmokeLoopLoweringContext<'a> {
    header_block: Block,
    after_block: Block,
    carried_locals: &'a [(String, i64)],
}

struct NativeSmokeLoweringResources<'module> {
    module: &'module mut ObjectModule,
    write_func_id: Option<FuncId>,
    get_std_handle_func_id: Option<FuncId>,
    write_file_func_id: Option<FuncId>,
    next_string_literal_id: usize,
}

impl<'module> NativeSmokeLoweringResources<'module> {
    fn new(module: &'module mut ObjectModule) -> Self {
        Self {
            module,
            write_func_id: None,
            get_std_handle_func_id: None,
            write_file_func_id: None,
            next_string_literal_id: 0,
        }
    }

    fn declare_write_function(&mut self) -> Result<FuncId, BackendError> {
        if let Some(function_id) = self.write_func_id {
            return Ok(function_id);
        }

        let pointer_type = self.module.target_config().pointer_type();
        let mut signature = self.module.make_signature();
        signature.params.push(AbiParam::new(types::I32));
        signature.params.push(AbiParam::new(pointer_type));
        signature.params.push(AbiParam::new(pointer_type));
        signature.returns.push(AbiParam::new(pointer_type));

        let function_id = self
            .module
            .declare_function("write", Linkage::Import, &signature)
            .map_err(|error| BackendError::new(format!("backend emission failure: {error}")))?;
        self.write_func_id = Some(function_id);
        Ok(function_id)
    }

    fn declare_get_std_handle_function(&mut self) -> Result<FuncId, BackendError> {
        if let Some(function_id) = self.get_std_handle_func_id {
            return Ok(function_id);
        }

        let pointer_type = self.module.target_config().pointer_type();
        let mut signature = self.module.make_signature();
        signature.params.push(AbiParam::new(types::I32));
        signature.returns.push(AbiParam::new(pointer_type));

        let function_id = self
            .module
            .declare_function("GetStdHandle", Linkage::Import, &signature)
            .map_err(|error| BackendError::new(format!("backend emission failure: {error}")))?;
        self.get_std_handle_func_id = Some(function_id);
        Ok(function_id)
    }

    fn declare_write_file_function(&mut self) -> Result<FuncId, BackendError> {
        if let Some(function_id) = self.write_file_func_id {
            return Ok(function_id);
        }

        let pointer_type = self.module.target_config().pointer_type();
        let mut signature = self.module.make_signature();
        signature.params.push(AbiParam::new(pointer_type));
        signature.params.push(AbiParam::new(pointer_type));
        signature.params.push(AbiParam::new(types::I32));
        signature.params.push(AbiParam::new(pointer_type));
        signature.params.push(AbiParam::new(pointer_type));
        signature.returns.push(AbiParam::new(types::I32));

        let function_id = self
            .module
            .declare_function("WriteFile", Linkage::Import, &signature)
            .map_err(|error| BackendError::new(format!("backend emission failure: {error}")))?;
        self.write_file_func_id = Some(function_id);
        Ok(function_id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeSmokeStdoutPlatform {
    Unix,
    Windows,
    Unsupported,
}

fn native_smoke_stdout_platform(windows: bool, unix: bool) -> NativeSmokeStdoutPlatform {
    if windows {
        NativeSmokeStdoutPlatform::Windows
    } else if unix {
        NativeSmokeStdoutPlatform::Unix
    } else {
        NativeSmokeStdoutPlatform::Unsupported
    }
}

fn host_native_smoke_stdout_platform() -> NativeSmokeStdoutPlatform {
    native_smoke_stdout_platform(cfg!(windows), cfg!(unix))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeSmokeAssignOp {
    Assign,
    AddAssign,
    SubAssign,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum NativeSmokeExpr {
    Int(i64),
    Local(String),
    Binary {
        op: NativeSmokeBinaryOp,
        left: Box<NativeSmokeExpr>,
        right: Box<NativeSmokeExpr>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeSmokeBinaryOp {
    Add,
    Subtract,
    Multiply,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum NativeSmokeTerminator {
    ExitSuccess,
    Return {
        expr: NativeSmokeExpr,
        evaluated_exit_code: i32,
    },
    IfElse {
        condition: NativeSmokeCondition,
        evaluated_condition: bool,
        then_block: Box<NativeSmokeBlock>,
        else_block: Box<NativeSmokeBlock>,
    },
    Guard {
        condition: NativeSmokeCondition,
        evaluated_condition: bool,
        then_block: Box<NativeSmokeBlock>,
        fallback: Box<NativeSmokeBlock>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeSmokeMainReturn {
    Int,
    Void,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum NativeSmokeCondition {
    Bool(bool),
    Compare {
        op: NativeSmokeCompareOp,
        left: NativeSmokeExpr,
        right: NativeSmokeExpr,
    },
    Not(Box<NativeSmokeCondition>),
    And {
        left: Box<NativeSmokeCondition>,
        right: Box<NativeSmokeCondition>,
    },
    Or {
        left: Box<NativeSmokeCondition>,
        right: Box<NativeSmokeCondition>,
    },
    Xor {
        left: Box<NativeSmokeCondition>,
        right: Box<NativeSmokeCondition>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeSmokeCompareOp {
    Equal,
    NotEqual,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ValidatedNativeSmokeExpr {
    expr: NativeSmokeExpr,
    value: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ValidatedNativeSmokeCondition {
    condition: NativeSmokeCondition,
    value: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeSmokeLocalState {
    writable: bool,
    value: i64,
}

pub(crate) fn validate(program: &hir::Program) -> Result<NativeSmokeModule, BackendError> {
    validate_stage_6c(program)
}

fn validate_stage_6c(program: &hir::Program) -> Result<NativeSmokeModule, BackendError> {
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
                "no native entrypoint found; native Stage 7b output requires exactly one top-level `function main(): int` or `function main(): void`",
            ),
            _ => BackendError::new(
                "multiple native entrypoints found; native Stage 7b output requires exactly one top-level `function main(): int` or `function main(): void`",
            ),
        });
    };

    if !main.params.is_empty() {
        return Err(BackendError::new(
            "wrong main signature for native Stage 7b: `main` must not declare parameters",
        ));
    }

    let main_return = validate_native_main_return(main)?;

    let body = validate_stage_6c_block(&main.body.statements, &HashMap::new(), main_return)?;
    Ok(NativeSmokeModule { body })
}

fn validate_native_main_return(
    main: &hir::FunctionDecl,
) -> Result<NativeSmokeMainReturn, BackendError> {
    match main.return_type.as_ref() {
        Some(return_type) if return_type.name == "int" && return_type.args.is_empty() => {
            Ok(NativeSmokeMainReturn::Int)
        }
        Some(return_type) if return_type.name == "void" && return_type.args.is_empty() => {
            Ok(NativeSmokeMainReturn::Void)
        }
        _ => Err(BackendError::new(
            "wrong main signature for native Stage 7b: expected `function main(): int` or `function main(): void`",
        )),
    }
}

fn validate_stage_6c_block(
    statements: &[Stmt],
    local_states: &HashMap<String, NativeSmokeLocalState>,
    main_return: NativeSmokeMainReturn,
) -> Result<NativeSmokeBlock, BackendError> {
    let mut block_states = local_states.clone();
    let mut native_statements = Vec::new();
    let mut terminal_index = 0;

    while let Some(statement) = statements.get(terminal_index) {
        match statement {
            Stmt::VarDecl(decl) => {
                let local = validate_stage_6c_local(decl, &block_states)?;
                block_states.insert(
                    local.name.clone(),
                    NativeSmokeLocalState {
                        writable: local.writable,
                        value: local.evaluated_value,
                    },
                );
                native_statements.push(NativeSmokeStmt::Local(local));
                terminal_index += 1;
            }
            Stmt::Assignment(assignment) => {
                let assignment = validate_stage_6c_assignment(assignment, &block_states)?;
                let Some(state) = block_states.get_mut(&assignment.target) else {
                    return Err(BackendError::new(
                        "backend validation failure: validated native assignment target was not declared",
                    ));
                };
                state.value = assignment.evaluated_value;
                native_statements.push(NativeSmokeStmt::Assign(assignment));
                terminal_index += 1;
            }
            Stmt::Echo { expr, .. } => {
                native_statements.push(validate_stage_6c_echo(expr)?);
                terminal_index += 1;
            }
            Stmt::While(while_stmt) => {
                let native_while = validate_stage_6c_while(while_stmt, &block_states)?;
                for (name, value) in &native_while.final_values {
                    let Some(state) = block_states.get_mut(name) else {
                        return Err(BackendError::new(
                            "backend validation failure: validated native while target was not declared",
                        ));
                    };
                    state.value = *value;
                }
                native_statements.push(NativeSmokeStmt::While(native_while));
                terminal_index += 1;
            }
            Stmt::If(if_stmt) => match validate_stage_6c_fallthrough_if(if_stmt, &block_states) {
                Ok(native_if) => {
                    merge_native_values(&mut block_states, &native_if.merged_values)?;
                    native_statements.push(NativeSmokeStmt::If(native_if));
                    terminal_index += 1;
                }
                Err(error) if should_defer_if_to_native_terminator(&error.message) => break,
                Err(error) => return Err(error),
            },
            _ => break,
        }
    }

    let terminator = validate_stage_6c_statement_sequence(
        &statements[terminal_index..],
        &block_states,
        main_return,
    )?;

    Ok(NativeSmokeBlock {
        statements: native_statements,
        terminator,
    })
}

fn merge_native_values(
    local_states: &mut HashMap<String, NativeSmokeLocalState>,
    values: &[(String, i64)],
) -> Result<(), BackendError> {
    for (name, value) in values {
        let Some(state) = local_states.get_mut(name) else {
            return Err(BackendError::new(format!(
                "backend validation failure: validated native merged local `{name}` was not declared",
            )));
        };
        state.value = *value;
    }

    Ok(())
}

fn validate_stage_6c_statement_sequence(
    statements: &[Stmt],
    local_states: &HashMap<String, NativeSmokeLocalState>,
    main_return: NativeSmokeMainReturn,
) -> Result<NativeSmokeTerminator, BackendError> {
    match statements {
        [] if main_return == NativeSmokeMainReturn::Void => Ok(NativeSmokeTerminator::ExitSuccess),
        [] => Err(BackendError::new(
            "unsupported native block for Stage 7b: expected supported local declarations, assignments, string-literal echo statements, bounded while statements, or fallthrough if statements followed by a return, terminal if/else, or guard if with fallback",
        )),
        [Stmt::If(if_stmt), rest @ ..] if if_stmt.else_branch.is_none() => {
            validate_stage_6c_guard(if_stmt, rest, local_states, main_return)
        }
        [statement] => validate_stage_6c_terminator(statement, local_states, main_return),
        [Stmt::If(if_stmt), _] if if_stmt.else_branch.is_some() => {
            Err(BackendError::new(
                "unsupported statement after native terminator for Stage 7b: no statements may follow a terminal if/else",
            ))
        }
        [Stmt::Return { .. }, ..] => Err(BackendError::new(
            "unsupported statement after native terminator for Stage 7b: no statements may follow a final return",
        )),
        [first, ..] => Err(BackendError::new(format!(
            "unsupported native statement for Stage 7b: expected supported block local declaration, block assignment, bounded while statement, fallthrough if statement, final return, terminal if/else, or guard if followed by fallback block, found {}",
            describe_statement(first)
        ))),
    }
}

fn validate_stage_6c_guard(
    if_stmt: &hir::IfStmt,
    fallback_statements: &[Stmt],
    local_states: &HashMap<String, NativeSmokeLocalState>,
    main_return: NativeSmokeMainReturn,
) -> Result<NativeSmokeTerminator, BackendError> {
    if fallback_statements.is_empty() && main_return != NativeSmokeMainReturn::Void {
        return Err(BackendError::new(
            "unsupported native branch fallthrough for Stage 7b: guard `if` without `else` requires a supported fallback block",
        ));
    }

    let condition = validate_stage_6c_condition(&if_stmt.condition, local_states)?;
    let then_block =
        validate_stage_6c_branch(&if_stmt.then_block.statements, local_states, main_return)?;
    let fallback = validate_stage_6c_block(fallback_statements, local_states, main_return)?;

    Ok(NativeSmokeTerminator::Guard {
        condition: condition.condition,
        evaluated_condition: condition.value,
        then_block: Box::new(then_block),
        fallback: Box::new(fallback),
    })
}

fn validate_stage_6c_fallthrough_if(
    if_stmt: &hir::IfStmt,
    local_states: &HashMap<String, NativeSmokeLocalState>,
) -> Result<NativeSmokeIf, BackendError> {
    let condition =
        validate_stage_6c_condition(&if_stmt.condition, local_states).map_err(|error| {
            if should_preserve_native_expression_error(&error.message) {
                error
            } else {
                BackendError::new(
                    "unsupported native fallthrough if for Stage 7b: expected supported boolean condition",
                )
            }
        })?;

    let then_block =
        validate_stage_6c_fallthrough_block(&if_stmt.then_block.statements, local_states)?;
    let else_block = match &if_stmt.else_branch {
        Some(ElseBranch::Block(block)) => Some(validate_stage_6c_fallthrough_block(
            &block.statements,
            local_states,
        )?),
        Some(ElseBranch::If(else_if)) => {
            let else_if_statement = Stmt::If((**else_if).clone());
            Some(validate_stage_6c_fallthrough_block(
                &[else_if_statement],
                local_states,
            )?)
        }
        None => None,
    };

    let selected_states = if condition.value {
        &then_block.final_states
    } else {
        else_block
            .as_ref()
            .map(|block| &block.final_states)
            .unwrap_or(local_states)
    };

    let mut merged_values = Vec::new();
    for name in sorted_native_local_names(local_states) {
        let Some(state) = selected_states.get(&name) else {
            return Err(BackendError::new(format!(
                "backend validation failure: validated native fallthrough if lost visible local `{name}`",
            )));
        };
        merged_values.push((name, state.value));
    }

    Ok(NativeSmokeIf {
        condition: condition.condition,
        evaluated_condition: condition.value,
        then_block,
        else_block,
        merged_values,
    })
}

fn validate_stage_6c_fallthrough_block(
    statements: &[Stmt],
    local_states: &HashMap<String, NativeSmokeLocalState>,
) -> Result<NativeSmokeFallthroughBlock, BackendError> {
    let mut block_states = local_states.clone();
    let mut visible_states = local_states.clone();
    let mut shadowed_locals = HashSet::new();
    let mut native_statements = Vec::new();

    for statement in statements {
        match statement {
            Stmt::VarDecl(decl) => {
                let local = validate_stage_6c_local(decl, &block_states)?;
                block_states.insert(
                    local.name.clone(),
                    NativeSmokeLocalState {
                        writable: local.writable,
                        value: local.evaluated_value,
                    },
                );
                if visible_states.contains_key(&local.name) {
                    shadowed_locals.insert(local.name.clone());
                }
                native_statements.push(NativeSmokeStmt::Local(local));
            }
            Stmt::Assignment(assignment) => {
                let assignment = validate_stage_6c_assignment(assignment, &block_states)?;
                let Some(state) = block_states.get_mut(&assignment.target) else {
                    return Err(BackendError::new(
                        "backend validation failure: validated native fallthrough assignment target was not declared",
                    ));
                };
                state.value = assignment.evaluated_value;
                if visible_states.contains_key(&assignment.target)
                    && !shadowed_locals.contains(&assignment.target)
                {
                    let Some(visible_state) = visible_states.get_mut(&assignment.target) else {
                        return Err(BackendError::new(
                            "backend validation failure: validated native visible fallthrough assignment target was not declared",
                        ));
                    };
                    visible_state.value = assignment.evaluated_value;
                }
                native_statements.push(NativeSmokeStmt::Assign(assignment));
            }
            Stmt::Echo { expr, .. } => {
                native_statements.push(validate_stage_6c_echo(expr)?);
            }
            Stmt::While(while_stmt) => {
                let native_while = validate_stage_6c_while(while_stmt, &block_states)?;
                merge_native_values(&mut block_states, &native_while.final_values)?;
                merge_visible_native_values(
                    &mut visible_states,
                    &shadowed_locals,
                    &native_while.final_values,
                )?;
                native_statements.push(NativeSmokeStmt::While(native_while));
            }
            Stmt::If(if_stmt) => {
                let native_if = validate_stage_6c_fallthrough_if(if_stmt, &block_states)?;
                merge_native_values(&mut block_states, &native_if.merged_values)?;
                merge_visible_native_values(
                    &mut visible_states,
                    &shadowed_locals,
                    &native_if.merged_values,
                )?;
                native_statements.push(NativeSmokeStmt::If(native_if));
            }
            Stmt::Return { .. } => {
                return Err(BackendError::new(
                    "unsupported native fallthrough branch for Stage 7b: return inside a fallthrough branch is future native work",
                ));
            }
            other => {
                return Err(BackendError::new(format!(
                    "unsupported native fallthrough branch for Stage 7b: expected supported local declaration, assignment, string-literal echo, bounded structured while, or nested fallthrough if, found {}",
                    describe_statement(other)
                )));
            }
        }
    }

    Ok(NativeSmokeFallthroughBlock {
        statements: native_statements,
        final_states: visible_states,
    })
}

fn merge_visible_native_values(
    visible_states: &mut HashMap<String, NativeSmokeLocalState>,
    shadowed_locals: &HashSet<String>,
    values: &[(String, i64)],
) -> Result<(), BackendError> {
    for (name, value) in values {
        if !visible_states.contains_key(name) || shadowed_locals.contains(name) {
            continue;
        }

        let Some(state) = visible_states.get_mut(name) else {
            return Err(BackendError::new(format!(
                "backend validation failure: validated native visible local `{name}` was not declared",
            )));
        };
        state.value = *value;
    }

    Ok(())
}

fn should_defer_if_to_native_terminator(message: &str) -> bool {
    message.contains("return inside a fallthrough branch")
}

fn sorted_native_local_names(local_states: &HashMap<String, NativeSmokeLocalState>) -> Vec<String> {
    let mut names = local_states.keys().cloned().collect::<Vec<_>>();
    names.sort();
    names
}

fn validate_stage_6c_local(
    decl: &hir::VarDecl,
    local_states: &HashMap<String, NativeSmokeLocalState>,
) -> Result<NativeSmokeLocal, BackendError> {
    if let Some(ty) = &decl.ty {
        if ty.name != "int" || !ty.args.is_empty() {
            return Err(unsupported_current_native_local());
        }
    }

    let initializer =
        validate_stage_6c_int_expr(&decl.initializer, local_states).map_err(|error| {
            if should_preserve_native_expression_error(&error.message) {
                error
            } else {
                unsupported_current_native_local()
            }
        })?;
    Ok(NativeSmokeLocal {
        name: decl.name.clone(),
        writable: decl.writable,
        expr: initializer.expr,
        evaluated_value: initializer.value,
    })
}

fn validate_stage_6c_echo(expr: &Expr) -> Result<NativeSmokeStmt, BackendError> {
    let Expr::String { value, .. } = expr else {
        return Err(BackendError::new(format!(
            "unsupported native echo expression for Stage 7b: expected string literal, found `{}`",
            describe_expression(expr)
        )));
    };

    Ok(NativeSmokeStmt::EchoStringLiteral {
        value: value.clone(),
    })
}

fn unsupported_current_native_local() -> BackendError {
    BackendError::new(
        "unsupported native local for current native smoke backend: expected readonly or writable `int` local initialized from integer literals, supported integer locals, or supported integer arithmetic",
    )
}

fn validate_stage_6c_assignment(
    assignment: &hir::Assignment,
    local_states: &HashMap<String, NativeSmokeLocalState>,
) -> Result<NativeSmokeAssign, BackendError> {
    let Expr::Variable { name, .. } = &assignment.target else {
        return Err(BackendError::new(
            "unsupported native assignment target for Stage 7b: expected writable `int` local",
        ));
    };

    let Some(target) = local_states.get(name) else {
        return Err(BackendError::new(format!(
            "unsupported native assignment target for Stage 7b: undeclared local `${name}`"
        )));
    };

    if !target.writable {
        return Err(BackendError::new(format!(
            "unsupported native assignment to readonly local for Stage 7b: `${name}`"
        )));
    }

    let value = validate_stage_6c_int_expr(&assignment.value, local_states)?;
    let (op, evaluated_value) = match assignment.op {
        AssignOp::Assign => (NativeSmokeAssignOp::Assign, value.value),
        AssignOp::AddAssign => (
            NativeSmokeAssignOp::AddAssign,
            checked_native_arithmetic(target.value, NativeSmokeBinaryOp::Add, value.value)
                .ok_or_else(|| {
                    BackendError::new("integer arithmetic overflows the Doria `int` range")
                })?,
        ),
        AssignOp::SubAssign => (
            NativeSmokeAssignOp::SubAssign,
            checked_native_arithmetic(target.value, NativeSmokeBinaryOp::Subtract, value.value)
                .ok_or_else(|| {
                    BackendError::new("integer arithmetic overflows the Doria `int` range")
                })?,
        ),
    };

    Ok(NativeSmokeAssign {
        target: name.clone(),
        op,
        expr: value.expr,
        evaluated_value,
    })
}

fn validate_stage_6c_while(
    while_stmt: &hir::WhileStmt,
    local_states: &HashMap<String, NativeSmokeLocalState>,
) -> Result<NativeSmokeWhile, BackendError> {
    let condition =
        validate_stage_6c_condition(&while_stmt.condition, local_states).map_err(|error| {
            if should_preserve_native_expression_error(&error.message) {
                error
            } else {
                BackendError::new(
                    "unsupported native while condition for Stage 7b: expected supported boolean condition",
                )
            }
        })?;

    let body = validate_stage_6c_while_body(&while_stmt.body.statements, local_states)?;
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

        if iterations == STAGE_7B_LOOP_VERIFICATION_CAP {
            return Err(stage_6c_loop_cap_error());
        }

        let body_evaluation =
            evaluate_native_scoped_statements(&body.statements, &simulated_states)?;
        simulated_states = body_evaluation.visible_states;

        iterations += 1;

        if body_evaluation.control == NativeSmokeLoopControl::Break {
            break;
        }
    }

    let mut final_values = Vec::new();
    for name in sorted_native_local_names(local_states) {
        let Some(state) = simulated_states.get(&name) else {
            return Err(BackendError::new(
                "backend validation failure: validated native while target was not declared",
            ));
        };
        final_values.push((name, state.value));
    }

    Ok(NativeSmokeWhile {
        condition: condition.condition,
        body,
        final_values,
        evaluated_iterations: iterations,
    })
}

fn validate_stage_6c_while_body(
    statements: &[Stmt],
    local_states: &HashMap<String, NativeSmokeLocalState>,
) -> Result<NativeSmokeFallthroughBlock, BackendError> {
    if statements.is_empty() {
        return Err(BackendError::new(
            "unsupported native while body for Stage 7b: expected one or more supported local declarations, assignments, or fallthrough if statements",
        ));
    }

    validate_stage_6c_while_scoped_body(statements, local_states)
}

fn validate_stage_6c_while_branch_body(
    statements: &[Stmt],
    local_states: &HashMap<String, NativeSmokeLocalState>,
) -> Result<NativeSmokeFallthroughBlock, BackendError> {
    validate_stage_6c_while_scoped_body(statements, local_states)
}

fn validate_stage_6c_while_scoped_body(
    statements: &[Stmt],
    local_states: &HashMap<String, NativeSmokeLocalState>,
) -> Result<NativeSmokeFallthroughBlock, BackendError> {
    let mut block_states = local_states.clone();
    let mut visible_states = local_states.clone();
    let mut shadowed_locals = HashSet::new();
    let mut native_statements = Vec::new();

    for statement in statements {
        match statement {
            Stmt::VarDecl(decl) => {
                let local = validate_stage_6c_loop_local(decl, &block_states)?;
                block_states.insert(
                    local.name.clone(),
                    NativeSmokeLocalState {
                        writable: local.writable,
                        value: local.evaluated_value,
                    },
                );
                if visible_states.contains_key(&local.name) {
                    shadowed_locals.insert(local.name.clone());
                }
                native_statements.push(NativeSmokeStmt::Local(local));
            }
            Stmt::Assignment(assignment) => {
                let assignment = validate_stage_6c_loop_assignment(assignment, &block_states)?;
                let Some(state) = block_states.get_mut(&assignment.target) else {
                    return Err(BackendError::new(
                        "backend validation failure: validated native while assignment target was not declared",
                    ));
                };
                state.value = assignment.evaluated_value;
                if visible_states.contains_key(&assignment.target)
                    && !shadowed_locals.contains(&assignment.target)
                {
                    let Some(visible_state) = visible_states.get_mut(&assignment.target) else {
                        return Err(BackendError::new(
                            "backend validation failure: validated native visible while assignment target was not declared",
                        ));
                    };
                    visible_state.value = assignment.evaluated_value;
                }
                native_statements.push(NativeSmokeStmt::Assign(assignment));
            }
            Stmt::Echo { expr, .. } => {
                native_statements.push(validate_stage_6c_echo(expr)?);
            }
            Stmt::If(if_stmt) => {
                let native_if = validate_stage_6c_loop_fallthrough_if(if_stmt, &block_states)?;
                merge_native_values(&mut block_states, &native_if.merged_values)?;
                merge_visible_native_values(
                    &mut visible_states,
                    &shadowed_locals,
                    &native_if.merged_values,
                )?;
                native_statements.push(NativeSmokeStmt::If(native_if));
            }
            Stmt::Break { .. } => {
                native_statements.push(NativeSmokeStmt::Break);
            }
            Stmt::Continue { .. } => {
                native_statements.push(NativeSmokeStmt::Continue);
            }
            Stmt::While(_) => {
                return Err(BackendError::new(
                    "unsupported native while body statement for Stage 7b: nested while loops are future native work",
                ));
            }
            Stmt::Return { .. } => {
                return Err(BackendError::new(
                    "unsupported native while body statement for Stage 7b: return inside while bodies is future native work",
                ));
            }
            other => {
                return Err(BackendError::new(format!(
                    "unsupported native while body statement for Stage 7b: expected local declaration, assignment, or fallthrough if, found {}",
                    describe_statement(other)
                )));
            }
        }
    }

    Ok(NativeSmokeFallthroughBlock {
        statements: native_statements,
        final_states: visible_states,
    })
}

fn validate_stage_6c_loop_local(
    decl: &hir::VarDecl,
    local_states: &HashMap<String, NativeSmokeLocalState>,
) -> Result<NativeSmokeLocal, BackendError> {
    if let Some(ty) = &decl.ty {
        if ty.name != "int" || !ty.args.is_empty() {
            return Err(unsupported_current_native_local());
        }
    }

    let expr =
        validate_stage_6c_loop_int_expr(&decl.initializer, local_states).map_err(|error| {
            if should_preserve_native_expression_error(&error.message) {
                error
            } else {
                unsupported_current_native_local()
            }
        })?;

    Ok(NativeSmokeLocal {
        name: decl.name.clone(),
        writable: decl.writable,
        expr,
        evaluated_value: 0,
    })
}

fn validate_stage_6c_loop_assignment(
    assignment: &hir::Assignment,
    local_states: &HashMap<String, NativeSmokeLocalState>,
) -> Result<NativeSmokeAssign, BackendError> {
    let Expr::Variable { name, .. } = &assignment.target else {
        return Err(BackendError::new(
            "unsupported native while assignment target for Stage 7b: expected writable `int` local",
        ));
    };

    let Some(target) = local_states.get(name) else {
        return Err(BackendError::new(format!(
            "unsupported native while assignment target for Stage 7b: undeclared local `${name}`"
        )));
    };

    if !target.writable {
        return Err(BackendError::new(format!(
            "unsupported native while assignment target for Stage 7b: readonly local `${name}`"
        )));
    }

    let value = validate_stage_6c_loop_int_expr(&assignment.value, local_states)?;
    Ok(NativeSmokeAssign {
        target: name.clone(),
        op: match assignment.op {
            AssignOp::Assign => NativeSmokeAssignOp::Assign,
            AssignOp::AddAssign => NativeSmokeAssignOp::AddAssign,
            AssignOp::SubAssign => NativeSmokeAssignOp::SubAssign,
        },
        expr: value,
        evaluated_value: 0,
    })
}

fn validate_stage_6c_loop_fallthrough_if(
    if_stmt: &hir::IfStmt,
    local_states: &HashMap<String, NativeSmokeLocalState>,
) -> Result<NativeSmokeIf, BackendError> {
    let condition =
        validate_stage_6c_loop_condition(&if_stmt.condition, local_states).map_err(|error| {
            if should_preserve_native_expression_error(&error.message) {
                error
            } else {
                BackendError::new(
                    "unsupported native while body statement for Stage 7b: expected supported fallthrough if condition",
                )
            }
        })?;

    let then_block =
        validate_stage_6c_while_branch_body(&if_stmt.then_block.statements, local_states)?;
    let else_block = match &if_stmt.else_branch {
        Some(ElseBranch::Block(block)) => Some(validate_stage_6c_while_branch_body(
            &block.statements,
            local_states,
        )?),
        Some(ElseBranch::If(else_if)) => {
            let else_if_statement = Stmt::If((**else_if).clone());
            Some(validate_stage_6c_while_branch_body(
                &[else_if_statement],
                local_states,
            )?)
        }
        None => None,
    };

    let merged_values = sorted_native_local_names(local_states)
        .into_iter()
        .map(|name| {
            let state = local_states.get(&name).ok_or_else(|| {
                BackendError::new(format!(
                    "backend validation failure: validated native while if lost visible local `{name}`",
                ))
            })?;
            Ok((name, state.value))
        })
        .collect::<Result<Vec<_>, BackendError>>()?;

    Ok(NativeSmokeIf {
        condition,
        evaluated_condition: false,
        then_block,
        else_block,
        merged_values,
    })
}

fn validate_stage_6c_loop_condition(
    expr: &Expr,
    local_states: &HashMap<String, NativeSmokeLocalState>,
) -> Result<NativeSmokeCondition, BackendError> {
    match expr {
        Expr::Bool { value, .. } => Ok(NativeSmokeCondition::Bool(*value)),
        Expr::Grouped { expr, .. } => validate_stage_6c_loop_condition(expr, local_states),
        Expr::Unary {
            op: UnaryOp::Not,
            expr,
            ..
        } => Ok(NativeSmokeCondition::Not(Box::new(
            validate_stage_6c_loop_condition(expr, local_states)?,
        ))),
        Expr::Binary {
            left, op, right, ..
        } if native_compare_op(op).is_some() => {
            let native_op = native_compare_op(op).expect("checked by guard");
            Ok(NativeSmokeCondition::Compare {
                op: native_op,
                left: validate_stage_6c_loop_int_expr(left, local_states)?,
                right: validate_stage_6c_loop_int_expr(right, local_states)?,
            })
        }
        Expr::Binary {
            left,
            op: BinaryOp::And,
            right,
            ..
        } => Ok(NativeSmokeCondition::And {
            left: Box::new(validate_stage_6c_loop_condition(left, local_states)?),
            right: Box::new(validate_stage_6c_loop_condition(right, local_states)?),
        }),
        Expr::Binary {
            left,
            op: BinaryOp::Or,
            right,
            ..
        } => Ok(NativeSmokeCondition::Or {
            left: Box::new(validate_stage_6c_loop_condition(left, local_states)?),
            right: Box::new(validate_stage_6c_loop_condition(right, local_states)?),
        }),
        Expr::Binary {
            left,
            op: BinaryOp::Xor,
            right,
            ..
        } => Ok(NativeSmokeCondition::Xor {
            left: Box::new(validate_stage_6c_loop_condition(left, local_states)?),
            right: Box::new(validate_stage_6c_loop_condition(right, local_states)?),
        }),
        _ => Err(BackendError::new(
            "unsupported native condition for Stage 7b: expected bool literal, supported integer comparison, or supported boolean condition",
        )),
    }
}

fn validate_stage_6c_loop_int_expr(
    expr: &Expr,
    local_states: &HashMap<String, NativeSmokeLocalState>,
) -> Result<NativeSmokeExpr, BackendError> {
    match expr {
        Expr::Int { value, .. } => Ok(NativeSmokeExpr::Int(parse_doria_int_literal(value)?)),
        Expr::Variable { name, .. } => {
            if !local_states.contains_key(name) {
                return Err(BackendError::new(
                    "unsupported native expression for Stage 7b: expected integer literal, supported integer local, or supported integer arithmetic",
                ));
            }

            Ok(NativeSmokeExpr::Local(name.clone()))
        }
        Expr::Grouped { expr, .. } => validate_stage_6c_loop_int_expr(expr, local_states),
        Expr::Binary {
            left, op, right, ..
        } if native_binary_op(op).is_some() => {
            let native_op = native_binary_op(op).expect("checked by guard");
            Ok(NativeSmokeExpr::Binary {
                op: native_op,
                left: Box::new(validate_stage_6c_loop_int_expr(left, local_states)?),
                right: Box::new(validate_stage_6c_loop_int_expr(right, local_states)?),
            })
        }
        Expr::Binary {
            op: BinaryOp::Div | BinaryOp::Mod,
            ..
        } => Err(BackendError::new(
            "unsupported native arithmetic operator for Stage 7b",
        )),
        other => Err(BackendError::new(format!(
            "unsupported native expression for Stage 7b: expected integer literal, supported integer local, or supported integer arithmetic, found `{}`",
            describe_expression(other)
        ))),
    }
}

fn native_state_values(
    local_states: &HashMap<String, NativeSmokeLocalState>,
) -> HashMap<String, i64> {
    local_states
        .iter()
        .map(|(name, state)| (name.clone(), state.value))
        .collect()
}

fn evaluate_native_assignment_value(
    op: NativeSmokeAssignOp,
    current_value: i64,
    expr: &NativeSmokeExpr,
    local_values: &HashMap<String, i64>,
) -> Result<i64, BackendError> {
    let value = evaluate_native_expr(expr, local_values).ok_or_else(|| {
        BackendError::new(
            "backend validation failure: validated native while assignment could not be re-evaluated",
        )
    })?;

    match op {
        NativeSmokeAssignOp::Assign => Ok(value),
        NativeSmokeAssignOp::AddAssign => {
            checked_native_arithmetic(current_value, NativeSmokeBinaryOp::Add, value).ok_or_else(
                || BackendError::new("integer arithmetic overflows the Doria `int` range"),
            )
        }
        NativeSmokeAssignOp::SubAssign => {
            checked_native_arithmetic(current_value, NativeSmokeBinaryOp::Subtract, value)
                .ok_or_else(|| {
                    BackendError::new("integer arithmetic overflows the Doria `int` range")
                })
        }
    }
}

fn evaluate_native_scoped_statements(
    statements: &[NativeSmokeStmt],
    visible_states: &HashMap<String, NativeSmokeLocalState>,
) -> Result<NativeSmokeLoopEvaluation, BackendError> {
    let mut block_states = visible_states.clone();
    let mut next_visible_states = visible_states.clone();
    let mut shadowed_locals = HashSet::new();

    for statement in statements {
        match statement {
            NativeSmokeStmt::Local(local) => {
                let values = native_state_values(&block_states);
                let value = evaluate_native_expr(&local.expr, &values).ok_or_else(|| {
                    BackendError::new(
                        "backend validation failure: validated native while local initializer could not be re-evaluated",
                    )
                })?;
                block_states.insert(
                    local.name.clone(),
                    NativeSmokeLocalState {
                        writable: local.writable,
                        value,
                    },
                );
                if next_visible_states.contains_key(&local.name) {
                    shadowed_locals.insert(local.name.clone());
                }
            }
            NativeSmokeStmt::Assign(assignment) => {
                let values = native_state_values(&block_states);
                let Some(target) = block_states.get(&assignment.target) else {
                    return Err(BackendError::new(
                        "backend validation failure: validated native while assignment target was not declared",
                    ));
                };
                let evaluated_value = evaluate_native_assignment_value(
                    assignment.op,
                    target.value,
                    &assignment.expr,
                    &values,
                )?;
                let Some(target) = block_states.get_mut(&assignment.target) else {
                    return Err(BackendError::new(
                        "backend validation failure: validated native while assignment target was not declared",
                    ));
                };
                target.value = evaluated_value;

                if next_visible_states.contains_key(&assignment.target)
                    && !shadowed_locals.contains(&assignment.target)
                {
                    let Some(visible_target) = next_visible_states.get_mut(&assignment.target)
                    else {
                        return Err(BackendError::new(
                            "backend validation failure: validated native visible while assignment target was not declared",
                        ));
                    };
                    visible_target.value = evaluated_value;
                }
            }
            NativeSmokeStmt::EchoStringLiteral { .. } => {}
            NativeSmokeStmt::If(native_if) => {
                let updated_states = evaluate_native_scoped_if(native_if, &block_states)?;
                for name in sorted_native_local_names(&block_states) {
                    if let Some(updated_state) = updated_states.visible_states.get(&name) {
                        block_states.insert(name, updated_state.clone());
                    }
                }
                for name in sorted_native_local_names(&next_visible_states) {
                    if shadowed_locals.contains(&name) {
                        continue;
                    }
                    if let Some(updated_state) = updated_states.visible_states.get(&name) {
                        next_visible_states.insert(name, updated_state.clone());
                    }
                }
                if updated_states.control != NativeSmokeLoopControl::Fallthrough {
                    return Ok(NativeSmokeLoopEvaluation {
                        visible_states: next_visible_states,
                        control: updated_states.control,
                    });
                }
            }
            NativeSmokeStmt::While(_) => {
                return Err(BackendError::new(
                    "unsupported native while body statement for Stage 7b: nested while loops are future native work",
                ));
            }
            NativeSmokeStmt::Break => {
                return Ok(NativeSmokeLoopEvaluation {
                    visible_states: next_visible_states,
                    control: NativeSmokeLoopControl::Break,
                });
            }
            NativeSmokeStmt::Continue => {
                return Ok(NativeSmokeLoopEvaluation {
                    visible_states: next_visible_states,
                    control: NativeSmokeLoopControl::Continue,
                });
            }
        }
    }

    Ok(NativeSmokeLoopEvaluation {
        visible_states: next_visible_states,
        control: NativeSmokeLoopControl::Fallthrough,
    })
}

fn evaluate_native_scoped_if(
    native_if: &NativeSmokeIf,
    local_states: &HashMap<String, NativeSmokeLocalState>,
) -> Result<NativeSmokeLoopEvaluation, BackendError> {
    let values = native_state_values(local_states);
    let Some(condition_value) = evaluate_native_condition(&native_if.condition, &values) else {
        return Err(BackendError::new(
            "backend validation failure: validated native while if condition could not be re-evaluated",
        ));
    };

    if condition_value {
        evaluate_native_scoped_statements(&native_if.then_block.statements, local_states)
    } else if let Some(else_block) = &native_if.else_block {
        evaluate_native_scoped_statements(&else_block.statements, local_states)
    } else {
        Ok(NativeSmokeLoopEvaluation {
            visible_states: local_states.clone(),
            control: NativeSmokeLoopControl::Fallthrough,
        })
    }
}

fn stage_6c_loop_cap_error() -> BackendError {
    BackendError::new(
        "unsupported native while loop for Stage 7b: loop could not be proven to terminate within the current native smoke verification cap",
    )
}

fn validate_stage_6c_terminator(
    statement: &Stmt,
    local_states: &HashMap<String, NativeSmokeLocalState>,
    main_return: NativeSmokeMainReturn,
) -> Result<NativeSmokeTerminator, BackendError> {
    match statement {
        Stmt::Return { expr: None, .. } if main_return == NativeSmokeMainReturn::Void => {
            Ok(NativeSmokeTerminator::ExitSuccess)
        }
        Stmt::Return { expr: Some(expr), .. } => {
            let (expr, evaluated_exit_code) = validate_stage_6c_return_expr(expr, local_states)?;
            Ok(NativeSmokeTerminator::Return {
                expr,
                evaluated_exit_code,
            })
        }
        Stmt::Return { expr: None, .. } => Err(BackendError::new(
            "unsupported native terminal statement for Stage 7b: expected `return <portable integer expression>;`, found bare `return;`",
        )),
        Stmt::If(if_stmt) => {
            let condition = validate_stage_6c_condition(&if_stmt.condition, local_states)?;
            let then_block =
                validate_stage_6c_branch(&if_stmt.then_block.statements, local_states, main_return)?;

            let Some(else_branch) = &if_stmt.else_branch else {
                return Err(BackendError::new(
                    "unsupported native terminal if for Stage 7b: terminal if requires else; guard if without else is supported only when followed by a fallback return",
                ));
            };

            let else_block = match else_branch {
                ElseBranch::Block(else_block) => {
                    validate_stage_6c_branch(&else_block.statements, local_states, main_return)?
                }
                ElseBranch::If(else_if) => {
                    validate_stage_6c_if_as_block(else_if, local_states, main_return)?
                }
            };

            Ok(NativeSmokeTerminator::IfElse {
                condition: condition.condition,
                evaluated_condition: condition.value,
                then_block: Box::new(then_block),
                else_block: Box::new(else_block),
            })
        }
        other => Err(BackendError::new(format!(
            "unsupported native terminal statement for Stage 7b: expected final return or terminal if/else, found {}",
            describe_statement(other)
        ))),
    }
}

fn validate_stage_6c_branch(
    statements: &[Stmt],
    local_states: &HashMap<String, NativeSmokeLocalState>,
    main_return: NativeSmokeMainReturn,
) -> Result<NativeSmokeBlock, BackendError> {
    validate_stage_6c_block(statements, local_states, main_return).map_err(|error| {
        if should_preserve_native_block_error(&error.message) {
            error
        } else {
            BackendError::new(
                "unsupported native branch body shape for Stage 7b: expected supported local declarations, assignments, string-literal echo statements, bounded while statements, or fallthrough if statements followed by a supported native terminator",
            )
        }
    })
}

fn validate_stage_6c_if_as_block(
    if_stmt: &hir::IfStmt,
    local_states: &HashMap<String, NativeSmokeLocalState>,
    main_return: NativeSmokeMainReturn,
) -> Result<NativeSmokeBlock, BackendError> {
    let statement = Stmt::If(if_stmt.clone());
    let terminator = validate_stage_6c_terminator(&statement, local_states, main_return)?;
    Ok(NativeSmokeBlock {
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

fn validate_stage_6c_return_expr(
    expr: &Expr,
    local_states: &HashMap<String, NativeSmokeLocalState>,
) -> Result<(NativeSmokeExpr, i32), BackendError> {
    let return_expr = validate_stage_6c_int_expr(expr, local_states)?;
    let evaluated_exit_code = parse_stage_6c_exit_code(return_expr.value)?;
    Ok((return_expr.expr, evaluated_exit_code))
}

fn validate_stage_6c_int_expr(
    expr: &Expr,
    local_states: &HashMap<String, NativeSmokeLocalState>,
) -> Result<ValidatedNativeSmokeExpr, BackendError> {
    match expr {
        Expr::Int { value, .. } => {
            let value = parse_doria_int_literal(value)?;
            Ok(ValidatedNativeSmokeExpr {
                expr: NativeSmokeExpr::Int(value),
                value,
            })
        }
        Expr::Variable { name, .. } => {
            let value = local_states.get(name).map(|state| state.value).ok_or_else(|| {
                BackendError::new(
                    "unsupported native expression for Stage 7b: expected integer literal, supported integer local, or supported integer arithmetic",
                )
            })?;
            Ok(ValidatedNativeSmokeExpr {
                expr: NativeSmokeExpr::Local(name.clone()),
                value,
            })
        }
        Expr::Grouped { expr, .. } => validate_stage_6c_int_expr(expr, local_states),
        Expr::Binary {
            left, op, right, ..
        } if native_binary_op(op).is_some() => {
            let native_op = native_binary_op(op).expect("checked by guard");
            let left = validate_stage_6c_int_expr(left, local_states)?;
            let right = validate_stage_6c_int_expr(right, local_states)?;
            let value = checked_native_arithmetic(left.value, native_op, right.value).ok_or_else(|| {
                BackendError::new("integer arithmetic overflows the Doria `int` range")
            })?;
            Ok(ValidatedNativeSmokeExpr {
                expr: NativeSmokeExpr::Binary {
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
                "unsupported native arithmetic operator for Stage 7b",
            ))
        }
        other => Err(BackendError::new(format!(
            "unsupported native expression for Stage 7b: expected integer literal, supported integer local, or supported integer arithmetic, found `{}`",
            describe_expression(other)
        ))),
    }
}

fn validate_stage_6c_condition(
    expr: &Expr,
    local_states: &HashMap<String, NativeSmokeLocalState>,
) -> Result<ValidatedNativeSmokeCondition, BackendError> {
    match expr {
        Expr::Bool { value, .. } => Ok(ValidatedNativeSmokeCondition {
            condition: NativeSmokeCondition::Bool(*value),
            value: *value,
        }),
        Expr::Grouped { expr, .. } => validate_stage_6c_condition(expr, local_states),
        Expr::Unary {
            op: UnaryOp::Not,
            expr,
            ..
        } => {
            let condition = validate_stage_6c_condition(expr, local_states)?;
            Ok(ValidatedNativeSmokeCondition {
                condition: NativeSmokeCondition::Not(Box::new(condition.condition)),
                value: !condition.value,
            })
        }
        Expr::Binary {
            left, op, right, ..
        } if native_compare_op(op).is_some() => {
            let native_op = native_compare_op(op).expect("checked by guard");
            let left = validate_stage_6c_comparison_operand(left, local_states)?;
            let right = validate_stage_6c_comparison_operand(right, local_states)?;
            Ok(ValidatedNativeSmokeCondition {
                condition: NativeSmokeCondition::Compare {
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
            let left = validate_stage_6c_condition(left, local_states)?;
            let right = validate_stage_6c_condition(right, local_states)?;
            let value = left.value && right.value;
            Ok(ValidatedNativeSmokeCondition {
                condition: NativeSmokeCondition::And {
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
            let left = validate_stage_6c_condition(left, local_states)?;
            let right = validate_stage_6c_condition(right, local_states)?;
            let value = left.value || right.value;
            Ok(ValidatedNativeSmokeCondition {
                condition: NativeSmokeCondition::Or {
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
            let left = validate_stage_6c_condition(left, local_states)?;
            let right = validate_stage_6c_condition(right, local_states)?;
            let value = left.value ^ right.value;
            Ok(ValidatedNativeSmokeCondition {
                condition: NativeSmokeCondition::Xor {
                    left: Box::new(left.condition),
                    right: Box::new(right.condition),
                },
                value,
            })
        }

        _ => Err(BackendError::new(
            "unsupported native condition for Stage 7b: expected bool literal, supported integer comparison, or supported boolean condition",
        )),
    }
}

fn validate_stage_6c_comparison_operand(
    expr: &Expr,
    local_states: &HashMap<String, NativeSmokeLocalState>,
) -> Result<ValidatedNativeSmokeExpr, BackendError> {
    validate_stage_6c_int_expr(expr, local_states).map_err(|error| {
        if should_preserve_native_expression_error(&error.message) {
            error
        } else {
            BackendError::new(
                "unsupported native comparison for Stage 7b: expected supported integer expressions",
            )
        }
    })
}

fn should_preserve_native_expression_error(message: &str) -> bool {
    message.contains("unsupported native arithmetic operator")
        || message.contains("integer arithmetic overflows")
        || message.contains("integer literal is outside")
}

fn native_binary_op(op: &BinaryOp) -> Option<NativeSmokeBinaryOp> {
    match op {
        BinaryOp::Add => Some(NativeSmokeBinaryOp::Add),
        BinaryOp::Sub => Some(NativeSmokeBinaryOp::Subtract),
        BinaryOp::Mul => Some(NativeSmokeBinaryOp::Multiply),
        _ => None,
    }
}

fn native_compare_op(op: &BinaryOp) -> Option<NativeSmokeCompareOp> {
    match op {
        BinaryOp::Equal => Some(NativeSmokeCompareOp::Equal),
        BinaryOp::NotEqual => Some(NativeSmokeCompareOp::NotEqual),
        BinaryOp::Less => Some(NativeSmokeCompareOp::LessThan),
        BinaryOp::LessEqual => Some(NativeSmokeCompareOp::LessThanOrEqual),
        BinaryOp::Greater => Some(NativeSmokeCompareOp::GreaterThan),
        BinaryOp::GreaterEqual => Some(NativeSmokeCompareOp::GreaterThanOrEqual),
        _ => None,
    }
}

fn checked_native_arithmetic(left: i64, op: NativeSmokeBinaryOp, right: i64) -> Option<i64> {
    match op {
        NativeSmokeBinaryOp::Add => left.checked_add(right),
        NativeSmokeBinaryOp::Subtract => left.checked_sub(right),
        NativeSmokeBinaryOp::Multiply => left.checked_mul(right),
    }
}

fn evaluate_native_compare(left: i64, op: NativeSmokeCompareOp, right: i64) -> bool {
    match op {
        NativeSmokeCompareOp::Equal => left == right,
        NativeSmokeCompareOp::NotEqual => left != right,
        NativeSmokeCompareOp::LessThan => left < right,
        NativeSmokeCompareOp::LessThanOrEqual => left <= right,
        NativeSmokeCompareOp::GreaterThan => left > right,
        NativeSmokeCompareOp::GreaterThanOrEqual => left >= right,
    }
}

pub(crate) fn evaluate_exit_code(module: &NativeSmokeModule) -> i32 {
    evaluate_native_block_exit_code(&module.body)
}

fn evaluate_native_block_exit_code(block: &NativeSmokeBlock) -> i32 {
    evaluate_native_terminator_exit_code(&block.terminator)
}

fn evaluate_native_terminator_exit_code(terminator: &NativeSmokeTerminator) -> i32 {
    match terminator {
        NativeSmokeTerminator::ExitSuccess => 0,
        NativeSmokeTerminator::Return {
            evaluated_exit_code,
            ..
        } => *evaluated_exit_code,
        NativeSmokeTerminator::IfElse {
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
        NativeSmokeTerminator::Guard {
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

fn parse_stage_6c_exit_code(value: i64) -> Result<i32, BackendError> {
    if !(0..=125).contains(&value) {
        return Err(BackendError::new(
            "native Stage 7b exit code must be in the range 0..125",
        ));
    }

    Ok(value as i32)
}

pub(crate) fn lower_to_object(native_module: &NativeSmokeModule) -> Result<Vec<u8>, BackendError> {
    let isa_builder = cranelift_native::builder()
        .map_err(|error| BackendError::new(format!("backend emission failure: {error}")))?;
    let mut flag_builder = settings::builder();
    flag_builder
        .set("is_pic", "true")
        .map_err(|error| BackendError::new(format!("backend emission failure: {error}")))?;
    let isa = isa_builder
        .finish(settings::Flags::new(flag_builder))
        .map_err(|error| BackendError::new(format!("backend emission failure: {error}")))?;
    let mut module = ObjectModule::new(
        ObjectBuilder::new(isa, "doria_stage_7b", default_libcall_names())
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
        let mut resources = NativeSmokeLoweringResources::new(&mut module);
        lower_native_block(
            &mut builder,
            &native_module.body,
            &mut resources,
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
    block: &NativeSmokeBlock,
    resources: &mut NativeSmokeLoweringResources<'_>,
    lowered_local_values: &mut HashMap<String, Value>,
    evaluated_local_values: &mut HashMap<String, i64>,
) -> Result<(), BackendError> {
    for statement in &block.statements {
        lower_native_statement(
            builder,
            statement,
            resources,
            lowered_local_values,
            evaluated_local_values,
            None,
            None,
        )?;
    }

    lower_native_terminator(
        builder,
        &block.terminator,
        resources,
        lowered_local_values,
        evaluated_local_values,
    )
}

fn lower_native_terminator(
    builder: &mut FunctionBuilder,
    terminator: &NativeSmokeTerminator,
    resources: &mut NativeSmokeLoweringResources<'_>,
    lowered_local_values: &HashMap<String, Value>,
    evaluated_local_values: &HashMap<String, i64>,
) -> Result<(), BackendError> {
    match terminator {
        NativeSmokeTerminator::ExitSuccess => {
            lower_native_success_return(builder);
            Ok(())
        }
        NativeSmokeTerminator::Return {
            expr,
            evaluated_exit_code,
        } => lower_native_return(
            builder,
            expr,
            *evaluated_exit_code,
            lowered_local_values,
            evaluated_local_values,
        ),
        NativeSmokeTerminator::IfElse {
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
                resources,
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
                resources,
                &mut else_lowered_local_values,
                &mut else_evaluated_local_values,
            )
        }
        NativeSmokeTerminator::Guard {
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
                resources,
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
                resources,
                &mut fallback_lowered_local_values,
                &mut fallback_evaluated_local_values,
            )
        }
    }
}

fn lower_native_statement(
    builder: &mut FunctionBuilder,
    statement: &NativeSmokeStmt,
    resources: &mut NativeSmokeLoweringResources<'_>,
    lowered_local_values: &mut HashMap<String, Value>,
    evaluated_local_values: &mut HashMap<String, i64>,
    visible_local_values: Option<&HashMap<String, Value>>,
    loop_context: Option<&NativeSmokeLoopLoweringContext<'_>>,
) -> Result<NativeSmokeLoweringFlow, BackendError> {
    match statement {
        NativeSmokeStmt::Local(local) => {
            let value = lower_native_expr(builder, &local.expr, lowered_local_values)?;
            lowered_local_values.insert(local.name.clone(), value);
            evaluated_local_values.insert(local.name.clone(), local.evaluated_value);
            Ok(NativeSmokeLoweringFlow::Fallthrough)
        }
        NativeSmokeStmt::Assign(assignment) => {
            let value = lower_native_assignment(builder, assignment, lowered_local_values)?;
            lowered_local_values.insert(assignment.target.clone(), value);
            evaluated_local_values.insert(assignment.target.clone(), assignment.evaluated_value);
            Ok(NativeSmokeLoweringFlow::Fallthrough)
        }
        NativeSmokeStmt::EchoStringLiteral { value } => {
            lower_native_echo_string_literal(builder, value, resources)?;
            Ok(NativeSmokeLoweringFlow::Fallthrough)
        }
        NativeSmokeStmt::While(native_while) => lower_native_while(
            builder,
            native_while,
            resources,
            lowered_local_values,
            evaluated_local_values,
        )
        .map(|()| NativeSmokeLoweringFlow::Fallthrough),
        NativeSmokeStmt::If(native_if) => lower_native_fallthrough_if(
            builder,
            native_if,
            resources,
            lowered_local_values,
            evaluated_local_values,
            visible_local_values,
            loop_context,
        )
        .map(|visible_values| {
            if visible_values.is_some() {
                NativeSmokeLoweringFlow::Fallthrough
            } else {
                NativeSmokeLoweringFlow::Diverged
            }
        }),
        NativeSmokeStmt::Break => {
            let Some(loop_context) = loop_context else {
                return Err(BackendError::new(
                    "unsupported native break for Stage 7b: expected enclosing while loop",
                ));
            };
            jump_to_native_carried_block(
                builder,
                loop_context.after_block,
                loop_context.carried_locals,
                visible_local_values.unwrap_or(lowered_local_values),
            )?;
            Ok(NativeSmokeLoweringFlow::Diverged)
        }
        NativeSmokeStmt::Continue => {
            let Some(loop_context) = loop_context else {
                return Err(BackendError::new(
                    "unsupported native continue for Stage 7b: expected enclosing while loop",
                ));
            };
            jump_to_native_carried_block(
                builder,
                loop_context.header_block,
                loop_context.carried_locals,
                visible_local_values.unwrap_or(lowered_local_values),
            )?;
            Ok(NativeSmokeLoweringFlow::Diverged)
        }
    }
}

fn lower_native_fallthrough_if(
    builder: &mut FunctionBuilder,
    native_if: &NativeSmokeIf,
    resources: &mut NativeSmokeLoweringResources<'_>,
    lowered_local_values: &mut HashMap<String, Value>,
    evaluated_local_values: &mut HashMap<String, i64>,
    visible_local_values: Option<&HashMap<String, Value>>,
    loop_context: Option<&NativeSmokeLoopLoweringContext<'_>>,
) -> Result<Option<HashMap<String, Value>>, BackendError> {
    let _validated_condition = native_if.evaluated_condition;
    let then_ir_block = builder.create_block();
    let else_ir_block = builder.create_block();
    let base_visible_local_values =
        scoped_native_visible_values(lowered_local_values, visible_local_values);

    lower_native_condition_branch(
        builder,
        &native_if.condition,
        then_ir_block,
        else_ir_block,
        lowered_local_values,
    )?;

    builder.switch_to_block(then_ir_block);
    builder.seal_block(then_ir_block);
    let mut then_lowered_local_values = lowered_local_values.clone();
    let mut then_evaluated_local_values = evaluated_local_values.clone();
    let then_visible_local_values = lower_native_fallthrough_block(
        builder,
        &native_if.then_block,
        resources,
        &mut then_lowered_local_values,
        &mut then_evaluated_local_values,
        &base_visible_local_values,
        loop_context,
    )?;
    let mut merge_ir_block = None;
    if let Some(then_visible_local_values) = &then_visible_local_values {
        let merge_block = ensure_native_merge_block(builder, &mut merge_ir_block, native_if);
        jump_to_native_merge(
            builder,
            merge_block,
            &native_if.merged_values,
            then_visible_local_values,
        )?;
    }

    builder.switch_to_block(else_ir_block);
    builder.seal_block(else_ir_block);
    let mut else_lowered_local_values = lowered_local_values.clone();
    let mut else_evaluated_local_values = evaluated_local_values.clone();
    let else_visible_local_values = if let Some(else_block) = &native_if.else_block {
        lower_native_fallthrough_block(
            builder,
            else_block,
            resources,
            &mut else_lowered_local_values,
            &mut else_evaluated_local_values,
            &base_visible_local_values,
            loop_context,
        )?
    } else {
        Some(base_visible_local_values.clone())
    };
    if let Some(else_visible_local_values) = &else_visible_local_values {
        let merge_block = ensure_native_merge_block(builder, &mut merge_ir_block, native_if);
        jump_to_native_merge(
            builder,
            merge_block,
            &native_if.merged_values,
            else_visible_local_values,
        )?;
    }

    let Some(merge_ir_block) = merge_ir_block else {
        return Ok(None);
    };

    builder.switch_to_block(merge_ir_block);
    builder.seal_block(merge_ir_block);
    for (index, (name, value)) in native_if.merged_values.iter().enumerate() {
        lowered_local_values.insert(name.clone(), builder.block_params(merge_ir_block)[index]);
        evaluated_local_values.insert(name.clone(), *value);
    }

    Ok(Some(lowered_local_values.clone()))
}

fn ensure_native_merge_block(
    builder: &mut FunctionBuilder,
    merge_ir_block: &mut Option<Block>,
    native_if: &NativeSmokeIf,
) -> Block {
    if let Some(block) = *merge_ir_block {
        return block;
    }

    let block = builder.create_block();
    for _ in &native_if.merged_values {
        builder.append_block_param(block, types::I64);
    }
    *merge_ir_block = Some(block);
    block
}

fn lower_native_fallthrough_block(
    builder: &mut FunctionBuilder,
    block: &NativeSmokeFallthroughBlock,
    resources: &mut NativeSmokeLoweringResources<'_>,
    lowered_local_values: &mut HashMap<String, Value>,
    evaluated_local_values: &mut HashMap<String, i64>,
    visible_local_values: &HashMap<String, Value>,
    loop_context: Option<&NativeSmokeLoopLoweringContext<'_>>,
) -> Result<Option<HashMap<String, Value>>, BackendError> {
    let mut visible_lowered_local_values =
        scoped_native_visible_values(lowered_local_values, Some(visible_local_values));
    let mut shadowed_locals = HashSet::new();

    for statement in &block.statements {
        match statement {
            NativeSmokeStmt::Local(local) => {
                lower_native_statement(
                    builder,
                    statement,
                    resources,
                    lowered_local_values,
                    evaluated_local_values,
                    Some(&visible_lowered_local_values),
                    loop_context,
                )?;
                if visible_lowered_local_values.contains_key(&local.name) {
                    shadowed_locals.insert(local.name.clone());
                }
            }
            NativeSmokeStmt::Assign(assignment) => {
                lower_native_statement(
                    builder,
                    statement,
                    resources,
                    lowered_local_values,
                    evaluated_local_values,
                    Some(&visible_lowered_local_values),
                    loop_context,
                )?;
                update_visible_lowered_value(
                    &mut visible_lowered_local_values,
                    &shadowed_locals,
                    &assignment.target,
                    lowered_local_values,
                )?;
            }
            NativeSmokeStmt::While(native_while) => {
                lower_native_statement(
                    builder,
                    statement,
                    resources,
                    lowered_local_values,
                    evaluated_local_values,
                    Some(&visible_lowered_local_values),
                    loop_context,
                )?;
                for (name, _) in &native_while.final_values {
                    update_visible_lowered_value(
                        &mut visible_lowered_local_values,
                        &shadowed_locals,
                        name,
                        lowered_local_values,
                    )?;
                }
            }
            NativeSmokeStmt::EchoStringLiteral { .. } => {
                lower_native_statement(
                    builder,
                    statement,
                    resources,
                    lowered_local_values,
                    evaluated_local_values,
                    Some(&visible_lowered_local_values),
                    loop_context,
                )?;
            }
            NativeSmokeStmt::If(native_if) => {
                if lower_native_statement(
                    builder,
                    statement,
                    resources,
                    lowered_local_values,
                    evaluated_local_values,
                    Some(&visible_lowered_local_values),
                    loop_context,
                )? == NativeSmokeLoweringFlow::Diverged
                {
                    return Ok(None);
                }
                for (name, _) in &native_if.merged_values {
                    update_visible_lowered_value(
                        &mut visible_lowered_local_values,
                        &shadowed_locals,
                        name,
                        lowered_local_values,
                    )?;
                }
            }
            NativeSmokeStmt::Break | NativeSmokeStmt::Continue => {
                lower_native_statement(
                    builder,
                    statement,
                    resources,
                    lowered_local_values,
                    evaluated_local_values,
                    Some(&visible_lowered_local_values),
                    loop_context,
                )?;
                return Ok(None);
            }
        }
    }

    Ok(Some(visible_lowered_local_values))
}

fn update_visible_lowered_value(
    visible_lowered_local_values: &mut HashMap<String, Value>,
    shadowed_locals: &HashSet<String>,
    name: &str,
    lowered_local_values: &HashMap<String, Value>,
) -> Result<(), BackendError> {
    if !visible_lowered_local_values.contains_key(name) || shadowed_locals.contains(name) {
        return Ok(());
    }

    let Some(value) = lowered_local_values.get(name).copied() else {
        return Err(BackendError::new(format!(
            "backend emission failure: validated native visible local `{name}` was not lowered",
        )));
    };
    visible_lowered_local_values.insert(name.to_string(), value);
    Ok(())
}

fn scoped_native_visible_values(
    scoped_local_values: &HashMap<String, Value>,
    visible_local_values: Option<&HashMap<String, Value>>,
) -> HashMap<String, Value> {
    let mut values = scoped_local_values.clone();
    if let Some(visible_local_values) = visible_local_values {
        values.extend(
            visible_local_values
                .iter()
                .map(|(name, value)| (name.clone(), *value)),
        );
    }
    values
}

fn jump_to_native_merge(
    builder: &mut FunctionBuilder,
    merge_block: Block,
    merged_values: &[(String, i64)],
    local_values: &HashMap<String, Value>,
) -> Result<(), BackendError> {
    let args = merged_values
        .iter()
        .map(|(name, _)| {
            local_values
                .get(name)
                .copied()
                .map(BlockArg::Value)
                .ok_or_else(|| {
                    BackendError::new(format!(
                        "backend emission failure: validated native merged local `{name}` was not lowered"
                    ))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    builder.ins().jump(merge_block, &args);
    Ok(())
}

fn jump_to_native_carried_block(
    builder: &mut FunctionBuilder,
    target_block: Block,
    carried_values: &[(String, i64)],
    local_values: &HashMap<String, Value>,
) -> Result<(), BackendError> {
    let args = carried_values
        .iter()
        .map(|(name, _)| {
            local_values
                .get(name)
                .copied()
                .map(BlockArg::Value)
                .ok_or_else(|| {
                    BackendError::new(format!(
                        "backend emission failure: validated native loop-carried local `{name}` was not lowered"
                    ))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    builder.ins().jump(target_block, &args);
    Ok(())
}

fn lower_native_while(
    builder: &mut FunctionBuilder,
    native_while: &NativeSmokeWhile,
    resources: &mut NativeSmokeLoweringResources<'_>,
    lowered_local_values: &mut HashMap<String, Value>,
    evaluated_local_values: &mut HashMap<String, i64>,
) -> Result<(), BackendError> {
    debug_assert!(native_while.evaluated_iterations <= STAGE_7B_LOOP_VERIFICATION_CAP);

    let loop_header = builder.create_block();
    let loop_body = builder.create_block();
    let loop_after = builder.create_block();

    for _ in &native_while.final_values {
        builder.append_block_param(loop_header, types::I64);
        builder.append_block_param(loop_after, types::I64);
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
    let after_args = native_while
        .final_values
        .iter()
        .map(|(name, _)| {
            header_local_values
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
    lower_native_condition_branch_with_args(
        builder,
        &native_while.condition,
        loop_body,
        &[],
        loop_after,
        &after_args,
        &header_local_values,
    )?;

    builder.switch_to_block(loop_body);
    builder.seal_block(loop_body);
    let mut body_local_values = header_local_values.clone();
    let mut body_evaluated_values = evaluated_local_values.clone();
    let loop_context = NativeSmokeLoopLoweringContext {
        header_block: loop_header,
        after_block: loop_after,
        carried_locals: &native_while.final_values,
    };
    let visible_body_values = lower_native_fallthrough_block(
        builder,
        &native_while.body,
        resources,
        &mut body_local_values,
        &mut body_evaluated_values,
        &header_local_values,
        Some(&loop_context),
    )?;
    if let Some(visible_body_values) = visible_body_values {
        let next_args = native_while
            .final_values
            .iter()
            .map(|(name, _)| {
                visible_body_values
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
    }
    builder.seal_block(loop_header);

    builder.switch_to_block(loop_after);
    builder.seal_block(loop_after);
    for (index, (name, value)) in native_while.final_values.iter().enumerate() {
        lowered_local_values.insert(name.clone(), builder.block_params(loop_after)[index]);
        evaluated_local_values.insert(name.clone(), *value);
    }

    Ok(())
}

fn lower_native_assignment(
    builder: &mut FunctionBuilder,
    assignment: &NativeSmokeAssign,
    local_values: &HashMap<String, Value>,
) -> Result<Value, BackendError> {
    let right = lower_native_expr(builder, &assignment.expr, local_values)?;
    match assignment.op {
        NativeSmokeAssignOp::Assign => Ok(right),
        NativeSmokeAssignOp::AddAssign | NativeSmokeAssignOp::SubAssign => {
            let left = local_values.get(&assignment.target).copied().ok_or_else(|| {
                BackendError::new(format!(
                    "backend emission failure: validated native assignment target `{}` was not lowered",
                    assignment.target
                ))
            })?;
            Ok(match assignment.op {
                NativeSmokeAssignOp::Assign => unreachable!("handled above"),
                NativeSmokeAssignOp::AddAssign => builder.ins().iadd(left, right),
                NativeSmokeAssignOp::SubAssign => builder.ins().isub(left, right),
            })
        }
    }
}

fn lower_native_success_return(builder: &mut FunctionBuilder) {
    let exit_value = builder.ins().iconst(types::I32, 0);
    builder.ins().return_(&[exit_value]);
}

fn lower_native_echo_string_literal(
    builder: &mut FunctionBuilder,
    value: &str,
    resources: &mut NativeSmokeLoweringResources<'_>,
) -> Result<(), BackendError> {
    if value.is_empty() {
        return Ok(());
    }

    let data_pointer = define_native_echo_string_literal(builder, value, resources)?;

    match host_native_smoke_stdout_platform() {
        NativeSmokeStdoutPlatform::Unix => {
            lower_native_unix_echo_string_literal(builder, value, data_pointer, resources)
        }
        NativeSmokeStdoutPlatform::Windows => {
            lower_native_windows_echo_string_literal(builder, value, data_pointer, resources)
        }
        NativeSmokeStdoutPlatform::Unsupported => Err(BackendError::new(
            "unsupported native echo statement for Stage 7b: string-literal stdout smoke path is currently available only on Unix-like and Windows targets",
        )),
    }
}

fn define_native_echo_string_literal(
    builder: &mut FunctionBuilder,
    value: &str,
    resources: &mut NativeSmokeLoweringResources<'_>,
) -> Result<Value, BackendError> {
    let data_name = format!("__doria_stage_7b_echo_{}", resources.next_string_literal_id);
    resources.next_string_literal_id += 1;

    let data_id = resources
        .module
        .declare_data(&data_name, Linkage::Local, false, false)
        .map_err(|error| BackendError::new(format!("backend emission failure: {error}")))?;
    let mut data_description = DataDescription::new();
    data_description.define(value.as_bytes().to_vec().into_boxed_slice());
    resources
        .module
        .define_data(data_id, &data_description)
        .map_err(|error| BackendError::new(format!("backend emission failure: {error}")))?;

    let pointer_type = resources.module.target_config().pointer_type();
    let global_value = resources.module.declare_data_in_func(data_id, builder.func);
    Ok(builder.ins().global_value(pointer_type, global_value))
}

fn lower_native_unix_echo_string_literal(
    builder: &mut FunctionBuilder,
    value: &str,
    data_pointer: Value,
    resources: &mut NativeSmokeLoweringResources<'_>,
) -> Result<(), BackendError> {
    // Stage 7b's stdout smoke path is intentionally narrow: string literals go
    // straight to stdout, with no native string/runtime model implied.
    let write_function_id = resources.declare_write_function()?;
    let write_function = resources
        .module
        .declare_func_in_func(write_function_id, builder.func);
    let pointer_type = resources.module.target_config().pointer_type();
    let fd = builder.ins().iconst(types::I32, 1);
    let byte_count = builder.ins().iconst(pointer_type, value.len() as i64);
    builder
        .ins()
        .call(write_function, &[fd, data_pointer, byte_count]);

    Ok(())
}

fn lower_native_windows_echo_string_literal(
    builder: &mut FunctionBuilder,
    value: &str,
    data_pointer: Value,
    resources: &mut NativeSmokeLoweringResources<'_>,
) -> Result<(), BackendError> {
    // Stage 7b's stdout smoke path uses Kernel32 directly on Windows so it can
    // work with Doria's generated `main` entrypoint without CRT startup.
    let pointer_type = resources.module.target_config().pointer_type();
    let get_std_handle_id = resources.declare_get_std_handle_function()?;
    let get_std_handle = resources
        .module
        .declare_func_in_func(get_std_handle_id, builder.func);
    let std_output_handle = builder.ins().iconst(types::I32, -11);
    let handle_call = builder.ins().call(get_std_handle, &[std_output_handle]);
    let handle = builder.inst_results(handle_call)[0];

    let written_slot =
        builder.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 4, 2));
    let zero = builder.ins().iconst(types::I32, 0);
    builder.ins().stack_store(zero, written_slot, 0);
    let written_pointer = builder.ins().stack_addr(pointer_type, written_slot, 0);
    let overlapped_pointer = builder.ins().iconst(pointer_type, 0);

    let write_file_id = resources.declare_write_file_function()?;
    let write_file = resources
        .module
        .declare_func_in_func(write_file_id, builder.func);
    let byte_count = builder.ins().iconst(types::I32, value.len() as i64);
    builder.ins().call(
        write_file,
        &[
            handle,
            data_pointer,
            byte_count,
            written_pointer,
            overlapped_pointer,
        ],
    );

    Ok(())
}

fn lower_native_return(
    builder: &mut FunctionBuilder,
    expr: &NativeSmokeExpr,
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
    expr: &NativeSmokeExpr,
    local_values: &HashMap<String, Value>,
) -> Result<Value, BackendError> {
    match expr {
        NativeSmokeExpr::Int(value) => Ok(builder.ins().iconst(types::I64, *value)),
        NativeSmokeExpr::Local(name) => local_values.get(name).copied().ok_or_else(|| {
            BackendError::new(format!(
                "backend emission failure: validated native local `{name}` was not lowered"
            ))
        }),
        NativeSmokeExpr::Binary { op, left, right } => {
            let left = lower_native_expr(builder, left, local_values)?;
            let right = lower_native_expr(builder, right, local_values)?;
            Ok(match op {
                NativeSmokeBinaryOp::Add => builder.ins().iadd(left, right),
                NativeSmokeBinaryOp::Subtract => builder.ins().isub(left, right),
                NativeSmokeBinaryOp::Multiply => builder.ins().imul(left, right),
            })
        }
    }
}

fn lower_native_condition_branch(
    builder: &mut FunctionBuilder,
    condition: &NativeSmokeCondition,
    then_block: Block,
    else_block: Block,
    local_values: &HashMap<String, Value>,
) -> Result<(), BackendError> {
    lower_native_condition_branch_with_args(
        builder,
        condition,
        then_block,
        &[],
        else_block,
        &[],
        local_values,
    )
}

fn lower_native_condition_branch_with_args(
    builder: &mut FunctionBuilder,
    condition: &NativeSmokeCondition,
    then_block: Block,
    then_args: &[BlockArg],
    else_block: Block,
    else_args: &[BlockArg],
    local_values: &HashMap<String, Value>,
) -> Result<(), BackendError> {
    match condition {
        NativeSmokeCondition::Bool(value) => {
            let value = builder.ins().iconst(types::I8, i64::from(*value));
            builder
                .ins()
                .brif(value, then_block, then_args, else_block, else_args);
            Ok(())
        }
        NativeSmokeCondition::Compare { op, left, right } => {
            let left = lower_native_expr(builder, left, local_values)?;
            let right = lower_native_expr(builder, right, local_values)?;
            let condition = builder.ins().icmp(native_compare_intcc(*op), left, right);
            builder
                .ins()
                .brif(condition, then_block, then_args, else_block, else_args);
            Ok(())
        }
        NativeSmokeCondition::Not(condition) => lower_native_condition_branch_with_args(
            builder,
            condition,
            else_block,
            else_args,
            then_block,
            then_args,
            local_values,
        ),
        NativeSmokeCondition::And { left, right } => {
            let right_block = builder.create_block();
            lower_native_condition_branch_with_args(
                builder,
                left,
                right_block,
                &[],
                else_block,
                else_args,
                local_values,
            )?;

            builder.switch_to_block(right_block);
            builder.seal_block(right_block);
            lower_native_condition_branch_with_args(
                builder,
                right,
                then_block,
                then_args,
                else_block,
                else_args,
                local_values,
            )
        }
        NativeSmokeCondition::Or { left, right } => {
            let right_block = builder.create_block();
            lower_native_condition_branch_with_args(
                builder,
                left,
                then_block,
                then_args,
                right_block,
                &[],
                local_values,
            )?;

            builder.switch_to_block(right_block);
            builder.seal_block(right_block);
            lower_native_condition_branch_with_args(
                builder,
                right,
                then_block,
                then_args,
                else_block,
                else_args,
                local_values,
            )
        }
        NativeSmokeCondition::Xor { left, right } => {
            let left = lower_native_condition_value(builder, left, local_values)?;
            let right = lower_native_condition_value(builder, right, local_values)?;
            let condition = builder.ins().icmp(IntCC::NotEqual, left, right);
            builder
                .ins()
                .brif(condition, then_block, then_args, else_block, else_args);
            Ok(())
        }
    }
}

fn lower_native_condition_value(
    builder: &mut FunctionBuilder,
    condition: &NativeSmokeCondition,
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

fn native_compare_intcc(op: NativeSmokeCompareOp) -> IntCC {
    match op {
        NativeSmokeCompareOp::Equal => IntCC::Equal,
        NativeSmokeCompareOp::NotEqual => IntCC::NotEqual,
        NativeSmokeCompareOp::LessThan => IntCC::SignedLessThan,
        NativeSmokeCompareOp::LessThanOrEqual => IntCC::SignedLessThanOrEqual,
        NativeSmokeCompareOp::GreaterThan => IntCC::SignedGreaterThan,
        NativeSmokeCompareOp::GreaterThanOrEqual => IntCC::SignedGreaterThanOrEqual,
    }
}

fn evaluate_native_expr(
    expr: &NativeSmokeExpr,
    local_values: &HashMap<String, i64>,
) -> Option<i64> {
    match expr {
        NativeSmokeExpr::Int(value) => Some(*value),
        NativeSmokeExpr::Local(name) => local_values.get(name).copied(),
        NativeSmokeExpr::Binary { op, left, right } => checked_native_arithmetic(
            evaluate_native_expr(left, local_values)?,
            *op,
            evaluate_native_expr(right, local_values)?,
        ),
    }
}

fn evaluate_native_condition(
    condition: &NativeSmokeCondition,
    local_values: &HashMap<String, i64>,
) -> Option<bool> {
    match condition {
        NativeSmokeCondition::Bool(value) => Some(*value),
        NativeSmokeCondition::Compare { op, left, right } => Some(evaluate_native_compare(
            evaluate_native_expr(left, local_values)?,
            *op,
            evaluate_native_expr(right, local_values)?,
        )),
        NativeSmokeCondition::Not(condition) => {
            Some(!evaluate_native_condition(condition, local_values)?)
        }
        NativeSmokeCondition::And { left, right } => {
            if !evaluate_native_condition(left, local_values)? {
                Some(false)
            } else {
                evaluate_native_condition(right, local_values)
            }
        }
        NativeSmokeCondition::Or { left, right } => {
            if evaluate_native_condition(left, local_values)? {
                Some(true)
            } else {
                evaluate_native_condition(right, local_values)
            }
        }
        NativeSmokeCondition::Xor { left, right } => Some(
            evaluate_native_condition(left, local_values)?
                ^ evaluate_native_condition(right, local_values)?,
        ),
    }
}

fn describe_statement(statement: &Stmt) -> &'static str {
    match statement {
        Stmt::VarDecl(_) => "local variable declaration",
        Stmt::Assignment(_) => "assignment",
        Stmt::Echo { .. } => "echo statement",
        Stmt::Return { .. } => "return statement",
        Stmt::If(_) => "if statement",
        Stmt::While(_) => "while statement",
        Stmt::Break { .. } => "break statement",
        Stmt::Continue { .. } => "continue statement",
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

    fn lower_test_source(source: &str) -> hir::Program {
        crate::lower_source("test.doria", source).unwrap_or_else(|diagnostics| {
            panic!("expected source to lower cleanly, got diagnostics: {diagnostics:#?}")
        })
    }

    fn validate_test_source(source: &str) -> NativeSmokeModule {
        let program = lower_test_source(source);
        validate(&program).expect("expected native smoke validation to pass")
    }

    #[test]
    fn stdout_platform_selection_supports_unix_and_windows() {
        assert_eq!(
            native_smoke_stdout_platform(false, true),
            NativeSmokeStdoutPlatform::Unix
        );
        assert_eq!(
            native_smoke_stdout_platform(true, false),
            NativeSmokeStdoutPlatform::Windows
        );
        assert_eq!(
            native_smoke_stdout_platform(false, false),
            NativeSmokeStdoutPlatform::Unsupported
        );
    }

    #[test]
    fn validation_accepts_string_literal_echo_without_platform_gate() {
        let module = validate_test_source(
            r#"
function main(): void
{
    echo "Hello Doria!";
}
"#,
        );

        assert!(matches!(
            module.body.statements.as_slice(),
            [NativeSmokeStmt::EchoStringLiteral { value }] if value == "Hello Doria!"
        ));
        assert_eq!(module.body.terminator, NativeSmokeTerminator::ExitSuccess);
    }

    #[test]
    fn validation_accepts_void_guard_if_with_implicit_success_fallback() {
        let module = validate_test_source(
            r#"
function main(): void
{
    if (true) {
        return;
    }
}
"#,
        );

        assert!(matches!(
            module.body.terminator,
            NativeSmokeTerminator::Guard { .. }
        ));
        assert_eq!(evaluate_exit_code(&module), 0);
    }

    #[test]
    fn validation_produces_native_smoke_module_for_structured_while() {
        let module = validate_test_source(
            r#"
function main(): int
{
    let writable $count = 0;

    while ($count < 42) {
        $count += 1;
    }

    return $count;
}
"#,
        );

        assert!(matches!(
            module.body.statements.as_slice(),
            [
                NativeSmokeStmt::Local(_),
                NativeSmokeStmt::While(NativeSmokeWhile { .. })
            ]
        ));
        assert!(matches!(
            module.body.terminator,
            NativeSmokeTerminator::Return { .. }
        ));
    }

    #[test]
    fn evaluation_returns_exit_code_for_structured_while() {
        let module = validate_test_source(
            r#"
function main(): int
{
    let writable $count = 0;

    while ($count < 42) {
        $count += 1;
    }

    return $count;
}
"#,
        );

        assert_eq!(evaluate_exit_code(&module), 42);
    }

    #[test]
    fn evaluation_preserves_outer_local_when_loop_body_shadows_it() {
        let module = validate_test_source(
            r#"
function main(): int
{
    let writable $code = 0;
    let writable $count = 0;

    while ($count < 1) {
        let $code = 42;
        $count += 1;
    }

    return $code;
}
"#,
        );

        assert_eq!(evaluate_exit_code(&module), 0);
    }

    #[test]
    fn evaluation_preserves_pre_shadow_assignment_in_loop_body() {
        let module = validate_test_source(
            r#"
function main(): int
{
    let writable $code = 1;
    let writable $count = 0;

    while ($count < 1) {
        $code = 2;
        let $code = 42;
        $count += 1;
    }

    return $code;
}
"#,
        );

        assert_eq!(evaluate_exit_code(&module), 2);
    }

    #[test]
    fn loop_cap_failure_diagnostic_is_preserved() {
        let program = lower_test_source(
            r#"
function main(): int
{
    let writable $count = 0;

    while ($count < 1) {
        $count = $count;
    }

    return $count;
}
"#,
        );

        let error = validate(&program).expect_err("expected loop proof to fail at the cap");
        assert!(
            error.message.contains(
                "unsupported native while loop for Stage 7b: loop could not be proven to terminate within the current native smoke verification cap"
            ),
            "unexpected error: {}",
            error.message
        );
    }
}

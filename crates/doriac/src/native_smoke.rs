use std::collections::{HashMap, HashSet};

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{
    types, AbiParam, Block, BlockArg, InstBuilder, Signature, StackSlotData, StackSlotKind,
    TrapCode, Value,
};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::DataDescription;
use cranelift_module::{default_libcall_names, FuncId, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};

use crate::backend::BackendError;
use crate::hir::{
    self, AssignOp, BinaryOp, ElseBranch, Expr, ForIncrement, ForInitializer, IncrementOp, Item,
    Stmt, UnaryOp,
};

const STAGE_7B_LOOP_VERIFICATION_CAP: u64 = 10_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeSmokeModule {
    functions: Vec<NativeSmokeFunction>,
    main_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeSmokeFunction {
    name: String,
    params: Vec<NativeSmokeParam>,
    return_type: NativeSmokeFunctionReturn,
    body: NativeSmokeBlock,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeSmokeParam {
    name: String,
    writable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeSmokeFunctionReturn {
    Int,
    Void,
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
    Echo(NativeSmokeExpr),
    Call(NativeSmokeCall),
    While(NativeSmokeWhile),
    For(Box<NativeSmokeFor>),
    If(NativeSmokeIf),
    Break,
    Continue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeSmokeLocal {
    name: String,
    writable: bool,
    expr: NativeSmokeExpr,
    evaluated_value: NativeSmokeValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeSmokeAssign {
    target: String,
    op: NativeSmokeAssignOp,
    expr: NativeSmokeExpr,
    evaluated_value: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeSmokeCall {
    function: String,
    args: Vec<NativeSmokeExpr>,
    return_type: NativeSmokeFunctionReturn,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeSmokeWhile {
    condition: NativeSmokeCondition,
    body: NativeSmokeFallthroughBlock,
    final_values: Vec<(String, NativeSmokeValue)>,
    evaluated_iterations: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeSmokeFor {
    initializer: Option<NativeSmokeForInitializer>,
    condition: NativeSmokeCondition,
    increment: Option<NativeSmokeAssign>,
    increment_exit_condition: Option<NativeSmokeCondition>,
    body: NativeSmokeFallthroughBlock,
    carried_values: Vec<(String, NativeSmokeValue)>,
    final_values: Vec<(String, NativeSmokeValue)>,
    evaluated_iterations: u64,
}

struct NativeSmokeForLikeParts {
    initializer: Option<NativeSmokeForInitializer>,
    condition: NativeSmokeCondition,
    increment: Option<NativeSmokeAssign>,
    increment_exit_condition: Option<NativeSmokeCondition>,
    body: NativeSmokeFallthroughBlock,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum NativeSmokeForInitializer {
    Local(NativeSmokeLocal),
    Assign(NativeSmokeAssign),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeSmokeIf {
    condition: NativeSmokeCondition,
    evaluated_condition: bool,
    then_block: NativeSmokeFallthroughBlock,
    else_block: Option<NativeSmokeFallthroughBlock>,
    merged_values: Vec<(String, NativeSmokeValue)>,
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
    continue_block: Block,
    after_block: Block,
    carried_locals: &'a [(String, NativeSmokeValue)],
}

#[derive(Debug, Clone, Copy)]
struct NativeSmokeBranchTarget<'a> {
    block: Block,
    args: &'a [BlockArg],
}

struct NativeSmokeLoweringResources<'module> {
    module: &'module mut ObjectModule,
    write_func_id: Option<FuncId>,
    get_std_handle_func_id: Option<FuncId>,
    write_file_func_id: Option<FuncId>,
    next_string_literal_id: usize,
    string_literal_namespace: String,
    function_ids: HashMap<String, FuncId>,
}

impl<'module> NativeSmokeLoweringResources<'module> {
    fn new(
        module: &'module mut ObjectModule,
        function_ids: HashMap<String, FuncId>,
        string_literal_namespace: String,
    ) -> Self {
        Self {
            module,
            write_func_id: None,
            get_std_handle_func_id: None,
            write_file_func_id: None,
            next_string_literal_id: 0,
            string_literal_namespace,
            function_ids,
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
    StringLiteral(String),
    Binary {
        op: NativeSmokeBinaryOp,
        left: Box<NativeSmokeExpr>,
        right: Box<NativeSmokeExpr>,
    },
    Call {
        function: String,
        args: Vec<NativeSmokeExpr>,
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
        evaluated_value: i64,
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
    value: NativeSmokeValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum NativeSmokeValue {
    Int(i64),
    StringLiteral(String),
}

impl NativeSmokeValue {
    fn as_int(&self) -> Option<i64> {
        match self {
            Self::Int(value) => Some(*value),
            Self::StringLiteral(_) => None,
        }
    }

    fn as_string_literal(&self) -> Option<&str> {
        match self {
            Self::StringLiteral(value) => Some(value),
            Self::Int(_) => None,
        }
    }
}

pub(crate) fn validate(program: &hir::Program) -> Result<NativeSmokeModule, BackendError> {
    validate_stage_10(program)
}

#[derive(Debug, Clone)]
struct NativeSmokeFunctionSignature {
    params: Vec<NativeSmokeParam>,
    return_type: NativeSmokeFunctionReturn,
}

struct NativeSmokeValidationContext<'program> {
    declarations: HashMap<String, &'program hir::FunctionDecl>,
    signatures: HashMap<String, NativeSmokeFunctionSignature>,
    function_order: Vec<String>,
    validated: HashMap<String, NativeSmokeFunction>,
    validated_call_sites: HashSet<(String, Vec<i64>)>,
    validating: Vec<String>,
}

fn validate_stage_10(program: &hir::Program) -> Result<NativeSmokeModule, BackendError> {
    let mut context = NativeSmokeValidationContext {
        declarations: HashMap::new(),
        signatures: HashMap::new(),
        function_order: Vec::new(),
        validated: HashMap::new(),
        validated_call_sites: HashSet::new(),
        validating: Vec::new(),
    };
    let mut main_count = 0;

    for item in &program.items {
        match item {
            Item::Function(function) => {
                if function.name == "main" {
                    main_count += 1;
                }
                if context.declarations.contains_key(&function.name) {
                    return Err(BackendError::new(format!(
                        "function `{}` is already declared",
                        function.name
                    )));
                }
                let signature = validate_native_function_signature(function)?;
                context.function_order.push(function.name.clone());
                context.signatures.insert(function.name.clone(), signature);
                context.declarations.insert(function.name.clone(), function);
            }
            Item::Class(class_decl) => {
                return Err(BackendError::new(format!(
                    "unsupported top-level item for native Stage 10: class `{}`",
                    class_decl.name
                )));
            }
            Item::Statement(statement) => {
                return Err(BackendError::new(format!(
                    "unsupported top-level item for native Stage 10: {}",
                    describe_statement(statement)
                )));
            }
        }
    }

    if main_count != 1 {
        return Err(match main_count {
            0 => BackendError::new(
                "no native entrypoint found; native Stage 10 output requires exactly one top-level `function main(): int` or `function main(): void`",
            ),
            _ => BackendError::new(
                "multiple native entrypoints found; native Stage 10 output requires exactly one top-level `function main(): int` or `function main(): void`",
            ),
        });
    }

    let Some(main) = context.declarations.get("main") else {
        return Err(BackendError::new(
            "no native entrypoint found; native Stage 10 output requires exactly one top-level `function main(): int` or `function main(): void`",
        ));
    };
    if !main.params.is_empty() {
        return Err(BackendError::new(
            "wrong main signature for native Stage 10: `main` must not declare parameters",
        ));
    }

    validate_stage_10_function("main", &[], &mut context)?;
    for name in context.function_order.clone() {
        if name == "main" {
            continue;
        }
        let is_parameterless = context
            .signatures
            .get(&name)
            .map(|signature| signature.params.is_empty())
            .ok_or_else(|| {
                BackendError::new(format!(
                    "backend validation failure: native function `{name}` signature was not collected"
                ))
            })?;
        if is_parameterless {
            validate_stage_10_function(&name, &[], &mut context)?;
        }
    }

    let functions = context
        .function_order
        .iter()
        .filter_map(|name| context.validated.get(name).cloned())
        .collect::<Vec<_>>();
    let module = NativeSmokeModule {
        functions,
        main_name: "main".to_string(),
    };
    validate_native_process_exit_boundary(&module)?;
    Ok(module)
}

fn validate_native_function_signature(
    function: &hir::FunctionDecl,
) -> Result<NativeSmokeFunctionSignature, BackendError> {
    let return_type = match function.return_type.as_ref() {
        Some(return_type) if is_plain_type(return_type, "int") => NativeSmokeFunctionReturn::Int,
        Some(return_type) if is_plain_type(return_type, "void") => NativeSmokeFunctionReturn::Void,
        _ if function.name == "main" => {
            return Err(BackendError::new(
                "wrong main signature for native Stage 10: expected `function main(): int` or `function main(): void`",
            ));
        }
        _ => return Err(unsupported_native_function_signature(&function.name)),
    };

    let mut params = Vec::new();
    for param in &function.params {
        if param.default.is_some() || !is_plain_type(&param.ty, "int") {
            return Err(unsupported_native_function_signature(&function.name));
        }
        params.push(NativeSmokeParam {
            name: param.name.clone(),
            writable: param.writable,
        });
    }

    Ok(NativeSmokeFunctionSignature {
        params,
        return_type,
    })
}

fn is_plain_type(ty: &crate::types::TypeRef, name: &str) -> bool {
    ty.name == name && ty.args.is_empty()
}

fn unsupported_native_function_signature(function_name: &str) -> BackendError {
    BackendError::new(format!(
        "unsupported native function signature for Stage 10: function `{function_name}` supports only `int` parameters and `int` or `void` returns"
    ))
}

fn validate_stage_10_function(
    name: &str,
    args: &[i64],
    context: &mut NativeSmokeValidationContext<'_>,
) -> Result<(), BackendError> {
    if context.validating.iter().any(|active| active == name) {
        return Err(BackendError::new(
            "unsupported native recursive function call for Stage 10",
        ));
    }

    let function = *context.declarations.get(name).ok_or_else(|| {
        BackendError::new(format!(
            "unsupported native function call for Stage 10: unknown function `{name}`"
        ))
    })?;
    let signature = context.signatures.get(name).cloned().ok_or_else(|| {
        BackendError::new(format!(
            "backend validation failure: native function `{name}` signature was not collected"
        ))
    })?;
    if args.len() != signature.params.len() {
        return Err(BackendError::new(format!(
            "backend validation failure: native function `{name}` expected {} evaluated argument(s), got {}",
            signature.params.len(),
            args.len()
        )));
    }

    let call_site = (name.to_string(), args.to_vec());
    if context.validated_call_sites.contains(&call_site) {
        return Ok(());
    }

    context.validating.push(name.to_string());
    let mut local_states = HashMap::new();
    for (param, value) in signature.params.iter().zip(args.iter().copied()) {
        local_states.insert(
            param.name.clone(),
            NativeSmokeLocalState {
                writable: param.writable,
                value: NativeSmokeValue::Int(value),
            },
        );
    }
    let body = validate_stage_6c_block(
        &function.body.statements,
        &local_states,
        signature.return_type,
        context,
    );
    context.validating.pop();
    let body = body?;

    context.validated_call_sites.insert(call_site);
    context
        .validated
        .entry(name.to_string())
        .or_insert(NativeSmokeFunction {
            name: name.to_string(),
            params: signature.params,
            return_type: signature.return_type,
            body,
        });
    Ok(())
}

fn validate_stage_10_call_statement(
    expr: &Expr,
    local_states: &HashMap<String, NativeSmokeLocalState>,
    context: &mut NativeSmokeValidationContext<'_>,
) -> Result<NativeSmokeStmt, BackendError> {
    let Expr::FunctionCall { name, args, .. } = expr else {
        return Err(BackendError::new(
            "unsupported native expression statement for Stage 10: expected supported free-function call",
        ));
    };
    let signature = context.signatures.get(name).cloned().ok_or_else(|| {
        BackendError::new(format!(
            "unsupported native function call for Stage 10: unknown function `{name}`"
        ))
    })?;
    if signature.return_type != NativeSmokeFunctionReturn::Void {
        return Err(BackendError::new(format!(
            "unsupported native function call for Stage 10: non-void function `{name}` cannot be used as a statement"
        )));
    }
    let (args, values) =
        validate_stage_10_call_args(name, args, &signature, local_states, context)?;
    validate_stage_10_function(name, &values, context)?;
    evaluate_native_function_from_context(context, name, &values)?;
    Ok(NativeSmokeStmt::Call(NativeSmokeCall {
        function: name.clone(),
        args,
        return_type: signature.return_type,
    }))
}

fn validate_stage_10_int_call(
    name: &str,
    args: &[Expr],
    local_states: &HashMap<String, NativeSmokeLocalState>,
    context: &mut NativeSmokeValidationContext<'_>,
) -> Result<ValidatedNativeSmokeExpr, BackendError> {
    let signature = context.signatures.get(name).cloned().ok_or_else(|| {
        BackendError::new(format!(
            "unsupported native function call for Stage 10: unknown function `{name}`"
        ))
    })?;
    if signature.return_type != NativeSmokeFunctionReturn::Int {
        return Err(BackendError::new(format!(
            "unsupported native function call for Stage 10: void function `{name}` cannot be used as an integer expression"
        )));
    }
    let (args, values) =
        validate_stage_10_call_args(name, args, &signature, local_states, context)?;
    validate_stage_10_function(name, &values, context)?;
    let value = match evaluate_native_function_from_context(context, name, &values)? {
        NativeSmokeFunctionOutcome::Int(value) => value,
        NativeSmokeFunctionOutcome::Void => {
            return Err(BackendError::new(format!(
                "backend validation failure: native function `{name}` returned void in integer expression"
            )));
        }
    };
    Ok(ValidatedNativeSmokeExpr {
        expr: NativeSmokeExpr::Call {
            function: name.to_string(),
            args,
        },
        value,
    })
}

fn validate_stage_10_call_args(
    name: &str,
    args: &[Expr],
    signature: &NativeSmokeFunctionSignature,
    local_states: &HashMap<String, NativeSmokeLocalState>,
    context: &mut NativeSmokeValidationContext<'_>,
) -> Result<(Vec<NativeSmokeExpr>, Vec<i64>), BackendError> {
    if args.len() != signature.params.len() {
        return Err(BackendError::new(format!(
            "unsupported native function call for Stage 10: function `{name}` expected {} argument(s), got {}",
            signature.params.len(),
            args.len()
        )));
    }

    let mut native_args = Vec::new();
    let mut values = Vec::new();
    for arg in args {
        let arg = validate_stage_6c_int_expr(arg, local_states, context)?;
        native_args.push(arg.expr);
        values.push(arg.value);
    }
    Ok((native_args, values))
}

fn validate_stage_6c_block(
    statements: &[Stmt],
    local_states: &HashMap<String, NativeSmokeLocalState>,
    function_return: NativeSmokeFunctionReturn,
    context: &mut NativeSmokeValidationContext<'_>,
) -> Result<NativeSmokeBlock, BackendError> {
    let mut block_states = local_states.clone();
    let mut native_statements = Vec::new();
    let mut terminal_index = 0;

    while let Some(statement) = statements.get(terminal_index) {
        match statement {
            Stmt::VarDecl(decl) => {
                let local = validate_stage_6c_local(decl, &block_states, context)?;
                block_states.insert(
                    local.name.clone(),
                    NativeSmokeLocalState {
                        writable: local.writable,
                        value: local.evaluated_value.clone(),
                    },
                );
                native_statements.push(NativeSmokeStmt::Local(local));
                terminal_index += 1;
            }
            Stmt::Assignment(assignment) => {
                let assignment = validate_stage_6c_assignment(assignment, &block_states, context)?;
                let Some(state) = block_states.get_mut(&assignment.target) else {
                    return Err(BackendError::new(
                        "backend validation failure: validated native assignment target was not declared",
                    ));
                };
                state.value = NativeSmokeValue::Int(assignment.evaluated_value);
                native_statements.push(NativeSmokeStmt::Assign(assignment));
                terminal_index += 1;
            }
            Stmt::Increment(increment) => {
                let assignment = validate_stage_6c_increment(increment, &block_states)?;
                let Some(state) = block_states.get_mut(&assignment.target) else {
                    return Err(BackendError::new(
                        "backend validation failure: validated native increment target was not declared",
                    ));
                };
                state.value = NativeSmokeValue::Int(assignment.evaluated_value);
                native_statements.push(NativeSmokeStmt::Assign(assignment));
                terminal_index += 1;
            }
            Stmt::Echo { expr, .. } => {
                native_statements.push(validate_stage_6c_echo(expr, &block_states)?);
                terminal_index += 1;
            }
            Stmt::Expr { expr, .. } if matches!(expr, Expr::FunctionCall { .. }) => {
                native_statements.push(validate_stage_10_call_statement(
                    expr,
                    &block_states,
                    context,
                )?);
                terminal_index += 1;
            }
            Stmt::While(while_stmt) => {
                let native_while = validate_stage_6c_while(while_stmt, &block_states, context)?;
                for (name, value) in &native_while.final_values {
                    let Some(state) = block_states.get_mut(name) else {
                        return Err(BackendError::new(
                            "backend validation failure: validated native while target was not declared",
                        ));
                    };
                    state.value = value.clone();
                }
                native_statements.push(NativeSmokeStmt::While(native_while));
                terminal_index += 1;
            }
            Stmt::For(for_stmt) => {
                let native_for = validate_stage_9_for(for_stmt, &block_states, context)?;
                merge_native_values(&mut block_states, &native_for.final_values)?;
                native_statements.push(NativeSmokeStmt::For(Box::new(native_for)));
                terminal_index += 1;
            }
            Stmt::Foreach(foreach) if grouped_range_expr(&foreach.iterable).is_some() => {
                let native_for = validate_stage_9_range_foreach(foreach, &block_states, context)?;
                merge_native_values(&mut block_states, &native_for.final_values)?;
                native_statements.push(NativeSmokeStmt::For(Box::new(native_for)));
                terminal_index += 1;
            }
            Stmt::If(if_stmt) => {
                match validate_stage_6c_fallthrough_if(if_stmt, &block_states, context) {
                    Ok(native_if) => {
                        merge_native_values(&mut block_states, &native_if.merged_values)?;
                        native_statements.push(NativeSmokeStmt::If(native_if));
                        terminal_index += 1;
                    }
                    Err(error) if should_defer_if_to_native_terminator(&error.message) => break,
                    Err(error) => return Err(error),
                }
            }
            _ => break,
        }
    }

    let terminator = validate_stage_6c_statement_sequence(
        &statements[terminal_index..],
        &block_states,
        function_return,
        context,
    )?;

    Ok(NativeSmokeBlock {
        statements: native_statements,
        terminator,
    })
}

fn merge_native_values(
    local_states: &mut HashMap<String, NativeSmokeLocalState>,
    values: &[(String, NativeSmokeValue)],
) -> Result<(), BackendError> {
    for (name, value) in values {
        let Some(state) = local_states.get_mut(name) else {
            return Err(BackendError::new(format!(
                "backend validation failure: validated native merged local `{name}` was not declared",
            )));
        };
        state.value = value.clone();
    }

    Ok(())
}

fn validate_stage_6c_statement_sequence(
    statements: &[Stmt],
    local_states: &HashMap<String, NativeSmokeLocalState>,
    function_return: NativeSmokeFunctionReturn,
    context: &mut NativeSmokeValidationContext<'_>,
) -> Result<NativeSmokeTerminator, BackendError> {
    match statements {
        [] if function_return == NativeSmokeFunctionReturn::Void => Ok(NativeSmokeTerminator::ExitSuccess),
        [] => Err(BackendError::new(
            "unsupported native block for Stage 7b: expected supported local declarations, assignments, string-literal echo statements, bounded while statements, or fallthrough if statements followed by a return, terminal if/else, or guard if with fallback",
        )),
        [Stmt::If(if_stmt), rest @ ..] if if_stmt.else_branch.is_none() => {
            validate_stage_6c_guard(if_stmt, rest, local_states, function_return, context)
        }
        [statement] => validate_stage_6c_terminator(statement, local_states, function_return, context),
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
    function_return: NativeSmokeFunctionReturn,
    context: &mut NativeSmokeValidationContext<'_>,
) -> Result<NativeSmokeTerminator, BackendError> {
    if fallback_statements.is_empty() && function_return != NativeSmokeFunctionReturn::Void {
        return Err(BackendError::new(
            "unsupported native branch fallthrough for Stage 7b: guard `if` without `else` requires a supported fallback block",
        ));
    }

    let condition = validate_stage_6c_condition(&if_stmt.condition, local_states, context)?;
    let then_block = validate_stage_6c_branch(
        &if_stmt.then_block.statements,
        local_states,
        function_return,
        context,
    )?;
    let fallback =
        validate_stage_6c_block(fallback_statements, local_states, function_return, context)?;

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
    context: &mut NativeSmokeValidationContext<'_>,
) -> Result<NativeSmokeIf, BackendError> {
    let condition =
        validate_stage_6c_condition(&if_stmt.condition, local_states, context).map_err(|error| {
            if should_preserve_native_expression_error(&error.message) {
                error
            } else {
                BackendError::new(
                    "unsupported native fallthrough if for Stage 7b: expected supported boolean condition",
                )
            }
        })?;

    let then_block =
        validate_stage_6c_fallthrough_block(&if_stmt.then_block.statements, local_states, context)?;
    let else_block = match &if_stmt.else_branch {
        Some(ElseBranch::Block(block)) => Some(validate_stage_6c_fallthrough_block(
            &block.statements,
            local_states,
            context,
        )?),
        Some(ElseBranch::If(else_if)) => {
            let else_if_statement = Stmt::If((**else_if).clone());
            Some(validate_stage_6c_fallthrough_block(
                &[else_if_statement],
                local_states,
                context,
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
        merged_values.push((name, state.value.clone()));
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
    context: &mut NativeSmokeValidationContext<'_>,
) -> Result<NativeSmokeFallthroughBlock, BackendError> {
    let mut block_states = local_states.clone();
    let mut visible_states = local_states.clone();
    let mut shadowed_locals = HashSet::new();
    let mut native_statements = Vec::new();

    for statement in statements {
        match statement {
            Stmt::VarDecl(decl) => {
                let local = validate_stage_6c_local(decl, &block_states, context)?;
                block_states.insert(
                    local.name.clone(),
                    NativeSmokeLocalState {
                        writable: local.writable,
                        value: local.evaluated_value.clone(),
                    },
                );
                if visible_states.contains_key(&local.name) {
                    shadowed_locals.insert(local.name.clone());
                }
                native_statements.push(NativeSmokeStmt::Local(local));
            }
            Stmt::Assignment(assignment) => {
                let assignment = validate_stage_6c_assignment(assignment, &block_states, context)?;
                let Some(state) = block_states.get_mut(&assignment.target) else {
                    return Err(BackendError::new(
                        "backend validation failure: validated native fallthrough assignment target was not declared",
                    ));
                };
                state.value = NativeSmokeValue::Int(assignment.evaluated_value);
                if visible_states.contains_key(&assignment.target)
                    && !shadowed_locals.contains(&assignment.target)
                {
                    let Some(visible_state) = visible_states.get_mut(&assignment.target) else {
                        return Err(BackendError::new(
                            "backend validation failure: validated native visible fallthrough assignment target was not declared",
                        ));
                    };
                    visible_state.value = NativeSmokeValue::Int(assignment.evaluated_value);
                }
                native_statements.push(NativeSmokeStmt::Assign(assignment));
            }
            Stmt::Increment(increment) => {
                let assignment = validate_stage_6c_increment(increment, &block_states)?;
                let Some(state) = block_states.get_mut(&assignment.target) else {
                    return Err(BackendError::new(
                        "backend validation failure: validated native fallthrough increment target was not declared",
                    ));
                };
                state.value = NativeSmokeValue::Int(assignment.evaluated_value);
                if visible_states.contains_key(&assignment.target)
                    && !shadowed_locals.contains(&assignment.target)
                {
                    let Some(visible_state) = visible_states.get_mut(&assignment.target) else {
                        return Err(BackendError::new(
                            "backend validation failure: validated native visible fallthrough increment target was not declared",
                        ));
                    };
                    visible_state.value = NativeSmokeValue::Int(assignment.evaluated_value);
                }
                native_statements.push(NativeSmokeStmt::Assign(assignment));
            }
            Stmt::Echo { expr, .. } => {
                native_statements.push(validate_stage_6c_echo(expr, &block_states)?);
            }
            Stmt::Expr { expr, .. } if matches!(expr, Expr::FunctionCall { .. }) => {
                native_statements.push(validate_stage_10_call_statement(
                    expr,
                    &block_states,
                    context,
                )?);
            }
            Stmt::While(while_stmt) => {
                let native_while = validate_stage_6c_while(while_stmt, &block_states, context)?;
                merge_native_values(&mut block_states, &native_while.final_values)?;
                merge_visible_native_values(
                    &mut visible_states,
                    &shadowed_locals,
                    &native_while.final_values,
                )?;
                native_statements.push(NativeSmokeStmt::While(native_while));
            }
            Stmt::If(if_stmt) => {
                let native_if = validate_stage_6c_fallthrough_if(if_stmt, &block_states, context)?;
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
    values: &[(String, NativeSmokeValue)],
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
        state.value = value.clone();
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
    context: &mut NativeSmokeValidationContext<'_>,
) -> Result<NativeSmokeLocal, BackendError> {
    if let Some(local) = validate_stage_8_string_local(decl, local_states)? {
        return Ok(local);
    }

    if let Some(ty) = &decl.ty {
        if ty.name != "int" || !ty.args.is_empty() {
            return Err(unsupported_current_native_local());
        }
    }

    let initializer = validate_stage_6c_int_expr(&decl.initializer, local_states, context)
        .map_err(|error| {
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
        evaluated_value: NativeSmokeValue::Int(initializer.value),
    })
}

fn validate_stage_8_string_local(
    decl: &hir::VarDecl,
    local_states: &HashMap<String, NativeSmokeLocalState>,
) -> Result<Option<NativeSmokeLocal>, BackendError> {
    let Some(expr) = try_validate_stage_8_string_expr(&decl.initializer, local_states)? else {
        return Ok(None);
    };

    if let Some(ty) = &decl.ty {
        if ty.name != "string" || !ty.args.is_empty() {
            return Ok(None);
        }
    }

    if decl.writable {
        return Err(unsupported_stage_8_writable_string_local());
    }

    let NativeSmokeExpr::StringLiteral(value) = &expr else {
        return Err(BackendError::new(
            "backend validation failure: validated native string local was not compile-time-known",
        ));
    };
    let evaluated_value = NativeSmokeValue::StringLiteral(value.clone());

    Ok(Some(NativeSmokeLocal {
        name: decl.name.clone(),
        writable: false,
        expr,
        evaluated_value,
    }))
}

fn try_validate_stage_8_string_expr(
    expr: &Expr,
    local_states: &HashMap<String, NativeSmokeLocalState>,
) -> Result<Option<NativeSmokeExpr>, BackendError> {
    match expr {
        Expr::String { value, .. } => Ok(Some(NativeSmokeExpr::StringLiteral(value.clone()))),
        Expr::Grouped { expr, .. } => try_validate_stage_8_string_expr(expr, local_states),
        Expr::Variable { name, .. } => Ok(local_states
            .get(name)
            .and_then(|state| state.value.as_string_literal())
            .map(|value| NativeSmokeExpr::StringLiteral(value.to_string()))),
        Expr::Binary {
            left,
            op: BinaryOp::Concat,
            right,
            ..
        } => {
            let left = validate_stage_8_string_expr(left, local_states)?;
            let right = validate_stage_8_string_expr(right, local_states)?;
            let NativeSmokeExpr::StringLiteral(left_value) = left else {
                return Err(BackendError::new(
                    "backend validation failure: validated native string concat left operand was not compile-time-known",
                ));
            };
            let NativeSmokeExpr::StringLiteral(right_value) = right else {
                return Err(BackendError::new(
                    "backend validation failure: validated native string concat right operand was not compile-time-known",
                ));
            };
            Ok(Some(NativeSmokeExpr::StringLiteral(format!(
                "{left_value}{right_value}"
            ))))
        }
        Expr::InterpolatedString { .. } => Err(unsupported_stage_8_string_interpolation()),
        _ => Ok(None),
    }
}

fn validate_stage_8_string_expr(
    expr: &Expr,
    local_states: &HashMap<String, NativeSmokeLocalState>,
) -> Result<NativeSmokeExpr, BackendError> {
    try_validate_stage_8_string_expr(expr, local_states)?
        .ok_or_else(|| unsupported_stage_8_string_expression(expr))
}

fn validate_stage_8_echo_expr(
    expr: &Expr,
    local_states: &HashMap<String, NativeSmokeLocalState>,
) -> Result<NativeSmokeExpr, BackendError> {
    try_validate_stage_8_string_expr(expr, local_states)?
        .ok_or_else(|| unsupported_stage_8_echo_expression(expr))
}

fn validate_stage_6c_echo(
    expr: &Expr,
    local_states: &HashMap<String, NativeSmokeLocalState>,
) -> Result<NativeSmokeStmt, BackendError> {
    Ok(NativeSmokeStmt::Echo(validate_stage_8_echo_expr(
        expr,
        local_states,
    )?))
}

fn unsupported_current_native_local() -> BackendError {
    BackendError::new(
        "unsupported native local for current native smoke backend: expected readonly or writable `int` local initialized from integer literals, supported integer locals, or supported integer arithmetic, or readonly `string` local initialized from a supported string expression",
    )
}

fn unsupported_stage_8_writable_string_local() -> BackendError {
    BackendError::new(
        "unsupported native string local for Stage 8: writable string locals are future work",
    )
}

fn unsupported_stage_8_string_expression(expr: &Expr) -> BackendError {
    BackendError::new(format!(
        "unsupported native string expression for Stage 8: expected supported string expression, found `{}`",
        describe_expression(expr)
    ))
}

fn unsupported_stage_8_echo_expression(expr: &Expr) -> BackendError {
    BackendError::new(format!(
        "unsupported native echo expression for Stage 8: expected supported string expression, found `{}`",
        describe_expression(expr)
    ))
}

fn unsupported_stage_8_string_interpolation() -> BackendError {
    BackendError::new(
        "unsupported native string interpolation for Stage 8: interpolation is future native string work",
    )
}

fn validate_stage_6c_assignment(
    assignment: &hir::Assignment,
    local_states: &HashMap<String, NativeSmokeLocalState>,
    context: &mut NativeSmokeValidationContext<'_>,
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

    let Some(target_value) = target.value.as_int() else {
        return Err(BackendError::new(
            "unsupported native string assignment for Stage 8: string assignments are future work",
        ));
    };

    let value = validate_stage_6c_int_expr(&assignment.value, local_states, context)?;
    let (op, evaluated_value) = match assignment.op {
        AssignOp::Assign => (NativeSmokeAssignOp::Assign, value.value),
        AssignOp::AddAssign => (
            NativeSmokeAssignOp::AddAssign,
            checked_native_arithmetic(target_value, NativeSmokeBinaryOp::Add, value.value)
                .ok_or_else(|| {
                    BackendError::new("integer arithmetic overflows the Doria `int` range")
                })?,
        ),
        AssignOp::SubAssign => (
            NativeSmokeAssignOp::SubAssign,
            checked_native_arithmetic(target_value, NativeSmokeBinaryOp::Subtract, value.value)
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

fn validate_stage_6c_increment(
    increment: &hir::IncrementStmt,
    local_states: &HashMap<String, NativeSmokeLocalState>,
) -> Result<NativeSmokeAssign, BackendError> {
    let (name, op, target_value) = validate_native_increment_target(increment, local_states)?;
    let evaluated_value = checked_native_increment_value(op, target_value)?;

    Ok(NativeSmokeAssign {
        target: name,
        op,
        expr: NativeSmokeExpr::Int(1),
        evaluated_value,
    })
}

fn validate_stage_6c_loop_increment(
    increment: &hir::IncrementStmt,
    local_states: &HashMap<String, NativeSmokeLocalState>,
) -> Result<NativeSmokeAssign, BackendError> {
    let (name, op, _) = validate_native_increment_target(increment, local_states)?;

    Ok(NativeSmokeAssign {
        target: name,
        op,
        expr: NativeSmokeExpr::Int(1),
        evaluated_value: 0,
    })
}

fn validate_native_increment_target(
    increment: &hir::IncrementStmt,
    local_states: &HashMap<String, NativeSmokeLocalState>,
) -> Result<(String, NativeSmokeAssignOp, i64), BackendError> {
    let Expr::Variable { name, .. } = &increment.target else {
        return Err(BackendError::new(
            "unsupported native increment for Stage 9: expected writable `int` local",
        ));
    };
    let Some(target) = local_states.get(name) else {
        return Err(BackendError::new(format!(
            "unsupported native increment for Stage 9: undeclared local `${name}`"
        )));
    };
    if !target.writable {
        return Err(BackendError::new(format!(
            "unsupported native increment for Stage 9: readonly local `${name}`"
        )));
    }
    let Some(target_value) = target.value.as_int() else {
        return Err(BackendError::new(
            "unsupported native increment for Stage 9: expected integer local",
        ));
    };
    let op = match increment.op {
        IncrementOp::Increment => NativeSmokeAssignOp::AddAssign,
        IncrementOp::Decrement => NativeSmokeAssignOp::SubAssign,
    };
    Ok((name.clone(), op, target_value))
}

fn checked_native_increment_value(
    op: NativeSmokeAssignOp,
    target_value: i64,
) -> Result<i64, BackendError> {
    let native_op = match op {
        NativeSmokeAssignOp::AddAssign => NativeSmokeBinaryOp::Add,
        NativeSmokeAssignOp::SubAssign => NativeSmokeBinaryOp::Subtract,
        NativeSmokeAssignOp::Assign => unreachable!("increments cannot lower to plain assignment"),
    };
    checked_native_arithmetic(target_value, native_op, 1)
        .ok_or_else(|| BackendError::new("integer arithmetic overflows the Doria `int` range"))
}

fn validate_stage_6c_while(
    while_stmt: &hir::WhileStmt,
    local_states: &HashMap<String, NativeSmokeLocalState>,
    context: &mut NativeSmokeValidationContext<'_>,
) -> Result<NativeSmokeWhile, BackendError> {
    let condition =
        validate_stage_6c_condition(&while_stmt.condition, local_states, context).map_err(|error| {
            if should_preserve_native_expression_error(&error.message) {
                error
            } else {
                BackendError::new(
                    "unsupported native while condition for Stage 7b: expected supported boolean condition",
                )
            }
        })?;

    let body = validate_stage_6c_while_body(&while_stmt.body.statements, local_states, context)?;
    let mut simulated_states = local_states.clone();
    let mut iterations = 0;

    loop {
        let values = native_state_values(&simulated_states);
        let condition_value = evaluate_native_condition_with_functions(
            &condition.condition,
            &values,
            &context.validated,
            &mut Vec::new(),
        )?;

        if !condition_value {
            break;
        }

        if iterations == STAGE_7B_LOOP_VERIFICATION_CAP {
            return Err(stage_6c_loop_cap_error());
        }

        let body_evaluation = evaluate_native_scoped_statements_with_functions(
            &context.validated,
            &body.statements,
            &simulated_states,
            &mut Vec::new(),
        )?;
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
        final_values.push((name, state.value.clone()));
    }

    Ok(NativeSmokeWhile {
        condition: condition.condition,
        body,
        final_values,
        evaluated_iterations: iterations,
    })
}

fn validate_stage_9_for(
    for_stmt: &hir::ForStmt,
    local_states: &HashMap<String, NativeSmokeLocalState>,
    context: &mut NativeSmokeValidationContext<'_>,
) -> Result<NativeSmokeFor, BackendError> {
    let mut loop_states = local_states.clone();
    let mut shadowed_initializer_binding = None;
    let initializer = match &for_stmt.initializer {
        Some(ForInitializer::VarDecl(decl)) => {
            let mut local = validate_stage_6c_local(decl, &loop_states, context)?;
            let source_binding_name = local.name.clone();
            if local_states.contains_key(&source_binding_name) {
                let native_binding_name = native_for_initializer_shadow_name(&source_binding_name);
                local.name = native_binding_name.clone();
                shadowed_initializer_binding = Some((source_binding_name, native_binding_name));
            }
            loop_states.insert(
                local.name.clone(),
                NativeSmokeLocalState {
                    writable: local.writable,
                    value: local.evaluated_value.clone(),
                },
            );
            Some(NativeSmokeForInitializer::Local(local))
        }
        Some(ForInitializer::Assignment(assignment)) => {
            let assignment = validate_stage_6c_assignment(assignment, &loop_states, context)?;
            let Some(state) = loop_states.get_mut(&assignment.target) else {
                return Err(BackendError::new(
                    "backend validation failure: validated native for assignment target was not declared",
                ));
            };
            state.value = NativeSmokeValue::Int(assignment.evaluated_value);
            Some(NativeSmokeForInitializer::Assign(assignment))
        }
        None => None,
    };

    let mut validation_loop_states = loop_states.clone();
    if let Some((source_binding_name, native_binding_name)) = &shadowed_initializer_binding {
        let Some(binding_state) = loop_states.get(native_binding_name).cloned() else {
            return Err(BackendError::new(
                "backend validation failure: validated native for initializer shadow binding was not declared",
            ));
        };
        validation_loop_states.insert(source_binding_name.clone(), binding_state);
    }

    let condition = if let Some(condition) = &for_stmt.condition {
        validate_stage_6c_loop_condition(condition, &validation_loop_states, context).map_err(|error| {
            if should_preserve_native_expression_error(&error.message) {
                error
            } else {
                BackendError::new(
                    "unsupported native for condition for Stage 9: expected supported boolean condition",
                )
            }
        })?
    } else {
        NativeSmokeCondition::Bool(true)
    };
    let condition =
        rename_native_shadowed_for_condition(condition, shadowed_initializer_binding.as_ref());

    let body = validate_stage_6c_while_branch_body(
        &for_stmt.body.statements,
        &validation_loop_states,
        context,
    )?;
    let body =
        if let Some((source_binding_name, native_binding_name)) = &shadowed_initializer_binding {
            rename_native_fallthrough_binding(body, source_binding_name, native_binding_name)
        } else {
            body
        };
    let increment = for_stmt
        .increment
        .as_ref()
        .map(|increment| {
            validate_stage_9_for_increment(increment, &validation_loop_states, context)
        })
        .transpose()?
        .map(|increment| {
            rename_native_shadowed_for_increment(increment, shadowed_initializer_binding.as_ref())
        });

    validate_stage_9_for_like(
        NativeSmokeForLikeParts {
            initializer,
            condition,
            increment,
            increment_exit_condition: None,
            body,
        },
        &loop_states,
        local_states,
        context,
    )
}

fn native_for_initializer_shadow_name(source_name: &str) -> String {
    format!("<for_initializer:{source_name}>")
}

fn grouped_range_expr(expr: &Expr) -> Option<(&Expr, &Expr, bool)> {
    match expr {
        Expr::Grouped { expr, .. } => grouped_range_expr(expr),
        Expr::Range {
            start,
            end,
            inclusive,
            ..
        } => Some((start, end, *inclusive)),
        _ => None,
    }
}

fn validate_stage_9_range_foreach(
    foreach: &hir::ForeachStmt,
    local_states: &HashMap<String, NativeSmokeLocalState>,
    context: &mut NativeSmokeValidationContext<'_>,
) -> Result<NativeSmokeFor, BackendError> {
    if foreach.key.is_some() {
        return Err(BackendError::new(
            "unsupported native foreach range for Stage 9: key bindings are future work",
        ));
    }

    let Some((start, end, inclusive)) = grouped_range_expr(&foreach.iterable) else {
        return Err(BackendError::new(
            "backend validation failure: Stage 9 range foreach validator received non-range iterable",
        ));
    };

    let start = validate_stage_6c_int_expr(start, local_states, context)?;
    let end = validate_stage_6c_int_expr(end, local_states, context)?;
    let source_binding_name = foreach.value.name.clone();
    let native_binding_name = if local_states.contains_key(&source_binding_name) {
        native_range_foreach_shadow_name(&source_binding_name)
    } else {
        source_binding_name.clone()
    };
    let binding_state = NativeSmokeLocalState {
        writable: false,
        value: NativeSmokeValue::Int(start.value),
    };
    let initializer = NativeSmokeForInitializer::Local(NativeSmokeLocal {
        name: native_binding_name.clone(),
        writable: false,
        expr: NativeSmokeExpr::Int(start.value),
        evaluated_value: NativeSmokeValue::Int(start.value),
    });

    let mut loop_states = local_states.clone();
    loop_states.insert(native_binding_name.clone(), binding_state.clone());

    let mut body_validation_states = loop_states.clone();
    if native_binding_name != source_binding_name {
        body_validation_states.insert(source_binding_name.clone(), binding_state);
    }

    let condition = NativeSmokeCondition::Compare {
        op: if inclusive {
            NativeSmokeCompareOp::LessThanOrEqual
        } else {
            NativeSmokeCompareOp::LessThan
        },
        left: NativeSmokeExpr::Local(native_binding_name.clone()),
        right: NativeSmokeExpr::Int(end.value),
    };
    let increment = NativeSmokeAssign {
        target: native_binding_name.clone(),
        op: NativeSmokeAssignOp::AddAssign,
        expr: NativeSmokeExpr::Int(1),
        evaluated_value: 0,
    };
    let increment_exit_condition = inclusive.then(|| NativeSmokeCondition::Compare {
        op: NativeSmokeCompareOp::Equal,
        left: NativeSmokeExpr::Local(native_binding_name.clone()),
        right: NativeSmokeExpr::Int(end.value),
    });
    let body = validate_stage_6c_while_branch_body(
        &foreach.body.statements,
        &body_validation_states,
        context,
    )?;
    let body = if native_binding_name != source_binding_name {
        rename_native_fallthrough_binding(body, &source_binding_name, &native_binding_name)
    } else {
        body
    };

    validate_stage_9_for_like(
        NativeSmokeForLikeParts {
            initializer: Some(initializer),
            condition,
            increment: Some(increment),
            increment_exit_condition,
            body,
        },
        &loop_states,
        local_states,
        context,
    )
}

fn native_range_foreach_shadow_name(source_name: &str) -> String {
    format!("<range_foreach:{source_name}>")
}

fn rename_native_shadowed_for_condition(
    mut condition: NativeSmokeCondition,
    binding: Option<&(String, String)>,
) -> NativeSmokeCondition {
    if let Some((source_name, native_name)) = binding {
        rename_native_condition_binding(&mut condition, source_name, native_name);
    }
    condition
}

fn rename_native_shadowed_for_increment(
    mut increment: NativeSmokeAssign,
    binding: Option<&(String, String)>,
) -> NativeSmokeAssign {
    if let Some((source_name, native_name)) = binding {
        rename_native_assignment_binding(&mut increment, source_name, native_name);
    }
    increment
}

fn rename_native_fallthrough_binding(
    mut block: NativeSmokeFallthroughBlock,
    source_name: &str,
    native_name: &str,
) -> NativeSmokeFallthroughBlock {
    rename_native_fallthrough_binding_in_scope(&mut block, source_name, native_name, true);
    block
}

fn rename_native_fallthrough_binding_in_scope(
    block: &mut NativeSmokeFallthroughBlock,
    source_name: &str,
    native_name: &str,
    rename_active: bool,
) {
    let mut active = rename_active;
    for statement in &mut block.statements {
        active = rename_native_statement_binding(statement, source_name, native_name, active);
    }
    if rename_active {
        rename_native_state_binding(&mut block.final_states, source_name, native_name);
    }
}

fn rename_native_statement_binding(
    statement: &mut NativeSmokeStmt,
    source_name: &str,
    native_name: &str,
    rename_active: bool,
) -> bool {
    match statement {
        NativeSmokeStmt::Local(local) => {
            if rename_active {
                rename_native_expr_binding(&mut local.expr, source_name, native_name);
            }
            rename_active && local.name != source_name
        }
        NativeSmokeStmt::Assign(assignment) => {
            if rename_active {
                rename_native_assignment_binding(assignment, source_name, native_name);
            }
            rename_active
        }
        NativeSmokeStmt::Echo(expr) => {
            if rename_active {
                rename_native_expr_binding(expr, source_name, native_name);
            }
            rename_active
        }
        NativeSmokeStmt::Call(call) => {
            if rename_active {
                for arg in &mut call.args {
                    rename_native_expr_binding(arg, source_name, native_name);
                }
            }
            rename_active
        }
        NativeSmokeStmt::While(native_while) => {
            if rename_active {
                rename_native_condition_binding(
                    &mut native_while.condition,
                    source_name,
                    native_name,
                );
                rename_native_fallthrough_binding_in_scope(
                    &mut native_while.body,
                    source_name,
                    native_name,
                    true,
                );
                rename_native_value_entries(
                    &mut native_while.final_values,
                    source_name,
                    native_name,
                );
            }
            rename_active
        }
        NativeSmokeStmt::For(native_for) => {
            if rename_active {
                if let Some(initializer) = &mut native_for.initializer {
                    rename_native_for_initializer_binding(initializer, source_name, native_name);
                }
                rename_native_condition_binding(
                    &mut native_for.condition,
                    source_name,
                    native_name,
                );
                if let Some(increment) = &mut native_for.increment {
                    rename_native_assignment_binding(increment, source_name, native_name);
                }
                if let Some(condition) = &mut native_for.increment_exit_condition {
                    rename_native_condition_binding(condition, source_name, native_name);
                }
                rename_native_fallthrough_binding_in_scope(
                    &mut native_for.body,
                    source_name,
                    native_name,
                    true,
                );
                rename_native_value_entries(
                    &mut native_for.carried_values,
                    source_name,
                    native_name,
                );
                rename_native_value_entries(&mut native_for.final_values, source_name, native_name);
            }
            rename_active
        }
        NativeSmokeStmt::If(native_if) => {
            if rename_active {
                rename_native_condition_binding(&mut native_if.condition, source_name, native_name);
                rename_native_fallthrough_binding_in_scope(
                    &mut native_if.then_block,
                    source_name,
                    native_name,
                    true,
                );
                if let Some(else_block) = &mut native_if.else_block {
                    rename_native_fallthrough_binding_in_scope(
                        else_block,
                        source_name,
                        native_name,
                        true,
                    );
                }
                rename_native_value_entries(&mut native_if.merged_values, source_name, native_name);
            }
            rename_active
        }
        NativeSmokeStmt::Break | NativeSmokeStmt::Continue => rename_active,
    }
}

fn rename_native_for_initializer_binding(
    initializer: &mut NativeSmokeForInitializer,
    source_name: &str,
    native_name: &str,
) {
    match initializer {
        NativeSmokeForInitializer::Local(local) => {
            rename_native_expr_binding(&mut local.expr, source_name, native_name);
        }
        NativeSmokeForInitializer::Assign(assignment) => {
            rename_native_assignment_binding(assignment, source_name, native_name);
        }
    }
}

fn rename_native_assignment_binding(
    assignment: &mut NativeSmokeAssign,
    source_name: &str,
    native_name: &str,
) {
    if assignment.target == source_name {
        assignment.target = native_name.to_string();
    }
    rename_native_expr_binding(&mut assignment.expr, source_name, native_name);
}

fn rename_native_expr_binding(expr: &mut NativeSmokeExpr, source_name: &str, native_name: &str) {
    match expr {
        NativeSmokeExpr::Local(name) if name == source_name => {
            *name = native_name.to_string();
        }
        NativeSmokeExpr::Binary { left, right, .. } => {
            rename_native_expr_binding(left, source_name, native_name);
            rename_native_expr_binding(right, source_name, native_name);
        }
        NativeSmokeExpr::Call { args, .. } => {
            for arg in args {
                rename_native_expr_binding(arg, source_name, native_name);
            }
        }
        NativeSmokeExpr::Int(_) | NativeSmokeExpr::Local(_) | NativeSmokeExpr::StringLiteral(_) => {
        }
    }
}

fn rename_native_condition_binding(
    condition: &mut NativeSmokeCondition,
    source_name: &str,
    native_name: &str,
) {
    match condition {
        NativeSmokeCondition::Compare { left, right, .. } => {
            rename_native_expr_binding(left, source_name, native_name);
            rename_native_expr_binding(right, source_name, native_name);
        }
        NativeSmokeCondition::Not(condition) => {
            rename_native_condition_binding(condition, source_name, native_name);
        }
        NativeSmokeCondition::And { left, right }
        | NativeSmokeCondition::Or { left, right }
        | NativeSmokeCondition::Xor { left, right } => {
            rename_native_condition_binding(left, source_name, native_name);
            rename_native_condition_binding(right, source_name, native_name);
        }
        NativeSmokeCondition::Bool(_) => {}
    }
}

fn rename_native_state_binding(
    states: &mut HashMap<String, NativeSmokeLocalState>,
    source_name: &str,
    native_name: &str,
) {
    if let Some(state) = states.remove(source_name) {
        states.insert(native_name.to_string(), state);
    }
}

fn rename_native_value_entries(
    entries: &mut Vec<(String, NativeSmokeValue)>,
    source_name: &str,
    native_name: &str,
) {
    let mut renamed = HashMap::new();
    for (name, value) in entries.drain(..) {
        let name = if name == source_name {
            native_name.to_string()
        } else {
            name
        };
        renamed.insert(name, value);
    }
    *entries = renamed.into_iter().collect();
    entries.sort_by(|left, right| left.0.cmp(&right.0));
}
fn validate_stage_9_for_increment(
    increment: &ForIncrement,
    local_states: &HashMap<String, NativeSmokeLocalState>,
    context: &mut NativeSmokeValidationContext<'_>,
) -> Result<NativeSmokeAssign, BackendError> {
    match increment {
        ForIncrement::Increment(increment) => {
            validate_stage_6c_loop_increment(increment, local_states)
        }
        ForIncrement::Assignment(assignment) => {
            validate_stage_6c_loop_assignment(assignment, local_states, context)
        }
    }
}

fn validate_stage_9_for_like(
    parts: NativeSmokeForLikeParts,
    initial_loop_states: &HashMap<String, NativeSmokeLocalState>,
    outer_states: &HashMap<String, NativeSmokeLocalState>,
    context: &mut NativeSmokeValidationContext<'_>,
) -> Result<NativeSmokeFor, BackendError> {
    let NativeSmokeForLikeParts {
        initializer,
        condition,
        increment,
        increment_exit_condition,
        body,
    } = parts;
    let mut simulated_states = initial_loop_states.clone();
    let mut iterations = 0;

    loop {
        let values = native_state_values(&simulated_states);
        let condition_value = evaluate_native_condition_with_functions(
            &condition,
            &values,
            &context.validated,
            &mut Vec::new(),
        )?;

        if !condition_value {
            break;
        }

        if iterations == STAGE_7B_LOOP_VERIFICATION_CAP {
            return Err(stage_9_loop_cap_error());
        }

        let body_evaluation = evaluate_native_scoped_statements_with_functions(
            &context.validated,
            &body.statements,
            &simulated_states,
            &mut Vec::new(),
        )?;
        simulated_states = body_evaluation.visible_states;

        if body_evaluation.control == NativeSmokeLoopControl::Break {
            break;
        }

        if let Some(condition) = &increment_exit_condition {
            let values = native_state_values(&simulated_states);
            let exit_before_increment = evaluate_native_condition_with_functions(
                condition,
                &values,
                &context.validated,
                &mut Vec::new(),
            )?;
            if exit_before_increment {
                break;
            }
        }

        if let Some(increment) = &increment {
            evaluate_native_for_increment(
                increment,
                &mut simulated_states,
                &context.validated,
                &mut Vec::new(),
            )?;
        }

        iterations += 1;
    }

    let mut carried_values = Vec::new();
    for name in sorted_native_local_names(initial_loop_states) {
        let Some(state) = simulated_states.get(&name) else {
            return Err(BackendError::new(
                "backend validation failure: validated native for carried local was not declared",
            ));
        };
        carried_values.push((name, state.value.clone()));
    }

    let mut final_values = Vec::new();
    for name in sorted_native_local_names(outer_states) {
        let Some(state) = simulated_states.get(&name) else {
            return Err(BackendError::new(
                "backend validation failure: validated native for target was not declared",
            ));
        };
        final_values.push((name, state.value.clone()));
    }

    Ok(NativeSmokeFor {
        initializer,
        condition,
        increment,
        increment_exit_condition,
        body,
        carried_values,
        final_values,
        evaluated_iterations: iterations,
    })
}

fn evaluate_native_for_increment(
    increment: &NativeSmokeAssign,
    local_states: &mut HashMap<String, NativeSmokeLocalState>,
    functions: &HashMap<String, NativeSmokeFunction>,
    call_stack: &mut Vec<String>,
) -> Result<(), BackendError> {
    let values = native_state_values(local_states);
    let Some(target) = local_states.get(&increment.target) else {
        return Err(BackendError::new(
            "backend validation failure: validated native for increment target was not declared",
        ));
    };
    let Some(target_value) = target.value.as_int() else {
        return Err(BackendError::new(
            "backend validation failure: validated native for increment target was not an integer local",
        ));
    };
    let evaluated_value = evaluate_native_assignment_value_with_functions(
        increment.op,
        target_value,
        &increment.expr,
        &values,
        functions,
        call_stack,
    )?;
    let Some(target) = local_states.get_mut(&increment.target) else {
        return Err(BackendError::new(
            "backend validation failure: validated native for increment target was not declared",
        ));
    };
    target.value = NativeSmokeValue::Int(evaluated_value);
    Ok(())
}

fn validate_stage_6c_while_body(
    statements: &[Stmt],
    local_states: &HashMap<String, NativeSmokeLocalState>,
    context: &mut NativeSmokeValidationContext<'_>,
) -> Result<NativeSmokeFallthroughBlock, BackendError> {
    if statements.is_empty() {
        return Err(BackendError::new(
            "unsupported native while body for Stage 7b: expected one or more supported local declarations, assignments, or fallthrough if statements",
        ));
    }

    validate_stage_6c_while_scoped_body(statements, local_states, context)
}

fn validate_stage_6c_while_branch_body(
    statements: &[Stmt],
    local_states: &HashMap<String, NativeSmokeLocalState>,
    context: &mut NativeSmokeValidationContext<'_>,
) -> Result<NativeSmokeFallthroughBlock, BackendError> {
    validate_stage_6c_while_scoped_body(statements, local_states, context)
}

fn validate_stage_6c_while_scoped_body(
    statements: &[Stmt],
    local_states: &HashMap<String, NativeSmokeLocalState>,
    context: &mut NativeSmokeValidationContext<'_>,
) -> Result<NativeSmokeFallthroughBlock, BackendError> {
    let mut block_states = local_states.clone();
    let mut visible_states = local_states.clone();
    let mut shadowed_locals = HashSet::new();
    let mut native_statements = Vec::new();

    for statement in statements {
        match statement {
            Stmt::VarDecl(decl) => {
                let local = validate_stage_6c_loop_local(decl, &block_states, context)?;
                block_states.insert(
                    local.name.clone(),
                    NativeSmokeLocalState {
                        writable: local.writable,
                        value: local.evaluated_value.clone(),
                    },
                );
                if visible_states.contains_key(&local.name) {
                    shadowed_locals.insert(local.name.clone());
                }
                native_statements.push(NativeSmokeStmt::Local(local));
            }
            Stmt::Assignment(assignment) => {
                let assignment =
                    validate_stage_6c_loop_assignment(assignment, &block_states, context)?;
                let Some(state) = block_states.get_mut(&assignment.target) else {
                    return Err(BackendError::new(
                        "backend validation failure: validated native while assignment target was not declared",
                    ));
                };
                state.value = NativeSmokeValue::Int(assignment.evaluated_value);
                if visible_states.contains_key(&assignment.target)
                    && !shadowed_locals.contains(&assignment.target)
                {
                    let Some(visible_state) = visible_states.get_mut(&assignment.target) else {
                        return Err(BackendError::new(
                            "backend validation failure: validated native visible while assignment target was not declared",
                        ));
                    };
                    visible_state.value = NativeSmokeValue::Int(assignment.evaluated_value);
                }
                native_statements.push(NativeSmokeStmt::Assign(assignment));
            }
            Stmt::Increment(increment) => {
                let assignment = validate_stage_6c_loop_increment(increment, &block_states)?;
                let Some(state) = block_states.get_mut(&assignment.target) else {
                    return Err(BackendError::new(
                        "backend validation failure: validated native while increment target was not declared",
                    ));
                };
                state.value = NativeSmokeValue::Int(assignment.evaluated_value);
                if visible_states.contains_key(&assignment.target)
                    && !shadowed_locals.contains(&assignment.target)
                {
                    let Some(visible_state) = visible_states.get_mut(&assignment.target) else {
                        return Err(BackendError::new(
                            "backend validation failure: validated native visible while increment target was not declared",
                        ));
                    };
                    visible_state.value = NativeSmokeValue::Int(assignment.evaluated_value);
                }
                native_statements.push(NativeSmokeStmt::Assign(assignment));
            }
            Stmt::Echo { expr, .. } => {
                native_statements.push(validate_stage_6c_echo(expr, &block_states)?);
            }
            Stmt::Expr { expr, .. } if matches!(expr, Expr::FunctionCall { .. }) => {
                native_statements.push(validate_stage_10_call_statement(
                    expr,
                    &block_states,
                    context,
                )?);
            }
            Stmt::If(if_stmt) => {
                let native_if =
                    validate_stage_6c_loop_fallthrough_if(if_stmt, &block_states, context)?;
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
    context: &mut NativeSmokeValidationContext<'_>,
) -> Result<NativeSmokeLocal, BackendError> {
    if let Some(local) = validate_stage_8_string_local(decl, local_states)? {
        return Ok(local);
    }

    if let Some(ty) = &decl.ty {
        if ty.name != "int" || !ty.args.is_empty() {
            return Err(unsupported_current_native_local());
        }
    }

    let expr = validate_stage_6c_loop_int_expr(&decl.initializer, local_states, context).map_err(
        |error| {
            if should_preserve_native_expression_error(&error.message) {
                error
            } else {
                unsupported_current_native_local()
            }
        },
    )?;

    Ok(NativeSmokeLocal {
        name: decl.name.clone(),
        writable: decl.writable,
        expr,
        evaluated_value: NativeSmokeValue::Int(0),
    })
}

fn validate_stage_6c_loop_assignment(
    assignment: &hir::Assignment,
    local_states: &HashMap<String, NativeSmokeLocalState>,
    context: &mut NativeSmokeValidationContext<'_>,
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

    if target.value.as_int().is_none() {
        return Err(BackendError::new(
            "unsupported native string assignment for Stage 8: string assignments are future work",
        ));
    }

    let value = validate_stage_6c_loop_int_expr(&assignment.value, local_states, context)?;
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
    context: &mut NativeSmokeValidationContext<'_>,
) -> Result<NativeSmokeIf, BackendError> {
    let condition =
        validate_stage_6c_loop_condition(&if_stmt.condition, local_states, context).map_err(|error| {
            if should_preserve_native_expression_error(&error.message) {
                error
            } else {
                BackendError::new(
                    "unsupported native while body statement for Stage 7b: expected supported fallthrough if condition",
                )
            }
        })?;

    let then_block =
        validate_stage_6c_while_branch_body(&if_stmt.then_block.statements, local_states, context)?;
    let else_block = match &if_stmt.else_branch {
        Some(ElseBranch::Block(block)) => Some(validate_stage_6c_while_branch_body(
            &block.statements,
            local_states,
            context,
        )?),
        Some(ElseBranch::If(else_if)) => {
            let else_if_statement = Stmt::If((**else_if).clone());
            Some(validate_stage_6c_while_branch_body(
                &[else_if_statement],
                local_states,
                context,
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
            Ok((name, state.value.clone()))
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
    context: &mut NativeSmokeValidationContext<'_>,
) -> Result<NativeSmokeCondition, BackendError> {
    match expr {
        Expr::Bool { value, .. } => Ok(NativeSmokeCondition::Bool(*value)),
        Expr::Grouped { expr, .. } => validate_stage_6c_loop_condition(expr, local_states, context),
        Expr::Unary {
            op: UnaryOp::Not,
            expr,
            ..
        } => Ok(NativeSmokeCondition::Not(Box::new(
            validate_stage_6c_loop_condition(expr, local_states, context)?,
        ))),
        Expr::Binary {
            left, op, right, ..
        } if native_compare_op(op).is_some() => {
            let native_op = native_compare_op(op).expect("checked by guard");
            Ok(NativeSmokeCondition::Compare {
                op: native_op,
                left: validate_stage_6c_loop_int_expr(left, local_states, context)?,
                right: validate_stage_6c_loop_int_expr(right, local_states, context)?,
            })
        }
        Expr::Binary {
            left,
            op: BinaryOp::And,
            right,
            ..
        } => Ok(NativeSmokeCondition::And {
            left: Box::new(validate_stage_6c_loop_condition(left, local_states, context)?),
            right: Box::new(validate_stage_6c_loop_condition(right, local_states, context)?),
        }),
        Expr::Binary {
            left,
            op: BinaryOp::Or,
            right,
            ..
        } => Ok(NativeSmokeCondition::Or {
            left: Box::new(validate_stage_6c_loop_condition(left, local_states, context)?),
            right: Box::new(validate_stage_6c_loop_condition(right, local_states, context)?),
        }),
        Expr::Binary {
            left,
            op: BinaryOp::Xor,
            right,
            ..
        } => Ok(NativeSmokeCondition::Xor {
            left: Box::new(validate_stage_6c_loop_condition(left, local_states, context)?),
            right: Box::new(validate_stage_6c_loop_condition(right, local_states, context)?),
        }),
        _ => Err(BackendError::new(
            "unsupported native condition for Stage 7b: expected bool literal, supported integer comparison, or supported boolean condition",
        )),
    }
}

fn validate_stage_6c_loop_int_expr(
    expr: &Expr,
    local_states: &HashMap<String, NativeSmokeLocalState>,
    context: &mut NativeSmokeValidationContext<'_>,
) -> Result<NativeSmokeExpr, BackendError> {
    match expr {
        Expr::Int { value, .. } => Ok(NativeSmokeExpr::Int(parse_doria_int_literal(value)?)),
        Expr::Variable { name, .. } => {
            let Some(state) = local_states.get(name) else {
                return Err(BackendError::new(
                    "unsupported native expression for Stage 7b: expected integer literal, supported integer local, or supported integer arithmetic",
                ));
            };
            if state.value.as_int().is_none() {
                return Err(BackendError::new(
                    "unsupported native expression for Stage 7b: expected integer literal, supported integer local, or supported integer arithmetic",
                ));
            }

            Ok(NativeSmokeExpr::Local(name.clone()))
        }
        Expr::Grouped { expr, .. } => validate_stage_6c_loop_int_expr(expr, local_states, context),
        Expr::FunctionCall { name, args, .. } => Ok(validate_stage_10_int_call(
            name,
            args,
            local_states,
            context,
        )?
        .expr),
        Expr::Binary {
            left, op, right, ..
        } if native_binary_op(op).is_some() => {
            let native_op = native_binary_op(op).expect("checked by guard");
            Ok(NativeSmokeExpr::Binary {
                op: native_op,
                left: Box::new(validate_stage_6c_loop_int_expr(left, local_states, context)?),
                right: Box::new(validate_stage_6c_loop_int_expr(right, local_states, context)?),
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
        .filter_map(|(name, state)| state.value.as_int().map(|value| (name.clone(), value)))
        .collect()
}

fn evaluate_native_assignment_value_with_functions(
    op: NativeSmokeAssignOp,
    current_value: i64,
    expr: &NativeSmokeExpr,
    local_values: &HashMap<String, i64>,
    functions: &HashMap<String, NativeSmokeFunction>,
    call_stack: &mut Vec<String>,
) -> Result<i64, BackendError> {
    let value = evaluate_native_expr_with_functions(expr, local_values, functions, call_stack)?;
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

fn evaluate_native_scoped_statements_with_functions(
    functions: &HashMap<String, NativeSmokeFunction>,
    statements: &[NativeSmokeStmt],
    visible_states: &HashMap<String, NativeSmokeLocalState>,
    call_stack: &mut Vec<String>,
) -> Result<NativeSmokeLoopEvaluation, BackendError> {
    let mut block_states = visible_states.clone();
    let mut next_visible_states = visible_states.clone();
    let mut shadowed_locals = HashSet::new();

    for statement in statements {
        match statement {
            NativeSmokeStmt::Local(local) => {
                let value =
                    evaluate_native_local_value(functions, local, &block_states, call_stack)?;
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
                evaluate_native_assignment(functions, assignment, &mut block_states, call_stack)?;
                if next_visible_states.contains_key(&assignment.target)
                    && !shadowed_locals.contains(&assignment.target)
                {
                    let Some(updated) = block_states.get(&assignment.target).cloned() else {
                        return Err(BackendError::new(
                            "backend validation failure: validated native visible assignment target was not declared",
                        ));
                    };
                    next_visible_states.insert(assignment.target.clone(), updated);
                }
            }
            NativeSmokeStmt::Echo(_) => {}
            NativeSmokeStmt::Call(call) => {
                evaluate_native_call_statement(functions, call, &block_states, call_stack)?;
            }
            NativeSmokeStmt::If(native_if) => {
                let updated_states = evaluate_native_scoped_if_with_functions(
                    functions,
                    native_if,
                    &block_states,
                    call_stack,
                )?;
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
            NativeSmokeStmt::For(_) => {
                return Err(BackendError::new(
                    "unsupported native loop body statement for Stage 9: nested for/range loops are future native work",
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

fn evaluate_native_scoped_if_with_functions(
    functions: &HashMap<String, NativeSmokeFunction>,
    native_if: &NativeSmokeIf,
    local_states: &HashMap<String, NativeSmokeLocalState>,
    call_stack: &mut Vec<String>,
) -> Result<NativeSmokeLoopEvaluation, BackendError> {
    let values = native_state_values(local_states);
    let condition_value = evaluate_native_condition_with_functions(
        &native_if.condition,
        &values,
        functions,
        call_stack,
    )?;

    if condition_value {
        evaluate_native_scoped_statements_with_functions(
            functions,
            &native_if.then_block.statements,
            local_states,
            call_stack,
        )
    } else if let Some(else_block) = &native_if.else_block {
        evaluate_native_scoped_statements_with_functions(
            functions,
            &else_block.statements,
            local_states,
            call_stack,
        )
    } else {
        Ok(NativeSmokeLoopEvaluation {
            visible_states: local_states.clone(),
            control: NativeSmokeLoopControl::Fallthrough,
        })
    }
}

fn evaluate_native_expr_with_functions(
    expr: &NativeSmokeExpr,
    local_values: &HashMap<String, i64>,
    functions: &HashMap<String, NativeSmokeFunction>,
    call_stack: &mut Vec<String>,
) -> Result<i64, BackendError> {
    match expr {
        NativeSmokeExpr::Int(value) => Ok(*value),
        NativeSmokeExpr::Local(name) => local_values.get(name).copied().ok_or_else(|| {
            BackendError::new(format!(
                "backend validation failure: validated native local `{name}` was unavailable for evaluation"
            ))
        }),
        NativeSmokeExpr::StringLiteral(_) => Err(BackendError::new(
            "backend validation failure: native string expression reached integer evaluation",
        )),
        NativeSmokeExpr::Binary { op, left, right } => checked_native_arithmetic(
            evaluate_native_expr_with_functions(left, local_values, functions, call_stack)?,
            *op,
            evaluate_native_expr_with_functions(right, local_values, functions, call_stack)?,
        )
        .ok_or_else(|| BackendError::new("integer arithmetic overflows the Doria `int` range")),
        NativeSmokeExpr::Call { function, args } => {
            let values = args
                .iter()
                .map(|arg| evaluate_native_expr_with_functions(arg, local_values, functions, call_stack))
                .collect::<Result<Vec<_>, _>>()?;
            match evaluate_native_function_from_map(functions, function, &values, call_stack)? {
                NativeSmokeFunctionOutcome::Int(value) => Ok(value),
                NativeSmokeFunctionOutcome::Void => Err(BackendError::new(format!(
                    "backend validation failure: void native function `{function}` reached integer evaluation"
                ))),
            }
        }
    }
}

fn evaluate_native_condition_with_functions(
    condition: &NativeSmokeCondition,
    local_values: &HashMap<String, i64>,
    functions: &HashMap<String, NativeSmokeFunction>,
    call_stack: &mut Vec<String>,
) -> Result<bool, BackendError> {
    match condition {
        NativeSmokeCondition::Bool(value) => Ok(*value),
        NativeSmokeCondition::Compare { op, left, right } => Ok(evaluate_native_compare(
            evaluate_native_expr_with_functions(left, local_values, functions, call_stack)?,
            *op,
            evaluate_native_expr_with_functions(right, local_values, functions, call_stack)?,
        )),
        NativeSmokeCondition::Not(condition) => Ok(!evaluate_native_condition_with_functions(
            condition,
            local_values,
            functions,
            call_stack,
        )?),
        NativeSmokeCondition::And { left, right } => {
            if !evaluate_native_condition_with_functions(left, local_values, functions, call_stack)?
            {
                Ok(false)
            } else {
                evaluate_native_condition_with_functions(right, local_values, functions, call_stack)
            }
        }
        NativeSmokeCondition::Or { left, right } => {
            if evaluate_native_condition_with_functions(left, local_values, functions, call_stack)?
            {
                Ok(true)
            } else {
                evaluate_native_condition_with_functions(right, local_values, functions, call_stack)
            }
        }
        NativeSmokeCondition::Xor { left, right } => Ok(evaluate_native_condition_with_functions(
            left,
            local_values,
            functions,
            call_stack,
        )?
            ^ evaluate_native_condition_with_functions(
                right,
                local_values,
                functions,
                call_stack,
            )?),
    }
}

fn stage_6c_loop_cap_error() -> BackendError {
    BackendError::new(
        "unsupported native while loop for Stage 7b: loop could not be proven to terminate within the current native smoke verification cap",
    )
}

fn stage_9_loop_cap_error() -> BackendError {
    BackendError::new(
        "unsupported native Stage 9 loop: loop could not be proven to terminate within the current native smoke verification cap",
    )
}

fn validate_stage_6c_terminator(
    statement: &Stmt,
    local_states: &HashMap<String, NativeSmokeLocalState>,
    function_return: NativeSmokeFunctionReturn,
    context: &mut NativeSmokeValidationContext<'_>,
) -> Result<NativeSmokeTerminator, BackendError> {
    match statement {
        Stmt::Return { expr: None, .. } if function_return == NativeSmokeFunctionReturn::Void => {
            Ok(NativeSmokeTerminator::ExitSuccess)
        }
        Stmt::Return { expr: Some(expr), .. } => {
            let (expr, evaluated_value) = validate_stage_6c_return_expr(expr, local_states, context)?;
            Ok(NativeSmokeTerminator::Return {
                expr,
                evaluated_value,
            })
        }
        Stmt::Return { expr: None, .. } => Err(BackendError::new(
            "unsupported native terminal statement for Stage 7b: expected `return <portable integer expression>;`, found bare `return;`",
        )),
        Stmt::If(if_stmt) => {
            let condition = validate_stage_6c_condition(&if_stmt.condition, local_states, context)?;
            let then_block =
                validate_stage_6c_branch(&if_stmt.then_block.statements, local_states, function_return, context)?;

            let Some(else_branch) = &if_stmt.else_branch else {
                return Err(BackendError::new(
                    "unsupported native terminal if for Stage 7b: terminal if requires else; guard if without else is supported only when followed by a fallback return",
                ));
            };

            let else_block = match else_branch {
                ElseBranch::Block(else_block) => {
                    validate_stage_6c_branch(&else_block.statements, local_states, function_return, context)?
                }
                ElseBranch::If(else_if) => {
                    validate_stage_6c_if_as_block(else_if, local_states, function_return, context)?
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
    function_return: NativeSmokeFunctionReturn,
    context: &mut NativeSmokeValidationContext<'_>,
) -> Result<NativeSmokeBlock, BackendError> {
    validate_stage_6c_block(statements, local_states, function_return, context).map_err(|error| {
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
    function_return: NativeSmokeFunctionReturn,
    context: &mut NativeSmokeValidationContext<'_>,
) -> Result<NativeSmokeBlock, BackendError> {
    let statement = Stmt::If(if_stmt.clone());
    validate_stage_6c_block(&[statement], local_states, function_return, context)
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
    context: &mut NativeSmokeValidationContext<'_>,
) -> Result<(NativeSmokeExpr, i64), BackendError> {
    let return_expr = validate_stage_6c_int_expr(expr, local_states, context)?;
    Ok((return_expr.expr, return_expr.value))
}

fn validate_stage_6c_int_expr(
    expr: &Expr,
    local_states: &HashMap<String, NativeSmokeLocalState>,
    context: &mut NativeSmokeValidationContext<'_>,
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
            let value = local_states
                .get(name)
                .and_then(|state| state.value.as_int())
                .ok_or_else(|| {
                    BackendError::new(
                        "unsupported native expression for Stage 7b: expected integer literal, supported integer local, or supported integer arithmetic",
                    )
                })?;
            Ok(ValidatedNativeSmokeExpr {
                expr: NativeSmokeExpr::Local(name.clone()),
                value,
            })
        }
        Expr::Grouped { expr, .. } => validate_stage_6c_int_expr(expr, local_states, context),
        Expr::FunctionCall { name, args, .. } => {
            validate_stage_10_int_call(name, args, local_states, context)
        }
        Expr::Binary {
            left, op, right, ..
        } if native_binary_op(op).is_some() => {
            let native_op = native_binary_op(op).expect("checked by guard");
            let left = validate_stage_6c_int_expr(left, local_states, context)?;
            let right = validate_stage_6c_int_expr(right, local_states, context)?;
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
    context: &mut NativeSmokeValidationContext<'_>,
) -> Result<ValidatedNativeSmokeCondition, BackendError> {
    match expr {
        Expr::Bool { value, .. } => Ok(ValidatedNativeSmokeCondition {
            condition: NativeSmokeCondition::Bool(*value),
            value: *value,
        }),
        Expr::Grouped { expr, .. } => validate_stage_6c_condition(expr, local_states, context),
        Expr::Unary {
            op: UnaryOp::Not,
            expr,
            ..
        } => {
            let condition = validate_stage_6c_condition(expr, local_states, context)?;
            Ok(ValidatedNativeSmokeCondition {
                condition: NativeSmokeCondition::Not(Box::new(condition.condition)),
                value: !condition.value,
            })
        }
        Expr::Binary {
            left, op, right, ..
        } if native_compare_op(op).is_some() => {
            let native_op = native_compare_op(op).expect("checked by guard");
            let left = validate_stage_6c_comparison_operand(left, local_states, context)?;
            let right = validate_stage_6c_comparison_operand(right, local_states, context)?;
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
            let left = validate_stage_6c_condition(left, local_states, context)?;
            let right = validate_stage_6c_condition(right, local_states, context)?;
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
            let left = validate_stage_6c_condition(left, local_states, context)?;
            let right = validate_stage_6c_condition(right, local_states, context)?;
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
            let left = validate_stage_6c_condition(left, local_states, context)?;
            let right = validate_stage_6c_condition(right, local_states, context)?;
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
    context: &mut NativeSmokeValidationContext<'_>,
) -> Result<ValidatedNativeSmokeExpr, BackendError> {
    validate_stage_6c_int_expr(expr, local_states, context).map_err(|error| {
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

fn validate_native_process_exit_boundary(module: &NativeSmokeModule) -> Result<(), BackendError> {
    let main = module
        .functions
        .iter()
        .find(|function| function.name == module.main_name)
        .ok_or_else(|| {
            BackendError::new("backend validation failure: native main was not found")
        })?;

    match main.return_type {
        NativeSmokeFunctionReturn::Int => validate_native_process_return_block(&main.body),
        NativeSmokeFunctionReturn::Void => try_evaluate_exit_code(module).map(|_| ()),
    }
}

fn validate_native_process_return_block(block: &NativeSmokeBlock) -> Result<(), BackendError> {
    match &block.terminator {
        NativeSmokeTerminator::Return {
            evaluated_value, ..
        } => parse_stage_6c_exit_code(*evaluated_value).map(|_| ()),
        NativeSmokeTerminator::IfElse {
            then_block,
            else_block,
            ..
        } => {
            validate_native_process_return_block(then_block)?;
            validate_native_process_return_block(else_block)
        }
        NativeSmokeTerminator::Guard {
            then_block,
            fallback,
            ..
        } => {
            validate_native_process_return_block(then_block)?;
            validate_native_process_return_block(fallback)
        }
        NativeSmokeTerminator::ExitSuccess => Err(BackendError::new(
            "backend validation failure: int-returning native main fell through",
        )),
    }
}

pub(crate) fn evaluate_exit_code(module: &NativeSmokeModule) -> i32 {
    try_evaluate_exit_code(module).expect("validated native smoke module should evaluate")
}

fn try_evaluate_exit_code(module: &NativeSmokeModule) -> Result<i32, BackendError> {
    match evaluate_native_function_from_module(module, &module.main_name, &[])? {
        NativeSmokeFunctionOutcome::Void => Ok(0),
        NativeSmokeFunctionOutcome::Int(value) => parse_stage_6c_exit_code(value),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeSmokeFunctionOutcome {
    Int(i64),
    Void,
}

fn native_function_map(module: &NativeSmokeModule) -> HashMap<String, NativeSmokeFunction> {
    module
        .functions
        .iter()
        .map(|function| (function.name.clone(), function.clone()))
        .collect()
}

fn evaluate_native_function_from_module(
    module: &NativeSmokeModule,
    name: &str,
    args: &[i64],
) -> Result<NativeSmokeFunctionOutcome, BackendError> {
    let functions = native_function_map(module);
    evaluate_native_function_from_map(&functions, name, args, &mut Vec::new())
}

fn evaluate_native_function_from_context(
    context: &NativeSmokeValidationContext<'_>,
    name: &str,
    args: &[i64],
) -> Result<NativeSmokeFunctionOutcome, BackendError> {
    evaluate_native_function_from_map(&context.validated, name, args, &mut Vec::new())
}

fn evaluate_native_function_from_map(
    functions: &HashMap<String, NativeSmokeFunction>,
    name: &str,
    args: &[i64],
    call_stack: &mut Vec<String>,
) -> Result<NativeSmokeFunctionOutcome, BackendError> {
    if call_stack.iter().any(|active| active == name) {
        return Err(BackendError::new(
            "unsupported native recursive function call for Stage 10",
        ));
    }

    let function = functions.get(name).ok_or_else(|| {
        BackendError::new(format!(
            "backend validation failure: native function `{name}` was not available for evaluation"
        ))
    })?;
    if function.params.len() != args.len() {
        return Err(BackendError::new(format!(
            "backend validation failure: native function `{name}` expected {} evaluated argument(s), got {}",
            function.params.len(),
            args.len()
        )));
    }

    let mut states = HashMap::new();
    for (param, value) in function.params.iter().zip(args.iter().copied()) {
        states.insert(
            param.name.clone(),
            NativeSmokeLocalState {
                writable: param.writable,
                value: NativeSmokeValue::Int(value),
            },
        );
    }

    call_stack.push(name.to_string());
    let result = evaluate_native_block(
        functions,
        &function.body,
        states,
        function.return_type,
        call_stack,
    );
    call_stack.pop();
    result
}

fn evaluate_native_block(
    functions: &HashMap<String, NativeSmokeFunction>,
    block: &NativeSmokeBlock,
    mut local_states: HashMap<String, NativeSmokeLocalState>,
    function_return: NativeSmokeFunctionReturn,
    call_stack: &mut Vec<String>,
) -> Result<NativeSmokeFunctionOutcome, BackendError> {
    evaluate_native_block_statements(functions, &block.statements, &mut local_states, call_stack)?;
    evaluate_native_terminator(
        functions,
        &block.terminator,
        &local_states,
        function_return,
        call_stack,
    )
}

fn evaluate_native_block_statements(
    functions: &HashMap<String, NativeSmokeFunction>,
    statements: &[NativeSmokeStmt],
    local_states: &mut HashMap<String, NativeSmokeLocalState>,
    call_stack: &mut Vec<String>,
) -> Result<(), BackendError> {
    for statement in statements {
        match statement {
            NativeSmokeStmt::Local(local) => {
                let value =
                    evaluate_native_local_value(functions, local, local_states, call_stack)?;
                local_states.insert(
                    local.name.clone(),
                    NativeSmokeLocalState {
                        writable: local.writable,
                        value,
                    },
                );
            }
            NativeSmokeStmt::Assign(assignment) => {
                evaluate_native_assignment(functions, assignment, local_states, call_stack)?;
            }
            NativeSmokeStmt::Echo(_) => {}
            NativeSmokeStmt::Call(call) => {
                evaluate_native_call_statement(functions, call, local_states, call_stack)?;
            }
            NativeSmokeStmt::If(native_if) => {
                let evaluation = evaluate_native_scoped_if_with_functions(
                    functions,
                    native_if,
                    local_states,
                    call_stack,
                )?;
                if evaluation.control != NativeSmokeLoopControl::Fallthrough {
                    return Err(BackendError::new(
                        "backend validation failure: loop control escaped native function block",
                    ));
                }
                *local_states = evaluation.visible_states;
            }
            NativeSmokeStmt::While(native_while) => {
                evaluate_native_while_statement(functions, native_while, local_states, call_stack)?;
            }
            NativeSmokeStmt::For(native_for) => {
                evaluate_native_for_statement(functions, native_for, local_states, call_stack)?;
            }
            NativeSmokeStmt::Break | NativeSmokeStmt::Continue => {
                return Err(BackendError::new(
                    "backend validation failure: loop control escaped native function block",
                ));
            }
        }
    }
    Ok(())
}

fn evaluate_native_local_value(
    functions: &HashMap<String, NativeSmokeFunction>,
    local: &NativeSmokeLocal,
    local_states: &HashMap<String, NativeSmokeLocalState>,
    call_stack: &mut Vec<String>,
) -> Result<NativeSmokeValue, BackendError> {
    match &local.evaluated_value {
        NativeSmokeValue::Int(_) => Ok(NativeSmokeValue::Int(evaluate_native_expr_with_functions(
            &local.expr,
            &native_state_values(local_states),
            functions,
            call_stack,
        )?)),
        NativeSmokeValue::StringLiteral(value) => {
            Ok(NativeSmokeValue::StringLiteral(value.clone()))
        }
    }
}

fn evaluate_native_assignment(
    functions: &HashMap<String, NativeSmokeFunction>,
    assignment: &NativeSmokeAssign,
    local_states: &mut HashMap<String, NativeSmokeLocalState>,
    call_stack: &mut Vec<String>,
) -> Result<(), BackendError> {
    let values = native_state_values(local_states);
    let Some(target) = local_states.get(&assignment.target) else {
        return Err(BackendError::new(
            "backend validation failure: validated native assignment target was not declared",
        ));
    };
    let Some(target_value) = target.value.as_int() else {
        return Err(BackendError::new(
            "backend validation failure: validated native assignment target was not an integer local",
        ));
    };
    let evaluated_value = evaluate_native_assignment_value_with_functions(
        assignment.op,
        target_value,
        &assignment.expr,
        &values,
        functions,
        call_stack,
    )?;
    let Some(target) = local_states.get_mut(&assignment.target) else {
        return Err(BackendError::new(
            "backend validation failure: validated native assignment target was not declared",
        ));
    };
    target.value = NativeSmokeValue::Int(evaluated_value);
    Ok(())
}

fn evaluate_native_call_statement(
    functions: &HashMap<String, NativeSmokeFunction>,
    call: &NativeSmokeCall,
    local_states: &HashMap<String, NativeSmokeLocalState>,
    call_stack: &mut Vec<String>,
) -> Result<(), BackendError> {
    let values = native_state_values(local_states);
    let args = call
        .args
        .iter()
        .map(|arg| evaluate_native_expr_with_functions(arg, &values, functions, call_stack))
        .collect::<Result<Vec<_>, _>>()?;
    match (
        call.return_type,
        evaluate_native_function_from_map(functions, &call.function, &args, call_stack)?,
    ) {
        (NativeSmokeFunctionReturn::Void, NativeSmokeFunctionOutcome::Void) => Ok(()),
        (NativeSmokeFunctionReturn::Int, NativeSmokeFunctionOutcome::Int(_)) => Ok(()),
        _ => Err(BackendError::new(format!(
            "backend validation failure: native function `{}` returned unexpected type",
            call.function
        ))),
    }
}

fn evaluate_native_while_statement(
    functions: &HashMap<String, NativeSmokeFunction>,
    native_while: &NativeSmokeWhile,
    local_states: &mut HashMap<String, NativeSmokeLocalState>,
    call_stack: &mut Vec<String>,
) -> Result<(), BackendError> {
    let mut simulated_states = local_states.clone();
    let mut iterations = 0;
    loop {
        let values = native_state_values(&simulated_states);
        if !evaluate_native_condition_with_functions(
            &native_while.condition,
            &values,
            functions,
            call_stack,
        )? {
            break;
        }
        if iterations == STAGE_7B_LOOP_VERIFICATION_CAP {
            return Err(stage_6c_loop_cap_error());
        }
        let evaluation = evaluate_native_scoped_statements_with_functions(
            functions,
            &native_while.body.statements,
            &simulated_states,
            call_stack,
        )?;
        simulated_states = evaluation.visible_states;
        iterations += 1;
        if evaluation.control == NativeSmokeLoopControl::Break {
            break;
        }
    }
    for (name, _) in &native_while.final_values {
        let Some(state) = simulated_states.get(name).cloned() else {
            return Err(BackendError::new(
                "backend validation failure: validated native while target was not declared",
            ));
        };
        local_states.insert(name.clone(), state);
    }
    Ok(())
}

fn evaluate_native_for_statement(
    functions: &HashMap<String, NativeSmokeFunction>,
    native_for: &NativeSmokeFor,
    local_states: &mut HashMap<String, NativeSmokeLocalState>,
    call_stack: &mut Vec<String>,
) -> Result<(), BackendError> {
    let mut simulated_states = local_states.clone();
    if let Some(initializer) = &native_for.initializer {
        match initializer {
            NativeSmokeForInitializer::Local(local) => {
                let value =
                    evaluate_native_local_value(functions, local, &simulated_states, call_stack)?;
                simulated_states.insert(
                    local.name.clone(),
                    NativeSmokeLocalState {
                        writable: local.writable,
                        value,
                    },
                );
            }
            NativeSmokeForInitializer::Assign(assignment) => {
                evaluate_native_assignment(
                    functions,
                    assignment,
                    &mut simulated_states,
                    call_stack,
                )?;
            }
        }
    }

    let mut iterations = 0;
    loop {
        let values = native_state_values(&simulated_states);
        if !evaluate_native_condition_with_functions(
            &native_for.condition,
            &values,
            functions,
            call_stack,
        )? {
            break;
        }
        if iterations == STAGE_7B_LOOP_VERIFICATION_CAP {
            return Err(stage_9_loop_cap_error());
        }
        let evaluation = evaluate_native_scoped_statements_with_functions(
            functions,
            &native_for.body.statements,
            &simulated_states,
            call_stack,
        )?;
        simulated_states = evaluation.visible_states;
        if evaluation.control == NativeSmokeLoopControl::Break {
            break;
        }
        if let Some(condition) = &native_for.increment_exit_condition {
            let values = native_state_values(&simulated_states);
            if evaluate_native_condition_with_functions(condition, &values, functions, call_stack)?
            {
                break;
            }
        }
        if let Some(increment) = &native_for.increment {
            evaluate_native_assignment(functions, increment, &mut simulated_states, call_stack)?;
        }
        iterations += 1;
    }

    for (name, _) in &native_for.final_values {
        let Some(state) = simulated_states.get(name).cloned() else {
            return Err(BackendError::new(
                "backend validation failure: validated native for target was not declared",
            ));
        };
        local_states.insert(name.clone(), state);
    }
    Ok(())
}

fn evaluate_native_terminator(
    functions: &HashMap<String, NativeSmokeFunction>,
    terminator: &NativeSmokeTerminator,
    local_states: &HashMap<String, NativeSmokeLocalState>,
    function_return: NativeSmokeFunctionReturn,
    call_stack: &mut Vec<String>,
) -> Result<NativeSmokeFunctionOutcome, BackendError> {
    match terminator {
        NativeSmokeTerminator::ExitSuccess
            if function_return == NativeSmokeFunctionReturn::Void =>
        {
            Ok(NativeSmokeFunctionOutcome::Void)
        }
        NativeSmokeTerminator::ExitSuccess => Err(BackendError::new(
            "backend validation failure: int-returning native function fell through",
        )),
        NativeSmokeTerminator::Return { expr, .. } => {
            let value = evaluate_native_expr_with_functions(
                expr,
                &native_state_values(local_states),
                functions,
                call_stack,
            )?;
            Ok(NativeSmokeFunctionOutcome::Int(value))
        }
        NativeSmokeTerminator::IfElse {
            condition,
            then_block,
            else_block,
            ..
        } => {
            if evaluate_native_condition_with_functions(
                condition,
                &native_state_values(local_states),
                functions,
                call_stack,
            )? {
                evaluate_native_block(
                    functions,
                    then_block,
                    local_states.clone(),
                    function_return,
                    call_stack,
                )
            } else {
                evaluate_native_block(
                    functions,
                    else_block,
                    local_states.clone(),
                    function_return,
                    call_stack,
                )
            }
        }
        NativeSmokeTerminator::Guard {
            condition,
            then_block,
            fallback,
            ..
        } => {
            if evaluate_native_condition_with_functions(
                condition,
                &native_state_values(local_states),
                functions,
                call_stack,
            )? {
                evaluate_native_block(
                    functions,
                    then_block,
                    local_states.clone(),
                    function_return,
                    call_stack,
                )
            } else {
                evaluate_native_block(
                    functions,
                    fallback,
                    local_states.clone(),
                    function_return,
                    call_stack,
                )
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
        ObjectBuilder::new(isa, "doria_stage_10", default_libcall_names())
            .map_err(|error| BackendError::new(format!("backend emission failure: {error}")))?,
    );

    let mut function_ids = HashMap::new();
    for function in &native_module.functions {
        let signature = native_function_signature(&mut module, function);
        let function_id = module
            .declare_function(
                &native_function_symbol(&function.name),
                Linkage::Local,
                &signature,
            )
            .map_err(|error| BackendError::new(format!("backend emission failure: {error}")))?;
        function_ids.insert(function.name.clone(), function_id);
    }

    let mut process_signature = module.make_signature();
    process_signature.returns.push(AbiParam::new(types::I32));
    let process_main_id = module
        .declare_function("main", Linkage::Export, &process_signature)
        .map_err(|error| BackendError::new(format!("backend emission failure: {error}")))?;

    for function in &native_module.functions {
        define_native_function(&mut module, function, &function_ids)?;
    }
    define_native_process_main(
        &mut module,
        native_module,
        process_main_id,
        &process_signature,
        &function_ids,
    )?;

    module
        .finish()
        .emit()
        .map_err(|error| BackendError::new(format!("backend emission failure: {error}")))
}

fn native_function_symbol(name: &str) -> String {
    format!("__doria_stage_10_{name}")
}

fn native_function_signature(
    module: &mut ObjectModule,
    function: &NativeSmokeFunction,
) -> Signature {
    let mut signature = module.make_signature();
    for _ in &function.params {
        signature.params.push(AbiParam::new(types::I64));
    }
    if function.return_type == NativeSmokeFunctionReturn::Int {
        signature.returns.push(AbiParam::new(types::I64));
    }
    signature
}

fn define_native_function(
    module: &mut ObjectModule,
    function: &NativeSmokeFunction,
    function_ids: &HashMap<String, FuncId>,
) -> Result<(), BackendError> {
    let function_id = function_ids.get(&function.name).copied().ok_or_else(|| {
        BackendError::new(format!(
            "backend emission failure: native function `{}` was not declared",
            function.name
        ))
    })?;
    let signature = native_function_signature(module, function);
    let mut context = module.make_context();
    context.func.signature = signature;
    let mut function_context = FunctionBuilderContext::new();
    {
        let mut builder = FunctionBuilder::new(&mut context.func, &mut function_context);
        let entry_block = builder.create_block();
        builder.append_block_params_for_function_params(entry_block);
        builder.switch_to_block(entry_block);
        builder.seal_block(entry_block);

        let mut lowered_local_values = HashMap::new();
        let mut evaluated_local_values = HashMap::new();
        for (index, param) in function.params.iter().enumerate() {
            lowered_local_values
                .insert(param.name.clone(), builder.block_params(entry_block)[index]);
            evaluated_local_values.insert(param.name.clone(), 0);
        }

        let mut resources = NativeSmokeLoweringResources::new(
            module,
            function_ids.clone(),
            native_function_symbol(&function.name),
        );
        lower_native_block(
            &mut builder,
            &function.body,
            function.return_type,
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
    Ok(())
}

fn define_native_process_main(
    module: &mut ObjectModule,
    native_module: &NativeSmokeModule,
    process_main_id: FuncId,
    process_signature: &Signature,
    function_ids: &HashMap<String, FuncId>,
) -> Result<(), BackendError> {
    let doria_main = native_module
        .functions
        .iter()
        .find(|function| function.name == native_module.main_name)
        .ok_or_else(|| BackendError::new("backend emission failure: native main was not found"))?;
    let doria_main_id = function_ids
        .get(&native_module.main_name)
        .copied()
        .ok_or_else(|| {
            BackendError::new("backend emission failure: native main was not declared")
        })?;

    let mut context = module.make_context();
    context.func.signature = process_signature.clone();
    let mut function_context = FunctionBuilderContext::new();
    {
        let mut builder = FunctionBuilder::new(&mut context.func, &mut function_context);
        let entry_block = builder.create_block();
        builder.switch_to_block(entry_block);
        builder.seal_block(entry_block);
        let resources = NativeSmokeLoweringResources::new(
            module,
            function_ids.clone(),
            "process_main".to_string(),
        );
        let callee = resources
            .module
            .declare_func_in_func(doria_main_id, builder.func);
        let call = builder.ins().call(callee, &[]);
        match doria_main.return_type {
            NativeSmokeFunctionReturn::Int => {
                let value = builder.inst_results(call)[0];
                let exit_value = builder.ins().ireduce(types::I32, value);
                builder.ins().return_(&[exit_value]);
            }
            NativeSmokeFunctionReturn::Void => {
                lower_native_success_return(&mut builder);
            }
        }
        builder.finalize();
    }

    module
        .define_function(process_main_id, &mut context)
        .map_err(|error| BackendError::new(format!("backend emission failure: {error}")))?;
    module.clear_context(&mut context);
    Ok(())
}

fn lower_native_block(
    builder: &mut FunctionBuilder,
    block: &NativeSmokeBlock,
    function_return: NativeSmokeFunctionReturn,
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
        function_return,
        resources,
        lowered_local_values,
        evaluated_local_values,
    )
}

fn lower_native_terminator(
    builder: &mut FunctionBuilder,
    terminator: &NativeSmokeTerminator,
    function_return: NativeSmokeFunctionReturn,
    resources: &mut NativeSmokeLoweringResources<'_>,
    lowered_local_values: &HashMap<String, Value>,
    evaluated_local_values: &HashMap<String, i64>,
) -> Result<(), BackendError> {
    match terminator {
        NativeSmokeTerminator::ExitSuccess
            if function_return == NativeSmokeFunctionReturn::Void =>
        {
            builder.ins().return_(&[]);
            Ok(())
        }
        NativeSmokeTerminator::ExitSuccess => Err(BackendError::new(
            "backend emission failure: int-returning native function fell through",
        )),
        NativeSmokeTerminator::Return {
            expr,
            evaluated_value,
        } => lower_native_return(
            builder,
            expr,
            *evaluated_value,
            resources,
            lowered_local_values,
            evaluated_local_values,
        ),
        NativeSmokeTerminator::IfElse {
            condition,
            evaluated_condition,
            then_block,
            else_block,
        } => {
            let _validated_condition = evaluated_condition;

            let then_ir_block = builder.create_block();
            let else_ir_block = builder.create_block();
            lower_native_condition_branch(
                builder,
                condition,
                then_ir_block,
                else_ir_block,
                lowered_local_values,
                resources,
            )?;

            builder.switch_to_block(then_ir_block);
            builder.seal_block(then_ir_block);
            let mut then_lowered_local_values = lowered_local_values.clone();
            let mut then_evaluated_local_values = evaluated_local_values.clone();
            lower_native_block(
                builder,
                then_block,
                function_return,
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
                function_return,
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
            let _validated_condition = evaluated_condition;

            let then_ir_block = builder.create_block();
            let fallback_ir_block = builder.create_block();
            lower_native_condition_branch(
                builder,
                condition,
                then_ir_block,
                fallback_ir_block,
                lowered_local_values,
                resources,
            )?;

            builder.switch_to_block(then_ir_block);
            builder.seal_block(then_ir_block);
            let mut then_lowered_local_values = lowered_local_values.clone();
            let mut then_evaluated_local_values = evaluated_local_values.clone();
            lower_native_block(
                builder,
                then_block,
                function_return,
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
                function_return,
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
            match &local.evaluated_value {
                NativeSmokeValue::Int(value) => {
                    let lowered_value =
                        lower_native_expr(builder, &local.expr, lowered_local_values, resources)?;
                    lowered_local_values.insert(local.name.clone(), lowered_value);
                    evaluated_local_values.insert(local.name.clone(), *value);
                }
                NativeSmokeValue::StringLiteral(_) => {
                    lowered_local_values.remove(&local.name);
                    evaluated_local_values.remove(&local.name);
                }
            }
            Ok(NativeSmokeLoweringFlow::Fallthrough)
        }
        NativeSmokeStmt::Assign(assignment) => {
            let value =
                lower_native_assignment(builder, assignment, lowered_local_values, resources)?;
            lowered_local_values.insert(assignment.target.clone(), value);
            evaluated_local_values.insert(assignment.target.clone(), assignment.evaluated_value);
            Ok(NativeSmokeLoweringFlow::Fallthrough)
        }
        NativeSmokeStmt::Echo(expr) => {
            lower_native_echo_string_expr(builder, expr, resources)?;
            Ok(NativeSmokeLoweringFlow::Fallthrough)
        }
        NativeSmokeStmt::Call(call) => {
            lower_native_call_statement(builder, call, resources, lowered_local_values)?;
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
        NativeSmokeStmt::For(native_for) => lower_native_for(
            builder,
            native_for,
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
                    "unsupported native continue for Stage 7b: expected enclosing loop",
                ));
            };
            jump_to_native_carried_block(
                builder,
                loop_context.continue_block,
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
        resources,
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
    let mut param_index = 0;
    for (name, value) in &native_if.merged_values {
        if let Some(value) = value.as_int() {
            lowered_local_values.insert(
                name.clone(),
                builder.block_params(merge_ir_block)[param_index],
            );
            evaluated_local_values.insert(name.clone(), value);
            param_index += 1;
        } else {
            lowered_local_values.remove(name);
            evaluated_local_values.remove(name);
        }
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
    for (_, value) in &native_if.merged_values {
        if value.as_int().is_some() {
            builder.append_block_param(block, types::I64);
        }
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
            NativeSmokeStmt::For(_) => {
                return Err(BackendError::new(
                    "unsupported native loop body statement for Stage 9: nested for/range loops are future native work",
                ));
            }
            NativeSmokeStmt::Echo(_) | NativeSmokeStmt::Call(_) => {
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
    merged_values: &[(String, NativeSmokeValue)],
    local_values: &HashMap<String, Value>,
) -> Result<(), BackendError> {
    let args = merged_values
        .iter()
        .filter(|(_, value)| value.as_int().is_some())
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
    carried_values: &[(String, NativeSmokeValue)],
    local_values: &HashMap<String, Value>,
) -> Result<(), BackendError> {
    let args = carried_values
        .iter()
        .filter(|(_, value)| value.as_int().is_some())
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

    for (_, value) in &native_while.final_values {
        if value.as_int().is_some() {
            builder.append_block_param(loop_header, types::I64);
            builder.append_block_param(loop_after, types::I64);
        }
    }

    let initial_args = native_while
        .final_values
        .iter()
        .filter(|(_, value)| value.as_int().is_some())
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
    let mut param_index = 0;
    for (name, value) in &native_while.final_values {
        if value.as_int().is_some() {
            header_local_values
                .insert(name.clone(), builder.block_params(loop_header)[param_index]);
            param_index += 1;
        }
    }
    let after_args = native_while
        .final_values
        .iter()
        .filter(|(_, value)| value.as_int().is_some())
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
        NativeSmokeBranchTarget {
            block: loop_body,
            args: &[],
        },
        NativeSmokeBranchTarget {
            block: loop_after,
            args: &after_args,
        },
        &header_local_values,
        resources,
    )?;

    builder.switch_to_block(loop_body);
    builder.seal_block(loop_body);
    let mut body_local_values = header_local_values.clone();
    let mut body_evaluated_values = evaluated_local_values.clone();
    let loop_context = NativeSmokeLoopLoweringContext {
        continue_block: loop_header,
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
            .filter(|(_, value)| value.as_int().is_some())
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
    let mut param_index = 0;
    for (name, value) in &native_while.final_values {
        if let Some(value) = value.as_int() {
            lowered_local_values
                .insert(name.clone(), builder.block_params(loop_after)[param_index]);
            evaluated_local_values.insert(name.clone(), value);
            param_index += 1;
        } else {
            lowered_local_values.remove(name);
            evaluated_local_values.remove(name);
        }
    }

    Ok(())
}

fn lower_native_for(
    builder: &mut FunctionBuilder,
    native_for: &NativeSmokeFor,
    resources: &mut NativeSmokeLoweringResources<'_>,
    lowered_local_values: &mut HashMap<String, Value>,
    evaluated_local_values: &mut HashMap<String, i64>,
) -> Result<(), BackendError> {
    debug_assert!(native_for.evaluated_iterations <= STAGE_7B_LOOP_VERIFICATION_CAP);

    if let Some(initializer) = &native_for.initializer {
        lower_native_for_initializer(
            builder,
            initializer,
            resources,
            lowered_local_values,
            evaluated_local_values,
        )?;
    }

    let loop_header = builder.create_block();
    let loop_body = builder.create_block();
    let loop_increment = builder.create_block();
    let loop_increment_apply = native_for
        .increment_exit_condition
        .as_ref()
        .map(|_| builder.create_block());
    let loop_after = builder.create_block();

    for (_, value) in &native_for.carried_values {
        if value.as_int().is_some() {
            builder.append_block_param(loop_header, types::I64);
            builder.append_block_param(loop_increment, types::I64);
            builder.append_block_param(loop_after, types::I64);
        }
    }

    let initial_args = native_for
        .carried_values
        .iter()
        .filter(|(_, value)| value.as_int().is_some())
        .map(|(name, _)| {
            lowered_local_values
                .get(name)
                .copied()
                .map(BlockArg::Value)
                .ok_or_else(|| {
                    BackendError::new(format!(
                        "backend emission failure: validated native for carried local `{name}` was not lowered"
                    ))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    builder.ins().jump(loop_header, &initial_args);

    builder.switch_to_block(loop_header);
    let mut header_local_values = lowered_local_values.clone();
    let mut param_index = 0;
    for (name, value) in &native_for.carried_values {
        if value.as_int().is_some() {
            header_local_values
                .insert(name.clone(), builder.block_params(loop_header)[param_index]);
            param_index += 1;
        }
    }
    let after_args = native_for
        .carried_values
        .iter()
        .filter(|(_, value)| value.as_int().is_some())
        .map(|(name, _)| {
            header_local_values
                .get(name)
                .copied()
                .map(BlockArg::Value)
                .ok_or_else(|| {
                    BackendError::new(format!(
                        "backend emission failure: validated native for carried local `{name}` was not lowered"
                    ))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    lower_native_condition_branch_with_args(
        builder,
        &native_for.condition,
        NativeSmokeBranchTarget {
            block: loop_body,
            args: &[],
        },
        NativeSmokeBranchTarget {
            block: loop_after,
            args: &after_args,
        },
        &header_local_values,
        resources,
    )?;

    builder.switch_to_block(loop_body);
    builder.seal_block(loop_body);
    let mut body_local_values = header_local_values.clone();
    let mut body_evaluated_values = evaluated_local_values.clone();
    let loop_context = NativeSmokeLoopLoweringContext {
        continue_block: loop_increment,
        after_block: loop_after,
        carried_locals: &native_for.carried_values,
    };
    let visible_body_values = lower_native_fallthrough_block(
        builder,
        &native_for.body,
        resources,
        &mut body_local_values,
        &mut body_evaluated_values,
        &header_local_values,
        Some(&loop_context),
    )?;
    if let Some(visible_body_values) = visible_body_values {
        jump_to_native_carried_block(
            builder,
            loop_increment,
            &native_for.carried_values,
            &visible_body_values,
        )?;
    }

    builder.switch_to_block(loop_increment);
    builder.seal_block(loop_increment);
    let mut increment_local_values = lowered_local_values.clone();
    let mut param_index = 0;
    for (name, value) in &native_for.carried_values {
        if value.as_int().is_some() {
            increment_local_values.insert(
                name.clone(),
                builder.block_params(loop_increment)[param_index],
            );
            param_index += 1;
        }
    }
    if let Some(condition) = &native_for.increment_exit_condition {
        let Some(loop_increment_apply) = loop_increment_apply else {
            return Err(BackendError::new(
                "backend emission failure: validated native for increment guard block was not created",
            ));
        };
        let after_args = native_for
            .carried_values
            .iter()
            .filter(|(_, value)| value.as_int().is_some())
            .map(|(name, _)| {
                increment_local_values
                    .get(name)
                    .copied()
                    .map(BlockArg::Value)
                    .ok_or_else(|| {
                        BackendError::new(format!(
                            "backend emission failure: validated native for carried local `{name}` was not lowered"
                        ))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        lower_native_condition_branch_with_args(
            builder,
            condition,
            NativeSmokeBranchTarget {
                block: loop_after,
                args: &after_args,
            },
            NativeSmokeBranchTarget {
                block: loop_increment_apply,
                args: &[],
            },
            &increment_local_values,
            resources,
        )?;
        builder.switch_to_block(loop_increment_apply);
        builder.seal_block(loop_increment_apply);
    }
    if let Some(increment) = &native_for.increment {
        let value =
            lower_native_assignment(builder, increment, &increment_local_values, resources)?;
        increment_local_values.insert(increment.target.clone(), value);
    }
    jump_to_native_carried_block(
        builder,
        loop_header,
        &native_for.carried_values,
        &increment_local_values,
    )?;
    builder.seal_block(loop_header);

    builder.switch_to_block(loop_after);
    builder.seal_block(loop_after);
    let mut param_index = 0;
    let final_names = native_for
        .final_values
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<HashSet<_>>();
    for (name, value) in &native_for.carried_values {
        if value.as_int().is_some() {
            if final_names.contains(name.as_str()) {
                lowered_local_values
                    .insert(name.clone(), builder.block_params(loop_after)[param_index]);
            } else {
                lowered_local_values.remove(name);
            }
            param_index += 1;
        } else {
            lowered_local_values.remove(name);
        }
    }
    for (name, value) in &native_for.final_values {
        if let Some(value) = value.as_int() {
            evaluated_local_values.insert(name.clone(), value);
        } else {
            lowered_local_values.remove(name);
            evaluated_local_values.remove(name);
        }
    }

    Ok(())
}

fn lower_native_for_initializer(
    builder: &mut FunctionBuilder,
    initializer: &NativeSmokeForInitializer,
    resources: &mut NativeSmokeLoweringResources<'_>,
    lowered_local_values: &mut HashMap<String, Value>,
    evaluated_local_values: &mut HashMap<String, i64>,
) -> Result<(), BackendError> {
    match initializer {
        NativeSmokeForInitializer::Local(local) => {
            lower_native_statement(
                builder,
                &NativeSmokeStmt::Local(local.clone()),
                resources,
                lowered_local_values,
                evaluated_local_values,
                None,
                None,
            )?;
        }
        NativeSmokeForInitializer::Assign(assignment) => {
            let value =
                lower_native_assignment(builder, assignment, lowered_local_values, resources)?;
            lowered_local_values.insert(assignment.target.clone(), value);
            evaluated_local_values.insert(assignment.target.clone(), assignment.evaluated_value);
        }
    }
    Ok(())
}

fn lower_native_call_statement(
    builder: &mut FunctionBuilder,
    call: &NativeSmokeCall,
    resources: &mut NativeSmokeLoweringResources<'_>,
    local_values: &HashMap<String, Value>,
) -> Result<(), BackendError> {
    let args = call
        .args
        .iter()
        .map(|arg| lower_native_expr(builder, arg, local_values, resources))
        .collect::<Result<Vec<_>, _>>()?;
    let function_id = resources
        .function_ids
        .get(&call.function)
        .copied()
        .ok_or_else(|| {
            BackendError::new(format!(
                "backend emission failure: native function `{}` was not declared",
                call.function
            ))
        })?;
    let callee = resources
        .module
        .declare_func_in_func(function_id, builder.func);
    builder.ins().call(callee, &args);
    Ok(())
}

fn lower_native_call_expr(
    builder: &mut FunctionBuilder,
    function: &str,
    args: &[NativeSmokeExpr],
    local_values: &HashMap<String, Value>,
    resources: &mut NativeSmokeLoweringResources<'_>,
) -> Result<Value, BackendError> {
    let args = args
        .iter()
        .map(|arg| lower_native_expr(builder, arg, local_values, resources))
        .collect::<Result<Vec<_>, _>>()?;
    let function_id = resources
        .function_ids
        .get(function)
        .copied()
        .ok_or_else(|| {
            BackendError::new(format!(
                "backend emission failure: native function `{function}` was not declared"
            ))
        })?;
    let callee = resources
        .module
        .declare_func_in_func(function_id, builder.func);
    let call = builder.ins().call(callee, &args);
    builder.inst_results(call).first().copied().ok_or_else(|| {
        BackendError::new(format!(
            "backend emission failure: native function `{function}` did not return an integer value"
        ))
    })
}

fn lower_native_assignment(
    builder: &mut FunctionBuilder,
    assignment: &NativeSmokeAssign,
    local_values: &HashMap<String, Value>,
    resources: &mut NativeSmokeLoweringResources<'_>,
) -> Result<Value, BackendError> {
    let right = lower_native_expr(builder, &assignment.expr, local_values, resources)?;
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

fn lower_native_error_return(builder: &mut FunctionBuilder) {
    builder.ins().trap(TrapCode::unwrap_user(1));
}

fn lower_native_echo_string_expr(
    builder: &mut FunctionBuilder,
    expr: &NativeSmokeExpr,
    resources: &mut NativeSmokeLoweringResources<'_>,
) -> Result<(), BackendError> {
    let NativeSmokeExpr::StringLiteral(value) = expr else {
        return Err(BackendError::new(
            "backend emission failure: validated native echo expression was not a string literal",
        ));
    };
    lower_native_echo_string_literal(builder, value, resources)
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
    let data_name = format!(
        "__doria_stage_7b_echo_{}_{}",
        resources.string_literal_namespace, resources.next_string_literal_id
    );
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

    let loop_block = builder.create_block();
    let write_block = builder.create_block();
    let advance_block = builder.create_block();
    let error_block = builder.create_block();
    let done_block = builder.create_block();
    builder.append_block_param(loop_block, pointer_type);
    builder.append_block_param(loop_block, pointer_type);

    let zero = builder.ins().iconst(pointer_type, 0);
    let byte_count = builder.ins().iconst(pointer_type, value.len() as i64);
    builder.ins().jump(
        loop_block,
        &[BlockArg::Value(zero), BlockArg::Value(byte_count)],
    );

    builder.switch_to_block(loop_block);
    let offset = builder.block_params(loop_block)[0];
    let remaining = builder.block_params(loop_block)[1];
    let is_done = builder.ins().icmp_imm(IntCC::Equal, remaining, 0);
    builder
        .ins()
        .brif(is_done, done_block, &[], write_block, &[]);

    builder.switch_to_block(write_block);
    builder.seal_block(write_block);
    let current_pointer = builder.ins().iadd(data_pointer, offset);
    let write_call = builder
        .ins()
        .call(write_function, &[fd, current_pointer, remaining]);
    let written = builder.inst_results(write_call)[0];
    let made_progress = builder.ins().icmp_imm(IntCC::SignedGreaterThan, written, 0);
    builder
        .ins()
        .brif(made_progress, advance_block, &[], error_block, &[]);

    builder.switch_to_block(advance_block);
    builder.seal_block(advance_block);
    let next_offset = builder.ins().iadd(offset, written);
    let next_remaining = builder.ins().isub(remaining, written);
    builder.ins().jump(
        loop_block,
        &[
            BlockArg::Value(next_offset),
            BlockArg::Value(next_remaining),
        ],
    );
    builder.seal_block(loop_block);

    builder.switch_to_block(error_block);
    builder.seal_block(error_block);
    lower_native_error_return(builder);

    builder.switch_to_block(done_block);
    builder.seal_block(done_block);

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
    let byte_count = i64::try_from(value.len())
        .ok()
        .and_then(|len| i32::try_from(len).ok())
        .ok_or_else(|| {
            BackendError::new(
                "unsupported native echo statement for Stage 7b: Windows string-literal stdout smoke path supports literals up to 2147483647 bytes",
            )
        })?;
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

    let loop_block = builder.create_block();
    let write_block = builder.create_block();
    let check_written_block = builder.create_block();
    let advance_block = builder.create_block();
    let error_block = builder.create_block();
    let done_block = builder.create_block();
    builder.append_block_param(loop_block, pointer_type);
    builder.append_block_param(loop_block, types::I32);

    let offset_zero = builder.ins().iconst(pointer_type, 0);
    let remaining_count = builder.ins().iconst(types::I32, i64::from(byte_count));
    builder.ins().jump(
        loop_block,
        &[
            BlockArg::Value(offset_zero),
            BlockArg::Value(remaining_count),
        ],
    );

    builder.switch_to_block(loop_block);
    let offset = builder.block_params(loop_block)[0];
    let remaining = builder.block_params(loop_block)[1];
    let is_done = builder.ins().icmp_imm(IntCC::Equal, remaining, 0);
    builder
        .ins()
        .brif(is_done, done_block, &[], write_block, &[]);

    builder.switch_to_block(write_block);
    builder.seal_block(write_block);
    builder.ins().stack_store(zero, written_slot, 0);
    let current_pointer = builder.ins().iadd(data_pointer, offset);
    let write_call = builder.ins().call(
        write_file,
        &[
            handle,
            current_pointer,
            remaining,
            written_pointer,
            overlapped_pointer,
        ],
    );
    let write_ok = builder.inst_results(write_call)[0];
    let succeeded = builder.ins().icmp_imm(IntCC::NotEqual, write_ok, 0);
    builder
        .ins()
        .brif(succeeded, check_written_block, &[], error_block, &[]);

    builder.switch_to_block(check_written_block);
    builder.seal_block(check_written_block);
    let written = builder.ins().stack_load(types::I32, written_slot, 0);
    let made_progress = builder.ins().icmp_imm(IntCC::NotEqual, written, 0);
    builder
        .ins()
        .brif(made_progress, advance_block, &[], error_block, &[]);

    builder.switch_to_block(advance_block);
    builder.seal_block(advance_block);
    let written_offset = builder.ins().uextend(pointer_type, written);
    let next_offset = builder.ins().iadd(offset, written_offset);
    let next_remaining = builder.ins().isub(remaining, written);
    builder.ins().jump(
        loop_block,
        &[
            BlockArg::Value(next_offset),
            BlockArg::Value(next_remaining),
        ],
    );
    builder.seal_block(loop_block);

    builder.switch_to_block(error_block);
    builder.seal_block(error_block);
    lower_native_error_return(builder);

    builder.switch_to_block(done_block);
    builder.seal_block(done_block);

    Ok(())
}

fn lower_native_return(
    builder: &mut FunctionBuilder,
    expr: &NativeSmokeExpr,
    _evaluated_value: i64,
    resources: &mut NativeSmokeLoweringResources<'_>,
    lowered_local_values: &HashMap<String, Value>,
    _evaluated_local_values: &HashMap<String, i64>,
) -> Result<(), BackendError> {
    let return_value = lower_native_expr(builder, expr, lowered_local_values, resources)?;
    builder.ins().return_(&[return_value]);
    Ok(())
}

fn lower_native_expr(
    builder: &mut FunctionBuilder,
    expr: &NativeSmokeExpr,
    local_values: &HashMap<String, Value>,
    resources: &mut NativeSmokeLoweringResources<'_>,
) -> Result<Value, BackendError> {
    match expr {
        NativeSmokeExpr::Int(value) => Ok(builder.ins().iconst(types::I64, *value)),
        NativeSmokeExpr::Local(name) => local_values.get(name).copied().ok_or_else(|| {
            BackendError::new(format!(
                "backend emission failure: validated native local `{name}` was not lowered"
            ))
        }),
        NativeSmokeExpr::StringLiteral(_) => Err(BackendError::new(
            "backend emission failure: validated native string expression reached integer lowering",
        )),
        NativeSmokeExpr::Call { function, args } => {
            lower_native_call_expr(builder, function, args, local_values, resources)
        }
        NativeSmokeExpr::Binary { op, left, right } => {
            let left = lower_native_expr(builder, left, local_values, resources)?;
            let right = lower_native_expr(builder, right, local_values, resources)?;
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
    resources: &mut NativeSmokeLoweringResources<'_>,
) -> Result<(), BackendError> {
    lower_native_condition_branch_with_args(
        builder,
        condition,
        NativeSmokeBranchTarget {
            block: then_block,
            args: &[],
        },
        NativeSmokeBranchTarget {
            block: else_block,
            args: &[],
        },
        local_values,
        resources,
    )
}

fn lower_native_condition_branch_with_args(
    builder: &mut FunctionBuilder,
    condition: &NativeSmokeCondition,
    then_target: NativeSmokeBranchTarget<'_>,
    else_target: NativeSmokeBranchTarget<'_>,
    local_values: &HashMap<String, Value>,
    resources: &mut NativeSmokeLoweringResources<'_>,
) -> Result<(), BackendError> {
    match condition {
        NativeSmokeCondition::Bool(value) => {
            let value = builder.ins().iconst(types::I8, i64::from(*value));
            builder.ins().brif(
                value,
                then_target.block,
                then_target.args,
                else_target.block,
                else_target.args,
            );
            Ok(())
        }
        NativeSmokeCondition::Compare { op, left, right } => {
            let left = lower_native_expr(builder, left, local_values, resources)?;
            let right = lower_native_expr(builder, right, local_values, resources)?;
            let condition = builder.ins().icmp(native_compare_intcc(*op), left, right);
            builder.ins().brif(
                condition,
                then_target.block,
                then_target.args,
                else_target.block,
                else_target.args,
            );
            Ok(())
        }
        NativeSmokeCondition::Not(condition) => lower_native_condition_branch_with_args(
            builder,
            condition,
            else_target,
            then_target,
            local_values,
            resources,
        ),
        NativeSmokeCondition::And { left, right } => {
            let right_block = builder.create_block();
            lower_native_condition_branch_with_args(
                builder,
                left,
                NativeSmokeBranchTarget {
                    block: right_block,
                    args: &[],
                },
                else_target,
                local_values,
                resources,
            )?;

            builder.switch_to_block(right_block);
            builder.seal_block(right_block);
            lower_native_condition_branch_with_args(
                builder,
                right,
                then_target,
                else_target,
                local_values,
                resources,
            )
        }
        NativeSmokeCondition::Or { left, right } => {
            let right_block = builder.create_block();
            lower_native_condition_branch_with_args(
                builder,
                left,
                then_target,
                NativeSmokeBranchTarget {
                    block: right_block,
                    args: &[],
                },
                local_values,
                resources,
            )?;

            builder.switch_to_block(right_block);
            builder.seal_block(right_block);
            lower_native_condition_branch_with_args(
                builder,
                right,
                then_target,
                else_target,
                local_values,
                resources,
            )
        }
        NativeSmokeCondition::Xor { left, right } => {
            let left = lower_native_condition_value(builder, left, local_values, resources)?;
            let right = lower_native_condition_value(builder, right, local_values, resources)?;
            let condition = builder.ins().icmp(IntCC::NotEqual, left, right);
            builder.ins().brif(
                condition,
                then_target.block,
                then_target.args,
                else_target.block,
                else_target.args,
            );
            Ok(())
        }
    }
}

fn lower_native_condition_value(
    builder: &mut FunctionBuilder,
    condition: &NativeSmokeCondition,
    local_values: &HashMap<String, Value>,
    resources: &mut NativeSmokeLoweringResources<'_>,
) -> Result<Value, BackendError> {
    let true_block = builder.create_block();
    let false_block = builder.create_block();
    let done_block = builder.create_block();
    builder.append_block_param(done_block, types::I8);

    lower_native_condition_branch(
        builder,
        condition,
        true_block,
        false_block,
        local_values,
        resources,
    )?;

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

fn describe_statement(statement: &Stmt) -> &'static str {
    match statement {
        Stmt::VarDecl(_) => "local variable declaration",
        Stmt::Assignment(_) => "assignment",
        Stmt::Echo { .. } => "echo statement",
        Stmt::Return { .. } => "return statement",
        Stmt::If(_) => "if statement",
        Stmt::While(_) => "while statement",
        Stmt::For(_) => "for statement",
        Stmt::Break { .. } => "break statement",
        Stmt::Continue { .. } => "continue statement",
        Stmt::Foreach(_) => "foreach statement",
        Stmt::Increment(_) => "increment statement",
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
        Expr::Range { .. } => "range expression",
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

    fn main_function(module: &NativeSmokeModule) -> &NativeSmokeFunction {
        module
            .functions
            .iter()
            .find(|function| function.name == module.main_name)
            .expect("validated native module should contain main")
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
            main_function(&module).body.statements.as_slice(),
            [NativeSmokeStmt::Echo(NativeSmokeExpr::StringLiteral(value))] if value == "Hello Doria!"
        ));
        assert_eq!(
            main_function(&module).body.terminator,
            NativeSmokeTerminator::ExitSuccess
        );
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
            main_function(&module).body.terminator,
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
            main_function(&module).body.statements.as_slice(),
            [
                NativeSmokeStmt::Local(_),
                NativeSmokeStmt::While(NativeSmokeWhile { .. })
            ]
        ));
        assert!(matches!(
            main_function(&module).body.terminator,
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
    fn validation_uses_actual_helper_arguments_for_loop_proofs() {
        let module = validate_test_source(
            r#"
function prove(writable int $n): int
{
    while ($n == 0) {
        $n = $n;
    }

    return 42;
}

function main(): int
{
    return prove(1);
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

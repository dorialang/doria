use std::collections::HashMap;

use crate::diagnostics::{Diagnostic, DiagnosticResult};
use crate::source::Span;
use crate::{hir, mir};

#[derive(Clone, Copy)]
struct FunctionSignature {
    id: mir::FunctionId,
    return_type: mir::ReturnType,
    parameter_count: usize,
}

pub fn lower_program(program: &hir::Program) -> DiagnosticResult<mir::Program> {
    let mut declarations = Vec::new();

    for item in &program.items {
        match item {
            hir::Item::Function(function) => declarations.push(function),
            hir::Item::Class(class_decl) => {
                return Err(vec![unsupported(
                    class_decl.span,
                    "classes are not lowered to MIR in Stage 11g",
                )]);
            }
            hir::Item::Statement(statement) => {
                return Err(vec![unsupported(
                    stmt_span(statement),
                    "top-level statements are not lowered to MIR in Stage 11g",
                )]);
            }
        }
    }

    let main_indices = declarations
        .iter()
        .enumerate()
        .filter_map(|(index, function)| (function.name == "main").then_some(index))
        .collect::<Vec<_>>();
    if main_indices.len() != 1 {
        let span = main_indices
            .get(1)
            .map_or_else(Span::default, |index| declarations[*index].span);
        return Err(vec![unsupported(
            span,
            "Stage 11g requires exactly one top-level function main",
        )]);
    }

    let mut signatures = HashMap::new();
    for (index, function) in declarations.iter().enumerate() {
        if signatures.contains_key(&function.name) {
            return Err(vec![unsupported(
                function.span,
                format!(
                    "duplicate top-level function `{}` is not lowered to MIR",
                    function.name
                ),
            )]);
        }
        let signature = collect_function_signature(function, mir::FunctionId(index))?;
        signatures.insert(function.name.clone(), signature);
    }

    let entry = signatures
        .get("main")
        .expect("exactly one collected main signature")
        .id;
    let spans = declarations
        .iter()
        .map(|function| function.span)
        .collect::<Vec<_>>();
    let functions = declarations
        .iter()
        .map(|function| {
            let signature = signatures
                .get(&function.name)
                .copied()
                .expect("every function signature must be collected");
            lower_function(function, signature, &signatures)
        })
        .collect::<Result<Vec<_>, _>>()?;

    validate_call_graph(&functions, &spans)?;

    Ok(mir::Program { functions, entry })
}

fn collect_function_signature(
    function: &hir::FunctionDecl,
    id: mir::FunctionId,
) -> DiagnosticResult<FunctionSignature> {
    let return_type = match function.return_type.as_ref() {
        Some(ty) if is_plain_type(ty, "int") => mir::ReturnType::Int,
        Some(ty) if is_plain_type(ty, "void") => mir::ReturnType::Void,
        Some(ty) => {
            return Err(vec![unsupported(
                function.span,
                format!(
                    "function `{}` has unsupported return type `{ty}`; Stage 11g supports only int and void returns",
                    function.name
                ),
            )]);
        }
        None => {
            return Err(vec![unsupported(
                function.span,
                format!(
                    "function `{}` requires an explicit int or void return type for MIR Stage 11g",
                    function.name
                ),
            )]);
        }
    };

    if function.name == "main" && !function.params.is_empty() {
        return Err(vec![unsupported(
            function.params[0].span,
            "main parameters are not lowered to MIR in Stage 11g",
        )]);
    }

    for param in &function.params {
        if param.default.is_some() {
            return Err(vec![unsupported(
                param.span,
                format!(
                    "default arguments are not lowered for function `{}` in MIR Stage 11g",
                    function.name
                ),
            )]);
        }
        if !is_plain_type(&param.ty, "int") {
            return Err(vec![unsupported(
                param.span,
                format!(
                    "function `{}` has unsupported parameter type `{}`; Stage 11g supports only int parameters",
                    function.name, param.ty
                ),
            )]);
        }
    }

    Ok(FunctionSignature {
        id,
        return_type,
        parameter_count: function.params.len(),
    })
}

fn is_plain_type(ty: &crate::types::TypeRef, name: &str) -> bool {
    ty.name == name && ty.args.is_empty()
}

fn lower_function(
    function: &hir::FunctionDecl,
    signature: FunctionSignature,
    signatures: &HashMap<String, FunctionSignature>,
) -> DiagnosticResult<mir::Function> {
    let mut context = LoweringContext::new(signatures.clone());
    let params = function
        .params
        .iter()
        .map(|param| context.declare_user_local(&param.name, param.writable, mir::Type::Int))
        .collect::<Vec<_>>();

    lower_function_body(
        &function.body,
        &function.name,
        signature.return_type,
        &mut context,
    )?;
    let (locals, blocks) = context.finish();

    Ok(mir::Function {
        id: signature.id,
        name: function.name.clone(),
        params,
        return_type: signature.return_type,
        locals,
        blocks,
        entry_block: mir::BlockId(0),
    })
}

fn lower_function_body(
    body: &hir::Block,
    function_name: &str,
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    lower_statement_sequence(&body.statements, return_type, context)?;

    if context.current_block.is_some() {
        if return_type == mir::ReturnType::Void {
            context.terminate_current(mir::Terminator::ReturnVoid);
        } else {
            return Err(vec![unsupported(
                body.span,
                format!(
                    "function `{function_name}` returning int may not fall through in MIR Stage 11g"
                ),
            )]);
        }
    }

    Ok(())
}

fn lower_statement_sequence(
    statements: &[hir::Stmt],
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    for statement in statements {
        if context.current_block.is_none() {
            return Err(vec![unsupported(
                stmt_span(statement),
                "statements after terminating control flow are not lowered to MIR in Stage 11g",
            )]);
        }

        match statement {
            hir::Stmt::Echo { expr, span } => {
                if return_type != mir::ReturnType::Void {
                    return Err(vec![unsupported(
                        *span,
                        "string echo is only lowered inside void functions in Stage 11g",
                    )]);
                }
                let echo = lower_echo(expr, context)?;
                context.push_statement(echo);
            }
            hir::Stmt::Return { expr, span } => {
                let terminator = lower_return(expr.as_ref(), *span, return_type, context)?;
                context.terminate_current(terminator);
            }
            hir::Stmt::VarDecl(decl) => lower_var_decl(decl, context)?,
            hir::Stmt::Assignment(assignment) => lower_assignment(assignment, context)?,
            hir::Stmt::Increment(increment) => lower_increment(increment, context)?,
            hir::Stmt::If(if_stmt) => lower_if_statement(if_stmt, return_type, context)?,
            hir::Stmt::While(while_stmt) => {
                lower_while_statement(while_stmt, return_type, context)?;
            }
            hir::Stmt::For(for_stmt) => {
                lower_for_statement(for_stmt, return_type, context)?;
            }
            hir::Stmt::Foreach(foreach) => {
                lower_foreach_statement(foreach, return_type, context)?;
            }
            hir::Stmt::Break { span } => lower_loop_control(*span, LoopControl::Break, context)?,
            hir::Stmt::Continue { span } => {
                lower_loop_control(*span, LoopControl::Continue, context)?;
            }
            hir::Stmt::Expr { expr, span } => {
                if let hir::Expr::FunctionCall {
                    name,
                    args,
                    span: call_span,
                } = expr
                {
                    let call = lower_void_call(name, args, *call_span, context)?;
                    context.push_statement(call);
                } else {
                    return Err(vec![unsupported(
                        *span,
                        "expression statements other than void free-function calls are not lowered to MIR in Stage 11g",
                    )]);
                }
            }
        }
    }

    Ok(())
}

fn lower_if_statement(
    if_stmt: &hir::IfStmt,
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    let condition_block = context.current_block();
    let fallthrough_blocks = lower_if_tree(if_stmt, condition_block, return_type, context)?;

    if fallthrough_blocks.is_empty() {
        context.current_block = None;
        return Ok(());
    }

    let continuation = context.create_block();
    for block in fallthrough_blocks {
        context.terminate_block(block, mir::Terminator::Jump(continuation));
    }
    context.current_block = Some(continuation);
    Ok(())
}

fn lower_if_tree(
    if_stmt: &hir::IfStmt,
    condition_block: mir::BlockId,
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<Vec<mir::BlockId>> {
    context.current_block = Some(condition_block);
    let condition = lower_condition(&if_stmt.condition, context)?;
    let then_block = context.create_block();
    let else_block = context.create_block();
    context.terminate_current(mir::Terminator::Branch {
        condition,
        then_block,
        else_block,
    });

    let mut fallthrough_blocks =
        lower_scoped_block(&if_stmt.then_block, then_block, return_type, context)?;

    match &if_stmt.else_branch {
        None => fallthrough_blocks.push(else_block),
        Some(hir::ElseBranch::Block(block)) => {
            fallthrough_blocks.extend(lower_scoped_block(block, else_block, return_type, context)?);
        }
        Some(hir::ElseBranch::If(nested)) => {
            fallthrough_blocks.extend(lower_if_tree(nested, else_block, return_type, context)?);
        }
    }

    Ok(fallthrough_blocks)
}

fn lower_while_statement(
    while_stmt: &hir::WhileStmt,
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    let header_block = context.create_block();
    let body_block = context.create_block();
    let exit_block = context.create_block();

    context.terminate_current(mir::Terminator::Jump(header_block));
    context.current_block = Some(header_block);
    let condition = lower_condition(&while_stmt.condition, context)?;
    context.terminate_current(mir::Terminator::Branch {
        condition,
        then_block: body_block,
        else_block: exit_block,
    });

    context.push_loop_targets(LoopTargets {
        continue_block: header_block,
        break_block: exit_block,
    });
    let body_result = lower_scoped_block(&while_stmt.body, body_block, return_type, context);
    context.pop_loop_targets();
    let fallthrough_blocks = body_result?;

    for block in fallthrough_blocks {
        context.terminate_block(block, mir::Terminator::Jump(header_block));
    }
    context.current_block = Some(exit_block);
    Ok(())
}

fn lower_for_statement(
    for_stmt: &hir::ForStmt,
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    context.push_scope();
    let result = lower_for_statement_in_scope(for_stmt, return_type, context);
    context.pop_scope();
    result
}

fn lower_for_statement_in_scope(
    for_stmt: &hir::ForStmt,
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    if let Some(initializer) = &for_stmt.initializer {
        match initializer {
            hir::ForInitializer::VarDecl(decl) => lower_var_decl(decl, context)?,
            hir::ForInitializer::Assignment(assignment) => {
                lower_assignment(assignment, context)?;
            }
        }
    }

    let header_block = context.create_block();
    let body_block = context.create_block();
    let increment_block = context.create_block();
    let exit_block = context.create_block();

    context.terminate_current(mir::Terminator::Jump(header_block));
    context.current_block = Some(header_block);
    let condition = for_stmt
        .condition
        .as_ref()
        .map(|condition| lower_condition(condition, context))
        .transpose()?
        .unwrap_or(mir::Condition::Bool(true));
    context.terminate_current(mir::Terminator::Branch {
        condition,
        then_block: body_block,
        else_block: exit_block,
    });

    context.push_loop_targets(LoopTargets {
        continue_block: increment_block,
        break_block: exit_block,
    });
    let body_result = lower_scoped_block(&for_stmt.body, body_block, return_type, context);
    context.pop_loop_targets();
    let fallthrough_blocks = body_result?;

    for block in fallthrough_blocks {
        context.terminate_block(block, mir::Terminator::Jump(increment_block));
    }

    context.current_block = Some(increment_block);
    if let Some(increment) = &for_stmt.increment {
        match increment {
            hir::ForIncrement::Increment(increment) => lower_increment(increment, context)?,
            hir::ForIncrement::Assignment(assignment) => {
                lower_assignment(assignment, context)?;
            }
        }
    }
    context.terminate_current(mir::Terminator::Jump(header_block));
    context.current_block = Some(exit_block);
    Ok(())
}

fn lower_foreach_statement(
    foreach: &hir::ForeachStmt,
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    if foreach.key.is_some() {
        return Err(vec![unsupported(
            foreach.span,
            "integer range foreach key bindings are not lowered to MIR in Stage 11g",
        )]);
    }

    let Some((start, end, inclusive)) = grouped_range_parts(&foreach.iterable) else {
        return Err(vec![unsupported(
            foreach.iterable.span(),
            "collection and general iterable foreach are not lowered to MIR in Stage 11g; only integer ranges are supported",
        )]);
    };

    if let Some(ty) = &foreach.value.ty {
        if ty.name != "int" || !ty.args.is_empty() {
            return Err(vec![unsupported(
                foreach.span,
                format!("integer range foreach bindings must use int in Stage 11g; got type {ty}"),
            )]);
        }
    }

    context.push_scope();
    let result = lower_range_foreach_in_scope(foreach, start, end, inclusive, return_type, context);
    context.pop_scope();
    result
}

fn lower_range_foreach_in_scope(
    foreach: &hir::ForeachStmt,
    start: &hir::Expr,
    end: &hir::Expr,
    inclusive: bool,
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    let start_value = lower_int_rvalue(start, context)?;
    let current_local = context.declare_temp(true);
    context.push_statement(mir::Statement::AssignLocal {
        target: current_local,
        value: start_value,
    });

    let end_value = lower_int_rvalue(end, context)?;
    let end_local = context.declare_temp(false);
    context.push_statement(mir::Statement::AssignLocal {
        target: end_local,
        value: end_value,
    });

    let header_block = context.create_block();
    let body_block = context.create_block();
    let update_block = context.create_block();
    let increment_block = inclusive.then(|| context.create_block());
    let exit_block = context.create_block();

    context.terminate_current(mir::Terminator::Jump(header_block));
    context.current_block = Some(header_block);
    context.terminate_current(mir::Terminator::Branch {
        condition: mir::Condition::Compare {
            op: if inclusive {
                mir::CompareOp::LessEqual
            } else {
                mir::CompareOp::Less
            },
            left: mir::IntExpression::Use(mir::Operand::Local(current_local)),
            right: mir::IntExpression::Use(mir::Operand::Local(end_local)),
        },
        then_block: body_block,
        else_block: exit_block,
    });

    let binding_local = context.declare_user_local(&foreach.value.name, false, mir::Type::Int);
    context.push_loop_targets(LoopTargets {
        continue_block: update_block,
        break_block: exit_block,
    });
    context.current_block = Some(body_block);
    context.push_statement(mir::Statement::AssignLocal {
        target: binding_local,
        value: mir::Rvalue::Use(mir::Operand::Local(current_local)),
    });
    let body_result = lower_statement_sequence(&foreach.body.statements, return_type, context);
    let body_fallthrough = context.current_block;
    context.pop_loop_targets();
    body_result?;

    if let Some(block) = body_fallthrough {
        context.terminate_block(block, mir::Terminator::Jump(update_block));
    }

    context.current_block = Some(update_block);
    if let Some(increment_block) = increment_block {
        context.terminate_current(mir::Terminator::Branch {
            condition: mir::Condition::Compare {
                op: mir::CompareOp::Equal,
                left: mir::IntExpression::Use(mir::Operand::Local(current_local)),
                right: mir::IntExpression::Use(mir::Operand::Local(end_local)),
            },
            then_block: exit_block,
            else_block: increment_block,
        });
        context.current_block = Some(increment_block);
    }
    context.push_statement(mir::Statement::AssignLocal {
        target: current_local,
        value: mir::Rvalue::Binary {
            op: mir::BinaryOp::Add,
            left: mir::Operand::Local(current_local),
            right: mir::Operand::Int(1),
        },
    });
    context.terminate_current(mir::Terminator::Jump(header_block));
    context.current_block = Some(exit_block);
    Ok(())
}

fn grouped_range_parts(expr: &hir::Expr) -> Option<(&hir::Expr, &hir::Expr, bool)> {
    match expr {
        hir::Expr::Grouped { expr, .. } => grouped_range_parts(expr),
        hir::Expr::Range {
            start,
            end,
            inclusive,
            ..
        } => Some((start, end, *inclusive)),
        _ => None,
    }
}

#[derive(Clone, Copy)]
enum LoopControl {
    Break,
    Continue,
}

fn lower_loop_control(
    span: Span,
    control: LoopControl,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    let targets = context.current_loop_targets().ok_or_else(|| {
        let keyword = match control {
            LoopControl::Break => "break",
            LoopControl::Continue => "continue",
        };
        vec![unsupported(
            span,
            format!("{keyword} requires an enclosing loop in MIR Stage 11g"),
        )]
    })?;
    let target = match control {
        LoopControl::Break => targets.break_block,
        LoopControl::Continue => targets.continue_block,
    };
    context.terminate_current(mir::Terminator::Jump(target));
    Ok(())
}

fn lower_scoped_block(
    block: &hir::Block,
    entry_block: mir::BlockId,
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<Vec<mir::BlockId>> {
    context.push_scope();
    context.current_block = Some(entry_block);
    let result = lower_statement_sequence(&block.statements, return_type, context);
    let current_block = context.current_block;
    context.pop_scope();
    result?;
    Ok(current_block.into_iter().collect())
}

struct BlockBuilder {
    id: mir::BlockId,
    statements: Vec<mir::Statement>,
    terminator: Option<mir::Terminator>,
}

#[derive(Clone, Copy)]
struct LoopTargets {
    continue_block: mir::BlockId,
    break_block: mir::BlockId,
}

struct LoweringContext {
    signatures: HashMap<String, FunctionSignature>,
    locals: Vec<mir::Local>,
    local_scopes: Vec<HashMap<String, mir::LocalId>>,
    temp_counter: usize,
    blocks: Vec<BlockBuilder>,
    current_block: Option<mir::BlockId>,
    loop_targets: Vec<LoopTargets>,
}

impl LoweringContext {
    fn new(signatures: HashMap<String, FunctionSignature>) -> Self {
        Self {
            signatures,
            locals: Vec::new(),
            local_scopes: vec![HashMap::new()],
            temp_counter: 0,
            blocks: vec![BlockBuilder {
                id: mir::BlockId(0),
                statements: Vec::new(),
                terminator: None,
            }],
            current_block: Some(mir::BlockId(0)),
            loop_targets: Vec::new(),
        }
    }

    fn finish(self) -> (Vec<mir::Local>, Vec<mir::BasicBlock>) {
        let blocks = self
            .blocks
            .into_iter()
            .map(|block| mir::BasicBlock {
                id: block.id,
                statements: block.statements,
                terminator: block
                    .terminator
                    .expect("every lowered MIR block must have a terminator"),
            })
            .collect();
        (self.locals, blocks)
    }

    fn create_block(&mut self) -> mir::BlockId {
        let id = mir::BlockId(self.blocks.len());
        self.blocks.push(BlockBuilder {
            id,
            statements: Vec::new(),
            terminator: None,
        });
        id
    }

    fn current_block(&self) -> mir::BlockId {
        self.current_block
            .expect("MIR lowering requires a current block")
    }

    fn push_statement(&mut self, statement: mir::Statement) {
        let block = self.current_block();
        self.blocks[block.0].statements.push(statement);
    }

    fn terminate_current(&mut self, terminator: mir::Terminator) {
        let block = self.current_block();
        self.terminate_block(block, terminator);
        self.current_block = None;
    }

    fn terminate_block(&mut self, block: mir::BlockId, terminator: mir::Terminator) {
        let slot = &mut self.blocks[block.0].terminator;
        assert!(slot.is_none(), "MIR block terminated more than once");
        *slot = Some(terminator);
    }

    fn push_scope(&mut self) {
        self.local_scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        assert!(
            self.local_scopes.len() > 1,
            "MIR lowering cannot pop the root local scope"
        );
        self.local_scopes.pop();
    }

    fn push_loop_targets(&mut self, targets: LoopTargets) {
        self.loop_targets.push(targets);
    }

    fn pop_loop_targets(&mut self) {
        self.loop_targets
            .pop()
            .expect("MIR lowering cannot pop an empty loop-target stack");
    }

    fn current_loop_targets(&self) -> Option<LoopTargets> {
        self.loop_targets.last().copied()
    }

    fn declare_user_local(&mut self, name: &str, writable: bool, ty: mir::Type) -> mir::LocalId {
        let id = mir::LocalId(self.locals.len());
        self.locals.push(mir::Local {
            id,
            name: name.to_string(),
            ty,
            writable,
            synthetic: false,
        });
        self.local_scopes
            .last_mut()
            .expect("MIR lowering must have a local scope")
            .insert(name.to_string(), id);
        id
    }

    fn declare_temp(&mut self, writable: bool) -> mir::LocalId {
        let id = mir::LocalId(self.locals.len());
        let name = format!("_tmp{}", self.temp_counter);
        self.temp_counter += 1;
        self.locals.push(mir::Local {
            id,
            name,
            ty: mir::Type::Int,
            writable,
            synthetic: true,
        });
        id
    }

    fn lookup_local(&self, name: &str, span: Span) -> DiagnosticResult<mir::LocalId> {
        self.local_scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
            .ok_or_else(|| {
                vec![unsupported(
                    span,
                    format!("local `${name}` is not available in MIR Stage 11g"),
                )]
            })
    }

    fn local_type(&self, id: mir::LocalId) -> mir::Type {
        self.locals
            .get(id.0)
            .filter(|local| local.id == id)
            .expect("lowered MIR local must have a matching slot")
            .ty
    }

    fn lookup_int_local(&self, name: &str, span: Span) -> DiagnosticResult<mir::LocalId> {
        let local = self.lookup_local(name, span)?;
        if self.local_type(local) == mir::Type::Int {
            Ok(local)
        } else {
            Err(vec![unsupported(
                span,
                format!("string local `${name}` cannot be used as an int expression in Stage 11g"),
            )])
        }
    }

    fn lookup_function(&self, name: &str, span: Span) -> DiagnosticResult<FunctionSignature> {
        self.signatures.get(name).copied().ok_or_else(|| {
            vec![unsupported(
                span,
                format!("call references unknown top-level function `{name}`"),
            )]
        })
    }
}

fn lower_var_decl(decl: &hir::VarDecl, context: &mut LoweringContext) -> DiagnosticResult<()> {
    let ty = match &decl.ty {
        Some(ty) if is_plain_type(ty, "int") => mir::Type::Int,
        Some(ty) if is_plain_type(ty, "string") => mir::Type::String,
        Some(ty) => {
            return Err(vec![unsupported(
                decl.span,
                format!("only int and readonly string locals are lowered to MIR in Stage 11g; got `{ty}`"),
            )]);
        }
        None if is_string_local_initializer(&decl.initializer, context) => mir::Type::String,
        None => mir::Type::Int,
    };

    if ty == mir::Type::String {
        return lower_string_var_decl(decl, context);
    }

    let value = lower_int_rvalue(&decl.initializer, context)?;
    let local = context.declare_user_local(&decl.name, decl.writable, mir::Type::Int);
    context.push_statement(mir::Statement::AssignLocal {
        target: local,
        value,
    });
    Ok(())
}

fn is_string_local_initializer(expr: &hir::Expr, context: &LoweringContext) -> bool {
    match expr {
        hir::Expr::String { .. } | hir::Expr::InterpolatedString { .. } => true,
        hir::Expr::Grouped { expr, .. } => is_string_local_initializer(expr, context),
        hir::Expr::Binary {
            op: hir::BinaryOp::Concat,
            ..
        } => true,
        hir::Expr::Variable { name, span } => context
            .lookup_local(name, *span)
            .is_ok_and(|local| context.local_type(local) == mir::Type::String),
        _ => false,
    }
}

fn lower_string_var_decl(
    decl: &hir::VarDecl,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    if decl.writable {
        return Err(vec![unsupported(
            decl.span,
            "writable string locals are not lowered to MIR in Stage 11g",
        )]);
    }

    let value = lower_string_expression(&decl.initializer, context)?;
    let local = context.declare_user_local(&decl.name, false, mir::Type::String);
    context.push_statement(mir::Statement::AssignLocal {
        target: local,
        value: mir::Rvalue::String(value),
    });
    Ok(())
}

fn lower_assignment(
    assignment: &hir::Assignment,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    let target = lower_assignment_target(&assignment.target, context)?;
    if context.local_type(target) == mir::Type::String {
        return Err(vec![unsupported(
            assignment.span,
            "string assignment is not lowered to MIR in Stage 11g",
        )]);
    }

    let value = match assignment.op {
        hir::AssignOp::Assign => lower_int_rvalue(&assignment.value, context)?,
        hir::AssignOp::AddAssign => {
            let right = lower_int_operand(&assignment.value, context)?;
            mir::Rvalue::Binary {
                op: mir::BinaryOp::Add,
                left: mir::Operand::Local(target),
                right,
            }
        }
        hir::AssignOp::SubAssign => {
            let right = lower_int_operand(&assignment.value, context)?;
            mir::Rvalue::Binary {
                op: mir::BinaryOp::Subtract,
                left: mir::Operand::Local(target),
                right,
            }
        }
    };
    context.push_statement(mir::Statement::AssignLocal { target, value });
    Ok(())
}

fn lower_increment(
    increment: &hir::IncrementStmt,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    let target = lower_assignment_target(&increment.target, context)?;
    if context.local_type(target) == mir::Type::String {
        return Err(vec![unsupported(
            increment.span,
            "string increment and decrement are not lowered to MIR in Stage 11g",
        )]);
    }

    let op = match increment.op {
        hir::IncrementOp::Increment => mir::BinaryOp::Add,
        hir::IncrementOp::Decrement => mir::BinaryOp::Subtract,
    };
    context.push_statement(mir::Statement::AssignLocal {
        target,
        value: mir::Rvalue::Binary {
            op,
            left: mir::Operand::Local(target),
            right: mir::Operand::Int(1),
        },
    });
    Ok(())
}

fn lower_assignment_target(
    target: &hir::Expr,
    context: &LoweringContext,
) -> DiagnosticResult<mir::LocalId> {
    match target {
        hir::Expr::Grouped { expr, .. } => lower_assignment_target(expr, context),
        hir::Expr::Variable { name, span } => context.lookup_local(name, *span),
        _ => Err(vec![unsupported(
            target.span(),
            "only local variable assignment targets are lowered to MIR in Stage 11g",
        )]),
    }
}

fn lower_echo(expr: &hir::Expr, context: &LoweringContext) -> DiagnosticResult<mir::Statement> {
    match expr {
        hir::Expr::String { value, .. } => Ok(mir::Statement::EchoStringLiteral(value.clone())),
        _ => lower_string_expression(expr, context).map(mir::Statement::EchoString),
    }
}

fn lower_string_expression(
    expr: &hir::Expr,
    context: &LoweringContext,
) -> DiagnosticResult<mir::StringExpression> {
    match expr {
        hir::Expr::String { value, .. } => Ok(mir::StringExpression::Literal(value.clone())),
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            if context.local_type(local) == mir::Type::String {
                Ok(mir::StringExpression::Local(local))
            } else {
                Err(vec![unsupported(
                    *span,
                    "string expressions may reference only readonly string locals in Stage 11g",
                )])
            }
        }
        hir::Expr::Grouped { expr, .. } => lower_string_expression(expr, context),
        hir::Expr::Binary {
            op: hir::BinaryOp::Concat,
            ..
        } => {
            let mut parts = Vec::new();
            append_string_concat_parts(expr, context, &mut parts)?;
            Ok(mir::StringExpression::Concat(parts))
        }
        hir::Expr::InterpolatedString { .. } => Err(vec![unsupported(
            expr.span(),
            "string interpolation expansion is not lowered to MIR in Stage 11g",
        )]),
        _ => Err(vec![unsupported(
            expr.span(),
            "echo supports only string literals, readonly string locals, and string concatenation in Stage 11g",
        )]),
    }
}

fn append_string_concat_parts(
    expr: &hir::Expr,
    context: &LoweringContext,
    parts: &mut Vec<mir::StringExpression>,
) -> DiagnosticResult<()> {
    match expr {
        hir::Expr::Grouped { expr, .. } => append_string_concat_parts(expr, context, parts),
        hir::Expr::Binary {
            left,
            op: hir::BinaryOp::Concat,
            right,
            ..
        } => {
            append_string_concat_parts(left, context, parts)?;
            append_string_concat_parts(right, context, parts)
        }
        hir::Expr::String { value, .. } => {
            parts.push(mir::StringExpression::Literal(value.clone()));
            Ok(())
        }
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            if context.local_type(local) != mir::Type::String {
                return Err(vec![unsupported(
                    *span,
                    "string concatenation operands must be string expressions",
                )]);
            }
            parts.push(mir::StringExpression::Local(local));
            Ok(())
        }
        _ => Err(vec![unsupported(
            expr.span(),
            "string concatenation operands must be string expressions",
        )]),
    }
}

fn lower_void_call(
    name: &str,
    args: &[hir::Expr],
    span: Span,
    context: &LoweringContext,
) -> DiagnosticResult<mir::Statement> {
    let signature = context.lookup_function(name, span)?;
    if signature.return_type != mir::ReturnType::Void {
        return Err(vec![unsupported(
            span,
            format!("non-void function `{name}` cannot be used as a statement in MIR Stage 11g"),
        )]);
    }

    Ok(mir::Statement::CallVoid {
        function: signature.id,
        args: lower_call_args(name, args, signature, span, context)?,
    })
}

fn lower_int_call(
    name: &str,
    args: &[hir::Expr],
    span: Span,
    context: &LoweringContext,
) -> DiagnosticResult<(mir::FunctionId, Vec<mir::IntExpression>)> {
    let signature = context.lookup_function(name, span)?;
    if signature.return_type != mir::ReturnType::Int {
        return Err(vec![unsupported(
            span,
            format!(
                "void function `{name}` cannot be used as an integer expression in MIR Stage 11g"
            ),
        )]);
    }

    let args = lower_call_args(name, args, signature, span, context)?;
    Ok((signature.id, args))
}

fn lower_call_args(
    name: &str,
    args: &[hir::Expr],
    signature: FunctionSignature,
    span: Span,
    context: &LoweringContext,
) -> DiagnosticResult<Vec<mir::IntExpression>> {
    if args.len() != signature.parameter_count {
        return Err(vec![unsupported(
            span,
            format!(
                "function `{name}` expects {} positional argument(s), got {}",
                signature.parameter_count,
                args.len()
            ),
        )]);
    }

    args.iter()
        .map(|arg| lower_call_argument(arg, context))
        .collect()
}

fn lower_call_argument(
    expr: &hir::Expr,
    context: &LoweringContext,
) -> DiagnosticResult<mir::IntExpression> {
    match expr {
        hir::Expr::Int { value, span } => {
            parse_int_literal(value, *span).map(mir::IntExpression::Use)
        }
        hir::Expr::Variable { name, span } => context
            .lookup_int_local(name, *span)
            .map(mir::Operand::Local)
            .map(mir::IntExpression::Use),
        hir::Expr::Grouped { expr, .. } => lower_call_argument(expr, context),
        hir::Expr::Binary {
            left, op, right, ..
        } => {
            let op = lower_condition_int_binary_op(op, expr.span())?;
            Ok(mir::IntExpression::Binary {
                op,
                left: Box::new(lower_call_argument(left, context)?),
                right: Box::new(lower_call_argument(right, context)?),
            })
        }
        _ => Err(vec![unsupported(
            expr.span(),
            "call arguments support only Stage 11b integer expressions in MIR Stage 11g",
        )]),
    }
}

fn lower_return(
    expr: Option<&hir::Expr>,
    span: Span,
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::Terminator> {
    match (return_type, expr) {
        (mir::ReturnType::Void, None) => Ok(mir::Terminator::ReturnVoid),
        (mir::ReturnType::Int, Some(expr)) => {
            let operand = lower_int_operand(expr, context)?;
            Ok(mir::Terminator::Return(operand))
        }
        (mir::ReturnType::Int, None) => Err(vec![unsupported(
            span,
            "bare return is not lowered for int-returning functions in Stage 11g",
        )]),
        (mir::ReturnType::Void, Some(expr)) => Err(vec![unsupported(
            expr.span(),
            "return values are not lowered for void functions in Stage 11g",
        )]),
    }
}

fn lower_condition(
    expr: &hir::Expr,
    context: &LoweringContext,
) -> DiagnosticResult<mir::Condition> {
    match expr {
        hir::Expr::Bool { value, .. } => Ok(mir::Condition::Bool(*value)),
        hir::Expr::Grouped { expr, .. } => lower_condition(expr, context),
        hir::Expr::Unary {
            op: hir::UnaryOp::Not,
            expr,
            ..
        } => Ok(mir::Condition::Not(Box::new(lower_condition(
            expr, context,
        )?))),
        hir::Expr::Binary {
            left, op, right, ..
        } => match op {
            hir::BinaryOp::Equal
            | hir::BinaryOp::NotEqual
            | hir::BinaryOp::Less
            | hir::BinaryOp::LessEqual
            | hir::BinaryOp::Greater
            | hir::BinaryOp::GreaterEqual => Ok(mir::Condition::Compare {
                op: lower_compare_op(op),
                left: lower_condition_int_expr(left, context)?,
                right: lower_condition_int_expr(right, context)?,
            }),
            hir::BinaryOp::And | hir::BinaryOp::Or | hir::BinaryOp::Xor => {
                Ok(mir::Condition::Binary {
                    op: lower_condition_binary_op(op),
                    left: Box::new(lower_condition(left, context)?),
                    right: Box::new(lower_condition(right, context)?),
                })
            }
            _ => Err(vec![unsupported(
                expr.span(),
                "conditions require bool literals, integer comparisons, or boolean condition operators in Stage 11g",
            )]),
        },
        hir::Expr::FunctionCall { .. }
        | hir::Expr::MethodCall { .. }
        | hir::Expr::StaticCall { .. } => Err(vec![unsupported(
            expr.span(),
            "function and method calls in conditions are not lowered to MIR in Stage 11g",
        )]),
        hir::Expr::Int { .. } => Err(vec![unsupported(
            expr.span(),
            "integer truthiness is not Doria condition semantics; Stage 11g requires a bool condition",
        )]),
        hir::Expr::Variable { .. } => Err(vec![unsupported(
            expr.span(),
            "user-authored bool locals are not lowered to MIR in Stage 11g",
        )]),
        _ => Err(vec![unsupported(
            expr.span(),
            "this condition expression is not lowered to MIR in Stage 11g",
        )]),
    }
}

fn lower_condition_int_expr(
    expr: &hir::Expr,
    context: &LoweringContext,
) -> DiagnosticResult<mir::IntExpression> {
    match expr {
        hir::Expr::Int { value, span } => {
            parse_int_literal(value, *span).map(mir::IntExpression::Use)
        }
        hir::Expr::Variable { name, span } => context
            .lookup_int_local(name, *span)
            .map(mir::Operand::Local)
            .map(mir::IntExpression::Use),
        hir::Expr::Grouped { expr, .. } => lower_condition_int_expr(expr, context),
        hir::Expr::Binary {
            left, op, right, ..
        } => {
            let op = lower_condition_int_binary_op(op, expr.span())?;
            Ok(mir::IntExpression::Binary {
                op,
                left: Box::new(lower_condition_int_expr(left, context)?),
                right: Box::new(lower_condition_int_expr(right, context)?),
            })
        }
        hir::Expr::FunctionCall { name, args, span } => {
            let (function, args) = lower_int_call(name, args, *span, context)?;
            Ok(mir::IntExpression::Call { function, args })
        }
        _ => Err(vec![unsupported(
            expr.span(),
            "only Stage 11b integer expressions are lowered as comparison operands in Stage 11g",
        )]),
    }
}

fn lower_compare_op(op: &hir::BinaryOp) -> mir::CompareOp {
    match op {
        hir::BinaryOp::Equal => mir::CompareOp::Equal,
        hir::BinaryOp::NotEqual => mir::CompareOp::NotEqual,
        hir::BinaryOp::Less => mir::CompareOp::Less,
        hir::BinaryOp::LessEqual => mir::CompareOp::LessEqual,
        hir::BinaryOp::Greater => mir::CompareOp::Greater,
        hir::BinaryOp::GreaterEqual => mir::CompareOp::GreaterEqual,
        _ => unreachable!("only comparison operators are lowered as MIR comparisons"),
    }
}

fn lower_condition_binary_op(op: &hir::BinaryOp) -> mir::ConditionBinaryOp {
    match op {
        hir::BinaryOp::And => mir::ConditionBinaryOp::And,
        hir::BinaryOp::Or => mir::ConditionBinaryOp::Or,
        hir::BinaryOp::Xor => mir::ConditionBinaryOp::Xor,
        _ => unreachable!("only boolean operators are lowered as MIR condition operators"),
    }
}

fn lower_condition_int_binary_op(
    op: &hir::BinaryOp,
    span: Span,
) -> DiagnosticResult<mir::BinaryOp> {
    match op {
        hir::BinaryOp::Add => Ok(mir::BinaryOp::Add),
        hir::BinaryOp::Sub => Ok(mir::BinaryOp::Subtract),
        hir::BinaryOp::Mul => Ok(mir::BinaryOp::Multiply),
        hir::BinaryOp::Div | hir::BinaryOp::Mod => Err(vec![unsupported(
            span,
            "division and modulo in condition operands are not lowered to MIR in Stage 11g",
        )]),
        _ => Err(vec![unsupported(
            span,
            "only Stage 11b integer arithmetic is lowered inside MIR Stage 11g comparisons",
        )]),
    }
}

fn lower_int_operand(
    expr: &hir::Expr,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::Operand> {
    match expr {
        hir::Expr::Int { value, span } => parse_int_literal(value, *span),
        hir::Expr::Variable { name, span } => context
            .lookup_int_local(name, *span)
            .map(mir::Operand::Local),
        hir::Expr::Grouped { expr, .. } => lower_int_operand(expr, context),
        hir::Expr::Binary { .. } | hir::Expr::FunctionCall { .. } => {
            let value = lower_int_rvalue(expr, context)?;
            let temp = context.declare_temp(false);
            context.push_statement(mir::Statement::AssignLocal {
                target: temp,
                value,
            });
            Ok(mir::Operand::Local(temp))
        }
        _ => Err(vec![unsupported_int_expr(expr)]),
    }
}

fn lower_int_rvalue(
    expr: &hir::Expr,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::Rvalue> {
    match expr {
        hir::Expr::Int { .. } | hir::Expr::Variable { .. } | hir::Expr::Grouped { .. } => {
            lower_int_operand(expr, context).map(mir::Rvalue::Use)
        }
        hir::Expr::Binary {
            left, op, right, ..
        } => {
            let op = lower_binary_op(op, expr.span())?;
            let left = lower_int_operand(left, context)?;
            let right = lower_int_operand(right, context)?;
            Ok(mir::Rvalue::Binary { op, left, right })
        }
        hir::Expr::FunctionCall { name, args, span } => {
            let (function, args) = lower_int_call(name, args, *span, context)?;
            Ok(mir::Rvalue::Call { function, args })
        }
        _ => Err(vec![unsupported_int_expr(expr)]),
    }
}

fn lower_binary_op(op: &hir::BinaryOp, span: Span) -> DiagnosticResult<mir::BinaryOp> {
    match op {
        hir::BinaryOp::Add => Ok(mir::BinaryOp::Add),
        hir::BinaryOp::Sub => Ok(mir::BinaryOp::Subtract),
        hir::BinaryOp::Mul => Ok(mir::BinaryOp::Multiply),
        hir::BinaryOp::Div | hir::BinaryOp::Mod => Err(vec![unsupported(
            span,
            "division and modulo are not lowered to MIR in Stage 11g",
        )]),
        hir::BinaryOp::Less
        | hir::BinaryOp::LessEqual
        | hir::BinaryOp::Greater
        | hir::BinaryOp::GreaterEqual
        | hir::BinaryOp::Equal
        | hir::BinaryOp::NotEqual => Err(vec![unsupported(
            span,
            "comparison results are condition-only and are not lowered as runtime values in MIR Stage 11g",
        )]),
        hir::BinaryOp::Concat => Err(vec![unsupported(
            span,
            "string concatenation is not lowered to MIR in Stage 11g",
        )]),
        hir::BinaryOp::And | hir::BinaryOp::Or | hir::BinaryOp::Xor => Err(vec![unsupported(
            span,
            "bool runtime values are not lowered to MIR in Stage 11g; boolean operators are condition-only",
        )]),
        hir::BinaryOp::Coalesce => Err(vec![unsupported(
            span,
            "null coalescing is not lowered to MIR in Stage 11g",
        )]),
    }
}

fn parse_int_literal(value: &str, span: Span) -> DiagnosticResult<mir::Operand> {
    value.parse::<i64>().map(mir::Operand::Int).map_err(|_| {
        vec![unsupported(
            span,
            "integer literals outside int64 are not lowered to MIR in Stage 11g",
        )]
    })
}

fn unsupported_int_expr(expr: &hir::Expr) -> Diagnostic {
    let detail = match expr {
        hir::Expr::String { .. } | hir::Expr::InterpolatedString { .. } => {
            "string expressions are not lowered to MIR in Stage 11g"
        }
        hir::Expr::Float { .. } => "float expressions are not lowered to MIR in Stage 11g",
        hir::Expr::Bool { .. } => "bool runtime values are not lowered to MIR in Stage 11g",
        hir::Expr::Null { .. } => "null values are not lowered to MIR in Stage 11g",
        hir::Expr::Array { .. } => "collections are not lowered to MIR in Stage 11g",
        hir::Expr::FunctionCall { .. } => "function calls are not lowered to MIR in Stage 11g",
        hir::Expr::MethodCall { .. } | hir::Expr::StaticCall { .. } => {
            "method calls are not lowered to MIR in Stage 11g"
        }
        hir::Expr::PropertyAccess { .. } => "property access is not lowered to MIR in Stage 11g",
        hir::Expr::New { .. } => "object construction is not lowered to MIR in Stage 11g",
        hir::Expr::This { .. } => "$this is not lowered to MIR in Stage 11g",
        hir::Expr::Identifier { .. } => {
            "identifiers are not lowered as int expressions in Stage 11g"
        }
        hir::Expr::Unary { .. } => "unary expressions are not lowered to MIR in Stage 11g",
        hir::Expr::Range { .. } => "ranges are not lowered to MIR in Stage 11g",
        hir::Expr::Binary { .. } => "this binary expression is not lowered to MIR in Stage 11g",
        hir::Expr::Int { .. } | hir::Expr::Variable { .. } | hir::Expr::Grouped { .. } => {
            "this int expression is not lowered to MIR in Stage 11g"
        }
    };
    unsupported(expr.span(), detail)
}

fn validate_call_graph(functions: &[mir::Function], spans: &[Span]) -> DiagnosticResult<()> {
    let mut graph = Vec::with_capacity(functions.len());
    for (index, function) in functions.iter().enumerate() {
        if function.id.0 != index {
            return Err(vec![unsupported(
                spans.get(index).copied().unwrap_or_default(),
                format!(
                    "function `{}` has non-deterministic MIR id function{}",
                    function.name, function.id.0
                ),
            )]);
        }

        let mut calls = collect_function_calls(function);
        calls.sort_by_key(|function| function.0);
        calls.dedup();
        for callee in &calls {
            if callee.0 >= functions.len() {
                return Err(vec![unsupported(
                    spans.get(index).copied().unwrap_or_default(),
                    format!("call target function{} does not exist", callee.0),
                )]);
            }
            if *callee == function.id {
                return Err(vec![unsupported(
                    spans.get(index).copied().unwrap_or_default(),
                    "recursive calls are not supported",
                )]);
            }
        }
        graph.push(calls);
    }

    let mut states = vec![0_u8; graph.len()];
    for function in 0..graph.len() {
        if states[function] == 0 {
            if let Some(caller) = visit_call_graph(function, &graph, &mut states) {
                return Err(vec![unsupported(
                    spans.get(caller).copied().unwrap_or_default(),
                    "mutual recursion is not supported",
                )]);
            }
        }
    }

    Ok(())
}

fn visit_call_graph(
    function: usize,
    graph: &[Vec<mir::FunctionId>],
    states: &mut [u8],
) -> Option<usize> {
    states[function] = 1;
    for callee in &graph[function] {
        match states[callee.0] {
            0 => {
                if let Some(caller) = visit_call_graph(callee.0, graph, states) {
                    return Some(caller);
                }
            }
            1 => return Some(function),
            _ => {}
        }
    }
    states[function] = 2;
    None
}

fn collect_function_calls(function: &mir::Function) -> Vec<mir::FunctionId> {
    let mut calls = Vec::new();
    for block in &function.blocks {
        for statement in &block.statements {
            match statement {
                mir::Statement::AssignLocal { value, .. } => {
                    collect_rvalue_calls(value, &mut calls);
                }
                mir::Statement::EchoStringLiteral(_) | mir::Statement::EchoString(_) => {}
                mir::Statement::CallVoid { function, args } => {
                    calls.push(*function);
                    for arg in args {
                        collect_int_expression_calls(arg, &mut calls);
                    }
                }
            }
        }
        if let mir::Terminator::Branch { condition, .. } = &block.terminator {
            collect_condition_calls(condition, &mut calls);
        }
    }
    calls
}

fn collect_rvalue_calls(value: &mir::Rvalue, calls: &mut Vec<mir::FunctionId>) {
    if let mir::Rvalue::Call { function, args } = value {
        calls.push(*function);
        for arg in args {
            collect_int_expression_calls(arg, calls);
        }
    }
}

fn collect_int_expression_calls(expression: &mir::IntExpression, calls: &mut Vec<mir::FunctionId>) {
    match expression {
        mir::IntExpression::Use(_) => {}
        mir::IntExpression::Binary { left, right, .. } => {
            collect_int_expression_calls(left, calls);
            collect_int_expression_calls(right, calls);
        }
        mir::IntExpression::Call { function, args } => {
            calls.push(*function);
            for arg in args {
                collect_int_expression_calls(arg, calls);
            }
        }
    }
}

fn collect_condition_calls(condition: &mir::Condition, calls: &mut Vec<mir::FunctionId>) {
    match condition {
        mir::Condition::Bool(_) => {}
        mir::Condition::Compare { left, right, .. } => {
            collect_int_expression_calls(left, calls);
            collect_int_expression_calls(right, calls);
        }
        mir::Condition::Not(condition) => collect_condition_calls(condition, calls),
        mir::Condition::Binary { left, right, .. } => {
            collect_condition_calls(left, calls);
            collect_condition_calls(right, calls);
        }
    }
}

fn stmt_span(statement: &hir::Stmt) -> Span {
    match statement {
        hir::Stmt::VarDecl(decl) => decl.span,
        hir::Stmt::Assignment(assignment) => assignment.span,
        hir::Stmt::Echo { span, .. } | hir::Stmt::Return { span, .. } => *span,
        hir::Stmt::If(if_stmt) => if_stmt.span,
        hir::Stmt::While(while_stmt) => while_stmt.span,
        hir::Stmt::For(for_stmt) => for_stmt.span,
        hir::Stmt::Break { span } | hir::Stmt::Continue { span } => *span,
        hir::Stmt::Foreach(foreach) => foreach.span,
        hir::Stmt::Increment(increment) => increment.span,
        hir::Stmt::Expr { span, .. } => *span,
    }
}

fn unsupported(span: Span, detail: impl Into<String>) -> Diagnostic {
    Diagnostic::new(
        "M1101",
        format!("unsupported MIR Stage 11g coverage: {}", detail.into()),
        span,
    )
}

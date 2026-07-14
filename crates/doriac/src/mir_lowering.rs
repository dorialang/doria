use std::collections::HashMap;

use crate::diagnostics::{Diagnostic, DiagnosticResult};
use crate::format_string::{self, FormatConversion, FormatPiece};
use crate::numeric::{parse_decimal_magnitude, FloatType, FloatValue, IntegerType, IntegerValue};
use crate::semantics::SemanticInfo;
use crate::source::Span;
use crate::{hir, mir};

#[derive(Clone)]
struct FunctionSignature {
    id: mir::FunctionId,
    return_type: mir::ReturnType,
    parameter_types: Vec<mir::Type>,
}

pub fn lower_program(program: &hir::Program) -> DiagnosticResult<mir::Program> {
    let mut declarations = Vec::new();

    for item in &program.items {
        match item {
            hir::Item::Function(function) => declarations.push(function),
            hir::Item::Class(class_decl) => {
                return Err(vec![unsupported(
                    class_decl.span,
                    "classes are not lowered to MIR in Stage 11",
                )]);
            }
            hir::Item::Statement(statement) => {
                return Err(vec![unsupported(
                    stmt_span(statement),
                    "top-level statements are not lowered to MIR in Stage 11",
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
            "Stage 11 requires exactly one top-level function main",
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
    let functions = declarations
        .iter()
        .map(|function| {
            let signature = signatures
                .get(&function.name)
                .cloned()
                .expect("every function signature must be collected");
            lower_function(function, signature, &signatures, &program.semantic_info)
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(mir::Program { functions, entry })
}

fn collect_function_signature(
    function: &hir::FunctionDecl,
    id: mir::FunctionId,
) -> DiagnosticResult<FunctionSignature> {
    let return_type = match function.return_type.as_ref() {
        Some(ty) if scalar_type_ref(ty).is_some() => mir::ReturnType::Value(mir::Type::Scalar(
            scalar_type_ref(ty).expect("checked scalar type"),
        )),
        Some(ty) if is_plain_type(ty, "string") => mir::ReturnType::Value(mir::Type::String),
        Some(ty) if is_nullable_string_type(ty) => {
            mir::ReturnType::Value(mir::Type::NullableString)
        }
        Some(ty) if is_plain_type(ty, "void") => mir::ReturnType::Void,
        Some(ty) => {
            return Err(vec![unsupported(
                function.span,
                format!(
                    "function `{}` has unsupported return type `{ty}`; Stage 14 MIR supports scalar and void returns",
                    function.name
                ),
            )]);
        }
        None => {
            return Err(vec![unsupported(
                function.span,
                format!(
                    "function `{}` requires an explicit scalar or void return type for MIR Stage 14",
                    function.name
                ),
            )]);
        }
    };

    if function.name == "main" && !function.params.is_empty() {
        return Err(vec![unsupported(
            function.params[0].span,
            "main parameters are not lowered to MIR in Stage 11",
        )]);
    }

    if function.name == "main"
        && !matches!(
            return_type,
            mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Integer(
                IntegerType::Int64,
            ))) | mir::ReturnType::Void
        )
    {
        return Err(vec![unsupported(
            function.span,
            "main must return int/int64 or void in Stage 13 MIR",
        )]);
    }

    let mut parameter_types = Vec::with_capacity(function.params.len());
    for param in &function.params {
        if param.default.is_some() {
            return Err(vec![unsupported(
                param.span,
                format!(
                    "default arguments are not lowered for function `{}` in MIR Stage 11",
                    function.name
                ),
            )]);
        }
        let parameter_type = if let Some(scalar) = scalar_type_ref(&param.ty) {
            mir::Type::Scalar(scalar)
        } else if is_plain_type(&param.ty, "string") {
            mir::Type::String
        } else if is_nullable_string_type(&param.ty) {
            mir::Type::NullableString
        } else {
            return Err(vec![unsupported(
                param.span,
                format!(
                    "function `{}` has unsupported parameter type `{}`; Stage 14 MIR supports scalar parameters",
                    function.name, param.ty
                ),
            )]);
        };
        parameter_types.push(parameter_type);
    }

    Ok(FunctionSignature {
        id,
        return_type,
        parameter_types,
    })
}

fn integer_type_ref(ty: &crate::types::TypeRef) -> Option<IntegerType> {
    ty.args
        .is_empty()
        .then(|| IntegerType::from_source_name(&ty.name))
        .flatten()
}

fn float_type_ref(ty: &crate::types::TypeRef) -> Option<FloatType> {
    ty.args
        .is_empty()
        .then(|| FloatType::from_source_name(&ty.name))
        .flatten()
}

fn scalar_type_ref(ty: &crate::types::TypeRef) -> Option<mir::ScalarType> {
    integer_type_ref(ty)
        .map(mir::ScalarType::Integer)
        .or_else(|| float_type_ref(ty).map(mir::ScalarType::Float))
        .or_else(|| is_plain_type(ty, "bool").then_some(mir::ScalarType::Bool))
}

fn is_plain_type(ty: &crate::types::TypeRef, name: &str) -> bool {
    !ty.nullable && ty.name == name && ty.args.is_empty()
}

fn is_nullable_string_type(ty: &crate::types::TypeRef) -> bool {
    ty.nullable && ty.name == "string" && ty.args.is_empty()
}

fn lower_function(
    function: &hir::FunctionDecl,
    signature: FunctionSignature,
    signatures: &HashMap<String, FunctionSignature>,
    semantic_info: &SemanticInfo,
) -> DiagnosticResult<mir::Function> {
    let mut context = LoweringContext::new(signatures.clone(), semantic_info);
    let params = function
        .params
        .iter()
        .zip(signature.parameter_types.iter().copied())
        .map(|(param, ty)| context.declare_user_local(&param.name, param.writable, ty))
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
            return Err(vec![Diagnostic::new(
                "I1101",
                format!(
                    "internal compiler consistency error: checked int function `{function_name}` reaches MIR fallthrough"
                ),
                body.span,
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
            break;
        }

        match statement {
            hir::Stmt::Echo { expr, .. } => {
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
                    if name == "panic" {
                        let message = lower_panic_message(args, *call_span, context)?;
                        context.terminate_current(mir::Terminator::Panic(message));
                    } else if name == "printf" {
                        context.push_statement(mir::Statement::Printf(lower_format_expression(
                            args, *call_span, context,
                        )?));
                    } else if name == "write_file" {
                        let [path, contents] = args.as_slice() else {
                            return Err(vec![unsupported(
                                *call_span,
                                "write_file expects 2 arguments",
                            )]);
                        };
                        context.push_statement(mir::Statement::WriteFile {
                            path: lower_string_expression(path, context)?,
                            contents: lower_string_expression(contents, context)?,
                        });
                    } else if name == "write_stderr" {
                        let [value] = args.as_slice() else {
                            return Err(vec![unsupported(
                                *call_span,
                                "write_stderr expects 1 argument",
                            )]);
                        };
                        context.push_statement(mir::Statement::WriteStderr(
                            lower_string_expression(value, context)?,
                        ));
                    } else {
                        let call = lower_void_call(name, args, *call_span, context)?;
                        context.push_statement(call);
                    }
                } else {
                    return Err(vec![unsupported(
                        *span,
                        "expression statements other than void free-function calls are not lowered to MIR in Stage 11",
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
    context.current_block = context.is_reachable(continuation).then_some(continuation);
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
    context.terminate_condition(condition, then_block, else_block);

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
    context.terminate_condition(condition, body_block, exit_block);

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
    context.current_block = context.is_reachable(exit_block).then_some(exit_block);
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
        .unwrap_or(mir::BoolExpression::Use {
            operand: mir::Operand::Scalar(mir::ScalarValue::Bool(true)),
        });
    context.terminate_condition(condition, body_block, exit_block);

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
    context.current_block = context.is_reachable(exit_block).then_some(exit_block);
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
            "integer range foreach key bindings are not lowered to MIR in Stage 11",
        )]);
    }

    let Some((start, end, inclusive)) = grouped_range_parts(&foreach.iterable) else {
        return Err(vec![unsupported(
            foreach.iterable.span(),
            "collection and general iterable foreach are not lowered to MIR in Stage 11; only integer ranges are supported",
        )]);
    };

    if let Some(ty) = &foreach.value.ty {
        if integer_type_ref(ty).is_none() {
            return Err(vec![unsupported(
                foreach.span,
                format!("integer range foreach bindings require an integer type; got `{ty}`"),
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
    let integer_type = context.integer_type(start)?;
    let end_type = context.integer_type(end)?;
    if end_type != integer_type {
        return Err(vec![Diagnostic::new(
            "I1301",
            "internal compiler consistency error: checked range endpoints have different integer types",
            foreach.span,
        )]);
    }

    let start_value = lower_integer_expression(start, context)?;
    ensure_expression_type(&start_value, integer_type, start.span())?;
    let current_local = context.declare_temp(true, integer_type);
    context.push_statement(mir::Statement::AssignLocal {
        target: current_local,
        value: mir::Rvalue::Value(mir::ValueExpression::Integer(start_value)),
    });

    let end_value = lower_integer_expression(end, context)?;
    ensure_expression_type(&end_value, integer_type, end.span())?;
    let end_local = context.declare_temp(false, integer_type);
    context.push_statement(mir::Statement::AssignLocal {
        target: end_local,
        value: mir::Rvalue::Value(mir::ValueExpression::Integer(end_value)),
    });

    let header_block = context.create_block();
    let body_block = context.create_block();
    let update_block = context.create_block();
    let increment_block = inclusive.then(|| context.create_block());
    let exit_block = context.create_block();

    context.terminate_current(mir::Terminator::Jump(header_block));
    context.current_block = Some(header_block);
    context.terminate_current(mir::Terminator::Branch {
        condition: mir::BoolExpression::Compare {
            op: if inclusive {
                mir::CompareOp::LessEqual
            } else {
                mir::CompareOp::Less
            },
            left: Box::new(mir::ValueExpression::Integer(local_integer_expression(
                current_local,
                integer_type,
            ))),
            right: Box::new(mir::ValueExpression::Integer(local_integer_expression(
                end_local,
                integer_type,
            ))),
        },
        then_block: body_block,
        else_block: exit_block,
    });

    let binding_local = context.declare_user_local(
        &foreach.value.name,
        false,
        mir::Type::Scalar(mir::ScalarType::Integer(integer_type)),
    );
    context.push_loop_targets(LoopTargets {
        continue_block: update_block,
        break_block: exit_block,
    });
    context.current_block = Some(body_block);
    context.push_statement(mir::Statement::AssignLocal {
        target: binding_local,
        value: mir::Rvalue::Value(mir::ValueExpression::Integer(local_integer_expression(
            current_local,
            integer_type,
        ))),
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
            condition: mir::BoolExpression::Compare {
                op: mir::CompareOp::Equal,
                left: Box::new(mir::ValueExpression::Integer(local_integer_expression(
                    current_local,
                    integer_type,
                ))),
                right: Box::new(mir::ValueExpression::Integer(local_integer_expression(
                    end_local,
                    integer_type,
                ))),
            },
            then_block: exit_block,
            else_block: increment_block,
        });
        context.current_block = Some(increment_block);
    }
    context.push_statement(mir::Statement::AssignLocal {
        target: current_local,
        value: mir::Rvalue::Value(mir::ValueExpression::Integer(
            mir::IntegerExpression::Binary {
                ty: integer_type,
                op: mir::IntegerBinaryOp::Add,
                left: Box::new(local_integer_expression(current_local, integer_type)),
                right: Box::new(mir::IntegerExpression::constant(IntegerValue::one(
                    integer_type,
                ))),
            },
        )),
    });
    context.terminate_current(mir::Terminator::Jump(header_block));
    context.current_block = context.is_reachable(exit_block).then_some(exit_block);
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
            format!("{keyword} requires an enclosing loop in MIR Stage 11"),
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

struct LoweringContext<'semantic> {
    signatures: HashMap<String, FunctionSignature>,
    semantic_info: &'semantic SemanticInfo,
    locals: Vec<mir::Local>,
    local_scopes: Vec<HashMap<String, mir::LocalId>>,
    temp_counter: usize,
    blocks: Vec<BlockBuilder>,
    reachable_blocks: Vec<bool>,
    current_block: Option<mir::BlockId>,
    loop_targets: Vec<LoopTargets>,
}

impl<'semantic> LoweringContext<'semantic> {
    fn new(
        signatures: HashMap<String, FunctionSignature>,
        semantic_info: &'semantic SemanticInfo,
    ) -> Self {
        Self {
            signatures,
            semantic_info,
            locals: Vec::new(),
            local_scopes: vec![HashMap::new()],
            temp_counter: 0,
            blocks: vec![BlockBuilder {
                id: mir::BlockId(0),
                statements: Vec::new(),
                terminator: None,
            }],
            reachable_blocks: vec![true],
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
                terminator: block.terminator.unwrap_or(mir::Terminator::Unreachable),
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
        self.reachable_blocks.push(false);
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
        if self.is_reachable(block) {
            for target in terminator_targets(&terminator) {
                self.reachable_blocks[target.0] = true;
            }
        }
        let slot = &mut self.blocks[block.0].terminator;
        assert!(slot.is_none(), "MIR block terminated more than once");
        *slot = Some(terminator);
    }

    fn terminate_condition(
        &mut self,
        condition: mir::BoolExpression,
        then_block: mir::BlockId,
        else_block: mir::BlockId,
    ) {
        match condition {
            mir::BoolExpression::Use {
                operand: mir::Operand::Scalar(mir::ScalarValue::Bool(true)),
            } => {
                self.terminate_current(mir::Terminator::Jump(then_block));
            }
            mir::BoolExpression::Use {
                operand: mir::Operand::Scalar(mir::ScalarValue::Bool(false)),
            } => {
                self.terminate_current(mir::Terminator::Jump(else_block));
            }
            condition => self.terminate_current(mir::Terminator::Branch {
                condition,
                then_block,
                else_block,
            }),
        }
    }

    fn is_reachable(&self, block: mir::BlockId) -> bool {
        self.reachable_blocks.get(block.0).copied().unwrap_or(false)
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

    fn declare_temp(&mut self, writable: bool, ty: IntegerType) -> mir::LocalId {
        let id = mir::LocalId(self.locals.len());
        let name = format!("_tmp{}", self.temp_counter);
        self.temp_counter += 1;
        self.locals.push(mir::Local {
            id,
            name,
            ty: mir::Type::Scalar(mir::ScalarType::Integer(ty)),
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
                    format!("local `${name}` is not available in MIR Stage 11"),
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
        if matches!(
            self.local_type(local),
            mir::Type::Scalar(mir::ScalarType::Integer(_))
        ) {
            Ok(local)
        } else {
            Err(vec![unsupported(
                span,
                format!("string local `${name}` cannot be used as an int expression in Stage 11"),
            )])
        }
    }

    fn lookup_function(&self, name: &str, span: Span) -> DiagnosticResult<FunctionSignature> {
        self.signatures.get(name).cloned().ok_or_else(|| {
            vec![unsupported(
                span,
                format!("call references unknown top-level function `{name}`"),
            )]
        })
    }

    fn integer_type(&self, expr: &hir::Expr) -> DiagnosticResult<IntegerType> {
        self.semantic_info
            .integer_type(expr.span())
            .ok_or_else(|| {
                vec![Diagnostic::new(
                    "I1301",
                    "internal compiler consistency error: checked integer expression has no canonical Stage 13 type",
                    expr.span(),
                )]
            })
    }

    fn float_type(&self, expr: &hir::Expr) -> DiagnosticResult<FloatType> {
        self.semantic_info.float_type(expr.span()).ok_or_else(|| {
            vec![Diagnostic::new(
                "I1401",
                "internal compiler consistency error: checked float expression has no canonical Stage 14 type",
                expr.span(),
            )]
        })
    }

    fn local_integer_type(&self, id: mir::LocalId) -> DiagnosticResult<IntegerType> {
        match self.local_type(id) {
            mir::Type::Scalar(mir::ScalarType::Integer(ty)) => Ok(ty),
            _ => Err(vec![Diagnostic::new(
                "I1301",
                format!(
                    "internal compiler consistency error: non-integer local local{} used as an integer",
                    id.0
                ),
                Span::default(),
            )]),
        }
    }

    fn local_scalar_type(&self, id: mir::LocalId) -> DiagnosticResult<mir::ScalarType> {
        match self.local_type(id) {
            mir::Type::Scalar(ty) => Ok(ty),
            mir::Type::String | mir::Type::NullableString => Err(vec![Diagnostic::new(
                "I1401",
                format!(
                    "internal compiler consistency error: string local local{} used as a scalar",
                    id.0
                ),
                Span::default(),
            )]),
        }
    }
}

fn terminator_targets(terminator: &mir::Terminator) -> Vec<mir::BlockId> {
    match terminator {
        mir::Terminator::Jump(target) => vec![*target],
        mir::Terminator::Branch {
            then_block,
            else_block,
            ..
        } => vec![*then_block, *else_block],
        mir::Terminator::Return(_)
        | mir::Terminator::ReturnVoid
        | mir::Terminator::Panic(_)
        | mir::Terminator::Unreachable => Vec::new(),
    }
}

fn lower_var_decl(decl: &hir::VarDecl, context: &mut LoweringContext) -> DiagnosticResult<()> {
    let ty = match &decl.ty {
        Some(ty) if scalar_type_ref(ty).is_some() => {
            mir::Type::Scalar(scalar_type_ref(ty).expect("checked scalar type"))
        }
        Some(ty) if is_plain_type(ty, "string") => mir::Type::String,
        Some(ty) if is_nullable_string_type(ty) => mir::Type::NullableString,
        Some(ty) => {
            return Err(vec![unsupported(
                decl.span,
                format!("only scalar and readonly string locals are lowered to MIR in Stage 14; got `{ty}`"),
            )]);
        }
        None if is_string_local_initializer(&decl.initializer, context) => mir::Type::String,
        None if is_nullable_string_initializer(&decl.initializer, context) => {
            mir::Type::NullableString
        }
        None => mir::Type::Scalar(lower_value_expression(&decl.initializer, context)?.ty()),
    };

    if ty == mir::Type::String {
        return lower_string_var_decl(decl, context);
    }
    if ty == mir::Type::NullableString {
        let value = lower_nullable_string_expression(&decl.initializer, context)?;
        let local = context.declare_user_local(&decl.name, decl.writable, ty);
        context.push_statement(mir::Statement::AssignLocal {
            target: local,
            value: mir::Rvalue::NullableString(value),
        });
        return Ok(());
    }

    let mir::Type::Scalar(scalar_type) = ty else {
        unreachable!("string locals return through lower_string_var_decl")
    };
    let value = lower_value_expression(&decl.initializer, context)?;
    ensure_value_type(&value, scalar_type, decl.initializer.span())?;
    let local =
        context.declare_user_local(&decl.name, decl.writable, mir::Type::Scalar(scalar_type));
    context.push_statement(mir::Statement::AssignLocal {
        target: local,
        value: mir::Rvalue::Value(value),
    });
    Ok(())
}

fn is_nullable_string_initializer(expr: &hir::Expr, context: &LoweringContext) -> bool {
    match expr {
        hir::Expr::Null { .. } => true,
        hir::Expr::Grouped { expr, .. } => is_nullable_string_initializer(expr, context),
        hir::Expr::Variable { name, span } => context
            .lookup_local(name, *span)
            .is_ok_and(|local| context.local_type(local) == mir::Type::NullableString),
        hir::Expr::FunctionCall { name, span, .. } if name == "read_line" => true,
        hir::Expr::FunctionCall { name, span, .. } => {
            context.lookup_function(name, *span).is_ok_and(|signature| {
                signature.return_type == mir::ReturnType::Value(mir::Type::NullableString)
            })
        }
        _ => false,
    }
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
        hir::Expr::FunctionCall { name, .. }
            if matches!(name.as_str(), "sprintf" | "read_file") =>
        {
            true
        }
        hir::Expr::FunctionCall { name, span, .. } => {
            context.lookup_function(name, *span).is_ok_and(|signature| {
                signature.return_type == mir::ReturnType::Value(mir::Type::String)
            })
        }
        _ => false,
    }
}

fn lower_string_var_decl(
    decl: &hir::VarDecl,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    let value = lower_string_expression(&decl.initializer, context)?;
    let local = context.declare_user_local(&decl.name, decl.writable, mir::Type::String);
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
        if assignment.op != hir::AssignOp::Assign {
            return Err(vec![unsupported(
                assignment.span,
                "string compound assignment is invalid",
            )]);
        }
        context.push_statement(mir::Statement::AssignLocal {
            target,
            value: mir::Rvalue::String(lower_string_expression(&assignment.value, context)?),
        });
        return Ok(());
    }
    if context.local_type(target) == mir::Type::NullableString {
        if assignment.op != hir::AssignOp::Assign {
            return Err(vec![unsupported(
                assignment.span,
                "nullable-string compound assignment is invalid",
            )]);
        }
        context.push_statement(mir::Statement::AssignLocal {
            target,
            value: mir::Rvalue::NullableString(lower_nullable_string_expression(
                &assignment.value,
                context,
            )?),
        });
        return Ok(());
    }

    let scalar_type = context.local_scalar_type(target)?;
    let value = match assignment.op {
        hir::AssignOp::Assign => lower_value_expression(&assignment.value, context)?,
        ref op => lower_compound_value(target, scalar_type, op, &assignment.value, context)?,
    };
    ensure_value_type(&value, scalar_type, assignment.value.span())?;
    context.push_statement(mir::Statement::AssignLocal {
        target,
        value: mir::Rvalue::Value(value),
    });
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
            "string increment and decrement are not lowered to MIR in Stage 11",
        )]);
    }

    let scalar_type = context.local_scalar_type(target)?;
    let value = match scalar_type {
        mir::ScalarType::Integer(integer_type) => {
            let op = match increment.op {
                hir::IncrementOp::Increment => mir::IntegerBinaryOp::Add,
                hir::IncrementOp::Decrement => mir::IntegerBinaryOp::Subtract,
            };
            mir::ValueExpression::Integer(mir::IntegerExpression::Binary {
                ty: integer_type,
                op,
                left: Box::new(local_integer_expression(target, integer_type)),
                right: Box::new(mir::IntegerExpression::constant(IntegerValue::one(
                    integer_type,
                ))),
            })
        }
        mir::ScalarType::Float(float_type) => {
            let op = match increment.op {
                hir::IncrementOp::Increment => mir::FloatBinaryOp::Add,
                hir::IncrementOp::Decrement => mir::FloatBinaryOp::Subtract,
            };
            let one = match float_type {
                FloatType::Float32 => FloatValue::from_f32(1.0),
                FloatType::Float64 => FloatValue::from_f64(1.0),
            };
            mir::ValueExpression::Float(mir::FloatExpression::Binary {
                ty: float_type,
                op,
                left: Box::new(local_float_expression(target, float_type)),
                right: Box::new(mir::FloatExpression::constant(one)),
            })
        }
        mir::ScalarType::Bool => {
            return Err(vec![unsupported(
                increment.span,
                "bool increment is invalid",
            )])
        }
    };
    context.push_statement(mir::Statement::AssignLocal {
        target,
        value: mir::Rvalue::Value(value),
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
            "only local variable assignment targets are lowered to MIR in Stage 11",
        )]),
    }
}

fn lower_echo(expr: &hir::Expr, context: &LoweringContext) -> DiagnosticResult<mir::Statement> {
    match expr {
        hir::Expr::String { value, .. } => Ok(mir::Statement::EchoStringLiteral(value.clone())),
        _ => lower_display_string_expression(expr, context).map(mir::Statement::EchoString),
    }
}

fn lower_panic_message(
    args: &[hir::Expr],
    span: Span,
    context: &LoweringContext,
) -> DiagnosticResult<mir::StringExpression> {
    let [message] = args else {
        return Err(vec![unsupported(
            span,
            format!("panic expects exactly 1 argument, got {}", args.len()),
        )]);
    };
    lower_string_expression(message, context)
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
            } else if context.local_type(local) == mir::Type::NullableString {
                Ok(mir::StringExpression::NullableLocalAssumeNonNull(local))
            } else {
                Err(vec![unsupported(
                    *span,
                    "string expressions may reference only readonly string locals in Stage 11",
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
        hir::Expr::InterpolatedString { parts, .. } => {
            let mut lowered = Vec::new();
            for part in parts {
                match part {
                    hir::InterpolatedStringPart::Text { value: text, .. } => {
                        lowered.push(mir::StringExpression::Literal(text.clone()));
                    }
                    hir::InterpolatedStringPart::Expr(expr) => {
                        lowered.push(lower_display_string_expression(expr, context)?);
                    }
                }
            }
            Ok(mir::StringExpression::Concat(lowered))
        }
        hir::Expr::FunctionCall { name, args, span } => {
            if name == "read_file" {
                let [path] = args.as_slice() else {
                    return Err(vec![unsupported(*span, "read_file expects 1 argument")]);
                };
                return Ok(mir::StringExpression::ReadFile(Box::new(
                    lower_string_expression(path, context)?,
                )));
            }
            if name == "sprintf" {
                return Ok(mir::StringExpression::Format(Box::new(
                    lower_format_expression(args, *span, context)?,
                )));
            }
            let signature = context.lookup_function(name, *span)?;
            if signature.return_type != mir::ReturnType::Value(mir::Type::String) {
                return Err(vec![unsupported(*span, format!("function `{name}` does not return string"))]);
            }
            Ok(mir::StringExpression::Call {
                function: signature.id,
                args: lower_call_args(name, args, signature, *span, context)?,
            })
        }
        _ => Err(vec![unsupported(
            expr.span(),
            "echo supports only string literals, readonly string locals, and string concatenation in Stage 11",
        )]),
    }
}

fn lower_nullable_string_expression(
    expr: &hir::Expr,
    context: &LoweringContext,
) -> DiagnosticResult<mir::NullableStringExpression> {
    match expr {
        hir::Expr::Null { .. } => Ok(mir::NullableStringExpression::Null),
        hir::Expr::Grouped { expr, .. } => lower_nullable_string_expression(expr, context),
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            match context.local_type(local) {
                mir::Type::NullableString => Ok(mir::NullableStringExpression::Local(local)),
                mir::Type::String => Ok(mir::NullableStringExpression::String(
                    mir::StringExpression::Local(local),
                )),
                _ => Err(vec![unsupported(
                    *span,
                    "expected nullable string expression",
                )]),
            }
        }
        hir::Expr::FunctionCall { name, args, span } if name == "read_line" => {
            if !args.is_empty() {
                return Err(vec![unsupported(*span, "read_line expects no arguments")]);
            }
            Ok(mir::NullableStringExpression::ReadLine)
        }
        hir::Expr::FunctionCall { name, args, span } => {
            let signature = context.lookup_function(name, *span)?;
            if signature.return_type != mir::ReturnType::Value(mir::Type::NullableString) {
                return Err(vec![unsupported(
                    *span,
                    format!("function `{name}` does not return ?string"),
                )]);
            }
            Ok(mir::NullableStringExpression::Call {
                function: signature.id,
                args: lower_call_args(name, args, signature, *span, context)?,
            })
        }
        _ if is_string_local_initializer(expr, context) => Ok(
            mir::NullableStringExpression::String(lower_string_expression(expr, context)?),
        ),
        _ => Err(vec![unsupported(
            expr.span(),
            "expected nullable string expression",
        )]),
    }
}

fn lower_format_expression(
    args: &[hir::Expr],
    span: Span,
    context: &LoweringContext,
) -> DiagnosticResult<mir::FormatExpression> {
    let Some(hir::Expr::String {
        value,
        span: format_span,
    }) = args.first()
    else {
        return Err(vec![unsupported(
            span,
            "format must be a direct string literal",
        )]);
    };
    let pieces = format_string::parse(value, *format_span).map_err(|error| vec![error])?;
    let specs = pieces.iter().filter_map(|piece| match piece {
        FormatPiece::Argument { spec, .. } => Some(*spec),
        FormatPiece::Literal(_) => None,
    });
    let arguments = args[1..]
        .iter()
        .zip(specs)
        .map(|(argument, spec)| {
            if spec.conversion == FormatConversion::Display {
                lower_display_string_expression(argument, context).map(mir::FormatArgument::String)
            } else if is_string_local_initializer(argument, context) {
                lower_string_expression(argument, context).map(mir::FormatArgument::String)
            } else {
                lower_value_expression(argument, context).map(mir::FormatArgument::Value)
            }
        })
        .collect::<DiagnosticResult<Vec<_>>>()?;
    Ok(mir::FormatExpression { pieces, arguments })
}

fn lower_display_string_expression(
    expr: &hir::Expr,
    context: &LoweringContext,
) -> DiagnosticResult<mir::StringExpression> {
    let narrowed_nullable_local = matches!(expr, hir::Expr::Variable { name, span }
        if context.lookup_local(name, *span).is_ok_and(|local| context.local_type(local) == mir::Type::NullableString));
    if is_string_local_initializer(expr, context) || narrowed_nullable_local {
        lower_string_expression(expr, context)
    } else {
        lower_value_expression(expr, context).map(mir::StringExpression::Display)
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
            if context.local_type(local) == mir::Type::String {
                parts.push(mir::StringExpression::Local(local));
            } else if context.local_type(local) == mir::Type::NullableString {
                parts.push(mir::StringExpression::NullableLocalAssumeNonNull(local));
            } else {
                parts.push(mir::StringExpression::Display(lower_value_expression(
                    expr, context,
                )?));
            }
            Ok(())
        }
        _ => {
            parts.push(lower_display_string_expression(expr, context)?);
            Ok(())
        }
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
            format!("non-void function `{name}` cannot be used as a statement in MIR Stage 11"),
        )]);
    }

    Ok(mir::Statement::CallVoid {
        function: signature.id,
        args: lower_call_args(name, args, signature, span, context)?,
    })
}

fn lower_integer_call(
    name: &str,
    args: &[hir::Expr],
    span: Span,
    context: &LoweringContext,
) -> DiagnosticResult<(mir::FunctionId, IntegerType, Vec<mir::Rvalue>)> {
    let signature = context.lookup_function(name, span)?;
    let mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Integer(return_type))) =
        signature.return_type
    else {
        return Err(vec![unsupported(
            span,
            format!(
                "void function `{name}` cannot be used as an integer expression in MIR Stage 11"
            ),
        )]);
    };

    let function = signature.id;
    let args = lower_call_args(name, args, signature, span, context)?;
    Ok((function, return_type, args))
}

fn lower_call_args(
    name: &str,
    args: &[hir::Expr],
    signature: FunctionSignature,
    span: Span,
    context: &LoweringContext,
) -> DiagnosticResult<Vec<mir::Rvalue>> {
    if args.len() != signature.parameter_types.len() {
        return Err(vec![unsupported(
            span,
            format!(
                "function `{name}` expects {} positional argument(s), got {}",
                signature.parameter_types.len(),
                args.len()
            ),
        )]);
    }

    args.iter()
        .zip(signature.parameter_types)
        .map(|(arg, expected)| {
            let lowered = lower_rvalue_as_expected(arg, expected, context)?;
            if lowered.ty() != expected {
                return Err(vec![Diagnostic::new(
                    "I1301",
                    format!(
                        "internal compiler consistency error: argument to `{name}` has MIR type `{}`, expected `{expected}`",
                        lowered.ty()
                    ),
                    arg.span(),
                )]);
            }
            Ok(lowered)
        })
        .collect()
}

fn lower_return(
    expr: Option<&hir::Expr>,
    span: Span,
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::Terminator> {
    match (return_type, expr) {
        (mir::ReturnType::Void, None) => Ok(mir::Terminator::ReturnVoid),
        (mir::ReturnType::Value(expected), Some(expr)) => {
            let value = lower_rvalue_as_expected(expr, expected, context)?;
            if value.ty() != expected {
                return Err(vec![Diagnostic::new(
                    "I1301",
                    format!(
                        "internal compiler consistency error: return expression has MIR type `{}`, expected `{expected}`",
                        value.ty()
                    ),
                    expr.span(),
                )]);
            }
            Ok(mir::Terminator::Return(value))
        }
        (mir::ReturnType::Value(_), None) => Err(vec![unsupported(
            span,
            "bare return is not lowered for scalar-returning functions in Stage 14",
        )]),
        (mir::ReturnType::Void, Some(expr)) => Err(vec![unsupported(
            expr.span(),
            "return values are not lowered for void functions in Stage 11",
        )]),
    }
}

fn lower_condition(
    expr: &hir::Expr,
    context: &LoweringContext,
) -> DiagnosticResult<mir::BoolExpression> {
    match expr {
        hir::Expr::Bool { value, .. } => Ok(mir::BoolExpression::Use {
            operand: mir::Operand::Scalar(mir::ScalarValue::Bool(*value)),
        }),
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            if context.local_type(local) != mir::Type::Scalar(mir::ScalarType::Bool) {
                return Err(vec![unsupported(
                    *span,
                    "only bool locals may be used as conditions",
                )]);
            }
            Ok(mir::BoolExpression::Use {
                operand: mir::Operand::Local(local),
            })
        }
        hir::Expr::Grouped { expr, .. } => lower_condition(expr, context),
        hir::Expr::Unary {
            op: hir::UnaryOp::Not,
            expr,
            ..
        } => Ok(mir::BoolExpression::Not(Box::new(lower_condition(
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
            | hir::BinaryOp::GreaterEqual => {
                if is_string_local_initializer(left, context)
                    || is_string_local_initializer(right, context)
                {
                    Ok(mir::BoolExpression::StringCompare {
                        op: lower_compare_op(op),
                        left: Box::new(lower_string_expression(left, context)?),
                        right: Box::new(lower_string_expression(right, context)?),
                    })
                } else if is_nullable_string_initializer(left, context)
                    || is_nullable_string_initializer(right, context)
                {
                    Ok(mir::BoolExpression::NullableStringCompare {
                        op: lower_compare_op(op),
                        left: Box::new(lower_nullable_string_expression(left, context)?),
                        right: Box::new(lower_nullable_string_expression(right, context)?),
                    })
                } else {
                    Ok(mir::BoolExpression::Compare {
                        op: lower_compare_op(op),
                        left: Box::new(lower_value_expression(left, context)?),
                        right: Box::new(lower_value_expression(right, context)?),
                    })
                }
            }
            hir::BinaryOp::And | hir::BinaryOp::Or | hir::BinaryOp::Xor => {
                Ok(mir::BoolExpression::Binary {
                    op: lower_condition_binary_op(op),
                    left: Box::new(lower_condition(left, context)?),
                    right: Box::new(lower_condition(right, context)?),
                })
            }
            _ => Err(vec![unsupported(
                expr.span(),
                "conditions require bool values, scalar comparisons, or boolean operators in Stage 14",
            )]),
        },
        hir::Expr::FunctionCall { name, args, span } => {
            let signature = context.lookup_function(name, *span)?;
            if signature.return_type
                != mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Bool))
            {
                return Err(vec![unsupported(
                    *span,
                    format!("function `{name}` does not return bool"),
                )]);
            }
            Ok(mir::BoolExpression::Call {
                function: signature.id,
                args: lower_call_args(name, args, signature, *span, context)?,
            })
        }
        hir::Expr::MethodCall { .. } | hir::Expr::StaticCall { .. } => Err(vec![unsupported(
            expr.span(),
            "method calls in conditions are not lowered to Stage 14 MIR",
        )]),
        hir::Expr::Int { .. } => Err(vec![unsupported(
            expr.span(),
            "integer truthiness is not Doria condition semantics; Stage 11 requires a bool condition",
        )]),
        _ => Err(vec![unsupported(
            expr.span(),
            "this condition expression is not lowered to MIR in Stage 11",
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

fn lower_condition_binary_op(op: &hir::BinaryOp) -> mir::BoolBinaryOp {
    match op {
        hir::BinaryOp::And => mir::BoolBinaryOp::And,
        hir::BinaryOp::Or => mir::BoolBinaryOp::Or,
        hir::BinaryOp::Xor => mir::BoolBinaryOp::Xor,
        _ => unreachable!("only boolean operators are lowered as MIR condition operators"),
    }
}

fn lower_value_expression(
    expr: &hir::Expr,
    context: &LoweringContext,
) -> DiagnosticResult<mir::ValueExpression> {
    if let hir::Expr::FunctionCall { name, span, .. } = expr {
        if context.lookup_function(name, *span)?.return_type == mir::ReturnType::Void {
            return Err(vec![unsupported(
                *span,
                format!("void function `{name}` cannot be used as a scalar expression"),
            )]);
        }
    }
    if context.semantic_info.integer_type(expr.span()).is_some() {
        return lower_integer_expression(expr, context).map(mir::ValueExpression::Integer);
    }
    if context.semantic_info.float_type(expr.span()).is_some() {
        return lower_float_expression(expr, context).map(mir::ValueExpression::Float);
    }
    lower_condition(expr, context).map(mir::ValueExpression::Bool)
}

fn lower_rvalue_as_expected(
    expr: &hir::Expr,
    expected: mir::Type,
    context: &LoweringContext,
) -> DiagnosticResult<mir::Rvalue> {
    match expected {
        mir::Type::String => lower_string_expression(expr, context).map(mir::Rvalue::String),
        mir::Type::NullableString => {
            lower_nullable_string_expression(expr, context).map(mir::Rvalue::NullableString)
        }
        mir::Type::Scalar(_) => lower_value_expression(expr, context).map(mir::Rvalue::Value),
    }
}

fn lower_float_expression(
    expr: &hir::Expr,
    context: &LoweringContext,
) -> DiagnosticResult<mir::FloatExpression> {
    let ty = context.float_type(expr)?;
    match expr {
        hir::Expr::Float { value, .. } => FloatValue::parse_decimal(ty, value)
            .map(mir::FloatExpression::constant)
            .ok_or_else(|| {
                vec![Diagnostic::new(
                    "I1401",
                    format!("checked floating literal does not fit `{ty}`"),
                    expr.span(),
                )]
            }),
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            let expected = mir::Type::Scalar(mir::ScalarType::Float(ty));
            if context.local_type(local) != expected {
                return Err(vec![Diagnostic::new(
                    "I1401",
                    format!("float local `${name}` does not have expected MIR type `{ty}`"),
                    *span,
                )]);
            }
            Ok(local_float_expression(local, ty))
        }
        hir::Expr::Grouped { expr, .. } => lower_float_expression(expr, context),
        hir::Expr::Unary {
            op: hir::UnaryOp::Negate,
            expr,
            ..
        } => Ok(mir::FloatExpression::Negate {
            ty,
            operand: Box::new(lower_float_expression(expr, context)?),
        }),
        hir::Expr::Binary {
            left, op, right, ..
        } => Ok(mir::FloatExpression::Binary {
            ty,
            op: match op {
                hir::BinaryOp::Add => mir::FloatBinaryOp::Add,
                hir::BinaryOp::Sub => mir::FloatBinaryOp::Subtract,
                hir::BinaryOp::Mul => mir::FloatBinaryOp::Multiply,
                hir::BinaryOp::Div => mir::FloatBinaryOp::Divide,
                _ => return Err(vec![unsupported(expr.span(), "invalid float operator")]),
            },
            left: Box::new(lower_float_expression(left, context)?),
            right: Box::new(lower_float_expression(right, context)?),
        }),
        hir::Expr::FunctionCall { name, args, span } => {
            let signature = context.lookup_function(name, *span)?;
            if signature.return_type
                != mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Float(ty)))
            {
                return Err(vec![Diagnostic::new(
                    "I1401",
                    format!("function `{name}` does not return `{ty}`"),
                    *span,
                )]);
            }
            Ok(mir::FloatExpression::Call {
                ty,
                function: signature.id,
                args: lower_call_args(name, args, signature, *span, context)?,
            })
        }
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } if class_name == "Int" && method == "toFloat" => {
            let [value] = args.as_slice() else {
                return Err(vec![Diagnostic::new(
                    "I1401",
                    "checked Int::toFloat call does not have one argument",
                    *span,
                )]);
            };
            Ok(mir::FloatExpression::IntToFloat {
                value: Box::new(lower_integer_expression(value, context)?),
            })
        }
        _ => Err(vec![unsupported(
            expr.span(),
            "this float expression is not lowered to Stage 14 MIR",
        )]),
    }
}

fn lower_integer_expression(
    expr: &hir::Expr,
    context: &LoweringContext,
) -> DiagnosticResult<mir::IntegerExpression> {
    if let hir::Expr::FunctionCall { name, span, .. } = expr {
        if context.lookup_function(name, *span)?.return_type == mir::ReturnType::Void {
            return Err(vec![unsupported(
                *span,
                format!(
                    "void function `{name}` cannot be used as an integer expression in MIR Stage 11"
                ),
            )]);
        }
    }

    if let Some((magnitude, negative)) = integer_literal_parts(expr) {
        let ty = context.integer_type(expr)?;
        let value = IntegerValue::from_literal(ty, magnitude, negative).ok_or_else(|| {
            vec![Diagnostic::new(
                "I1301",
                format!("internal compiler consistency error: checked literal does not fit `{ty}`"),
                expr.span(),
            )]
        })?;
        return Ok(mir::IntegerExpression::constant(value));
    }

    if let hir::Expr::FunctionCall { name, args, span } = expr {
        let (function, return_type, args) = lower_integer_call(name, args, *span, context)?;
        let ty = context.integer_type(expr)?;
        if return_type != ty {
            return Err(vec![Diagnostic::new(
                "I1301",
                format!(
                    "internal compiler consistency error: function `{name}` returns `{return_type}`, expression metadata says `{ty}`"
                ),
                *span,
            )]);
        }
        return Ok(mir::IntegerExpression::Call { ty, function, args });
    }

    let ty = context.integer_type(expr)?;
    match expr {
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_int_local(name, *span)?;
            let local_type = context.local_integer_type(local)?;
            if local_type != ty {
                return Err(vec![Diagnostic::new(
                    "I1301",
                    format!(
                        "internal compiler consistency error: `${name}` has MIR type `{local_type}`, expression metadata says `{ty}`"
                    ),
                    *span,
                )]);
            }
            Ok(local_integer_expression(local, ty))
        }
        hir::Expr::Grouped { expr, .. } => {
            let lowered = lower_integer_expression(expr, context)?;
            ensure_expression_type(&lowered, ty, expr.span())?;
            Ok(lowered)
        }
        hir::Expr::Unary { op, expr, .. } => {
            let operand = lower_integer_expression(expr, context)?;
            ensure_expression_type(&operand, ty, expr.span())?;
            let op = match op {
                hir::UnaryOp::Negate => mir::IntegerUnaryOp::Negate,
                hir::UnaryOp::BitwiseNot => mir::IntegerUnaryOp::BitwiseNot,
                hir::UnaryOp::Not => return Err(vec![unsupported_int_expr(expr)]),
            };
            Ok(mir::IntegerExpression::Unary {
                ty,
                op,
                operand: Box::new(operand),
            })
        }
        hir::Expr::Binary {
            left, op, right, ..
        } => {
            let op = lower_integer_binary_op(op, expr.span())?;
            let left = lower_integer_expression(left, context)?;
            let right = lower_integer_expression(right, context)?;
            ensure_expression_type(&left, ty, expr.span())?;
            ensure_expression_type(&right, ty, expr.span())?;
            Ok(mir::IntegerExpression::Binary {
                ty,
                op,
                left: Box::new(left),
                right: Box::new(right),
            })
        }
        hir::Expr::FunctionCall { .. } => unreachable!("function calls return before type lookup"),
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } if class_name == "Float" && method == "toInt" => {
            let [value] = args.as_slice() else {
                return Err(vec![Diagnostic::new(
                    "I1401",
                    "checked Float::toInt call does not have one argument",
                    *span,
                )]);
            };
            Ok(mir::IntegerExpression::FloatToInt {
                value: Box::new(lower_float_expression(value, context)?),
            })
        }
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } if method == "from" && IntegerType::from_companion_name(class_name).is_some() => {
            let [value] = args.as_slice() else {
                return Err(vec![Diagnostic::new(
                    "I1301",
                    "internal compiler consistency error: checked integer conversion does not have exactly one argument",
                    *span,
                )]);
            };
            let target = IntegerType::from_companion_name(class_name)
                .expect("guarded integer companion name");
            if target != ty {
                return Err(vec![Diagnostic::new(
                    "I1301",
                    format!(
                        "internal compiler consistency error: `{class_name}::from` targets `{target}`, expression metadata says `{ty}`"
                    ),
                    *span,
                )]);
            }
            Ok(mir::IntegerExpression::Convert {
                ty,
                value: Box::new(lower_integer_expression(value, context)?),
            })
        }
        hir::Expr::Int { .. } => unreachable!("integer literal handled before expression match"),
        _ => Err(vec![unsupported_int_expr(expr)]),
    }
}

fn lower_integer_binary_op(
    op: &hir::BinaryOp,
    span: Span,
) -> DiagnosticResult<mir::IntegerBinaryOp> {
    match op {
        hir::BinaryOp::Add => Ok(mir::IntegerBinaryOp::Add),
        hir::BinaryOp::Sub => Ok(mir::IntegerBinaryOp::Subtract),
        hir::BinaryOp::Mul => Ok(mir::IntegerBinaryOp::Multiply),
        hir::BinaryOp::Div => Ok(mir::IntegerBinaryOp::Divide),
        hir::BinaryOp::Mod => Ok(mir::IntegerBinaryOp::Remainder),
        hir::BinaryOp::ShiftLeft => Ok(mir::IntegerBinaryOp::ShiftLeft),
        hir::BinaryOp::ShiftRight => Ok(mir::IntegerBinaryOp::ShiftRight),
        hir::BinaryOp::BitwiseAnd => Ok(mir::IntegerBinaryOp::BitwiseAnd),
        hir::BinaryOp::BitwiseXor => Ok(mir::IntegerBinaryOp::BitwiseXor),
        hir::BinaryOp::BitwiseOr => Ok(mir::IntegerBinaryOp::BitwiseOr),
        hir::BinaryOp::Less
        | hir::BinaryOp::LessEqual
        | hir::BinaryOp::Greater
        | hir::BinaryOp::GreaterEqual
        | hir::BinaryOp::Equal
        | hir::BinaryOp::NotEqual => Err(vec![unsupported(
            span,
            "comparison results are condition-only and are not lowered as runtime values in MIR Stage 11",
        )]),
        hir::BinaryOp::Concat => Err(vec![unsupported(
            span,
            "string concatenation is not lowered to MIR in Stage 11",
        )]),
        hir::BinaryOp::And | hir::BinaryOp::Or | hir::BinaryOp::Xor => Err(vec![unsupported(
            span,
            "boolean operator reached integer-only MIR lowering",
        )]),
        hir::BinaryOp::Coalesce => Err(vec![unsupported(
            span,
            "null coalescing is not lowered to MIR in Stage 11",
        )]),
    }
}

fn lower_compound_assignment_op(op: &hir::AssignOp) -> mir::IntegerBinaryOp {
    match op {
        hir::AssignOp::AddAssign => mir::IntegerBinaryOp::Add,
        hir::AssignOp::SubAssign => mir::IntegerBinaryOp::Subtract,
        hir::AssignOp::MulAssign => mir::IntegerBinaryOp::Multiply,
        hir::AssignOp::DivAssign => mir::IntegerBinaryOp::Divide,
        hir::AssignOp::ModAssign => mir::IntegerBinaryOp::Remainder,
        hir::AssignOp::ShiftLeftAssign => mir::IntegerBinaryOp::ShiftLeft,
        hir::AssignOp::ShiftRightAssign => mir::IntegerBinaryOp::ShiftRight,
        hir::AssignOp::BitwiseAndAssign => mir::IntegerBinaryOp::BitwiseAnd,
        hir::AssignOp::BitwiseXorAssign => mir::IntegerBinaryOp::BitwiseXor,
        hir::AssignOp::BitwiseOrAssign => mir::IntegerBinaryOp::BitwiseOr,
        hir::AssignOp::Assign => unreachable!("plain assignment does not have a binary operator"),
    }
}

fn lower_compound_value(
    target: mir::LocalId,
    ty: mir::ScalarType,
    op: &hir::AssignOp,
    right: &hir::Expr,
    context: &LoweringContext,
) -> DiagnosticResult<mir::ValueExpression> {
    match ty {
        mir::ScalarType::Integer(integer) => {
            let right_span = right.span();
            let right = lower_integer_expression(right, context)?;
            ensure_expression_type(&right, integer, right_span)?;
            Ok(mir::ValueExpression::Integer(
                mir::IntegerExpression::Binary {
                    ty: integer,
                    op: lower_compound_assignment_op(op),
                    left: Box::new(local_integer_expression(target, integer)),
                    right: Box::new(right),
                },
            ))
        }
        mir::ScalarType::Float(float) => {
            let right = lower_float_expression(right, context)?;
            let op = match op {
                hir::AssignOp::AddAssign => mir::FloatBinaryOp::Add,
                hir::AssignOp::SubAssign => mir::FloatBinaryOp::Subtract,
                hir::AssignOp::MulAssign => mir::FloatBinaryOp::Multiply,
                hir::AssignOp::DivAssign => mir::FloatBinaryOp::Divide,
                _ => {
                    return Err(vec![unsupported(
                        Span::default(),
                        "invalid float compound assignment",
                    )])
                }
            };
            Ok(mir::ValueExpression::Float(mir::FloatExpression::Binary {
                ty: float,
                op,
                left: Box::new(local_float_expression(target, float)),
                right: Box::new(right),
            }))
        }
        mir::ScalarType::Bool => Err(vec![unsupported(
            Span::default(),
            "bool compound assignment is invalid",
        )]),
    }
}

fn local_integer_expression(local: mir::LocalId, ty: IntegerType) -> mir::IntegerExpression {
    mir::IntegerExpression::use_operand(ty, mir::Operand::Local(local))
}

fn local_float_expression(local: mir::LocalId, ty: FloatType) -> mir::FloatExpression {
    mir::FloatExpression::Use {
        ty,
        operand: mir::Operand::Local(local),
    }
}

fn ensure_value_type(
    expression: &mir::ValueExpression,
    expected: mir::ScalarType,
    span: Span,
) -> DiagnosticResult<()> {
    if expression.ty() == expected {
        Ok(())
    } else {
        Err(vec![Diagnostic::new(
            "I1401",
            format!(
                "internal compiler consistency error: scalar expression has MIR type `{}`, expected `{expected}`",
                expression.ty()
            ),
            span,
        )])
    }
}

fn ensure_expression_type(
    expression: &mir::IntegerExpression,
    expected: IntegerType,
    span: Span,
) -> DiagnosticResult<()> {
    if expression.ty() == expected {
        Ok(())
    } else {
        Err(vec![Diagnostic::new(
            "I1301",
            format!(
                "internal compiler consistency error: integer expression has MIR type `{}`, expected `{expected}`",
                expression.ty()
            ),
            span,
        )])
    }
}

fn integer_literal_parts(expr: &hir::Expr) -> Option<(u128, bool)> {
    match expr {
        hir::Expr::Int { value, .. } => parse_decimal_magnitude(value).map(|value| (value, false)),
        hir::Expr::Grouped { expr, .. } => integer_literal_parts(expr),
        hir::Expr::Unary {
            op: hir::UnaryOp::Negate,
            expr,
            ..
        } => unsigned_integer_literal_magnitude(expr).map(|magnitude| (magnitude, true)),
        _ => None,
    }
}

fn unsigned_integer_literal_magnitude(expr: &hir::Expr) -> Option<u128> {
    match expr {
        hir::Expr::Int { value, .. } => parse_decimal_magnitude(value),
        hir::Expr::Grouped { expr, .. } => unsigned_integer_literal_magnitude(expr),
        _ => None,
    }
}

fn unsupported_int_expr(expr: &hir::Expr) -> Diagnostic {
    let detail = match expr {
        hir::Expr::String { .. } | hir::Expr::InterpolatedString { .. } => {
            "string expressions are not lowered to MIR in Stage 11"
        }
        hir::Expr::Float { .. } => "float expressions are not lowered to MIR in Stage 11",
        hir::Expr::Bool { .. } => "bool value reached integer-only MIR lowering",
        hir::Expr::Null { .. } => "null values are not lowered to MIR in Stage 11",
        hir::Expr::Array { .. } => "collections are not lowered to MIR in Stage 11",
        hir::Expr::FunctionCall { .. } => "function calls are not lowered to MIR in Stage 11",
        hir::Expr::MethodCall { .. } | hir::Expr::StaticCall { .. } => {
            "method calls are not lowered to MIR in Stage 11"
        }
        hir::Expr::PropertyAccess { .. } => "property access is not lowered to MIR in Stage 11",
        hir::Expr::New { .. } => "object construction is not lowered to MIR in Stage 11",
        hir::Expr::This { .. } => "$this is not lowered to MIR in Stage 11",
        hir::Expr::Identifier { .. } => {
            "identifiers are not lowered as int expressions in Stage 11"
        }
        hir::Expr::Unary { .. } => "unary expressions are not lowered to MIR in Stage 11",
        hir::Expr::Range { .. } => "ranges are not lowered to MIR in Stage 11",
        hir::Expr::Binary {
            op:
                hir::BinaryOp::Equal
                | hir::BinaryOp::NotEqual
                | hir::BinaryOp::Less
                | hir::BinaryOp::LessEqual
                | hir::BinaryOp::Greater
                | hir::BinaryOp::GreaterEqual,
            ..
        } => {
            "comparison results are condition-only and are not lowered as runtime values in Stage 13 MIR"
        }
        hir::Expr::Binary { .. } => "this binary expression is not lowered to MIR in Stage 13",
        hir::Expr::Int { .. } | hir::Expr::Variable { .. } | hir::Expr::Grouped { .. } => {
            "this int expression is not lowered to MIR in Stage 11"
        }
    };
    unsupported(expr.span(), detail)
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
        format!("unsupported MIR Stage 11 coverage: {}", detail.into()),
        span,
    )
}

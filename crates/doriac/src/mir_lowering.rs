use std::collections::HashMap;

use crate::diagnostics::{Diagnostic, DiagnosticResult};
use crate::source::Span;
use crate::{hir, mir};

pub fn lower_program(program: &hir::Program) -> DiagnosticResult<mir::Program> {
    let mut functions = Vec::new();

    for item in &program.items {
        match item {
            hir::Item::Function(function) => functions.push(function),
            hir::Item::Class(class_decl) => {
                return Err(vec![unsupported(
                    class_decl.span,
                    "classes are not lowered to MIR in Stage 11c",
                )]);
            }
            hir::Item::Statement(statement) => {
                return Err(vec![unsupported(
                    stmt_span(statement),
                    "top-level statements are not lowered to MIR in Stage 11c",
                )]);
            }
        }
    }

    if functions.len() != 1 {
        let span = functions
            .iter()
            .find(|function| function.name != "main")
            .map_or_else(Span::default, |function| function.span);
        return Err(vec![unsupported(
            span,
            "Stage 11c requires exactly one top-level function main and no helper functions",
        )]);
    }

    let function = functions[0];
    if function.name != "main" {
        return Err(vec![unsupported(
            function.span,
            "Stage 11c requires the single lowered function to be main",
        )]);
    }

    let function = lower_main_function(function)?;
    Ok(mir::Program {
        functions: vec![function],
        entry: mir::FunctionId(0),
    })
}

fn lower_main_function(function: &hir::FunctionDecl) -> DiagnosticResult<mir::Function> {
    if !function.params.is_empty() {
        return Err(vec![unsupported(
            function.params[0].span,
            "main parameters are not lowered to MIR in Stage 11c",
        )]);
    }

    let return_type = lower_return_type(function)?;
    let mut context = LoweringContext::new();
    lower_main_body(&function.body, return_type, &mut context)?;
    let (locals, blocks) = context.finish();

    Ok(mir::Function {
        id: mir::FunctionId(0),
        name: function.name.clone(),
        return_type,
        locals,
        blocks,
        entry_block: mir::BlockId(0),
    })
}

fn lower_return_type(function: &hir::FunctionDecl) -> DiagnosticResult<mir::ReturnType> {
    match function.return_type.as_ref().map(|ty| ty.name.as_str()) {
        Some("int") => Ok(mir::ReturnType::Int),
        Some("void") => Ok(mir::ReturnType::Void),
        Some(_) => Err(vec![unsupported(
            function.span,
            "only main(): int and main(): void are lowered to MIR in Stage 11c",
        )]),
        None => Err(vec![unsupported(
            function.span,
            "Stage 11c MIR lowering requires an explicit main return type",
        )]),
    }
}

fn lower_main_body(
    body: &hir::Block,
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
                "main(): int fallthrough is not lowered to MIR in Stage 11c",
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
                "statements after return are not lowered to MIR in Stage 11c",
            )]);
        }

        match statement {
            hir::Stmt::Echo { expr, span } => {
                if return_type != mir::ReturnType::Void {
                    return Err(vec![unsupported(
                        *span,
                        "string-literal echo is only lowered for main(): void in Stage 11c",
                    )]);
                }
                context.push_statement(lower_echo(expr)?);
            }
            hir::Stmt::Return { expr, span } => {
                let terminator = lower_return(expr.as_ref(), *span, return_type, context)?;
                context.terminate_current(terminator);
            }
            hir::Stmt::VarDecl(decl) => lower_var_decl(decl, context)?,
            hir::Stmt::Assignment(assignment) => lower_assignment(assignment, context)?,
            hir::Stmt::Increment(increment) => lower_increment(increment, context)?,
            hir::Stmt::If(if_stmt) => lower_if_statement(if_stmt, return_type, context)?,
            hir::Stmt::While(_) | hir::Stmt::For(_) | hir::Stmt::Foreach(_) => {
                return Err(vec![unsupported(
                    stmt_span(statement),
                    "loops are not lowered to MIR in Stage 11c",
                )]);
            }
            hir::Stmt::Break { .. } | hir::Stmt::Continue { .. } => {
                return Err(vec![unsupported(
                    stmt_span(statement),
                    "loop control is not lowered to MIR in Stage 11c",
                )]);
            }
            hir::Stmt::Expr { expr, span } => {
                if matches!(expr, hir::Expr::FunctionCall { .. }) {
                    return Err(vec![unsupported(
                        *span,
                        "function calls are not lowered to MIR in Stage 11c",
                    )]);
                }
                return Err(vec![unsupported(
                    *span,
                    "expression statements are not lowered to MIR in Stage 11c",
                )]);
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

struct LoweringContext {
    locals: Vec<mir::Local>,
    local_scopes: Vec<HashMap<String, mir::LocalId>>,
    temp_counter: usize,
    blocks: Vec<BlockBuilder>,
    current_block: Option<mir::BlockId>,
}

impl LoweringContext {
    fn new() -> Self {
        Self {
            locals: Vec::new(),
            local_scopes: vec![HashMap::new()],
            temp_counter: 0,
            blocks: vec![BlockBuilder {
                id: mir::BlockId(0),
                statements: Vec::new(),
                terminator: None,
            }],
            current_block: Some(mir::BlockId(0)),
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

    fn declare_user_local(&mut self, name: &str, writable: bool) -> mir::LocalId {
        let id = mir::LocalId(self.locals.len());
        self.locals.push(mir::Local {
            id,
            name: name.to_string(),
            ty: mir::Type::Int,
            writable,
            synthetic: false,
        });
        self.local_scopes
            .last_mut()
            .expect("MIR lowering must have a local scope")
            .insert(name.to_string(), id);
        id
    }

    fn declare_temp(&mut self) -> mir::LocalId {
        let id = mir::LocalId(self.locals.len());
        let name = format!("_tmp{}", self.temp_counter);
        self.temp_counter += 1;
        self.locals.push(mir::Local {
            id,
            name,
            ty: mir::Type::Int,
            writable: false,
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
                    format!("local `${name}` is not available in MIR Stage 11c"),
                )]
            })
    }
}

fn lower_var_decl(decl: &hir::VarDecl, context: &mut LoweringContext) -> DiagnosticResult<()> {
    if let Some(ty) = &decl.ty {
        if ty.name != "int" || !ty.args.is_empty() {
            return Err(vec![unsupported(
                decl.span,
                format!("only int locals are lowered to MIR in Stage 11c; got `{ty}`"),
            )]);
        }
    } else if matches!(
        decl.initializer,
        hir::Expr::String { .. } | hir::Expr::InterpolatedString { .. }
    ) {
        return Err(vec![unsupported(
            decl.span,
            "string locals are not lowered to MIR in Stage 11c",
        )]);
    }

    let value = lower_int_rvalue(&decl.initializer, context)?;
    let local = context.declare_user_local(&decl.name, decl.writable);
    context.push_statement(mir::Statement::AssignLocal {
        target: local,
        value,
    });
    Ok(())
}

fn lower_assignment(
    assignment: &hir::Assignment,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    let target = lower_assignment_target(&assignment.target, context)?;
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
            "only local variable assignment targets are lowered to MIR in Stage 11c",
        )]),
    }
}

fn lower_echo(expr: &hir::Expr) -> DiagnosticResult<mir::Statement> {
    match expr {
        hir::Expr::String { value, .. } => Ok(mir::Statement::EchoStringLiteral(value.clone())),
        hir::Expr::Binary { .. } => Err(vec![unsupported(
            expr.span(),
            "string concatenation in echo is not lowered to MIR in Stage 11c; only exact string-literal echo is supported",
        )]),
        _ => Err(vec![unsupported(
            expr.span(),
            "non-literal echo expressions are not lowered to MIR in Stage 11c; only exact string-literal echo is supported",
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
            "bare return is not lowered for main(): int in Stage 11c",
        )]),
        (mir::ReturnType::Void, Some(expr)) => Err(vec![unsupported(
            expr.span(),
            "return values are not lowered for main(): void in Stage 11c",
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
                "conditions require bool literals, integer comparisons, or boolean condition operators in Stage 11c",
            )]),
        },
        hir::Expr::FunctionCall { .. }
        | hir::Expr::MethodCall { .. }
        | hir::Expr::StaticCall { .. } => Err(vec![unsupported(
            expr.span(),
            "function and method calls in conditions are not lowered to MIR in Stage 11c",
        )]),
        hir::Expr::Int { .. } => Err(vec![unsupported(
            expr.span(),
            "integer truthiness is not Doria condition semantics; Stage 11c requires a bool condition",
        )]),
        hir::Expr::Variable { .. } => Err(vec![unsupported(
            expr.span(),
            "user-authored bool locals are not lowered to MIR in Stage 11c",
        )]),
        _ => Err(vec![unsupported(
            expr.span(),
            "this condition expression is not lowered to MIR in Stage 11c",
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
            .lookup_local(name, *span)
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
        hir::Expr::FunctionCall { .. } => Err(vec![unsupported(
            expr.span(),
            "function calls in comparison operands are not lowered to MIR in Stage 11c",
        )]),
        _ => Err(vec![unsupported(
            expr.span(),
            "only Stage 11b integer expressions are lowered as comparison operands in Stage 11c",
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
            "division and modulo in condition operands are not lowered to MIR in Stage 11c",
        )]),
        _ => Err(vec![unsupported(
            span,
            "only Stage 11b integer arithmetic is lowered inside MIR Stage 11c comparisons",
        )]),
    }
}

fn lower_int_operand(
    expr: &hir::Expr,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::Operand> {
    match expr {
        hir::Expr::Int { value, span } => parse_int_literal(value, *span),
        hir::Expr::Variable { name, span } => {
            context.lookup_local(name, *span).map(mir::Operand::Local)
        }
        hir::Expr::Grouped { expr, .. } => lower_int_operand(expr, context),
        hir::Expr::Binary { .. } => {
            let value = lower_int_rvalue(expr, context)?;
            let temp = context.declare_temp();
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
            "division and modulo are not lowered to MIR in Stage 11c",
        )]),
        hir::BinaryOp::Less
        | hir::BinaryOp::LessEqual
        | hir::BinaryOp::Greater
        | hir::BinaryOp::GreaterEqual
        | hir::BinaryOp::Equal
        | hir::BinaryOp::NotEqual => Err(vec![unsupported(
            span,
            "comparison results are condition-only and are not lowered as runtime values in MIR Stage 11c",
        )]),
        hir::BinaryOp::Concat => Err(vec![unsupported(
            span,
            "string concatenation is not lowered to MIR in Stage 11c",
        )]),
        hir::BinaryOp::And | hir::BinaryOp::Or | hir::BinaryOp::Xor => Err(vec![unsupported(
            span,
            "bool runtime values are not lowered to MIR in Stage 11c; boolean operators are condition-only",
        )]),
        hir::BinaryOp::Coalesce => Err(vec![unsupported(
            span,
            "null coalescing is not lowered to MIR in Stage 11c",
        )]),
    }
}

fn parse_int_literal(value: &str, span: Span) -> DiagnosticResult<mir::Operand> {
    value.parse::<i64>().map(mir::Operand::Int).map_err(|_| {
        vec![unsupported(
            span,
            "integer literals outside int64 are not lowered to MIR in Stage 11c",
        )]
    })
}

fn unsupported_int_expr(expr: &hir::Expr) -> Diagnostic {
    let detail = match expr {
        hir::Expr::String { .. } | hir::Expr::InterpolatedString { .. } => {
            "string expressions are not lowered to MIR in Stage 11c"
        }
        hir::Expr::Float { .. } => "float expressions are not lowered to MIR in Stage 11c",
        hir::Expr::Bool { .. } => "bool runtime values are not lowered to MIR in Stage 11c",
        hir::Expr::Null { .. } => "null values are not lowered to MIR in Stage 11c",
        hir::Expr::Array { .. } => "collections are not lowered to MIR in Stage 11c",
        hir::Expr::FunctionCall { .. } => "function calls are not lowered to MIR in Stage 11c",
        hir::Expr::MethodCall { .. } | hir::Expr::StaticCall { .. } => {
            "method calls are not lowered to MIR in Stage 11c"
        }
        hir::Expr::PropertyAccess { .. } => "property access is not lowered to MIR in Stage 11c",
        hir::Expr::New { .. } => "object construction is not lowered to MIR in Stage 11c",
        hir::Expr::This { .. } => "$this is not lowered to MIR in Stage 11c",
        hir::Expr::Identifier { .. } => {
            "identifiers are not lowered as int expressions in Stage 11c"
        }
        hir::Expr::Unary { .. } => "unary expressions are not lowered to MIR in Stage 11c",
        hir::Expr::Range { .. } => "ranges are not lowered to MIR in Stage 11c",
        hir::Expr::Binary { .. } => "this binary expression is not lowered to MIR in Stage 11c",
        hir::Expr::Int { .. } | hir::Expr::Variable { .. } | hir::Expr::Grouped { .. } => {
            "this int expression is not lowered to MIR in Stage 11c"
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
        format!("unsupported MIR Stage 11c coverage: {}", detail.into()),
        span,
    )
}

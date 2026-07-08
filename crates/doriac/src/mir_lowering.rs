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
                    "classes are not lowered to MIR in Stage 11b",
                )]);
            }
            hir::Item::Statement(statement) => {
                return Err(vec![unsupported(
                    stmt_span(statement),
                    "top-level statements are not lowered to MIR in Stage 11b",
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
            "Stage 11b requires exactly one top-level function main and no helper functions",
        )]);
    }

    let function = functions[0];
    if function.name != "main" {
        return Err(vec![unsupported(
            function.span,
            "Stage 11b requires the single lowered function to be main",
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
            "main parameters are not lowered to MIR in Stage 11b",
        )]);
    }

    let return_type = lower_return_type(function)?;
    let mut context = LoweringContext::default();
    let terminator = lower_main_body(&function.body, return_type, &mut context)?;
    let block = mir::BasicBlock {
        id: mir::BlockId(0),
        statements: context.statements,
        terminator,
    };

    Ok(mir::Function {
        id: mir::FunctionId(0),
        name: function.name.clone(),
        return_type,
        locals: context.locals,
        blocks: vec![block],
        entry_block: mir::BlockId(0),
    })
}

fn lower_return_type(function: &hir::FunctionDecl) -> DiagnosticResult<mir::ReturnType> {
    match function.return_type.as_ref().map(|ty| ty.name.as_str()) {
        Some("int") => Ok(mir::ReturnType::Int),
        Some("void") => Ok(mir::ReturnType::Void),
        Some(_) => Err(vec![unsupported(
            function.span,
            "only main(): int and main(): void are lowered to MIR in Stage 11b",
        )]),
        None => Err(vec![unsupported(
            function.span,
            "Stage 11b MIR lowering requires an explicit main return type",
        )]),
    }
}

fn lower_main_body(
    body: &hir::Block,
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::Terminator> {
    let mut terminator = None;

    for statement in &body.statements {
        if terminator.is_some() {
            return Err(vec![unsupported(
                stmt_span(statement),
                "statements after return are not lowered to MIR in Stage 11b",
            )]);
        }

        match statement {
            hir::Stmt::Echo { expr, span } => {
                if return_type != mir::ReturnType::Void {
                    return Err(vec![unsupported(
                        *span,
                        "string-literal echo is only lowered for main(): void in Stage 11b",
                    )]);
                }
                let statement = lower_echo(expr)?;
                context.statements.push(statement);
            }
            hir::Stmt::Return { expr, span } => {
                terminator = Some(lower_return(expr.as_ref(), *span, return_type, context)?);
            }
            hir::Stmt::VarDecl(decl) => lower_var_decl(decl, context)?,
            hir::Stmt::Assignment(assignment) => lower_assignment(assignment, context)?,
            hir::Stmt::Increment(increment) => lower_increment(increment, context)?,
            hir::Stmt::If(_) => {
                return Err(vec![unsupported(
                    stmt_span(statement),
                    "if statements are not lowered to MIR in Stage 11b",
                )]);
            }
            hir::Stmt::While(_) | hir::Stmt::For(_) | hir::Stmt::Foreach(_) => {
                return Err(vec![unsupported(
                    stmt_span(statement),
                    "loops are not lowered to MIR in Stage 11b",
                )]);
            }
            hir::Stmt::Break { .. } | hir::Stmt::Continue { .. } => {
                return Err(vec![unsupported(
                    stmt_span(statement),
                    "loop control is not lowered to MIR in Stage 11b",
                )]);
            }
            hir::Stmt::Expr { expr, span } => {
                if matches!(expr, hir::Expr::FunctionCall { .. }) {
                    return Err(vec![unsupported(
                        *span,
                        "function calls are not lowered to MIR in Stage 11b",
                    )]);
                }
                return Err(vec![unsupported(
                    *span,
                    "expression statements are not lowered to MIR in Stage 11b",
                )]);
            }
        }
    }

    match terminator {
        Some(terminator) => Ok(terminator),
        None if return_type == mir::ReturnType::Void => Ok(mir::Terminator::ReturnVoid),
        None => Err(vec![unsupported(
            body.span,
            "main(): int fallthrough is not lowered to MIR in Stage 11b",
        )]),
    }
}

#[derive(Default)]
struct LoweringContext {
    locals: Vec<mir::Local>,
    local_names: HashMap<String, mir::LocalId>,
    temp_counter: usize,
    statements: Vec<mir::Statement>,
}

impl LoweringContext {
    fn declare_user_local(&mut self, name: &str, writable: bool) -> mir::LocalId {
        let id = mir::LocalId(self.locals.len());
        self.locals.push(mir::Local {
            id,
            name: name.to_string(),
            ty: mir::Type::Int,
            writable,
            synthetic: false,
        });
        self.local_names.insert(name.to_string(), id);
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
        self.local_names.get(name).copied().ok_or_else(|| {
            vec![unsupported(
                span,
                format!("local `${name}` is not available in MIR Stage 11b"),
            )]
        })
    }
}

fn lower_var_decl(decl: &hir::VarDecl, context: &mut LoweringContext) -> DiagnosticResult<()> {
    if let Some(ty) = &decl.ty {
        if ty.name != "int" || !ty.args.is_empty() {
            return Err(vec![unsupported(
                decl.span,
                format!("only int locals are lowered to MIR in Stage 11b; got `{ty}`"),
            )]);
        }
    } else if matches!(
        decl.initializer,
        hir::Expr::String { .. } | hir::Expr::InterpolatedString { .. }
    ) {
        return Err(vec![unsupported(
            decl.span,
            "string locals are not lowered to MIR in Stage 11b",
        )]);
    }

    let value = lower_int_rvalue(&decl.initializer, context)?;
    let local = context.declare_user_local(&decl.name, decl.writable);
    context.statements.push(mir::Statement::AssignLocal {
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
    context
        .statements
        .push(mir::Statement::AssignLocal { target, value });
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
    context.statements.push(mir::Statement::AssignLocal {
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
            "only local variable assignment targets are lowered to MIR in Stage 11b",
        )]),
    }
}

fn lower_echo(expr: &hir::Expr) -> DiagnosticResult<mir::Statement> {
    match expr {
        hir::Expr::String { value, .. } => Ok(mir::Statement::EchoStringLiteral(value.clone())),
        hir::Expr::Binary { .. } => Err(vec![unsupported(
            expr.span(),
            "string concatenation in echo is not lowered to MIR in Stage 11b; only exact string-literal echo is supported",
        )]),
        _ => Err(vec![unsupported(
            expr.span(),
            "non-literal echo expressions are not lowered to MIR in Stage 11b; only exact string-literal echo is supported",
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
            "bare return is not lowered for main(): int in Stage 11b",
        )]),
        (mir::ReturnType::Void, Some(expr)) => Err(vec![unsupported(
            expr.span(),
            "return values are not lowered for main(): void in Stage 11b",
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
            context.statements.push(mir::Statement::AssignLocal {
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
            "division and modulo are not lowered to MIR in Stage 11b",
        )]),
        hir::BinaryOp::Less
        | hir::BinaryOp::LessEqual
        | hir::BinaryOp::Greater
        | hir::BinaryOp::GreaterEqual
        | hir::BinaryOp::Equal
        | hir::BinaryOp::NotEqual => Err(vec![unsupported(
            span,
            "comparisons are not lowered to MIR in Stage 11b",
        )]),
        hir::BinaryOp::Concat => Err(vec![unsupported(
            span,
            "string concatenation is not lowered to MIR in Stage 11b",
        )]),
        hir::BinaryOp::And | hir::BinaryOp::Or | hir::BinaryOp::Xor => Err(vec![unsupported(
            span,
            "bool runtime values and boolean operators are not lowered to MIR in Stage 11b",
        )]),
        hir::BinaryOp::Coalesce => Err(vec![unsupported(
            span,
            "null coalescing is not lowered to MIR in Stage 11b",
        )]),
    }
}

fn parse_int_literal(value: &str, span: Span) -> DiagnosticResult<mir::Operand> {
    value.parse::<i64>().map(mir::Operand::Int).map_err(|_| {
        vec![unsupported(
            span,
            "integer literals outside int64 are not lowered to MIR in Stage 11b",
        )]
    })
}

fn unsupported_int_expr(expr: &hir::Expr) -> Diagnostic {
    let detail = match expr {
        hir::Expr::String { .. } | hir::Expr::InterpolatedString { .. } => {
            "string expressions are not lowered to MIR in Stage 11b"
        }
        hir::Expr::Float { .. } => "float expressions are not lowered to MIR in Stage 11b",
        hir::Expr::Bool { .. } => "bool runtime values are not lowered to MIR in Stage 11b",
        hir::Expr::Null { .. } => "null values are not lowered to MIR in Stage 11b",
        hir::Expr::Array { .. } => "collections are not lowered to MIR in Stage 11b",
        hir::Expr::FunctionCall { .. } => "function calls are not lowered to MIR in Stage 11b",
        hir::Expr::MethodCall { .. } | hir::Expr::StaticCall { .. } => {
            "method calls are not lowered to MIR in Stage 11b"
        }
        hir::Expr::PropertyAccess { .. } => "property access is not lowered to MIR in Stage 11b",
        hir::Expr::New { .. } => "object construction is not lowered to MIR in Stage 11b",
        hir::Expr::This { .. } => "$this is not lowered to MIR in Stage 11b",
        hir::Expr::Identifier { .. } => {
            "identifiers are not lowered as int expressions in Stage 11b"
        }
        hir::Expr::Unary { .. } => "unary expressions are not lowered to MIR in Stage 11b",
        hir::Expr::Range { .. } => "ranges are not lowered to MIR in Stage 11b",
        hir::Expr::Binary { .. } => "this binary expression is not lowered to MIR in Stage 11b",
        hir::Expr::Int { .. } | hir::Expr::Variable { .. } | hir::Expr::Grouped { .. } => {
            "this int expression is not lowered to MIR in Stage 11b"
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
        format!("unsupported MIR Stage 11b coverage: {}", detail.into()),
        span,
    )
}

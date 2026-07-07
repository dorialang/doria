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
                    "classes are not lowered to MIR in Stage 11a",
                )]);
            }
            hir::Item::Statement(statement) => {
                return Err(vec![unsupported(
                    stmt_span(statement),
                    "top-level statements are not lowered to MIR in Stage 11a",
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
            "Stage 11a requires exactly one top-level function main and no helper functions",
        )]);
    }

    let function = functions[0];
    if function.name != "main" {
        return Err(vec![unsupported(
            function.span,
            "Stage 11a requires the single lowered function to be main",
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
            "main parameters are not lowered to MIR in Stage 11a",
        )]);
    }

    let return_type = lower_return_type(function)?;
    let block = lower_main_body(&function.body, return_type)?;

    Ok(mir::Function {
        id: mir::FunctionId(0),
        name: function.name.clone(),
        return_type,
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
            "only main(): int and main(): void are lowered to MIR in Stage 11a",
        )]),
        None => Err(vec![unsupported(
            function.span,
            "Stage 11a MIR lowering requires an explicit main return type",
        )]),
    }
}

fn lower_main_body(
    body: &hir::Block,
    return_type: mir::ReturnType,
) -> DiagnosticResult<mir::BasicBlock> {
    let mut statements = Vec::new();
    let mut terminator = None;

    for statement in &body.statements {
        if terminator.is_some() {
            return Err(vec![unsupported(
                stmt_span(statement),
                "statements after return are not lowered to MIR in Stage 11a",
            )]);
        }

        match statement {
            hir::Stmt::Echo { expr, span } => {
                if return_type != mir::ReturnType::Void {
                    return Err(vec![unsupported(
                        *span,
                        "string-literal echo is only lowered for main(): void in Stage 11a",
                    )]);
                }
                statements.push(lower_echo(expr)?);
            }
            hir::Stmt::Return { expr, span } => {
                terminator = Some(lower_return(expr.as_ref(), *span, return_type)?);
            }
            hir::Stmt::VarDecl(_) => {
                return Err(vec![unsupported(
                    stmt_span(statement),
                    "local variables are not lowered to MIR in Stage 11a",
                )]);
            }
            hir::Stmt::Assignment(_) => {
                return Err(vec![unsupported(
                    stmt_span(statement),
                    "assignments are not lowered to MIR in Stage 11a",
                )]);
            }
            hir::Stmt::If(_) => {
                return Err(vec![unsupported(
                    stmt_span(statement),
                    "if statements are not lowered to MIR in Stage 11a",
                )]);
            }
            hir::Stmt::While(_) | hir::Stmt::For(_) | hir::Stmt::Foreach(_) => {
                return Err(vec![unsupported(
                    stmt_span(statement),
                    "loops are not lowered to MIR in Stage 11a",
                )]);
            }
            hir::Stmt::Break { .. } | hir::Stmt::Continue { .. } => {
                return Err(vec![unsupported(
                    stmt_span(statement),
                    "loop control is not lowered to MIR in Stage 11a",
                )]);
            }
            hir::Stmt::Increment(_) => {
                return Err(vec![unsupported(
                    stmt_span(statement),
                    "increments are not lowered to MIR in Stage 11a",
                )]);
            }
            hir::Stmt::Expr { expr, span } => {
                if matches!(expr, hir::Expr::FunctionCall { .. }) {
                    return Err(vec![unsupported(
                        *span,
                        "function calls are not lowered to MIR in Stage 11a",
                    )]);
                }
                return Err(vec![unsupported(
                    *span,
                    "expression statements are not lowered to MIR in Stage 11a",
                )]);
            }
        }
    }

    let terminator = match terminator {
        Some(terminator) => terminator,
        None if return_type == mir::ReturnType::Void => mir::Terminator::ReturnVoid,
        None => {
            return Err(vec![unsupported(
                body.span,
                "main(): int fallthrough is not lowered to MIR in Stage 11a",
            )]);
        }
    };

    Ok(mir::BasicBlock {
        id: mir::BlockId(0),
        statements,
        terminator,
    })
}

fn lower_echo(expr: &hir::Expr) -> DiagnosticResult<mir::Statement> {
    match expr {
        hir::Expr::String { value, .. } => Ok(mir::Statement::EchoStringLiteral(value.clone())),
        hir::Expr::Binary { .. } => Err(vec![unsupported(
            expr.span(),
            "string concatenation in echo is not lowered to MIR in Stage 11a; only exact string-literal echo is supported",
        )]),
        _ => Err(vec![unsupported(
            expr.span(),
            "non-literal echo expressions are not lowered to MIR in Stage 11a; only exact string-literal echo is supported",
        )]),
    }
}

fn lower_return(
    expr: Option<&hir::Expr>,
    span: Span,
    return_type: mir::ReturnType,
) -> DiagnosticResult<mir::Terminator> {
    match (return_type, expr) {
        (mir::ReturnType::Void, None) => Ok(mir::Terminator::ReturnVoid),
        (mir::ReturnType::Int, Some(hir::Expr::Int { value, span })) => value
            .parse::<i64>()
            .map(mir::Terminator::ReturnInt)
            .map_err(|_| {
                vec![unsupported(
                    *span,
                    "integer literals outside int64 are not lowered to MIR in Stage 11a",
                )]
            }),
        (mir::ReturnType::Int, Some(expr)) => Err(vec![unsupported(
            expr.span(),
            "main(): int returning a non-literal expression is not lowered to MIR in Stage 11a",
        )]),
        (mir::ReturnType::Int, None) => Err(vec![unsupported(
            span,
            "bare return is not lowered for main(): int in Stage 11a",
        )]),
        (mir::ReturnType::Void, Some(expr)) => Err(vec![unsupported(
            expr.span(),
            "return values are not lowered for main(): void in Stage 11a",
        )]),
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
        format!("unsupported MIR Stage 11a coverage: {}", detail.into()),
        span,
    )
}

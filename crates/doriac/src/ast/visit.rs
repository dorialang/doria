//! Source-order expression traversal, including closures and structured finalizers.

use super::*;

pub fn block(block: &Block, visitor: &mut dyn FnMut(&Expr)) {
    for statement in &block.statements {
        stmt(statement, visitor);
    }
}

fn optional(expression: &Option<Expr>, visitor: &mut dyn FnMut(&Expr)) {
    if let Some(expression) = expression {
        expr(expression, visitor);
    }
}

fn given(prelude: &Option<GivenPrelude>, visitor: &mut dyn FnMut(&Expr)) {
    if let Some(prelude) = prelude {
        block(&prelude.block, visitor);
    }
}

fn finally(finalizer: &Option<ControlFlowFinally>, visitor: &mut dyn FnMut(&Expr)) {
    if let Some(finalizer) = finalizer {
        block(&finalizer.block, visitor);
    }
}

fn assignment(statement: &Assignment, visitor: &mut dyn FnMut(&Expr)) {
    expr(&statement.target, visitor);
    expr(&statement.value, visitor);
}

fn conditional(statement: &IfStmt, visitor: &mut dyn FnMut(&Expr)) {
    given(&statement.given, visitor);
    expr(&statement.condition, visitor);
    block(&statement.then_block, visitor);
    match &statement.else_branch {
        Some(ElseBranch::If(statement)) => conditional(statement, visitor),
        Some(ElseBranch::Block(body)) => block(body, visitor),
        None => {}
    }
    finally(&statement.finally, visitor);
}

pub fn stmt(statement: &Stmt, visitor: &mut dyn FnMut(&Expr)) {
    match statement {
        Stmt::Block(body) => block(body, visitor),
        Stmt::VarDecl(declaration) => expr(&declaration.initializer, visitor),
        Stmt::Assignment(statement) => assignment(statement, visitor),
        Stmt::Echo { expr: value, .. } | Stmt::Expr { expr: value, .. } => expr(value, visitor),
        Stmt::Return { expr: value, .. } => optional(value, visitor),
        Stmt::Throw(statement) => expr(&statement.expr, visitor),
        Stmt::Try(statement) => {
            block(&statement.body, visitor);
            for catch in &statement.catches {
                block(&catch.body, visitor);
            }
            if let Some(finalizer) = &statement.finally {
                block(&finalizer.body, visitor);
            }
        }
        Stmt::If(statement) => conditional(statement, visitor),
        Stmt::While(statement) => {
            given(&statement.given, visitor);
            expr(&statement.condition, visitor);
            block(&statement.body, visitor);
            finally(&statement.finally, visitor);
        }
        Stmt::DoWhile(statement) => {
            block(&statement.body, visitor);
            expr(&statement.condition, visitor);
            finally(&statement.finally, visitor);
        }
        Stmt::For(statement) => {
            match &statement.initializer {
                Some(ForInitializer::VarDecl(declaration)) => {
                    expr(&declaration.initializer, visitor)
                }
                Some(ForInitializer::Assignment(statement)) => assignment(statement, visitor),
                None => {}
            }
            optional(&statement.condition, visitor);
            match &statement.increment {
                Some(ForIncrement::Assignment(statement)) => assignment(statement, visitor),
                Some(ForIncrement::Increment(statement)) => expr(&statement.target, visitor),
                None => {}
            }
            block(&statement.body, visitor);
        }
        Stmt::Foreach(statement) => {
            expr(&statement.iterable, visitor);
            block(&statement.body, visitor);
        }
        Stmt::Increment(statement) => expr(&statement.target, visitor),
        Stmt::Break { .. } | Stmt::Continue { .. } => {}
    }
}

fn arguments(arguments: &[Argument], visitor: &mut dyn FnMut(&Expr)) {
    for argument in arguments {
        expr(&argument.value, visitor);
    }
}

pub fn expr(expression: &Expr, visitor: &mut dyn FnMut(&Expr)) {
    visitor(expression);
    match expression {
        Expr::Variable { .. }
        | Expr::This { .. }
        | Expr::Identifier { .. }
        | Expr::String { .. }
        | Expr::Int { .. }
        | Expr::Float { .. }
        | Expr::Bool { .. }
        | Expr::Null { .. }
        | Expr::StaticMember { .. } => {}
        Expr::InterpolatedString { parts, .. } => {
            for part in parts {
                if let InterpolatedStringPart::Expr(expression) = part {
                    expr(expression, visitor);
                }
            }
        }
        Expr::Array { elements, .. } => {
            for element in elements {
                optional(&element.key, visitor);
                expr(&element.value, visitor);
            }
        }
        Expr::ArrayRepeat { value, count, .. } => {
            expr(value, visitor);
            expr(count, visitor);
        }
        Expr::Index {
            collection, index, ..
        } => {
            expr(collection, visitor);
            expr(index, visitor);
        }
        Expr::PropertyAccess { object, .. } => expr(object, visitor),
        Expr::MethodCall { object, args, .. } => {
            expr(object, visitor);
            arguments(args, visitor);
        }
        Expr::IsType { expr: value, .. }
        | Expr::Grouped { expr: value, .. }
        | Expr::Unary { expr: value, .. } => expr(value, visitor),
        Expr::FunctionCall { args, .. }
        | Expr::StaticCall { args, .. }
        | Expr::New { args, .. } => arguments(args, visitor),
        Expr::CallableCall { callee, args, .. } => {
            expr(callee, visitor);
            arguments(args, visitor);
        }
        Expr::Binary { left, right, .. } => {
            expr(left, visitor);
            expr(right, visitor);
        }
        Expr::Range { start, end, .. } => {
            expr(start, visitor);
            expr(end, visitor);
        }
        Expr::Match {
            scrutinee, arms, ..
        } => {
            expr(scrutinee, visitor);
            for arm in arms {
                if let MatchPattern::Expression(expression) = &arm.pattern {
                    expr(expression, visitor);
                }
                if let Some(guard) = &arm.guard {
                    expr(&guard.condition, visitor);
                }
                expr(&arm.value, visitor);
            }
        }
        Expr::When(expression) => {
            given(&expression.given, visitor);
            for branch in &expression.branches {
                optional(&branch.condition, visitor);
                block(&branch.block, visitor);
            }
            finally(&expression.finally, visitor);
        }
        Expr::Closure(closure) => match &closure.body {
            ClosureBody::Expression { expression, .. } => expr(expression, visitor),
            ClosureBody::Block(body) => block(body, visitor),
        },
    }
}

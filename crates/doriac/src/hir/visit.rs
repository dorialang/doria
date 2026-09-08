//! Complete structural expression traversal, including deferred closure bodies.

use super::*;

pub(crate) fn expressions<'a>(program: &'a Program, visitor: &mut impl FnMut(&'a Expr)) {
    for item in &program.items {
        match item {
            Item::Function(function) => block(&function.body, visitor),
            Item::Class(class) => {
                for member in &class.members {
                    match member {
                        ClassMember::Method(function) => block(&function.body, visitor),
                        ClassMember::Property(property) => {
                            if let Some(value) = &property.initializer {
                                expression(value, visitor);
                            }
                        }
                        ClassMember::Constant(constant) => {
                            expression(&constant.initializer, visitor)
                        }
                    }
                }
            }
            Item::Statement(stmt) => statement(stmt, visitor),
            Item::Constant(constant) => expression(&constant.initializer, visitor),
            Item::Enum(enumeration) => {
                for case in &enumeration.cases {
                    if let Some(value) = &case.backing_value {
                        expression(value, visitor);
                    }
                }
            }
        }
    }
}

fn block<'a>(body: &'a Block, visitor: &mut impl FnMut(&'a Expr)) {
    for stmt in &body.statements {
        statement(stmt, visitor);
    }
}

fn given<'a>(prelude: &'a Option<GivenPrelude>, visitor: &mut impl FnMut(&'a Expr)) {
    if let Some(prelude) = prelude {
        block(&prelude.block, visitor);
    }
}

fn finally<'a>(finalizer: &'a Option<ControlFlowFinally>, visitor: &mut impl FnMut(&'a Expr)) {
    if let Some(finalizer) = finalizer {
        block(&finalizer.block, visitor);
    }
}

fn assignment<'a>(value: &'a Assignment, visitor: &mut impl FnMut(&'a Expr)) {
    expression(&value.target, visitor);
    expression(&value.value, visitor);
}

fn if_statement<'a>(value: &'a IfStmt, visitor: &mut impl FnMut(&'a Expr)) {
    given(&value.given, visitor);
    expression(&value.condition, visitor);
    block(&value.then_block, visitor);
    match &value.else_branch {
        Some(ElseBranch::If(value)) => if_statement(value, visitor),
        Some(ElseBranch::Block(value)) => block(value, visitor),
        None => {}
    }
    finally(&value.finally, visitor);
}

fn statement<'a>(value: &'a Stmt, visitor: &mut impl FnMut(&'a Expr)) {
    match value {
        Stmt::Block(value) => block(value, visitor),
        Stmt::VarDecl(value) => expression(&value.initializer, visitor),
        Stmt::Assignment(value) => assignment(value, visitor),
        Stmt::Expr { expr, .. } | Stmt::Echo { expr, .. } => expression(expr, visitor),
        Stmt::Return { expr, .. } => {
            if let Some(expr) = expr {
                expression(expr, visitor);
            }
        }
        Stmt::Throw(value) => expression(&value.expr, visitor),
        Stmt::Try(value) => {
            block(&value.body, visitor);
            for catch in &value.catches {
                block(&catch.body, visitor);
            }
            if let Some(finalizer) = &value.finally {
                block(&finalizer.body, visitor);
            }
        }
        Stmt::If(value) => if_statement(value, visitor),
        Stmt::While(value) => {
            given(&value.given, visitor);
            expression(&value.condition, visitor);
            block(&value.body, visitor);
            finally(&value.finally, visitor);
        }
        Stmt::DoWhile(value) => {
            block(&value.body, visitor);
            expression(&value.condition, visitor);
            finally(&value.finally, visitor);
        }
        Stmt::For(value) => {
            if let Some(initializer) = &value.initializer {
                match initializer {
                    ForInitializer::VarDecl(value) => expression(&value.initializer, visitor),
                    ForInitializer::Assignment(value) => assignment(value, visitor),
                }
            }
            if let Some(condition) = &value.condition {
                expression(condition, visitor);
            }
            if let Some(increment) = &value.increment {
                match increment {
                    ForIncrement::Increment(value) => expression(&value.target, visitor),
                    ForIncrement::Assignment(value) => assignment(value, visitor),
                }
            }
            block(&value.body, visitor);
        }
        Stmt::Foreach(value) => {
            expression(&value.iterable, visitor);
            block(&value.body, visitor);
        }
        Stmt::Increment(value) => expression(&value.target, visitor),
        Stmt::Break { .. } | Stmt::Continue { .. } => {}
    }
}

fn expression<'a>(value: &'a Expr, visitor: &mut impl FnMut(&'a Expr)) {
    visitor(value);
    match value {
        Expr::Assertion(value) => {
            for value in [
                value.actual.as_deref(),
                value.expected.as_deref(),
                value.user_message.as_deref(),
            ]
            .into_iter()
            .flatten()
            {
                expression(value, visitor);
            }
        }
        Expr::Closure(value) => match &value.body {
            ClosureBody::Expression(value) => expression(value, visitor),
            ClosureBody::Block(value) => block(value, visitor),
        },
        Expr::CallableCall(value) => {
            expression(&value.callee, visitor);
            for argument in &value.args {
                expression(&argument.value, visitor);
            }
        }
        Expr::ListAlgorithmCall(value) => {
            expression(&value.receiver, visitor);
            for argument in &value.arguments {
                expression(&argument.value, visitor);
            }
        }
        Expr::MethodCall { object, args, .. } => {
            expression(object, visitor);
            for argument in args {
                expression(&argument.value, visitor);
            }
        }
        Expr::FunctionCall { args, .. }
        | Expr::StaticCall { args, .. }
        | Expr::New { args, .. } => {
            for argument in args {
                expression(&argument.value, visitor);
            }
        }
        Expr::PropertyAccess { object: expr, .. }
        | Expr::Grouped { expr, .. }
        | Expr::Unary { expr, .. }
        | Expr::IsType { expr, .. } => expression(expr, visitor),
        Expr::Binary { left, right, .. }
        | Expr::Index {
            collection: left,
            index: right,
            ..
        }
        | Expr::Range {
            start: left,
            end: right,
            ..
        }
        | Expr::ArrayRepeat {
            value: left,
            count: right,
            ..
        } => {
            expression(left, visitor);
            expression(right, visitor);
        }
        Expr::Array { elements, .. } => {
            for element in elements {
                if let Some(key) = &element.key {
                    expression(key, visitor);
                }
                expression(&element.value, visitor);
            }
        }
        Expr::InterpolatedString { parts, .. } => {
            for part in parts {
                if let InterpolatedStringPart::Expr(value) = part {
                    expression(value, visitor);
                }
            }
        }
        Expr::Match {
            scrutinee, arms, ..
        } => {
            expression(scrutinee, visitor);
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    expression(&guard.condition, visitor);
                }
                expression(&arm.value, visitor);
            }
        }
        Expr::When(value) => {
            given(&value.given, visitor);
            for branch in &value.branches {
                if let Some(condition) = &branch.condition {
                    expression(condition, visitor);
                }
                block(&branch.block, visitor);
            }
            finally(&value.finally, visitor);
        }
        Expr::Variable { .. }
        | Expr::This { .. }
        | Expr::Identifier { .. }
        | Expr::String { .. }
        | Expr::Int { .. }
        | Expr::Float { .. }
        | Expr::Bool { .. }
        | Expr::Null { .. }
        | Expr::StaticMember { .. } => {}
    }
}

use crate::diagnostics::{Diagnostic, DiagnosticResult};
use crate::{ast, hir};

pub fn lower_program(program: &ast::Program) -> DiagnosticResult<hir::Program> {
    lower_program_with_semantics(program, crate::semantics::SemanticInfo::default())
}

pub fn lower_program_with_semantics(
    program: &ast::Program,
    semantic_info: crate::semantics::SemanticInfo,
) -> DiagnosticResult<hir::Program> {
    let mut items = Vec::with_capacity(program.items.len());
    let mut diagnostics = Vec::new();
    for item in &program.items {
        match lower_item(item) {
            Ok(item) => items.push(item),
            Err(diagnostic) => diagnostics.push(diagnostic),
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }

    Ok(hir::Program {
        namespace: program
            .namespace
            .as_ref()
            .map(|namespace| hir::NamespaceDecl {
                name: namespace.name.clone(),
                span: namespace.span,
            }),
        items,
        semantic_info,
    })
}

fn lower_item(item: &ast::Item) -> Result<hir::Item, Diagnostic> {
    match item {
        ast::Item::Class(class_decl) => Ok(hir::Item::Class(lower_class(class_decl))),
        ast::Item::Interface(interface_decl) => Err(
            crate::semantics::interface_declaration_diagnostic(interface_decl),
        ),
        ast::Item::Trait(trait_decl) => {
            Err(crate::semantics::trait_declaration_diagnostic(trait_decl))
        }
        ast::Item::Function(function) => Ok(hir::Item::Function(lower_function(function, None))),
        ast::Item::Constant(constant) => Ok(hir::Item::Constant(lower_constant(constant, None))),
        ast::Item::Statement(statement) => Ok(hir::Item::Statement(lower_stmt(statement, None))),
    }
}

fn lower_class(class_decl: &ast::ClassDecl) -> hir::ClassDecl {
    hir::ClassDecl {
        name: class_decl.name.clone(),
        parent: class_decl.parent.clone(),
        parent_span: class_decl.parent_span,
        implements: class_decl.implements.clone(),
        members: class_decl
            .members
            .iter()
            .map(|member| lower_class_member(member, &class_decl.name))
            .collect(),
        span: class_decl.span,
    }
}

fn lower_class_member(member: &ast::ClassMember, class_name: &str) -> hir::ClassMember {
    match member {
        ast::ClassMember::Property(property) => {
            hir::ClassMember::Property(lower_property(property, Some(class_name)))
        }
        ast::ClassMember::Method(method) => {
            hir::ClassMember::Method(lower_function(method, Some(class_name)))
        }
        ast::ClassMember::Constant(constant) => {
            hir::ClassMember::Constant(lower_constant(constant, Some(class_name)))
        }
    }
}

fn lower_property(property: &ast::PropertyDecl, class_name: Option<&str>) -> hir::PropertyDecl {
    hir::PropertyDecl {
        access: property.access.clone(),
        is_static: property.is_static,
        writable: property.writable,
        ty: lower_type_ref(&property.ty, class_name),
        name: property.name.clone(),
        initializer: property
            .initializer
            .as_ref()
            .map(|expr| lower_expr(expr, class_name)),
        span: property.span,
    }
}

fn lower_constant(constant: &ast::ConstDecl, class_name: Option<&str>) -> hir::ConstDecl {
    hir::ConstDecl {
        access: constant.access.clone(),
        ty: constant
            .ty
            .as_ref()
            .map(|ty| lower_type_ref(ty, class_name)),
        name: constant.name.clone(),
        initializer: lower_expr(&constant.initializer, class_name),
        span: constant.span,
    }
}

fn lower_function(function: &ast::FunctionDecl, class_name: Option<&str>) -> hir::FunctionDecl {
    hir::FunctionDecl {
        access: function.access.clone(),
        writable_this: function.writable_this,
        is_static: function.is_static,
        name: function.name.clone(),
        params: function
            .params
            .iter()
            .map(|param| lower_param(param, class_name))
            .collect(),
        return_type: function
            .return_type
            .as_ref()
            .map(|ty| lower_type_ref(ty, class_name)),
        body: lower_block(&function.body, class_name),
        span: function.span,
    }
}

fn lower_param(param: &ast::Param, class_name: Option<&str>) -> hir::Param {
    hir::Param {
        promoted_access: param.promoted_access.clone(),
        take: param.take,
        writable: param.writable,
        ty: lower_type_ref(&param.ty, class_name),
        name: param.name.clone(),
        default: param
            .default
            .as_ref()
            .map(|expr| lower_expr(expr, class_name)),
        span: param.span,
    }
}

fn lower_block(block: &ast::Block, class_name: Option<&str>) -> hir::Block {
    hir::Block {
        statements: block
            .statements
            .iter()
            .map(|statement| lower_stmt(statement, class_name))
            .collect(),
        span: block.span,
    }
}

fn lower_stmt(statement: &ast::Stmt, class_name: Option<&str>) -> hir::Stmt {
    match statement {
        ast::Stmt::VarDecl(decl) => hir::Stmt::VarDecl(hir::VarDecl {
            writable: decl.writable,
            ty: decl.ty.as_ref().map(|ty| lower_type_ref(ty, class_name)),
            name: decl.name.clone(),
            initializer: lower_expr(&decl.initializer, class_name),
            span: decl.span,
        }),
        ast::Stmt::Assignment(assignment) => hir::Stmt::Assignment(hir::Assignment {
            target: lower_expr(&assignment.target, class_name),
            op: assignment.op.clone(),
            value: lower_expr(&assignment.value, class_name),
            span: assignment.span,
        }),
        ast::Stmt::Echo { expr, span } => hir::Stmt::Echo {
            expr: lower_expr(expr, class_name),
            span: *span,
        },
        ast::Stmt::Return { expr, span } => hir::Stmt::Return {
            expr: expr.as_ref().map(|expr| lower_expr(expr, class_name)),
            span: *span,
        },
        ast::Stmt::If(if_stmt) => hir::Stmt::If(lower_if_stmt(if_stmt, class_name)),
        ast::Stmt::While(while_stmt) => hir::Stmt::While(hir::WhileStmt {
            condition: lower_expr(&while_stmt.condition, class_name),
            body: lower_block(&while_stmt.body, class_name),
            span: while_stmt.span,
        }),
        ast::Stmt::For(for_stmt) => hir::Stmt::For(Box::new(hir::ForStmt {
            initializer: for_stmt
                .initializer
                .as_ref()
                .map(|initializer| lower_for_initializer(initializer, class_name)),
            condition: for_stmt
                .condition
                .as_ref()
                .map(|expr| lower_expr(expr, class_name)),
            increment: for_stmt
                .increment
                .as_ref()
                .map(|increment| lower_for_increment(increment, class_name)),
            body: lower_block(&for_stmt.body, class_name),
            span: for_stmt.span,
        })),
        ast::Stmt::Break { span } => hir::Stmt::Break { span: *span },
        ast::Stmt::Continue { span } => hir::Stmt::Continue { span: *span },
        ast::Stmt::Foreach(foreach) => hir::Stmt::Foreach(hir::ForeachStmt {
            iterable: lower_expr(&foreach.iterable, class_name),
            key: foreach
                .key
                .as_ref()
                .map(|binding| lower_foreach_binding(binding, class_name)),
            value: lower_foreach_binding(&foreach.value, class_name),
            body: lower_block(&foreach.body, class_name),
            span: foreach.span,
        }),
        ast::Stmt::Increment(increment) => hir::Stmt::Increment(hir::IncrementStmt {
            target: lower_expr(&increment.target, class_name),
            op: increment.op.clone(),
            position: increment.position.clone(),
            span: increment.span,
        }),
        ast::Stmt::Expr { expr, span } => hir::Stmt::Expr {
            expr: lower_expr(expr, class_name),
            span: *span,
        },
    }
}

fn lower_for_initializer(
    initializer: &ast::ForInitializer,
    class_name: Option<&str>,
) -> hir::ForInitializer {
    match initializer {
        ast::ForInitializer::VarDecl(decl) => hir::ForInitializer::VarDecl(hir::VarDecl {
            writable: decl.writable,
            ty: decl.ty.as_ref().map(|ty| lower_type_ref(ty, class_name)),
            name: decl.name.clone(),
            initializer: lower_expr(&decl.initializer, class_name),
            span: decl.span,
        }),
        ast::ForInitializer::Assignment(assignment) => {
            hir::ForInitializer::Assignment(hir::Assignment {
                target: lower_expr(&assignment.target, class_name),
                op: assignment.op.clone(),
                value: lower_expr(&assignment.value, class_name),
                span: assignment.span,
            })
        }
    }
}

fn lower_for_increment(
    increment: &ast::ForIncrement,
    class_name: Option<&str>,
) -> hir::ForIncrement {
    match increment {
        ast::ForIncrement::Increment(increment) => {
            hir::ForIncrement::Increment(hir::IncrementStmt {
                target: lower_expr(&increment.target, class_name),
                op: increment.op.clone(),
                position: increment.position.clone(),
                span: increment.span,
            })
        }
        ast::ForIncrement::Assignment(assignment) => {
            hir::ForIncrement::Assignment(hir::Assignment {
                target: lower_expr(&assignment.target, class_name),
                op: assignment.op.clone(),
                value: lower_expr(&assignment.value, class_name),
                span: assignment.span,
            })
        }
    }
}

fn lower_if_stmt(if_stmt: &ast::IfStmt, class_name: Option<&str>) -> hir::IfStmt {
    hir::IfStmt {
        condition: lower_expr(&if_stmt.condition, class_name),
        then_block: lower_block(&if_stmt.then_block, class_name),
        else_branch: if_stmt
            .else_branch
            .as_ref()
            .map(|branch| lower_else_branch(branch, class_name)),
        span: if_stmt.span,
    }
}

fn lower_else_branch(branch: &ast::ElseBranch, class_name: Option<&str>) -> hir::ElseBranch {
    match branch {
        ast::ElseBranch::If(if_stmt) => {
            hir::ElseBranch::If(Box::new(lower_if_stmt(if_stmt, class_name)))
        }
        ast::ElseBranch::Block(block) => hir::ElseBranch::Block(lower_block(block, class_name)),
    }
}

fn lower_foreach_binding(
    binding: &ast::ForeachBinding,
    class_name: Option<&str>,
) -> hir::ForeachBinding {
    hir::ForeachBinding {
        ty: binding.ty.as_ref().map(|ty| lower_type_ref(ty, class_name)),
        name: binding.name.clone(),
    }
}

fn lower_expr(expr: &ast::Expr, class_name: Option<&str>) -> hir::Expr {
    match expr {
        ast::Expr::Variable { name, span } => hir::Expr::Variable {
            name: name.clone(),
            span: *span,
        },
        ast::Expr::This { span } => hir::Expr::This { span: *span },
        ast::Expr::Identifier { name, span } => hir::Expr::Identifier {
            name: name.clone(),
            span: *span,
        },
        ast::Expr::String { value, span } => hir::Expr::String {
            value: value.clone(),
            span: *span,
        },
        ast::Expr::InterpolatedString { parts, span } => hir::Expr::InterpolatedString {
            parts: parts
                .iter()
                .map(|part| lower_interpolated_string_part(part, class_name))
                .collect(),
            span: *span,
        },
        ast::Expr::Int { value, span } => hir::Expr::Int {
            value: value.clone(),
            span: *span,
        },
        ast::Expr::Float { value, span } => hir::Expr::Float {
            value: value.clone(),
            span: *span,
        },
        ast::Expr::Bool { value, span } => hir::Expr::Bool {
            value: *value,
            span: *span,
        },
        ast::Expr::Null { span } => hir::Expr::Null { span: *span },
        ast::Expr::Array { elements, span } => hir::Expr::Array {
            elements: elements
                .iter()
                .map(|element| lower_array_element(element, class_name))
                .collect(),
            span: *span,
        },
        ast::Expr::PropertyAccess {
            object,
            property,
            span,
        } => hir::Expr::PropertyAccess {
            object: Box::new(lower_expr(object, class_name)),
            property: property.clone(),
            span: *span,
        },
        ast::Expr::MethodCall {
            object,
            method,
            args,
            span,
        } => hir::Expr::MethodCall {
            object: Box::new(lower_expr(object, class_name)),
            method: method.clone(),
            args: args.iter().map(|arg| lower_expr(arg, class_name)).collect(),
            span: *span,
        },
        ast::Expr::FunctionCall { name, args, span } => hir::Expr::FunctionCall {
            name: name.clone(),
            args: args.iter().map(|arg| lower_expr(arg, class_name)).collect(),
            span: *span,
        },
        ast::Expr::StaticCall {
            qualifier,
            method,
            args,
            span,
            ..
        } => hir::Expr::StaticCall {
            class_name: resolved_qualifier_name(qualifier, class_name),
            method: method.clone(),
            args: args.iter().map(|arg| lower_expr(arg, class_name)).collect(),
            span: *span,
        },
        ast::Expr::StaticMember {
            qualifier,
            member,
            span,
            ..
        } => hir::Expr::StaticMember {
            class_name: resolved_qualifier_name(qualifier, class_name),
            member: member.clone(),
            span: *span,
        },
        ast::Expr::New {
            class_name: constructed_class,
            args,
            span,
        } => hir::Expr::New {
            class_name: constructed_class.clone(),
            args: args.iter().map(|arg| lower_expr(arg, class_name)).collect(),
            span: *span,
        },
        ast::Expr::Grouped { expr, span } => hir::Expr::Grouped {
            expr: Box::new(lower_expr(expr, class_name)),
            span: *span,
        },
        ast::Expr::Unary { op, expr, span } => hir::Expr::Unary {
            op: op.clone(),
            expr: Box::new(lower_expr(expr, class_name)),
            span: *span,
        },
        ast::Expr::Binary {
            left,
            op,
            right,
            span,
        } => hir::Expr::Binary {
            left: Box::new(lower_expr(left, class_name)),
            op: op.clone(),
            right: Box::new(lower_expr(right, class_name)),
            span: *span,
        },
        ast::Expr::Range {
            start,
            end,
            inclusive,
            span,
        } => hir::Expr::Range {
            start: Box::new(lower_expr(start, class_name)),
            end: Box::new(lower_expr(end, class_name)),
            inclusive: *inclusive,
            span: *span,
        },
    }
}

fn lower_interpolated_string_part(
    part: &ast::InterpolatedStringPart,
    class_name: Option<&str>,
) -> hir::InterpolatedStringPart {
    match part {
        ast::InterpolatedStringPart::Text { value, span } => hir::InterpolatedStringPart::Text {
            value: value.clone(),
            span: *span,
        },
        ast::InterpolatedStringPart::Expr(expr) => {
            hir::InterpolatedStringPart::Expr(lower_expr(expr, class_name))
        }
    }
}

fn lower_array_element(element: &ast::ArrayElement, class_name: Option<&str>) -> hir::ArrayElement {
    hir::ArrayElement {
        key: element.key.as_ref().map(|key| lower_expr(key, class_name)),
        value: lower_expr(&element.value, class_name),
    }
}

fn resolved_qualifier_name(qualifier: &ast::StaticQualifier, class_name: Option<&str>) -> String {
    match qualifier {
        ast::StaticQualifier::Class(name) => name.clone(),
        ast::StaticQualifier::SelfType => class_name
            .expect("checked `self::` access has a declaring class")
            .to_string(),
        ast::StaticQualifier::Parent | ast::StaticQualifier::InvalidStatic => {
            unreachable!("rejected or unsupported qualifier must not reach Doria IR lowering")
        }
    }
}

fn lower_type_ref(ty: &crate::types::TypeRef, class_name: Option<&str>) -> crate::types::TypeRef {
    class_name.map_or_else(|| ty.clone(), |class_name| ty.resolve_self_in(class_name))
}

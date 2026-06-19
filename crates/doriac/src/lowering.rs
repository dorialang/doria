use crate::{ast, hir};

pub fn lower_program(program: &ast::Program) -> hir::Program {
    hir::Program {
        items: program.items.iter().map(lower_item).collect(),
    }
}

fn lower_item(item: &ast::Item) -> hir::Item {
    match item {
        ast::Item::Class(class_decl) => hir::Item::Class(lower_class(class_decl)),
        ast::Item::Function(function) => hir::Item::Function(lower_function(function)),
        ast::Item::Statement(statement) => hir::Item::Statement(lower_stmt(statement)),
    }
}

fn lower_class(class_decl: &ast::ClassDecl) -> hir::ClassDecl {
    hir::ClassDecl {
        name: class_decl.name.clone(),
        members: class_decl.members.iter().map(lower_class_member).collect(),
        span: class_decl.span,
    }
}

fn lower_class_member(member: &ast::ClassMember) -> hir::ClassMember {
    match member {
        ast::ClassMember::Property(property) => {
            hir::ClassMember::Property(lower_property(property))
        }
        ast::ClassMember::Method(method) => hir::ClassMember::Method(lower_function(method)),
    }
}

fn lower_property(property: &ast::PropertyDecl) -> hir::PropertyDecl {
    hir::PropertyDecl {
        access: property.access.clone(),
        writable: property.writable,
        ty: property.ty.clone(),
        name: property.name.clone(),
        initializer: property.initializer.as_ref().map(lower_expr),
        span: property.span,
    }
}

fn lower_function(function: &ast::FunctionDecl) -> hir::FunctionDecl {
    hir::FunctionDecl {
        access: function.access.clone(),
        writable_this: function.writable_this,
        name: function.name.clone(),
        params: function.params.iter().map(lower_param).collect(),
        return_type: function.return_type.clone(),
        body: lower_block(&function.body),
        span: function.span,
    }
}

fn lower_param(param: &ast::Param) -> hir::Param {
    hir::Param {
        promoted_access: param.promoted_access.clone(),
        writable: param.writable,
        ty: param.ty.clone(),
        name: param.name.clone(),
        default: param.default.as_ref().map(lower_expr),
        span: param.span,
    }
}

fn lower_block(block: &ast::Block) -> hir::Block {
    hir::Block {
        statements: block.statements.iter().map(lower_stmt).collect(),
        span: block.span,
    }
}

fn lower_stmt(statement: &ast::Stmt) -> hir::Stmt {
    match statement {
        ast::Stmt::VarDecl(decl) => hir::Stmt::VarDecl(hir::VarDecl {
            writable: decl.writable,
            ty: decl.ty.clone(),
            name: decl.name.clone(),
            initializer: lower_expr(&decl.initializer),
            span: decl.span,
        }),
        ast::Stmt::Assignment(assignment) => hir::Stmt::Assignment(hir::Assignment {
            target: lower_expr(&assignment.target),
            op: assignment.op.clone(),
            value: lower_expr(&assignment.value),
            span: assignment.span,
        }),
        ast::Stmt::Echo { expr, span } => hir::Stmt::Echo {
            expr: lower_expr(expr),
            span: *span,
        },
        ast::Stmt::Return { expr, span } => hir::Stmt::Return {
            expr: expr.as_ref().map(lower_expr),
            span: *span,
        },
        ast::Stmt::Foreach(foreach) => hir::Stmt::Foreach(hir::ForeachStmt {
            iterable: lower_expr(&foreach.iterable),
            key: foreach.key.as_ref().map(lower_foreach_binding),
            value: lower_foreach_binding(&foreach.value),
            body: lower_block(&foreach.body),
            span: foreach.span,
        }),
        ast::Stmt::Expr { expr, span } => hir::Stmt::Expr {
            expr: lower_expr(expr),
            span: *span,
        },
    }
}

fn lower_foreach_binding(binding: &ast::ForeachBinding) -> hir::ForeachBinding {
    hir::ForeachBinding {
        ty: binding.ty.clone(),
        name: binding.name.clone(),
    }
}

fn lower_expr(expr: &ast::Expr) -> hir::Expr {
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
            parts: parts.iter().map(lower_interpolated_string_part).collect(),
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
            elements: elements.iter().map(lower_array_element).collect(),
            span: *span,
        },
        ast::Expr::PropertyAccess {
            object,
            property,
            span,
        } => hir::Expr::PropertyAccess {
            object: Box::new(lower_expr(object)),
            property: property.clone(),
            span: *span,
        },
        ast::Expr::MethodCall {
            object,
            method,
            args,
            span,
        } => hir::Expr::MethodCall {
            object: Box::new(lower_expr(object)),
            method: method.clone(),
            args: args.iter().map(lower_expr).collect(),
            span: *span,
        },
        ast::Expr::FunctionCall { name, args, span } => hir::Expr::FunctionCall {
            name: name.clone(),
            args: args.iter().map(lower_expr).collect(),
            span: *span,
        },
        ast::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } => hir::Expr::StaticCall {
            class_name: class_name.clone(),
            method: method.clone(),
            args: args.iter().map(lower_expr).collect(),
            span: *span,
        },
        ast::Expr::New {
            class_name,
            args,
            span,
        } => hir::Expr::New {
            class_name: class_name.clone(),
            args: args.iter().map(lower_expr).collect(),
            span: *span,
        },
        ast::Expr::Binary {
            left,
            op,
            right,
            span,
        } => hir::Expr::Binary {
            left: Box::new(lower_expr(left)),
            op: op.clone(),
            right: Box::new(lower_expr(right)),
            span: *span,
        },
    }
}

fn lower_interpolated_string_part(
    part: &ast::InterpolatedStringPart,
) -> hir::InterpolatedStringPart {
    match part {
        ast::InterpolatedStringPart::Text(text) => hir::InterpolatedStringPart::Text(text.clone()),
        ast::InterpolatedStringPart::Expr(expr) => {
            hir::InterpolatedStringPart::Expr(lower_expr(expr))
        }
    }
}

fn lower_array_element(element: &ast::ArrayElement) -> hir::ArrayElement {
    hir::ArrayElement {
        key: element.key.as_ref().map(lower_expr),
        value: lower_expr(&element.value),
    }
}

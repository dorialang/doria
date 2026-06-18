use crate::hir::*;
use crate::types::TypeRef;

pub fn generate(program: &Program) -> String {
    let mut output = String::from("<?php\n\n");
    for item in &program.items {
        emit_item(item, &mut output, 0);
        if !output.ends_with("\n\n") {
            output.push('\n');
        }
        output.push('\n');
    }
    output
}

fn emit_item(item: &Item, output: &mut String, indent: usize) {
    match item {
        Item::Class(class_decl) => emit_class(class_decl, output, indent),
        Item::Function(function) => emit_function(function, output, indent, false),
        Item::Statement(statement) => emit_statement(statement, output, indent),
    }
}

fn emit_class(class_decl: &ClassDecl, output: &mut String, indent: usize) {
    writeln(output, indent, &format!("class {}", class_decl.name));
    writeln(output, indent, "{");
    for member in &class_decl.members {
        match member {
            ClassMember::Property(property) => emit_property(property, output, indent + 1),
            ClassMember::Method(method) => emit_function(method, output, indent + 1, true),
        }
        output.push('\n');
    }
    writeln(output, indent, "}");
}

fn emit_property(property: &PropertyDecl, output: &mut String, indent: usize) {
    let visibility = emit_member_access(&property.access);
    let ty = php_type(&property.ty);
    write_indent(output, indent);
    output.push_str(visibility);
    output.push(' ');
    output.push_str(&ty);
    output.push_str(" $");
    output.push_str(&property.name);
    if let Some(initializer) = &property.initializer {
        output.push_str(" = ");
        output.push_str(&emit_expr(initializer));
    }
    output.push_str(";\n");
}

fn emit_function(function: &FunctionDecl, output: &mut String, indent: usize, is_method: bool) {
    write_indent(output, indent);
    if is_method {
        output.push_str(emit_member_access(&function.access));
        output.push(' ');
    }
    output.push_str("function ");
    output.push_str(&function.name);
    output.push('(');
    output.push_str(
        &function
            .params
            .iter()
            .map(emit_param)
            .collect::<Vec<_>>()
            .join(", "),
    );
    output.push(')');
    let is_lifecycle_method =
        is_method && matches!(function.name.as_str(), "__construct" | "__destruct");
    if let Some(return_type) = function
        .return_type
        .as_ref()
        .filter(|_| !is_lifecycle_method)
    {
        output.push_str(": ");
        output.push_str(&php_type(return_type));
    }
    output.push('\n');
    emit_block(&function.body, output, indent);
}

fn emit_param(param: &Param) -> String {
    let mut output = String::new();
    if let Some(access) = &param.promoted_access {
        output.push_str(emit_member_access(access));
        output.push(' ');
    }
    output.push_str(&php_type(&param.ty));
    output.push_str(" $");
    output.push_str(&param.name);
    if let Some(default) = &param.default {
        output.push_str(" = ");
        output.push_str(&emit_expr(default));
    }
    output
}

fn emit_block(block: &Block, output: &mut String, indent: usize) {
    writeln(output, indent, "{");
    for statement in &block.statements {
        emit_statement(statement, output, indent + 1);
    }
    writeln(output, indent, "}");
}

fn emit_statement(statement: &Stmt, output: &mut String, indent: usize) {
    match statement {
        Stmt::VarDecl(decl) => {
            writeln(
                output,
                indent,
                &format!("${} = {};", decl.name, emit_expr(&decl.initializer)),
            );
        }
        Stmt::Assignment(assignment) => {
            let op = match assignment.op {
                AssignOp::Assign => "=",
                AssignOp::AddAssign => "+=",
                AssignOp::SubAssign => "-=",
            };
            writeln(
                output,
                indent,
                &format!(
                    "{} {} {};",
                    emit_expr(&assignment.target),
                    op,
                    emit_expr(&assignment.value)
                ),
            );
        }
        Stmt::Echo { expr, .. } => {
            writeln(output, indent, &format!("echo {};", emit_expr(expr)));
        }
        Stmt::Return { expr, .. } => {
            if let Some(expr) = expr {
                writeln(output, indent, &format!("return {};", emit_expr(expr)));
            } else {
                writeln(output, indent, "return;");
            }
        }
        Stmt::Foreach(foreach) => {
            write_indent(output, indent);
            output.push_str("foreach (");
            output.push_str(&emit_expr(&foreach.iterable));
            output.push_str(" as ");
            if let Some(key) = &foreach.key {
                output.push('$');
                output.push_str(&key.name);
                output.push_str(" => ");
            }
            output.push('$');
            output.push_str(&foreach.value.name);
            output.push_str(")\n");
            emit_block(&foreach.body, output, indent);
        }
        Stmt::Expr { expr, .. } => {
            writeln(output, indent, &format!("{};", emit_expr(expr)));
        }
    }
}

fn emit_expr(expr: &Expr) -> String {
    match expr {
        Expr::Variable { name, .. } => format!("${name}"),
        Expr::This { .. } => "$this".to_string(),
        Expr::Identifier { name, .. } => name.clone(),
        Expr::String { value, .. } => format!("\"{}\"", escape_php_string(value)),
        Expr::Int { value, .. } | Expr::Float { value, .. } => value.clone(),
        Expr::Bool { value, .. } => {
            if *value {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        Expr::Null { .. } => "null".to_string(),
        Expr::Array { elements, .. } => {
            let inner = elements
                .iter()
                .map(|element| {
                    if let Some(key) = &element.key {
                        format!("{} => {}", emit_expr(key), emit_expr(&element.value))
                    } else {
                        emit_expr(&element.value)
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{inner}]")
        }
        Expr::PropertyAccess {
            object, property, ..
        } => format!("{}->{property}", emit_expr(object)),
        Expr::MethodCall {
            object,
            method,
            args,
            ..
        } => format!(
            "{}->{method}({})",
            emit_expr(object),
            args.iter().map(emit_expr).collect::<Vec<_>>().join(", ")
        ),
        Expr::FunctionCall { name, args, .. } => format!(
            "{name}({})",
            args.iter().map(emit_expr).collect::<Vec<_>>().join(", ")
        ),
        Expr::StaticCall {
            class_name,
            method,
            args,
            ..
        } => format!(
            "{class_name}::{method}({})",
            args.iter().map(emit_expr).collect::<Vec<_>>().join(", ")
        ),
        Expr::New {
            class_name, args, ..
        } => format!(
            "new {class_name}({})",
            args.iter().map(emit_expr).collect::<Vec<_>>().join(", ")
        ),
        Expr::Binary {
            left, op, right, ..
        } => format!(
            "{} {} {}",
            emit_expr(left),
            emit_binary_op(op),
            emit_expr(right)
        ),
    }
}

fn emit_binary_op(op: &BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
        BinaryOp::Mod => "%",
        BinaryOp::Concat => ".",
        BinaryOp::Equal => "==",
        BinaryOp::StrictEqual => "===",
        BinaryOp::NotEqual => "!=",
        BinaryOp::NotStrictEqual => "!==",
        BinaryOp::Less => "<",
        BinaryOp::LessEqual => "<=",
        BinaryOp::Greater => ">",
        BinaryOp::GreaterEqual => ">=",
        BinaryOp::And => "&&",
        BinaryOp::Or => "||",
        BinaryOp::Coalesce => "??",
    }
}

fn emit_member_access(access: &MemberAccess) -> &'static str {
    match access {
        MemberAccess::External => "public",
        MemberAccess::Internal => "private",
    }
}

fn php_type(ty: &TypeRef) -> String {
    match ty.name.as_str() {
        "List" | "Dictionary" | "Set" => "array".to_string(),
        "resource" => "mixed".to_string(),
        name => name.to_string(),
    }
}

fn escape_php_string(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            _ => output.push(character),
        }
    }
    output
}

fn writeln(output: &mut String, indent: usize, line: &str) {
    write_indent(output, indent);
    output.push_str(line);
    output.push('\n');
}

fn write_indent(output: &mut String, indent: usize) {
    for _ in 0..indent {
        output.push_str("    ");
    }
}

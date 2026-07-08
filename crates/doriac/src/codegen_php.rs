use std::collections::{HashMap, HashSet};

use crate::hir::*;
use crate::types::TypeRef;

pub fn generate(program: &Program) -> String {
    let mut output = String::from("<?php\n\n");
    let mut scopes = PhpNameScopes::new();
    for item in &program.items {
        emit_item(item, &mut output, 0, &mut scopes);
        if !output.ends_with("\n\n") {
            output.push('\n');
        }
        output.push('\n');
    }
    output
}

#[derive(Debug, Default)]
struct PhpNameScopes {
    scopes: Vec<HashMap<String, String>>,
    used_php_names: HashSet<String>,
    next_mangled_id: usize,
}

impl PhpNameScopes {
    fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()],
            used_php_names: HashSet::new(),
            next_mangled_id: 0,
        }
    }

    fn push(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop(&mut self) {
        self.scopes.pop();
    }

    fn declare(&mut self, name: &str) -> String {
        let shadows_outer = self.lookup(name).is_some();
        let php_name = if shadows_outer || self.used_php_names.contains(name) {
            self.next_mangled_name(name)
        } else {
            name.to_string()
        };
        self.insert_current(name, php_name.clone());
        php_name
    }

    fn declare_unmangled(&mut self, name: &str) -> String {
        let php_name = name.to_string();
        self.insert_current(name, php_name.clone());
        php_name
    }

    fn fresh_temp(&mut self, prefix: &str) -> String {
        loop {
            self.next_mangled_id += 1;
            let candidate = format!("{prefix}__doria{}", self.next_mangled_id);
            if !self.used_php_names.contains(&candidate) {
                self.used_php_names.insert(candidate.clone());
                return candidate;
            }
        }
    }

    fn lookup(&self, name: &str) -> Option<&str> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name))
            .map(String::as_str)
    }

    fn php_name(&self, name: &str) -> String {
        self.lookup(name).unwrap_or(name).to_string()
    }

    fn insert_current(&mut self, name: &str, php_name: String) {
        self.used_php_names.insert(php_name.clone());
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), php_name);
        }
    }

    fn next_mangled_name(&mut self, name: &str) -> String {
        loop {
            self.next_mangled_id += 1;
            let candidate = format!("{name}__doria{}", self.next_mangled_id);
            if !self.used_php_names.contains(&candidate) {
                return candidate;
            }
        }
    }
}

fn emit_item(item: &Item, output: &mut String, indent: usize, scopes: &mut PhpNameScopes) {
    match item {
        Item::Class(class_decl) => emit_class(class_decl, output, indent),
        Item::Function(function) => emit_function(function, output, indent, false),
        Item::Statement(statement) => emit_statement(statement, output, indent, scopes),
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
        let scopes = PhpNameScopes::new();
        output.push_str(" = ");
        output.push_str(&emit_expr(initializer, &scopes));
    }
    output.push_str(";\n");
}

fn emit_function(function: &FunctionDecl, output: &mut String, indent: usize, is_method: bool) {
    let mut scopes = PhpNameScopes::new();
    for param in &function.params {
        scopes.declare_unmangled(&param.name);
    }

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
            .map(|param| emit_param(param, &scopes))
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
    emit_block(&function.body, output, indent, &mut scopes);
}

fn emit_param(param: &Param, scopes: &PhpNameScopes) -> String {
    let mut output = String::new();
    if let Some(access) = &param.promoted_access {
        output.push_str(emit_member_access(access));
        output.push(' ');
    }
    output.push_str(&php_type(&param.ty));
    output.push_str(" $");
    output.push_str(&scopes.php_name(&param.name));
    if let Some(default) = &param.default {
        output.push_str(" = ");
        output.push_str(&emit_expr(default, scopes));
    }
    output
}

fn emit_block(block: &Block, output: &mut String, indent: usize, scopes: &mut PhpNameScopes) {
    writeln(output, indent, "{");
    scopes.push();
    for statement in &block.statements {
        emit_statement(statement, output, indent + 1, scopes);
    }
    scopes.pop();
    writeln(output, indent, "}");
}

fn emit_statement(
    statement: &Stmt,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    match statement {
        Stmt::VarDecl(decl) => {
            let initializer = emit_expr(&decl.initializer, scopes);
            let php_name = scopes.declare(&decl.name);
            writeln(output, indent, &format!("${php_name} = {initializer};"));
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
                    emit_assignment_target(&assignment.target, scopes),
                    op,
                    emit_expr(&assignment.value, scopes)
                ),
            );
        }
        Stmt::Echo { expr, .. } => {
            writeln(
                output,
                indent,
                &format!("echo {};", emit_expr(expr, scopes)),
            );
        }
        Stmt::Return { expr, .. } => {
            if let Some(expr) = expr {
                writeln(
                    output,
                    indent,
                    &format!("return {};", emit_expr(expr, scopes)),
                );
            } else {
                writeln(output, indent, "return;");
            }
        }
        Stmt::If(if_stmt) => emit_if(if_stmt, output, indent, "if", scopes),
        Stmt::While(while_stmt) => {
            write_indent(output, indent);
            output.push_str("while (");
            output.push_str(&emit_expr(&while_stmt.condition, scopes));
            output.push_str(")\n");
            emit_block(&while_stmt.body, output, indent, scopes);
        }
        Stmt::For(for_stmt) => emit_for(for_stmt, output, indent, scopes),
        Stmt::Break { .. } => {
            writeln(output, indent, "break;");
        }
        Stmt::Continue { .. } => {
            writeln(output, indent, "continue;");
        }
        Stmt::Foreach(foreach) => emit_foreach(foreach, output, indent, scopes),
        Stmt::Increment(increment) => {
            writeln(
                output,
                indent,
                &format!("{};", emit_increment(increment, scopes)),
            );
        }
        Stmt::Expr { expr, .. } => {
            writeln(output, indent, &format!("{};", emit_expr(expr, scopes)));
        }
    }
}

fn emit_for(for_stmt: &ForStmt, output: &mut String, indent: usize, scopes: &mut PhpNameScopes) {
    scopes.push();
    let initializer = for_stmt
        .initializer
        .as_ref()
        .map(|initializer| emit_for_initializer(initializer, scopes))
        .unwrap_or_default();
    let condition = for_stmt
        .condition
        .as_ref()
        .map(|condition| emit_expr(condition, scopes))
        .unwrap_or_default();
    let increment = for_stmt
        .increment
        .as_ref()
        .map(|increment| emit_for_increment(increment, scopes))
        .unwrap_or_default();

    write_indent(output, indent);
    output.push_str("for (");
    output.push_str(&initializer);
    output.push_str("; ");
    output.push_str(&condition);
    output.push_str("; ");
    output.push_str(&increment);
    output.push_str(")\n");
    emit_block(&for_stmt.body, output, indent, scopes);
    scopes.pop();
}

fn emit_for_initializer(initializer: &ForInitializer, scopes: &mut PhpNameScopes) -> String {
    match initializer {
        ForInitializer::VarDecl(decl) => {
            let initializer = emit_expr(&decl.initializer, scopes);
            let php_name = scopes.declare(&decl.name);
            format!("${php_name} = {initializer}")
        }
        ForInitializer::Assignment(assignment) => emit_assignment(assignment, scopes),
    }
}

fn emit_for_increment(increment: &ForIncrement, scopes: &PhpNameScopes) -> String {
    match increment {
        ForIncrement::Increment(increment) => emit_increment(increment, scopes),
        ForIncrement::Assignment(assignment) => emit_assignment(assignment, scopes),
    }
}

fn emit_assignment(assignment: &Assignment, scopes: &PhpNameScopes) -> String {
    let op = match assignment.op {
        AssignOp::Assign => "=",
        AssignOp::AddAssign => "+=",
        AssignOp::SubAssign => "-=",
    };
    format!(
        "{} {} {}",
        emit_assignment_target(&assignment.target, scopes),
        op,
        emit_expr(&assignment.value, scopes)
    )
}

fn emit_increment(increment: &IncrementStmt, scopes: &PhpNameScopes) -> String {
    let target = emit_assignment_target(&increment.target, scopes);
    let op = match increment.op {
        IncrementOp::Increment => "++",
        IncrementOp::Decrement => "--",
    };
    match increment.position {
        IncrementPosition::Pre => format!("{op}{target}"),
        IncrementPosition::Post => format!("{target}{op}"),
    }
}

fn emit_assignment_target(expr: &Expr, scopes: &PhpNameScopes) -> String {
    match expr {
        Expr::Grouped { expr, .. } => emit_assignment_target(expr, scopes),
        _ => emit_expr(expr, scopes),
    }
}

fn emit_if(
    if_stmt: &IfStmt,
    output: &mut String,
    indent: usize,
    keyword: &str,
    scopes: &mut PhpNameScopes,
) {
    write_indent(output, indent);
    output.push_str(keyword);
    output.push_str(" (");
    output.push_str(&emit_expr(&if_stmt.condition, scopes));
    output.push_str(")\n");
    emit_block(&if_stmt.then_block, output, indent, scopes);

    if let Some(else_branch) = &if_stmt.else_branch {
        match else_branch {
            ElseBranch::If(else_if) => emit_if(else_if, output, indent, "else if", scopes),
            ElseBranch::Block(block) => {
                write_indent(output, indent);
                output.push_str("else\n");
                emit_block(block, output, indent, scopes);
            }
        }
    }
}

fn emit_foreach(
    foreach: &ForeachStmt,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    if let Some((start, end, inclusive)) = grouped_range_expr(&foreach.iterable) {
        emit_range_foreach(foreach, start, end, inclusive, output, indent, scopes);
        return;
    }

    let iterable = emit_expr(&foreach.iterable, scopes);
    scopes.push();
    let key_name = foreach.key.as_ref().map(|key| scopes.declare(&key.name));
    let value_name = scopes.declare(&foreach.value.name);

    write_indent(output, indent);
    output.push_str("foreach (");
    output.push_str(&iterable);
    output.push_str(" as ");
    if let Some(key_name) = key_name {
        output.push('$');
        output.push_str(&key_name);
        output.push_str(" => ");
    }
    output.push('$');
    output.push_str(&value_name);
    output.push_str(")\n");
    writeln(output, indent, "{");
    for statement in &foreach.body.statements {
        emit_statement(statement, output, indent + 1, scopes);
    }
    scopes.pop();
    writeln(output, indent, "}");
}

fn grouped_range_expr(expr: &Expr) -> Option<(&Expr, &Expr, bool)> {
    match expr {
        Expr::Grouped { expr, .. } => grouped_range_expr(expr),
        Expr::Range {
            start,
            end,
            inclusive,
            ..
        } => Some((start, end, *inclusive)),
        _ => None,
    }
}

fn emit_range_foreach(
    foreach: &ForeachStmt,
    start: &Expr,
    end: &Expr,
    inclusive: bool,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    let start_expr = emit_expr(start, scopes);
    let end_expr = emit_expr(end, scopes);
    let start_temp = scopes.fresh_temp("__doria_range_start");
    let end_temp = scopes.fresh_temp("__doria_range_end");

    writeln(output, indent, &format!("${start_temp} = {start_expr};"));
    writeln(output, indent, &format!("${end_temp} = {end_expr};"));

    scopes.push();
    let value_name = scopes.declare(&foreach.value.name);
    let done_temp = inclusive.then(|| scopes.fresh_temp("__doria_range_done"));
    if let Some(done_temp) = &done_temp {
        writeln(output, indent, &format!("${done_temp} = false;"));
    }

    write_indent(output, indent);
    output.push_str("for ($");
    output.push_str(&value_name);
    output.push_str(" = $");
    output.push_str(&start_temp);
    output.push_str("; ");
    if let Some(done_temp) = &done_temp {
        output.push_str("!$");
        output.push_str(done_temp);
        output.push_str(" && ");
    }
    output.push('$');
    output.push_str(&value_name);
    output.push(' ');
    output.push_str(if inclusive { "<=" } else { "<" });
    output.push_str(" $");
    output.push_str(&end_temp);
    output.push_str("; ");
    if let Some(done_temp) = &done_temp {
        output.push('$');
        output.push_str(&value_name);
        output.push_str(" < $");
        output.push_str(&end_temp);
        output.push_str(" ? $");
        output.push_str(&value_name);
        output.push_str("++ : ($");
        output.push_str(done_temp);
        output.push_str(" = true)");
    } else {
        output.push('$');
        output.push_str(&value_name);
        output.push_str("++");
    }
    output.push_str(")\n");
    writeln(output, indent, "{");
    for statement in &foreach.body.statements {
        emit_statement(statement, output, indent + 1, scopes);
    }
    scopes.pop();
    writeln(output, indent, "}");
}

fn emit_expr(expr: &Expr, scopes: &PhpNameScopes) -> String {
    match expr {
        Expr::Variable { name, .. } => format!("${}", scopes.php_name(name)),
        Expr::This { .. } => "$this".to_string(),
        Expr::Identifier { name, .. } => name.clone(),
        Expr::String { value, .. } => emit_php_string_literal(value),
        Expr::InterpolatedString { parts, .. } => emit_interpolated_string(parts, scopes),
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
                        format!(
                            "{} => {}",
                            emit_expr(key, scopes),
                            emit_expr(&element.value, scopes)
                        )
                    } else {
                        emit_expr(&element.value, scopes)
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{inner}]")
        }
        Expr::PropertyAccess {
            object, property, ..
        } => format!("{}->{property}", emit_expr(object, scopes)),
        Expr::MethodCall {
            object,
            method,
            args,
            ..
        } => format!(
            "{}->{method}({})",
            emit_expr(object, scopes),
            args.iter()
                .map(|arg| emit_expr(arg, scopes))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Expr::FunctionCall { name, args, .. } => format!(
            "{name}({})",
            args.iter()
                .map(|arg| emit_expr(arg, scopes))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Expr::StaticCall {
            class_name,
            method,
            args,
            ..
        } => format!(
            "{class_name}::{method}({})",
            args.iter()
                .map(|arg| emit_expr(arg, scopes))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Expr::New {
            class_name, args, ..
        } => format!(
            "new {class_name}({})",
            args.iter()
                .map(|arg| emit_expr(arg, scopes))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Expr::Grouped { expr, .. } => format!("({})", emit_expr(expr, scopes)),
        Expr::Unary { op, expr, .. } => match op {
            UnaryOp::Not => format!("!({})", emit_expr(expr, scopes)),
        },
        Expr::Binary {
            left, op, right, ..
        } => match op {
            BinaryOp::And => format!(
                "(({}) && ({}))",
                emit_expr(left, scopes),
                emit_expr(right, scopes)
            ),
            BinaryOp::Or => format!(
                "(({}) || ({}))",
                emit_expr(left, scopes),
                emit_expr(right, scopes)
            ),
            BinaryOp::Xor => format!(
                "(({}) !== ({}))",
                emit_expr(left, scopes),
                emit_expr(right, scopes)
            ),
            _ => format!(
                "{} {} {}",
                emit_expr(left, scopes),
                emit_binary_op(op),
                emit_expr(right, scopes)
            ),
        },
        Expr::Range { start, end, .. } => format!(
            "null /* unsupported range expression {}..{} */",
            emit_expr(start, scopes),
            emit_expr(end, scopes)
        ),
    }
}

fn emit_interpolated_string(parts: &[InterpolatedStringPart], scopes: &PhpNameScopes) -> String {
    let mut emitted = Vec::new();
    let mut has_expr = false;

    for part in parts {
        match part {
            InterpolatedStringPart::Text(text) => {
                if !text.is_empty() {
                    emitted.push(emit_php_string_literal(text));
                }
            }
            InterpolatedStringPart::Expr(expr) => {
                has_expr = true;
                emitted.push(emit_expr(expr, scopes));
            }
        }
    }

    match emitted.len() {
        0 => emit_php_string_literal(""),
        1 if has_expr => format!("{} . {}", emit_php_string_literal(""), emitted[0]),
        _ => emitted.join(" . "),
    }
}

fn emit_php_string_literal(value: &str) -> String {
    format!("\"{}\"", escape_php_string(value))
}

fn emit_binary_op(op: &BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
        BinaryOp::Mod => "%",
        BinaryOp::Concat => ".",
        BinaryOp::Equal => "===",
        BinaryOp::NotEqual => "!==",
        BinaryOp::Less => "<",
        BinaryOp::LessEqual => "<=",
        BinaryOp::Greater => ">",
        BinaryOp::GreaterEqual => ">=",
        BinaryOp::And => "&&",
        BinaryOp::Or => "||",
        BinaryOp::Xor => unreachable!("xor is emitted by the boolean-specialized binary branch"),
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
        name => name.to_string(),
    }
}

fn escape_php_string(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '$' => output.push_str("\\$"),
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

use std::collections::{HashMap, HashSet};

use crate::backend::BackendError;
use crate::diagnostics::Diagnostic;
use crate::format_string::{self, FormatConversion, FormatPiece};
use crate::hir::*;
use crate::numeric::{parse_decimal_magnitude, FloatType, IntegerType};
use crate::semantics::SemanticInfo;
use crate::source::Span;
use crate::types::TypeRef;

const PHP_INTEGER_UNSUPPORTED_CODE: &str = "B1301";

pub fn generate(program: &Program) -> Result<String, BackendError> {
    validate_program(program)?;

    let mut output = String::from(
        "<?php\n\nfunction __doria_display(string|int|bool $value): string\n{\n    if (is_bool($value)) { return $value ? 'true' : 'false'; }\n    return (string) $value;\n}\n\nfunction __doria_less(string|int|float|bool $left, string|int|float|bool $right): bool\n{\n    if (is_string($left) && is_string($right)) { return strcmp($left, $right) < 0; }\n    return $left < $right;\n}\n\nfunction __doria_less_equal(string|int|float|bool $left, string|int|float|bool $right): bool\n{\n    if (is_string($left) && is_string($right)) { return strcmp($left, $right) <= 0; }\n    return $left <= $right;\n}\n\nfunction __doria_greater(string|int|float|bool $left, string|int|float|bool $right): bool\n{\n    if (is_string($left) && is_string($right)) { return strcmp($left, $right) > 0; }\n    return $left > $right;\n}\n\nfunction __doria_greater_equal(string|int|float|bool $left, string|int|float|bool $right): bool\n{\n    if (is_string($left) && is_string($right)) { return strcmp($left, $right) >= 0; }\n    return $left >= $right;\n}\n\n",
    );
    output.push_str(
        r#"function __doria_io_panic(string $message): never
{
    fwrite(STDERR, "Panic: " . $message . "\nStack Trace:\n");
    foreach (debug_backtrace(DEBUG_BACKTRACE_IGNORE_ARGS) as $frame) {
        if (isset($frame["function"]) && !str_starts_with($frame["function"], "__doria_")) {
            fwrite(STDERR, "  at " . $frame["function"] . "\n");
        }
    }
    exit(101);
}

function __doria_readline(): ?string
{
    $line = fgets(STDIN);
    if ($line === false) { return null; }
    if (str_ends_with($line, "\n")) {
        $line = substr($line, 0, -1);
        if (str_ends_with($line, "\r")) { $line = substr($line, 0, -1); }
    }
    return $line;
}

function __doria_read_file(string $path): string
{
    $contents = file_get_contents($path);
    if ($contents === false) { __doria_io_panic("failed to read file"); }
    if (preg_match('//u', $contents) !== 1) { __doria_io_panic("file contained invalid UTF-8"); }
    return $contents;
}

function __doria_write_file(string $path, string $contents): void
{
    $written = file_put_contents($path, $contents);
    if ($written === false || $written !== strlen($contents)) { __doria_io_panic("failed to write file"); }
}

function __doria_write_stderr(string $value): void
{
    $written = fwrite(STDERR, $value);
    if ($written === false || $written !== strlen($value)) { __doria_io_panic("failed to write stderr"); }
}

function __doria_sprintf(string $format, mixed ...$values): string
{
    return sprintf($format, ...$values);
}

function __doria_printf(string $format, mixed ...$values): void
{
    $value = sprintf($format, ...$values);
    $written = fwrite(STDOUT, $value);
    if ($written === false || $written !== strlen($value)) { __doria_io_panic("failed to write stdout"); }
}

"#,
    );
    let mut scopes = PhpNameScopes::new();
    for item in &program.items {
        emit_item(item, &mut output, 0, &mut scopes);
        if !output.ends_with("\n\n") {
            output.push('\n');
        }
        output.push('\n');
    }
    Ok(output)
}

fn validate_program(program: &Program) -> Result<(), BackendError> {
    for item in &program.items {
        validate_item(item, &program.semantic_info)?;
    }
    Ok(())
}

fn validate_item(item: &Item, semantic_info: &SemanticInfo) -> Result<(), BackendError> {
    match item {
        Item::Class(class_decl) => {
            for member in &class_decl.members {
                match member {
                    ClassMember::Property(property) => {
                        validate_type(&property.ty, property.span)?;
                        if let Some(initializer) = &property.initializer {
                            validate_expr(initializer, semantic_info)?;
                        }
                    }
                    ClassMember::Method(method) => validate_function(method, semantic_info)?,
                }
            }
            Ok(())
        }
        Item::Function(function) => validate_function(function, semantic_info),
        Item::Statement(statement) => validate_statement(statement, semantic_info),
    }
}

fn validate_function(
    function: &FunctionDecl,
    semantic_info: &SemanticInfo,
) -> Result<(), BackendError> {
    for param in &function.params {
        validate_type(&param.ty, param.span)?;
        if let Some(default) = &param.default {
            validate_expr(default, semantic_info)?;
        }
    }
    if let Some(return_type) = &function.return_type {
        validate_type(return_type, function.span)?;
    }
    validate_block(&function.body, semantic_info)
}

fn validate_type(ty: &TypeRef, span: Span) -> Result<(), BackendError> {
    if let Some(integer) = IntegerType::from_source_name(&ty.name) {
        if !integer.is_default_int() {
            return Err(unsupported_integer_shape(
                span,
                format!(
                    "Doria `{}` width and signedness with PHP's single signed integer type",
                    ty.name
                ),
            ));
        }
    }
    if let Some(float) = FloatType::from_source_name(&ty.name) {
        if !float.is_default_float() {
            return Err(unsupported_numeric_shape(
                span,
                "Doria `float32` precision with PHP's platform `float` type",
            ));
        }
    }
    for argument in &ty.args {
        validate_type(argument, span)?;
    }
    Ok(())
}

fn validate_block(block: &Block, semantic_info: &SemanticInfo) -> Result<(), BackendError> {
    for statement in &block.statements {
        validate_statement(statement, semantic_info)?;
    }
    Ok(())
}

fn validate_statement(statement: &Stmt, semantic_info: &SemanticInfo) -> Result<(), BackendError> {
    match statement {
        Stmt::VarDecl(decl) => {
            if let Some(ty) = &decl.ty {
                validate_type(ty, decl.span)?;
            }
            validate_expr(&decl.initializer, semantic_info)
        }
        Stmt::Assignment(assignment) => validate_assignment(assignment, semantic_info),
        Stmt::Echo { expr, .. } => validate_display_expr(expr, semantic_info),
        Stmt::Return { expr, .. } => {
            if let Some(expr) = expr {
                validate_expr(expr, semantic_info)?;
            }
            Ok(())
        }
        Stmt::If(if_stmt) => validate_if(if_stmt, semantic_info),
        Stmt::While(while_stmt) => {
            validate_expr(&while_stmt.condition, semantic_info)?;
            validate_block(&while_stmt.body, semantic_info)
        }
        Stmt::For(for_stmt) => {
            if let Some(initializer) = &for_stmt.initializer {
                match initializer {
                    ForInitializer::VarDecl(decl) => {
                        if let Some(ty) = &decl.ty {
                            validate_type(ty, decl.span)?;
                        }
                        validate_expr(&decl.initializer, semantic_info)?;
                    }
                    ForInitializer::Assignment(assignment) => {
                        validate_assignment(assignment, semantic_info)?;
                    }
                }
            }
            if let Some(condition) = &for_stmt.condition {
                validate_expr(condition, semantic_info)?;
            }
            if let Some(increment) = &for_stmt.increment {
                match increment {
                    ForIncrement::Increment(increment) => {
                        return Err(unsupported_increment(increment));
                    }
                    ForIncrement::Assignment(assignment) => {
                        validate_assignment(assignment, semantic_info)?;
                    }
                }
            }
            validate_block(&for_stmt.body, semantic_info)
        }
        Stmt::Break { .. } | Stmt::Continue { .. } => Ok(()),
        Stmt::Foreach(foreach) => {
            validate_expr(&foreach.iterable, semantic_info)?;
            if let Some(key) = &foreach.key {
                if let Some(ty) = &key.ty {
                    validate_type(ty, foreach.span)?;
                }
            }
            if let Some(ty) = &foreach.value.ty {
                validate_type(ty, foreach.span)?;
            }
            validate_block(&foreach.body, semantic_info)
        }
        Stmt::Increment(increment) => Err(unsupported_increment(increment)),
        Stmt::Expr { expr, .. } => validate_expr(expr, semantic_info),
    }
}

fn validate_if(if_stmt: &IfStmt, semantic_info: &SemanticInfo) -> Result<(), BackendError> {
    validate_expr(&if_stmt.condition, semantic_info)?;
    validate_block(&if_stmt.then_block, semantic_info)?;
    if let Some(else_branch) = &if_stmt.else_branch {
        match else_branch {
            ElseBranch::If(else_if) => validate_if(else_if, semantic_info)?,
            ElseBranch::Block(block) => validate_block(block, semantic_info)?,
        }
    }
    Ok(())
}

fn validate_assignment(
    assignment: &Assignment,
    semantic_info: &SemanticInfo,
) -> Result<(), BackendError> {
    validate_expr(&assignment.target, semantic_info)?;
    validate_expr(&assignment.value, semantic_info)?;

    // Semantic checking has already required compound-assignment operands to
    // have one compatible numeric type. The value metadata is sufficient here
    // because assignment targets are not expression-valued in Doria IR.
    let float_assignment = semantic_info.float_type(assignment.value.span()).is_some();
    let feature = match assignment.op {
        AssignOp::Assign => None,
        AssignOp::AddAssign if float_assignment => None,
        AssignOp::SubAssign if float_assignment => None,
        AssignOp::MulAssign if float_assignment => None,
        AssignOp::DivAssign if float_assignment => None,
        AssignOp::AddAssign => Some("checked integer overflow behavior for `+=`"),
        AssignOp::SubAssign => Some("checked integer overflow behavior for `-=`"),
        AssignOp::MulAssign => Some("checked integer overflow behavior for `*=`"),
        AssignOp::DivAssign => Some("Doria integer division semantics for `/=`"),
        AssignOp::ModAssign => Some("Doria integer remainder semantics for `%=`"),
        AssignOp::ShiftLeftAssign => Some("Doria integer shift semantics for `<<=`"),
        AssignOp::ShiftRightAssign => Some("Doria integer shift semantics for `>>=`"),
        AssignOp::BitwiseAndAssign => Some("fixed-width Doria bitwise semantics for `&=`"),
        AssignOp::BitwiseOrAssign => Some("fixed-width Doria bitwise semantics for `|=`"),
        AssignOp::BitwiseXorAssign => Some("fixed-width Doria bitwise semantics for `^=`"),
    };
    if let Some(feature) = feature {
        return Err(unsupported_integer_shape(assignment.span, feature));
    }
    Ok(())
}

fn unsupported_increment(increment: &IncrementStmt) -> BackendError {
    let operator = match increment.op {
        IncrementOp::Increment => "++",
        IncrementOp::Decrement => "--",
    };
    unsupported_integer_shape(
        increment.span,
        format!("checked integer overflow behavior for `{operator}`"),
    )
}

fn validate_expr(expr: &Expr, semantic_info: &SemanticInfo) -> Result<(), BackendError> {
    match expr {
        Expr::Variable { .. }
        | Expr::This { .. }
        | Expr::Identifier { .. }
        | Expr::String { .. }
        | Expr::Float { .. }
        | Expr::Bool { .. }
        | Expr::Null { .. } => Ok(()),
        Expr::Int { value, span } => {
            if parse_decimal_magnitude(value).is_some_and(|value| value > i64::MAX as u128) {
                return Err(unsupported_integer_shape(
                    *span,
                    format!(
                        "integer literal `{value}` outside PHP's signed integer range; the `uint64` maximum must not become a PHP float"
                    ),
                ));
            }
            Ok(())
        }
        Expr::InterpolatedString { parts, .. } => {
            for part in parts {
                if let InterpolatedStringPart::Expr(expr) = part {
                    validate_display_expr(expr, semantic_info)?;
                }
            }
            Ok(())
        }
        Expr::Array { elements, .. } => {
            for element in elements {
                if let Some(key) = &element.key {
                    validate_expr(key, semantic_info)?;
                }
                validate_expr(&element.value, semantic_info)?;
            }
            Ok(())
        }
        Expr::PropertyAccess { object, .. } => validate_expr(object, semantic_info),
        Expr::MethodCall { object, args, .. } => {
            validate_expr(object, semantic_info)?;
            validate_exprs(args, semantic_info)
        }
        Expr::FunctionCall { args, .. } | Expr::New { args, .. } => {
            validate_exprs(args, semantic_info)
        }
        Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } => {
            if (class_name == "Int" && method == "toFloat")
                || (class_name == "Float" && method == "toInt")
            {
                return Err(unsupported_numeric_shape(
                    *span,
                    format!("exact Doria conversion semantics for `{class_name}::{method}(...)`"),
                ));
            }
            if IntegerType::from_companion_name(class_name).is_some() && method == "from" {
                return Err(unsupported_integer_shape(
                    *span,
                    format!(
                        "checked Doria integer conversion semantics for `{class_name}::from(...)`"
                    ),
                ));
            }
            validate_exprs(args, semantic_info)
        }
        Expr::Grouped { expr, .. } => validate_expr(expr, semantic_info),
        Expr::Unary { op, expr, span } => {
            if *op == UnaryOp::Negate {
                if let Some(magnitude) = integer_literal_magnitude(expr) {
                    if magnitude <= (i64::MAX as u128) + 1 {
                        return Ok(());
                    }
                    return Err(unsupported_integer_shape(
                        *span,
                        "an integer literal outside PHP's signed integer range",
                    ));
                }
            }
            let feature = match op {
                UnaryOp::Not => None,
                UnaryOp::Negate if semantic_info.float_type(expr.span()).is_some() => None,
                UnaryOp::Negate => Some("checked integer overflow behavior for unary `-`"),
                UnaryOp::BitwiseNot => Some("fixed-width Doria bitwise semantics for `~`"),
            };
            if let Some(feature) = feature {
                return Err(unsupported_integer_shape(*span, feature));
            }
            validate_expr(expr, semantic_info)
        }
        Expr::Binary {
            left,
            op,
            right,
            span,
        } => {
            validate_expr(left, semantic_info)?;
            validate_expr(right, semantic_info)?;
            if *op == BinaryOp::Concat {
                validate_display_expr(left, semantic_info)?;
                validate_display_expr(right, semantic_info)?;
            }
            let float_operands = matches!(
                (
                    semantic_info.float_type(left.span()),
                    semantic_info.float_type(right.span()),
                ),
                (Some(left), Some(right)) if left == right
            );
            let feature = match op {
                BinaryOp::Add if float_operands => None,
                BinaryOp::Sub if float_operands => None,
                BinaryOp::Mul if float_operands => None,
                BinaryOp::Div if float_operands => None,
                BinaryOp::Add => Some("checked integer overflow behavior for `+`"),
                BinaryOp::Sub => Some("checked integer overflow behavior for `-`"),
                BinaryOp::Mul => Some("checked integer overflow behavior for `*`"),
                BinaryOp::Div => Some("Doria integer division semantics for `/`"),
                BinaryOp::Mod => Some("Doria integer remainder semantics for `%`"),
                BinaryOp::ShiftLeft => Some("Doria integer shift semantics for `<<`"),
                BinaryOp::ShiftRight => Some("Doria integer shift semantics for `>>`"),
                BinaryOp::BitwiseAnd => Some("fixed-width Doria bitwise semantics for `&`"),
                BinaryOp::BitwiseXor => Some("fixed-width Doria bitwise semantics for `^`"),
                BinaryOp::BitwiseOr => Some("fixed-width Doria bitwise semantics for `|`"),
                BinaryOp::Concat
                | BinaryOp::Equal
                | BinaryOp::NotEqual
                | BinaryOp::Less
                | BinaryOp::LessEqual
                | BinaryOp::Greater
                | BinaryOp::GreaterEqual
                | BinaryOp::And
                | BinaryOp::Or
                | BinaryOp::Xor
                | BinaryOp::Coalesce => None,
            };
            if let Some(feature) = feature {
                return Err(unsupported_integer_shape(*span, feature));
            }
            Ok(())
        }
        Expr::Range { start, end, .. } => {
            validate_expr(start, semantic_info)?;
            validate_expr(end, semantic_info)
        }
    }
}

fn validate_display_expr(expr: &Expr, semantic_info: &SemanticInfo) -> Result<(), BackendError> {
    if semantic_info.float_type(expr.span()).is_some() {
        return Err(unsupported_numeric_shape(
            expr.span(),
            "canonical Stage 16 float display formatting",
        ));
    }
    validate_expr(expr, semantic_info)
}

fn validate_exprs(expressions: &[Expr], semantic_info: &SemanticInfo) -> Result<(), BackendError> {
    for expression in expressions {
        validate_expr(expression, semantic_info)?;
    }
    Ok(())
}

fn integer_literal_magnitude(expr: &Expr) -> Option<u128> {
    match expr {
        Expr::Int { value, .. } => parse_decimal_magnitude(value),
        Expr::Grouped { expr, .. } => integer_literal_magnitude(expr),
        _ => None,
    }
}

fn unsupported_integer_shape(span: Span, feature: impl Into<String>) -> BackendError {
    unsupported_numeric_shape(span, feature)
}

fn unsupported_numeric_shape(span: Span, feature: impl Into<String>) -> BackendError {
    BackendError::from_diagnostics(vec![Diagnostic::new(
        PHP_INTEGER_UNSUPPORTED_CODE,
        format!(
            "PHP compatibility backend cannot preserve {} exactly; use the `native` or `debug` target for this valid Doria program",
            feature.into()
        ),
        span,
    )])
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
            if assignment.op == AssignOp::DivAssign {
                let target = emit_assignment_target(&assignment.target, scopes);
                writeln(
                    output,
                    indent,
                    &format!(
                        "{target} = fdiv({target}, {});",
                        emit_expr(&assignment.value, scopes)
                    ),
                );
                return;
            }
            let op = match assignment.op {
                AssignOp::Assign => "=",
                AssignOp::AddAssign => "+=",
                AssignOp::SubAssign => "-=",
                AssignOp::MulAssign => "*=",
                AssignOp::DivAssign => "/=",
                AssignOp::ModAssign => "%=",
                AssignOp::ShiftLeftAssign => "<<=",
                AssignOp::ShiftRightAssign => ">>=",
                AssignOp::BitwiseAndAssign => "&=",
                AssignOp::BitwiseOrAssign => "|=",
                AssignOp::BitwiseXorAssign => "^=",
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
                &format!("echo __doria_display({});", emit_expr(expr, scopes)),
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
            if let Expr::FunctionCall { name, args, .. } = expr {
                if name == "panic" && args.len() == 1 {
                    emit_panic(&args[0], output, indent, scopes);
                    return;
                }
            }
            writeln(output, indent, &format!("{};", emit_expr(expr, scopes)));
        }
    }
}

fn emit_panic(message: &Expr, output: &mut String, indent: usize, scopes: &mut PhpNameScopes) {
    let frame_name = scopes.fresh_temp("frame");
    writeln(
        output,
        indent,
        &format!(
            "fwrite(STDERR, \"Panic: \" . {} . \"\\nStack Trace:\\n\");",
            emit_expr(message, scopes)
        ),
    );
    writeln(
        output,
        indent,
        &format!("foreach (debug_backtrace(DEBUG_BACKTRACE_IGNORE_ARGS) as ${frame_name})"),
    );
    writeln(output, indent, "{");
    writeln(
        output,
        indent + 1,
        &format!("if (isset(${frame_name}[\"function\"]))"),
    );
    writeln(output, indent + 1, "{");
    writeln(
        output,
        indent + 2,
        &format!("fwrite(STDERR, \"  at \" . ${frame_name}[\"function\"] . \"\\n\");"),
    );
    writeln(output, indent + 1, "}");
    writeln(output, indent, "}");
    writeln(output, indent, "exit(101);");
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
    if assignment.op == AssignOp::DivAssign {
        let target = emit_assignment_target(&assignment.target, scopes);
        return format!(
            "{target} = fdiv({target}, {})",
            emit_expr(&assignment.value, scopes)
        );
    }
    let op = match assignment.op {
        AssignOp::Assign => "=",
        AssignOp::AddAssign => "+=",
        AssignOp::SubAssign => "-=",
        AssignOp::MulAssign => "*=",
        AssignOp::DivAssign => "/=",
        AssignOp::ModAssign => "%=",
        AssignOp::ShiftLeftAssign => "<<=",
        AssignOp::ShiftRightAssign => ">>=",
        AssignOp::BitwiseAndAssign => "&=",
        AssignOp::BitwiseOrAssign => "|=",
        AssignOp::BitwiseXorAssign => "^=",
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
        Expr::FunctionCall { name, args, .. } => emit_function_call(name, args, scopes),
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
            UnaryOp::Negate if integer_literal_magnitude(expr) == Some((i64::MAX as u128) + 1) => {
                "(-9223372036854775807 - 1)".to_string()
            }
            UnaryOp::Negate => format!("-({})", emit_expr(expr, scopes)),
            UnaryOp::BitwiseNot => {
                unreachable!("unsupported integer unary operator passed PHP capability validation")
            }
        },
        Expr::Binary {
            left, op, right, ..
        } => match op {
            BinaryOp::Div => format!(
                "fdiv({}, {})",
                emit_expr(left, scopes),
                emit_expr(right, scopes)
            ),
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
            BinaryOp::Concat => format!(
                "__doria_display({}) . __doria_display({})",
                emit_expr(left, scopes),
                emit_expr(right, scopes)
            ),
            BinaryOp::Less => format!(
                "__doria_less({}, {})",
                emit_expr(left, scopes),
                emit_expr(right, scopes)
            ),
            BinaryOp::LessEqual => format!(
                "__doria_less_equal({}, {})",
                emit_expr(left, scopes),
                emit_expr(right, scopes)
            ),
            BinaryOp::Greater => format!(
                "__doria_greater({}, {})",
                emit_expr(left, scopes),
                emit_expr(right, scopes)
            ),
            BinaryOp::GreaterEqual => format!(
                "__doria_greater_equal({}, {})",
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
                emitted.push(format!("__doria_display({})", emit_expr(expr, scopes)));
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
        BinaryOp::ShiftLeft => "<<",
        BinaryOp::ShiftRight => ">>",
        BinaryOp::BitwiseAnd => "&",
        BinaryOp::BitwiseXor => "^",
        BinaryOp::BitwiseOr => "|",
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

fn emit_function_call(name: &str, args: &[Expr], scopes: &PhpNameScopes) -> String {
    let helper = match name {
        "readline" => "__doria_readline",
        "read_file" => "__doria_read_file",
        "write_file" => "__doria_write_file",
        "write_stderr" => "__doria_write_stderr",
        "sprintf" => "__doria_sprintf",
        "printf" => "__doria_printf",
        _ => name,
    };
    let mut emitted = args
        .iter()
        .map(|argument| emit_expr(argument, scopes))
        .collect::<Vec<_>>();
    if matches!(name, "sprintf" | "printf") {
        if let Some(Expr::String { value, span }) = args.first() {
            if let Ok(pieces) = format_string::parse(value, *span) {
                emitted[0] = emit_php_string_literal(&php_format_from_plan(&pieces));
                let conversions = pieces.iter().filter_map(|piece| match piece {
                    FormatPiece::Argument { spec, .. } => Some(spec.conversion),
                    FormatPiece::Literal(_) => None,
                });
                for (argument, conversion) in emitted.iter_mut().skip(1).zip(conversions) {
                    if conversion == FormatConversion::Display {
                        *argument = format!("__doria_display({argument})");
                    }
                }
            }
        }
    }
    format!("{helper}({})", emitted.join(", "))
}

fn php_format_from_plan(pieces: &[FormatPiece]) -> String {
    let mut format = String::new();
    for piece in pieces {
        match piece {
            FormatPiece::Literal(value) => format.push_str(&value.replace('%', "%%")),
            FormatPiece::Argument { spec, .. } => {
                format.push('%');
                if spec.left_align {
                    format.push('-');
                }
                if spec.zero_pad {
                    format.push('0');
                }
                if let Some(width) = spec.width {
                    format.push_str(&width.to_string());
                }
                if let Some(precision) = spec.precision {
                    format.push('.');
                    format.push_str(&precision.to_string());
                }
                format.push(match spec.conversion {
                    FormatConversion::Display => 's',
                    FormatConversion::Decimal => 'd',
                    FormatConversion::Float => 'F',
                    FormatConversion::HexLower => 'x',
                    FormatConversion::HexUpper => 'X',
                    FormatConversion::Octal => 'o',
                    FormatConversion::Binary => 'b',
                });
            }
        }
    }
    format
}

fn emit_member_access(access: &MemberAccess) -> &'static str {
    match access {
        MemberAccess::External => "public",
        MemberAccess::Internal => "private",
    }
}

fn php_type(ty: &TypeRef) -> String {
    let name = match ty.name.as_str() {
        "int" | "int64" => "int".to_string(),
        "float" | "float32" | "float64" => "float".to_string(),
        "List" | "Dictionary" | "Set" | "[]" => "array".to_string(),
        name => name.to_string(),
    };
    if ty.nullable {
        format!("?{name}")
    } else {
        name
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

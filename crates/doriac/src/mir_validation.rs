//! Backend-independent structural and type validation for native MIR.

use crate::backend::BackendError;
use crate::mir;
use crate::numeric::IntegerType;

pub fn validate_program(program: &mir::Program) -> Result<(), BackendError> {
    let entry = program
        .functions
        .get(program.entry.0)
        .ok_or_else(|| malformed_mir("entry function does not exist"))?;
    if entry.id != program.entry {
        return Err(malformed_mir(
            "entry FunctionId does not match its table slot",
        ));
    }
    if !entry.params.is_empty() {
        return Err(malformed_mir("entry function declares parameters"));
    }
    if !matches!(
        entry.return_type,
        mir::ReturnType::Void
            | mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Integer(
                IntegerType::Int64
            )))
    ) {
        return Err(malformed_mir(
            "entry function must return void or int/int64",
        ));
    }

    for (index, function) in program.functions.iter().enumerate() {
        if function.id != mir::FunctionId(index) {
            return Err(malformed_mir(format!(
                "function table slot {index} contains function{}",
                function.id.0
            )));
        }
        validate_function(program, function)?;
    }
    Ok(())
}

fn validate_function(program: &mir::Program, function: &mir::Function) -> Result<(), BackendError> {
    for (index, local) in function.locals.iter().enumerate() {
        if local.id != mir::LocalId(index) {
            return Err(malformed_mir(format!(
                "function {} local slot {index} contains local{}",
                function.name, local.id.0
            )));
        }
    }
    for parameter in &function.params {
        let local = local_in(function, *parameter)?;
        let _ = local;
    }
    block_in(function, function.entry_block)?;
    for (index, block) in function.blocks.iter().enumerate() {
        if block.id != mir::BlockId(index) {
            return Err(malformed_mir(format!(
                "function {} block slot {index} contains block{}",
                function.name, block.id.0
            )));
        }
        for statement in &block.statements {
            validate_statement(program, function, statement)?;
        }
        validate_terminator(program, function, &block.terminator)?;
    }
    Ok(())
}

fn validate_statement(
    program: &mir::Program,
    function: &mir::Function,
    statement: &mir::Statement,
) -> Result<(), BackendError> {
    match statement {
        mir::Statement::AssignLocal { target, value } => {
            let local = local_in(function, *target)?;
            match (local.ty, value) {
                (mir::Type::String, mir::Rvalue::String(expression)) => {
                    validate_string_expression(program, function, expression)
                }
                (mir::Type::String, mir::Rvalue::NullableString(_)) => Err(malformed_mir(format!(
                    "string local local{} receives a nullable-string rvalue",
                    target.0
                ))),
                (mir::Type::String, mir::Rvalue::Value(value)) => Err(malformed_mir(format!(
                    "string local local{} receives a {} rvalue",
                    target.0,
                    value.ty()
                ))),
                (mir::Type::NullableString, mir::Rvalue::NullableString(expression)) => {
                    validate_nullable_string_expression(program, function, expression)
                }
                (mir::Type::NullableString, _) => Err(malformed_mir(format!(
                    "nullable-string local local{} receives another rvalue type",
                    target.0
                ))),
                (mir::Type::Scalar(_), mir::Rvalue::String(_) | mir::Rvalue::NullableString(_)) => {
                    Err(malformed_mir(format!(
                        "scalar local local{} receives a string rvalue",
                        target.0
                    )))
                }
                (mir::Type::Scalar(ty), mir::Rvalue::Value(expression)) => {
                    if expression.ty() != ty {
                        return Err(malformed_mir(format!(
                            "{} local local{} receives {} expression",
                            ty,
                            target.0,
                            expression.ty()
                        )));
                    }
                    validate_value_expression(program, function, expression)
                }
            }
        }
        mir::Statement::EchoStringLiteral(_) => Ok(()),
        mir::Statement::EchoString(expression) => {
            validate_string_expression(program, function, expression)
        }
        mir::Statement::CallVoid {
            function: callee,
            args,
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type != mir::ReturnType::Void {
                return Err(malformed_mir(format!(
                    "void call targets integer function {}",
                    callee.name
                )));
            }
            validate_call_args(program, function, callee, args)
        }
        mir::Statement::Printf(format) => validate_format_expression(program, function, format),
        mir::Statement::WriteFile { path, contents } => {
            validate_string_expression(program, function, path)?;
            validate_string_expression(program, function, contents)
        }
        mir::Statement::WriteStderr(value) => validate_string_expression(program, function, value),
    }
}

fn validate_terminator(
    program: &mir::Program,
    function: &mir::Function,
    terminator: &mir::Terminator,
) -> Result<(), BackendError> {
    match terminator {
        mir::Terminator::Return(expression) => {
            let mir::ReturnType::Value(return_type) = function.return_type else {
                return Err(malformed_mir(format!(
                    "void function {} has an integer return",
                    function.name
                )));
            };
            if expression.ty() != return_type {
                return Err(malformed_mir(format!(
                    "function {} returns {} expression from {} function",
                    function.name,
                    expression.ty(),
                    return_type
                )));
            }
            validate_rvalue(program, function, expression)
        }
        mir::Terminator::ReturnVoid => {
            if function.return_type != mir::ReturnType::Void {
                return Err(malformed_mir(format!(
                    "scalar function {} has a void return",
                    function.name
                )));
            }
            Ok(())
        }
        mir::Terminator::Panic(message) => validate_string_expression(program, function, message),
        mir::Terminator::Unreachable => Ok(()),
        mir::Terminator::Jump(target) => block_in(function, *target).map(|_| ()),
        mir::Terminator::Branch {
            condition,
            then_block,
            else_block,
        } => {
            block_in(function, *then_block)?;
            block_in(function, *else_block)?;
            validate_condition(program, function, condition)
        }
    }
}

fn validate_integer_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::IntegerExpression,
) -> Result<(), BackendError> {
    match expression {
        mir::IntegerExpression::Use { ty, operand } => {
            validate_integer_operand(function, *ty, operand)
        }
        mir::IntegerExpression::Unary { ty, op, operand } => {
            if operand.ty() != *ty {
                return Err(malformed_mir(format!(
                    "{ty} unary expression contains {} operand",
                    operand.ty()
                )));
            }
            if matches!(op, mir::IntegerUnaryOp::Negate) && !ty.is_signed() {
                return Err(malformed_mir(format!(
                    "unsigned {ty} expression uses unary negation"
                )));
            }
            validate_integer_expression(program, function, operand)
        }
        mir::IntegerExpression::Binary {
            ty, left, right, ..
        } => {
            if left.ty() != *ty || right.ty() != *ty {
                return Err(malformed_mir(format!(
                    "{ty} binary expression has {} and {} operands",
                    left.ty(),
                    right.ty()
                )));
            }
            validate_integer_expression(program, function, left)?;
            validate_integer_expression(program, function, right)
        }
        mir::IntegerExpression::Convert { value, .. } => {
            validate_integer_expression(program, function, value)
        }
        mir::IntegerExpression::FloatToInt { value } => {
            validate_float_expression(program, function, value)
        }
        mir::IntegerExpression::Call {
            ty,
            function: callee,
            args,
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type
                != mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Integer(*ty)))
            {
                return Err(malformed_mir(format!(
                    "{ty} call targets function {} returning {}",
                    callee.name, callee.return_type
                )));
            }
            validate_call_args(program, function, callee, args)
        }
    }
}

fn validate_value_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::ValueExpression,
) -> Result<(), BackendError> {
    match expression {
        mir::ValueExpression::Integer(value) => {
            validate_integer_expression(program, function, value)
        }
        mir::ValueExpression::Float(value) => validate_float_expression(program, function, value),
        mir::ValueExpression::Bool(value) => validate_condition(program, function, value),
    }
}

fn validate_rvalue(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::Rvalue,
) -> Result<(), BackendError> {
    match expression {
        mir::Rvalue::Value(value) => validate_value_expression(program, function, value),
        mir::Rvalue::String(value) => validate_string_expression(program, function, value),
        mir::Rvalue::NullableString(value) => {
            validate_nullable_string_expression(program, function, value)
        }
    }
}

fn validate_float_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::FloatExpression,
) -> Result<(), BackendError> {
    match expression {
        mir::FloatExpression::Use { ty, operand } => match operand {
            mir::Operand::Scalar(mir::ScalarValue::Float(value)) if value.ty == *ty => Ok(()),
            mir::Operand::Local(local)
                if local_in(function, *local)?.ty
                    == mir::Type::Scalar(mir::ScalarType::Float(*ty)) =>
            {
                Ok(())
            }
            _ => Err(malformed_mir(
                "float expression has an incompatible operand",
            )),
        },
        mir::FloatExpression::Negate { ty, operand } => {
            if operand.ty() != *ty {
                return Err(malformed_mir(format!(
                    "{ty} negate expression contains {} operand",
                    operand.ty()
                )));
            }
            validate_float_expression(program, function, operand)
        }
        mir::FloatExpression::Binary {
            ty, left, right, ..
        } => {
            if left.ty() != *ty || right.ty() != *ty {
                return Err(malformed_mir(format!(
                    "{ty} binary expression has {} and {} operands",
                    left.ty(),
                    right.ty()
                )));
            }
            validate_float_expression(program, function, left)?;
            validate_float_expression(program, function, right)
        }
        mir::FloatExpression::IntToFloat { value } => {
            if value.ty() != IntegerType::Int64 {
                return Err(malformed_mir("Int::toFloat operand is not canonical int"));
            }
            validate_integer_expression(program, function, value)
        }
        mir::FloatExpression::Call {
            ty,
            function: callee,
            args,
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type
                != mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Float(*ty)))
            {
                return Err(malformed_mir(
                    "float call targets a function with another return type",
                ));
            }
            validate_call_args(program, function, callee, args)
        }
    }
}

fn validate_call_args(
    program: &mir::Program,
    caller: &mir::Function,
    callee: &mir::Function,
    args: &[mir::Rvalue],
) -> Result<(), BackendError> {
    if args.len() != callee.params.len() {
        return Err(malformed_mir(format!(
            "call to {} expects {} arguments, got {}",
            callee.name,
            callee.params.len(),
            args.len()
        )));
    }
    for (index, (argument, parameter)) in args.iter().zip(&callee.params).enumerate() {
        let parameter_type = local_in(callee, *parameter)?.ty;
        if argument.ty() != parameter_type {
            return Err(malformed_mir(format!(
                "call to {} passes {} argument {} to {} parameter",
                callee.name,
                argument.ty(),
                index + 1,
                parameter_type
            )));
        }
        validate_rvalue(program, caller, argument)?;
    }
    Ok(())
}

fn validate_condition(
    program: &mir::Program,
    function: &mir::Function,
    condition: &mir::BoolExpression,
) -> Result<(), BackendError> {
    match condition {
        mir::BoolExpression::Use { operand } => match operand {
            mir::Operand::Scalar(mir::ScalarValue::Bool(_)) => Ok(()),
            mir::Operand::Local(local)
                if local_in(function, *local)?.ty == mir::Type::Scalar(mir::ScalarType::Bool) =>
            {
                Ok(())
            }
            _ => Err(malformed_mir("bool expression has an incompatible operand")),
        },
        mir::BoolExpression::Compare { op, left, right } => {
            if left.ty() != right.ty() {
                return Err(malformed_mir(format!(
                    "comparison has {} and {} operands",
                    left.ty(),
                    right.ty()
                )));
            }
            if left.ty() == mir::ScalarType::Bool
                && !matches!(op, mir::CompareOp::Equal | mir::CompareOp::NotEqual)
            {
                return Err(malformed_mir("ordered bool comparison is invalid"));
            }
            validate_value_expression(program, function, left)?;
            validate_value_expression(program, function, right)
        }
        mir::BoolExpression::StringCompare { left, right, .. } => {
            validate_string_expression(program, function, left)?;
            validate_string_expression(program, function, right)
        }
        mir::BoolExpression::NullableStringCompare { op, left, right } => {
            if !matches!(op, mir::CompareOp::Equal | mir::CompareOp::NotEqual) {
                return Err(malformed_mir(
                    "ordered nullable-string comparison is invalid",
                ));
            }
            validate_nullable_string_expression(program, function, left)?;
            validate_nullable_string_expression(program, function, right)
        }
        mir::BoolExpression::Not(condition) => validate_condition(program, function, condition),
        mir::BoolExpression::Binary { left, right, .. } => {
            validate_condition(program, function, left)?;
            validate_condition(program, function, right)
        }
        mir::BoolExpression::Call {
            function: callee,
            args,
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type
                != mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Bool))
            {
                return Err(malformed_mir("bool call targets a non-bool function"));
            }
            validate_call_args(program, function, callee, args)
        }
    }
}

fn validate_integer_operand(
    function: &mir::Function,
    ty: IntegerType,
    operand: &mir::Operand,
) -> Result<(), BackendError> {
    match operand {
        mir::Operand::Scalar(mir::ScalarValue::Integer(value)) if value.ty != ty => Err(
            malformed_mir(format!("{ty} expression contains {} constant", value.ty)),
        ),
        mir::Operand::Scalar(mir::ScalarValue::Integer(_)) => Ok(()),
        mir::Operand::Local(local) => {
            let definition = local_in(function, *local)?;
            if definition.ty != mir::Type::Scalar(mir::ScalarType::Integer(ty)) {
                return Err(malformed_mir(format!(
                    "{ty} expression uses local{} with type {}",
                    local.0, definition.ty
                )));
            }
            Ok(())
        }
        mir::Operand::Scalar(_) => Err(malformed_mir(
            "integer expression contains non-integer constant",
        )),
    }
}

fn validate_string_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::StringExpression,
) -> Result<(), BackendError> {
    match expression {
        mir::StringExpression::Literal(_) => Ok(()),
        mir::StringExpression::Local(local) => {
            let definition = local_in(function, *local)?;
            if definition.ty != mir::Type::String {
                return Err(malformed_mir(format!(
                    "int local local{} is used as a string operand",
                    local.0
                )));
            }
            Ok(())
        }
        mir::StringExpression::NullableLocalAssumeNonNull(local) => {
            if local_in(function, *local)?.ty != mir::Type::NullableString {
                return Err(malformed_mir(
                    "nonnull string expression references another local type",
                ));
            }
            Ok(())
        }
        mir::StringExpression::Concat(parts) => {
            for part in parts {
                validate_string_expression(program, function, part)?;
            }
            Ok(())
        }
        mir::StringExpression::Display(value) => {
            validate_value_expression(program, function, value)
        }
        mir::StringExpression::Call {
            function: callee,
            args,
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type != mir::ReturnType::Value(mir::Type::String) {
                return Err(malformed_mir("string call targets a non-string function"));
            }
            validate_call_args(program, function, callee, args)
        }
        mir::StringExpression::ReadFile(path) => {
            validate_string_expression(program, function, path)
        }
        mir::StringExpression::Format(format) => {
            validate_format_expression(program, function, format)
        }
    }
}

fn validate_nullable_string_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::NullableStringExpression,
) -> Result<(), BackendError> {
    match expression {
        mir::NullableStringExpression::Null | mir::NullableStringExpression::ReadLine => Ok(()),
        mir::NullableStringExpression::String(value) => {
            validate_string_expression(program, function, value)
        }
        mir::NullableStringExpression::Local(local) => {
            if local_in(function, *local)?.ty != mir::Type::NullableString {
                return Err(malformed_mir(
                    "nullable-string expression references another local type",
                ));
            }
            Ok(())
        }
        mir::NullableStringExpression::Call {
            function: callee,
            args,
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type != mir::ReturnType::Value(mir::Type::NullableString) {
                return Err(malformed_mir(
                    "nullable-string call targets another return type",
                ));
            }
            validate_call_args(program, function, callee, args)
        }
    }
}

fn validate_format_expression(
    program: &mir::Program,
    function: &mir::Function,
    format: &mir::FormatExpression,
) -> Result<(), BackendError> {
    use crate::format_string::{FormatConversion, FormatPiece};
    for argument in &format.arguments {
        match argument {
            mir::FormatArgument::Value(value) => {
                validate_value_expression(program, function, value)?
            }
            mir::FormatArgument::String(value) => {
                validate_string_expression(program, function, value)?
            }
        }
    }
    for piece in &format.pieces {
        let FormatPiece::Argument { index, spec } = piece else {
            continue;
        };
        let argument = format
            .arguments
            .get(*index as usize)
            .ok_or_else(|| malformed_mir("format argument index is out of bounds"))?;
        let valid = matches!(
            (spec.conversion, argument),
            (FormatConversion::Display, mir::FormatArgument::Value(_))
                | (FormatConversion::Display, mir::FormatArgument::String(_))
                | (
                    FormatConversion::Decimal
                        | FormatConversion::HexLower
                        | FormatConversion::HexUpper
                        | FormatConversion::Octal
                        | FormatConversion::Binary,
                    mir::FormatArgument::Value(mir::ValueExpression::Integer(_)),
                )
                | (
                    FormatConversion::Float,
                    mir::FormatArgument::Value(mir::ValueExpression::Float(_)),
                )
        );
        if !valid {
            return Err(malformed_mir(
                "format conversion and argument type disagree",
            ));
        }
    }
    Ok(())
}

fn function_in(
    program: &mir::Program,
    id: mir::FunctionId,
) -> Result<&mir::Function, BackendError> {
    program
        .functions
        .get(id.0)
        .filter(|function| function.id == id)
        .ok_or_else(|| malformed_mir(format!("FunctionId function{} does not exist", id.0)))
}

fn local_in(function: &mir::Function, id: mir::LocalId) -> Result<&mir::Local, BackendError> {
    function
        .locals
        .get(id.0)
        .filter(|local| local.id == id)
        .ok_or_else(|| malformed_mir(format!("LocalId local{} does not exist", id.0)))
}

fn block_in(function: &mir::Function, id: mir::BlockId) -> Result<&mir::BasicBlock, BackendError> {
    function
        .blocks
        .get(id.0)
        .filter(|block| block.id == id)
        .ok_or_else(|| malformed_mir(format!("BlockId block{} does not exist", id.0)))
}

fn malformed_mir(message: impl Into<String>) -> BackendError {
    BackendError::new(format!(
        "backend emission failure: malformed MIR: {}",
        message.into()
    ))
}

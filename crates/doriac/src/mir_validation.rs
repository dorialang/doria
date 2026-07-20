//! Backend-independent structural and type validation for native MIR.

use std::collections::{HashMap, HashSet};

use crate::backend::BackendError;
use crate::class_layout::{compute_class_layout, ClassId, FieldType};
use crate::mir;
use crate::numeric::IntegerType;

pub fn validate_program(program: &mir::Program) -> Result<(), BackendError> {
    for (index, class) in program.classes.iter().enumerate() {
        validate_class(program, index, class)?;
    }
    for (index, property) in program.statics.iter().enumerate() {
        if property.id != mir::StaticId(index) {
            return Err(malformed_mir(format!(
                "static table slot {index} contains static{}",
                property.id.0
            )));
        }
        class_in(program, property.class)?;
        let valid = match (&property.initializer, property.ty) {
            (
                mir::StaticValue::Scalar(value),
                mir::Type::Scalar(ty) | mir::Type::NullableScalar(ty),
            ) => value.ty() == ty,
            (mir::StaticValue::String(_), mir::Type::String | mir::Type::NullableString)
            | (
                mir::StaticValue::Null,
                mir::Type::NullableScalar(_)
                | mir::Type::NullableString
                | mir::Type::NullableClass(_),
            ) => true,
            _ => false,
        };
        if !valid {
            return Err(malformed_mir(format!(
                "static{} initializer does not match {}",
                property.id.0, property.ty
            )));
        }
    }

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
    if entry.method.is_some() || entry.receiver_mode.is_some() {
        return Err(malformed_mir("entry function cannot be a method"));
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
        validate_method_identity(program, function)?;
        validate_function(program, function)?;
    }
    Ok(())
}

fn validate_method_identity(
    program: &mir::Program,
    function: &mir::Function,
) -> Result<(), BackendError> {
    match (&function.method, function.receiver_mode) {
        (None, None) => Ok(()),
        (None, Some(_)) => Err(malformed_mir(format!(
            "free function {} declares a receiver mode",
            function.name
        ))),
        (Some(method), None) => {
            class_in(program, method.class)?;
            Ok(())
        }
        (Some(method), Some(mir::ReceiverMode::UnsupportedConsuming)) => {
            Err(malformed_mir(format!(
                "method class#{}::{} uses the unsupported consuming receiver mode",
                method.class.0, method.name
            )))
        }
        (Some(method), Some(_)) => {
            class_in(program, method.class)?;
            let receiver = function.params.first().ok_or_else(|| {
                malformed_mir(format!(
                    "method class#{}::{} has no receiver parameter",
                    method.class.0, method.name
                ))
            })?;
            let receiver = local_in(function, *receiver)?;
            if receiver.ty != mir::Type::Class(method.class) || receiver.owned {
                return Err(malformed_mir(format!(
                    "method class#{}::{} has an invalid receiver parameter",
                    method.class.0, method.name
                )));
            }
            Ok(())
        }
    }
}

fn validate_class(
    program: &mir::Program,
    index: usize,
    class: &mir::Class,
) -> Result<(), BackendError> {
    let expected_id = ClassId(index);
    if class.id != expected_id {
        return Err(malformed_mir(format!(
            "class table slot {index} contains class#{}",
            class.id.0
        )));
    }

    for (property_index, property) in class.properties.iter().enumerate() {
        if property.id.class != class.id || property.id.index != property_index {
            return Err(malformed_mir(format!(
                "class#{} property slot {property_index} contains property#{}:{}",
                class.id.0, property.id.class.0, property.id.index
            )));
        }
        if let mir::Type::Class(referenced) = property.ty {
            class_in(program, referenced)?;
        }
    }

    let pointer_size = std::mem::size_of::<usize>() as u32;
    let expected_layout = compute_class_layout(
        class.id,
        class
            .properties
            .iter()
            .map(|property| (property.id, field_type(property.ty))),
        pointer_size,
    );
    if class.layout != expected_layout {
        return Err(malformed_mir(format!(
            "class#{} layout does not match its property table",
            class.id.0
        )));
    }

    if let Some(constructor) = class.constructor {
        validate_lifecycle(program, class.id, constructor, "constructor", false)?;
    }
    if let Some(destructor) = class.destructor {
        validate_lifecycle(program, class.id, destructor, "destructor", true)?;
    }
    Ok(())
}

fn validate_lifecycle(
    program: &mir::Program,
    class: ClassId,
    function: mir::FunctionId,
    kind: &str,
    receiver_only: bool,
) -> Result<(), BackendError> {
    let function = function_in(program, function)?;
    if function.return_type != mir::ReturnType::Void {
        return Err(malformed_mir(format!(
            "class#{} {kind} {} does not return void",
            class.0, function.name
        )));
    }
    let Some((receiver, parameters)) = function.params.split_first() else {
        return Err(malformed_mir(format!(
            "class#{} {kind} {} has no implicit receiver",
            class.0, function.name
        )));
    };
    let receiver_definition = local_in(function, *receiver)?;
    if receiver_definition.ty != mir::Type::Class(class) {
        return Err(malformed_mir(format!(
            "class#{} {kind} {} has an incompatible implicit receiver",
            class.0, function.name
        )));
    }
    if receiver_definition.owned {
        return Err(malformed_mir(format!(
            "class#{} {kind} {} marks its implicit receiver as owned",
            class.0, function.name
        )));
    }
    if receiver_only && !parameters.is_empty() {
        return Err(malformed_mir(format!(
            "class#{} destructor {} declares parameters",
            class.0, function.name
        )));
    }
    Ok(())
}

fn field_type(ty: mir::Type) -> FieldType {
    match ty {
        mir::Type::Scalar(mir::ScalarType::Integer(integer)) => FieldType::Integer(integer),
        mir::Type::Scalar(mir::ScalarType::Float(float)) => FieldType::Float(float),
        mir::Type::Scalar(mir::ScalarType::Bool) => FieldType::Bool,
        mir::Type::String => FieldType::String,
        mir::Type::NullableScalar(mir::ScalarType::Integer(integer)) => {
            FieldType::NullableInteger(integer)
        }
        mir::Type::NullableScalar(mir::ScalarType::Float(float)) => FieldType::NullableFloat(float),
        mir::Type::NullableScalar(mir::ScalarType::Bool) => FieldType::NullableBool,
        mir::Type::NullableString => FieldType::NullableString,
        mir::Type::Class(class) => FieldType::Class(class),
        mir::Type::NullableClass(class) => FieldType::NullableClass(class),
    }
}

fn validate_function(program: &mir::Program, function: &mir::Function) -> Result<(), BackendError> {
    if let mir::ReturnType::Value(ty) = function.return_type {
        validate_type(program, ty)?;
    }
    for (index, local) in function.locals.iter().enumerate() {
        if local.id != mir::LocalId(index) {
            return Err(malformed_mir(format!(
                "function {} local slot {index} contains local{}",
                function.name, local.id.0
            )));
        }
        validate_type(program, local.ty)?;
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
    }
    let (reachable, _) = reachable_blocks_and_predecessors(function, true)?;
    for block in &function.blocks {
        for statement in &block.statements {
            validate_statement(program, function, statement)?;
        }
        validate_terminator(program, function, &block.terminator, reachable[block.id.0])?;
    }
    validate_class_local_lifetimes(function)
}

fn validate_type(program: &mir::Program, ty: mir::Type) -> Result<(), BackendError> {
    if let mir::Type::Class(class) | mir::Type::NullableClass(class) = ty {
        class_in(program, class)?;
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
                (mir::Type::NullableScalar(expected), mir::Rvalue::NullableScalar(expression))
                    if expression.ty() == expected =>
                {
                    validate_nullable_scalar_expression(program, function, expression)
                }
                (mir::Type::NullableClass(expected), mir::Rvalue::NullableClass(expression))
                    if expression.class() == expected =>
                {
                    validate_nullable_class_expression(program, function, expression)
                }
                (mir::Type::NullableString, mir::Rvalue::Class(_)) => Err(malformed_mir(format!(
                    "nullable-string local local{} receives a class rvalue",
                    target.0
                ))),
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
                (mir::Type::Class(expected), mir::Rvalue::Class(expression))
                    if expression.class() == expected =>
                {
                    if !local.owned {
                        if !local.synthetic {
                            return Err(malformed_mir(format!(
                                "class assignment targets borrowed local local{}",
                                target.0
                            )));
                        }
                        if class_expression_accesses_local(expression, *target) {
                            return Err(malformed_mir(format!(
                                "borrowed class temporary local{} reads its own uninitialized value",
                                target.0
                            )));
                        }
                        validate_class_expression(program, function, expression)?;
                        if infer_expression_return_borrow(program, function, expression)?.is_none()
                        {
                            return Err(malformed_mir(format!(
                                "borrowed class temporary local{} receives an owning value",
                                target.0
                            )));
                        }
                        return Ok(());
                    }
                    validate_class_expression(program, function, expression)?;
                    require_owned_class_expression(
                        expression,
                        &format!("class assignment to local{}", target.0),
                    )
                }
                (mir::Type::Class(expected), _) => Err(malformed_mir(format!(
                    "class#{} local local{} receives a mismatched rvalue",
                    expected.0, target.0
                ))),
                (mir::Type::String | mir::Type::Scalar(_), mir::Rvalue::Class(_)) => {
                    Err(malformed_mir(format!(
                        "non-class local local{} receives a class rvalue",
                        target.0
                    )))
                }
                (mir::Type::NullableScalar(_) | mir::Type::NullableClass(_), _) => {
                    Err(malformed_mir(format!(
                        "nullable local local{} receives a mismatched rvalue",
                        target.0
                    )))
                }
                (
                    mir::Type::String | mir::Type::Scalar(_),
                    mir::Rvalue::NullableScalar(_) | mir::Rvalue::NullableClass(_),
                ) => Err(malformed_mir(format!(
                    "non-nullable local local{} receives a nullable rvalue",
                    target.0
                ))),
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
        mir::Statement::CallBorrowed {
            function: callee,
            args,
        } => {
            let callee = function_in(program, *callee)?;
            if !matches!(
                callee.return_type,
                mir::ReturnType::Value(mir::Type::Class(_) | mir::Type::NullableClass(_))
            ) || infer_function_return_borrow(program, callee)?.is_none()
            {
                return Err(malformed_mir(format!(
                    "borrowed call targets function {} without a borrowed class return",
                    callee.name
                )));
            }
            validate_call_args(program, function, callee, args)
        }
        mir::Statement::CallNullSafe {
            object,
            function: callee,
            args,
        } => validate_null_safe_statement_call(program, function, object, *callee, args),
        mir::Statement::Printf(format) => validate_format_expression(program, function, format),
        mir::Statement::WriteFile { path, contents } => {
            validate_string_expression(program, function, path)?;
            validate_string_expression(program, function, contents)
        }
        mir::Statement::WriteStderr(value) => validate_string_expression(program, function, value),
        mir::Statement::AssignProperty {
            object,
            property,
            value,
        } => {
            let object = local_in(function, *object)?;
            let mir::Type::Class(class) = object.ty else {
                return Err(malformed_mir(
                    "property assignment targets a non-class local",
                ));
            };
            let property_definition = property_in(program, class, *property)?;
            if value.ty() != property_definition.ty {
                return Err(malformed_mir(format!(
                    "property{} receives {} but has type {}",
                    property.index,
                    value.ty(),
                    property_definition.ty
                )));
            }
            if rvalue_transfers_class_local(value, object.id) {
                return Err(malformed_mir(format!(
                    "assignment to property{} consumes its receiver local{}",
                    property.index, object.id.0
                )));
            }
            if rvalue_borrows_class_local_outside_property(value, object.id, *property) {
                return Err(malformed_mir(format!(
                    "assignment to property{} borrows its receiver local{} through another access",
                    property.index, object.id.0
                )));
            }
            let constructor_receiver = class_in(program, class)?.constructor == Some(function.id)
                && function.params.first() == Some(&object.id);
            if !constructor_receiver && !property_definition.writable {
                return Err(malformed_mir(format!(
                    "assignment mutates readonly property{} outside its constructor initializer",
                    property.index
                )));
            }
            if !constructor_receiver && !object.writable {
                return Err(malformed_mir(format!(
                    "assignment to property{} uses readonly receiver local{}",
                    property.index, object.id.0
                )));
            }
            validate_rvalue(program, function, value)?;
            if let (mir::Type::Class(_), mir::Rvalue::Class(expression)) =
                (property_definition.ty, value)
            {
                require_owned_class_expression(
                    expression,
                    &format!("assignment to property{}", property.index),
                )?;
            }
            Ok(())
        }
        mir::Statement::AssignStatic { target, value } => {
            let property = static_in(program, *target)?;
            if !property.writable {
                return Err(malformed_mir(format!(
                    "assignment targets readonly static{}",
                    target.0
                )));
            }
            if value.ty() != property.ty {
                return Err(malformed_mir(format!(
                    "static{} receives {} but has type {}",
                    target.0,
                    value.ty(),
                    property.ty
                )));
            }
            validate_rvalue(program, function, value)
        }
        mir::Statement::DropClass { local, class } => {
            let definition = local_in(function, *local)?;
            if !matches!(
                definition.ty,
                mir::Type::Class(found) | mir::Type::NullableClass(found) if found == *class
            ) {
                return Err(malformed_mir(format!(
                    "drop class#{} references local{} with type {}",
                    class.0, local.0, definition.ty
                )));
            }
            if !definition.owned {
                return Err(malformed_mir(format!(
                    "drop class#{} references borrowed local{}",
                    class.0, local.0
                )));
            }
            class_in(program, *class).map(|_| ())
        }
    }
}

fn validate_terminator(
    program: &mir::Program,
    function: &mir::Function,
    terminator: &mir::Terminator,
    validate_return_ownership: bool,
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
            validate_rvalue(program, function, expression)?;
            if validate_return_ownership {
                if let (mir::Type::Class(_), mir::Rvalue::Class(class)) = (return_type, expression)
                {
                    let expected = infer_function_return_borrow(program, function)?;
                    let actual = infer_expression_return_borrow(program, function, class)?;
                    if !return_borrow_is_compatible(actual, expected) {
                        return Err(malformed_mir(format!(
                            "return from {} has inconsistent class ownership",
                            function.name
                        )));
                    }
                    if expected.is_none() {
                        require_owned_class_expression(
                            class,
                            &format!("return from {}", function.name),
                        )?;
                    }
                } else if let (mir::Type::NullableClass(_), mir::Rvalue::NullableClass(class)) =
                    (return_type, expression)
                {
                    let expected = infer_function_return_borrow(program, function)?;
                    let actual = infer_nullable_expression_return_borrow(program, function, class)?;
                    if !return_borrow_is_compatible(actual, expected) {
                        return Err(malformed_mir(format!(
                            "return from {} has inconsistent nullable class ownership",
                            function.name
                        )));
                    }
                    if expected.is_none() {
                        require_owned_nullable_class_expression(
                            class,
                            &format!("return from {}", function.name),
                        )?;
                    }
                }
            }
            Ok(())
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
            validate_integer_operand(program, function, *ty, operand)
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
        mir::IntegerExpression::Coalesce { ty, left, right } => {
            if left.ty() != mir::ScalarType::Integer(*ty) || right.ty() != *ty {
                return Err(malformed_mir("integer coalesce has incompatible operands"));
            }
            validate_nullable_scalar_expression(program, function, left)?;
            validate_integer_expression(program, function, right)
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
        mir::Rvalue::NullableScalar(value) => {
            validate_nullable_scalar_expression(program, function, value)
        }
        mir::Rvalue::NullableString(value) => {
            validate_nullable_string_expression(program, function, value)
        }
        mir::Rvalue::Class(value) => validate_class_expression(program, function, value),
        mir::Rvalue::NullableClass(value) => {
            validate_nullable_class_expression(program, function, value)
        }
    }?;
    let mut accesses = ClassLocalAccesses::default();
    collect_rvalue_class_local_accesses(expression, &mut accesses);
    validate_ordered_class_accesses(
        program,
        "rvalue",
        &accesses,
        &HashMap::new(),
        &mut HashSet::new(),
    )?;
    Ok(())
}

fn require_owned_class_expression(
    expression: &mir::ClassExpression,
    destination: &str,
) -> Result<(), BackendError> {
    match expression {
        mir::ClassExpression::Local { transfer: true, .. }
        | mir::ClassExpression::Call {
            return_borrow: None,
            ..
        }
        | mir::ClassExpression::New { .. }
        | mir::ClassExpression::NullableLocalAssumeNonNull { transfer: true, .. } => Ok(()),
        mir::ClassExpression::Local {
            local,
            transfer: false,
            ..
        } => Err(malformed_mir(format!(
            "{destination} receives borrowed class local local{}",
            local.0
        ))),
        mir::ClassExpression::Property { property, .. } => Err(malformed_mir(format!(
            "{destination} receives borrowed class property{}",
            property.index
        ))),
        mir::ClassExpression::Call {
            return_borrow: Some(_),
            ..
        } => Err(malformed_mir(format!(
            "{destination} receives a borrowed class call result"
        ))),
        mir::ClassExpression::NullableLocalAssumeNonNull {
            local,
            transfer: false,
            ..
        } => Err(malformed_mir(format!(
            "{destination} receives borrowed nullable class local local{}",
            local.0
        ))),
        mir::ClassExpression::Coalesce { left, right, .. } => {
            require_owned_nullable_class_expression(left, destination)?;
            require_owned_class_expression(right, destination)
        }
    }
}

fn require_owned_nullable_class_expression(
    expression: &mir::NullableClassExpression,
    destination: &str,
) -> Result<(), BackendError> {
    match expression {
        mir::NullableClassExpression::Null(_) => Ok(()),
        mir::NullableClassExpression::Class(value) => {
            require_owned_class_expression(value, destination)
        }
        mir::NullableClassExpression::Call {
            return_borrow: None,
            ..
        }
        | mir::NullableClassExpression::NullSafeCall {
            return_borrow: None,
            ..
        }
        | mir::NullableClassExpression::Local { transfer: true, .. } => Ok(()),
        mir::NullableClassExpression::Local {
            local,
            transfer: false,
            ..
        } => Err(malformed_mir(format!(
            "{destination} receives borrowed nullable class local local{}",
            local.0
        ))),
        mir::NullableClassExpression::Property { property, .. }
        | mir::NullableClassExpression::NullSafeProperty { property, .. } => {
            Err(malformed_mir(format!(
                "{destination} receives borrowed nullable class property{}",
                property.index
            )))
        }
        mir::NullableClassExpression::Call {
            return_borrow: Some(_),
            ..
        }
        | mir::NullableClassExpression::NullSafeCall {
            return_borrow: Some(_),
            ..
        } => Err(malformed_mir(format!(
            "{destination} receives a borrowed nullable class call result"
        ))),
    }
}

fn infer_function_return_borrow(
    program: &mir::Program,
    function: &mir::Function,
) -> Result<Option<mir::ReturnBorrow>, BackendError> {
    let mut inferred: Option<Option<mir::ReturnBorrow>> = None;
    let (reachable, _) = reachable_blocks_and_predecessors(function, true)?;
    for block in function.blocks.iter().filter(|block| reachable[block.id.0]) {
        let candidate = match &block.terminator {
            mir::Terminator::Return(mir::Rvalue::Class(expression)) => Some(
                infer_expression_return_borrow(program, function, expression)?,
            ),
            mir::Terminator::Return(mir::Rvalue::NullableClass(
                mir::NullableClassExpression::Null(_),
            )) => None,
            mir::Terminator::Return(mir::Rvalue::NullableClass(expression)) => Some(
                infer_nullable_expression_return_borrow(program, function, expression)?,
            ),
            _ => continue,
        };
        let Some(candidate) = candidate else {
            continue;
        };
        match (inferred.as_mut(), candidate) {
            (None, candidate) => inferred = Some(candidate),
            (Some(Some(existing)), Some(candidate)) if existing.source == candidate.source => {
                existing.writable &= candidate.writable;
            }
            (Some(None), None) => {}
            _ => {
                return Err(malformed_mir(format!(
                    "function {} mixes owned and borrowed class returns",
                    function.name
                )));
            }
        }
    }
    Ok(inferred.flatten())
}

fn infer_nullable_expression_return_borrow(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::NullableClassExpression,
) -> Result<Option<mir::ReturnBorrow>, BackendError> {
    match expression {
        mir::NullableClassExpression::Null(_) => Ok(None),
        mir::NullableClassExpression::Class(expression) => {
            infer_expression_return_borrow(program, function, expression)
        }
        mir::NullableClassExpression::Local {
            local,
            transfer: false,
            ..
        } => Ok(borrow_from_parameter(function, *local)),
        mir::NullableClassExpression::Property { object, .. } => Ok(borrow_from_parameter(
            function, *object,
        )
        .map(|borrow| mir::ReturnBorrow {
            writable: false,
            ..borrow
        })),
        mir::NullableClassExpression::Call {
            function: callee,
            args,
            return_borrow: Some(return_borrow),
            ..
        } => infer_borrowed_rvalue_source(program, function, *callee, args, *return_borrow),
        mir::NullableClassExpression::NullSafeProperty { object, .. } => Ok(
            infer_nullable_expression_return_borrow(program, function, object)?.map(|borrow| {
                mir::ReturnBorrow {
                    writable: false,
                    ..borrow
                }
            }),
        ),
        mir::NullableClassExpression::NullSafeCall {
            object,
            function: _,
            args,
            return_borrow: Some(return_borrow),
            ..
        } => {
            let source = match return_borrow.source {
                mir::BorrowSource::Receiver => {
                    return infer_nullable_expression_return_borrow(program, function, object).map(
                        |borrow| {
                            borrow.map(|borrow| mir::ReturnBorrow {
                                writable: borrow.writable && return_borrow.writable,
                                ..borrow
                            })
                        },
                    );
                }
                mir::BorrowSource::Parameter(index) => args.get(index),
            };
            let Some(source) = source else {
                return Err(malformed_mir(
                    "null-safe borrowed call has no source argument",
                ));
            };
            infer_rvalue_return_borrow(program, function, source).map(|borrow| {
                borrow.map(|borrow| mir::ReturnBorrow {
                    writable: borrow.writable && return_borrow.writable,
                    ..borrow
                })
            })
        }
        mir::NullableClassExpression::Local { transfer: true, .. }
        | mir::NullableClassExpression::Call {
            return_borrow: None,
            ..
        }
        | mir::NullableClassExpression::NullSafeCall {
            return_borrow: None,
            ..
        } => Ok(None),
    }
}

fn infer_borrowed_rvalue_source(
    program: &mir::Program,
    function: &mir::Function,
    callee: mir::FunctionId,
    args: &[mir::Rvalue],
    return_borrow: mir::ReturnBorrow,
) -> Result<Option<mir::ReturnBorrow>, BackendError> {
    let callee_definition = function_in(program, callee)?;
    let index = match return_borrow.source {
        mir::BorrowSource::Receiver => 0,
        mir::BorrowSource::Parameter(index) => {
            index + usize::from(callee_definition.receiver_mode.is_some())
        }
    };
    let source = args.get(index).ok_or_else(|| {
        malformed_mir(format!(
            "borrowed class call to {} has no source argument",
            callee_definition.name
        ))
    })?;
    infer_rvalue_return_borrow(program, function, source).map(|borrow| {
        borrow.map(|borrow| mir::ReturnBorrow {
            writable: borrow.writable && return_borrow.writable,
            ..borrow
        })
    })
}

fn infer_rvalue_return_borrow(
    program: &mir::Program,
    function: &mir::Function,
    source: &mir::Rvalue,
) -> Result<Option<mir::ReturnBorrow>, BackendError> {
    match source {
        mir::Rvalue::Class(source) => infer_expression_return_borrow(program, function, source),
        mir::Rvalue::NullableClass(source) => {
            infer_nullable_expression_return_borrow(program, function, source)
        }
        _ => Err(malformed_mir(
            "borrowed class call source is not a class value",
        )),
    }
}

fn return_borrow_is_compatible(
    actual: Option<mir::ReturnBorrow>,
    expected: Option<mir::ReturnBorrow>,
) -> bool {
    match (actual, expected) {
        (Some(actual), Some(expected)) => {
            actual.source == expected.source && (!expected.writable || actual.writable)
        }
        (None, None) => true,
        _ => false,
    }
}

fn infer_expression_return_borrow(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::ClassExpression,
) -> Result<Option<mir::ReturnBorrow>, BackendError> {
    match expression {
        mir::ClassExpression::Local {
            local,
            transfer: false,
            ..
        } => match borrow_from_parameter(function, *local) {
            Some(borrow) => Ok(Some(borrow)),
            None => infer_synthetic_local_return_borrow(program, function, *local),
        },
        mir::ClassExpression::NullableLocalAssumeNonNull {
            local,
            transfer: false,
            ..
        } => match borrow_from_parameter(function, *local) {
            Some(borrow) => Ok(Some(borrow)),
            None => infer_synthetic_local_return_borrow(program, function, *local),
        },
        mir::ClassExpression::Property { object, .. } => Ok(borrow_from_parameter(
            function, *object,
        )
        .map(|borrow| mir::ReturnBorrow {
            writable: false,
            ..borrow
        })),
        mir::ClassExpression::Call {
            function: callee,
            args,
            return_borrow: Some(return_borrow),
            ..
        } => {
            let source = borrowed_call_source(program, *callee, args, *return_borrow)?;
            Ok(
                infer_expression_return_borrow(program, function, source)?.map(|borrow| {
                    mir::ReturnBorrow {
                        writable: borrow.writable && return_borrow.writable,
                        ..borrow
                    }
                }),
            )
        }
        mir::ClassExpression::Local { transfer: true, .. }
        | mir::ClassExpression::NullableLocalAssumeNonNull { transfer: true, .. }
        | mir::ClassExpression::Call {
            return_borrow: None,
            ..
        }
        | mir::ClassExpression::New { .. } => Ok(None),
        mir::ClassExpression::Coalesce { .. } => Ok(None),
    }
}

fn infer_synthetic_local_return_borrow(
    program: &mir::Program,
    function: &mir::Function,
    local: mir::LocalId,
) -> Result<Option<mir::ReturnBorrow>, BackendError> {
    let definition = local_in(function, local)?;
    if definition.owned || !definition.synthetic {
        return Ok(None);
    }

    let (reachable, _) = reachable_blocks_and_predecessors(function, true)?;
    let mut inferred = None;
    for block in function.blocks.iter().filter(|block| reachable[block.id.0]) {
        for statement in &block.statements {
            let mir::Statement::AssignLocal {
                target,
                value: mir::Rvalue::Class(expression),
            } = statement
            else {
                continue;
            };
            if *target != local {
                continue;
            }
            if inferred.is_some() || class_expression_accesses_local(expression, local) {
                return Err(malformed_mir(format!(
                    "borrowed class temporary local{} must have one non-recursive assignment",
                    local.0
                )));
            }
            inferred = Some(infer_expression_return_borrow(
                program, function, expression,
            )?);
        }
    }
    match inferred {
        Some(Some(borrow)) => Ok(Some(borrow)),
        Some(None) => Err(malformed_mir(format!(
            "borrowed class temporary local{} receives an owning value",
            local.0
        ))),
        None => Ok(None),
    }
}

fn borrow_from_parameter(
    function: &mir::Function,
    local: mir::LocalId,
) -> Option<mir::ReturnBorrow> {
    let position = function
        .params
        .iter()
        .position(|parameter| *parameter == local)?;
    let definition = function.locals.get(local.0)?;
    if definition.owned {
        return None;
    }
    let has_receiver = function.receiver_mode.is_some();
    let source = if has_receiver && position == 0 {
        mir::BorrowSource::Receiver
    } else {
        mir::BorrowSource::Parameter(position - usize::from(has_receiver))
    };
    Some(mir::ReturnBorrow {
        source,
        writable: definition.writable,
    })
}

fn require_writable_class_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::ClassExpression,
    destination: &str,
) -> Result<(), BackendError> {
    let writable = match expression {
        mir::ClassExpression::Local {
            local,
            transfer: false,
            ..
        } => local_in(function, *local)?.writable,
        mir::ClassExpression::NullableLocalAssumeNonNull {
            local,
            transfer: false,
            ..
        } => local_in(function, *local)?.writable,
        mir::ClassExpression::Property {
            object, property, ..
        } => {
            let object = local_in(function, *object)?;
            let mir::Type::Class(class) = object.ty else {
                return Err(malformed_mir(format!(
                    "{destination} uses a property on non-class local local{}",
                    object.id.0
                )));
            };
            object.writable && property_in(program, class, *property)?.writable
        }
        mir::ClassExpression::Local { transfer: true, .. }
        | mir::ClassExpression::NullableLocalAssumeNonNull { transfer: true, .. } => false,
        mir::ClassExpression::Call { return_borrow, .. } => {
            return_borrow.is_none_or(|borrow| borrow.writable)
        }
        mir::ClassExpression::New { .. } => true,
        mir::ClassExpression::Coalesce { left: _, right, .. } => {
            require_writable_class_expression(program, function, right, destination).is_ok()
        }
    };
    if writable {
        Ok(())
    } else {
        Err(malformed_mir(format!(
            "{destination} requires a writable class value"
        )))
    }
}

#[derive(Clone, Copy)]
enum ClassLocalAccess<'a> {
    Borrow(mir::LocalId),
    PropertyBorrow(mir::LocalId, crate::class_layout::PropertyId),
    Transfer(mir::LocalId),
    BeginCall,
    Call(mir::FunctionId, &'a [mir::Rvalue], usize),
}

#[derive(Default)]
struct ClassLocalAccesses<'a> {
    accesses: Vec<ClassLocalAccess<'a>>,
}

impl<'a> ClassLocalAccesses<'a> {
    fn borrow(&mut self, local: mir::LocalId) {
        self.accesses.push(ClassLocalAccess::Borrow(local));
    }

    fn borrow_property(&mut self, local: mir::LocalId, property: crate::class_layout::PropertyId) {
        self.accesses
            .push(ClassLocalAccess::PropertyBorrow(local, property));
    }

    fn transfer(&mut self, local: mir::LocalId) {
        self.accesses.push(ClassLocalAccess::Transfer(local));
    }

    fn call(&mut self, function: mir::FunctionId, args: &'a [mir::Rvalue]) {
        self.accesses
            .push(ClassLocalAccess::Call(function, args, 0));
    }

    fn constructor_call(&mut self, function: mir::FunctionId, args: &'a [mir::Rvalue]) {
        self.accesses
            .push(ClassLocalAccess::Call(function, args, 1));
    }

    fn begin_call(&mut self) {
        self.accesses.push(ClassLocalAccess::BeginCall);
    }

    fn iter(&self) -> impl Iterator<Item = ClassLocalAccess<'a>> + '_ {
        self.accesses.iter().copied()
    }

    fn borrowed(&self) -> impl Iterator<Item = mir::LocalId> + '_ {
        self.iter().filter_map(|access| match access {
            ClassLocalAccess::Borrow(local) | ClassLocalAccess::PropertyBorrow(local, _) => {
                Some(local)
            }
            ClassLocalAccess::Transfer(_)
            | ClassLocalAccess::BeginCall
            | ClassLocalAccess::Call(_, _, _) => None,
        })
    }

    fn transferred(&self) -> impl Iterator<Item = mir::LocalId> + '_ {
        self.iter().filter_map(|access| match access {
            ClassLocalAccess::Transfer(local) => Some(local),
            ClassLocalAccess::Borrow(_)
            | ClassLocalAccess::PropertyBorrow(_, _)
            | ClassLocalAccess::BeginCall
            | ClassLocalAccess::Call(_, _, _) => None,
        })
    }

    fn property_borrowed(
        &self,
    ) -> impl Iterator<Item = (mir::LocalId, crate::class_layout::PropertyId)> + '_ {
        self.iter().filter_map(|access| match access {
            ClassLocalAccess::PropertyBorrow(local, property) => Some((local, property)),
            ClassLocalAccess::Borrow(_)
            | ClassLocalAccess::Transfer(_)
            | ClassLocalAccess::BeginCall
            | ClassLocalAccess::Call(_, _, _) => None,
        })
    }
}

#[derive(Clone, Copy)]
struct PropertyAliasInvalidation {
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
    alias: mir::LocalId,
}

fn rvalue_transfers_class_local(value: &mir::Rvalue, local: mir::LocalId) -> bool {
    let mut accesses = ClassLocalAccesses::default();
    collect_rvalue_class_local_accesses(value, &mut accesses);
    let transfers_local = accesses
        .transferred()
        .any(|transferred| transferred == local);
    transfers_local
}

fn rvalue_borrows_class_local_outside_property(
    value: &mir::Rvalue,
    local: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    let mut accesses = ClassLocalAccesses::default();
    collect_rvalue_class_local_accesses(value, &mut accesses);
    let receiver_borrows = accesses
        .borrowed()
        .filter(|borrowed| *borrowed == local)
        .count();
    let exact_target_borrows = accesses
        .property_borrowed()
        .filter(|borrowed| *borrowed == (local, property))
        .count();
    receiver_borrows != exact_target_borrows
}

fn collect_rvalue_class_local_accesses<'a>(
    value: &'a mir::Rvalue,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::Rvalue::Value(value) => collect_value_class_local_accesses(value, accesses),
        mir::Rvalue::String(value) => collect_string_class_local_accesses(value, accesses),
        mir::Rvalue::NullableScalar(value) => {
            collect_nullable_scalar_class_local_accesses(value, accesses)
        }
        mir::Rvalue::NullableString(value) => {
            collect_nullable_string_class_local_accesses(value, accesses)
        }
        mir::Rvalue::Class(value) => collect_class_expression_local_accesses(value, accesses),
        mir::Rvalue::NullableClass(value) => collect_nullable_class_local_accesses(value, accesses),
    }
}

fn collect_rvalue_args_class_local_accesses<'a>(
    args: &'a [mir::Rvalue],
    accesses: &mut ClassLocalAccesses<'a>,
) {
    for value in args {
        collect_rvalue_class_local_accesses(value, accesses);
    }
}

fn collect_value_class_local_accesses<'a>(
    value: &'a mir::ValueExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::ValueExpression::Integer(value) => {
            collect_integer_class_local_accesses(value, accesses)
        }
        mir::ValueExpression::Float(value) => collect_float_class_local_accesses(value, accesses),
        mir::ValueExpression::Bool(value) => collect_bool_class_local_accesses(value, accesses),
    }
}

fn collect_operand_class_local_accesses<'a>(
    operand: &'a mir::Operand,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    if let mir::Operand::Property { object, property } = operand {
        accesses.borrow_property(*object, *property);
    }
}

fn collect_integer_class_local_accesses<'a>(
    value: &'a mir::IntegerExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::IntegerExpression::Use { operand, .. } => {
            collect_operand_class_local_accesses(operand, accesses);
        }
        mir::IntegerExpression::Unary { operand, .. }
        | mir::IntegerExpression::Convert { value: operand, .. } => {
            collect_integer_class_local_accesses(operand, accesses);
        }
        mir::IntegerExpression::Binary { left, right, .. } => {
            collect_integer_class_local_accesses(left, accesses);
            collect_integer_class_local_accesses(right, accesses);
        }
        mir::IntegerExpression::FloatToInt { value } => {
            collect_float_class_local_accesses(value, accesses);
        }
        mir::IntegerExpression::Call { function, args, .. } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::IntegerExpression::Coalesce { left, right, .. } => {
            collect_nullable_scalar_class_local_accesses(left, accesses);
            collect_integer_class_local_accesses(right, accesses);
        }
    }
}

fn collect_float_class_local_accesses<'a>(
    value: &'a mir::FloatExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::FloatExpression::Use { operand, .. } => {
            collect_operand_class_local_accesses(operand, accesses);
        }
        mir::FloatExpression::Negate { operand, .. } => {
            collect_float_class_local_accesses(operand, accesses);
        }
        mir::FloatExpression::Binary { left, right, .. } => {
            collect_float_class_local_accesses(left, accesses);
            collect_float_class_local_accesses(right, accesses);
        }
        mir::FloatExpression::IntToFloat { value } => {
            collect_integer_class_local_accesses(value, accesses);
        }
        mir::FloatExpression::Call { function, args, .. } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::FloatExpression::Coalesce { left, right, .. } => {
            collect_nullable_scalar_class_local_accesses(left, accesses);
            collect_float_class_local_accesses(right, accesses);
        }
    }
}

fn collect_string_class_local_accesses<'a>(
    value: &'a mir::StringExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::StringExpression::Concat(parts) => {
            for part in parts {
                collect_string_class_local_accesses(part, accesses);
            }
        }
        mir::StringExpression::Display(value) => {
            collect_value_class_local_accesses(value, accesses);
        }
        mir::StringExpression::Call { function, args } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::StringExpression::ReadFile(path) => {
            collect_string_class_local_accesses(path, accesses);
        }
        mir::StringExpression::Format(format) => {
            collect_format_class_local_accesses(format, accesses);
        }
        mir::StringExpression::Coalesce { left, right } => {
            collect_nullable_string_class_local_accesses(left, accesses);
            collect_string_class_local_accesses(right, accesses);
        }
        mir::StringExpression::Literal(_)
        | mir::StringExpression::Local(_)
        | mir::StringExpression::Static(_)
        | mir::StringExpression::NullableLocalAssumeNonNull(_) => {}
        mir::StringExpression::Property { object, property } => {
            accesses.borrow_property(*object, *property)
        }
    }
}

fn collect_nullable_string_class_local_accesses<'a>(
    value: &'a mir::NullableStringExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::NullableStringExpression::String(value) => {
            collect_string_class_local_accesses(value, accesses);
        }
        mir::NullableStringExpression::Call { function, args } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::NullableStringExpression::Null
        | mir::NullableStringExpression::Local(_)
        | mir::NullableStringExpression::Static(_)
        | mir::NullableStringExpression::ReadLine => {}
        mir::NullableStringExpression::Property { object, property } => {
            accesses.borrow_property(*object, *property);
        }
        mir::NullableStringExpression::NullSafeProperty { object, .. } => {
            collect_nullable_class_local_accesses(object, accesses);
        }
        mir::NullableStringExpression::NullSafeCall {
            object,
            function,
            args,
        } => {
            collect_nullable_class_local_accesses(object, accesses);
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
    }
}

fn collect_class_expression_local_accesses<'a>(
    value: &'a mir::ClassExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::ClassExpression::Local {
            local,
            transfer: true,
            ..
        } => accesses.transfer(*local),
        mir::ClassExpression::Local {
            local,
            transfer: false,
            ..
        } => accesses.borrow(*local),
        mir::ClassExpression::NullableLocalAssumeNonNull {
            local,
            transfer: true,
            ..
        } => accesses.transfer(*local),
        mir::ClassExpression::NullableLocalAssumeNonNull {
            local,
            transfer: false,
            ..
        } => accesses.borrow(*local),
        mir::ClassExpression::Property {
            object, property, ..
        } => accesses.borrow_property(*object, *property),
        mir::ClassExpression::Call { function, args, .. } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::ClassExpression::New {
            properties,
            constructor,
            args,
            ..
        } => {
            for property in properties {
                if let mir::PropertyValueSource::Expression(value) = &property.source {
                    collect_rvalue_class_local_accesses(value, accesses);
                }
            }
            if constructor.is_some() {
                accesses.begin_call();
            }
            collect_rvalue_args_class_local_accesses(args, accesses);
            if let Some(function) = constructor {
                accesses.constructor_call(*function, args);
            }
        }
        mir::ClassExpression::Coalesce { left, right, .. } => {
            collect_nullable_class_local_accesses(left, accesses);
            collect_class_expression_local_accesses(right, accesses);
        }
    }
}

fn class_expression_accesses_local(expression: &mir::ClassExpression, local: mir::LocalId) -> bool {
    let mut accesses = ClassLocalAccesses::default();
    collect_class_expression_local_accesses(expression, &mut accesses);
    let accesses_local = accesses.iter().any(|access| match access {
        ClassLocalAccess::Borrow(accessed)
        | ClassLocalAccess::Transfer(accessed)
        | ClassLocalAccess::PropertyBorrow(accessed, _) => accessed == local,
        ClassLocalAccess::BeginCall | ClassLocalAccess::Call(_, _, _) => false,
    });
    accesses_local
}

fn collect_bool_class_local_accesses<'a>(
    value: &'a mir::BoolExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::BoolExpression::Use { operand } => {
            collect_operand_class_local_accesses(operand, accesses);
        }
        mir::BoolExpression::Compare { left, right, .. } => {
            collect_value_class_local_accesses(left, accesses);
            collect_value_class_local_accesses(right, accesses);
        }
        mir::BoolExpression::StringCompare { left, right, .. } => {
            collect_string_class_local_accesses(left, accesses);
            collect_string_class_local_accesses(right, accesses);
        }
        mir::BoolExpression::NullableStringCompare { left, right, .. } => {
            collect_nullable_string_class_local_accesses(left, accesses);
            collect_nullable_string_class_local_accesses(right, accesses);
        }
        mir::BoolExpression::NullableScalarIsPresent(value) => {
            collect_nullable_scalar_class_local_accesses(value, accesses);
        }
        mir::BoolExpression::NullableClassIsPresent(value) => {
            collect_nullable_class_local_accesses(value, accesses);
        }
        mir::BoolExpression::Not(value) => {
            collect_bool_class_local_accesses(value, accesses);
        }
        mir::BoolExpression::Binary { op, left, right } => {
            collect_bool_class_local_accesses(left, accesses);
            if !matches!(
                (op, constant_bool_expression(left)),
                (mir::BoolBinaryOp::And, Some(false)) | (mir::BoolBinaryOp::Or, Some(true))
            ) {
                collect_bool_class_local_accesses(right, accesses);
            }
        }
        mir::BoolExpression::Call { function, args } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::BoolExpression::Coalesce { left, right } => {
            collect_nullable_scalar_class_local_accesses(left, accesses);
            collect_bool_class_local_accesses(right, accesses);
        }
    }
}

fn collect_nullable_scalar_class_local_accesses<'a>(
    value: &'a mir::NullableScalarExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::NullableScalarExpression::Value(value) => {
            collect_value_class_local_accesses(value, accesses)
        }
        mir::NullableScalarExpression::Property {
            object, property, ..
        } => accesses.borrow_property(*object, *property),
        mir::NullableScalarExpression::Call { function, args, .. } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::NullableScalarExpression::NullSafeProperty { object, .. } => {
            collect_nullable_class_local_accesses(object, accesses)
        }
        mir::NullableScalarExpression::NullSafeCall {
            object,
            function,
            args,
            ..
        } => {
            collect_nullable_class_local_accesses(object, accesses);
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::NullableScalarExpression::Null(_)
        | mir::NullableScalarExpression::Local { .. }
        | mir::NullableScalarExpression::Static { .. } => {}
    }
}

fn collect_nullable_class_local_accesses<'a>(
    value: &'a mir::NullableClassExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::NullableClassExpression::Class(value) => {
            collect_class_expression_local_accesses(value, accesses)
        }
        mir::NullableClassExpression::Local {
            local,
            transfer: true,
            ..
        } => accesses.transfer(*local),
        mir::NullableClassExpression::Local {
            local,
            transfer: false,
            ..
        } => accesses.borrow(*local),
        mir::NullableClassExpression::Property {
            object, property, ..
        } => accesses.borrow_property(*object, *property),
        mir::NullableClassExpression::Call { function, args, .. } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::NullableClassExpression::NullSafeProperty { object, .. } => {
            collect_nullable_class_local_accesses(object, accesses)
        }
        mir::NullableClassExpression::NullSafeCall {
            object,
            function,
            args,
            ..
        } => {
            collect_nullable_class_local_accesses(object, accesses);
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::NullableClassExpression::Null(_) => {}
    }
}

fn collect_format_class_local_accesses<'a>(
    format: &'a mir::FormatExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    for argument in format.pieces.iter().filter_map(|piece| match piece {
        crate::format_string::FormatPiece::Argument { index, .. } => {
            format.arguments.get(*index as usize)
        }
        crate::format_string::FormatPiece::Literal(_) => None,
    }) {
        match argument {
            mir::FormatArgument::Value(value) => {
                collect_value_class_local_accesses(value, accesses)
            }
            mir::FormatArgument::String(value) | mir::FormatArgument::ClassDisplay(value) => {
                collect_string_class_local_accesses(value, accesses)
            }
        }
    }
}

fn collect_statement_class_local_accesses(statement: &mir::Statement) -> ClassLocalAccesses<'_> {
    let mut accesses = ClassLocalAccesses::default();
    match statement {
        mir::Statement::AssignLocal { value, .. } | mir::Statement::AssignStatic { value, .. } => {
            collect_rvalue_class_local_accesses(value, &mut accesses);
        }
        mir::Statement::EchoString(value) | mir::Statement::WriteStderr(value) => {
            collect_string_class_local_accesses(value, &mut accesses);
        }
        mir::Statement::CallVoid { args, .. } | mir::Statement::CallBorrowed { args, .. } => {
            collect_rvalue_args_class_local_accesses(args, &mut accesses);
        }
        mir::Statement::CallNullSafe { object, args, .. } => {
            collect_nullable_class_local_accesses(object, &mut accesses);
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, &mut accesses);
        }
        mir::Statement::Printf(format) => {
            collect_format_class_local_accesses(format, &mut accesses);
        }
        mir::Statement::WriteFile { path, contents } => {
            collect_string_class_local_accesses(path, &mut accesses);
            collect_string_class_local_accesses(contents, &mut accesses);
        }
        mir::Statement::AssignProperty { object, value, .. } => {
            collect_rvalue_class_local_accesses(value, &mut accesses);
            accesses.borrow(*object);
        }
        mir::Statement::EchoStringLiteral(_) | mir::Statement::DropClass { .. } => {}
    }
    accesses
}

fn collect_terminator_class_local_accesses(terminator: &mir::Terminator) -> ClassLocalAccesses<'_> {
    let mut accesses = ClassLocalAccesses::default();
    match terminator {
        mir::Terminator::Return(value) => {
            collect_rvalue_class_local_accesses(value, &mut accesses);
        }
        mir::Terminator::Panic(value) => {
            collect_string_class_local_accesses(value, &mut accesses);
        }
        mir::Terminator::Branch { condition, .. } => {
            collect_bool_class_local_accesses(condition, &mut accesses);
        }
        mir::Terminator::ReturnVoid | mir::Terminator::Unreachable | mir::Terminator::Jump(_) => {}
    }
    accesses
}

fn reachable_blocks_and_predecessors(
    function: &mir::Function,
    fold_constant_branches: bool,
) -> Result<(Vec<bool>, Vec<Vec<mir::BlockId>>), BackendError> {
    let mut reachable = vec![false; function.blocks.len()];
    let mut pending = vec![function.entry_block];
    while let Some(block_id) = pending.pop() {
        let block = block_in(function, block_id)?;
        if std::mem::replace(&mut reachable[block_id.0], true) {
            continue;
        }
        pending.extend(analysis_terminator_targets(
            &block.terminator,
            fold_constant_branches,
        ));
    }

    let mut predecessors = vec![Vec::new(); function.blocks.len()];
    for block in function.blocks.iter().filter(|block| reachable[block.id.0]) {
        for target in analysis_terminator_targets(&block.terminator, fold_constant_branches) {
            block_in(function, target)?;
            predecessors[target.0].push(block.id);
        }
    }
    Ok((reachable, predecessors))
}

fn apply_class_local_state(
    function: &mir::Function,
    statement: &mir::Statement,
    moved: &mut HashSet<mir::LocalId>,
    alias_invalidations: &[PropertyAliasInvalidation],
    validate: bool,
) -> Result<(), BackendError> {
    let accesses = collect_statement_class_local_accesses(statement);
    if validate {
        if let mir::Statement::AssignLocal { target, .. } = statement {
            if accesses
                .transferred()
                .any(|transferred| transferred == *target)
            {
                return Err(malformed_mir(format!(
                    "function {} assigns class local local{} from an overlapping transfer",
                    function.name, target.0
                )));
            }
        }
    }
    apply_class_local_accesses(function, &accesses, moved, validate)?;
    if let mir::Statement::AssignProperty {
        object, property, ..
    } = statement
    {
        for invalidation in alias_invalidations {
            if invalidation.receiver == *object && invalidation.property == *property {
                moved.insert(invalidation.alias);
            }
        }
    }
    match statement {
        mir::Statement::AssignLocal { target, .. }
            if matches!(
                local_in(function, *target)?.ty,
                mir::Type::Class(_) | mir::Type::NullableClass(_)
            ) =>
        {
            moved.remove(target);
        }
        mir::Statement::DropClass { local, .. } => {
            moved.insert(*local);
        }
        _ => {}
    }
    Ok(())
}

fn apply_class_local_accesses(
    function: &mir::Function,
    accesses: &ClassLocalAccesses,
    moved: &mut HashSet<mir::LocalId>,
    validate: bool,
) -> Result<(), BackendError> {
    for access in accesses.iter() {
        let (local, action) = match access {
            ClassLocalAccess::Borrow(local) | ClassLocalAccess::PropertyBorrow(local, _) => {
                (local, "uses")
            }
            ClassLocalAccess::Transfer(local) => (local, "transfers"),
            ClassLocalAccess::BeginCall | ClassLocalAccess::Call(_, _, _) => continue,
        };
        if validate && moved.contains(&local) {
            return Err(malformed_mir(format!(
                "function {} {action} class local local{} after its ownership ended",
                function.name, local.0
            )));
        }
        if matches!(access, ClassLocalAccess::Transfer(_)) {
            moved.insert(local);
        }
    }
    Ok(())
}

fn validate_class_local_lifetimes(function: &mir::Function) -> Result<(), BackendError> {
    validate_class_local_lifetimes_with_aliases(function, &[])
}

fn validate_class_local_lifetimes_with_aliases(
    function: &mir::Function,
    alias_invalidations: &[PropertyAliasInvalidation],
) -> Result<(), BackendError> {
    let (reachable, predecessors) = reachable_blocks_and_predecessors(function, true)?;
    let mut moved_on_entry = vec![HashSet::new(); function.blocks.len()];
    let mut moved_on_exit = vec![HashSet::new(); function.blocks.len()];

    loop {
        let mut changed = false;
        for block in function.blocks.iter().filter(|block| reachable[block.id.0]) {
            let moved_at_entry = predecessors[block.id.0]
                .iter()
                .flat_map(|predecessor| moved_on_exit[predecessor.0].iter().copied())
                .collect::<HashSet<_>>();
            let mut moved_at_exit = moved_at_entry.clone();
            for statement in &block.statements {
                apply_class_local_state(
                    function,
                    statement,
                    &mut moved_at_exit,
                    alias_invalidations,
                    false,
                )?;
            }
            apply_class_local_accesses(
                function,
                &collect_terminator_class_local_accesses(&block.terminator),
                &mut moved_at_exit,
                false,
            )?;
            if moved_on_entry[block.id.0] != moved_at_entry
                || moved_on_exit[block.id.0] != moved_at_exit
            {
                moved_on_entry[block.id.0] = moved_at_entry;
                moved_on_exit[block.id.0] = moved_at_exit;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    for block in function.blocks.iter().filter(|block| reachable[block.id.0]) {
        let mut moved = moved_on_entry[block.id.0].clone();
        for statement in &block.statements {
            apply_class_local_state(function, statement, &mut moved, alias_invalidations, true)?;
        }
        let accesses = collect_terminator_class_local_accesses(&block.terminator);
        apply_class_local_accesses(function, &accesses, &mut moved, true)?;
    }
    Ok(())
}

fn validate_class_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::ClassExpression,
) -> Result<(), BackendError> {
    let class = expression.class();
    let Some(class_definition) = program
        .classes
        .get(class.0)
        .filter(|definition| definition.id == class)
    else {
        return Err(malformed_mir(format!("unknown class#{}", class.0)));
    };
    match expression {
        mir::ClassExpression::Local {
            local, transfer, ..
        } => {
            let definition = local_in(function, *local)?;
            if definition.ty != mir::Type::Class(class) {
                return Err(malformed_mir(format!(
                    "class rvalue uses non-class local local{}",
                    local.0
                )));
            }
            if *transfer && !definition.owned {
                return Err(malformed_mir(format!(
                    "class rvalue transfers borrowed local local{}",
                    local.0
                )));
            }
            Ok(())
        }
        mir::ClassExpression::NullableLocalAssumeNonNull {
            local, transfer, ..
        } => {
            let definition = local_in(function, *local)?;
            if definition.ty != mir::Type::NullableClass(class) {
                return Err(malformed_mir(
                    "nonnull class expression references another local type",
                ));
            }
            if *transfer && !definition.owned {
                return Err(malformed_mir(
                    "nonnull class expression transfers a borrowed local",
                ));
            }
            Ok(())
        }
        mir::ClassExpression::Property {
            object, property, ..
        } => validate_property_operand(
            program,
            function,
            *object,
            *property,
            mir::Type::Class(class),
        ),
        mir::ClassExpression::Call {
            function: callee,
            args,
            return_borrow,
            ..
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type != mir::ReturnType::Value(mir::Type::Class(class)) {
                return Err(malformed_mir(format!(
                    "class#{} call targets a function with another return type",
                    class.0
                )));
            }
            let expected_return_borrow = infer_function_return_borrow(program, callee)?;
            if *return_borrow != expected_return_borrow {
                return Err(malformed_mir(format!(
                    "class#{} call disagrees with function {} return ownership",
                    class.0, callee.name
                )));
            }
            validate_call_args(program, function, callee, args)
        }
        mir::ClassExpression::New {
            properties,
            args,
            constructor,
            ..
        } => {
            if class_definition.constructor != *constructor {
                return Err(malformed_mir(format!(
                    "class#{} new expression names the wrong constructor",
                    class.0
                )));
            }
            let constructor = constructor
                .map(|constructor| function_in(program, constructor))
                .transpose()?;
            let constructor_parameters = if let Some(constructor) = constructor {
                if constructor.return_type != mir::ReturnType::Void {
                    return Err(malformed_mir(format!(
                        "constructor {} does not return void",
                        constructor.name
                    )));
                }
                let Some((receiver, parameters)) = constructor.params.split_first() else {
                    return Err(malformed_mir(format!(
                        "constructor {} has no implicit receiver",
                        constructor.name
                    )));
                };
                if local_in(constructor, *receiver)?.ty != mir::Type::Class(class) {
                    return Err(malformed_mir(format!(
                        "constructor {} has an incompatible implicit receiver",
                        constructor.name
                    )));
                }
                parameters
            } else {
                if !args.is_empty() {
                    return Err(malformed_mir(format!(
                        "class#{} without a constructor receives arguments",
                        class.0
                    )));
                }
                &[]
            };

            let mut initialized = HashSet::new();
            let mut consumed_class_arguments = HashSet::new();
            let mut construction_accesses = ClassLocalAccesses::default();
            for (position, property) in properties.iter().enumerate() {
                if property.property.index != position {
                    return Err(malformed_mir(format!(
                        "class#{} new expression initializes property{} out of construction order",
                        class.0, property.property.index
                    )));
                }
                let Some(definition) = class_definition
                    .properties
                    .get(property.property.index)
                    .filter(|definition| definition.id == property.property)
                else {
                    return Err(malformed_mir(format!(
                        "class#{} new expression initializes an unknown property slot",
                        class.0
                    )));
                };
                if !initialized.insert(property.property) {
                    return Err(malformed_mir(format!(
                        "class#{} new expression initializes property{} more than once",
                        class.0, property.property.index
                    )));
                }
                let source_type = match &property.source {
                    mir::PropertyValueSource::Expression(value) => {
                        validate_rvalue(program, function, value)?;
                        if let (mir::Type::Class(_), mir::Rvalue::Class(expression)) =
                            (definition.ty, value)
                        {
                            require_owned_class_expression(
                                expression,
                                &format!(
                                    "class#{} property{} initializer",
                                    class.0, property.property.index
                                ),
                            )?;
                        }
                        collect_rvalue_class_local_accesses(value, &mut construction_accesses);
                        value.ty()
                    }
                    mir::PropertyValueSource::ConstructorArgument(index) => {
                        let argument = args.get(*index).ok_or_else(|| {
                            malformed_mir(format!(
                                "class#{} property{} references constructor argument {} but only {} exist",
                                class.0,
                                property.property.index,
                                index,
                                args.len()
                            ))
                        })?;
                        if matches!(argument.ty(), mir::Type::Class(_))
                            && !consumed_class_arguments.insert(*index)
                        {
                            return Err(malformed_mir(format!(
                                "class#{} new expression gives constructor argument {} to more than one property",
                                class.0, index
                            )));
                        }
                        argument.ty()
                    }
                    mir::PropertyValueSource::ConstructorBody => {
                        let Some(constructor) = constructor else {
                            return Err(malformed_mir(format!(
                                "class#{} property{} relies on a missing constructor body",
                                class.0, property.property.index
                            )));
                        };
                        let receiver = *constructor.params.first().ok_or_else(|| {
                            malformed_mir(format!(
                                "constructor {} has no implicit receiver",
                                constructor.name
                            ))
                        })?;
                        validate_constructor_body_initializer(
                            constructor,
                            receiver,
                            property.property,
                            definition.writable,
                        )?;
                        definition.ty
                    }
                };
                if !definition.writable
                    && !matches!(property.source, mir::PropertyValueSource::ConstructorBody)
                {
                    if let Some(constructor) = constructor {
                        if constructor_property_assignment_count(
                            constructor,
                            constructor.params[0],
                            property.property,
                        )? > 0
                        {
                            return Err(malformed_mir(format!(
                                "class#{} readonly property{} is initialized before its constructor assigns it",
                                class.0, property.property.index
                            )));
                        }
                    }
                }
                if source_type != definition.ty {
                    return Err(malformed_mir(format!(
                        "class#{} property{} has type {} but its initializer has type {}",
                        class.0, property.property.index, definition.ty, source_type
                    )));
                }
            }
            if initialized.len() != class_definition.properties.len() {
                let missing = class_definition
                    .properties
                    .iter()
                    .find(|property| !initialized.contains(&property.id))
                    .expect("property count differs");
                return Err(malformed_mir(format!(
                    "class#{} new expression does not initialize property{}",
                    class.0, missing.id.index
                )));
            }
            if constructor.is_some() {
                construction_accesses.begin_call();
            }
            collect_rvalue_args_class_local_accesses(args, &mut construction_accesses);
            if let Some(constructor) = constructor {
                construction_accesses.constructor_call(constructor.id, args);
            }
            validate_ordered_class_accesses(
                program,
                &format!("class#{} new expression", class.0),
                &construction_accesses,
                &HashMap::new(),
                &mut HashSet::new(),
            )?;
            if let Some(constructor) = constructor {
                validate_call_args_for_params(
                    program,
                    function,
                    constructor,
                    constructor_parameters,
                    args,
                    Some(
                        &properties
                            .iter()
                            .filter_map(|property| match property.source {
                                mir::PropertyValueSource::ConstructorArgument(index)
                                    if matches!(
                                        class_definition.properties[property.property.index].ty,
                                        mir::Type::Class(_)
                                    ) =>
                                {
                                    Some(index)
                                }
                                _ => None,
                            })
                            .collect(),
                    ),
                )?;
                for index in &consumed_class_arguments {
                    let parameter = constructor_parameters.get(*index).ok_or_else(|| {
                        malformed_mir(format!(
                            "constructor {} has no parameter {}",
                            constructor.name, index
                        ))
                    })?;
                    if local_in(constructor, *parameter)?.owned {
                        return Err(malformed_mir(format!(
                            "class#{} new expression gives constructor argument {} to a property and an owning constructor parameter",
                            class.0, index
                        )));
                    }
                }
                let alias_invalidations = properties
                    .iter()
                    .filter_map(|property| {
                        let mir::PropertyValueSource::ConstructorArgument(index) = property.source
                        else {
                            return None;
                        };
                        let definition = &class_definition.properties[property.property.index];
                        if !matches!(definition.ty, mir::Type::Class(_)) {
                            return None;
                        }
                        Some(PropertyAliasInvalidation {
                            receiver: constructor.params[0],
                            property: property.property,
                            alias: constructor_parameters[index],
                        })
                    })
                    .collect::<Vec<_>>();
                validate_class_local_lifetimes_with_aliases(constructor, &alias_invalidations)?;
            }
            Ok(())
        }
        mir::ClassExpression::Coalesce { left, right, .. } => {
            if left.class() != class || right.class() != class {
                return Err(malformed_mir("class coalesce has incompatible operands"));
            }
            validate_nullable_class_expression(program, function, left)?;
            validate_class_expression(program, function, right)
        }
    }
}

fn validate_constructor_body_initializer(
    constructor: &mir::Function,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
    writable: bool,
) -> Result<(), BackendError> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum State {
        Uninitialized,
        Initialized,
        MaybeInitialized,
    }
    impl State {
        fn join(self, incoming: Self) -> Self {
            if self == incoming {
                self
            } else {
                Self::MaybeInitialized
            }
        }
    }

    let (reachable, _) = reachable_blocks_and_predecessors(constructor, true)?;
    let mut inputs = vec![None; constructor.blocks.len()];
    let mut outputs = vec![None; constructor.blocks.len()];
    inputs[constructor.entry_block.0] = Some(State::Uninitialized);
    let mut pending = std::collections::VecDeque::from([constructor.entry_block]);
    let mut queued = vec![false; constructor.blocks.len()];
    queued[constructor.entry_block.0] = true;
    while let Some(block_id) = pending.pop_front() {
        queued[block_id.0] = false;
        let block = block_in(constructor, block_id)?;
        let mut state = inputs[block_id.0].expect("queued constructor block has input state");
        for statement in &block.statements {
            if matches!(
                statement,
                mir::Statement::AssignProperty {
                    object,
                    property: assigned,
                    ..
                } if *object == receiver && *assigned == property
            ) {
                state = if writable {
                    State::Initialized
                } else {
                    match state {
                        State::Uninitialized => State::Initialized,
                        State::Initialized | State::MaybeInitialized => {
                            return Err(malformed_mir(format!(
                                "constructor {} initializes readonly property{} more than once on one path",
                                constructor.name, property.index
                            )));
                        }
                    }
                };
            }
        }
        outputs[block_id.0] = Some(state);
        for successor in analysis_terminator_targets(&block.terminator, true) {
            if !reachable[successor.0] {
                continue;
            }
            let changed = match inputs[successor.0] {
                Some(current) => {
                    let joined = current.join(state);
                    if joined == current {
                        false
                    } else {
                        inputs[successor.0] = Some(joined);
                        true
                    }
                }
                None => {
                    inputs[successor.0] = Some(state);
                    true
                }
            };
            if changed && !queued[successor.0] {
                queued[successor.0] = true;
                pending.push_back(successor);
            }
        }
    }

    for block in constructor
        .blocks
        .iter()
        .filter(|block| inputs[block.id.0].is_some())
    {
        let mut state = inputs[block.id.0].expect("reachable constructor block state");
        for statement in &block.statements {
            if state != State::Initialized
                && statement_observes_property(statement, receiver, property)
            {
                return Err(malformed_mir(format!(
                    "constructor {} reads or exposes property{} before it is initialized",
                    constructor.name, property.index
                )));
            }
            if matches!(
                statement,
                mir::Statement::AssignProperty {
                    object,
                    property: assigned,
                    ..
                } if *object == receiver && *assigned == property
            ) {
                state = State::Initialized;
            }
        }
        if state != State::Initialized
            && terminator_observes_property(&block.terminator, receiver, property)
        {
            return Err(malformed_mir(format!(
                "constructor {} reads or exposes property{} before it is initialized",
                constructor.name, property.index
            )));
        }
        if state != State::Initialized
            && matches!(
                block.terminator,
                mir::Terminator::Return(_) | mir::Terminator::ReturnVoid
            )
        {
            return Err(malformed_mir(format!(
                "constructor {} can return without initializing property{}",
                constructor.name, property.index
            )));
        }
    }
    Ok(())
}

fn constructor_property_assignment_count(
    constructor: &mir::Function,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> Result<usize, BackendError> {
    let (reachable, _) = reachable_blocks_and_predecessors(constructor, true)?;
    Ok(constructor
        .blocks
        .iter()
        .filter(|block| reachable[block.id.0])
        .flat_map(|block| block.statements.iter())
        .filter(|statement| {
            matches!(
                statement,
                mir::Statement::AssignProperty {
                    object,
                    property: assigned,
                    ..
                } if *object == receiver && *assigned == property
            )
        })
        .count())
}

fn terminator_targets(terminator: &mir::Terminator) -> Vec<mir::BlockId> {
    match terminator {
        mir::Terminator::Jump(target) => vec![*target],
        mir::Terminator::Branch {
            then_block,
            else_block,
            ..
        } => vec![*then_block, *else_block],
        mir::Terminator::Return(_)
        | mir::Terminator::ReturnVoid
        | mir::Terminator::Panic(_)
        | mir::Terminator::Unreachable => Vec::new(),
    }
}

fn analysis_terminator_targets(
    terminator: &mir::Terminator,
    fold_constant_branches: bool,
) -> Vec<mir::BlockId> {
    if !fold_constant_branches {
        return terminator_targets(terminator);
    }
    match terminator {
        mir::Terminator::Branch {
            condition,
            then_block,
            else_block,
        } => match constant_bool_expression(condition) {
            Some(true) => vec![*then_block],
            Some(false) => vec![*else_block],
            None => vec![*then_block, *else_block],
        },
        _ => terminator_targets(terminator),
    }
}

fn constant_bool_expression(expression: &mir::BoolExpression) -> Option<bool> {
    match expression {
        mir::BoolExpression::Use {
            operand: mir::Operand::Scalar(mir::ScalarValue::Bool(value)),
        } => Some(*value),
        mir::BoolExpression::Not(value) => constant_bool_expression(value).map(|value| !value),
        mir::BoolExpression::Binary { op, left, right } => match op {
            mir::BoolBinaryOp::And => match constant_bool_expression(left) {
                Some(false) => Some(false),
                Some(true) => constant_bool_expression(right),
                None if constant_bool_expression(right) == Some(false) => Some(false),
                None => None,
            },
            mir::BoolBinaryOp::Or => match constant_bool_expression(left) {
                Some(true) => Some(true),
                Some(false) => constant_bool_expression(right),
                None if constant_bool_expression(right) == Some(true) => Some(true),
                None => None,
            },
            mir::BoolBinaryOp::Xor => {
                Some(constant_bool_expression(left)? ^ constant_bool_expression(right)?)
            }
        },
        _ => None,
    }
}

fn statement_observes_property(
    statement: &mir::Statement,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match statement {
        mir::Statement::AssignLocal { value, .. } => {
            rvalue_observes_property(value, receiver, property)
        }
        mir::Statement::EchoStringLiteral(_) | mir::Statement::DropClass { .. } => false,
        mir::Statement::AssignStatic { value, .. } => {
            rvalue_observes_property(value, receiver, property)
        }
        mir::Statement::EchoString(value) | mir::Statement::WriteStderr(value) => {
            string_observes_property(value, receiver, property)
        }
        mir::Statement::CallVoid { args, .. } | mir::Statement::CallBorrowed { args, .. } => args
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::Statement::CallNullSafe { object, args, .. } => {
            nullable_class_observes_property(object, receiver, property)
                || args
                    .iter()
                    .any(|value| rvalue_observes_property(value, receiver, property))
        }
        mir::Statement::Printf(format) => format_observes_property(format, receiver, property),
        mir::Statement::WriteFile { path, contents } => {
            string_observes_property(path, receiver, property)
                || string_observes_property(contents, receiver, property)
        }
        mir::Statement::AssignProperty { value, .. } => {
            rvalue_observes_property(value, receiver, property)
        }
    }
}

fn terminator_observes_property(
    terminator: &mir::Terminator,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match terminator {
        mir::Terminator::Return(value) => rvalue_observes_property(value, receiver, property),
        mir::Terminator::Panic(value) => string_observes_property(value, receiver, property),
        mir::Terminator::Branch { condition, .. } => {
            bool_observes_property(condition, receiver, property)
        }
        mir::Terminator::ReturnVoid | mir::Terminator::Unreachable | mir::Terminator::Jump(_) => {
            false
        }
    }
}

fn rvalue_observes_property(
    value: &mir::Rvalue,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::Rvalue::Value(value) => value_observes_property(value, receiver, property),
        mir::Rvalue::String(value) => string_observes_property(value, receiver, property),
        mir::Rvalue::NullableScalar(value) => {
            nullable_scalar_observes_property(value, receiver, property)
        }
        mir::Rvalue::NullableString(value) => {
            nullable_string_observes_property(value, receiver, property)
        }
        mir::Rvalue::Class(value) => class_observes_property(value, receiver, property),
        mir::Rvalue::NullableClass(value) => {
            nullable_class_observes_property(value, receiver, property)
        }
    }
}

fn value_observes_property(
    value: &mir::ValueExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::ValueExpression::Integer(value) => {
            integer_observes_property(value, receiver, property)
        }
        mir::ValueExpression::Float(value) => float_observes_property(value, receiver, property),
        mir::ValueExpression::Bool(value) => bool_observes_property(value, receiver, property),
    }
}

fn operand_observes_property(
    operand: &mir::Operand,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    matches!(
        operand,
        mir::Operand::Property {
            object,
            property: observed,
        } if *object == receiver && *observed == property
    )
}

fn integer_observes_property(
    value: &mir::IntegerExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::IntegerExpression::Use { operand, .. } => {
            operand_observes_property(operand, receiver, property)
        }
        mir::IntegerExpression::Unary { operand, .. }
        | mir::IntegerExpression::Convert { value: operand, .. } => {
            integer_observes_property(operand, receiver, property)
        }
        mir::IntegerExpression::Binary { left, right, .. } => {
            integer_observes_property(left, receiver, property)
                || integer_observes_property(right, receiver, property)
        }
        mir::IntegerExpression::FloatToInt { value } => {
            float_observes_property(value, receiver, property)
        }
        mir::IntegerExpression::Call { args, .. } => args
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::IntegerExpression::Coalesce { left, right, .. } => {
            nullable_scalar_observes_property(left, receiver, property)
                || integer_observes_property(right, receiver, property)
        }
    }
}

fn float_observes_property(
    value: &mir::FloatExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::FloatExpression::Use { operand, .. } => {
            operand_observes_property(operand, receiver, property)
        }
        mir::FloatExpression::Negate { operand, .. } => {
            float_observes_property(operand, receiver, property)
        }
        mir::FloatExpression::Binary { left, right, .. } => {
            float_observes_property(left, receiver, property)
                || float_observes_property(right, receiver, property)
        }
        mir::FloatExpression::IntToFloat { value } => {
            integer_observes_property(value, receiver, property)
        }
        mir::FloatExpression::Call { args, .. } => args
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::FloatExpression::Coalesce { left, right, .. } => {
            nullable_scalar_observes_property(left, receiver, property)
                || float_observes_property(right, receiver, property)
        }
    }
}

fn string_observes_property(
    value: &mir::StringExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::StringExpression::Property {
            object,
            property: observed,
        } => *object == receiver && *observed == property,
        mir::StringExpression::Concat(parts) => parts
            .iter()
            .any(|part| string_observes_property(part, receiver, property)),
        mir::StringExpression::Display(value) => value_observes_property(value, receiver, property),
        mir::StringExpression::Call { args, .. } => args
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::StringExpression::ReadFile(path) => string_observes_property(path, receiver, property),
        mir::StringExpression::Format(format) => {
            format_observes_property(format, receiver, property)
        }
        mir::StringExpression::Coalesce { left, right } => {
            nullable_string_observes_property(left, receiver, property)
                || string_observes_property(right, receiver, property)
        }
        mir::StringExpression::Literal(_)
        | mir::StringExpression::Local(_)
        | mir::StringExpression::Static(_)
        | mir::StringExpression::NullableLocalAssumeNonNull(_) => false,
    }
}

fn nullable_string_observes_property(
    value: &mir::NullableStringExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::NullableStringExpression::String(value) => {
            string_observes_property(value, receiver, property)
        }
        mir::NullableStringExpression::Property {
            object,
            property: observed,
        } => *object == receiver && *observed == property,
        mir::NullableStringExpression::Call { args, .. } => args
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::NullableStringExpression::NullSafeProperty { object, .. } => {
            nullable_class_observes_property(object, receiver, property)
        }
        mir::NullableStringExpression::NullSafeCall { object, args, .. } => {
            nullable_class_observes_property(object, receiver, property)
                || args
                    .iter()
                    .any(|value| rvalue_observes_property(value, receiver, property))
        }
        mir::NullableStringExpression::Null
        | mir::NullableStringExpression::Local(_)
        | mir::NullableStringExpression::Static(_)
        | mir::NullableStringExpression::ReadLine => false,
    }
}

fn class_observes_property(
    value: &mir::ClassExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::ClassExpression::Local { local, .. }
        | mir::ClassExpression::NullableLocalAssumeNonNull { local, .. } => *local == receiver,
        mir::ClassExpression::Property {
            object,
            property: observed,
            ..
        } => *object == receiver && *observed == property,
        mir::ClassExpression::Call { args, .. } => args
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::ClassExpression::New {
            properties, args, ..
        } => {
            properties.iter().any(|value| {
                matches!(
                    &value.source,
                    mir::PropertyValueSource::Expression(value)
                        if rvalue_observes_property(value, receiver, property)
                )
            }) || args
                .iter()
                .any(|value| rvalue_observes_property(value, receiver, property))
        }
        mir::ClassExpression::Coalesce { left, right, .. } => {
            nullable_class_observes_property(left, receiver, property)
                || class_observes_property(right, receiver, property)
        }
    }
}

fn bool_observes_property(
    value: &mir::BoolExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::BoolExpression::Use { operand } => {
            operand_observes_property(operand, receiver, property)
        }
        mir::BoolExpression::Compare { left, right, .. } => {
            value_observes_property(left, receiver, property)
                || value_observes_property(right, receiver, property)
        }
        mir::BoolExpression::StringCompare { left, right, .. } => {
            string_observes_property(left, receiver, property)
                || string_observes_property(right, receiver, property)
        }
        mir::BoolExpression::NullableStringCompare { left, right, .. } => {
            nullable_string_observes_property(left, receiver, property)
                || nullable_string_observes_property(right, receiver, property)
        }
        mir::BoolExpression::NullableScalarIsPresent(value) => {
            nullable_scalar_observes_property(value, receiver, property)
        }
        mir::BoolExpression::NullableClassIsPresent(value) => {
            nullable_class_observes_property(value, receiver, property)
        }
        mir::BoolExpression::Not(value) => bool_observes_property(value, receiver, property),
        mir::BoolExpression::Binary { left, right, .. } => {
            bool_observes_property(left, receiver, property)
                || bool_observes_property(right, receiver, property)
        }
        mir::BoolExpression::Call { args, .. } => args
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::BoolExpression::Coalesce { left, right } => {
            nullable_scalar_observes_property(left, receiver, property)
                || bool_observes_property(right, receiver, property)
        }
    }
}

fn nullable_scalar_observes_property(
    value: &mir::NullableScalarExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::NullableScalarExpression::Value(value) => {
            value_observes_property(value, receiver, property)
        }
        mir::NullableScalarExpression::Property {
            object,
            property: observed,
            ..
        } => *object == receiver && *observed == property,
        mir::NullableScalarExpression::Call { args, .. } => args
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::NullableScalarExpression::NullSafeProperty { object, .. } => {
            nullable_class_observes_property(object, receiver, property)
        }
        mir::NullableScalarExpression::NullSafeCall { object, args, .. } => {
            nullable_class_observes_property(object, receiver, property)
                || args
                    .iter()
                    .any(|value| rvalue_observes_property(value, receiver, property))
        }
        mir::NullableScalarExpression::Null(_)
        | mir::NullableScalarExpression::Local { .. }
        | mir::NullableScalarExpression::Static { .. } => false,
    }
}

fn nullable_class_observes_property(
    value: &mir::NullableClassExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::NullableClassExpression::Class(value) => {
            class_observes_property(value, receiver, property)
        }
        mir::NullableClassExpression::Local { local, .. } => *local == receiver,
        mir::NullableClassExpression::Property {
            object,
            property: observed,
            ..
        } => *object == receiver && *observed == property,
        mir::NullableClassExpression::Call { args, .. } => args
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::NullableClassExpression::NullSafeProperty { object, .. } => {
            nullable_class_observes_property(object, receiver, property)
        }
        mir::NullableClassExpression::NullSafeCall { object, args, .. } => {
            nullable_class_observes_property(object, receiver, property)
                || args
                    .iter()
                    .any(|value| rvalue_observes_property(value, receiver, property))
        }
        mir::NullableClassExpression::Null(_) => false,
    }
}

fn format_observes_property(
    format: &mir::FormatExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    format.arguments.iter().any(|argument| match argument {
        mir::FormatArgument::Value(value) => value_observes_property(value, receiver, property),
        mir::FormatArgument::String(value) | mir::FormatArgument::ClassDisplay(value) => {
            string_observes_property(value, receiver, property)
        }
    })
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
            mir::Operand::NullablePayload(local)
                if local_in(function, *local)?.ty
                    == mir::Type::NullableScalar(mir::ScalarType::Float(*ty)) =>
            {
                Ok(())
            }
            mir::Operand::Property { object, property } => validate_property_operand(
                program,
                function,
                *object,
                *property,
                mir::Type::Scalar(mir::ScalarType::Float(*ty)),
            ),
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
        mir::FloatExpression::Coalesce { ty, left, right } => {
            if left.ty() != mir::ScalarType::Float(*ty) || right.ty() != *ty {
                return Err(malformed_mir("float coalesce has incompatible operands"));
            }
            validate_nullable_scalar_expression(program, function, left)?;
            validate_float_expression(program, function, right)
        }
    }
}

fn validate_call_args(
    program: &mir::Program,
    caller: &mir::Function,
    callee: &mir::Function,
    args: &[mir::Rvalue],
) -> Result<(), BackendError> {
    if program
        .classes
        .iter()
        .any(|class| class.constructor == Some(callee.id) || class.destructor == Some(callee.id))
    {
        return Err(malformed_mir(format!(
            "ordinary call targets lifecycle function {}",
            callee.name
        )));
    }
    if let (Some(method), Some(receiver_mode)) = (&callee.method, callee.receiver_mode) {
        let Some(mir::Rvalue::Class(receiver)) = args.first() else {
            return Err(malformed_mir(format!(
                "call to method class#{}::{} has no explicit borrowed receiver",
                method.class.0, method.name
            )));
        };
        if matches!(receiver, mir::ClassExpression::Local { transfer: true, .. }) {
            return Err(malformed_mir(format!(
                "call to method class#{}::{} transfers its receiver",
                method.class.0, method.name
            )));
        }
        if receiver.class() != method.class {
            return Err(malformed_mir(format!(
                "call to method class#{}::{} uses class#{} as receiver",
                method.class.0,
                method.name,
                receiver.class().0
            )));
        }
        if receiver_mode == mir::ReceiverMode::Writable {
            require_writable_class_expression(
                program,
                caller,
                receiver,
                &format!("call to method class#{}::{}", method.class.0, method.name),
            )?;
        }
    }
    validate_call_args_for_params(program, caller, callee, &callee.params, args, None)
}

fn validate_call_args_for_params(
    program: &mir::Program,
    caller: &mir::Function,
    callee: &mir::Function,
    params: &[mir::LocalId],
    args: &[mir::Rvalue],
    promoted_transfers: Option<&HashSet<usize>>,
) -> Result<(), BackendError> {
    if args.len() != params.len() {
        return Err(malformed_mir(format!(
            "call to {} expects {} arguments, got {}",
            callee.name,
            params.len(),
            args.len()
        )));
    }
    let mut borrowed_class_locals: HashMap<mir::LocalId, ClassBorrowMode> = HashMap::new();
    let mut transferred_class_locals = HashSet::new();
    let operation = format!("call to {}", callee.name);
    for (index, (argument, parameter)) in args.iter().zip(params).enumerate() {
        let parameter_definition = local_in(callee, *parameter)?;
        let parameter_type = parameter_definition.ty;
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
        let mut accesses = ClassLocalAccesses::default();
        collect_rvalue_class_local_accesses(argument, &mut accesses);
        let mut argument_borrows = validate_ordered_class_accesses(
            program,
            &operation,
            &accesses,
            &borrowed_class_locals,
            &mut transferred_class_locals,
        )?;
        if matches!(parameter_type, mir::Type::Class(_)) {
            let promoted_transfer =
                promoted_transfers.is_some_and(|indices| indices.contains(&index));
            let mir::Rvalue::Class(expression) = argument else {
                unreachable!("class parameter type was checked against its argument")
            };
            if parameter_definition.owned
                && matches!(
                    expression,
                    mir::ClassExpression::Local {
                        transfer: false,
                        ..
                    } | mir::ClassExpression::Property { .. }
                )
            {
                return Err(malformed_mir(format!(
                    "call to {} borrows argument {} for an owned parameter",
                    callee.name,
                    index + 1
                )));
            } else if parameter_definition.owned || promoted_transfer {
                require_owned_class_expression(
                    expression,
                    &format!("call to {} argument {}", callee.name, index + 1),
                )?;
            } else if matches!(
                expression,
                mir::ClassExpression::Local { transfer: true, .. }
            ) {
                return Err(malformed_mir(format!(
                    "call to {} transfers argument {} into a borrowed parameter",
                    callee.name,
                    index + 1
                )));
            }
            if parameter_definition.writable {
                require_writable_class_expression(
                    program,
                    caller,
                    expression,
                    &format!("call to {} argument {}", callee.name, index + 1),
                )?;
            }
            if !parameter_definition.owned && !promoted_transfer {
                if let Some(local) = escaping_class_local_borrow(program, argument)? {
                    let mode = if parameter_definition.writable {
                        ClassBorrowMode::Writable
                    } else {
                        ClassBorrowMode::Readonly
                    };
                    argument_borrows.insert(local, mode);
                }
            }
        }
        for (local, mode) in argument_borrows {
            if transferred_class_locals.contains(&local) {
                return Err(class_access_error(
                    &operation,
                    "both borrows and transfers",
                    local,
                ));
            }
            if borrowed_class_locals
                .get(&local)
                .is_some_and(|previous| previous.conflicts_with(mode))
            {
                return Err(class_access_error(
                    &operation,
                    "takes overlapping writable borrows of",
                    local,
                ));
            }
            borrowed_class_locals.insert(local, mode);
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum ClassBorrowMode {
    Readonly,
    Writable,
}

impl ClassBorrowMode {
    fn conflicts_with(self, other: Self) -> bool {
        matches!(self, Self::Writable) || matches!(other, Self::Writable)
    }
}

fn validate_ordered_class_accesses(
    program: &mir::Program,
    operation: &str,
    accesses: &ClassLocalAccesses<'_>,
    active_borrows: &HashMap<mir::LocalId, ClassBorrowMode>,
    transfers: &mut HashSet<mir::LocalId>,
) -> Result<HashMap<mir::LocalId, ClassBorrowMode>, BackendError> {
    let mut property_borrows = HashMap::new();
    let mut call_entry_borrows = Vec::new();
    for access in accesses.iter() {
        match access {
            ClassLocalAccess::Borrow(local) => {
                if transfers.contains(&local) {
                    return Err(class_access_error(
                        operation,
                        "both borrows and transfers",
                        local,
                    ));
                }
            }
            ClassLocalAccess::PropertyBorrow(local, _) => {
                if transfers.contains(&local) {
                    return Err(class_access_error(
                        operation,
                        "both borrows and transfers",
                        local,
                    ));
                }
                property_borrows.insert(local, ClassBorrowMode::Readonly);
            }
            ClassLocalAccess::Transfer(local) => {
                if active_borrows.contains_key(&local) || property_borrows.contains_key(&local) {
                    return Err(class_access_error(
                        operation,
                        "both borrows and transfers",
                        local,
                    ));
                }
                if !transfers.insert(local) {
                    return Err(duplicate_class_transfer_error(operation, local));
                }
            }
            ClassLocalAccess::BeginCall => {
                call_entry_borrows.push(property_borrows.clone());
            }
            ClassLocalAccess::Call(function, args, parameter_offset) => {
                let entry_borrows = call_entry_borrows
                    .pop()
                    .ok_or_else(|| malformed_mir("class access call marker is unbalanced"))?;
                for (local, mode) in
                    borrowed_class_call_locals(program, function, args, parameter_offset)?
                {
                    if transfers.contains(&local) {
                        return Err(class_access_error(
                            operation,
                            "both borrows and transfers",
                            local,
                        ));
                    }
                    let conflicts = active_borrows
                        .get(&local)
                        .or_else(|| entry_borrows.get(&local))
                        .is_some_and(|previous| previous.conflicts_with(mode));
                    if conflicts {
                        return Err(class_access_error(
                            operation,
                            "takes overlapping writable borrows of",
                            local,
                        ));
                    }
                }
            }
        }
    }
    if !call_entry_borrows.is_empty() {
        return Err(malformed_mir("class access call marker is unbalanced"));
    }
    Ok(property_borrows)
}

fn class_access_error(operation: &str, action: &str, local: mir::LocalId) -> BackendError {
    malformed_mir(format!("{operation} {action} class local local{}", local.0))
}

fn duplicate_class_transfer_error(operation: &str, local: mir::LocalId) -> BackendError {
    malformed_mir(format!(
        "{operation} transfers class local local{} more than once",
        local.0
    ))
}

fn escaping_class_local_borrow(
    program: &mir::Program,
    argument: &mir::Rvalue,
) -> Result<Option<mir::LocalId>, BackendError> {
    let mir::Rvalue::Class(expression) = argument else {
        return Ok(None);
    };
    escaping_class_expression_local_borrow(program, expression)
}

fn escaping_class_expression_local_borrow(
    program: &mir::Program,
    expression: &mir::ClassExpression,
) -> Result<Option<mir::LocalId>, BackendError> {
    match expression {
        mir::ClassExpression::Local {
            local,
            transfer: false,
            ..
        }
        | mir::ClassExpression::NullableLocalAssumeNonNull {
            local,
            transfer: false,
            ..
        }
        | mir::ClassExpression::Property { object: local, .. } => Ok(Some(*local)),
        mir::ClassExpression::Call {
            function,
            args,
            return_borrow: Some(return_borrow),
            ..
        } => escaping_class_expression_local_borrow(
            program,
            borrowed_call_source(program, *function, args, *return_borrow)?,
        ),
        mir::ClassExpression::Local { transfer: true, .. }
        | mir::ClassExpression::NullableLocalAssumeNonNull { transfer: true, .. }
        | mir::ClassExpression::Call {
            return_borrow: None,
            ..
        }
        | mir::ClassExpression::New { .. }
        | mir::ClassExpression::Coalesce { .. } => Ok(None),
    }
}

fn borrowed_call_source<'a>(
    program: &mir::Program,
    function: mir::FunctionId,
    args: &'a [mir::Rvalue],
    return_borrow: mir::ReturnBorrow,
) -> Result<&'a mir::ClassExpression, BackendError> {
    let callee = function_in(program, function)?;
    let index = match return_borrow.source {
        mir::BorrowSource::Receiver => 0,
        mir::BorrowSource::Parameter(index) => index + usize::from(callee.receiver_mode.is_some()),
    };
    let Some(mir::Rvalue::Class(source)) = args.get(index) else {
        return Err(malformed_mir(format!(
            "borrowed class call to {} has no class source argument",
            callee.name
        )));
    };
    Ok(source)
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
            mir::Operand::NullablePayload(local)
                if local_in(function, *local)?.ty
                    == mir::Type::NullableScalar(mir::ScalarType::Bool) =>
            {
                Ok(())
            }
            mir::Operand::Property { object, property } => validate_property_operand(
                program,
                function,
                *object,
                *property,
                mir::Type::Scalar(mir::ScalarType::Bool),
            ),
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
        mir::BoolExpression::NullableScalarIsPresent(value) => {
            validate_nullable_scalar_expression(program, function, value)
        }
        mir::BoolExpression::NullableClassIsPresent(value) => {
            validate_nullable_class_expression(program, function, value)
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
        mir::BoolExpression::Coalesce { left, right } => {
            if left.ty() != mir::ScalarType::Bool {
                return Err(malformed_mir("bool coalesce has a non-bool left operand"));
            }
            validate_nullable_scalar_expression(program, function, left)?;
            validate_condition(program, function, right)
        }
    }
}

fn validate_integer_operand(
    program: &mir::Program,
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
        mir::Operand::NullablePayload(local) => {
            let definition = local_in(function, *local)?;
            if definition.ty != mir::Type::NullableScalar(mir::ScalarType::Integer(ty)) {
                return Err(malformed_mir(format!(
                    "{ty} expression uses nullable payload local{} with type {}",
                    local.0, definition.ty
                )));
            }
            Ok(())
        }
        mir::Operand::Static(id) => validate_static_operand(
            program,
            *id,
            mir::Type::Scalar(mir::ScalarType::Integer(ty)),
        ),
        mir::Operand::Property { object, property } => validate_property_operand(
            program,
            function,
            *object,
            *property,
            mir::Type::Scalar(mir::ScalarType::Integer(ty)),
        ),
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
        mir::StringExpression::Static(id) => {
            validate_static_operand(program, *id, mir::Type::String)
        }
        mir::StringExpression::Property { object, property } => {
            validate_property_operand(program, function, *object, *property, mir::Type::String)
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
        mir::StringExpression::Coalesce { left, right } => {
            validate_nullable_string_expression(program, function, left)?;
            validate_string_expression(program, function, right)
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
        mir::NullableStringExpression::Static(id) => {
            validate_static_operand(program, *id, mir::Type::NullableString)
        }
        mir::NullableStringExpression::Property { object, property } => validate_property_operand(
            program,
            function,
            *object,
            *property,
            mir::Type::NullableString,
        ),
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
        mir::NullableStringExpression::NullSafeProperty { object, property } => {
            let class = object.class();
            validate_nullable_class_expression(program, function, object)?;
            property_in(program, class, *property).and_then(|definition| {
                (matches!(definition.ty, mir::Type::String | mir::Type::NullableString))
                    .then_some(())
                    .ok_or_else(|| malformed_mir("null-safe property has another type"))
            })
        }
        mir::NullableStringExpression::NullSafeCall {
            object,
            function: callee,
            args,
        } => validate_null_safe_call(program, function, object, *callee, args, mir::Type::String),
    }
}

fn validate_nullable_scalar_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::NullableScalarExpression,
) -> Result<(), BackendError> {
    let ty = expression.ty();
    match expression {
        mir::NullableScalarExpression::Null(_) => Ok(()),
        mir::NullableScalarExpression::Value(value) if value.ty() == ty => {
            validate_value_expression(program, function, value)
        }
        mir::NullableScalarExpression::Local { local, .. } => (local_in(function, *local)?.ty
            == mir::Type::NullableScalar(ty))
        .then_some(())
        .ok_or_else(|| malformed_mir("nullable scalar references another local type")),
        mir::NullableScalarExpression::Property {
            object, property, ..
        } => validate_property_operand(
            program,
            function,
            *object,
            *property,
            mir::Type::NullableScalar(ty),
        ),
        mir::NullableScalarExpression::Static { id, .. } => {
            validate_static_operand(program, *id, mir::Type::NullableScalar(ty))
        }
        mir::NullableScalarExpression::Call {
            function: callee,
            args,
            ..
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type != mir::ReturnType::Value(mir::Type::NullableScalar(ty)) {
                return Err(malformed_mir(
                    "nullable scalar call has another return type",
                ));
            }
            validate_call_args(program, function, callee, args)
        }
        mir::NullableScalarExpression::NullSafeProperty {
            object, property, ..
        } => {
            let class = object.class();
            validate_nullable_class_expression(program, function, object)?;
            property_in(program, class, *property).and_then(|definition| {
                (matches!(
                    definition.ty,
                    mir::Type::Scalar(actual) | mir::Type::NullableScalar(actual)
                        if actual == ty
                ))
                .then_some(())
                .ok_or_else(|| malformed_mir("null-safe property has another scalar type"))
            })
        }
        mir::NullableScalarExpression::NullSafeCall {
            object,
            function: callee,
            args,
            ..
        } => validate_null_safe_call(
            program,
            function,
            object,
            *callee,
            args,
            mir::Type::Scalar(ty),
        ),
        mir::NullableScalarExpression::Value(_) => {
            Err(malformed_mir("nullable scalar wraps another scalar type"))
        }
    }
}

fn validate_nullable_class_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::NullableClassExpression,
) -> Result<(), BackendError> {
    let class = expression.class();
    class_in(program, class)?;
    match expression {
        mir::NullableClassExpression::Null(_) => Ok(()),
        mir::NullableClassExpression::Class(value) if value.class() == class => {
            validate_class_expression(program, function, value)
        }
        mir::NullableClassExpression::Local {
            local, transfer, ..
        } => {
            let definition = local_in(function, *local)?;
            if definition.ty != mir::Type::NullableClass(class) {
                return Err(malformed_mir(
                    "nullable class references another local type",
                ));
            }
            if *transfer && !definition.owned {
                return Err(malformed_mir("nullable class transfers a borrowed local"));
            }
            Ok(())
        }
        mir::NullableClassExpression::Property {
            object, property, ..
        } => validate_property_operand(
            program,
            function,
            *object,
            *property,
            mir::Type::NullableClass(class),
        ),
        mir::NullableClassExpression::Call {
            function: callee,
            args,
            return_borrow,
            ..
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type != mir::ReturnType::Value(mir::Type::NullableClass(class)) {
                return Err(malformed_mir("nullable class call has another return type"));
            }
            if *return_borrow != infer_function_return_borrow(program, callee)? {
                return Err(malformed_mir(
                    "nullable class call has inconsistent ownership",
                ));
            }
            validate_call_args(program, function, callee, args)
        }
        mir::NullableClassExpression::NullSafeProperty {
            object, property, ..
        } => {
            let receiver = object.class();
            validate_nullable_class_expression(program, function, object)?;
            property_in(program, receiver, *property).and_then(|definition| {
                (matches!(
                    definition.ty,
                    mir::Type::Class(actual) | mir::Type::NullableClass(actual)
                        if actual == class
                ))
                .then_some(())
                .ok_or_else(|| malformed_mir("null-safe property has another class type"))
            })
        }
        mir::NullableClassExpression::NullSafeCall {
            object,
            function: callee,
            args,
            return_borrow,
            ..
        } => {
            let callee_definition = function_in(program, *callee)?;
            if *return_borrow != infer_function_return_borrow(program, callee_definition)? {
                return Err(malformed_mir(
                    "null-safe class call has inconsistent ownership",
                ));
            }
            validate_null_safe_call(
                program,
                function,
                object,
                *callee,
                args,
                mir::Type::Class(class),
            )
        }
        mir::NullableClassExpression::Class(_) => {
            Err(malformed_mir("nullable class wraps another class type"))
        }
    }
}

fn validate_null_safe_call(
    program: &mir::Program,
    caller: &mir::Function,
    object: &mir::NullableClassExpression,
    callee: mir::FunctionId,
    args: &[mir::Rvalue],
    return_type: mir::Type,
) -> Result<(), BackendError> {
    validate_nullable_class_expression(program, caller, object)?;
    let callee = function_in(program, callee)?;
    let Some(method) = &callee.method else {
        return Err(malformed_mir("null-safe call targets a free function"));
    };
    let nullable_return_type = match return_type {
        mir::Type::Scalar(ty) => mir::Type::NullableScalar(ty),
        mir::Type::String => mir::Type::NullableString,
        mir::Type::Class(class) => mir::Type::NullableClass(class),
        mir::Type::NullableScalar(_) | mir::Type::NullableString | mir::Type::NullableClass(_) => {
            return Err(malformed_mir(
                "null-safe call validator requires a non-null result type",
            ))
        }
    };
    if method.class != object.class()
        || !matches!(
            callee.return_type,
            mir::ReturnType::Value(actual)
                if actual == return_type || actual == nullable_return_type
        )
    {
        return Err(malformed_mir(
            "null-safe call has an incompatible signature",
        ));
    }
    let Some((receiver, parameters)) = callee.params.split_first() else {
        return Err(malformed_mir("null-safe method has no receiver"));
    };
    if local_in(callee, *receiver)?.ty != mir::Type::Class(object.class()) {
        return Err(malformed_mir("null-safe method has another receiver type"));
    }
    validate_call_args_for_params(program, caller, callee, parameters, args, None)
}

fn validate_null_safe_statement_call(
    program: &mir::Program,
    caller: &mir::Function,
    object: &mir::NullableClassExpression,
    callee: mir::FunctionId,
    args: &[mir::Rvalue],
) -> Result<(), BackendError> {
    validate_nullable_class_expression(program, caller, object)?;
    let callee = function_in(program, callee)?;
    let Some(method) = &callee.method else {
        return Err(malformed_mir(
            "null-safe statement call targets a free function",
        ));
    };
    let discards_borrow = matches!(
        callee.return_type,
        mir::ReturnType::Value(mir::Type::Class(_) | mir::Type::NullableClass(_))
    ) && infer_function_return_borrow(program, callee)?.is_some();
    if method.class != object.class()
        || (!matches!(callee.return_type, mir::ReturnType::Void) && !discards_borrow)
    {
        return Err(malformed_mir(
            "null-safe statement call has an incompatible signature",
        ));
    }
    let Some((receiver, parameters)) = callee.params.split_first() else {
        return Err(malformed_mir("null-safe statement method has no receiver"));
    };
    if local_in(callee, *receiver)?.ty != mir::Type::Class(object.class()) {
        return Err(malformed_mir(
            "null-safe statement method has another receiver type",
        ));
    }
    validate_call_args_for_params(program, caller, callee, parameters, args, None)
}

fn validate_format_expression(
    program: &mir::Program,
    function: &mir::Function,
    format: &mir::FormatExpression,
) -> Result<(), BackendError> {
    use crate::format_string::{FormatConversion, FormatPiece};
    let mut borrowed_class_locals: HashMap<mir::LocalId, ClassBorrowMode> = HashMap::new();
    let mut transferred_class_locals = HashSet::new();
    let mut expected_index = 0_usize;
    for piece in &format.pieces {
        let FormatPiece::Argument { index, spec } = piece else {
            continue;
        };
        if *index as usize != expected_index {
            return Err(malformed_mir(
                "format argument indices are not in canonical evaluation order",
            ));
        }
        let argument = format
            .arguments
            .get(expected_index)
            .ok_or_else(|| malformed_mir("format argument index is out of bounds"))?;
        expected_index += 1;
        let valid = matches!(
            (spec.conversion, argument),
            (FormatConversion::Display, mir::FormatArgument::Value(_))
                | (FormatConversion::Display, mir::FormatArgument::String(_))
                | (
                    FormatConversion::Display,
                    mir::FormatArgument::ClassDisplay(_),
                )
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
        match argument {
            mir::FormatArgument::Value(value) => {
                validate_value_expression(program, function, value)?
            }
            mir::FormatArgument::String(value) | mir::FormatArgument::ClassDisplay(value) => {
                validate_string_expression(program, function, value)?
            }
        }
        let mut accesses = ClassLocalAccesses::default();
        collect_format_argument_class_local_accesses(argument, &mut accesses);
        let mut argument_borrows = validate_ordered_class_accesses(
            program,
            "format expression",
            &accesses,
            &borrowed_class_locals,
            &mut transferred_class_locals,
        )?;
        let call = format_argument_call(argument);
        if matches!(argument, mir::FormatArgument::ClassDisplay(_)) && call.is_none() {
            return Err(malformed_mir(
                "class display argument is not lowered through a string call",
            ));
        }
        let call_borrows = call
            .map(|(callee, args)| borrowed_class_call_locals(program, callee, args, 0))
            .transpose()?
            .unwrap_or_default();
        if matches!(argument, mir::FormatArgument::ClassDisplay(_)) {
            for (local, mode) in call_borrows {
                argument_borrows.insert(local, mode);
            }
        }
        for (local, mode) in argument_borrows {
            if transferred_class_locals.contains(&local) {
                return Err(class_access_error(
                    "format expression",
                    "both borrows and transfers",
                    local,
                ));
            }
            if borrowed_class_locals
                .get(&local)
                .is_some_and(|previous| previous.conflicts_with(mode))
            {
                return Err(class_access_error(
                    "format expression",
                    "takes overlapping writable borrows of",
                    local,
                ));
            }
            borrowed_class_locals.insert(local, mode);
        }
    }
    if expected_index != format.arguments.len() {
        return Err(malformed_mir(
            "format expression contains unreferenced arguments",
        ));
    }
    Ok(())
}

fn collect_format_argument_class_local_accesses<'a>(
    argument: &'a mir::FormatArgument,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match argument {
        mir::FormatArgument::Value(value) => collect_value_class_local_accesses(value, accesses),
        mir::FormatArgument::String(value) | mir::FormatArgument::ClassDisplay(value) => {
            collect_string_class_local_accesses(value, accesses)
        }
    }
}

fn format_argument_call(
    argument: &mir::FormatArgument,
) -> Option<(mir::FunctionId, &[mir::Rvalue])> {
    match argument {
        mir::FormatArgument::Value(mir::ValueExpression::Integer(
            mir::IntegerExpression::Call { function, args, .. },
        ))
        | mir::FormatArgument::Value(mir::ValueExpression::Float(mir::FloatExpression::Call {
            function,
            args,
            ..
        }))
        | mir::FormatArgument::Value(mir::ValueExpression::Bool(mir::BoolExpression::Call {
            function,
            args,
        }))
        | mir::FormatArgument::String(mir::StringExpression::Call { function, args })
        | mir::FormatArgument::ClassDisplay(mir::StringExpression::Call { function, args }) => {
            Some((*function, args))
        }
        _ => None,
    }
}

fn borrowed_class_call_locals(
    program: &mir::Program,
    callee: mir::FunctionId,
    args: &[mir::Rvalue],
    parameter_offset: usize,
) -> Result<Vec<(mir::LocalId, ClassBorrowMode)>, BackendError> {
    let callee = function_in(program, callee)?;
    let mut borrows = Vec::new();
    for (argument, parameter) in args.iter().zip(callee.params.iter().skip(parameter_offset)) {
        let parameter = local_in(callee, *parameter)?;
        if !matches!(parameter.ty, mir::Type::Class(_)) || parameter.owned {
            continue;
        }
        let Some(local) = escaping_class_local_borrow(program, argument)? else {
            continue;
        };
        let mode = if parameter.writable {
            ClassBorrowMode::Writable
        } else {
            ClassBorrowMode::Readonly
        };
        if let Some((_, existing)) = borrows.iter_mut().find(|(borrowed, _)| *borrowed == local) {
            if mode.conflicts_with(*existing) {
                *existing = ClassBorrowMode::Writable;
            }
        } else {
            borrows.push((local, mode));
        }
    }
    Ok(borrows)
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

fn class_in(program: &mir::Program, id: ClassId) -> Result<&mir::Class, BackendError> {
    program
        .classes
        .get(id.0)
        .filter(|class| class.id == id)
        .ok_or_else(|| malformed_mir(format!("ClassId class#{} does not exist", id.0)))
}

fn static_in(
    program: &mir::Program,
    id: mir::StaticId,
) -> Result<&mir::StaticProperty, BackendError> {
    program
        .statics
        .get(id.0)
        .filter(|property| property.id == id)
        .ok_or_else(|| malformed_mir(format!("static{} does not exist", id.0)))
}

fn validate_static_operand(
    program: &mir::Program,
    id: mir::StaticId,
    expected: mir::Type,
) -> Result<(), BackendError> {
    let property = static_in(program, id)?;
    if property.ty != expected {
        return Err(malformed_mir(format!(
            "static{} has type {} but is used as {}",
            id.0, property.ty, expected
        )));
    }
    Ok(())
}

fn property_in(
    program: &mir::Program,
    class: ClassId,
    id: crate::class_layout::PropertyId,
) -> Result<&mir::Property, BackendError> {
    let class_definition = class_in(program, class)?;
    if id.class != class {
        return Err(malformed_mir(format!(
            "property#{}:{} does not belong to class#{}",
            id.class.0, id.index, class.0
        )));
    }
    class_definition
        .properties
        .get(id.index)
        .filter(|property| property.id == id)
        .ok_or_else(|| malformed_mir(format!("property{} does not exist", id.index)))
}

fn validate_property_operand(
    program: &mir::Program,
    function: &mir::Function,
    object: mir::LocalId,
    property: crate::class_layout::PropertyId,
    expected: mir::Type,
) -> Result<(), BackendError> {
    let object_definition = local_in(function, object)?;
    let mir::Type::Class(class) = object_definition.ty else {
        return Err(malformed_mir(format!(
            "property operand uses non-class local local{}",
            object.0
        )));
    };
    let property_definition = property_in(program, class, property)?;
    if property_definition.ty != expected {
        return Err(malformed_mir(format!(
            "property{} has type {} but expression expects {}",
            property.index, property_definition.ty, expected
        )));
    }
    Ok(())
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

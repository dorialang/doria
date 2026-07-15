//! Backend-independent structural and type validation for native MIR.

use std::collections::HashSet;

use crate::backend::BackendError;
use crate::class_layout::{compute_class_layout, ClassId, FieldType};
use crate::mir;
use crate::numeric::IntegerType;

pub fn validate_program(program: &mir::Program) -> Result<(), BackendError> {
    for (index, class) in program.classes.iter().enumerate() {
        validate_class(program, index, class)?;
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
    if local_in(function, *receiver)?.ty != mir::Type::Class(class) {
        return Err(malformed_mir(format!(
            "class#{} {kind} {} has an incompatible implicit receiver",
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
        mir::Type::NullableString => FieldType::NullableString,
        mir::Type::Class(class) => FieldType::Class(class),
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
        for statement in &block.statements {
            validate_statement(program, function, statement)?;
        }
        validate_terminator(program, function, &block.terminator)?;
    }
    Ok(())
}

fn validate_type(program: &mir::Program, ty: mir::Type) -> Result<(), BackendError> {
    if let mir::Type::Class(class) = ty {
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
                        return Err(malformed_mir(format!(
                            "class assignment targets borrowed local local{}",
                            target.0
                        )));
                    }
                    validate_class_expression(program, function, expression)
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
            validate_rvalue(program, function, value)
        }
        mir::Statement::DropClass { local, class } => {
            let definition = local_in(function, *local)?;
            if definition.ty != mir::Type::Class(*class) {
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
        mir::Rvalue::Class(value) => validate_class_expression(program, function, value),
    }
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
            ..
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type != mir::ReturnType::Value(mir::Type::Class(class)) {
                return Err(malformed_mir(format!(
                    "class#{} call targets a function with another return type",
                    class.0
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
            let mut consumed_class_locals = HashSet::new();
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
                        if let mir::Rvalue::Class(mir::ClassExpression::Local { local, .. }) = value
                        {
                            if !consumed_class_locals.insert(*local) {
                                return Err(malformed_mir(format!(
                                    "class#{} new expression gives class local local{} to more than one property",
                                    class.0, local.0
                                )));
                            }
                        }
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
                        let assignments = constructor
                            .blocks
                            .iter()
                            .flat_map(|block| block.statements.iter())
                            .filter(|statement| {
                                matches!(
                                    statement,
                                    mir::Statement::AssignProperty {
                                        object,
                                        property: assigned,
                                        ..
                                    } if *object == receiver && *assigned == property.property
                                )
                            })
                            .count();
                        if assignments == 0 {
                            return Err(malformed_mir(format!(
                                "class#{} property{} requires a direct constructor-body initializer",
                                class.0, property.property.index
                            )));
                        }
                        if assignments > 1 && !definition.writable {
                            return Err(malformed_mir(format!(
                                "class#{} readonly property{} is assigned more than once in its constructor body",
                                class.0, property.property.index
                            )));
                        }
                        definition.ty
                    }
                };
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
            }
            Ok(())
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
    }
}

fn validate_call_args(
    program: &mir::Program,
    caller: &mir::Function,
    callee: &mir::Function,
    args: &[mir::Rvalue],
) -> Result<(), BackendError> {
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
        if matches!(parameter_type, mir::Type::Class(_)) {
            match argument {
                mir::Rvalue::Class(mir::ClassExpression::Local { transfer: true, .. })
                    if !parameter_definition.owned
                        && !promoted_transfers.is_some_and(|indices| indices.contains(&index)) =>
                {
                    return Err(malformed_mir(format!(
                        "call to {} transfers argument {} into a borrowed parameter",
                        callee.name,
                        index + 1
                    )));
                }
                mir::Rvalue::Class(
                    mir::ClassExpression::Local {
                        transfer: false, ..
                    }
                    | mir::ClassExpression::Property { .. },
                ) if parameter_definition.owned => {
                    return Err(malformed_mir(format!(
                        "call to {} borrows argument {} for an owned parameter",
                        callee.name,
                        index + 1
                    )));
                }
                _ => {}
            }
        }
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

fn class_in(program: &mir::Program, id: ClassId) -> Result<&mir::Class, BackendError> {
    program
        .classes
        .get(id.0)
        .filter(|class| class.id == id)
        .ok_or_else(|| malformed_mir(format!("ClassId class#{} does not exist", id.0)))
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

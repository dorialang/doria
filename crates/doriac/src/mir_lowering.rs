use std::collections::{HashMap, HashSet};

use crate::class_layout::{compute_class_layout, ClassId, FieldType, PropertyId};
use crate::diagnostics::{Diagnostic, DiagnosticResult};
use crate::format_string::{self, FormatConversion, FormatPiece};
use crate::numeric::{parse_decimal_magnitude, FloatType, FloatValue, IntegerType, IntegerValue};
use crate::semantics::SemanticInfo;
use crate::source::Span;
use crate::{hir, mir};

#[derive(Clone)]
struct FunctionSignature {
    id: mir::FunctionId,
    return_type: mir::ReturnType,
    return_borrow: Option<mir::ReturnBorrow>,
    parameter_types: Vec<mir::Type>,
    parameter_defaults: Vec<Option<crate::const_eval::ConstValue>>,
    parameter_transfers: Vec<bool>,
    parameter_owns: Vec<bool>,
    method_class: Option<ClassId>,
    receiver_mode: Option<mir::ReceiverMode>,
}

#[derive(Clone, Copy)]
struct CallableDecl<'a> {
    function: &'a hir::FunctionDecl,
    class: Option<ClassId>,
    receiver: Option<ClassId>,
}

impl CallableDecl<'_> {
    fn is_top_level(self) -> bool {
        self.class.is_none()
    }
}

pub fn lower_program(program: &hir::Program) -> DiagnosticResult<mir::Program> {
    let class_ids = program
        .semantic_info
        .classes
        .iter()
        .map(|class| (class.name.clone(), class.id))
        .collect::<HashMap<_, _>>();
    let mut static_ids = HashMap::new();
    let mut statics = Vec::new();
    for class in program.items.iter().filter_map(|item| match item {
        hir::Item::Class(class) => Some(class),
        _ => None,
    }) {
        let class_id = class_ids[&class.name];
        for property in class.members.iter().filter_map(|member| match member {
            hir::ClassMember::Property(property) if property.is_static => Some(property),
            _ => None,
        }) {
            let id = mir::StaticId(statics.len());
            let ty = mir_type_ref(&property.ty, &class_ids).ok_or_else(|| {
                vec![unsupported_native_type(
                    &property.ty,
                    property.span,
                    format!(
                        "static property `{}::{}` has a type not supported by native compilation",
                        class.name, property.name
                    ),
                )]
            })?;
            let key = crate::const_eval::ConstKey::Static {
                class_name: class.name.clone(),
                name: property.name.clone(),
            };
            let evaluated = program
                .semantic_info
                .const_evaluation
                .values
                .get(&key)
                .ok_or_else(|| {
                    vec![unsupported(
                        property.span,
                        format!(
                            "static property `{}::{}` has no evaluated initializer",
                            class.name, property.name
                        ),
                    )]
                })?;
            let initializer = lower_static_value(&evaluated.value, ty, property.span)?;
            static_ids.insert((class_id, property.name.clone()), id);
            statics.push(mir::StaticProperty {
                id,
                class: class_id,
                name: property.name.clone(),
                ty,
                writable: property.writable,
                initializer,
            });
        }
    }
    let property_initializers = program
        .items
        .iter()
        .filter_map(|item| match item {
            hir::Item::Class(class) => Some(class),
            _ => None,
        })
        .flat_map(|class| {
            let class_id = class_ids[&class.name];
            class.members.iter().filter_map(move |member| match member {
                hir::ClassMember::Property(property) if !property.is_static => {
                    property.initializer.clone().map(|value| {
                        let property_id = program
                            .semantic_info
                            .classes
                            .iter()
                            .find(|info| info.id == class_id)
                            .and_then(|info| {
                                info.properties
                                    .iter()
                                    .find(|info| info.name == property.name)
                            })
                            .expect("checked property has a stable identity")
                            .id;
                        (property_id, value)
                    })
                }
                hir::ClassMember::Property(_)
                | hir::ClassMember::Method(_)
                | hir::ClassMember::Constant(_) => None,
            })
        })
        .collect::<HashMap<_, _>>();
    let mut constructor_body_initializers = HashSet::new();
    for class in program.items.iter().filter_map(|item| match item {
        hir::Item::Class(class) => Some(class),
        _ => None,
    }) {
        let class_id = class_ids[&class.name];
        if !class.members.iter().any(|member| {
            matches!(member, hir::ClassMember::Method(method) if method.name == "__construct")
        }) {
            continue;
        }

        if let Some(class_info) = program
            .semantic_info
            .classes
            .iter()
            .find(|info| info.id == class_id)
        {
            for property in &class_info.properties {
                if !property.promoted && !property_initializers.contains_key(&property.id) {
                    constructor_body_initializers.insert(property.id);
                }
            }
        }
    }
    let mut declarations = Vec::new();

    for item in &program.items {
        match item {
            hir::Item::Function(function) => declarations.push(CallableDecl {
                function,
                class: None,
                receiver: None,
            }),
            hir::Item::Class(class_decl) => {
                let class = *class_ids
                    .get(&class_decl.name)
                    .expect("checked class has a stable identity");
                for member in &class_decl.members {
                    if let hir::ClassMember::Method(method) = member {
                        declarations.push(CallableDecl {
                            function: method,
                            class: Some(class),
                            receiver: (!method.is_static).then_some(class),
                        });
                    }
                }
            }
            hir::Item::Statement(statement) => {
                return Err(vec![unsupported(
                    stmt_span(statement),
                    "top-level executable statements are not supported by native compilation",
                )]);
            }
            hir::Item::Constant(_) => {}
        }
    }

    let main_indices = declarations
        .iter()
        .enumerate()
        .filter_map(|(index, declaration)| {
            (declaration.is_top_level() && declaration.function.name == "main").then_some(index)
        })
        .collect::<Vec<_>>();
    if main_indices.len() != 1 {
        let span = main_indices
            .get(1)
            .map_or_else(Span::default, |index| declarations[*index].function.span);
        return Err(vec![unsupported(
            span,
            "native programs require exactly one top-level `main` function",
        )]);
    }

    let mut signatures = HashMap::new();
    let mut callable_signatures = Vec::new();
    for (index, declaration) in declarations.iter().enumerate() {
        let function = declaration.function;
        if declaration.is_top_level() && signatures.contains_key(&function.name) {
            return Err(vec![unsupported(
                function.span,
                format!(
                    "duplicate top-level function `{}` cannot be compiled",
                    function.name
                ),
            )]);
        }
        let mut signature = collect_function_signature(
            function,
            mir::FunctionId(index),
            &class_ids,
            &program.semantic_info,
            matches!(function.name.as_str(), "__construct" | "__destruct"),
            declaration.is_top_level() && function.name == "main",
        )?;
        signature.method_class = declaration.class;
        signature.receiver_mode = declaration.receiver.map(|_| {
            if function.writable_this {
                mir::ReceiverMode::Writable
            } else {
                mir::ReceiverMode::Readonly
            }
        });
        if declaration.is_top_level() {
            signatures.insert(function.name.clone(), signature.clone());
        }
        callable_signatures.push(signature);
    }
    let method_signatures = callable_signatures_by_method(&declarations, &callable_signatures);

    let entry = signatures
        .get("main")
        .expect("exactly one collected main signature")
        .id;
    let functions = declarations
        .iter()
        .zip(callable_signatures)
        .map(|(declaration, signature)| {
            let inputs = FunctionLoweringInputs {
                signatures: &signatures,
                method_signatures: &method_signatures,
                semantic_info: &program.semantic_info,
                property_initializers: &property_initializers,
                constructor_body_initializers: &constructor_body_initializers,
                static_ids: &static_ids,
            };
            lower_function(
                declaration.function,
                signature,
                inputs,
                declaration.class,
                declaration.receiver,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;

    let classes = program
        .semantic_info
        .classes
        .iter()
        .map(|class| {
            let properties = class
                .properties
                .iter()
                .map(|property| {
                    Ok(mir::Property {
                        id: property.id,
                        name: property.name.clone(),
                        ty: mir_type_ref(&property.ty, &class_ids).ok_or_else(|| {
                            vec![unsupported_native_type(
                                &property.ty,
                                Span::default(),
                                format!(
                                    "property `${}` has a type that is not supported by native class compilation",
                                    property.name
                                ),
                            )]
                        })?,
                        writable: property.writable,
                        promoted: property.promoted,
                    })
                })
                .collect::<DiagnosticResult<Vec<_>>>()?;
            let layout = compute_class_layout(
                class.id,
                properties.iter().map(|property| {
                    (
                        property.id,
                        field_type(property.ty).expect("checked native property type"),
                    )
                }),
                std::mem::size_of::<usize>() as u32,
            );
            let lifecycle = |name: &str| {
                declarations.iter().enumerate().find_map(|(index, declaration)| {
                    (declaration.receiver == Some(class.id) && declaration.function.name == name)
                        .then_some(mir::FunctionId(index))
                })
            };
            Ok(mir::Class {
                id: class.id,
                name: class.name.clone(),
                properties,
                layout,
                constructor: lifecycle("__construct"),
                destructor: lifecycle("__destruct"),
            })
        })
        .collect::<DiagnosticResult<Vec<_>>>()?;

    Ok(mir::Program {
        classes,
        statics,
        functions,
        entry,
    })
}

fn lower_static_value(
    value: &crate::const_eval::ConstValue,
    ty: mir::Type,
    span: Span,
) -> DiagnosticResult<mir::StaticValue> {
    match (value, ty) {
        (
            crate::const_eval::ConstValue::Integer(value),
            mir::Type::Scalar(mir::ScalarType::Integer(expected))
            | mir::Type::NullableScalar(mir::ScalarType::Integer(expected)),
        ) if value.ty == expected => {
            Ok(mir::StaticValue::Scalar(mir::ScalarValue::Integer(*value)))
        }
        (
            crate::const_eval::ConstValue::Float(value),
            mir::Type::Scalar(mir::ScalarType::Float(expected))
            | mir::Type::NullableScalar(mir::ScalarType::Float(expected)),
        ) if value.ty == expected => Ok(mir::StaticValue::Scalar(mir::ScalarValue::Float(*value))),
        (
            crate::const_eval::ConstValue::Bool(value),
            mir::Type::Scalar(mir::ScalarType::Bool)
            | mir::Type::NullableScalar(mir::ScalarType::Bool),
        ) => Ok(mir::StaticValue::Scalar(mir::ScalarValue::Bool(*value))),
        (
            crate::const_eval::ConstValue::String(value),
            mir::Type::String | mir::Type::NullableString,
        ) => Ok(mir::StaticValue::String(value.clone())),
        (
            crate::const_eval::ConstValue::Null,
            mir::Type::NullableScalar(_) | mir::Type::NullableString | mir::Type::NullableClass(_),
        ) => Ok(mir::StaticValue::Null),
        _ => Err(vec![unsupported(
            span,
            "evaluated static initializer does not match its native type",
        )]),
    }
}

fn collect_function_signature(
    function: &hir::FunctionDecl,
    id: mir::FunctionId,
    class_ids: &HashMap<String, ClassId>,
    semantic_info: &SemanticInfo,
    lifecycle: bool,
    is_entry: bool,
) -> DiagnosticResult<FunctionSignature> {
    let return_type = match function.return_type.as_ref() {
        Some(ty) if scalar_type_ref(ty).is_some() => mir::ReturnType::Value(mir::Type::Scalar(
            scalar_type_ref(ty).expect("checked scalar type"),
        )),
        Some(ty) if is_plain_type(ty, "string") => mir::ReturnType::Value(mir::Type::String),
        Some(ty) if is_nullable_string_type(ty) => {
            mir::ReturnType::Value(mir::Type::NullableString)
        }
        Some(ty) if is_plain_type(ty, "void") => mir::ReturnType::Void,
        Some(ty) if mir_type_ref(ty, class_ids).is_some() => {
            mir::ReturnType::Value(mir_type_ref(ty, class_ids).expect("checked class return"))
        }
        Some(ty) => {
            return Err(vec![unsupported_native_type(
                ty,
                function.span,
                format!(
                    "function `{}` has return type `{ty}`, which is not supported by native compilation",
                    function.name
                ),
            )]);
        }
        None if lifecycle => mir::ReturnType::Void,
        None => {
            return Err(vec![unsupported(
                function.span,
                format!(
                    "function `{}` requires an explicit return type for native compilation",
                    function.name
                ),
            )]);
        }
    };

    if is_entry && !function.params.is_empty() {
        return Err(vec![unsupported(
            function.params[0].span,
            "the native entry function `main` cannot declare parameters",
        )]);
    }

    if is_entry
        && !matches!(
            return_type,
            mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Integer(
                IntegerType::Int64,
            ))) | mir::ReturnType::Void
        )
    {
        return Err(vec![unsupported(
            function.span,
            "the native entry function `main` must return `int`, `int64`, or `void`",
        )]);
    }

    let mut parameter_types = Vec::with_capacity(function.params.len());
    let mut parameter_defaults = Vec::with_capacity(function.params.len());
    let mut parameter_transfers = Vec::with_capacity(function.params.len());
    let mut parameter_owns = Vec::with_capacity(function.params.len());
    for (parameter_index, param) in function.params.iter().enumerate() {
        let parameter_type = if let Some(ty) = mir_type_ref(&param.ty, class_ids) {
            ty
        } else {
            return Err(vec![unsupported_native_type(
                &param.ty,
                param.span,
                format!(
                    "function `{}` has parameter type `{}`, which is not supported by native compilation",
                    function.name, param.ty
                ),
            )]);
        };
        let transfers = matches!(
            parameter_type,
            mir::Type::Class(_) | mir::Type::NullableClass(_)
        ) && param.take;
        let owns = transfers && param.promoted_access.is_none();
        let default = if param.default.is_some() {
            Some(
                semantic_info
                    .parameter_defaults
                    .get(&crate::const_eval::ParameterDefaultKey {
                        function_start: function.span.start,
                        parameter_index,
                    })
                    .cloned()
                    .ok_or_else(|| {
                        vec![Diagnostic::new(
                            "I2001",
                            format!(
                                "checked default for parameter `${}` of `{}` is missing",
                                param.name, function.name
                            ),
                            param.span,
                        )]
                    })?,
            )
        } else {
            None
        };
        parameter_types.push(parameter_type);
        parameter_defaults.push(default);
        parameter_transfers.push(transfers);
        parameter_owns.push(owns);
    }

    Ok(FunctionSignature {
        id,
        return_type,
        return_borrow: semantic_info
            .return_borrows
            .get(&function.span.start)
            .copied()
            .map(mir_return_borrow),
        parameter_types,
        parameter_defaults,
        parameter_transfers,
        parameter_owns,
        method_class: None,
        receiver_mode: None,
    })
}

fn mir_return_borrow(borrow: crate::symbols::ReturnBorrow) -> mir::ReturnBorrow {
    mir::ReturnBorrow {
        source: match borrow.source {
            crate::symbols::BorrowSource::Receiver => mir::BorrowSource::Receiver,
            crate::symbols::BorrowSource::Parameter(index) => mir::BorrowSource::Parameter(index),
        },
        writable: borrow.writable,
    }
}

fn callable_signatures_by_method(
    declarations: &[CallableDecl<'_>],
    signatures: &[FunctionSignature],
) -> HashMap<(ClassId, String), FunctionSignature> {
    declarations
        .iter()
        .zip(signatures.iter())
        .filter_map(|(declaration, signature)| {
            declaration.class.map(|class| {
                (
                    (class, declaration.function.name.clone()),
                    signature.clone(),
                )
            })
        })
        .collect()
}

fn mir_type_ref(
    ty: &crate::types::TypeRef,
    class_ids: &HashMap<String, ClassId>,
) -> Option<mir::Type> {
    let mut plain = ty.clone();
    plain.nullable = false;
    let base = scalar_type_ref(&plain)
        .map(mir::Type::Scalar)
        .or_else(|| is_plain_type(&plain, "string").then_some(mir::Type::String))
        .or_else(|| {
            ty.args
                .is_empty()
                .then(|| class_ids.get(&ty.name).copied().map(mir::Type::Class))
                .flatten()
        })?;
    if ty.nullable {
        Some(match base {
            mir::Type::Scalar(ty) => mir::Type::NullableScalar(ty),
            mir::Type::String => mir::Type::NullableString,
            mir::Type::Class(class) => mir::Type::NullableClass(class),
            mir::Type::NullableScalar(_)
            | mir::Type::NullableString
            | mir::Type::NullableClass(_) => return None,
        })
    } else {
        Some(base)
    }
}

fn field_type(ty: mir::Type) -> Option<FieldType> {
    match ty {
        mir::Type::Scalar(mir::ScalarType::Integer(ty)) => Some(FieldType::Integer(ty)),
        mir::Type::Scalar(mir::ScalarType::Float(ty)) => Some(FieldType::Float(ty)),
        mir::Type::Scalar(mir::ScalarType::Bool) => Some(FieldType::Bool),
        mir::Type::String => Some(FieldType::String),
        mir::Type::NullableScalar(mir::ScalarType::Integer(ty)) => {
            Some(FieldType::NullableInteger(ty))
        }
        mir::Type::NullableScalar(mir::ScalarType::Float(ty)) => Some(FieldType::NullableFloat(ty)),
        mir::Type::NullableScalar(mir::ScalarType::Bool) => Some(FieldType::NullableBool),
        mir::Type::NullableString => Some(FieldType::NullableString),
        mir::Type::Class(class) => Some(FieldType::Class(class)),
        mir::Type::NullableClass(class) => Some(FieldType::NullableClass(class)),
    }
}

fn integer_type_ref(ty: &crate::types::TypeRef) -> Option<IntegerType> {
    (!ty.nullable).then_some(()).and_then(|()| {
        ty.args
            .is_empty()
            .then(|| IntegerType::from_source_name(&ty.name))
            .flatten()
    })
}

fn float_type_ref(ty: &crate::types::TypeRef) -> Option<FloatType> {
    (!ty.nullable).then_some(()).and_then(|()| {
        ty.args
            .is_empty()
            .then(|| FloatType::from_source_name(&ty.name))
            .flatten()
    })
}

fn scalar_type_ref(ty: &crate::types::TypeRef) -> Option<mir::ScalarType> {
    integer_type_ref(ty)
        .map(mir::ScalarType::Integer)
        .or_else(|| float_type_ref(ty).map(mir::ScalarType::Float))
        .or_else(|| is_plain_type(ty, "bool").then_some(mir::ScalarType::Bool))
}

fn is_plain_type(ty: &crate::types::TypeRef, name: &str) -> bool {
    !ty.nullable && ty.name == name && ty.args.is_empty()
}

fn is_nullable_string_type(ty: &crate::types::TypeRef) -> bool {
    ty.nullable && ty.name == "string" && ty.args.is_empty()
}

struct FunctionLoweringInputs<'a> {
    signatures: &'a HashMap<String, FunctionSignature>,
    method_signatures: &'a HashMap<(ClassId, String), FunctionSignature>,
    semantic_info: &'a SemanticInfo,
    property_initializers: &'a HashMap<crate::class_layout::PropertyId, hir::Expr>,
    constructor_body_initializers: &'a HashSet<crate::class_layout::PropertyId>,
    static_ids: &'a HashMap<(ClassId, String), mir::StaticId>,
}

fn lower_function(
    function: &hir::FunctionDecl,
    signature: FunctionSignature,
    inputs: FunctionLoweringInputs<'_>,
    class: Option<ClassId>,
    receiver: Option<ClassId>,
) -> DiagnosticResult<mir::Function> {
    let mut context = LoweringContext::new(
        inputs.signatures.clone(),
        inputs.method_signatures.clone(),
        inputs.semantic_info,
        inputs.property_initializers.clone(),
        inputs.constructor_body_initializers.clone(),
        inputs.static_ids.clone(),
    );
    context.return_borrow = signature.return_borrow;
    let mut params = Vec::new();
    if let Some(class) = receiver {
        let writable = matches!(signature.receiver_mode, Some(mir::ReceiverMode::Writable));
        params.push(context.declare_user_local_owned(
            "this",
            writable,
            mir::Type::Class(class),
            false,
        ));
    }
    params.extend(
        function
            .params
            .iter()
            .zip(signature.parameter_types.iter().copied())
            .zip(signature.parameter_owns.iter().copied())
            .map(|((param, ty), owned)| {
                context.declare_user_local_owned(&param.name, param.writable, ty, owned)
            })
            .collect::<Vec<_>>(),
    );

    lower_function_body(
        &function.body,
        &function.name,
        signature.return_type,
        &mut context,
    )?;
    let (locals, blocks) = context.finish();

    Ok(mir::Function {
        id: signature.id,
        name: inputs_method_name(function, class, inputs.semantic_info),
        method: class.map(|class| mir::MethodIdentity {
            class,
            name: function.name.clone(),
        }),
        receiver_mode: receiver.map(|_| {
            if function.writable_this {
                mir::ReceiverMode::Writable
            } else {
                mir::ReceiverMode::Readonly
            }
        }),
        params,
        return_type: signature.return_type,
        locals,
        blocks,
        entry_block: mir::BlockId(0),
    })
}

fn inputs_method_name(
    function: &hir::FunctionDecl,
    class: Option<ClassId>,
    semantic_info: &crate::semantics::SemanticInfo,
) -> String {
    class.map_or_else(
        || function.name.clone(),
        |class| {
            let class_name = semantic_info
                .classes
                .iter()
                .find(|info| info.id == class)
                .map(|info| info.name.as_str())
                .expect("checked method class has semantic metadata");
            format!("{class_name}::{}", function.name)
        },
    )
}

fn lower_function_body(
    body: &hir::Block,
    function_name: &str,
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    lower_statement_sequence(&body.statements, return_type, context)?;

    if context.current_block.is_some() {
        if return_type == mir::ReturnType::Void {
            context.cleanup_scopes_from(0);
            context.terminate_current(mir::Terminator::ReturnVoid);
        } else {
            return Err(vec![Diagnostic::new(
                "I1101",
                format!(
                    "internal compiler consistency error: checked int function `{function_name}` reaches MIR fallthrough"
                ),
                body.span,
            )]);
        }
    }

    Ok(())
}

fn lower_statement_sequence(
    statements: &[hir::Stmt],
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    for statement in statements {
        if context.current_block.is_none() {
            break;
        }

        match statement {
            hir::Stmt::Echo { expr, .. } => {
                let echo = lower_echo(expr, context)?;
                context.push_statement(echo);
            }
            hir::Stmt::Return { expr, span } => {
                let terminator = lower_return(expr.as_ref(), *span, return_type, context)?;
                context.terminate_current(terminator);
            }
            hir::Stmt::VarDecl(decl) => lower_var_decl(decl, context)?,
            hir::Stmt::Assignment(assignment) => lower_assignment(assignment, context)?,
            hir::Stmt::Increment(increment) => lower_increment(increment, context)?,
            hir::Stmt::If(if_stmt) => lower_if_statement(if_stmt, return_type, context)?,
            hir::Stmt::While(while_stmt) => {
                lower_while_statement(while_stmt, return_type, context)?;
            }
            hir::Stmt::For(for_stmt) => {
                lower_for_statement(for_stmt, return_type, context)?;
            }
            hir::Stmt::Foreach(foreach) => {
                lower_foreach_statement(foreach, return_type, context)?;
            }
            hir::Stmt::Break { span } => lower_loop_control(*span, LoopControl::Break, context)?,
            hir::Stmt::Continue { span } => {
                lower_loop_control(*span, LoopControl::Continue, context)?;
            }
            hir::Stmt::Expr { expr, span } => {
                if let hir::Expr::MethodCall {
                    object,
                    method,
                    args,
                    span: call_span,
                    null_safe,
                } = expr
                {
                    let statement = if *null_safe {
                        let (object, signature, args) =
                            lower_null_safe_method_call(object, method, args, *call_span, context)?;
                        discarded_null_safe_call_statement(object, signature, args, *span)?
                    } else {
                        let (signature, args) =
                            lower_instance_method_call(object, method, args, *call_span, context)?;
                        discarded_call_statement("method", signature, args, *span)?
                    };
                    context.push_statement(statement);
                    continue;
                }
                if let hir::Expr::StaticCall {
                    class_name,
                    method,
                    args,
                    span: call_span,
                } = expr
                {
                    let (signature, args) =
                        lower_static_method_call(class_name, method, args, *call_span, context)?;
                    let statement =
                        discarded_call_statement("static method", signature, args, *span)?;
                    context.push_statement(statement);
                    continue;
                }
                if let hir::Expr::FunctionCall {
                    name,
                    args,
                    span: call_span,
                } = expr
                {
                    if name == "panic" {
                        let message = lower_panic_message(args, *call_span, context)?;
                        context.terminate_current(mir::Terminator::Panic(message));
                    } else if name == "printf" {
                        context.push_statement(mir::Statement::Printf(lower_format_expression(
                            args, *call_span, context,
                        )?));
                    } else if name == "write_file" {
                        let [path, contents] = args.as_slice() else {
                            return Err(vec![unsupported(
                                *call_span,
                                "write_file expects 2 arguments",
                            )]);
                        };
                        context.push_statement(mir::Statement::WriteFile {
                            path: lower_string_expression(path, context)?,
                            contents: lower_string_expression(contents, context)?,
                        });
                    } else if name == "write_stderr" {
                        let [value] = args.as_slice() else {
                            return Err(vec![unsupported(
                                *call_span,
                                "write_stderr expects 1 argument",
                            )]);
                        };
                        context.push_statement(mir::Statement::WriteStderr(
                            lower_string_expression(value, context)?,
                        ));
                    } else {
                        let call = lower_statement_call(name, args, *call_span, context)?;
                        context.push_statement(call);
                    }
                } else {
                    return Err(vec![unsupported(
                        *span,
                        "only calls to void free functions can be used as expression statements in native compilation",
                    )]);
                }
            }
        }
    }

    Ok(())
}

fn lower_if_statement(
    if_stmt: &hir::IfStmt,
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    let condition_block = context.current_block();
    let fallthrough_blocks = lower_if_tree(if_stmt, condition_block, return_type, context)?;

    if fallthrough_blocks.is_empty() {
        context.current_block = None;
        return Ok(());
    }

    let continuation = context.create_block();
    for block in fallthrough_blocks {
        context.terminate_block(block, mir::Terminator::Jump(continuation));
    }
    context.current_block = context.is_reachable(continuation).then_some(continuation);
    Ok(())
}

fn lower_if_tree(
    if_stmt: &hir::IfStmt,
    condition_block: mir::BlockId,
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<Vec<mir::BlockId>> {
    context.current_block = Some(condition_block);
    let condition = lower_condition(&if_stmt.condition, context)?;
    let then_block = context.create_block();
    let else_block = context.create_block();
    context.terminate_condition(condition, then_block, else_block);

    let mut fallthrough_blocks =
        lower_scoped_block(&if_stmt.then_block, then_block, return_type, context)?;

    match &if_stmt.else_branch {
        None => fallthrough_blocks.push(else_block),
        Some(hir::ElseBranch::Block(block)) => {
            fallthrough_blocks.extend(lower_scoped_block(block, else_block, return_type, context)?);
        }
        Some(hir::ElseBranch::If(nested)) => {
            fallthrough_blocks.extend(lower_if_tree(nested, else_block, return_type, context)?);
        }
    }

    Ok(fallthrough_blocks)
}

fn lower_while_statement(
    while_stmt: &hir::WhileStmt,
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    let header_block = context.create_block();
    let body_block = context.create_block();
    let exit_block = context.create_block();

    context.terminate_current(mir::Terminator::Jump(header_block));
    context.current_block = Some(header_block);
    let condition = lower_condition(&while_stmt.condition, context)?;
    context.terminate_condition(condition, body_block, exit_block);

    context.push_loop_targets(LoopTargets {
        continue_block: header_block,
        break_block: exit_block,
        cleanup_depth: context.local_scopes.len(),
    });
    let body_result = lower_scoped_block(&while_stmt.body, body_block, return_type, context);
    context.pop_loop_targets();
    let fallthrough_blocks = body_result?;

    for block in fallthrough_blocks {
        context.terminate_block(block, mir::Terminator::Jump(header_block));
    }
    context.current_block = context.is_reachable(exit_block).then_some(exit_block);
    Ok(())
}

fn lower_for_statement(
    for_stmt: &hir::ForStmt,
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    context.push_scope();
    let result = lower_for_statement_in_scope(for_stmt, return_type, context);
    context.pop_scope();
    result
}

fn lower_for_statement_in_scope(
    for_stmt: &hir::ForStmt,
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    if let Some(initializer) = &for_stmt.initializer {
        match initializer {
            hir::ForInitializer::VarDecl(decl) => lower_var_decl(decl, context)?,
            hir::ForInitializer::Assignment(assignment) => {
                lower_assignment(assignment, context)?;
            }
        }
    }

    let header_block = context.create_block();
    let body_block = context.create_block();
    let increment_block = context.create_block();
    let exit_block = context.create_block();

    context.terminate_current(mir::Terminator::Jump(header_block));
    context.current_block = Some(header_block);
    let condition = for_stmt
        .condition
        .as_ref()
        .map(|condition| lower_condition(condition, context))
        .transpose()?
        .unwrap_or(mir::BoolExpression::Use {
            operand: mir::Operand::Scalar(mir::ScalarValue::Bool(true)),
        });
    context.terminate_condition(condition, body_block, exit_block);

    context.push_loop_targets(LoopTargets {
        continue_block: increment_block,
        break_block: exit_block,
        cleanup_depth: context.local_scopes.len(),
    });
    let body_result = lower_scoped_block(&for_stmt.body, body_block, return_type, context);
    context.pop_loop_targets();
    let fallthrough_blocks = body_result?;

    for block in fallthrough_blocks {
        context.terminate_block(block, mir::Terminator::Jump(increment_block));
    }

    context.current_block = Some(increment_block);
    if let Some(increment) = &for_stmt.increment {
        match increment {
            hir::ForIncrement::Increment(increment) => lower_increment(increment, context)?,
            hir::ForIncrement::Assignment(assignment) => {
                lower_assignment(assignment, context)?;
            }
        }
    }
    context.terminate_current(mir::Terminator::Jump(header_block));
    context.current_block = context.is_reachable(exit_block).then_some(exit_block);
    Ok(())
}

fn lower_foreach_statement(
    foreach: &hir::ForeachStmt,
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    if foreach.key.is_some() {
        return Err(vec![unsupported(
            foreach.span,
            "integer range `foreach` does not support key bindings in native compilation",
        )]);
    }

    let Some((start, end, inclusive)) = grouped_range_parts(&foreach.iterable) else {
        return Err(vec![unsupported(
            foreach.iterable.span(),
            "native compilation currently supports `foreach` only over integer ranges",
        )]);
    };

    if let Some(ty) = &foreach.value.ty {
        if integer_type_ref(ty).is_none() {
            return Err(vec![unsupported(
                foreach.span,
                format!("integer range foreach bindings require an integer type; got `{ty}`"),
            )]);
        }
    }

    context.push_scope();
    let result = lower_range_foreach_in_scope(foreach, start, end, inclusive, return_type, context);
    context.pop_scope();
    result
}

fn lower_range_foreach_in_scope(
    foreach: &hir::ForeachStmt,
    start: &hir::Expr,
    end: &hir::Expr,
    inclusive: bool,
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    let integer_type = context.integer_type(start)?;
    let end_type = context.integer_type(end)?;
    if end_type != integer_type {
        return Err(vec![Diagnostic::new(
            "I1301",
            "internal compiler consistency error: checked range endpoints have different integer types",
            foreach.span,
        )]);
    }

    let start_value = lower_integer_expression(start, context)?;
    ensure_expression_type(&start_value, integer_type, start.span())?;
    let current_local = context.declare_temp(true, integer_type);
    context.push_statement(mir::Statement::AssignLocal {
        target: current_local,
        value: mir::Rvalue::Value(mir::ValueExpression::Integer(start_value)),
    });

    let end_value = lower_integer_expression(end, context)?;
    ensure_expression_type(&end_value, integer_type, end.span())?;
    let end_local = context.declare_temp(false, integer_type);
    context.push_statement(mir::Statement::AssignLocal {
        target: end_local,
        value: mir::Rvalue::Value(mir::ValueExpression::Integer(end_value)),
    });

    let header_block = context.create_block();
    let body_block = context.create_block();
    let update_block = context.create_block();
    let increment_block = inclusive.then(|| context.create_block());
    let exit_block = context.create_block();

    context.terminate_current(mir::Terminator::Jump(header_block));
    context.current_block = Some(header_block);
    context.terminate_current(mir::Terminator::Branch {
        condition: mir::BoolExpression::Compare {
            op: if inclusive {
                mir::CompareOp::LessEqual
            } else {
                mir::CompareOp::Less
            },
            left: Box::new(mir::ValueExpression::Integer(local_integer_expression(
                current_local,
                integer_type,
            ))),
            right: Box::new(mir::ValueExpression::Integer(local_integer_expression(
                end_local,
                integer_type,
            ))),
        },
        then_block: body_block,
        else_block: exit_block,
    });

    let binding_local = context.declare_user_local(
        &foreach.value.name,
        false,
        mir::Type::Scalar(mir::ScalarType::Integer(integer_type)),
    );
    context.push_loop_targets(LoopTargets {
        continue_block: update_block,
        break_block: exit_block,
        cleanup_depth: context.local_scopes.len(),
    });
    context.push_scope();
    context.current_block = Some(body_block);
    context.push_statement(mir::Statement::AssignLocal {
        target: binding_local,
        value: mir::Rvalue::Value(mir::ValueExpression::Integer(local_integer_expression(
            current_local,
            integer_type,
        ))),
    });
    let body_result = lower_statement_sequence(&foreach.body.statements, return_type, context);
    let body_fallthrough = context.current_block;
    context.pop_scope();
    context.pop_loop_targets();
    body_result?;

    if let Some(block) = body_fallthrough {
        context.terminate_block(block, mir::Terminator::Jump(update_block));
    }

    context.current_block = Some(update_block);
    if let Some(increment_block) = increment_block {
        context.terminate_current(mir::Terminator::Branch {
            condition: mir::BoolExpression::Compare {
                op: mir::CompareOp::Equal,
                left: Box::new(mir::ValueExpression::Integer(local_integer_expression(
                    current_local,
                    integer_type,
                ))),
                right: Box::new(mir::ValueExpression::Integer(local_integer_expression(
                    end_local,
                    integer_type,
                ))),
            },
            then_block: exit_block,
            else_block: increment_block,
        });
        context.current_block = Some(increment_block);
    }
    context.push_statement(mir::Statement::AssignLocal {
        target: current_local,
        value: mir::Rvalue::Value(mir::ValueExpression::Integer(
            mir::IntegerExpression::Binary {
                ty: integer_type,
                op: mir::IntegerBinaryOp::Add,
                left: Box::new(local_integer_expression(current_local, integer_type)),
                right: Box::new(mir::IntegerExpression::constant(IntegerValue::one(
                    integer_type,
                ))),
            },
        )),
    });
    context.terminate_current(mir::Terminator::Jump(header_block));
    context.current_block = context.is_reachable(exit_block).then_some(exit_block);
    Ok(())
}

fn grouped_range_parts(expr: &hir::Expr) -> Option<(&hir::Expr, &hir::Expr, bool)> {
    match expr {
        hir::Expr::Grouped { expr, .. } => grouped_range_parts(expr),
        hir::Expr::Range {
            start,
            end,
            inclusive,
            ..
        } => Some((start, end, *inclusive)),
        _ => None,
    }
}

#[derive(Clone, Copy)]
enum LoopControl {
    Break,
    Continue,
}

fn lower_loop_control(
    span: Span,
    control: LoopControl,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    let targets = context.current_loop_targets().ok_or_else(|| {
        let keyword = match control {
            LoopControl::Break => "break",
            LoopControl::Continue => "continue",
        };
        vec![unsupported(
            span,
            format!("`{keyword}` requires an enclosing loop"),
        )]
    })?;
    let target = match control {
        LoopControl::Break => targets.break_block,
        LoopControl::Continue => targets.continue_block,
    };
    context.cleanup_scopes_from(targets.cleanup_depth);
    context.terminate_current(mir::Terminator::Jump(target));
    Ok(())
}

fn lower_scoped_block(
    block: &hir::Block,
    entry_block: mir::BlockId,
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<Vec<mir::BlockId>> {
    context.push_scope();
    context.current_block = Some(entry_block);
    let result = lower_statement_sequence(&block.statements, return_type, context);
    let current_block = context.current_block;
    context.pop_scope();
    result?;
    Ok(current_block.into_iter().collect())
}

struct BlockBuilder {
    id: mir::BlockId,
    statements: Vec<mir::Statement>,
    terminator: Option<mir::Terminator>,
}

#[derive(Clone, Copy)]
struct LoopTargets {
    continue_block: mir::BlockId,
    break_block: mir::BlockId,
    cleanup_depth: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CoalesceSelection {
    Left,
    Right,
    Dynamic,
}

struct LoweringContext<'semantic> {
    signatures: HashMap<String, FunctionSignature>,
    method_signatures: HashMap<(ClassId, String), FunctionSignature>,
    semantic_info: &'semantic SemanticInfo,
    property_initializers: HashMap<crate::class_layout::PropertyId, hir::Expr>,
    constructor_body_initializers: HashSet<crate::class_layout::PropertyId>,
    static_ids: HashMap<(ClassId, String), mir::StaticId>,
    locals: Vec<mir::Local>,
    local_scopes: Vec<HashMap<String, mir::LocalId>>,
    scope_owned_locals: Vec<Vec<(mir::LocalId, ClassId)>>,
    temp_counter: usize,
    blocks: Vec<BlockBuilder>,
    reachable_blocks: Vec<bool>,
    current_block: Option<mir::BlockId>,
    loop_targets: Vec<LoopTargets>,
    return_borrow: Option<mir::ReturnBorrow>,
}

impl<'semantic> LoweringContext<'semantic> {
    fn new(
        signatures: HashMap<String, FunctionSignature>,
        method_signatures: HashMap<(ClassId, String), FunctionSignature>,
        semantic_info: &'semantic SemanticInfo,
        property_initializers: HashMap<crate::class_layout::PropertyId, hir::Expr>,
        constructor_body_initializers: HashSet<crate::class_layout::PropertyId>,
        static_ids: HashMap<(ClassId, String), mir::StaticId>,
    ) -> Self {
        Self {
            signatures,
            method_signatures,
            semantic_info,
            property_initializers,
            constructor_body_initializers,
            static_ids,
            locals: Vec::new(),
            local_scopes: vec![HashMap::new()],
            scope_owned_locals: vec![Vec::new()],
            temp_counter: 0,
            blocks: vec![BlockBuilder {
                id: mir::BlockId(0),
                statements: Vec::new(),
                terminator: None,
            }],
            reachable_blocks: vec![true],
            current_block: Some(mir::BlockId(0)),
            loop_targets: Vec::new(),
            return_borrow: None,
        }
    }

    fn finish(self) -> (Vec<mir::Local>, Vec<mir::BasicBlock>) {
        let blocks = self
            .blocks
            .into_iter()
            .map(|block| mir::BasicBlock {
                id: block.id,
                statements: block.statements,
                terminator: block.terminator.unwrap_or(mir::Terminator::Unreachable),
            })
            .collect();
        (self.locals, blocks)
    }

    fn create_block(&mut self) -> mir::BlockId {
        let id = mir::BlockId(self.blocks.len());
        self.blocks.push(BlockBuilder {
            id,
            statements: Vec::new(),
            terminator: None,
        });
        self.reachable_blocks.push(false);
        id
    }

    fn current_block(&self) -> mir::BlockId {
        self.current_block
            .expect("MIR lowering requires a current block")
    }

    fn push_statement(&mut self, statement: mir::Statement) {
        let block = self.current_block();
        self.blocks[block.0].statements.push(statement);
    }

    fn terminate_current(&mut self, terminator: mir::Terminator) {
        let block = self.current_block();
        self.terminate_block(block, terminator);
        self.current_block = None;
    }

    fn terminate_block(&mut self, block: mir::BlockId, terminator: mir::Terminator) {
        if self.is_reachable(block) {
            for target in terminator_targets(&terminator) {
                self.reachable_blocks[target.0] = true;
            }
        }
        let slot = &mut self.blocks[block.0].terminator;
        assert!(slot.is_none(), "MIR block terminated more than once");
        *slot = Some(terminator);
    }

    fn terminate_condition(
        &mut self,
        condition: mir::BoolExpression,
        then_block: mir::BlockId,
        else_block: mir::BlockId,
    ) {
        match condition {
            mir::BoolExpression::Use {
                operand: mir::Operand::Scalar(mir::ScalarValue::Bool(true)),
            } => {
                self.terminate_current(mir::Terminator::Jump(then_block));
            }
            mir::BoolExpression::Use {
                operand: mir::Operand::Scalar(mir::ScalarValue::Bool(false)),
            } => {
                self.terminate_current(mir::Terminator::Jump(else_block));
            }
            condition => self.terminate_current(mir::Terminator::Branch {
                condition,
                then_block,
                else_block,
            }),
        }
    }

    fn is_reachable(&self, block: mir::BlockId) -> bool {
        self.reachable_blocks.get(block.0).copied().unwrap_or(false)
    }

    fn push_scope(&mut self) {
        self.local_scopes.push(HashMap::new());
        self.scope_owned_locals.push(Vec::new());
    }

    fn pop_scope(&mut self) {
        assert!(
            self.local_scopes.len() > 1,
            "MIR lowering cannot pop the root local scope"
        );
        if self.current_block.is_some() {
            self.cleanup_scopes_from(self.local_scopes.len() - 1);
        }
        self.local_scopes.pop();
        self.scope_owned_locals.pop();
    }

    fn cleanup_scopes_from(&mut self, depth: usize) {
        let cleanup = self.scope_owned_locals[depth..]
            .iter()
            .rev()
            .flat_map(|scope| scope.iter().rev().copied())
            .collect::<Vec<_>>();
        for (local, class) in cleanup {
            self.push_statement(mir::Statement::DropClass { local, class });
        }
    }

    fn has_cleanup_obligations(&self) -> bool {
        self.scope_owned_locals
            .iter()
            .any(|scope| !scope.is_empty())
    }

    fn push_loop_targets(&mut self, targets: LoopTargets) {
        self.loop_targets.push(targets);
    }

    fn pop_loop_targets(&mut self) {
        self.loop_targets
            .pop()
            .expect("MIR lowering cannot pop an empty loop-target stack");
    }

    fn current_loop_targets(&self) -> Option<LoopTargets> {
        self.loop_targets.last().copied()
    }

    fn declare_user_local(&mut self, name: &str, writable: bool, ty: mir::Type) -> mir::LocalId {
        let owned = matches!(ty, mir::Type::Class(_) | mir::Type::NullableClass(_));
        self.declare_user_local_owned(name, writable, ty, owned)
    }

    fn declare_user_local_owned(
        &mut self,
        name: &str,
        writable: bool,
        ty: mir::Type,
        owned: bool,
    ) -> mir::LocalId {
        let id = mir::LocalId(self.locals.len());
        self.locals.push(mir::Local {
            id,
            name: name.to_string(),
            ty,
            writable,
            owned,
            synthetic: false,
        });
        self.local_scopes
            .last_mut()
            .expect("MIR lowering must have a local scope")
            .insert(name.to_string(), id);
        if owned {
            let class = match ty {
                mir::Type::Class(class) | mir::Type::NullableClass(class) => class,
                _ => unreachable!("only class locals may own native drop obligations"),
            };
            self.scope_owned_locals
                .last_mut()
                .expect("MIR lowering must have an ownership scope")
                .push((id, class));
        }
        id
    }

    fn declare_temp(&mut self, writable: bool, ty: IntegerType) -> mir::LocalId {
        let id = mir::LocalId(self.locals.len());
        let name = format!("_tmp{}", self.temp_counter);
        self.temp_counter += 1;
        self.locals.push(mir::Local {
            id,
            name,
            ty: mir::Type::Scalar(mir::ScalarType::Integer(ty)),
            writable,
            owned: false,
            synthetic: true,
        });
        id
    }

    fn declare_return_temp(&mut self, ty: mir::Type, owned: bool) -> mir::LocalId {
        let id = mir::LocalId(self.locals.len());
        let name = format!("_return{}", self.temp_counter);
        self.temp_counter += 1;
        self.locals.push(mir::Local {
            id,
            name,
            ty,
            writable: false,
            owned,
            synthetic: true,
        });
        id
    }

    fn lookup_local(&self, name: &str, span: Span) -> DiagnosticResult<mir::LocalId> {
        self.local_scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
            .ok_or_else(|| {
                vec![unsupported(
                    span,
                    format!("local `${name}` is not available in this native expression"),
                )]
            })
    }

    fn local_type(&self, id: mir::LocalId) -> mir::Type {
        self.locals
            .get(id.0)
            .filter(|local| local.id == id)
            .expect("lowered MIR local must have a matching slot")
            .ty
    }

    fn local_owns(&self, id: mir::LocalId) -> bool {
        self.locals
            .get(id.0)
            .filter(|local| local.id == id)
            .expect("lowered MIR local must have a matching slot")
            .owned
    }

    fn class_id_for_name(&self, name: &str) -> Option<ClassId> {
        self.semantic_info
            .classes
            .iter()
            .find(|class| class.name == name)
            .map(|class| class.id)
    }

    fn class_info(&self, id: ClassId) -> Option<&crate::semantics::ClassSemanticInfo> {
        self.semantic_info
            .classes
            .iter()
            .find(|class| class.id == id)
    }

    fn property_info(
        &self,
        class: ClassId,
        name: &str,
    ) -> Option<&crate::semantics::PropertySemanticInfo> {
        self.class_info(class)?
            .properties
            .iter()
            .find(|property| property.name == name)
    }

    fn lookup_lifecycle(&self, class: ClassId, name: &str) -> Option<FunctionSignature> {
        self.method_signatures
            .get(&(class, name.to_string()))
            .cloned()
    }

    fn lookup_method(
        &self,
        class: ClassId,
        name: &str,
        span: Span,
    ) -> DiagnosticResult<FunctionSignature> {
        self.method_signatures
            .get(&(class, name.to_string()))
            .cloned()
            .ok_or_else(|| {
                vec![unsupported(
                    span,
                    format!("call references unknown method `class#{}::{name}`", class.0),
                )]
            })
    }

    fn constant_decl(&self, expr: &hir::Expr) -> Option<&crate::const_eval::EvaluatedDecl> {
        let key = match expr {
            hir::Expr::Identifier { name, .. } => {
                crate::const_eval::ConstKey::TopLevel(name.clone())
            }
            hir::Expr::StaticMember {
                class_name, member, ..
            } => crate::const_eval::ConstKey::Class {
                class_name: class_name.clone(),
                name: member.clone(),
            },
            hir::Expr::Grouped { expr, .. } => return self.constant_decl(expr),
            _ => return None,
        };
        self.semantic_info.const_evaluation.values.get(&key)
    }

    fn constant_value(&self, expr: &hir::Expr) -> Option<&crate::const_eval::ConstValue> {
        self.constant_decl(expr).map(|decl| &decl.value)
    }

    fn static_property(
        &self,
        class_name: &str,
        member: &str,
        span: Span,
    ) -> DiagnosticResult<(mir::StaticId, mir::Type)> {
        let class = self
            .class_id_for_name(class_name)
            .ok_or_else(|| vec![unsupported(span, format!("unknown class `{class_name}`"))])?;
        let id = self
            .static_ids
            .get(&(class, member.to_string()))
            .copied()
            .ok_or_else(|| {
                vec![unsupported(
                    span,
                    format!("unknown static property `{class_name}::{member}`"),
                )]
            })?;
        let key = crate::const_eval::ConstKey::Static {
            class_name: class_name.to_string(),
            name: member.to_string(),
        };
        let const_ty = self
            .semantic_info
            .const_evaluation
            .values
            .get(&key)
            .expect("checked static has evaluated metadata")
            .ty;
        let ty = native_const_type(const_ty).expect("checked static has a native type");
        Ok((id, ty))
    }

    fn native_type_ref(&self, ty: &crate::types::TypeRef) -> Option<mir::Type> {
        let mut plain = ty.clone();
        plain.nullable = false;
        let base = scalar_type_ref(&plain)
            .map(mir::Type::Scalar)
            .or_else(|| is_plain_type(&plain, "string").then_some(mir::Type::String))
            .or_else(|| {
                ty.args
                    .is_empty()
                    .then(|| self.class_id_for_name(&ty.name).map(mir::Type::Class))
                    .flatten()
            })?;
        if ty.nullable {
            Some(match base {
                mir::Type::Scalar(ty) => mir::Type::NullableScalar(ty),
                mir::Type::String => mir::Type::NullableString,
                mir::Type::Class(class) => mir::Type::NullableClass(class),
                mir::Type::NullableScalar(_)
                | mir::Type::NullableString
                | mir::Type::NullableClass(_) => return None,
            })
        } else {
            Some(base)
        }
    }

    fn lookup_function(&self, name: &str, span: Span) -> DiagnosticResult<FunctionSignature> {
        self.signatures.get(name).cloned().ok_or_else(|| {
            vec![unsupported(
                span,
                format!("call references unknown top-level function `{name}`"),
            )]
        })
    }

    fn integer_type(&self, expr: &hir::Expr) -> DiagnosticResult<IntegerType> {
        self.semantic_info
            .integer_type(expr.span())
            .ok_or_else(|| {
                vec![Diagnostic::new(
                    "I1301",
                    "internal compiler consistency error: checked integer expression has no canonical integer type",
                    expr.span(),
                )]
            })
    }

    fn float_type(&self, expr: &hir::Expr) -> DiagnosticResult<FloatType> {
        self.semantic_info.float_type(expr.span()).ok_or_else(|| {
            vec![Diagnostic::new(
                "I1401",
                "internal compiler consistency error: checked float expression has no canonical float type",
                expr.span(),
            )]
        })
    }

    fn expression_type(&self, expr: &hir::Expr) -> DiagnosticResult<mir::Type> {
        let resolved = self
            .semantic_info
            .expression_type(expr.span())
            .ok_or_else(|| {
                vec![Diagnostic::new(
                    "I2201",
                    "checked expression is missing its resolved semantic type",
                    expr.span(),
                )]
            })?;
        self.mir_resolved_type(resolved).ok_or_else(|| {
            vec![unsupported(
                expr.span(),
                format!("resolved type `{resolved:?}` has no native representation"),
            )]
        })
    }

    fn expression_is_null(&self, expr: &hir::Expr) -> bool {
        matches!(
            self.semantic_info.expression_type(expr.span()),
            Some(crate::types::ResolvedType::Null)
        )
    }

    fn flow_fact(&self, expr: &hir::Expr) -> Option<&crate::narrowing::Fact> {
        let expr = match expr {
            hir::Expr::Grouped { expr, .. } => return self.flow_fact(expr),
            expr => expr,
        };
        self.semantic_info
            .flow_facts
            .get(&(expr.span().start, expr.span().end))
    }

    fn coalesce_selection(&self, left: &hir::Expr) -> CoalesceSelection {
        match self.flow_fact(left) {
            Some(crate::narrowing::Fact::Null) => CoalesceSelection::Right,
            Some(crate::narrowing::Fact::NonNull | crate::narrowing::Fact::Exact(_)) => {
                CoalesceSelection::Left
            }
            None if self.expression_is_null(left) => CoalesceSelection::Right,
            None => CoalesceSelection::Dynamic,
        }
    }

    fn mir_resolved_type(&self, ty: &crate::types::ResolvedType) -> Option<mir::Type> {
        use crate::types::ResolvedType;
        match ty {
            ResolvedType::Integer(ty) => Some(mir::Type::Scalar(mir::ScalarType::Integer(*ty))),
            ResolvedType::Float(ty) => Some(mir::Type::Scalar(mir::ScalarType::Float(*ty))),
            ResolvedType::Bool => Some(mir::Type::Scalar(mir::ScalarType::Bool)),
            ResolvedType::String => Some(mir::Type::String),
            ResolvedType::Class(name) => self.class_id_for_name(name).map(mir::Type::Class),
            ResolvedType::Nullable(inner) => match self.mir_resolved_type(inner)? {
                mir::Type::Scalar(ty) => Some(mir::Type::NullableScalar(ty)),
                mir::Type::String => Some(mir::Type::NullableString),
                mir::Type::Class(class) => Some(mir::Type::NullableClass(class)),
                _ => None,
            },
            ResolvedType::Void
            | ResolvedType::Null
            | ResolvedType::Mixed
            | ResolvedType::Unsupported => None,
        }
    }

    fn local_scalar_type(&self, id: mir::LocalId) -> DiagnosticResult<mir::ScalarType> {
        match self.local_type(id) {
            mir::Type::Scalar(ty) => Ok(ty),
            mir::Type::String
            | mir::Type::NullableScalar(_)
            | mir::Type::NullableString
            | mir::Type::Class(_)
            | mir::Type::NullableClass(_) => Err(vec![Diagnostic::new(
                "I1401",
                format!(
                    "internal compiler consistency error: string local local{} used as a scalar",
                    id.0
                ),
                Span::default(),
            )]),
        }
    }
}

fn native_const_type(ty: crate::const_eval::ConstType) -> Option<mir::Type> {
    match ty {
        crate::const_eval::ConstType::Integer(ty) => {
            Some(mir::Type::Scalar(mir::ScalarType::Integer(ty)))
        }
        crate::const_eval::ConstType::NullableInteger(ty) => {
            Some(mir::Type::NullableScalar(mir::ScalarType::Integer(ty)))
        }
        crate::const_eval::ConstType::Float(ty) => {
            Some(mir::Type::Scalar(mir::ScalarType::Float(ty)))
        }
        crate::const_eval::ConstType::NullableFloat(ty) => {
            Some(mir::Type::NullableScalar(mir::ScalarType::Float(ty)))
        }
        crate::const_eval::ConstType::String => Some(mir::Type::String),
        crate::const_eval::ConstType::Bool => Some(mir::Type::Scalar(mir::ScalarType::Bool)),
        crate::const_eval::ConstType::NullableBool => {
            Some(mir::Type::NullableScalar(mir::ScalarType::Bool))
        }
        crate::const_eval::ConstType::NullableString => Some(mir::Type::NullableString),
        crate::const_eval::ConstType::Null => None,
    }
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

fn lower_var_decl(decl: &hir::VarDecl, context: &mut LoweringContext) -> DiagnosticResult<()> {
    let ty = match &decl.ty {
        Some(ty) if context.native_type_ref(ty).is_some() => {
            context.native_type_ref(ty).expect("guarded native type")
        }
        Some(ty) => {
            return Err(vec![unsupported_native_type(
                ty,
                decl.span,
                format!("local type `{ty}` is not supported by native compilation"),
            )]);
        }
        None => context.expression_type(&decl.initializer)?,
    };

    if ty == mir::Type::String {
        return lower_string_var_decl(decl, context);
    }
    if ty == mir::Type::NullableString {
        let value = lower_nullable_string_expression(&decl.initializer, context)?;
        let local = context.declare_user_local(&decl.name, decl.writable, ty);
        context.push_statement(mir::Statement::AssignLocal {
            target: local,
            value: mir::Rvalue::NullableString(value),
        });
        return Ok(());
    }
    if let mir::Type::NullableScalar(scalar) = ty {
        let value = lower_nullable_scalar_expression(&decl.initializer, scalar, context)?;
        let local = context.declare_user_local(&decl.name, decl.writable, ty);
        context.push_statement(mir::Statement::AssignLocal {
            target: local,
            value: mir::Rvalue::NullableScalar(value),
        });
        return Ok(());
    }
    if let mir::Type::NullableClass(class) = ty {
        let value = lower_nullable_class_expression(&decl.initializer, class, true, context)?;
        let local = context.declare_user_local(&decl.name, decl.writable, ty);
        context.push_statement(mir::Statement::AssignLocal {
            target: local,
            value: mir::Rvalue::NullableClass(value),
        });
        return Ok(());
    }
    if let mir::Type::Class(class) = ty {
        let value = lower_class_expression(&decl.initializer, class, true, context)?;
        let local = context.declare_user_local(&decl.name, decl.writable, ty);
        context.push_statement(mir::Statement::AssignLocal {
            target: local,
            value: mir::Rvalue::Class(value),
        });
        return Ok(());
    }

    let mir::Type::Scalar(scalar_type) = ty else {
        unreachable!("string locals return through lower_string_var_decl")
    };
    let value = lower_value_expression(&decl.initializer, context)?;
    ensure_value_type(&value, scalar_type, decl.initializer.span())?;
    let local =
        context.declare_user_local(&decl.name, decl.writable, mir::Type::Scalar(scalar_type));
    context.push_statement(mir::Statement::AssignLocal {
        target: local,
        value: mir::Rvalue::Value(value),
    });
    Ok(())
}

fn inferred_class_type(expr: &hir::Expr, context: &LoweringContext) -> Option<ClassId> {
    match context.expression_type(expr).ok()? {
        mir::Type::Class(class) | mir::Type::NullableClass(class) => Some(class),
        _ => None,
    }
}

fn is_nullable_string_expression(expr: &hir::Expr, context: &LoweringContext) -> bool {
    context
        .expression_type(expr)
        .is_ok_and(|ty| ty == mir::Type::NullableString)
}

fn is_string_local_initializer(expr: &hir::Expr, context: &LoweringContext) -> bool {
    match expr {
        hir::Expr::String { .. } | hir::Expr::InterpolatedString { .. } => true,
        hir::Expr::Grouped { expr, .. } => is_string_local_initializer(expr, context),
        _ if matches!(
            context.constant_value(expr),
            Some(crate::const_eval::ConstValue::String(_))
        ) =>
        {
            true
        }
        hir::Expr::Binary {
            op: hir::BinaryOp::Concat,
            ..
        } => true,
        hir::Expr::Variable { name, span } => context
            .lookup_local(name, *span)
            .is_ok_and(|local| context.local_type(local) == mir::Type::String),
        hir::Expr::PropertyAccess { .. } => {
            lower_property_place(expr, context).is_ok_and(|(_, _, ty)| ty == mir::Type::String)
        }
        hir::Expr::StaticMember {
            class_name,
            member,
            span,
        } => context
            .static_property(class_name, member, *span)
            .is_ok_and(|(_, ty)| ty == mir::Type::String),
        hir::Expr::FunctionCall { name, .. }
            if matches!(name.as_str(), "sprintf" | "read_file") =>
        {
            true
        }
        hir::Expr::FunctionCall { name, span, .. } => {
            context.lookup_function(name, *span).is_ok_and(|signature| {
                signature.return_type == mir::ReturnType::Value(mir::Type::String)
            })
        }
        hir::Expr::MethodCall {
            object,
            method,
            span,
            ..
        } => inferred_class_type(object, context).is_some_and(|class| {
            context
                .lookup_method(class, method, *span)
                .is_ok_and(|signature| {
                    signature.return_type == mir::ReturnType::Value(mir::Type::String)
                })
        }),
        hir::Expr::StaticCall {
            class_name,
            method,
            span,
            ..
        } => context.class_id_for_name(class_name).is_some_and(|class| {
            context
                .lookup_method(class, method, *span)
                .is_ok_and(|signature| {
                    signature.return_type == mir::ReturnType::Value(mir::Type::String)
                })
        }),
        _ => false,
    }
}

fn lower_string_var_decl(
    decl: &hir::VarDecl,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    let value = lower_string_expression(&decl.initializer, context)?;
    let local = context.declare_user_local(&decl.name, decl.writable, mir::Type::String);
    context.push_statement(mir::Statement::AssignLocal {
        target: local,
        value: mir::Rvalue::String(value),
    });
    Ok(())
}

#[derive(Clone, Copy)]
enum ScalarPlace {
    Local(mir::LocalId),
    NullableLocal(mir::LocalId),
    Property {
        object: mir::LocalId,
        property: crate::class_layout::PropertyId,
    },
    Static(mir::StaticId),
}

impl ScalarPlace {
    fn operand(self) -> mir::Operand {
        match self {
            Self::Local(local) => mir::Operand::Local(local),
            Self::NullableLocal(local) => mir::Operand::NullablePayload(local),
            Self::Property { object, property } => mir::Operand::Property { object, property },
            Self::Static(id) => mir::Operand::Static(id),
        }
    }

    fn assignment(self, value: mir::ValueExpression) -> mir::Statement {
        match self {
            Self::Local(target) => mir::Statement::AssignLocal {
                target,
                value: mir::Rvalue::Value(value),
            },
            Self::NullableLocal(target) => mir::Statement::AssignLocal {
                target,
                value: mir::Rvalue::NullableScalar(mir::NullableScalarExpression::Value(value)),
            },
            Self::Property { object, property } => mir::Statement::AssignProperty {
                object,
                property,
                value: mir::Rvalue::Value(value),
            },
            Self::Static(target) => mir::Statement::AssignStatic {
                target,
                value: mir::Rvalue::Value(value),
            },
        }
    }
}

fn unparenthesized_place(expr: &hir::Expr) -> &hir::Expr {
    match expr {
        hir::Expr::Grouped { expr, .. } => unparenthesized_place(expr),
        _ => expr,
    }
}

fn lower_scalar_place(
    expr: &hir::Expr,
    context: &LoweringContext,
) -> DiagnosticResult<(ScalarPlace, mir::ScalarType)> {
    match unparenthesized_place(expr) {
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            match context.local_type(local) {
                mir::Type::Scalar(ty) => Ok((ScalarPlace::Local(local), ty)),
                mir::Type::NullableScalar(ty) => Ok((ScalarPlace::NullableLocal(local), ty)),
                _ => Err(vec![unsupported(
                    *span,
                    "local is not a scalar mutation place",
                )]),
            }
        }
        hir::Expr::PropertyAccess { .. } => {
            let (object, property, ty) = lower_property_place(expr, context)?;
            let mir::Type::Scalar(scalar) = ty else {
                return Err(vec![unsupported(
                    expr.span(),
                    "class property is not a scalar mutation place",
                )]);
            };
            Ok((ScalarPlace::Property { object, property }, scalar))
        }
        hir::Expr::StaticMember {
            class_name,
            member,
            span,
        } => {
            let (id, ty) = context.static_property(class_name, member, *span)?;
            let mir::Type::Scalar(scalar) = ty else {
                return Err(vec![unsupported(
                    *span,
                    "static property is not a scalar mutation place",
                )]);
            };
            Ok((ScalarPlace::Static(id), scalar))
        }
        _ => Err(vec![unsupported(
            expr.span(),
            "this scalar mutation place is not supported by native compilation",
        )]),
    }
}

fn lower_assignment(
    assignment: &hir::Assignment,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    if assignment.op != hir::AssignOp::Assign {
        let (place, scalar_type) = lower_scalar_place(&assignment.target, context)?;
        let value = lower_compound_value(
            place.operand(),
            scalar_type,
            &assignment.op,
            &assignment.value,
            context,
        )?;
        context.push_statement(place.assignment(value));
        return Ok(());
    }

    let target = unparenthesized_place(&assignment.target);
    if let hir::Expr::StaticMember {
        class_name,
        member,
        span,
    } = target
    {
        let (target, ty) = context.static_property(class_name, member, *span)?;
        let value = lower_rvalue_as_expected(&assignment.value, ty, context)?;
        context.push_statement(mir::Statement::AssignStatic { target, value });
        return Ok(());
    }
    if matches!(target, hir::Expr::PropertyAccess { .. }) {
        if let Ok((object, property, property_type)) = lower_property_place(target, context) {
            let value = lower_rvalue_as_expected(&assignment.value, property_type, context)?;
            context.push_statement(mir::Statement::AssignProperty {
                object,
                property,
                value,
            });
            return Ok(());
        }
    }
    let target = lower_assignment_target(target, context)?;
    if context.local_type(target) == mir::Type::String {
        context.push_statement(mir::Statement::AssignLocal {
            target,
            value: mir::Rvalue::String(lower_string_expression(&assignment.value, context)?),
        });
        return Ok(());
    }
    if context.local_type(target) == mir::Type::NullableString {
        context.push_statement(mir::Statement::AssignLocal {
            target,
            value: mir::Rvalue::NullableString(lower_nullable_string_expression(
                &assignment.value,
                context,
            )?),
        });
        return Ok(());
    }
    if let mir::Type::NullableScalar(scalar) = context.local_type(target) {
        context.push_statement(mir::Statement::AssignLocal {
            target,
            value: mir::Rvalue::NullableScalar(lower_nullable_scalar_expression(
                &assignment.value,
                scalar,
                context,
            )?),
        });
        return Ok(());
    }
    if let mir::Type::NullableClass(class) = context.local_type(target) {
        if !context.local_owns(target) {
            return Err(vec![unsupported(
                assignment.span,
                "this compiler version cannot replace a borrowed nullable class value",
            )]);
        }
        context.push_statement(mir::Statement::AssignLocal {
            target,
            value: mir::Rvalue::NullableClass(lower_nullable_class_expression(
                &assignment.value,
                class,
                true,
                context,
            )?),
        });
        return Ok(());
    }
    if let mir::Type::Class(class) = context.local_type(target) {
        if !context.local_owns(target) {
            return Err(vec![
                Diagnostic::new(
                    "E0505",
                    "this compiler version cannot replace the class value held through a borrowed parameter",
                    assignment.span,
                )
                .with_help("mutate the object's writable properties, or use a `take` parameter when the callee should own a replacement"),
            ]);
        }
        context.push_statement(mir::Statement::AssignLocal {
            target,
            value: mir::Rvalue::Class(lower_class_expression(
                &assignment.value,
                class,
                true,
                context,
            )?),
        });
        return Ok(());
    }

    let scalar_type = context.local_scalar_type(target)?;
    let value = lower_value_expression(&assignment.value, context)?;
    ensure_value_type(&value, scalar_type, assignment.value.span())?;
    context.push_statement(mir::Statement::AssignLocal {
        target,
        value: mir::Rvalue::Value(value),
    });
    Ok(())
}

fn lower_increment(
    increment: &hir::IncrementStmt,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    let (place, scalar_type) = lower_scalar_place(&increment.target, context)?;
    let value = lower_increment_value(place.operand(), scalar_type, &increment.op, increment.span)?;
    context.push_statement(place.assignment(value));
    Ok(())
}

fn lower_increment_value(
    target: mir::Operand,
    scalar_type: mir::ScalarType,
    op: &hir::IncrementOp,
    span: Span,
) -> DiagnosticResult<mir::ValueExpression> {
    match scalar_type {
        mir::ScalarType::Integer(integer_type) => {
            let op = match op {
                hir::IncrementOp::Increment => mir::IntegerBinaryOp::Add,
                hir::IncrementOp::Decrement => mir::IntegerBinaryOp::Subtract,
            };
            Ok(mir::ValueExpression::Integer(
                mir::IntegerExpression::Binary {
                    ty: integer_type,
                    op,
                    left: Box::new(mir::IntegerExpression::use_operand(integer_type, target)),
                    right: Box::new(mir::IntegerExpression::constant(IntegerValue::one(
                        integer_type,
                    ))),
                },
            ))
        }
        mir::ScalarType::Float(float_type) => {
            let op = match op {
                hir::IncrementOp::Increment => mir::FloatBinaryOp::Add,
                hir::IncrementOp::Decrement => mir::FloatBinaryOp::Subtract,
            };
            let one = match float_type {
                FloatType::Float32 => FloatValue::from_f32(1.0),
                FloatType::Float64 => FloatValue::from_f64(1.0),
            };
            Ok(mir::ValueExpression::Float(mir::FloatExpression::Binary {
                ty: float_type,
                op,
                left: Box::new(mir::FloatExpression::Use {
                    ty: float_type,
                    operand: target,
                }),
                right: Box::new(mir::FloatExpression::constant(one)),
            }))
        }
        mir::ScalarType::Bool => Err(vec![unsupported(span, "bool increment is invalid")]),
    }
}

fn lower_assignment_target(
    target: &hir::Expr,
    context: &LoweringContext,
) -> DiagnosticResult<mir::LocalId> {
    match unparenthesized_place(target) {
        hir::Expr::Variable { name, span } => context.lookup_local(name, *span),
        _ => Err(vec![unsupported(
            target.span(),
            "this assignment target is not supported by native compilation",
        )]),
    }
}

fn lower_echo(expr: &hir::Expr, context: &LoweringContext) -> DiagnosticResult<mir::Statement> {
    match expr {
        hir::Expr::String { value, .. } => Ok(mir::Statement::EchoStringLiteral(value.clone())),
        _ => lower_display_string_expression(expr, context).map(mir::Statement::EchoString),
    }
}

fn lower_panic_message(
    args: &[hir::Expr],
    span: Span,
    context: &LoweringContext,
) -> DiagnosticResult<mir::StringExpression> {
    let [message] = args else {
        return Err(vec![unsupported(
            span,
            format!("panic expects exactly 1 argument, got {}", args.len()),
        )]);
    };
    lower_string_expression(message, context)
}

fn lower_string_expression(
    expr: &hir::Expr,
    context: &LoweringContext,
) -> DiagnosticResult<mir::StringExpression> {
    if let Some(crate::const_eval::ConstValue::String(value)) = context.constant_value(expr) {
        return Ok(mir::StringExpression::Literal(value.clone()));
    }
    match expr {
        hir::Expr::Binary {
            left,
            op: hir::BinaryOp::Coalesce,
            right,
            ..
        } => match context.coalesce_selection(left) {
            CoalesceSelection::Left => lower_string_expression(left, context),
            CoalesceSelection::Right => lower_string_expression(right, context),
            CoalesceSelection::Dynamic => Ok(mir::StringExpression::Coalesce {
                left: Box::new(lower_nullable_string_expression(left, context)?),
                right: Box::new(lower_string_expression(right, context)?),
            }),
        },
        hir::Expr::String { value, .. } => Ok(mir::StringExpression::Literal(value.clone())),
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            if context.local_type(local) == mir::Type::String {
                Ok(mir::StringExpression::Local(local))
            } else if context.local_type(local) == mir::Type::NullableString {
                Ok(mir::StringExpression::NullableLocalAssumeNonNull(local))
            } else {
                Err(vec![unsupported(
                    *span,
                    "this local cannot be used as a string expression",
                )])
            }
        }
        hir::Expr::Grouped { expr, .. } => lower_string_expression(expr, context),
        hir::Expr::PropertyAccess { .. } => {
            let (object, property) = lower_property_operand(expr, mir::Type::String, context)?;
            Ok(mir::StringExpression::Property { object, property })
        }
        hir::Expr::StaticMember {
            class_name,
            member,
            span,
        } => {
            let (id, ty) = context.static_property(class_name, member, *span)?;
            if ty != mir::Type::String {
                return Err(vec![unsupported(*span, "static property is not string")]);
            }
            Ok(mir::StringExpression::Static(id))
        }
        hir::Expr::Binary {
            op: hir::BinaryOp::Concat,
            ..
        } => {
            let mut parts = Vec::new();
            append_string_concat_parts(expr, context, &mut parts)?;
            Ok(mir::StringExpression::Concat(parts))
        }
        hir::Expr::InterpolatedString { parts, .. } => {
            let mut lowered = Vec::new();
            for part in parts {
                match part {
                    hir::InterpolatedStringPart::Text { value: text, .. } => {
                        lowered.push(mir::StringExpression::Literal(text.clone()));
                    }
                    hir::InterpolatedStringPart::Expr(expr) => {
                        lowered.push(lower_display_string_expression(expr, context)?);
                    }
                }
            }
            Ok(mir::StringExpression::Concat(lowered))
        }
        hir::Expr::FunctionCall { name, args, span } => {
            if name == "read_file" {
                let [path] = args.as_slice() else {
                    return Err(vec![unsupported(*span, "read_file expects 1 argument")]);
                };
                return Ok(mir::StringExpression::ReadFile(Box::new(
                    lower_string_expression(path, context)?,
                )));
            }
            if name == "sprintf" {
                return Ok(mir::StringExpression::Format(Box::new(
                    lower_format_expression(args, *span, context)?,
                )));
            }
            let signature = context.lookup_function(name, *span)?;
            if signature.return_type != mir::ReturnType::Value(mir::Type::String) {
                return Err(vec![unsupported(
                    *span,
                    format!("function `{name}` does not return string"),
                )]);
            }
            Ok(mir::StringExpression::Call {
                function: signature.id,
                args: lower_call_args(name, args, signature, *span, context)?,
            })
        }
        hir::Expr::MethodCall {
            object,
            method,
            args,
            span,
            ..
        } => {
            let (signature, args) =
                lower_instance_method_call(object, method, args, *span, context)?;
            if signature.return_type != mir::ReturnType::Value(mir::Type::String) {
                return Err(vec![unsupported(*span, "method does not return string")]);
            }
            Ok(mir::StringExpression::Call {
                function: signature.id,
                args,
            })
        }
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } => {
            let (signature, args) =
                lower_static_method_call(class_name, method, args, *span, context)?;
            if signature.return_type != mir::ReturnType::Value(mir::Type::String) {
                return Err(vec![unsupported(
                    *span,
                    "static method does not return string",
                )]);
            }
            Ok(mir::StringExpression::Call {
                function: signature.id,
                args,
            })
        }
        _ => Err(vec![unsupported(
            expr.span(),
            "this expression cannot be written by `echo` in native compilation",
        )]),
    }
}

fn lower_nullable_string_expression(
    expr: &hir::Expr,
    context: &LoweringContext,
) -> DiagnosticResult<mir::NullableStringExpression> {
    if let Some(value) = context.constant_value(expr) {
        return match value {
            crate::const_eval::ConstValue::String(value) => {
                Ok(mir::NullableStringExpression::String(
                    mir::StringExpression::Literal(value.clone()),
                ))
            }
            crate::const_eval::ConstValue::Null => Ok(mir::NullableStringExpression::Null),
            _ => Err(vec![unsupported(
                expr.span(),
                "constant is not a nullable string",
            )]),
        };
    }
    match expr {
        hir::Expr::Null { .. } => Ok(mir::NullableStringExpression::Null),
        hir::Expr::Grouped { expr, .. } => lower_nullable_string_expression(expr, context),
        hir::Expr::Binary {
            left,
            op: hir::BinaryOp::Coalesce,
            right,
            ..
        } => match context.coalesce_selection(left) {
            CoalesceSelection::Left => lower_nullable_string_expression(left, context),
            CoalesceSelection::Right => lower_nullable_string_expression(right, context),
            CoalesceSelection::Dynamic => Ok(mir::NullableStringExpression::Coalesce {
                left: Box::new(lower_nullable_string_expression(left, context)?),
                right: Box::new(lower_nullable_string_expression(right, context)?),
            }),
        },
        hir::Expr::PropertyAccess {
            object,
            property,
            null_safe: true,
            span,
        } => {
            let (object, property, ty) =
                lower_null_safe_property(object, property, *span, context)?;
            if !matches!(ty, mir::Type::String | mir::Type::NullableString) {
                return Err(vec![unsupported(
                    *span,
                    "null-safe property does not produce ?string",
                )]);
            }
            Ok(mir::NullableStringExpression::NullSafeProperty {
                object: Box::new(object),
                property,
            })
        }
        hir::Expr::PropertyAccess { .. } => {
            let (object, property, ty) = lower_property_place(expr, context)?;
            match ty {
                mir::Type::NullableString => {
                    Ok(mir::NullableStringExpression::Property { object, property })
                }
                mir::Type::String => Ok(mir::NullableStringExpression::String(
                    mir::StringExpression::Property { object, property },
                )),
                _ => Err(vec![unsupported(
                    expr.span(),
                    "property does not produce string or ?string",
                )]),
            }
        }
        hir::Expr::StaticMember {
            class_name,
            member,
            span,
        } => {
            let (id, ty) = context.static_property(class_name, member, *span)?;
            match ty {
                mir::Type::NullableString => Ok(mir::NullableStringExpression::Static(id)),
                mir::Type::String => Ok(mir::NullableStringExpression::String(
                    mir::StringExpression::Static(id),
                )),
                _ => Err(vec![unsupported(
                    *span,
                    "static property does not produce string or ?string",
                )]),
            }
        }
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            match context.local_type(local) {
                mir::Type::NullableString => match context.flow_fact(expr) {
                    Some(crate::narrowing::Fact::Null) => Ok(mir::NullableStringExpression::Null),
                    Some(crate::narrowing::Fact::NonNull | crate::narrowing::Fact::Exact(_)) => {
                        Ok(mir::NullableStringExpression::String(
                            mir::StringExpression::NullableLocalAssumeNonNull(local),
                        ))
                    }
                    None => Ok(mir::NullableStringExpression::Local(local)),
                },
                mir::Type::String => Ok(mir::NullableStringExpression::String(
                    mir::StringExpression::Local(local),
                )),
                _ => Err(vec![unsupported(
                    *span,
                    "expected nullable string expression",
                )]),
            }
        }
        hir::Expr::FunctionCall { name, args, span } if name == "read_line" => {
            if !args.is_empty() {
                return Err(vec![unsupported(*span, "read_line expects no arguments")]);
            }
            Ok(mir::NullableStringExpression::ReadLine)
        }
        hir::Expr::FunctionCall { name, args, span } => {
            let signature = context.lookup_function(name, *span)?;
            match signature.return_type {
                mir::ReturnType::Value(mir::Type::NullableString) => {
                    Ok(mir::NullableStringExpression::Call {
                        function: signature.id,
                        args: lower_call_args(name, args, signature, *span, context)?,
                    })
                }
                mir::ReturnType::Value(mir::Type::String) => Ok(
                    mir::NullableStringExpression::String(lower_string_expression(expr, context)?),
                ),
                _ => Err(vec![unsupported(
                    *span,
                    format!("function `{name}` does not return string or ?string"),
                )]),
            }
        }
        hir::Expr::MethodCall {
            object,
            method,
            args,
            span,
            null_safe: true,
        } => {
            let (object, signature, args) =
                lower_null_safe_method_call(object, method, args, *span, context)?;
            if !matches!(
                signature.return_type,
                mir::ReturnType::Value(mir::Type::String | mir::Type::NullableString)
            ) {
                return Err(vec![unsupported(
                    *span,
                    "null-safe method does not produce ?string",
                )]);
            }
            Ok(mir::NullableStringExpression::NullSafeCall {
                object: Box::new(object),
                function: signature.id,
                args,
            })
        }
        hir::Expr::MethodCall {
            object,
            method,
            args,
            span,
            null_safe: false,
        } => {
            let (signature, args) =
                lower_instance_method_call(object, method, args, *span, context)?;
            match signature.return_type {
                mir::ReturnType::Value(mir::Type::NullableString) => {
                    Ok(mir::NullableStringExpression::Call {
                        function: signature.id,
                        args,
                    })
                }
                mir::ReturnType::Value(mir::Type::String) => Ok(
                    mir::NullableStringExpression::String(mir::StringExpression::Call {
                        function: signature.id,
                        args,
                    }),
                ),
                _ => Err(vec![unsupported(
                    *span,
                    "method does not return string or ?string",
                )]),
            }
        }
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } => {
            let (signature, args) =
                lower_static_method_call(class_name, method, args, *span, context)?;
            match signature.return_type {
                mir::ReturnType::Value(mir::Type::NullableString) => {
                    Ok(mir::NullableStringExpression::Call {
                        function: signature.id,
                        args,
                    })
                }
                mir::ReturnType::Value(mir::Type::String) => Ok(
                    mir::NullableStringExpression::String(mir::StringExpression::Call {
                        function: signature.id,
                        args,
                    }),
                ),
                _ => Err(vec![unsupported(
                    *span,
                    "static method does not return string or ?string",
                )]),
            }
        }
        _ if is_string_local_initializer(expr, context) => Ok(
            mir::NullableStringExpression::String(lower_string_expression(expr, context)?),
        ),
        _ => Err(vec![unsupported(
            expr.span(),
            "expected nullable string expression",
        )]),
    }
}

fn lower_nullable_scalar_expression(
    expr: &hir::Expr,
    expected: mir::ScalarType,
    context: &LoweringContext,
) -> DiagnosticResult<mir::NullableScalarExpression> {
    match expr {
        hir::Expr::Null { .. } => Ok(mir::NullableScalarExpression::Null(expected)),
        hir::Expr::Grouped { expr, .. } => {
            lower_nullable_scalar_expression(expr, expected, context)
        }
        hir::Expr::Binary {
            left,
            op: hir::BinaryOp::Coalesce,
            right,
            ..
        } => match context.coalesce_selection(left) {
            CoalesceSelection::Left => lower_nullable_scalar_expression(left, expected, context),
            CoalesceSelection::Right => lower_nullable_scalar_expression(right, expected, context),
            CoalesceSelection::Dynamic => Ok(mir::NullableScalarExpression::Coalesce {
                ty: expected,
                left: Box::new(lower_nullable_scalar_expression(left, expected, context)?),
                right: Box::new(lower_nullable_scalar_expression(right, expected, context)?),
            }),
        },
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            match context.local_type(local) {
                mir::Type::NullableScalar(ty) if ty == expected => match context.flow_fact(expr) {
                    Some(crate::narrowing::Fact::Null) => {
                        Ok(mir::NullableScalarExpression::Null(ty))
                    }
                    Some(crate::narrowing::Fact::NonNull | crate::narrowing::Fact::Exact(_)) => {
                        Ok(mir::NullableScalarExpression::Value(
                            value_expression_from_operand(ty, mir::Operand::NullablePayload(local)),
                        ))
                    }
                    None => Ok(mir::NullableScalarExpression::Local { ty, local }),
                },
                mir::Type::Scalar(ty) if ty == expected => Ok(
                    mir::NullableScalarExpression::Value(lower_value_expression(expr, context)?),
                ),
                _ => Err(vec![unsupported(
                    *span,
                    "expected nullable scalar expression",
                )]),
            }
        }
        hir::Expr::PropertyAccess {
            object,
            property,
            null_safe: true,
            span,
        } => {
            let (object, property, ty) =
                lower_null_safe_property(object, property, *span, context)?;
            if !matches!(
                ty,
                mir::Type::Scalar(actual) | mir::Type::NullableScalar(actual)
                    if actual == expected
            ) {
                return Err(vec![unsupported(
                    *span,
                    "null-safe property has another scalar type",
                )]);
            }
            Ok(mir::NullableScalarExpression::NullSafeProperty {
                ty: expected,
                object: Box::new(object),
                property,
            })
        }
        hir::Expr::PropertyAccess { .. } => {
            let (object, property, ty) = lower_property_place(expr, context)?;
            match ty {
                mir::Type::NullableScalar(actual) if actual == expected => {
                    Ok(mir::NullableScalarExpression::Property {
                        ty: expected,
                        object,
                        property,
                    })
                }
                mir::Type::Scalar(actual) if actual == expected => Ok(
                    mir::NullableScalarExpression::Value(value_expression_from_operand(
                        expected,
                        mir::Operand::Property { object, property },
                    )),
                ),
                _ => Err(vec![unsupported(
                    expr.span(),
                    "property has another scalar type",
                )]),
            }
        }
        hir::Expr::StaticMember {
            class_name,
            member,
            span,
        } => {
            let (id, ty) = context.static_property(class_name, member, *span)?;
            match ty {
                mir::Type::NullableScalar(actual) if actual == expected => {
                    Ok(mir::NullableScalarExpression::Static { ty: expected, id })
                }
                mir::Type::Scalar(actual) if actual == expected => {
                    Ok(mir::NullableScalarExpression::Value(
                        value_expression_from_operand(expected, mir::Operand::Static(id)),
                    ))
                }
                _ => Err(vec![unsupported(
                    *span,
                    "static property has another scalar type",
                )]),
            }
        }
        hir::Expr::FunctionCall { name, args, span } => {
            let signature = context.lookup_function(name, *span)?;
            match signature.return_type {
                mir::ReturnType::Value(mir::Type::NullableScalar(actual)) if actual == expected => {
                    Ok(mir::NullableScalarExpression::Call {
                        ty: expected,
                        function: signature.id,
                        args: lower_call_args(name, args, signature, *span, context)?,
                    })
                }
                mir::ReturnType::Value(mir::Type::Scalar(actual)) if actual == expected => {
                    let value = lower_value_expression(expr, context)?;
                    ensure_value_type(&value, expected, *span)?;
                    Ok(mir::NullableScalarExpression::Value(value))
                }
                _ => Err(vec![unsupported(
                    *span,
                    "function has another scalar return type",
                )]),
            }
        }
        hir::Expr::MethodCall {
            object,
            method,
            args,
            span,
            null_safe: true,
        } => {
            let (object, signature, args) =
                lower_null_safe_method_call(object, method, args, *span, context)?;
            if !matches!(
                signature.return_type,
                mir::ReturnType::Value(
                    mir::Type::Scalar(actual) | mir::Type::NullableScalar(actual)
                ) if actual == expected
            ) {
                return Err(vec![unsupported(
                    *span,
                    "null-safe method has another scalar return type",
                )]);
            }
            Ok(mir::NullableScalarExpression::NullSafeCall {
                ty: expected,
                object: Box::new(object),
                function: signature.id,
                args,
            })
        }
        hir::Expr::MethodCall {
            object,
            method,
            args,
            span,
            null_safe: false,
        } => {
            let (signature, args) =
                lower_instance_method_call(object, method, args, *span, context)?;
            match signature.return_type {
                mir::ReturnType::Value(mir::Type::NullableScalar(actual)) if actual == expected => {
                    Ok(mir::NullableScalarExpression::Call {
                        ty: expected,
                        function: signature.id,
                        args,
                    })
                }
                mir::ReturnType::Value(mir::Type::Scalar(actual)) if actual == expected => {
                    Ok(mir::NullableScalarExpression::Value(call_value_expression(
                        expected,
                        signature.id,
                        args,
                    )))
                }
                _ => Err(vec![unsupported(
                    *span,
                    "method has another scalar return type",
                )]),
            }
        }
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } => {
            let (signature, args) =
                lower_static_method_call(class_name, method, args, *span, context)?;
            match signature.return_type {
                mir::ReturnType::Value(mir::Type::NullableScalar(actual)) if actual == expected => {
                    Ok(mir::NullableScalarExpression::Call {
                        ty: expected,
                        function: signature.id,
                        args,
                    })
                }
                mir::ReturnType::Value(mir::Type::Scalar(actual)) if actual == expected => {
                    Ok(mir::NullableScalarExpression::Value(call_value_expression(
                        expected,
                        signature.id,
                        args,
                    )))
                }
                _ => Err(vec![unsupported(
                    *span,
                    "static method has another scalar return type",
                )]),
            }
        }
        _ => {
            let value = lower_value_expression(expr, context)?;
            ensure_value_type(&value, expected, expr.span())?;
            Ok(mir::NullableScalarExpression::Value(value))
        }
    }
}

fn lower_nullable_class_expression(
    expr: &hir::Expr,
    expected: ClassId,
    transfer: bool,
    context: &LoweringContext,
) -> DiagnosticResult<mir::NullableClassExpression> {
    match expr {
        hir::Expr::Null { .. } => Ok(mir::NullableClassExpression::Null(expected)),
        hir::Expr::Grouped { expr, .. } => {
            lower_nullable_class_expression(expr, expected, transfer, context)
        }
        hir::Expr::Binary {
            left,
            op: hir::BinaryOp::Coalesce,
            right,
            ..
        } => match context.coalesce_selection(left) {
            CoalesceSelection::Left => {
                lower_nullable_class_expression(left, expected, transfer, context)
            }
            CoalesceSelection::Right => {
                lower_nullable_class_expression(right, expected, transfer, context)
            }
            CoalesceSelection::Dynamic => Ok(mir::NullableClassExpression::Coalesce {
                class: expected,
                left: Box::new(lower_nullable_class_expression(
                    left, expected, transfer, context,
                )?),
                right: Box::new(lower_nullable_class_expression(
                    right, expected, transfer, context,
                )?),
                transfer,
            }),
        },
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            match context.local_type(local) {
                mir::Type::NullableClass(class) if class == expected => {
                    if transfer && !context.local_owns(local) {
                        return Err(vec![unsupported(
                            *span,
                            "borrowed nullable class value cannot be given away",
                        )]);
                    }
                    match context.flow_fact(expr) {
                        Some(crate::narrowing::Fact::Null) => {
                            Ok(mir::NullableClassExpression::Null(class))
                        }
                        Some(
                            crate::narrowing::Fact::NonNull | crate::narrowing::Fact::Exact(_),
                        ) => Ok(mir::NullableClassExpression::Class(lower_class_expression(
                            expr, expected, transfer, context,
                        )?)),
                        None => Ok(mir::NullableClassExpression::Local {
                            class,
                            local,
                            transfer,
                        }),
                    }
                }
                mir::Type::Class(class) if class == expected => {
                    Ok(mir::NullableClassExpression::Class(lower_class_expression(
                        expr, expected, transfer, context,
                    )?))
                }
                _ => Err(vec![unsupported(
                    *span,
                    "expected nullable class expression",
                )]),
            }
        }
        hir::Expr::PropertyAccess {
            object,
            property,
            null_safe: true,
            span,
        } => {
            let (object, property, ty) =
                lower_null_safe_property(object, property, *span, context)?;
            if !matches!(
                ty,
                mir::Type::Class(actual) | mir::Type::NullableClass(actual)
                    if actual == expected
            ) {
                return Err(vec![unsupported(
                    *span,
                    "null-safe property has another class type",
                )]);
            }
            Ok(mir::NullableClassExpression::NullSafeProperty {
                class: expected,
                object: Box::new(object),
                property,
            })
        }
        hir::Expr::PropertyAccess { span, .. } => {
            if transfer {
                return Err(vec![unsupported(
                    *span,
                    "moving directly out of an owned nullable class property is not supported",
                )]);
            }
            let (object, property, ty) = lower_property_place(expr, context)?;
            match ty {
                mir::Type::NullableClass(actual) if actual == expected => {
                    Ok(mir::NullableClassExpression::Property {
                        class: expected,
                        object,
                        property,
                    })
                }
                mir::Type::Class(actual) if actual == expected => Ok(
                    mir::NullableClassExpression::Class(mir::ClassExpression::Property {
                        class: expected,
                        object,
                        property,
                    }),
                ),
                _ => Err(vec![unsupported(*span, "property has another class type")]),
            }
        }
        hir::Expr::FunctionCall { name, args, span } => {
            let signature = context.lookup_function(name, *span)?;
            match signature.return_type {
                mir::ReturnType::Value(mir::Type::NullableClass(actual)) if actual == expected => {
                    Ok(mir::NullableClassExpression::Call {
                        class: expected,
                        function: signature.id,
                        return_borrow: signature.return_borrow,
                        args: lower_call_args_with_ownership(
                            name, args, signature, *span, context,
                        )?,
                    })
                }
                mir::ReturnType::Value(mir::Type::Class(actual)) if actual == expected => {
                    Ok(mir::NullableClassExpression::Class(lower_class_expression(
                        expr, expected, transfer, context,
                    )?))
                }
                _ => Err(vec![unsupported(
                    *span,
                    "function has another class return type",
                )]),
            }
        }
        hir::Expr::MethodCall {
            object,
            method,
            args,
            span,
            null_safe: true,
        } => {
            let (object, signature, args) =
                lower_null_safe_method_call(object, method, args, *span, context)?;
            if !matches!(
                signature.return_type,
                mir::ReturnType::Value(
                    mir::Type::Class(actual) | mir::Type::NullableClass(actual)
                ) if actual == expected
            ) {
                return Err(vec![unsupported(
                    *span,
                    "null-safe method has another class return type",
                )]);
            }
            Ok(mir::NullableClassExpression::NullSafeCall {
                class: expected,
                object: Box::new(object),
                function: signature.id,
                args,
                return_borrow: signature.return_borrow,
            })
        }
        hir::Expr::MethodCall {
            object,
            method,
            args,
            span,
            null_safe: false,
        } => {
            let (signature, args) =
                lower_instance_method_call(object, method, args, *span, context)?;
            match signature.return_type {
                mir::ReturnType::Value(mir::Type::NullableClass(actual)) if actual == expected => {
                    Ok(mir::NullableClassExpression::Call {
                        class: expected,
                        function: signature.id,
                        args,
                        return_borrow: signature.return_borrow,
                    })
                }
                mir::ReturnType::Value(mir::Type::Class(actual)) if actual == expected => Ok(
                    mir::NullableClassExpression::Class(mir::ClassExpression::Call {
                        class: expected,
                        function: signature.id,
                        args,
                        return_borrow: signature.return_borrow,
                    }),
                ),
                _ => Err(vec![unsupported(
                    *span,
                    "method has another class return type",
                )]),
            }
        }
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } => {
            let (signature, args) =
                lower_static_method_call(class_name, method, args, *span, context)?;
            match signature.return_type {
                mir::ReturnType::Value(mir::Type::NullableClass(actual)) if actual == expected => {
                    Ok(mir::NullableClassExpression::Call {
                        class: expected,
                        function: signature.id,
                        args,
                        return_borrow: signature.return_borrow,
                    })
                }
                mir::ReturnType::Value(mir::Type::Class(actual)) if actual == expected => Ok(
                    mir::NullableClassExpression::Class(mir::ClassExpression::Call {
                        class: expected,
                        function: signature.id,
                        args,
                        return_borrow: signature.return_borrow,
                    }),
                ),
                _ => Err(vec![unsupported(
                    *span,
                    "static method has another class return type",
                )]),
            }
        }
        _ => Ok(mir::NullableClassExpression::Class(lower_class_expression(
            expr, expected, transfer, context,
        )?)),
    }
}

fn lower_null_safe_property(
    object: &hir::Expr,
    property: &str,
    span: Span,
    context: &LoweringContext,
) -> DiagnosticResult<(mir::NullableClassExpression, PropertyId, mir::Type)> {
    let mir::Type::NullableClass(class) = context.expression_type(object)? else {
        return Err(vec![unsupported(
            object.span(),
            "null-safe receiver is not a nullable class",
        )]);
    };
    let property_info = context.property_info(class, property).ok_or_else(|| {
        vec![unsupported(
            span,
            format!("class#{} has no property `${property}`", class.0),
        )]
    })?;
    let ty = context.native_type_ref(&property_info.ty).ok_or_else(|| {
        vec![unsupported(
            span,
            format!("property `${property}` is not native-lowerable"),
        )]
    })?;
    Ok((
        lower_nullable_class_expression(object, class, false, context)?,
        property_info.id,
        ty,
    ))
}

fn lower_null_safe_method_call(
    object: &hir::Expr,
    method: &str,
    args: &[hir::Expr],
    span: Span,
    context: &LoweringContext,
) -> DiagnosticResult<(
    mir::NullableClassExpression,
    FunctionSignature,
    Vec<mir::Rvalue>,
)> {
    let mir::Type::NullableClass(class) = context.expression_type(object)? else {
        return Err(vec![unsupported(
            object.span(),
            "null-safe receiver is not a nullable class",
        )]);
    };
    let signature = context.lookup_method(class, method, span)?;
    if signature.receiver_mode.is_none() {
        return Err(vec![unsupported(
            span,
            "null-safe call requires an instance method",
        )]);
    }
    let args = lower_call_args_with_ownership(method, args, signature.clone(), span, context)?;
    Ok((
        lower_nullable_class_expression(object, class, false, context)?,
        signature,
        args,
    ))
}

fn lower_format_expression(
    args: &[hir::Expr],
    span: Span,
    context: &LoweringContext,
) -> DiagnosticResult<mir::FormatExpression> {
    let Some(hir::Expr::String {
        value,
        span: format_span,
    }) = args.first()
    else {
        return Err(vec![unsupported(
            span,
            "format must be a direct string literal",
        )]);
    };
    let pieces = format_string::parse(value, *format_span).map_err(|error| vec![error])?;
    let specs = pieces.iter().filter_map(|piece| match piece {
        FormatPiece::Argument { spec, .. } => Some(*spec),
        FormatPiece::Literal(_) => None,
    });
    let arguments = args[1..]
        .iter()
        .zip(specs)
        .map(|(argument, spec)| {
            if spec.conversion == FormatConversion::Display {
                let lowered = lower_display_string_expression(argument, context)?;
                if inferred_class_type(argument, context).is_some() {
                    Ok(mir::FormatArgument::ClassDisplay(lowered))
                } else {
                    Ok(mir::FormatArgument::String(lowered))
                }
            } else if is_string_local_initializer(argument, context) {
                lower_string_expression(argument, context).map(mir::FormatArgument::String)
            } else {
                lower_value_expression(argument, context).map(mir::FormatArgument::Value)
            }
        })
        .collect::<DiagnosticResult<Vec<_>>>()?;
    Ok(mir::FormatExpression { pieces, arguments })
}

fn lower_display_string_expression(
    expr: &hir::Expr,
    context: &LoweringContext,
) -> DiagnosticResult<mir::StringExpression> {
    let ty = context.expression_type(expr)?;
    if let mir::Type::Class(class) = ty {
        let class_info = context.class_info(class).ok_or_else(|| {
            vec![unsupported(
                expr.span(),
                format!("unknown native class#{}", class.0),
            )]
        })?;
        if !class_info.implements_displayable {
            return Err(vec![unsupported(
                expr.span(),
                format!(
                    "class `{}` does not implement `Displayable`",
                    class_info.name
                ),
            )]);
        }
        let (signature, args) =
            lower_instance_method_call(expr, "toString", &[], expr.span(), context)?;
        if signature.return_type != mir::ReturnType::Value(mir::Type::String) {
            return Err(vec![unsupported(
                expr.span(),
                "`Displayable::toString` does not return string",
            )]);
        }
        return Ok(mir::StringExpression::Call {
            function: signature.id,
            args,
        });
    }
    match ty {
        mir::Type::String => lower_string_expression(expr, context),
        mir::Type::Scalar(_) => {
            lower_value_expression(expr, context).map(mir::StringExpression::Display)
        }
        mir::Type::NullableScalar(_) | mir::Type::NullableString | mir::Type::NullableClass(_) => {
            Err(vec![unsupported(
                expr.span(),
                "nullable values must be narrowed or defaulted before display",
            )])
        }
        mir::Type::Class(_) => unreachable!("class display handled above"),
    }
}

fn append_string_concat_parts(
    expr: &hir::Expr,
    context: &LoweringContext,
    parts: &mut Vec<mir::StringExpression>,
) -> DiagnosticResult<()> {
    match expr {
        hir::Expr::Grouped { expr, .. } => append_string_concat_parts(expr, context, parts),
        hir::Expr::Binary {
            left,
            op: hir::BinaryOp::Concat,
            right,
            ..
        } => {
            append_string_concat_parts(left, context, parts)?;
            append_string_concat_parts(right, context, parts)
        }
        hir::Expr::String { value, .. } => {
            parts.push(mir::StringExpression::Literal(value.clone()));
            Ok(())
        }
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            if context.local_type(local) == mir::Type::String {
                parts.push(mir::StringExpression::Local(local));
            } else if context.local_type(local) == mir::Type::NullableString {
                parts.push(mir::StringExpression::NullableLocalAssumeNonNull(local));
            } else {
                parts.push(lower_display_string_expression(expr, context)?);
            }
            Ok(())
        }
        _ => {
            parts.push(lower_display_string_expression(expr, context)?);
            Ok(())
        }
    }
}

fn lower_statement_call(
    name: &str,
    args: &[hir::Expr],
    span: Span,
    context: &LoweringContext,
) -> DiagnosticResult<mir::Statement> {
    let signature = context.lookup_function(name, span)?;
    let args = lower_call_args(name, args, signature.clone(), span, context)?;
    discarded_call_statement("function", signature, args, span)
}

fn discarded_call_statement(
    kind: &str,
    signature: FunctionSignature,
    args: Vec<mir::Rvalue>,
    span: Span,
) -> DiagnosticResult<mir::Statement> {
    let statement = match signature.return_type {
        mir::ReturnType::Void => mir::Statement::CallVoid {
            function: signature.id,
            args,
        },
        mir::ReturnType::Value(mir::Type::Class(_) | mir::Type::NullableClass(_))
            if signature.return_borrow.is_some() =>
        {
            mir::Statement::CallBorrowed {
                function: signature.id,
                args,
            }
        }
        mir::ReturnType::Value(_) => {
            return Err(vec![unsupported(
                span,
                format!("non-void {kind} call cannot be used as a statement"),
            )]);
        }
    };
    Ok(statement)
}

fn discarded_null_safe_call_statement(
    object: mir::NullableClassExpression,
    signature: FunctionSignature,
    args: Vec<mir::Rvalue>,
    span: Span,
) -> DiagnosticResult<mir::Statement> {
    let supported = matches!(signature.return_type, mir::ReturnType::Void)
        || matches!(
            signature.return_type,
            mir::ReturnType::Value(mir::Type::Class(_) | mir::Type::NullableClass(_))
        ) && signature.return_borrow.is_some();
    if !supported {
        return Err(vec![unsupported(
            span,
            "non-void method call cannot be used as a statement",
        )]);
    }
    Ok(mir::Statement::CallNullSafe {
        object,
        function: signature.id,
        args,
    })
}

fn lower_integer_call(
    name: &str,
    args: &[hir::Expr],
    span: Span,
    context: &LoweringContext,
) -> DiagnosticResult<(mir::FunctionId, IntegerType, Vec<mir::Rvalue>)> {
    let signature = context.lookup_function(name, span)?;
    let mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Integer(return_type))) =
        signature.return_type
    else {
        return Err(vec![unsupported(
            span,
            format!("void function `{name}` cannot be used as an integer expression"),
        )]);
    };

    let function = signature.id;
    let args = lower_call_args(name, args, signature, span, context)?;
    Ok((function, return_type, args))
}

fn lower_call_args(
    name: &str,
    args: &[hir::Expr],
    signature: FunctionSignature,
    span: Span,
    context: &LoweringContext,
) -> DiagnosticResult<Vec<mir::Rvalue>> {
    lower_call_args_with_ownership(name, args, signature, span, context)
}

fn lower_call_args_with_ownership(
    name: &str,
    args: &[hir::Expr],
    signature: FunctionSignature,
    span: Span,
    context: &LoweringContext,
) -> DiagnosticResult<Vec<mir::Rvalue>> {
    let required = signature
        .parameter_defaults
        .iter()
        .filter(|default| default.is_none())
        .count();
    let total = signature.parameter_types.len();
    if args.len() < required || args.len() > total {
        return Err(vec![unsupported(
            span,
            format!(
                "function `{name}` expects {required}..={total} positional argument(s), got {}",
                args.len()
            ),
        )]);
    }

    let mut lowered_args = Vec::with_capacity(total);
    for (index, arg) in args.iter().enumerate() {
        let expected = signature.parameter_types[index];
        let transfers = signature.parameter_transfers[index];
        let lowered = match expected {
            mir::Type::Class(class) => {
                mir::Rvalue::Class(lower_class_expression(arg, class, transfers, context)?)
            }
            mir::Type::NullableClass(class) => mir::Rvalue::NullableClass(
                lower_nullable_class_expression(arg, class, transfers, context)?,
            ),
            _ => lower_rvalue_as_expected(arg, expected, context)?,
        };
        if lowered.ty() != expected {
            return Err(vec![Diagnostic::new(
                "I1301",
                format!(
                    "internal compiler consistency error: argument to `{name}` has MIR type `{}`, expected `{expected}`",
                    lowered.ty()
                ),
                arg.span(),
            )]);
        }
        lowered_args.push(lowered);
    }

    append_omitted_trailing_defaults(name, args.len(), &signature, span, &mut lowered_args)?;
    Ok(lowered_args)
}

fn append_omitted_trailing_defaults(
    name: &str,
    supplied: usize,
    signature: &FunctionSignature,
    span: Span,
    args: &mut Vec<mir::Rvalue>,
) -> DiagnosticResult<()> {
    for index in supplied..signature.parameter_types.len() {
        let value = signature.parameter_defaults[index]
            .as_ref()
            .ok_or_else(|| {
                vec![Diagnostic::new(
                    "I2002",
                    format!(
                        "required parameter {} of `{name}` was omitted after semantic checking",
                        index + 1
                    ),
                    span,
                )]
            })?;
        args.push(lower_const_parameter_default(
            value,
            signature.parameter_types[index],
            span,
        )?);
    }
    Ok(())
}

fn lower_const_parameter_default(
    value: &crate::const_eval::ConstValue,
    expected: mir::Type,
    span: Span,
) -> DiagnosticResult<mir::Rvalue> {
    if let (crate::const_eval::ConstValue::String(value), mir::Type::String) = (value, expected) {
        return Ok(mir::Rvalue::String(mir::StringExpression::Literal(
            value.clone(),
        )));
    }

    let value = match (value, expected) {
        (
            crate::const_eval::ConstValue::Integer(value),
            mir::Type::Scalar(mir::ScalarType::Integer(integer)),
        ) if value.ty == integer => {
            mir::ValueExpression::Integer(mir::IntegerExpression::constant(*value))
        }
        (
            crate::const_eval::ConstValue::Float(value),
            mir::Type::Scalar(mir::ScalarType::Float(float)),
        ) if value.ty == float => {
            mir::ValueExpression::Float(mir::FloatExpression::constant(*value))
        }
        (crate::const_eval::ConstValue::Bool(value), mir::Type::Scalar(mir::ScalarType::Bool)) => {
            mir::ValueExpression::Bool(mir::BoolExpression::Use {
                operand: mir::Operand::Scalar(mir::ScalarValue::Bool(*value)),
            })
        }
        _ => {
            return Err(vec![Diagnostic::new(
                "I2003",
                "checked parameter default does not match its MIR parameter type",
                span,
            )]);
        }
    };
    Ok(mir::Rvalue::Value(value))
}

fn lower_instance_method_call(
    object: &hir::Expr,
    method: &str,
    args: &[hir::Expr],
    span: Span,
    context: &LoweringContext,
) -> DiagnosticResult<(FunctionSignature, Vec<mir::Rvalue>)> {
    let class = inferred_class_type(object, context).ok_or_else(|| {
        vec![unsupported(
            object.span(),
            "method receiver does not have a concrete native class type",
        )]
    })?;
    let signature = context.lookup_method(class, method, span)?;
    if signature.receiver_mode.is_none() {
        return Err(vec![unsupported(
            span,
            format!(
                "static method `class#{}::{method}` has no receiver",
                class.0
            ),
        )]);
    }
    let mut lowered =
        lower_call_args_with_ownership(method, args, signature.clone(), span, context)?;
    lowered.insert(
        0,
        mir::Rvalue::Class(lower_class_expression(object, class, false, context)?),
    );
    Ok((signature, lowered))
}

fn lower_static_method_call(
    class_name: &str,
    method: &str,
    args: &[hir::Expr],
    span: Span,
    context: &LoweringContext,
) -> DiagnosticResult<(FunctionSignature, Vec<mir::Rvalue>)> {
    let class = context
        .class_id_for_name(class_name)
        .ok_or_else(|| vec![unsupported(span, format!("unknown class `{class_name}`"))])?;
    let signature = context.lookup_method(class, method, span)?;
    if signature.receiver_mode.is_some() {
        return Err(vec![unsupported(
            span,
            format!("instance method `{class_name}::{method}` requires a receiver"),
        )]);
    }
    let lowered = lower_call_args_with_ownership(method, args, signature.clone(), span, context)?;
    Ok((signature, lowered))
}

fn lower_return(
    expr: Option<&hir::Expr>,
    span: Span,
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::Terminator> {
    match (return_type, expr) {
        (mir::ReturnType::Void, None) => {
            context.cleanup_scopes_from(0);
            Ok(mir::Terminator::ReturnVoid)
        }
        (mir::ReturnType::Value(expected), Some(expr)) => {
            let borrowed_class =
                matches!(expected, mir::Type::Class(_) | mir::Type::NullableClass(_))
                    && context.return_borrow.is_some();
            let value = match expected {
                mir::Type::Class(class) => {
                    lower_class_expression(expr, class, !borrowed_class, context)
                        .map(mir::Rvalue::Class)?
                }
                mir::Type::NullableClass(class) => {
                    lower_nullable_class_expression(expr, class, !borrowed_class, context)
                        .map(mir::Rvalue::NullableClass)?
                }
                _ => lower_rvalue_as_expected(expr, expected, context)?,
            };
            if value.ty() != expected {
                return Err(vec![Diagnostic::new(
                    "I1301",
                    format!(
                        "internal compiler consistency error: return expression has MIR type `{}`, expected `{expected}`",
                        value.ty()
                    ),
                    expr.span(),
                )]);
            }
            if context.has_cleanup_obligations() {
                let result = context.declare_return_temp(
                    expected,
                    matches!(expected, mir::Type::Class(_) | mir::Type::NullableClass(_))
                        && !borrowed_class,
                );
                context.push_statement(mir::Statement::AssignLocal {
                    target: result,
                    value,
                });
                context.cleanup_scopes_from(0);
                Ok(mir::Terminator::Return(local_rvalue(
                    result,
                    expected,
                    !borrowed_class,
                )))
            } else {
                Ok(mir::Terminator::Return(value))
            }
        }
        (mir::ReturnType::Value(_), None) => Err(vec![unsupported(
            span,
            "a value-returning function cannot use a bare `return`",
        )]),
        (mir::ReturnType::Void, Some(expr)) => Err(vec![unsupported(
            expr.span(),
            "a `void` function cannot return a value",
        )]),
    }
}

fn local_rvalue(local: mir::LocalId, ty: mir::Type, transfer: bool) -> mir::Rvalue {
    match ty {
        mir::Type::Scalar(mir::ScalarType::Integer(ty)) => mir::Rvalue::Value(
            mir::ValueExpression::Integer(local_integer_expression(local, ty)),
        ),
        mir::Type::Scalar(mir::ScalarType::Float(ty)) => mir::Rvalue::Value(
            mir::ValueExpression::Float(local_float_expression(local, ty)),
        ),
        mir::Type::Scalar(mir::ScalarType::Bool) => {
            mir::Rvalue::Value(mir::ValueExpression::Bool(mir::BoolExpression::Use {
                operand: mir::Operand::Local(local),
            }))
        }
        mir::Type::String => mir::Rvalue::String(mir::StringExpression::Local(local)),
        mir::Type::NullableScalar(ty) => {
            mir::Rvalue::NullableScalar(mir::NullableScalarExpression::Local { ty, local })
        }
        mir::Type::NullableString => {
            mir::Rvalue::NullableString(mir::NullableStringExpression::Local(local))
        }
        mir::Type::Class(class) => mir::Rvalue::Class(mir::ClassExpression::Local {
            class,
            local,
            transfer,
        }),
        mir::Type::NullableClass(class) => {
            mir::Rvalue::NullableClass(mir::NullableClassExpression::Local {
                class,
                local,
                transfer,
            })
        }
    }
}

fn lower_condition(
    expr: &hir::Expr,
    context: &LoweringContext,
) -> DiagnosticResult<mir::BoolExpression> {
    if let Some(crate::const_eval::ConstValue::Bool(value)) = context.constant_value(expr) {
        return Ok(mir::BoolExpression::Use {
            operand: mir::Operand::Scalar(mir::ScalarValue::Bool(*value)),
        });
    }
    match expr {
        hir::Expr::Binary {
            left,
            op: hir::BinaryOp::Coalesce,
            right,
            ..
        } => match context.coalesce_selection(left) {
            CoalesceSelection::Left => lower_condition(left, context),
            CoalesceSelection::Right => lower_condition(right, context),
            CoalesceSelection::Dynamic => Ok(mir::BoolExpression::Coalesce {
                left: Box::new(lower_nullable_scalar_expression(
                    left,
                    mir::ScalarType::Bool,
                    context,
                )?),
                right: Box::new(lower_condition(right, context)?),
            }),
        },
        hir::Expr::IsType { expr, span, .. } => lower_is_condition(expr, *span, context),
        hir::Expr::Bool { value, .. } => Ok(mir::BoolExpression::Use {
            operand: mir::Operand::Scalar(mir::ScalarValue::Bool(*value)),
        }),
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            match context.local_type(local) {
                mir::Type::Scalar(mir::ScalarType::Bool) => Ok(mir::BoolExpression::Use {
                    operand: mir::Operand::Local(local),
                }),
                mir::Type::NullableScalar(mir::ScalarType::Bool) => Ok(mir::BoolExpression::Use {
                    operand: mir::Operand::NullablePayload(local),
                }),
                _ => Err(vec![unsupported(
                    *span,
                    "only bool locals may be used as conditions",
                )]),
            }
        }
        hir::Expr::Grouped { expr, .. } => lower_condition(expr, context),
        hir::Expr::PropertyAccess { .. } => {
            let (object, property) =
                lower_property_operand(expr, mir::Type::Scalar(mir::ScalarType::Bool), context)?;
            Ok(mir::BoolExpression::Use {
                operand: mir::Operand::Property { object, property },
            })
        }
        hir::Expr::StaticMember {
            class_name,
            member,
            span,
        } => {
            let (id, ty) = context.static_property(class_name, member, *span)?;
            if ty != mir::Type::Scalar(mir::ScalarType::Bool) {
                return Err(vec![unsupported(*span, "static property is not bool")]);
            }
            Ok(mir::BoolExpression::Use {
                operand: mir::Operand::Static(id),
            })
        }
        hir::Expr::Unary {
            op: hir::UnaryOp::Not,
            expr,
            ..
        } => Ok(mir::BoolExpression::Not(Box::new(lower_condition(
            expr, context,
        )?))),
        hir::Expr::Binary {
            left, op, right, ..
        } => match op {
            hir::BinaryOp::Equal
            | hir::BinaryOp::NotEqual
            | hir::BinaryOp::Less
            | hir::BinaryOp::LessEqual
            | hir::BinaryOp::Greater
            | hir::BinaryOp::GreaterEqual => {
                if matches!(unparenthesized_place(left), hir::Expr::Null { .. })
                    || matches!(unparenthesized_place(right), hir::Expr::Null { .. })
                {
                    lower_null_comparison(left, op, right, context)
                } else if is_nullable_string_expression(left, context)
                    || is_nullable_string_expression(right, context)
                {
                    Ok(mir::BoolExpression::NullableStringCompare {
                        op: lower_compare_op(op),
                        left: Box::new(lower_nullable_string_expression(left, context)?),
                        right: Box::new(lower_nullable_string_expression(right, context)?),
                    })
                } else if is_string_local_initializer(left, context)
                    || is_string_local_initializer(right, context)
                {
                    Ok(mir::BoolExpression::StringCompare {
                        op: lower_compare_op(op),
                        left: Box::new(lower_string_expression(left, context)?),
                        right: Box::new(lower_string_expression(right, context)?),
                    })
                } else {
                    Ok(mir::BoolExpression::Compare {
                        op: lower_compare_op(op),
                        left: Box::new(lower_value_expression(left, context)?),
                        right: Box::new(lower_value_expression(right, context)?),
                    })
                }
            }
            hir::BinaryOp::And | hir::BinaryOp::Or | hir::BinaryOp::Xor => {
                Ok(mir::BoolExpression::Binary {
                    op: lower_condition_binary_op(op),
                    left: Box::new(lower_condition(left, context)?),
                    right: Box::new(lower_condition(right, context)?),
                })
            }
            _ => Err(vec![unsupported(
                expr.span(),
                "conditions require boolean values, scalar comparisons, or boolean operators",
            )]),
        },
        hir::Expr::FunctionCall { name, args, span } => {
            let signature = context.lookup_function(name, *span)?;
            if signature.return_type
                != mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Bool))
            {
                return Err(vec![unsupported(
                    *span,
                    format!("function `{name}` does not return bool"),
                )]);
            }
            Ok(mir::BoolExpression::Call {
                function: signature.id,
                args: lower_call_args(name, args, signature, *span, context)?,
            })
        }
        hir::Expr::MethodCall {
            object,
            method,
            args,
            span,
            ..
        } => {
            let (signature, args) =
                lower_instance_method_call(object, method, args, *span, context)?;
            if signature.return_type
                != mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Bool))
            {
                return Err(vec![unsupported(*span, "method does not return bool")]);
            }
            Ok(mir::BoolExpression::Call {
                function: signature.id,
                args,
            })
        }
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } => {
            let (signature, args) =
                lower_static_method_call(class_name, method, args, *span, context)?;
            if signature.return_type
                != mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Bool))
            {
                return Err(vec![unsupported(
                    *span,
                    "static method does not return bool",
                )]);
            }
            Ok(mir::BoolExpression::Call {
                function: signature.id,
                args,
            })
        }
        hir::Expr::Int { .. } => Err(vec![unsupported(
            expr.span(),
            "integer truthiness is not supported; conditions require a `bool` value",
        )]),
        _ => Err(vec![unsupported(
            expr.span(),
            "this expression cannot be used as a condition in native compilation",
        )]),
    }
}

fn lower_null_comparison(
    left: &hir::Expr,
    op: &hir::BinaryOp,
    right: &hir::Expr,
    context: &LoweringContext,
) -> DiagnosticResult<mir::BoolExpression> {
    let value = if matches!(unparenthesized_place(left), hir::Expr::Null { .. }) {
        right
    } else {
        left
    };
    let present = match context.expression_type(value)? {
        mir::Type::NullableScalar(ty) => mir::BoolExpression::NullableScalarIsPresent(Box::new(
            lower_nullable_scalar_expression(value, ty, context)?,
        )),
        mir::Type::NullableString => {
            return Ok(mir::BoolExpression::NullableStringCompare {
                op: lower_compare_op(op),
                left: Box::new(lower_nullable_string_expression(left, context)?),
                right: Box::new(lower_nullable_string_expression(right, context)?),
            });
        }
        mir::Type::NullableClass(class) => mir::BoolExpression::NullableClassIsPresent(Box::new(
            lower_nullable_class_expression(value, class, false, context)?,
        )),
        _ => {
            return Err(vec![unsupported(
                value.span(),
                "null comparison requires a nullable value",
            )]);
        }
    };
    Ok(if matches!(op, hir::BinaryOp::Equal) {
        mir::BoolExpression::Not(Box::new(present))
    } else {
        present
    })
}

fn lower_is_condition(
    expr: &hir::Expr,
    type_test_span: Span,
    context: &LoweringContext,
) -> DiagnosticResult<mir::BoolExpression> {
    let tested_type = context
        .semantic_info
        .type_test_type(type_test_span)
        .and_then(|resolved| context.mir_resolved_type(resolved));
    let Some(tested_type) = tested_type else {
        return Err(vec![unsupported(
            expr.span(),
            "type test does not name a native concrete type",
        )]);
    };
    if context.expression_is_null(expr) {
        return Ok(mir::BoolExpression::Use {
            operand: mir::Operand::Scalar(mir::ScalarValue::Bool(false)),
        });
    }
    let value_type = context.expression_type(expr)?;
    let result = match value_type {
        mir::Type::NullableScalar(ty) if tested_type == mir::Type::Scalar(ty) => {
            mir::BoolExpression::NullableScalarIsPresent(Box::new(
                lower_nullable_scalar_expression(expr, ty, context)?,
            ))
        }
        mir::Type::NullableString if tested_type == mir::Type::String => {
            mir::BoolExpression::Not(Box::new(mir::BoolExpression::NullableStringCompare {
                op: mir::CompareOp::Equal,
                left: Box::new(lower_nullable_string_expression(expr, context)?),
                right: Box::new(mir::NullableStringExpression::Null),
            }))
        }
        mir::Type::NullableClass(class) if tested_type == mir::Type::Class(class) => {
            mir::BoolExpression::NullableClassIsPresent(Box::new(lower_nullable_class_expression(
                expr, class, false, context,
            )?))
        }
        mir::Type::Scalar(_) | mir::Type::String | mir::Type::Class(_) => {
            let evaluated = lower_concrete_is_presence(expr, value_type, context)?;
            if value_type == tested_type {
                evaluated
            } else {
                mir::BoolExpression::Not(Box::new(evaluated))
            }
        }
        mir::Type::NullableScalar(ty) => {
            evaluate_then_false(mir::BoolExpression::NullableScalarIsPresent(Box::new(
                lower_nullable_scalar_expression(expr, ty, context)?,
            )))
        }
        mir::Type::NullableString => evaluate_then_false(mir::BoolExpression::Not(Box::new(
            mir::BoolExpression::NullableStringCompare {
                op: mir::CompareOp::Equal,
                left: Box::new(lower_nullable_string_expression(expr, context)?),
                right: Box::new(mir::NullableStringExpression::Null),
            },
        ))),
        mir::Type::NullableClass(class) => {
            evaluate_then_false(mir::BoolExpression::NullableClassIsPresent(Box::new(
                lower_nullable_class_expression(expr, class, false, context)?,
            )))
        }
    };
    Ok(result)
}

fn evaluate_then_false(condition: mir::BoolExpression) -> mir::BoolExpression {
    mir::BoolExpression::Binary {
        op: mir::BoolBinaryOp::And,
        left: Box::new(condition),
        right: Box::new(mir::BoolExpression::Use {
            operand: mir::Operand::Scalar(mir::ScalarValue::Bool(false)),
        }),
    }
}

fn lower_concrete_is_presence(
    expr: &hir::Expr,
    value_type: mir::Type,
    context: &LoweringContext,
) -> DiagnosticResult<mir::BoolExpression> {
    match value_type {
        mir::Type::Scalar(ty) => Ok(mir::BoolExpression::NullableScalarIsPresent(Box::new(
            mir::NullableScalarExpression::Value({
                let value = lower_value_expression(expr, context)?;
                ensure_value_type(&value, ty, expr.span())?;
                value
            }),
        ))),
        mir::Type::String => Ok(mir::BoolExpression::Not(Box::new(
            mir::BoolExpression::NullableStringCompare {
                op: mir::CompareOp::Equal,
                left: Box::new(mir::NullableStringExpression::String(
                    lower_string_expression(expr, context)?,
                )),
                right: Box::new(mir::NullableStringExpression::Null),
            },
        ))),
        mir::Type::Class(class) => Ok(mir::BoolExpression::NullableClassIsPresent(Box::new(
            mir::NullableClassExpression::Class(lower_class_expression(
                expr, class, false, context,
            )?),
        ))),
        mir::Type::NullableScalar(_) | mir::Type::NullableString | mir::Type::NullableClass(_) => {
            unreachable!("concrete `is` value type")
        }
    }
}

fn lower_compare_op(op: &hir::BinaryOp) -> mir::CompareOp {
    match op {
        hir::BinaryOp::Equal => mir::CompareOp::Equal,
        hir::BinaryOp::NotEqual => mir::CompareOp::NotEqual,
        hir::BinaryOp::Less => mir::CompareOp::Less,
        hir::BinaryOp::LessEqual => mir::CompareOp::LessEqual,
        hir::BinaryOp::Greater => mir::CompareOp::Greater,
        hir::BinaryOp::GreaterEqual => mir::CompareOp::GreaterEqual,
        _ => unreachable!("only comparison operators are lowered as MIR comparisons"),
    }
}

fn lower_condition_binary_op(op: &hir::BinaryOp) -> mir::BoolBinaryOp {
    match op {
        hir::BinaryOp::And => mir::BoolBinaryOp::And,
        hir::BinaryOp::Or => mir::BoolBinaryOp::Or,
        hir::BinaryOp::Xor => mir::BoolBinaryOp::Xor,
        _ => unreachable!("only boolean operators are lowered as MIR condition operators"),
    }
}

fn lower_value_expression(
    expr: &hir::Expr,
    context: &LoweringContext,
) -> DiagnosticResult<mir::ValueExpression> {
    if let hir::Expr::FunctionCall { name, span, .. } = expr {
        if context.lookup_function(name, *span)?.return_type == mir::ReturnType::Void {
            return Err(vec![unsupported(
                *span,
                format!("void function `{name}` cannot be used as a scalar expression"),
            )]);
        }
    }
    if context.semantic_info.integer_type(expr.span()).is_some() {
        return lower_integer_expression(expr, context).map(mir::ValueExpression::Integer);
    }
    if context.semantic_info.float_type(expr.span()).is_some() {
        return lower_float_expression(expr, context).map(mir::ValueExpression::Float);
    }
    lower_condition(expr, context).map(mir::ValueExpression::Bool)
}

fn call_value_expression(
    ty: mir::ScalarType,
    function: mir::FunctionId,
    args: Vec<mir::Rvalue>,
) -> mir::ValueExpression {
    match ty {
        mir::ScalarType::Integer(ty) => {
            mir::ValueExpression::Integer(mir::IntegerExpression::Call { ty, function, args })
        }
        mir::ScalarType::Float(ty) => {
            mir::ValueExpression::Float(mir::FloatExpression::Call { ty, function, args })
        }
        mir::ScalarType::Bool => {
            mir::ValueExpression::Bool(mir::BoolExpression::Call { function, args })
        }
    }
}

fn value_expression_from_operand(
    ty: mir::ScalarType,
    operand: mir::Operand,
) -> mir::ValueExpression {
    match ty {
        mir::ScalarType::Integer(ty) => {
            mir::ValueExpression::Integer(mir::IntegerExpression::Use { ty, operand })
        }
        mir::ScalarType::Float(ty) => {
            mir::ValueExpression::Float(mir::FloatExpression::Use { ty, operand })
        }
        mir::ScalarType::Bool => mir::ValueExpression::Bool(mir::BoolExpression::Use { operand }),
    }
}

fn lower_rvalue_as_expected(
    expr: &hir::Expr,
    expected: mir::Type,
    context: &LoweringContext,
) -> DiagnosticResult<mir::Rvalue> {
    match expected {
        mir::Type::String => lower_string_expression(expr, context).map(mir::Rvalue::String),
        mir::Type::NullableScalar(ty) => {
            lower_nullable_scalar_expression(expr, ty, context).map(mir::Rvalue::NullableScalar)
        }
        mir::Type::NullableString => {
            lower_nullable_string_expression(expr, context).map(mir::Rvalue::NullableString)
        }
        mir::Type::Scalar(_) => lower_value_expression(expr, context).map(mir::Rvalue::Value),
        mir::Type::Class(class) => {
            lower_class_expression(expr, class, true, context).map(mir::Rvalue::Class)
        }
        mir::Type::NullableClass(class) => {
            lower_nullable_class_expression(expr, class, true, context)
                .map(mir::Rvalue::NullableClass)
        }
    }
}

fn lower_class_expression(
    expr: &hir::Expr,
    expected: ClassId,
    transfer: bool,
    context: &LoweringContext,
) -> DiagnosticResult<mir::ClassExpression> {
    match expr {
        hir::Expr::Grouped { expr, .. } => {
            lower_class_expression(expr, expected, transfer, context)
        }
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            match context.local_type(local) {
                mir::Type::Class(class) if class == expected => {
                    if transfer && !context.local_owns(local) {
                        return Err(vec![unsupported(
                            *span,
                            format!("borrowed class local `${name}` cannot be given away"),
                        )]);
                    }
                    Ok(mir::ClassExpression::Local {
                        class: expected,
                        local,
                        transfer,
                    })
                }
                mir::Type::NullableClass(class) if class == expected => {
                    if transfer && !context.local_owns(local) {
                        return Err(vec![unsupported(
                            *span,
                            format!("borrowed nullable class local `${name}` cannot be given away"),
                        )]);
                    }
                    Ok(mir::ClassExpression::NullableLocalAssumeNonNull {
                        class: expected,
                        local,
                        transfer,
                    })
                }
                _ => Err(vec![unsupported(
                    *span,
                    format!("local `${name}` does not have the expected class type"),
                )]),
            }
        }
        hir::Expr::This { span } => {
            if transfer {
                return Err(vec![unsupported(*span, "`$this` cannot be given away")]);
            }
            let local = context.lookup_local("this", *span)?;
            if context.local_type(local) != mir::Type::Class(expected) {
                return Err(vec![unsupported(
                    *span,
                    "`$this` does not have the expected class type",
                )]);
            }
            Ok(mir::ClassExpression::Local {
                class: expected,
                local,
                transfer: false,
            })
        }
        hir::Expr::PropertyAccess { span, .. } => {
            if transfer {
                return Err(vec![unsupported(
                    *span,
                    "moving directly out of an owned class property is not supported",
                )]);
            }
            let (object, property, property_type) = lower_property_place(expr, context)?;
            if property_type != mir::Type::Class(expected) {
                return Err(vec![unsupported(
                    *span,
                    "class property does not have the expected class type",
                )]);
            }
            Ok(mir::ClassExpression::Property {
                class: expected,
                object,
                property,
            })
        }
        hir::Expr::New {
            class_name,
            args,
            span,
        } => {
            let class = context
                .class_id_for_name(class_name)
                .ok_or_else(|| vec![unsupported(*span, format!("unknown class `{class_name}`"))])?;
            if class != expected {
                return Err(vec![unsupported(
                    *span,
                    format!("constructor for `{class_name}` does not produce expected class"),
                )]);
            }
            let constructor = context.lookup_lifecycle(class, "__construct");
            let constructor_args = if let Some(signature) = constructor.as_ref() {
                lower_call_args_with_ownership(class_name, args, signature.clone(), *span, context)?
            } else {
                if !args.is_empty() {
                    return Err(vec![unsupported(
                        *span,
                        format!("class `{class_name}` does not declare a constructor"),
                    )]);
                }
                Vec::new()
            };
            let properties = lower_new_property_values(class, context)?;
            Ok(mir::ClassExpression::New {
                class,
                properties,
                constructor: constructor.map(|signature| signature.id),
                args: constructor_args,
            })
        }
        hir::Expr::FunctionCall { name, args, span } => {
            let signature = context.lookup_function(name, *span)?;
            if signature.return_type != mir::ReturnType::Value(mir::Type::Class(expected)) {
                return Err(vec![unsupported(
                    *span,
                    format!("function `{name}` does not return the expected class"),
                )]);
            }
            Ok(mir::ClassExpression::Call {
                class: expected,
                function: signature.id,
                return_borrow: signature.return_borrow,
                args: lower_call_args_with_ownership(name, args, signature, *span, context)?,
            })
        }
        hir::Expr::MethodCall {
            object,
            method,
            args,
            span,
            ..
        } => {
            let (signature, args) =
                lower_instance_method_call(object, method, args, *span, context)?;
            if signature.return_type != mir::ReturnType::Value(mir::Type::Class(expected)) {
                return Err(vec![unsupported(
                    *span,
                    "method does not return expected class",
                )]);
            }
            Ok(mir::ClassExpression::Call {
                class: expected,
                function: signature.id,
                return_borrow: signature.return_borrow,
                args,
            })
        }
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } => {
            let (signature, args) =
                lower_static_method_call(class_name, method, args, *span, context)?;
            if signature.return_type != mir::ReturnType::Value(mir::Type::Class(expected)) {
                return Err(vec![unsupported(
                    *span,
                    "static method does not return expected class",
                )]);
            }
            Ok(mir::ClassExpression::Call {
                class: expected,
                function: signature.id,
                return_borrow: signature.return_borrow,
                args,
            })
        }
        hir::Expr::Binary {
            left,
            op: hir::BinaryOp::Coalesce,
            right,
            ..
        } => match context.coalesce_selection(left) {
            CoalesceSelection::Left => lower_class_expression(left, expected, transfer, context),
            CoalesceSelection::Right => lower_class_expression(right, expected, transfer, context),
            CoalesceSelection::Dynamic => Ok(mir::ClassExpression::Coalesce {
                class: expected,
                left: Box::new(lower_nullable_class_expression(
                    left, expected, transfer, context,
                )?),
                right: Box::new(lower_class_expression(right, expected, transfer, context)?),
                transfer,
            }),
        },
        _ => Err(vec![unsupported(
            expr.span(),
            "this class expression is not supported by native compilation",
        )]),
    }
}

fn lower_property_operand(
    expr: &hir::Expr,
    expected: mir::Type,
    context: &LoweringContext,
) -> DiagnosticResult<(mir::LocalId, crate::class_layout::PropertyId)> {
    let (object, property, property_type) = lower_property_place(expr, context)?;
    if property_type != expected {
        return Err(vec![unsupported(
            expr.span(),
            format!("property has MIR type `{property_type}`, expected `{expected}`"),
        )]);
    }
    Ok((object, property))
}

fn lower_property_place(
    expr: &hir::Expr,
    context: &LoweringContext,
) -> DiagnosticResult<(mir::LocalId, crate::class_layout::PropertyId, mir::Type)> {
    let expr = unparenthesized_place(expr);
    let hir::Expr::PropertyAccess {
        object,
        property,
        span,
        ..
    } = expr
    else {
        return Err(vec![unsupported(
            expr.span(),
            "expected class property access",
        )]);
    };
    let object_local = match unparenthesized_place(object) {
        hir::Expr::Variable { name, span } => context.lookup_local(name, *span)?,
        hir::Expr::This { span } => context.lookup_local("this", *span)?,
        _ => {
            return Err(vec![unsupported(
                object.span(),
                "native class property access requires a local object path in Stage 19",
            )])
        }
    };
    let class = match context.local_type(object_local) {
        mir::Type::Class(class) | mir::Type::NullableClass(class) => class,
        _ => {
            return Err(vec![unsupported(
                object.span(),
                "property access object is not a native class value",
            )])
        }
    };
    let property_info = context.property_info(class, property).ok_or_else(|| {
        vec![unsupported(
            *span,
            format!("class#{} has no property `${property}`", class.0),
        )]
    })?;
    let property_type = context.native_type_ref(&property_info.ty).ok_or_else(|| {
        vec![unsupported(
            *span,
            format!("property `${property}` is not native-lowerable"),
        )]
    })?;
    Ok((object_local, property_info.id, property_type))
}

fn lower_new_property_values(
    class: ClassId,
    context: &LoweringContext,
) -> DiagnosticResult<Vec<mir::PropertyValue>> {
    let info = context.class_info(class).ok_or_else(|| {
        vec![unsupported(
            Span::default(),
            format!("unknown class#{}", class.0),
        )]
    })?;
    info.properties
        .iter()
        .map(|property| {
            if property.promoted {
                let index = promoted_constructor_argument_index(class, &property.name, context)
                    .ok_or_else(|| {
                        vec![unsupported(
                            Span::default(),
                            format!(
                                "promoted property `${}` has no constructor argument",
                                property.name
                            ),
                        )]
                    })?;
                return Ok(mir::PropertyValue {
                    property: property.id,
                    source: mir::PropertyValueSource::ConstructorArgument(index),
                });
            }
            if let Some(initializer) = context.property_initializers.get(&property.id) {
                let property_type = context.native_type_ref(&property.ty).ok_or_else(|| {
                    vec![unsupported(
                        initializer.span(),
                        format!("property `${}` is not native-lowerable", property.name),
                    )]
                })?;
                return Ok(mir::PropertyValue {
                    property: property.id,
                    source: mir::PropertyValueSource::Expression(lower_rvalue_as_expected(
                        initializer,
                        property_type,
                        context,
                    )?),
                });
            }
            if context
                .constructor_body_initializers
                .contains(&property.id)
            {
                return Ok(mir::PropertyValue {
                    property: property.id,
                    source: mir::PropertyValueSource::ConstructorBody,
                });
            }
            Err(vec![unsupported(
                Span::default(),
                format!(
                    "class property `${}` is not definitely initialized before construction completes",
                    property.name
                ),
            )])
        })
        .collect()
}

fn promoted_constructor_argument_index(
    class: ClassId,
    property_name: &str,
    context: &LoweringContext,
) -> Option<usize> {
    let constructor = context.lookup_lifecycle(class, "__construct")?;
    let class_info = context.class_info(class)?;
    class_info
        .properties
        .iter()
        .filter(|property| property.promoted)
        .position(|property| property.name == property_name)
        .filter(|index| *index < constructor.parameter_types.len())
}

fn lower_float_expression(
    expr: &hir::Expr,
    context: &LoweringContext,
) -> DiagnosticResult<mir::FloatExpression> {
    if let Some(crate::const_eval::ConstValue::Float(value)) = context.constant_value(expr) {
        return Ok(mir::FloatExpression::constant(*value));
    }
    let ty = context.float_type(expr)?;
    match expr {
        hir::Expr::Binary {
            left,
            op: hir::BinaryOp::Coalesce,
            right,
            ..
        } => match context.coalesce_selection(left) {
            CoalesceSelection::Left => lower_float_expression(left, context),
            CoalesceSelection::Right => lower_float_expression(right, context),
            CoalesceSelection::Dynamic => Ok(mir::FloatExpression::Coalesce {
                ty,
                left: Box::new(lower_nullable_scalar_expression(
                    left,
                    mir::ScalarType::Float(ty),
                    context,
                )?),
                right: Box::new(lower_float_expression(right, context)?),
            }),
        },
        hir::Expr::Float { value, .. } => FloatValue::parse_decimal(ty, value)
            .map(mir::FloatExpression::constant)
            .ok_or_else(|| {
                vec![Diagnostic::new(
                    "I1401",
                    format!("checked floating literal does not fit `{ty}`"),
                    expr.span(),
                )]
            }),
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            match context.local_type(local) {
                mir::Type::Scalar(mir::ScalarType::Float(local_ty)) if local_ty == ty => {
                    Ok(local_float_expression(local, ty))
                }
                mir::Type::NullableScalar(mir::ScalarType::Float(local_ty)) if local_ty == ty => {
                    Ok(mir::FloatExpression::Use {
                        ty,
                        operand: mir::Operand::NullablePayload(local),
                    })
                }
                _ => Err(vec![Diagnostic::new(
                    "I1401",
                    format!("float local `${name}` does not have expected MIR type `{ty}`"),
                    *span,
                )]),
            }
        }
        hir::Expr::PropertyAccess { .. } => {
            let (object, property) = lower_property_operand(
                expr,
                mir::Type::Scalar(mir::ScalarType::Float(ty)),
                context,
            )?;
            Ok(mir::FloatExpression::Use {
                ty,
                operand: mir::Operand::Property { object, property },
            })
        }
        hir::Expr::StaticMember {
            class_name,
            member,
            span,
        } => {
            let (id, static_ty) = context.static_property(class_name, member, *span)?;
            if static_ty != mir::Type::Scalar(mir::ScalarType::Float(ty)) {
                return Err(vec![unsupported(
                    *span,
                    "static property has another float type",
                )]);
            }
            Ok(mir::FloatExpression::Use {
                ty,
                operand: mir::Operand::Static(id),
            })
        }
        hir::Expr::Grouped { expr, .. } => lower_float_expression(expr, context),
        hir::Expr::Unary {
            op: hir::UnaryOp::Negate,
            expr,
            ..
        } => Ok(mir::FloatExpression::Negate {
            ty,
            operand: Box::new(lower_float_expression(expr, context)?),
        }),
        hir::Expr::Binary {
            left, op, right, ..
        } => Ok(mir::FloatExpression::Binary {
            ty,
            op: match op {
                hir::BinaryOp::Add => mir::FloatBinaryOp::Add,
                hir::BinaryOp::Sub => mir::FloatBinaryOp::Subtract,
                hir::BinaryOp::Mul => mir::FloatBinaryOp::Multiply,
                hir::BinaryOp::Div => mir::FloatBinaryOp::Divide,
                _ => return Err(vec![unsupported(expr.span(), "invalid float operator")]),
            },
            left: Box::new(lower_float_expression(left, context)?),
            right: Box::new(lower_float_expression(right, context)?),
        }),
        hir::Expr::FunctionCall { name, args, span } => {
            let signature = context.lookup_function(name, *span)?;
            if signature.return_type
                != mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Float(ty)))
            {
                return Err(vec![Diagnostic::new(
                    "I1401",
                    format!("function `{name}` does not return `{ty}`"),
                    *span,
                )]);
            }
            Ok(mir::FloatExpression::Call {
                ty,
                function: signature.id,
                args: lower_call_args(name, args, signature, *span, context)?,
            })
        }
        hir::Expr::MethodCall {
            object,
            method,
            args,
            span,
            ..
        } => {
            let (signature, args) =
                lower_instance_method_call(object, method, args, *span, context)?;
            if signature.return_type
                != mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Float(ty)))
            {
                return Err(vec![unsupported(
                    *span,
                    "method does not return expected float",
                )]);
            }
            Ok(mir::FloatExpression::Call {
                ty,
                function: signature.id,
                args,
            })
        }
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } if class_name == "Int" && method == "toFloat" => {
            let [value] = args.as_slice() else {
                return Err(vec![Diagnostic::new(
                    "I1401",
                    "checked Int::toFloat call does not have one argument",
                    *span,
                )]);
            };
            Ok(mir::FloatExpression::IntToFloat {
                value: Box::new(lower_integer_expression(value, context)?),
            })
        }
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } => {
            let (signature, args) =
                lower_static_method_call(class_name, method, args, *span, context)?;
            if signature.return_type
                != mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Float(ty)))
            {
                return Err(vec![unsupported(
                    *span,
                    "static method does not return expected float",
                )]);
            }
            Ok(mir::FloatExpression::Call {
                ty,
                function: signature.id,
                args,
            })
        }
        _ => Err(vec![unsupported(
            expr.span(),
            "this float expression is not supported by native compilation",
        )]),
    }
}

fn lower_integer_expression(
    expr: &hir::Expr,
    context: &LoweringContext,
) -> DiagnosticResult<mir::IntegerExpression> {
    if let Some(crate::const_eval::ConstValue::Integer(value)) = context.constant_value(expr) {
        return Ok(mir::IntegerExpression::constant(*value));
    }
    if let hir::Expr::FunctionCall { name, span, .. } = expr {
        if context.lookup_function(name, *span)?.return_type == mir::ReturnType::Void {
            return Err(vec![unsupported(
                *span,
                format!("void function `{name}` cannot be used as an integer expression"),
            )]);
        }
    }

    if let Some((magnitude, negative)) = integer_literal_parts(expr) {
        let ty = context.integer_type(expr)?;
        let value = IntegerValue::from_literal(ty, magnitude, negative).ok_or_else(|| {
            vec![Diagnostic::new(
                "I1301",
                format!("internal compiler consistency error: checked literal does not fit `{ty}`"),
                expr.span(),
            )]
        })?;
        return Ok(mir::IntegerExpression::constant(value));
    }

    if let hir::Expr::FunctionCall { name, args, span } = expr {
        let (function, return_type, args) = lower_integer_call(name, args, *span, context)?;
        let ty = context.integer_type(expr)?;
        if return_type != ty {
            return Err(vec![Diagnostic::new(
                "I1301",
                format!(
                    "internal compiler consistency error: function `{name}` returns `{return_type}`, expression metadata says `{ty}`"
                ),
                *span,
            )]);
        }
        return Ok(mir::IntegerExpression::Call { ty, function, args });
    }

    if let hir::Expr::MethodCall {
        object,
        method,
        args,
        span,
        ..
    } = expr
    {
        let (signature, args) = lower_instance_method_call(object, method, args, *span, context)?;
        let ty = context.integer_type(expr)?;
        if signature.return_type
            != mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Integer(ty)))
        {
            return Err(vec![unsupported(
                *span,
                "method has a different return type",
            )]);
        }
        return Ok(mir::IntegerExpression::Call {
            ty,
            function: signature.id,
            args,
        });
    }

    let ty = context.integer_type(expr)?;
    match expr {
        hir::Expr::Binary {
            left,
            op: hir::BinaryOp::Coalesce,
            right,
            ..
        } => match context.coalesce_selection(left) {
            CoalesceSelection::Left => lower_integer_expression(left, context),
            CoalesceSelection::Right => lower_integer_expression(right, context),
            CoalesceSelection::Dynamic => Ok(mir::IntegerExpression::Coalesce {
                ty,
                left: Box::new(lower_nullable_scalar_expression(
                    left,
                    mir::ScalarType::Integer(ty),
                    context,
                )?),
                right: Box::new(lower_integer_expression(right, context)?),
            }),
        },
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            match context.local_type(local) {
                mir::Type::Scalar(mir::ScalarType::Integer(local_ty)) if local_ty == ty => {
                    Ok(local_integer_expression(local, ty))
                }
                mir::Type::NullableScalar(mir::ScalarType::Integer(local_ty))
                    if local_ty == ty =>
                {
                    Ok(mir::IntegerExpression::Use {
                        ty,
                        operand: mir::Operand::NullablePayload(local),
                    })
                }
                _ => Err(vec![Diagnostic::new(
                    "I1301",
                    format!(
                        "internal compiler consistency error: `${name}` does not have MIR type `{ty}`"
                    ),
                    *span,
                )]),
            }
        }
        hir::Expr::PropertyAccess { .. } => {
            let (object, property) = lower_property_operand(
                expr,
                mir::Type::Scalar(mir::ScalarType::Integer(ty)),
                context,
            )?;
            Ok(mir::IntegerExpression::Use {
                ty,
                operand: mir::Operand::Property { object, property },
            })
        }
        hir::Expr::StaticMember {
            class_name,
            member,
            span,
        } => {
            let (id, static_ty) = context.static_property(class_name, member, *span)?;
            if static_ty != mir::Type::Scalar(mir::ScalarType::Integer(ty)) {
                return Err(vec![unsupported(
                    *span,
                    "static property has another integer type",
                )]);
            }
            Ok(mir::IntegerExpression::Use {
                ty,
                operand: mir::Operand::Static(id),
            })
        }
        hir::Expr::Grouped { expr, .. } => {
            let lowered = lower_integer_expression(expr, context)?;
            ensure_expression_type(&lowered, ty, expr.span())?;
            Ok(lowered)
        }
        hir::Expr::Unary { op, expr, .. } => {
            let operand = lower_integer_expression(expr, context)?;
            ensure_expression_type(&operand, ty, expr.span())?;
            let op = match op {
                hir::UnaryOp::Negate => mir::IntegerUnaryOp::Negate,
                hir::UnaryOp::BitwiseNot => mir::IntegerUnaryOp::BitwiseNot,
                hir::UnaryOp::Not => return Err(vec![unsupported_int_expr(expr)]),
            };
            Ok(mir::IntegerExpression::Unary {
                ty,
                op,
                operand: Box::new(operand),
            })
        }
        hir::Expr::Binary {
            left, op, right, ..
        } => {
            let op = lower_integer_binary_op(op, expr.span())?;
            let left = lower_integer_expression(left, context)?;
            let right = lower_integer_expression(right, context)?;
            ensure_expression_type(&left, ty, expr.span())?;
            ensure_expression_type(&right, ty, expr.span())?;
            Ok(mir::IntegerExpression::Binary {
                ty,
                op,
                left: Box::new(left),
                right: Box::new(right),
            })
        }
        hir::Expr::FunctionCall { .. } => unreachable!("function calls return before type lookup"),
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } if class_name == "Float" && method == "toInt" => {
            let [value] = args.as_slice() else {
                return Err(vec![Diagnostic::new(
                    "I1401",
                    "checked Float::toInt call does not have one argument",
                    *span,
                )]);
            };
            Ok(mir::IntegerExpression::FloatToInt {
                value: Box::new(lower_float_expression(value, context)?),
            })
        }
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } if method == "from" && IntegerType::from_companion_name(class_name).is_some() => {
            let [value] = args.as_slice() else {
                return Err(vec![Diagnostic::new(
                    "I1301",
                    "internal compiler consistency error: checked integer conversion does not have exactly one argument",
                    *span,
                )]);
            };
            let target = IntegerType::from_companion_name(class_name)
                .expect("guarded integer companion name");
            if target != ty {
                return Err(vec![Diagnostic::new(
                    "I1301",
                    format!(
                        "internal compiler consistency error: `{class_name}::from` targets `{target}`, expression metadata says `{ty}`"
                    ),
                    *span,
                )]);
            }
            Ok(mir::IntegerExpression::Convert {
                ty,
                value: Box::new(lower_integer_expression(value, context)?),
            })
        }
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } => {
            let (signature, args) =
                lower_static_method_call(class_name, method, args, *span, context)?;
            if signature.return_type
                != mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Integer(ty)))
            {
                return Err(vec![unsupported(
                    *span,
                    "static method has a different return type",
                )]);
            }
            Ok(mir::IntegerExpression::Call {
                ty,
                function: signature.id,
                args,
            })
        }
        hir::Expr::Int { .. } => unreachable!("integer literal handled before expression match"),
        _ => Err(vec![unsupported_int_expr(expr)]),
    }
}

fn lower_integer_binary_op(
    op: &hir::BinaryOp,
    span: Span,
) -> DiagnosticResult<mir::IntegerBinaryOp> {
    match op {
        hir::BinaryOp::Add => Ok(mir::IntegerBinaryOp::Add),
        hir::BinaryOp::Sub => Ok(mir::IntegerBinaryOp::Subtract),
        hir::BinaryOp::Mul => Ok(mir::IntegerBinaryOp::Multiply),
        hir::BinaryOp::Div => Ok(mir::IntegerBinaryOp::Divide),
        hir::BinaryOp::Mod => Ok(mir::IntegerBinaryOp::Remainder),
        hir::BinaryOp::ShiftLeft => Ok(mir::IntegerBinaryOp::ShiftLeft),
        hir::BinaryOp::ShiftRight => Ok(mir::IntegerBinaryOp::ShiftRight),
        hir::BinaryOp::BitwiseAnd => Ok(mir::IntegerBinaryOp::BitwiseAnd),
        hir::BinaryOp::BitwiseXor => Ok(mir::IntegerBinaryOp::BitwiseXor),
        hir::BinaryOp::BitwiseOr => Ok(mir::IntegerBinaryOp::BitwiseOr),
        hir::BinaryOp::Less
        | hir::BinaryOp::LessEqual
        | hir::BinaryOp::Greater
        | hir::BinaryOp::GreaterEqual
        | hir::BinaryOp::Equal
        | hir::BinaryOp::NotEqual => Err(vec![unsupported(
            span,
            "comparison results cannot be used as integer runtime values",
        )]),
        hir::BinaryOp::Concat => Err(vec![unsupported(
            span,
            "string concatenation cannot be used as an integer expression",
        )]),
        hir::BinaryOp::And | hir::BinaryOp::Or | hir::BinaryOp::Xor => Err(vec![unsupported(
            span,
            "boolean operator reached integer-only MIR lowering",
        )]),
        hir::BinaryOp::Coalesce => Err(vec![unsupported(
            span,
            "null coalescing cannot be used as an integer expression",
        )]),
    }
}

fn lower_compound_assignment_op(op: &hir::AssignOp) -> mir::IntegerBinaryOp {
    match op {
        hir::AssignOp::AddAssign => mir::IntegerBinaryOp::Add,
        hir::AssignOp::SubAssign => mir::IntegerBinaryOp::Subtract,
        hir::AssignOp::MulAssign => mir::IntegerBinaryOp::Multiply,
        hir::AssignOp::DivAssign => mir::IntegerBinaryOp::Divide,
        hir::AssignOp::ModAssign => mir::IntegerBinaryOp::Remainder,
        hir::AssignOp::ShiftLeftAssign => mir::IntegerBinaryOp::ShiftLeft,
        hir::AssignOp::ShiftRightAssign => mir::IntegerBinaryOp::ShiftRight,
        hir::AssignOp::BitwiseAndAssign => mir::IntegerBinaryOp::BitwiseAnd,
        hir::AssignOp::BitwiseXorAssign => mir::IntegerBinaryOp::BitwiseXor,
        hir::AssignOp::BitwiseOrAssign => mir::IntegerBinaryOp::BitwiseOr,
        hir::AssignOp::Assign => unreachable!("plain assignment does not have a binary operator"),
    }
}

fn lower_compound_value(
    target: mir::Operand,
    ty: mir::ScalarType,
    op: &hir::AssignOp,
    right: &hir::Expr,
    context: &LoweringContext,
) -> DiagnosticResult<mir::ValueExpression> {
    match ty {
        mir::ScalarType::Integer(integer) => {
            let right_span = right.span();
            let right = lower_integer_expression(right, context)?;
            ensure_expression_type(&right, integer, right_span)?;
            Ok(mir::ValueExpression::Integer(
                mir::IntegerExpression::Binary {
                    ty: integer,
                    op: lower_compound_assignment_op(op),
                    left: Box::new(mir::IntegerExpression::use_operand(integer, target)),
                    right: Box::new(right),
                },
            ))
        }
        mir::ScalarType::Float(float) => {
            let right = lower_float_expression(right, context)?;
            let op = match op {
                hir::AssignOp::AddAssign => mir::FloatBinaryOp::Add,
                hir::AssignOp::SubAssign => mir::FloatBinaryOp::Subtract,
                hir::AssignOp::MulAssign => mir::FloatBinaryOp::Multiply,
                hir::AssignOp::DivAssign => mir::FloatBinaryOp::Divide,
                _ => {
                    return Err(vec![unsupported(
                        Span::default(),
                        "invalid float compound assignment",
                    )])
                }
            };
            Ok(mir::ValueExpression::Float(mir::FloatExpression::Binary {
                ty: float,
                op,
                left: Box::new(mir::FloatExpression::Use {
                    ty: float,
                    operand: target,
                }),
                right: Box::new(right),
            }))
        }
        mir::ScalarType::Bool => Err(vec![unsupported(
            Span::default(),
            "bool compound assignment is invalid",
        )]),
    }
}

fn local_integer_expression(local: mir::LocalId, ty: IntegerType) -> mir::IntegerExpression {
    mir::IntegerExpression::use_operand(ty, mir::Operand::Local(local))
}

fn local_float_expression(local: mir::LocalId, ty: FloatType) -> mir::FloatExpression {
    mir::FloatExpression::Use {
        ty,
        operand: mir::Operand::Local(local),
    }
}

fn ensure_value_type(
    expression: &mir::ValueExpression,
    expected: mir::ScalarType,
    span: Span,
) -> DiagnosticResult<()> {
    if expression.ty() == expected {
        Ok(())
    } else {
        Err(vec![Diagnostic::new(
            "I1401",
            format!(
                "internal compiler consistency error: scalar expression has MIR type `{}`, expected `{expected}`",
                expression.ty()
            ),
            span,
        )])
    }
}

fn ensure_expression_type(
    expression: &mir::IntegerExpression,
    expected: IntegerType,
    span: Span,
) -> DiagnosticResult<()> {
    if expression.ty() == expected {
        Ok(())
    } else {
        Err(vec![Diagnostic::new(
            "I1301",
            format!(
                "internal compiler consistency error: integer expression has MIR type `{}`, expected `{expected}`",
                expression.ty()
            ),
            span,
        )])
    }
}

fn integer_literal_parts(expr: &hir::Expr) -> Option<(u128, bool)> {
    match expr {
        hir::Expr::Int { value, .. } => parse_decimal_magnitude(value).map(|value| (value, false)),
        hir::Expr::Grouped { expr, .. } => integer_literal_parts(expr),
        hir::Expr::Unary {
            op: hir::UnaryOp::Negate,
            expr,
            ..
        } => unsigned_integer_literal_magnitude(expr).map(|magnitude| (magnitude, true)),
        _ => None,
    }
}

fn unsigned_integer_literal_magnitude(expr: &hir::Expr) -> Option<u128> {
    match expr {
        hir::Expr::Int { value, .. } => parse_decimal_magnitude(value),
        hir::Expr::Grouped { expr, .. } => unsigned_integer_literal_magnitude(expr),
        _ => None,
    }
}

fn unsupported_int_expr(expr: &hir::Expr) -> Diagnostic {
    let detail = match expr {
        hir::Expr::String { .. } | hir::Expr::InterpolatedString { .. } => {
            "a string expression cannot be used as an integer expression"
        }
        hir::Expr::Float { .. } => "a float expression cannot be used as an integer expression",
        hir::Expr::Bool { .. } => "bool value reached integer-only MIR lowering",
        hir::Expr::IsType { .. } => "a type-test result cannot be used as an integer expression",
        hir::Expr::Null { .. } => "`null` cannot be used as an integer expression",
        hir::Expr::Array { .. } => "a collection cannot be used as an integer expression",
        hir::Expr::FunctionCall { .. } => {
            "this function call cannot be used as an integer expression"
        }
        hir::Expr::MethodCall { .. } | hir::Expr::StaticCall { .. } => {
            "a method call cannot be used as an integer expression"
        }
        hir::Expr::StaticMember { .. } => "a static member cannot be used as an integer expression",
        hir::Expr::PropertyAccess { .. } => {
            "class property access cannot be used as an integer expression"
        }
        hir::Expr::New { .. } => "object construction cannot be used as an integer expression",
        hir::Expr::This { .. } => "`$this` cannot be used as an integer expression",
        hir::Expr::Identifier { .. } => "this identifier cannot be used as an integer expression",
        hir::Expr::Unary { .. } => "this unary expression cannot be used as an integer expression",
        hir::Expr::Range { .. } => "a range cannot be used as an integer expression",
        hir::Expr::Binary {
            op:
                hir::BinaryOp::Equal
                | hir::BinaryOp::NotEqual
                | hir::BinaryOp::Less
                | hir::BinaryOp::LessEqual
                | hir::BinaryOp::Greater
                | hir::BinaryOp::GreaterEqual,
            ..
        } => "comparison results cannot be used as integer runtime values",
        hir::Expr::Binary { .. } => {
            "this binary expression cannot be used as an integer expression"
        }
        hir::Expr::Int { .. } | hir::Expr::Variable { .. } | hir::Expr::Grouped { .. } => {
            "this integer expression is not supported by native compilation"
        }
    };
    unsupported(expr.span(), detail)
}

fn stmt_span(statement: &hir::Stmt) -> Span {
    match statement {
        hir::Stmt::VarDecl(decl) => decl.span,
        hir::Stmt::Assignment(assignment) => assignment.span,
        hir::Stmt::Echo { span, .. } | hir::Stmt::Return { span, .. } => *span,
        hir::Stmt::If(if_stmt) => if_stmt.span,
        hir::Stmt::While(while_stmt) => while_stmt.span,
        hir::Stmt::For(for_stmt) => for_stmt.span,
        hir::Stmt::Break { span } | hir::Stmt::Continue { span } => *span,
        hir::Stmt::Foreach(foreach) => foreach.span,
        hir::Stmt::Increment(increment) => increment.span,
        hir::Stmt::Expr { span, .. } => *span,
    }
}

fn unsupported(span: Span, detail: impl Into<String>) -> Diagnostic {
    Diagnostic::new("M1101", detail, span)
}

fn unsupported_native_type(
    ty: &crate::types::TypeRef,
    span: Span,
    detail: impl Into<String>,
) -> Diagnostic {
    if ty.name == "mixed" {
        Diagnostic::unsupported_stage(
            "M1101",
            "the `mixed` runtime representation lands at Stage 23",
            span,
        )
    } else {
        unsupported(span, detail)
    }
}

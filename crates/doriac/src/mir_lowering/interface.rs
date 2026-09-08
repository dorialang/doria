use super::*;
use crate::semantics::contracts::{ConformanceStatus, RequirementFacts};
use crate::types::{
    FunctionBorrowSource, FunctionInvocationMode, FunctionReturnBorrow, FunctionTypeParameterMode,
    SemanticFunctionParameter, SemanticFunctionType,
};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct MethodKey {
    interface: mir::InterfaceTypeId,
    requirement: Span,
    arguments: Vec<GenericArgument>,
}

#[derive(Clone)]
pub(super) struct CallPlan {
    slot: usize,
    signature: FunctionSignature,
}

pub(super) fn convert_value(
    value: mir::Rvalue,
    target: mir::Type,
    span: Span,
    context: &LoweringContext<'_>,
) -> DiagnosticResult<mir::Rvalue> {
    let (target_type, nullable) = non_null_match_type(target);
    let mir::Type::Interface(interface) = target_type else {
        unreachable!("interface result conversion")
    };
    let upcast = |source: mir::InterfaceTypeId| {
        source == interface
            || context.collection_registry.interface_types[source.0]
                .ancestors
                .contains(&interface)
    };
    let value = match value {
        mir::Rvalue::Class(object) => mir::InterfaceValue::FromClass {
            vtable: context.class_interface_vtable(object.class(), interface, span)?,
            object: Box::new(object),
        },
        mir::Rvalue::NullableClass(object) if nullable => {
            return Ok(mir::Rvalue::nullable_interface(
                interface,
                mir::NullableInterfaceValue::Present(mir::InterfaceValue::FromNullableClass {
                    vtable: context.class_interface_vtable(object.class(), interface, span)?,
                    object: Box::new(object),
                }),
            ));
        }
        mir::Rvalue::Interface(source) if upcast(source.interface) => {
            if source.interface == interface {
                source.value
            } else {
                mir::InterfaceValue::Upcast {
                    source: Box::new(source),
                    interface,
                }
            }
        }
        mir::Rvalue::NullableInterface(source) if nullable && upcast(source.interface) => {
            return Ok(if source.interface == interface {
                mir::Rvalue::NullableInterface(source)
            } else {
                mir::Rvalue::nullable_interface(
                    interface,
                    mir::NullableInterfaceValue::Upcast {
                        source: Box::new(source),
                        interface,
                    },
                )
            });
        }
        _ => {
            return Err(vec![unsupported(
                span,
                "call result has no checked conversion to the required interface",
            )])
        }
    };
    Ok(if nullable {
        mir::Rvalue::nullable_interface(interface, mir::NullableInterfaceValue::Present(value))
    } else {
        mir::Rvalue::interface(interface, value)
    })
}

pub(super) fn call_plan(
    expr: &hir::Expr,
    context: &LoweringContext<'_>,
) -> DiagnosticResult<Option<(mir::InterfaceTypeId, CallPlan)>> {
    if !matches!(expr, hir::Expr::MethodCall { .. }) {
        return Ok(None);
    }
    let Some(CallableTarget::InterfaceMethod {
        interface,
        requirement,
        ..
    }) = context
        .semantic_info
        .call_targets
        .get(&expr.span())
        .and_then(|target| {
            target.specialize(|ty| substitute_resolved_type(ty, &context.type_substitutions))
        })
    else {
        return Ok(None);
    };
    let interface = context
        .collection_registry
        .interface_ids
        .get(&interface)
        .copied()
        .ok_or_else(|| {
            vec![Diagnostic::new(
                "I2401",
                "checked interface call has no native interface specialization",
                expr.span(),
            )]
        })?;
    let key = MethodKey {
        interface,
        requirement,
        arguments: context.specialization_arguments(expr.span()),
    };
    context
        .collection_registry
        .interface_methods
        .get(&key)
        .cloned()
        .map(|plan| Some((interface, plan)))
        .ok_or_else(|| {
            vec![Diagnostic::new(
                "I2401",
                "checked interface call has no specialized requirement slot",
                expr.span(),
            )]
        })
}

pub(super) fn materialize_call(
    expr: &hir::Expr,
    (interface, plan): (mir::InterfaceTypeId, CallPlan),
    consume_result: bool,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<Option<(mir::LocalId, mir::Type, bool)>> {
    let hir::Expr::MethodCall {
        object,
        method,
        args,
        span,
        null_safe,
    } = expr
    else {
        unreachable!("interface call plan")
    };
    let definition = context.collection_registry.function_types[context
        .collection_registry
        .interface_types[interface.0]
        .methods[plan.slot]
        .signature
        .0]
        .clone();
    materialize_nested_collection_places(object, false, context)?;
    if *null_safe {
        let receiver = mir::Rvalue::nullable_interface(
            interface,
            lower_nullable_interface_expression(object, interface, false, context)?,
        );
        let writable = definition.parameters[0].mode == mir::FunctionParameterMode::Writable;
        let result = match definition.return_type {
            mir::ReturnType::Void => None,
            mir::ReturnType::Value(_) => Some((context.expression_type(expr)?, consume_result)),
        };
        return materialize_null_safe_call(
            receiver,
            writable,
            definition.return_borrow,
            *span,
            result,
            |receiver, context| {
                materialize_receiver_call(
                    mir::InterfaceValue::NullableLocalAssumeNonNull {
                        local: receiver,
                        transfer: false,
                    },
                    interface,
                    plan,
                    &definition,
                    method,
                    args,
                    *span,
                    result.is_some(),
                    context,
                )
            },
            context,
        );
    }
    let value = lower_interface_expression(object, interface, false, context)?;
    materialize_receiver_call(
        value,
        interface,
        plan,
        &definition,
        method,
        args,
        *span,
        consume_result,
        context,
    )
}

#[allow(clippy::too_many_arguments)]
fn materialize_receiver_call(
    value: mir::InterfaceValue,
    interface: mir::InterfaceTypeId,
    plan: CallPlan,
    definition: &mir::FunctionType,
    method: &str,
    args: &[hir::Argument],
    span: Span,
    consume_result: bool,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<Option<(mir::LocalId, mir::Type, bool)>> {
    let owned = !value.is_borrowed();
    let receiver = context.declare_checked_call_slot(mir::Type::Interface(interface), owned);
    context.locals[receiver.0].writable =
        definition.parameters[0].mode == mir::FunctionParameterMode::Writable;
    context.push_statement(mir::Statement::AssignLocal {
        target: receiver,
        value: mir::Rvalue::Interface(mir::InterfaceExpression { interface, value }),
    });
    if owned {
        context.track_statement_owned_local(receiver, mir::Type::Interface(interface));
    }
    let mut lowered = vec![local_rvalue(
        receiver,
        mir::Type::Interface(interface),
        false,
    )];
    lowered.extend(lower_call_args_with_ownership(
        method,
        args,
        plan.signature,
        span,
        context,
    )?);
    emit_indirect_call(
        mir::IndirectCallee::InterfaceMethod {
            receiver,
            interface,
            slot: plan.slot,
        },
        definition,
        lowered,
        span,
        consume_result,
        context,
    )
}

pub(super) fn materialize_display(
    expr: &hir::Expr,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<mir::StringExpression> {
    let displayable = crate::types::InterfaceType::new("Displayable", Vec::new());
    let interface = *context
        .collection_registry
        .interface_ids
        .get(&displayable)
        .ok_or_else(|| {
            vec![unsupported(
                expr.span(),
                "Displayable has no registered native identity",
            )]
        })?;
    let requirement = context
        .semantic_info
        .contracts
        .interface_specializations
        .iter()
        .find(|fact| fact.specialization == displayable)
        .and_then(|fact| fact.requirements.first())
        .and_then(|requirement| requirement.origins.first())
        .ok_or_else(|| {
            vec![unsupported(
                expr.span(),
                "Displayable has no checked requirement",
            )]
        })?;
    let key = MethodKey {
        interface,
        requirement: requirement.declaration,
        arguments: Vec::new(),
    };
    let plan = context
        .collection_registry
        .interface_methods
        .get(&key)
        .cloned()
        .ok_or_else(|| {
            vec![unsupported(
                expr.span(),
                "Displayable has no executable requirement slot",
            )]
        })?;
    let definition = context.collection_registry.function_types[context
        .collection_registry
        .interface_types[interface.0]
        .methods[plan.slot]
        .signature
        .0]
        .clone();
    materialize_nested_collection_places(expr, false, context)?;
    let value = lower_interface_expression(expr, interface, false, context)?;
    let (local, ty, _) = materialize_receiver_call(
        value,
        interface,
        plan,
        &definition,
        "Displayable::toString",
        &[],
        expr.span(),
        false,
        context,
    )?
    .ok_or_else(|| vec![unsupported(expr.span(), "Displayable call returned void")])?;
    debug_assert_eq!(ty, mir::Type::String);
    Ok(mir::StringExpression::Local(local))
}

pub(super) fn register_methods(
    semantic: &SemanticInfo,
    calls: &[InterfaceGenericCall],
    classes: &ClassIds,
    registry: &mut NativeTypeRegistry,
) -> DiagnosticResult<()> {
    for fact in &semantic.contracts.interface_specializations {
        if crate::compiler_known_contracts::requires_core_execution(&fact.specialization.name)
            || fact.ancestors.iter().any(|ancestor| {
                crate::compiler_known_contracts::requires_core_execution(&ancestor.name)
            })
        {
            continue;
        }
        let Some(interface) = registry.interface_ids.get(&fact.specialization).copied() else {
            continue;
        };
        // Compiler-known declarations exist in every analysis graph. Only a
        // reachable call or a concrete implementation needs executable entries.
        if semantic.contracts.interfaces.iter().any(|declaration| {
            declaration.name == fact.specialization.name
                && declaration.declaration.source == crate::compiler_known_contracts::SOURCE_ID
        }) && semantic.display_conversion_sites.is_empty()
            && !calls
                .iter()
                .any(|call| call.interface == fact.specialization)
            && !registry
                .interface_vtables
                .iter()
                .any(|table| table.interface == interface)
        {
            continue;
        }
        for requirement in &fact.requirements {
            let mut tuples = Vec::new();
            if requirement.generic_parameters.is_empty() {
                tuples.push(Vec::new());
            } else {
                for call in calls {
                    if call.interface == fact.specialization
                        && requirement
                            .origins
                            .iter()
                            .any(|origin| origin.declaration == call.requirement)
                        && !tuples.contains(&call.arguments)
                    {
                        tuples.push(call.arguments.clone());
                    }
                }
            }
            for arguments in tuples {
                let substitutions = requirement
                    .generic_parameters
                    .iter()
                    .zip(&arguments)
                    .map(|(parameter, GenericArgument::Type(ty))| {
                        (parameter.name.clone(), ty.clone())
                    })
                    .collect::<HashMap<_, _>>();
                let signature = requirement_signature(
                    requirement,
                    interface,
                    &substitutions,
                    classes,
                    registry,
                )?;
                let mir_arguments = arguments
                    .iter()
                    .map(|argument| match argument {
                        GenericArgument::Type(ty) => {
                            intern_resolved_collection_types(ty, classes, registry)
                        }
                    })
                    .collect::<Option<Vec<_>>>()
                    .ok_or_else(|| {
                        vec![unsupported(
                            requirement.origins[0].declaration,
                            "interface method specialization has no native type arguments",
                        )]
                    })?;
                let slot = registry.interface_types[interface.0].methods.len();
                registry.interface_types[interface.0]
                    .methods
                    .push(mir::InterfaceMethod {
                        requirement: requirement.origins[0].declaration,
                        name: requirement.name.clone(),
                        arguments: mir_arguments,
                        signature,
                        writable_receiver: requirement.writable_receiver,
                        exact_dynamic_return: matches!(
                            requirement.signature.return_type,
                            ResolvedType::InterfaceSelf(_)
                        ),
                    });
                let definition = &registry.function_types[signature.0];
                let parameter_types = definition.parameters[1..]
                    .iter()
                    .map(|param| param.ty)
                    .collect::<Vec<_>>();
                let parameter_modes = definition.parameters[1..]
                    .iter()
                    .map(|param| param.mode)
                    .collect::<Vec<_>>();
                let binding = FunctionSignature {
                    id: mir::FunctionId(usize::MAX),
                    direct_id: None,
                    return_type: definition.return_type,
                    return_borrow: definition.return_borrow,
                    parameter_names: requirement
                        .signature
                        .parameters
                        .iter()
                        .map(|param| param.name.clone())
                        .collect(),
                    parameter_defaults: vec![None; parameter_types.len()],
                    parameter_owns: parameter_types
                        .iter()
                        .zip(&parameter_modes)
                        .map(|(ty, mode)| {
                            ty.has_move_ownership() && *mode == mir::FunctionParameterMode::Take
                        })
                        .collect(),
                    parameter_types,
                    parameter_modes,
                    method_class: None,
                    receiver_mode: None,
                    required_checked_effects: definition.checked_effects.clone(),
                    ambient_checked_effects: definition.ambient_checked_effects.clone(),
                    test_assertion_checked_effects: definition
                        .test_assertion_checked_effects
                        .clone(),
                    checked_effects: definition.complete_checked_effects(),
                };
                for origin in &requirement.origins {
                    registry.interface_methods.insert(
                        MethodKey {
                            interface,
                            requirement: origin.declaration,
                            arguments: arguments.clone(),
                        },
                        CallPlan {
                            slot,
                            signature: binding.clone(),
                        },
                    );
                }
            }
        }
    }
    Ok(())
}

fn requirement_signature(
    requirement: &RequirementFacts,
    interface: mir::InterfaceTypeId,
    substitutions: &HashMap<String, ResolvedType>,
    classes: &ClassIds,
    registry: &mut NativeTypeRegistry,
) -> DiagnosticResult<mir::FunctionTypeId> {
    let receiver = registry
        .interface_ids
        .iter()
        .find_map(|(ty, id)| (*id == interface).then_some(ty.clone()))
        .expect("registered interface");
    let mut parameters = vec![SemanticFunctionParameter {
        ty: ResolvedType::Interface(receiver.clone()),
        ownership_mode: if requirement.writable_receiver {
            FunctionTypeParameterMode::Writable
        } else {
            FunctionTypeParameterMode::Readonly
        },
    }];
    parameters.extend(requirement.signature.parameters.iter().map(|parameter| {
        SemanticFunctionParameter {
            ty: substitute_resolved_type(&parameter.r#type, substitutions),
            ownership_mode: if parameter.take {
                FunctionTypeParameterMode::Take
            } else if parameter.writable {
                FunctionTypeParameterMode::Writable
            } else {
                FunctionTypeParameterMode::Readonly
            },
        }
    }));
    let result = substitute_resolved_type(&requirement.signature.return_type, substitutions);
    let result = if matches!(result, ResolvedType::InterfaceSelf(_)) {
        ResolvedType::Interface(receiver)
    } else {
        result
    };
    let function = ResolvedType::Function(Box::new(SemanticFunctionType {
        invocation_mode: if requirement.writable_receiver {
            FunctionInvocationMode::Writable
        } else {
            FunctionInvocationMode::Readonly
        },
        parameters,
        return_type: result,
        checked_effects: requirement
            .checked_effects
            .iter()
            .map(|effect| substitute_resolved_type(effect, substitutions))
            .collect(),
        return_borrow: requirement
            .return_borrow
            .map(|borrow| FunctionReturnBorrow {
                source: FunctionBorrowSource::Parameter(match borrow.source {
                    crate::symbols::BorrowSource::Receiver => 0,
                    crate::symbols::BorrowSource::Parameter(index) => index + 1,
                }),
                writable: borrow.writable,
            }),
    }));
    match intern_resolved_collection_types(&function, classes, registry) {
        Some(mir::Type::Function(id)) => Ok(id),
        _ => Err(vec![unsupported(
            requirement.origins[0].declaration,
            "checked interface requirement has no native function signature",
        )]),
    }
}

pub(super) fn build_entries(
    semantic: &SemanticInfo,
    classes: &ClassIds,
    methods: &HashMap<MethodInstanceKey, FunctionSignature>,
    registry: &mut NativeTypeRegistry,
    functions: &mut Vec<mir::Function>,
) -> DiagnosticResult<()> {
    for index in 0..registry.interface_vtables.len() {
        let table = registry.interface_vtables[index].clone();
        let mir::ImplementingType::Class(class) = table.implementing_type else {
            continue;
        };
        let conformance = semantic.contracts.conformances.iter().find(|fact| {
            fact.status == ConformanceStatus::Checked
                && registry.interface_ids.get(&fact.interface) == Some(&table.interface)
                && matches!(&fact.implementing_type, ResolvedType::Class(ty) if classes.get(ty) == Some(&class))
        }).expect("registered vtable has checked conformance");
        for method in registry.interface_types[table.interface.0].methods.clone() {
            let implementation = conformance
                .implementations
                .iter()
                .find(|item| {
                    item.requirement_origins
                        .iter()
                        .any(|origin| origin.declaration == method.requirement)
                })
                .expect("checked conformance selects an implementation");
            if method.exact_dynamic_return
                && implementation.exact_dynamic_return.as_ref()
                    != Some(&conformance.implementing_type)
            {
                return Err(vec![Diagnostic::new(
                    "I2401",
                    "interface self result lacks checked exact dynamic return proof",
                    method.requirement,
                )]);
            }
            let implementation = implementation
                .implementation
                .expect("checked implementation declaration");
            let key = registry
                .interface_methods
                .keys()
                .find(|key| {
                    key.interface == table.interface
                        && key.requirement == method.requirement
                        && registry.interface_methods[*key].slot
                            == registry.interface_vtables[index].methods.len()
                })
                .expect("registered slot has call plan");
            let signature = std::iter::once(class)
                .chain(
                    semantic.classes[class.0]
                        .ancestors
                        .iter()
                        .filter_map(|ty| classes.get(ty).copied()),
                )
                .find_map(|class| {
                    methods.get(&MethodInstanceKey {
                        class,
                        name: method.name.clone(),
                        arguments: key.arguments.clone(),
                    })
                })
                .filter(|signature| functions[signature.id.0].source_span == implementation)
                .ok_or_else(|| {
                    vec![Diagnostic::new(
                        "I2401",
                        "checked interface slot has no concrete method specialization",
                        implementation,
                    )]
                })?;
            let id = mir::FunctionId(functions.len());
            let definition = &registry.function_types[method.signature.0];
            let entry = build_entry(
                id,
                &table,
                definition,
                signature,
                &functions[signature.id.0],
                registry,
            )?;
            functions.push(entry);
            registry.interface_vtables[index].methods.push(id);
        }
    }
    Ok(())
}

fn build_entry(
    id: mir::FunctionId,
    table: &mir::InterfaceVtable,
    contract: &mir::FunctionType,
    signature: &FunctionSignature,
    implementation: &mir::Function,
    registry: &NativeTypeRegistry,
) -> DiagnosticResult<mir::Function> {
    let mir::ImplementingType::Class(class) = table.implementing_type else {
        unreachable!("class conformance")
    };
    let span = implementation.source_span;
    let mut locals = contract
        .parameters
        .iter()
        .enumerate()
        .map(|(index, parameter)| mir::Local {
            id: mir::LocalId(index),
            name: format!("__interface_arg{index}"),
            ty: parameter.ty,
            writable: parameter.mode == mir::FunctionParameterMode::Writable,
            owned: parameter.mode == mir::FunctionParameterMode::Take
                && parameter.ty.has_move_ownership(),
            synthetic: true,
        })
        .collect::<Vec<_>>();
    let params = locals.iter().map(|local| local.id).collect::<Vec<_>>();
    let mut statements = Vec::new();
    let mut args = vec![mir::Rvalue::Class(
        mir::ClassExpression::InterfaceReceiver {
            class,
            receiver: params[0],
            vtable: table.id,
        },
    )];
    if let Some(parent) = implementation
        .method
        .as_ref()
        .map(|method| method.class)
        .filter(|parent| *parent != class)
    {
        let mir::Rvalue::Class(receiver) = args.remove(0) else {
            unreachable!()
        };
        let target = mir::LocalId(locals.len());
        locals.push(mir::Local {
            id: target,
            name: "__interface_payload".into(),
            ty: mir::Type::Class(class),
            writable: locals[0].writable,
            owned: false,
            synthetic: true,
        });
        statements.push(mir::Statement::AssignLocal {
            target,
            value: mir::Rvalue::Class(receiver),
        });
        args.push(mir::Rvalue::Class(mir::ClassExpression::Local {
            class: parent,
            local: target,
            transfer: false,
        }));
    }
    args.extend(params.iter().skip(1).map(|id| {
        let local = &locals[id.0];
        local_rvalue(*id, local.ty, local.owned)
    }));
    let direct = signature.direct_id.unwrap_or(signature.id);
    let mut blocks = Vec::new();
    let result = if let mir::ReturnType::Value(ty) = signature.return_type {
        let local = mir::LocalId(locals.len());
        locals.push(mir::Local {
            id: local,
            name: "__interface_result".into(),
            ty,
            writable: true,
            owned: ty.has_move_ownership() && signature.return_borrow.is_none(),
            synthetic: true,
        });
        Some((local, ty))
    } else {
        None
    };
    let return_value = match (result, contract.return_type) {
        (None, mir::ReturnType::Void) => mir::Terminator::ReturnVoid,
        (Some((local, actual)), mir::ReturnType::Value(target)) => {
            let value = local_rvalue(local, actual, signature.return_borrow.is_none());
            let value = if target == actual {
                value
            } else if let (mir::Type::Interface(interface), mir::Rvalue::Class(object)) =
                (target, value.clone())
            {
                let vtable = registry
                    .interface_vtable_ids
                    .get(&(mir::ImplementingType::Class(object.class()), interface))
                    .copied()
                    .ok_or_else(|| {
                        vec![unsupported(
                            span,
                            "interface return adaptation lacks checked conformance",
                        )]
                    })?;
                mir::Rvalue::Interface(mir::InterfaceExpression {
                    interface,
                    value: mir::InterfaceValue::FromClass {
                        object: Box::new(object),
                        vtable,
                    },
                })
            } else {
                virtual_adapter_local_result(
                    local,
                    target,
                    actual,
                    signature.return_borrow.is_none(),
                    span,
                )?
            };
            mir::Terminator::Return(value)
        }
        _ => {
            return Err(vec![unsupported(
                span,
                "interface entry return contract disagrees with its implementation",
            )])
        }
    };
    if signature.checked_effects.is_empty() {
        let statement = match result {
            Some((target, ty)) => mir::Statement::AssignLocal {
                target,
                value: direct_call_result(ty, direct, args, signature.return_borrow),
            },
            None => mir::Statement::CallVoid {
                function: direct,
                args,
                span,
            },
        };
        statements.push(statement);
        blocks.push(mir::BasicBlock {
            id: mir::BlockId(0),
            statements,
            terminator: return_value,
        });
    } else {
        let error = mir::LocalId(locals.len());
        locals.push(mir::Local {
            id: error,
            name: "__interface_error".into(),
            ty: mir::Type::ERROR,
            writable: true,
            owned: true,
            synthetic: true,
        });
        blocks.push(mir::BasicBlock {
            id: mir::BlockId(0),
            statements,
            terminator: mir::Terminator::CheckedCall {
                function: direct,
                args,
                result: result.map(|(local, _)| local),
                error,
                success: mir::BlockId(1),
                failure: mir::BlockId(2),
                span,
            },
        });
        blocks.push(mir::BasicBlock {
            id: mir::BlockId(1),
            statements: vec![],
            terminator: return_value,
        });
        blocks.push(mir::BasicBlock {
            id: mir::BlockId(2),
            statements: vec![],
            terminator: mir::Terminator::PropagateError { error },
        });
    }
    Ok(mir::Function {
        id,
        name: format!("{}::<interface-entry#{}>", implementation.name, table.id.0),
        source_span: span,
        method: None,
        virtual_slot: None,
        receiver_mode: None,
        closure: None,
        params,
        parameter_modes: contract
            .parameters
            .iter()
            .map(|parameter| parameter.mode)
            .collect(),
        return_type: contract.return_type,
        return_borrow: contract.return_borrow,
        required_checked_effects: contract.checked_effects.clone(),
        ambient_checked_effects: contract.ambient_checked_effects.clone(),
        test_assertion_checked_effects: contract.test_assertion_checked_effects.clone(),
        checked_effects: contract.complete_checked_effects(),
        locals,
        blocks,
        entry_block: mir::BlockId(0),
    })
}

fn direct_call_result(
    ty: mir::Type,
    function: mir::FunctionId,
    args: Vec<mir::Rvalue>,
    return_borrow: Option<mir::ReturnBorrow>,
) -> mir::Rvalue {
    use mir::*;
    match ty {
        Type::ClosureEnvironment(_) => {
            unreachable!("closure environments are not source return types")
        }
        Type::Scalar(ScalarType::Integer(ty)) => {
            Rvalue::Value(ValueExpression::Integer(IntegerExpression::Call {
                ty,
                function,
                args,
            }))
        }
        Type::Scalar(ScalarType::Float(ty)) => {
            Rvalue::Value(ValueExpression::Float(FloatExpression::Call {
                ty,
                function,
                args,
            }))
        }
        Type::Scalar(ScalarType::Bool) => {
            Rvalue::Value(ValueExpression::Bool(BoolExpression::Call {
                function,
                args,
            }))
        }
        Type::Scalar(ScalarType::Enum(enum_id)) => {
            Rvalue::Value(ValueExpression::Enum(EnumExpression::Call {
                enum_id,
                function,
                args,
            }))
        }
        Type::NullableScalar(ty) => {
            Rvalue::NullableScalar(NullableScalarExpression::Call { ty, function, args })
        }
        Type::String => Rvalue::String(StringExpression::Call { function, args }),
        Type::NullableString => {
            Rvalue::NullableString(NullableStringExpression::Call { function, args })
        }
        Type::Class(class) => Rvalue::Class(ClassExpression::Call {
            class,
            function,
            args,
            return_borrow,
        }),
        Type::NullableClass(class) => Rvalue::NullableClass(NullableClassExpression::Call {
            class,
            function,
            args,
            return_borrow,
        }),
        Type::Interface(interface) => Rvalue::Interface(InterfaceExpression {
            interface,
            value: InterfaceValue::Call {
                function,
                args,
                return_borrow,
            },
        }),
        Type::NullableInterface(interface) => {
            Rvalue::NullableInterface(NullableInterfaceExpression {
                interface,
                value: NullableInterfaceValue::Call {
                    function,
                    args,
                    return_borrow,
                },
            })
        }
        Type::Collection(collection) => Rvalue::Collection(CollectionExpression::Call {
            collection,
            function,
            args,
            return_borrow,
        }),
        Type::NullableCollection(collection) => {
            Rvalue::NullableCollection(NullableCollectionExpression::Call {
                collection,
                function,
                args,
                return_borrow,
            })
        }
        Type::Function(function_type) => Rvalue::Function(FunctionExpression::Call {
            function_type,
            function,
            args,
            return_borrow,
        }),
        Type::NullableFunction(function_type) => {
            Rvalue::NullableFunction(NullableFunctionExpression::Call {
                function_type,
                function,
                args,
                return_borrow,
            })
        }
        Type::Mixed => Rvalue::Mixed(MixedExpression::Call {
            function,
            args,
            return_borrow,
        }),
        Type::NullableMixed => Rvalue::NullableMixed(NullableMixedExpression::Call {
            function,
            args,
            return_borrow,
        }),
        Type::PayloadEnum(ty) => {
            Rvalue::PayloadEnum(PayloadEnumExpression::Call { ty, function, args })
        }
        Type::NullablePayloadEnum(ty) => {
            Rvalue::NullablePayloadEnum(NullablePayloadEnumExpression::Call { ty, function, args })
        }
        Type::SharedReference(class) => Rvalue::SharedReference(SharedReferenceExpression::Call {
            payload: class,
            function,
            args,
            return_borrow,
        }),
        Type::WeakReference(class) => Rvalue::WeakReference(WeakReferenceExpression::Call {
            payload: class,
            function,
            args,
            return_borrow,
        }),
        Type::NullableSharedReference(class) => {
            Rvalue::NullableSharedReference(NullableSharedReferenceExpression::Call {
                payload: class,
                function,
                args,
                return_borrow,
            })
        }
        Type::NullableWeakReference(class) => {
            Rvalue::NullableWeakReference(NullableWeakReferenceExpression::Call {
                payload: class,
                function,
                args,
                return_borrow,
            })
        }
        Type::WritableSharedReference(payload) => {
            Rvalue::WritableSharedReference(WritableSharedReferenceExpression::Call {
                payload,
                function,
                args,
                return_borrow,
            })
        }
        Type::WritableWeakReference(payload) => {
            Rvalue::WritableWeakReference(WritableWeakReferenceExpression::Call {
                payload,
                function,
                args,
                return_borrow,
            })
        }
        Type::NullableWritableSharedReference(payload) => Rvalue::NullableWritableSharedReference(
            NullableWritableSharedReferenceExpression::Call {
                payload,
                function,
                args,
                return_borrow,
            },
        ),
        Type::NullableWritableWeakReference(payload) => {
            Rvalue::NullableWritableWeakReference(NullableWritableWeakReferenceExpression::Call {
                payload,
                function,
                args,
                return_borrow,
            })
        }
        Type::ReadonlySharedReferenceAccess(payload)
        | Type::WritableSharedReferenceAccess(payload) => {
            Rvalue::SharedReferenceAccess(SharedReferenceAccessExpression::Call {
                payload,
                function,
                args,
                return_borrow,
                writable: matches!(ty, Type::WritableSharedReferenceAccess(_)),
            })
        }
        Type::NullableReadonlySharedReferenceAccess(payload)
        | Type::NullableWritableSharedReferenceAccess(payload) => {
            Rvalue::NullableSharedReferenceAccess(NullableSharedReferenceAccessExpression::Call {
                payload,
                function,
                args,
                return_borrow,
                writable: matches!(ty, Type::NullableWritableSharedReferenceAccess(_)),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_directory() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        loop {
            let path = std::env::temp_dir().join(format!(
                "doriac-interface-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed),
            ));
            match std::fs::create_dir(&path) {
                Ok(()) => return path,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("cannot create interface test directory: {error}"),
            }
        }
    }

    fn assert_native_output(program: &mir::Program, expected: &[u8]) {
        use crate::backend::NativeProfile;
        use std::{
            fs,
            process::{Command, Stdio},
        };
        #[cfg(not(feature = "llvm-backend"))]
        let profiles = [NativeProfile::Fast];
        #[cfg(feature = "llvm-backend")]
        let profiles = [NativeProfile::Fast, NativeProfile::Release];
        for profile in profiles {
            let bytes = crate::codegen_native::generate_executable(program, profile).unwrap();
            let directory = test_directory();
            let executable = directory.join(if cfg!(windows) {
                "program.exe"
            } else {
                "program"
            });
            fs::write(&executable, bytes).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
            }
            let output = crate::native_process::spawn(
                Command::new(&executable)
                    .current_dir(&directory)
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped()),
            )
            .and_then(|child| child.wait_with_output());
            fs::remove_dir_all(&directory).unwrap();
            let output = output.unwrap();
            assert_eq!(
                output.status.code(),
                Some(0),
                "{profile:?}: {output:?}\n{program}"
            );
            assert_eq!(output.stdout, expected, "{profile:?}");
            assert!(
                output.stderr.is_empty(),
                "{profile:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    fn lower_checked_source(text: &str) -> mir::Program {
        lower_checked_source_in_scope(text, crate::build_plan::SourceScope::Main)
    }

    fn lower_checked_source_in_scope(
        text: &str,
        scope: crate::build_plan::SourceScope,
    ) -> mir::Program {
        let hir = checked_hir(text, scope);
        let program = lower_program(&hir).unwrap_or_else(|diagnostics| {
            panic!(
                "{}",
                crate::render_diagnostics("interface-runtime.doria", text, &diagnostics)
            )
        });
        crate::mir_validation::validate_program(&program)
            .unwrap_or_else(|error| panic!("{error:?}\n{program}"));
        program
    }

    fn checked_hir(text: &str, scope: crate::build_plan::SourceScope) -> hir::Program {
        let source = crate::source::SourceFile::new("interface-runtime.doria", text);
        let context = crate::names::CompilationContext::standalone(source.path.clone());
        let prepared = crate::prepare_source(&source, context.clone()).unwrap();
        let mut source_context = crate::testing::SourceSemanticContext::standalone(context.clone());
        source_context.scope = scope;
        let (_, analysis) = crate::analyze_source_for_ide_with_source_context(
            source.path.clone(),
            text,
            source_context,
        )
        .unwrap();
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
        let hir = crate::lowering::lower_program_with_semantics(&prepared.resolved, analysis.info)
            .unwrap();
        let mut hir = crate::complete_standalone_hir(hir, source, &context);
        hir.sources[0].scope = scope;
        hir
    }

    fn assert_php_output(text: &str, expected: &[u8]) {
        assert_php_output_in_scope(text, expected, crate::build_plan::SourceScope::Main);
    }

    fn assert_php_output_in_scope(
        text: &str,
        expected: &[u8],
        scope: crate::build_plan::SourceScope,
    ) {
        let hir = checked_hir(text, scope);
        let mir = lower_program(&hir).unwrap();
        crate::mir_validation::validate_program(&mir).unwrap();
        let php = crate::codegen_php::generate(&hir, Some(&mir)).unwrap();
        let directory = test_directory();
        let file = directory.join("program.php");
        std::fs::write(&file, &php).unwrap();
        let lint = std::process::Command::new("php")
            .arg("-l")
            .arg(&file)
            .output();
        let output = std::process::Command::new("php").arg(&file).output();
        std::fs::remove_dir_all(&directory).unwrap();
        let lint = lint.expect("PHP is required for interface compatibility parity");
        assert!(lint.status.success(), "{lint:?}\n{php}");
        let output = output.unwrap();
        assert!(output.status.success(), "{output:?}\n{php}");
        assert!(output.stderr.is_empty(), "{output:?}");
        assert_eq!(output.stdout, expected, "{php}");
    }

    #[test]
    fn php_interface_markers_preserve_checked_specializations_and_dispatch() {
        assert_php_output(
            r#"
interface Renderable { function render(): string; }
interface Read<T> { function read(): T; }
class Report implements Renderable, Read<int> {
    function __construct(string $title) {}
    function render(): string { return $this->title; }
    function read(): int { return 42; }
}
function renderReport(Renderable $report): string { return $report->render(); }
function inspect(mixed $value): void {
    if ($value is Read<int>) { echo $value->read(); }
    if ($value is Read<string>) { echo "wrong"; }
    echo match ($value) { Renderable $report => $report->render(), default => "absent" };
}
function main(): void {
    Renderable $report = new Report("report");
    echo renderReport($report) . "\n";
    inspect($report);
}
"#,
            b"report\n42report",
        );
    }

    #[test]
    fn interface_argument_temporaries_drop_on_success_and_checked_failure() {
        let source = include_str!(
            "../../../../examples/native/main_stage35_interface_argument_cleanup.doria"
        );
        let program = lower_checked_source(source);
        let expected = include_bytes!("../../tests/fixtures/native_io/main_stage35_interface_argument_cleanup/expected_stdout");
        let output = crate::mir_interpreter::interpret(&program).unwrap();
        assert_eq!(output.stdout, expected);
        assert_eq!(output.exit_status, 0);
        assert_native_output(&program, expected);
        assert_php_output(source, expected);
    }

    #[test]
    fn interface_arguments_reject_untracked_temporary_owners() {
        let program = lower_checked_source(
            r#"
interface Value { function read(): int; }
class Item implements Value { function read(): int { return 1; } }
function read(Value $value): void { echo $value->read(); }
function main(): void {
    read(new Item());
    let $callback = function(Value $value): void { read($value); };
    $callback(new Item());
}
"#,
        );
        for indirect in [false, true] {
            let mut changed = program.clone();
            let main = changed
                .functions
                .iter_mut()
                .find(|function| function.name == "main")
                .unwrap();
            let owner = main
                .blocks
                .iter()
                .flat_map(|block| &block.statements)
                .find_map(|statement| match statement {
                    mir::Statement::AssignLocal {
                        value: value @ mir::Rvalue::Interface(_),
                        ..
                    } if !value.borrows_move_value() => Some(value.clone()),
                    _ => None,
                })
                .unwrap();
            let args = main
                .blocks
                .iter_mut()
                .find_map(|block| match &mut block.terminator {
                    mir::Terminator::CheckedCall { args, .. } if !indirect => Some(args),
                    mir::Terminator::CheckedIndirectCall { args, .. } if indirect => Some(args),
                    _ => None,
                })
                .unwrap();
            args[0] = owner;
            let error = crate::mir_validation::validate_program(&changed).unwrap_err();
            assert!(
                error
                    .message
                    .contains("borrowed interface argument requires a tracked temporary owner"),
                "{error:?}"
            );
        }
    }

    #[test]
    fn checked_renderable_source_lowers_to_a_static_interface_entry() {
        let program = lower_checked_source(include_str!(
            "../../../../examples/native/main_stage35_interface_dispatch.doria"
        ));
        let output = crate::mir_interpreter::interpret(&program).unwrap();
        assert_eq!(output.stdout, b"report\n");
        assert_eq!(output.exit_status, 0);
        assert_native_output(&program, &output.stdout);
        assert!(program
            .functions
            .iter()
            .flat_map(|function| &function.blocks)
            .any(|block| matches!(
                block.terminator,
                mir::Terminator::CheckedIndirectCall {
                    callee: mir::IndirectCallee::InterfaceMethod { .. },
                    ..
                }
            )));
        assert!(!crate::codegen_cranelift::lower_mir_to_object(&program)
            .unwrap()
            .is_empty());
        #[cfg(feature = "llvm-backend")]
        assert!(!crate::codegen_llvm::lower_mir_to_object(&program)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn owned_interface_branch_results_preserve_temporary_lifetimes() {
        let text =
            include_str!("../../../../examples/native/main_stage35_interface_branch_results.doria");
        let expected = b"fallback;match;when;drop when;drop match;drop fallback;";
        let program = lower_checked_source(text);
        assert_eq!(
            crate::mir_interpreter::interpret(&program).unwrap().stdout,
            expected
        );
        assert_native_output(&program, expected);
        assert_php_output(text, expected);
    }

    #[test]
    fn consumed_interface_matches_and_closure_results_transfer_one_owner() {
        let text = include_str!(
            "../../../../examples/native/main_stage35_interface_consuming_match.doria"
        );
        let expected = b"first;fallback;closure;drop closure;drop fallback;drop first;";
        let program = lower_checked_source(text);
        assert_eq!(
            crate::mir_interpreter::interpret(&program).unwrap().stdout,
            expected
        );
        assert_native_output(&program, expected);
        assert_php_output(text, expected);
    }

    #[test]
    fn interface_cleanup_covers_loop_scopes_increments_and_given_finalizers() {
        let text = include_str!("../../../../examples/native/main_stage35_interface_cleanup.doria");
        let expected = b"setup;drop body;drop old;drop setup;iteration 1;drop iteration 1;iteration 2;drop iteration 2;given;finally given;drop given;done;drop new;";
        let program = lower_checked_source(text);
        assert_eq!(
            crate::mir_interpreter::interpret(&program).unwrap().stdout,
            expected
        );
        assert_native_output(&program, expected);
        assert_php_output(text, expected);
    }

    #[test]
    fn interface_constructor_failure_drops_initialized_fields_and_taken_parameters() {
        let text = include_str!(
            "../../../../examples/native/main_stage35_interface_constructor_failure.doria"
        );
        let expected = b"drop input;drop explicit;caught;constructed;drop second;drop implicit;";
        let program = lower_checked_source(text);
        assert_eq!(
            crate::mir_interpreter::interpret(&program).unwrap().stdout,
            expected
        );
        assert_native_output(&program, expected);
        assert_php_output(text, expected);
    }

    #[test]
    fn interface_pending_returns_are_dropped_when_finalizers_replace_them() {
        let text = include_str!(
            "../../../../examples/native/main_stage35_interface_finalizer_returns.doria"
        );
        let expected = b"finally return;drop return;caught return;finally when;drop when;caught when;finally retained;retained;drop retained;";
        let program = lower_checked_source(text);
        assert_eq!(
            crate::mir_interpreter::interpret(&program).unwrap().stdout,
            expected
        );
        assert_native_output(&program, expected);
        assert_php_output(text, expected);
    }

    #[test]
    fn erased_calls_preserve_writable_generic_and_dynamic_contracts() {
        let cases = [
            (
                r#"
interface Read<T> { function read(take T $fallback): T; }
class Reader<T> implements Read<T> { function read(take T $fallback): T { return $fallback; } }
function forward<T>(Read<T> $reader, take T $value): T { return $reader->read($value); }
function main(): void {
    Read<int> $reader = new Reader<int>();
    echo forward($reader, 42);
}
"#,
                "42",
            ),
            (
                r#"
interface Counter { writable function set(int $amount): void; function read(): int; }
class Count implements Counter {
    writable int $value = 0;
    writable function set(int $amount = 1): void { $this->value = $amount; }
    function read(): int { return $this->value; }
}
function bump(writable Counter $counter): int { $counter->set(amount: 4); return $counter->read(); }
function main(): void { writable Counter $counter = new Count(); echo bump($counter); }
"#,
                "4",
            ),
            (
                r#"
interface Identify { function identity<T>(take T $value): T; }
class Identity implements Identify { function identity<U>(take U $value): U { return $value; } }
function identity(Identify $value): int { return $value->identity(42); }
function main(): void { Identify $value = new Identity(); echo identity($value); }
"#,
                "42",
            ),
            (
                r#"
interface Read<T> { function read(take T $fallback): T; }
class Reader<T> implements Read<T> { function read(take T $fallback): T { return $fallback; } }
function main(): void { Read<int> $value = new Reader<int>(); echo $value->read(42); }
"#,
                "42",
            ),
            (
                r#"
interface Named { function name(): string; }
open class Base implements Named { open function name(): string { return "base"; } }
class Child extends Base { override function name(): string { return "child"; } }
class Inherited extends Base {}
function display(Named $value): void { echo $value->name(); }
function main(): void { Base $first = new Child(); Base $second = new Inherited(); display($first); display($second); }
"#,
                "childbase",
            ),
        ];
        for (source, expected) in cases {
            let program = lower_checked_source(source);
            let output = crate::mir_interpreter::interpret(&program).unwrap();
            assert_eq!(output.stdout, expected.as_bytes(), "{source}");
            assert_eq!(output.exit_status, 0);
            assert_native_output(&program, &output.stdout);
            assert_php_output(source, &output.stdout);
            assert!(!crate::codegen_cranelift::lower_mir_to_object(&program)
                .unwrap()
                .is_empty());
            #[cfg(feature = "llvm-backend")]
            assert!(!crate::codegen_llvm::lower_mir_to_object(&program)
                .unwrap()
                .is_empty());
        }
    }

    #[test]
    fn ancestor_interface_views_preserve_borrows_moves_and_dynamic_drop() {
        let program = lower_checked_source(include_str!(
            "../../../../examples/native/main_stage35_interface_ancestor_views.doria"
        ));
        let output = crate::mir_interpreter::interpret(&program).unwrap();
        assert_eq!(
            output.stdout,
            b"childchild42childnullpresentpresentdropdrop"
        );
        assert_eq!(output.exit_status, 0);
        assert_native_output(&program, &output.stdout);
        assert!(!crate::codegen_cranelift::lower_mir_to_object(&program)
            .unwrap()
            .is_empty());
        #[cfg(feature = "llvm-backend")]
        assert!(!crate::codegen_llvm::lower_mir_to_object(&program)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn generic_interface_calls_in_property_initializers_have_reachable_slots() {
        let program = lower_checked_source(include_str!(
            "../../../../examples/native/main_stage35_interface_property_call_slots.doria"
        ));
        let output = crate::mir_interpreter::interpret(&program).unwrap();
        assert_eq!(output.stdout, b"42");
        assert_eq!(output.exit_status, 0);
        assert!(!crate::codegen_cranelift::lower_mir_to_object(&program)
            .unwrap()
            .is_empty());
        #[cfg(feature = "llvm-backend")]
        assert!(!crate::codegen_llvm::lower_mir_to_object(&program)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn nullable_interface_calls_short_circuit_and_preserve_result_views() {
        let program = lower_checked_source(include_str!(
            "../../../../examples/native/main_stage35_interface_nullable_calls.doria"
        ));
        let output = crate::mir_interpreter::interpret(&program).unwrap();
        assert_eq!(output.stdout, b"absent-1argumentvalue422directfallback");
        assert_eq!(output.exit_status, 0);
        assert_native_output(&program, &output.stdout);
        assert!(!crate::codegen_cranelift::lower_mir_to_object(&program)
            .unwrap()
            .is_empty());
        #[cfg(feature = "llvm-backend")]
        assert!(!crate::codegen_llvm::lower_mir_to_object(&program)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn displayable_uses_canonical_conversion_once_in_each_display_context() {
        let program = lower_checked_source(include_str!(
            "../../../../examples/native/main_stage35_interface_display.doria"
        ));
        let output = crate::mir_interpreter::interpret(&program).unwrap();
        assert_eq!(output.stdout, b"convert:textconvert:[text]convert:concat:textconvert:format:textconvert:generic:textconvert:nullableconvert:generic:direct");
        assert_eq!(output.exit_status, 0);
        assert_native_output(&program, &output.stdout);
    }

    #[test]
    fn nominal_interface_narrowing_preserves_dynamic_identity() {
        let program = lower_checked_source(include_str!(
            "../../../../examples/native/main_stage35_interface_narrowing.doria"
        ));
        let output = crate::mir_interpreter::interpret(&program).unwrap();
        assert_eq!(
            output.stdout,
            b"42both7both42bothno-numberabsent-or-otherabsent-or-other42"
        );
        assert_eq!(output.exit_status, 0);
        assert_native_output(&program, &output.stdout);

        // Replacing a test by true must not authorize the projected view.
        let mut unproved = program.clone();
        let function = unproved
            .functions
            .iter_mut()
            .find(|function| function.name == "inspect")
            .unwrap();
        for block in &mut function.blocks {
            if let mir::Terminator::Branch { condition, .. } = &mut block.terminator {
                *condition = mir::BoolExpression::Use {
                    operand: mir::Operand::Scalar(mir::ScalarValue::Bool(true)),
                };
            }
        }
        let error = crate::mir_validation::validate_program(&unproved).unwrap_err();
        assert!(
            error.message.contains("dominating hierarchy `is` proof"),
            "{error:?}"
        );

        let mut invalidated = program;
        let function = invalidated
            .functions
            .iter_mut()
            .find(|function| function.name == "optional")
            .unwrap();
        let (local, then_block) = function
            .blocks
            .iter()
            .find_map(|block| match block.terminator {
                mir::Terminator::Branch {
                    condition: mir::BoolExpression::NominalIs { local, .. },
                    then_block,
                    ..
                } => Some((local, then_block)),
                _ => None,
            })
            .unwrap();
        let mir::Type::NullableInterface(interface) = function.locals[local.0].ty else {
            panic!("nullable interface parameter");
        };
        function.blocks[then_block.0].statements.insert(
            0,
            mir::Statement::AssignLocal {
                target: local,
                value: mir::Rvalue::nullable_interface(
                    interface,
                    mir::NullableInterfaceValue::Null,
                ),
            },
        );
        let error = crate::mir_validation::validate_program(&invalidated).unwrap_err();
        assert!(
            error.message.contains("dominating hierarchy `is` proof"),
            "{error:?}"
        );
    }

    #[test]
    fn interface_patterns_preserve_borrows_consumption_and_writability() {
        let program = lower_checked_source(include_str!(
            "../../../../examples/native/main_stage35_interface_patterns.doria"
        ));
        let output = crate::mir_interpreter::interpret(&program).unwrap();
        assert_eq!(
            output.stdout, b"1counter 1dropvalue!other-dropother",
            "{program}"
        );
        assert_eq!(output.exit_status, 0);
        assert_native_output(&program, &output.stdout);
    }

    #[test]
    fn mixed_interface_views_preserve_identity_and_single_ownership() {
        let program = lower_checked_source(include_str!(
            "../../../../examples/native/main_stage35_interface_mixed.doria"
        ));
        let output = crate::mir_interpreter::interpret(&program).unwrap();
        assert_eq!(
            output.stdout, b"value4277valuevalue4277drop;drop;number 42drop;number 42",
            "{program}"
        );
        assert_eq!(output.exit_status, 0);
        assert_native_output(&program, &output.stdout);
    }

    #[test]
    fn generic_closures_keep_distinct_callable_instances() {
        let source = include_str!(
            "../../../../examples/native/main_stage35_interface_generic_closures.doria"
        );
        let hir = checked_hir(source, crate::build_plan::SourceScope::Main);
        let program = lower_program(&hir).unwrap();
        crate::mir_validation::validate_program(&program).unwrap();
        let output = crate::mir_interpreter::interpret(&program).unwrap();
        assert_eq!(output.stdout, b"42true7");
        assert_native_output(&program, &output.stdout);
        assert_php_output(source, &output.stdout);
        assert_eq!(program.closure_descriptors.len(), 3);
        let plan = crate::php_closure::PhpClosurePlan::build(&hir, Some(&program));
        assert_eq!(plan.descriptors.len(), 3);
        for descriptor in &program.closure_descriptors {
            assert_eq!(
                plan.descriptor(descriptor.source_closure, Some(descriptor.source_instance))
                    .descriptor,
                descriptor.id
            );
        }
        let mut duplicate = program.clone();
        duplicate.closure_descriptors[1].source_instance =
            duplicate.closure_descriptors[0].source_instance;
        let error = crate::mir_validation::validate_program(&duplicate).unwrap_err();
        assert!(
            error.message.contains("duplicate source closure"),
            "{error:?}"
        );
    }

    #[test]
    fn php_closure_planning_walks_receivers_loop_clauses_and_finalizers() {
        let source = include_str!(
            "../../../../examples/native/main_stage35_interface_closure_lifetimes.doria"
        );
        let hir = checked_hir(source, crate::build_plan::SourceScope::Main);
        let program = lower_program(&hir).unwrap();
        crate::mir_validation::validate_program(&program).unwrap();
        let plan = crate::php_closure::PhpClosurePlan::build(&hir, Some(&program));
        assert_eq!(plan.closures.len(), hir.semantic_info.closures.len());
        assert_eq!(plan.closures.len(), 5);
        let output = crate::mir_interpreter::interpret(&program).unwrap();
        assert_eq!(output.stdout, b"1drop;3drop;4drop;");
        assert_native_output(&program, &output.stdout);
        assert_php_output(source, &output.stdout);
    }

    #[test]
    fn php_generic_properties_keep_interface_and_callable_specializations() {
        let source = include_str!(
            "../../../../examples/native/main_stage35_interface_generic_properties.doria"
        );
        let program = lower_checked_source(source);
        let output = crate::mir_interpreter::interpret(&program).unwrap();
        assert_eq!(output.stdout, b"8427drop;drop;");
        assert_native_output(&program, &output.stdout);
        assert_php_output(source, &output.stdout);
    }

    #[test]
    fn php_validation_uses_concrete_generic_shared_payloads() {
        let source =
            include_str!("../../../../examples/native/main_stage35_interface_generic_shared.doria");
        let program = lower_checked_source(source);
        let output = crate::mir_interpreter::interpret(&program).unwrap();
        assert_eq!(output.stdout, b"7drop;");
        assert_native_output(&program, &output.stdout);
        assert_php_output(source, &output.stdout);
    }

    #[test]
    fn php_validation_keeps_concrete_restrictions_inside_generic_bodies() {
        let source = r#"
function render<T implements Displayable>(T $value): void { echo $value; }
function main(): void { render(1.5); }
"#;
        let hir = checked_hir(source, crate::build_plan::SourceScope::Main);
        let program = lower_program(&hir).unwrap();
        crate::mir_validation::validate_program(&program).unwrap();
        let error = crate::codegen_php::generate(&hir, Some(&program)).unwrap_err();
        assert!(
            error
                .diagnostics
                .as_ref()
                .unwrap()
                .iter()
                .any(|diagnostic| diagnostic.message.contains("canonical float display")),
            "{error:?}"
        );
    }

    #[test]
    fn inherited_generic_interface_entries_keep_the_implementation_abi() {
        let source = include_str!(
            "../../../../examples/native/main_stage35_interface_inherited_generic.doria"
        );
        let program = lower_checked_source(source);
        let output = crate::mir_interpreter::interpret(&program).unwrap();
        assert_eq!(output.stdout, b"true7drop;");
        assert_native_output(&program, &output.stdout);
        assert_php_output(source, &output.stdout);
    }

    #[test]
    fn borrowed_interface_arguments_keep_branch_temporaries_until_the_call_finishes() {
        let source = r#"
interface Value { function number(): int; }
class Item implements Value {
    function __construct(int $value) {}
    function number(): int { return $this->value; }
    function __destruct(): void { try { echo "drop{$this->value};"; } catch (Error $error) {} }
}
function show(Value $value): void { echo $value->number(); }
function inspect(bool $flag): void {
    EXPRESSION;
    echo "done;";
}
function main(): void { inspect(true); inspect(false); }
"#;
        for (expression, expected) in [
            ("show(match ($flag) { true => new Item(1), false => new Item(2) })", "1drop1;done;2drop2;done;"),
            ("show(when ($flag): Item { return new Item(1); } else { return new Item(2); })", "1drop1;done;2drop2;done;"),
            ("show(when ($flag): Value { return new Item(1); } else { return new Item(2); } finally { try { echo \"finally;\"; } catch (Error) {} })", "finally;1drop1;done;finally;2drop2;done;"),
        ] {
        let source = source.replace("EXPRESSION", expression);
        let program = lower_checked_source(&source);
        let output = crate::mir_interpreter::interpret(&program).unwrap();
        assert_eq!(output.stdout, expected.as_bytes());
        assert_native_output(&program, &output.stdout);
        assert_php_output(&source, &output.stdout);
        }
    }

    #[test]
    fn interface_foreach_temporaries_drop_after_all_loop_exit_kinds() {
        let prefix = r#"
interface Value { function number(): int; }
class Item implements Value {
    function __construct(int $value) {}
    function number(): int { return $this->value; }
    function __destruct(): void { try { echo "drop{$this->value};"; } catch (Error) {} }
}
class Stop implements Error { string $message = "stop"; }
function items(): List<Value> { return [new Item(1), new Item(2)]; }
function values(): Dictionary<string, Value> { return ["a" => new Item(1), "b" => new Item(2)]; }
"#;
        for iterable in ["items()", "values()->values"] {
            for (exit, expected) in [
                ("", "12drop2;drop1;done;"),
                ("break;", "1drop2;drop1;done;"),
                ("throw new Stop();", "1drop2;drop1;caught;done;"),
                ("return;", "1drop2;drop1;done;"),
            ] {
                let body = format!(
                    "foreach ({iterable} as Value $item) {{ echo $item->number(); {exit} }}"
                );
                let body = if exit.starts_with("throw") {
                    format!("try {{ {body} }} catch (Stop) {{ echo \"caught;\"; }}")
                } else {
                    body
                };
                let source = format!("{prefix}\nfunction inspect(): void {{ {body} }}\nfunction main(): void {{ inspect(); echo \"done;\"; }}");
                let program = lower_checked_source(&source);
                let output = crate::mir_interpreter::interpret(&program).unwrap();
                assert_eq!(output.stdout, expected.as_bytes(), "{source}");
                assert_native_output(&program, &output.stdout);
                assert_php_output(&source, &output.stdout);
            }
        }
    }

    #[test]
    fn php_generic_interfaces_keep_specialized_ownership_and_closure_owners() {
        let source = include_str!(
            "../../../../examples/native/main_stage35_interface_generic_ownership.doria"
        );
        let program = lower_checked_source(source);
        let output = crate::mir_interpreter::interpret(&program).unwrap();
        assert_eq!(output.stdout, b"424237drop;typed;96drop;");
        assert_native_output(&program, &output.stdout);
        assert_php_output(source, &output.stdout);
    }

    #[test]
    fn nested_operand_calls_invalidate_type_proofs_in_evaluation_order() {
        for (ty, condition, message) in [
            ("?int", "$value != null", "dominating presence proof"),
            ("?Value", "$value != null", "dominating presence proof"),
            ("mixed", "$value is int", "dominating exact `is` proof"),
            ("Value", "$value is Item", "dominating hierarchy `is` proof"),
        ] {
            let source = format!(
                r#"
interface Value {{ function number(): int; }}
class Item implements Value {{ function number(): int {{ return 1; }} }}
function read(Item $item): int {{ return 1; }}
function readValue(Value $item): int {{ return 1; }}
function touch(writable {ty} $value): int {{ return 0; }}
function probe(writable {ty} $value): int {{ if ({condition}) {{ return 1; }} return 0; }}
function main(): void {{}}
"#
            );
            let original = lower_checked_source(&source);
            let touch = original
                .functions
                .iter()
                .find(|function| function.name == "touch")
                .unwrap()
                .id;
            let read = original
                .functions
                .iter()
                .find(|function| function.name == "read")
                .unwrap()
                .id;
            let read_value = original
                .functions
                .iter()
                .find(|function| function.name == "readValue")
                .unwrap()
                .id;
            let class = original
                .classes
                .iter()
                .find(|class| class.name == "Item")
                .unwrap()
                .id;
            for invalidates_first in [false, true] {
                let mut program = original.clone();
                let function = program
                    .functions
                    .iter_mut()
                    .find(|function| function.name == "probe")
                    .unwrap();
                let local = function
                    .locals
                    .iter()
                    .find(|local| local.name == "value")
                    .unwrap()
                    .clone();
                let then_block = function
                    .blocks
                    .iter()
                    .find_map(|block| match &block.terminator {
                        mir::Terminator::Branch { then_block, .. } => Some(*then_block),
                        _ => None,
                    })
                    .unwrap();
                let read = match ty {
                    "?int" => mir::IntegerExpression::Use {
                        ty: IntegerType::Int64,
                        operand: mir::Operand::NullablePayload(local.id),
                    },
                    "mixed" => mir::IntegerExpression::Use {
                        ty: IntegerType::Int64,
                        operand: mir::Operand::MixedPayload {
                            mixed: local.id,
                            tag: mir::MixedTag::Integer(IntegerType::Int64),
                        },
                    },
                    "?Value" => {
                        let mir::Type::NullableInterface(interface) = local.ty else {
                            unreachable!()
                        };
                        mir::IntegerExpression::Call {
                            ty: IntegerType::Int64,
                            function: read_value,
                            args: vec![mir::Rvalue::Interface(mir::InterfaceExpression {
                                interface,
                                value: mir::InterfaceValue::NullableLocalAssumeNonNull {
                                    local: local.id,
                                    transfer: false,
                                },
                            })],
                        }
                    }
                    _ => mir::IntegerExpression::Call {
                        ty: IntegerType::Int64,
                        function: read,
                        args: vec![mir::Rvalue::Class(mir::ClassExpression::InterfacePayload {
                            class,
                            local: local.id,
                            transfer: false,
                        })],
                    },
                };
                let call = mir::IntegerExpression::Call {
                    ty: IntegerType::Int64,
                    function: touch,
                    args: vec![local_rvalue(local.id, local.ty, false)],
                };
                let (left, right) = if invalidates_first {
                    (call, read)
                } else {
                    (read, call)
                };
                assert!(matches!(
                    function.blocks[then_block.0].terminator,
                    mir::Terminator::Return(_)
                ));
                function.blocks[then_block.0].terminator =
                    mir::Terminator::Return(mir::Rvalue::Value(mir::ValueExpression::Integer(
                        mir::IntegerExpression::Binary {
                            ty: IntegerType::Int64,
                            op: mir::IntegerBinaryOp::Add,
                            left: Box::new(left),
                            right: Box::new(right),
                            span: Span::new(0, 0),
                            right_span: Span::new(0, 0),
                        },
                    )));
                let result = crate::mir_validation::validate_program(&program);
                // Copy arguments do not expose the caller's nullable scalar place.
                if invalidates_first && ty != "?int" {
                    let error = result.unwrap_err();
                    assert!(error.message.contains(message), "{ty}: {error:?}");
                } else {
                    result.unwrap();
                }
            }
        }
    }

    #[test]
    fn indirect_result_writes_invalidate_previous_type_proofs() {
        for (ty, condition, value, expected) in [
            ("?int", "$value != null", "$value", "presence proof"),
            (
                "?string",
                "$value != null",
                "$value->length",
                "presence proof",
            ),
            (
                "?Item",
                "$value != null",
                "$value->number()",
                "presence proof",
            ),
            (
                "?Value",
                "$value != null",
                "$value->number()",
                "presence proof",
            ),
            (
                "Value",
                "$value is Item",
                "$value->number()",
                "hierarchy `is` proof",
            ),
            ("mixed", "$value is int", "$value", "exact `is` proof"),
        ] {
            let source = format!(
                r#"
interface Value {{ function number(): int; }}
class Item implements Value {{ function number(): int {{ return 1; }} }}
function probe(function(): {ty} $read): int {{
    writable {ty} $value = $read();
    if ({condition}) {{ $read(); return {value}; }}
    return 0;
}}
function main(): void {{}}
"#
            );
            let mut program = lower_checked_source(&source);
            let function = program
                .functions
                .iter_mut()
                .find(|function| function.name == "probe")
                .unwrap();
            let local = function
                .locals
                .iter_mut()
                .find(|local| local.name == "value")
                .unwrap();
            local.synthetic = true;
            let target = local.id;
            let mut calls = 0;
            for block in &mut function.blocks {
                match &mut block.terminator {
                    mir::Terminator::IndirectCall {
                        callee: mir::IndirectCallee::Closure(_),
                        result,
                        ..
                    }
                    | mir::Terminator::CheckedIndirectCall {
                        callee: mir::IndirectCallee::Closure(_),
                        result,
                        ..
                    } => {
                        calls += 1;
                        if calls == 2 {
                            *result = Some(target);
                        }
                    }
                    _ => {}
                }
            }
            assert_eq!(calls, 2, "{ty}");
            let error = crate::mir_validation::validate_program(&program).unwrap_err();
            assert!(error.message.contains(expected), "{ty}: {error:?}");
        }
    }

    #[test]
    fn erased_calls_reject_overlapping_argument_ownership() {
        let program = lower_checked_source(
            r#"
interface Value {
    writable function combine(writable Value $other): void;
    function inspect(Value $other): void;
    function consume(take Value $other): void;
}
class Item implements Value {
    writable function combine(writable Value $other): void {}
    function inspect(Value $other): void {}
    function consume(take Value $other): void {}
}
function main(): void {
    writable Value $first = new Item();
    writable Value $second = new Item();
    $first->combine($second);
    $first->inspect($second);
    $first->consume($second);
}
"#,
        );
        for (method_name, rejected) in [("combine", true), ("inspect", false), ("consume", true)] {
            let mut changed = program.clone();
            let mut replacements = 0;
            for function in &mut changed.functions {
                for block in &mut function.blocks {
                    let (mir::Terminator::IndirectCall { callee, args, .. }
                    | mir::Terminator::CheckedIndirectCall { callee, args, .. }) =
                        &mut block.terminator
                    else {
                        continue;
                    };
                    let mir::IndirectCallee::InterfaceMethod {
                        interface, slot, ..
                    } = callee
                    else {
                        continue;
                    };
                    if changed.interface_types[interface.0].methods[*slot].name != method_name {
                        continue;
                    }
                    args[1] = args[0].clone();
                    if method_name == "consume" {
                        let mir::Rvalue::Interface(mir::InterfaceExpression {
                            value: mir::InterfaceValue::Local { transfer, .. },
                            ..
                        }) = &mut args[1]
                        else {
                            panic!("expected interface argument");
                        };
                        *transfer = true;
                    }
                    replacements += 1;
                }
            }
            assert_eq!(replacements, 1);
            let result = crate::mir_validation::validate_program(&changed);
            if rejected {
                let error = result.unwrap_err();
                assert!(
                    error.message.contains("overlapping writable borrows")
                        || error.message.contains("both borrows and transfers")
                        || (method_name == "consume"
                            && error.message.contains("incompatible local")),
                    "{method_name}: {error:?}"
                );
            } else {
                result.unwrap();
            }
        }
    }

    #[test]
    fn interface_view_provenance_survives_synthetic_locals() {
        let program = lower_checked_source(
            r#"
interface Value { writable function combine(writable Value $other): void; }
class Item implements Value { writable function combine(writable Value $other): void {} }
function main(): void {
    writable Value $first = new Item();
    writable Value $second = new Item();
    $first->combine($second);
}
"#,
        );
        for end_owner in [false, true] {
            let mut changed = program.clone();
            let function = changed
                .functions
                .iter_mut()
                .find(|function| function.name == "main")
                .unwrap();
            let block = function
                .blocks
                .iter_mut()
                .find(|block| {
                    matches!(
                        block.terminator,
                        mir::Terminator::IndirectCall {
                            callee: mir::IndirectCallee::InterfaceMethod { .. },
                            ..
                        } | mir::Terminator::CheckedIndirectCall {
                            callee: mir::IndirectCallee::InterfaceMethod { .. },
                            ..
                        }
                    )
                })
                .unwrap();
            let (mir::Terminator::IndirectCall { callee, args, .. }
            | mir::Terminator::CheckedIndirectCall { callee, args, .. }) = &mut block.terminator
            else {
                unreachable!()
            };
            let mir::IndirectCallee::InterfaceMethod {
                receiver,
                interface,
                ..
            } = callee
            else {
                unreachable!()
            };
            let owner = *receiver;
            for (index, argument) in args.iter_mut().enumerate().take(2) {
                let id = mir::LocalId(function.locals.len());
                let mut view = function.locals[owner.0].clone();
                view.id = id;
                view.owned = false;
                view.synthetic = true;
                function.locals.push(view);
                block.statements.push(mir::Statement::AssignLocal {
                    target: id,
                    value: mir::Rvalue::Interface(mir::InterfaceExpression {
                        interface: *interface,
                        value: mir::InterfaceValue::Local {
                            local: owner,
                            transfer: false,
                        },
                    }),
                });
                *argument = mir::Rvalue::Interface(mir::InterfaceExpression {
                    interface: *interface,
                    value: mir::InterfaceValue::Local {
                        local: id,
                        transfer: false,
                    },
                });
                if index == 0 {
                    *receiver = id;
                }
            }
            if end_owner {
                let allocation = function
                    .locals
                    .iter()
                    .find(|local| local.owned && local.ty == mir::Type::Interface(*interface))
                    .unwrap()
                    .id;
                block
                    .statements
                    .push(mir::Statement::DropError { local: allocation });
            }
            let error = crate::mir_validation::validate_program(&changed).unwrap_err();
            assert!(
                error.message.contains(if end_owner {
                    "ownership ended"
                } else {
                    "overlapping writable borrows"
                }),
                "{error:?}"
            );
        }
    }

    #[test]
    fn interface_view_cannot_outlive_its_access_lease() {
        let mut program = lower_checked_source(
            r#"
interface Value { function number(): int; }
class Item implements Value { function number(): int { return 7; } }
function main(): void {
    let $owner = new WritableSharedReference<Value>(new Item());
    let $read = $owner->acquireReadonlyAccess();
    echo $read->number();
}
"#,
        );
        let function = program
            .functions
            .iter_mut()
            .find(|function| function.name == "main")
            .unwrap();
        let lease = function
            .locals
            .iter()
            .find(|local| {
                local.owned && matches!(local.ty, mir::Type::ReadonlySharedReferenceAccess(_))
            })
            .unwrap();
        let release = mir::Statement::DropSharedReferenceAccess {
            local: lease.id,
            payload: lease.ty.shared_payload().unwrap(),
            writable: false,
        };
        let block = function
            .blocks
            .iter_mut()
            .find(|block| {
                matches!(
                    block.terminator,
                    mir::Terminator::IndirectCall {
                        callee: mir::IndirectCallee::InterfaceMethod { .. },
                        ..
                    } | mir::Terminator::CheckedIndirectCall {
                        callee: mir::IndirectCallee::InterfaceMethod { .. },
                        ..
                    }
                )
            })
            .unwrap();
        block.statements.push(release);
        let error = crate::mir_validation::validate_program(&program).unwrap_err();
        assert!(error.message.contains("ownership ended"), "{error:?}");
    }

    #[test]
    fn borrowed_mixed_shells_keep_the_interface_access_lease() {
        let mut program = lower_checked_source(
            r#"
interface Value { function number(): int; }
class Item implements Value { function number(): int { return 7; } }
class Holder { function __construct(take Value $value) {} }
function inspect(mixed $value): void { if ($value is Value) { echo $value->number(); } }
function main(): void {
    let $owner = new WritableSharedReference<Holder>(new Holder(new Item()));
    let $read = $owner->acquireReadonlyAccess();
    inspect($read->value);
}
"#,
        );
        let inspect = program
            .functions
            .iter()
            .find(|function| function.name == "inspect")
            .unwrap()
            .id;
        let function = program
            .functions
            .iter_mut()
            .find(|function| function.name == "main")
            .unwrap();
        let lease = function
            .locals
            .iter()
            .find(|local| {
                local.owned && matches!(local.ty, mir::Type::ReadonlySharedReferenceAccess(_))
            })
            .unwrap();
        let release = mir::Statement::DropSharedReferenceAccess {
            local: lease.id,
            payload: lease.ty.shared_payload().unwrap(),
            writable: false,
        };
        let block = function
            .blocks
            .iter_mut()
            .find(|block| {
                matches!(block.terminator,
                    mir::Terminator::CheckedCall { function, .. } if function == inspect
                )
            })
            .unwrap();
        block.statements.push(release);
        let error = crate::mir_validation::validate_program(&program).unwrap_err();
        assert!(error.message.contains("ownership ended"), "{error:?}");
    }

    #[test]
    fn interface_view_lifetime_joins_do_not_revive_replaced_owners() {
        let mut program = lower_checked_source(
            r#"
interface Value { function number(): int; }
class Item implements Value { function number(): int { return 7; } }
function inspect(bool $replace): void {
    writable Value $value = new Item();
    if ($replace) { $value = new Item(); }
    echo $value->number();
}
function main(): void { inspect(true); inspect(false); }
"#,
        );
        let function = program
            .functions
            .iter_mut()
            .find(|function| function.name == "inspect")
            .unwrap();
        let original = function
            .locals
            .iter()
            .find(|local| {
                local.owned && !local.synthetic && matches!(local.ty, mir::Type::Interface(_))
            })
            .unwrap()
            .clone();
        let mir::Type::Interface(interface) = original.ty else {
            unreachable!()
        };
        let alias = mir::LocalId(function.locals.len());
        function.locals.push(mir::Local {
            id: alias,
            owned: false,
            synthetic: true,
            ..original.clone()
        });
        let mut inserted = false;
        let mut replaced = false;
        for block in &mut function.blocks {
            for statement in &mut block.statements {
                if let mir::Statement::AssignLocal {
                    value:
                        mir::Rvalue::Interface(mir::InterfaceExpression {
                            value:
                                mir::InterfaceValue::Local {
                                    local,
                                    transfer: false,
                                },
                            ..
                        }),
                    ..
                } = statement
                {
                    if *local == original.id {
                        *local = alias;
                        replaced = true;
                    }
                }
            }
            if !inserted {
                if let Some(index) = block.statements.iter().position(|statement| matches!(statement, mir::Statement::AssignLocal { target, .. } if *target == original.id)) {
                    block.statements.insert(index + 1, mir::Statement::AssignLocal { target: alias,
                        value: mir::Rvalue::Interface(mir::InterfaceExpression { interface, value: mir::InterfaceValue::Local { local: original.id, transfer: false } }),
                    });
                    inserted = true;
                }
            }
        }
        assert!(
            inserted && replaced,
            "inserted={inserted}, replaced={replaced}\n{program}"
        );
        let error = crate::mir_validation::validate_program(&program).unwrap_err();
        assert!(error.message.contains("ownership ended"), "{error:?}");
    }

    #[test]
    fn exact_dynamic_self_results_preserve_the_implementing_class() {
        let program = lower_checked_source(include_str!(
            "../../../../examples/native/main_stage35_interface_exact_self.doria"
        ));
        let output = crate::mir_interpreter::interpret(&program).unwrap();
        assert_eq!(output.stdout, b"123");
        assert_native_output(&program, &output.stdout);

        let base = program
            .classes
            .iter()
            .find(|class| class.name == "Base")
            .unwrap()
            .id;
        let child = program
            .classes
            .iter()
            .find(|class| class.name == "Child")
            .unwrap()
            .id;
        let mut wrong = program.clone();
        let child_constructor = wrong.classes[child.0].constructor;
        let mut changed = 0;
        for statement in wrong
            .functions
            .iter_mut()
            .flat_map(|function| &mut function.blocks)
            .flat_map(|block| &mut block.statements)
        {
            if let mir::Statement::AssignLocal {
                value:
                    mir::Rvalue::Class(mir::ClassExpression::New {
                        concrete_class,
                        constructor,
                        ..
                    }),
                ..
            } = statement
            {
                if *concrete_class == base {
                    *concrete_class = child;
                    *constructor = child_constructor;
                    changed += 1;
                }
            }
        }
        assert!(changed > 0);
        let error = crate::mir_validation::validate_program(&wrong).unwrap_err();
        assert!(
            error.message.contains("does not prove exact dynamic"),
            "{error:?}"
        );
    }

    #[test]
    fn erased_requirements_preserve_automatic_assertion_effects() {
        let source = r#"
use Doria\Std\Test\expect;
use Doria\Std\Test\AssertionError;
interface Probe { function run(): void; }
class Check implements Probe {
    function run(): void { expect(false)->toBeTrue(); }
}
function invoke(Probe $probe): void { $probe->run(); }
function main(): void {
    expect(function(): void { invoke(new Check()); })
        ->toThrow(function(AssertionError $error): void { echo "assertion;"; });
}
"#;
        let scope = crate::build_plan::SourceScope::Development;
        let program = lower_checked_source_in_scope(source, scope);
        let output = crate::mir_interpreter::interpret(&program).unwrap();
        assert_eq!(output.stdout, b"assertion;");
        assert_eq!(output.exit_status, 0);
        assert_native_output(&program, &output.stdout);
        assert_php_output_in_scope(source, &output.stdout, scope);
    }

    #[test]
    fn generic_error_inspectors_keep_their_concrete_assertion_contract() {
        let source = r#"
use Doria\Std\Test\expect;
interface Failure extends Error { function code(): int; }
class Failed<T> implements Failure {
    function __construct(string $message, take T $value) {}
    function code(): int { return 7; }
}
function inspect<T implements Error>(function(): void throws Error $operation,
    function(T): void $inspector): void {
    expect($operation)->toThrow($inspector);
}
function main(): void {
    inspect(function(): void { throw new Failed<int>("concrete", 1); },
        function(Failed<int> $error): void { echo "{$error->message}:{$error->value};"; });
    inspect(function(): void { throw new Failed<bool>("interface", true); },
        function(Failure $error): void { echo "{$error->message}:{$error->code()};"; });
}
"#;
        let scope = crate::build_plan::SourceScope::Development;
        let program = lower_checked_source_in_scope(source, scope);
        let output = crate::mir_interpreter::interpret(&program).unwrap();
        assert_eq!(output.stdout, b"concrete:1;interface:7;");
        assert_eq!(output.exit_status, 0);
        assert_native_output(&program, &output.stdout);
        assert_php_output_in_scope(source, &output.stdout, scope);
    }

    #[test]
    fn error_subinterfaces_preserve_effects_ordered_catches_and_rethrow() {
        let source = r#"
use Doria\Std\Test\expect;
interface Failure extends Error { function code(): int; }
interface Missing extends Failure {}
class Gone implements Missing {
    function __construct(string $message) {}
    function code(): int { return 404; }
    function __destruct() { try { echo "drop;"; } catch (Error) {} }
}
class Other implements Failure {
    function __construct(string $message) {}
    function code(): int { return 500; }
    function __destruct() { try { echo "other-drop;"; } catch (Error) {} }
}
function forward(take Failure $value): void throws Failure {
    try { throw $value; }
    catch (Missing $caught) { echo "inner {$caught->code()};"; throw $caught; }
}
interface Task { function run(): void throws Failure; }
class Job implements Task {
    function run(): void throws Missing { throw new Gone("job"); }
}
function invoke(Task $task): void throws Failure { $task->run(); }
function optionalFailure(): ?Failure { return new Gone("nullable"); }
function optionalMessage(?Error $error): ?string { return $error?->message; }
function main(): void {
    try { forward(new Gone("gone")); }
    catch (Missing $caught) { echo "{$caught->message};"; }
    catch (Failure $caught) { echo "wrong;"; }
    try { forward(new Other("other")); }
    catch (Missing $caught) { echo "wrong;"; }
    catch (Failure $caught) { echo "{$caught->message}:{$caught->code()};"; }
    try { invoke(new Job()); }
    catch (Failure $caught) { echo "{$caught->message};"; }
    expect(function(): void { throw new Gone("inspect"); })
        ->toThrow(function(Missing $caught): void { echo "{$caught->message};"; });
    SharedReference<Failure> $owner = shared new Gone("shared");
    echo "{$owner->message}:{$owner->code()};";
    ?SharedReference<Failure> $maybeOwner = $owner->share();
    ?SharedReference<Failure> $noOwner = null;
    echo ($maybeOwner?->message ?? "wrong") . ";" . ($noOwner?->message ?? "absent") . ";";
    ?Failure $maybeError = optionalFailure();
    ?Failure $noError = null;
    ?string $message = $owner->message;
    echo ($maybeError?->message ?? "wrong") . ";" . ($noError?->message ?? "absent") . ";";
    echo ($message ?? "wrong") . ";";
    echo (optionalMessage($maybeError) ?? "wrong") . ";";
}
"#;
        let program =
            lower_checked_source_in_scope(source, crate::build_plan::SourceScope::Development);
        let output = crate::mir_interpreter::interpret(&program).unwrap();
        assert_eq!(output.stdout, b"inner 404;gone;drop;other:500;other-drop;job;drop;inspect;drop;shared:404;shared;absent;nullable;absent;shared;nullable;drop;drop;", "{program}");
        assert_eq!(output.exit_status, 0);
        assert_native_output(&program, &output.stdout);
        assert_php_output_in_scope(
            source,
            &output.stdout,
            crate::build_plan::SourceScope::Development,
        );
        assert!(program.functions.iter().any(|function| function
            .required_checked_effects
            .iter()
            .any(|effect| matches!(effect, mir::CheckedEffect::Interface(_)))));
    }

    #[test]
    fn shared_interface_operations_reject_released_handle_roots() {
        for body in [
            "SharedReference<I> $root = shared new Item(); let $result = $root->share();",
            "SharedReference<I> $owner = shared new Item(); let $root = $owner->createWeakReference(); let $result = $root->acquire();",
            "let $root = new WritableSharedReference<I>(new Item()); let $result = $root->share();",
            "let $owner = new WritableSharedReference<I>(new Item()); let $root = $owner->createWeakReference(); let $result = $root->acquire();",
        ] {
            let source = format!("interface I {{}} class Item implements I {{}} function main(): void {{ {body} }}");
            let mut program = lower_checked_source(&source);
            let main = &mut program.functions[program.entry.0];
            let root = main.locals.iter().find(|local| local.name == "root").unwrap().id;
            let drop = main.blocks.iter().flat_map(|block| &block.statements).find(|statement| matches!(statement,
                mir::Statement::DropSharedReference { local, .. }
                | mir::Statement::DropWeakReference { local, .. }
                | mir::Statement::DropWritableSharedReference { local, .. }
                | mir::Statement::DropWritableWeakReference { local, .. } if *local == root)).unwrap().clone();
            let mut inserted = false;
            for block in &mut main.blocks {
                if let Some(index) = block.statements.iter().position(|statement| {
                    let mir::Statement::AssignLocal { value, .. } = statement else { return false; };
                    let Some(value) = crate::native_shared::Expression::from_rvalue(value) else { return false; };
                    let crate::native_shared::Operation::Runtime { value, .. } = value.operation() else { return false; };
                    matches!(value.operation(), crate::native_shared::Operation::Local { local, .. } if local == root)
                }) {
                    block.statements.insert(index, drop.clone());
                    inserted = true;
                    break;
                }
            }
            assert!(inserted, "{program}");
            let error = crate::mir_validation::validate_program(&program).unwrap_err();
            assert!(error.message.contains("ownership ended"), "{body}: {error:?}");
        }
    }

    #[test]
    fn shared_interface_views_survive_sharing_and_weak_expiration() {
        let source =
            include_str!("../../../../examples/native/main_stage35_interface_shared_weak.doria");
        let program = lower_checked_source(source);
        let output = crate::mir_interpreter::interpret(&program).unwrap();
        assert_eq!(output.stdout, b"livelivedrop;expired;drop;", "{program}");
        assert_eq!(output.exit_status, 0);
        assert_native_output(&program, &output.stdout);
        assert_php_output(source, &output.stdout);
    }

    #[test]
    fn shared_interface_access_leases_keep_the_view_and_release_once() {
        let source =
            include_str!("../../../../examples/native/main_stage35_interface_shared_access.doria");
        let program = lower_checked_source(source);
        let output = crate::mir_interpreter::interpret(&program).unwrap();
        assert_eq!(output.stdout, b"56drop;expired;drop;", "{program}");
        assert_eq!(output.exit_status, 0);
        assert_native_output(&program, &output.stdout);
        assert_php_output(source, &output.stdout);
    }

    #[test]
    fn shared_interface_storage_preserves_invariant_views() {
        let source =
            include_str!("../../../../examples/native/main_stage35_interface_shared_storage.doria");
        let program = lower_checked_source(source);
        let output = crate::mir_interpreter::interpret(&program).unwrap();
        assert_eq!(output.stdout, b"storedstoredstoredabsentdrop;", "{program}");
        assert_eq!(output.exit_status, 0);
        assert_native_output(&program, &output.stdout);
        assert_php_output(source, &output.stdout);
    }

    #[test]
    fn shared_interface_closures_and_nullable_paths_preserve_both_words() {
        let source = include_str!(
            "../../../../examples/native/main_stage35_interface_shared_closures.doria"
        );
        let program = lower_checked_source(source);
        let output = crate::mir_interpreter::interpret(&program).unwrap();
        assert_eq!(
            output.stdout, b"livelive7liveabsentfallbackdrop;livedrop;",
            "{program}"
        );
        assert_eq!(output.exit_status, 0);
        assert_native_output(&program, &output.stdout);
        assert_php_output(source, &output.stdout);
    }

    #[test]
    fn interface_storage_and_nullable_presence_keep_the_static_view() {
        let prefix = r#"
interface Named { function name(): string; }
class Name implements Named {
    function __construct(string $text) {}
    function name(): string { return $this->text; }
}
function display(?Named $value): void {
    if ($value != null) { echo $value->name(); } else { echo "absent"; }
}
"#;
        let cases = [
            (
                r#"
function maybe(bool $present): ?Named { if ($present) { return new Name("yes"); } return null; }
function main(): void {
    ?Named $none = null;
    if ($none == null) { echo "null"; } else { echo $none->name(); }
    ?Named $value = new Name("local");
    if ($value is Named) { echo $value->name(); }
    display(maybe(true)); display(maybe(false));
}
"#,
                "nulllocalyesabsent",
            ),
            (
                r#"
class Holder { function __construct(take Named $value) {} }
function main(): void {
    let $holder = new Holder(new Name("property"));
    display($holder->value);
    List<Named> $items = [new Name("list")];
    foreach ($items as Named $item) { display($item); }
    Dictionary<string, Named> $map = ["key" => new Name("dictionary")];
    display($map->get("key")); display($map->get("missing"));
}
"#,
                "propertylistdictionaryabsent",
            ),
        ];
        for (body, expected) in cases {
            let program = lower_checked_source(&format!("{prefix}{body}"));
            let output = crate::mir_interpreter::interpret(&program).unwrap();
            assert_eq!(output.stdout, expected.as_bytes());
            assert_eq!(output.exit_status, 0);
            assert_native_output(&program, &output.stdout);
            assert!(!crate::codegen_cranelift::lower_mir_to_object(&program)
                .unwrap()
                .is_empty());
            #[cfg(feature = "llvm-backend")]
            assert!(!crate::codegen_llvm::lower_mir_to_object(&program)
                .unwrap()
                .is_empty());
        }
    }
}

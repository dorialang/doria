//! Canonical return adapters and checked nominal conversions.

use crate::class_layout::ClassId;
use crate::diagnostics::{Diagnostic, DiagnosticResult};
use crate::mir;
use crate::mir::value_factory::{
    call_result_is_writable, direct_call_result, local_rvalue, nullable_present_rvalue,
    pending_owned_rvalue,
};
use crate::source::Span;

pub(crate) trait TypeFacts {
    fn interface_inherits(
        &self,
        source: mir::InterfaceTypeId,
        target: mir::InterfaceTypeId,
    ) -> bool;
    fn interface_vtable(
        &self,
        implementing: mir::ImplementingType,
        target: mir::InterfaceTypeId,
    ) -> Option<mir::InterfaceVtableId>;
}

impl TypeFacts for mir::Program {
    fn interface_inherits(
        &self,
        source: mir::InterfaceTypeId,
        target: mir::InterfaceTypeId,
    ) -> bool {
        self.interface_types
            .get(source.0)
            .is_some_and(|source| source.ancestors.contains(&target))
    }
    fn interface_vtable(
        &self,
        implementing: mir::ImplementingType,
        target: mir::InterfaceTypeId,
    ) -> Option<mir::InterfaceVtableId> {
        mir::Program::interface_vtable(self, implementing, target)
    }
}

fn required_class_vtable(
    facts: &impl TypeFacts,
    class: ClassId,
    interface: mir::InterfaceTypeId,
    span: Span,
) -> DiagnosticResult<mir::InterfaceVtableId> {
    facts
        .interface_vtable(mir::ImplementingType::Class(class), interface)
        .ok_or_else(|| {
            vec![Diagnostic::new(
                "I2401",
                "interface conversion has no checked conformance plan",
                span,
            )]
        })
}

fn unsupported(span: Span, message: &str) -> Diagnostic {
    Diagnostic::new("I2401", message, span)
}

pub(crate) fn convert_value(
    value: mir::Rvalue,
    target: mir::Type,
    span: Span,
    registry: &impl TypeFacts,
) -> DiagnosticResult<mir::Rvalue> {
    let payload = crate::native_abi::nullable_payload_type(target);
    let (target_type, nullable) = (payload.unwrap_or(target), payload.is_some());
    let mir::Type::Interface(interface) = target_type else {
        unreachable!("interface result conversion")
    };
    let upcast = |source: mir::InterfaceTypeId| {
        source == interface || registry.interface_inherits(source, interface)
    };
    let value = match value {
        value @ (mir::Rvalue::Collection(_)
        | mir::Rvalue::NullableCollection(_)
        | mir::Rvalue::Value(mir::ValueExpression::Inline(_))
        | mir::Rvalue::NullableScalar(_)) => {
            let payload = crate::native_abi::nullable_payload_type(value.ty());
            let (source, source_nullable) = (payload.unwrap_or(value.ty()), payload.is_some());
            if source_nullable && !nullable {
                return Err(vec![unsupported(
                    span,
                    "nullable value requires a presence proof before interface conversion",
                )]);
            }
            let implementing = source.interface_implementing_type().ok_or_else(|| {
                vec![unsupported(
                    span,
                    "value has no nominal interface implementation",
                )]
            })?;
            let vtable = registry
                .interface_vtable(implementing, interface)
                .ok_or_else(|| {
                    vec![unsupported(
                        span,
                        "value has no checked implementation of this interface",
                    )]
                })?;
            mir::InterfaceValue::FromValue {
                vtable,
                value: Box::new(value),
            }
        }
        mir::Rvalue::Class(object) => mir::InterfaceValue::FromClass {
            vtable: required_class_vtable(registry, object.class(), interface, span)?,
            object: Box::new(object),
        },
        mir::Rvalue::NullableClass(object) if nullable => {
            return Ok(mir::Rvalue::nullable_interface(
                interface,
                mir::NullableInterfaceValue::Present(mir::InterfaceValue::FromNullableClass {
                    vtable: required_class_vtable(registry, object.class(), interface, span)?,
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
            )]);
        }
    };
    Ok(if nullable {
        mir::Rvalue::nullable_interface(interface, mir::NullableInterfaceValue::Present(value))
    } else {
        mir::Rvalue::interface(interface, value)
    })
}

pub(crate) fn build_virtual_return_adapter(
    id: mir::FunctionId,
    slot: u32,
    return_type: mir::ReturnType,
    return_borrow: Option<mir::ReturnBorrow>,
    implementation: &mir::Function,
    direct: &mir::Function,
    registry: &impl TypeFacts,
) -> DiagnosticResult<mir::Function> {
    let direct_implementation = direct.id;
    let mut locals = Vec::with_capacity(implementation.params.len() + 2);
    let mut params = Vec::with_capacity(implementation.params.len());
    for parameter in &implementation.params {
        let source = implementation
            .locals
            .get(parameter.0)
            .expect("method parameter has a local definition");
        let local = mir::LocalId(locals.len());
        let mut definition = source.clone();
        definition.id = local;
        locals.push(definition);
        params.push(local);
    }
    let args = params
        .iter()
        .enumerate()
        .map(|(index, local)| {
            let ty = locals[local.0].ty;
            let transfer = implementation.parameter_modes[index]
                == mir::FunctionParameterMode::Take
                && ty.requires_owned_cleanup();
            local_rvalue(*local, ty, transfer)
        })
        .collect::<Vec<_>>();
    let span = implementation.source_span;
    let actual = match implementation.return_type {
        mir::ReturnType::Value(ty) => ty,
        mir::ReturnType::Void => {
            return Err(vec![unsupported(
                span,
                "a virtual return adapter requires a value-returning implementation",
            )]);
        }
    };
    let target = match return_type {
        mir::ReturnType::Value(ty) => ty,
        mir::ReturnType::Void => {
            return Err(vec![unsupported(
                span,
                "a virtual return adapter requires a value-returning contract",
            )]);
        }
    };

    let blocks = if direct.checked_effects.is_empty() {
        vec![mir::BasicBlock {
            id: mir::BlockId(0),
            statements: Vec::new(),
            terminator: mir::Terminator::Return(virtual_adapter_call_result(
                target,
                actual,
                direct_implementation,
                implementation.return_borrow,
                args,
                span,
                registry,
            )?),
        }]
    } else {
        if !implementation.has_checked_transport() {
            return Err(vec![unsupported(
                span,
                "a checked virtual implementation requires checked transport on its root contract",
            )]);
        }
        let result = mir::LocalId(locals.len());
        let owned = !actual.borrows_returned_value(implementation.return_borrow)
            && actual.has_move_ownership();
        locals.push(mir::Local {
            id: result,
            name: "__virtual_result".to_string(),
            ty: actual,
            writable: call_result_is_writable(owned, implementation.return_borrow),
            owned,
            synthetic: true,
        });
        let error = mir::LocalId(locals.len());
        locals.push(mir::Local {
            id: error,
            name: "__virtual_error".to_string(),
            ty: mir::Type::ERROR,
            writable: true,
            owned: true,
            synthetic: true,
        });
        vec![
            mir::BasicBlock {
                id: mir::BlockId(0),
                statements: Vec::new(),
                terminator: mir::Terminator::CheckedCall {
                    function: direct_implementation,
                    args,
                    result: Some(result),
                    error,
                    success: mir::BlockId(1),
                    failure: mir::BlockId(2),
                    span,
                },
            },
            mir::BasicBlock {
                id: mir::BlockId(1),
                statements: Vec::new(),
                terminator: mir::Terminator::Return(virtual_adapter_local_result(
                    result,
                    target,
                    actual,
                    !actual.borrows_returned_value(implementation.return_borrow),
                    span,
                    registry,
                )?),
            },
            mir::BasicBlock {
                id: mir::BlockId(2),
                statements: Vec::new(),
                terminator: mir::Terminator::PropagateError { error },
            },
        ]
    };

    // The adapter returns what the implementation returns, so it retains the
    // implementation's sources through its own identical parameters.
    let mut blocks = blocks;
    if let Some(plan) = direct.retained_sources_plan() {
        let forwarded = plan.forwarded(|local| {
            params[direct
                .params
                .iter()
                .position(|parameter| *parameter == local)
                .expect("retained input parameter")]
        });
        blocks[0].statements.insert(
            0,
            mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::RetainedSources(forwarded)),
        );
    }

    let mut method = implementation.method.clone();
    method
        .as_mut()
        .expect("virtual adapter belongs to a method")
        .return_adapter = Some(mir::VirtualReturnAdapter {
        implementation: implementation.id,
        direct: direct.id,
    });
    Ok(mir::Function {
        id,
        name: format!("{}::<virtual-return-adapter>", implementation.name),
        source_span: span,
        diagnostic_frame: mir::DiagnosticFrame::Forwarded(direct_implementation),
        method,
        virtual_slot: Some(slot),
        receiver_mode: implementation.receiver_mode,
        closure: None,
        params,
        parameter_modes: implementation.parameter_modes.clone(),
        return_type,
        return_borrow,
        required_checked_effects: implementation.required_checked_effects.clone(),
        ambient_checked_effects: implementation.ambient_checked_effects.clone(),
        test_assertion_checked_effects: implementation.test_assertion_checked_effects.clone(),
        checked_effects: implementation.checked_effects.clone(),
        locals,
        blocks,
        entry_block: mir::BlockId(0),
    })
}

fn virtual_adapter_call_result(
    target: mir::Type,
    actual: mir::Type,
    function: mir::FunctionId,
    return_borrow: Option<mir::ReturnBorrow>,
    args: Vec<mir::Rvalue>,
    span: Span,
    registry: &impl TypeFacts,
) -> DiagnosticResult<mir::Rvalue> {
    adapt_return_value(
        direct_call_result(actual, function, args, return_borrow, span),
        target,
        span,
        registry,
    )
}

fn virtual_adapter_local_result(
    local: mir::LocalId,
    target: mir::Type,
    actual: mir::Type,
    transfer: bool,
    span: Span,
    registry: &impl TypeFacts,
) -> DiagnosticResult<mir::Rvalue> {
    adapt_return_value(
        pending_owned_rvalue(local, actual, transfer),
        target,
        span,
        registry,
    )
}

pub(crate) fn adapt_return_value(
    value: mir::Rvalue,
    target: mir::Type,
    span: Span,
    registry: &impl TypeFacts,
) -> DiagnosticResult<mir::Rvalue> {
    let actual = value.ty();
    if actual == target {
        return Ok(value);
    }
    if matches!(
        target,
        mir::Type::Interface(_) | mir::Type::NullableInterface(_)
    ) {
        return convert_value(value, target, span, registry);
    }
    if crate::native_abi::nullable_payload_type(target) == Some(actual) {
        return Ok(nullable_present_rvalue(value));
    }
    match (target, value) {
        (mir::Type::Class(class), mir::Rvalue::Class(value)) => {
            Ok(mir::Rvalue::Class(mir::ClassExpression::Upcast {
                class,
                value: Box::new(value),
            }))
        }
        (mir::Type::NullableClass(class), mir::Rvalue::Class(value)) => {
            Ok(mir::Rvalue::NullableClass(
                mir::NullableClassExpression::Class(mir::ClassExpression::Upcast {
                    class,
                    value: Box::new(value),
                }),
            ))
        }
        (mir::Type::NullableClass(class), mir::Rvalue::NullableClass(value)) => Ok(
            mir::Rvalue::NullableClass(mir::NullableClassExpression::Upcast {
                class,
                value: Box::new(value),
            }),
        ),
        _ => Err(vec![Diagnostic::new(
            "I2401",
            "checked return has no native result conversion",
            span,
        )]),
    }
}

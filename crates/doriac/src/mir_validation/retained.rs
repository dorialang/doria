//! Validate retained source contracts independently of source ownership analysis.

use super::*;

pub(super) fn contract(function: &mir::Function) -> Option<&mir::RetainedSourcesPlan> {
    function
        .blocks
        .get(function.entry_block.0)?
        .statements
        .iter()
        .find_map(|statement| match statement {
            mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::RetainedSources(plan)) => {
                Some(plan)
            }
            _ => None,
        })
}

pub(super) fn validate_contract(
    program: &mir::Program,
    function: &mir::Function,
    plan: &mir::RetainedSourcesPlan,
) -> Result<(), BackendError> {
    if function
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter(|statement| {
            matches!(
                statement,
                mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::RetainedSources(_))
            )
        })
        .count()
        != 1
    {
        return Err(malformed_mir("retained source contract is duplicated"));
    }
    let mut sources = HashSet::new();
    for source in &plan.returns {
        let local = local_in(function, source.local)?;
        if !function.params.contains(&source.local)
            || !sources.insert(*source)
            || (!source.inherited && local.owned)
            || !matches!(function.return_type, mir::ReturnType::Value(ty) if ty.has_move_ownership())
        {
            return Err(malformed_mir(
                "retained return must name a unique live input source",
            ));
        }
    }
    let mut independent = HashSet::new();
    let mut constructed = HashSet::new();
    for source in &plan.constructs {
        let local = local_in(function, source.local)?;
        if function
            .method
            .as_ref()
            .is_none_or(|method| method.name != "__construct")
            || !function.params.contains(&source.local)
            || local.owned
            || local.writable
            || source.inherited
            || !constructed.insert(*source)
        {
            return Err(malformed_mir(
                "constructed iterator source must be a readonly constructor input",
            ));
        }
    }
    for parameter in &plan.independent_parameters {
        if !function.params.contains(parameter) || !independent.insert(*parameter) {
            return Err(malformed_mir(
                "retained source precondition names an invalid or duplicate parameter",
            ));
        }
    }
    let mut properties = HashSet::new();
    for (property, parameter) in &plan.promotions {
        let definition = class_in(program, property.class)?;
        let property = property_in(program, property.class, *property)?;
        let parameter = local_in(function, *parameter)?;
        if definition.constructor != Some(function.id)
            || !program.interface_vtables.iter().any(|table| {
                table.implementing_type == mir::ImplementingType::Class(property.id.class)
                    && program.interface_types[table.interface.0]
                        .methods
                        .iter()
                        .enumerate()
                        .any(|(slot, _)| {
                            iteration_operation(program, table.interface, slot)
                                == Some(
                                    crate::compiler_known_contracts::IterationOperation::GetCurrent,
                                )
                        })
            })
            || !property.borrowed_source
            || !function.params.contains(&parameter.id)
            || parameter.owned
            || parameter.writable
            || parameter.ty != property.ty
            || !properties.insert(property.id)
            || !plan.constructs.contains(&mir::RetainedSource {
                local: parameter.id,
                inherited: false,
            })
        {
            return Err(malformed_mir(
                "retained promotion must name its readonly constructor input",
            ));
        }
    }
    Ok(())
}

pub(super) fn carries_source(program: &mir::Program, ty: mir::Type) -> bool {
    match ty {
        mir::Type::Class(class) | mir::Type::NullableClass(class) => {
            program.classes.get(class.0).is_some_and(|class| {
                class
                    .properties
                    .iter()
                    .any(|property| property.borrowed_source)
            })
        }
        mir::Type::Interface(interface) | mir::Type::NullableInterface(interface) => program
            .interface_types
            .get(interface.0)
            .is_some_and(|definition| {
                definition.methods.iter().enumerate().any(|(slot, _)| {
                    iteration_operation(program, interface, slot)
                        == Some(crate::compiler_known_contracts::IterationOperation::HasCurrent)
                })
            }),
        _ => false,
    }
}

pub(super) fn iteration_operation(
    program: &mir::Program,
    interface: mir::InterfaceTypeId,
    slot: usize,
) -> Option<crate::compiler_known_contracts::IterationOperation> {
    let interface = program.interface_types.get(interface.0)?;
    interface.iteration_operation(slot, &program.interface_types)
}

pub(super) fn indirect_sources(
    program: &mir::Program,
    callee: &mir::IndirectCallee,
    args: &[mir::Rvalue],
    roots: &[HashSet<mir::LocalId>],
    loans: &[HashSet<mir::RetainedSource>],
) -> Result<HashSet<mir::RetainedSource>, BackendError> {
    let mir::IndirectCallee::InterfaceMethod {
        interface, slot, ..
    } = callee
    else {
        return Ok(HashSet::new());
    };
    let mut sources = HashSet::new();
    if iteration_operation(program, *interface, *slot)
        == Some(crate::compiler_known_contracts::IterationOperation::Acquire)
    {
        let receiver = args
            .first()
            .ok_or_else(|| malformed_mir("iterator acquisition has no receiver"))?;
        sources.extend(
            source_roots(program, receiver, roots)?
                .into_iter()
                .map(|local| mir::RetainedSource {
                    local,
                    inherited: false,
                }),
        );
    }
    for table in &program.interface_vtables {
        if table.interface == *interface {
            let entry = table
                .methods
                .get(*slot)
                .ok_or_else(|| malformed_mir("retained interface call has no method entry"))?;
            sources.extend(call_sources(program, *entry, args, false, roots, loans)?);
        }
    }
    Ok(sources)
}

fn possible_callees(
    program: &mir::Program,
    callee: mir::FunctionId,
) -> Result<Vec<&mir::Function>, BackendError> {
    let callee = function_in(program, callee)?;
    let mut candidates = vec![callee];
    if let (Some(slot), Some(method)) = (callee.virtual_slot, &callee.method) {
        for class in &program.classes {
            if class.id == method.class || class.ancestors.contains(&method.class) {
                let target = class
                    .virtual_methods
                    .get(slot as usize)
                    .ok_or_else(|| malformed_mir("retained virtual call has no method entry"))?;
                if !candidates.iter().any(|candidate| candidate.id == *target) {
                    candidates.push(function_in(program, *target)?);
                }
            }
        }
    }
    Ok(candidates)
}

pub(super) fn call_sources(
    program: &mir::Program,
    callee: mir::FunctionId,
    args: &[mir::Rvalue],
    constructing: bool,
    roots: &[HashSet<mir::LocalId>],
    loans: &[HashSet<mir::RetainedSource>],
) -> Result<HashSet<mir::RetainedSource>, BackendError> {
    let mut result = HashSet::new();
    for callee in possible_callees(program, callee)? {
        result.extend(direct_call_sources(
            program,
            callee,
            args,
            constructing,
            roots,
            loans,
        )?);
    }
    Ok(result)
}

fn direct_call_sources(
    program: &mir::Program,
    callee: &mir::Function,
    args: &[mir::Rvalue],
    constructing: bool,
    roots: &[HashSet<mir::LocalId>],
    loans: &[HashSet<mir::RetainedSource>],
) -> Result<HashSet<mir::RetainedSource>, BackendError> {
    let Some(plan) = contract(callee) else {
        return Ok(HashSet::new());
    };
    let sources = if constructing {
        plan.constructs.clone()
    } else {
        plan.returns.clone()
    };
    let mut result = HashSet::new();
    for source in sources {
        let index = callee
            .params
            .iter()
            .position(|parameter| *parameter == source.local)
            .ok_or_else(|| malformed_mir("retained call source is not a callee parameter"))?;
        let index = index
            .checked_sub(usize::from(constructing))
            .ok_or_else(|| malformed_mir("constructor retains its unconstructed receiver"))?;
        let value = args
            .get(index)
            .ok_or_else(|| malformed_mir("retained call source has no argument"))?;
        if source.inherited {
            result.extend(value_sources(program, value, roots, loans)?);
        } else if value.ty().has_move_ownership() {
            result.extend(
                source_roots(program, value, roots)?
                    .into_iter()
                    .map(|local| mir::RetainedSource {
                        local,
                        inherited: false,
                    }),
            );
        }
    }
    Ok(result)
}

fn source_roots(
    program: &mir::Program,
    value: &mir::Rvalue,
    roots: &[HashSet<mir::LocalId>],
) -> Result<HashSet<mir::LocalId>, BackendError> {
    let locals = if let Some(local) = value.direct_place_local() {
        vec![local]
    } else {
        escaping_class_local_borrows(program, value)?
            .into_iter()
            .collect()
    };
    if locals.is_empty() {
        return Err(malformed_mir(
            "retained source has no materialized lifetime owner",
        ));
    }
    Ok(locals
        .into_iter()
        .flat_map(|local| roots[local.0].iter().copied())
        .collect())
}

pub(super) fn value_sources(
    program: &mir::Program,
    value: &mir::Rvalue,
    roots: &[HashSet<mir::LocalId>],
    loans: &[HashSet<mir::RetainedSource>],
) -> Result<HashSet<mir::RetainedSource>, BackendError> {
    if let Some(local) = value.direct_place_local() {
        return Ok(loans[local.0].clone());
    }
    match value {
        mir::Rvalue::Interface(mir::InterfaceExpression {
            value: mir::InterfaceValue::NewCollectionIterator { source, .. },
            ..
        }) => Ok(roots[source.0]
            .iter()
            .map(|local| mir::RetainedSource {
                local: *local,
                inherited: false,
            })
            .collect()),
        mir::Rvalue::Interface(mir::InterfaceExpression {
            value: mir::InterfaceValue::FromCollection { value, .. },
            ..
        }) => value_sources(program, value, roots, loans),
        mir::Rvalue::Collection(
            mir::CollectionExpression::InterfaceReceiver { receiver, .. }
            | mir::CollectionExpression::IteratorSource { receiver, .. },
        ) => Ok(loans[receiver.0].clone()),
        mir::Rvalue::Class(mir::ClassExpression::New {
            constructor: Some(callee),
            args,
            ..
        }) => call_sources(program, *callee, args, true, roots, loans),
        mir::Rvalue::Class(mir::ClassExpression::Call {
            function: callee,
            args,
            ..
        })
        | mir::Rvalue::Function(mir::FunctionExpression::Call {
            function: callee,
            args,
            ..
        })
        | mir::Rvalue::NullableFunction(mir::NullableFunctionExpression::Call {
            function: callee,
            args,
            ..
        })
        | mir::Rvalue::NullableClass(mir::NullableClassExpression::Call {
            function: callee,
            args,
            ..
        })
        | mir::Rvalue::Interface(mir::InterfaceExpression {
            value:
                mir::InterfaceValue::Call {
                    function: callee,
                    args,
                    ..
                },
            ..
        })
        | mir::Rvalue::NullableInterface(mir::NullableInterfaceExpression {
            value:
                mir::NullableInterfaceValue::Call {
                    function: callee,
                    args,
                    ..
                },
            ..
        }) => call_sources(program, *callee, args, false, roots, loans),
        mir::Rvalue::NullableClass(mir::NullableClassExpression::Class(value)) => {
            value_sources(program, &mir::Rvalue::Class(value.clone()), roots, loans)
        }
        mir::Rvalue::Interface(mir::InterfaceExpression {
            value: mir::InterfaceValue::FromClass { object, .. },
            ..
        }) => value_sources(
            program,
            &mir::Rvalue::Class((**object).clone()),
            roots,
            loans,
        ),
        mir::Rvalue::Class(
            mir::ClassExpression::InterfaceReceiver {
                receiver: local, ..
            }
            | mir::ClassExpression::InterfacePayload { local, .. },
        ) => Ok(loans[local.0].clone()),
        mir::Rvalue::Class(mir::ClassExpression::Coalesce { left, right, .. }) => {
            let mut sources = value_sources(
                program,
                &mir::Rvalue::NullableClass((**left).clone()),
                roots,
                loans,
            )?;
            sources.extend(value_sources(
                program,
                &mir::Rvalue::Class((**right).clone()),
                roots,
                loans,
            )?);
            Ok(sources)
        }
        mir::Rvalue::Interface(mir::InterfaceExpression {
            value: mir::InterfaceValue::FromNullableClass { object, .. },
            ..
        }) => value_sources(
            program,
            &mir::Rvalue::NullableClass((**object).clone()),
            roots,
            loans,
        ),
        mir::Rvalue::Interface(mir::InterfaceExpression {
            value: mir::InterfaceValue::Upcast { source, .. },
            ..
        }) => value_sources(
            program,
            &mir::Rvalue::Interface((**source).clone()),
            roots,
            loans,
        ),
        mir::Rvalue::NullableInterface(mir::NullableInterfaceExpression {
            value: mir::NullableInterfaceValue::Upcast { source, .. },
            ..
        }) => value_sources(
            program,
            &mir::Rvalue::NullableInterface((**source).clone()),
            roots,
            loans,
        ),
        mir::Rvalue::NullableInterface(mir::NullableInterfaceExpression {
            interface,
            value: mir::NullableInterfaceValue::Present(value),
        }) => value_sources(
            program,
            &mir::Rvalue::Interface(mir::InterfaceExpression {
                interface: *interface,
                value: value.clone(),
            }),
            roots,
            loans,
        ),
        mir::Rvalue::Function(mir::FunctionExpression::Create { captures, .. }) => {
            let mut sources = HashSet::new();
            for capture in captures {
                match capture {
                    mir::ClosureCaptureOperand::BorrowLocal { local, .. } => {
                        sources.extend(loans[local.0].iter().copied())
                    }
                    mir::ClosureCaptureOperand::CopyValue(value)
                    | mir::ClosureCaptureOperand::MoveValue(value) => {
                        sources.extend(value_sources(program, value, roots, loans)?)
                    }
                }
            }
            Ok(sources)
        }
        mir::Rvalue::NullableFunction(mir::NullableFunctionExpression::Present(value)) => {
            value_sources(program, &mir::Rvalue::Function(value.clone()), roots, loans)
        }
        mir::Rvalue::Mixed(mir::MixedExpression::BoxClass { value, .. }) => {
            value_sources(program, &mir::Rvalue::Class(value.clone()), roots, loans)
        }
        mir::Rvalue::Mixed(mir::MixedExpression::BoxInterface { value, .. }) => value_sources(
            program,
            &mir::Rvalue::Interface((**value).clone()),
            roots,
            loans,
        ),
        mir::Rvalue::Mixed(mir::MixedExpression::BoxFunction { value, .. }) => value_sources(
            program,
            &mir::Rvalue::Function((**value).clone()),
            roots,
            loans,
        ),
        _ => Ok(HashSet::new()),
    }
}

pub(super) fn validate_return(
    function: &mir::Function,
    sources: &HashSet<mir::RetainedSource>,
) -> Result<(), BackendError> {
    for source in sources {
        let local = local_in(function, source.local)?;
        if !function.params.contains(&source.local)
            || (!source.inherited && local.owned)
            || contract(function).is_none_or(|plan| !plan.returns.contains(source))
        {
            return Err(malformed_mir(
                "owned return loses or outlives a retained source loan",
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_independent(
    function: &mir::Function,
    sources: &HashSet<mir::RetainedSource>,
) -> Result<(), BackendError> {
    for source in sources {
        if !source.inherited
            || contract(function)
                .is_none_or(|plan| !plan.independent_parameters.contains(&source.local))
        {
            return Err(malformed_mir(
                "retained source loan escapes into independent ownership",
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_call_inputs(
    program: &mir::Program,
    function: &mir::Function,
    target: CallTarget,
    args: &[mir::Rvalue],
    roots: &[HashSet<mir::LocalId>],
    loans: &[HashSet<mir::RetainedSource>],
) -> Result<(), BackendError> {
    let CallTarget::Direct {
        function: callee,
        parameter_offset,
    } = target
    else {
        return Ok(());
    };
    for callee in possible_callees(program, callee)? {
        let Some(plan) = contract(callee) else {
            continue;
        };
        for source in &plan.independent_parameters {
            let index = callee
                .params
                .iter()
                .position(|parameter| parameter == source)
                .and_then(|index| index.checked_sub(parameter_offset))
                .ok_or_else(|| malformed_mir("independent input has no caller argument"))?;
            let value = args
                .get(index)
                .ok_or_else(|| malformed_mir("independent input is missing"))?;
            validate_independent(function, &value_sources(program, value, roots, loans)?)?;
        }
    }
    Ok(())
}

pub(super) fn validate_indirect_inputs(
    program: &mir::Program,
    function: &mir::Function,
    terminator: &mir::Terminator,
    roots: &[HashSet<mir::LocalId>],
    loans: &[HashSet<mir::RetainedSource>],
) -> Result<(), BackendError> {
    let (callee, args) = match terminator {
        mir::Terminator::IndirectCall { callee, args, .. }
        | mir::Terminator::CheckedIndirectCall { callee, args, .. } => (callee, args),
        _ => return Ok(()),
    };
    let mir::IndirectCallee::InterfaceMethod {
        interface, slot, ..
    } = callee
    else {
        return Ok(());
    };
    for table in &program.interface_vtables {
        if table.interface == *interface {
            let entry = table
                .methods
                .get(*slot)
                .ok_or_else(|| malformed_mir("retained interface call has no method entry"))?;
            validate_call_inputs(
                program,
                function,
                CallTarget::Direct {
                    function: *entry,
                    parameter_offset: 0,
                },
                args,
                roots,
                loans,
            )?;
        }
    }
    Ok(())
}

use super::*;
use crate::compiler_known_contracts::IterationOperation;

pub(super) fn builtin_acquire(
    expr: &hir::Expr,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<Option<mir::Rvalue>> {
    let hir::Expr::MethodCall { object, span, .. } = expr else {
        return Ok(None);
    };
    let Some(CallableTarget::ConstrainedMethod { requirement, .. }) = context
        .semantic_info
        .call_targets
        .get(span)
        .and_then(|target| {
            target.specialize(|ty| substitute_resolved_type(ty, &context.type_substitutions))
        })
    else {
        return Ok(None);
    };
    if IterationOperation::from_requirement(requirement) != Some(IterationOperation::Acquire) {
        return Ok(None);
    }
    let mir::Type::Collection(collection) = context.expression_type(object)? else {
        return Ok(None);
    };
    let mir::Type::Interface(iterator) = context.expression_type(expr)? else {
        return Err(vec![unsupported(
            *span,
            "collection iterator has no native result type",
        )]);
    };
    let vtable = context
        .collection_registry
        .interface_vtable_ids
        .get(&(
            mir::ImplementingType::CollectionIterator(collection),
            iterator,
        ))
        .copied()
        .ok_or_else(|| {
            vec![unsupported(
                *span,
                "collection iterator has no canonical implementation",
            )]
        })?;
    let source = context.declare_borrowed_temp(mir::Type::Collection(collection), false);
    let value = lower_rvalue_as_borrowed(object, mir::Type::Collection(collection), context)?;
    context.push_statement(mir::Statement::AssignLocal {
        target: source,
        value,
    });
    Ok(Some(mir::Rvalue::interface(
        iterator,
        mir::InterfaceValue::NewCollectionIterator { source, vtable },
    )))
}

fn invoke(
    operation: IterationOperation,
    receiver: mir::LocalId,
    span: Span,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<(Option<mir::Rvalue>, mir::IterationCallPlan)> {
    let ty = context.local_type(receiver);
    let call = context.current_block.expect("live protocol call");
    let (interface, slot, target, result) = if let mir::Type::Interface(interface) = ty {
        let definition = &context.collection_registry.interface_types[interface.0];
        let (slot, method) = definition
            .methods
            .iter()
            .enumerate()
            .find(|(slot, _)| {
                definition.iteration_operation(*slot, &context.collection_registry.interface_types)
                    == Some(operation)
            })
            .ok_or_else(|| {
                vec![unsupported(
                    span,
                    "public iteration has no checked requirement slot",
                )]
            })?;
        let signature = context.collection_registry.function_types[method.signature.0].clone();
        let result = emit_indirect_call(
            mir::IndirectCallee::InterfaceMethod {
                receiver,
                interface,
                slot,
            },
            &signature,
            vec![local_rvalue(receiver, ty, false)],
            span,
            true,
            context,
        )?;
        (interface, slot, mir::CoreValueCallTarget::Erased, result)
    } else {
        let mir::Type::Class(class) = ty else {
            return Err(vec![unsupported(
                span,
                "public iteration requires a checked nominal receiver",
            )]);
        };
        let (vtable, interface, slot) = context
            .collection_registry
            .interface_vtables
            .iter()
            .filter(|table| table.implementing_type == mir::ImplementingType::Class(class))
            .find_map(|table| {
                let definition = &context.collection_registry.interface_types[table.interface.0];
                (0..definition.methods.len())
                    .find(|slot| {
                        definition.iteration_operation(
                            *slot,
                            &context.collection_registry.interface_types,
                        ) == Some(operation)
                    })
                    .map(|slot| (table.id, table.interface, slot))
            })
            .ok_or_else(|| {
                vec![unsupported(
                    span,
                    "public iteration has no nominal conformance proof",
                )]
            })?;
        let signature = context.lookup_method(class, operation.method(), span)?;
        let function = signature.id;
        let receiver_class = signature.method_class.unwrap_or(class);
        let args = vec![mir::Rvalue::Class(mir::ClassExpression::Local {
            class: receiver_class,
            local: receiver,
            transfer: false,
        })];
        let result = if !signature.checked_effects.is_empty() {
            materialize_checked_signature_call(signature, args, span, true, context)?
        } else {
            match signature.return_type {
                mir::ReturnType::Value(ty) => {
                    let value = interface::direct_call_result(
                        ty,
                        signature.id,
                        args,
                        signature.return_borrow,
                    );
                    let owned = user_local_type_owns_value(ty) && !value.borrows_move_value();
                    let local = context.declare_checked_call_slot(ty, owned);
                    context.push_statement(mir::Statement::AssignLocal {
                        target: local,
                        value,
                    });
                    Some((local, ty, owned))
                }
                mir::ReturnType::Void => {
                    context.push_statement(mir::Statement::CallVoid {
                        function: signature.id,
                        args,
                        span,
                    });
                    None
                }
            }
        };
        (
            interface,
            slot,
            mir::CoreValueCallTarget::Direct { function, vtable },
            result,
        )
    };
    let plan = mir::IterationCallPlan {
        operation,
        receiver,
        interface,
        slot,
        target,
        result: result.map(|result| result.0),
        call,
        success: context.current_block.expect("protocol success"),
    };
    Ok((
        result.map(|(local, ty, transfer)| local_rvalue(local, ty, transfer)),
        plan,
    ))
}

fn loop_local(
    value: mir::Rvalue,
    writable: bool,
    context: &mut LoweringContext<'_>,
) -> mir::LocalId {
    let ty = value.ty();
    let owned = user_local_type_owns_value(ty) && !value.borrows_move_value();
    let local = context.declare_checked_call_slot(ty, owned);
    context.locals[local.0].writable = writable;
    if owned {
        context
            .scope_owned_locals
            .last_mut()
            .expect("loop scope")
            .push(drop_obligation_for_owned_local(local, ty));
    }
    context.push_statement(mir::Statement::AssignLocal {
        target: local,
        value,
    });
    local
}

pub(super) fn lower_foreach(
    foreach: &hir::ForeachStmt,
    return_type: mir::ReturnType,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<()> {
    context.push_scope();
    let result = lower_foreach_in_scope(foreach, return_type, context);
    context.pop_scope();
    result
}

fn lower_foreach_in_scope(
    foreach: &hir::ForeachStmt,
    return_type: mir::ReturnType,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<()> {
    let span = foreach.span;
    let mut source = None;
    let mut cursor = None;
    let mut calls = Vec::new();
    lower_with_statement_temporaries(context, |context| {
        materialize_nested_collection_places(&foreach.iterable, false, context)?;
        let ty = context.expression_type(&foreach.iterable)?;
        let value = lower_rvalue_as_borrowed(&foreach.iterable, ty, context)?;
        let source_owners = std::mem::take(
            &mut context
                .statement_owned_locals
                .last_mut()
                .expect("source temporaries")
                .drops,
        );
        context
            .scope_owned_locals
            .last_mut()
            .expect("loop scope")
            .extend(source_owners);
        let local = loop_local(value, false, context);
        source = Some(local);
        let (value, plan) = invoke(IterationOperation::Acquire, local, span, context)?;
        calls.push(plan);
        let value =
            value.ok_or_else(|| vec![unsupported(span, "iterator acquisition returned void")])?;
        cursor = Some(loop_local(value, true, context));
        Ok(())
    })?;
    let source = source.expect("evaluated source");
    let cursor = cursor.expect("acquired iterator");
    let header = context.create_block();
    let body = context.create_block();
    let advance = context.create_block();
    let exit = context.create_block();
    context.terminate_current(mir::Terminator::Jump(header));
    context.current_block = Some(header);
    let mut condition = None;
    lower_with_statement_temporaries(context, |context| {
        let (value, plan) = invoke(IterationOperation::HasCurrent, cursor, span, context)?;
        calls.push(plan);
        condition = value;
        Ok(())
    })?;
    let Some(mir::Rvalue::Value(mir::ValueExpression::Bool(condition))) = condition else {
        return Err(vec![unsupported(span, "hasCurrent must return bool")]);
    };
    context.terminate_current(mir::Terminator::Branch {
        condition,
        then_block: body,
        else_block: exit,
    });

    context.current_block = Some(body);
    context.push_loop_targets(LoopTargets {
        continue_block: advance,
        break_block: exit,
        continue_cleanup_depth: context.local_scopes.len(),
        break_cleanup_depth: context.local_scopes.len(),
        continue_finalizer_depth: context.active_finalizers.len(),
        break_finalizer_depth: context.active_finalizers.len(),
        continue_sources: Vec::new(),
    });
    context.push_scope();
    let mut value_binding = None;
    lower_with_statement_temporaries(context, |context| {
        let (value, plan) = invoke(IterationOperation::GetCurrent, cursor, span, context)?;
        calls.push(plan);
        let value = value.ok_or_else(|| vec![unsupported(span, "getCurrent returned void")])?;
        let ty = context
            .semantic_info
            .foreach_loops
            .get(&span)
            .and_then(|plan| context.mir_resolved_type(&plan.value_binding_type))
            .ok_or_else(|| {
                vec![unsupported(
                    span,
                    "public iteration has no checked element type",
                )]
            })?;
        let value = if value.ty() == ty {
            value
        } else {
            let actual = value.ty();
            let temporary = context.declare_checked_call_slot(actual, false);
            context.push_statement(mir::Statement::AssignLocal {
                target: temporary,
                value,
            });
            virtual_adapter_local_result(temporary, ty, actual, false, span)?
        };
        let owned = user_local_type_owns_value(ty) && !ty.has_move_ownership();
        let target =
            context.declare_user_local_owned(&foreach.value_binding.name, false, ty, owned);
        value_binding = Some(target);
        context.push_statement(mir::Statement::AssignLocal { target, value });
        Ok(())
    })?;
    let body_result = lower_statement_sequence(&foreach.body.statements, return_type, context);
    context.pop_scope();
    let targets = context.pop_loop_targets();
    body_result?;
    if context.current_block.is_some() {
        context.terminate_current(mir::Terminator::Jump(advance));
    }
    context.current_block = Some(advance);
    lower_with_statement_temporaries(context, |context| {
        let (_, plan) = invoke(IterationOperation::Advance, cursor, span, context)?;
        calls.push(plan);
        Ok(())
    })?;
    context.terminate_current(mir::Terminator::Jump(header));
    context.current_block = Some(exit);
    let setup = calls[0].call;
    context.blocks[setup.0]
        .statements
        .push(mir::Statement::ControlFlowPlan(
            mir::ControlFlowPlan::PublicForeach(Box::new(mir::PublicForeachPlan {
                source,
                cursor,
                value_binding: value_binding.expect("element binding"),
                calls: calls.try_into().expect("four protocol operations"),
                header,
                body,
                update: advance,
                exit,
                continue_sources: targets.continue_sources,
                source_span: span,
            })),
        ));
    Ok(())
}

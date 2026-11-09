//! Byte-region shape and capability checks. Lifetime checking remains in the
//! shared retained-source dataflow, independently of descriptor ownership.
use super::*;

const INT: mir::Type = mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64));

pub(super) fn kind(ty: mir::Type) -> Option<mir::ByteRegionKind> {
    match ty {
        mir::Type::ByteRegion(kind) | mir::Type::NullableByteRegion(kind) => Some(kind),
        _ => None,
    }
}

pub(super) fn validate_expression(
    program: &mir::Program,
    function: &mir::Function,
    value: &mir::ByteRegionExpression,
) -> Result<(), BackendError> {
    use mir::ByteRegionExpression as E;
    match value {
        E::CollectionIndex {
            kind,
            collection,
            index,
            positional,
            remove,
            ..
        } => validate_collection_projection(
            program,
            function,
            *collection,
            index,
            mir::Type::ByteRegion(*kind),
            *positional,
            *remove,
        ),
        E::Region {
            kind,
            source,
            offset,
            length,
            ..
        } => {
            validate_rvalue(program, function, source)?;
            let source_kind = match source.ty() {
                mir::Type::Collection(id)
                    if collection_in(program, id)?.kind == mir::CollectionKind::Bytes =>
                {
                    None
                }
                mir::Type::ByteRegion(kind) => Some(kind),
                // `string->byteRegion`: the string's own UTF-8 bytes, readonly.
                mir::Type::String => Some(mir::ByteRegionKind::Readonly),
                _ => {
                    return Err(malformed_mir(
                        "byte region source is not Bytes, a string or a non-null byte region",
                    ));
                }
            };
            if source.ty().has_move_ownership() && !source.borrows_move_value() {
                return Err(malformed_mir(
                    "byte region formation must borrow its source",
                ));
            }
            if kind.is_writable() {
                if source_kind.is_some_and(|kind| !kind.is_writable()) {
                    return Err(malformed_mir(
                        "writable byte region borrows a readonly region or string",
                    ));
                }
                require_writable(program, function, source)?;
            }
            for bound in [offset, length] {
                if bound.ty() != INT {
                    return Err(malformed_mir("byte region offset and length must be int"));
                }
                validate_rvalue(program, function, bound)?;
            }
            Ok(())
        }
        E::Local {
            kind,
            local,
            transfer,
        } => validate_local(function, *kind, *local, *transfer, false),
        E::Call {
            kind,
            function: callee,
            args,
            return_borrow,
            ..
        } => validate_call(
            program,
            function,
            *kind,
            *callee,
            args,
            *return_borrow,
            false,
        ),
        E::AssumePresent { kind, value } => {
            if *kind != value.kind() {
                return Err(malformed_mir(
                    "byte region presence changes its access capability",
                ));
            }
            validate_nullable_expression(program, function, value)
        }
        E::Coalesce { kind, left, right } => {
            if *kind != left.kind() || *kind != right.kind() {
                return Err(malformed_mir(
                    "byte region coalesce changes its access capability",
                ));
            }
            validate_nullable_expression(program, function, left)?;
            validate_expression(program, function, right)
        }
    }
}

pub(super) fn validate_nullable_expression(
    program: &mir::Program,
    function: &mir::Function,
    value: &mir::NullableByteRegionExpression,
) -> Result<(), BackendError> {
    use mir::NullableByteRegionExpression as E;
    match value {
        E::CollectionIndex {
            kind,
            collection,
            index,
            positional,
            remove,
            ..
        } => validate_collection_projection(
            program,
            function,
            *collection,
            index,
            mir::Type::NullableByteRegion(*kind),
            *positional,
            *remove,
        ),
        E::CollectionGet {
            kind,
            collection,
            key,
            access,
            ..
        } => validate_dictionary_get(
            program,
            function,
            *collection,
            key,
            mir::Type::ByteRegion(*kind),
            *access,
        ),
        E::Null { .. } => Ok(()),
        E::Present(value) => validate_expression(program, function, value),
        E::Local {
            kind,
            local,
            transfer,
        } => validate_local(function, *kind, *local, *transfer, true),
        E::Call {
            kind,
            function: callee,
            args,
            return_borrow,
            ..
        } => validate_call(
            program,
            function,
            *kind,
            *callee,
            args,
            *return_borrow,
            true,
        ),
    }
}

fn validate_collection_projection(
    program: &mir::Program,
    function: &mir::Function,
    collection: mir::LocalId,
    index: &mir::Rvalue,
    expected: mir::Type,
    positional: bool,
    remove: bool,
) -> Result<(), BackendError> {
    let local = local_in(function, collection)?;
    let mir::Type::Collection(id) = local.ty else {
        return Err(malformed_mir(
            "byte region projection uses a non-collection local",
        ));
    };
    let definition = collection_in(program, id)?;
    if definition.value != expected {
        return Err(malformed_mir(
            "byte region collection projection changes access capability or nullability",
        ));
    }
    validate_collection_element_access(
        program, function, local, definition, index, remove, positional,
    )
}

fn validate_local(
    function: &mir::Function,
    kind: mir::ByteRegionKind,
    local: mir::LocalId,
    transfer: bool,
    nullable: bool,
) -> Result<(), BackendError> {
    let definition = local_in(function, local)?;
    let expected = if nullable {
        mir::Type::NullableByteRegion(kind)
    } else {
        mir::Type::ByteRegion(kind)
    };
    if definition.ty != expected {
        return Err(malformed_mir(
            "byte region local has a mismatched access capability or nullability",
        ));
    }
    if transfer && (!kind.is_writable() || !definition.owned) {
        return Err(malformed_mir(
            "byte region transfers a Copy or borrowed descriptor",
        ));
    }
    Ok(())
}

fn validate_call(
    program: &mir::Program,
    caller: &mir::Function,
    kind: mir::ByteRegionKind,
    callee: mir::FunctionId,
    args: &[mir::Rvalue],
    borrow: Option<mir::ReturnBorrow>,
    nullable: bool,
) -> Result<(), BackendError> {
    let callee = function_in(program, callee)?;
    let ty = if nullable {
        mir::Type::NullableByteRegion(kind)
    } else {
        mir::Type::ByteRegion(kind)
    };
    if callee.return_type != mir::ReturnType::Value(ty)
        || borrow != callee.return_borrow
        || borrow.is_some_and(|borrow| borrow.kind != mir::ReturnBorrowKind::Retained)
    {
        return Err(malformed_mir(
            "byte region call has inconsistent retained return contract",
        ));
    }
    validate_call_args(program, caller, callee, args)
}

pub(super) fn require_writable(
    program: &mir::Program,
    function: &mir::Function,
    value: &mir::Rvalue,
) -> Result<(), BackendError> {
    if let Some(collection) = retained::collection_element_source(value) {
        if !local_in(function, collection)?.writable {
            return Err(malformed_mir(
                "writable byte region projection uses a readonly collection",
            ));
        }
        return Ok(());
    }
    if let Some(local) = place(value) {
        if !local_in(function, local)?.writable {
            return Err(malformed_mir(
                "writable byte region access uses a readonly source binding",
            ));
        }
    } else if let mir::Rvalue::Collection(expression) = value {
        validate_collection_borrow_writability(program, function, expression, true)?;
    } else if !value.ty().has_move_ownership() || value.borrows_move_value() {
        return Err(malformed_mir(
            "writable byte region access lacks a writable source place",
        ));
    }
    Ok(())
}

pub(super) fn validate_assignment(
    program: &mir::Program,
    function: &mir::Function,
    local: &mir::Local,
    value: &mir::Rvalue,
) -> Result<(), BackendError> {
    if value.ty() != local.ty {
        return Err(malformed_mir(
            "byte region assignment changes access capability or nullability",
        ));
    }
    validate_rvalue(program, function, value)?;
    if let Some(kind) = kind(local.ty) {
        let expected_owned = kind.is_writable() && !value.borrows_move_value();
        if local.owned != expected_owned && !value.is_null_value() {
            return Err(malformed_mir(
                "byte region local has inconsistent descriptor ownership",
            ));
        }
        if kind.is_writable() && local.writable && value.borrows_move_value() {
            require_writable(program, function, value)?;
        }
    }
    Ok(())
}

pub(super) fn validate_index(
    program: &mir::Program,
    function: &mir::Function,
    local: mir::LocalId,
    index: &mir::Rvalue,
    writing: bool,
) -> Result<(), BackendError> {
    let definition = local_in(function, local)?;
    let mir::Type::ByteRegion(kind) = definition.ty else {
        return Err(malformed_mir(
            "byte region index requires a non-null region",
        ));
    };
    if writing && (!kind.is_writable() || !definition.writable) {
        return Err(malformed_mir(
            "byte region index write requires writable region access",
        ));
    }
    if index.ty() != INT {
        return Err(malformed_mir("byte region index must be int"));
    }
    validate_rvalue(program, function, index)
}

pub(super) fn collect_expression<'a>(
    value: &'a mir::ByteRegionExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    use mir::ByteRegionExpression as E;
    match value {
        E::CollectionIndex {
            collection,
            index,
            remove,
            ..
        } => {
            collect_rvalue_class_local_accesses(index, accesses);
            accesses.collection_access(*collection, *remove);
        }
        E::Region {
            kind,
            source,
            offset,
            length,
            ..
        } => {
            collect_rvalue_class_local_accesses(source, accesses);
            collect_rvalue_class_local_accesses(offset, accesses);
            collect_rvalue_class_local_accesses(length, accesses);
            accesses.retained_acquisition(source, kind.is_writable());
        }
        E::Local {
            local, transfer, ..
        } => {
            accesses.resource_read(*local);
            if *transfer {
                accesses.resource_transfer(*local);
            }
        }
        E::Call {
            function,
            span,
            args,
            ..
        } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args, Some(*span));
        }
        E::AssumePresent { value, .. } => {
            collect_nullable_expression(value, accesses);
            if let mir::NullableByteRegionExpression::Local { local, .. } = value.as_ref() {
                accesses.assume_nullable_present(*local);
            }
        }
        E::Coalesce { left, right, .. } => {
            collect_nullable_expression(left, accesses);
            accesses.conditional(
                nullable_presence(left, &HashSet::new()).map(|present| !present),
                |accesses| {
                    collect_expression(right, accesses);
                },
            );
        }
    }
}

pub(super) fn nullable_present(
    value: &mir::NullableByteRegionExpression,
    present: &HashSet<mir::LocalId>,
) -> bool {
    nullable_presence(value, present) == Some(true)
}

fn nullable_presence(
    value: &mir::NullableByteRegionExpression,
    present: &HashSet<mir::LocalId>,
) -> Option<bool> {
    match value {
        mir::NullableByteRegionExpression::Present(_) => Some(true),
        mir::NullableByteRegionExpression::Local { local, .. } => {
            present.contains(local).then_some(true)
        }
        mir::NullableByteRegionExpression::Null { .. } => Some(false),
        mir::NullableByteRegionExpression::Call { .. }
        | mir::NullableByteRegionExpression::CollectionIndex { .. }
        | mir::NullableByteRegionExpression::CollectionGet { .. } => None,
    }
}

/// The original owner and this intermediate carrier are different lifetimes:
/// a child preserves the owner's loan while suspending only its borrowed parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct ParentLoan {
    pub local: mir::LocalId,
    pub writable: bool,
}

pub(super) fn value_parents(
    program: &mir::Program,
    function: &mir::Function,
    value: &mir::Rvalue,
    parents: &[HashSet<ParentLoan>],
) -> Result<HashSet<ParentLoan>, BackendError> {
    if let Some(local) = value.direct_place_local() {
        return Ok(parents[local.0].clone());
    }
    match value {
        mir::Rvalue::ByteRegion(value) => match value {
            mir::ByteRegionExpression::CollectionIndex { collection, .. } => {
                Ok(parents[collection.0].clone())
            }
            mir::ByteRegionExpression::Region { kind, source, .. } => {
                reborrow_parents(program, function, source, kind.is_writable(), parents)
            }
            mir::ByteRegionExpression::Local { local, .. } => Ok(parents[local.0].clone()),
            mir::ByteRegionExpression::Call {
                function: callee,
                args,
                ..
            } => call_parents(program, function, *callee, args, parents),
            mir::ByteRegionExpression::AssumePresent { value, .. } => value_parents(
                program,
                function,
                &mir::Rvalue::NullableByteRegion((**value).clone()),
                parents,
            ),
            mir::ByteRegionExpression::Coalesce { left, right, .. } => {
                let mut result = value_parents(
                    program,
                    function,
                    &mir::Rvalue::NullableByteRegion((**left).clone()),
                    parents,
                )?;
                result.extend(value_parents(
                    program,
                    function,
                    &mir::Rvalue::ByteRegion((**right).clone()),
                    parents,
                )?);
                Ok(result)
            }
        },
        mir::Rvalue::NullableByteRegion(value) => match value {
            mir::NullableByteRegionExpression::CollectionIndex { collection, .. }
            | mir::NullableByteRegionExpression::CollectionGet { collection, .. } => {
                Ok(parents[collection.0].clone())
            }
            mir::NullableByteRegionExpression::Null { .. } => Ok(HashSet::new()),
            mir::NullableByteRegionExpression::Local { local, .. } => Ok(parents[local.0].clone()),
            mir::NullableByteRegionExpression::Present(value) => value_parents(
                program,
                function,
                &mir::Rvalue::ByteRegion(value.clone()),
                parents,
            ),
            mir::NullableByteRegionExpression::Call {
                function: callee,
                args,
                ..
            } => call_parents(program, function, *callee, args, parents),
        },
        mir::Rvalue::Function(mir::FunctionExpression::Create { captures, .. }) => {
            let mut result = HashSet::new();
            for capture in captures {
                match capture {
                    mir::ClosureCaptureOperand::BorrowLocal { local, writable } => {
                        result.extend(parents[local.0].iter().copied());
                        if kind(local_in(function, *local)?.ty).is_some() {
                            result.insert(ParentLoan {
                                local: *local,
                                writable: *writable,
                            });
                        }
                    }
                    mir::ClosureCaptureOperand::CopyValue(value)
                    | mir::ClosureCaptureOperand::MoveValue(value) => {
                        result.extend(value_parents(program, function, value, parents)?);
                    }
                }
            }
            Ok(result)
        }
        mir::Rvalue::Function(mir::FunctionExpression::Call {
            function: callee,
            args,
            ..
        })
        | mir::Rvalue::NullableFunction(mir::NullableFunctionExpression::Call {
            function: callee,
            args,
            ..
        }) => call_parents(program, function, *callee, args, parents),
        mir::Rvalue::NullableFunction(mir::NullableFunctionExpression::Present(value)) => {
            value_parents(
                program,
                function,
                &mir::Rvalue::Function(value.clone()),
                parents,
            )
        }
        _ => Ok(HashSet::new()),
    }
}

pub(super) fn reborrow_parents(
    program: &mir::Program,
    function: &mir::Function,
    value: &mir::Rvalue,
    writable: bool,
    parents: &[HashSet<ParentLoan>],
) -> Result<HashSet<ParentLoan>, BackendError> {
    let mut result = value_parents(program, function, value, parents)?;
    // Readonly descriptors are copied. Preserve their storage and ancestor
    // loans, while suspending only an exclusive parent view.
    if kind(value.ty()).is_some_and(|kind| kind.is_writable()) {
        if let Some(local) = place(value) {
            result.insert(ParentLoan { local, writable });
        }
    }
    Ok(result)
}

pub(super) fn place(value: &mir::Rvalue) -> Option<mir::LocalId> {
    value.direct_place_local().or_else(|| match value {
        mir::Rvalue::ByteRegion(mir::ByteRegionExpression::AssumePresent { value, .. }) => {
            match value.as_ref() {
                mir::NullableByteRegionExpression::Local { local, .. } => Some(*local),
                _ => None,
            }
        }
        mir::Rvalue::NullableByteRegion(mir::NullableByteRegionExpression::Present(value)) => {
            place(&mir::Rvalue::ByteRegion(value.clone()))
        }
        _ => None,
    })
}

pub(super) fn escaping(
    program: &mir::Program,
    value: &mir::Rvalue,
) -> Result<Vec<mir::LocalId>, BackendError> {
    match value {
        mir::Rvalue::ByteRegion(value) => match value {
            mir::ByteRegionExpression::CollectionIndex { collection, .. } => Ok(vec![*collection]),
            mir::ByteRegionExpression::Local {
                local, transfer, ..
            } => Ok(if *transfer { Vec::new() } else { vec![*local] }),
            mir::ByteRegionExpression::Region { source, .. } => {
                escaping_class_local_borrows(program, source)
            }
            mir::ByteRegionExpression::Call {
                function,
                args,
                return_borrow,
                ..
            } => return_borrow
                .map(|borrow| {
                    borrowed_call_rvalue_source(program, *function, args, borrow)
                        .and_then(|source| escaping_class_local_borrows(program, source))
                })
                .transpose()
                .map(Option::unwrap_or_default),
            mir::ByteRegionExpression::AssumePresent { value, .. } => {
                escaping(program, &mir::Rvalue::NullableByteRegion((**value).clone()))
            }
            mir::ByteRegionExpression::Coalesce { left, right, .. } => {
                let mut sources =
                    escaping(program, &mir::Rvalue::NullableByteRegion((**left).clone()))?;
                sources.extend(escaping(
                    program,
                    &mir::Rvalue::ByteRegion((**right).clone()),
                )?);
                Ok(sources)
            }
        },
        mir::Rvalue::NullableByteRegion(value) => match value {
            mir::NullableByteRegionExpression::CollectionIndex { collection, .. }
            | mir::NullableByteRegionExpression::CollectionGet { collection, .. } => {
                Ok(vec![*collection])
            }
            mir::NullableByteRegionExpression::Null { .. } => Ok(Vec::new()),
            mir::NullableByteRegionExpression::Present(value) => {
                escaping(program, &mir::Rvalue::ByteRegion(value.clone()))
            }
            mir::NullableByteRegionExpression::Local {
                local, transfer, ..
            } => Ok(if *transfer { Vec::new() } else { vec![*local] }),
            mir::NullableByteRegionExpression::Call {
                function,
                args,
                return_borrow,
                ..
            } => return_borrow
                .map(|borrow| {
                    borrowed_call_rvalue_source(program, *function, args, borrow)
                        .and_then(|source| escaping_class_local_borrows(program, source))
                })
                .transpose()
                .map(Option::unwrap_or_default),
        },
        _ => Err(malformed_mir(
            "region provenance requested for another carrier",
        )),
    }
}

/// Region return signatures are checked against their full retained-source plan;
/// the shared forward dataflow independently checks each actual returned value.
pub(super) fn return_contract(
    function: &mir::Function,
) -> Result<Option<mir::ReturnBorrow>, BackendError> {
    let Some(borrow) = function.return_borrow else {
        // Source elision remains the one-root rule in Decision 0089. The
        // complete retained plan is independently checked below the signature;
        // local branch joins preserve all candidate sources until return.
        return Ok(None);
    };
    if borrow.kind != mir::ReturnBorrowKind::Retained {
        return Err(malformed_mir(
            "byte region return must retain its source, not borrow its carrier",
        ));
    }
    let index = match borrow.source {
        mir::BorrowSource::Receiver => 0,
        mir::BorrowSource::Parameter(index) => {
            index
                + usize::from(function.receiver_mode.is_some())
                + usize::from(function.closure.is_some())
        }
    };
    let local = function
        .params
        .get(index)
        .ok_or_else(|| malformed_mir("byte region return source parameter is missing"))?;
    if retained::contract(function).is_none_or(|plan| {
        !plan.returns.iter().any(|source| {
            source.local == *local
                && (!source.writable || borrow.writable)
                && borrow.lifetime.accepts(source.lifetime)
        })
    }) {
        return Err(malformed_mir(
            "byte region return signature loses its retained source plan",
        ));
    }
    Ok(Some(borrow))
}

pub(super) fn inferred_borrow(
    program: &mir::Program,
    function: &mir::Function,
    value: &mir::Rvalue,
) -> Result<Option<mir::ReturnBorrow>, BackendError> {
    if value.is_null_value() {
        return Ok(None);
    }
    if let Some(local) = place(value) {
        if let Some(position) = function
            .params
            .iter()
            .position(|parameter| *parameter == local)
        {
            let implicit = usize::from(function.receiver_mode.is_some())
                + usize::from(function.closure.is_some());
            return Ok(Some(mir::ReturnBorrow {
                kind: mir::ReturnBorrowKind::Retained,
                lifetime: mir::RetainedLifetime::LastUse,
                source: if function.receiver_mode.is_some() && position == 0 {
                    mir::BorrowSource::Receiver
                } else {
                    mir::BorrowSource::Parameter(position.checked_sub(implicit).ok_or_else(
                        || malformed_mir("byte region cannot borrow a hidden closure environment"),
                    )?)
                },
                writable: kind(value.ty()).is_some_and(mir::ByteRegionKind::is_writable),
            }));
        }
        // The local's actual provenance is checked by the independent retained
        // dataflow at every return; use its enclosing callable contract here.
        return return_contract(function);
    }
    match value {
        mir::Rvalue::ByteRegion(mir::ByteRegionExpression::Region { kind, source, .. }) => Ok(
            infer_rvalue_return_borrow(program, function, source)?.map(|borrow| {
                mir::ReturnBorrow {
                    kind: mir::ReturnBorrowKind::Retained,
                    writable: kind.is_writable(),
                    ..borrow
                }
            }),
        ),
        mir::Rvalue::ByteRegion(mir::ByteRegionExpression::Call {
            function: callee,
            args,
            return_borrow: Some(borrow),
            ..
        })
        | mir::Rvalue::NullableByteRegion(mir::NullableByteRegionExpression::Call {
            function: callee,
            args,
            return_borrow: Some(borrow),
            ..
        }) => infer_borrowed_rvalue_source(program, function, *callee, args, *borrow),
        _ => return_contract(function),
    }
}

pub(super) fn call_parents(
    program: &mir::Program,
    function: &mir::Function,
    callee: mir::FunctionId,
    args: &[mir::Rvalue],
    parents: &[HashSet<ParentLoan>],
) -> Result<HashSet<ParentLoan>, BackendError> {
    let callee = function_in(program, callee)?;
    let Some(plan) = retained::contract(callee) else {
        return Ok(HashSet::new());
    };
    let mut result = HashSet::new();
    for source in &plan.returns {
        let index = callee
            .params
            .iter()
            .position(|local| *local == source.local)
            .ok_or_else(|| malformed_mir("retained parent is not a call parameter"))?;
        let value = args
            .get(index)
            .ok_or_else(|| malformed_mir("retained parent call argument is missing"))?;
        result.extend(
            if source.inherited && !local_in(callee, source.local)?.owned {
                reborrow_parents(program, function, value, source.writable, parents)?
            } else {
                value_parents(program, function, value, parents)?
            },
        );
    }
    Ok(result)
}

pub(super) fn collect_nullable_expression<'a>(
    value: &'a mir::NullableByteRegionExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    use mir::NullableByteRegionExpression as E;
    match value {
        E::CollectionIndex {
            collection,
            index,
            remove,
            ..
        } => {
            collect_rvalue_class_local_accesses(index, accesses);
            accesses.collection_access(*collection, *remove);
        }
        E::CollectionGet {
            collection,
            key,
            access,
            ..
        } => {
            collect_rvalue_class_local_accesses(key, accesses);
            accesses.collection_access(*collection, access.removes_element());
        }
        E::Null { .. } => {}
        E::Present(value) => collect_expression(value, accesses),
        E::Local {
            local, transfer, ..
        } => {
            accesses.resource_read(*local);
            if *transfer {
                accesses.resource_transfer(*local);
            }
        }
        E::Call {
            function,
            span,
            args,
            ..
        } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args, Some(*span));
        }
    }
}

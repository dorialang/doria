//! Claim ownership is independent of source-level Copy classification.

use super::*;
use crate::inline_value::InlineValueType;

fn validate_inline_use_mode(
    ty: InlineValueType,
    mode: mir::InlineValueUseMode,
) -> Result<(), BackendError> {
    if mode != mir::InlineValueUseMode::Borrow && !ty.requires_owned_cleanup() {
        return Err(malformed_mir(
            "explicit inline claim requires a managed Copy type",
        ));
    }
    Ok(())
}

fn validate_inline_handoff_local(
    function: &mir::Function,
    local: mir::LocalId,
) -> Result<(), BackendError> {
    let definition = local_in(function, local)?;
    if !definition.synthetic || !definition.owned {
        return Err(malformed_mir(
            "inline Copy handoff requires a synthetic owned local",
        ));
    }
    Ok(())
}

pub(super) fn validate_inline_claim_operand(
    function: &mir::Function,
    expression: &mir::InlineValueExpression,
) -> Result<(), BackendError> {
    let mir::InlineValueExpression::Use { ty, operand, mode } = expression else {
        return Ok(());
    };
    validate_inline_use_mode(*ty, *mode)?;
    if ty.requires_owned_cleanup() && matches!(operand, mir::Operand::Scalar(_)) {
        return Err(malformed_mir(
            "managed inline Copy state cannot be fabricated from constant bits",
        ));
    }
    if *mode == mir::InlineValueUseMode::TransferOwnedCopy {
        match operand {
            mir::Operand::Local(local) | mir::Operand::NullablePayload(local) => {
                validate_inline_handoff_local(function, *local)?;
            }
            mir::Operand::CollectionIndex { remove: true, .. } => {}
            _ => {
                return Err(malformed_mir(
                    "inline Copy handoff requires an owned local or removed element",
                ));
            }
        }
    } else if ty.requires_owned_cleanup()
        && matches!(operand, mir::Operand::CollectionIndex { remove: true, .. })
    {
        return Err(malformed_mir(
            "removed managed inline value must hand off its existing claim",
        ));
    }
    Ok(())
}

pub(super) fn validate_nullable_inline_claim(
    function: &mir::Function,
    ty: InlineValueType,
    value: &mir::NullableScalarExpression,
    mode: mir::InlineValueUseMode,
) -> Result<(), BackendError> {
    validate_inline_use_mode(ty, mode)?;
    if !ty.requires_owned_cleanup()
        || value.ty() != mir::ScalarType::Inline(ty)
        || value.owned_inline_temporary()
    {
        return Err(malformed_mir(
            "nullable inline claim requires a matching borrowed value",
        ));
    }
    if mode == mir::InlineValueUseMode::TransferOwnedCopy {
        let mir::NullableScalarExpression::Local { local, .. } = value else {
            return Err(malformed_mir(
                "nullable inline Copy handoff requires a local",
            ));
        };
        validate_inline_handoff_local(function, *local)?;
    }
    Ok(())
}

pub(super) fn validate_inline_assignment_ownership(
    local: &mir::Local,
    value: &mir::Rvalue,
) -> Result<(), BackendError> {
    if !matches!(
        local.ty,
        mir::Type::Scalar(mir::ScalarType::Inline(ty))
            | mir::Type::NullableScalar(mir::ScalarType::Inline(ty))
            if ty.requires_owned_cleanup()
    ) || value.is_null_value()
    {
        return Ok(());
    }
    if local.owned == value.borrows_move_value() {
        return Err(malformed_mir(format!(
            "inline Copy local local{} has inconsistent claim ownership",
            local.id.0
        )));
    }
    Ok(())
}

pub(super) fn require_owned_inline_claim(value: &mir::Rvalue) -> Result<(), BackendError> {
    if value.ty().managed_inline_carrier().is_some()
        && !value.is_null_value()
        && value.borrows_move_value()
    {
        return Err(malformed_mir(
            "owning inline Copy destination receives a borrowed claim",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inline_value::InlineValue;
    use mir::InlineValueUseMode::{Borrow, Copy, TransferOwnedCopy};

    const TOKEN: InlineValueType = InlineValueType::CancellationToken(ClassId(0));

    fn function() -> mir::Function {
        let program =
            crate::lower_source_to_mir("claim-proof.doria", "function main(): void {}").unwrap();
        let mut function = program.functions[program.entry.0].clone();
        function.locals.push(mir::Local {
            id: mir::LocalId(0),
            name: "claim".into(),
            ty: mir::Type::Scalar(mir::ScalarType::Inline(TOKEN)),
            writable: false,
            owned: true,
            synthetic: true,
        });
        function
    }

    fn use_local(mode: mir::InlineValueUseMode) -> mir::InlineValueExpression {
        mir::InlineValueExpression::Use {
            ty: TOKEN,
            operand: mir::Operand::Local(mir::LocalId(0)),
            mode,
        }
    }

    #[test]
    fn claim_modes_require_managed_types_and_handoffs_require_owned_temporaries() {
        for mode in [Copy, TransferOwnedCopy] {
            assert!(validate_inline_use_mode(InlineValueType::Duration, mode).is_err());
            assert!(validate_inline_use_mode(TOKEN, mode).is_ok());
        }
        assert!(validate_inline_claim_operand(&function(), &use_local(TransferOwnedCopy)).is_ok());
        for (owned, synthetic) in [(false, true), (true, false), (false, false)] {
            let mut function = function();
            function.locals[0].owned = owned;
            function.locals[0].synthetic = synthetic;
            assert!(
                validate_inline_claim_operand(&function, &use_local(TransferOwnedCopy)).is_err()
            );
            assert!(validate_inline_claim_operand(&function, &use_local(Copy)).is_ok());
        }
    }

    #[test]
    fn managed_state_cannot_be_fabricated_or_removed_without_handoff() {
        let function = function();
        for mode in [Borrow, Copy, TransferOwnedCopy] {
            let expression = mir::InlineValueExpression::Use {
                ty: TOKEN,
                operand: mir::Operand::Scalar(mir::ScalarValue::Inline(InlineValue {
                    ty: TOKEN,
                    bits: 1,
                })),
                mode,
            };
            assert!(validate_inline_claim_operand(&function, &expression).is_err());
            let removed = mir::InlineValueExpression::Use {
                ty: TOKEN,
                operand: mir::Operand::CollectionIndex {
                    collection: mir::LocalId(0),
                    index: Box::new(mir::Rvalue::Value(mir::ValueExpression::Integer(
                        mir::IntegerExpression::Use {
                            ty: IntegerType::Int64,
                            operand: mir::Operand::Scalar(mir::ScalarValue::Integer(
                                crate::numeric::IntegerValue {
                                    ty: IntegerType::Int64,
                                    bits: 0,
                                },
                            )),
                        },
                    ))),
                    index_span: Span::default(),
                    positional: false,
                    remove: true,
                },
                mode,
            };
            assert_eq!(
                validate_inline_claim_operand(&function, &removed).is_ok(),
                mode == TransferOwnedCopy
            );
        }
    }

    #[test]
    fn owning_storage_requires_a_copy_or_handoff_claim_including_nullable_values() {
        for mode in [Borrow, Copy, TransferOwnedCopy] {
            let value = mir::Rvalue::Value(mir::ValueExpression::Inline(use_local(mode)));
            assert_eq!(
                require_owned_inline_claim(&value).is_ok(),
                mode.owns_claim()
            );
            let borrowed = mir::NullableScalarExpression::Local {
                ty: mir::ScalarType::Inline(TOKEN),
                local: mir::LocalId(0),
            };
            let value = if mode == Borrow {
                borrowed
            } else {
                mir::NullableScalarExpression::InlineClaim {
                    ty: TOKEN,
                    value: Box::new(borrowed),
                    mode,
                }
            };
            assert_eq!(
                require_owned_inline_claim(&mir::Rvalue::NullableScalar(value)).is_ok(),
                mode.owns_claim(),
            );
        }
        assert!(require_owned_inline_claim(&mir::Rvalue::NullableScalar(
            mir::NullableScalarExpression::Null(mir::ScalarType::Inline(TOKEN)),
        ))
        .is_ok());
    }

    #[test]
    fn assignments_distinguish_owned_claims_borrowed_views_and_absence() {
        let mut local = function().locals[0].clone();
        for owned in [true, false] {
            local.owned = owned;
            for mode in [Borrow, Copy, TransferOwnedCopy] {
                let value = mir::Rvalue::Value(mir::ValueExpression::Inline(use_local(mode)));
                assert_eq!(
                    validate_inline_assignment_ownership(&local, &value).is_ok(),
                    owned == mode.owns_claim()
                );
                assert_eq!(value.direct_place_local().is_some(), mode == Borrow);
            }
            local.ty = mir::Type::NullableScalar(mir::ScalarType::Inline(TOKEN));
            let absent = mir::Rvalue::NullableScalar(mir::NullableScalarExpression::Null(
                mir::ScalarType::Inline(TOKEN),
            ));
            assert!(validate_inline_assignment_ownership(&local, &absent).is_ok());
        }
    }

    #[test]
    fn nullable_claims_cannot_retain_an_already_owned_expression() {
        let function = function();
        let borrowed = mir::NullableScalarExpression::Local {
            ty: mir::ScalarType::Inline(TOKEN),
            local: mir::LocalId(0),
        };
        assert!(validate_nullable_inline_claim(&function, TOKEN, &borrowed, Copy).is_ok());
        let owned = mir::NullableScalarExpression::InlineClaim {
            ty: TOKEN,
            value: Box::new(borrowed),
            mode: Copy,
        };
        assert!(validate_nullable_inline_claim(&function, TOKEN, &owned, Copy).is_err());
        let wrong =
            mir::NullableScalarExpression::Null(mir::ScalarType::Inline(InlineValueType::Duration));
        assert!(validate_nullable_inline_claim(&function, TOKEN, &wrong, Copy).is_err());
    }

    #[test]
    fn claim_handoffs_and_cleanup_end_both_plain_and_nullable_local_access() {
        let mut function = function();
        for nullable in [false, true] {
            function.locals[0].ty = if nullable {
                mir::Type::NullableScalar(mir::ScalarType::Inline(TOKEN))
            } else {
                mir::Type::Scalar(mir::ScalarType::Inline(TOKEN))
            };
            let borrowed = if nullable {
                mir::Rvalue::NullableScalar(mir::NullableScalarExpression::Local {
                    ty: mir::ScalarType::Inline(TOKEN),
                    local: mir::LocalId(0),
                })
            } else {
                mir::Rvalue::Value(mir::ValueExpression::Inline(use_local(Borrow)))
            };
            let mut accesses = ClassLocalAccesses::default();
            collect_rvalue_class_local_accesses(&borrowed, &mut accesses);
            let mut ended = HashSet::new();
            apply_class_local_accesses(&function, &accesses, &mut ended, true).unwrap();
            apply_class_local_state(
                &function,
                &mir::Statement::DropInlineValue {
                    local: mir::LocalId(0),
                    ty: TOKEN,
                    nullable,
                },
                &mut ended,
                &[],
                true,
            )
            .unwrap();
            assert!(apply_class_local_accesses(&function, &accesses, &mut ended, true).is_err());
            ended.clear();
            let mut handoff = ClassLocalAccesses::default();
            handoff.transfer(mir::LocalId(0));
            apply_class_local_accesses(&function, &handoff, &mut ended, true).unwrap();
            assert!(apply_class_local_accesses(&function, &accesses, &mut ended, true).is_err());
        }
    }
}

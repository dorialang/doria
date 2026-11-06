//! Typed MIR values preserve one ownership and nullable-carrier construction path.

use crate::mir;
use crate::numeric::{FloatType, IntegerType};
use crate::source::Span;

pub(crate) fn direct_call_result(
    ty: mir::Type,
    function: mir::FunctionId,
    args: Vec<mir::Rvalue>,
    return_borrow: Option<mir::ReturnBorrow>,
    span: Span,
) -> mir::Rvalue {
    use mir::*;
    match ty {
        Type::ByteRegion(kind) => Rvalue::ByteRegion(ByteRegionExpression::Call {
            span,
            kind,
            function,
            args,
            return_borrow,
        }),
        Type::NullableByteRegion(kind) => {
            Rvalue::NullableByteRegion(NullableByteRegionExpression::Call {
                span,
                kind,
                function,
                args,
                return_borrow,
            })
        }
        Type::ClosureEnvironment(_) => {
            unreachable!("closure environments are not source return types")
        }
        Type::Scalar(ScalarType::Integer(ty)) => {
            Rvalue::Value(ValueExpression::Integer(IntegerExpression::Call {
                span,
                ty,
                function,
                args,
            }))
        }
        Type::Scalar(ScalarType::Float(ty)) => {
            Rvalue::Value(ValueExpression::Float(FloatExpression::Call {
                span,
                ty,
                function,
                args,
            }))
        }
        Type::Scalar(ScalarType::Bool) => {
            Rvalue::Value(ValueExpression::Bool(BoolExpression::Call {
                span,
                function,
                args,
            }))
        }
        Type::Scalar(ScalarType::Enum(enum_id)) => {
            Rvalue::Value(ValueExpression::Enum(EnumExpression::Call {
                span,
                enum_id,
                function,
                args,
            }))
        }
        Type::Scalar(ScalarType::Inline(ty)) => {
            Rvalue::Value(ValueExpression::Inline(InlineValueExpression::Call {
                ty,
                function,
                args,
                span,
            }))
        }
        Type::NullableScalar(ty) => Rvalue::NullableScalar(NullableScalarExpression::Call {
            ty,
            function,
            args,
            span,
        }),
        Type::String => Rvalue::String(StringExpression::Call {
            span,
            function,
            args,
        }),
        Type::NullableString => Rvalue::NullableString(NullableStringExpression::Call {
            span,
            function,
            args,
        }),
        Type::Class(class) => Rvalue::Class(ClassExpression::Call {
            class,
            function,
            args,
            return_borrow,
            span,
        }),
        Type::NullableClass(class) => Rvalue::NullableClass(NullableClassExpression::Call {
            class,
            function,
            args,
            return_borrow,
            span,
        }),
        Type::Interface(interface) => Rvalue::Interface(InterfaceExpression {
            interface,
            value: InterfaceValue::Call {
                function,
                args,
                return_borrow,
                span,
            },
        }),
        Type::NullableInterface(interface) => {
            Rvalue::NullableInterface(NullableInterfaceExpression {
                interface,
                value: NullableInterfaceValue::Call {
                    function,
                    args,
                    return_borrow,
                    span,
                },
            })
        }
        Type::Collection(collection) => Rvalue::Collection(CollectionExpression::Call {
            span,
            collection,
            function,
            args,
            return_borrow,
        }),
        Type::NullableCollection(collection) => {
            Rvalue::NullableCollection(NullableCollectionExpression::Call {
                span,
                collection,
                function,
                args,
                return_borrow,
            })
        }
        Type::Function(function_type) => Rvalue::Function(FunctionExpression::Call {
            span,
            function_type,
            function,
            args,
            return_borrow,
        }),
        Type::NullableFunction(function_type) => {
            Rvalue::NullableFunction(NullableFunctionExpression::Call {
                span,
                function_type,
                function,
                args,
                return_borrow,
            })
        }
        Type::Mixed => Rvalue::Mixed(MixedExpression::Call {
            span,
            function,
            args,
            return_borrow,
        }),
        Type::NullableMixed => Rvalue::NullableMixed(NullableMixedExpression::Call {
            span,
            function,
            args,
            return_borrow,
        }),
        Type::PayloadEnum(ty) => Rvalue::PayloadEnum(PayloadEnumExpression::Call {
            span,
            ty,
            function,
            args,
            return_borrow,
        }),
        Type::NullablePayloadEnum(ty) => {
            Rvalue::NullablePayloadEnum(NullablePayloadEnumExpression::Call {
                span,
                ty,
                function,
                args,
                return_borrow,
            })
        }
        Type::SharedReference(class) => Rvalue::SharedReference(SharedReferenceExpression::Call {
            span,
            payload: class,
            function,
            args,
            return_borrow,
        }),
        Type::WeakReference(class) => Rvalue::WeakReference(WeakReferenceExpression::Call {
            span,
            payload: class,
            function,
            args,
            return_borrow,
        }),
        Type::NullableSharedReference(class) => {
            Rvalue::NullableSharedReference(NullableSharedReferenceExpression::Call {
                span,
                payload: class,
                function,
                args,
                return_borrow,
            })
        }
        Type::NullableWeakReference(class) => {
            Rvalue::NullableWeakReference(NullableWeakReferenceExpression::Call {
                span,
                payload: class,
                function,
                args,
                return_borrow,
            })
        }
        Type::WritableSharedReference(payload) => {
            Rvalue::WritableSharedReference(WritableSharedReferenceExpression::Call {
                span,
                payload,
                function,
                args,
                return_borrow,
            })
        }
        Type::WritableWeakReference(payload) => {
            Rvalue::WritableWeakReference(WritableWeakReferenceExpression::Call {
                span,
                payload,
                function,
                args,
                return_borrow,
            })
        }
        Type::NullableWritableSharedReference(payload) => Rvalue::NullableWritableSharedReference(
            NullableWritableSharedReferenceExpression::Call {
                span,
                payload,
                function,
                args,
                return_borrow,
            },
        ),
        Type::NullableWritableWeakReference(payload) => {
            Rvalue::NullableWritableWeakReference(NullableWritableWeakReferenceExpression::Call {
                span,
                payload,
                function,
                args,
                return_borrow,
            })
        }
        Type::ReadonlySharedReferenceAccess(payload)
        | Type::WritableSharedReferenceAccess(payload) => {
            Rvalue::SharedReferenceAccess(SharedReferenceAccessExpression::Call {
                span,
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
                span,
                payload,
                function,
                args,
                return_borrow,
                writable: matches!(ty, Type::NullableWritableSharedReferenceAccess(_)),
            })
        }
    }
}

pub(crate) fn pending_owned_rvalue(
    local: mir::LocalId,
    ty: mir::Type,
    transfer: bool,
) -> mir::Rvalue {
    // Cleanup-free Copy results still leave their temporary as values, not storage loans.
    let copy = matches!(ty, mir::Type::PayloadEnum(ty) | mir::Type::NullablePayloadEnum(ty)
        if ty.capabilities.trivial_copy);
    let mut value = local_rvalue(local, ty, transfer || copy);
    if transfer {
        match &mut value {
            mir::Rvalue::PayloadEnum(mir::PayloadEnumExpression::Use { ty, mode, .. })
            | mir::Rvalue::NullablePayloadEnum(mir::NullablePayloadEnumExpression::Use {
                ty,
                mode,
                ..
            }) if ty.capabilities.copy && ty.capabilities.needs_drop => {
                *mode = mir::PayloadEnumUseMode::TransferOwnedCopy;
            }
            mir::Rvalue::Value(mir::ValueExpression::Inline(mir::InlineValueExpression::Use {
                ty,
                mode,
                ..
            }))
            | mir::Rvalue::NullableScalar(mir::NullableScalarExpression::InlineClaim {
                ty,
                mode,
                ..
            }) if ty.requires_owned_cleanup() => {
                *mode = mir::InlineValueUseMode::TransferOwnedCopy;
            }
            _ => {}
        }
    }
    value
}

pub(crate) fn local_rvalue(local: mir::LocalId, ty: mir::Type, transfer: bool) -> mir::Rvalue {
    match ty {
        mir::Type::ByteRegion(kind) => mir::Rvalue::ByteRegion(mir::ByteRegionExpression::Local {
            kind,
            local,
            transfer: transfer && kind.is_writable(),
        }),
        mir::Type::NullableByteRegion(kind) => {
            mir::Rvalue::NullableByteRegion(mir::NullableByteRegionExpression::Local {
                kind,
                local,
                transfer: transfer && kind.is_writable(),
            })
        }
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
        mir::Type::Scalar(mir::ScalarType::Enum(enum_id)) => {
            mir::Rvalue::Value(mir::ValueExpression::Enum(mir::EnumExpression::Use {
                enum_id,
                operand: mir::Operand::Local(local),
            }))
        }
        mir::Type::Scalar(mir::ScalarType::Inline(ty)) => mir::Rvalue::Value(
            mir::ValueExpression::Inline(mir::InlineValueExpression::Use {
                ty,
                operand: mir::Operand::Local(local),
                mode: if transfer && ty.requires_owned_cleanup() {
                    mir::InlineValueUseMode::Copy
                } else {
                    mir::InlineValueUseMode::Borrow
                },
            }),
        ),
        mir::Type::String => mir::Rvalue::String(mir::StringExpression::Local(local)),
        mir::Type::NullableScalar(ty) => {
            let value = mir::NullableScalarExpression::Local { ty, local };
            mir::Rvalue::NullableScalar(match ty {
                mir::ScalarType::Inline(ty) if transfer && ty.requires_owned_cleanup() => {
                    mir::NullableScalarExpression::InlineClaim {
                        ty,
                        value: Box::new(value),
                        mode: mir::InlineValueUseMode::Copy,
                    }
                }
                _ => value,
            })
        }
        mir::Type::NullableString => {
            mir::Rvalue::NullableString(mir::NullableStringExpression::Local(local))
        }
        mir::Type::Mixed => mir::Rvalue::Mixed(mir::MixedExpression::Local { local, transfer }),
        mir::Type::NullableMixed => {
            mir::Rvalue::NullableMixed(mir::NullableMixedExpression::Local { local, transfer })
        }
        mir::Type::Interface(interface) => mir::Rvalue::Interface(mir::InterfaceExpression {
            interface,
            value: mir::InterfaceValue::Local { local, transfer },
        }),
        mir::Type::NullableInterface(interface) => {
            mir::Rvalue::NullableInterface(mir::NullableInterfaceExpression {
                interface,
                value: mir::NullableInterfaceValue::Local { local, transfer },
            })
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
        mir::Type::SharedReference(class) => {
            mir::Rvalue::SharedReference(mir::SharedReferenceExpression::Local {
                payload: class,
                local,
                transfer,
            })
        }
        mir::Type::WeakReference(class) => {
            mir::Rvalue::WeakReference(mir::WeakReferenceExpression::Local {
                payload: class,
                local,
                transfer,
            })
        }
        mir::Type::NullableSharedReference(class) => {
            mir::Rvalue::NullableSharedReference(mir::NullableSharedReferenceExpression::Local {
                payload: class,
                local,
                transfer,
            })
        }
        mir::Type::NullableWeakReference(class) => {
            mir::Rvalue::NullableWeakReference(mir::NullableWeakReferenceExpression::Local {
                payload: class,
                local,
                transfer,
            })
        }
        mir::Type::WritableSharedReference(payload) => {
            mir::Rvalue::WritableSharedReference(mir::WritableSharedReferenceExpression::Local {
                payload,
                local,
                transfer,
            })
        }
        mir::Type::WritableWeakReference(payload) => {
            mir::Rvalue::WritableWeakReference(mir::WritableWeakReferenceExpression::Local {
                payload,
                local,
                transfer,
            })
        }
        mir::Type::NullableWritableSharedReference(payload) => {
            mir::Rvalue::NullableWritableSharedReference(
                mir::NullableWritableSharedReferenceExpression::Local {
                    payload,
                    local,
                    transfer,
                },
            )
        }
        mir::Type::NullableWritableWeakReference(payload) => {
            mir::Rvalue::NullableWritableWeakReference(
                mir::NullableWritableWeakReferenceExpression::Local {
                    payload,
                    local,
                    transfer,
                },
            )
        }
        mir::Type::ReadonlySharedReferenceAccess(payload)
        | mir::Type::WritableSharedReferenceAccess(payload) => {
            mir::Rvalue::SharedReferenceAccess(mir::SharedReferenceAccessExpression::Local {
                payload,
                local,
                writable: matches!(ty, mir::Type::WritableSharedReferenceAccess(_)),
                transfer,
            })
        }
        mir::Type::NullableReadonlySharedReferenceAccess(payload)
        | mir::Type::NullableWritableSharedReferenceAccess(payload) => {
            mir::Rvalue::NullableSharedReferenceAccess(
                mir::NullableSharedReferenceAccessExpression::Local {
                    payload,
                    local,
                    writable: matches!(ty, mir::Type::NullableWritableSharedReferenceAccess(_)),
                    transfer,
                },
            )
        }
        mir::Type::Collection(collection) => {
            mir::Rvalue::Collection(mir::CollectionExpression::Local {
                collection,
                local,
                transfer,
                assume_non_null: false,
            })
        }
        mir::Type::NullableCollection(collection) => {
            mir::Rvalue::NullableCollection(mir::NullableCollectionExpression::Local {
                collection,
                local,
                transfer,
            })
        }
        mir::Type::PayloadEnum(ty) => mir::Rvalue::PayloadEnum(mir::PayloadEnumExpression::Use {
            ty,
            place: mir::PayloadEnumPlace::Local(local),
            mode: payload_enum_use_mode(ty, transfer),
        }),
        mir::Type::NullablePayloadEnum(ty) => {
            mir::Rvalue::NullablePayloadEnum(mir::NullablePayloadEnumExpression::Use {
                ty,
                place: mir::PayloadEnumPlace::Local(local),
                mode: payload_enum_use_mode(ty, transfer),
            })
        }
        mir::Type::Function(function_type) => {
            mir::Rvalue::Function(mir::FunctionExpression::Local {
                function_type,
                local,
                transfer,
            })
        }
        mir::Type::NullableFunction(function_type) => {
            mir::Rvalue::NullableFunction(mir::NullableFunctionExpression::Local {
                function_type,
                local,
                transfer,
            })
        }
        mir::Type::ClosureEnvironment(_) => {
            unreachable!("closure environments are compiler-private and cannot be source rvalues")
        }
    }
}

pub(crate) fn nullable_present_rvalue(value: mir::Rvalue) -> mir::Rvalue {
    use mir::Rvalue;
    match value {
        Rvalue::ByteRegion(value) => {
            Rvalue::NullableByteRegion(mir::NullableByteRegionExpression::Present(value))
        }
        Rvalue::Value(value) => Rvalue::NullableScalar(mir::NullableScalarExpression::Value(value)),
        Rvalue::String(value) => {
            Rvalue::NullableString(mir::NullableStringExpression::String(value))
        }
        Rvalue::Class(value) => Rvalue::NullableClass(mir::NullableClassExpression::Class(value)),
        Rvalue::Interface(value) => Rvalue::nullable_interface(
            value.interface,
            mir::NullableInterfaceValue::Present(value.value),
        ),
        Rvalue::Collection(value) => {
            Rvalue::NullableCollection(mir::NullableCollectionExpression::Collection(value))
        }
        Rvalue::Mixed(value) => Rvalue::NullableMixed(mir::NullableMixedExpression::Mixed(value)),
        Rvalue::PayloadEnum(value) => {
            Rvalue::NullablePayloadEnum(mir::NullablePayloadEnumExpression::Value(value))
        }
        Rvalue::Function(value) => {
            Rvalue::NullableFunction(mir::NullableFunctionExpression::Present(value))
        }
        Rvalue::SharedReference(value) => {
            Rvalue::NullableSharedReference(mir::NullableSharedReferenceExpression::Shared(value))
        }
        Rvalue::WeakReference(value) => {
            Rvalue::NullableWeakReference(mir::NullableWeakReferenceExpression::Weak(value))
        }
        Rvalue::WritableSharedReference(value) => Rvalue::NullableWritableSharedReference(
            mir::NullableWritableSharedReferenceExpression::Strong(value),
        ),
        Rvalue::WritableWeakReference(value) => Rvalue::NullableWritableWeakReference(
            mir::NullableWritableWeakReferenceExpression::Weak(value),
        ),
        Rvalue::SharedReferenceAccess(value) => Rvalue::NullableSharedReferenceAccess(
            mir::NullableSharedReferenceAccessExpression::Access(Box::new(value)),
        ),
        _ => unreachable!("validated non-null result type"),
    }
}

pub(crate) fn payload_enum_use_mode(
    ty: mir::PayloadEnumType,
    transfer: bool,
) -> mir::PayloadEnumUseMode {
    if !transfer {
        mir::PayloadEnumUseMode::Borrow
    } else if ty.capabilities.copy {
        mir::PayloadEnumUseMode::Copy
    } else {
        mir::PayloadEnumUseMode::Move
    }
}

pub(crate) fn local_integer_expression(
    local: mir::LocalId,
    ty: IntegerType,
) -> mir::IntegerExpression {
    mir::IntegerExpression::use_operand(ty, mir::Operand::Local(local))
}

pub(crate) fn local_float_expression(local: mir::LocalId, ty: FloatType) -> mir::FloatExpression {
    mir::FloatExpression::Use {
        ty,
        operand: mir::Operand::Local(local),
    }
}

pub(crate) fn call_result_is_writable(
    owned: bool,
    return_borrow: Option<mir::ReturnBorrow>,
) -> bool {
    owned
        || return_borrow
            .is_some_and(|borrow| borrow.kind == mir::ReturnBorrowKind::Value && borrow.writable)
}

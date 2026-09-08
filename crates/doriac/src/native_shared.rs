//! Allocation-free operation views over the typed shared MIR expressions.
//! Both native backends use this projection so handle ownership and transport agree.
use crate::class_layout::PropertyId;
use crate::mir::{
    self, FunctionId, LocalId, NullableCollectionAccess, Rvalue, SharedPayload, Type,
};
use crate::native_abi::*;
use crate::source::Span;

#[derive(Clone, Copy)]
pub(crate) enum Expression<'a> {
    Strong(&'a mir::SharedReferenceExpression),
    Weak(&'a mir::WeakReferenceExpression),
    NullableStrong(&'a mir::NullableSharedReferenceExpression),
    NullableWeak(&'a mir::NullableWeakReferenceExpression),
    WritableStrong(&'a mir::WritableSharedReferenceExpression),
    WritableWeak(&'a mir::WritableWeakReferenceExpression),
    NullableWritableStrong(&'a mir::NullableWritableSharedReferenceExpression),
    NullableWritableWeak(&'a mir::NullableWritableWeakReferenceExpression),
    Access(&'a mir::SharedReferenceAccessExpression),
    NullableAccess(&'a mir::NullableSharedReferenceAccessExpression),
}

pub(crate) enum Operation<'a> {
    New {
        value: &'a Rvalue,
        symbol: &'static str,
    },
    Null,
    Present(Expression<'a>),
    Local {
        local: LocalId,
        transfer: bool,
    },
    Property {
        object: LocalId,
        property: PropertyId,
    },
    Call {
        function: FunctionId,
        args: &'a [Rvalue],
    },
    Runtime {
        value: Expression<'a>,
        symbol: &'static str,
        null_safe: bool,
        span: Option<Span>,
    },
    Coalesce {
        left: Expression<'a>,
        right: Expression<'a>,
        transfer: bool,
    },
    Index {
        collection: LocalId,
        index: &'a Rvalue,
        remove: bool,
        positional: bool,
    },
    Get {
        collection: LocalId,
        key: &'a Rvalue,
        access: NullableCollectionAccess,
        stored: Type,
    },
}

impl<'a> Expression<'a> {
    pub fn from_rvalue(value: &'a Rvalue) -> Option<Self> {
        Some(match value {
            Rvalue::SharedReference(value) => Self::Strong(value),
            Rvalue::WeakReference(value) => Self::Weak(value),
            Rvalue::NullableSharedReference(value) => Self::NullableStrong(value),
            Rvalue::NullableWeakReference(value) => Self::NullableWeak(value),
            Rvalue::WritableSharedReference(value) => Self::WritableStrong(value),
            Rvalue::WritableWeakReference(value) => Self::WritableWeak(value),
            Rvalue::NullableWritableSharedReference(value) => Self::NullableWritableStrong(value),
            Rvalue::NullableWritableWeakReference(value) => Self::NullableWritableWeak(value),
            Rvalue::SharedReferenceAccess(value) => Self::Access(value),
            Rvalue::NullableSharedReferenceAccess(value) => Self::NullableAccess(value),
            _ => return None,
        })
    }
    pub fn ty(self) -> Type {
        match self {
            Self::Strong(e) => Type::SharedReference(e.payload()),
            Self::Weak(e) => Type::WeakReference(e.payload()),
            Self::NullableStrong(e) => Type::NullableSharedReference(e.payload()),
            Self::NullableWeak(e) => Type::NullableWeakReference(e.payload()),
            Self::WritableStrong(e) => Type::WritableSharedReference(e.payload()),
            Self::WritableWeak(e) => Type::WritableWeakReference(e.payload()),
            Self::NullableWritableStrong(e) => Type::NullableWritableSharedReference(e.payload()),
            Self::NullableWritableWeak(e) => Type::NullableWritableWeakReference(e.payload()),
            Self::Access(e) => e.ty(),
            Self::NullableAccess(e) => e.ty(),
        }
    }
    pub fn payload(self) -> SharedPayload {
        self.ty().shared_payload().expect("shared expression type")
    }
    pub fn owned(self) -> bool {
        match self {
            Self::Strong(e) => e.owned_temporary().is_some(),
            Self::Weak(e) => e.owned_temporary().is_some(),
            Self::NullableStrong(e) => e.owned_temporary().is_some(),
            Self::NullableWeak(e) => e.owned_temporary().is_some(),
            Self::WritableStrong(e) => e.owned_temporary(),
            Self::WritableWeak(e) => e.owned_temporary(),
            Self::NullableWritableStrong(e) => e.owned_temporary(),
            Self::NullableWritableWeak(e) => e.owned_temporary(),
            Self::Access(e) => e.owned_temporary(),
            Self::NullableAccess(e) => e.owned_temporary(),
        }
    }
    pub fn release(self) -> &'static str {
        match self.ty() {
            Type::SharedReference(_) | Type::NullableSharedReference(_) => SHARED_RELEASE,
            Type::WeakReference(_) | Type::NullableWeakReference(_) => SHARED_RELEASE_WEAK,
            Type::WritableSharedReference(_) | Type::NullableWritableSharedReference(_) => {
                WRITABLE_SHARED_RELEASE
            }
            Type::WritableWeakReference(_) | Type::NullableWritableWeakReference(_) => {
                WRITABLE_SHARED_RELEASE_WEAK
            }
            Type::ReadonlySharedReferenceAccess(_)
            | Type::NullableReadonlySharedReferenceAccess(_) => {
                WRITABLE_SHARED_RELEASE_READONLY_ACCESS
            }
            Type::WritableSharedReferenceAccess(_)
            | Type::NullableWritableSharedReferenceAccess(_) => {
                WRITABLE_SHARED_RELEASE_WRITABLE_ACCESS
            }
            _ => unreachable!("shared expression type"),
        }
    }
    pub fn operation(self) -> Operation<'a> {
        use Operation as O;
        match self {
            Self::Strong(e) => {
                use mir::SharedReferenceExpression as E;
                match e {
                    E::New { value, .. } => O::New {
                        value,
                        symbol: SHARED_CREATE,
                    },
                    E::Local {
                        local, transfer, ..
                    }
                    | E::NullableLocalAssumeNonNull {
                        local, transfer, ..
                    } => O::Local {
                        local: *local,
                        transfer: *transfer,
                    },
                    E::Property {
                        object, property, ..
                    } => O::Property {
                        object: *object,
                        property: *property,
                    },
                    E::Call { function, args, .. } => O::Call {
                        function: *function,
                        args,
                    },
                    E::Share { value, .. } => O::Runtime {
                        value: Self::Strong(value),
                        symbol: SHARED_RETAIN,
                        null_safe: false,
                        span: None,
                    },
                    E::Coalesce {
                        left,
                        right,
                        transfer,
                        ..
                    } => O::Coalesce {
                        left: Self::NullableStrong(left),
                        right: Self::Strong(right),
                        transfer: *transfer,
                    },
                    E::CollectionIndex {
                        collection,
                        index,
                        remove,
                        positional,
                        ..
                    } => O::Index {
                        collection: *collection,
                        index,
                        remove: *remove,
                        positional: *positional,
                    },
                }
            }
            Self::Weak(e) => {
                use mir::WeakReferenceExpression as E;
                match e {
                    E::Local {
                        local, transfer, ..
                    }
                    | E::NullableLocalAssumeNonNull {
                        local, transfer, ..
                    } => O::Local {
                        local: *local,
                        transfer: *transfer,
                    },
                    E::Property {
                        object, property, ..
                    } => O::Property {
                        object: *object,
                        property: *property,
                    },
                    E::Call { function, args, .. } => O::Call {
                        function: *function,
                        args,
                    },
                    E::Create { value, .. } => O::Runtime {
                        value: Self::Strong(value),
                        symbol: SHARED_CREATE_WEAK,
                        null_safe: false,
                        span: None,
                    },
                    E::Coalesce {
                        left,
                        right,
                        transfer,
                        ..
                    } => O::Coalesce {
                        left: Self::NullableWeak(left),
                        right: Self::Weak(right),
                        transfer: *transfer,
                    },
                    E::CollectionIndex {
                        collection,
                        index,
                        remove,
                        positional,
                        ..
                    } => O::Index {
                        collection: *collection,
                        index,
                        remove: *remove,
                        positional: *positional,
                    },
                }
            }
            Self::NullableStrong(e) => {
                use mir::NullableSharedReferenceExpression as E;
                match e {
                    E::Null(_) => O::Null,
                    E::Shared(value) => O::Present(Self::Strong(value)),
                    E::Local {
                        local, transfer, ..
                    } => O::Local {
                        local: *local,
                        transfer: *transfer,
                    },
                    E::Property {
                        object, property, ..
                    } => O::Property {
                        object: *object,
                        property: *property,
                    },
                    E::Call { function, args, .. } => O::Call {
                        function: *function,
                        args,
                    },
                    E::Acquire { value, .. } => O::Runtime {
                        value: Self::Weak(value),
                        symbol: SHARED_ACQUIRE,
                        null_safe: false,
                        span: None,
                    },
                    E::NullSafeShare { value, .. } => O::Runtime {
                        value: Self::NullableStrong(value),
                        symbol: SHARED_RETAIN,
                        null_safe: true,
                        span: None,
                    },
                    E::NullSafeAcquire { value, .. } => O::Runtime {
                        value: Self::NullableWeak(value),
                        symbol: SHARED_ACQUIRE,
                        null_safe: true,
                        span: None,
                    },
                    E::Coalesce {
                        left,
                        right,
                        transfer,
                        ..
                    } => O::Coalesce {
                        left: Self::NullableStrong(left),
                        right: Self::NullableStrong(right),
                        transfer: *transfer,
                    },
                    E::CollectionIndex {
                        collection,
                        index,
                        remove,
                        positional,
                        ..
                    } => O::Index {
                        collection: *collection,
                        index,
                        remove: *remove,
                        positional: *positional,
                    },
                    E::DictionaryGet {
                        payload,
                        collection,
                        key,
                        access,
                        stored_nullable,
                    } => O::Get {
                        collection: *collection,
                        key,
                        access: *access,
                        stored: if *stored_nullable {
                            Type::NullableSharedReference(*payload)
                        } else {
                            Type::SharedReference(*payload)
                        },
                    },
                }
            }
            Self::NullableWeak(e) => {
                use mir::NullableWeakReferenceExpression as E;
                match e {
                    E::Null(_) => O::Null,
                    E::Weak(value) => O::Present(Self::Weak(value)),
                    E::Local {
                        local, transfer, ..
                    } => O::Local {
                        local: *local,
                        transfer: *transfer,
                    },
                    E::Property {
                        object, property, ..
                    } => O::Property {
                        object: *object,
                        property: *property,
                    },
                    E::Call { function, args, .. } => O::Call {
                        function: *function,
                        args,
                    },
                    E::NullSafeCreate { value, .. } => O::Runtime {
                        value: Self::NullableStrong(value),
                        symbol: SHARED_CREATE_WEAK,
                        null_safe: true,
                        span: None,
                    },
                    E::Coalesce {
                        left,
                        right,
                        transfer,
                        ..
                    } => O::Coalesce {
                        left: Self::NullableWeak(left),
                        right: Self::NullableWeak(right),
                        transfer: *transfer,
                    },
                    E::CollectionIndex {
                        collection,
                        index,
                        remove,
                        positional,
                        ..
                    } => O::Index {
                        collection: *collection,
                        index,
                        remove: *remove,
                        positional: *positional,
                    },
                    E::DictionaryGet {
                        payload,
                        collection,
                        key,
                        access,
                        stored_nullable,
                    } => O::Get {
                        collection: *collection,
                        key,
                        access: *access,
                        stored: if *stored_nullable {
                            Type::NullableWeakReference(*payload)
                        } else {
                            Type::WeakReference(*payload)
                        },
                    },
                }
            }
            Self::WritableStrong(e) => {
                use mir::WritableSharedReferenceExpression as E;
                match e {
                    E::New { value, .. } => O::New {
                        value,
                        symbol: WRITABLE_SHARED_CREATE,
                    },
                    E::Local {
                        local, transfer, ..
                    }
                    | E::NullableLocalAssumeNonNull {
                        local, transfer, ..
                    } => O::Local {
                        local: *local,
                        transfer: *transfer,
                    },
                    E::Property {
                        object, property, ..
                    } => O::Property {
                        object: *object,
                        property: *property,
                    },
                    E::Call { function, args, .. } => O::Call {
                        function: *function,
                        args,
                    },
                    E::Share { value, .. } => O::Runtime {
                        value: Self::WritableStrong(value),
                        symbol: WRITABLE_SHARED_RETAIN,
                        null_safe: false,
                        span: None,
                    },
                    E::Coalesce {
                        left,
                        right,
                        transfer,
                        ..
                    } => O::Coalesce {
                        left: Self::NullableWritableStrong(left),
                        right: Self::WritableStrong(right),
                        transfer: *transfer,
                    },
                    E::CollectionIndex {
                        collection,
                        index,
                        remove,
                        positional,
                        ..
                    } => O::Index {
                        collection: *collection,
                        index,
                        remove: *remove,
                        positional: *positional,
                    },
                }
            }
            Self::WritableWeak(e) => {
                use mir::WritableWeakReferenceExpression as E;
                match e {
                    E::Local {
                        local, transfer, ..
                    }
                    | E::NullableLocalAssumeNonNull {
                        local, transfer, ..
                    } => O::Local {
                        local: *local,
                        transfer: *transfer,
                    },
                    E::Property {
                        object, property, ..
                    } => O::Property {
                        object: *object,
                        property: *property,
                    },
                    E::Call { function, args, .. } => O::Call {
                        function: *function,
                        args,
                    },
                    E::Create { value, .. } => O::Runtime {
                        value: Self::WritableStrong(value),
                        symbol: WRITABLE_SHARED_CREATE_WEAK,
                        null_safe: false,
                        span: None,
                    },
                    E::Coalesce {
                        left,
                        right,
                        transfer,
                        ..
                    } => O::Coalesce {
                        left: Self::NullableWritableWeak(left),
                        right: Self::WritableWeak(right),
                        transfer: *transfer,
                    },
                    E::CollectionIndex {
                        collection,
                        index,
                        remove,
                        positional,
                        ..
                    } => O::Index {
                        collection: *collection,
                        index,
                        remove: *remove,
                        positional: *positional,
                    },
                }
            }
            Self::NullableWritableStrong(e) => {
                use mir::NullableWritableSharedReferenceExpression as E;
                match e {
                    E::Null(_) => O::Null,
                    E::Strong(value) => O::Present(Self::WritableStrong(value)),
                    E::Local {
                        local, transfer, ..
                    } => O::Local {
                        local: *local,
                        transfer: *transfer,
                    },
                    E::Property {
                        object, property, ..
                    } => O::Property {
                        object: *object,
                        property: *property,
                    },
                    E::Call { function, args, .. } => O::Call {
                        function: *function,
                        args,
                    },
                    E::Acquire { value, .. } => O::Runtime {
                        value: Self::WritableWeak(value),
                        symbol: WRITABLE_SHARED_ACQUIRE,
                        null_safe: false,
                        span: None,
                    },
                    E::NullSafeShare { value, .. } => O::Runtime {
                        value: Self::NullableWritableStrong(value),
                        symbol: WRITABLE_SHARED_RETAIN,
                        null_safe: true,
                        span: None,
                    },
                    E::NullSafeAcquire { value, .. } => O::Runtime {
                        value: Self::NullableWritableWeak(value),
                        symbol: WRITABLE_SHARED_ACQUIRE,
                        null_safe: true,
                        span: None,
                    },
                    E::Coalesce {
                        left,
                        right,
                        transfer,
                        ..
                    } => O::Coalesce {
                        left: Self::NullableWritableStrong(left),
                        right: Self::NullableWritableStrong(right),
                        transfer: *transfer,
                    },
                    E::DictionaryGet {
                        payload,
                        collection,
                        key,
                        access,
                        stored_nullable,
                    } => O::Get {
                        collection: *collection,
                        key,
                        access: *access,
                        stored: if *stored_nullable {
                            Type::NullableWritableSharedReference(*payload)
                        } else {
                            Type::WritableSharedReference(*payload)
                        },
                    },
                }
            }
            Self::NullableWritableWeak(e) => {
                use mir::NullableWritableWeakReferenceExpression as E;
                match e {
                    E::Null(_) => O::Null,
                    E::Weak(value) => O::Present(Self::WritableWeak(value)),
                    E::Local {
                        local, transfer, ..
                    } => O::Local {
                        local: *local,
                        transfer: *transfer,
                    },
                    E::Property {
                        object, property, ..
                    } => O::Property {
                        object: *object,
                        property: *property,
                    },
                    E::Call { function, args, .. } => O::Call {
                        function: *function,
                        args,
                    },
                    E::NullSafeCreate { value, .. } => O::Runtime {
                        value: Self::NullableWritableStrong(value),
                        symbol: WRITABLE_SHARED_CREATE_WEAK,
                        null_safe: true,
                        span: None,
                    },
                    E::Coalesce {
                        left,
                        right,
                        transfer,
                        ..
                    } => O::Coalesce {
                        left: Self::NullableWritableWeak(left),
                        right: Self::NullableWritableWeak(right),
                        transfer: *transfer,
                    },
                    E::DictionaryGet {
                        payload,
                        collection,
                        key,
                        access,
                        stored_nullable,
                    } => O::Get {
                        collection: *collection,
                        key,
                        access: *access,
                        stored: if *stored_nullable {
                            Type::NullableWritableWeakReference(*payload)
                        } else {
                            Type::WritableWeakReference(*payload)
                        },
                    },
                }
            }
            Self::Access(e) => {
                use mir::SharedReferenceAccessExpression as E;
                match e {
                    E::Local {
                        local, transfer, ..
                    }
                    | E::NullableLocalAssumeNonNull {
                        local, transfer, ..
                    } => O::Local {
                        local: *local,
                        transfer: *transfer,
                    },
                    E::Property {
                        object, property, ..
                    } => O::Property {
                        object: *object,
                        property: *property,
                    },
                    E::Call { function, args, .. } => O::Call {
                        function: *function,
                        args,
                    },
                    E::Acquire {
                        value,
                        writable,
                        span,
                        ..
                    } => O::Runtime {
                        value: Self::WritableStrong(value),
                        symbol: if *writable {
                            WRITABLE_SHARED_ACQUIRE_WRITABLE_ACCESS
                        } else {
                            WRITABLE_SHARED_ACQUIRE_READONLY_ACCESS
                        },
                        null_safe: false,
                        span: Some(*span),
                    },
                    E::CollectionIndex {
                        collection,
                        index,
                        remove,
                        positional,
                        ..
                    } => O::Index {
                        collection: *collection,
                        index,
                        remove: *remove,
                        positional: *positional,
                    },
                }
            }
            Self::NullableAccess(e) => {
                use mir::NullableSharedReferenceAccessExpression as E;
                match e {
                    E::Null { .. } => O::Null,
                    E::Access(value) => O::Present(Self::Access(value)),
                    E::Local {
                        local, transfer, ..
                    } => O::Local {
                        local: *local,
                        transfer: *transfer,
                    },
                    E::Property {
                        object, property, ..
                    } => O::Property {
                        object: *object,
                        property: *property,
                    },
                    E::Call { function, args, .. } => O::Call {
                        function: *function,
                        args,
                    },
                    E::NullSafeAcquire {
                        value,
                        writable,
                        span,
                        ..
                    } => O::Runtime {
                        value: Self::NullableWritableStrong(value),
                        symbol: if *writable {
                            WRITABLE_SHARED_ACQUIRE_WRITABLE_ACCESS
                        } else {
                            WRITABLE_SHARED_ACQUIRE_READONLY_ACCESS
                        },
                        null_safe: true,
                        span: Some(*span),
                    },
                    E::CollectionIndex {
                        collection,
                        index,
                        remove,
                        positional,
                        ..
                    } => O::Index {
                        collection: *collection,
                        index,
                        remove: *remove,
                        positional: *positional,
                    },
                    E::CollectionGet {
                        collection,
                        key,
                        access,
                        stored,
                    } => O::Get {
                        collection: *collection,
                        key,
                        access: *access,
                        stored: stored.into_type(),
                    },
                }
            }
        }
    }
}

//! Source contracts for handles (Decision 0148 section 11, amendment
//! Readable Names of 2026-11-20).
//!
//! A handle is a `WritablePointer<Opaque>` that holds a Doria value for C:
//! `Handle::create(take T $value)` hands an owned class value over and
//! `Handle::reclaim` takes it back; `$reference->toHandle()` hands one
//! strong claim of a `SharedReference<T>` or `WritableSharedReference<T>`
//! over and the family's `reclaim` takes it back; each family's `borrow`
//! lends the value for the enclosing unsafe context without taking it
//! back. Checking types each operation from this table, ownership
//! recognizes them through [`static_operation`] and
//! [`instance_operation`], and lowering keys on [`HandleOperation`].

use crate::types::SharedHandleKind;

/// The compiler-known `Doria\Std\Ffi\Handle` companion.
pub const HANDLE: &str = crate::compiler_known_ffi::HANDLE;
/// The instance member of the shared references that makes a handle.
pub const TO_HANDLE: &str = "toHandle";

/// Which values a handle holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HandleFamily {
    /// An owned class value, through `Handle`.
    Owned,
    /// A strong claim of a `SharedReference<T>` or a
    /// `WritableSharedReference<T>`, which keeps its family and, for the
    /// writable family, its runtime access checks.
    Shared(SharedHandleKind),
}

/// What a handle member does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HandleOperation {
    /// `Handle::create(take T $value)` and `$reference->toHandle()`.
    Create,
    /// `Handle::reclaim($handle)` and the shared families' `reclaim`: the
    /// value back, owned, its type from the expected type.
    Reclaim,
    /// `Handle::borrow($handle)` and the shared families' `borrow`: the
    /// value lent for the enclosing unsafe context.
    Borrow,
}

impl HandleFamily {
    /// The companion the family's static members are spelled on.
    pub fn companion(self) -> &'static str {
        match self {
            Self::Owned => "Handle",
            Self::Shared(kind) => kind.source_name(),
        }
    }
}

impl HandleOperation {
    pub const fn name(self, family: HandleFamily) -> &'static str {
        match (self, family) {
            (Self::Create, HandleFamily::Owned) => "create",
            (Self::Create, HandleFamily::Shared(_)) => TO_HANDLE,
            (Self::Reclaim, _) => "reclaim",
            (Self::Borrow, _) => "borrow",
        }
    }

    /// What the operation does, completing "… needs an unsafe context" in
    /// E0785, or `None` when it is safe. Handing a value to C keeps every
    /// guarantee: the value stays alive and unaliased until a `reclaim`
    /// takes it back. Only `reclaim` and `borrow` trust the handle to hold
    /// a value of the stated type and family, so only they are unsafe
    /// (Decision 0148 section 11).
    pub const fn unsafe_description(self) -> Option<&'static str> {
        match self {
            Self::Create => None,
            Self::Reclaim => Some("taking a Doria value back from a handle"),
            Self::Borrow => Some("borrowing a Doria value through a handle"),
        }
    }

    pub const fn documentation(self, family: HandleFamily) -> &'static str {
        match (self, family) {
            (Self::Create, HandleFamily::Owned) => "Hands an owned class value to C as a handle, a `WritablePointer<Opaque>` C keeps as user data (Decision 0148 section 11). Safe: the value is not destroyed until `Handle::reclaim` takes it back. A closed class's handle is its object; an open class's handle addresses a small box holding the object and its class, so the value keeps its exact type, and `reclaim` frees the box.",
            (Self::Create, HandleFamily::Shared(_)) => "Hands this reference's strong claim to C as a handle, a `WritablePointer<Opaque>` (Decision 0148 section 11). Safe: the reference is consumed; the value stays alive until the family's `reclaim` takes the claim back. A reference to an interface is two words, so its handle addresses a small box holding both, which `reclaim` frees.",
            (Self::Reclaim, HandleFamily::Owned) => "Takes back the owned class value `Handle::create` handed to C (Decision 0148 section 11); its type comes from the expected type. Needs an unsafe context. Reclaiming a handle twice, or as another type, is undefined behavior.",
            (Self::Reclaim, HandleFamily::Shared(_)) => "Takes back the strong claim `toHandle` handed to C as a reference of this family (Decision 0148 section 11); the payload type comes from the expected type. Needs an unsafe context. Reclaiming a handle twice, or as another type or family, is undefined behavior.",
            (Self::Borrow, HandleFamily::Owned) => "Lends the class value a handle holds, with readonly access, for the enclosing unsafe context, without taking it back (Decision 0148 section 11); its type comes from the expected type. Needs an unsafe context; the value cannot leave it.",
            (Self::Borrow, HandleFamily::Shared(_)) => "Lends the reference a handle holds for the enclosing unsafe context, without taking its claim back (Decision 0148 section 11); the payload type comes from the expected type. Needs an unsafe context; the reference cannot leave it. A writable reference keeps its runtime access checks.",
        }
    }
}

/// The handle operation a static call `<class_name>::<method>` names.
pub fn static_operation(class_name: &str, method: &str) -> Option<(HandleFamily, HandleOperation)> {
    let family = if class_name == HANDLE {
        HandleFamily::Owned
    } else {
        match SharedHandleKind::from_source_name(class_name)? {
            kind @ (SharedHandleKind::SharedReference
            | SharedHandleKind::WritableSharedReference) => HandleFamily::Shared(kind),
            _ => return None,
        }
    };
    let operation = match (family, method) {
        (HandleFamily::Owned, "create") => HandleOperation::Create,
        (_, "reclaim") => HandleOperation::Reclaim,
        (_, "borrow") => HandleOperation::Borrow,
        _ => return None,
    };
    Some((family, operation))
}

/// The handle operation of a static call, from its qualifier.
pub fn static_call(
    qualifier: &crate::ast::StaticQualifier,
    method: &str,
) -> Option<(HandleFamily, HandleOperation)> {
    match qualifier {
        crate::ast::StaticQualifier::Class(name) => static_operation(name, method),
        _ => None,
    }
}

/// `$reference->toHandle()` on a shared reference of `kind`.
pub fn instance_operation(kind: SharedHandleKind, method: &str) -> Option<HandleFamily> {
    (method == TO_HANDLE
        && matches!(
            kind,
            SharedHandleKind::SharedReference | SharedHandleKind::WritableSharedReference
        ))
    .then_some(HandleFamily::Shared(kind))
}

/// The static members of `family`, in documentation order.
pub fn static_members(family: HandleFamily) -> &'static [&'static str] {
    match family {
        HandleFamily::Owned => &["create", "reclaim", "borrow"],
        HandleFamily::Shared(_) => &["reclaim", "borrow"],
    }
}

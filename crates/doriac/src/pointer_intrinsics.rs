//! Source contracts for raw pointers and C boundary values (Decision 0148
//! sections 4, 5 and 7).
//!
//! The table names every member of `Ptr<T>`, `MutPtr<T>` and the C boundary
//! types, the operation it performs, whether it needs an unsafe context and
//! whether it needs a pointee with a size. Checking types each member from
//! its operation; ownership recognizes pointer formation through
//! [`formation`]; lowering keys on [`PointerOperation`].

use crate::ast::StaticQualifier;
use crate::foreign::{CBoundaryType, PointerAccess};

/// What a pointer member does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PointerOperation {
    /// `$pointer->read(): T` (aligned) and `readUnaligned(): T`.
    Read { aligned: bool },
    /// `$mutable->write(T $value): void` (aligned) and `writeUnaligned`.
    Write { aligned: bool },
    /// `$pointer->offset(int $count)`, scaled by the pointee's size.
    Offset,
    /// `$pointer->byteOffset(int $bytes)`.
    ByteOffset,
    /// `$pointer->address: uint64`.
    Address,
    /// `Ptr::cast($pointer)` and `MutPtr::cast($pointer)`: the target comes
    /// from the expected type.
    Cast,
    /// `Ptr::fromAddress(uint64 $address): ?Ptr<T>`, `T` from the expected
    /// type.
    FromAddress,
    /// `Ptr::toFunction(Ptr<Opaque> $address)`: the C function pointer type of
    /// the expected type.
    ToFunction,
    /// `Ptr::from(MutPtr<T> $pointer): Ptr<T>`.
    DropWriteAccess,
    /// `Ptr::to($place)` and `MutPtr::to($place)`: the address of an FFI-safe
    /// place, pinned to the end of the innermost enclosing unsafe context.
    AddressOf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerMemberKind {
    Method,
    Property,
    StaticMethod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointerMember {
    pub name: &'static str,
    pub kind: PointerMemberKind,
    pub operation: PointerOperation,
    pub documentation: &'static str,
}

impl PointerMember {
    /// Every use needs an unsafe context.
    pub const fn requires_unsafe(&self) -> bool {
        self.operation.unsafe_description().is_some()
    }
}

impl PointerOperation {
    /// Reading, writing and moving by elements use the pointee's size, so
    /// `Ptr<Opaque>` and a pointer to a foreign struct declared without
    /// fields have no such member.
    pub const fn needs_sized_pointee(self) -> bool {
        matches!(self, Self::Read { .. } | Self::Write { .. } | Self::Offset)
    }

    /// What an operation that needs an unsafe context does, completing
    /// "… needs an unsafe context" in E0785; `None` for a safe operation.
    pub const fn unsafe_description(self) -> Option<&'static str> {
        match self {
            Self::Read { .. } => Some("reading through a pointer"),
            Self::Write { .. } => Some("writing through a pointer"),
            Self::Offset | Self::ByteOffset => Some("offsetting a pointer"),
            Self::Cast => Some("converting a pointer to another pointee type"),
            Self::FromAddress => Some("converting an address to a pointer"),
            Self::ToFunction => Some("converting an address to a C function"),
            Self::AddressOf => Some("forming a pointer to a Doria place"),
            Self::Address | Self::DropWriteAccess => None,
        }
    }
}

const INSTANCE_MEMBERS: [&str; 7] = [
    "read",
    "readUnaligned",
    "write",
    "writeUnaligned",
    "offset",
    "byteOffset",
    "address",
];

/// Every instance member of a pointer with `access`, in documentation order.
pub fn instance_members(access: PointerAccess) -> Vec<PointerMember> {
    INSTANCE_MEMBERS
        .into_iter()
        .filter_map(|name| instance_member(access, name))
        .collect()
}

/// The instance member `name` of `Ptr<T>` or `MutPtr<T>`.
pub fn instance_member(access: PointerAccess, name: &str) -> Option<PointerMember> {
    let member = |name, kind, operation, documentation| {
        Some(PointerMember {
            name,
            kind,
            operation,
            documentation,
        })
    };
    use PointerMemberKind::{Method, Property};
    match name {
        "read" => member(
            "read",
            Method,
            PointerOperation::Read { aligned: true },
            "Reads the pointee. Needs an unsafe context. The address must be aligned for `T`; a misaligned address panics in every profile.",
        ),
        "readUnaligned" => member(
            "readUnaligned",
            Method,
            PointerOperation::Read { aligned: false },
            "Reads the pointee at an address with no alignment requirement. Needs an unsafe context.",
        ),
        "write" if access == PointerAccess::Write => member(
            "write",
            Method,
            PointerOperation::Write { aligned: true },
            "Writes `$value` to the pointee. Needs an unsafe context. The address must be aligned for `T`; a misaligned address panics in every profile.",
        ),
        "writeUnaligned" if access == PointerAccess::Write => member(
            "writeUnaligned",
            Method,
            PointerOperation::Write { aligned: false },
            "Writes `$value` to the pointee at an address with no alignment requirement. Needs an unsafe context.",
        ),
        "offset" => member(
            "offset",
            Method,
            PointerOperation::Offset,
            "The pointer `$count` elements of `T` away, of the same type. Needs an unsafe context. Address arithmetic that overflows panics.",
        ),
        "byteOffset" => member(
            "byteOffset",
            Method,
            PointerOperation::ByteOffset,
            "The pointer `$bytes` bytes away, of the same type. Needs an unsafe context. Address arithmetic that overflows panics.",
        ),
        "address" => member(
            "address",
            Property,
            PointerOperation::Address,
            "The address as a `uint64`.",
        ),
        _ => None,
    }
}

/// The static member `name` of the `Ptr` or `MutPtr` companion.
pub fn static_member(access: PointerAccess, name: &str) -> Option<PointerMember> {
    let member = |name, operation, documentation| {
        Some(PointerMember {
            name,
            kind: PointerMemberKind::StaticMethod,
            operation,
            documentation,
        })
    };
    match (access, name) {
        (_, "cast") => member(
            "cast",
            PointerOperation::Cast,
            "Converts a pointer to the pointee type the expected type names, keeping its address. A nullable pointer converts to a nullable one. Needs an unsafe context.",
        ),
        (_, "to") => member(
            "to",
            PointerOperation::AddressOf,
            "The address of a local, a foreign struct field or a property of FFI-safe type. `MutPtr::to` needs a writable place. Forming it pins the place until the innermost enclosing unsafe context ends; the pointer is valid only within that context.",
        ),
        (PointerAccess::Read, "fromAddress") => member(
            "fromAddress",
            PointerOperation::FromAddress,
            "The pointer at `$address`, or `null` for address zero; the pointee type comes from the expected type. Needs an unsafe context.",
        ),
        (PointerAccess::Read, "toFunction") => member(
            "toFunction",
            PointerOperation::ToFunction,
            "Converts an address from `dlsym` or `GetProcAddress` to the C function pointer type the expected type names. Needs an unsafe context.",
        ),
        (PointerAccess::Read, "from") => member(
            "from",
            PointerOperation::DropWriteAccess,
            "The same address with read access only.",
        ),
        _ => None,
    }
}

const STATIC_MEMBERS: [&str; 5] = ["cast", "to", "fromAddress", "toFunction", "from"];

/// Every static member of the `Ptr` or `MutPtr` companion.
pub fn static_members(access: PointerAccess) -> Vec<PointerMember> {
    STATIC_MEMBERS
        .into_iter()
        .filter_map(|name| static_member(access, name))
        .collect()
}

/// A conversion from an integer to a C boundary value (section 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryConversion {
    /// `CLong::from($value)`: panics when the value does not fit on this
    /// target.
    From,
    /// `CLong::tryFrom($value)`: `null` when the value does not fit.
    TryFrom,
}

impl BoundaryConversion {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "from" => Some(Self::From),
            "tryFrom" => Some(Self::TryFrom),
            _ => None,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::From => "from",
            Self::TryFrom => "tryFrom",
        }
    }

    pub const fn documentation(self) -> &'static str {
        match self {
            Self::From => "Converts any integer to this C type. Panics when the value does not fit in the type's width on this target.",
            Self::TryFrom => "Converts any integer to this C type, or returns `null` when the value does not fit in the type's width on this target.",
        }
    }
}

/// The companion a static call names, when it is `Ptr`, `MutPtr` or a C
/// boundary type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForeignCompanion {
    Pointer(PointerAccess),
    Boundary(CBoundaryType),
    /// `CString` or `Utf16String` ([`crate::foreign_strings`]).
    OwnedString(crate::foreign::ForeignStringEncoding),
}

impl ForeignCompanion {
    /// `class` is the canonical name name resolution gives the qualifier.
    pub fn from_class_name(class: &str) -> Option<Self> {
        PointerAccess::from_source_name(class)
            .map(Self::Pointer)
            .or_else(|| CBoundaryType::from_source_name(class).map(Self::Boundary))
            .or_else(|| {
                crate::foreign::ForeignStringEncoding::from_source_name(class)
                    .map(Self::OwnedString)
            })
    }

    pub fn short_name(self) -> &'static str {
        match self {
            Self::Pointer(access) => access.short_name(),
            Self::Boundary(boundary) => boundary.short_name(),
            Self::OwnedString(encoding) => encoding.short_name(),
        }
    }
}

/// `Ptr::to($place)` or `MutPtr::to($place)`: the access the pointer gives.
pub fn formation(qualifier: &StaticQualifier, method: &str) -> Option<PointerAccess> {
    let StaticQualifier::Class(class) = qualifier else {
        return None;
    };
    let access = PointerAccess::from_source_name(class)?;
    (static_member(access, method)?.operation == PointerOperation::AddressOf).then_some(access)
}

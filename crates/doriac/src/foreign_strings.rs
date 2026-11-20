//! `CString` and `Utf16String` (Decision 0148 section 9, amended
//! 2026-11-20): Move values that each own one NUL-terminated buffer.
//!
//! This table is the one copy of their members. Checking types and checks
//! them through it, ownership pins a value whose `pointer` is formed, and
//! MIR lowering builds each member from it.

use crate::byte_intrinsics::{ByteIntrinsic, ByteMemberKind, ByteParameter};
use crate::foreign::{CBoundaryType, ForeignStringEncoding, ForeignType, PointerAccess};
use crate::symbols::ReceiverMode;
use crate::types::{IntegerType, ResolvedType};

/// The owned string type of `encoding`.
pub fn owned_type(encoding: ForeignStringEncoding) -> ResolvedType {
    ResolvedType::Foreign(Box::new(ForeignType::OwnedString(encoding)))
}

/// The C type of one code unit: `CChar` for `CString`, `uint16` for
/// `Utf16String`.
pub fn unit_type(encoding: ForeignStringEncoding) -> ResolvedType {
    match encoding {
        ForeignStringEncoding::Utf8 => {
            ResolvedType::Foreign(Box::new(ForeignType::Boundary(CBoundaryType::Char)))
        }
        ForeignStringEncoding::Utf16 => ResolvedType::Integer(IntegerType::UInt16),
    }
}

/// `Ptr<CChar>` or `Ptr<uint16>`: what `pointer` gives and what
/// `String::fromCString` and `String::fromUtf16` read.
pub fn unit_pointer(encoding: ForeignStringEncoding) -> ResolvedType {
    ResolvedType::Foreign(Box::new(ForeignType::Pointer {
        access: PointerAccess::Read,
        pointee: unit_type(encoding),
    }))
}

/// The encoding of an owned string type, or `None` for any other type.
pub fn encoding_of(ty: &ResolvedType) -> Option<ForeignStringEncoding> {
    match ty {
        ResolvedType::Foreign(foreign) => match foreign.as_ref() {
            ForeignType::OwnedString(encoding) => Some(*encoding),
            _ => None,
        },
        _ => None,
    }
}

/// Whether `from` panics or `tryFrom` returns `null` for text with NUL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnedStringConstruction {
    /// `CString::from($text)`: panics with
    /// [`crate::foreign::INTERIOR_NUL_PANIC_CODE`] on an interior NUL.
    From,
    /// `CString::tryFrom($text)`: `null` on an interior NUL.
    TryFrom,
}

impl OwnedStringConstruction {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "from" => Some(Self::From),
            "tryFrom" => Some(Self::TryFrom),
            _ => None,
        }
    }
}

pub const MEMBER_NAMES: [&str; 3] = ["from", "tryFrom", "pointer"];

pub fn type_documentation(encoding: ForeignStringEncoding) -> &'static str {
    match encoding {
        ForeignStringEncoding::Utf8 => {
            "A NUL-terminated copy of a string's UTF-8 bytes for C (Move). It owns its one buffer, which is released when its owner ends, with no reference count. `CString::from` copies a string, `pointer` gives C the address of its first byte in an unsafe context, and `String::fromCString` reads a C string back."
        }
        ForeignStringEncoding::Utf16 => {
            "A NUL-terminated UTF-16 copy of a string for the Windows `W` functions (Move). It owns its one buffer, which is released when its owner ends, with no reference count. `Utf16String::from` converts a string, `pointer` gives C the address of its first code unit in an unsafe context, and `String::fromUtf16` reads UTF-16 text back."
        }
    }
}

/// The member `name` of `CString` or `Utf16String`.
pub fn member(encoding: ForeignStringEncoding, name: &str) -> Option<ByteIntrinsic> {
    let owned = owned_type(encoding);
    let mut spec = ByteIntrinsic {
        name: "",
        kind: ByteMemberKind::StaticMethod,
        parameters: vec![ByteParameter {
            name: "text",
            ty: ResolvedType::String,
        }],
        return_type: owned.clone(),
        receiver_mode: None,
        accepts_named_arguments: true,
        writable_place: false,
        descriptor_metadata: false,
        unsafe_operation: None,
        documentation: "",
    };
    match (name, encoding) {
        ("from", ForeignStringEncoding::Utf8) => {
            spec.name = "from";
            spec.documentation = "Copies the string's UTF-8 bytes into a new buffer and appends NUL. Panics when the string contains NUL, which would end the C string early.";
        }
        ("from", ForeignStringEncoding::Utf16) => {
            spec.name = "from";
            spec.documentation = "Converts the string to UTF-16 in a new buffer and appends NUL. Panics when the string contains NUL, which would end the C string early.";
        }
        ("tryFrom", _) => {
            spec.name = "tryFrom";
            spec.return_type = ResolvedType::Nullable(Box::new(owned));
            spec.documentation = match encoding {
                ForeignStringEncoding::Utf8 => "Copies the string's UTF-8 bytes into a new buffer and appends NUL, or returns `null` when the string contains NUL.",
                ForeignStringEncoding::Utf16 => "Converts the string to UTF-16 in a new buffer and appends NUL, or returns `null` when the string contains NUL.",
            };
        }
        ("pointer", _) => {
            spec.name = "pointer";
            spec.kind = ByteMemberKind::Property;
            spec.parameters = Vec::new();
            spec.receiver_mode = Some(ReceiverMode::Readonly);
            spec.accepts_named_arguments = false;
            spec.return_type = unit_pointer(encoding);
            spec.unsafe_operation = Some(match encoding {
                ForeignStringEncoding::Utf8 => "forming a pointer to a `CString`",
                ForeignStringEncoding::Utf16 => "forming a pointer to a `Utf16String`",
            });
            spec.documentation = match encoding {
                ForeignStringEncoding::Utf8 => "The address of the first byte of this NUL-terminated buffer (`Ptr<CChar>`), for C functions that read a string. Needs an unsafe context. The value stays in place, and cannot be given away, replaced or released, until the innermost enclosing unsafe context ends; the pointer is valid only within that context.",
                ForeignStringEncoding::Utf16 => "The address of the first code unit of this NUL-terminated buffer (`Ptr<uint16>`), for the Windows `W` functions. Needs an unsafe context. The value stays in place, and cannot be given away, replaced or released, until the innermost enclosing unsafe context ends; the pointer is valid only within that context.",
            };
        }
        _ => return None,
    }
    Some(spec)
}

//! The types that cross the C boundary (Decision 0148 sections 3, 5, 6 and 12).
//!
//! One [`ForeignType`] carries every compiler-known foreign type, so the type
//! model gains one variant (`TypeKind::Foreign`, `ResolvedType::Foreign`) and
//! each kind stays distinct through its discriminator, as `SharedHandle` does
//! for the shared-ownership family.

use crate::ast::ForeignAggregateKind;
use crate::source::Span;

pub const NAMESPACE: &str = "Doria\\Std\\Ffi";
pub const POINTER: &str = "Doria\\Std\\Ffi\\Pointer";
pub const WRITABLE_POINTER: &str = "Doria\\Std\\Ffi\\WritablePointer";
pub const OPAQUE: &str = "Doria\\Std\\Ffi\\Opaque";

/// The panic of `CLong::from` and the other boundary types' `from` for a
/// value outside the C type's range on this target (section 5).
pub const BOUNDARY_CONVERSION_PANIC_CODE: &str = "P1701";

/// The panic of `read` and `write` at an address that is not aligned for
/// the pointee (section 4).
pub const POINTER_ALIGNMENT_PANIC_CODE: &str = "P1702";

/// The panic of `offset` and `byteOffset` when the address leaves the
/// address space or reaches zero, which no `Pointer<T>` holds (section 4).
pub const POINTER_ADDRESS_PANIC_CODE: &str = "P1703";

/// The panic of a construction that supplies a fixed-length array field
/// `T[N]` from a region whose length is not `N` (section 6).
pub const ARRAY_FIELD_LENGTH_PANIC_CODE: &str = "P1704";

/// The panic of `CString::from` and `Utf16String::from` for text that
/// contains NUL, which would end the C string early (section 9).
pub const INTERIOR_NUL_PANIC_CODE: &str = "P1705";

/// The text encoding of an owned foreign string (section 9): `CString`
/// holds UTF-8 bytes and `Utf16String` UTF-16 code units, each in one
/// NUL-terminated buffer the value owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ForeignStringEncoding {
    Utf8,
    Utf16,
}

impl ForeignStringEncoding {
    pub const ALL: [Self; 2] = [Self::Utf8, Self::Utf16];

    pub const fn source_name(self) -> &'static str {
        match self {
            Self::Utf8 => crate::compiler_known_ffi::C_STRING,
            Self::Utf16 => crate::compiler_known_ffi::UTF16_STRING,
        }
    }

    pub const fn short_name(self) -> &'static str {
        match self {
            Self::Utf8 => "CString",
            Self::Utf16 => "Utf16String",
        }
    }

    pub fn from_source_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|encoding| encoding.source_name() == name)
    }

    /// The bytes of one code unit: 1 for UTF-8, 2 for UTF-16.
    pub const fn unit_bytes(self) -> u64 {
        match self {
            Self::Utf8 => 1,
            Self::Utf16 => 2,
        }
    }

    /// The `String` companion member that reads a NUL-terminated string of
    /// this encoding from C: `String::fromCString` or `String::fromUtf16`.
    pub const fn string_reader(self) -> &'static str {
        match self {
            Self::Utf8 => "fromCString",
            Self::Utf16 => "fromUtf16",
        }
    }

    pub fn from_string_reader(member: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|encoding| encoding.string_reader() == member)
    }
}

/// Read access (`Pointer<T>`) or read and write access (`WritablePointer<T>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PointerAccess {
    Read,
    Write,
}

impl PointerAccess {
    pub const fn source_name(self) -> &'static str {
        match self {
            Self::Read => POINTER,
            Self::Write => WRITABLE_POINTER,
        }
    }

    pub const fn short_name(self) -> &'static str {
        match self {
            Self::Read => "Pointer",
            Self::Write => "WritablePointer",
        }
    }

    pub fn from_source_name(name: &str) -> Option<Self> {
        match name {
            POINTER => Some(Self::Read),
            WRITABLE_POINTER => Some(Self::Write),
            _ => None,
        }
    }
}

/// The C types whose width or signedness differs across tier-1 targets
/// (section 5). Each is an inline Copy value with the target's layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CBoundaryType {
    Char,
    Long,
    UnsignedLong,
    Size,
    PointerDifference,
    WideChar,
}

impl CBoundaryType {
    pub const ALL: [Self; 6] = [
        Self::Char,
        Self::Long,
        Self::UnsignedLong,
        Self::Size,
        Self::PointerDifference,
        Self::WideChar,
    ];

    pub const fn source_name(self) -> &'static str {
        match self {
            Self::Char => "Doria\\Std\\Ffi\\CChar",
            Self::Long => "Doria\\Std\\Ffi\\CLong",
            Self::UnsignedLong => "Doria\\Std\\Ffi\\CUnsignedLong",
            Self::Size => "Doria\\Std\\Ffi\\CSize",
            Self::PointerDifference => "Doria\\Std\\Ffi\\CPointerDifference",
            Self::WideChar => "Doria\\Std\\Ffi\\CWideChar",
        }
    }

    pub fn short_name(self) -> &'static str {
        self.source_name()
            .rsplit('\\')
            .next()
            .expect("a canonical name has a last segment")
    }

    pub fn from_source_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|boundary| boundary.source_name() == name)
    }

    /// The C spelling, for messages that name the type a binding stands for.
    pub const fn c_spelling(self) -> &'static str {
        match self {
            Self::Char => "char",
            Self::Long => "long",
            Self::UnsignedLong => "unsigned long",
            Self::Size => "size_t",
            Self::PointerDifference => "ptrdiff_t",
            Self::WideChar => "wchar_t",
        }
    }
}

/// Where `#[CaptureSystemCode]` reads the code a foreign call leaves
/// (section 16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SystemCodeSource {
    Errno,
    WindowsLastError,
}

impl SystemCodeSource {
    pub fn from_case_name(case: &str) -> Option<Self> {
        match case {
            "Errno" => Some(Self::Errno),
            "WindowsLastError" => Some(Self::WindowsLastError),
            _ => None,
        }
    }
}

/// How one extern declaration binds its C function (sections 14 to 16 and
/// 18). The checker derives it from the declaration and its block; MIR
/// lowering builds the declaration's foreign call from it, and the
/// execution gate reads it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ForeignBinding {
    /// The symbol the call links to: the `#[LinkName]` argument, otherwise
    /// the declaration's Doria name.
    pub symbol: String,
    /// The block's `#[Library]` logical name. `None` binds the platform C
    /// runtime (section 15).
    pub library: Option<String>,
    /// `#[Variadic(fixedParameters: n)]`: the count of fixed parameters.
    pub fixed_parameters: Option<usize>,
    /// `#[CaptureSystemCode(source)]`.
    pub system_code: Option<SystemCodeSource>,
}

/// How an unsafe context is written (section 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum UnsafeContextForm {
    /// `unsafe { }`.
    Block,
    /// `unsafe(<expression>)`.
    Expression,
}

/// An unsafe context written outside every other one. Each is a region,
/// which the native companion library runs as one function for the
/// interpreter and PHP output (sections 22 and 23). An unsafe context inside
/// another belongs to the outer region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UnsafeContextFact {
    /// The `unsafe` keyword.
    pub keyword: Span,
    /// The span MIR lowering sees: the block of `unsafe { }`, or the whole
    /// `unsafe(<expression>)`.
    pub span: Span,
    pub form: UnsafeContextForm,
}

/// A C function pointer type, `extern "C" function(T...): R` (section 12).
/// It has no invocation mode, ownership modes or checked effects.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CFunctionType<T> {
    pub parameters: Vec<T>,
    pub return_type: T,
}

/// A foreign struct or union declared with `extern "C"`: a nominal inline Copy
/// value with the target C layout (section 6).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ForeignAggregateType {
    pub name: String,
    pub kind: ForeignAggregateKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ForeignType<T> {
    /// `Pointer<T>` or `WritablePointer<T>`: one machine word, never null. `?Pointer<T>` is the
    /// nullable C pointer.
    Pointer {
        access: PointerAccess,
        pointee: T,
    },
    /// The pointee-only type behind `Pointer<Opaque>`: no values and no size.
    Opaque,
    Boundary(CBoundaryType),
    Function(CFunctionType<T>),
    Aggregate(ForeignAggregateType),
    /// `T[N]`, written only as a foreign struct or union field.
    FixedArray {
        element: T,
        length: u64,
    },
    /// `CString` or `Utf16String`: a Move value owning one NUL-terminated
    /// buffer, released when its owner ends (section 9, amended 2026-11-20).
    OwnedString(ForeignStringEncoding),
}

impl<T> ForeignType<T> {
    pub fn map<U>(&self, mut map: impl FnMut(&T) -> U) -> ForeignType<U> {
        match self {
            Self::Pointer { access, pointee } => ForeignType::Pointer {
                access: *access,
                pointee: map(pointee),
            },
            Self::Opaque => ForeignType::Opaque,
            Self::Boundary(boundary) => ForeignType::Boundary(*boundary),
            Self::Function(function) => ForeignType::Function(CFunctionType {
                parameters: function.parameters.iter().map(&mut map).collect(),
                return_type: map(&function.return_type),
            }),
            Self::Aggregate(aggregate) => ForeignType::Aggregate(aggregate.clone()),
            Self::FixedArray { element, length } => ForeignType::FixedArray {
                element: map(element),
                length: *length,
            },
            Self::OwnedString(encoding) => ForeignType::OwnedString(*encoding),
        }
    }

    /// The Doria spelling, with `show` spelling each component type.
    pub fn display_with(&self, show: impl Fn(&T) -> String) -> String {
        match self {
            Self::Pointer { access, pointee } => {
                format!("{}<{}>", access.short_name(), show(pointee))
            }
            Self::Opaque => "Opaque".to_string(),
            Self::Boundary(boundary) => boundary.short_name().to_string(),
            Self::Function(function) => format!(
                "extern \"C\" function({}): {}",
                function
                    .parameters
                    .iter()
                    .map(&show)
                    .collect::<Vec<_>>()
                    .join(", "),
                show(&function.return_type)
            ),
            Self::Aggregate(aggregate) => aggregate.name.clone(),
            Self::FixedArray { element, length } => format!("{}[{length}]", show(element)),
            Self::OwnedString(encoding) => encoding.short_name().to_string(),
        }
    }

    /// Every type this one is built from, in source order.
    pub fn components(&self) -> Vec<&T> {
        match self {
            Self::Pointer { pointee, .. } => vec![pointee],
            Self::Function(function) => function
                .parameters
                .iter()
                .chain(std::iter::once(&function.return_type))
                .collect(),
            Self::FixedArray { element, .. } => vec![element],
            Self::Opaque | Self::Boundary(_) | Self::Aggregate(_) | Self::OwnedString(_) => {
                Vec::new()
            }
        }
    }
}

/// Every compiler-known type name of `Doria\Std\Ffi` that the type model
/// carries itself, rather than a declaration in the module source.
pub fn type_names() -> impl Iterator<Item = &'static str> {
    [POINTER, WRITABLE_POINTER, OPAQUE]
        .into_iter()
        .chain(
            CBoundaryType::ALL
                .into_iter()
                .map(CBoundaryType::source_name),
        )
        .chain(
            ForeignStringEncoding::ALL
                .into_iter()
                .map(ForeignStringEncoding::source_name),
        )
}

pub fn is_type_name(name: &str) -> bool {
    type_names().any(|candidate| candidate == name)
}

/// Whether a value of `ty` crosses the C boundary unchanged (section 6):
/// fixed-width integers and floats, `bool`, pointers and their nullable
/// forms, the boundary types, C function pointers and foreign structs and
/// unions.
pub fn is_ffi_safe(ty: &crate::types::ResolvedType) -> bool {
    use crate::types::ResolvedType;
    match ty {
        ResolvedType::Integer(_) | ResolvedType::Float(_) | ResolvedType::Bool => true,
        ResolvedType::Foreign(foreign) => match foreign.as_ref() {
            ForeignType::Pointer { .. }
            | ForeignType::Boundary(_)
            | ForeignType::Function(_)
            | ForeignType::Aggregate(_) => true,
            ForeignType::Opaque | ForeignType::FixedArray { .. } | ForeignType::OwnedString(_) => {
                false
            }
        },
        ResolvedType::Nullable(inner) => matches!(
            inner.as_ref(),
            ResolvedType::Foreign(foreign)
                if matches!(foreign.as_ref(), ForeignType::Pointer { .. } | ForeignType::Function(_))
        ),
        _ => false,
    }
}

/// What checking establishes about one foreign struct or union (Decision
/// 0148 section 6): its fields in declaration order and, when it has a body,
/// its C layout on the native target. MIR lowering builds the aggregate's
/// MIR definition, its construction and its field accesses from these facts
/// alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignAggregateFacts {
    pub kind: ForeignAggregateKind,
    /// Empty for an incomplete type declared without a body.
    pub fields: Vec<ForeignFieldFacts>,
    /// `None` for an incomplete type, and for one whose layout does not
    /// exist (an aggregate that contains itself, reported by checking).
    pub layout: Option<std::sync::Arc<crate::c_layout::CRecord>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignFieldFacts {
    pub name: String,
    pub ty: crate::types::ResolvedType,
    /// The declared constant default. A fixed-length array field has none:
    /// omitted, it is zero-filled.
    pub default: Option<crate::const_eval::ConstValue>,
}

impl ForeignAggregateFacts {
    /// The index of the field `name` and its facts.
    pub fn field(&self, name: &str) -> Option<(usize, &ForeignFieldFacts)> {
        self.fields
            .iter()
            .enumerate()
            .find(|(_, field)| field.name == name)
    }

    /// `SIZE` or `ALIGNMENT` of the layout; `None` for any other member or
    /// an aggregate without a layout.
    pub fn layout_constant(&self, member: &str) -> Option<u64> {
        let layout = self.layout.as_ref()?;
        match member {
            "SIZE" => Some(layout.size),
            "ALIGNMENT" => Some(layout.align),
            _ => None,
        }
    }
}

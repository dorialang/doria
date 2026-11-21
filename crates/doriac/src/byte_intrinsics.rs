//! Source contracts for owned bytes and inline borrowed byte regions.

use crate::symbols::{BindingOwnership, ParamInfo, ReceiverMode};
use crate::types::{ByteRegionKind, IntegerType, ResolvedType, TypeRegistry};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteMemberKind {
    StaticMethod,
    Method,
    Property,
    Index,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ByteParameter {
    pub name: &'static str,
    pub ty: ResolvedType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ByteIntrinsic {
    pub name: &'static str,
    pub kind: ByteMemberKind,
    pub parameters: Vec<ByteParameter>,
    pub return_type: ResolvedType,
    pub receiver_mode: Option<ReceiverMode>,
    pub accepts_named_arguments: bool,
    pub writable_place: bool,
    /// Reads fixed descriptor metadata without accessing the selected bytes.
    pub descriptor_metadata: bool,
    /// What the member does when it needs an unsafe context, completing
    /// "… needs an unsafe context"; `None` for a safe member.
    pub unsafe_operation: Option<&'static str>,
    pub documentation: &'static str,
}

impl ByteIntrinsic {
    /// All byte parameters are required readonly inputs; none consumes its argument.
    pub fn parameter_info(&self, types: &mut TypeRegistry) -> Vec<ParamInfo> {
        self.parameters
            .iter()
            .map(|parameter| ParamInfo {
                name: parameter.name.to_string(),
                ty: types.intern_resolved(&parameter.ty),
                take: false,
                writable: false,
                has_default: false,
                default: None,
                declaration: None,
            })
            .collect()
    }

    /// A borrowed result retains the receiver's source provenance, not its ownership.
    pub fn result_ownership(&self) -> BindingOwnership {
        match self.return_type {
            ResolvedType::ByteRegion(ByteRegionKind::Readonly) => BindingOwnership::ReadonlyBorrow,
            ResolvedType::ByteRegion(ByteRegionKind::Writable) => BindingOwnership::WritableBorrow,
            _ => BindingOwnership::Owned,
        }
    }
}

pub fn type_from_source_name(name: &str) -> Option<ResolvedType> {
    if name == "Bytes" {
        Some(ResolvedType::Bytes)
    } else {
        ByteRegionKind::from_source_name(name).map(ResolvedType::ByteRegion)
    }
}

pub fn type_documentation(ty: &ResolvedType) -> Option<&'static str> {
    match ty {
        ResolvedType::Bytes => Some(
            "Owned, fixed-length mutable byte storage (Move), with byte-wise equality. `allocate` creates zero-initialized storage; `fromArray` and `toArray` explicitly copy between `Bytes` and `uint8[]`. `length` counts bytes. Indexing reads `uint8`; writes require writable access. `getRegion` and `getWritableRegion` borrow the storage without copying or extending its lifetime.",
        ),
        ResolvedType::ByteRegion(ByteRegionKind::Readonly) => Some(
            "Readonly borrowed byte region (Copy), carried inline without owning storage or retaining a runtime reference count. `length` counts bytes; indexing reads `uint8`. `getRegion` reborrows a subregion. In an unsafe context `pointer` gives the address of its first byte for C. The source must outlive every use and cannot be mutated, moved, or destroyed while borrowed. A writable binding does not make this region writable.",
        ),
        ResolvedType::ByteRegion(ByteRegionKind::Writable) => Some(
            "Exclusive borrowed byte region (Move), carried inline without owning storage or retaining a runtime reference count. `length` counts bytes; indexing reads `uint8`. Indexed writes and `getWritableRegion` require writable access. `getRegion` creates a readonly reborrow. `copyFrom` copies another region's bytes into this one and `copyWithin` moves bytes inside it, both in bulk. In an unsafe context `pointer` and, with writable access, `writablePointer` give the address of its first byte for C. The source must outlive every use; its complete place remains exclusively borrowed. Subregions do not establish disjoint ownership.",
        ),
        ResolvedType::ArrayRegion(ByteRegionKind::Readonly, _) => Some(
            "Readonly borrowed view of elements of a `T[]` (Copy), carried inline without owning storage or retaining a runtime reference count. `length` counts elements; indexing reads `T`. `getRegion` reborrows a subregion. When `T` is FFI-safe, `pointer` gives the address of its first element in an unsafe context. The source must outlive every use and cannot be mutated, moved, or destroyed while borrowed.",
        ),
        ResolvedType::ArrayRegion(ByteRegionKind::Writable, _) => Some(
            "Exclusive borrowed view of elements of a `T[]` (Move), carried inline without owning storage or retaining a runtime reference count. `length` counts elements; indexing reads `T`. Indexed writes and `getWritableRegion` require writable access. `getRegion` creates a readonly reborrow. `copyFrom` copies another region's elements into this one. When `T` is FFI-safe, `pointer` and, with writable access, `writablePointer` give the address of its first element in an unsafe context. The source's complete place remains exclusively borrowed.",
        ),
        ty => crate::foreign_strings::encoding_of(ty).map(crate::foreign_strings::type_documentation),
    }
}

pub fn members(receiver: &ResolvedType, kind: ByteMemberKind) -> Vec<ByteIntrinsic> {
    [
        "allocate",
        "fromArray",
        "toArray",
        "getRegion",
        "getWritableRegion",
        "copyFrom",
        "copyWithin",
        "length",
        "byteRegion",
        "pointer",
        "writablePointer",
        "[]",
        "from",
        "tryFrom",
    ]
    .into_iter()
    .filter_map(|name| member(receiver, name))
    .filter(|member| member.kind == kind)
    .collect()
}

/// The static member of the four region companions (Decision 0148 section
/// 10): `ByteRegion::fromPointer($owner, $pointer, $length)`,
/// `WritableByteRegion::fromPointer(writable $owner, ...)` and the typed
/// `ArrayRegion::fromPointer` and `WritableArrayRegion::fromPointer`.
pub const FROM_POINTER: &str = "fromPointer";

/// The parameters of [`FROM_POINTER`], in declaration order. The region
/// borrows the first, `owner`.
pub const FROM_POINTER_PARAMETERS: [&str; 3] = ["owner", "pointer", "length"];

/// A region companion a static call names: the region's access and
/// whether it is a typed array region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionCompanion {
    pub kind: ByteRegionKind,
    pub typed: bool,
}

impl RegionCompanion {
    /// The companion a canonical class name names, if it is one of the four
    /// region types.
    pub fn from_class_name(name: &str) -> Option<Self> {
        ByteRegionKind::from_source_name(name)
            .map(|kind| Self { kind, typed: false })
            .or_else(|| {
                ByteRegionKind::from_array_source_name(name).map(|kind| Self { kind, typed: true })
            })
    }

    /// The companion and member of a `fromPointer` call, from its qualifier.
    pub fn from_pointer_call(
        qualifier: &crate::ast::StaticQualifier,
        method: &str,
    ) -> Option<Self> {
        match qualifier {
            crate::ast::StaticQualifier::Class(name) if method == FROM_POINTER => {
                Self::from_class_name(name)
            }
            _ => None,
        }
    }

    /// The name source spells, without its namespace.
    pub fn short_name(self) -> &'static str {
        if self.typed {
            self.kind.array_source_name()
        } else {
            self.kind
                .source_name()
                .rsplit('\\')
                .next()
                .expect("a region name has a last segment")
        }
    }

    /// The pointer access `fromPointer` takes: write access for a writable
    /// region, which C fills through.
    pub const fn pointer_access(self) -> crate::foreign::PointerAccess {
        if self.kind.is_writable() {
            crate::foreign::PointerAccess::Write
        } else {
            crate::foreign::PointerAccess::Read
        }
    }

    /// The region type over elements of `element`; a byte region ignores it.
    pub fn region_type(self, element: ResolvedType) -> ResolvedType {
        if self.typed {
            ResolvedType::ArrayRegion(self.kind, Box::new(element))
        } else {
            ResolvedType::ByteRegion(self.kind)
        }
    }

    pub const fn documentation(self) -> &'static str {
        match (self.typed, self.kind.is_writable()) {
            (false, false) => "Views `length` bytes of memory a foreign library owns, starting at `pointer`, as a readonly region (Decision 0148 section 10). Needs an unsafe context. The region borrows `owner`, the Doria value whose destructor releases the memory, so it cannot outlive it; a method returning the region borrows `$this`. A negative length, or a range past the end of the address space, panics with P1703. The memory is not copied, and Doria never releases it.",
            (false, true) => "Views `length` bytes of memory a foreign library owns, starting at `pointer`, as a writable region (Decision 0148 section 10). Needs an unsafe context and writable access to `owner`, the Doria value whose destructor releases the memory; the region borrows it, so it cannot outlive it. A negative length, or a range past the end of the address space, panics with P1703. The memory is not copied, and Doria never releases it.",
            (true, false) => "Views `length` elements of type `T` of memory a foreign library owns, starting at `pointer`, as a readonly region (Decision 0148 section 10); `T` is the pointer's pointee. Needs an unsafe context. The region borrows `owner`, the Doria value whose destructor releases the memory, so it cannot outlive it. A pointer not aligned for `T` panics with P1702; a negative length, or a range past the end of the address space, panics with P1703. The memory is not copied, and Doria never releases it.",
            (true, true) => "Views `length` elements of type `T` of memory a foreign library owns, starting at `pointer`, as a writable region (Decision 0148 section 10); `T` is the pointer's pointee. Needs an unsafe context and writable access to `owner`, the Doria value whose destructor releases the memory; the region borrows it, so it cannot outlive it. A pointer not aligned for `T` panics with P1702; a negative length, or a range past the end of the address space, panics with P1703. The memory is not copied, and Doria never releases it.",
        }
    }
}

/// The source-order argument of a `fromPointer` call bound to parameter
/// `parameter` under named-argument binding (Decision 0098), with its index.
pub fn from_pointer_argument(
    args: &[crate::ast::Argument],
    parameter: usize,
) -> Option<(usize, &crate::ast::Argument)> {
    let index = from_pointer_binding(
        &args
            .iter()
            .map(|argument| argument.name.as_ref().map(|name| name.text.as_str()))
            .collect::<Vec<_>>(),
    )
    .param_to_arg
    .get(parameter)
    .copied()
    .flatten()?;
    Some((index, args.get(index)?))
}

/// The binding of `fromPointer`'s arguments, by their names in source
/// order, onto [`FROM_POINTER_PARAMETERS`].
pub fn from_pointer_binding(names: &[Option<&str>]) -> crate::arg_binding::BoundArguments {
    crate::arg_binding::bind_arguments(&FROM_POINTER_PARAMETERS, &[false; 3], names)
}

/// A string's UTF-8 bytes as a readonly region (Stage 36a decision log entry
/// 49). Decision 0103 keeps the string instance surface to properties for
/// data, measurements and views; this is the borrowed byte view, beside
/// `bytes`, which copies. Ranges come from the region's own `getRegion`.
/// Its name and documentation are the String surface table's entry.
fn string_member(name: &str) -> Option<ByteIntrinsic> {
    let entry = crate::string_surface::property(name)
        .filter(|entry| entry.result == crate::string_surface::StringValueType::ByteRegion)?;
    Some(ByteIntrinsic {
        name: entry.name,
        kind: ByteMemberKind::Property,
        parameters: Vec::new(),
        return_type: ResolvedType::ByteRegion(ByteRegionKind::Readonly),
        receiver_mode: Some(ReceiverMode::Readonly),
        accepts_named_arguments: false,
        writable_place: false,
        descriptor_metadata: false,
        unsafe_operation: None,
        documentation: entry.documentation,
    })
}

/// Exact receiver types only. Nullable access and shared forwarding are resolved by the caller.
///
/// Typed array regions (Decision 0148 section 8) mirror the byte regions:
/// `T[]` has `getRegion` and `getWritableRegion` (its other members are the
/// typed array's own), and `ArrayRegion<T>` and `WritableArrayRegion<T>`
/// have the region members over elements of `T`, without `copyWithin`, with
/// `pointer` and `writablePointer` when `T` is FFI-safe.
pub fn member(receiver: &ResolvedType, name: &str) -> Option<ByteIntrinsic> {
    let byte = ResolvedType::Integer(IntegerType::UInt8);
    let (writable_storage, element, typed) = match receiver {
        ResolvedType::Bytes | ResolvedType::ByteRegion(ByteRegionKind::Writable) => {
            (true, byte.clone(), false)
        }
        ResolvedType::ByteRegion(ByteRegionKind::Readonly) => (false, byte.clone(), false),
        ResolvedType::TypedArray(element) => {
            if !matches!(name, "getRegion" | "getWritableRegion") {
                return None;
            }
            (true, element.as_ref().clone(), true)
        }
        ResolvedType::ArrayRegion(kind, element) => {
            (kind.is_writable(), element.as_ref().clone(), true)
        }
        ResolvedType::String => return string_member(name),
        ResolvedType::Foreign(_) => {
            return crate::foreign_strings::member(
                crate::foreign_strings::encoding_of(receiver)?,
                name,
            )
        }
        _ => return None,
    };
    let region = |kind: ByteRegionKind| {
        if typed {
            ResolvedType::ArrayRegion(kind, Box::new(element.clone()))
        } else {
            ResolvedType::ByteRegion(kind)
        }
    };
    let int = ResolvedType::Integer(IntegerType::Int64);
    let mut spec = ByteIntrinsic {
        name: "",
        kind: ByteMemberKind::Method,
        parameters: Vec::new(),
        return_type: ResolvedType::Void,
        receiver_mode: Some(ReceiverMode::Readonly),
        accepts_named_arguments: false,
        writable_place: false,
        descriptor_metadata: false,
        unsafe_operation: None,
        documentation: "",
    };
    match name {
        "allocate" if matches!(receiver, ResolvedType::Bytes) => {
            spec.name = "allocate";
            spec.kind = ByteMemberKind::StaticMethod;
            spec.receiver_mode = None;
            spec.parameters.push(ByteParameter {
                name: "length",
                ty: int,
            });
            spec.accepts_named_arguments = true;
            spec.return_type = ResolvedType::Bytes;
            spec.documentation = "Allocates fixed-length, zero-initialized byte storage. A negative length panics; allocation failure follows the ordinary allocation panic contract.";
        }
        "fromArray" if matches!(receiver, ResolvedType::Bytes) => {
            spec.name = "fromArray";
            spec.kind = ByteMemberKind::StaticMethod;
            spec.receiver_mode = None;
            spec.parameters.push(ByteParameter {
                name: "bytes",
                ty: ResolvedType::TypedArray(Box::new(byte)),
            });
            spec.return_type = ResolvedType::Bytes;
            spec.documentation = "Copies a fixed-length `uint8[]` into owned byte storage. The input is not consumed. Arguments are positional only.";
        }
        "toArray" if matches!(receiver, ResolvedType::Bytes) => {
            spec.name = "toArray";
            spec.return_type = ResolvedType::TypedArray(Box::new(byte));
            spec.documentation = "Copies this byte storage into a fixed-length `uint8[]`. The receiver is not consumed.";
        }
        "getRegion" | "getWritableRegion" if name == "getRegion" || writable_storage => {
            let writable = name == "getWritableRegion";
            spec.name = if writable {
                "getWritableRegion"
            } else {
                "getRegion"
            };
            spec.parameters = ["offset", "length"]
                .map(|name| ByteParameter {
                    name,
                    ty: int.clone(),
                })
                .into();
            spec.accepts_named_arguments = true;
            spec.receiver_mode = Some(if writable {
                ReceiverMode::Writable
            } else {
                ReceiverMode::Readonly
            });
            spec.return_type = region(if writable {
                ByteRegionKind::Writable
            } else {
                ByteRegionKind::Readonly
            });
            spec.documentation = if typed {
                "Borrows a region of elements without allocation, copying elements, or extending the source lifetime. Offsets and lengths are nonnegative element counts; an empty region at the end is valid. Invalid bounds panic. The result retains the source provenance and the borrow applies to the complete source place."
            } else {
                "Borrows a byte subregion without allocation, copying bytes, or extending the source lifetime. Offsets and lengths are nonnegative byte counts; an empty region at the end is valid. Invalid bounds panic. The result retains the source provenance and the borrow applies to the complete source place."
            };
        }
        "copyFrom"
            if matches!(
                receiver,
                ResolvedType::ByteRegion(ByteRegionKind::Writable)
                    | ResolvedType::ArrayRegion(ByteRegionKind::Writable, _)
            ) =>
        {
            spec.name = "copyFrom";
            spec.parameters.push(ByteParameter {
                name: "source",
                ty: region(ByteRegionKind::Readonly),
            });
            spec.receiver_mode = Some(ReceiverMode::Writable);
            spec.return_type = int;
            spec.documentation = if typed {
                "Copies elements from the start of `source` to the start of this region and returns the count: the smaller of the two lengths. Neither region is resized and no storage is allocated. Requires writable access to this region."
            } else {
                "Copies bytes from the start of `source` to the start of this region and returns the count: the smaller of the two lengths. Neither region is resized and no storage is allocated. Requires writable access to this region."
            };
        }
        "copyWithin" if matches!(receiver, ResolvedType::ByteRegion(ByteRegionKind::Writable)) => {
            spec.name = "copyWithin";
            spec.parameters = ["offset", "length", "destination"]
                .map(|name| ByteParameter {
                    name,
                    ty: int.clone(),
                })
                .into();
            spec.accepts_named_arguments = true;
            spec.receiver_mode = Some(ReceiverMode::Writable);
            spec.documentation = "Copies `length` bytes starting at `offset` to `destination` inside this region. The ranges may overlap. Both ranges must lie within the region; invalid bounds panic. Requires writable access to this region.";
        }
        // Decision 0148 section 7: the address of the region's first byte,
        // valid until the innermost enclosing unsafe context ends. Forming
        // it pins the region's storage for that context.
        "pointer" | "writablePointer"
            if (matches!(receiver, ResolvedType::ByteRegion(_))
                || matches!(receiver, ResolvedType::ArrayRegion(..))
                    && (crate::foreign::is_ffi_safe(&element)
                        || matches!(element, ResolvedType::TypeParameter(_))))
                && (name == "pointer" || writable_storage) =>
        {
            let writable = name == "writablePointer";
            spec.name = if writable {
                "writablePointer"
            } else {
                "pointer"
            };
            spec.kind = ByteMemberKind::Property;
            spec.receiver_mode = Some(if writable {
                ReceiverMode::Writable
            } else {
                ReceiverMode::Readonly
            });
            spec.return_type =
                ResolvedType::Foreign(Box::new(crate::foreign::ForeignType::Pointer {
                    access: if writable {
                        crate::foreign::PointerAccess::Write
                    } else {
                        crate::foreign::PointerAccess::Read
                    },
                    pointee: element.clone(),
                }));
            spec.unsafe_operation = Some(if writable {
                "forming a writable pointer to a region"
            } else {
                "forming a pointer to a region"
            });
            spec.documentation = if typed && writable {
                "The address of this region's first element, with write access (`WritablePointer<T>`), for C functions that fill an array. Needs an unsafe context and writable access to the region. The region's storage stays in place, and cannot be moved, replaced or released, until the innermost enclosing unsafe context ends; the pointer is valid only within that context. An empty region gives a non-null address aligned for `T` that must not be dereferenced."
            } else if typed {
                "The address of this region's first element (`Pointer<T>`), for C functions that read an array. Needs an unsafe context. The region's storage stays in place, and cannot be moved, replaced or released, until the innermost enclosing unsafe context ends; the pointer is valid only within that context. An empty region gives a non-null address aligned for `T` that must not be dereferenced."
            } else if writable {
                "The address of this region's first byte, with write access (`WritablePointer<uint8>`), for C functions that fill a buffer. Needs an unsafe context and writable access to the region. The region's storage stays in place, and cannot be moved, replaced or released, until the innermost enclosing unsafe context ends; the pointer is valid only within that context. An empty region gives a non-null address that must not be dereferenced."
            } else {
                "The address of this region's first byte (`Pointer<uint8>`), for C functions that read a buffer. Needs an unsafe context. The region's storage stays in place, and cannot be moved, replaced or released, until the innermost enclosing unsafe context ends; the pointer is valid only within that context. An empty region gives a non-null address that must not be dereferenced."
            };
        }
        "length" => {
            spec.name = "length";
            spec.kind = ByteMemberKind::Property;
            spec.return_type = int;
            spec.descriptor_metadata = true;
            spec.documentation = if typed {
                "Readonly number of elements in this region."
            } else {
                "Readonly number of bytes in this storage or region."
            };
        }
        "[]" => {
            spec.name = "[]";
            spec.kind = ByteMemberKind::Index;
            spec.parameters.push(ByteParameter {
                name: "index",
                ty: int,
            });
            spec.return_type = element.clone();
            spec.writable_place = writable_storage;
            spec.documentation = if typed {
                "Reads one element by its zero-based index. Invalid bounds panic as `T[]` indexing does. Writes require a writable region and writable access to it."
            } else {
                "Reads one byte by its zero-based index. Invalid bounds panic. Writes and read-modify-write require writable storage and writable access to the receiver."
            };
        }
        _ => return None,
    }
    Some(spec)
}

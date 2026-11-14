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
            "Readonly borrowed byte region (Copy), carried inline without owning storage or retaining a runtime reference count. `length` counts bytes; indexing reads `uint8`. `getRegion` reborrows a subregion. The source must outlive every use and cannot be mutated, moved, or destroyed while borrowed. A writable binding does not make this region writable.",
        ),
        ResolvedType::ByteRegion(ByteRegionKind::Writable) => Some(
            "Exclusive borrowed byte region (Move), carried inline without owning storage or retaining a runtime reference count. `length` counts bytes; indexing reads `uint8`. Indexed writes and `getWritableRegion` require writable access. `getRegion` creates a readonly reborrow. `copyFrom` copies another region's bytes into this one and `copyWithin` moves bytes inside it, both in bulk. The source must outlive every use; its complete place remains exclusively borrowed. Subregions do not establish disjoint ownership.",
        ),
        _ => None,
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
        "[]",
    ]
    .into_iter()
    .filter_map(|name| member(receiver, name))
    .filter(|member| member.kind == kind)
    .collect()
}

/// A string's UTF-8 bytes as a readonly region (Stage 36a decision log entry
/// 49). Decision 0103 keeps the string instance surface to properties for
/// data, measurements and views; this is the borrowed byte view, beside
/// `bytes`, which copies. Ranges come from the region's own `getRegion`.
fn string_member(name: &str) -> Option<ByteIntrinsic> {
    (name == "byteRegion").then(|| ByteIntrinsic {
        name: "byteRegion",
        kind: ByteMemberKind::Property,
        parameters: Vec::new(),
        return_type: ResolvedType::ByteRegion(ByteRegionKind::Readonly),
        receiver_mode: Some(ReceiverMode::Readonly),
        accepts_named_arguments: false,
        writable_place: false,
        descriptor_metadata: false,
        documentation: "Borrows this string's UTF-8 bytes as a readonly region without allocation or copying. Use the region's `getRegion` for a byte range. The region keeps the string borrowed while it is in use.",
    })
}

/// Exact receiver types only. Nullable access and shared forwarding are resolved by the caller.
pub fn member(receiver: &ResolvedType, name: &str) -> Option<ByteIntrinsic> {
    let writable_storage = match receiver {
        ResolvedType::Bytes | ResolvedType::ByteRegion(ByteRegionKind::Writable) => true,
        ResolvedType::ByteRegion(ByteRegionKind::Readonly) => false,
        ResolvedType::String => return string_member(name),
        _ => return None,
    };
    let int = ResolvedType::Integer(IntegerType::Int64);
    let byte = ResolvedType::Integer(IntegerType::UInt8);
    let mut spec = ByteIntrinsic {
        name: "",
        kind: ByteMemberKind::Method,
        parameters: Vec::new(),
        return_type: ResolvedType::Void,
        receiver_mode: Some(ReceiverMode::Readonly),
        accepts_named_arguments: false,
        writable_place: false,
        descriptor_metadata: false,
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
            spec.return_type = ResolvedType::ByteRegion(if writable {
                ByteRegionKind::Writable
            } else {
                ByteRegionKind::Readonly
            });
            spec.documentation = "Borrows a byte subregion without allocation, copying bytes, or extending the source lifetime. Offsets and lengths are nonnegative byte counts; an empty region at the end is valid. Invalid bounds panic. The result retains the source provenance and the borrow applies to the complete source place.";
        }
        "copyFrom" if matches!(receiver, ResolvedType::ByteRegion(ByteRegionKind::Writable)) => {
            spec.name = "copyFrom";
            spec.parameters.push(ByteParameter {
                name: "source",
                ty: ResolvedType::ByteRegion(ByteRegionKind::Readonly),
            });
            spec.receiver_mode = Some(ReceiverMode::Writable);
            spec.return_type = int;
            spec.documentation = "Copies bytes from the start of `source` to the start of this region and returns the count: the smaller of the two lengths. Neither region is resized and no storage is allocated. Requires writable access to this region.";
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
        "length" => {
            spec.name = "length";
            spec.kind = ByteMemberKind::Property;
            spec.return_type = int;
            spec.descriptor_metadata = true;
            spec.documentation = "Readonly number of bytes in this storage or region.";
        }
        "[]" => {
            spec.name = "[]";
            spec.kind = ByteMemberKind::Index;
            spec.parameters.push(ByteParameter {
                name: "index",
                ty: int,
            });
            spec.return_type = byte;
            spec.writable_place = writable_storage;
            spec.documentation = "Reads one byte by its zero-based index. Invalid bounds panic. Writes and read-modify-write require writable storage and writable access to the receiver.";
        }
        _ => return None,
    }
    Some(spec)
}

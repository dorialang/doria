//! Unstable native-oriented MIR.
//!
//! MIR is the compiler-internal, backend-independent representation that Stage
//! 11 grows into the debug/interpreter oracle and future native lowering input.
//! The text dump is deterministic but not a stable public format.

use std::fmt;

use crate::class_layout::{ClassId, ClassLayout, PropertyId};
use crate::enums::{
    EnumBackingType, EnumBackingValue, EnumCapabilities, EnumCaseId, EnumId, EnumLayout, EnumValue,
};
use crate::format_string::FormatPiece;
use crate::numeric::{FloatType, FloatValue, IntegerType, IntegerValue};
use crate::source::{SourceFile, Span};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FunctionId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LocalId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StaticId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CollectionTypeId(pub usize);

#[derive(Debug, Clone)]
pub struct Program {
    pub source: SourceFile,
    pub classes: Vec<Class>,
    pub enums: Vec<EnumDefinition>,
    pub collection_types: Vec<CollectionType>,
    pub statics: Vec<StaticProperty>,
    pub functions: Vec<Function>,
    pub entry: FunctionId,
}

impl PartialEq for Program {
    fn eq(&self, other: &Self) -> bool {
        self.classes == other.classes
            && self.enums == other.enums
            && self.collection_types == other.collection_types
            && self.statics == other.statics
            && self.functions == other.functions
            && self.entry == other.entry
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumDefinition {
    pub id: EnumId,
    pub name: String,
    pub backing_type: Option<EnumBackingType>,
    pub cases: Vec<EnumCaseDefinition>,
    pub capabilities: EnumCapabilities,
    pub layout: EnumLayout,
}

impl EnumDefinition {
    pub fn payload_type(&self) -> Option<PayloadEnumType> {
        self.cases
            .iter()
            .any(|case| !case.payload.is_empty())
            .then(|| {
                let nullable = crate::enums::nullable_enum_layout(&self.layout)
                    .expect("validated payload enum has finite nullable layout");
                PayloadEnumType {
                    id: self.id,
                    capabilities: self.capabilities,
                    size: self.layout.size,
                    align: self.layout.align,
                    nullable_size: nullable.size,
                    nullable_payload_offset: nullable.payload_offset,
                }
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumCaseDefinition {
    pub id: EnumCaseId,
    pub name: String,
    pub tag: u32,
    pub backing_value: Option<EnumBackingValue>,
    pub payload: Vec<EnumPayloadDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumPayloadDefinition {
    pub name: String,
    pub ty: Type,
}

impl Eq for Program {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CollectionKind {
    Bytes,
    TypedArray,
    List,
    Dictionary,
    SortedDictionary,
    Set,
    SortedSet,
    PriorityQueue,
    Deque,
}

impl CollectionKind {
    pub const fn is_dictionary(self) -> bool {
        matches!(self, Self::Dictionary | Self::SortedDictionary)
    }

    pub const fn is_set(self) -> bool {
        matches!(self, Self::Set | Self::SortedSet)
    }

    pub const fn is_ordered(self) -> bool {
        matches!(
            self,
            Self::SortedDictionary | Self::SortedSet | Self::PriorityQueue
        )
    }

    pub const fn supports_foreach(self) -> bool {
        !matches!(self, Self::PriorityQueue)
    }

    pub const fn supports_writable_element_iteration(self) -> bool {
        matches!(
            self,
            Self::TypedArray | Self::List | Self::Dictionary | Self::SortedDictionary | Self::Deque
        )
    }
}

/// Backend-neutral ordering identity for collection kinds whose public
/// semantics depend on a total order. This prevents native backends from
/// guessing signedness or delegating string order to their host platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CollectionComparator {
    SignedInteger(u8),
    UnsignedInteger(u8),
    Bool,
    StringBytes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionType {
    pub id: CollectionTypeId,
    pub kind: CollectionKind,
    pub key: Option<Type>,
    pub value: Type,
    pub comparator: Option<CollectionComparator>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticProperty {
    pub id: StaticId,
    pub class: ClassId,
    pub name: String,
    pub ty: Type,
    pub writable: bool,
    pub initializer: StaticValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StaticValue {
    Scalar(ScalarValue),
    String(String),
    Null,
    PayloadEnum(PayloadEnumConstant),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayloadEnumConstant {
    pub ty: PayloadEnumType,
    pub case: EnumCaseId,
    pub fields: Vec<StaticValue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Class {
    pub id: ClassId,
    pub name: String,
    pub properties: Vec<Property>,
    pub layout: ClassLayout,
    pub constructor: Option<FunctionId>,
    pub destructor: Option<FunctionId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    pub id: PropertyId,
    pub name: String,
    pub ty: Type,
    pub writable: bool,
    pub promoted: bool,
}

#[derive(Debug, Clone)]
pub struct Function {
    pub id: FunctionId,
    pub name: String,
    pub source_span: Span,
    pub method: Option<MethodIdentity>,
    pub receiver_mode: Option<ReceiverMode>,
    pub params: Vec<LocalId>,
    pub return_type: ReturnType,
    pub locals: Vec<Local>,
    pub blocks: Vec<BasicBlock>,
    pub entry_block: BlockId,
}

impl PartialEq for Function {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.name == other.name
            && self.method == other.method
            && self.receiver_mode == other.receiver_mode
            && self.params == other.params
            && self.return_type == other.return_type
            && self.locals == other.locals
            && self.blocks == other.blocks
            && self.entry_block == other.entry_block
    }
}

impl Eq for Function {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodIdentity {
    pub class: ClassId,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiverMode {
    Readonly,
    Writable,
    /// Reserved for a future accepted consuming-receiver design.
    UnsupportedConsuming,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReturnBorrow {
    pub source: BorrowSource,
    pub writable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorrowSource {
    Receiver,
    Parameter(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReturnType {
    Value(Type),
    Void,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScalarType {
    Integer(IntegerType),
    Float(FloatType),
    Bool,
    /// A nominal unit/backed enum whose physical tag width is backend-private.
    Enum(EnumId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScalarValue {
    Integer(IntegerValue),
    Float(FloatValue),
    Bool(bool),
    Enum(EnumValue),
}

impl ScalarValue {
    pub const fn ty(self) -> ScalarType {
        match self {
            Self::Integer(value) => ScalarType::Integer(value.ty),
            Self::Float(value) => ScalarType::Float(value.ty),
            Self::Bool(_) => ScalarType::Bool,
            Self::Enum(value) => ScalarType::Enum(value.enum_id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Local {
    pub id: LocalId,
    pub name: String,
    pub ty: Type,
    pub writable: bool,
    pub owned: bool,
    pub synthetic: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Type {
    Scalar(ScalarType),
    String,
    Mixed,
    NullableScalar(ScalarType),
    NullableString,
    NullableMixed,
    Class(ClassId),
    NullableClass(ClassId),
    SharedReference(ClassId),
    WeakReference(ClassId),
    NullableSharedReference(ClassId),
    NullableWeakReference(ClassId),
    WritableSharedReference(WritableSharedPayload),
    WritableWeakReference(WritableSharedPayload),
    NullableWritableSharedReference(WritableSharedPayload),
    NullableWritableWeakReference(WritableSharedPayload),
    ReadonlySharedReferenceAccess(WritableSharedPayload),
    WritableSharedReferenceAccess(WritableSharedPayload),
    NullableReadonlySharedReferenceAccess(WritableSharedPayload),
    NullableWritableSharedReferenceAccess(WritableSharedPayload),
    Collection(CollectionTypeId),
    NullableCollection(CollectionTypeId),
    PayloadEnum(PayloadEnumType),
    NullablePayloadEnum(PayloadEnumType),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PayloadEnumType {
    pub id: EnumId,
    pub capabilities: EnumCapabilities,
    pub size: u32,
    pub align: u32,
    pub nullable_size: u32,
    pub nullable_payload_offset: u32,
}

impl PayloadEnumType {
    pub const fn storage_size(self, nullable: bool) -> u32 {
        if nullable {
            self.nullable_size
        } else {
            self.size
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WritableSharedPayload {
    Class(ClassId),
    Collection(CollectionTypeId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedAccessType {
    pub payload: WritableSharedPayload,
    pub writable: bool,
    pub nullable: bool,
}

impl SharedAccessType {
    pub const fn into_type(self) -> Type {
        match (self.nullable, self.writable) {
            (false, false) => Type::ReadonlySharedReferenceAccess(self.payload),
            (false, true) => Type::WritableSharedReferenceAccess(self.payload),
            (true, false) => Type::NullableReadonlySharedReferenceAccess(self.payload),
            (true, true) => Type::NullableWritableSharedReferenceAccess(self.payload),
        }
    }
}

impl Type {
    pub const fn has_move_ownership(self) -> bool {
        matches!(
            self,
            Self::Mixed
                | Self::NullableMixed
                | Self::Class(_)
                | Self::NullableClass(_)
                | Self::SharedReference(_)
                | Self::WeakReference(_)
                | Self::NullableSharedReference(_)
                | Self::NullableWeakReference(_)
                | Self::WritableSharedReference(_)
                | Self::WritableWeakReference(_)
                | Self::NullableWritableSharedReference(_)
                | Self::NullableWritableWeakReference(_)
                | Self::ReadonlySharedReferenceAccess(_)
                | Self::WritableSharedReferenceAccess(_)
                | Self::NullableReadonlySharedReferenceAccess(_)
                | Self::NullableWritableSharedReferenceAccess(_)
                | Self::Collection(_)
                | Self::NullableCollection(_)
                | Self::PayloadEnum(PayloadEnumType {
                    capabilities: EnumCapabilities { copy: false, .. },
                    ..
                })
                | Self::NullablePayloadEnum(PayloadEnumType {
                    capabilities: EnumCapabilities { copy: false, .. },
                    ..
                })
        )
    }

    pub const fn shared_access(self) -> Option<SharedAccessType> {
        match self {
            Self::ReadonlySharedReferenceAccess(payload) => Some(SharedAccessType {
                payload,
                writable: false,
                nullable: false,
            }),
            Self::WritableSharedReferenceAccess(payload) => Some(SharedAccessType {
                payload,
                writable: true,
                nullable: false,
            }),
            Self::NullableReadonlySharedReferenceAccess(payload) => Some(SharedAccessType {
                payload,
                writable: false,
                nullable: true,
            }),
            Self::NullableWritableSharedReferenceAccess(payload) => Some(SharedAccessType {
                payload,
                writable: true,
                nullable: true,
            }),
            _ => None,
        }
    }
}

impl From<ScalarType> for Type {
    fn from(value: ScalarType) -> Self {
        Self::Scalar(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasicBlock {
    pub id: BlockId,
    pub statements: Vec<Statement>,
    pub terminator: Terminator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operand {
    Scalar(ScalarValue),
    Local(LocalId),
    NullablePayload(LocalId),
    Static(StaticId),
    Property {
        object: LocalId,
        property: PropertyId,
    },
    CollectionLength(LocalId),
    CollectionIndex {
        collection: LocalId,
        index: Box<Rvalue>,
        /// True when `index` is a position in the collection rather than a key.
        positional: bool,
        remove: bool,
    },
    CollectionKeyAt {
        collection: LocalId,
        offset: Box<Rvalue>,
    },
    MixedPayload {
        mixed: LocalId,
        tag: MixedTag,
    },
    StringIntrinsic(Box<StringIntrinsicCall>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StringIntrinsicKind {
    GraphemeLength,
    ByteLength,
    IsEmpty,
    ToBytes,
    Trim,
    TrimStart,
    TrimEnd,
    Lower,
    Upper,
    LowerFirst,
    UpperFirst,
    Contains,
    StartsWith,
    EndsWith,
    ContainsIgnoreCase,
    StartsWithIgnoreCase,
    EndsWithIgnoreCase,
    EqualsIgnoreCase,
    IndexOf,
    LastIndexOf,
    IndexOfIgnoreCase,
    LastIndexOfIgnoreCase,
    CountOccurrences,
    Replace,
    Split,
    Join,
    Slice,
    Repeat,
    PadStart,
    PadEnd,
    FromBytes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StringIntrinsicCall {
    pub kind: StringIntrinsicKind,
    pub args: Vec<Rvalue>,
    pub result: Type,
    pub span: Span,
    pub argument_spans: Vec<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rvalue {
    Value(ValueExpression),
    String(StringExpression),
    Mixed(MixedExpression),
    NullableScalar(NullableScalarExpression),
    NullableString(NullableStringExpression),
    NullableMixed(NullableMixedExpression),
    Class(ClassExpression),
    NullableClass(NullableClassExpression),
    SharedReference(SharedReferenceExpression),
    WeakReference(WeakReferenceExpression),
    NullableSharedReference(NullableSharedReferenceExpression),
    NullableWeakReference(NullableWeakReferenceExpression),
    WritableSharedReference(WritableSharedReferenceExpression),
    WritableWeakReference(WritableWeakReferenceExpression),
    NullableWritableSharedReference(NullableWritableSharedReferenceExpression),
    NullableWritableWeakReference(NullableWritableWeakReferenceExpression),
    SharedReferenceAccess(SharedReferenceAccessExpression),
    NullableSharedReferenceAccess(NullableSharedReferenceAccessExpression),
    Collection(CollectionExpression),
    NullableCollection(NullableCollectionExpression),
    PayloadEnum(PayloadEnumExpression),
    NullablePayloadEnum(NullablePayloadEnumExpression),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnedSharedTemporary {
    Strong,
    Weak,
    WritableStrong,
    WritableWeak,
    ReadonlyAccess,
    WritableAccess,
}

impl Rvalue {
    pub const fn ty(&self) -> Type {
        match self {
            Self::Value(value) => Type::Scalar(value.ty()),
            Self::String(_) => Type::String,
            Self::Mixed(_) => Type::Mixed,
            Self::NullableScalar(value) => Type::NullableScalar(value.ty()),
            Self::NullableString(_) => Type::NullableString,
            Self::NullableMixed(_) => Type::NullableMixed,
            Self::Class(value) => Type::Class(value.class()),
            Self::NullableClass(value) => Type::NullableClass(value.class()),
            Self::SharedReference(value) => Type::SharedReference(value.class()),
            Self::WeakReference(value) => Type::WeakReference(value.class()),
            Self::NullableSharedReference(value) => Type::NullableSharedReference(value.class()),
            Self::NullableWeakReference(value) => Type::NullableWeakReference(value.class()),
            Self::WritableSharedReference(value) => Type::WritableSharedReference(value.payload()),
            Self::WritableWeakReference(value) => Type::WritableWeakReference(value.payload()),
            Self::NullableWritableSharedReference(value) => {
                Type::NullableWritableSharedReference(value.payload())
            }
            Self::NullableWritableWeakReference(value) => {
                Type::NullableWritableWeakReference(value.payload())
            }
            Self::SharedReferenceAccess(value) => value.ty(),
            Self::NullableSharedReferenceAccess(value) => value.ty(),
            Self::Collection(value) => Type::Collection(value.collection()),
            Self::NullableCollection(value) => Type::NullableCollection(value.collection()),
            Self::PayloadEnum(value) => Type::PayloadEnum(value.ty()),
            Self::NullablePayloadEnum(value) => Type::NullablePayloadEnum(value.ty()),
        }
    }

    pub const fn owned_temporary_class(&self) -> Option<ClassId> {
        match self {
            Self::Class(value) => value.owned_temporary_class(),
            Self::NullableClass(value) => value.owned_temporary_class(),
            Self::Collection(_) | Self::NullableCollection(_) => None,
            Self::Value(_)
            | Self::String(_)
            | Self::Mixed(_)
            | Self::NullableScalar(_)
            | Self::NullableMixed(_)
            | Self::NullableString(_)
            | Self::SharedReference(_)
            | Self::WeakReference(_)
            | Self::NullableSharedReference(_)
            | Self::NullableWeakReference(_)
            | Self::WritableSharedReference(_)
            | Self::WritableWeakReference(_)
            | Self::NullableWritableSharedReference(_)
            | Self::NullableWritableWeakReference(_)
            | Self::SharedReferenceAccess(_)
            | Self::NullableSharedReferenceAccess(_)
            | Self::PayloadEnum(_)
            | Self::NullablePayloadEnum(_) => None,
        }
    }

    pub const fn owned_temporary_collection(&self) -> Option<CollectionTypeId> {
        match self {
            Self::Collection(value) => value.owned_temporary_collection(),
            Self::NullableCollection(value) => value.owned_temporary_collection(),
            Self::Value(_)
            | Self::String(_)
            | Self::Mixed(_)
            | Self::NullableScalar(_)
            | Self::NullableString(_)
            | Self::NullableMixed(_)
            | Self::Class(_)
            | Self::NullableClass(_)
            | Self::SharedReference(_)
            | Self::WeakReference(_)
            | Self::NullableSharedReference(_)
            | Self::NullableWeakReference(_)
            | Self::WritableSharedReference(_)
            | Self::WritableWeakReference(_)
            | Self::NullableWritableSharedReference(_)
            | Self::NullableWritableWeakReference(_)
            | Self::SharedReferenceAccess(_)
            | Self::NullableSharedReferenceAccess(_)
            | Self::PayloadEnum(_)
            | Self::NullablePayloadEnum(_) => None,
        }
    }

    pub fn owned_temporary_shared(&self) -> Option<OwnedSharedTemporary> {
        match self {
            Self::SharedReference(value) => value.owned_temporary(),
            Self::WeakReference(value) => value.owned_temporary(),
            Self::NullableSharedReference(value) => value.owned_temporary(),
            Self::NullableWeakReference(value) => value.owned_temporary(),
            Self::WritableSharedReference(value) => {
                if value.owned_temporary() {
                    Some(OwnedSharedTemporary::WritableStrong)
                } else {
                    None
                }
            }
            Self::WritableWeakReference(value) => {
                if value.owned_temporary() {
                    Some(OwnedSharedTemporary::WritableWeak)
                } else {
                    None
                }
            }
            Self::NullableWritableSharedReference(value) => value
                .owned_temporary()
                .then_some(OwnedSharedTemporary::WritableStrong),
            Self::NullableWritableWeakReference(value) => value
                .owned_temporary()
                .then_some(OwnedSharedTemporary::WritableWeak),
            Self::SharedReferenceAccess(value) => {
                value.owned_temporary().then_some(if value.writable() {
                    OwnedSharedTemporary::WritableAccess
                } else {
                    OwnedSharedTemporary::ReadonlyAccess
                })
            }
            Self::NullableSharedReferenceAccess(value) => {
                value.owned_temporary().then_some(if value.writable() {
                    OwnedSharedTemporary::WritableAccess
                } else {
                    OwnedSharedTemporary::ReadonlyAccess
                })
            }
            Self::Value(_)
            | Self::String(_)
            | Self::Mixed(_)
            | Self::NullableScalar(_)
            | Self::NullableString(_)
            | Self::NullableMixed(_)
            | Self::Class(_)
            | Self::NullableClass(_)
            | Self::Collection(_)
            | Self::NullableCollection(_)
            | Self::PayloadEnum(_)
            | Self::NullablePayloadEnum(_) => None,
        }
    }

    pub const fn owned_temporary_payload_enum(&self) -> Option<(PayloadEnumType, bool)> {
        match self {
            Self::PayloadEnum(value) if value.owned_temporary() => Some((value.ty(), false)),
            Self::NullablePayloadEnum(value) if value.owned_temporary() => Some((value.ty(), true)),
            _ => None,
        }
    }

    pub const fn borrows_class_value(&self) -> bool {
        match self {
            Self::Class(value) => value.borrows_class_value(),
            Self::NullableClass(value) => value.borrows_class_value(),
            Self::Value(_)
            | Self::String(_)
            | Self::Mixed(_)
            | Self::NullableScalar(_)
            | Self::NullableString(_)
            | Self::NullableMixed(_)
            | Self::Collection(_)
            | Self::NullableCollection(_)
            | Self::SharedReference(_)
            | Self::WeakReference(_)
            | Self::NullableSharedReference(_)
            | Self::NullableWeakReference(_)
            | Self::WritableSharedReference(_)
            | Self::WritableWeakReference(_)
            | Self::NullableWritableSharedReference(_)
            | Self::NullableWritableWeakReference(_)
            | Self::SharedReferenceAccess(_)
            | Self::NullableSharedReferenceAccess(_)
            | Self::PayloadEnum(_)
            | Self::NullablePayloadEnum(_) => false,
        }
    }

    pub const fn transferred_owned_local(&self) -> Option<LocalId> {
        match self {
            Self::Mixed(MixedExpression::Local {
                local,
                transfer: true,
            })
            | Self::NullableMixed(NullableMixedExpression::Local {
                local,
                transfer: true,
            })
            | Self::Class(ClassExpression::Local {
                local,
                transfer: true,
                ..
            })
            | Self::NullableClass(NullableClassExpression::Local {
                local,
                transfer: true,
                ..
            })
            | Self::Collection(CollectionExpression::Local {
                local,
                transfer: true,
                ..
            })
            | Self::NullableCollection(NullableCollectionExpression::Local {
                local,
                transfer: true,
                ..
            }) => Some(*local),
            Self::SharedReference(SharedReferenceExpression::Local {
                local,
                transfer: true,
                ..
            })
            | Self::SharedReference(SharedReferenceExpression::NullableLocalAssumeNonNull {
                local,
                transfer: true,
                ..
            })
            | Self::WeakReference(WeakReferenceExpression::Local {
                local,
                transfer: true,
                ..
            })
            | Self::NullableSharedReference(NullableSharedReferenceExpression::Local {
                local,
                transfer: true,
                ..
            })
            | Self::NullableWeakReference(NullableWeakReferenceExpression::Local {
                local,
                transfer: true,
                ..
            })
            | Self::WritableSharedReference(WritableSharedReferenceExpression::Local {
                local,
                transfer: true,
                ..
            })
            | Self::WritableSharedReference(
                WritableSharedReferenceExpression::NullableLocalAssumeNonNull {
                    local,
                    transfer: true,
                    ..
                },
            )
            | Self::WritableWeakReference(WritableWeakReferenceExpression::Local {
                local,
                transfer: true,
                ..
            })
            | Self::NullableWritableSharedReference(
                NullableWritableSharedReferenceExpression::Local {
                    local,
                    transfer: true,
                    ..
                },
            )
            | Self::NullableWritableWeakReference(
                NullableWritableWeakReferenceExpression::Local {
                    local,
                    transfer: true,
                    ..
                },
            )
            | Self::SharedReferenceAccess(SharedReferenceAccessExpression::Local {
                local,
                transfer: true,
                ..
            })
            | Self::NullableSharedReferenceAccess(
                NullableSharedReferenceAccessExpression::Local {
                    local,
                    transfer: true,
                    ..
                },
            ) => Some(*local),
            Self::PayloadEnum(PayloadEnumExpression::Use {
                place:
                    PayloadEnumPlace::Local(local) | PayloadEnumPlace::NullableLocalAssumeNonNull(local),
                mode: PayloadEnumUseMode::Move,
                ..
            })
            | Self::NullablePayloadEnum(NullablePayloadEnumExpression::Use {
                place:
                    PayloadEnumPlace::Local(local) | PayloadEnumPlace::NullableLocalAssumeNonNull(local),
                mode: PayloadEnumUseMode::Move,
                ..
            }) => Some(*local),
            Self::Value(_)
            | Self::String(_)
            | Self::Mixed(_)
            | Self::NullableScalar(_)
            | Self::NullableString(_)
            | Self::NullableMixed(_)
            | Self::Class(_)
            | Self::NullableClass(_)
            | Self::Collection(_)
            | Self::NullableCollection(_)
            | Self::SharedReference(_)
            | Self::WeakReference(_)
            | Self::NullableSharedReference(_)
            | Self::NullableWeakReference(_)
            | Self::WritableSharedReference(_)
            | Self::WritableWeakReference(_)
            | Self::NullableWritableSharedReference(_)
            | Self::NullableWritableWeakReference(_)
            | Self::SharedReferenceAccess(_)
            | Self::NullableSharedReferenceAccess(_)
            | Self::PayloadEnum(_)
            | Self::NullablePayloadEnum(_) => None,
        }
    }

    pub const fn mixed_ownership(&self) -> MixedOwnership {
        match self {
            Self::Mixed(value) => value.ownership(),
            Self::NullableMixed(value) => value.ownership(),
            Self::Value(_)
            | Self::String(_)
            | Self::NullableScalar(_)
            | Self::NullableString(_)
            | Self::Class(_)
            | Self::NullableClass(_)
            | Self::Collection(_)
            | Self::NullableCollection(_)
            | Self::SharedReference(_)
            | Self::WeakReference(_)
            | Self::NullableSharedReference(_)
            | Self::NullableWeakReference(_)
            | Self::WritableSharedReference(_)
            | Self::WritableWeakReference(_)
            | Self::NullableWritableSharedReference(_)
            | Self::NullableWritableWeakReference(_)
            | Self::SharedReferenceAccess(_)
            | Self::NullableSharedReferenceAccess(_)
            | Self::PayloadEnum(_)
            | Self::NullablePayloadEnum(_) => MixedOwnership::None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadEnumUseMode {
    Borrow,
    Copy,
    Move,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PayloadEnumPlace {
    Local(LocalId),
    NullableLocalAssumeNonNull(LocalId),
    Static(StaticId),
    Property {
        object: LocalId,
        property: PropertyId,
    },
    CollectionIndex {
        collection: LocalId,
        index: Box<Rvalue>,
        positional: bool,
        remove: bool,
    },
    MixedPayload {
        mixed: LocalId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PayloadEnumExpression {
    Construct {
        ty: PayloadEnumType,
        case: EnumCaseId,
        fields: Vec<Rvalue>,
        span: Span,
    },
    Use {
        ty: PayloadEnumType,
        place: PayloadEnumPlace,
        mode: PayloadEnumUseMode,
    },
    Call {
        ty: PayloadEnumType,
        function: FunctionId,
        args: Vec<Rvalue>,
    },
    Coalesce {
        ty: PayloadEnumType,
        left: Box<NullablePayloadEnumExpression>,
        right: Box<PayloadEnumExpression>,
        mode: PayloadEnumUseMode,
    },
}

impl PayloadEnumExpression {
    pub const fn ty(&self) -> PayloadEnumType {
        match self {
            Self::Construct { ty, .. }
            | Self::Use { ty, .. }
            | Self::Call { ty, .. }
            | Self::Coalesce { ty, .. } => *ty,
        }
    }

    pub const fn use_mode(&self) -> Option<PayloadEnumUseMode> {
        match self {
            Self::Use { mode, .. } | Self::Coalesce { mode, .. } => Some(*mode),
            Self::Construct { .. } | Self::Call { .. } => None,
        }
    }

    pub const fn owned_temporary(&self) -> bool {
        match self {
            Self::Construct { .. } | Self::Call { .. } => true,
            Self::Use { mode, .. } | Self::Coalesce { mode, .. } => {
                !matches!(mode, PayloadEnumUseMode::Borrow)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NullablePayloadEnumExpression {
    Null(PayloadEnumType),
    Value(PayloadEnumExpression),
    Use {
        ty: PayloadEnumType,
        place: PayloadEnumPlace,
        mode: PayloadEnumUseMode,
    },
    Call {
        ty: PayloadEnumType,
        function: FunctionId,
        args: Vec<Rvalue>,
    },
    CollectionGet {
        ty: PayloadEnumType,
        collection: LocalId,
        key: Box<Rvalue>,
        access: NullableCollectionAccess,
        stored_nullable: bool,
        mode: PayloadEnumUseMode,
    },
    Coalesce {
        ty: PayloadEnumType,
        left: Box<NullablePayloadEnumExpression>,
        right: Box<NullablePayloadEnumExpression>,
        mode: PayloadEnumUseMode,
    },
}

impl NullablePayloadEnumExpression {
    pub const fn ty(&self) -> PayloadEnumType {
        match self {
            Self::Null(ty)
            | Self::Use { ty, .. }
            | Self::Call { ty, .. }
            | Self::CollectionGet { ty, .. }
            | Self::Coalesce { ty, .. } => *ty,
            Self::Value(value) => value.ty(),
        }
    }

    pub const fn owned_temporary(&self) -> bool {
        match self {
            Self::Null(_) => false,
            Self::Value(value) => value.owned_temporary(),
            Self::Call { .. } => true,
            Self::Use { mode, .. }
            | Self::CollectionGet { mode, .. }
            | Self::Coalesce { mode, .. } => !matches!(mode, PayloadEnumUseMode::Borrow),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SharedReferenceExpression {
    New {
        class: ClassId,
        value: Box<ClassExpression>,
    },
    Local {
        class: ClassId,
        local: LocalId,
        transfer: bool,
    },
    NullableLocalAssumeNonNull {
        class: ClassId,
        local: LocalId,
        transfer: bool,
    },
    Property {
        class: ClassId,
        object: LocalId,
        property: PropertyId,
    },
    Call {
        class: ClassId,
        function: FunctionId,
        args: Vec<Rvalue>,
        return_borrow: Option<ReturnBorrow>,
    },
    Share {
        class: ClassId,
        value: Box<SharedReferenceExpression>,
    },
    Coalesce {
        class: ClassId,
        left: Box<NullableSharedReferenceExpression>,
        right: Box<SharedReferenceExpression>,
        transfer: bool,
    },
    CollectionIndex {
        class: ClassId,
        collection: LocalId,
        index: Box<Rvalue>,
        /// True when `index` is a position in the collection rather than a key.
        positional: bool,
        remove: bool,
    },
}

impl SharedReferenceExpression {
    pub const fn class(&self) -> ClassId {
        match self {
            Self::New { class, .. }
            | Self::Local { class, .. }
            | Self::NullableLocalAssumeNonNull { class, .. }
            | Self::Property { class, .. }
            | Self::Call { class, .. }
            | Self::Share { class, .. }
            | Self::Coalesce { class, .. }
            | Self::CollectionIndex { class, .. } => *class,
        }
    }

    pub const fn owned_temporary(&self) -> Option<OwnedSharedTemporary> {
        match self {
            Self::New { .. } | Self::Share { .. } => Some(OwnedSharedTemporary::Strong),
            Self::Coalesce { transfer, .. } => {
                if *transfer {
                    Some(OwnedSharedTemporary::Strong)
                } else {
                    None
                }
            }
            Self::Local { transfer, .. } | Self::NullableLocalAssumeNonNull { transfer, .. } => {
                if *transfer {
                    Some(OwnedSharedTemporary::Strong)
                } else {
                    None
                }
            }
            Self::Call { return_borrow, .. } => {
                if return_borrow.is_none() {
                    Some(OwnedSharedTemporary::Strong)
                } else {
                    None
                }
            }
            Self::CollectionIndex { remove, .. } => {
                if *remove {
                    Some(OwnedSharedTemporary::Strong)
                } else {
                    None
                }
            }
            Self::Property { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WeakReferenceExpression {
    Local {
        class: ClassId,
        local: LocalId,
        transfer: bool,
    },
    NullableLocalAssumeNonNull {
        class: ClassId,
        local: LocalId,
        transfer: bool,
    },
    Property {
        class: ClassId,
        object: LocalId,
        property: PropertyId,
    },
    Call {
        class: ClassId,
        function: FunctionId,
        args: Vec<Rvalue>,
        return_borrow: Option<ReturnBorrow>,
    },
    Create {
        class: ClassId,
        value: Box<SharedReferenceExpression>,
    },
    Coalesce {
        class: ClassId,
        left: Box<NullableWeakReferenceExpression>,
        right: Box<WeakReferenceExpression>,
        transfer: bool,
    },
    CollectionIndex {
        class: ClassId,
        collection: LocalId,
        index: Box<Rvalue>,
        /// True when `index` is a position in the collection rather than a key.
        positional: bool,
        remove: bool,
    },
}

impl WeakReferenceExpression {
    pub const fn class(&self) -> ClassId {
        match self {
            Self::Local { class, .. }
            | Self::NullableLocalAssumeNonNull { class, .. }
            | Self::Property { class, .. }
            | Self::Call { class, .. }
            | Self::Create { class, .. }
            | Self::Coalesce { class, .. }
            | Self::CollectionIndex { class, .. } => *class,
        }
    }

    pub const fn owned_temporary(&self) -> Option<OwnedSharedTemporary> {
        match self {
            Self::Create { .. } => Some(OwnedSharedTemporary::Weak),
            Self::Coalesce { transfer, .. } => {
                if *transfer {
                    Some(OwnedSharedTemporary::Weak)
                } else {
                    None
                }
            }
            Self::Local { transfer, .. } | Self::NullableLocalAssumeNonNull { transfer, .. } => {
                if *transfer {
                    Some(OwnedSharedTemporary::Weak)
                } else {
                    None
                }
            }
            Self::Call { return_borrow, .. } => {
                if return_borrow.is_none() {
                    Some(OwnedSharedTemporary::Weak)
                } else {
                    None
                }
            }
            Self::CollectionIndex { remove, .. } => {
                if *remove {
                    Some(OwnedSharedTemporary::Weak)
                } else {
                    None
                }
            }
            Self::Property { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NullableSharedReferenceExpression {
    Null(ClassId),
    Shared(SharedReferenceExpression),
    Local {
        class: ClassId,
        local: LocalId,
        transfer: bool,
    },
    Property {
        class: ClassId,
        object: LocalId,
        property: PropertyId,
    },
    Call {
        class: ClassId,
        function: FunctionId,
        args: Vec<Rvalue>,
        return_borrow: Option<ReturnBorrow>,
    },
    Acquire {
        class: ClassId,
        value: Box<WeakReferenceExpression>,
    },
    NullSafeShare {
        class: ClassId,
        value: Box<NullableSharedReferenceExpression>,
    },
    NullSafeAcquire {
        class: ClassId,
        value: Box<NullableWeakReferenceExpression>,
    },
    Coalesce {
        class: ClassId,
        left: Box<NullableSharedReferenceExpression>,
        right: Box<NullableSharedReferenceExpression>,
        transfer: bool,
    },
    DictionaryGet {
        class: ClassId,
        collection: LocalId,
        key: Box<Rvalue>,
        access: NullableCollectionAccess,
        stored_nullable: bool,
    },
    CollectionIndex {
        class: ClassId,
        collection: LocalId,
        index: Box<Rvalue>,
        /// True when `index` is a position in the collection rather than a key.
        positional: bool,
        remove: bool,
    },
}

impl NullableSharedReferenceExpression {
    pub const fn class(&self) -> ClassId {
        match self {
            Self::Null(class)
            | Self::Local { class, .. }
            | Self::Property { class, .. }
            | Self::Call { class, .. }
            | Self::Acquire { class, .. }
            | Self::NullSafeShare { class, .. }
            | Self::NullSafeAcquire { class, .. }
            | Self::Coalesce { class, .. }
            | Self::DictionaryGet { class, .. }
            | Self::CollectionIndex { class, .. } => *class,
            Self::Shared(value) => value.class(),
        }
    }

    pub const fn owned_temporary(&self) -> Option<OwnedSharedTemporary> {
        match self {
            Self::Shared(value) => value.owned_temporary(),
            Self::Local { transfer, .. } => {
                if *transfer {
                    Some(OwnedSharedTemporary::Strong)
                } else {
                    None
                }
            }
            Self::Call { return_borrow, .. } => {
                if return_borrow.is_none() {
                    Some(OwnedSharedTemporary::Strong)
                } else {
                    None
                }
            }
            Self::Acquire { .. } => Some(OwnedSharedTemporary::Strong),
            Self::NullSafeShare { .. } | Self::NullSafeAcquire { .. } => {
                Some(OwnedSharedTemporary::Strong)
            }
            Self::Coalesce { transfer, .. } => {
                if *transfer {
                    Some(OwnedSharedTemporary::Strong)
                } else {
                    None
                }
            }
            Self::DictionaryGet { access, .. } => {
                if matches!(
                    access,
                    NullableCollectionAccess::Remove
                        | NullableCollectionAccess::Pop
                        | NullableCollectionAccess::PopFront
                        | NullableCollectionAccess::PopBack
                ) {
                    Some(OwnedSharedTemporary::Strong)
                } else {
                    None
                }
            }
            Self::CollectionIndex { remove, .. } => {
                if *remove {
                    Some(OwnedSharedTemporary::Strong)
                } else {
                    None
                }
            }
            Self::Null(_) | Self::Property { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NullableWeakReferenceExpression {
    Null(ClassId),
    Weak(WeakReferenceExpression),
    Local {
        class: ClassId,
        local: LocalId,
        transfer: bool,
    },
    Property {
        class: ClassId,
        object: LocalId,
        property: PropertyId,
    },
    Call {
        class: ClassId,
        function: FunctionId,
        args: Vec<Rvalue>,
        return_borrow: Option<ReturnBorrow>,
    },
    NullSafeCreate {
        class: ClassId,
        value: Box<NullableSharedReferenceExpression>,
    },
    Coalesce {
        class: ClassId,
        left: Box<NullableWeakReferenceExpression>,
        right: Box<NullableWeakReferenceExpression>,
        transfer: bool,
    },
    DictionaryGet {
        class: ClassId,
        collection: LocalId,
        key: Box<Rvalue>,
        access: NullableCollectionAccess,
        stored_nullable: bool,
    },
    CollectionIndex {
        class: ClassId,
        collection: LocalId,
        index: Box<Rvalue>,
        /// True when `index` is a position in the collection rather than a key.
        positional: bool,
        remove: bool,
    },
}

impl NullableWeakReferenceExpression {
    pub const fn class(&self) -> ClassId {
        match self {
            Self::Null(class)
            | Self::Local { class, .. }
            | Self::Property { class, .. }
            | Self::Call { class, .. }
            | Self::NullSafeCreate { class, .. }
            | Self::Coalesce { class, .. }
            | Self::DictionaryGet { class, .. }
            | Self::CollectionIndex { class, .. } => *class,
            Self::Weak(value) => value.class(),
        }
    }

    pub const fn owned_temporary(&self) -> Option<OwnedSharedTemporary> {
        match self {
            Self::Weak(value) => value.owned_temporary(),
            Self::Local { transfer, .. } => {
                if *transfer {
                    Some(OwnedSharedTemporary::Weak)
                } else {
                    None
                }
            }
            Self::Call { return_borrow, .. } => {
                if return_borrow.is_none() {
                    Some(OwnedSharedTemporary::Weak)
                } else {
                    None
                }
            }
            Self::NullSafeCreate { .. } => Some(OwnedSharedTemporary::Weak),
            Self::Coalesce { transfer, .. } => {
                if *transfer {
                    Some(OwnedSharedTemporary::Weak)
                } else {
                    None
                }
            }
            Self::DictionaryGet { access, .. } => {
                if matches!(
                    access,
                    NullableCollectionAccess::Remove
                        | NullableCollectionAccess::Pop
                        | NullableCollectionAccess::PopFront
                        | NullableCollectionAccess::PopBack
                ) {
                    Some(OwnedSharedTemporary::Weak)
                } else {
                    None
                }
            }
            Self::CollectionIndex { remove, .. } => {
                if *remove {
                    Some(OwnedSharedTemporary::Weak)
                } else {
                    None
                }
            }
            Self::Null(_) | Self::Property { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WritableSharedReferenceExpression {
    New {
        payload: WritableSharedPayload,
        value: Box<Rvalue>,
    },
    Local {
        payload: WritableSharedPayload,
        local: LocalId,
        transfer: bool,
    },
    NullableLocalAssumeNonNull {
        payload: WritableSharedPayload,
        local: LocalId,
        transfer: bool,
    },
    Property {
        payload: WritableSharedPayload,
        object: LocalId,
        property: PropertyId,
    },
    Call {
        payload: WritableSharedPayload,
        function: FunctionId,
        args: Vec<Rvalue>,
        return_borrow: Option<ReturnBorrow>,
    },
    Share {
        payload: WritableSharedPayload,
        value: Box<WritableSharedReferenceExpression>,
    },
    Coalesce {
        payload: WritableSharedPayload,
        left: Box<NullableWritableSharedReferenceExpression>,
        right: Box<WritableSharedReferenceExpression>,
        transfer: bool,
    },
    CollectionIndex {
        payload: WritableSharedPayload,
        collection: LocalId,
        index: Box<Rvalue>,
        /// True when `index` is a position in the collection rather than a key.
        positional: bool,
        remove: bool,
    },
}

impl WritableSharedReferenceExpression {
    pub const fn payload(&self) -> WritableSharedPayload {
        match self {
            Self::New { payload, .. }
            | Self::Local { payload, .. }
            | Self::NullableLocalAssumeNonNull { payload, .. }
            | Self::Property { payload, .. }
            | Self::Call { payload, .. }
            | Self::Share { payload, .. }
            | Self::Coalesce { payload, .. }
            | Self::CollectionIndex { payload, .. } => *payload,
        }
    }

    pub const fn owned_temporary(&self) -> bool {
        match self {
            Self::New { .. } | Self::Share { .. } => true,
            Self::Local { transfer, .. }
            | Self::NullableLocalAssumeNonNull { transfer, .. }
            | Self::Coalesce { transfer, .. } => *transfer,
            Self::Call { return_borrow, .. } => return_borrow.is_none(),
            Self::CollectionIndex { remove, .. } => *remove,
            Self::Property { .. } => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WritableWeakReferenceExpression {
    Local {
        payload: WritableSharedPayload,
        local: LocalId,
        transfer: bool,
    },
    NullableLocalAssumeNonNull {
        payload: WritableSharedPayload,
        local: LocalId,
        transfer: bool,
    },
    Property {
        payload: WritableSharedPayload,
        object: LocalId,
        property: PropertyId,
    },
    Call {
        payload: WritableSharedPayload,
        function: FunctionId,
        args: Vec<Rvalue>,
        return_borrow: Option<ReturnBorrow>,
    },
    Create {
        payload: WritableSharedPayload,
        value: Box<WritableSharedReferenceExpression>,
    },
    Coalesce {
        payload: WritableSharedPayload,
        left: Box<NullableWritableWeakReferenceExpression>,
        right: Box<WritableWeakReferenceExpression>,
        transfer: bool,
    },
    CollectionIndex {
        payload: WritableSharedPayload,
        collection: LocalId,
        index: Box<Rvalue>,
        /// True when `index` is a position in the collection rather than a key.
        positional: bool,
        remove: bool,
    },
}

impl WritableWeakReferenceExpression {
    pub const fn payload(&self) -> WritableSharedPayload {
        match self {
            Self::Local { payload, .. }
            | Self::NullableLocalAssumeNonNull { payload, .. }
            | Self::Property { payload, .. }
            | Self::Call { payload, .. }
            | Self::Create { payload, .. }
            | Self::Coalesce { payload, .. }
            | Self::CollectionIndex { payload, .. } => *payload,
        }
    }

    pub const fn owned_temporary(&self) -> bool {
        match self {
            Self::Create { .. } => true,
            Self::Local { transfer, .. }
            | Self::NullableLocalAssumeNonNull { transfer, .. }
            | Self::Coalesce { transfer, .. } => *transfer,
            Self::Call { return_borrow, .. } => return_borrow.is_none(),
            Self::CollectionIndex { remove, .. } => *remove,
            Self::Property { .. } => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NullableWritableSharedReferenceExpression {
    Null(WritableSharedPayload),
    Strong(WritableSharedReferenceExpression),
    Local {
        payload: WritableSharedPayload,
        local: LocalId,
        transfer: bool,
    },
    Property {
        payload: WritableSharedPayload,
        object: LocalId,
        property: PropertyId,
    },
    Call {
        payload: WritableSharedPayload,
        function: FunctionId,
        args: Vec<Rvalue>,
        return_borrow: Option<ReturnBorrow>,
    },
    Acquire {
        payload: WritableSharedPayload,
        value: Box<WritableWeakReferenceExpression>,
    },
    NullSafeShare {
        payload: WritableSharedPayload,
        value: Box<NullableWritableSharedReferenceExpression>,
    },
    NullSafeAcquire {
        payload: WritableSharedPayload,
        value: Box<NullableWritableWeakReferenceExpression>,
    },
    Coalesce {
        payload: WritableSharedPayload,
        left: Box<NullableWritableSharedReferenceExpression>,
        right: Box<NullableWritableSharedReferenceExpression>,
        transfer: bool,
    },
    DictionaryGet {
        payload: WritableSharedPayload,
        collection: LocalId,
        key: Box<Rvalue>,
        access: NullableCollectionAccess,
        stored_nullable: bool,
    },
}

impl NullableWritableSharedReferenceExpression {
    pub const fn payload(&self) -> WritableSharedPayload {
        match self {
            Self::Null(payload)
            | Self::Local { payload, .. }
            | Self::Property { payload, .. }
            | Self::Call { payload, .. }
            | Self::Acquire { payload, .. }
            | Self::NullSafeShare { payload, .. }
            | Self::NullSafeAcquire { payload, .. }
            | Self::Coalesce { payload, .. }
            | Self::DictionaryGet { payload, .. } => *payload,
            Self::Strong(value) => value.payload(),
        }
    }

    pub const fn owned_temporary(&self) -> bool {
        match self {
            Self::Strong(value) => value.owned_temporary(),
            Self::Local { transfer, .. } | Self::Coalesce { transfer, .. } => *transfer,
            Self::Call { return_borrow, .. } => return_borrow.is_none(),
            Self::Acquire { .. } | Self::NullSafeShare { .. } | Self::NullSafeAcquire { .. } => {
                true
            }
            Self::DictionaryGet { access, .. } => matches!(
                access,
                NullableCollectionAccess::Remove
                    | NullableCollectionAccess::Pop
                    | NullableCollectionAccess::PopFront
                    | NullableCollectionAccess::PopBack
            ),
            Self::Null(_) | Self::Property { .. } => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NullableWritableWeakReferenceExpression {
    Null(WritableSharedPayload),
    Weak(WritableWeakReferenceExpression),
    Local {
        payload: WritableSharedPayload,
        local: LocalId,
        transfer: bool,
    },
    Property {
        payload: WritableSharedPayload,
        object: LocalId,
        property: PropertyId,
    },
    Call {
        payload: WritableSharedPayload,
        function: FunctionId,
        args: Vec<Rvalue>,
        return_borrow: Option<ReturnBorrow>,
    },
    NullSafeCreate {
        payload: WritableSharedPayload,
        value: Box<NullableWritableSharedReferenceExpression>,
    },
    Coalesce {
        payload: WritableSharedPayload,
        left: Box<NullableWritableWeakReferenceExpression>,
        right: Box<NullableWritableWeakReferenceExpression>,
        transfer: bool,
    },
    DictionaryGet {
        payload: WritableSharedPayload,
        collection: LocalId,
        key: Box<Rvalue>,
        access: NullableCollectionAccess,
        stored_nullable: bool,
    },
}

impl NullableWritableWeakReferenceExpression {
    pub const fn payload(&self) -> WritableSharedPayload {
        match self {
            Self::Null(payload)
            | Self::Local { payload, .. }
            | Self::Property { payload, .. }
            | Self::Call { payload, .. }
            | Self::NullSafeCreate { payload, .. }
            | Self::Coalesce { payload, .. }
            | Self::DictionaryGet { payload, .. } => *payload,
            Self::Weak(value) => value.payload(),
        }
    }

    pub const fn owned_temporary(&self) -> bool {
        match self {
            Self::Weak(value) => value.owned_temporary(),
            Self::Local { transfer, .. } | Self::Coalesce { transfer, .. } => *transfer,
            Self::Call { return_borrow, .. } => return_borrow.is_none(),
            Self::NullSafeCreate { .. } => true,
            Self::DictionaryGet { access, .. } => matches!(
                access,
                NullableCollectionAccess::Remove
                    | NullableCollectionAccess::Pop
                    | NullableCollectionAccess::PopFront
                    | NullableCollectionAccess::PopBack
            ),
            Self::Null(_) | Self::Property { .. } => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SharedReferenceAccessExpression {
    Local {
        payload: WritableSharedPayload,
        local: LocalId,
        writable: bool,
        transfer: bool,
    },
    NullableLocalAssumeNonNull {
        payload: WritableSharedPayload,
        local: LocalId,
        writable: bool,
        transfer: bool,
    },
    Property {
        payload: WritableSharedPayload,
        object: LocalId,
        property: PropertyId,
        writable: bool,
    },
    CollectionIndex {
        payload: WritableSharedPayload,
        collection: LocalId,
        index: Box<Rvalue>,
        /// True when `index` is a position in the collection rather than a key.
        positional: bool,
        writable: bool,
        remove: bool,
    },
    Call {
        payload: WritableSharedPayload,
        function: FunctionId,
        args: Vec<Rvalue>,
        return_borrow: Option<ReturnBorrow>,
        writable: bool,
    },
    Acquire {
        payload: WritableSharedPayload,
        value: Box<WritableSharedReferenceExpression>,
        writable: bool,
        span: Span,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NullableSharedReferenceAccessExpression {
    Null {
        payload: WritableSharedPayload,
        writable: bool,
    },
    Access(Box<SharedReferenceAccessExpression>),
    Local {
        payload: WritableSharedPayload,
        local: LocalId,
        writable: bool,
        transfer: bool,
    },
    Property {
        payload: WritableSharedPayload,
        object: LocalId,
        property: PropertyId,
        writable: bool,
    },
    CollectionIndex {
        payload: WritableSharedPayload,
        collection: LocalId,
        index: Box<Rvalue>,
        /// True when `index` is a position in the collection rather than a key.
        positional: bool,
        writable: bool,
        remove: bool,
    },
    CollectionGet {
        collection: LocalId,
        key: Box<Rvalue>,
        access: NullableCollectionAccess,
        stored: SharedAccessType,
    },
    Call {
        payload: WritableSharedPayload,
        function: FunctionId,
        args: Vec<Rvalue>,
        return_borrow: Option<ReturnBorrow>,
        writable: bool,
    },
    NullSafeAcquire {
        payload: WritableSharedPayload,
        value: Box<NullableWritableSharedReferenceExpression>,
        writable: bool,
        span: Span,
    },
}

impl NullableSharedReferenceAccessExpression {
    pub const fn payload(&self) -> WritableSharedPayload {
        match self {
            Self::Null { payload, .. }
            | Self::Local { payload, .. }
            | Self::Property { payload, .. }
            | Self::CollectionIndex { payload, .. }
            | Self::Call { payload, .. }
            | Self::NullSafeAcquire { payload, .. } => *payload,
            Self::CollectionGet { stored, .. } => stored.payload,
            Self::Access(value) => value.payload(),
        }
    }

    pub const fn writable(&self) -> bool {
        match self {
            Self::Null { writable, .. }
            | Self::Local { writable, .. }
            | Self::Property { writable, .. }
            | Self::CollectionIndex { writable, .. }
            | Self::Call { writable, .. }
            | Self::NullSafeAcquire { writable, .. } => *writable,
            Self::CollectionGet { stored, .. } => stored.writable,
            Self::Access(value) => value.writable(),
        }
    }

    pub const fn ty(&self) -> Type {
        if self.writable() {
            Type::NullableWritableSharedReferenceAccess(self.payload())
        } else {
            Type::NullableReadonlySharedReferenceAccess(self.payload())
        }
    }

    pub const fn owned_temporary(&self) -> bool {
        match self {
            Self::Access(value) => value.owned_temporary(),
            Self::Local { transfer, .. } => *transfer,
            Self::Call { return_borrow, .. } => return_borrow.is_none(),
            Self::NullSafeAcquire { .. } => true,
            Self::CollectionIndex { remove, .. } => *remove,
            Self::CollectionGet { access, .. } => matches!(
                access,
                NullableCollectionAccess::Remove
                    | NullableCollectionAccess::Pop
                    | NullableCollectionAccess::PopFront
                    | NullableCollectionAccess::PopBack
            ),
            Self::Null { .. } | Self::Property { .. } => false,
        }
    }
}

impl SharedReferenceAccessExpression {
    pub const fn payload(&self) -> WritableSharedPayload {
        match self {
            Self::Local { payload, .. }
            | Self::NullableLocalAssumeNonNull { payload, .. }
            | Self::Property { payload, .. }
            | Self::CollectionIndex { payload, .. }
            | Self::Call { payload, .. }
            | Self::Acquire { payload, .. } => *payload,
        }
    }

    pub const fn writable(&self) -> bool {
        match self {
            Self::Local { writable, .. }
            | Self::NullableLocalAssumeNonNull { writable, .. }
            | Self::Property { writable, .. }
            | Self::CollectionIndex { writable, .. }
            | Self::Call { writable, .. }
            | Self::Acquire { writable, .. } => *writable,
        }
    }

    pub const fn ty(&self) -> Type {
        if self.writable() {
            Type::WritableSharedReferenceAccess(self.payload())
        } else {
            Type::ReadonlySharedReferenceAccess(self.payload())
        }
    }

    pub const fn owned_temporary(&self) -> bool {
        match self {
            Self::Acquire { .. } => true,
            Self::Local { transfer, .. } | Self::NullableLocalAssumeNonNull { transfer, .. } => {
                *transfer
            }
            Self::Call { return_borrow, .. } => return_borrow.is_none(),
            Self::CollectionIndex { remove, .. } => *remove,
            Self::Property { .. } => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollectionExpression {
    Local {
        collection: CollectionTypeId,
        local: LocalId,
        transfer: bool,
    },
    Literal {
        collection: CollectionTypeId,
        entries: Vec<CollectionEntry>,
    },
    Fill {
        collection: CollectionTypeId,
        value: Box<Rvalue>,
        count: Box<IntegerExpression>,
        count_span: Span,
    },
    Index {
        collection: CollectionTypeId,
        source: LocalId,
        index: Box<Rvalue>,
        /// True when `index` is a position in the collection rather than a key.
        positional: bool,
        index_span: Span,
        transfer: bool,
    },
    Property {
        collection: CollectionTypeId,
        object: LocalId,
        property: PropertyId,
    },
    SharedAccessPayload {
        collection: CollectionTypeId,
        access: LocalId,
        writable: bool,
    },
    From {
        collection: CollectionTypeId,
        source: LocalId,
        transfer: bool,
        algebra: Option<(SetAlgebraOp, LocalId)>,
    },
    FromBytes {
        collection: CollectionTypeId,
        source: LocalId,
    },
    BytesFromArray {
        collection: CollectionTypeId,
        source: LocalId,
    },
    ReadFileBytes {
        collection: CollectionTypeId,
        path: Box<StringExpression>,
        path_span: Span,
    },
    ReadStdinBytes {
        collection: CollectionTypeId,
    },
    StringIntrinsic(Box<StringIntrinsicCall>),
    Call {
        collection: CollectionTypeId,
        function: FunctionId,
        args: Vec<Rvalue>,
        return_borrow: Option<ReturnBorrow>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NullableCollectionExpression {
    Null(CollectionTypeId),
    Collection(CollectionExpression),
    Local {
        collection: CollectionTypeId,
        local: LocalId,
        transfer: bool,
    },
    Property {
        collection: CollectionTypeId,
        object: LocalId,
        property: PropertyId,
    },
    Call {
        collection: CollectionTypeId,
        function: FunctionId,
        args: Vec<Rvalue>,
        return_borrow: Option<ReturnBorrow>,
    },
    Coalesce {
        collection: CollectionTypeId,
        left: Box<NullableCollectionExpression>,
        right: Box<NullableCollectionExpression>,
        transfer: bool,
    },
}

impl NullableCollectionExpression {
    pub const fn collection(&self) -> CollectionTypeId {
        match self {
            Self::Null(collection)
            | Self::Local { collection, .. }
            | Self::Property { collection, .. }
            | Self::Call { collection, .. }
            | Self::Coalesce { collection, .. } => *collection,
            Self::Collection(value) => value.collection(),
        }
    }

    pub const fn owned_temporary_collection(&self) -> Option<CollectionTypeId> {
        match self {
            Self::Collection(value) => value.owned_temporary_collection(),
            Self::Local {
                collection,
                transfer: true,
                ..
            }
            | Self::Call {
                collection,
                return_borrow: None,
                ..
            }
            | Self::Coalesce {
                collection,
                transfer: true,
                ..
            } => Some(*collection),
            Self::Null(_)
            | Self::Local { .. }
            | Self::Property { .. }
            | Self::Call { .. }
            | Self::Coalesce { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetAlgebraOp {
    Union,
    Intersect,
    Difference,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NullableCollectionAccess {
    Get,
    Index,
    Remove,
    First,
    Last,
    Pop,
    PopFront,
    PopBack,
    At,
}

impl CollectionExpression {
    pub const fn collection(&self) -> CollectionTypeId {
        match self {
            Self::Local { collection, .. }
            | Self::Literal { collection, .. }
            | Self::Fill { collection, .. }
            | Self::Index { collection, .. }
            | Self::Property { collection, .. }
            | Self::SharedAccessPayload { collection, .. }
            | Self::From { collection, .. }
            | Self::FromBytes { collection, .. }
            | Self::BytesFromArray { collection, .. }
            | Self::ReadFileBytes { collection, .. }
            | Self::ReadStdinBytes { collection }
            | Self::Call { collection, .. } => *collection,
            Self::StringIntrinsic(call) => match call.result {
                Type::Collection(collection) => collection,
                _ => panic!("validated String intrinsic collection result"),
            },
        }
    }

    pub const fn owned_temporary_collection(&self) -> Option<CollectionTypeId> {
        match self {
            Self::Local {
                collection,
                transfer: true,
                ..
            }
            | Self::Literal { collection, .. }
            | Self::Fill { collection, .. }
            | Self::Index {
                collection,
                transfer: true,
                ..
            }
            | Self::From { collection, .. }
            | Self::FromBytes { collection, .. }
            | Self::BytesFromArray { collection, .. }
            | Self::ReadFileBytes { collection, .. }
            | Self::ReadStdinBytes { collection }
            | Self::Call {
                collection,
                return_borrow: None,
                ..
            } => Some(*collection),
            Self::StringIntrinsic(call) => match call.result {
                Type::Collection(collection) => Some(collection),
                _ => None,
            },
            Self::Local {
                transfer: false, ..
            }
            | Self::Index {
                transfer: false, ..
            }
            | Self::Property { .. }
            | Self::SharedAccessPayload { .. }
            | Self::Call {
                return_borrow: Some(_),
                ..
            } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionEntry {
    pub key: Option<Rvalue>,
    pub value: Rvalue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MixedTag {
    Bool,
    Integer(IntegerType),
    Float(FloatType),
    String,
    Class(ClassId),
    Enum(EnumId),
    PayloadEnum(PayloadEnumType),
}

impl MixedTag {
    pub const fn ty(self) -> Type {
        match self {
            Self::Bool => Type::Scalar(ScalarType::Bool),
            Self::Integer(ty) => Type::Scalar(ScalarType::Integer(ty)),
            Self::Float(ty) => Type::Scalar(ScalarType::Float(ty)),
            Self::String => Type::String,
            Self::Class(class) => Type::Class(class),
            Self::Enum(enum_id) => Type::Scalar(ScalarType::Enum(enum_id)),
            Self::PayloadEnum(ty) => Type::PayloadEnum(ty),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MixedOwnership {
    None,
    ShellOnly,
    Owned,
}

impl MixedOwnership {
    pub const fn has_shell(self) -> bool {
        !matches!(self, Self::None)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MixedExpression {
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
        args: Vec<Rvalue>,
        return_borrow: Option<ReturnBorrow>,
    },
    BoxValue(ValueExpression),
    BoxString {
        value: StringExpression,
        payload_owned: bool,
    },
    BoxClass {
        value: ClassExpression,
        payload_owned: bool,
    },
    BoxPayloadEnum {
        value: Box<PayloadEnumExpression>,
    },
    CollectionIndex {
        collection: LocalId,
        index: Box<Rvalue>,
        /// True when `index` is a position in the collection rather than a key.
        positional: bool,
        transfer: bool,
        remove: bool,
    },
}

impl MixedExpression {
    pub const fn ownership(&self) -> MixedOwnership {
        match self {
            Self::Local { transfer: true, .. } | Self::CollectionIndex { transfer: true, .. } => {
                MixedOwnership::Owned
            }
            Self::Call {
                return_borrow: None,
                ..
            } => MixedOwnership::Owned,
            Self::BoxValue(_) => MixedOwnership::ShellOnly,
            Self::BoxPayloadEnum { .. } => MixedOwnership::Owned,
            Self::BoxString { payload_owned, .. } | Self::BoxClass { payload_owned, .. } => {
                if *payload_owned {
                    MixedOwnership::Owned
                } else {
                    MixedOwnership::ShellOnly
                }
            }
            Self::Local {
                transfer: false, ..
            }
            | Self::Property { .. }
            | Self::Call {
                return_borrow: Some(_),
                ..
            }
            | Self::CollectionIndex {
                transfer: false, ..
            } => MixedOwnership::None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NullableMixedExpression {
    Null,
    Mixed(MixedExpression),
    BoxNullablePayloadEnum(Box<NullablePayloadEnumExpression>),
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
        args: Vec<Rvalue>,
        return_borrow: Option<ReturnBorrow>,
    },
    Coalesce {
        left: Box<NullableMixedExpression>,
        right: Box<NullableMixedExpression>,
        transfer: bool,
    },
}

impl NullableMixedExpression {
    pub const fn ownership(&self) -> MixedOwnership {
        match self {
            Self::Mixed(value) => value.ownership(),
            Self::BoxNullablePayloadEnum(_) => MixedOwnership::Owned,
            Self::Local { transfer: true, .. } | Self::Coalesce { .. } => MixedOwnership::Owned,
            Self::Call {
                return_borrow: None,
                ..
            } => MixedOwnership::Owned,
            Self::Null
            | Self::Local {
                transfer: false, ..
            }
            | Self::Property { .. }
            | Self::Call {
                return_borrow: Some(_),
                ..
            } => MixedOwnership::None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NullableScalarExpression {
    Null(ScalarType),
    Value(ValueExpression),
    Local {
        ty: ScalarType,
        local: LocalId,
    },
    Property {
        ty: ScalarType,
        object: LocalId,
        property: PropertyId,
    },
    Static {
        ty: ScalarType,
        id: StaticId,
    },
    Call {
        ty: ScalarType,
        function: FunctionId,
        args: Vec<Rvalue>,
    },
    NullSafeProperty {
        ty: ScalarType,
        object: Box<NullableClassExpression>,
        property: PropertyId,
    },
    NullSafeCall {
        ty: ScalarType,
        object: Box<NullableClassExpression>,
        function: FunctionId,
        args: Vec<Rvalue>,
    },
    EnumBacking {
        enum_id: EnumId,
        value: Box<NullableScalarExpression>,
    },
    Coalesce {
        ty: ScalarType,
        left: Box<NullableScalarExpression>,
        right: Box<NullableScalarExpression>,
    },
    DictionaryGet {
        ty: ScalarType,
        collection: LocalId,
        key: Box<Rvalue>,
        access: NullableCollectionAccess,
    },
    CollectionIndexOf {
        collection: LocalId,
        value: Box<Rvalue>,
    },
    /// `Int::parse(string)` / `Float::parse(string)`: parses the string and yields
    /// `?ty`, producing the absent value when the text is not a valid number.
    Parse {
        ty: ScalarType,
        value: Box<StringExpression>,
    },
    StringIntrinsic(Box<StringIntrinsicCall>),
}

impl NullableScalarExpression {
    pub const fn ty(&self) -> ScalarType {
        match self {
            Self::Null(ty)
            | Self::Local { ty, .. }
            | Self::Property { ty, .. }
            | Self::Static { ty, .. }
            | Self::Call { ty, .. }
            | Self::NullSafeProperty { ty, .. }
            | Self::NullSafeCall { ty, .. }
            | Self::Coalesce { ty, .. }
            | Self::DictionaryGet { ty, .. }
            | Self::Parse { ty, .. } => *ty,
            Self::EnumBacking { .. } => ScalarType::Integer(IntegerType::Int64),
            Self::CollectionIndexOf { .. } => ScalarType::Integer(IntegerType::Int64),
            Self::StringIntrinsic(call) => match call.result {
                Type::NullableScalar(ty) => ty,
                _ => panic!("validated String intrinsic nullable scalar result"),
            },
            Self::Value(value) => value.ty(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassExpression {
    Local {
        class: ClassId,
        local: LocalId,
        transfer: bool,
    },
    Property {
        class: ClassId,
        object: LocalId,
        property: PropertyId,
    },
    Call {
        class: ClassId,
        function: FunctionId,
        args: Vec<Rvalue>,
        return_borrow: Option<ReturnBorrow>,
    },
    New {
        class: ClassId,
        properties: Vec<PropertyValue>,
        constructor: Option<FunctionId>,
        args: Vec<Rvalue>,
    },
    NullableLocalAssumeNonNull {
        class: ClassId,
        local: LocalId,
        transfer: bool,
    },
    Coalesce {
        class: ClassId,
        left: Box<NullableClassExpression>,
        right: Box<ClassExpression>,
        transfer: bool,
    },
    CollectionIndex {
        class: ClassId,
        collection: LocalId,
        index: Box<Rvalue>,
        /// True when `index` is a position in the collection rather than a key.
        positional: bool,
        transfer: bool,
    },
    MixedPayload {
        class: ClassId,
        mixed: LocalId,
        transfer: bool,
    },
    SharedPayload {
        class: ClassId,
        reference: Box<SharedReferenceExpression>,
    },
    SharedAccessPayload {
        class: ClassId,
        access: LocalId,
        writable: bool,
    },
}

impl ClassExpression {
    pub const fn class(&self) -> ClassId {
        match self {
            Self::Local { class, .. }
            | Self::Property { class, .. }
            | Self::Call { class, .. }
            | Self::New { class, .. }
            | Self::NullableLocalAssumeNonNull { class, .. }
            | Self::Coalesce { class, .. }
            | Self::CollectionIndex { class, .. }
            | Self::MixedPayload { class, .. }
            | Self::SharedPayload { class, .. }
            | Self::SharedAccessPayload { class, .. } => *class,
        }
    }

    pub const fn owned_temporary_class(&self) -> Option<ClassId> {
        match self {
            Self::New { class, .. }
            | Self::Call {
                class,
                return_borrow: None,
                ..
            } => Some(*class),
            Self::Local {
                class,
                transfer: true,
                ..
            } => Some(*class),
            Self::Local {
                transfer: false, ..
            }
            | Self::Property { .. }
            | Self::NullableLocalAssumeNonNull { .. }
            | Self::Coalesce { .. }
            | Self::CollectionIndex { .. }
            | Self::MixedPayload { .. }
            | Self::SharedPayload { .. }
            | Self::SharedAccessPayload { .. }
            | Self::Call {
                return_borrow: Some(_),
                ..
            } => None,
        }
    }

    pub const fn borrows_class_value(&self) -> bool {
        match self {
            Self::Local { transfer, .. }
            | Self::NullableLocalAssumeNonNull { transfer, .. }
            | Self::Coalesce { transfer, .. }
            | Self::CollectionIndex { transfer, .. }
            | Self::MixedPayload { transfer, .. } => !*transfer,
            Self::Property { .. }
            | Self::SharedPayload { .. }
            | Self::SharedAccessPayload { .. }
            | Self::Call {
                return_borrow: Some(_),
                ..
            } => true,
            Self::Call {
                return_borrow: None,
                ..
            }
            | Self::New { .. } => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyValue {
    pub property: PropertyId,
    pub source: PropertyValueSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropertyValueSource {
    Expression(Rvalue),
    ConstructorArgument(usize),
    ConstructorBody,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueExpression {
    Integer(IntegerExpression),
    Float(FloatExpression),
    Bool(BoolExpression),
    Enum(EnumExpression),
}

impl ValueExpression {
    pub const fn ty(&self) -> ScalarType {
        match self {
            Self::Integer(value) => ScalarType::Integer(value.ty()),
            Self::Float(value) => ScalarType::Float(value.ty()),
            Self::Bool(_) => ScalarType::Bool,
            Self::Enum(value) => ScalarType::Enum(value.enum_id()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnumExpression {
    Use {
        enum_id: EnumId,
        operand: Operand,
    },
    Case(EnumValue),
    Call {
        enum_id: EnumId,
        function: FunctionId,
        args: Vec<Rvalue>,
    },
    Coalesce {
        enum_id: EnumId,
        left: Box<NullableScalarExpression>,
        right: Box<EnumExpression>,
    },
}

impl EnumExpression {
    pub const fn enum_id(&self) -> EnumId {
        match self {
            Self::Use { enum_id, .. }
            | Self::Call { enum_id, .. }
            | Self::Coalesce { enum_id, .. } => *enum_id,
            Self::Case(value) => value.enum_id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegerUnaryOp {
    Negate,
    BitwiseNot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegerBinaryOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    ShiftLeft,
    ShiftRight,
    BitwiseAnd,
    BitwiseXor,
    BitwiseOr,
}

#[derive(Debug, Clone)]
pub enum IntegerExpression {
    Use {
        ty: IntegerType,
        operand: Operand,
    },
    Unary {
        ty: IntegerType,
        op: IntegerUnaryOp,
        operand: Box<IntegerExpression>,
        span: Span,
    },
    Binary {
        ty: IntegerType,
        op: IntegerBinaryOp,
        left: Box<IntegerExpression>,
        right: Box<IntegerExpression>,
        span: Span,
        right_span: Span,
    },
    Convert {
        ty: IntegerType,
        value: Box<IntegerExpression>,
        span: Span,
        value_span: Span,
    },
    FloatToInt {
        value: Box<FloatExpression>,
        span: Span,
        value_span: Span,
    },
    Call {
        ty: IntegerType,
        function: FunctionId,
        args: Vec<Rvalue>,
    },
    Coalesce {
        ty: IntegerType,
        left: Box<NullableScalarExpression>,
        right: Box<IntegerExpression>,
    },
    EnumBacking {
        enum_id: EnumId,
        value: Box<EnumExpression>,
    },
}

impl PartialEq for IntegerExpression {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::Use {
                    ty: left_ty,
                    operand: left,
                },
                Self::Use {
                    ty: right_ty,
                    operand: right,
                },
            ) => left_ty == right_ty && left == right,
            (
                Self::Unary {
                    ty: left_ty,
                    op: left_op,
                    operand: left,
                    ..
                },
                Self::Unary {
                    ty: right_ty,
                    op: right_op,
                    operand: right,
                    ..
                },
            ) => left_ty == right_ty && left_op == right_op && left == right,
            (
                Self::Binary {
                    ty: left_ty,
                    op: left_op,
                    left: left_left,
                    right: left_right,
                    ..
                },
                Self::Binary {
                    ty: right_ty,
                    op: right_op,
                    left: right_left,
                    right: right_right,
                    ..
                },
            ) => {
                left_ty == right_ty
                    && left_op == right_op
                    && left_left == right_left
                    && left_right == right_right
            }
            (
                Self::Convert {
                    ty: left_ty,
                    value: left,
                    ..
                },
                Self::Convert {
                    ty: right_ty,
                    value: right,
                    ..
                },
            ) => left_ty == right_ty && left == right,
            (Self::FloatToInt { value: left, .. }, Self::FloatToInt { value: right, .. }) => {
                left == right
            }
            (
                Self::Call {
                    ty: left_ty,
                    function: left_function,
                    args: left_args,
                },
                Self::Call {
                    ty: right_ty,
                    function: right_function,
                    args: right_args,
                },
            ) => left_ty == right_ty && left_function == right_function && left_args == right_args,
            (
                Self::Coalesce {
                    ty: left_ty,
                    left: left_left,
                    right: left_right,
                },
                Self::Coalesce {
                    ty: right_ty,
                    left: right_left,
                    right: right_right,
                },
            ) => left_ty == right_ty && left_left == right_left && left_right == right_right,
            (
                Self::EnumBacking {
                    enum_id: left_enum,
                    value: left,
                },
                Self::EnumBacking {
                    enum_id: right_enum,
                    value: right,
                },
            ) => left_enum == right_enum && left == right,
            _ => false,
        }
    }
}

impl Eq for IntegerExpression {}

impl IntegerExpression {
    pub const fn ty(&self) -> IntegerType {
        match self {
            Self::Use { ty, .. }
            | Self::Unary { ty, .. }
            | Self::Binary { ty, .. }
            | Self::Convert { ty, .. }
            | Self::Call { ty, .. }
            | Self::Coalesce { ty, .. } => *ty,
            Self::FloatToInt { .. } => IntegerType::Int64,
            Self::EnumBacking { .. } => IntegerType::Int64,
        }
    }

    pub const fn use_operand(ty: IntegerType, operand: Operand) -> Self {
        Self::Use { ty, operand }
    }

    pub const fn constant(value: IntegerValue) -> Self {
        Self::Use {
            ty: value.ty,
            operand: Operand::Scalar(ScalarValue::Integer(value)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloatBinaryOp {
    Add,
    Subtract,
    Multiply,
    Divide,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FloatExpression {
    Use {
        ty: FloatType,
        operand: Operand,
    },
    Negate {
        ty: FloatType,
        operand: Box<FloatExpression>,
    },
    Binary {
        ty: FloatType,
        op: FloatBinaryOp,
        left: Box<FloatExpression>,
        right: Box<FloatExpression>,
    },
    IntToFloat {
        value: Box<IntegerExpression>,
    },
    Call {
        ty: FloatType,
        function: FunctionId,
        args: Vec<Rvalue>,
    },
    Coalesce {
        ty: FloatType,
        left: Box<NullableScalarExpression>,
        right: Box<FloatExpression>,
    },
}

impl FloatExpression {
    pub const fn ty(&self) -> FloatType {
        match self {
            Self::Use { ty, .. }
            | Self::Negate { ty, .. }
            | Self::Binary { ty, .. }
            | Self::Call { ty, .. }
            | Self::Coalesce { ty, .. } => *ty,
            Self::IntToFloat { .. } => FloatType::Float64,
        }
    }

    pub const fn constant(value: FloatValue) -> Self {
        Self::Use {
            ty: value.ty,
            operand: Operand::Scalar(ScalarValue::Float(value)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StringExpression {
    Literal(String),
    Local(LocalId),
    NullableLocalAssumeNonNull(LocalId),
    Property {
        object: LocalId,
        property: PropertyId,
    },
    Static(StaticId),
    Concat(Vec<StringExpression>),
    Display(ValueExpression),
    Call {
        function: FunctionId,
        args: Vec<Rvalue>,
    },
    ReadFile {
        path: Box<StringExpression>,
        path_span: Span,
    },
    Format(Box<FormatExpression>),
    Coalesce {
        left: Box<NullableStringExpression>,
        right: Box<StringExpression>,
    },
    CollectionIndex {
        collection: LocalId,
        index: Box<Rvalue>,
        /// True when `index` is a position in the collection rather than a key.
        positional: bool,
        remove: bool,
    },
    CollectionKeyAt {
        collection: LocalId,
        offset: Box<Rvalue>,
    },
    MixedPayload(LocalId),
    Intrinsic(Box<StringIntrinsicCall>),
    EnumBacking {
        enum_id: EnumId,
        value: Box<EnumExpression>,
    },
}

impl StringExpression {
    pub const fn is_borrowed_place(&self) -> bool {
        matches!(
            self,
            Self::Local(_)
                | Self::NullableLocalAssumeNonNull(_)
                | Self::Property { .. }
                | Self::Static(_)
                | Self::MixedPayload(_)
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NullableStringExpression {
    Null,
    String(StringExpression),
    Local(LocalId),
    Property {
        object: LocalId,
        property: PropertyId,
    },
    Static(StaticId),
    /// `read_line(string $prompt = ""): ?string`.
    ///
    /// One canonical operation owns the complete ordering contract: the prompt is
    /// evaluated exactly once, written to stdout with no added characters, stdout
    /// is flushed, and only then is one line read. The zero-argument source form
    /// lowers to the canonical empty-string prompt, so there is no second
    /// operation and no overload pair.
    ReadLine {
        prompt: Box<StringExpression>,
        prompt_span: Span,
    },
    Call {
        function: FunctionId,
        args: Vec<Rvalue>,
    },
    NullSafeProperty {
        object: Box<NullableClassExpression>,
        property: PropertyId,
    },
    NullSafeCall {
        object: Box<NullableClassExpression>,
        function: FunctionId,
        args: Vec<Rvalue>,
    },
    EnumBacking {
        enum_id: EnumId,
        value: Box<NullableScalarExpression>,
    },
    Coalesce {
        left: Box<NullableStringExpression>,
        right: Box<NullableStringExpression>,
    },
    DictionaryGet {
        collection: LocalId,
        key: Box<Rvalue>,
        access: NullableCollectionAccess,
    },
    Intrinsic(Box<StringIntrinsicCall>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NullableClassExpression {
    Null(ClassId),
    Class(ClassExpression),
    SharedPayload {
        class: ClassId,
        reference: Box<NullableSharedReferenceExpression>,
    },
    Local {
        class: ClassId,
        local: LocalId,
        transfer: bool,
    },
    Property {
        class: ClassId,
        object: LocalId,
        property: PropertyId,
    },
    Call {
        class: ClassId,
        function: FunctionId,
        args: Vec<Rvalue>,
        return_borrow: Option<ReturnBorrow>,
    },
    NullSafeProperty {
        class: ClassId,
        object: Box<NullableClassExpression>,
        property: PropertyId,
    },
    NullSafeCall {
        class: ClassId,
        object: Box<NullableClassExpression>,
        function: FunctionId,
        args: Vec<Rvalue>,
        return_borrow: Option<ReturnBorrow>,
    },
    Coalesce {
        class: ClassId,
        left: Box<NullableClassExpression>,
        right: Box<NullableClassExpression>,
        transfer: bool,
    },
    DictionaryGet {
        class: ClassId,
        collection: LocalId,
        key: Box<Rvalue>,
        access: NullableCollectionAccess,
    },
}

impl NullableClassExpression {
    pub const fn class(&self) -> ClassId {
        match self {
            Self::Null(class)
            | Self::SharedPayload { class, .. }
            | Self::Local { class, .. }
            | Self::Property { class, .. }
            | Self::Call { class, .. }
            | Self::NullSafeProperty { class, .. }
            | Self::NullSafeCall { class, .. }
            | Self::Coalesce { class, .. }
            | Self::DictionaryGet { class, .. } => *class,
            Self::Class(value) => value.class(),
        }
    }

    pub const fn owned_temporary_class(&self) -> Option<ClassId> {
        match self {
            Self::Class(value) => value.owned_temporary_class(),
            Self::Local {
                class,
                transfer: true,
                ..
            } => Some(*class),
            Self::Call {
                class,
                return_borrow: None,
                ..
            }
            | Self::NullSafeCall {
                class,
                return_borrow: None,
                ..
            } => Some(*class),
            Self::Null(_)
            | Self::SharedPayload { .. }
            | Self::Local {
                transfer: false, ..
            }
            | Self::Property { .. }
            | Self::Call {
                return_borrow: Some(_),
                ..
            }
            | Self::NullSafeProperty { .. }
            | Self::Coalesce { .. }
            | Self::DictionaryGet { .. }
            | Self::NullSafeCall {
                return_borrow: Some(_),
                ..
            } => None,
        }
    }

    pub const fn borrows_class_value(&self) -> bool {
        match self {
            Self::Class(value) => value.borrows_class_value(),
            Self::Local { transfer, .. } | Self::Coalesce { transfer, .. } => !*transfer,
            Self::Property { .. }
            | Self::SharedPayload { .. }
            | Self::NullSafeProperty { .. }
            | Self::Call {
                return_borrow: Some(_),
                ..
            }
            | Self::NullSafeCall {
                return_borrow: Some(_),
                ..
            } => true,
            Self::DictionaryGet { access, .. } => !matches!(
                access,
                NullableCollectionAccess::Remove
                    | NullableCollectionAccess::Pop
                    | NullableCollectionAccess::PopFront
                    | NullableCollectionAccess::PopBack
            ),
            Self::Null(_)
            | Self::Call {
                return_borrow: None,
                ..
            }
            | Self::NullSafeCall {
                return_borrow: None,
                ..
            } => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatArgument {
    Value(ValueExpression),
    String(StringExpression),
    ClassDisplay(StringExpression),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatExpression {
    pub pieces: Vec<FormatPiece>,
    pub arguments: Vec<FormatArgument>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoolExpression {
    Use {
        operand: Operand,
    },
    Compare {
        op: CompareOp,
        left: Box<ValueExpression>,
        right: Box<ValueExpression>,
    },
    StringCompare {
        op: CompareOp,
        left: Box<StringExpression>,
        right: Box<StringExpression>,
    },
    NullableStringCompare {
        op: CompareOp,
        left: Box<NullableStringExpression>,
        right: Box<NullableStringExpression>,
    },
    NullableScalarIsPresent(Box<NullableScalarExpression>),
    NullableClassIsPresent(Box<NullableClassExpression>),
    NullableCollectionIsPresent(Box<NullableCollectionExpression>),
    NullableSharedReferenceIsPresent(Box<NullableSharedReferenceExpression>),
    NullableWeakReferenceIsPresent(Box<NullableWeakReferenceExpression>),
    NullableWritableSharedReferenceIsPresent(Box<NullableWritableSharedReferenceExpression>),
    NullableWritableWeakReferenceIsPresent(Box<NullableWritableWeakReferenceExpression>),
    NullableSharedReferenceAccessIsPresent(Box<NullableSharedReferenceAccessExpression>),
    NullableMixedIsPresent(Box<NullableMixedExpression>),
    NullablePayloadEnumIsPresent(Box<NullablePayloadEnumExpression>),
    PayloadEnumCompare {
        op: CompareOp,
        left: Box<PayloadEnumExpression>,
        right: Box<PayloadEnumExpression>,
    },
    PayloadEnumIsCase {
        local: LocalId,
        ty: PayloadEnumType,
        case: EnumCaseId,
        nullable: bool,
    },
    NullablePayloadEnumCompare {
        op: CompareOp,
        left: Box<NullablePayloadEnumExpression>,
        right: Box<NullablePayloadEnumExpression>,
    },
    Not(Box<BoolExpression>),
    Binary {
        op: BoolBinaryOp,
        left: Box<BoolExpression>,
        right: Box<BoolExpression>,
    },
    Call {
        function: FunctionId,
        args: Vec<Rvalue>,
    },
    Coalesce {
        left: Box<NullableScalarExpression>,
        right: Box<BoolExpression>,
    },
    CollectionHas {
        collection: LocalId,
        value: Box<Rvalue>,
        op: CollectionMembershipOp,
    },
    CollectionIsEmpty {
        collection: LocalId,
    },
    CollectionEqual {
        left: LocalId,
        right: LocalId,
    },
    MixedIs {
        mixed: Box<MixedExpression>,
        tag: MixedTag,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionMembershipOp {
    Contains,
    ContainsValue,
    Add,
    Remove,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareOp {
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoolBinaryOp {
    And,
    Or,
    Xor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Statement {
    AssignLocal {
        target: LocalId,
        value: Rvalue,
    },
    /// Evaluates `value` once, then initializes every target from left to right.
    /// Validation restricts this to Copy values and nullable move-type `null`.
    AssignLocalGroup {
        targets: Vec<LocalId>,
        value: Rvalue,
    },
    BindPayloadEnumFields {
        source: LocalId,
        ty: PayloadEnumType,
        case: EnumCaseId,
        nullable: bool,
        mode: MatchBindingMode,
        targets: Vec<LocalId>,
    },
    /// Validation-only identity for a lowered match result. Backends execute the
    /// explicit CFG and ignore this statement after shared validation.
    MatchResultPlan {
        scrutinee: LocalId,
        mode: MatchOwnershipMode,
        result: LocalId,
        arms: Vec<MatchArmPlan>,
        merge: BlockId,
    },
    /// Validation-only identity for source control flow that lowers to ordinary
    /// blocks and branches. Backends execute the CFG and ignore this statement.
    ControlFlowPlan(ControlFlowPlan),
    EchoStringLiteral(String),
    EchoString(StringExpression),
    CallVoid {
        function: FunctionId,
        args: Vec<Rvalue>,
        span: Span,
    },
    CallBorrowed {
        function: FunctionId,
        args: Vec<Rvalue>,
        span: Span,
    },
    CallNullSafe {
        object: NullableClassExpression,
        function: FunctionId,
        args: Vec<Rvalue>,
        span: Span,
    },
    Printf(FormatExpression),
    WriteFile {
        path: StringExpression,
        contents: StringExpression,
    },
    AppendFile {
        path: StringExpression,
        contents: StringExpression,
    },
    WriteStderr(StringExpression),
    WriteFileBytes {
        path: StringExpression,
        contents: LocalId,
        append: bool,
    },
    WriteStreamBytes {
        contents: LocalId,
        stderr: bool,
    },
    AssignProperty {
        object: LocalId,
        property: PropertyId,
        value: Rvalue,
    },
    AssignStatic {
        target: StaticId,
        value: Rvalue,
    },
    DropClass {
        local: LocalId,
        class: ClassId,
    },
    DropSharedReference {
        local: LocalId,
        class: ClassId,
    },
    DropWeakReference {
        local: LocalId,
        class: ClassId,
    },
    DropWritableSharedReference {
        local: LocalId,
        payload: WritableSharedPayload,
    },
    DropWritableWeakReference {
        local: LocalId,
        payload: WritableSharedPayload,
    },
    DropSharedReferenceAccess {
        local: LocalId,
        payload: WritableSharedPayload,
        writable: bool,
    },
    DropString {
        local: LocalId,
    },
    DropMixed {
        local: LocalId,
    },
    CollectionAdd {
        collection: LocalId,
        value: Rvalue,
        index: Option<Rvalue>,
        op: CollectionMutationOp,
    },
    CollectionSet {
        collection: LocalId,
        key: Rvalue,
        value: Rvalue,
    },
    AssignCollectionIndex {
        collection: LocalId,
        index: Rvalue,
        /// True when `index` is a position in the collection rather than a key.
        positional: bool,
        value: Rvalue,
    },
    CollectionClear {
        collection: LocalId,
        collection_type: CollectionTypeId,
    },
    DropCollection {
        local: LocalId,
        collection: CollectionTypeId,
    },
    DropPayloadEnum {
        local: LocalId,
        ty: PayloadEnumType,
        nullable: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchOwnershipMode {
    Borrowed,
    Consumed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchBindingMode {
    GuardView,
    BorrowedArm,
    ConsumedArm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchArmPlan {
    pub guard: Option<BlockId>,
    pub binding: BlockId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlFlowPlan {
    Given(GivenControlFlowPlan),
    When(WhenResultPlan),
    DoWhile(DoWhilePlan),
    /// Reserved solely so validation can prove that pending finalizer lowering
    /// never reaches a Slice 1 backend.
    PendingFinally {
        span: Span,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GivenAttachment {
    If,
    When,
    While,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GivenControlFlowPlan {
    pub attachment: GivenAttachment,
    pub setup_entry: BlockId,
    pub setup_exit: BlockId,
    pub predicates: Vec<GivenPredicatePlan>,
    pub condition: BlockId,
    pub condition_type: Type,
    pub gate_failed: Option<BlockId>,
    pub continue_sources: Vec<BlockId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GivenPredicatePlan {
    pub block: BlockId,
    pub ty: Type,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhenResultOwnership {
    Borrowed,
    Owned,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhenResultPlan {
    pub result: LocalId,
    pub ownership: WhenResultOwnership,
    pub branches: Vec<BlockId>,
    pub merge: BlockId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoWhilePlan {
    pub entry: BlockId,
    pub body: BlockId,
    pub condition: BlockId,
    pub condition_type: Type,
    pub exit: BlockId,
    pub continue_sources: Vec<BlockId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionMutationOp {
    Add,
    InsertAt,
    Remove,
    Push,
    PushFront,
    PushBack,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Terminator {
    Return(Rvalue),
    ReturnVoid,
    Panic {
        message: StringExpression,
        span: Span,
    },
    Unreachable,
    Jump(BlockId),
    Branch {
        condition: BoolExpression,
        then_block: BlockId,
        else_block: BlockId,
    },
}

pub(crate) fn class_temporary_capacity(function: &Function) -> usize {
    function
        .blocks
        .iter()
        .map(|block| {
            block
                .statements
                .iter()
                .map(statement_class_temporary_capacity)
                .sum::<usize>()
                + terminator_class_temporary_capacity(&block.terminator)
        })
        .sum()
}

fn statement_class_temporary_capacity(statement: &Statement) -> usize {
    match statement {
        Statement::AssignLocal { value, .. }
        | Statement::AssignLocalGroup { value, .. }
        | Statement::AssignProperty { value, .. }
        | Statement::AssignStatic { value, .. } => rvalue_class_temporary_capacity(value),
        Statement::CollectionAdd { value, .. } => rvalue_class_temporary_capacity(value),
        Statement::CollectionSet { key, value, .. } => {
            rvalue_class_temporary_capacity(key) + rvalue_class_temporary_capacity(value)
        }
        Statement::AssignCollectionIndex { index, value, .. } => {
            rvalue_class_temporary_capacity(index) + rvalue_class_temporary_capacity(value)
        }
        Statement::EchoStringLiteral(_)
        | Statement::BindPayloadEnumFields { .. }
        | Statement::MatchResultPlan { .. }
        | Statement::ControlFlowPlan(_)
        | Statement::DropClass { .. }
        | Statement::DropSharedReference { .. }
        | Statement::DropWeakReference { .. }
        | Statement::DropWritableSharedReference { .. }
        | Statement::DropWritableWeakReference { .. }
        | Statement::DropSharedReferenceAccess { .. }
        | Statement::DropString { .. }
        | Statement::DropMixed { .. }
        | Statement::DropPayloadEnum { .. }
        | Statement::CollectionClear { .. }
        | Statement::DropCollection { .. }
        | Statement::WriteStreamBytes { .. } => 0,
        Statement::EchoString(value) | Statement::WriteStderr(value) => {
            string_class_temporary_capacity(value)
        }
        Statement::CallVoid { args, .. } | Statement::CallBorrowed { args, .. } => {
            args.iter().map(rvalue_class_temporary_capacity).sum()
        }
        Statement::CallNullSafe { object, args, .. } => {
            nullable_class_temporary_capacity(object)
                + args
                    .iter()
                    .map(rvalue_class_temporary_capacity)
                    .sum::<usize>()
        }
        Statement::Printf(format) => format_class_temporary_capacity(format),
        Statement::WriteFile { path, contents } | Statement::AppendFile { path, contents } => {
            string_class_temporary_capacity(path) + string_class_temporary_capacity(contents)
        }
        Statement::WriteFileBytes { path, .. } => string_class_temporary_capacity(path),
    }
}

fn terminator_class_temporary_capacity(terminator: &Terminator) -> usize {
    match terminator {
        Terminator::Return(value) => rvalue_class_temporary_capacity(value),
        Terminator::Panic { message, .. } => string_class_temporary_capacity(message),
        Terminator::Branch { condition, .. } => bool_class_temporary_capacity(condition),
        Terminator::ReturnVoid | Terminator::Unreachable | Terminator::Jump(_) => 0,
    }
}

fn rvalue_class_temporary_capacity(value: &Rvalue) -> usize {
    usize::from(value.owned_temporary_shared().is_some())
        + match value {
            Rvalue::Value(value) => value_class_temporary_capacity(value),
            Rvalue::String(value) => string_class_temporary_capacity(value),
            Rvalue::Mixed(value) => mixed_class_temporary_capacity(value),
            Rvalue::NullableScalar(value) => nullable_scalar_class_temporary_capacity(value),
            Rvalue::NullableString(value) => nullable_string_class_temporary_capacity(value),
            Rvalue::NullableMixed(value) => nullable_mixed_class_temporary_capacity(value),
            Rvalue::Class(value) => class_expression_temporary_capacity(value),
            Rvalue::NullableClass(value) => nullable_class_temporary_capacity(value),
            Rvalue::SharedReference(value) => shared_class_temporary_capacity(value),
            Rvalue::WeakReference(value) => weak_class_temporary_capacity(value),
            Rvalue::NullableSharedReference(value) => {
                nullable_shared_class_temporary_capacity(value)
            }
            Rvalue::NullableWeakReference(value) => nullable_weak_class_temporary_capacity(value),
            Rvalue::WritableSharedReference(value) => match value {
                WritableSharedReferenceExpression::New { value, .. } => {
                    rvalue_class_temporary_capacity(value)
                }
                WritableSharedReferenceExpression::Call { args, .. } => {
                    args.iter().map(rvalue_class_temporary_capacity).sum()
                }
                WritableSharedReferenceExpression::Share { value, .. } => {
                    writable_shared_class_temporary_capacity(value)
                }
                WritableSharedReferenceExpression::Coalesce { left, right, .. } => {
                    nullable_writable_shared_class_temporary_capacity(left)
                        + writable_shared_class_temporary_capacity(right)
                }
                WritableSharedReferenceExpression::CollectionIndex { index, .. } => {
                    rvalue_class_temporary_capacity(index)
                }
                WritableSharedReferenceExpression::Local { .. }
                | WritableSharedReferenceExpression::NullableLocalAssumeNonNull { .. }
                | WritableSharedReferenceExpression::Property { .. } => 0,
            },
            Rvalue::WritableWeakReference(value) => match value {
                WritableWeakReferenceExpression::Call { args, .. } => {
                    args.iter().map(rvalue_class_temporary_capacity).sum()
                }
                WritableWeakReferenceExpression::Create { value, .. } => {
                    writable_shared_class_temporary_capacity(value)
                }
                WritableWeakReferenceExpression::Coalesce { left, right, .. } => {
                    nullable_writable_weak_class_temporary_capacity(left)
                        + writable_weak_class_temporary_capacity(right)
                }
                WritableWeakReferenceExpression::CollectionIndex { index, .. } => {
                    rvalue_class_temporary_capacity(index)
                }
                WritableWeakReferenceExpression::Local { .. }
                | WritableWeakReferenceExpression::NullableLocalAssumeNonNull { .. }
                | WritableWeakReferenceExpression::Property { .. } => 0,
            },
            Rvalue::NullableWritableSharedReference(value) => {
                nullable_writable_shared_class_temporary_capacity(value)
            }
            Rvalue::NullableWritableWeakReference(value) => {
                nullable_writable_weak_class_temporary_capacity(value)
            }
            Rvalue::SharedReferenceAccess(value) => match value {
                SharedReferenceAccessExpression::Call { args, .. } => {
                    args.iter().map(rvalue_class_temporary_capacity).sum()
                }
                SharedReferenceAccessExpression::Acquire { value, .. } => {
                    writable_shared_class_temporary_capacity(value)
                }
                SharedReferenceAccessExpression::CollectionIndex { index, .. } => {
                    rvalue_class_temporary_capacity(index)
                }
                SharedReferenceAccessExpression::Local { .. }
                | SharedReferenceAccessExpression::NullableLocalAssumeNonNull { .. }
                | SharedReferenceAccessExpression::Property { .. } => 0,
            },
            Rvalue::NullableSharedReferenceAccess(value) => {
                nullable_shared_access_class_temporary_capacity(value)
            }
            Rvalue::Collection(value) => collection_class_temporary_capacity(value),
            Rvalue::NullableCollection(value) => {
                nullable_collection_class_temporary_capacity(value)
            }
            Rvalue::PayloadEnum(value) => payload_enum_class_temporary_capacity(value),
            Rvalue::NullablePayloadEnum(value) => {
                nullable_payload_enum_class_temporary_capacity(value)
            }
        }
}

fn payload_enum_class_temporary_capacity(value: &PayloadEnumExpression) -> usize {
    match value {
        PayloadEnumExpression::Construct { fields, .. }
        | PayloadEnumExpression::Call { args: fields, .. } => {
            fields.iter().map(rvalue_class_temporary_capacity).sum()
        }
        PayloadEnumExpression::Use { place, .. } => payload_enum_place_class_capacity(place),
        PayloadEnumExpression::Coalesce { left, right, .. } => {
            nullable_payload_enum_class_temporary_capacity(left)
                + payload_enum_class_temporary_capacity(right)
        }
    }
}

fn nullable_payload_enum_class_temporary_capacity(value: &NullablePayloadEnumExpression) -> usize {
    match value {
        NullablePayloadEnumExpression::Null(_) => 0,
        NullablePayloadEnumExpression::Value(value) => payload_enum_class_temporary_capacity(value),
        NullablePayloadEnumExpression::Use { place, .. } => {
            payload_enum_place_class_capacity(place)
        }
        NullablePayloadEnumExpression::Call { args, .. } => {
            args.iter().map(rvalue_class_temporary_capacity).sum()
        }
        NullablePayloadEnumExpression::CollectionGet { key, .. } => {
            rvalue_class_temporary_capacity(key)
        }
        NullablePayloadEnumExpression::Coalesce { left, right, .. } => {
            nullable_payload_enum_class_temporary_capacity(left)
                + nullable_payload_enum_class_temporary_capacity(right)
        }
    }
}

fn payload_enum_place_class_capacity(place: &PayloadEnumPlace) -> usize {
    match place {
        PayloadEnumPlace::CollectionIndex { index, .. } => rvalue_class_temporary_capacity(index),
        PayloadEnumPlace::Local(_)
        | PayloadEnumPlace::NullableLocalAssumeNonNull(_)
        | PayloadEnumPlace::Static(_)
        | PayloadEnumPlace::Property { .. }
        | PayloadEnumPlace::MixedPayload { .. } => 0,
    }
}

fn nullable_collection_class_temporary_capacity(value: &NullableCollectionExpression) -> usize {
    match value {
        NullableCollectionExpression::Collection(value) => {
            collection_class_temporary_capacity(value)
        }
        NullableCollectionExpression::Call { args, .. } => {
            args.iter().map(rvalue_class_temporary_capacity).sum()
        }
        NullableCollectionExpression::Coalesce { left, right, .. } => {
            nullable_collection_class_temporary_capacity(left)
                + nullable_collection_class_temporary_capacity(right)
        }
        NullableCollectionExpression::Null(_)
        | NullableCollectionExpression::Local { .. }
        | NullableCollectionExpression::Property { .. } => 0,
    }
}

fn nullable_shared_access_class_temporary_capacity(
    value: &NullableSharedReferenceAccessExpression,
) -> usize {
    match value {
        NullableSharedReferenceAccessExpression::Access(value) => match value.as_ref() {
            SharedReferenceAccessExpression::Call { args, .. } => {
                args.iter().map(rvalue_class_temporary_capacity).sum()
            }
            SharedReferenceAccessExpression::Acquire { value, .. } => {
                writable_shared_class_temporary_capacity(value)
            }
            SharedReferenceAccessExpression::CollectionIndex { index, .. } => {
                rvalue_class_temporary_capacity(index)
            }
            SharedReferenceAccessExpression::Local { .. }
            | SharedReferenceAccessExpression::NullableLocalAssumeNonNull { .. }
            | SharedReferenceAccessExpression::Property { .. } => 0,
        },
        NullableSharedReferenceAccessExpression::Call { args, .. } => {
            args.iter().map(rvalue_class_temporary_capacity).sum()
        }
        NullableSharedReferenceAccessExpression::NullSafeAcquire { value, .. } => {
            nullable_writable_shared_class_temporary_capacity(value)
        }
        NullableSharedReferenceAccessExpression::CollectionIndex { index, .. } => {
            rvalue_class_temporary_capacity(index)
        }
        NullableSharedReferenceAccessExpression::CollectionGet { key, .. } => {
            rvalue_class_temporary_capacity(key)
        }
        NullableSharedReferenceAccessExpression::Null { .. }
        | NullableSharedReferenceAccessExpression::Local { .. }
        | NullableSharedReferenceAccessExpression::Property { .. } => 0,
    }
}

fn writable_shared_class_temporary_capacity(value: &WritableSharedReferenceExpression) -> usize {
    match value {
        WritableSharedReferenceExpression::New { value, .. } => {
            rvalue_class_temporary_capacity(value)
        }
        WritableSharedReferenceExpression::Call { args, .. } => {
            args.iter().map(rvalue_class_temporary_capacity).sum()
        }
        WritableSharedReferenceExpression::Share { value, .. } => {
            writable_shared_class_temporary_capacity(value)
        }
        WritableSharedReferenceExpression::Coalesce { left, right, .. } => {
            nullable_writable_shared_class_temporary_capacity(left)
                .max(writable_shared_class_temporary_capacity(right))
        }
        WritableSharedReferenceExpression::CollectionIndex { index, .. } => {
            rvalue_class_temporary_capacity(index)
        }
        WritableSharedReferenceExpression::Local { .. }
        | WritableSharedReferenceExpression::NullableLocalAssumeNonNull { .. }
        | WritableSharedReferenceExpression::Property { .. } => 0,
    }
}

fn writable_weak_class_temporary_capacity(value: &WritableWeakReferenceExpression) -> usize {
    match value {
        WritableWeakReferenceExpression::Call { args, .. } => {
            args.iter().map(rvalue_class_temporary_capacity).sum()
        }
        WritableWeakReferenceExpression::Create { value, .. } => {
            writable_shared_class_temporary_capacity(value)
        }
        WritableWeakReferenceExpression::Coalesce { left, right, .. } => {
            nullable_writable_weak_class_temporary_capacity(left)
                .max(writable_weak_class_temporary_capacity(right))
        }
        WritableWeakReferenceExpression::CollectionIndex { index, .. } => {
            rvalue_class_temporary_capacity(index)
        }
        WritableWeakReferenceExpression::Local { .. }
        | WritableWeakReferenceExpression::NullableLocalAssumeNonNull { .. }
        | WritableWeakReferenceExpression::Property { .. } => 0,
    }
}

fn nullable_writable_shared_class_temporary_capacity(
    value: &NullableWritableSharedReferenceExpression,
) -> usize {
    match value {
        NullableWritableSharedReferenceExpression::Strong(value) => {
            writable_shared_class_temporary_capacity(value)
        }
        NullableWritableSharedReferenceExpression::Call { args, .. } => {
            args.iter().map(rvalue_class_temporary_capacity).sum()
        }
        NullableWritableSharedReferenceExpression::Acquire { value, .. } => {
            writable_weak_class_temporary_capacity(value)
        }
        NullableWritableSharedReferenceExpression::NullSafeShare { value, .. }
        | NullableWritableSharedReferenceExpression::Coalesce { left: value, .. } => {
            nullable_writable_shared_class_temporary_capacity(value)
        }
        NullableWritableSharedReferenceExpression::NullSafeAcquire { value, .. } => {
            nullable_writable_weak_class_temporary_capacity(value)
        }
        NullableWritableSharedReferenceExpression::DictionaryGet { key, .. } => {
            rvalue_class_temporary_capacity(key)
        }
        NullableWritableSharedReferenceExpression::Null(_)
        | NullableWritableSharedReferenceExpression::Local { .. }
        | NullableWritableSharedReferenceExpression::Property { .. } => 0,
    }
}

fn nullable_writable_weak_class_temporary_capacity(
    value: &NullableWritableWeakReferenceExpression,
) -> usize {
    match value {
        NullableWritableWeakReferenceExpression::Weak(value) => {
            writable_weak_class_temporary_capacity(value)
        }
        NullableWritableWeakReferenceExpression::Call { args, .. } => {
            args.iter().map(rvalue_class_temporary_capacity).sum()
        }
        NullableWritableWeakReferenceExpression::NullSafeCreate { value, .. } => {
            nullable_writable_shared_class_temporary_capacity(value)
        }
        NullableWritableWeakReferenceExpression::Coalesce { left, right, .. } => {
            nullable_writable_weak_class_temporary_capacity(left)
                .max(nullable_writable_weak_class_temporary_capacity(right))
        }
        NullableWritableWeakReferenceExpression::DictionaryGet { key, .. } => {
            rvalue_class_temporary_capacity(key)
        }
        NullableWritableWeakReferenceExpression::Null(_)
        | NullableWritableWeakReferenceExpression::Local { .. }
        | NullableWritableWeakReferenceExpression::Property { .. } => 0,
    }
}

fn shared_class_temporary_capacity(value: &SharedReferenceExpression) -> usize {
    match value {
        SharedReferenceExpression::New { value, .. } => class_expression_temporary_capacity(value),
        SharedReferenceExpression::Call { args, .. } => {
            args.iter().map(rvalue_class_temporary_capacity).sum()
        }
        SharedReferenceExpression::Share { value, .. } => shared_class_temporary_capacity(value),
        SharedReferenceExpression::Coalesce { left, right, .. } => {
            nullable_shared_class_temporary_capacity(left)
                .max(shared_class_temporary_capacity(right))
        }
        SharedReferenceExpression::CollectionIndex { index, .. } => {
            rvalue_class_temporary_capacity(index)
        }
        SharedReferenceExpression::Local { .. }
        | SharedReferenceExpression::NullableLocalAssumeNonNull { .. }
        | SharedReferenceExpression::Property { .. } => 0,
    }
}

fn weak_class_temporary_capacity(value: &WeakReferenceExpression) -> usize {
    match value {
        WeakReferenceExpression::Call { args, .. } => {
            args.iter().map(rvalue_class_temporary_capacity).sum()
        }
        WeakReferenceExpression::Create { value, .. } => shared_class_temporary_capacity(value),
        WeakReferenceExpression::Coalesce { left, right, .. } => {
            nullable_weak_class_temporary_capacity(left).max(weak_class_temporary_capacity(right))
        }
        WeakReferenceExpression::CollectionIndex { index, .. } => {
            rvalue_class_temporary_capacity(index)
        }
        WeakReferenceExpression::Local { .. }
        | WeakReferenceExpression::NullableLocalAssumeNonNull { .. }
        | WeakReferenceExpression::Property { .. } => 0,
    }
}

fn nullable_shared_class_temporary_capacity(value: &NullableSharedReferenceExpression) -> usize {
    match value {
        NullableSharedReferenceExpression::Shared(value) => shared_class_temporary_capacity(value),
        NullableSharedReferenceExpression::Call { args, .. } => {
            args.iter().map(rvalue_class_temporary_capacity).sum()
        }
        NullableSharedReferenceExpression::Acquire { value, .. } => {
            weak_class_temporary_capacity(value)
        }
        NullableSharedReferenceExpression::NullSafeShare { value, .. } => {
            nullable_shared_class_temporary_capacity(value)
        }
        NullableSharedReferenceExpression::NullSafeAcquire { value, .. } => {
            nullable_weak_class_temporary_capacity(value)
        }
        NullableSharedReferenceExpression::Coalesce { left, right, .. } => {
            nullable_shared_class_temporary_capacity(left)
                .max(nullable_shared_class_temporary_capacity(right))
        }
        NullableSharedReferenceExpression::DictionaryGet { key, .. } => {
            rvalue_class_temporary_capacity(key)
        }
        NullableSharedReferenceExpression::CollectionIndex { index, .. } => {
            rvalue_class_temporary_capacity(index)
        }
        NullableSharedReferenceExpression::Null(_)
        | NullableSharedReferenceExpression::Local { .. }
        | NullableSharedReferenceExpression::Property { .. } => 0,
    }
}

fn nullable_weak_class_temporary_capacity(value: &NullableWeakReferenceExpression) -> usize {
    match value {
        NullableWeakReferenceExpression::Weak(value) => weak_class_temporary_capacity(value),
        NullableWeakReferenceExpression::Call { args, .. } => {
            args.iter().map(rvalue_class_temporary_capacity).sum()
        }
        NullableWeakReferenceExpression::NullSafeCreate { value, .. } => {
            nullable_shared_class_temporary_capacity(value)
        }
        NullableWeakReferenceExpression::Coalesce { left, right, .. } => {
            nullable_weak_class_temporary_capacity(left)
                .max(nullable_weak_class_temporary_capacity(right))
        }
        NullableWeakReferenceExpression::DictionaryGet { key, .. } => {
            rvalue_class_temporary_capacity(key)
        }
        NullableWeakReferenceExpression::CollectionIndex { index, .. } => {
            rvalue_class_temporary_capacity(index)
        }
        NullableWeakReferenceExpression::Null(_)
        | NullableWeakReferenceExpression::Local { .. }
        | NullableWeakReferenceExpression::Property { .. } => 0,
    }
}

fn mixed_class_temporary_capacity(value: &MixedExpression) -> usize {
    match value {
        MixedExpression::BoxValue(value) => value_class_temporary_capacity(value),
        MixedExpression::BoxString { value, .. } => string_class_temporary_capacity(value),
        MixedExpression::BoxClass { value, .. } => class_expression_temporary_capacity(value),
        MixedExpression::BoxPayloadEnum { value } => payload_enum_class_temporary_capacity(value),
        MixedExpression::Call { args, .. } => {
            args.iter().map(rvalue_class_temporary_capacity).sum()
        }
        MixedExpression::CollectionIndex { index, .. } => rvalue_class_temporary_capacity(index),
        MixedExpression::Local { transfer, .. } => usize::from(*transfer),
        MixedExpression::Property { .. } => 0,
    }
}

fn nullable_mixed_class_temporary_capacity(value: &NullableMixedExpression) -> usize {
    match value {
        NullableMixedExpression::Mixed(value) => mixed_class_temporary_capacity(value),
        NullableMixedExpression::BoxNullablePayloadEnum(value) => {
            nullable_payload_enum_class_temporary_capacity(value)
        }
        NullableMixedExpression::Call { args, .. } => {
            args.iter().map(rvalue_class_temporary_capacity).sum()
        }
        NullableMixedExpression::Coalesce { left, right, .. } => {
            nullable_mixed_class_temporary_capacity(left)
                + nullable_mixed_class_temporary_capacity(right)
        }
        NullableMixedExpression::Local { transfer, .. } => usize::from(*transfer),
        NullableMixedExpression::Null | NullableMixedExpression::Property { .. } => 0,
    }
}

fn collection_class_temporary_capacity(value: &CollectionExpression) -> usize {
    usize::from(value.owned_temporary_collection().is_some())
        + match value {
            CollectionExpression::Local { .. } => 0,
            CollectionExpression::From { .. }
            | CollectionExpression::FromBytes { .. }
            | CollectionExpression::BytesFromArray { .. }
            | CollectionExpression::ReadStdinBytes { .. } => 0,
            CollectionExpression::Call { args, .. } => {
                args.iter().map(rvalue_class_temporary_capacity).sum()
            }
            CollectionExpression::StringIntrinsic(call) => {
                call.args.iter().map(rvalue_class_temporary_capacity).sum()
            }
            CollectionExpression::Literal { entries, .. } => entries
                .iter()
                .map(|entry| {
                    entry
                        .key
                        .as_ref()
                        .map_or(0, rvalue_class_temporary_capacity)
                        + rvalue_class_temporary_capacity(&entry.value)
                })
                .sum(),
            CollectionExpression::Fill { value, count, .. } => {
                rvalue_class_temporary_capacity(value) + integer_class_temporary_capacity(count)
            }
            CollectionExpression::Index { index, .. } => rvalue_class_temporary_capacity(index),
            CollectionExpression::Property { .. }
            | CollectionExpression::SharedAccessPayload { .. } => 0,
            CollectionExpression::ReadFileBytes { path, .. } => {
                string_class_temporary_capacity(path)
            }
        }
}

fn value_class_temporary_capacity(value: &ValueExpression) -> usize {
    match value {
        ValueExpression::Integer(value) => integer_class_temporary_capacity(value),
        ValueExpression::Float(value) => float_class_temporary_capacity(value),
        ValueExpression::Bool(value) => bool_class_temporary_capacity(value),
        ValueExpression::Enum(value) => enum_class_temporary_capacity(value),
    }
}

fn enum_class_temporary_capacity(value: &EnumExpression) -> usize {
    match value {
        EnumExpression::Use { .. } | EnumExpression::Case(_) => 0,
        EnumExpression::Call { args, .. } => args.iter().map(rvalue_class_temporary_capacity).sum(),
        EnumExpression::Coalesce { left, right, .. } => {
            nullable_scalar_class_temporary_capacity(left) + enum_class_temporary_capacity(right)
        }
    }
}

fn integer_class_temporary_capacity(value: &IntegerExpression) -> usize {
    match value {
        IntegerExpression::Use { .. } => 0,
        IntegerExpression::Unary { operand, .. }
        | IntegerExpression::Convert { value: operand, .. } => {
            integer_class_temporary_capacity(operand)
        }
        IntegerExpression::Binary { left, right, .. } => {
            integer_class_temporary_capacity(left) + integer_class_temporary_capacity(right)
        }
        IntegerExpression::FloatToInt { value, .. } => float_class_temporary_capacity(value),
        IntegerExpression::Call { args, .. } => {
            args.iter().map(rvalue_class_temporary_capacity).sum()
        }
        IntegerExpression::Coalesce { left, right, .. } => {
            nullable_scalar_class_temporary_capacity(left) + integer_class_temporary_capacity(right)
        }
        IntegerExpression::EnumBacking { value, .. } => enum_class_temporary_capacity(value),
    }
}

fn float_class_temporary_capacity(value: &FloatExpression) -> usize {
    match value {
        FloatExpression::Use { .. } => 0,
        FloatExpression::Negate { operand, .. } => float_class_temporary_capacity(operand),
        FloatExpression::Binary { left, right, .. } => {
            float_class_temporary_capacity(left) + float_class_temporary_capacity(right)
        }
        FloatExpression::IntToFloat { value } => integer_class_temporary_capacity(value),
        FloatExpression::Call { args, .. } => {
            args.iter().map(rvalue_class_temporary_capacity).sum()
        }
        FloatExpression::Coalesce { left, right, .. } => {
            nullable_scalar_class_temporary_capacity(left) + float_class_temporary_capacity(right)
        }
    }
}

fn string_class_temporary_capacity(value: &StringExpression) -> usize {
    match value {
        StringExpression::Concat(parts) => parts.iter().map(string_class_temporary_capacity).sum(),
        StringExpression::Display(value) => value_class_temporary_capacity(value),
        StringExpression::Call { args, .. } => {
            args.iter().map(rvalue_class_temporary_capacity).sum()
        }
        StringExpression::ReadFile { path, .. } => string_class_temporary_capacity(path),
        StringExpression::Format(format) => format_class_temporary_capacity(format),
        StringExpression::Coalesce { left, right } => {
            nullable_string_class_temporary_capacity(left) + string_class_temporary_capacity(right)
        }
        StringExpression::CollectionIndex { index, .. } => rvalue_class_temporary_capacity(index),
        StringExpression::CollectionKeyAt { offset, .. } => rvalue_class_temporary_capacity(offset),
        StringExpression::Intrinsic(call) => {
            call.args.iter().map(rvalue_class_temporary_capacity).sum()
        }
        StringExpression::EnumBacking { value, .. } => enum_class_temporary_capacity(value),
        StringExpression::Literal(_)
        | StringExpression::Local(_)
        | StringExpression::NullableLocalAssumeNonNull(_)
        | StringExpression::MixedPayload(_)
        | StringExpression::Static(_)
        | StringExpression::Property { .. } => 0,
    }
}

fn nullable_string_class_temporary_capacity(value: &NullableStringExpression) -> usize {
    match value {
        NullableStringExpression::String(value) => string_class_temporary_capacity(value),
        NullableStringExpression::Call { args, .. } => {
            args.iter().map(rvalue_class_temporary_capacity).sum()
        }
        NullableStringExpression::EnumBacking { value, .. } => {
            nullable_scalar_class_temporary_capacity(value)
        }
        NullableStringExpression::Intrinsic(call) => {
            call.args.iter().map(rvalue_class_temporary_capacity).sum()
        }
        NullableStringExpression::NullSafeProperty { object, .. } => {
            nullable_class_temporary_capacity(object)
        }
        NullableStringExpression::NullSafeCall { object, args, .. } => {
            nullable_class_temporary_capacity(object)
                + args
                    .iter()
                    .map(rvalue_class_temporary_capacity)
                    .sum::<usize>()
        }
        NullableStringExpression::Coalesce { left, right } => {
            nullable_string_class_temporary_capacity(left)
                + nullable_string_class_temporary_capacity(right)
        }
        NullableStringExpression::DictionaryGet { key, .. } => rvalue_class_temporary_capacity(key),
        NullableStringExpression::ReadLine { prompt, .. } => {
            string_class_temporary_capacity(prompt)
        }
        NullableStringExpression::Null
        | NullableStringExpression::Local(_)
        | NullableStringExpression::Static(_)
        | NullableStringExpression::Property { .. } => 0,
    }
}

fn class_expression_temporary_capacity(value: &ClassExpression) -> usize {
    match value {
        ClassExpression::Local { .. }
        | ClassExpression::Property { .. }
        | ClassExpression::NullableLocalAssumeNonNull { .. }
        | ClassExpression::MixedPayload { .. }
        | ClassExpression::SharedAccessPayload { .. } => 0,
        ClassExpression::SharedPayload { reference, .. } => {
            shared_class_temporary_capacity(reference)
        }
        ClassExpression::Call { args, .. } => {
            usize::from(value.owned_temporary_class().is_some())
                + args
                    .iter()
                    .map(rvalue_class_temporary_capacity)
                    .sum::<usize>()
        }
        ClassExpression::New {
            properties, args, ..
        } => {
            1 + properties
                .iter()
                .filter_map(|property| match &property.source {
                    PropertyValueSource::Expression(value) => {
                        Some(rvalue_class_temporary_capacity(value))
                    }
                    PropertyValueSource::ConstructorArgument(_)
                    | PropertyValueSource::ConstructorBody => None,
                })
                .sum::<usize>()
                + args
                    .iter()
                    .map(rvalue_class_temporary_capacity)
                    .sum::<usize>()
        }
        ClassExpression::Coalesce { left, right, .. } => {
            nullable_class_temporary_capacity(left) + class_expression_temporary_capacity(right)
        }
        ClassExpression::CollectionIndex { index, .. } => rvalue_class_temporary_capacity(index),
    }
}

fn nullable_scalar_class_temporary_capacity(value: &NullableScalarExpression) -> usize {
    match value {
        NullableScalarExpression::Value(value) => value_class_temporary_capacity(value),
        NullableScalarExpression::Call { args, .. } => {
            args.iter().map(rvalue_class_temporary_capacity).sum()
        }
        NullableScalarExpression::EnumBacking { value, .. } => {
            nullable_scalar_class_temporary_capacity(value)
        }
        NullableScalarExpression::NullSafeProperty { object, .. } => {
            nullable_class_temporary_capacity(object)
        }
        NullableScalarExpression::NullSafeCall { object, args, .. } => {
            nullable_class_temporary_capacity(object)
                + args
                    .iter()
                    .map(rvalue_class_temporary_capacity)
                    .sum::<usize>()
        }
        NullableScalarExpression::Coalesce { left, right, .. } => {
            nullable_scalar_class_temporary_capacity(left)
                + nullable_scalar_class_temporary_capacity(right)
        }
        NullableScalarExpression::DictionaryGet { key, .. } => rvalue_class_temporary_capacity(key),
        NullableScalarExpression::CollectionIndexOf { value, .. } => {
            rvalue_class_temporary_capacity(value)
        }
        NullableScalarExpression::StringIntrinsic(call) => {
            call.args.iter().map(rvalue_class_temporary_capacity).sum()
        }
        NullableScalarExpression::Parse { .. }
        | NullableScalarExpression::Null(_)
        | NullableScalarExpression::Local { .. }
        | NullableScalarExpression::Property { .. }
        | NullableScalarExpression::Static { .. } => 0,
    }
}

fn nullable_class_temporary_capacity(value: &NullableClassExpression) -> usize {
    match value {
        NullableClassExpression::Class(value) => class_expression_temporary_capacity(value),
        NullableClassExpression::Call { args, .. } => {
            usize::from(value.owned_temporary_class().is_some())
                + args
                    .iter()
                    .map(rvalue_class_temporary_capacity)
                    .sum::<usize>()
        }
        NullableClassExpression::NullSafeCall { object, args, .. } => {
            usize::from(value.owned_temporary_class().is_some())
                + nullable_class_temporary_capacity(object)
                + args
                    .iter()
                    .map(rvalue_class_temporary_capacity)
                    .sum::<usize>()
        }
        NullableClassExpression::NullSafeProperty { object, .. } => {
            nullable_class_temporary_capacity(object)
        }
        NullableClassExpression::Coalesce { left, right, .. } => {
            nullable_class_temporary_capacity(left) + nullable_class_temporary_capacity(right)
        }
        NullableClassExpression::DictionaryGet { key, .. } => rvalue_class_temporary_capacity(key),
        NullableClassExpression::Null(_)
        | NullableClassExpression::SharedPayload { .. }
        | NullableClassExpression::Local { .. }
        | NullableClassExpression::Property { .. } => 0,
    }
}

pub(crate) fn bool_class_temporary_capacity(value: &BoolExpression) -> usize {
    match value {
        BoolExpression::Use { .. } => 0,
        BoolExpression::Compare { left, right, .. } => {
            value_class_temporary_capacity(left) + value_class_temporary_capacity(right)
        }
        BoolExpression::StringCompare { left, right, .. } => {
            string_class_temporary_capacity(left) + string_class_temporary_capacity(right)
        }
        BoolExpression::NullableStringCompare { left, right, .. } => {
            nullable_string_class_temporary_capacity(left)
                + nullable_string_class_temporary_capacity(right)
        }
        BoolExpression::NullableScalarIsPresent(value) => {
            nullable_scalar_class_temporary_capacity(value)
        }
        BoolExpression::NullableClassIsPresent(value) => nullable_class_temporary_capacity(value),
        BoolExpression::NullableCollectionIsPresent(value) => {
            nullable_collection_class_temporary_capacity(value)
        }
        BoolExpression::NullableSharedReferenceIsPresent(value) => {
            nullable_shared_class_temporary_capacity(value)
        }
        BoolExpression::NullableWeakReferenceIsPresent(value) => {
            nullable_weak_class_temporary_capacity(value)
        }
        BoolExpression::NullableWritableSharedReferenceIsPresent(value) => {
            nullable_writable_shared_class_temporary_capacity(value)
        }
        BoolExpression::NullableWritableWeakReferenceIsPresent(value) => {
            nullable_writable_weak_class_temporary_capacity(value)
        }
        BoolExpression::NullableSharedReferenceAccessIsPresent(value) => {
            nullable_shared_access_class_temporary_capacity(value)
        }
        BoolExpression::NullableMixedIsPresent(value) => {
            nullable_mixed_class_temporary_capacity(value)
        }
        BoolExpression::NullablePayloadEnumIsPresent(value) => {
            nullable_payload_enum_class_temporary_capacity(value)
        }
        BoolExpression::PayloadEnumCompare { left, right, .. } => {
            payload_enum_class_temporary_capacity(left)
                + payload_enum_class_temporary_capacity(right)
        }
        BoolExpression::NullablePayloadEnumCompare { left, right, .. } => {
            nullable_payload_enum_class_temporary_capacity(left)
                + nullable_payload_enum_class_temporary_capacity(right)
        }
        BoolExpression::PayloadEnumIsCase { .. } => 0,
        BoolExpression::Not(value) => bool_class_temporary_capacity(value),
        BoolExpression::Binary { left, right, .. } => {
            bool_class_temporary_capacity(left) + bool_class_temporary_capacity(right)
        }
        BoolExpression::Call { args, .. } => args.iter().map(rvalue_class_temporary_capacity).sum(),
        BoolExpression::Coalesce { left, right } => {
            nullable_scalar_class_temporary_capacity(left) + bool_class_temporary_capacity(right)
        }
        BoolExpression::CollectionHas { value, .. } => rvalue_class_temporary_capacity(value),
        BoolExpression::CollectionIsEmpty { .. } => 0,
        BoolExpression::CollectionEqual { .. } => 0,
        BoolExpression::MixedIs { mixed, .. } => mixed_class_temporary_capacity(mixed),
    }
}

fn format_class_temporary_capacity(format: &FormatExpression) -> usize {
    format
        .arguments
        .iter()
        .map(|argument| match argument {
            FormatArgument::Value(value) => value_class_temporary_capacity(value),
            FormatArgument::String(value) | FormatArgument::ClassDisplay(value) => {
                string_class_temporary_capacity(value)
            }
        })
        .sum()
}

impl fmt::Display for Program {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, function) in self.functions.iter().enumerate() {
            if index > 0 {
                writeln!(formatter)?;
            }
            write!(formatter, "{function}")?;
        }
        Ok(())
    }
}

impl fmt::Display for Function {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "function {}(", self.name)?;
        for (index, parameter) in self.params.iter().enumerate() {
            if index > 0 {
                write!(formatter, ", ")?;
            }
            if let Some(local) = self
                .locals
                .get(parameter.0)
                .filter(|local| local.id == *parameter)
            {
                write!(formatter, "${}: {}", local.name, local.ty)?;
            } else {
                write!(formatter, "local{}", parameter.0)?;
            }
        }
        writeln!(formatter, "): {} {{", self.return_type)?;
        if !self.locals.is_empty() {
            writeln!(formatter, "locals:")?;
            for local in &self.locals {
                writeln!(formatter, "    {local}")?;
            }
        }
        for block in &self.blocks {
            write!(formatter, "{block}")?;
        }
        writeln!(formatter, "}}")
    }
}

impl fmt::Display for ReturnType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReturnType::Value(ty) => write!(formatter, "{ty}"),
            ReturnType::Void => write!(formatter, "void"),
        }
    }
}

impl fmt::Display for Local {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let role = if self.synthetic {
            "temp"
        } else if self.writable {
            "writable"
        } else {
            "readonly"
        };
        let name = if self.synthetic {
            self.name.clone()
        } else {
            format!("${}", self.name)
        };
        write!(
            formatter,
            "local{} {} {}: {}",
            self.id.0, role, name, self.ty
        )
    }
}

impl fmt::Display for Type {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Scalar(ty) => write!(formatter, "{ty}"),
            Type::String => write!(formatter, "string"),
            Type::Mixed => write!(formatter, "mixed"),
            Type::NullableScalar(ty) => write!(formatter, "?{ty}"),
            Type::NullableString => write!(formatter, "?string"),
            Type::NullableMixed => write!(formatter, "?mixed"),
            Type::Class(class) => write!(formatter, "class#{}", class.0),
            Type::NullableClass(class) => write!(formatter, "?class#{}", class.0),
            Type::SharedReference(class) => write!(formatter, "shared<class#{}>", class.0),
            Type::WeakReference(class) => write!(formatter, "weak<class#{}>", class.0),
            Type::NullableSharedReference(class) => {
                write!(formatter, "?shared<class#{}>", class.0)
            }
            Type::NullableWeakReference(class) => {
                write!(formatter, "?weak<class#{}>", class.0)
            }
            Type::WritableSharedReference(payload) => {
                write!(formatter, "writable-shared<{payload}>")
            }
            Type::WritableWeakReference(payload) => {
                write!(formatter, "writable-weak<{payload}>")
            }
            Type::NullableWritableSharedReference(payload) => {
                write!(formatter, "?writable-shared<{payload}>")
            }
            Type::NullableWritableWeakReference(payload) => {
                write!(formatter, "?writable-weak<{payload}>")
            }
            Type::ReadonlySharedReferenceAccess(payload) => {
                write!(formatter, "readonly-shared-access<{payload}>")
            }
            Type::WritableSharedReferenceAccess(payload) => {
                write!(formatter, "writable-shared-access<{payload}>")
            }
            Type::NullableReadonlySharedReferenceAccess(payload) => {
                write!(formatter, "?readonly-shared-access<{payload}>")
            }
            Type::NullableWritableSharedReferenceAccess(payload) => {
                write!(formatter, "?writable-shared-access<{payload}>")
            }
            Type::Collection(collection) => write!(formatter, "collection#{}", collection.0),
            Type::NullableCollection(collection) => {
                write!(formatter, "?collection#{}", collection.0)
            }
            Type::PayloadEnum(ty) => write!(formatter, "payload-enum#{}", ty.id.0),
            Type::NullablePayloadEnum(ty) => {
                write!(formatter, "?payload-enum#{}", ty.id.0)
            }
        }
    }
}

impl fmt::Display for WritableSharedPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Class(class) => write!(formatter, "class#{}", class.0),
            Self::Collection(collection) => write!(formatter, "collection#{}", collection.0),
        }
    }
}

impl fmt::Display for BasicBlock {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(formatter, "block{}:", self.id.0)?;
        for statement in &self.statements {
            writeln!(formatter, "    {statement}")?;
        }
        writeln!(formatter, "    {}", self.terminator)
    }
}

impl fmt::Display for Operand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Operand::Scalar(value) => write!(formatter, "{value}"),
            Operand::Local(id) => write!(formatter, "local{}", id.0),
            Operand::NullablePayload(id) => write!(formatter, "payload(local{})", id.0),
            Operand::Static(id) => write!(formatter, "static{}", id.0),
            Operand::Property { object, property } => {
                write!(
                    formatter,
                    "local{}->property#{}:{}",
                    object.0, property.class.0, property.index
                )
            }
            Operand::CollectionLength(local) => {
                write!(formatter, "length(local{})", local.0)
            }
            Operand::CollectionIndex {
                collection, index, ..
            } => {
                write!(formatter, "local{}[{index}]", collection.0)
            }
            Operand::CollectionKeyAt { collection, offset } => {
                write!(formatter, "key_at(local{}, {offset})", collection.0)
            }
            Operand::MixedPayload { mixed, tag } => {
                write!(formatter, "mixed_payload<{tag}>(local{})", mixed.0)
            }
            Operand::StringIntrinsic(call) => write!(formatter, "{call}"),
        }
    }
}

impl fmt::Display for Rvalue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Rvalue::Value(expression) => write!(formatter, "{expression}"),
            Rvalue::String(value) => write!(formatter, "{value}"),
            Rvalue::NullableScalar(value) => write!(formatter, "{value}"),
            Rvalue::NullableString(value) => write!(formatter, "{value}"),
            Rvalue::Mixed(value) => write!(formatter, "{value}"),
            Rvalue::NullableMixed(value) => write!(formatter, "{value}"),
            Rvalue::Class(value) => write!(formatter, "{value}"),
            Rvalue::NullableClass(value) => write!(formatter, "{value}"),
            Rvalue::SharedReference(value) => write!(formatter, "{value}"),
            Rvalue::WeakReference(value) => write!(formatter, "{value}"),
            Rvalue::NullableSharedReference(value) => write!(formatter, "{value}"),
            Rvalue::NullableWeakReference(value) => write!(formatter, "{value}"),
            Rvalue::WritableSharedReference(value) => {
                write!(formatter, "writable_shared<{:?}>", value.payload())
            }
            Rvalue::WritableWeakReference(value) => {
                write!(formatter, "writable_weak<{:?}>", value.payload())
            }
            Rvalue::NullableWritableSharedReference(value) => {
                write!(formatter, "?writable_shared<{:?}>", value.payload())
            }
            Rvalue::NullableWritableWeakReference(value) => {
                write!(formatter, "?writable_weak<{:?}>", value.payload())
            }
            Rvalue::SharedReferenceAccess(value) => {
                write!(formatter, "shared_access<{:?}>", value.payload())
            }
            Rvalue::NullableSharedReferenceAccess(value) => {
                write!(formatter, "?shared_access<{:?}>", value.payload())
            }
            Rvalue::Collection(value) => write!(formatter, "{value}"),
            Rvalue::NullableCollection(value) => {
                write!(formatter, "?collection#{}", value.collection().0)
            }
            Rvalue::PayloadEnum(value) => write!(formatter, "payload-enum#{}", value.ty().id.0),
            Rvalue::NullablePayloadEnum(value) => {
                write!(formatter, "?payload-enum#{}", value.ty().id.0)
            }
        }
    }
}

impl fmt::Display for CollectionExpression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local {
                local,
                transfer: true,
                ..
            } => write!(formatter, "move local{}", local.0),
            Self::Local {
                local,
                transfer: false,
                ..
            } => write!(formatter, "borrow local{}", local.0),
            Self::Literal {
                collection,
                entries,
            } => {
                write!(formatter, "collection#{}[", collection.0)?;
                for (index, entry) in entries.iter().enumerate() {
                    if index != 0 {
                        formatter.write_str(", ")?;
                    }
                    if let Some(key) = &entry.key {
                        write!(formatter, "{key} => ")?;
                    }
                    write!(formatter, "{}", entry.value)?;
                }
                formatter.write_str("]")
            }
            Self::Fill {
                collection,
                value,
                count,
                ..
            } => write!(formatter, "collection#{}[{value}; {count}]", collection.0),
            Self::Index {
                source,
                index,
                transfer,
                ..
            } => write!(
                formatter,
                "{} local{}[{index}]",
                if *transfer { "move" } else { "borrow" },
                source.0
            ),
            Self::Property {
                object, property, ..
            } => write!(
                formatter,
                "borrow local{}->property{}",
                object.0, property.index
            ),
            Self::SharedAccessPayload {
                collection,
                access,
                writable,
            } => write!(
                formatter,
                "{}_access_payload(local{}): collection#{}",
                if *writable { "writable" } else { "readonly" },
                access.0,
                collection.0
            ),
            Self::From {
                source, transfer, ..
            } => write!(
                formatter,
                "Set::from({} local{})",
                if *transfer { "move" } else { "borrow" },
                source.0
            ),
            Self::FromBytes { source, .. } => {
                write!(formatter, "local{}->toArray()", source.0)
            }
            Self::BytesFromArray { source, .. } => {
                write!(formatter, "Bytes::fromArray(local{})", source.0)
            }
            Self::ReadFileBytes { path, .. } => write!(formatter, "read_file_bytes({path})"),
            Self::ReadStdinBytes { .. } => formatter.write_str("read_stdin_bytes()"),
            Self::StringIntrinsic(call) => write!(formatter, "{call}"),
            Self::Call { function, args, .. } => write_call(formatter, *function, args),
        }
    }
}

impl fmt::Display for ClassExpression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local {
                local,
                transfer: true,
                ..
            } => write!(formatter, "move local{}", local.0),
            Self::Local {
                local,
                transfer: false,
                ..
            } => write!(formatter, "borrow local{}", local.0),
            Self::Property {
                object, property, ..
            } => write!(
                formatter,
                "borrow local{}->property#{}:{}",
                object.0, property.class.0, property.index
            ),
            Self::Call {
                class, function, ..
            } => {
                write!(formatter, "call fn{} -> class#{}", function.0, class.0)
            }
            Self::New { class, .. } => write!(formatter, "new class#{}", class.0),
            Self::NullableLocalAssumeNonNull {
                class,
                local,
                transfer,
            } => write!(
                formatter,
                "{} nonnull(local{}): class#{}",
                if *transfer { "move" } else { "borrow" },
                local.0,
                class.0
            ),
            Self::Coalesce { left, right, .. } => write!(formatter, "({left} ?? {right})"),
            Self::CollectionIndex {
                positional: _,
                class,
                collection,
                index,
                transfer,
            } => write!(
                formatter,
                "{} local{}[{index}]: class#{}",
                if *transfer { "move" } else { "borrow" },
                collection.0,
                class.0
            ),
            Self::MixedPayload {
                class,
                mixed,
                transfer,
            } => write!(
                formatter,
                "{} mixed_payload<class#{}>(local{})",
                if *transfer { "move" } else { "borrow" },
                class.0,
                mixed.0
            ),
            Self::SharedPayload { class, reference } => {
                write!(formatter, "payload({reference}): class#{}", class.0)
            }
            Self::SharedAccessPayload {
                class,
                access,
                writable,
            } => write!(
                formatter,
                "{}_access_payload(local{}): class#{}",
                if *writable { "writable" } else { "readonly" },
                access.0,
                class.0
            ),
        }
    }
}

impl fmt::Display for SharedReferenceExpression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::New { class, .. } => write!(formatter, "shared new class#{}", class.0),
            Self::Local {
                local, transfer, ..
            } => write!(
                formatter,
                "{} shared local{}",
                if *transfer { "move" } else { "borrow" },
                local.0
            ),
            Self::NullableLocalAssumeNonNull {
                local, transfer, ..
            } => write!(
                formatter,
                "{} present nullable shared local{}",
                if *transfer { "move" } else { "borrow" },
                local.0
            ),
            Self::Property {
                object, property, ..
            } => write!(
                formatter,
                "borrow local{}->property{}",
                object.0, property.index
            ),
            Self::Call { function, args, .. } => write_call(formatter, *function, args),
            Self::Share { value, .. } => write!(formatter, "share({value})"),
            Self::Coalesce { left, right, .. } => write!(formatter, "({left} ?? {right})"),
            Self::CollectionIndex {
                collection, index, ..
            } => write!(formatter, "local{}[{index}]", collection.0),
        }
    }
}

impl fmt::Display for WeakReferenceExpression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local {
                local, transfer, ..
            } => write!(
                formatter,
                "{} weak local{}",
                if *transfer { "move" } else { "borrow" },
                local.0
            ),
            Self::NullableLocalAssumeNonNull {
                local, transfer, ..
            } => write!(
                formatter,
                "{} present nullable weak local{}",
                if *transfer { "move" } else { "borrow" },
                local.0
            ),
            Self::Property {
                object, property, ..
            } => write!(
                formatter,
                "borrow local{}->property{}",
                object.0, property.index
            ),
            Self::Call { function, args, .. } => write_call(formatter, *function, args),
            Self::Create { value, .. } => write!(formatter, "weak({value})"),
            Self::Coalesce { left, right, .. } => write!(formatter, "({left} ?? {right})"),
            Self::CollectionIndex {
                collection, index, ..
            } => write!(formatter, "local{}[{index}]", collection.0),
        }
    }
}

impl fmt::Display for NullableSharedReferenceExpression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null(class) => write!(formatter, "null: ?shared<class#{}>", class.0),
            Self::Shared(value) => write!(formatter, "some({value})"),
            Self::Local { local, .. } => write!(formatter, "local{}", local.0),
            Self::Property {
                object, property, ..
            } => write!(formatter, "local{}->property{}", object.0, property.index),
            Self::Call { function, args, .. } => write_call(formatter, *function, args),
            Self::Acquire { value, .. } => write!(formatter, "acquire({value})"),
            Self::NullSafeShare { value, .. } => write!(formatter, "null_safe_share({value})"),
            Self::NullSafeAcquire { value, .. } => {
                write!(formatter, "null_safe_acquire({value})")
            }
            Self::Coalesce { left, right, .. } => write!(formatter, "({left} ?? {right})"),
            Self::DictionaryGet {
                collection, key, ..
            } => write!(formatter, "local{}.get({key})", collection.0),
            Self::CollectionIndex {
                collection, index, ..
            } => write!(formatter, "local{}[{index}]", collection.0),
        }
    }
}

impl fmt::Display for NullableWeakReferenceExpression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null(class) => write!(formatter, "null: ?weak<class#{}>", class.0),
            Self::Weak(value) => write!(formatter, "some({value})"),
            Self::Local { local, .. } => write!(formatter, "local{}", local.0),
            Self::Property {
                object, property, ..
            } => write!(formatter, "local{}->property{}", object.0, property.index),
            Self::Call { function, args, .. } => write_call(formatter, *function, args),
            Self::NullSafeCreate { value, .. } => {
                write!(formatter, "null_safe_create_weak({value})")
            }
            Self::Coalesce { left, right, .. } => write!(formatter, "({left} ?? {right})"),
            Self::DictionaryGet {
                collection, key, ..
            } => write!(formatter, "local{}.get({key})", collection.0),
            Self::CollectionIndex {
                collection, index, ..
            } => write!(formatter, "local{}[{index}]", collection.0),
        }
    }
}

impl fmt::Display for MixedTag {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bool => formatter.write_str("bool"),
            Self::Enum(enum_id) => write!(formatter, "enum#{}", enum_id.0),
            Self::Integer(ty) => write!(formatter, "{ty}"),
            Self::Float(ty) => write!(formatter, "{ty}"),
            Self::String => formatter.write_str("string"),
            Self::Class(class) => write!(formatter, "class#{}", class.0),
            Self::PayloadEnum(ty) => write!(formatter, "payload-enum#{}", ty.id.0),
        }
    }
}

impl fmt::Display for MixedExpression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local {
                local,
                transfer: true,
            } => write!(formatter, "move mixed local{}", local.0),
            Self::Local {
                local,
                transfer: false,
            } => write!(formatter, "borrow mixed local{}", local.0),
            Self::Property { object, property } => write!(
                formatter,
                "borrow local{}->property#{}:{}",
                object.0, property.class.0, property.index
            ),
            Self::Call { function, args, .. } => write_call(formatter, *function, args),
            Self::BoxValue(value) => write!(formatter, "mixed({value})"),
            Self::BoxString { value, .. } => write!(formatter, "mixed({value})"),
            Self::BoxClass { value, .. } => write!(formatter, "mixed({value})"),
            Self::BoxPayloadEnum { value } => {
                write!(formatter, "mixed(payload-enum#{})", value.ty().id.0)
            }
            Self::CollectionIndex {
                positional: _,
                collection,
                index,
                transfer,
                remove,
            } => write!(
                formatter,
                "{} mixed local{}[{index}]",
                if *remove {
                    "remove"
                } else if *transfer {
                    "move"
                } else {
                    "borrow"
                },
                collection.0
            ),
        }
    }
}

impl fmt::Display for NullableMixedExpression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => formatter.write_str("null"),
            Self::Mixed(value) => write!(formatter, "some({value})"),
            Self::BoxNullablePayloadEnum(value) => {
                write!(formatter, "mixed(?payload-enum#{})", value.ty().id.0)
            }
            Self::Local {
                local,
                transfer: true,
            } => write!(formatter, "move ?mixed local{}", local.0),
            Self::Local {
                local,
                transfer: false,
            } => write!(formatter, "borrow ?mixed local{}", local.0),
            Self::Property { object, property } => write!(
                formatter,
                "borrow local{}->property#{}:{}",
                object.0, property.class.0, property.index
            ),
            Self::Call { function, args, .. } => write_call(formatter, *function, args),
            Self::Coalesce { left, right, .. } => write!(formatter, "({left} ?? {right})"),
        }
    }
}

impl fmt::Display for ScalarType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Integer(ty) => write!(formatter, "{ty}"),
            Self::Float(ty) => write!(formatter, "{ty}"),
            Self::Bool => formatter.write_str("bool"),
            Self::Enum(enum_id) => write!(formatter, "enum#{}", enum_id.0),
        }
    }
}

impl fmt::Display for ScalarValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Integer(value) => write!(formatter, "{value}: {}", value.ty),
            Self::Float(value) => match value.ty {
                FloatType::Float32 => write!(formatter, "0x{:08x}: float32", value.bits),
                FloatType::Float64 => write!(formatter, "0x{:016x}: float", value.bits),
            },
            Self::Bool(value) => write!(formatter, "{value}: bool"),
            Self::Enum(value) => write!(
                formatter,
                "enum#{}::case{}",
                value.enum_id.0, value.case_id.index
            ),
        }
    }
}

impl fmt::Display for ValueExpression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Integer(value) => write!(formatter, "{value}"),
            Self::Float(value) => write!(formatter, "{value}"),
            Self::Bool(value) => write!(formatter, "{value}"),
            Self::Enum(value) => write!(formatter, "{value}"),
        }
    }
}

impl fmt::Display for EnumExpression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Use { enum_id, operand } => write!(formatter, "{operand:?}: enum#{}", enum_id.0),
            Self::Case(value) => write!(
                formatter,
                "enum#{}::case{}",
                value.enum_id.0, value.case_id.index
            ),
            Self::Call {
                enum_id,
                function,
                args,
            } => {
                write_call(formatter, *function, args)?;
                write!(formatter, ": enum#{}", enum_id.0)
            }
            Self::Coalesce {
                enum_id,
                left,
                right,
            } => write!(formatter, "({left} ?? {right}): enum#{}", enum_id.0),
        }
    }
}

impl fmt::Display for IntegerUnaryOp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IntegerUnaryOp::Negate => write!(formatter, "-"),
            IntegerUnaryOp::BitwiseNot => write!(formatter, "~"),
        }
    }
}

impl fmt::Display for IntegerBinaryOp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IntegerBinaryOp::Add => write!(formatter, "+"),
            IntegerBinaryOp::Subtract => write!(formatter, "-"),
            IntegerBinaryOp::Multiply => write!(formatter, "*"),
            IntegerBinaryOp::Divide => write!(formatter, "/"),
            IntegerBinaryOp::Remainder => write!(formatter, "%"),
            IntegerBinaryOp::ShiftLeft => write!(formatter, "<<"),
            IntegerBinaryOp::ShiftRight => write!(formatter, ">>"),
            IntegerBinaryOp::BitwiseAnd => write!(formatter, "&"),
            IntegerBinaryOp::BitwiseXor => write!(formatter, "^"),
            IntegerBinaryOp::BitwiseOr => write!(formatter, "|"),
        }
    }
}

impl fmt::Display for IntegerExpression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IntegerExpression::Use { ty, operand } => match operand {
                Operand::Scalar(ScalarValue::Integer(value)) => write!(formatter, "{value}: {ty}"),
                Operand::Local(id) => write!(formatter, "local{}: {ty}", id.0),
                Operand::NullablePayload(id) => {
                    write!(formatter, "payload(local{}): {ty}", id.0)
                }
                Operand::Static(id) => write!(formatter, "static{}: {ty}", id.0),
                Operand::Property { object, property } => write!(
                    formatter,
                    "local{}->property{}: {ty}",
                    object.0, property.index
                ),
                Operand::Scalar(_) => write!(formatter, "<malformed scalar>: {ty}"),
                Operand::CollectionLength(local) => {
                    write!(formatter, "length(local{}): {ty}", local.0)
                }
                Operand::CollectionIndex {
                    collection, index, ..
                } => {
                    write!(formatter, "local{}[{index}]: {ty}", collection.0)
                }
                Operand::CollectionKeyAt { collection, offset } => {
                    write!(formatter, "key_at(local{}, {offset}): {ty}", collection.0)
                }
                Operand::MixedPayload { mixed, tag } => {
                    write!(formatter, "mixed_payload<{tag}>(local{}): {ty}", mixed.0)
                }
                Operand::StringIntrinsic(call) => write!(formatter, "{call}: {ty}"),
            },
            IntegerExpression::Unary {
                ty, op, operand, ..
            } => {
                write!(formatter, "({op}{operand}): {ty}")
            }
            IntegerExpression::Binary {
                ty,
                op,
                left,
                right,
                ..
            } => write!(formatter, "({left} {op} {right}): {ty}"),
            IntegerExpression::Convert { ty, value, .. } => {
                write!(formatter, "convert<{ty}>({value}): {ty}")
            }
            IntegerExpression::FloatToInt { value, .. } => {
                write!(formatter, "Float::toInt({value}): int")
            }
            IntegerExpression::Call { ty, function, args } => {
                write_call(formatter, *function, args)?;
                write!(formatter, ": {ty}")
            }
            IntegerExpression::Coalesce { ty, left, right } => {
                write!(formatter, "({left} ?? {right}): {ty}")
            }
            IntegerExpression::EnumBacking { enum_id, value } => {
                write!(formatter, "enum_backing<{:?}>({value})", enum_id)
            }
        }
    }
}

impl fmt::Display for FloatBinaryOp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Add => "+",
            Self::Subtract => "-",
            Self::Multiply => "*",
            Self::Divide => "/",
        })
    }
}

impl fmt::Display for FloatExpression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Use { ty, operand } => match operand {
                Operand::Scalar(ScalarValue::Float(value)) => write!(formatter, "{value}: {ty}"),
                Operand::Local(id) => write!(formatter, "local{}: {ty}", id.0),
                Operand::NullablePayload(id) => {
                    write!(formatter, "payload(local{}): {ty}", id.0)
                }
                Operand::Static(id) => write!(formatter, "static{}: {ty}", id.0),
                Operand::Property { object, property } => write!(
                    formatter,
                    "local{}->property{}: {ty}",
                    object.0, property.index
                ),
                Operand::Scalar(_) => write!(formatter, "<malformed scalar>: {ty}"),
                Operand::CollectionLength(local) => {
                    write!(formatter, "length(local{}): {ty}", local.0)
                }
                Operand::CollectionIndex {
                    collection, index, ..
                } => {
                    write!(formatter, "local{}[{index}]: {ty}", collection.0)
                }
                Operand::CollectionKeyAt { collection, offset } => {
                    write!(formatter, "key_at(local{}, {offset}): {ty}", collection.0)
                }
                Operand::MixedPayload { mixed, tag } => {
                    write!(formatter, "mixed_payload<{tag}>(local{}): {ty}", mixed.0)
                }
                Operand::StringIntrinsic(call) => write!(formatter, "{call}: {ty}"),
            },
            Self::Negate { ty, operand } => write!(formatter, "(-{operand}): {ty}"),
            Self::Binary {
                ty,
                op,
                left,
                right,
            } => write!(formatter, "({left} {op} {right}): {ty}"),
            Self::IntToFloat { value } => write!(formatter, "Int::toFloat({value}): float"),
            Self::Call { ty, function, args } => {
                write_call(formatter, *function, args)?;
                write!(formatter, ": {ty}")
            }
            Self::Coalesce { ty, left, right } => {
                write!(formatter, "({left} ?? {right}): {ty}")
            }
        }
    }
}

impl fmt::Display for StringExpression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StringExpression::Literal(value) => {
                write!(formatter, "\"{}\"", escape_debug_string(value))
            }
            StringExpression::Local(id) => write!(formatter, "local{}", id.0),
            StringExpression::NullableLocalAssumeNonNull(id) => {
                write!(formatter, "nonnull(local{})", id.0)
            }
            StringExpression::Property { object, property } => {
                write!(formatter, "local{}->property{}", object.0, property.index)
            }
            StringExpression::Static(id) => write!(formatter, "static{}", id.0),
            StringExpression::MixedPayload(local) => {
                write!(formatter, "mixed_payload<string>(local{})", local.0)
            }
            StringExpression::Concat(parts) => {
                write!(formatter, "(")?;
                for (index, part) in parts.iter().enumerate() {
                    if index > 0 {
                        write!(formatter, " . ")?;
                    }
                    write!(formatter, "{part}")?;
                }
                write!(formatter, ")")
            }
            StringExpression::Display(value) => write!(formatter, "display({value})"),
            StringExpression::Call { function, args } => write_call(formatter, *function, args),
            StringExpression::ReadFile { path, .. } => write!(formatter, "read_file({path})"),
            StringExpression::Format(format) => write!(formatter, "format({format})"),
            StringExpression::Coalesce { left, right } => {
                write!(formatter, "({left} ?? {right})")
            }
            StringExpression::CollectionIndex {
                collection, index, ..
            } => {
                write!(formatter, "local{}[{index}]", collection.0)
            }
            StringExpression::CollectionKeyAt { collection, offset } => {
                write!(formatter, "key_at(local{}, {offset})", collection.0)
            }
            StringExpression::Intrinsic(call) => write!(formatter, "{call}"),
            StringExpression::EnumBacking { enum_id, value } => {
                write!(formatter, "enum_backing<{:?}>({value})", enum_id)
            }
        }
    }
}

impl fmt::Display for NullableStringExpression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => formatter.write_str("null"),
            Self::String(value) => write!(formatter, "some({value})"),
            Self::Local(local) => write!(formatter, "local{}", local.0),
            Self::Property { object, property } => {
                write!(formatter, "local{}->property{}", object.0, property.index)
            }
            Self::Static(id) => write!(formatter, "static{}", id.0),
            Self::ReadLine { prompt, .. } => write!(formatter, "read_line({prompt})"),
            Self::Call { function, args } => write_call(formatter, *function, args),
            Self::NullSafeProperty { object, property } => {
                write!(formatter, "{object}?->property{}", property.index)
            }
            Self::NullSafeCall {
                object,
                function,
                args,
            } => {
                write!(formatter, "{object}?->")?;
                write_call(formatter, *function, args)
            }
            Self::EnumBacking { enum_id, value } => {
                write!(formatter, "nullable_enum_backing<{enum_id:?}>({value})")
            }
            Self::Coalesce { left, right } => write!(formatter, "({left} ?? {right})"),
            Self::DictionaryGet {
                collection, key, ..
            } => {
                write!(formatter, "local{}.get({key})", collection.0)
            }
            Self::Intrinsic(call) => write!(formatter, "{call}"),
        }
    }
}

impl fmt::Display for FormatArgument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Value(value) => write!(formatter, "{value}"),
            Self::String(value) => write!(formatter, "{value}"),
            Self::ClassDisplay(value) => write!(formatter, "class-display({value})"),
        }
    }
}

impl fmt::Display for FormatExpression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "plan[{} pieces]", self.pieces.len())?;
        write!(formatter, "(")?;
        for (index, argument) in self.arguments.iter().enumerate() {
            if index != 0 {
                write!(formatter, ", ")?;
            }
            write!(formatter, "{argument}")?;
        }
        write!(formatter, ")")
    }
}

impl fmt::Display for BoolExpression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Use { operand } => match operand {
                Operand::Scalar(ScalarValue::Bool(value)) => write!(formatter, "{value}: bool"),
                Operand::Local(id) => write!(formatter, "local{}: bool", id.0),
                Operand::NullablePayload(id) => {
                    write!(formatter, "payload(local{}): bool", id.0)
                }
                Operand::Static(id) => write!(formatter, "static{}: bool", id.0),
                Operand::Property { object, property } => {
                    write!(
                        formatter,
                        "local{}->property{}: bool",
                        object.0, property.index
                    )
                }
                Operand::Scalar(_) => formatter.write_str("<malformed scalar>: bool"),
                Operand::CollectionLength(local) => {
                    write!(formatter, "length(local{}): bool", local.0)
                }
                Operand::CollectionIndex {
                    collection, index, ..
                } => {
                    write!(formatter, "local{}[{index}]: bool", collection.0)
                }
                Operand::CollectionKeyAt { collection, offset } => {
                    write!(formatter, "key_at(local{}, {offset}): bool", collection.0)
                }
                Operand::MixedPayload { mixed, tag } => {
                    write!(formatter, "mixed_payload<{tag}>(local{}): bool", mixed.0)
                }
                Operand::StringIntrinsic(call) => write!(formatter, "{call}: bool"),
            },
            Self::Compare { op, left, right } => write!(formatter, "{left} {op} {right}"),
            Self::StringCompare { op, left, right } => write!(formatter, "{left} {op} {right}"),
            Self::NullableStringCompare { op, left, right } => {
                write!(formatter, "{left} {op} {right}")
            }
            Self::NullableScalarIsPresent(value) => write!(formatter, "present({value})"),
            Self::NullableClassIsPresent(value) => write!(formatter, "present({value})"),
            Self::NullableCollectionIsPresent(value) => {
                write!(formatter, "present(?collection#{})", value.collection().0)
            }
            Self::NullableSharedReferenceIsPresent(value) => {
                write!(formatter, "present({value})")
            }
            Self::NullableWeakReferenceIsPresent(value) => {
                write!(formatter, "present({value})")
            }
            Self::NullableWritableSharedReferenceIsPresent(value) => {
                write!(formatter, "present(?writable-shared<{}>)", value.payload())
            }
            Self::NullableWritableWeakReferenceIsPresent(value) => {
                write!(formatter, "present(?writable-weak<{}>)", value.payload())
            }
            Self::NullableSharedReferenceAccessIsPresent(value) => {
                write!(formatter, "present(?shared-access<{}>)", value.payload())
            }
            Self::NullableMixedIsPresent(value) => write!(formatter, "present({value})"),
            Self::NullablePayloadEnumIsPresent(value) => {
                write!(formatter, "present(?payload-enum#{})", value.ty().id.0)
            }
            Self::PayloadEnumCompare { op, left, right } => {
                write!(formatter, "{left:?} {op} {right:?}")
            }
            Self::PayloadEnumIsCase {
                local,
                ty,
                case,
                nullable,
            } => write!(
                formatter,
                "{}payload-enum#{} local{} is case{}",
                if *nullable { "nullable " } else { "" },
                ty.id.0,
                local.0,
                case.index
            ),
            Self::NullablePayloadEnumCompare { op, left, right } => {
                write!(formatter, "{left:?} {op} {right:?}")
            }
            Self::MixedIs { mixed, tag } => write!(formatter, "{mixed} is {tag}"),
            Self::Not(condition) => write!(formatter, "!({condition})"),
            Self::Binary { op, left, right } => {
                write!(formatter, "({left}) {op} ({right})")
            }
            Self::Call { function, args } => write_call(formatter, *function, args),
            Self::Coalesce { left, right } => write!(formatter, "({left} ?? {right})"),
            Self::CollectionHas {
                collection, value, ..
            } => {
                write!(formatter, "local{}.has({value})", collection.0)
            }
            Self::CollectionIsEmpty { collection } => {
                write!(formatter, "local{}.isEmpty", collection.0)
            }
            Self::CollectionEqual { left, right } => {
                write!(formatter, "local{} == local{}", left.0, right.0)
            }
        }
    }
}

impl fmt::Display for NullableScalarExpression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null(ty) => write!(formatter, "null: ?{ty}"),
            Self::Value(value) => write!(formatter, "some({value})"),
            Self::Local { local, .. } => write!(formatter, "local{}", local.0),
            Self::Property {
                object, property, ..
            } => {
                write!(formatter, "local{}->property{}", object.0, property.index)
            }
            Self::Static { id, .. } => write!(formatter, "static{}", id.0),
            Self::Call { function, args, .. } => write_call(formatter, *function, args),
            Self::EnumBacking { enum_id, value } => {
                write!(formatter, "nullable_enum_backing<{enum_id:?}>({value})")
            }
            Self::NullSafeProperty {
                object, property, ..
            } => {
                write!(formatter, "{object}?->property{}", property.index)
            }
            Self::NullSafeCall {
                object,
                function,
                args,
                ..
            } => {
                write!(formatter, "{object}?->")?;
                write_call(formatter, *function, args)
            }
            Self::Coalesce { left, right, .. } => write!(formatter, "({left} ?? {right})"),
            Self::DictionaryGet {
                collection, key, ..
            } => {
                write!(formatter, "local{}.get({key})", collection.0)
            }
            Self::CollectionIndexOf { collection, value } => {
                write!(formatter, "local{}.indexOf({value})", collection.0)
            }
            Self::Parse { ty, value } => write!(formatter, "parse::<{ty}>({value})"),
            Self::StringIntrinsic(call) => write!(formatter, "{call}"),
        }
    }
}

impl fmt::Display for StringIntrinsicCall {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "String::{}(", self.kind)?;
        for (index, argument) in self.args.iter().enumerate() {
            if index > 0 {
                formatter.write_str(", ")?;
            }
            write!(formatter, "{argument}")?;
        }
        write!(formatter, "): {}", self.result)
    }
}

impl fmt::Display for StringIntrinsicKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::GraphemeLength => "graphemeLength",
            Self::ByteLength => "byteLength",
            Self::IsEmpty => "isEmpty",
            Self::ToBytes => "toBytes",
            Self::Trim => "trim",
            Self::TrimStart => "trimStart",
            Self::TrimEnd => "trimEnd",
            Self::Lower => "lower",
            Self::Upper => "upper",
            Self::LowerFirst => "lowerFirst",
            Self::UpperFirst => "upperFirst",
            Self::Contains => "contains",
            Self::StartsWith => "startsWith",
            Self::EndsWith => "endsWith",
            Self::ContainsIgnoreCase => "containsIgnoreCase",
            Self::StartsWithIgnoreCase => "startsWithIgnoreCase",
            Self::EndsWithIgnoreCase => "endsWithIgnoreCase",
            Self::EqualsIgnoreCase => "equalsIgnoreCase",
            Self::IndexOf => "indexOf",
            Self::LastIndexOf => "lastIndexOf",
            Self::IndexOfIgnoreCase => "indexOfIgnoreCase",
            Self::LastIndexOfIgnoreCase => "lastIndexOfIgnoreCase",
            Self::CountOccurrences => "countOccurrences",
            Self::Replace => "replace",
            Self::Split => "split",
            Self::Join => "join",
            Self::Slice => "slice",
            Self::Repeat => "repeat",
            Self::PadStart => "padStart",
            Self::PadEnd => "padEnd",
            Self::FromBytes => "fromBytes",
        })
    }
}

impl fmt::Display for NullableClassExpression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null(class) => write!(formatter, "null: ?class#{}", class.0),
            Self::Class(value) => write!(formatter, "some({value})"),
            Self::SharedPayload { reference, .. } => {
                write!(formatter, "shared_payload({reference})")
            }
            Self::Local { local, .. } => write!(formatter, "local{}", local.0),
            Self::Property {
                object, property, ..
            } => {
                write!(formatter, "local{}->property{}", object.0, property.index)
            }
            Self::Call { function, args, .. } => write_call(formatter, *function, args),
            Self::NullSafeProperty {
                object, property, ..
            } => {
                write!(formatter, "{object}?->property{}", property.index)
            }
            Self::NullSafeCall {
                object,
                function,
                args,
                ..
            } => {
                write!(formatter, "{object}?->")?;
                write_call(formatter, *function, args)
            }
            Self::Coalesce { left, right, .. } => write!(formatter, "({left} ?? {right})"),
            Self::DictionaryGet {
                collection, key, ..
            } => {
                write!(formatter, "local{}.get({key})", collection.0)
            }
        }
    }
}

impl fmt::Display for CompareOp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompareOp::Equal => write!(formatter, "=="),
            CompareOp::NotEqual => write!(formatter, "!="),
            CompareOp::Less => write!(formatter, "<"),
            CompareOp::LessEqual => write!(formatter, "<="),
            CompareOp::Greater => write!(formatter, ">"),
            CompareOp::GreaterEqual => write!(formatter, ">="),
        }
    }
}

impl fmt::Display for BoolBinaryOp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BoolBinaryOp::And => write!(formatter, "&&"),
            BoolBinaryOp::Or => write!(formatter, "||"),
            BoolBinaryOp::Xor => write!(formatter, "xor"),
        }
    }
}

impl fmt::Display for Statement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Statement::AssignLocal { target, value } => {
                write!(formatter, "local{} = {value}", target.0)
            }
            Statement::AssignLocalGroup { targets, value } => {
                write!(formatter, "group [")?;
                for (index, target) in targets.iter().enumerate() {
                    if index > 0 {
                        write!(formatter, ", ")?;
                    }
                    write!(formatter, "local{}", target.0)?;
                }
                write!(formatter, "] = {value}")
            }
            Statement::BindPayloadEnumFields {
                source,
                case,
                targets,
                ..
            } => {
                write!(formatter, "bind case{} local{} -> [", case.index, source.0)?;
                for (index, target) in targets.iter().enumerate() {
                    if index > 0 {
                        write!(formatter, ", ")?;
                    }
                    write!(formatter, "local{}", target.0)?;
                }
                write!(formatter, "]")
            }
            Statement::MatchResultPlan {
                mode,
                result,
                arms,
                merge,
                ..
            } => write!(
                formatter,
                "match {:?} local{} arms [{}] -> block{}",
                mode,
                result.0,
                arms.iter()
                    .map(|arm| match arm.guard {
                        Some(guard) => format!("block{}=>block{}", guard.0, arm.binding.0),
                        None => format!("block{}", arm.binding.0),
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
                merge.0
            ),
            Statement::ControlFlowPlan(plan) => write!(formatter, "{plan}"),
            Statement::EchoStringLiteral(value) => {
                write!(formatter, "echo \"{}\"", escape_debug_string(value))
            }
            Statement::EchoString(value) => write!(formatter, "echo {value}"),
            Statement::CallVoid { function, args, .. }
            | Statement::CallBorrowed { function, args, .. } => {
                write_call(formatter, *function, args)
            }
            Statement::CallNullSafe {
                object,
                function,
                args,
                ..
            } => {
                write!(formatter, "null_safe {object} -> ")?;
                write_call(formatter, *function, args)
            }
            Statement::Printf(format) => write!(formatter, "printf {format}"),
            Statement::WriteFile { path, contents } => {
                write!(formatter, "write_file({path}, {contents})")
            }
            Statement::AppendFile { path, contents } => {
                write!(formatter, "append_file({path}, {contents})")
            }
            Statement::WriteStderr(value) => write!(formatter, "write_stderr({value})"),
            Statement::WriteFileBytes {
                path,
                contents,
                append,
            } => write!(
                formatter,
                "{}({path}, local{})",
                if *append {
                    "append_file_bytes"
                } else {
                    "write_file_bytes"
                },
                contents.0
            ),
            Statement::WriteStreamBytes { contents, stderr } => write!(
                formatter,
                "{}(local{})",
                if *stderr {
                    "write_stderr_bytes"
                } else {
                    "write_stdout_bytes"
                },
                contents.0
            ),
            Statement::AssignProperty {
                object,
                property,
                value,
            } => write!(
                formatter,
                "local{}->property{} = {value}",
                object.0, property.index
            ),
            Statement::AssignStatic { target, value } => {
                write!(formatter, "static{} = {value}", target.0)
            }
            Statement::DropClass { local, class } => {
                write!(formatter, "drop class#{} local{}", class.0, local.0)
            }
            Statement::DropSharedReference { local, class } => {
                write!(formatter, "drop shared<class#{}> local{}", class.0, local.0)
            }
            Statement::DropWeakReference { local, class } => {
                write!(formatter, "drop weak<class#{}> local{}", class.0, local.0)
            }
            Statement::DropWritableSharedReference { local, payload } => {
                write!(
                    formatter,
                    "drop writable-shared<{payload}> local{}",
                    local.0
                )
            }
            Statement::DropWritableWeakReference { local, payload } => {
                write!(formatter, "drop writable-weak<{payload}> local{}", local.0)
            }
            Statement::DropSharedReferenceAccess {
                local,
                payload,
                writable,
            } => write!(
                formatter,
                "drop {}-shared-access<{payload}> local{}",
                if *writable { "writable" } else { "readonly" },
                local.0
            ),
            Statement::DropString { local } => write!(formatter, "drop string local{}", local.0),
            Statement::DropMixed { local } => write!(formatter, "drop mixed local{}", local.0),
            Statement::CollectionAdd {
                collection, value, ..
            } => {
                write!(formatter, "local{}.add({value})", collection.0)
            }
            Statement::CollectionSet {
                collection,
                key,
                value,
            } => write!(formatter, "local{}.set({key}, {value})", collection.0),
            Statement::AssignCollectionIndex {
                positional: _,
                collection,
                index,
                value,
            } => write!(formatter, "local{}[{index}] = {value}", collection.0),
            Statement::CollectionClear {
                collection,
                collection_type,
            } => write!(
                formatter,
                "clear collection#{} local{}",
                collection_type.0, collection.0
            ),
            Statement::DropCollection { local, collection } => {
                write!(
                    formatter,
                    "drop collection#{} local{}",
                    collection.0, local.0
                )
            }
            Statement::DropPayloadEnum {
                local,
                ty,
                nullable,
            } => write!(
                formatter,
                "drop {}payload-enum#{} local{}",
                if *nullable { "nullable " } else { "" },
                ty.id.0,
                local.0
            ),
        }
    }
}

impl fmt::Display for ControlFlowPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Given(plan) => write!(
                formatter,
                "given {:?} setup block{}..block{} predicates [{}] condition block{} false {}",
                plan.attachment,
                plan.setup_entry.0,
                plan.setup_exit.0,
                plan.predicates
                    .iter()
                    .map(|predicate| format!("block{}", predicate.block.0))
                    .collect::<Vec<_>>()
                    .join(", "),
                plan.condition.0,
                plan.gate_failed
                    .map(|block| format!("block{}", block.0))
                    .unwrap_or_else(|| "none".to_string())
            ),
            Self::When(plan) => write!(
                formatter,
                "when {:?} local{} branches [{}] -> block{}",
                plan.ownership,
                plan.result.0,
                plan.branches
                    .iter()
                    .map(|block| format!("block{}", block.0))
                    .collect::<Vec<_>>()
                    .join(", "),
                plan.merge.0
            ),
            Self::DoWhile(plan) => write!(
                formatter,
                "do block{} while block{} -> block{}",
                plan.body.0, plan.condition.0, plan.exit.0
            ),
            Self::PendingFinally { .. } => write!(formatter, "pending finally"),
        }
    }
}

impl fmt::Display for Terminator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Terminator::Return(operand) => write!(formatter, "return {operand}"),
            Terminator::ReturnVoid => write!(formatter, "return"),
            Terminator::Panic { message, .. } => write!(formatter, "panic {message}"),
            Terminator::Unreachable => write!(formatter, "unreachable"),
            Terminator::Jump(target) => write!(formatter, "jump block{}", target.0),
            Terminator::Branch {
                condition,
                then_block,
                else_block,
            } => write!(
                formatter,
                "branch {condition} -> block{}, block{}",
                then_block.0, else_block.0
            ),
        }
    }
}

fn escape_debug_string(value: &str) -> String {
    value.escape_default().collect()
}

fn write_call<T: fmt::Display>(
    formatter: &mut fmt::Formatter<'_>,
    function: FunctionId,
    args: &[T],
) -> fmt::Result {
    write!(formatter, "call function{}(", function.0)?;
    for (index, arg) in args.iter().enumerate() {
        if index > 0 {
            write!(formatter, ", ")?;
        }
        write!(formatter, "{arg}")?;
    }
    write!(formatter, ")")
}

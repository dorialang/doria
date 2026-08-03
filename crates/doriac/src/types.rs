use std::collections::HashMap;
use std::fmt;

pub use crate::numeric::{FloatType, IntegerType};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeRef {
    pub name: String,
    /// Generic arguments in source order. Decision 0105 reserves value
    /// arguments, so the syntax tree must preserve both their kind and their
    /// position without pretending every argument is a type.
    pub arguments: Vec<TypeArgumentRef>,
    pub nullable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeArgumentRef {
    Type(TypeRef),
    Value(String),
}

impl TypeRef {
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            arguments: Vec::new(),
            nullable: false,
        }
    }

    pub fn generic(name: impl Into<String>, args: Vec<TypeRef>) -> Self {
        Self {
            name: name.into(),
            arguments: args.into_iter().map(TypeArgumentRef::Type).collect(),
            nullable: false,
        }
    }

    pub fn generic_with_arguments(
        name: impl Into<String>,
        arguments: Vec<TypeArgumentRef>,
    ) -> Self {
        Self {
            name: name.into(),
            arguments,
            nullable: false,
        }
    }

    pub fn array_of(element: TypeRef) -> Self {
        Self::generic("[]", vec![element])
    }

    pub fn unknown() -> Self {
        Self::named("Unknown")
    }

    pub fn nullable(mut self) -> Self {
        self.nullable = true;
        self
    }

    pub fn type_arguments(&self) -> impl Iterator<Item = &TypeRef> {
        self.arguments.iter().filter_map(|argument| match argument {
            TypeArgumentRef::Type(ty) => Some(ty),
            TypeArgumentRef::Value(_) => None,
        })
    }

    pub fn type_argument(&self, index: usize) -> Option<&TypeRef> {
        self.type_arguments().nth(index)
    }

    pub fn type_argument_count(&self) -> usize {
        self.type_arguments().count()
    }

    pub fn has_value_arguments(&self) -> bool {
        self.arguments
            .iter()
            .any(|argument| matches!(argument, TypeArgumentRef::Value(_)))
    }

    pub fn resolve_self_in(&self, self_type: &TypeRef) -> Self {
        if self.name == "self" {
            let mut resolved = self_type.clone();
            resolved.nullable |= self.nullable;
            return resolved;
        }

        Self {
            name: self.name.clone(),
            arguments: self
                .arguments
                .iter()
                .map(|argument| match argument {
                    TypeArgumentRef::Type(ty) => {
                        TypeArgumentRef::Type(ty.resolve_self_in(self_type))
                    }
                    TypeArgumentRef::Value(value) => TypeArgumentRef::Value(value.clone()),
                })
                .collect(),
            nullable: self.nullable,
        }
    }

    pub fn as_class_name(&self) -> Option<&str> {
        if IntegerType::from_source_name(&self.name).is_some() {
            return None;
        }
        if SharedHandleKind::from_source_name(&self.name).is_some() {
            return None;
        }
        match self.name.as_str() {
            "void" | "float" | "float32" | "float64" | "string" | "bool" | "mixed" | "null"
            | "resource" | "Bytes" | "List" | "Dictionary" | "Set" | "[]" | "Unknown" => None,
            _ => Some(&self.name),
        }
    }
}

impl fmt::Display for TypeRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.nullable {
            write!(formatter, "?")?;
        }
        if self.name == "[]" && self.arguments.len() == 1 {
            if let TypeArgumentRef::Type(element) = &self.arguments[0] {
                return write!(formatter, "{element}[]");
            }
        }

        if self.arguments.is_empty() {
            write!(formatter, "{}", self.name)
        } else {
            let args = self
                .arguments
                .iter()
                .map(|argument| match argument {
                    TypeArgumentRef::Type(ty) => ty.to_string(),
                    TypeArgumentRef::Value(value) => value.clone(),
                })
                .collect::<Vec<_>>();
            write!(formatter, "{}<{}>", self.name, args.join(", "))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeId(usize);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClassType<T> {
    pub name: String,
    pub arguments: Vec<T>,
}

impl<T> ClassType<T> {
    pub fn new(name: impl Into<String>, arguments: Vec<T>) -> Self {
        Self {
            name: name.into(),
            arguments,
        }
    }
}

/// The six compiler-known Stage 25a shared-ownership types (record 0106). Each
/// takes exactly one type argument and belongs to one of two permanently disjoint
/// families; the family is fixed at construction and never converts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SharedHandleKind {
    SharedReference,
    WeakReference,
    WritableSharedReference,
    WritableWeakReference,
    ReadonlySharedReferenceAccess,
    WritableSharedReferenceAccess,
}

/// Which disjoint shared-ownership family a handle belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SharedFamily {
    /// `SharedReference<T>` / `WeakReference<T>`: direct readonly payload access,
    /// no writable-access runtime state on the allocation.
    Readonly,
    /// `WritableSharedReference<T>` and friends: access must be acquired, and the
    /// allocation carries one access state shared by all its strong handles.
    Writable,
}

impl SharedHandleKind {
    pub const ALL: [SharedHandleKind; 6] = [
        SharedHandleKind::SharedReference,
        SharedHandleKind::WeakReference,
        SharedHandleKind::WritableSharedReference,
        SharedHandleKind::WritableWeakReference,
        SharedHandleKind::ReadonlySharedReferenceAccess,
        SharedHandleKind::WritableSharedReferenceAccess,
    ];

    pub fn source_name(self) -> &'static str {
        match self {
            SharedHandleKind::SharedReference => "SharedReference",
            SharedHandleKind::WeakReference => "WeakReference",
            SharedHandleKind::WritableSharedReference => "WritableSharedReference",
            SharedHandleKind::WritableWeakReference => "WritableWeakReference",
            SharedHandleKind::ReadonlySharedReferenceAccess => "ReadonlySharedReferenceAccess",
            SharedHandleKind::WritableSharedReferenceAccess => "WritableSharedReferenceAccess",
        }
    }

    pub fn from_source_name(name: &str) -> Option<Self> {
        SharedHandleKind::ALL
            .into_iter()
            .find(|kind| kind.source_name() == name)
    }

    /// Whether ordinary member lookup is forwarded to the payload value.
    /// Writable-family handles require an access object before forwarding.
    pub const fn forwards_payload(self) -> bool {
        matches!(
            self,
            Self::SharedReference
                | Self::ReadonlySharedReferenceAccess
                | Self::WritableSharedReferenceAccess
        )
    }

    pub fn family(self) -> SharedFamily {
        match self {
            SharedHandleKind::SharedReference | SharedHandleKind::WeakReference => {
                SharedFamily::Readonly
            }
            SharedHandleKind::WritableSharedReference
            | SharedHandleKind::WritableWeakReference
            | SharedHandleKind::ReadonlySharedReferenceAccess
            | SharedHandleKind::WritableSharedReferenceAccess => SharedFamily::Writable,
        }
    }

    /// Weak handles do not keep their payload alive.
    pub fn is_weak(self) -> bool {
        matches!(
            self,
            SharedHandleKind::WeakReference | SharedHandleKind::WritableWeakReference
        )
    }

    /// Access guards returned by the writable family's acquire operations.
    pub fn is_access(self) -> bool {
        matches!(
            self,
            SharedHandleKind::ReadonlySharedReferenceAccess
                | SharedHandleKind::WritableSharedReferenceAccess
        )
    }

    /// Only `SharedReference<T>` is user-constructible through `shared new`, and
    /// only `WritableSharedReference<T>` through its ordinary constructor. Weak and
    /// access handles are produced exclusively by their approved operations.
    pub fn is_directly_constructible(self) -> bool {
        matches!(self, SharedHandleKind::WritableSharedReference)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TypeKind {
    Void,
    Integer(IntegerType),
    Float(FloatType),
    String,
    Bytes,
    Nullable(TypeId),
    Bool,
    Null,
    Mixed,
    TypedArray(TypeId),
    Unknown,
    Heterogeneous,
    EmptyCollection,
    TypeParameter(String),
    Class(ClassType<TypeId>),
    List(TypeId),
    Dictionary(TypeId, TypeId),
    SortedDictionary(TypeId, TypeId),
    Set(TypeId),
    SortedSet(TypeId),
    PriorityQueue(TypeId),
    Deque(TypeId),
    SharedHandle(SharedHandleKind, TypeId),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ResolvedType {
    Void,
    Integer(IntegerType),
    Float(FloatType),
    String,
    Bytes,
    Bool,
    Null,
    Mixed,
    TypeParameter(String),
    Nullable(Box<ResolvedType>),
    Class(ClassType<ResolvedType>),
    TypedArray(Box<ResolvedType>),
    List(Box<ResolvedType>),
    Dictionary(Box<ResolvedType>, Box<ResolvedType>),
    SortedDictionary(Box<ResolvedType>, Box<ResolvedType>),
    Set(Box<ResolvedType>),
    SortedSet(Box<ResolvedType>),
    PriorityQueue(Box<ResolvedType>),
    Deque(Box<ResolvedType>),
    SharedHandle(SharedHandleKind, Box<ResolvedType>),
    Unsupported,
}

pub(crate) fn resolved_type_complexity(ty: &ResolvedType) -> usize {
    match ty {
        ResolvedType::Nullable(inner)
        | ResolvedType::TypedArray(inner)
        | ResolvedType::List(inner)
        | ResolvedType::Set(inner)
        | ResolvedType::SortedSet(inner)
        | ResolvedType::PriorityQueue(inner)
        | ResolvedType::Deque(inner)
        | ResolvedType::SharedHandle(_, inner) => 1 + resolved_type_complexity(inner),
        ResolvedType::Dictionary(key, value) | ResolvedType::SortedDictionary(key, value) => {
            1 + resolved_type_complexity(key) + resolved_type_complexity(value)
        }
        ResolvedType::Class(class) => {
            1 + class
                .arguments
                .iter()
                .map(resolved_type_complexity)
                .sum::<usize>()
        }
        ResolvedType::Integer(_)
        | ResolvedType::Float(_)
        | ResolvedType::Bool
        | ResolvedType::String
        | ResolvedType::Bytes
        | ResolvedType::Mixed
        | ResolvedType::Void
        | ResolvedType::Null
        | ResolvedType::TypeParameter(_)
        | ResolvedType::Unsupported => 1,
    }
}

#[derive(Debug, Default)]
pub struct TypeRegistry {
    ids: HashMap<TypeKind, TypeId>,
    kinds: Vec<TypeKind>,
}

impl TypeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn intern(&mut self, kind: TypeKind) -> TypeId {
        if let Some(id) = self.ids.get(&kind) {
            return *id;
        }

        let id = TypeId(self.kinds.len());
        self.kinds.push(kind.clone());
        self.ids.insert(kind, id);
        id
    }

    pub fn kind(&self, id: TypeId) -> &TypeKind {
        &self.kinds[id.0]
    }

    pub fn unknown(&mut self) -> TypeId {
        self.intern(TypeKind::Unknown)
    }

    pub fn class_name(&self, id: TypeId) -> Option<&str> {
        match self.kind(id) {
            TypeKind::Class(class) => Some(&class.name),
            _ => None,
        }
    }

    pub fn display(&self, id: TypeId) -> String {
        match self.kind(id) {
            TypeKind::Void => "void".to_string(),
            TypeKind::Integer(integer) => integer.source_name().to_string(),
            TypeKind::Float(float) => float.source_name().to_string(),
            TypeKind::String => "string".to_string(),
            TypeKind::Bytes => "Bytes".to_string(),
            TypeKind::Nullable(inner) => format!("?{}", self.display(*inner)),
            TypeKind::Bool => "bool".to_string(),
            TypeKind::Null => "null".to_string(),
            TypeKind::Mixed => "mixed".to_string(),
            TypeKind::TypedArray(element) => format!("{}[]", self.display(*element)),
            TypeKind::Unknown => "Unknown".to_string(),
            TypeKind::Heterogeneous => "heterogeneous".to_string(),
            TypeKind::EmptyCollection => "[]".to_string(),
            TypeKind::TypeParameter(name) => name.clone(),
            TypeKind::Class(class) => {
                if class.arguments.is_empty() {
                    class.name.clone()
                } else {
                    format!(
                        "{}<{}>",
                        class.name,
                        class
                            .arguments
                            .iter()
                            .map(|argument| self.display(*argument))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
            }
            TypeKind::List(element) => format!("List<{}>", self.display(*element)),
            TypeKind::Dictionary(key, value) => {
                format!(
                    "Dictionary<{}, {}>",
                    self.display(*key),
                    self.display(*value)
                )
            }
            TypeKind::SortedDictionary(key, value) => {
                format!(
                    "SortedDictionary<{}, {}>",
                    self.display(*key),
                    self.display(*value)
                )
            }
            TypeKind::Set(element) => format!("Set<{}>", self.display(*element)),
            TypeKind::SortedSet(element) => format!("SortedSet<{}>", self.display(*element)),
            TypeKind::PriorityQueue(element) => {
                format!("PriorityQueue<{}>", self.display(*element))
            }
            TypeKind::Deque(element) => format!("Deque<{}>", self.display(*element)),
            TypeKind::SharedHandle(kind, payload) => {
                format!("{}<{}>", kind.source_name(), self.display(*payload))
            }
        }
    }

    pub fn resolved(&self, id: TypeId) -> ResolvedType {
        match self.kind(id) {
            TypeKind::Void => ResolvedType::Void,
            TypeKind::Integer(ty) => ResolvedType::Integer(*ty),
            TypeKind::Float(ty) => ResolvedType::Float(*ty),
            TypeKind::String => ResolvedType::String,
            TypeKind::Bytes => ResolvedType::Bytes,
            TypeKind::Bool => ResolvedType::Bool,
            TypeKind::Null => ResolvedType::Null,
            TypeKind::Mixed => ResolvedType::Mixed,
            TypeKind::TypeParameter(name) => ResolvedType::TypeParameter(name.clone()),
            TypeKind::Nullable(inner) => ResolvedType::Nullable(Box::new(self.resolved(*inner))),
            TypeKind::Class(class) => ResolvedType::Class(ClassType::new(
                class.name.clone(),
                class
                    .arguments
                    .iter()
                    .map(|argument| self.resolved(*argument))
                    .collect(),
            )),
            TypeKind::TypedArray(element) => {
                ResolvedType::TypedArray(Box::new(self.resolved(*element)))
            }
            TypeKind::List(element) => ResolvedType::List(Box::new(self.resolved(*element))),
            TypeKind::Dictionary(key, value) => ResolvedType::Dictionary(
                Box::new(self.resolved(*key)),
                Box::new(self.resolved(*value)),
            ),
            TypeKind::SortedDictionary(key, value) => ResolvedType::SortedDictionary(
                Box::new(self.resolved(*key)),
                Box::new(self.resolved(*value)),
            ),
            TypeKind::Set(element) => ResolvedType::Set(Box::new(self.resolved(*element))),
            TypeKind::SortedSet(element) => {
                ResolvedType::SortedSet(Box::new(self.resolved(*element)))
            }
            TypeKind::PriorityQueue(element) => {
                ResolvedType::PriorityQueue(Box::new(self.resolved(*element)))
            }
            TypeKind::Deque(element) => ResolvedType::Deque(Box::new(self.resolved(*element))),
            TypeKind::SharedHandle(kind, payload) => {
                ResolvedType::SharedHandle(*kind, Box::new(self.resolved(*payload)))
            }
            TypeKind::Unknown | TypeKind::Heterogeneous | TypeKind::EmptyCollection => {
                ResolvedType::Unsupported
            }
        }
    }
}

use std::collections::HashMap;
use std::fmt;

pub use crate::numeric::{FloatType, IntegerType};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeRef {
    pub name: String,
    pub args: Vec<TypeRef>,
}

impl TypeRef {
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            args: Vec::new(),
        }
    }

    pub fn generic(name: impl Into<String>, args: Vec<TypeRef>) -> Self {
        Self {
            name: name.into(),
            args,
        }
    }

    pub fn array_of(element: TypeRef) -> Self {
        Self::generic("[]", vec![element])
    }

    pub fn unknown() -> Self {
        Self::named("Unknown")
    }

    pub fn as_class_name(&self) -> Option<&str> {
        if IntegerType::from_source_name(&self.name).is_some() {
            return None;
        }
        match self.name.as_str() {
            "void" | "float" | "float32" | "float64" | "string" | "bool" | "mixed" | "null"
            | "resource" | "List" | "Dictionary" | "Set" | "[]" | "Unknown" => None,
            _ => Some(&self.name),
        }
    }
}

impl fmt::Display for TypeRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.name == "[]" && self.args.len() == 1 {
            return write!(formatter, "{}[]", self.args[0]);
        }

        if self.args.is_empty() {
            write!(formatter, "{}", self.name)
        } else {
            let args = self
                .args
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            write!(formatter, "{}<{}>", self.name, args)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeId(usize);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TypeKind {
    Void,
    Integer(IntegerType),
    Float(FloatType),
    String,
    Bool,
    Null,
    Mixed,
    TypedArray(TypeId),
    Unknown,
    Heterogeneous,
    EmptyCollection,
    Class(String),
    List(TypeId),
    Dictionary(TypeId, TypeId),
    Set(TypeId),
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
            TypeKind::Class(name) => Some(name),
            _ => None,
        }
    }

    pub fn display(&self, id: TypeId) -> String {
        match self.kind(id) {
            TypeKind::Void => "void".to_string(),
            TypeKind::Integer(integer) => integer.source_name().to_string(),
            TypeKind::Float(float) => float.source_name().to_string(),
            TypeKind::String => "string".to_string(),
            TypeKind::Bool => "bool".to_string(),
            TypeKind::Null => "null".to_string(),
            TypeKind::Mixed => "mixed".to_string(),
            TypeKind::TypedArray(element) => format!("{}[]", self.display(*element)),
            TypeKind::Unknown => "Unknown".to_string(),
            TypeKind::Heterogeneous => "heterogeneous".to_string(),
            TypeKind::EmptyCollection => "[]".to_string(),
            TypeKind::Class(name) => name.clone(),
            TypeKind::List(element) => format!("List<{}>", self.display(*element)),
            TypeKind::Dictionary(key, value) => {
                format!(
                    "Dictionary<{}, {}>",
                    self.display(*key),
                    self.display(*value)
                )
            }
            TypeKind::Set(element) => format!("Set<{}>", self.display(*element)),
        }
    }
}

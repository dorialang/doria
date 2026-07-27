use std::collections::HashMap;
use std::fmt;

pub use crate::numeric::{FloatType, IntegerType};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeRef {
    pub name: String,
    pub args: Vec<TypeRef>,
    /// Parsed non-type generic arguments. Decision 0105 reserves this argument
    /// kind so accepting its syntax does not force the semantic model to
    /// pretend every future generic argument is a type.
    pub value_args: Vec<String>,
    pub nullable: bool,
}

impl TypeRef {
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            args: Vec::new(),
            value_args: Vec::new(),
            nullable: false,
        }
    }

    pub fn generic(name: impl Into<String>, args: Vec<TypeRef>) -> Self {
        Self {
            name: name.into(),
            args,
            value_args: Vec::new(),
            nullable: false,
        }
    }

    pub fn generic_with_values(
        name: impl Into<String>,
        args: Vec<TypeRef>,
        value_args: Vec<String>,
    ) -> Self {
        Self {
            name: name.into(),
            args,
            value_args,
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

    pub fn resolve_self_in(&self, class_name: &str) -> Self {
        Self {
            name: if self.name == "self" {
                class_name.to_string()
            } else {
                self.name.clone()
            },
            args: self
                .args
                .iter()
                .map(|argument| argument.resolve_self_in(class_name))
                .collect(),
            value_args: self.value_args.clone(),
            nullable: self.nullable,
        }
    }

    pub fn as_class_name(&self) -> Option<&str> {
        if IntegerType::from_source_name(&self.name).is_some() {
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
        if self.name == "[]" && self.args.len() == 1 {
            return write!(formatter, "{}[]", self.args[0]);
        }

        if self.args.is_empty() && self.value_args.is_empty() {
            write!(formatter, "{}", self.name)
        } else {
            let mut args = self
                .args
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            args.extend(self.value_args.iter().cloned());
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
    Set(TypeId),
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
    Set(Box<ResolvedType>),
    Unsupported,
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
            TypeKind::Set(element) => format!("Set<{}>", self.display(*element)),
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
            TypeKind::Set(element) => ResolvedType::Set(Box::new(self.resolved(*element))),
            TypeKind::Unknown | TypeKind::Heterogeneous | TypeKind::EmptyCollection => {
                ResolvedType::Unsupported
            }
        }
    }
}

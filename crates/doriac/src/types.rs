use std::fmt;

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

    pub fn unknown() -> Self {
        Self::named("Unknown")
    }

    pub fn as_class_name(&self) -> Option<&str> {
        match self.name.as_str() {
            "void" | "int" | "float" | "string" | "bool" | "array" | "mixed" | "null" | "List"
            | "Dictionary" | "Set" | "Unknown" => None,
            _ => Some(&self.name),
        }
    }
}

impl fmt::Display for TypeRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
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

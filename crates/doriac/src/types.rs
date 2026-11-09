use std::collections::HashMap;
use std::fmt;

/// Access retained by a promoted non-owning field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RetainedFieldAccess {
    Readonly,
    Writable,
}

impl RetainedFieldAccess {
    pub fn promoted(borrow: bool, writable: bool) -> Option<Self> {
        borrow.then_some(if writable {
            Self::Writable
        } else {
            Self::Readonly
        })
    }

    pub fn is_writable(self) -> bool {
        matches!(self, Self::Writable)
    }

    pub fn permits_write_through(access: Option<Self>, slot_writable: bool) -> bool {
        access.map_or(slot_writable, Self::is_writable)
    }
}

use crate::enums::EnumType;
use crate::source::{QualifiedNameRef, Span};

pub use crate::numeric::{FloatType, IntegerType};

/// Collection storage families share ingestion rules across effect and ownership analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CollectionFamily {
    TypedArray,
    List,
    Dictionary,
    Set,
    PriorityQueue,
    Deque,
    Bytes,
}

impl CollectionFamily {
    pub(crate) fn from_resolved(ty: &ResolvedType) -> Option<Self> {
        match ty {
            ResolvedType::TypedArray(_) => Some(Self::TypedArray),
            ResolvedType::List(_) => Some(Self::List),
            ResolvedType::Dictionary(_, _) | ResolvedType::SortedDictionary(_, _) => {
                Some(Self::Dictionary)
            }
            ResolvedType::Set(_) | ResolvedType::SortedSet(_) => Some(Self::Set),
            ResolvedType::PriorityQueue(_) => Some(Self::PriorityQueue),
            ResolvedType::Deque(_) => Some(Self::Deque),
            ResolvedType::Bytes => Some(Self::Bytes),
            ResolvedType::Nullable(inner) | ResolvedType::SharedHandle(_, inner) => {
                Self::from_resolved(inner)
            }
            _ => None,
        }
    }

    pub(crate) fn ingested_arguments(self, method: &str) -> &'static [usize] {
        match (self, method) {
            (Self::List, "add")
            | (Self::Set, "add")
            | (Self::PriorityQueue, "push")
            | (Self::Deque, "pushFront" | "pushBack") => &[0],
            (Self::List, "insertAt") => &[1],
            (Self::Dictionary, "set") => &[0, 1],
            _ => &[],
        }
    }

    pub(crate) fn borrows_element(self, member: &str) -> bool {
        matches!(
            (self, member),
            (Self::Dictionary, "get")
                | (Self::List | Self::Set, "first" | "last")
                | (Self::PriorityQueue, "peek")
                | (Self::Deque, "peekFront" | "peekBack")
        )
    }

    pub(crate) fn removes_element(self, method: &str) -> bool {
        matches!(
            (self, method),
            (Self::List, "removeAt" | "pop")
                | (Self::Dictionary, "remove")
                | (Self::PriorityQueue, "pop")
                | (Self::Deque, "popFront" | "popBack")
        )
    }
}

/// Preserve compiler-checked nominal kinds when substituting source type syntax.
pub(crate) fn resolve_nominal_kinds<I>(
    ty: ResolvedType,
    enums: &HashMap<ClassType<ResolvedType>, crate::enums::EnumType>,
    interfaces: &HashMap<crate::types::InterfaceType<ResolvedType>, I>,
) -> ResolvedType {
    resolve_nominal_kinds_with(ty, &|nominal| {
        if let Some(definition) = enums.get(&nominal) {
            ResolvedType::Enum(definition.clone())
        } else if interfaces.contains_key(&nominal) {
            ResolvedType::Interface(nominal)
        } else {
            ResolvedType::Class(nominal)
        }
    })
}

pub(crate) fn resolve_nominal_kinds_with(
    ty: ResolvedType,
    resolve: &impl Fn(ClassType<ResolvedType>) -> ResolvedType,
) -> ResolvedType {
    match ty {
        ResolvedType::Nullable(inner) => {
            ResolvedType::Nullable(Box::new(resolve_nominal_kinds_with(*inner, resolve)))
        }
        ResolvedType::TypedArray(inner) => {
            ResolvedType::TypedArray(Box::new(resolve_nominal_kinds_with(*inner, resolve)))
        }
        ResolvedType::List(inner) => {
            ResolvedType::List(Box::new(resolve_nominal_kinds_with(*inner, resolve)))
        }
        ResolvedType::Set(inner) => {
            ResolvedType::Set(Box::new(resolve_nominal_kinds_with(*inner, resolve)))
        }
        ResolvedType::SortedSet(inner) => {
            ResolvedType::SortedSet(Box::new(resolve_nominal_kinds_with(*inner, resolve)))
        }
        ResolvedType::PriorityQueue(inner) => {
            ResolvedType::PriorityQueue(Box::new(resolve_nominal_kinds_with(*inner, resolve)))
        }
        ResolvedType::Deque(inner) => {
            ResolvedType::Deque(Box::new(resolve_nominal_kinds_with(*inner, resolve)))
        }
        ResolvedType::Dictionary(key, value) => ResolvedType::Dictionary(
            Box::new(resolve_nominal_kinds_with(*key, resolve)),
            Box::new(resolve_nominal_kinds_with(*value, resolve)),
        ),
        ResolvedType::SortedDictionary(key, value) => ResolvedType::SortedDictionary(
            Box::new(resolve_nominal_kinds_with(*key, resolve)),
            Box::new(resolve_nominal_kinds_with(*value, resolve)),
        ),
        ResolvedType::SharedHandle(kind, payload) => ResolvedType::SharedHandle(
            kind,
            Box::new(resolve_nominal_kinds_with(*payload, resolve)),
        ),
        ResolvedType::Enum(enum_type) => ResolvedType::Enum(
            enum_type
                .map_arguments(|argument| resolve_nominal_kinds_with(argument.clone(), resolve)),
        ),
        ResolvedType::Class(class) => {
            let nominal = ClassType::new(
                class.name,
                class
                    .arguments
                    .into_iter()
                    .map(|argument| resolve_nominal_kinds_with(argument, resolve))
                    .collect(),
            );
            resolve(nominal)
        }
        ResolvedType::Function(mut function) => {
            for parameter in &mut function.parameters {
                parameter.ty = resolve_nominal_kinds_with(parameter.ty.clone(), resolve);
            }
            function.return_type = resolve_nominal_kinds_with(function.return_type, resolve);
            function.checked_effects = function
                .checked_effects
                .into_iter()
                .map(|effect| resolve_nominal_kinds_with(effect, resolve))
                .collect();
            ResolvedType::Function(function)
        }
        ty => ty,
    }
}

/// Resolve source type syntax for consumers that do not need inferred callable contracts.
/// Execution lowering must use `resolved_type_ref_with_checked_functions` instead.
pub(crate) fn resolved_type_ref_with_substitutions(
    ty: &crate::types::TypeRef,
    substitutions: &HashMap<String, ResolvedType>,
) -> Option<ResolvedType> {
    resolve_type_ref(ty, substitutions, &|function| {
        Some(ResolvedType::Function(Box::new(
            crate::types::SemanticFunctionType {
                invocation_mode: function.invocation_mode,
                parameters: function
                    .parameters
                    .iter()
                    .map(|parameter| {
                        Some(crate::types::SemanticFunctionParameter {
                            ownership_mode: parameter.ownership_mode,
                            ty: resolved_type_ref_with_substitutions(&parameter.ty, substitutions)?,
                        })
                    })
                    .collect::<Option<Vec<_>>>()?,
                return_type: resolved_type_ref_with_substitutions(
                    &function.return_type,
                    substitutions,
                )?,
                checked_effects: if let Some(clause) = &function.throws_clause {
                    clause
                        .entries
                        .iter()
                        .map(|effect| {
                            resolved_type_ref_with_substitutions(&effect.ty, substitutions)
                        })
                        .collect::<Option<Vec<_>>>()?
                        .into_iter()
                        .filter(|effect| !crate::checked_effects::is_ambient_io_effect(effect))
                        .collect()
                } else {
                    Vec::new()
                },
                return_borrow: None,
            },
        )))
    })
}

/// Lowering must preserve checked callable contracts rather than reconstruct them from syntax.
pub(crate) fn resolved_type_ref_with_checked_functions(
    ty: &TypeRef,
    substitutions: &HashMap<String, ResolvedType>,
    resolve_function: &impl Fn(Span) -> Option<ResolvedType>,
) -> Option<ResolvedType> {
    resolve_type_ref(ty, substitutions, &|function| {
        let checked = resolve_function(function.span)?;
        Some(substitute_resolved_type(&checked, substitutions))
    })
}

fn resolve_type_ref(
    ty: &TypeRef,
    substitutions: &HashMap<String, ResolvedType>,
    resolve_function: &impl Fn(&FunctionTypeRef) -> Option<ResolvedType>,
) -> Option<ResolvedType> {
    if let Some(grouped) = &ty.grouped {
        let resolved = resolve_type_ref(&grouped.inner, substitutions, resolve_function)?;
        return Some(if ty.nullable {
            nullable_of(resolved)
        } else {
            resolved
        });
    }
    if let Some(function) = &ty.function {
        let resolved = resolve_function(function)?;
        return Some(if ty.nullable {
            nullable_of(resolved)
        } else {
            resolved
        });
    }
    let mut plain = ty.clone();
    plain.nullable = false;
    let base = if plain.arguments.is_empty() {
        if let Some(substitution) = substitutions.get(&plain.name) {
            substitution.clone()
        } else if let Some(integer) = IntegerType::from_source_name(&plain.name) {
            ResolvedType::Integer(integer)
        } else if let Some(float) = FloatType::from_source_name(&plain.name) {
            ResolvedType::Float(float)
        } else {
            match plain.name.as_str() {
                "void" => ResolvedType::Void,
                "string" => ResolvedType::String,
                "Bytes" => ResolvedType::Bytes,
                name if ByteRegionKind::from_source_name(name).is_some() => {
                    ResolvedType::ByteRegion(ByteRegionKind::from_source_name(name)?)
                }
                "bool" => ResolvedType::Bool,
                "mixed" => ResolvedType::Mixed,
                "Error" => ResolvedType::Error,
                _ => ResolvedType::Class(ClassType::new(plain.name, Vec::new())),
            }
        }
    } else {
        let arguments = plain
            .type_arguments()
            .map(|argument| resolve_type_ref(argument, substitutions, resolve_function))
            .collect::<Option<Vec<_>>>()?;
        match plain.name.as_str() {
            "[]" if arguments.len() == 1 => {
                ResolvedType::TypedArray(Box::new(arguments[0].clone()))
            }
            "List" if arguments.len() == 1 => ResolvedType::List(Box::new(arguments[0].clone())),
            "Dictionary" if arguments.len() == 2 => ResolvedType::Dictionary(
                Box::new(arguments[0].clone()),
                Box::new(arguments[1].clone()),
            ),
            "SortedDictionary" if arguments.len() == 2 => ResolvedType::SortedDictionary(
                Box::new(arguments[0].clone()),
                Box::new(arguments[1].clone()),
            ),
            "Set" if arguments.len() == 1 => ResolvedType::Set(Box::new(arguments[0].clone())),
            "SortedSet" if arguments.len() == 1 => {
                ResolvedType::SortedSet(Box::new(arguments[0].clone()))
            }
            "PriorityQueue" if arguments.len() == 1 => {
                ResolvedType::PriorityQueue(Box::new(arguments[0].clone()))
            }
            "Deque" if arguments.len() == 1 => ResolvedType::Deque(Box::new(arguments[0].clone())),
            name if arguments.len() == 1 => {
                if let Some(kind) = crate::types::SharedHandleKind::from_source_name(name) {
                    ResolvedType::SharedHandle(kind, Box::new(arguments[0].clone()))
                } else {
                    ResolvedType::Class(ClassType::new(plain.name, arguments))
                }
            }
            _ => ResolvedType::Class(ClassType::new(plain.name, arguments)),
        }
    };
    if ty.nullable {
        Some(nullable_of(base))
    } else {
        Some(base)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeRef {
    pub name: String,
    pub source_name: Option<QualifiedNameRef>,
    /// Generic arguments in source order. Decision 0105 reserves value
    /// arguments, so the syntax tree must preserve both their kind and their
    /// position without pretending every argument is a type.
    pub arguments: Vec<TypeArgumentRef>,
    pub nullable: bool,
    /// Source-preserving syntax for the accepted `function(T): R` type form.
    /// Semantic callable compatibility remains a Stage 30 concern.
    pub function: Option<Box<FunctionTypeRef>>,
    /// Authored parentheses around a type. Grouping is transparent to semantic
    /// type resolution, but its delimiters remain available to tooling.
    pub grouped: Option<Box<GroupedTypeRef>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeArgumentRef {
    Type(TypeRef),
    Value(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionTypeRef {
    pub keyword_span: Span,
    pub invocation_mode: FunctionInvocationMode,
    pub invocation_modifier_span: Option<Span>,
    pub parameter_list_open_span: Span,
    pub parameter_list_close_span: Span,
    pub parameter_list_span: Span,
    pub parameters: Vec<FunctionTypeParameterRef>,
    pub colon_span: Span,
    pub return_type: Box<TypeRef>,
    pub return_type_span: Span,
    pub throws_clause: Option<FunctionTypeThrowsRef>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionTypeParameterRef {
    pub ownership_mode: FunctionTypeParameterMode,
    pub ownership_modifier_span: Option<Span>,
    pub ty: TypeRef,
    pub type_span: Span,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FunctionInvocationMode {
    Readonly,
    Writable,
    Once,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FunctionTypeParameterMode {
    Readonly,
    Writable,
    Take,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionTypeThrowsRef {
    pub keyword_span: Span,
    pub entries: Vec<FunctionTypeEffectRef>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionTypeEffectRef {
    pub ty: TypeRef,
    pub type_span: Span,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupedTypeRef {
    pub open_span: Span,
    pub inner: TypeRef,
    pub close_span: Span,
    pub span: Span,
}

impl TypeRef {
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            source_name: None,
            arguments: Vec::new(),
            nullable: false,
            function: None,
            grouped: None,
        }
    }

    pub fn generic(name: impl Into<String>, args: Vec<TypeRef>) -> Self {
        Self {
            name: name.into(),
            source_name: None,
            arguments: args.into_iter().map(TypeArgumentRef::Type).collect(),
            nullable: false,
            function: None,
            grouped: None,
        }
    }

    pub fn generic_with_arguments(
        name: impl Into<String>,
        arguments: Vec<TypeArgumentRef>,
    ) -> Self {
        Self {
            name: name.into(),
            source_name: None,
            arguments,
            nullable: false,
            function: None,
            grouped: None,
        }
    }

    pub fn function(function: FunctionTypeRef) -> Self {
        Self {
            name: "function".to_string(),
            source_name: None,
            arguments: Vec::new(),
            nullable: false,
            function: Some(Box::new(function)),
            grouped: None,
        }
    }

    pub fn grouped(inner: TypeRef, open_span: Span, close_span: Span) -> Self {
        Self {
            name: inner.name.clone(),
            source_name: inner.source_name.clone(),
            arguments: inner.arguments.clone(),
            nullable: false,
            function: inner.function.clone(),
            grouped: Some(Box::new(GroupedTypeRef {
                open_span,
                close_span,
                span: open_span.merge(close_span),
                inner,
            })),
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

    pub fn with_source_name(mut self, source_name: QualifiedNameRef) -> Self {
        self.source_name = Some(source_name);
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
        if let Some(grouped) = &self.grouped {
            let mut resolved = TypeRef::grouped(
                grouped.inner.resolve_self_in(self_type),
                grouped.open_span,
                grouped.close_span,
            );
            resolved.nullable = self.nullable;
            return resolved;
        }
        if self.name == "self" {
            let mut resolved = self_type.clone();
            resolved.nullable |= self.nullable;
            return resolved;
        }

        Self {
            name: self.name.clone(),
            source_name: self.source_name.clone(),
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
            function: self.function.as_ref().map(|function| {
                Box::new(FunctionTypeRef {
                    keyword_span: function.keyword_span,
                    invocation_mode: function.invocation_mode,
                    invocation_modifier_span: function.invocation_modifier_span,
                    parameter_list_open_span: function.parameter_list_open_span,
                    parameter_list_close_span: function.parameter_list_close_span,
                    parameter_list_span: function.parameter_list_span,
                    parameters: function
                        .parameters
                        .iter()
                        .map(|parameter| FunctionTypeParameterRef {
                            ownership_mode: parameter.ownership_mode,
                            ownership_modifier_span: parameter.ownership_modifier_span,
                            ty: parameter.ty.resolve_self_in(self_type),
                            type_span: parameter.type_span,
                            span: parameter.span,
                        })
                        .collect(),
                    colon_span: function.colon_span,
                    return_type: Box::new(function.return_type.resolve_self_in(self_type)),
                    return_type_span: function.return_type_span,
                    throws_clause: function.throws_clause.as_ref().map(|clause| {
                        FunctionTypeThrowsRef {
                            keyword_span: clause.keyword_span,
                            entries: clause
                                .entries
                                .iter()
                                .map(|entry| FunctionTypeEffectRef {
                                    ty: entry.ty.resolve_self_in(self_type),
                                    type_span: entry.type_span,
                                    span: entry.span,
                                })
                                .collect(),
                            span: clause.span,
                        }
                    }),
                    span: function.span,
                })
            }),
            grouped: None,
        }
    }

    pub fn as_class_name(&self) -> Option<&str> {
        if let Some(grouped) = &self.grouped {
            return grouped.inner.as_class_name();
        }
        if self.function.is_some() {
            return None;
        }
        if IntegerType::from_source_name(&self.name).is_some() {
            return None;
        }
        if SharedHandleKind::from_source_name(&self.name).is_some() {
            return None;
        }
        match self.name.as_str() {
            "void" | "float" | "float32" | "float64" | "string" | "bool" | "mixed" | "null"
            | "resource" | "Error" | "Bytes" | "List" | "Dictionary" | "Set" | "[]" | "Unknown" => {
                None
            }
            _ => Some(&self.name),
        }
    }
}

impl fmt::Display for TypeRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.nullable {
            write!(formatter, "?")?;
        }
        if let Some(grouped) = &self.grouped {
            return write!(formatter, "({})", grouped.inner);
        }
        if let Some(function) = &self.function {
            let parameters = function
                .parameters
                .iter()
                .map(|parameter| {
                    let prefix = match parameter.ownership_mode {
                        FunctionTypeParameterMode::Readonly => "",
                        FunctionTypeParameterMode::Writable => "writable ",
                        FunctionTypeParameterMode::Take => "take ",
                    };
                    format!("{prefix}{}", parameter.ty)
                })
                .collect::<Vec<_>>()
                .join(", ");
            let invocation = match function.invocation_mode {
                FunctionInvocationMode::Readonly => "",
                FunctionInvocationMode::Writable => " writable",
                FunctionInvocationMode::Once => " once",
            };
            write!(
                formatter,
                "function{invocation}({parameters}): {}",
                function.return_type
            )?;
            if let Some(clause) = &function.throws_clause {
                let effects = clause
                    .entries
                    .iter()
                    .map(|entry| entry.ty.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(formatter, " throws {effects}")?;
            }
            return Ok(());
        }
        if self.name == "[]" && self.arguments.len() == 1 {
            if let TypeArgumentRef::Type(element) = &self.arguments[0] {
                return write!(formatter, "{element}[]");
            }
        }

        if self.arguments.is_empty() {
            write!(
                formatter,
                "{}",
                crate::trait_composition::authored_type_parameter_name(&self.name)
            )
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

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SemanticFunctionParameter<T> {
    pub ownership_mode: FunctionTypeParameterMode,
    pub ty: T,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SemanticFunctionType<T> {
    pub invocation_mode: FunctionInvocationMode,
    pub parameters: Vec<SemanticFunctionParameter<T>>,
    pub return_type: T,
    /// Canonical set order used for semantic identity. Source-facing metadata
    /// separately preserves authored or first-origin order.
    pub checked_effects: Vec<T>,
    pub return_borrow: Option<FunctionReturnBorrow>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RetainedLifetime {
    /// The source is observed only while the returned value is used.
    LastUse,
    /// Destruction of an owned value can still observe the source.
    Cleanup,
}

impl RetainedLifetime {
    pub const fn join(self, other: Self) -> Self {
        match (self, other) {
            (Self::LastUse, Self::LastUse) => Self::LastUse,
            _ => Self::Cleanup,
        }
    }

    /// An implementation cannot retain an input longer than its contract.
    pub const fn accepts(self, actual: Self) -> bool {
        matches!(self, Self::Cleanup) || matches!(actual, Self::LastUse)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ReturnBorrowKind {
    /// The returned value itself is lent by the source; callers do not own it.
    Value,
    /// The returned value is owned, but retains a borrow of the source.
    Retained,
}

impl ReturnBorrowKind {
    pub const fn applies_to_move_type(self, move_type: bool) -> bool {
        matches!(self, Self::Retained) || move_type
    }

    /// Branches promise their common value access, but retain every source loan
    /// that either branch can require.
    pub const fn join_writable(self, left: bool, right: bool) -> bool {
        match self {
            Self::Value => left && right,
            Self::Retained => left || right,
        }
    }

    /// An implementation may provide stronger value access, or require weaker
    /// retained-source access, than its declared contract.
    pub const fn accepts_writable(self, required: bool, actual: bool) -> bool {
        match self {
            Self::Value => !required || actual,
            Self::Retained => !actual || required,
        }
    }

    /// Compose a callee's returned contract with its source expression. Access
    /// to a borrowed value may narrow; an already retained source loan cannot
    /// be weakened by wrapping its owned carrier in another owned result.
    pub const fn compose_writable(
        self,
        source_kind: Self,
        source_writable: bool,
        returned_writable: bool,
    ) -> bool {
        match (source_kind, self) {
            (Self::Value, Self::Value) => source_writable && returned_writable,
            (Self::Retained, Self::Retained) => source_writable || returned_writable,
            _ => returned_writable,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FunctionReturnBorrow {
    pub source: FunctionBorrowSource,
    pub writable: bool,
    pub kind: ReturnBorrowKind,
    pub lifetime: RetainedLifetime,
}

#[cfg(test)]
mod return_borrow_tests {
    use super::RetainedLifetime::{Cleanup, LastUse};
    use super::ReturnBorrowKind::{Retained, Value};

    #[test]
    fn retained_lifetime_joins_preserve_cleanup_and_contracts_bound_demand() {
        assert_eq!(LastUse.join(LastUse), LastUse);
        assert_eq!(LastUse.join(Cleanup), Cleanup);
        assert_eq!(Cleanup.join(LastUse), Cleanup);
        assert_eq!(Cleanup.join(Cleanup), Cleanup);
        assert!(LastUse.accepts(LastUse));
        assert!(!LastUse.accepts(Cleanup));
        assert!(Cleanup.accepts(LastUse));
        assert!(Cleanup.accepts(Cleanup));
    }

    #[test]
    fn returned_access_and_retained_loan_demand_compose_in_opposite_directions() {
        for left in [false, true] {
            for right in [false, true] {
                assert_eq!(Value.join_writable(left, right), left && right);
                assert_eq!(Retained.join_writable(left, right), left || right);
                assert_eq!(Value.accepts_writable(left, right), !left || right);
                assert_eq!(Retained.accepts_writable(left, right), !right || left);
                assert_eq!(Value.compose_writable(Value, left, right), left && right);
                assert_eq!(
                    Retained.compose_writable(Retained, left, right),
                    left || right
                );
                assert_eq!(Value.compose_writable(Retained, left, right), right);
                assert_eq!(Retained.compose_writable(Value, left, right), right);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FunctionBorrowSource {
    Parameter(usize),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NominalType<T> {
    pub name: String,
    pub arguments: Vec<T>,
}

impl<T> NominalType<T> {
    pub fn new(name: impl Into<String>, arguments: Vec<T>) -> Self {
        Self {
            name: name.into(),
            arguments,
        }
    }

    pub fn inline_copy_type(&self) -> Option<crate::inline_value::InlineValueKind> {
        crate::inline_value::InlineValueKind::for_nominal(&self.name, self.arguments.len())
    }
}

pub type ClassType<T> = NominalType<T>;
pub type InterfaceType<T> = NominalType<T>;

/// The six compiler-known Stage 25a shared-ownership types (record 0106). Each
/// takes exactly one type argument and belongs to one of two permanently disjoint
/// families; the family is fixed at construction and never converts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
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

    /// Compiler-owned operations take precedence over payload member lookup.
    /// The result preserves the payload type; the flag denotes a nullable result.
    pub(crate) fn method_result(self, method: &str) -> Option<(Self, bool)> {
        use SharedHandleKind::*;
        let result = match (self, method) {
            (SharedReference, "share") => (SharedReference, false),
            (SharedReference, "createWeakReference") => (WeakReference, false),
            (WritableSharedReference, "share") => (WritableSharedReference, false),
            (WritableSharedReference, "createWeakReference") => (WritableWeakReference, false),
            (WritableSharedReference, "acquireReadonlyAccess") => {
                (ReadonlySharedReferenceAccess, false)
            }
            (WritableSharedReference, "acquireWritableAccess") => {
                (WritableSharedReferenceAccess, false)
            }
            (WeakReference, "acquire") => (SharedReference, true),
            (WritableWeakReference, "acquire") => (WritableSharedReference, true),
            _ => return None,
        };
        Some(result)
    }

    pub(crate) fn projects_payload_property(self, property: &str) -> bool {
        self == Self::SharedReference && property == "referencedValue"
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ByteRegionKind {
    Readonly,
    Writable,
}

impl ByteRegionKind {
    pub const fn source_name(self) -> &'static str {
        match self {
            Self::Readonly => "Doria\\Std\\Io\\ByteRegion",
            Self::Writable => "Doria\\Std\\Io\\WritableByteRegion",
        }
    }

    pub fn from_source_name(name: &str) -> Option<Self> {
        [Self::Readonly, Self::Writable]
            .into_iter()
            .find(|kind| kind.source_name() == name)
    }

    pub(crate) fn from_resolved(ty: &ResolvedType) -> Option<Self> {
        match ty {
            ResolvedType::ByteRegion(kind) => Some(*kind),
            ResolvedType::Nullable(inner) => Self::from_resolved(inner),
            _ => None,
        }
    }

    pub const fn is_writable(self) -> bool {
        matches!(self, Self::Writable)
    }

    /// The region kind of a compiler-known property that views its receiver,
    /// such as `string->byteRegion` (decision log entry 49).
    pub(crate) fn property_result(receiver: &ResolvedType, property: &str) -> Option<Self> {
        let receiver = match receiver {
            ResolvedType::Nullable(inner) => inner,
            _ => receiver,
        };
        let member = crate::byte_intrinsics::member(receiver, property)?;
        (member.kind == crate::byte_intrinsics::ByteMemberKind::Property
            && matches!(receiver, ResolvedType::String))
        .then(|| Self::from_resolved(&member.return_type))
        .flatten()
    }

    pub(crate) fn method_result(receiver: &ResolvedType, method: &str) -> Option<Self> {
        let receiver = match receiver {
            ResolvedType::Nullable(inner) => inner,
            _ => receiver,
        };
        let member = crate::byte_intrinsics::member(receiver, method)?;
        (member.kind == crate::byte_intrinsics::ByteMemberKind::Method)
            .then(|| Self::from_resolved(&member.return_type))
            .flatten()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TypeKind {
    Void,
    Integer(IntegerType),
    Float(FloatType),
    String,
    Bytes,
    ByteRegion(ByteRegionKind),
    Nullable(TypeId),
    Bool,
    Null,
    Mixed,
    Error,
    TypedArray(TypeId),
    Unknown,
    Heterogeneous,
    EmptyCollection,
    TypeParameter(String),
    Function(SemanticFunctionType<TypeId>),
    Enum(EnumType<TypeId>),
    Class(ClassType<TypeId>),
    Interface(InterfaceType<TypeId>),
    /// Exact dynamic implementer, only in an interface's owned return contract.
    InterfaceSelf(String),
    TraitSelf(String),
    List(TypeId),
    Dictionary(TypeId, TypeId),
    SortedDictionary(TypeId, TypeId),
    Set(TypeId),
    SortedSet(TypeId),
    PriorityQueue(TypeId),
    Deque(TypeId),
    SharedHandle(SharedHandleKind, TypeId),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ResolvedType {
    Void,
    Integer(IntegerType),
    Float(FloatType),
    String,
    Bytes,
    ByteRegion(ByteRegionKind),
    Bool,
    Null,
    Mixed,
    Error,
    TypeParameter(String),
    Function(Box<SemanticFunctionType<ResolvedType>>),
    Enum(EnumType),
    Nullable(Box<ResolvedType>),
    Class(ClassType<ResolvedType>),
    Interface(InterfaceType<ResolvedType>),
    InterfaceSelf(String),
    TraitSelf(String),
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

pub(crate) fn resolved_type_is_symbolic(ty: &ResolvedType) -> bool {
    match ty {
        ResolvedType::TypeParameter(_) => true,
        ResolvedType::Function(function) => {
            function
                .parameters
                .iter()
                .any(|parameter| resolved_type_is_symbolic(&parameter.ty))
                || resolved_type_is_symbolic(&function.return_type)
                || function
                    .checked_effects
                    .iter()
                    .any(resolved_type_is_symbolic)
        }
        ResolvedType::Nullable(inner)
        | ResolvedType::TypedArray(inner)
        | ResolvedType::List(inner)
        | ResolvedType::Set(inner)
        | ResolvedType::SortedSet(inner)
        | ResolvedType::PriorityQueue(inner)
        | ResolvedType::Deque(inner)
        | ResolvedType::SharedHandle(_, inner) => resolved_type_is_symbolic(inner),
        ResolvedType::Dictionary(key, value) | ResolvedType::SortedDictionary(key, value) => {
            resolved_type_is_symbolic(key) || resolved_type_is_symbolic(value)
        }
        ResolvedType::Class(class) | ResolvedType::Interface(class) => {
            class.arguments.iter().any(resolved_type_is_symbolic)
        }
        ResolvedType::Enum(enum_type) => enum_type.arguments.iter().any(resolved_type_is_symbolic),
        ResolvedType::Void
        | ResolvedType::Integer(_)
        | ResolvedType::Float(_)
        | ResolvedType::String
        | ResolvedType::Bytes
        | ResolvedType::ByteRegion(_)
        | ResolvedType::Bool
        | ResolvedType::Null
        | ResolvedType::Mixed
        | ResolvedType::Error
        | ResolvedType::InterfaceSelf(_)
        | ResolvedType::TraitSelf(_)
        | ResolvedType::Unsupported => false,
    }
}

/// Wrap `inner` in a nullable, collapsing `?(?X)` to `?X`. Substituting a `?T`
/// field's parameter with a nullable argument must not yield a doubly-nullable
/// type, which has no downstream representation.
pub(crate) fn nullable_of(inner: ResolvedType) -> ResolvedType {
    if matches!(inner, ResolvedType::Nullable(_)) {
        inner
    } else {
        ResolvedType::Nullable(Box::new(inner))
    }
}

/// Apply compiler-recorded generic bindings, preserving canonical nullable forms.
/// Tooling uses this same operation when displaying specialized contracts.
pub fn substitute_resolved_type(
    ty: &ResolvedType,
    substitutions: &HashMap<String, ResolvedType>,
) -> ResolvedType {
    substitute_resolved_type_impl(ty, substitutions, None)
}

/// Trait composition binds `self` to the composing class without rewriting
/// parameters inside that receiver or other simultaneous replacements.
pub(crate) fn substitute_trait_type(
    ty: &ResolvedType,
    substitutions: &HashMap<String, ResolvedType>,
    receiver: &ResolvedType,
) -> ResolvedType {
    substitute_resolved_type_impl(ty, substitutions, Some(receiver))
}

fn substitute_resolved_type_impl(
    ty: &ResolvedType,
    substitutions: &HashMap<String, ResolvedType>,
    trait_receiver: Option<&ResolvedType>,
) -> ResolvedType {
    match ty {
        ResolvedType::TypeParameter(name) => substitutions
            .get(name)
            .cloned()
            .unwrap_or_else(|| ty.clone()),
        ResolvedType::TraitSelf(_) => trait_receiver.unwrap_or(ty).clone(),
        ResolvedType::Nullable(inner) => nullable_of(substitute_resolved_type_impl(
            inner,
            substitutions,
            trait_receiver,
        )),
        ResolvedType::TypedArray(inner) => ResolvedType::TypedArray(Box::new(
            substitute_resolved_type_impl(inner, substitutions, trait_receiver),
        )),
        ResolvedType::List(inner) => ResolvedType::List(Box::new(substitute_resolved_type_impl(
            inner,
            substitutions,
            trait_receiver,
        ))),
        ResolvedType::Dictionary(key, value) => ResolvedType::Dictionary(
            Box::new(substitute_resolved_type_impl(
                key,
                substitutions,
                trait_receiver,
            )),
            Box::new(substitute_resolved_type_impl(
                value,
                substitutions,
                trait_receiver,
            )),
        ),
        ResolvedType::Set(inner) => ResolvedType::Set(Box::new(substitute_resolved_type_impl(
            inner,
            substitutions,
            trait_receiver,
        ))),
        ResolvedType::SortedDictionary(key, value) => ResolvedType::SortedDictionary(
            Box::new(substitute_resolved_type_impl(
                key,
                substitutions,
                trait_receiver,
            )),
            Box::new(substitute_resolved_type_impl(
                value,
                substitutions,
                trait_receiver,
            )),
        ),
        ResolvedType::SortedSet(inner) => ResolvedType::SortedSet(Box::new(
            substitute_resolved_type_impl(inner, substitutions, trait_receiver),
        )),
        ResolvedType::PriorityQueue(inner) => ResolvedType::PriorityQueue(Box::new(
            substitute_resolved_type_impl(inner, substitutions, trait_receiver),
        )),
        ResolvedType::Deque(inner) => ResolvedType::Deque(Box::new(substitute_resolved_type_impl(
            inner,
            substitutions,
            trait_receiver,
        ))),
        ResolvedType::Interface(interface) => ResolvedType::Interface(InterfaceType::new(
            interface.name.clone(),
            interface
                .arguments
                .iter()
                .map(|argument| {
                    substitute_resolved_type_impl(argument, substitutions, trait_receiver)
                })
                .collect(),
        )),
        ResolvedType::Function(function) => {
            ResolvedType::Function(Box::new(crate::types::SemanticFunctionType {
                invocation_mode: function.invocation_mode,
                parameters: function
                    .parameters
                    .iter()
                    .map(|parameter| crate::types::SemanticFunctionParameter {
                        ownership_mode: parameter.ownership_mode,
                        ty: substitute_resolved_type_impl(
                            &parameter.ty,
                            substitutions,
                            trait_receiver,
                        ),
                    })
                    .collect(),
                return_type: substitute_resolved_type_impl(
                    &function.return_type,
                    substitutions,
                    trait_receiver,
                ),
                checked_effects: function
                    .checked_effects
                    .iter()
                    .map(|effect| {
                        substitute_resolved_type_impl(effect, substitutions, trait_receiver)
                    })
                    .collect(),
                return_borrow: function.return_borrow,
            }))
        }
        ResolvedType::SharedHandle(kind, payload) => ResolvedType::SharedHandle(
            *kind,
            Box::new(substitute_resolved_type_impl(
                payload,
                substitutions,
                trait_receiver,
            )),
        ),
        ResolvedType::Enum(enum_type) => ResolvedType::Enum(enum_type.map_arguments(|argument| {
            substitute_resolved_type_impl(argument, substitutions, trait_receiver)
        })),
        ResolvedType::Class(class) => ResolvedType::Class(ClassType::new(
            class.name.clone(),
            class
                .arguments
                .iter()
                .map(|argument| {
                    substitute_resolved_type_impl(argument, substitutions, trait_receiver)
                })
                .collect(),
        )),
        ResolvedType::Void
        | ResolvedType::Integer(_)
        | ResolvedType::Float(_)
        | ResolvedType::String
        | ResolvedType::Bytes
        | ResolvedType::ByteRegion(_)
        | ResolvedType::Bool
        | ResolvedType::Null
        | ResolvedType::Mixed
        | ResolvedType::Error
        | ResolvedType::InterfaceSelf(_)
        | ResolvedType::Unsupported => ty.clone(),
    }
}

#[cfg(test)]
mod specialization_tests {
    use super::*;

    #[test]
    fn enum_arguments_preserve_identity_through_substitution_and_interning() {
        let declaration = crate::enums::EnumId(3);
        let symbolic = ResolvedType::Enum(EnumType::with_arguments(
            declaration,
            "Packet",
            vec![
                ResolvedType::TypeParameter("T".into()),
                ResolvedType::Class(ClassType::new(
                    "Box",
                    vec![nullable_of(ResolvedType::TypeParameter("T".into()))],
                )),
            ],
        ));
        assert!(resolved_type_is_symbolic(&symbolic));
        assert_eq!(resolved_type_complexity(&symbolic), 5);
        let concrete = substitute_resolved_type(
            &symbolic,
            &HashMap::from([("T".into(), ResolvedType::String)]),
        );
        let expected = ResolvedType::Enum(EnumType::with_arguments(
            declaration,
            "Packet",
            vec![
                ResolvedType::String,
                ResolvedType::Class(ClassType::new(
                    "Box",
                    vec![nullable_of(ResolvedType::String)],
                )),
            ],
        ));
        assert_eq!(concrete, expected);
        assert!(!resolved_type_is_symbolic(&concrete));
        let mut types = TypeRegistry::new();
        let symbolic_id = types.intern_resolved(&symbolic);
        let concrete_id = types.intern_resolved(&concrete);
        assert_ne!(symbolic_id, concrete_id);
        assert_eq!(types.resolved(symbolic_id), symbolic);
        assert_eq!(types.resolved(concrete_id), concrete);
        assert_eq!(types.intern_resolved(&expected), concrete_id);
        assert_eq!(types.display(concrete_id), "Packet<string, Box<?string>>");
        let other = ResolvedType::Enum(EnumType::with_arguments(
            declaration,
            "Packet",
            vec![ResolvedType::Bool],
        ));
        assert_ne!(types.intern_resolved(&other), concrete_id);
        let plain = ResolvedType::Enum(EnumType::new(declaration, "Packet"));
        let plain_id = types.intern_resolved(&plain);
        assert_ne!(plain_id, concrete_id);
        assert_eq!(types.display(plain_id), "Packet");
        assert_eq!(types.resolved(plain_id), plain);
    }

    #[test]
    fn nominal_resolution_recurses_through_enum_arguments() {
        let status = EnumType::new(crate::enums::EnumId(1), "Status");
        let enums = HashMap::from([(ClassType::new("Status", Vec::new()), status.clone())]);
        let readable = InterfaceType::new("Readable", Vec::new());
        let interfaces = HashMap::from([(readable.clone(), ())]);
        let outer = EnumType::with_arguments(
            crate::enums::EnumId(2),
            "Packet",
            vec![
                ResolvedType::List(Box::new(ResolvedType::Class(ClassType::new(
                    "Status",
                    Vec::new(),
                )))),
                ResolvedType::Class(readable.clone()),
            ],
        );
        let actual = resolve_nominal_kinds(ResolvedType::Enum(outer), &enums, &interfaces);
        let expected = ResolvedType::Enum(EnumType::with_arguments(
            crate::enums::EnumId(2),
            "Packet",
            vec![
                ResolvedType::List(Box::new(ResolvedType::Enum(status))),
                ResolvedType::Interface(readable),
            ],
        ));
        assert_eq!(actual, expected);
    }

    #[test]
    fn nominal_enum_resolution_requires_the_recorded_type_application() {
        let status = EnumType::new(crate::enums::EnumId(1), "Status");
        let packet =
            |argument| EnumType::with_arguments(crate::enums::EnumId(2), "Packet", vec![argument]);
        let first = packet(ResolvedType::Bool);
        let second = packet(ResolvedType::Enum(status.clone()));
        let enums = [status, first.clone(), second.clone()]
            .into_iter()
            .map(|ty| (ClassType::new(ty.name.clone(), ty.arguments.clone()), ty))
            .collect();
        let interfaces = HashMap::<InterfaceType<ResolvedType>, ()>::new();
        for (arguments, expected) in [
            (vec![ResolvedType::Bool], first),
            (
                vec![ResolvedType::Class(ClassType::new("Status", Vec::new()))],
                second,
            ),
        ] {
            assert_eq!(
                resolve_nominal_kinds(
                    ResolvedType::Class(ClassType::new("Packet", arguments)),
                    &enums,
                    &interfaces,
                ),
                ResolvedType::Enum(expected),
            );
        }
        for arguments in [Vec::new(), vec![ResolvedType::String]] {
            let missing = ResolvedType::Class(ClassType::new("Packet", arguments));
            assert_eq!(
                resolve_nominal_kinds(missing.clone(), &enums, &interfaces),
                missing,
            );
        }
    }

    #[test]
    fn checked_callable_resolution_preserves_contracts_inside_type_wrappers() {
        let span = Span::default();
        let syntax = TypeRef::function(FunctionTypeRef {
            keyword_span: span,
            invocation_mode: FunctionInvocationMode::Writable,
            invocation_modifier_span: Some(span),
            parameter_list_open_span: span,
            parameter_list_close_span: span,
            parameter_list_span: span,
            parameters: vec![FunctionTypeParameterRef {
                ownership_mode: FunctionTypeParameterMode::Take,
                ownership_modifier_span: Some(span),
                ty: TypeRef::named("T"),
                type_span: span,
                span,
            }],
            colon_span: span,
            return_type: Box::new(TypeRef::named("T")),
            return_type_span: span,
            throws_clause: None,
            span,
        });
        let checked = ResolvedType::Function(Box::new(SemanticFunctionType {
            invocation_mode: FunctionInvocationMode::Writable,
            parameters: vec![SemanticFunctionParameter {
                ownership_mode: FunctionTypeParameterMode::Take,
                ty: ResolvedType::TypeParameter("T".to_string()),
            }],
            return_type: ResolvedType::TypeParameter("T".to_string()),
            checked_effects: vec![ResolvedType::Error],
            return_borrow: Some(FunctionReturnBorrow {
                source: FunctionBorrowSource::Parameter(0),
                writable: true,
                kind: ReturnBorrowKind::Retained,
                lifetime: RetainedLifetime::LastUse,
            }),
        }));
        let substitutions = HashMap::from([(
            "T".to_string(),
            ResolvedType::ByteRegion(ByteRegionKind::Writable),
        )]);
        let expected = substitute_resolved_type(&checked, &substitutions);
        let wrapped = TypeRef::generic(
            "List",
            vec![TypeRef::grouped(syntax.clone().nullable(), span, span).nullable()],
        );
        assert_eq!(
            resolved_type_ref_with_checked_functions(&wrapped, &substitutions, &|at| {
                assert_eq!(at, span);
                Some(checked.clone())
            }),
            Some(ResolvedType::List(Box::new(nullable_of(expected))))
        );
        assert_eq!(
            resolved_type_ref_with_checked_functions(&wrapped, &substitutions, &|_| None),
            None
        );
        let ResolvedType::Function(syntactic) =
            resolved_type_ref_with_substitutions(&syntax, &substitutions).unwrap()
        else {
            panic!("expected a syntactic function type");
        };
        assert_eq!(syntactic.return_borrow, None);
        assert_eq!(syntactic.return_type, substitutions["T"]);
    }

    #[test]
    fn substitutions_reach_every_runtime_type_constructor() {
        let parameter = ResolvedType::TypeParameter("T".to_string());
        let constructors: &[fn(ResolvedType) -> ResolvedType] = &[
            |inner| ResolvedType::Nullable(Box::new(inner)),
            |inner| ResolvedType::TypedArray(Box::new(inner)),
            |inner| ResolvedType::List(Box::new(inner)),
            |inner| ResolvedType::Dictionary(Box::new(inner.clone()), Box::new(inner)),
            |inner| ResolvedType::SortedDictionary(Box::new(inner.clone()), Box::new(inner)),
            |inner| ResolvedType::Set(Box::new(inner)),
            |inner| ResolvedType::SortedSet(Box::new(inner)),
            |inner| ResolvedType::PriorityQueue(Box::new(inner)),
            |inner| ResolvedType::Deque(Box::new(inner)),
            |inner| ResolvedType::Class(ClassType::new("Box", vec![inner])),
            |inner| ResolvedType::Interface(InterfaceType::new("Read", vec![inner])),
            |inner| {
                ResolvedType::Enum(EnumType::with_arguments(
                    crate::enums::EnumId(0),
                    "Packet",
                    vec![inner],
                ))
            },
            |inner| {
                ResolvedType::Function(Box::new(SemanticFunctionType {
                    invocation_mode: FunctionInvocationMode::Readonly,
                    parameters: vec![SemanticFunctionParameter {
                        ownership_mode: FunctionTypeParameterMode::Take,
                        ty: inner.clone(),
                    }],
                    return_type: inner.clone(),
                    checked_effects: vec![inner],
                    return_borrow: None,
                }))
            },
        ];
        let substitutions = HashMap::from([("T".to_string(), ResolvedType::String)]);
        let trait_self = ResolvedType::TraitSelf("Inspection".into());
        let receiver = ResolvedType::Class(ClassType::new("Record", vec![ResolvedType::String]));
        for outer in constructors {
            for inner in constructors {
                let symbolic = outer(inner(parameter.clone()));
                assert!(resolved_type_is_symbolic(&symbolic), "{symbolic:?}");
                let concrete = substitute_resolved_type(&symbolic, &substitutions);
                let expected =
                    substitute_resolved_type(&outer(inner(ResolvedType::String)), &HashMap::new());
                assert_eq!(concrete, expected);
                assert!(!resolved_type_is_symbolic(&concrete), "{concrete:?}");
                let authored = outer(inner(trait_self.clone()));
                assert_eq!(
                    substitute_resolved_type(&authored, &substitutions),
                    substitute_resolved_type(&authored, &HashMap::new()),
                );
                assert_eq!(
                    substitute_trait_type(&authored, &substitutions, &receiver),
                    substitute_resolved_type(&outer(inner(receiver.clone())), &HashMap::new()),
                );
            }
        }
        for kind in SharedHandleKind::ALL {
            let symbolic =
                ResolvedType::SharedHandle(kind, Box::new(constructors[10](parameter.clone())));
            assert!(resolved_type_is_symbolic(&symbolic));
            assert_eq!(
                substitute_resolved_type(&symbolic, &substitutions),
                ResolvedType::SharedHandle(kind, Box::new(constructors[10](ResolvedType::String)))
            );
            let authored =
                ResolvedType::SharedHandle(kind, Box::new(constructors[10](trait_self.clone())));
            assert_eq!(
                substitute_trait_type(&authored, &substitutions, &receiver),
                ResolvedType::SharedHandle(kind, Box::new(constructors[10](receiver.clone())))
            );
        }
    }

    #[test]
    fn trait_receivers_preserve_owner_parameters_during_simultaneous_method_substitution() {
        let owner = ResolvedType::TypeParameter("U".into());
        let receiver = ResolvedType::Class(ClassType::new("Record", vec![owner]));
        let bindings = HashMap::from([("U".into(), ResolvedType::String)]);
        let self_type = ResolvedType::TraitSelf("Inspection".into());
        let authored = ResolvedType::Interface(InterfaceType::new(
            "Ready",
            vec![self_type, ResolvedType::TypeParameter("U".into())],
        ));
        assert_eq!(
            substitute_trait_type(&authored, &bindings, &receiver),
            ResolvedType::Interface(InterfaceType::new(
                "Ready",
                vec![receiver.clone(), ResolvedType::String]
            ))
        );
        let interface_self = ResolvedType::InterfaceSelf("Ready".into());
        assert_eq!(
            substitute_trait_type(&interface_self, &bindings, &receiver),
            interface_self
        );
    }
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
        ResolvedType::Class(class) | ResolvedType::Interface(class) => {
            1 + class
                .arguments
                .iter()
                .map(resolved_type_complexity)
                .sum::<usize>()
        }
        ResolvedType::Enum(enum_type) => {
            1 + enum_type
                .arguments
                .iter()
                .map(resolved_type_complexity)
                .sum::<usize>()
        }
        ResolvedType::Function(function) => {
            1 + function
                .parameters
                .iter()
                .map(|parameter| resolved_type_complexity(&parameter.ty))
                .sum::<usize>()
                + resolved_type_complexity(&function.return_type)
                + function
                    .checked_effects
                    .iter()
                    .map(resolved_type_complexity)
                    .sum::<usize>()
        }
        ResolvedType::Integer(_)
        | ResolvedType::Float(_)
        | ResolvedType::Bool
        | ResolvedType::String
        | ResolvedType::Bytes
        | ResolvedType::ByteRegion(_)
        | ResolvedType::Mixed
        | ResolvedType::Error
        | ResolvedType::Void
        | ResolvedType::Null
        | ResolvedType::TypeParameter(_)
        | ResolvedType::InterfaceSelf(_)
        | ResolvedType::TraitSelf(_)
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
        // Nullable adds absence to a type; applying it again adds no state.
        if let TypeKind::Nullable(inner) = &kind {
            if matches!(self.kind(*inner), TypeKind::Nullable(_) | TypeKind::Null) {
                return *inner;
            }
        }
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

    pub(crate) fn kinds(&self) -> &[TypeKind] {
        &self.kinds
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

    fn display_nominal(&self, name: &str, arguments: &[TypeId]) -> String {
        if arguments.is_empty() {
            name.to_owned()
        } else {
            format!(
                "{}<{}>",
                name,
                arguments
                    .iter()
                    .map(|argument| self.display(*argument))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    }

    pub fn display(&self, id: TypeId) -> String {
        match self.kind(id) {
            TypeKind::Void => "void".to_string(),
            TypeKind::Integer(integer) => integer.source_name().to_string(),
            TypeKind::Float(float) => float.source_name().to_string(),
            TypeKind::String => "string".to_string(),
            TypeKind::Bytes => "Bytes".to_string(),
            TypeKind::ByteRegion(kind) => kind.source_name().to_string(),
            TypeKind::Nullable(inner) => format!("?{}", self.display(*inner)),
            TypeKind::Bool => "bool".to_string(),
            TypeKind::Null => "null".to_string(),
            TypeKind::Mixed => "mixed".to_string(),
            TypeKind::Error => "Error".to_string(),
            TypeKind::TypedArray(element) => format!("{}[]", self.display(*element)),
            TypeKind::Unknown => "Unknown".to_string(),
            TypeKind::Heterogeneous => "heterogeneous".to_string(),
            TypeKind::EmptyCollection => "[]".to_string(),
            TypeKind::TypeParameter(name) => {
                crate::trait_composition::authored_type_parameter_name(name).to_owned()
            }
            TypeKind::Function(function) => self.display_function(function),
            TypeKind::Enum(enum_type) => {
                self.display_nominal(&enum_type.name, &enum_type.arguments)
            }
            TypeKind::Class(class) | TypeKind::Interface(class) => {
                self.display_nominal(&class.name, &class.arguments)
            }
            TypeKind::List(element) => format!("List<{}>", self.display(*element)),
            TypeKind::InterfaceSelf(_) | TypeKind::TraitSelf(_) => "self".to_string(),
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

    fn display_function(&self, function: &SemanticFunctionType<TypeId>) -> String {
        let invocation = match function.invocation_mode {
            FunctionInvocationMode::Readonly => "",
            FunctionInvocationMode::Writable => " writable",
            FunctionInvocationMode::Once => " once",
        };
        let parameters = function
            .parameters
            .iter()
            .map(|parameter| {
                let ownership = match parameter.ownership_mode {
                    FunctionTypeParameterMode::Readonly => "",
                    FunctionTypeParameterMode::Writable => "writable ",
                    FunctionTypeParameterMode::Take => "take ",
                };
                format!("{ownership}{}", self.display(parameter.ty))
            })
            .collect::<Vec<_>>()
            .join(", ");
        let mut display = format!(
            "function{invocation}({parameters}): {}",
            self.display(function.return_type)
        );
        if !function.checked_effects.is_empty() {
            display.push_str(" throws ");
            display.push_str(
                &function
                    .checked_effects
                    .iter()
                    .map(|effect| self.display(*effect))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
        }
        display
    }

    pub fn resolved(&self, id: TypeId) -> ResolvedType {
        match self.kind(id) {
            TypeKind::Void => ResolvedType::Void,
            TypeKind::Integer(ty) => ResolvedType::Integer(*ty),
            TypeKind::Float(ty) => ResolvedType::Float(*ty),
            TypeKind::String => ResolvedType::String,
            TypeKind::Bytes => ResolvedType::Bytes,
            TypeKind::ByteRegion(kind) => ResolvedType::ByteRegion(*kind),
            TypeKind::Bool => ResolvedType::Bool,
            TypeKind::Null => ResolvedType::Null,
            TypeKind::Mixed => ResolvedType::Mixed,
            TypeKind::Error => ResolvedType::Error,
            TypeKind::TypeParameter(name) => ResolvedType::TypeParameter(name.clone()),
            TypeKind::Function(function) => {
                ResolvedType::Function(Box::new(SemanticFunctionType {
                    invocation_mode: function.invocation_mode,
                    parameters: function
                        .parameters
                        .iter()
                        .map(|parameter| SemanticFunctionParameter {
                            ownership_mode: parameter.ownership_mode,
                            ty: self.resolved(parameter.ty),
                        })
                        .collect(),
                    return_type: self.resolved(function.return_type),
                    checked_effects: function
                        .checked_effects
                        .iter()
                        .map(|effect| self.resolved(*effect))
                        .collect(),
                    return_borrow: function.return_borrow,
                }))
            }
            TypeKind::Enum(enum_type) => {
                ResolvedType::Enum(enum_type.map_arguments(|argument| self.resolved(*argument)))
            }
            TypeKind::InterfaceSelf(name) => ResolvedType::InterfaceSelf(name.clone()),
            TypeKind::TraitSelf(name) => ResolvedType::TraitSelf(name.clone()),
            TypeKind::Interface(interface) => ResolvedType::Interface(InterfaceType::new(
                interface.name.clone(),
                interface
                    .arguments
                    .iter()
                    .map(|argument| self.resolved(*argument))
                    .collect(),
            )),
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

    pub fn intern_resolved(&mut self, ty: &ResolvedType) -> TypeId {
        let kind = match ty {
            ResolvedType::Void => TypeKind::Void,
            ResolvedType::Integer(ty) => TypeKind::Integer(*ty),
            ResolvedType::Float(ty) => TypeKind::Float(*ty),
            ResolvedType::String => TypeKind::String,
            ResolvedType::Bytes => TypeKind::Bytes,
            ResolvedType::ByteRegion(kind) => TypeKind::ByteRegion(*kind),
            ResolvedType::Bool => TypeKind::Bool,
            ResolvedType::Null => TypeKind::Null,
            ResolvedType::Mixed => TypeKind::Mixed,
            ResolvedType::Error => TypeKind::Error,
            ResolvedType::TypeParameter(name) => TypeKind::TypeParameter(name.clone()),
            ResolvedType::Function(function) => {
                let parameters = function
                    .parameters
                    .iter()
                    .map(|parameter| SemanticFunctionParameter {
                        ownership_mode: parameter.ownership_mode,
                        ty: self.intern_resolved(&parameter.ty),
                    })
                    .collect();
                let return_type = self.intern_resolved(&function.return_type);
                let checked_effects = function
                    .checked_effects
                    .iter()
                    .map(|effect| self.intern_resolved(effect))
                    .collect();
                TypeKind::Function(SemanticFunctionType {
                    invocation_mode: function.invocation_mode,
                    parameters,
                    return_type,
                    checked_effects,
                    return_borrow: function.return_borrow,
                })
            }
            ResolvedType::Enum(ty) => {
                TypeKind::Enum(ty.map_arguments(|argument| self.intern_resolved(argument)))
            }
            ResolvedType::InterfaceSelf(name) => TypeKind::InterfaceSelf(name.clone()),
            ResolvedType::TraitSelf(name) => TypeKind::TraitSelf(name.clone()),
            ResolvedType::Interface(interface) => TypeKind::Interface(InterfaceType::new(
                interface.name.clone(),
                interface
                    .arguments
                    .iter()
                    .map(|argument| self.intern_resolved(argument))
                    .collect(),
            )),
            ResolvedType::Nullable(inner) => {
                let inner = self.intern_resolved(inner);
                TypeKind::Nullable(inner)
            }
            ResolvedType::Class(class) => TypeKind::Class(ClassType::new(
                class.name.clone(),
                class
                    .arguments
                    .iter()
                    .map(|argument| self.intern_resolved(argument))
                    .collect(),
            )),
            ResolvedType::TypedArray(element) => {
                let element = self.intern_resolved(element);
                TypeKind::TypedArray(element)
            }
            ResolvedType::List(element) => {
                let element = self.intern_resolved(element);
                TypeKind::List(element)
            }
            ResolvedType::Dictionary(key, value) => {
                let key = self.intern_resolved(key);
                let value = self.intern_resolved(value);
                TypeKind::Dictionary(key, value)
            }
            ResolvedType::SortedDictionary(key, value) => {
                let key = self.intern_resolved(key);
                let value = self.intern_resolved(value);
                TypeKind::SortedDictionary(key, value)
            }
            ResolvedType::Set(element) => {
                let element = self.intern_resolved(element);
                TypeKind::Set(element)
            }
            ResolvedType::SortedSet(element) => {
                let element = self.intern_resolved(element);
                TypeKind::SortedSet(element)
            }
            ResolvedType::PriorityQueue(element) => {
                let element = self.intern_resolved(element);
                TypeKind::PriorityQueue(element)
            }
            ResolvedType::Deque(element) => {
                let element = self.intern_resolved(element);
                TypeKind::Deque(element)
            }
            ResolvedType::SharedHandle(kind, payload) => {
                let payload = self.intern_resolved(payload);
                TypeKind::SharedHandle(*kind, payload)
            }
            ResolvedType::Unsupported => TypeKind::Unknown,
        };
        self.intern(kind)
    }
}

use std::collections::{HashMap, HashSet};

use crate::ast::MemberAccess;
use crate::numeric::IntegerValue;
use crate::source::Span;
use crate::types::{ResolvedType, TypeId, TypeRef};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BindingId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClosureId {
    pub start: usize,
    pub end: usize,
}

impl ClosureId {
    pub const fn from_span(span: Span) -> Self {
        Self {
            start: span.start,
            end: span.end,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LexicalOwner {
    TopLevel,
    Callable(usize),
    Closure(ClosureId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BindingKind {
    FunctionParameter,
    MethodParameter,
    ClosureParameter,
    Local,
    GroupedLocal,
    ForeachKey,
    ForeachValue,
    MatchBinding,
    CatchBinding,
    GivenBinding,
    LoopBinding,
    ClosureCapture,
    MethodReceiver,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BindingOwnership {
    Owned,
    ReadonlyBorrow,
    WritableBorrow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingDeclaration {
    pub id: BindingId,
    pub name: String,
    /// Source declaration location. Synthetic identities such as the method
    /// receiver deliberately have no invented source declaration span.
    pub span: Option<Span>,
    pub kind: BindingKind,
    pub writable: bool,
    pub ownership: BindingOwnership,
    pub owner: LexicalOwner,
    pub source_type: Option<ResolvedType>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BindingResolution {
    pub declarations_by_id: HashMap<BindingId, BindingDeclaration>,
    pub uses_by_span: HashMap<(usize, usize), BindingId>,
    pub declaration_by_span: HashMap<(usize, usize), BindingId>,
    pub closure_owners: HashMap<ClosureId, LexicalOwner>,
    pub lexical_parents: HashMap<LexicalOwner, LexicalOwner>,
}

#[derive(Debug, Clone)]
pub struct Binding {
    pub id: BindingId,
    pub kind: BindingKind,
    pub ownership: BindingOwnership,
    pub owner: LexicalOwner,
    pub writable: bool,
    pub ty: TypeId,
    pub declared_ty: TypeId,
    pub int_constant: Option<IntegerValue>,
    pub string_constant: Option<String>,
}

impl Binding {
    pub fn unresolved(
        writable: bool,
        ty: TypeId,
        declared_ty: TypeId,
        int_constant: Option<IntegerValue>,
        string_constant: Option<String>,
    ) -> Self {
        Self {
            id: BindingId(usize::MAX),
            kind: BindingKind::Local,
            ownership: BindingOwnership::Owned,
            owner: LexicalOwner::TopLevel,
            writable,
            ty,
            declared_ty,
            int_constant,
            string_constant,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ClassInfo {
    pub type_params: Vec<TypeParamInfo>,
    pub builtin_interfaces: HashSet<BuiltinInterface>,
    pub properties: HashMap<String, PropertyInfo>,
    pub static_properties: HashMap<String, StaticPropertyInfo>,
    pub constants: HashMap<String, ConstantInfo>,
    pub methods: HashMap<String, MethodInfo>,
    pub members: HashMap<String, MemberDeclaration>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BuiltinInterface {
    Displayable,
    Error,
}

impl ClassInfo {
    pub fn implements(&self, interface: BuiltinInterface) -> bool {
        self.builtin_interfaces.contains(&interface)
    }
}

#[derive(Debug, Clone)]
pub struct TypeParamInfo {
    pub name: String,
    pub constraints: Vec<TypeRef>,
}

#[derive(Debug, Clone)]
pub struct MemberDeclaration {
    pub kind: MemberKind,
    pub span: crate::source::Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberKind {
    InstanceProperty,
    StaticProperty,
    Constant,
    InstanceMethod,
    StaticMethod,
    PromotedProperty,
}

impl MemberKind {
    pub const fn description(self) -> &'static str {
        match self {
            Self::InstanceProperty => "instance property",
            Self::StaticProperty => "static property",
            Self::Constant => "class constant",
            Self::InstanceMethod => "instance method",
            Self::StaticMethod => "static method",
            Self::PromotedProperty => "promoted property",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PropertyInfo {
    pub access: MemberAccess,
    pub writable: bool,
    pub ty: TypeId,
    pub init_state: PropertyInitState,
    pub declaration_span: Span,
}

#[derive(Debug, Clone)]
pub struct StaticPropertyInfo {
    pub access: MemberAccess,
    pub writable: bool,
    pub ty: TypeId,
}

#[derive(Debug, Clone)]
pub struct ConstantInfo {
    pub access: MemberAccess,
    pub ty: TypeId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyInitState {
    Uninitialized,
    HasInitializer,
    PromotedParameter,
}

#[derive(Debug, Clone)]
pub struct ParamInfo {
    pub name: String,
    pub ty: TypeId,
    pub take: bool,
    pub writable: bool,
    pub has_default: bool,
}

#[derive(Debug, Clone)]
pub struct FunctionInfo {
    pub declaration: usize,
    pub type_params: Vec<TypeParamInfo>,
    pub params: Vec<ParamInfo>,
    pub return_ty: TypeId,
    pub return_borrow: Option<ReturnBorrow>,
    pub checked_effects: Vec<TypeId>,
}

#[derive(Debug, Clone)]
pub struct MethodInfo {
    pub declaration: usize,
    pub access: MemberAccess,
    pub receiver_mode: Option<ReceiverMode>,
    pub return_borrow: Option<ReturnBorrow>,
    pub is_static: bool,
    pub enclosing_type_bindings: HashMap<String, TypeId>,
    pub type_params: Vec<TypeParamInfo>,
    pub params: Vec<ParamInfo>,
    pub return_ty: TypeId,
    pub checked_effects: Vec<TypeId>,
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
pub enum ReceiverMode {
    Readonly,
    Writable,
    /// Reserved representation point for a future accepted consuming receiver.
    UnsupportedConsuming,
}

impl ReceiverMode {
    pub const fn is_writable(self) -> bool {
        matches!(self, Self::Writable)
    }
}

#[derive(Debug, Default, Clone)]
pub struct ScopeStack {
    scopes: Vec<HashMap<String, Binding>>,
}

impl ScopeStack {
    pub fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()],
        }
    }

    pub fn push(&mut self) {
        self.scopes.push(HashMap::new());
    }

    pub fn pop(&mut self) {
        self.scopes.pop();
    }

    pub fn declare(&mut self, name: String, binding: Binding) -> bool {
        if let Some(scope) = self.scopes.last_mut() {
            if scope.contains_key(&name) {
                return false;
            }
            scope.insert(name, binding);
            true
        } else {
            false
        }
    }

    pub fn contains_in_current_scope(&self, name: &str) -> bool {
        self.scopes
            .last()
            .is_some_and(|scope| scope.contains_key(name))
    }

    pub fn lookup(&self, name: &str) -> Option<&Binding> {
        self.scopes.iter().rev().find_map(|scope| scope.get(name))
    }

    pub fn lookup_mut(&mut self, name: &str) -> Option<&mut Binding> {
        self.scopes
            .iter_mut()
            .rev()
            .find_map(|scope| scope.get_mut(name))
    }

    pub fn replace_types_from_branches<F>(&mut self, branches: &[ScopeStack], mut merge_type: F)
    where
        F: FnMut(TypeId, TypeId) -> TypeId,
    {
        for (scope_index, scope) in self.scopes.iter_mut().enumerate() {
            for (name, binding) in scope.iter_mut() {
                let mut merged = None;
                for branch in branches {
                    let Some(branch_binding) = branch
                        .scopes
                        .get(scope_index)
                        .and_then(|scope| scope.get(name))
                    else {
                        continue;
                    };

                    merged = Some(match merged {
                        Some(current) => merge_type(current, branch_binding.ty),
                        None => branch_binding.ty,
                    });
                }

                if let Some(ty) = merged {
                    binding.ty = ty;
                }
            }
        }
    }
}

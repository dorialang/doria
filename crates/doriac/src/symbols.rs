use std::collections::HashMap;

use crate::ast::MemberAccess;
use crate::numeric::IntegerValue;
use crate::types::TypeId;

#[derive(Debug, Clone)]
pub struct Binding {
    pub writable: bool,
    pub ty: TypeId,
    pub declared_ty: TypeId,
    pub int_constant: Option<IntegerValue>,
    pub string_constant: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ClassInfo {
    pub implements_displayable: bool,
    pub properties: HashMap<String, PropertyInfo>,
    pub static_properties: HashMap<String, StaticPropertyInfo>,
    pub constants: HashMap<String, ConstantInfo>,
    pub methods: HashMap<String, MethodInfo>,
    pub members: HashMap<String, MemberDeclaration>,
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
    pub params: Vec<ParamInfo>,
    pub return_ty: TypeId,
    pub return_borrow: Option<ReturnBorrow>,
}

#[derive(Debug, Clone)]
pub struct MethodInfo {
    pub access: MemberAccess,
    pub receiver_mode: Option<ReceiverMode>,
    pub return_borrow: Option<ReturnBorrow>,
    pub is_static: bool,
    pub params: Vec<ParamInfo>,
    pub return_ty: TypeId,
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

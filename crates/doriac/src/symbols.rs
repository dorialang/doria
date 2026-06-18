use std::collections::HashMap;

use crate::ast::MemberAccess;
use crate::types::TypeId;

#[derive(Debug, Clone)]
pub struct Binding {
    pub writable: bool,
    pub ty: TypeId,
}

#[derive(Debug, Clone)]
pub struct ClassInfo {
    pub properties: HashMap<String, PropertyInfo>,
    pub methods: HashMap<String, MethodInfo>,
}

#[derive(Debug, Clone)]
pub struct PropertyInfo {
    pub access: MemberAccess,
    pub writable: bool,
    pub ty: TypeId,
    pub init_state: PropertyInitState,
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
    pub has_default: bool,
}

#[derive(Debug, Clone)]
pub struct FunctionInfo {
    pub params: Vec<ParamInfo>,
    pub return_ty: TypeId,
}

#[derive(Debug, Clone)]
pub struct MethodInfo {
    pub access: MemberAccess,
    pub writable_this: bool,
    pub params: Vec<ParamInfo>,
    pub return_ty: TypeId,
}

#[derive(Debug, Default)]
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
}

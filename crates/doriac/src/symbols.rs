use std::collections::HashMap;

use crate::ast::MemberAccess;
use crate::types::TypeId;

#[derive(Debug, Clone)]
pub struct Binding {
    pub writable: bool,
    pub ty: TypeId,
    pub int_constant: Option<i64>,
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

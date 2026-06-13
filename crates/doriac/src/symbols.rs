use std::collections::HashMap;

use crate::types::TypeRef;

#[derive(Debug, Clone)]
pub struct Binding {
    pub writable: bool,
    pub ty: TypeRef,
}

#[derive(Debug, Clone)]
pub struct ClassInfo {
    pub properties: HashMap<String, PropertyInfo>,
    pub methods: HashMap<String, MethodInfo>,
}

#[derive(Debug, Clone)]
pub struct PropertyInfo {
    pub writable: bool,
    pub ty: TypeRef,
}

#[derive(Debug, Clone)]
pub struct MethodInfo {
    pub writable_this: bool,
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

    pub fn declare(&mut self, name: String, binding: Binding) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, binding);
        }
    }

    pub fn lookup(&self, name: &str) -> Option<&Binding> {
        self.scopes.iter().rev().find_map(|scope| scope.get(name))
    }
}

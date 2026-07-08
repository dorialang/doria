use std::collections::{HashMap, HashSet};

use crate::ast::*;
use crate::diagnostics::{Diagnostic, DiagnosticResult};
use crate::source::Span;
use crate::symbols::{
    Binding, ClassInfo, FunctionInfo, MethodInfo, ParamInfo, PropertyInfo, PropertyInitState,
    ScopeStack,
};
use crate::types::{TypeId, TypeKind, TypeRef, TypeRegistry};

pub fn check_program(program: &Program) -> DiagnosticResult<()> {
    let mut checker = Checker::new(program);
    checker.check();
    if checker.diagnostics.is_empty() {
        Ok(())
    } else {
        Err(checker.diagnostics)
    }
}

struct Checker<'program> {
    program: &'program Program,
    classes: HashMap<String, ClassInfo>,
    functions: HashMap<String, FunctionInfo>,
    function_signatures: HashMap<usize, FunctionInfo>,
    types: TypeRegistry,
    diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone)]
struct MethodContext {
    class_name: String,
    writable_this: bool,
    this_available: bool,
}

#[derive(Debug, Clone)]
struct ConstructorInitContext {
    class_name: String,
    readonly_init_allowed: bool,
    initialized: HashSet<String>,
}

impl ConstructorInitContext {
    fn without_readonly_init(&self) -> Self {
        Self {
            class_name: self.class_name.clone(),
            readonly_init_allowed: false,
            initialized: HashSet::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConstructorInitDecision {
    Allowed,
    Rejected,
    NotApplicable,
}

#[derive(Debug, Clone)]
struct ReturnContext {
    name: String,
    expected: Option<TypeId>,
    lifecycle: Option<LifecycleMethod>,
    is_method: bool,
}

impl ReturnContext {
    fn kind_name(&self) -> &'static str {
        if self.is_method {
            "method"
        } else {
            "function"
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LifecycleMethod {
    Constructor,
    Destructor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TypePosition {
    Return,
    Value,
}

impl LifecycleMethod {
    fn from_method_name(name: &str) -> Option<Self> {
        match name {
            "__construct" => Some(Self::Constructor),
            "__destruct" => Some(Self::Destructor),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Constructor => "constructor",
            Self::Destructor => "destructor",
        }
    }

    fn doria_name(self) -> &'static str {
        match self {
            Self::Constructor => "__construct",
            Self::Destructor => "__destruct",
        }
    }

    fn return_value_message(self) -> &'static str {
        match self {
            Self::Constructor => "constructors cannot return a value",
            Self::Destructor => "destructors cannot return a value",
        }
    }
}

#[derive(Debug, Clone)]
struct AssignmentTarget {
    ty: TypeId,
    destination: AssignmentDestination,
}

#[derive(Debug, Clone)]
enum AssignmentDestination {
    Type,
    Parameter { name: String },
    Property { class_name: String, name: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IntConstantEval {
    Known(i64),
    Unknown,
    Invalid,
}

impl<'program> Checker<'program> {
    fn new(program: &'program Program) -> Self {
        Self {
            program,
            classes: HashMap::new(),
            functions: HashMap::new(),
            function_signatures: HashMap::new(),
            types: TypeRegistry::new(),
            diagnostics: Vec::new(),
        }
    }

    fn check(&mut self) {
        self.collect_classes();
        self.collect_functions();
        self.infer_unannotated_mixed_return_signatures();

        let mut scopes = ScopeStack::new();
        for item in &self.program.items {
            match item {
                Item::Statement(statement) => {
                    self.check_statement(statement, &mut scopes, None, None, None, 0);
                }
                Item::Function(function) => self.check_function(function, None),
                Item::Class(class_decl) => self.check_class(class_decl),
            }
        }
    }

    fn collect_classes(&mut self) {
        let mut declared_classes = Vec::new();

        for item in &self.program.items {
            let Item::Class(class_decl) = item else {
                continue;
            };

            if let Some(message) = Self::reserved_class_name_message(&class_decl.name) {
                self.diagnostics
                    .push(Diagnostic::new("E0309", message, class_decl.span));
                continue;
            }

            if self.classes.contains_key(&class_decl.name) {
                self.diagnostics.push(Diagnostic::new(
                    "E0300",
                    format!("class `{}` is already declared", class_decl.name),
                    class_decl.span,
                ));
                continue;
            }

            self.classes.insert(
                class_decl.name.clone(),
                ClassInfo {
                    properties: HashMap::new(),
                    methods: HashMap::new(),
                },
            );
            declared_classes.push(class_decl);
        }

        for class_decl in declared_classes {
            let mut info = ClassInfo {
                properties: HashMap::new(),
                methods: HashMap::new(),
            };

            for member in &class_decl.members {
                match member {
                    ClassMember::Property(property) => {
                        self.declare_property(&mut info, &class_decl.name, property);
                    }
                    ClassMember::Method(method) => {
                        let signature = self.resolve_function_signature(method);
                        self.function_signatures
                            .insert(method.span.start, signature.clone());

                        if method.name == "__destruct" && !method.params.is_empty() {
                            self.diagnostics.push(Diagnostic::new(
                                "E0411",
                                "destructor `__destruct` cannot declare parameters",
                                method.span,
                            ));
                        }

                        if info.methods.contains_key(&method.name) {
                            self.diagnostics.push(Diagnostic::new(
                                "E0302",
                                format!(
                                    "class `{}` already has a method `{}`",
                                    class_decl.name, method.name
                                ),
                                method.span,
                            ));
                        } else {
                            info.methods.insert(
                                method.name.clone(),
                                MethodInfo {
                                    access: method.access.clone(),
                                    writable_this: method.writable_this,
                                    params: signature.params,
                                    return_ty: signature.return_ty,
                                },
                            );
                        }

                        if method.name == "__construct" {
                            for param in &method.params {
                                if param.promoted_access.is_some() {
                                    self.declare_promoted_property(
                                        &mut info,
                                        &class_decl.name,
                                        param,
                                    );
                                }
                            }
                        }
                    }
                }
            }

            self.classes.insert(class_decl.name.clone(), info);
        }
    }

    fn reserved_class_name_message(name: &str) -> Option<String> {
        match name {
            "array" => Some(
                "`array` is not a Doria class name; use typed arrays like `T[]` or collection aliases"
                    .to_string(),
            ),
            "mixed" => Some(
                "`mixed` is a Doria dynamic-boundary type and cannot be used as a class name"
                    .to_string(),
            ),
            "object" => Some(
                "`object` is not a Doria type and cannot be used as a class name".to_string(),
            ),
            "resource" => Some(
                "`resource` is reserved for future PHP interop and cannot be used as a class name"
                    .to_string(),
            ),
            _ => None,
        }
    }

    fn collect_functions(&mut self) {
        for item in &self.program.items {
            let Item::Function(function) = item else {
                continue;
            };

            let signature = self.resolve_function_signature(function);
            self.function_signatures
                .insert(function.span.start, signature.clone());
            if self.functions.contains_key(&function.name) {
                self.diagnostics.push(Diagnostic::new(
                    "E0308",
                    format!("function `{}` is already declared", function.name),
                    function.span,
                ));
                continue;
            }

            self.functions.insert(function.name.clone(), signature);
        }
    }

    fn resolve_function_signature(&mut self, function: &FunctionDecl) -> FunctionInfo {
        let params = self.resolve_param_infos(function);
        let return_ty = self.resolve_function_return_type(function);

        FunctionInfo { params, return_ty }
    }

    fn resolve_param_infos(&mut self, function: &FunctionDecl) -> Vec<ParamInfo> {
        let mut params = Vec::new();
        let mut saw_optional = false;

        for param in &function.params {
            let ty = self.resolve_type_ref(&param.ty, param.span);
            let has_default = param.default.is_some();

            if !has_default && saw_optional {
                self.diagnostics.push(Diagnostic::new(
                    "E0410",
                    format!(
                        "required parameter `${}` cannot follow an optional parameter",
                        param.name
                    ),
                    param.span,
                ));
            }

            if has_default {
                saw_optional = true;
            }

            params.push(ParamInfo {
                name: param.name.clone(),
                ty,
                has_default,
            });
        }

        params
    }

    fn resolve_function_return_type(&mut self, function: &FunctionDecl) -> TypeId {
        function
            .return_type
            .as_ref()
            .map(|return_type| {
                self.resolve_type_ref_in_position(return_type, function.span, TypePosition::Return)
            })
            .unwrap_or_else(|| self.types.unknown())
    }

    fn infer_unannotated_mixed_return_signatures(&mut self) {
        let max_iterations = self.mixed_return_inference_signature_count();

        for _ in 0..max_iterations {
            let mut changed = false;

            for item in &self.program.items {
                match item {
                    Item::Function(function) => {
                        changed |= self.update_function_mixed_return_signature(function);
                    }
                    Item::Class(class_decl) => {
                        for member in &class_decl.members {
                            let ClassMember::Method(method) = member else {
                                continue;
                            };
                            changed |=
                                self.update_method_mixed_return_signature(&class_decl.name, method);
                        }
                    }
                    Item::Statement(_) => {}
                }
            }

            if !changed {
                break;
            }
        }
    }

    fn mixed_return_inference_signature_count(&self) -> usize {
        self.program
            .items
            .iter()
            .map(|item| match item {
                Item::Function(function) if function.return_type.is_none() => 1,
                Item::Class(class_decl) => class_decl
                    .members
                    .iter()
                    .filter(|member| match member {
                        ClassMember::Method(method) => {
                            method.return_type.is_none()
                                && LifecycleMethod::from_method_name(&method.name).is_none()
                        }
                        ClassMember::Property(_) => false,
                    })
                    .count(),
                _ => 0,
            })
            .sum::<usize>()
            .max(1)
    }

    fn update_function_mixed_return_signature(&mut self, function: &FunctionDecl) -> bool {
        if function.return_type.is_some() {
            return false;
        }

        let Some(signature) = self.function_signatures.get(&function.span.start).cloned() else {
            return false;
        };
        let inferred = self.infer_unannotated_mixed_return_type(function, &signature.params, None);

        if !self.type_contains_mixed(inferred) || signature.return_ty == inferred {
            return false;
        }

        if let Some(signature) = self.function_signatures.get_mut(&function.span.start) {
            signature.return_ty = inferred;
        }
        if let Some(function_info) = self.functions.get_mut(&function.name) {
            function_info.return_ty = inferred;
        }
        true
    }

    fn update_method_mixed_return_signature(
        &mut self,
        class_name: &str,
        method: &FunctionDecl,
    ) -> bool {
        if method.return_type.is_some() || LifecycleMethod::from_method_name(&method.name).is_some()
        {
            return false;
        }

        let Some(signature) = self.function_signatures.get(&method.span.start).cloned() else {
            return false;
        };
        let method_context = MethodContext {
            class_name: class_name.to_string(),
            writable_this: method.writable_this,
            this_available: true,
        };
        let inferred = self.infer_unannotated_mixed_return_type(
            method,
            &signature.params,
            Some(&method_context),
        );

        if !self.type_contains_mixed(inferred) || signature.return_ty == inferred {
            return false;
        }

        if let Some(signature) = self.function_signatures.get_mut(&method.span.start) {
            signature.return_ty = inferred;
        }
        if let Some(method_info) = self
            .classes
            .get_mut(class_name)
            .and_then(|class_info| class_info.methods.get_mut(&method.name))
        {
            method_info.return_ty = inferred;
        }
        true
    }

    fn infer_unannotated_mixed_return_type(
        &mut self,
        function: &FunctionDecl,
        params: &[ParamInfo],
        method_context: Option<&MethodContext>,
    ) -> TypeId {
        let mut scopes = ScopeStack::new();
        for param in params {
            let _ = scopes.declare(
                param.name.clone(),
                Binding {
                    writable: false,
                    ty: param.ty,
                    int_constant: None,
                },
            );
        }

        self.infer_mixed_return_from_statements(
            &function.body.statements,
            &mut scopes,
            method_context,
        )
        .unwrap_or_else(|| self.types.unknown())
    }

    fn infer_mixed_return_from_statements(
        &mut self,
        statements: &[Stmt],
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> Option<TypeId> {
        let mut inferred = None;

        for statement in statements {
            let statement_ty =
                self.infer_mixed_return_from_statement(statement, scopes, method_context);
            inferred = self.merge_optional_mixed_return_types(inferred, statement_ty);

            if Self::statement_is_terminal(statement) {
                break;
            }
        }

        inferred
    }

    fn infer_mixed_return_from_statement(
        &mut self,
        statement: &Stmt,
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> Option<TypeId> {
        match statement {
            Stmt::VarDecl(decl) => {
                let ty = self.infer_local_declaration_type(
                    decl.ty.as_ref(),
                    &decl.initializer,
                    scopes,
                    method_context,
                );
                let _ = scopes.declare(
                    decl.name.clone(),
                    Binding {
                        writable: decl.writable,
                        ty,
                        int_constant: None,
                    },
                );
                None
            }
            Stmt::Assignment(assignment) => {
                if matches!(assignment.op, AssignOp::Assign) {
                    if let Expr::Variable { name, .. } = &assignment.target {
                        let ty = self.infer_expr_type(&assignment.value, scopes, method_context);
                        if let Some(binding) = scopes.lookup_mut(name) {
                            binding.ty = self.merge_inferred_binding_type(binding.ty, ty);
                        }
                    }
                }
                None
            }
            Stmt::Return {
                expr: Some(expr), ..
            } => {
                let ty = self.infer_expr_type(expr, scopes, method_context);
                self.type_contains_mixed(ty).then_some(ty)
            }
            Stmt::If(if_stmt) => {
                let mut inferred =
                    self.infer_mixed_return_from_block(&if_stmt.then_block, scopes, method_context);

                if let Some(branch) = &if_stmt.else_branch {
                    let branch_ty =
                        self.infer_mixed_return_from_else_branch(branch, scopes, method_context);
                    inferred = self.merge_optional_mixed_return_types(inferred, branch_ty);
                }

                inferred
            }
            Stmt::While(while_stmt) => {
                self.infer_mixed_return_from_block(&while_stmt.body, scopes, method_context)
            }
            Stmt::For(for_stmt) => {
                scopes.push();
                if let Some(initializer) = &for_stmt.initializer {
                    self.infer_mixed_return_from_for_initializer(
                        initializer,
                        scopes,
                        method_context,
                    );
                }
                let result =
                    self.infer_mixed_return_from_block(&for_stmt.body, scopes, method_context);
                scopes.pop();
                result
            }
            Stmt::Foreach(foreach) => {
                self.infer_mixed_return_from_foreach(foreach, scopes, method_context)
            }
            _ => None,
        }
    }

    fn infer_mixed_return_from_block(
        &mut self,
        block: &Block,
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> Option<TypeId> {
        scopes.push();
        let inferred =
            self.infer_mixed_return_from_statements(&block.statements, scopes, method_context);
        scopes.pop();
        inferred
    }

    fn infer_mixed_return_from_else_branch(
        &mut self,
        branch: &ElseBranch,
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> Option<TypeId> {
        match branch {
            ElseBranch::If(if_stmt) => self.infer_mixed_return_from_statement(
                &Stmt::If((**if_stmt).clone()),
                scopes,
                method_context,
            ),
            ElseBranch::Block(block) => {
                self.infer_mixed_return_from_block(block, scopes, method_context)
            }
        }
    }

    fn infer_mixed_return_from_for_initializer(
        &mut self,
        initializer: &ForInitializer,
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        match initializer {
            ForInitializer::VarDecl(decl) => {
                let ty = self.infer_local_declaration_type(
                    decl.ty.as_ref(),
                    &decl.initializer,
                    scopes,
                    method_context,
                );
                let _ = scopes.declare(
                    decl.name.clone(),
                    Binding {
                        writable: decl.writable,
                        ty,
                        int_constant: None,
                    },
                );
            }
            ForInitializer::Assignment(assignment) => {
                if matches!(assignment.op, AssignOp::Assign) {
                    if let Expr::Variable { name, .. } = &assignment.target {
                        let ty = self.infer_expr_type(&assignment.value, scopes, method_context);
                        if let Some(binding) = scopes.lookup_mut(name) {
                            binding.ty = self.merge_inferred_binding_type(binding.ty, ty);
                        }
                    }
                }
            }
        }
    }

    fn infer_mixed_return_from_foreach(
        &mut self,
        foreach: &ForeachStmt,
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> Option<TypeId> {
        scopes.push();
        let (inferred_key, inferred_value) =
            self.infer_foreach_binding_types(foreach, scopes, method_context);
        if let Some(key) = &foreach.key {
            let ty = key
                .ty
                .as_ref()
                .map(|ty| self.resolve_type_ref_for_return_inference(ty))
                .unwrap_or(inferred_key);
            let _ = scopes.declare(
                key.name.clone(),
                Binding {
                    writable: false,
                    ty,
                    int_constant: None,
                },
            );
        }

        let value_ty = foreach
            .value
            .ty
            .as_ref()
            .map(|ty| self.resolve_type_ref_for_return_inference(ty))
            .unwrap_or(inferred_value);
        let _ = scopes.declare(
            foreach.value.name.clone(),
            Binding {
                writable: false,
                ty: value_ty,
                int_constant: None,
            },
        );

        let inferred = self.infer_mixed_return_from_statements(
            &foreach.body.statements,
            scopes,
            method_context,
        );
        scopes.pop();
        inferred
    }

    fn merge_optional_mixed_return_types(
        &mut self,
        current: Option<TypeId>,
        next: Option<TypeId>,
    ) -> Option<TypeId> {
        let next = next?;
        if !self.type_contains_mixed(next) {
            return current;
        }

        Some(match current {
            Some(current) => self.merge_mixed_return_types(current, next),
            None => next,
        })
    }

    fn merge_mixed_return_types(&mut self, left: TypeId, right: TypeId) -> TypeId {
        if left == right {
            return left;
        }

        let left_kind = self.types.kind(left).clone();
        let right_kind = self.types.kind(right).clone();
        match (left_kind, right_kind) {
            (TypeKind::List(left), TypeKind::List(right)) => {
                let element = self.merge_mixed_return_types(left, right);
                self.types.intern(TypeKind::List(element))
            }
            (TypeKind::TypedArray(left), TypeKind::TypedArray(right)) => {
                let element = self.merge_mixed_return_types(left, right);
                self.types.intern(TypeKind::TypedArray(element))
            }
            (
                TypeKind::Dictionary(left_key, left_value),
                TypeKind::Dictionary(right_key, right_value),
            ) => {
                let key = self.merge_mixed_return_types(left_key, right_key);
                let value = self.merge_mixed_return_types(left_value, right_value);
                self.types.intern(TypeKind::Dictionary(key, value))
            }
            (TypeKind::Set(left), TypeKind::Set(right)) => {
                let element = self.merge_mixed_return_types(left, right);
                self.types.intern(TypeKind::Set(element))
            }
            _ => self.types.intern(TypeKind::Mixed),
        }
    }

    fn infer_local_declaration_type(
        &mut self,
        annotation: Option<&TypeRef>,
        initializer: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> TypeId {
        let initializer_ty = self.infer_expr_type(initializer, scopes, method_context);
        annotation.map_or(initializer_ty, |ty| {
            self.resolve_type_ref_for_return_inference(ty)
        })
    }

    fn merge_inferred_binding_type(&mut self, current: TypeId, next: TypeId) -> TypeId {
        if self.type_contains_mixed(current) {
            self.merge_mixed_return_types(current, next)
        } else {
            next
        }
    }

    fn infer_foreach_binding_types(
        &mut self,
        foreach: &ForeachStmt,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> (TypeId, TypeId) {
        let unknown = self.types.unknown();
        if Self::is_grouped_range_expr(&foreach.iterable) {
            let int = self.types.intern(TypeKind::Int);
            return (unknown, int);
        }

        let iterable_ty = self.infer_expr_type(&foreach.iterable, scopes, method_context);
        match self.types.kind(iterable_ty).clone() {
            TypeKind::List(value) | TypeKind::Set(value) => {
                (self.types.intern(TypeKind::Int), value)
            }
            TypeKind::TypedArray(value) => (self.types.intern(TypeKind::Int), value),
            TypeKind::Dictionary(key, value) => (key, value),
            TypeKind::Mixed => (unknown, self.types.intern(TypeKind::Mixed)),
            _ => (unknown, unknown),
        }
    }

    fn resolve_type_ref_for_return_inference(&mut self, ty: &TypeRef) -> TypeId {
        match ty.name.as_str() {
            "void" if ty.args.is_empty() => self.types.intern(TypeKind::Void),
            "int" if ty.args.is_empty() => self.types.intern(TypeKind::Int),
            "float" if ty.args.is_empty() => self.types.intern(TypeKind::Float),
            "string" if ty.args.is_empty() => self.types.intern(TypeKind::String),
            "bool" if ty.args.is_empty() => self.types.intern(TypeKind::Bool),
            "mixed" if ty.args.is_empty() => self.types.intern(TypeKind::Mixed),
            "[]" if ty.args.len() == 1 => {
                let element = self.resolve_type_ref_for_return_inference(&ty.args[0]);
                self.types.intern(TypeKind::TypedArray(element))
            }
            "List" if ty.args.len() == 1 => {
                let element = self.resolve_type_ref_for_return_inference(&ty.args[0]);
                self.types.intern(TypeKind::List(element))
            }
            "Dictionary" if ty.args.len() == 2 => {
                let key = self.resolve_type_ref_for_return_inference(&ty.args[0]);
                let value = self.resolve_type_ref_for_return_inference(&ty.args[1]);
                self.types.intern(TypeKind::Dictionary(key, value))
            }
            "Set" if ty.args.len() == 1 => {
                let element = self.resolve_type_ref_for_return_inference(&ty.args[0]);
                self.types.intern(TypeKind::Set(element))
            }
            name if ty.args.is_empty() && self.classes.contains_key(name) => {
                self.types.intern(TypeKind::Class(name.to_string()))
            }
            _ => self.types.unknown(),
        }
    }

    fn check_class(&mut self, class_decl: &ClassDecl) {
        for member in &class_decl.members {
            match member {
                ClassMember::Property(property) => {
                    self.check_property_initializer(&class_decl.name, property);
                }
                ClassMember::Method(method) => {
                    self.check_function(
                        method,
                        Some(MethodContext {
                            class_name: class_decl.name.clone(),
                            writable_this: method.writable_this,
                            this_available: true,
                        }),
                    );
                }
            }
        }
    }

    fn check_property_initializer(&mut self, class_name: &str, property: &PropertyDecl) {
        let Some(initializer) = &property.initializer else {
            return;
        };

        let scopes = ScopeStack::new();
        let initializer_context = MethodContext {
            class_name: class_name.to_string(),
            writable_this: false,
            this_available: false,
        };
        self.check_expr(initializer, &scopes, Some(&initializer_context));
        let target_ty = self
            .classes
            .get(class_name)
            .and_then(|class_info| class_info.properties.get(&property.name))
            .map(|property| property.ty)
            .unwrap_or_else(|| self.resolve_type_ref(&property.ty, property.span));
        self.check_expr_assignable(
            target_ty,
            initializer,
            &scopes,
            Some(&initializer_context),
            AssignmentDestination::Property {
                class_name: class_name.to_string(),
                name: property.name.clone(),
            },
        );
    }

    fn declare_property(
        &mut self,
        info: &mut ClassInfo,
        class_name: &str,
        property: &PropertyDecl,
    ) {
        if info.properties.contains_key(&property.name) {
            self.diagnostics.push(Diagnostic::new(
                "E0301",
                format!(
                    "class `{class_name}` already has a property `${}`",
                    property.name
                ),
                property.span,
            ));
            return;
        }

        let ty = self.resolve_type_ref(&property.ty, property.span);
        info.properties.insert(
            property.name.clone(),
            PropertyInfo {
                access: property.access.clone(),
                writable: property.writable,
                ty,
                init_state: if property.initializer.is_some() {
                    PropertyInitState::HasInitializer
                } else {
                    PropertyInitState::Uninitialized
                },
            },
        );
    }

    fn declare_promoted_property(&mut self, info: &mut ClassInfo, class_name: &str, param: &Param) {
        if info.properties.contains_key(&param.name) {
            self.diagnostics.push(Diagnostic::new(
                "E0301",
                format!(
                    "class `{class_name}` already has a property `${}`",
                    param.name
                ),
                param.span,
            ));
            return;
        }

        let ty = self.resolve_type_ref(&param.ty, param.span);
        info.properties.insert(
            param.name.clone(),
            PropertyInfo {
                access: param
                    .promoted_access
                    .clone()
                    .unwrap_or(MemberAccess::External),
                writable: param.writable,
                ty,
                init_state: PropertyInitState::PromotedParameter,
            },
        );
    }

    fn declare_binding(
        &mut self,
        scopes: &mut ScopeStack,
        name: String,
        binding: Binding,
        span: Span,
    ) {
        if !scopes.declare(name.clone(), binding) {
            self.diagnostics.push(Diagnostic::new(
                "E0103",
                format!("variable `${name}` is already declared in this scope"),
                span,
            ));
        }
    }

    fn check_function(&mut self, function: &FunctionDecl, method_context: Option<MethodContext>) {
        let mut scopes = ScopeStack::new();
        let signature = self.current_function_signature(function);
        let return_context = self.return_context_for_function(function, method_context.as_ref());
        for (param, param_info) in function.params.iter().zip(signature.params.iter()) {
            let ty = param_info.ty;
            if let Some(default) = &param.default {
                let default_context = method_context.as_ref().map(|context| MethodContext {
                    class_name: context.class_name.clone(),
                    writable_this: context.writable_this,
                    this_available: false,
                });
                let default_context = default_context.as_ref();

                self.check_expr(default, &scopes, default_context);
                self.check_expr_assignable(
                    ty,
                    default,
                    &scopes,
                    default_context,
                    AssignmentDestination::Parameter {
                        name: param.name.clone(),
                    },
                );
            }
            self.declare_binding(
                &mut scopes,
                param.name.clone(),
                Binding {
                    writable: param.writable,
                    ty,
                    int_constant: None,
                },
                param.span,
            );
        }
        let mut constructor_init_context = method_context.as_ref().and_then(|context| {
            (function.name == "__construct").then(|| ConstructorInitContext {
                class_name: context.class_name.clone(),
                readonly_init_allowed: true,
                initialized: HashSet::new(),
            })
        });

        self.check_block(
            &function.body,
            &mut scopes,
            method_context.as_ref(),
            constructor_init_context.as_mut(),
            Some(&return_context),
            0,
        );
        self.check_missing_final_return(function, &return_context);
    }

    fn return_context_for_function(
        &mut self,
        function: &FunctionDecl,
        method_context: Option<&MethodContext>,
    ) -> ReturnContext {
        let is_method = method_context.is_some();
        let lifecycle = is_method
            .then(|| LifecycleMethod::from_method_name(&function.name))
            .flatten();
        let name = method_context
            .map(|context| format!("{}::{}", context.class_name, function.name))
            .unwrap_or_else(|| function.name.clone());

        if let Some(lifecycle) = lifecycle {
            self.check_lifecycle_return_type(function, lifecycle);
        }

        let expected = if lifecycle.is_some() {
            None
        } else if function.return_type.is_some() {
            Some(self.current_function_return_type(function))
        } else {
            None
        };

        ReturnContext {
            name,
            expected,
            lifecycle,
            is_method,
        }
    }

    fn check_lifecycle_return_type(&mut self, function: &FunctionDecl, lifecycle: LifecycleMethod) {
        if function.return_type.is_none() {
            return;
        }

        let return_ty = self.current_function_return_type(function);

        if self.is_void_type(return_ty) {
            return;
        }

        self.diagnostics.push(
            Diagnostic::new(
                "E0407",
                format!(
                    "{} `{}` cannot declare non-void return type",
                    lifecycle.label(),
                    lifecycle.doria_name()
                ),
                function.span,
            )
            .with_help(format!(
                "remove the return type annotation or use `{}(): void`",
                lifecycle.doria_name()
            )),
        );
    }

    fn current_function_return_type(&mut self, function: &FunctionDecl) -> TypeId {
        self.current_function_signature(function).return_ty
    }

    fn current_function_signature(&mut self, function: &FunctionDecl) -> FunctionInfo {
        self.function_signatures
            .get(&function.span.start)
            .cloned()
            .unwrap_or_else(|| self.resolve_function_signature(function))
    }

    fn check_block(
        &mut self,
        block: &Block,
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
        mut constructor_init_context: Option<&mut ConstructorInitContext>,
        return_context: Option<&ReturnContext>,
        loop_depth: usize,
    ) {
        scopes.push();
        for statement in &block.statements {
            self.check_statement(
                statement,
                scopes,
                method_context,
                constructor_init_context.as_deref_mut(),
                return_context,
                loop_depth,
            );
        }
        scopes.pop();
    }

    fn check_statement(
        &mut self,
        statement: &Stmt,
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
        constructor_init_context: Option<&mut ConstructorInitContext>,
        return_context: Option<&ReturnContext>,
        loop_depth: usize,
    ) {
        match statement {
            Stmt::VarDecl(decl) => {
                self.check_expr(&decl.initializer, scopes, method_context);
                let value_ty = self.infer_expr_type(&decl.initializer, scopes, method_context);
                let ty = match &decl.ty {
                    Some(ty) => {
                        let target_ty = self.resolve_type_ref(ty, decl.span);
                        self.check_expr_assignable(
                            target_ty,
                            &decl.initializer,
                            scopes,
                            method_context,
                            AssignmentDestination::Type,
                        );
                        target_ty
                    }
                    None => value_ty,
                };
                self.declare_binding(
                    scopes,
                    decl.name.clone(),
                    Binding {
                        writable: decl.writable,
                        ty,
                        int_constant: self.readonly_int_constant(
                            decl.writable,
                            ty,
                            &decl.initializer,
                            scopes,
                        ),
                    },
                    decl.span,
                );
            }
            Stmt::Assignment(assignment) => {
                self.check_expr(&assignment.value, scopes, method_context);
                if let Some(target) = self.check_assignment_target(
                    &assignment.target,
                    &assignment.op,
                    scopes,
                    method_context,
                    constructor_init_context,
                ) {
                    self.check_assignment_value(assignment, target, scopes, method_context);
                }
            }
            Stmt::Echo { expr, .. } => {
                self.check_expr(expr, scopes, method_context);
                self.check_mixed_value_operation(expr, "echo", scopes, method_context);
            }
            Stmt::Expr { expr, .. } => {
                self.check_expr(expr, scopes, method_context);
            }
            Stmt::Return { expr, span } => {
                self.check_return_statement(
                    expr.as_ref(),
                    *span,
                    scopes,
                    method_context,
                    return_context,
                );
            }
            Stmt::If(if_stmt) => {
                self.check_condition(&if_stmt.condition, scopes, method_context);
                let mut then_constructor_init_context = constructor_init_context
                    .as_deref()
                    .map(ConstructorInitContext::without_readonly_init);
                self.check_block(
                    &if_stmt.then_block,
                    scopes,
                    method_context,
                    then_constructor_init_context.as_mut(),
                    return_context,
                    loop_depth,
                );
                if let Some(else_branch) = &if_stmt.else_branch {
                    self.check_else_branch(
                        else_branch,
                        scopes,
                        method_context,
                        constructor_init_context.as_deref(),
                        return_context,
                        loop_depth,
                    );
                }
            }
            Stmt::While(while_stmt) => {
                self.check_condition(&while_stmt.condition, scopes, method_context);
                let mut loop_constructor_init_context = constructor_init_context
                    .as_deref()
                    .map(ConstructorInitContext::without_readonly_init);
                self.check_block(
                    &while_stmt.body,
                    scopes,
                    method_context,
                    loop_constructor_init_context.as_mut(),
                    return_context,
                    loop_depth + 1,
                );
            }
            Stmt::For(for_stmt) => {
                scopes.push();
                if let Some(initializer) = &for_stmt.initializer {
                    self.check_for_initializer(initializer, scopes, method_context, None);
                }
                if let Some(condition) = &for_stmt.condition {
                    self.check_condition(condition, scopes, method_context);
                }
                let mut loop_constructor_init_context = constructor_init_context
                    .as_deref()
                    .map(ConstructorInitContext::without_readonly_init);
                self.check_block(
                    &for_stmt.body,
                    scopes,
                    method_context,
                    loop_constructor_init_context.as_mut(),
                    return_context,
                    loop_depth + 1,
                );
                if let Some(increment) = &for_stmt.increment {
                    self.check_for_increment(increment, scopes, method_context);
                }
                scopes.pop();
            }
            Stmt::Break { span } => {
                if loop_depth == 0 {
                    self.diagnostics.push(Diagnostic::new(
                        "E0421",
                        "`break` may only be used inside a loop",
                        *span,
                    ));
                }
            }
            Stmt::Continue { span } => {
                if loop_depth == 0 {
                    self.diagnostics.push(Diagnostic::new(
                        "E0422",
                        "`continue` may only be used inside a loop",
                        *span,
                    ));
                }
            }
            Stmt::Foreach(foreach) => {
                let range_iterable = Self::is_grouped_range_expr(&foreach.iterable);
                let unknown_ty = self.types.unknown();
                let int_ty = self.types.intern(TypeKind::Int);
                let (iterable_key_ty, iterable_value_ty) = if range_iterable {
                    (unknown_ty, int_ty)
                } else {
                    self.infer_foreach_binding_types(foreach, scopes, method_context)
                };

                if range_iterable {
                    self.check_expr_with_range_context(
                        &foreach.iterable,
                        scopes,
                        method_context,
                        true,
                    );
                } else {
                    self.check_expr(&foreach.iterable, scopes, method_context);
                    self.check_mixed_operation(
                        &foreach.iterable,
                        "foreach iterable",
                        scopes,
                        method_context,
                    );
                }
                scopes.push();
                if let Some(key) = &foreach.key {
                    let ty = if range_iterable {
                        self.diagnostics.push(Diagnostic::new(
                            "E0425",
                            "foreach over integer ranges does not support key bindings",
                            foreach.span,
                        ));
                        self.types.unknown()
                    } else {
                        key.ty.as_ref().map_or(iterable_key_ty, |ty| {
                            let annotated_ty = self.resolve_type_ref(ty, foreach.span);
                            self.check_foreach_binding_type(
                                annotated_ty,
                                iterable_key_ty,
                                foreach.span,
                            );
                            annotated_ty
                        })
                    };
                    self.declare_binding(
                        scopes,
                        key.name.clone(),
                        Binding {
                            writable: false,
                            ty,
                            int_constant: None,
                        },
                        foreach.span,
                    );
                }
                let value_ty = if range_iterable {
                    if let Some(annotation) = &foreach.value.ty {
                        let annotated_ty = self.resolve_type_ref(annotation, foreach.span);
                        self.check_foreach_binding_type(annotated_ty, int_ty, foreach.span);
                    }
                    int_ty
                } else {
                    foreach.value.ty.as_ref().map_or(iterable_value_ty, |ty| {
                        let annotated_ty = self.resolve_type_ref(ty, foreach.span);
                        self.check_foreach_binding_type(
                            annotated_ty,
                            iterable_value_ty,
                            foreach.span,
                        );
                        annotated_ty
                    })
                };
                self.declare_binding(
                    scopes,
                    foreach.value.name.clone(),
                    Binding {
                        writable: false,
                        ty: value_ty,
                        int_constant: None,
                    },
                    foreach.span,
                );
                let mut loop_constructor_init_context = constructor_init_context
                    .as_deref()
                    .map(ConstructorInitContext::without_readonly_init);
                for statement in &foreach.body.statements {
                    self.check_statement(
                        statement,
                        scopes,
                        method_context,
                        loop_constructor_init_context.as_mut(),
                        return_context,
                        loop_depth + 1,
                    );
                }
                scopes.pop();
            }
            Stmt::Increment(increment) => {
                self.check_increment_statement(increment, scopes, method_context);
            }
        }
    }

    fn check_for_initializer(
        &mut self,
        initializer: &ForInitializer,
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
        constructor_init_context: Option<&mut ConstructorInitContext>,
    ) {
        match initializer {
            ForInitializer::VarDecl(decl) => {
                self.check_expr(&decl.initializer, scopes, method_context);
                let value_ty = self.infer_expr_type(&decl.initializer, scopes, method_context);
                let ty = match &decl.ty {
                    Some(ty) => {
                        let target_ty = self.resolve_type_ref(ty, decl.span);
                        self.check_expr_assignable(
                            target_ty,
                            &decl.initializer,
                            scopes,
                            method_context,
                            AssignmentDestination::Type,
                        );
                        target_ty
                    }
                    None => value_ty,
                };
                self.declare_binding(
                    scopes,
                    decl.name.clone(),
                    Binding {
                        writable: decl.writable,
                        ty,
                        int_constant: self.readonly_int_constant(
                            decl.writable,
                            ty,
                            &decl.initializer,
                            scopes,
                        ),
                    },
                    decl.span,
                );
            }
            ForInitializer::Assignment(assignment) => {
                self.check_expr(&assignment.value, scopes, method_context);
                if let Some(target) = self.check_assignment_target(
                    &assignment.target,
                    &assignment.op,
                    scopes,
                    method_context,
                    constructor_init_context,
                ) {
                    self.check_assignment_value(assignment, target, scopes, method_context);
                }
            }
        }
    }

    fn check_for_increment(
        &mut self,
        increment: &ForIncrement,
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        match increment {
            ForIncrement::Increment(increment) => {
                self.check_increment_statement(increment, scopes, method_context);
            }
            ForIncrement::Assignment(assignment) => {
                self.check_expr(&assignment.value, scopes, method_context);
                if let Some(target) = self.check_assignment_target(
                    &assignment.target,
                    &assignment.op,
                    scopes,
                    method_context,
                    None,
                ) {
                    self.check_assignment_value(assignment, target, scopes, method_context);
                }
            }
        }
    }
    fn check_assignment_value(
        &mut self,
        assignment: &Assignment,
        target: AssignmentTarget,
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        match assignment.op {
            AssignOp::Assign => {
                let target_ty = target.ty;
                let destination = target.destination.clone();
                let assignment_ok = self.check_expr_assignable(
                    target.ty,
                    &assignment.value,
                    scopes,
                    method_context,
                    destination,
                );
                if assignment_ok {
                    let value_ty = self.infer_expr_type(&assignment.value, scopes, method_context);
                    self.narrow_empty_collection_assignment(
                        &assignment.target,
                        target_ty,
                        value_ty,
                        scopes,
                    );
                }
            }
            AssignOp::AddAssign | AssignOp::SubAssign => {
                let value_ty = self.infer_expr_type(&assignment.value, scopes, method_context);
                let target_contains_mixed = self.type_contains_mixed(target.ty);
                let value_contains_mixed = self.type_contains_mixed(value_ty);

                if target_contains_mixed {
                    self.report_mixed_operation(assignment.target.span(), "compound assignment");
                }

                if value_contains_mixed {
                    self.report_mixed_operation(assignment.value.span(), "compound assignment");
                }

                if target_contains_mixed || value_contains_mixed {
                    return;
                }

                let result_ty = self.infer_numeric_binary_type(target.ty, value_ty);
                if !self.is_assignable(target.ty, result_ty) {
                    self.check_assignable(
                        target.ty,
                        result_ty,
                        assignment.value.span(),
                        target.destination,
                    );
                }
            }
        }
    }

    fn check_increment_statement(
        &mut self,
        increment: &IncrementStmt,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        self.check_increment_target(
            &increment.target,
            Self::increment_operator_name(&increment.op),
            scopes,
            method_context,
        );
    }

    fn check_increment_target(
        &mut self,
        target: &Expr,
        op_name: &'static str,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        match target {
            Expr::Grouped { expr, .. } => {
                self.check_increment_target(expr, op_name, scopes, method_context);
            }
            Expr::Variable { name, span } => {
                let Some(binding) = scopes.lookup(name) else {
                    self.undeclared_variable(name, *span);
                    return;
                };

                if !binding.writable {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0201",
                            format!("cannot increment readonly local `${name}`"),
                            *span,
                        )
                        .with_help(format!(
                            "declare it as `let writable ${name} = ...` if mutation is intended"
                        )),
                    );
                }

                if matches!(
                    self.types.kind(binding.ty),
                    TypeKind::Int | TypeKind::Unknown
                ) {
                    return;
                }

                self.diagnostics.push(Diagnostic::new(
                    "E0423",
                    format!("{op_name} requires writable int target"),
                    *span,
                ));
            }
            _ => {
                self.check_expr(target, scopes, method_context);
                self.diagnostics.push(Diagnostic::new(
                    "E0204",
                    "unsupported increment target",
                    target.span(),
                ));
            }
        }
    }

    fn increment_operator_name(op: &IncrementOp) -> &'static str {
        match op {
            IncrementOp::Increment => "++",
            IncrementOp::Decrement => "--",
        }
    }

    fn check_else_branch(
        &mut self,
        branch: &ElseBranch,
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
        constructor_init_context: Option<&ConstructorInitContext>,
        return_context: Option<&ReturnContext>,
        loop_depth: usize,
    ) {
        match branch {
            ElseBranch::If(if_stmt) => {
                self.check_condition(&if_stmt.condition, scopes, method_context);
                let mut then_constructor_init_context =
                    constructor_init_context.map(ConstructorInitContext::without_readonly_init);
                self.check_block(
                    &if_stmt.then_block,
                    scopes,
                    method_context,
                    then_constructor_init_context.as_mut(),
                    return_context,
                    loop_depth,
                );
                if let Some(else_branch) = &if_stmt.else_branch {
                    self.check_else_branch(
                        else_branch,
                        scopes,
                        method_context,
                        constructor_init_context,
                        return_context,
                        loop_depth,
                    );
                }
            }
            ElseBranch::Block(block) => {
                let mut block_constructor_init_context =
                    constructor_init_context.map(ConstructorInitContext::without_readonly_init);
                self.check_block(
                    block,
                    scopes,
                    method_context,
                    block_constructor_init_context.as_mut(),
                    return_context,
                    loop_depth,
                );
            }
        }
    }

    fn check_condition(
        &mut self,
        condition: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        self.check_expr(condition, scopes, method_context);
        let ty = self.infer_expr_type(condition, scopes, method_context);
        if matches!(self.types.kind(ty), TypeKind::Bool | TypeKind::Unknown) {
            return;
        }

        self.diagnostics.push(Diagnostic::new(
            "E0416",
            format!("condition must be `bool`, got `{}`", self.types.display(ty)),
            condition.span(),
        ));
    }

    fn check_return_statement(
        &mut self,
        expr: Option<&Expr>,
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
        return_context: Option<&ReturnContext>,
    ) {
        let Some(context) = return_context else {
            if let Some(expr) = expr {
                self.check_expr(expr, scopes, method_context);
            }
            return;
        };

        if let Some(expr) = expr {
            self.check_expr(expr, scopes, method_context);
        }

        if let Some(lifecycle) = context.lifecycle {
            if expr.is_some() {
                self.diagnostics.push(Diagnostic::new(
                    "E0405",
                    lifecycle.return_value_message(),
                    span,
                ));
            }
            return;
        }

        let Some(expected) = context.expected else {
            return;
        };

        if self.is_void_type(expected) {
            if expr.is_some() {
                self.diagnostics.push(Diagnostic::new(
                    "E0405",
                    format!(
                        "cannot return a value from void {} `{}`",
                        context.kind_name(),
                        context.name
                    ),
                    span,
                ));
            }
            return;
        }

        let Some(expr) = expr else {
            self.report_missing_return_value(context, expected, span);
            return;
        };

        let value = self.infer_expr_type(expr, scopes, method_context);
        if self.is_expr_assignable(expected, expr, scopes, method_context)
            || self.is_assignable(expected, value)
        {
            return;
        }

        self.report_return_type_mismatch(context, expected, value, expr.span());
    }

    fn check_missing_final_return(&mut self, function: &FunctionDecl, context: &ReturnContext) {
        if context.lifecycle.is_some() {
            return;
        }

        let Some(expected) = context.expected else {
            return;
        };

        if !self.requires_return_value(expected) {
            return;
        }

        match function.body.statements.last() {
            Some(statement) if Self::statement_is_terminal(statement) => {}
            _ => self.report_missing_return_value(context, expected, function.span),
        }
    }

    fn statement_is_terminal(statement: &Stmt) -> bool {
        match statement {
            Stmt::Return { .. } => true,
            Stmt::If(if_stmt) => Self::if_statement_is_terminal(if_stmt),
            _ => false,
        }
    }

    fn if_statement_is_terminal(if_stmt: &IfStmt) -> bool {
        Self::block_is_terminal(&if_stmt.then_block)
            && if_stmt
                .else_branch
                .as_ref()
                .is_some_and(Self::else_branch_is_terminal)
    }

    fn block_is_terminal(block: &Block) -> bool {
        block
            .statements
            .last()
            .is_some_and(Self::statement_is_terminal)
    }

    fn else_branch_is_terminal(branch: &ElseBranch) -> bool {
        match branch {
            ElseBranch::If(if_stmt) => Self::if_statement_is_terminal(if_stmt),
            ElseBranch::Block(block) => Self::block_is_terminal(block),
        }
    }

    fn report_return_type_mismatch(
        &mut self,
        context: &ReturnContext,
        expected: TypeId,
        value: TypeId,
        span: Span,
    ) {
        self.diagnostics.push(Diagnostic::new(
            "E0404",
            format!(
                "cannot return value of type `{}` from {} `{}` with return type `{}`",
                self.types.display(value),
                context.kind_name(),
                context.name,
                self.types.display(expected)
            ),
            span,
        ));
    }

    fn report_missing_return_value(
        &mut self,
        context: &ReturnContext,
        expected: TypeId,
        span: Span,
    ) {
        self.diagnostics.push(Diagnostic::new(
            "E0406",
            format!(
                "{} `{}` must return a value of type `{}`",
                context.kind_name(),
                context.name,
                self.types.display(expected)
            ),
            span,
        ));
    }

    fn is_void_type(&self, ty: TypeId) -> bool {
        matches!(self.types.kind(ty), TypeKind::Void)
    }

    fn requires_return_value(&self, ty: TypeId) -> bool {
        !matches!(self.types.kind(ty), TypeKind::Void | TypeKind::Unknown)
    }

    fn check_expr(
        &mut self,
        expr: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        self.check_expr_with_range_context(expr, scopes, method_context, false);
    }

    fn check_expr_with_range_context(
        &mut self,
        expr: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
        allow_range_expr: bool,
    ) {
        match expr {
            Expr::Variable { name, span } => {
                if scopes.lookup(name).is_none() {
                    self.undeclared_variable(name, *span);
                }
            }
            Expr::This { span } => {
                if !method_context
                    .map(|context| context.this_available)
                    .unwrap_or(false)
                {
                    self.diagnostics.push(Diagnostic::new(
                        "E0102",
                        "`$this` is only available inside methods",
                        *span,
                    ));
                }
            }
            Expr::InterpolatedString { parts, .. } => {
                self.check_interpolated_string(parts, scopes, method_context);
            }
            Expr::Array { elements, .. } => {
                for element in elements {
                    if let Some(key) = &element.key {
                        self.check_expr(key, scopes, method_context);
                    }
                    self.check_expr(&element.value, scopes, method_context);
                }
            }
            Expr::PropertyAccess {
                object,
                property,
                span,
            } => {
                self.check_expr(object, scopes, method_context);
                self.check_mixed_operation(object, "property access", scopes, method_context);
                self.lookup_property(object, property, *span, scopes, method_context);
            }
            Expr::MethodCall {
                object,
                method,
                span,
                args,
            } => {
                self.check_expr(object, scopes, method_context);
                for arg in args {
                    self.check_expr(arg, scopes, method_context);
                }
                self.check_mixed_operation(object, "method call", scopes, method_context);
                self.check_method_call(object, method, args, *span, scopes, method_context);
            }
            Expr::FunctionCall { name, args, span } => {
                for arg in args {
                    self.check_expr(arg, scopes, method_context);
                }
                self.check_function_call(name, args, *span, scopes, method_context);
            }
            Expr::StaticCall {
                class_name,
                method,
                args,
                span,
            } => {
                for arg in args {
                    self.check_expr(arg, scopes, method_context);
                }
                self.check_static_call(class_name, method, args, *span, scopes, method_context);
            }
            Expr::New {
                class_name,
                args,
                span,
            } => {
                let class_exists = self.classes.contains_key(class_name);
                if !class_exists {
                    self.diagnostics.push(Diagnostic::new(
                        "E0305",
                        format!("unknown class `{class_name}`"),
                        *span,
                    ));
                }
                for arg in args {
                    self.check_expr(arg, scopes, method_context);
                }
                if class_exists {
                    self.check_constructor_call(class_name, args, *span, scopes, method_context);
                }
            }
            Expr::Grouped { expr, .. } => {
                self.check_expr_with_range_context(expr, scopes, method_context, allow_range_expr)
            }
            Expr::Unary { op, expr, .. } => {
                self.check_expr(expr, scopes, method_context);
                self.check_unary_operand(op, expr, scopes, method_context);
            }
            Expr::Binary {
                left,
                op,
                right,
                span,
            } => {
                self.check_expr(left, scopes, method_context);
                self.check_expr(right, scopes, method_context);
                self.check_int_constant_arithmetic(left, op, right, *span, scopes);
                self.check_mixed_binary_operands(left, right, *span, scopes, method_context);
                self.check_binary_operands(left, op, right, *span, scopes, method_context);
            }
            Expr::Range {
                start, end, span, ..
            } => {
                self.check_expr(start, scopes, method_context);
                self.check_expr(end, scopes, method_context);
                self.check_range_endpoint_type(start, scopes, method_context);
                self.check_range_endpoint_type(end, scopes, method_context);
                if !allow_range_expr {
                    self.diagnostics.push(Diagnostic::new(
                        "E0426",
                        "range expressions are only supported as foreach iterables",
                        *span,
                    ));
                }
            }
            Expr::Identifier { .. }
            | Expr::String { .. }
            | Expr::Float { .. }
            | Expr::Bool { .. }
            | Expr::Null { .. } => {}
            Expr::Int { value, span } => self.check_int_literal_range(value, *span),
        }
    }

    fn is_grouped_range_expr(expr: &Expr) -> bool {
        match expr {
            Expr::Grouped { expr, .. } => Self::is_grouped_range_expr(expr),
            Expr::Range { .. } => true,
            _ => false,
        }
    }

    fn check_range_endpoint_type(
        &mut self,
        expr: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let ty = self.infer_expr_type(expr, scopes, method_context);
        if matches!(self.types.kind(ty), TypeKind::Int | TypeKind::Unknown) {
            return;
        }

        self.diagnostics.push(Diagnostic::new(
            "E0424",
            "range endpoints must be int",
            expr.span(),
        ));
    }

    fn check_int_literal_range(&mut self, value: &str, span: Span) {
        if value.parse::<i64>().is_err() {
            self.diagnostics.push(Diagnostic::new(
                "E0417",
                "integer literal is outside the Doria `int` range",
                span,
            ));
        }
    }

    fn check_unary_operand(
        &mut self,
        op: &UnaryOp,
        expr: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        match op {
            UnaryOp::Not => {
                let ty = self.infer_expr_type(expr, scopes, method_context);
                if self.is_mixed_type(ty) {
                    self.report_mixed_operation(expr.span(), "boolean operator");
                    return;
                }

                if self.is_bool_or_recovery_type(ty) {
                    return;
                }

                self.diagnostics.push(Diagnostic::new(
                    "E0419",
                    format!(
                        "boolean operator `not`/`!` requires a `bool` operand, got `{}`",
                        self.types.display(ty)
                    ),
                    expr.span(),
                ));
            }
        }
    }

    fn check_binary_operands(
        &mut self,
        left: &Expr,
        op: &BinaryOp,
        right: &Expr,
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        if self.has_mixed_operand(left, right, scopes, method_context) {
            return;
        }

        match op {
            BinaryOp::And | BinaryOp::Or | BinaryOp::Xor => {
                self.check_logical_binary_operands(left, op, right, span, scopes, method_context);
            }
            BinaryOp::Equal | BinaryOp::NotEqual => {
                self.check_equality_operands(left, right, span, scopes, method_context);
            }
            BinaryOp::Concat => {
                self.check_concat_operands(left, right, span, scopes, method_context);
            }
            _ => {}
        }
    }

    fn check_logical_binary_operands(
        &mut self,
        left: &Expr,
        op: &BinaryOp,
        right: &Expr,
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let left_ty = self.infer_expr_type(left, scopes, method_context);
        let right_ty = self.infer_expr_type(right, scopes, method_context);
        if self.is_bool_or_recovery_type(left_ty) && self.is_bool_or_recovery_type(right_ty) {
            return;
        }

        self.diagnostics.push(Diagnostic::new(
            "E0419",
            format!(
                "boolean operator {} requires `bool` operands, got `{}` and `{}`",
                Self::logical_operator_name(op),
                self.types.display(left_ty),
                self.types.display(right_ty)
            ),
            span,
        ));
    }

    fn check_equality_operands(
        &mut self,
        left: &Expr,
        right: &Expr,
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let left_ty = self.infer_expr_type(left, scopes, method_context);
        let right_ty = self.infer_expr_type(right, scopes, method_context);
        if self.is_equality_compatible(left_ty, right_ty) {
            return;
        }

        self.diagnostics.push(Diagnostic::new(
            "E0420",
            format!(
                "equality operands must have compatible types, got `{}` and `{}`",
                self.types.display(left_ty),
                self.types.display(right_ty)
            ),
            span,
        ));
    }

    fn check_concat_operands(
        &mut self,
        left: &Expr,
        right: &Expr,
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let left_ty = self.infer_expr_type(left, scopes, method_context);
        let right_ty = self.infer_expr_type(right, scopes, method_context);
        if self.is_string_or_recovery_type(left_ty) && self.is_string_or_recovery_type(right_ty) {
            return;
        }

        self.diagnostics.push(Diagnostic::new(
            "E0425",
            format!(
                "string concatenation operator `.` requires `string` operands, got `{}` and `{}`",
                self.types.display(left_ty),
                self.types.display(right_ty)
            ),
            span,
        ));
    }

    fn is_bool_or_recovery_type(&self, ty: TypeId) -> bool {
        matches!(self.types.kind(ty), TypeKind::Bool | TypeKind::Unknown)
    }

    fn is_string_or_recovery_type(&self, ty: TypeId) -> bool {
        matches!(self.types.kind(ty), TypeKind::String | TypeKind::Unknown)
    }

    fn is_equality_compatible(&self, left: TypeId, right: TypeId) -> bool {
        if self.type_contains_mixed(left) || self.type_contains_mixed(right) {
            return false;
        }

        self.is_assignable(left, right) || self.is_assignable(right, left)
    }

    fn logical_operator_name(op: &BinaryOp) -> &'static str {
        match op {
            BinaryOp::And => "`and`/`&&`",
            BinaryOp::Or => "`or`/`||`",
            BinaryOp::Xor => "`xor`",
            _ => "logical operator",
        }
    }

    fn check_mixed_operation(
        &mut self,
        expr: &Expr,
        operation: &'static str,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let ty = self.infer_expr_type(expr, scopes, method_context);
        if self.is_mixed_type(ty) {
            self.report_mixed_operation(expr.span(), operation);
        }
    }

    fn check_mixed_value_operation(
        &mut self,
        expr: &Expr,
        operation: &'static str,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let ty = self.infer_expr_type(expr, scopes, method_context);
        if self.type_contains_mixed(ty) {
            self.report_mixed_operation(expr.span(), operation);
        }
    }

    fn check_mixed_binary_operands(
        &mut self,
        left: &Expr,
        right: &Expr,
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        if self.has_mixed_operand(left, right, scopes, method_context) {
            self.report_mixed_operation(span, "operator");
        }
    }

    fn has_mixed_operand(
        &mut self,
        left: &Expr,
        right: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> bool {
        let left_ty = self.infer_expr_type(left, scopes, method_context);
        let right_ty = self.infer_expr_type(right, scopes, method_context);

        self.type_contains_mixed(left_ty) || self.type_contains_mixed(right_ty)
    }

    fn is_mixed_type(&self, ty: TypeId) -> bool {
        matches!(self.types.kind(ty), TypeKind::Mixed)
    }

    fn type_contains_mixed(&self, ty: TypeId) -> bool {
        match self.types.kind(ty) {
            TypeKind::Mixed => true,
            TypeKind::TypedArray(element) | TypeKind::List(element) | TypeKind::Set(element) => {
                self.type_contains_mixed(*element)
            }
            TypeKind::Dictionary(key, value) => {
                self.type_contains_mixed(*key) || self.type_contains_mixed(*value)
            }
            _ => false,
        }
    }

    fn report_mixed_operation(&mut self, span: Span, operation: &'static str) {
        self.diagnostics.push(
            Diagnostic::new(
                "E0433",
                format!("cannot use `mixed` value in {operation} before narrowing"),
                span,
            )
            .with_help("narrow the value with `is` or `match` before using it"),
        );
    }

    fn check_foreach_binding_type(&mut self, target: TypeId, value: TypeId, span: Span) {
        if self.is_unknown_type(target) || self.is_unknown_type(value) {
            return;
        }

        if self.type_contains_mixed(value) && !self.type_contains_mixed(target) {
            self.report_mixed_operation(span, "foreach binding");
            return;
        }

        if !self.is_assignable(target, value) {
            self.check_assignable(target, value, span, AssignmentDestination::Type);
        }
    }

    fn check_int_constant_arithmetic(
        &mut self,
        left: &Expr,
        op: &BinaryOp,
        right: &Expr,
        span: Span,
        scopes: &ScopeStack,
    ) {
        if !Self::is_checked_int_arithmetic_op(op) {
            return;
        }

        let (IntConstantEval::Known(left), IntConstantEval::Known(right)) = (
            Self::eval_int_constant(left, scopes),
            Self::eval_int_constant(right, scopes),
        ) else {
            return;
        };

        if Self::checked_int_arithmetic(left, op, right).is_some() {
            return;
        }

        self.diagnostics.push(Diagnostic::new(
            "E0418",
            "integer arithmetic overflows the Doria `int` range",
            span,
        ));
    }

    fn readonly_int_constant(
        &self,
        writable: bool,
        ty: TypeId,
        initializer: &Expr,
        scopes: &ScopeStack,
    ) -> Option<i64> {
        if writable || !matches!(self.types.kind(ty), TypeKind::Int) {
            return None;
        }

        match Self::eval_int_constant(initializer, scopes) {
            IntConstantEval::Known(value) => Some(value),
            IntConstantEval::Unknown | IntConstantEval::Invalid => None,
        }
    }

    fn eval_int_constant(expr: &Expr, scopes: &ScopeStack) -> IntConstantEval {
        match expr {
            Expr::Int { value, .. } => value
                .parse::<i64>()
                .map(IntConstantEval::Known)
                .unwrap_or(IntConstantEval::Invalid),
            Expr::Variable { name, .. } => scopes
                .lookup(name)
                .and_then(|binding| binding.int_constant)
                .map(IntConstantEval::Known)
                .unwrap_or(IntConstantEval::Unknown),
            Expr::Grouped { expr, .. } => Self::eval_int_constant(expr, scopes),
            Expr::Binary {
                left, op, right, ..
            } if Self::is_checked_int_arithmetic_op(op) => {
                let left = Self::eval_int_constant(left, scopes);
                let right = Self::eval_int_constant(right, scopes);
                match (left, right) {
                    (IntConstantEval::Known(left), IntConstantEval::Known(right)) => {
                        Self::checked_int_arithmetic(left, op, right)
                            .map(IntConstantEval::Known)
                            .unwrap_or(IntConstantEval::Invalid)
                    }
                    (IntConstantEval::Invalid, _) | (_, IntConstantEval::Invalid) => {
                        IntConstantEval::Invalid
                    }
                    _ => IntConstantEval::Unknown,
                }
            }
            _ => IntConstantEval::Unknown,
        }
    }

    fn is_checked_int_arithmetic_op(op: &BinaryOp) -> bool {
        matches!(op, BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul)
    }

    fn checked_int_arithmetic(left: i64, op: &BinaryOp, right: i64) -> Option<i64> {
        match op {
            BinaryOp::Add => left.checked_add(right),
            BinaryOp::Sub => left.checked_sub(right),
            BinaryOp::Mul => left.checked_mul(right),
            _ => None,
        }
    }

    fn check_interpolated_string(
        &mut self,
        parts: &[InterpolatedStringPart],
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        for part in parts {
            let InterpolatedStringPart::Expr(expr) = part else {
                continue;
            };

            self.check_expr(expr, scopes, method_context);
            let ty = self.infer_expr_type(expr, scopes, method_context);
            if !self.is_interpolatable_type(ty) {
                let ty_name = self.types.display(ty);
                self.diagnostics.push(Diagnostic::new(
                    "E0415",
                    format!("value of type {ty_name} cannot be interpolated into a string"),
                    expr.span(),
                ));
            }
        }
    }

    fn is_interpolatable_type(&self, ty: TypeId) -> bool {
        matches!(
            self.types.kind(ty),
            TypeKind::String
                | TypeKind::Int
                | TypeKind::Float
                | TypeKind::Bool
                | TypeKind::Null
                | TypeKind::Unknown
        )
    }

    fn check_assignment_target(
        &mut self,
        target: &Expr,
        op: &AssignOp,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
        constructor_init_context: Option<&mut ConstructorInitContext>,
    ) -> Option<AssignmentTarget> {
        match target {
            Expr::Grouped { expr, .. } => self.check_assignment_target(
                expr,
                op,
                scopes,
                method_context,
                constructor_init_context,
            ),
            Expr::Variable { name, span } => match scopes.lookup(name) {
                Some(binding) => {
                    if !binding.writable {
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0201",
                                format!("cannot assign to readonly variable `${name}`"),
                                *span,
                            )
                            .with_help(format!(
                                "declare it as `let writable ${name} = ...` if mutation is intended"
                            )),
                        );
                    }
                    Some(AssignmentTarget {
                        ty: binding.ty,
                        destination: AssignmentDestination::Type,
                    })
                }
                None => {
                    self.undeclared_variable(name, *span);
                    None
                }
            },
            Expr::PropertyAccess {
                object,
                property,
                span,
            } => {
                self.check_expr(object, scopes, method_context);
                self.check_mixed_operation(object, "property write", scopes, method_context);
                if let Some((class_name, property_info)) =
                    self.lookup_property(object, property, *span, scopes, method_context)
                {
                    let constructor_init_decision = if Self::is_direct_this(object) {
                        self.check_constructor_init_assignment(
                            &class_name,
                            property,
                            &property_info,
                            op,
                            *span,
                            constructor_init_context,
                        )
                    } else {
                        ConstructorInitDecision::NotApplicable
                    };

                    if matches!(
                        constructor_init_decision,
                        ConstructorInitDecision::NotApplicable
                    ) {
                        let writable_path =
                            self.is_writable_object_path(object, scopes, method_context);
                        if !writable_path {
                            let message = if Self::is_direct_this(object) {
                                "cannot mutate `$this` in a readonly method".to_string()
                            } else {
                                match object.as_ref() {
                                    Expr::Variable { name, .. } => {
                                        format!("cannot write through readonly variable `${name}`")
                                    }
                                    _ => "cannot write through readonly object path".to_string(),
                                }
                            };
                            self.diagnostics
                                .push(Diagnostic::new("E0201", message, object.span()));
                        }

                        if !property_info.writable {
                            self.diagnostics.push(
                                Diagnostic::new(
                                    "E0202",
                                    format!(
                                        "cannot assign to readonly property `{class_name}::${property}`"
                                    ),
                                    *span,
                                )
                                .with_help(format!(
                                    "mark the property writable: `writable {} ${property};`",
                                    self.types.display(property_info.ty)
                                )),
                            );
                        }
                    }
                    Some(AssignmentTarget {
                        ty: property_info.ty,
                        destination: AssignmentDestination::Property {
                            class_name,
                            name: property.clone(),
                        },
                    })
                } else {
                    None
                }
            }
            _ => {
                self.diagnostics.push(Diagnostic::new(
                    "E0204",
                    "unsupported assignment target",
                    target.span(),
                ));
                None
            }
        }
    }

    fn check_constructor_init_assignment(
        &mut self,
        class_name: &str,
        property: &str,
        property_info: &PropertyInfo,
        op: &AssignOp,
        span: Span,
        constructor_init_context: Option<&mut ConstructorInitContext>,
    ) -> ConstructorInitDecision {
        let Some(context) = constructor_init_context else {
            return ConstructorInitDecision::NotApplicable;
        };

        if context.class_name != class_name {
            return ConstructorInitDecision::NotApplicable;
        }

        if property_info.writable {
            return ConstructorInitDecision::Allowed;
        }

        if !context.readonly_init_allowed {
            return ConstructorInitDecision::NotApplicable;
        }

        if !matches!(op, AssignOp::Assign) {
            self.diagnostics.push(Diagnostic::new(
                "E0413",
                "constructor init access only applies to simple `$this->property = value` assignments",
                span,
            ));
            return ConstructorInitDecision::Rejected;
        }

        match property_info.init_state {
            PropertyInitState::HasInitializer => {
                self.report_readonly_property_already_initialized(
                    class_name,
                    property,
                    "is already initialized by its property initializer",
                    span,
                );
                ConstructorInitDecision::Rejected
            }
            PropertyInitState::PromotedParameter => {
                self.report_readonly_property_already_initialized(
                    class_name,
                    property,
                    "is already initialized by constructor promotion",
                    span,
                );
                ConstructorInitDecision::Rejected
            }
            PropertyInitState::Uninitialized => {
                if !context.initialized.insert(property.to_string()) {
                    self.report_readonly_property_already_initialized(
                        class_name,
                        property,
                        "has already been initialized in this constructor",
                        span,
                    );
                    return ConstructorInitDecision::Rejected;
                }

                ConstructorInitDecision::Allowed
            }
        }
    }

    fn report_readonly_property_already_initialized(
        &mut self,
        class_name: &str,
        property: &str,
        reason: &str,
        span: Span,
    ) {
        self.diagnostics.push(Diagnostic::new(
            "E0412",
            format!("readonly property `{class_name}::${property}` {reason}"),
            span,
        ));
    }

    fn check_function_call(
        &mut self,
        name: &str,
        args: &[Expr],
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let Some(function_info) = self.functions.get(name).cloned() else {
            self.diagnostics.push(Diagnostic::new(
                "E0309",
                format!("unknown function `{name}`"),
                span,
            ));
            return;
        };

        self.check_call_arguments(
            &format!("function `{name}`"),
            &function_info.params,
            args,
            span,
            scopes,
            method_context,
        );
    }

    fn check_method_call(
        &mut self,
        object: &Expr,
        method: &str,
        args: &[Expr],
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let Some(class_name) = self.expr_class_name(object, scopes, method_context) else {
            return;
        };
        let Some(class_info) = self.classes.get(&class_name).cloned() else {
            self.diagnostics.push(Diagnostic::new(
                "E0305",
                format!("unknown class `{class_name}`"),
                span,
            ));
            return;
        };
        let Some(method_info) = class_info.methods.get(method) else {
            self.diagnostics.push(Diagnostic::new(
                "E0304",
                format!("unknown method `{class_name}::{method}`"),
                span,
            ));
            return;
        };

        if self.check_direct_lifecycle_method_call(&class_name, method, span) {
            return;
        }

        if matches!(method_info.access, MemberAccess::Internal)
            && !self.can_access_internal_member(&class_name, method_context)
        {
            self.diagnostics.push(Diagnostic::new(
                "E0307",
                format!("method `{class_name}::{method}` is internal"),
                span,
            ));
        }

        if method_info.writable_this
            && !self.is_writable_object_path(object, scopes, method_context)
        {
            self.diagnostics.push(Diagnostic::new(
                "E0203",
                format!(
                    "cannot call writable method `{class_name}::{method}` through readonly value"
                ),
                span,
            ));
        }

        self.check_call_arguments(
            &format!("method `{class_name}::{method}`"),
            &method_info.params,
            args,
            span,
            scopes,
            method_context,
        );
    }

    fn check_static_call(
        &mut self,
        class_name: &str,
        method: &str,
        args: &[Expr],
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let Some(class_info) = self.classes.get(class_name).cloned() else {
            self.diagnostics.push(Diagnostic::new(
                "E0305",
                format!("unknown class `{class_name}`"),
                span,
            ));
            return;
        };
        let Some(method_info) = class_info.methods.get(method) else {
            self.diagnostics.push(Diagnostic::new(
                "E0304",
                format!("unknown method `{class_name}::{method}`"),
                span,
            ));
            return;
        };

        if self.check_direct_lifecycle_method_call(class_name, method, span) {
            return;
        }

        if matches!(method_info.access, MemberAccess::Internal)
            && !self.can_access_internal_member(class_name, method_context)
        {
            self.diagnostics.push(Diagnostic::new(
                "E0307",
                format!("method `{class_name}::{method}` is internal"),
                span,
            ));
        }

        self.check_call_arguments(
            &format!("method `{class_name}::{method}`"),
            &method_info.params,
            args,
            span,
            scopes,
            method_context,
        );
    }

    fn check_direct_lifecycle_method_call(
        &mut self,
        class_name: &str,
        method: &str,
        span: Span,
    ) -> bool {
        let Some(lifecycle) = LifecycleMethod::from_method_name(method) else {
            return false;
        };

        let help = match lifecycle {
            LifecycleMethod::Constructor => {
                format!("construct `{class_name}` with `new {class_name}(...)`")
            }
            LifecycleMethod::Destructor => {
                "destructors are invoked by the runtime, not user code".to_string()
            }
        };

        self.diagnostics.push(
            Diagnostic::new(
                "E0414",
                format!(
                    "{} `{class_name}::{method}` cannot be called directly",
                    lifecycle.label()
                ),
                span,
            )
            .with_help(help),
        );
        true
    }

    fn check_constructor_call(
        &mut self,
        class_name: &str,
        args: &[Expr],
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let Some(class_info) = self.classes.get(class_name).cloned() else {
            return;
        };

        let Some(constructor) = class_info.methods.get("__construct") else {
            if !args.is_empty() {
                self.report_argument_count_mismatch(
                    &format!("constructor `{class_name}::__construct`"),
                    0,
                    0,
                    args.len(),
                    span,
                );
            }
            return;
        };

        if matches!(constructor.access, MemberAccess::Internal)
            && !self.can_access_internal_member(class_name, method_context)
        {
            self.diagnostics.push(Diagnostic::new(
                "E0307",
                format!("method `{class_name}::__construct` is internal"),
                span,
            ));
        }

        self.check_call_arguments(
            &format!("constructor `{class_name}::__construct`"),
            &constructor.params,
            args,
            span,
            scopes,
            method_context,
        );
    }

    fn check_call_arguments(
        &mut self,
        callee: &str,
        params: &[ParamInfo],
        args: &[Expr],
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let required = params.iter().filter(|param| !param.has_default).count();
        let total = params.len();

        if args.len() < required || args.len() > total {
            self.report_argument_count_mismatch(callee, required, total, args.len(), span);
            return;
        }

        for (index, (arg, param)) in args.iter().zip(params.iter()).enumerate() {
            let got = self.infer_expr_type(arg, scopes, method_context);

            if self.is_expr_assignable(param.ty, arg, scopes, method_context)
                || self.is_assignable(param.ty, got)
            {
                continue;
            }

            self.diagnostics.push(Diagnostic::new(
                "E0408",
                format!(
                    "argument {} of {callee} expects `{}`, got `{}`",
                    index + 1,
                    self.types.display(param.ty),
                    self.types.display(got)
                ),
                arg.span(),
            ));
        }
    }

    fn report_argument_count_mismatch(
        &mut self,
        callee: &str,
        required: usize,
        total: usize,
        got: usize,
        span: Span,
    ) {
        let expectation = if required == total {
            format!("{} {}", required, Self::argument_word(required))
        } else {
            format!("between {} and {} arguments", required, total)
        };

        self.diagnostics.push(Diagnostic::new(
            "E0409",
            format!("{callee} expects {expectation}, got {got}"),
            span,
        ));
    }

    fn argument_word(count: usize) -> &'static str {
        if count == 1 {
            "argument"
        } else {
            "arguments"
        }
    }

    fn is_writable_object_path(
        &mut self,
        expr: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> bool {
        match expr {
            Expr::Grouped { expr, .. } => {
                self.is_writable_object_path(expr, scopes, method_context)
            }
            Expr::Variable { name, .. } => scopes
                .lookup(name)
                .map(|binding| binding.writable)
                .unwrap_or(false),
            Expr::This { .. } => method_context
                .map(|context| context.this_available && context.writable_this)
                .unwrap_or(false),
            Expr::PropertyAccess {
                object, property, ..
            } => {
                if !self.is_writable_object_path(object, scopes, method_context) {
                    return false;
                }
                let Some(class_name) = self.expr_class_name(object, scopes, method_context) else {
                    return false;
                };
                self.classes
                    .get(&class_name)
                    .and_then(|class_info| class_info.properties.get(property))
                    .map(|property| property.writable)
                    .unwrap_or(false)
            }
            _ => false,
        }
    }

    fn is_direct_this(expr: &Expr) -> bool {
        match expr {
            Expr::Grouped { expr, .. } => Self::is_direct_this(expr),
            Expr::This { .. } => true,
            _ => false,
        }
    }

    fn lookup_property(
        &mut self,
        object: &Expr,
        property: &str,
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> Option<(String, PropertyInfo)> {
        let class_name = self.expr_class_name(object, scopes, method_context)?;
        let Some(class_info) = self.classes.get(&class_name) else {
            self.diagnostics.push(Diagnostic::new(
                "E0305",
                format!("unknown class `{class_name}`"),
                span,
            ));
            return None;
        };
        let Some(property_info) = class_info.properties.get(property) else {
            self.diagnostics.push(Diagnostic::new(
                "E0303",
                format!("unknown property `{class_name}::${property}`"),
                span,
            ));
            return None;
        };

        let property_info = property_info.clone();
        if matches!(property_info.access, MemberAccess::Internal)
            && !self.can_access_internal_member(&class_name, method_context)
        {
            self.diagnostics.push(Diagnostic::new(
                "E0306",
                format!("property `{class_name}::${property}` is internal"),
                span,
            ));
        }

        Some((class_name, property_info))
    }

    fn can_access_internal_member(
        &self,
        declaring_class: &str,
        method_context: Option<&MethodContext>,
    ) -> bool {
        method_context
            .map(|context| context.class_name == declaring_class)
            .unwrap_or(false)
    }

    fn resolve_type_ref(&mut self, ty: &TypeRef, span: Span) -> TypeId {
        self.resolve_type_ref_in_position(ty, span, TypePosition::Value)
    }

    fn resolve_type_ref_in_position(
        &mut self,
        ty: &TypeRef,
        span: Span,
        position: TypePosition,
    ) -> TypeId {
        match ty.name.as_str() {
            "void" if position == TypePosition::Return => {
                self.resolve_zero_arg_type(ty, span, TypeKind::Void)
            }
            "void" => {
                self.reject_type_ref(ty, span, "E0430", "`void` is only valid as a return type")
            }
            "int" => self.resolve_zero_arg_type(ty, span, TypeKind::Int),
            "float" => self.resolve_zero_arg_type(ty, span, TypeKind::Float),
            "string" => self.resolve_zero_arg_type(ty, span, TypeKind::String),
            "bool" => self.resolve_zero_arg_type(ty, span, TypeKind::Bool),
            "null" => self.reject_type_ref_with_help(
                ty,
                span,
                "E0431",
                "`null` is a literal, not a type name",
                "spell nullable values as `?T`, such as `?User`",
            ),
            "mixed" => self.resolve_zero_arg_type(ty, span, TypeKind::Mixed),
            "object" => self.reject_type_ref_with_help(
                ty,
                span,
                "E0401",
                "unknown type `object`",
                "Doria has no `object` type; use `mixed` and narrow with `is` or `match`",
            ),
            "array" => self.reject_type_ref_with_help(
                ty,
                span,
                "E0401",
                "unknown type `array`",
                "use typed array suffixes like `T[]` or named collection aliases",
            ),
            "resource" => self.reject_type_ref(
                ty,
                span,
                "E0432",
                "`resource` is reserved for PHP interop and is not available yet",
            ),
            "[]" => {
                if !self.expect_type_arg_count(ty, 1, span) {
                    for arg in &ty.args {
                        self.resolve_type_ref_in_position(arg, span, TypePosition::Value);
                    }
                    return self.types.unknown();
                }
                let element =
                    self.resolve_type_ref_in_position(&ty.args[0], span, TypePosition::Value);
                self.types.intern(TypeKind::TypedArray(element))
            }
            "List" => {
                if !self.expect_type_arg_count(ty, 1, span) {
                    for arg in &ty.args {
                        self.resolve_type_ref_in_position(arg, span, TypePosition::Value);
                    }
                    return self.types.unknown();
                }
                let element =
                    self.resolve_type_ref_in_position(&ty.args[0], span, TypePosition::Value);
                self.types.intern(TypeKind::List(element))
            }
            "Dictionary" => {
                if !self.expect_type_arg_count(ty, 2, span) {
                    for arg in &ty.args {
                        self.resolve_type_ref_in_position(arg, span, TypePosition::Value);
                    }
                    return self.types.unknown();
                }
                let key = self.resolve_type_ref_in_position(&ty.args[0], span, TypePosition::Value);
                let value =
                    self.resolve_type_ref_in_position(&ty.args[1], span, TypePosition::Value);
                self.types.intern(TypeKind::Dictionary(key, value))
            }
            "Set" => {
                if !self.expect_type_arg_count(ty, 1, span) {
                    for arg in &ty.args {
                        self.resolve_type_ref_in_position(arg, span, TypePosition::Value);
                    }
                    return self.types.unknown();
                }
                let element =
                    self.resolve_type_ref_in_position(&ty.args[0], span, TypePosition::Value);
                self.types.intern(TypeKind::Set(element))
            }
            name if self.classes.contains_key(name) => {
                if !self.expect_type_arg_count(ty, 0, span) {
                    for arg in &ty.args {
                        self.resolve_type_ref_in_position(arg, span, TypePosition::Value);
                    }
                }
                self.types.intern(TypeKind::Class(name.to_string()))
            }
            name => {
                for arg in &ty.args {
                    self.resolve_type_ref_in_position(arg, span, TypePosition::Value);
                }
                self.diagnostics.push(Diagnostic::new(
                    "E0401",
                    format!("unknown type `{name}`"),
                    span,
                ));
                self.types.unknown()
            }
        }
    }

    fn reject_type_ref(
        &mut self,
        ty: &TypeRef,
        span: Span,
        code: &'static str,
        message: impl Into<String>,
    ) -> TypeId {
        for arg in &ty.args {
            self.resolve_type_ref_in_position(arg, span, TypePosition::Value);
        }
        self.diagnostics.push(Diagnostic::new(code, message, span));
        self.types.unknown()
    }

    fn reject_type_ref_with_help(
        &mut self,
        ty: &TypeRef,
        span: Span,
        code: &'static str,
        message: impl Into<String>,
        help: impl Into<String>,
    ) -> TypeId {
        for arg in &ty.args {
            self.resolve_type_ref_in_position(arg, span, TypePosition::Value);
        }
        self.diagnostics
            .push(Diagnostic::new(code, message, span).with_help(help));
        self.types.unknown()
    }

    fn resolve_zero_arg_type(&mut self, ty: &TypeRef, span: Span, kind: TypeKind) -> TypeId {
        self.expect_type_arg_count(ty, 0, span);
        for arg in &ty.args {
            self.resolve_type_ref(arg, span);
        }
        self.types.intern(kind)
    }

    fn expect_type_arg_count(&mut self, ty: &TypeRef, expected: usize, span: Span) -> bool {
        let found = ty.args.len();
        if found == expected {
            return true;
        }

        self.diagnostics.push(Diagnostic::new(
            "E0402",
            format!(
                "type `{}` expects {} type argument{}, found {}",
                ty.name,
                expected,
                if expected == 1 { "" } else { "s" },
                found
            ),
            span,
        ));
        false
    }

    fn check_assignable(
        &mut self,
        target: TypeId,
        value: TypeId,
        span: Span,
        destination: AssignmentDestination,
    ) {
        let target_name = self.types.display(target);
        let value_name = self.types.display(value);
        let message = match destination {
            AssignmentDestination::Type => {
                format!("cannot assign value of type `{value_name}` to `{target_name}`")
            }
            AssignmentDestination::Parameter { name } => format!(
                "cannot assign default value of type `{value_name}` to parameter `${name}` of type `{target_name}`"
            ),
            AssignmentDestination::Property { class_name, name } => format!(
                "cannot assign value of type `{value_name}` to property `{class_name}::${name}` of type `{target_name}`"
            ),
        };

        self.diagnostics
            .push(Diagnostic::new("E0403", message, span));
    }

    fn check_expr_assignable(
        &mut self,
        target: TypeId,
        value_expr: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
        destination: AssignmentDestination,
    ) -> bool {
        let value = self.infer_expr_type(value_expr, scopes, method_context);
        if self.is_expr_assignable(target, value_expr, scopes, method_context)
            || self.is_assignable(target, value)
        {
            return true;
        }

        self.check_assignable(target, value, value_expr.span(), destination);
        false
    }

    fn narrow_empty_collection_assignment(
        &self,
        target: &Expr,
        target_ty: TypeId,
        value_ty: TypeId,
        scopes: &mut ScopeStack,
    ) {
        if !matches!(self.types.kind(target_ty), TypeKind::EmptyCollection)
            || !self.is_non_empty_collection_like_type(value_ty)
        {
            return;
        }

        let Expr::Variable { name, .. } = target else {
            return;
        };

        let Some(binding) = scopes.lookup_mut(name) else {
            return;
        };

        if matches!(self.types.kind(binding.ty), TypeKind::EmptyCollection) {
            binding.ty = value_ty;
        }
    }

    fn is_expr_assignable(
        &mut self,
        target: TypeId,
        value_expr: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> bool {
        match value_expr {
            Expr::Grouped { expr, .. } => {
                self.is_expr_assignable(target, expr, scopes, method_context)
            }
            Expr::Array { elements, .. } => {
                self.is_array_literal_assignable(target, elements, scopes, method_context)
            }
            _ => {
                let value = self.infer_expr_type(value_expr, scopes, method_context);
                self.is_assignable(target, value)
            }
        }
    }

    fn is_array_literal_assignable(
        &mut self,
        target: TypeId,
        elements: &[ArrayElement],
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> bool {
        let target_kind = self.types.kind(target).clone();

        match target_kind {
            TypeKind::Mixed | TypeKind::Unknown => true,
            TypeKind::TypedArray(element) => {
                if elements.iter().any(|element| element.key.is_some()) {
                    return false;
                }

                elements.iter().all(|array_element| {
                    self.is_expr_assignable(element, &array_element.value, scopes, method_context)
                })
            }
            TypeKind::List(element) => {
                if self.is_unknown_type(element)
                    || elements.iter().any(|element| element.key.is_some())
                {
                    return false;
                }

                elements.iter().all(|array_element| {
                    self.is_expr_assignable(element, &array_element.value, scopes, method_context)
                })
            }
            TypeKind::Dictionary(key, value) => {
                if self.is_unknown_type(key) || self.is_unknown_type(value) {
                    return false;
                }

                if elements.is_empty() {
                    return true;
                }

                if !elements.iter().any(|element| element.key.is_some()) {
                    return false;
                }

                elements.iter().all(|array_element| {
                    let key_ok = if let Some(key_expr) = &array_element.key {
                        self.is_expr_assignable(key, key_expr, scopes, method_context)
                    } else {
                        let implicit_key = self.types.intern(TypeKind::Int);
                        self.is_assignable(key, implicit_key)
                    };
                    let value_ok = self.is_expr_assignable(
                        value,
                        &array_element.value,
                        scopes,
                        method_context,
                    );
                    key_ok && value_ok
                })
            }
            _ => {
                let value = self.infer_array_type(elements, scopes, method_context);
                self.is_assignable(target, value)
            }
        }
    }

    fn is_unknown_type(&self, ty: TypeId) -> bool {
        matches!(self.types.kind(ty), TypeKind::Unknown)
    }

    fn is_assignable(&self, target: TypeId, value: TypeId) -> bool {
        if target == value {
            return true;
        }

        let target_kind = self.types.kind(target).clone();
        let value_kind = self.types.kind(value).clone();
        match (target_kind, value_kind) {
            (TypeKind::Heterogeneous, _) | (_, TypeKind::Heterogeneous) => false,
            (TypeKind::Mixed, _) => true,
            (_, TypeKind::Mixed) => false,
            (TypeKind::Unknown, _) | (_, TypeKind::Unknown) => true,
            (
                TypeKind::TypedArray(_)
                | TypeKind::List(_)
                | TypeKind::Dictionary(_, _)
                | TypeKind::Set(_),
                TypeKind::EmptyCollection,
            ) => true,
            (
                TypeKind::EmptyCollection,
                TypeKind::TypedArray(_)
                | TypeKind::List(_)
                | TypeKind::Dictionary(_, _)
                | TypeKind::Set(_),
            ) => true,
            (TypeKind::Class(target), TypeKind::Class(value)) => target == value,
            (TypeKind::TypedArray(target), TypeKind::TypedArray(value)) => {
                self.is_assignable(target, value)
            }
            (TypeKind::List(target), TypeKind::List(value)) => self.is_assignable(target, value),
            (
                TypeKind::Dictionary(target_key, target_value),
                TypeKind::Dictionary(value_key, value_value),
            ) => {
                self.is_assignable(target_key, value_key)
                    && self.is_assignable(target_value, value_value)
            }
            (TypeKind::Set(target), TypeKind::Set(value)) => self.is_assignable(target, value),
            _ => false,
        }
    }

    fn infer_expr_type(
        &mut self,
        expr: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> TypeId {
        match expr {
            Expr::String { .. } | Expr::InterpolatedString { .. } => {
                self.types.intern(TypeKind::String)
            }
            Expr::Int { .. } => self.types.intern(TypeKind::Int),
            Expr::Float { .. } => self.types.intern(TypeKind::Float),
            Expr::Bool { .. } => self.types.intern(TypeKind::Bool),
            Expr::Null { .. } => self.types.intern(TypeKind::Null),
            Expr::New { class_name, .. } => {
                if self.classes.contains_key(class_name) {
                    self.types.intern(TypeKind::Class(class_name.clone()))
                } else {
                    self.types.unknown()
                }
            }
            Expr::Array { elements, .. } => self.infer_array_type(elements, scopes, method_context),
            Expr::Variable { name, .. } => scopes
                .lookup(name)
                .map(|binding| binding.ty)
                .unwrap_or_else(|| self.types.unknown()),
            Expr::This { .. } => method_context
                .filter(|context| context.this_available)
                .map(|context| {
                    self.types
                        .intern(TypeKind::Class(context.class_name.clone()))
                })
                .unwrap_or_else(|| self.types.unknown()),
            Expr::PropertyAccess {
                object, property, ..
            } => {
                let Some(class_name) = self.expr_class_name(object, scopes, method_context) else {
                    return self.types.unknown();
                };
                self.classes
                    .get(&class_name)
                    .and_then(|class_info| class_info.properties.get(property))
                    .map(|property| property.ty)
                    .unwrap_or_else(|| self.types.unknown())
            }
            Expr::MethodCall { object, method, .. } => {
                let Some(class_name) = self.expr_class_name(object, scopes, method_context) else {
                    return self.types.unknown();
                };
                self.classes
                    .get(&class_name)
                    .and_then(|class_info| class_info.methods.get(method))
                    .map(|method| method.return_ty)
                    .unwrap_or_else(|| self.types.unknown())
            }
            Expr::FunctionCall { name, .. } => self
                .functions
                .get(name)
                .map(|function| function.return_ty)
                .unwrap_or_else(|| self.types.unknown()),
            Expr::StaticCall {
                class_name, method, ..
            } => self
                .classes
                .get(class_name)
                .and_then(|class_info| class_info.methods.get(method))
                .map(|method| method.return_ty)
                .unwrap_or_else(|| self.types.unknown()),
            Expr::Grouped { expr, .. } => self.infer_expr_type(expr, scopes, method_context),
            Expr::Unary { op, expr, .. } => self.infer_unary_type(op, expr, scopes, method_context),
            Expr::Binary {
                left, op, right, ..
            } => self.infer_binary_type(left, op, right, scopes, method_context),
            Expr::Range { .. } => self.types.unknown(),
            _ => self.types.unknown(),
        }
    }

    fn infer_unary_type(
        &mut self,
        op: &UnaryOp,
        expr: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> TypeId {
        let ty = self.infer_expr_type(expr, scopes, method_context);
        match op {
            UnaryOp::Not => match self.types.kind(ty) {
                TypeKind::Bool => self.types.intern(TypeKind::Bool),
                TypeKind::Unknown => self.types.unknown(),
                _ => self.types.intern(TypeKind::Heterogeneous),
            },
        }
    }

    fn infer_binary_type(
        &mut self,
        left: &Expr,
        op: &BinaryOp,
        right: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> TypeId {
        let left_ty = self.infer_expr_type(left, scopes, method_context);
        let right_ty = self.infer_expr_type(right, scopes, method_context);

        match op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => {
                self.infer_numeric_binary_type(left_ty, right_ty)
            }
            BinaryOp::Concat => self.infer_concat_binary_type(left_ty, right_ty),
            BinaryOp::Equal | BinaryOp::NotEqual => self.types.intern(TypeKind::Bool),
            BinaryOp::Less | BinaryOp::LessEqual | BinaryOp::Greater | BinaryOp::GreaterEqual => {
                self.infer_relational_binary_type(left_ty, right_ty)
            }
            BinaryOp::And | BinaryOp::Or | BinaryOp::Xor => {
                self.infer_logical_binary_type(left_ty, right_ty)
            }
            BinaryOp::Coalesce => self.infer_coalesce_binary_type(left_ty, right_ty),
        }
    }

    fn infer_numeric_binary_type(&mut self, left: TypeId, right: TypeId) -> TypeId {
        if let Some(recovery) = self.recovery_binary_type(left, right) {
            return recovery;
        }

        let left_kind = self.types.kind(left).clone();
        let right_kind = self.types.kind(right).clone();
        match (left_kind, right_kind) {
            (TypeKind::Int, TypeKind::Int) => self.types.intern(TypeKind::Int),
            (TypeKind::Float, TypeKind::Float) => self.types.intern(TypeKind::Float),
            _ => self.types.intern(TypeKind::Heterogeneous),
        }
    }

    fn infer_concat_binary_type(&mut self, left: TypeId, right: TypeId) -> TypeId {
        if let Some(recovery) = self.recovery_binary_type(left, right) {
            return recovery;
        }

        let left_kind = self.types.kind(left).clone();
        let right_kind = self.types.kind(right).clone();
        match (left_kind, right_kind) {
            (TypeKind::String, TypeKind::String) => self.types.intern(TypeKind::String),
            _ => self.types.intern(TypeKind::Heterogeneous),
        }
    }

    fn infer_logical_binary_type(&mut self, left: TypeId, right: TypeId) -> TypeId {
        if let Some(recovery) = self.recovery_binary_type(left, right) {
            return recovery;
        }

        let left_kind = self.types.kind(left).clone();
        let right_kind = self.types.kind(right).clone();
        match (left_kind, right_kind) {
            (TypeKind::Bool, TypeKind::Bool) => self.types.intern(TypeKind::Bool),
            _ => self.types.intern(TypeKind::Heterogeneous),
        }
    }

    fn infer_relational_binary_type(&mut self, left: TypeId, right: TypeId) -> TypeId {
        if let Some(recovery) = self.recovery_binary_type(left, right) {
            return recovery;
        }

        let left_kind = self.types.kind(left).clone();
        let right_kind = self.types.kind(right).clone();
        match (left_kind, right_kind) {
            (TypeKind::Int, TypeKind::Int)
            | (TypeKind::Float, TypeKind::Float)
            | (TypeKind::String, TypeKind::String) => self.types.intern(TypeKind::Bool),
            _ => self.types.intern(TypeKind::Heterogeneous),
        }
    }

    fn infer_coalesce_binary_type(&mut self, left: TypeId, right: TypeId) -> TypeId {
        if let Some(recovery) = self.recovery_binary_type(left, right) {
            return recovery;
        }

        let left_kind = self.types.kind(left).clone();
        let right_kind = self.types.kind(right).clone();
        match (left_kind, right_kind) {
            (TypeKind::Null, _) => right,
            (_, TypeKind::Null) => left,
            _ if left == right => left,
            _ => self.types.intern(TypeKind::Heterogeneous),
        }
    }

    fn recovery_binary_type(&mut self, left: TypeId, right: TypeId) -> Option<TypeId> {
        let left_kind = self.types.kind(left).clone();
        let right_kind = self.types.kind(right).clone();

        match (left_kind, right_kind) {
            (TypeKind::Unknown, _) | (_, TypeKind::Unknown) => Some(self.types.unknown()),
            _ => None,
        }
    }

    fn infer_array_type(
        &mut self,
        elements: &[ArrayElement],
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> TypeId {
        if elements.is_empty() {
            return self.types.intern(TypeKind::EmptyCollection);
        }

        if elements.iter().any(|element| element.key.is_some()) {
            let mut key_types = Vec::new();
            let mut value_types = Vec::new();
            for element in elements {
                if let Some(key) = &element.key {
                    key_types.push(self.infer_expr_type(key, scopes, method_context));
                } else {
                    key_types.push(self.types.intern(TypeKind::Int));
                }
                value_types.push(self.infer_expr_type(&element.value, scopes, method_context));
            }
            let key = self.common_clear_type(key_types);
            let value = self.common_clear_type(value_types);
            self.types.intern(TypeKind::Dictionary(key, value))
        } else {
            let mut element_types = Vec::new();
            for element in elements {
                element_types.push(self.infer_expr_type(&element.value, scopes, method_context));
            }
            let element = self.common_clear_type(element_types);
            self.types.intern(TypeKind::List(element))
        }
    }

    fn common_clear_type(&mut self, types: Vec<TypeId>) -> TypeId {
        let mut common = None;
        let mut saw_empty_collection = false;
        let mut saw_mixed = false;
        let mut saw_heterogeneous = false;

        for ty in types {
            let kind = self.types.kind(ty).clone();
            if self.type_contains_mixed(ty) {
                saw_mixed = true;
            }

            match kind {
                TypeKind::Unknown => {
                    continue;
                }
                TypeKind::EmptyCollection => {
                    saw_empty_collection = true;
                    continue;
                }
                _ => {
                    if let Some(common_ty) = common {
                        if common_ty != ty {
                            if self.type_contains_mixed(common_ty) || self.type_contains_mixed(ty) {
                                common = Some(self.merge_mixed_return_types(common_ty, ty));
                            } else {
                                saw_heterogeneous = true;
                            }
                        }
                    } else {
                        common = Some(ty);
                    }
                }
            }
        }

        if saw_heterogeneous {
            if saw_mixed {
                return self.types.intern(TypeKind::Mixed);
            }
            return self.types.intern(TypeKind::Heterogeneous);
        }

        if let Some(common) = common {
            if saw_empty_collection && !self.is_collection_like_type(common) {
                return self.types.intern(TypeKind::Heterogeneous);
            }
            common
        } else if saw_empty_collection {
            self.types.intern(TypeKind::EmptyCollection)
        } else {
            self.types.unknown()
        }
    }

    fn is_collection_like_type(&self, ty: TypeId) -> bool {
        matches!(
            self.types.kind(ty),
            TypeKind::TypedArray(_)
                | TypeKind::List(_)
                | TypeKind::Dictionary(_, _)
                | TypeKind::Set(_)
                | TypeKind::EmptyCollection
        )
    }

    fn is_non_empty_collection_like_type(&self, ty: TypeId) -> bool {
        matches!(
            self.types.kind(ty),
            TypeKind::TypedArray(_)
                | TypeKind::List(_)
                | TypeKind::Dictionary(_, _)
                | TypeKind::Set(_)
        )
    }

    fn expr_class_name(
        &mut self,
        expr: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> Option<String> {
        let ty = self.infer_expr_type(expr, scopes, method_context);
        self.types.class_name(ty).map(ToOwned::to_owned)
    }

    fn undeclared_variable(&mut self, name: &str, span: Span) {
        self.diagnostics.push(
            Diagnostic::new(
                "E0101",
                format!("cannot assign to undeclared variable `${name}`"),
                span,
            )
            .with_help(format!(
                "use `let ${name} = ...` or an explicit type declaration"
            )),
        );
    }
}

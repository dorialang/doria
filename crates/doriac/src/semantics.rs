use std::collections::HashMap;

use crate::ast::*;
use crate::diagnostics::{Diagnostic, DiagnosticResult};
use crate::source::Span;
use crate::symbols::{Binding, ClassInfo, MethodInfo, PropertyInfo, ScopeStack};
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

impl<'program> Checker<'program> {
    fn new(program: &'program Program) -> Self {
        Self {
            program,
            classes: HashMap::new(),
            types: TypeRegistry::new(),
            diagnostics: Vec::new(),
        }
    }

    fn check(&mut self) {
        self.collect_classes();

        let mut scopes = ScopeStack::new();
        for item in &self.program.items {
            match item {
                Item::Statement(statement) => self.check_statement(statement, &mut scopes, None),
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
        let value_ty = self.infer_expr_type(initializer, &scopes, Some(&initializer_context));
        let target_ty = self
            .classes
            .get(class_name)
            .and_then(|class_info| class_info.properties.get(&property.name))
            .map(|property| property.ty)
            .unwrap_or_else(|| self.resolve_type_ref(&property.ty, property.span));
        self.check_assignable(
            target_ty,
            value_ty,
            initializer.span(),
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
        if let Some(return_type) = &function.return_type {
            self.resolve_type_ref(return_type, function.span);
        }
        for param in &function.params {
            let ty = self.resolve_type_ref(&param.ty, param.span);
            if let Some(default) = &param.default {
                self.check_expr(default, &scopes, method_context.as_ref());
                let value_ty = self.infer_expr_type(default, &scopes, method_context.as_ref());
                self.check_assignable(
                    ty,
                    value_ty,
                    default.span(),
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
                },
                param.span,
            );
        }
        self.check_block(&function.body, &mut scopes, method_context.as_ref());
    }

    fn check_block(
        &mut self,
        block: &Block,
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        scopes.push();
        for statement in &block.statements {
            self.check_statement(statement, scopes, method_context);
        }
        scopes.pop();
    }

    fn check_statement(
        &mut self,
        statement: &Stmt,
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        match statement {
            Stmt::VarDecl(decl) => {
                self.check_expr(&decl.initializer, scopes, method_context);
                let value_ty = self.infer_expr_type(&decl.initializer, scopes, method_context);
                let ty = match &decl.ty {
                    Some(ty) => {
                        let target_ty = self.resolve_type_ref(ty, decl.span);
                        self.check_assignable(
                            target_ty,
                            value_ty,
                            decl.initializer.span(),
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
                    },
                    decl.span,
                );
            }
            Stmt::Assignment(assignment) => {
                self.check_expr(&assignment.value, scopes, method_context);
                let value_ty = self.infer_expr_type(&assignment.value, scopes, method_context);
                if let Some(target) =
                    self.check_assignment_target(&assignment.target, scopes, method_context)
                {
                    self.check_assignable(
                        target.ty,
                        value_ty,
                        assignment.value.span(),
                        target.destination,
                    );
                }
            }
            Stmt::Echo { expr, .. } | Stmt::Expr { expr, .. } => {
                self.check_expr(expr, scopes, method_context);
            }
            Stmt::Return { expr, .. } => {
                if let Some(expr) = expr {
                    self.check_expr(expr, scopes, method_context);
                }
            }
            Stmt::Foreach(foreach) => {
                self.check_expr(&foreach.iterable, scopes, method_context);
                scopes.push();
                if let Some(key) = &foreach.key {
                    let ty = key
                        .ty
                        .as_ref()
                        .map(|ty| self.resolve_type_ref(ty, foreach.span))
                        .unwrap_or_else(|| self.types.unknown());
                    self.declare_binding(
                        scopes,
                        key.name.clone(),
                        Binding {
                            writable: false,
                            ty,
                        },
                        foreach.span,
                    );
                }
                let value_ty = foreach
                    .value
                    .ty
                    .as_ref()
                    .map(|ty| self.resolve_type_ref(ty, foreach.span))
                    .unwrap_or_else(|| self.types.unknown());
                self.declare_binding(
                    scopes,
                    foreach.value.name.clone(),
                    Binding {
                        writable: false,
                        ty: value_ty,
                    },
                    foreach.span,
                );
                for statement in &foreach.body.statements {
                    self.check_statement(statement, scopes, method_context);
                }
                scopes.pop();
            }
        }
    }

    fn check_expr(
        &mut self,
        expr: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
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
                self.check_method_call(object, method, *span, scopes, method_context);
            }
            Expr::FunctionCall { args, .. } => {
                for arg in args {
                    self.check_expr(arg, scopes, method_context);
                }
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
                self.check_static_call(class_name, method, *span, method_context);
            }
            Expr::New {
                class_name,
                args,
                span,
            } => {
                if !self.classes.contains_key(class_name) {
                    self.diagnostics.push(Diagnostic::new(
                        "E0305",
                        format!("unknown class `{class_name}`"),
                        *span,
                    ));
                }
                for arg in args {
                    self.check_expr(arg, scopes, method_context);
                }
            }
            Expr::Binary { left, right, .. } => {
                self.check_expr(left, scopes, method_context);
                self.check_expr(right, scopes, method_context);
            }
            Expr::Identifier { .. }
            | Expr::String { .. }
            | Expr::Int { .. }
            | Expr::Float { .. }
            | Expr::Bool { .. }
            | Expr::Null { .. } => {}
        }
    }

    fn check_assignment_target(
        &mut self,
        target: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> Option<AssignmentTarget> {
        match target {
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
                let writable_path = self.is_writable_object_path(object, scopes, method_context);
                if !writable_path {
                    let message = match object.as_ref() {
                        Expr::This { .. } => {
                            "cannot mutate `$this` in a readonly method".to_string()
                        }
                        Expr::Variable { name, .. } => {
                            format!("cannot write through readonly variable `${name}`")
                        }
                        _ => "cannot write through readonly object path".to_string(),
                    };
                    self.diagnostics
                        .push(Diagnostic::new("E0201", message, object.span()));
                }

                if let Some((class_name, property_info)) =
                    self.lookup_property(object, property, *span, scopes, method_context)
                {
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

    fn check_method_call(
        &mut self,
        object: &Expr,
        method: &str,
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
    }

    fn check_static_call(
        &mut self,
        class_name: &str,
        method: &str,
        span: Span,
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

        if matches!(method_info.access, MemberAccess::Internal)
            && !self.can_access_internal_member(class_name, method_context)
        {
            self.diagnostics.push(Diagnostic::new(
                "E0307",
                format!("method `{class_name}::{method}` is internal"),
                span,
            ));
        }
    }

    fn is_writable_object_path(
        &mut self,
        expr: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> bool {
        match expr {
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
        match ty.name.as_str() {
            "void" => self.resolve_zero_arg_type(ty, span, TypeKind::Void),
            "int" => self.resolve_zero_arg_type(ty, span, TypeKind::Int),
            "float" => self.resolve_zero_arg_type(ty, span, TypeKind::Float),
            "string" => self.resolve_zero_arg_type(ty, span, TypeKind::String),
            "bool" => self.resolve_zero_arg_type(ty, span, TypeKind::Bool),
            "null" => self.resolve_zero_arg_type(ty, span, TypeKind::Null),
            "mixed" => self.resolve_zero_arg_type(ty, span, TypeKind::Mixed),
            "object" => self.resolve_zero_arg_type(ty, span, TypeKind::Object),
            "resource" => self.resolve_zero_arg_type(ty, span, TypeKind::Resource),
            "array" => self.resolve_zero_arg_type(ty, span, TypeKind::Array),
            "List" => {
                if !self.expect_type_arg_count(ty, 1, span) {
                    for arg in &ty.args {
                        self.resolve_type_ref(arg, span);
                    }
                    return self.types.unknown();
                }
                let element = self.resolve_type_ref(&ty.args[0], span);
                self.types.intern(TypeKind::List(element))
            }
            "Dictionary" => {
                if !self.expect_type_arg_count(ty, 2, span) {
                    for arg in &ty.args {
                        self.resolve_type_ref(arg, span);
                    }
                    return self.types.unknown();
                }
                let key = self.resolve_type_ref(&ty.args[0], span);
                let value = self.resolve_type_ref(&ty.args[1], span);
                self.types.intern(TypeKind::Dictionary(key, value))
            }
            "Set" => {
                if !self.expect_type_arg_count(ty, 1, span) {
                    for arg in &ty.args {
                        self.resolve_type_ref(arg, span);
                    }
                    return self.types.unknown();
                }
                let element = self.resolve_type_ref(&ty.args[0], span);
                self.types.intern(TypeKind::Set(element))
            }
            name if self.classes.contains_key(name) => {
                if !self.expect_type_arg_count(ty, 0, span) {
                    for arg in &ty.args {
                        self.resolve_type_ref(arg, span);
                    }
                }
                self.types.intern(TypeKind::Class(name.to_string()))
            }
            name => {
                for arg in &ty.args {
                    self.resolve_type_ref(arg, span);
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
        if self.is_assignable(target, value) {
            return;
        }

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

    fn is_assignable(&self, target: TypeId, value: TypeId) -> bool {
        if target == value {
            return true;
        }

        let target_kind = self.types.kind(target).clone();
        let value_kind = self.types.kind(value).clone();
        match (target_kind, value_kind) {
            (TypeKind::Heterogeneous, _) | (_, TypeKind::Heterogeneous) => false,
            // TODO: tighten mixed later with narrowing or runtime checks.
            (TypeKind::Mixed, _) | (_, TypeKind::Mixed) => true,
            (TypeKind::Unknown, _) | (_, TypeKind::Unknown) => true,
            (TypeKind::Object, TypeKind::Class(_)) => true,
            (TypeKind::Array, TypeKind::List(_) | TypeKind::Dictionary(_, _)) => true,
            (
                TypeKind::Array | TypeKind::List(_) | TypeKind::Dictionary(_, _) | TypeKind::Set(_),
                TypeKind::EmptyCollection,
            ) => true,
            (TypeKind::Class(target), TypeKind::Class(value)) => target == value,
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
            Expr::String { .. } => self.types.intern(TypeKind::String),
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
            Expr::Binary {
                left, op, right, ..
            } => self.infer_binary_type(left, op, right, scopes, method_context),
            _ => self.types.unknown(),
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
            BinaryOp::Equal
            | BinaryOp::StrictEqual
            | BinaryOp::NotEqual
            | BinaryOp::NotStrictEqual
            | BinaryOp::Less
            | BinaryOp::LessEqual
            | BinaryOp::Greater
            | BinaryOp::GreaterEqual
            | BinaryOp::And
            | BinaryOp::Or => self.types.intern(TypeKind::Bool),
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
            (TypeKind::Mixed, _) | (_, TypeKind::Mixed) => Some(self.types.intern(TypeKind::Mixed)),
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

        for ty in types {
            if !self.is_clear_inferred_type(ty) {
                return self.types.unknown();
            }

            if matches!(self.types.kind(ty), TypeKind::EmptyCollection) {
                saw_empty_collection = true;
                continue;
            }

            if let Some(common_ty) = common {
                if common_ty != ty {
                    return self.types.intern(TypeKind::Heterogeneous);
                }
            } else {
                common = Some(ty);
            }
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

    fn is_clear_inferred_type(&self, ty: TypeId) -> bool {
        !matches!(self.types.kind(ty), TypeKind::Mixed | TypeKind::Unknown)
    }

    fn is_collection_like_type(&self, ty: TypeId) -> bool {
        matches!(
            self.types.kind(ty),
            TypeKind::Array
                | TypeKind::List(_)
                | TypeKind::Dictionary(_, _)
                | TypeKind::Set(_)
                | TypeKind::EmptyCollection
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

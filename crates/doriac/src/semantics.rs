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

        let mut scopes = ScopeStack::new();
        for item in &self.program.items {
            match item {
                Item::Statement(statement) => {
                    self.check_statement(statement, &mut scopes, None, None, None);
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
        FunctionInfo {
            params: self.resolve_param_infos(function),
            return_ty: self.resolve_function_return_type(function),
        }
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
            .map(|return_type| self.resolve_type_ref(return_type, function.span))
            .unwrap_or_else(|| self.types.unknown())
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
    ) {
        scopes.push();
        for statement in &block.statements {
            self.check_statement(
                statement,
                scopes,
                method_context,
                constructor_init_context.as_deref_mut(),
                return_context,
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
                    match assignment.op {
                        AssignOp::Assign => {
                            let target_ty = target.ty;
                            let assignment_ok = self.check_expr_assignable(
                                target.ty,
                                &assignment.value,
                                scopes,
                                method_context,
                                target.destination,
                            );
                            if assignment_ok {
                                let value_ty =
                                    self.infer_expr_type(&assignment.value, scopes, method_context);
                                self.narrow_empty_collection_assignment(
                                    &assignment.target,
                                    target_ty,
                                    value_ty,
                                    scopes,
                                );
                            }
                        }
                        AssignOp::AddAssign | AssignOp::SubAssign => {
                            let value_ty =
                                self.infer_expr_type(&assignment.value, scopes, method_context);
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
            }
            Stmt::Echo { expr, .. } | Stmt::Expr { expr, .. } => {
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
                    );
                }
                scopes.pop();
            }
        }
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
            Some(Stmt::Return { expr: Some(_), .. }) => {}
            Some(Stmt::Return { expr: None, .. }) => {}
            _ => self.report_missing_return_value(context, expected, function.span),
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
                | TypeKind::Mixed
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
                if let Some((class_name, property_info)) =
                    self.lookup_property(object, property, *span, scopes, method_context)
                {
                    let constructor_init_decision = if matches!(object.as_ref(), Expr::This { .. })
                    {
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
            TypeKind::Mixed | TypeKind::Unknown | TypeKind::Array => true,
            TypeKind::List(element) => {
                if self.is_mixed_or_unknown_type(element)
                    || elements.iter().any(|element| element.key.is_some())
                {
                    return false;
                }

                elements.iter().all(|array_element| {
                    self.is_expr_assignable(element, &array_element.value, scopes, method_context)
                })
            }
            TypeKind::Dictionary(key, value) => {
                if self.is_mixed_or_unknown_type(key) || self.is_mixed_or_unknown_type(value) {
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

    fn is_mixed_or_unknown_type(&self, ty: TypeId) -> bool {
        matches!(self.types.kind(ty), TypeKind::Mixed | TypeKind::Unknown)
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
            (
                TypeKind::Array,
                TypeKind::List(_) | TypeKind::Dictionary(_, _) | TypeKind::Set(_),
            ) => true,
            (
                TypeKind::Array | TypeKind::List(_) | TypeKind::Dictionary(_, _) | TypeKind::Set(_),
                TypeKind::EmptyCollection,
            ) => true,
            (
                TypeKind::EmptyCollection,
                TypeKind::Array | TypeKind::List(_) | TypeKind::Dictionary(_, _) | TypeKind::Set(_),
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
            | BinaryOp::NotStrictEqual => self.types.intern(TypeKind::Bool),
            BinaryOp::Less | BinaryOp::LessEqual | BinaryOp::Greater | BinaryOp::GreaterEqual => {
                self.infer_relational_binary_type(left_ty, right_ty)
            }
            BinaryOp::And | BinaryOp::Or => self.infer_logical_binary_type(left_ty, right_ty),
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
        let mut saw_mixed = false;

        for ty in types {
            let kind = self.types.kind(ty).clone();
            match kind {
                TypeKind::Mixed => {
                    saw_mixed = true;
                    continue;
                }
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
                            return self.types.intern(TypeKind::Heterogeneous);
                        }
                    } else {
                        common = Some(ty);
                    }
                }
            }
        }

        if let Some(common) = common {
            if saw_empty_collection && !self.is_collection_like_type(common) {
                return self.types.intern(TypeKind::Heterogeneous);
            }
            common
        } else if saw_empty_collection {
            self.types.intern(TypeKind::EmptyCollection)
        } else if saw_mixed {
            self.types.intern(TypeKind::Mixed)
        } else {
            self.types.unknown()
        }
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

    fn is_non_empty_collection_like_type(&self, ty: TypeId) -> bool {
        matches!(
            self.types.kind(ty),
            TypeKind::Array | TypeKind::List(_) | TypeKind::Dictionary(_, _) | TypeKind::Set(_)
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

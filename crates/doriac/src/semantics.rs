use std::collections::HashMap;

use crate::ast::*;
use crate::diagnostics::{Diagnostic, DiagnosticResult};
use crate::source::Span;
use crate::symbols::{Binding, ClassInfo, MethodInfo, PropertyInfo, ScopeStack};
use crate::types::TypeRef;

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
    diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone)]
struct MethodContext {
    class_name: String,
    writable_this: bool,
}

impl<'program> Checker<'program> {
    fn new(program: &'program Program) -> Self {
        Self {
            program,
            classes: HashMap::new(),
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
        for item in &self.program.items {
            let Item::Class(class_decl) = item else {
                continue;
            };

            let mut info = ClassInfo {
                properties: HashMap::new(),
                methods: HashMap::new(),
            };

            for member in &class_decl.members {
                match member {
                    ClassMember::Property(property) => {
                        info.properties.insert(
                            property.name.clone(),
                            PropertyInfo {
                                writable: property.writable,
                                ty: property.ty.clone(),
                            },
                        );
                    }
                    ClassMember::Method(method) => {
                        info.methods.insert(
                            method.name.clone(),
                            MethodInfo {
                                writable_this: method.writable_this,
                            },
                        );

                        if method.name == "__construct" {
                            for param in &method.params {
                                if param.promoted_visibility.is_some() {
                                    info.properties.insert(
                                        param.name.clone(),
                                        PropertyInfo {
                                            writable: param.writable,
                                            ty: param.ty.clone(),
                                        },
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
            if let ClassMember::Method(method) = member {
                self.check_function(
                    method,
                    Some(MethodContext {
                        class_name: class_decl.name.clone(),
                        writable_this: method.writable_this,
                    }),
                );
            }
        }
    }

    fn check_function(&mut self, function: &FunctionDecl, method_context: Option<MethodContext>) {
        let mut scopes = ScopeStack::new();
        for param in &function.params {
            scopes.declare(
                param.name.clone(),
                Binding {
                    writable: param.writable,
                    ty: param.ty.clone(),
                },
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
                let ty = decl.ty.clone().unwrap_or_else(|| {
                    self.infer_expr_type(&decl.initializer, scopes, method_context)
                });
                scopes.declare(
                    decl.name.clone(),
                    Binding {
                        writable: decl.writable,
                        ty,
                    },
                );
            }
            Stmt::Assignment(assignment) => {
                self.check_expr(&assignment.value, scopes, method_context);
                self.check_assignment_target(&assignment.target, scopes, method_context);
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
                    scopes.declare(
                        key.name.clone(),
                        Binding {
                            writable: false,
                            ty: key.ty.clone().unwrap_or_else(TypeRef::unknown),
                        },
                    );
                }
                scopes.declare(
                    foreach.value.name.clone(),
                    Binding {
                        writable: false,
                        ty: foreach.value.ty.clone().unwrap_or_else(TypeRef::unknown),
                    },
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
                if method_context.is_none() {
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
            Expr::PropertyAccess { object, .. } => {
                self.check_expr(object, scopes, method_context);
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
            Expr::FunctionCall { args, .. }
            | Expr::StaticCall { args, .. }
            | Expr::New { args, .. } => {
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
    ) {
        match target {
            Expr::Variable { name, span } => match scopes.lookup(name) {
                Some(binding) if binding.writable => {}
                Some(_) => self.diagnostics.push(
                    Diagnostic::new(
                        "E0201",
                        format!("cannot assign to readonly variable `${name}`"),
                        *span,
                    )
                    .with_help(format!(
                        "declare it as `let writable ${name} = ...` if mutation is intended"
                    )),
                ),
                None => self.undeclared_variable(name, *span),
            },
            Expr::PropertyAccess {
                object,
                property,
                span,
            } => {
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

                if let Some(class_name) = self.expr_class_name(object, scopes, method_context) {
                    if let Some(class_info) = self.classes.get(&class_name) {
                        if let Some(property_info) = class_info.properties.get(property) {
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
                                        "mark the property writable: `public writable {} ${property};`",
                                        property_info.ty
                                    )),
                                );
                            }
                        }
                    }
                }
            }
            _ => self.diagnostics.push(Diagnostic::new(
                "E0204",
                "unsupported assignment target",
                target.span(),
            )),
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
        let Some(class_info) = self.classes.get(&class_name) else {
            return;
        };
        let Some(method_info) = class_info.methods.get(method) else {
            return;
        };

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

    fn is_writable_object_path(
        &self,
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
                .map(|context| context.writable_this)
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

    fn infer_expr_type(
        &self,
        expr: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> TypeRef {
        match expr {
            Expr::String { .. } => TypeRef::named("string"),
            Expr::Int { .. } => TypeRef::named("int"),
            Expr::Float { .. } => TypeRef::named("float"),
            Expr::Bool { .. } => TypeRef::named("bool"),
            Expr::Null { .. } => TypeRef::named("null"),
            Expr::New { class_name, .. } => TypeRef::named(class_name.clone()),
            Expr::Array { elements, .. } => {
                if elements.iter().any(|element| element.key.is_some()) {
                    TypeRef::generic("Dictionary", vec![TypeRef::unknown(), TypeRef::unknown()])
                } else {
                    TypeRef::generic("List", vec![TypeRef::unknown()])
                }
            }
            Expr::Variable { name, .. } => scopes
                .lookup(name)
                .map(|binding| binding.ty.clone())
                .unwrap_or_else(TypeRef::unknown),
            Expr::This { .. } => method_context
                .map(|context| TypeRef::named(context.class_name.clone()))
                .unwrap_or_else(TypeRef::unknown),
            Expr::PropertyAccess {
                object, property, ..
            } => {
                let Some(class_name) = self.expr_class_name(object, scopes, method_context) else {
                    return TypeRef::unknown();
                };
                self.classes
                    .get(&class_name)
                    .and_then(|class_info| class_info.properties.get(property))
                    .map(|property| property.ty.clone())
                    .unwrap_or_else(TypeRef::unknown)
            }
            _ => TypeRef::unknown(),
        }
    }

    fn expr_class_name(
        &self,
        expr: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> Option<String> {
        self.infer_expr_type(expr, scopes, method_context)
            .as_class_name()
            .map(ToOwned::to_owned)
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

use std::collections::{HashMap, HashSet};

use crate::ast::*;
use crate::builtins::{php_function_suggestion, Builtin};
use crate::class_layout::{ClassId, PropertyId};
use crate::diagnostics::{Diagnostic, DiagnosticResult};
use crate::format_string::{self, FormatConversion, FormatPiece};
use crate::numeric::{parse_decimal_magnitude, FloatType, FloatValue, IntegerType, IntegerValue};
use crate::source::Span;
use crate::symbols::{
    Binding, ClassInfo, ConstantInfo, FunctionInfo, MemberDeclaration, MemberKind, MethodInfo,
    ParamInfo, PropertyInfo, PropertyInitState, ReceiverMode, ScopeStack, StaticPropertyInfo,
};
use crate::types::{TypeId, TypeKind, TypeRef, TypeRegistry};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SemanticInfo {
    /// Canonical integer type for every integer-valued source expression.
    ///
    /// Spans are stable across AST-to-HIR structural lowering, so the MIR
    /// lowering pass can consume semantic decisions without re-parsing type
    /// names or guessing contextual literal types.
    pub integer_expression_types: HashMap<(usize, usize), IntegerType>,
    /// Canonical width for every floating-point-valued source expression.
    pub float_expression_types: HashMap<(usize, usize), FloatType>,
    /// Stable nominal class identities and the total Stage 19 property order.
    pub classes: Vec<ClassSemanticInfo>,
    /// Values produced by the bounded Stage 20 constant evaluator.
    pub const_evaluation: crate::const_eval::Evaluation,
    /// Const-folded Copy-scalar defaults keyed by callable and parameter identity.
    pub parameter_defaults:
        HashMap<crate::const_eval::ParameterDefaultKey, crate::const_eval::ConstValue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassSemanticInfo {
    pub id: ClassId,
    pub name: String,
    pub implements_displayable: bool,
    pub properties: Vec<PropertySemanticInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertySemanticInfo {
    pub id: PropertyId,
    pub name: String,
    pub ty: TypeRef,
    pub writable: bool,
    pub promoted: bool,
}

impl SemanticInfo {
    pub fn integer_type(&self, span: Span) -> Option<IntegerType> {
        self.integer_expression_types
            .get(&(span.start, span.end))
            .copied()
    }

    pub fn float_type(&self, span: Span) -> Option<FloatType> {
        self.float_expression_types
            .get(&(span.start, span.end))
            .copied()
    }
}

pub fn analyze_program(program: &Program) -> DiagnosticResult<SemanticInfo> {
    let (const_evaluation, const_diagnostics) = match crate::const_eval::evaluate_program(program) {
        Ok(evaluation) => (evaluation, Vec::new()),
        Err(diagnostics) => (crate::const_eval::Evaluation::default(), diagnostics),
    };
    let mut checker = Checker::new(program, const_evaluation);
    checker.diagnostics.extend(const_diagnostics);
    checker.check();
    if checker.diagnostics.is_empty() {
        let inferred_move_returns = checker
            .function_signatures
            .iter()
            .filter_map(|(span_start, signature)| {
                checker
                    .type_is_move_type(signature.return_ty)
                    .then_some(*span_start)
            })
            .collect();
        checker
            .diagnostics
            .extend(crate::ownership::check_program_with_inferred_move_returns(
                program,
                &inferred_move_returns,
            ));
    }
    if checker.diagnostics.is_empty() {
        Ok(SemanticInfo {
            integer_expression_types: checker.integer_expression_types,
            float_expression_types: checker.float_expression_types,
            classes: collect_ordered_class_semantics(program),
            const_evaluation: checker.const_evaluation,
            parameter_defaults: checker.parameter_defaults,
        })
    } else {
        Err(checker.diagnostics)
    }
}

fn collect_ordered_class_semantics(program: &Program) -> Vec<ClassSemanticInfo> {
    program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Class(class) => Some(class),
            _ => None,
        })
        .enumerate()
        .map(|(class_index, class)| {
            let id = ClassId(class_index);
            let explicit = class.members.iter().filter_map(|member| match member {
                ClassMember::Property(property) if !property.is_static => Some((
                    property.name.clone(),
                    property.ty.resolve_self_in(&class.name),
                    property.writable,
                    false,
                )),
                ClassMember::Property(_) | ClassMember::Method(_) | ClassMember::Constant(_) => {
                    None
                }
            });
            let promoted = class.members.iter().find_map(|member| match member {
                ClassMember::Method(method) if method.name == "__construct" => {
                    Some(method.params.iter().filter_map(|param| {
                        param.promoted_access.as_ref().map(|_| {
                            (
                                param.name.clone(),
                                param.ty.resolve_self_in(&class.name),
                                param.writable,
                                true,
                            )
                        })
                    }))
                }
                _ => None,
            });
            let mut properties = explicit.collect::<Vec<_>>();
            if let Some(promoted) = promoted {
                properties.extend(promoted);
            }
            ClassSemanticInfo {
                id,
                name: class.name.clone(),
                implements_displayable: class
                    .implements
                    .iter()
                    .any(|interface| interface == "Displayable"),
                properties: properties
                    .into_iter()
                    .enumerate()
                    .map(
                        |(index, (name, ty, writable, promoted))| PropertySemanticInfo {
                            id: PropertyId { class: id, index },
                            name,
                            ty,
                            writable,
                            promoted,
                        },
                    )
                    .collect(),
            }
        })
        .collect()
}

pub fn check_program(program: &Program) -> DiagnosticResult<()> {
    analyze_program(program).map(|_| ())
}

pub(crate) fn interface_declaration_diagnostic(interface_decl: &InterfaceDecl) -> Diagnostic {
    let (code, message) = if interface_decl.name == "Displayable" {
        (
            "E0309",
            "`Displayable` is a compiler-known interface and cannot be redeclared".to_string(),
        )
    } else {
        (
            "E0464",
            format!(
                "interface declaration `{}` is accepted syntax but is not available in this compiler version",
                interface_decl.name
            ),
        )
    };
    Diagnostic::new(code, message, interface_decl.span)
}

pub(crate) fn trait_declaration_diagnostic(trait_decl: &TraitDecl) -> Diagnostic {
    Diagnostic::unsupported_stage(
        "E0493",
        format!(
            "trait declaration `{}` is accepted syntax; trait composition semantics land in Stage 35",
            trait_decl.name
        ),
        trait_decl.span,
    )
}

struct Checker<'program> {
    program: &'program Program,
    classes: HashMap<String, ClassInfo>,
    functions: HashMap<String, FunctionInfo>,
    function_signatures: HashMap<usize, FunctionInfo>,
    types: TypeRegistry,
    diagnostics: Vec<Diagnostic>,
    integer_expression_types: HashMap<(usize, usize), IntegerType>,
    float_expression_types: HashMap<(usize, usize), FloatType>,
    integer_literals: HashMap<(usize, usize), u128>,
    negative_integer_literals: HashMap<(usize, usize), u128>,
    negated_integer_literal_operands: HashSet<(usize, usize)>,
    const_evaluation: crate::const_eval::Evaluation,
    parameter_defaults:
        HashMap<crate::const_eval::ParameterDefaultKey, crate::const_eval::ConstValue>,
}

#[derive(Debug, Clone)]
struct MethodContext {
    class_name: String,
    receiver_mode: Option<ReceiverMode>,
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

#[derive(Debug, Clone, Copy)]
struct StaticAccess<'a> {
    qualifier: &'a StaticQualifier,
    qualifier_span: Span,
    member_sigil_span: Option<Span>,
    member: &'a str,
    span: Span,
}

#[derive(Debug, Clone)]
enum AssignmentDestination {
    Type,
    Parameter { name: String },
    Property { class_name: String, name: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IntConstantEval {
    Known(IntegerValue),
    Unknown,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DisplayConversionKind {
    Primitive,
    DisplayableClass,
    NonDisplayableClass,
    Excluded,
    Recovery,
}

impl<'program> Checker<'program> {
    fn new(program: &'program Program, const_evaluation: crate::const_eval::Evaluation) -> Self {
        Self {
            program,
            classes: HashMap::new(),
            functions: HashMap::new(),
            function_signatures: HashMap::new(),
            types: TypeRegistry::new(),
            diagnostics: Vec::new(),
            integer_expression_types: HashMap::new(),
            float_expression_types: HashMap::new(),
            integer_literals: HashMap::new(),
            negative_integer_literals: HashMap::new(),
            negated_integer_literal_operands: HashSet::new(),
            const_evaluation,
            parameter_defaults: HashMap::new(),
        }
    }

    fn check(&mut self) {
        if let Some(namespace) = &self.program.namespace {
            self.diagnostics.push(Diagnostic::new(
                "E0475",
                "namespace declarations are accepted syntax but namespace resolution is not available in this compiler version",
                namespace.span,
            ));
        }
        self.collect_classes();
        self.collect_functions();
        self.infer_unannotated_move_return_signatures();

        let mut scopes = ScopeStack::new();
        for item in &self.program.items {
            match item {
                Item::Statement(statement) => {
                    self.check_statement(statement, &mut scopes, None, None, None, 0);
                }
                Item::Function(function) => self.check_function(function, None),
                Item::Constant(constant) => {
                    self.check_constant_initializer(&constant.initializer, None)
                }
                Item::Class(class_decl) => self.check_class(class_decl),
                Item::Interface(interface_decl) => {
                    self.diagnostics
                        .push(interface_declaration_diagnostic(interface_decl));
                }
                Item::Trait(trait_decl) => {
                    self.diagnostics
                        .push(trait_declaration_diagnostic(trait_decl));
                }
            }
        }
        self.check_pending_integer_literal_ranges();
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
                    implements_displayable: false,
                    properties: HashMap::new(),
                    static_properties: HashMap::new(),
                    constants: HashMap::new(),
                    methods: HashMap::new(),
                    members: HashMap::new(),
                },
            );
            declared_classes.push(class_decl);
        }

        for class_decl in declared_classes {
            let mut info = ClassInfo {
                implements_displayable: class_decl
                    .implements
                    .iter()
                    .any(|name| name == "Displayable"),
                properties: HashMap::new(),
                static_properties: HashMap::new(),
                constants: HashMap::new(),
                methods: HashMap::new(),
                members: HashMap::new(),
            };

            for member in &class_decl.members {
                match member {
                    ClassMember::Property(property) => {
                        if property.is_static {
                            self.declare_static_property(&mut info, &class_decl.name, property);
                        } else {
                            self.declare_property(&mut info, &class_decl.name, property);
                        }
                    }
                    ClassMember::Constant(constant) => {
                        self.declare_class_constant(&mut info, &class_decl.name, constant);
                    }
                    ClassMember::Method(method) => {
                        if let Some(message) = Self::reserved_callable_name_message(&method.name) {
                            self.diagnostics
                                .push(Diagnostic::new("E0310", message, method.span));
                            continue;
                        }

                        let signature =
                            self.resolve_function_signature(method, Some(&class_decl.name));
                        self.function_signatures
                            .insert(method.span.start, signature.clone());

                        self.check_lifecycle_declaration_shape(method);

                        if method.is_static && method.writable_this {
                            self.diagnostics.push(
                                Diagnostic::new(
                                    "E0497",
                                    "static methods cannot be declared `writable` because they have no `$this` receiver",
                                    method.span,
                                )
                                .with_help("remove `writable`; mutate writable static properties through `ClassName::member` or `self::member`"),
                            );
                        }

                        if method.name == "__destruct" && !method.params.is_empty() {
                            self.diagnostics.push(Diagnostic::new(
                                "E0411",
                                "destructor `__destruct` cannot declare parameters",
                                method.span,
                            ));
                        }

                        let kind = if method.is_static {
                            MemberKind::StaticMethod
                        } else {
                            MemberKind::InstanceMethod
                        };
                        if self.declare_member_name(
                            &mut info,
                            &class_decl.name,
                            &method.name,
                            kind,
                            method.span,
                        ) {
                            info.methods.insert(
                                method.name.clone(),
                                MethodInfo {
                                    access: method.access.clone(),
                                    receiver_mode: (!method.is_static).then_some(
                                        if method.writable_this
                                            && LifecycleMethod::from_method_name(&method.name)
                                                .is_none()
                                        {
                                            ReceiverMode::Writable
                                        } else {
                                            ReceiverMode::Readonly
                                        },
                                    ),
                                    is_static: method.is_static,
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

            self.check_class_interfaces(class_decl, &info);

            self.classes.insert(class_decl.name.clone(), info);
        }
    }

    fn check_class_interfaces(&mut self, class_decl: &ClassDecl, info: &ClassInfo) {
        let mut seen = HashSet::new();
        for interface in &class_decl.implements {
            if !seen.insert(interface) {
                self.diagnostics.push(Diagnostic::new(
                    "E0464",
                    format!(
                        "class `{}` implements `{interface}` more than once",
                        class_decl.name
                    ),
                    class_decl.span,
                ));
                continue;
            }
            if interface != "Displayable" {
                self.diagnostics.push(Diagnostic::new(
                    "E0464",
                    format!(
                        "general interface conformance for `{interface}` is accepted syntax but is not available in this compiler version"
                    ),
                    class_decl.span,
                ));
            }
        }

        if !info.implements_displayable {
            return;
        }

        let Some(method) = info.methods.get("toString") else {
            let help = if info.methods.contains_key("__toString") {
                "Doria does not use `__toString`; declare `function toString(): string`"
            } else if info.methods.contains_key("to_string") {
                "Doria member names use camelCase; declare `function toString(): string`"
            } else {
                "declare `function toString(): string`"
            };
            self.diagnostics.push(
                Diagnostic::new(
                    "E0463",
                    format!(
                        "class `{}` implements `Displayable` but does not provide `toString(): string`",
                        class_decl.name
                    ),
                    class_decl.span,
                )
                .with_help(help),
            );
            return;
        };

        let valid = method.access == MemberAccess::External
            && method.receiver_mode == Some(ReceiverMode::Readonly)
            && !method.is_static
            && method.params.is_empty()
            && matches!(self.types.kind(method.return_ty), TypeKind::String);
        if !valid {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0463",
                    format!(
                        "class `{}` has an incompatible `Displayable::toString` method",
                        class_decl.name
                    ),
                    class_decl.span,
                )
                .with_help(
                    "declare exactly `function toString(): string` as an externally accessible readonly instance method",
                ),
            );
        }
    }

    fn check_lifecycle_declaration_shape(&mut self, method: &FunctionDecl) {
        let Some(lifecycle) = LifecycleMethod::from_method_name(&method.name) else {
            return;
        };

        if let Some(span) = method.static_span {
            let message = match lifecycle {
                LifecycleMethod::Constructor => {
                    "`__construct` is invoked by `new` and cannot be `static`"
                }
                LifecycleMethod::Destructor => {
                    "`__destruct` is invoked automatically when an instance is destroyed and cannot be `static`"
                }
            };
            self.diagnostics
                .push(Diagnostic::new("E0465", message, span));
        }

        if let Some(span) = method.writable_span {
            let help = match lifecycle {
                LifecycleMethod::Constructor => {
                    "remove `writable`; construction grants `__construct` its access to the new instance"
                }
                LifecycleMethod::Destructor => {
                    "remove `writable`; destruction invokes `__destruct` through the lifecycle protocol"
                }
            };
            self.diagnostics.push(
                Diagnostic::new(
                    "E0466",
                    format!("`{}` cannot be declared `writable`", lifecycle.doria_name()),
                    span,
                )
                .with_help(help)
                .with_fix(span, ""),
            );
        }
    }

    fn reserved_class_name_message(name: &str) -> Option<String> {
        if matches!(name, "Float" | "Float32" | "Float64" | "Bool") {
            return Some(format!(
                "`{name}` is a compiler-known scalar companion and cannot be redeclared"
            ));
        }
        if IntegerType::from_companion_name(name).is_some() {
            return Some(format!(
                "`{name}` is a compiler-known integer companion and cannot be redeclared"
            ));
        }
        if name.eq_ignore_ascii_case("__DoriaDisplayable") {
            return Some(
                "`__DoriaDisplayable` is reserved for compiler-generated PHP compatibility output"
                    .to_string(),
            );
        }
        match name {
            "self" => Some(
                "`self` is reserved for the declaring or composing class context and cannot be used as a class name"
                    .to_string(),
            ),
            "Displayable" => Some(
                "`Displayable` is a compiler-known interface and cannot be redeclared"
                    .to_string(),
            ),
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

    fn reserved_callable_name_message(name: &str) -> Option<String> {
        if matches!(name, "Float" | "Float32" | "Float64" | "Bool") {
            return Some(format!(
                "`{name}` is a compiler-known scalar companion and cannot be redeclared"
            ));
        }
        if IntegerType::from_companion_name(name).is_some() {
            return Some(format!(
                "`{name}` is a compiler-known integer companion and cannot be redeclared"
            ));
        }
        match name {
            name if php_function_suggestion(name) == Some("read_line") => Some(format!(
                "Doria uses `read_line`; the PHP spelling `{name}` cannot be declared"
            )),
            name if Builtin::from_name(name).is_some() => Some(format!(
                "`{name}` is a compiler-known Doria built-in and cannot be redeclared"
            )),
            "array" => Some(
                "`array` is not a Doria callable name; use typed arrays like `T[]` or collection aliases"
                    .to_string(),
            ),
            "mixed" => Some(
                "`mixed` is a Doria dynamic-boundary type and cannot be used as a callable name"
                    .to_string(),
            ),
            "object" => Some(
                "`object` is not a Doria type and cannot be used as a callable name".to_string(),
            ),
            "resource" => Some(
                "`resource` is reserved for future PHP interop and cannot be used as a callable name"
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

            if function
                .name
                .get(.."__doria_".len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("__doria_"))
            {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0310",
                        "top-level function names beginning with `__doria_` are reserved for compiler-generated helpers",
                        function.span,
                    )
                    .with_help("choose a function name that does not begin with `__doria_`"),
                );
                continue;
            }

            if function.name == "print" {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0310",
                        "Doria does not support a top-level `print` function; use `echo`",
                        function.span,
                    )
                    .with_help("remove the `print` declaration and use `echo` for output"),
                );
                continue;
            }

            if let Some(message) = Self::reserved_callable_name_message(&function.name) {
                self.diagnostics
                    .push(Diagnostic::new("E0310", message, function.span));
                continue;
            }

            let signature = self.resolve_function_signature(function, None);
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

    fn resolve_function_signature(
        &mut self,
        function: &FunctionDecl,
        declaring_class: Option<&str>,
    ) -> FunctionInfo {
        let params = self.resolve_param_infos(function, declaring_class);
        let return_ty = self.resolve_function_return_type(function, declaring_class);

        FunctionInfo { params, return_ty }
    }

    fn resolve_param_infos(
        &mut self,
        function: &FunctionDecl,
        declaring_class: Option<&str>,
    ) -> Vec<ParamInfo> {
        let mut params = Vec::new();
        let mut saw_optional = false;

        for param in &function.params {
            let ty = self.resolve_type_ref_with_class(&param.ty, param.span, declaring_class);
            let has_default = param.default.is_some();

            if param.take && param.writable {
                let span = param
                    .take_span
                    .zip(param.writable_span)
                    .map(|(take, writable)| take.merge(writable))
                    .unwrap_or(param.span);
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0467",
                        "a parameter cannot be both `take` and `writable`",
                        span,
                    )
                    .with_help(
                        "use `take` to give ownership to the callee, or `writable` for exclusive mutation without giving ownership",
                    ),
                );
            }

            if param.promoted_access.is_some() && self.type_is_move_type(ty) && !param.take {
                let diagnostic = Diagnostic::new(
                    "E0468",
                    format!(
                        "promoted move-type parameter `${}` must use `take`",
                        param.name
                    ),
                    param.span,
                );
                self.diagnostics
                    .push(if let Some(writable_span) = param.writable_span {
                        diagnostic
                            .with_help(
                                "promotion transfers ownership; replace `writable` with `take`",
                            )
                            .with_fix(writable_span, "take")
                    } else {
                        diagnostic
                        .with_help(
                            "promotion gives ownership directly to the new property; insert `take`",
                        )
                        .with_fix(param.ownership_modifier_insert, "take ")
                    });
            }

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
                take: param.take,
                writable: param.writable,
                has_default,
            });
        }

        params
    }

    fn resolve_function_return_type(
        &mut self,
        function: &FunctionDecl,
        declaring_class: Option<&str>,
    ) -> TypeId {
        function
            .return_type
            .as_ref()
            .map(|return_type| {
                self.resolve_type_ref_in_position(
                    return_type,
                    function.span,
                    TypePosition::Return,
                    declaring_class,
                )
            })
            .unwrap_or_else(|| self.types.unknown())
    }

    fn infer_unannotated_move_return_signatures(&mut self) {
        let max_iterations = self.move_return_inference_signature_count();

        for _ in 0..max_iterations {
            let mut changed = false;

            for item in &self.program.items {
                match item {
                    Item::Function(function) => {
                        changed |= self.update_function_move_return_signature(function);
                    }
                    Item::Class(class_decl) => {
                        for member in &class_decl.members {
                            let ClassMember::Method(method) = member else {
                                continue;
                            };
                            changed |=
                                self.update_method_move_return_signature(&class_decl.name, method);
                        }
                    }
                    Item::Interface(_)
                    | Item::Trait(_)
                    | Item::Constant(_)
                    | Item::Statement(_) => {}
                }
            }

            if !changed {
                break;
            }
        }
    }

    fn move_return_inference_signature_count(&self) -> usize {
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
                        ClassMember::Property(_) | ClassMember::Constant(_) => false,
                    })
                    .count(),
                _ => 0,
            })
            .sum::<usize>()
            .max(1)
    }

    fn update_function_move_return_signature(&mut self, function: &FunctionDecl) -> bool {
        if function.return_type.is_some() {
            return false;
        }

        let Some(signature) = self.function_signatures.get(&function.span.start).cloned() else {
            return false;
        };
        let inferred = self.infer_unannotated_move_return_type(function, &signature.params, None);

        if !self.type_is_move_type(inferred) || signature.return_ty == inferred {
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

    fn update_method_move_return_signature(
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
            receiver_mode: Some(if method.writable_this {
                ReceiverMode::Writable
            } else {
                ReceiverMode::Readonly
            }),
            this_available: true,
        };
        let inferred = self.infer_unannotated_move_return_type(
            method,
            &signature.params,
            Some(&method_context),
        );

        if !self.type_is_move_type(inferred) || signature.return_ty == inferred {
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

    fn infer_unannotated_move_return_type(
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
                    declared_ty: param.ty,
                    int_constant: None,
                    string_constant: None,
                },
            );
        }

        self.infer_move_return_from_block(&function.body, &mut scopes, method_context)
            .unwrap_or_else(|| self.types.unknown())
    }

    fn infer_move_return_from_statements(
        &mut self,
        statements: &[Stmt],
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> Option<TypeId> {
        let mut inferred = None;

        for statement in statements {
            let statement_ty =
                self.infer_move_return_from_statement(statement, scopes, method_context);
            inferred = self.merge_optional_inferred_return_types(inferred, statement_ty);

            if !crate::return_analysis::statement_falls_through(statement) {
                break;
            }
        }

        inferred
    }

    fn infer_move_return_from_statement(
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
                        declared_ty: ty,
                        int_constant: None,
                        string_constant: None,
                    },
                );
                None
            }
            Stmt::Assignment(assignment) => {
                self.infer_move_return_from_assignment(assignment, scopes, method_context);
                None
            }
            Stmt::Return {
                expr: Some(expr), ..
            } => Some(self.infer_expr_type(expr, scopes, method_context)),
            Stmt::If(if_stmt) => {
                self.infer_move_return_from_if_statement(if_stmt, scopes, method_context)
            }
            Stmt::While(while_stmt) => {
                self.infer_move_return_from_block(&while_stmt.body, scopes, method_context)
            }
            Stmt::For(for_stmt) => {
                scopes.push();
                if let Some(initializer) = &for_stmt.initializer {
                    self.infer_move_return_from_for_initializer(
                        initializer,
                        scopes,
                        method_context,
                    );
                }
                let result =
                    self.infer_move_return_from_block(&for_stmt.body, scopes, method_context);
                if let Some(increment) = &for_stmt.increment {
                    self.infer_move_return_from_for_increment(increment, scopes, method_context);
                }
                scopes.pop();
                result
            }
            Stmt::Foreach(foreach) => {
                self.infer_move_return_from_foreach(foreach, scopes, method_context)
            }
            _ => None,
        }
    }

    fn infer_move_return_from_if_statement(
        &mut self,
        if_stmt: &IfStmt,
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> Option<TypeId> {
        let incoming_scopes = scopes.clone();
        let mut falling_through_scopes = Vec::new();

        let mut then_scopes = incoming_scopes.clone();
        let mut inferred = self.infer_move_return_from_block(
            &if_stmt.then_block,
            &mut then_scopes,
            method_context,
        );
        if crate::return_analysis::block_falls_through(&if_stmt.then_block) {
            falling_through_scopes.push(then_scopes);
        }

        if let Some(branch) = &if_stmt.else_branch {
            let mut else_scopes = incoming_scopes;
            let branch_ty =
                self.infer_move_return_from_else_branch(branch, &mut else_scopes, method_context);
            inferred = self.merge_optional_inferred_return_types(inferred, branch_ty);
            if crate::return_analysis::else_branch_falls_through(branch) {
                falling_through_scopes.push(else_scopes);
            }
        } else {
            falling_through_scopes.push(incoming_scopes);
        }

        scopes.replace_types_from_branches(&falling_through_scopes, |left, right| {
            self.merge_inferred_binding_type(left, right)
        });

        inferred
    }
    fn infer_move_return_from_block(
        &mut self,
        block: &Block,
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> Option<TypeId> {
        scopes.push();
        let inferred =
            self.infer_move_return_from_statements(&block.statements, scopes, method_context);
        scopes.pop();
        inferred
    }

    fn infer_move_return_from_else_branch(
        &mut self,
        branch: &ElseBranch,
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> Option<TypeId> {
        match branch {
            ElseBranch::If(if_stmt) => self.infer_move_return_from_statement(
                &Stmt::If((**if_stmt).clone()),
                scopes,
                method_context,
            ),
            ElseBranch::Block(block) => {
                self.infer_move_return_from_block(block, scopes, method_context)
            }
        }
    }

    fn infer_move_return_from_for_initializer(
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
                        declared_ty: ty,
                        int_constant: None,
                        string_constant: None,
                    },
                );
            }
            ForInitializer::Assignment(assignment) => {
                self.infer_move_return_from_assignment(assignment, scopes, method_context);
            }
        }
    }

    fn infer_move_return_from_for_increment(
        &mut self,
        increment: &ForIncrement,
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        if let ForIncrement::Assignment(assignment) = increment {
            self.infer_move_return_from_assignment(assignment, scopes, method_context);
        }
    }

    fn infer_move_return_from_assignment(
        &mut self,
        assignment: &Assignment,
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        if !matches!(assignment.op, AssignOp::Assign) {
            return;
        }

        if let Some(name) = Self::assignment_target_variable_name(&assignment.target) {
            let ty = self.infer_expr_type(&assignment.value, scopes, method_context);
            if let Some(binding) = scopes.lookup_mut(name) {
                binding.ty = self.merge_inferred_binding_type(binding.ty, ty);
            }
        }
    }

    fn assignment_target_variable_name(target: &Expr) -> Option<&str> {
        match target {
            Expr::Grouped { expr, .. } => Self::assignment_target_variable_name(expr),
            Expr::Variable { name, .. } => Some(name),
            _ => None,
        }
    }

    fn infer_move_return_from_foreach(
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
                    declared_ty: ty,
                    int_constant: None,
                    string_constant: None,
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
                declared_ty: value_ty,
                int_constant: None,
                string_constant: None,
            },
        );

        let inferred = self.infer_move_return_from_statements(
            &foreach.body.statements,
            scopes,
            method_context,
        );
        scopes.pop();
        inferred
    }

    fn merge_optional_inferred_return_types(
        &mut self,
        current: Option<TypeId>,
        next: Option<TypeId>,
    ) -> Option<TypeId> {
        let next = next?;
        if matches!(self.types.kind(next), TypeKind::Unknown) {
            return current;
        }
        Some(match current {
            Some(current) if matches!(self.types.kind(current), TypeKind::Unknown) => next,
            Some(current) => self.merge_inferred_return_types(current, next),
            None => next,
        })
    }

    fn merge_inferred_return_types(&mut self, left: TypeId, right: TypeId) -> TypeId {
        if left == right {
            return left;
        }

        let left_kind = self.types.kind(left).clone();
        let right_kind = self.types.kind(right).clone();
        match (left_kind, right_kind) {
            (TypeKind::List(left), TypeKind::List(right)) => {
                let element = self.merge_inferred_return_types(left, right);
                self.types.intern(TypeKind::List(element))
            }
            (TypeKind::TypedArray(left), TypeKind::TypedArray(right)) => {
                let element = self.merge_inferred_return_types(left, right);
                self.types.intern(TypeKind::TypedArray(element))
            }
            (
                TypeKind::Dictionary(left_key, left_value),
                TypeKind::Dictionary(right_key, right_value),
            ) => {
                let key = self.merge_inferred_return_types(left_key, right_key);
                let value = self.merge_inferred_return_types(left_value, right_value);
                self.types.intern(TypeKind::Dictionary(key, value))
            }
            (TypeKind::Set(left), TypeKind::Set(right)) => {
                let element = self.merge_inferred_return_types(left, right);
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
            self.merge_inferred_return_types(current, next)
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
            let integer = self
                .range_integer_type(&foreach.iterable, scopes, method_context)
                .unwrap_or(IntegerType::Int64);
            return (unknown, self.types.intern(TypeKind::Integer(integer)));
        }

        let iterable_ty = self.infer_expr_type(&foreach.iterable, scopes, method_context);
        match self.types.kind(iterable_ty).clone() {
            TypeKind::List(value) | TypeKind::Set(value) => (
                self.types.intern(TypeKind::Integer(IntegerType::Int64)),
                value,
            ),
            TypeKind::TypedArray(value) => (
                self.types.intern(TypeKind::Integer(IntegerType::Int64)),
                value,
            ),
            TypeKind::Dictionary(key, value) => (key, value),
            TypeKind::Mixed => (unknown, self.types.intern(TypeKind::Mixed)),
            _ => (unknown, unknown),
        }
    }

    fn resolve_type_ref_for_return_inference(&mut self, ty: &TypeRef) -> TypeId {
        if ty.nullable {
            return if ty.name == "string" && ty.args.is_empty() {
                self.types.intern(TypeKind::NullableString)
            } else {
                self.types.unknown()
            };
        }
        if ty.args.is_empty() {
            if let Some(integer) = IntegerType::from_source_name(&ty.name) {
                return self.types.intern(TypeKind::Integer(integer));
            }
            if let Some(float) = FloatType::from_source_name(&ty.name) {
                return self.types.intern(TypeKind::Float(float));
            }
        }
        match ty.name.as_str() {
            "void" if ty.args.is_empty() => self.types.intern(TypeKind::Void),
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
        if class_decl.parent.is_some() {
            self.diagnostics.push(Diagnostic::new(
                "E0476",
                "class inheritance is accepted syntax but `extends` semantics are not available in this compiler version",
                class_decl.parent_span.unwrap_or(class_decl.span),
            ));
        }
        for member in &class_decl.members {
            match member {
                ClassMember::Property(property) => {
                    if property.is_static {
                        if let Some(initializer) = &property.initializer {
                            self.check_constant_initializer(initializer, Some(&class_decl.name));
                        }
                    } else {
                        self.check_property_initializer(&class_decl.name, property);
                    }
                }
                ClassMember::Constant(constant) => {
                    self.check_constant_initializer(&constant.initializer, Some(&class_decl.name))
                }
                ClassMember::Method(method) => {
                    let lifecycle = LifecycleMethod::from_method_name(&method.name);
                    self.check_function(
                        method,
                        Some(MethodContext {
                            class_name: class_decl.name.clone(),
                            receiver_mode: (!method.is_static).then_some(
                                if method.writable_this && lifecycle.is_none() {
                                    ReceiverMode::Writable
                                } else {
                                    ReceiverMode::Readonly
                                },
                            ),
                            this_available: !method.is_static,
                        }),
                    );
                }
            }
        }
    }

    fn check_constant_initializer(&mut self, initializer: &Expr, class_name: Option<&str>) {
        let scopes = ScopeStack::new();
        let context = class_name.map(|class_name| MethodContext {
            class_name: class_name.to_string(),
            receiver_mode: None,
            this_available: false,
        });
        self.check_expr(initializer, &scopes, context.as_ref());
    }

    fn check_property_initializer(&mut self, class_name: &str, property: &PropertyDecl) {
        let Some(initializer) = &property.initializer else {
            return;
        };

        let scopes = ScopeStack::new();
        let initializer_context = MethodContext {
            class_name: class_name.to_string(),
            receiver_mode: None,
            this_available: false,
        };
        self.check_expr(initializer, &scopes, Some(&initializer_context));
        let target_ty = self
            .classes
            .get(class_name)
            .and_then(|class_info| class_info.properties.get(&property.name))
            .map(|property| property.ty)
            .unwrap_or_else(|| {
                self.resolve_type_ref_with_class(&property.ty, property.span, Some(class_name))
            });
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
        if !self.declare_member_name(
            info,
            class_name,
            &property.name,
            MemberKind::InstanceProperty,
            property.span,
        ) {
            return;
        }

        let ty = self.resolve_type_ref_with_class(&property.ty, property.span, Some(class_name));
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

    fn declare_static_property(
        &mut self,
        info: &mut ClassInfo,
        class_name: &str,
        property: &PropertyDecl,
    ) {
        if !self.declare_member_name(
            info,
            class_name,
            &property.name,
            MemberKind::StaticProperty,
            property.span,
        ) {
            return;
        }
        let ty = self.resolve_type_ref_with_class(&property.ty, property.span, Some(class_name));
        if self.type_is_move_type(ty) {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0486",
                    format!(
                        "static property `{class_name}::{}` cannot use owned type `{}`",
                        property.name,
                        self.types.display(ty)
                    ),
                    property.span,
                )
                .with_help(
                    "owned-static lifetime and concurrency rules are deferred pending Sendable/Shareable design",
                ),
            );
        }
        info.static_properties.insert(
            property.name.clone(),
            StaticPropertyInfo {
                access: property.access.clone(),
                writable: property.writable,
                ty,
            },
        );
    }

    fn declare_class_constant(
        &mut self,
        info: &mut ClassInfo,
        class_name: &str,
        constant: &ConstDecl,
    ) {
        if !self.declare_member_name(
            info,
            class_name,
            &constant.name,
            MemberKind::Constant,
            constant.span,
        ) {
            return;
        }
        let key = crate::const_eval::ConstKey::Class {
            class_name: class_name.to_string(),
            name: constant.name.clone(),
        };
        let ty = if let Some(value) = self.const_evaluation.values.get(&key) {
            self.const_type_id(value.ty)
        } else {
            let ty_ref = constant.ty.clone().unwrap_or_else(TypeRef::unknown);
            self.resolve_type_ref_with_class(&ty_ref, constant.span, Some(class_name))
        };
        info.constants.insert(
            constant.name.clone(),
            ConstantInfo {
                access: constant.access.clone(),
                ty,
            },
        );
    }

    fn declare_promoted_property(&mut self, info: &mut ClassInfo, class_name: &str, param: &Param) {
        if !self.declare_member_name(
            info,
            class_name,
            &param.name,
            MemberKind::PromotedProperty,
            param.span,
        ) {
            return;
        }

        let ty = self.resolve_type_ref_with_class(&param.ty, param.span, Some(class_name));
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

    fn declare_member_name(
        &mut self,
        info: &mut ClassInfo,
        class_name: &str,
        name: &str,
        kind: MemberKind,
        span: Span,
    ) -> bool {
        if let Some(original) = info.members.get(name) {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0481",
                    format!(
                        "class `{class_name}` cannot declare {} `{name}` because that name is already used by a previous {}",
                        kind.description(),
                        original.kind.description()
                    ),
                    span,
                )
                .with_related(
                    original.span,
                    format!("original {} `{name}` is declared here", original.kind.description()),
                ),
            );
            return false;
        }

        info.members
            .insert(name.to_string(), MemberDeclaration { kind, span });
        true
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
        if method_context.is_none()
            && function.name == "main"
            && !matches!(
                self.types.kind(signature.return_ty),
                TypeKind::Integer(IntegerType::Int64) | TypeKind::Void | TypeKind::Unknown
            )
        {
            let actual = self.types.display(signature.return_ty);
            self.diagnostics.push(
                Diagnostic::new(
                    "E0442",
                    format!(
                        "process entrypoint `main` cannot return `{actual}`; expected `void`, `int`, or `int64`"
                    ),
                    function.span,
                )
                .with_help("helper functions may return fixed-width integers, floats, or bool"),
            );
        }
        let return_context = self.return_context_for_function(function, method_context.as_ref());
        for (parameter_index, (param, param_info)) in function
            .params
            .iter()
            .zip(signature.params.iter())
            .enumerate()
        {
            let ty = param_info.ty;
            if let Some(default) = &param.default {
                let default_context = method_context.as_ref().map(|context| MethodContext {
                    class_name: context.class_name.clone(),
                    receiver_mode: context.receiver_mode,
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
                self.check_parameter_default_support(
                    function,
                    parameter_index,
                    param,
                    ty,
                    method_context.as_ref(),
                );
            }
            self.declare_binding(
                &mut scopes,
                param.name.clone(),
                Binding {
                    writable: param.writable,
                    ty,
                    declared_ty: ty,
                    int_constant: None,
                    string_constant: None,
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

    fn check_parameter_default_support(
        &mut self,
        function: &FunctionDecl,
        parameter_index: usize,
        param: &Param,
        ty: TypeId,
        method_context: Option<&MethodContext>,
    ) {
        let default = param
            .default
            .as_ref()
            .expect("parameter-default validation requires a default");
        let kind = self.types.kind(ty).clone();

        if matches!(kind, TypeKind::String) && param.take {
            self.diagnostics.push(Diagnostic::new(
                "E0498",
                "default values for `take string` parameters are not yet supported",
                default.span(),
            ));
            return;
        }

        if matches!(kind, TypeKind::String) && param.writable {
            self.diagnostics.push(Diagnostic::new(
                "E0498",
                "default values for `writable string` parameters are not yet supported",
                default.span(),
            ));
            return;
        }

        if matches!(kind, TypeKind::NullableString) {
            self.diagnostics.push(Diagnostic::new(
                "E0498",
                "default values for nullable string parameters are not yet supported",
                default.span(),
            ));
            return;
        }

        if param.take || self.type_is_move_type(ty) {
            self.diagnostics.push(Diagnostic::new(
                "E0498",
                "default values for move-type or `take` parameters are not yet supported",
                default.span(),
            ));
            return;
        }

        if !matches!(
            kind,
            TypeKind::Integer(_) | TypeKind::Float(_) | TypeKind::Bool | TypeKind::String
        ) {
            self.diagnostics.push(Diagnostic::new(
                "E0498",
                "default values for this parameter type are not yet supported",
                default.span(),
            ));
            return;
        }

        let declaring_class = method_context.map(|context| context.class_name.as_str());
        let Some(value) = crate::const_eval::evaluate_parameter_default(
            &self.const_evaluation,
            default,
            &param.ty,
            declaring_class,
        ) else {
            self.diagnostics.push(Diagnostic::new(
                "E0498",
                "a default value must be a constant expression",
                default.span(),
            ));
            return;
        };

        self.parameter_defaults.insert(
            crate::const_eval::ParameterDefaultKey {
                function_start: function.span.start,
                parameter_index,
            },
            value,
        );
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
            .unwrap_or_else(|| self.resolve_function_signature(function, None))
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
                    None => {
                        if let TypeKind::Integer(integer) = *self.types.kind(value_ty) {
                            self.contextualize_integer_literals(&decl.initializer, integer);
                        }
                        value_ty
                    }
                };
                self.declare_binding(
                    scopes,
                    decl.name.clone(),
                    Binding {
                        writable: decl.writable,
                        ty,
                        declared_ty: ty,
                        int_constant: self.readonly_int_constant(
                            decl.writable,
                            ty,
                            &decl.initializer,
                            scopes,
                        ),
                        string_constant: self.readonly_string_constant(
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
                if let Some(target) = self.check_writable_place(
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
                let ty = self.infer_expr_type(expr, scopes, method_context);
                if matches!(
                    self.display_conversion_kind(ty),
                    DisplayConversionKind::NonDisplayableClass
                ) {
                    self.report_non_displayable_class(ty, expr.span());
                } else if !self.is_display_convertible_type(ty) {
                    self.diagnostics.push(Diagnostic::new(
                        "E0445",
                        format!(
                            "value of type `{}` cannot be displayed by echo",
                            self.types.display(ty)
                        ),
                        expr.span(),
                    ));
                }
            }
            Stmt::Expr { expr, .. } => match expr {
                Expr::FunctionCall { name, args, span } if name == "panic" => {
                    for arg in args {
                        self.check_expr(arg, scopes, method_context);
                    }
                    self.check_panic_call(args, *span, scopes, method_context);
                }
                _ => self.check_expr(expr, scopes, method_context),
            },
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
                let mut then_scopes = scopes.clone();
                self.apply_non_null_narrowing(&if_stmt.condition, &mut then_scopes);
                let mut then_constructor_init_context = constructor_init_context
                    .as_deref()
                    .map(ConstructorInitContext::without_readonly_init);
                self.check_block(
                    &if_stmt.then_block,
                    &mut then_scopes,
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
                let mut body_scopes = scopes.clone();
                self.apply_non_null_narrowing(&while_stmt.condition, &mut body_scopes);
                let mut loop_constructor_init_context = constructor_init_context
                    .as_deref()
                    .map(ConstructorInitContext::without_readonly_init);
                self.check_block(
                    &while_stmt.body,
                    &mut body_scopes,
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
                    self.check_for_increment(
                        increment,
                        scopes,
                        method_context,
                        loop_constructor_init_context.as_mut(),
                    );
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
                if range_iterable {
                    if let Some(integer) = foreach
                        .value
                        .ty
                        .as_ref()
                        .and_then(|ty| ty.args.is_empty().then_some(ty))
                        .and_then(|ty| IntegerType::from_source_name(&ty.name))
                    {
                        self.contextualize_range_literals(&foreach.iterable, integer);
                    }
                }
                let unknown_ty = self.types.unknown();
                let range_integer = self
                    .range_integer_type(&foreach.iterable, scopes, method_context)
                    .unwrap_or(IntegerType::Int64);
                let int_ty = self.types.intern(TypeKind::Integer(range_integer));
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
                            declared_ty: ty,
                            int_constant: None,
                            string_constant: None,
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
                        declared_ty: value_ty,
                        int_constant: None,
                        string_constant: None,
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
                self.check_increment_statement(
                    increment,
                    scopes,
                    method_context,
                    constructor_init_context,
                );
            }
        }
    }

    fn apply_non_null_narrowing(&mut self, condition: &Expr, scopes: &mut ScopeStack) {
        let Expr::Binary {
            left, op, right, ..
        } = condition
        else {
            if let Expr::Grouped { expr, .. } = condition {
                self.apply_non_null_narrowing(expr, scopes);
            }
            return;
        };
        if *op != BinaryOp::NotEqual {
            return;
        }
        let name = match (&**left, &**right) {
            (Expr::Variable { name, .. }, Expr::Null { .. })
            | (Expr::Null { .. }, Expr::Variable { name, .. }) => name,
            _ => return,
        };
        let Some(binding) = scopes.lookup_mut(name) else {
            return;
        };
        if matches!(self.types.kind(binding.ty), TypeKind::NullableString) {
            binding.ty = self.types.intern(TypeKind::String);
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
                    None => {
                        if let TypeKind::Integer(integer) = *self.types.kind(value_ty) {
                            self.contextualize_integer_literals(&decl.initializer, integer);
                        }
                        value_ty
                    }
                };
                self.declare_binding(
                    scopes,
                    decl.name.clone(),
                    Binding {
                        writable: decl.writable,
                        ty,
                        declared_ty: ty,
                        int_constant: self.readonly_int_constant(
                            decl.writable,
                            ty,
                            &decl.initializer,
                            scopes,
                        ),
                        string_constant: self.readonly_string_constant(
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
                if let Some(target) = self.check_writable_place(
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
        constructor_init_context: Option<&mut ConstructorInitContext>,
    ) {
        match increment {
            ForIncrement::Increment(increment) => {
                self.check_increment_statement(
                    increment,
                    scopes,
                    method_context,
                    constructor_init_context,
                );
            }
            ForIncrement::Assignment(assignment) => {
                self.check_expr(&assignment.value, scopes, method_context);
                if let Some(target) = self.check_writable_place(
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
                    self.update_nullable_assignment_flow_type(&assignment.target, value_ty, scopes);
                }
            }
            AssignOp::AddAssign
            | AssignOp::SubAssign
            | AssignOp::MulAssign
            | AssignOp::DivAssign
            | AssignOp::ModAssign
            | AssignOp::ShiftLeftAssign
            | AssignOp::ShiftRightAssign
            | AssignOp::BitwiseAndAssign
            | AssignOp::BitwiseOrAssign
            | AssignOp::BitwiseXorAssign => {
                let mut value_ty = self.infer_expr_type(&assignment.value, scopes, method_context);
                if let TypeKind::Integer(integer) = *self.types.kind(target.ty) {
                    self.contextualize_integer_literals(&assignment.value, integer);
                    value_ty = self.infer_expr_type(&assignment.value, scopes, method_context);
                } else if let TypeKind::Float(float) = *self.types.kind(target.ty) {
                    self.contextualize_float_literals(&assignment.value, float);
                    value_ty = self.infer_expr_type(&assignment.value, scopes, method_context);
                }
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

                let integers_only = matches!(
                    assignment.op,
                    AssignOp::ModAssign
                        | AssignOp::ShiftLeftAssign
                        | AssignOp::ShiftRightAssign
                        | AssignOp::BitwiseAndAssign
                        | AssignOp::BitwiseOrAssign
                        | AssignOp::BitwiseXorAssign
                );
                let result_ty = self.infer_numeric_binary_type(target.ty, value_ty);
                if integers_only
                    && !matches!(
                        self.types.kind(result_ty),
                        TypeKind::Integer(_) | TypeKind::Unknown
                    )
                {
                    self.report_integer_operand_mismatch(
                        target.ty,
                        value_ty,
                        assignment.span,
                        "compound assignment",
                    );
                    return;
                }
                if !self.is_assignable(target.ty, result_ty) {
                    if matches!(self.types.kind(target.ty), TypeKind::Integer(_))
                        && matches!(self.types.kind(value_ty), TypeKind::Integer(_))
                    {
                        self.report_integer_operand_mismatch(
                            target.ty,
                            value_ty,
                            assignment.span,
                            "compound assignment",
                        );
                    } else {
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

    fn check_increment_statement(
        &mut self,
        increment: &IncrementStmt,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
        constructor_init_context: Option<&mut ConstructorInitContext>,
    ) {
        self.check_increment_target(
            &increment.target,
            &increment.op,
            scopes,
            method_context,
            constructor_init_context,
        );
    }

    fn check_increment_target(
        &mut self,
        target: &Expr,
        op: &IncrementOp,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
        constructor_init_context: Option<&mut ConstructorInitContext>,
    ) {
        let (op_name, assignment_op) = match op {
            IncrementOp::Increment => ("++", AssignOp::AddAssign),
            IncrementOp::Decrement => ("--", AssignOp::SubAssign),
        };
        let target_span = target.span();
        let Some(place) = self.check_writable_place(
            target,
            &assignment_op,
            scopes,
            method_context,
            constructor_init_context,
        ) else {
            return;
        };

        if !matches!(
            self.types.kind(place.ty),
            TypeKind::Integer(_) | TypeKind::Float(_) | TypeKind::Unknown
        ) {
            self.diagnostics.push(Diagnostic::new(
                "E0423",
                format!("{op_name} requires a writable integer or float target"),
                target_span,
            ));
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

        if crate::return_analysis::analyze(function).fallthrough_reachable {
            self.report_missing_return_value(context, expected, function.span);
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
                qualifier,
                qualifier_span,
                method,
                member_sigil_span,
                args,
                span,
            } => {
                for arg in args {
                    self.check_expr(arg, scopes, method_context);
                }
                self.check_static_call(
                    StaticAccess {
                        qualifier,
                        qualifier_span: *qualifier_span,
                        member_sigil_span: *member_sigil_span,
                        member: method,
                        span: *span,
                    },
                    args,
                    scopes,
                    method_context,
                );
            }
            Expr::StaticMember {
                qualifier,
                qualifier_span,
                member,
                member_sigil_span,
                span,
            } => {
                self.check_static_member(
                    StaticAccess {
                        qualifier,
                        qualifier_span: *qualifier_span,
                        member_sigil_span: *member_sigil_span,
                        member,
                        span: *span,
                    },
                    method_context,
                );
            }
            Expr::New {
                class_name,
                args,
                span,
            } => {
                let qualified = class_name.contains('\\');
                let class_exists = !qualified && self.classes.contains_key(class_name);
                if qualified {
                    self.report_deferred_qualified_name(class_name, *span);
                } else if !class_exists {
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
            Expr::Unary { op, expr, span } => {
                if *op == UnaryOp::Negate {
                    if let (Some(magnitude), Some(operand_span)) = (
                        Self::unsigned_integer_literal_magnitude(expr),
                        Self::unsigned_integer_literal_span(expr),
                    ) {
                        self.negative_integer_literals
                            .insert((span.start, span.end), magnitude);
                        self.negated_integer_literal_operands
                            .insert((operand_span.start, operand_span.end));
                    }
                }
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
                self.check_mixed_binary_operands(left, right, *span, scopes, method_context);
                self.check_binary_operands(left, op, right, *span, scopes, method_context);
            }
            Expr::Range {
                start, end, span, ..
            } => {
                self.check_expr(start, scopes, method_context);
                self.check_expr(end, scopes, method_context);
                self.check_range_operands(start, end, *span, scopes, method_context);
                if !allow_range_expr {
                    self.diagnostics.push(Diagnostic::new(
                        "E0426",
                        "range expressions are only supported as foreach iterables",
                        *span,
                    ));
                }
            }
            Expr::Float { .. } => self.check_float_literal_range(expr, FloatType::Float64),
            Expr::Identifier { name, span } => {
                if name.contains('\\') {
                    self.report_deferred_qualified_name(name, *span);
                } else if !self
                    .const_evaluation
                    .values
                    .contains_key(&crate::const_eval::ConstKey::TopLevel(name.clone()))
                {
                    self.diagnostics.push(Diagnostic::new(
                        "E0491",
                        format!("unknown constant `{name}`"),
                        *span,
                    ));
                }
            }
            Expr::String { .. } | Expr::Bool { .. } | Expr::Null { .. } => {}
            Expr::Int { value, span } => {
                if let Some(magnitude) = parse_decimal_magnitude(value) {
                    self.integer_literals
                        .insert((span.start, span.end), magnitude);
                } else {
                    self.report_integer_literal_range(*span, IntegerType::Int64);
                }
            }
        }
    }

    fn is_grouped_range_expr(expr: &Expr) -> bool {
        match expr {
            Expr::Grouped { expr, .. } => Self::is_grouped_range_expr(expr),
            Expr::Range { .. } => true,
            _ => false,
        }
    }

    fn range_integer_type(
        &mut self,
        expr: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> Option<IntegerType> {
        match expr {
            Expr::Grouped { expr, .. } => self.range_integer_type(expr, scopes, method_context),
            Expr::Range { start, end, .. } => {
                let ty = self.infer_binary_type(start, &BinaryOp::Add, end, scopes, method_context);
                match self.types.kind(ty) {
                    TypeKind::Integer(integer) => Some(*integer),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn check_range_operands(
        &mut self,
        start: &Expr,
        end: &Expr,
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let (start_ty, end_ty) =
            self.infer_contextual_binary_operand_types(start, end, scopes, method_context);
        match (self.types.kind(start_ty), self.types.kind(end_ty)) {
            (TypeKind::Integer(start), TypeKind::Integer(end)) if start == end => {}
            (TypeKind::Integer(_), TypeKind::Integer(_)) => {
                self.report_integer_operand_mismatch(start_ty, end_ty, span, "range")
            }
            (TypeKind::Unknown, _) | (_, TypeKind::Unknown) => {}
            _ => self.diagnostics.push(Diagnostic::new(
                "E0424",
                format!(
                    "range endpoints must be integers of the same type, got `{}` and `{}`",
                    self.types.display(start_ty),
                    self.types.display(end_ty)
                ),
                span,
            )),
        }
    }

    fn contextualize_range_literals(&mut self, expr: &Expr, target: IntegerType) {
        match expr {
            Expr::Grouped { expr, .. } => self.contextualize_range_literals(expr, target),
            Expr::Range { start, end, .. } => {
                self.contextualize_integer_literals(start, target);
                self.contextualize_integer_literals(end, target);
            }
            _ => {}
        }
    }

    fn report_integer_literal_range(&mut self, span: Span, target: IntegerType) {
        if self
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0417" && diagnostic.span == span)
        {
            return;
        }

        let mut diagnostic = Diagnostic::new(
            "E0417",
            format!(
                "integer literal is outside the Doria `{}` range",
                target.source_name()
            ),
            span,
        );
        if target == IntegerType::Int64 {
            diagnostic = diagnostic.with_help(
                "unconstrained integer literals default to `int`; add a `uint64` context when that is intended",
            );
        }
        self.diagnostics.push(diagnostic);
    }

    fn check_pending_integer_literal_ranges(&mut self) {
        let mut literals = Vec::new();
        for ((start, end), magnitude) in &self.integer_literals {
            if self
                .negated_integer_literal_operands
                .contains(&(*start, *end))
            {
                continue;
            }
            literals.push((*start, *end, *magnitude, false));
        }
        literals.extend(
            self.negative_integer_literals
                .iter()
                .map(|((start, end), magnitude)| (*start, *end, *magnitude, true)),
        );
        literals.sort_unstable_by_key(|(start, end, _, _)| (*start, *end));

        for (start, end, magnitude, negative) in literals {
            let span = Span { start, end };
            let target = self
                .integer_expression_types
                .get(&(start, end))
                .copied()
                .unwrap_or(IntegerType::Int64);
            if IntegerValue::from_literal(target, magnitude, negative).is_none() {
                self.report_integer_literal_range(span, target);
            }
        }
    }

    fn integer_literal_parts(expr: &Expr) -> Option<(u128, bool)> {
        match expr {
            Expr::Int { value, .. } => parse_decimal_magnitude(value).map(|value| (value, false)),
            Expr::Grouped { expr, .. } => Self::integer_literal_parts(expr),
            Expr::Unary {
                op: UnaryOp::Negate,
                expr,
                ..
            } => Self::unsigned_integer_literal_magnitude(expr).map(|value| (value, true)),
            _ => None,
        }
    }

    fn unsigned_integer_literal_magnitude(expr: &Expr) -> Option<u128> {
        match expr {
            Expr::Int { value, .. } => parse_decimal_magnitude(value),
            Expr::Grouped { expr, .. } => Self::unsigned_integer_literal_magnitude(expr),
            _ => None,
        }
    }

    fn unsigned_integer_literal_span(expr: &Expr) -> Option<Span> {
        match expr {
            Expr::Int { span, .. } => Some(*span),
            Expr::Grouped { expr, .. } => Self::unsigned_integer_literal_span(expr),
            _ => None,
        }
    }

    fn record_integer_expression_type(&mut self, expr: &Expr, integer: IntegerType) {
        self.integer_expression_types
            .insert((expr.span().start, expr.span().end), integer);
        match expr {
            Expr::Grouped { expr, .. }
            | Expr::Unary {
                op: UnaryOp::Negate,
                expr,
                ..
            } => self.record_integer_expression_type(expr, integer),
            _ => {}
        }
    }

    /// Returns `Some(true/false)` for a literal form and `None` for a
    /// non-literal expression. A contextual literal is typing, not conversion.
    fn check_contextual_integer_literal(
        &mut self,
        expr: &Expr,
        target: IntegerType,
    ) -> Option<bool> {
        let (magnitude, negative) = Self::integer_literal_parts(expr)?;
        if IntegerValue::from_literal(target, magnitude, negative).is_some() {
            self.record_integer_expression_type(expr, target);
            Some(true)
        } else {
            self.report_integer_literal_range(expr.span(), target);
            Some(false)
        }
    }

    fn contextualize_integer_literals(&mut self, expr: &Expr, target: IntegerType) {
        if self
            .check_contextual_integer_literal(expr, target)
            .is_some()
        {
            return;
        }

        match expr {
            Expr::Grouped { expr, .. }
            | Expr::Unary {
                op: UnaryOp::Negate | UnaryOp::BitwiseNot,
                expr,
                ..
            } => self.contextualize_integer_literals(expr, target),
            Expr::Binary {
                left,
                op:
                    BinaryOp::Add
                    | BinaryOp::Sub
                    | BinaryOp::Mul
                    | BinaryOp::Div
                    | BinaryOp::Mod
                    | BinaryOp::ShiftLeft
                    | BinaryOp::ShiftRight
                    | BinaryOp::BitwiseAnd
                    | BinaryOp::BitwiseXor
                    | BinaryOp::BitwiseOr,
                right,
                ..
            } => {
                self.contextualize_integer_literals(left, target);
                self.contextualize_integer_literals(right, target);
            }
            _ => {}
        }
    }

    fn is_float_literal(expr: &Expr) -> bool {
        match expr {
            Expr::Float { .. } => true,
            Expr::Grouped { expr, .. } => Self::is_float_literal(expr),
            _ => false,
        }
    }

    fn record_float_expression_type(&mut self, expr: &Expr, float: FloatType) {
        self.float_expression_types
            .insert((expr.span().start, expr.span().end), float);
        self.check_float_literal_range(expr, float);
        if let Expr::Grouped { expr, .. } = expr {
            self.record_float_expression_type(expr, float);
        }
    }

    fn check_float_literal_range(&mut self, expr: &Expr, target: FloatType) {
        let Expr::Float { value, span } = expr else {
            return;
        };
        let out_of_range = FloatValue::parse_decimal(target, value)
            .map(FloatValue::is_infinite)
            .unwrap_or(true);
        if out_of_range
            && !self
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0444" && diagnostic.span == *span)
        {
            self.diagnostics.push(Diagnostic::new(
                "E0444",
                format!(
                    "floating literal is outside the Doria `{}` finite range",
                    target.source_name()
                ),
                *span,
            ));
        }
    }

    fn contextualize_float_literals(&mut self, expr: &Expr, target: FloatType) {
        if Self::is_float_literal(expr) {
            self.record_float_expression_type(expr, target);
            return;
        }

        match expr {
            Expr::Grouped { expr, .. }
            | Expr::Unary {
                op: UnaryOp::Negate,
                expr,
                ..
            } => self.contextualize_float_literals(expr, target),
            Expr::Binary {
                left,
                op: BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div,
                right,
                ..
            } => {
                self.contextualize_float_literals(left, target);
                self.contextualize_float_literals(right, target);
                self.float_expression_types
                    .insert((expr.span().start, expr.span().end), target);
            }
            _ => {}
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
            UnaryOp::Negate => {
                let ty = self.infer_expr_type(expr, scopes, method_context);
                match self.types.kind(ty) {
                    TypeKind::Integer(integer) if integer.is_signed() => {}
                    TypeKind::Float(_) => {}
                    TypeKind::Unknown => {}
                    TypeKind::Integer(integer) => self.diagnostics.push(
                        Diagnostic::new(
                            "E0440",
                            format!("unary `-` requires a signed integer operand, got `{integer}`"),
                            expr.span(),
                        )
                        .with_help("explicitly convert to a signed integer type first"),
                    ),
                    _ => self.diagnostics.push(Diagnostic::new(
                        "E0440",
                        format!(
                            "unary `-` requires a signed integer or float operand, got `{}`",
                            self.types.display(ty)
                        ),
                        expr.span(),
                    )),
                }
            }
            UnaryOp::BitwiseNot => {
                let ty = self.infer_expr_type(expr, scopes, method_context);
                if !matches!(
                    self.types.kind(ty),
                    TypeKind::Integer(_) | TypeKind::Unknown
                ) {
                    self.diagnostics.push(Diagnostic::new(
                        "E0440",
                        format!(
                            "bitwise operator `~` requires an integer operand, got `{}`",
                            self.types.display(ty)
                        ),
                        expr.span(),
                    ));
                }
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
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
                self.check_numeric_binary_operands(left, right, span, scopes, method_context, false)
            }
            BinaryOp::Mod
            | BinaryOp::ShiftLeft
            | BinaryOp::ShiftRight
            | BinaryOp::BitwiseAnd
            | BinaryOp::BitwiseXor
            | BinaryOp::BitwiseOr => {
                self.check_numeric_binary_operands(left, right, span, scopes, method_context, true)
            }
            BinaryOp::Less | BinaryOp::LessEqual | BinaryOp::Greater | BinaryOp::GreaterEqual => {
                let (left_ty, right_ty) =
                    self.infer_contextual_binary_operand_types(left, right, scopes, method_context);
                if matches!(
                    (self.types.kind(left_ty), self.types.kind(right_ty)),
                    (TypeKind::Integer(left), TypeKind::Integer(right)) if left == right
                ) || matches!(
                    (self.types.kind(left_ty), self.types.kind(right_ty)),
                    (TypeKind::Float(left), TypeKind::Float(right)) if left == right
                ) || matches!(
                    (self.types.kind(left_ty), self.types.kind(right_ty)),
                    (TypeKind::String, TypeKind::String)
                ) || matches!(
                    (self.types.kind(left_ty), self.types.kind(right_ty)),
                    (TypeKind::Unknown, _) | (_, TypeKind::Unknown)
                ) {
                    return;
                }
                self.report_integer_operand_mismatch(left_ty, right_ty, span, "comparison");
            }
            BinaryOp::Coalesce => {}
        }
    }

    fn check_numeric_binary_operands(
        &mut self,
        left: &Expr,
        right: &Expr,
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
        integers_only: bool,
    ) {
        let (left_ty, right_ty) =
            self.infer_contextual_binary_operand_types(left, right, scopes, method_context);
        let compatible_integer = matches!(
            (self.types.kind(left_ty), self.types.kind(right_ty)),
            (TypeKind::Integer(left), TypeKind::Integer(right)) if left == right
        );
        let compatible_float = !integers_only
            && matches!(
                (self.types.kind(left_ty), self.types.kind(right_ty)),
                (TypeKind::Float(left), TypeKind::Float(right)) if left == right
            );
        let recovering = matches!(
            (self.types.kind(left_ty), self.types.kind(right_ty)),
            (TypeKind::Unknown, _) | (_, TypeKind::Unknown)
        );
        if compatible_integer || compatible_float || recovering {
            return;
        }

        self.report_integer_operand_mismatch(left_ty, right_ty, span, "integer operator");
    }

    fn report_integer_operand_mismatch(
        &mut self,
        left: TypeId,
        right: TypeId,
        span: Span,
        operation: &str,
    ) {
        self.diagnostics.push(
            Diagnostic::new(
                "E0441",
                format!(
                    "{operation} operands must have the same integer type, got `{}` and `{}`",
                    self.types.display(left),
                    self.types.display(right)
                ),
                span,
            )
            .with_help("explicitly convert one operand with a companion `::from(...)` call"),
        );
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
        let (left_ty, right_ty) =
            self.infer_contextual_binary_operand_types(left, right, scopes, method_context);
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
        let (left_ty, right_ty) =
            self.infer_contextual_binary_operand_types(left, right, scopes, method_context);
        if self.is_equality_compatible(left_ty, right_ty) {
            return;
        }

        let mut diagnostic = Diagnostic::new(
            "E0420",
            format!(
                "equality operands must have compatible types, got `{}` and `{}`",
                self.types.display(left_ty),
                self.types.display(right_ty)
            ),
            span,
        );
        if matches!(self.types.kind(left_ty), TypeKind::Integer(_))
            && matches!(self.types.kind(right_ty), TypeKind::Integer(_))
        {
            diagnostic = diagnostic.with_help(
                "integer comparisons do not widen implicitly; explicitly convert one operand",
            );
        }
        self.diagnostics.push(diagnostic);
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
        let mut rejected_class = false;
        for (ty, expr) in [(left_ty, left), (right_ty, right)] {
            if matches!(
                self.display_conversion_kind(ty),
                DisplayConversionKind::NonDisplayableClass
            ) {
                self.report_non_displayable_class(ty, expr.span());
                rejected_class = true;
            }
        }
        if rejected_class {
            return;
        }
        let has_string = matches!(
            self.types.kind(left_ty),
            TypeKind::String | TypeKind::Unknown
        ) || matches!(
            self.types.kind(right_ty),
            TypeKind::String | TypeKind::Unknown
        );
        if has_string
            && self.is_display_convertible_type(left_ty)
            && self.is_display_convertible_type(right_ty)
        {
            return;
        }

        self.diagnostics.push(
            Diagnostic::new(
                "E0425",
                "concatenation requires at least one string operand",
                span,
            )
            .with_help("use + for numeric addition or add a string/interpolation context"),
        );
    }

    fn is_bool_or_recovery_type(&self, ty: TypeId) -> bool {
        matches!(self.types.kind(ty), TypeKind::Bool | TypeKind::Unknown)
    }

    fn is_display_convertible_type(&self, ty: TypeId) -> bool {
        matches!(
            self.display_conversion_kind(ty),
            DisplayConversionKind::Primitive
                | DisplayConversionKind::DisplayableClass
                | DisplayConversionKind::Recovery
        )
    }

    fn display_conversion_kind(&self, ty: TypeId) -> DisplayConversionKind {
        match self.types.kind(ty) {
            TypeKind::String | TypeKind::Integer(_) | TypeKind::Float(_) | TypeKind::Bool => {
                DisplayConversionKind::Primitive
            }
            TypeKind::Class(name) => {
                if self
                    .classes
                    .get(name)
                    .is_some_and(|class| class.implements_displayable)
                {
                    DisplayConversionKind::DisplayableClass
                } else {
                    DisplayConversionKind::NonDisplayableClass
                }
            }
            TypeKind::Unknown => DisplayConversionKind::Recovery,
            _ => DisplayConversionKind::Excluded,
        }
    }

    fn report_non_displayable_class(&mut self, ty: TypeId, span: Span) {
        let class_name = self.types.display(ty);
        self.diagnostics.push(
            Diagnostic::new(
                "E0462",
                format!(
                    "`{class_name}` cannot be displayed; implement `Displayable` with `function toString(): string`"
                ),
                span,
            )
            .with_help(
                "add `implements Displayable` and an externally accessible readonly `function toString(): string` method",
            ),
        );
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

    fn type_is_move_type(&self, ty: TypeId) -> bool {
        matches!(
            self.types.kind(ty),
            TypeKind::Class(_)
                | TypeKind::Mixed
                | TypeKind::TypedArray(_)
                | TypeKind::List(_)
                | TypeKind::Dictionary(_, _)
                | TypeKind::Set(_)
                | TypeKind::EmptyCollection
                | TypeKind::Heterogeneous
        )
    }

    fn report_mixed_operation(&mut self, span: Span, operation: &'static str) {
        self.diagnostics.push(
            Diagnostic::new(
                "E0433",
                format!("cannot use `mixed` value in {operation} before narrowing"),
                span,
            )
            .with_help(
                "mixed-value operations are unsupported until narrowing syntax is implemented",
            ),
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

    fn readonly_int_constant(
        &self,
        writable: bool,
        ty: TypeId,
        initializer: &Expr,
        scopes: &ScopeStack,
    ) -> Option<IntegerValue> {
        let TypeKind::Integer(integer) = *self.types.kind(ty) else {
            return None;
        };
        if writable {
            return None;
        }

        match Self::eval_int_constant(initializer, scopes, integer) {
            IntConstantEval::Known(value) => Some(value),
            IntConstantEval::Unknown | IntConstantEval::Invalid => None,
        }
    }

    fn eval_int_constant(expr: &Expr, scopes: &ScopeStack, target: IntegerType) -> IntConstantEval {
        match expr {
            Expr::Int { value, .. } => parse_decimal_magnitude(value)
                .and_then(|magnitude| IntegerValue::from_literal(target, magnitude, false))
                .map(IntConstantEval::Known)
                .unwrap_or(IntConstantEval::Invalid),
            Expr::Variable { name, .. } => scopes
                .lookup(name)
                .and_then(|binding| binding.int_constant)
                .filter(|value| value.ty == target)
                .map(IntConstantEval::Known)
                .unwrap_or(IntConstantEval::Unknown),
            Expr::Grouped { expr, .. } => Self::eval_int_constant(expr, scopes, target),
            Expr::Unary {
                op: UnaryOp::Negate,
                expr,
                ..
            } => match Self::unsigned_integer_literal_magnitude(expr)
                .and_then(|magnitude| IntegerValue::from_literal(target, magnitude, true))
            {
                Some(value) => IntConstantEval::Known(value),
                None => IntConstantEval::Invalid,
            },
            Expr::Binary {
                left, op, right, ..
            } if Self::is_checked_int_arithmetic_op(op) => {
                let left = Self::eval_int_constant(left, scopes, target);
                let right = Self::eval_int_constant(right, scopes, target);
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

    fn readonly_string_constant(
        &self,
        writable: bool,
        ty: TypeId,
        initializer: &Expr,
        scopes: &ScopeStack,
    ) -> Option<String> {
        if writable || !matches!(self.types.kind(ty), TypeKind::String) {
            return None;
        }

        Self::eval_string_constant(initializer, scopes)
    }

    fn eval_string_constant(expr: &Expr, scopes: &ScopeStack) -> Option<String> {
        match expr {
            Expr::String { value, .. } => Some(value.clone()),
            Expr::Variable { name, .. } => scopes
                .lookup(name)
                .and_then(|binding| binding.string_constant.clone()),
            Expr::Grouped { expr, .. } => Self::eval_string_constant(expr, scopes),
            Expr::Binary {
                left,
                op: BinaryOp::Concat,
                right,
                ..
            } => {
                let mut value = Self::eval_string_constant(left, scopes)?;
                value.push_str(&Self::eval_string_constant(right, scopes)?);
                Some(value)
            }
            _ => None,
        }
    }

    fn is_checked_int_arithmetic_op(op: &BinaryOp) -> bool {
        matches!(op, BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul)
    }

    fn checked_int_arithmetic(
        left: IntegerValue,
        op: &BinaryOp,
        right: IntegerValue,
    ) -> Option<IntegerValue> {
        match op {
            BinaryOp::Add => left.checked_add(right).ok(),
            BinaryOp::Sub => left.checked_sub(right).ok(),
            BinaryOp::Mul => left.checked_mul(right).ok(),
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
            if matches!(
                self.display_conversion_kind(ty),
                DisplayConversionKind::NonDisplayableClass
            ) {
                self.report_non_displayable_class(ty, expr.span());
            } else if !self.is_display_convertible_type(ty) {
                let ty_name = self.types.display(ty);
                self.diagnostics.push(Diagnostic::new(
                    "E0415",
                    format!("value of type {ty_name} cannot be interpolated into a string"),
                    expr.span(),
                ));
            }
        }
    }

    fn check_writable_place(
        &mut self,
        target: &Expr,
        op: &AssignOp,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
        constructor_init_context: Option<&mut ConstructorInitContext>,
    ) -> Option<AssignmentTarget> {
        match target {
            Expr::Grouped { expr, .. } => self.check_writable_place(
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
                        ty: if matches!(
                            self.types.kind(binding.declared_ty),
                            TypeKind::NullableString
                        ) {
                            binding.declared_ty
                        } else {
                            binding.ty
                        },
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
                                        "cannot assign to readonly property `{class_name}::{property}`"
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
            Expr::StaticMember {
                qualifier,
                qualifier_span,
                member,
                member_sigil_span,
                span,
            } => self.check_static_assignment_target(
                StaticAccess {
                    qualifier,
                    qualifier_span: *qualifier_span,
                    member_sigil_span: *member_sigil_span,
                    member,
                    span: *span,
                },
                method_context,
            ),
            _ => {
                self.diagnostics.push(Diagnostic::new(
                    "E0204",
                    "unsupported mutation target",
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
            format!("readonly property `{class_name}::{property}` {reason}"),
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
        if name.contains('\\') {
            self.report_deferred_qualified_name(name, span);
            return;
        }
        if name == "print" {
            self.diagnostics.push(
                Diagnostic::new("E0462", "Doria does not support `print`; use `echo`", span)
                    .with_help("echo writes output and does not return a value"),
            );
            for arg in args {
                self.check_expr(arg, scopes, method_context);
            }
            return;
        }

        if name == "panic" {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0436",
                    "`panic` may only be called as a standalone statement",
                    span,
                )
                .with_help("use `panic(\"message\");` as its own statement"),
            );
            self.check_panic_call(args, span, scopes, method_context);
            return;
        }

        if let Some(builtin) = Builtin::from_name(name) {
            self.check_builtin_call(builtin, args, span, scopes, method_context);
            return;
        }

        let Some(function_info) = self.functions.get(name).cloned() else {
            if let Some(suggestion) = php_function_suggestion(name) {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0461",
                        format!("unknown function `{name}`; did you mean `{suggestion}`?"),
                        span,
                    )
                    .with_help(format!("replace `{name}()` with `{suggestion}()`")),
                );
                for arg in args {
                    self.check_expr(arg, scopes, method_context);
                }
            } else {
                self.diagnostics.push(Diagnostic::new(
                    "E0309",
                    format!("unknown function `{name}`"),
                    span,
                ));
            }
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

    fn check_builtin_call(
        &mut self,
        builtin: Builtin,
        args: &[Expr],
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let expected = match builtin {
            Builtin::ReadLine => Some(0),
            Builtin::ReadFile | Builtin::WriteStderr => Some(1),
            Builtin::WriteFile => Some(2),
            Builtin::Sprintf | Builtin::Printf => None,
            Builtin::Panic => return self.check_panic_call(args, span, scopes, method_context),
        };
        if let Some(expected) = expected {
            if args.len() != expected {
                self.diagnostics.push(Diagnostic::new(
                    "E0450",
                    format!(
                        "{} expects exactly {expected} arguments, got {}",
                        builtin.name(),
                        args.len()
                    ),
                    span,
                ));
                return;
            }
        } else if args.is_empty() {
            self.diagnostics.push(Diagnostic::new(
                "E0451",
                format!("{} expects a literal format argument", builtin.name()),
                span,
            ));
            return;
        }

        match builtin {
            Builtin::ReadFile | Builtin::WriteStderr => {
                self.require_builtin_string_arg(builtin, &args[0], scopes, method_context)
            }
            Builtin::WriteFile => {
                self.require_builtin_string_arg(builtin, &args[0], scopes, method_context);
                self.require_builtin_string_arg(builtin, &args[1], scopes, method_context);
            }
            Builtin::Sprintf | Builtin::Printf => {
                let Some(Expr::String { value, span }) = args.first() else {
                    self.diagnostics.push(Diagnostic::new(
                        "E0452",
                        format!("{} format must be a direct string literal", builtin.name()),
                        args[0].span(),
                    ));
                    return;
                };
                let pieces = match format_string::parse(value, *span) {
                    Ok(pieces) => pieces,
                    Err(diagnostic) => {
                        self.diagnostics.push(diagnostic);
                        return;
                    }
                };
                let specs = pieces
                    .iter()
                    .filter_map(|piece| match piece {
                        FormatPiece::Argument { spec, .. } => Some(*spec),
                        FormatPiece::Literal(_) => None,
                    })
                    .collect::<Vec<_>>();
                if args.len() - 1 != specs.len() {
                    self.diagnostics.push(Diagnostic::new(
                        "E0456",
                        format!(
                            "{} format expects {} arguments, got {}",
                            builtin.name(),
                            specs.len(),
                            args.len() - 1
                        ),
                        *span,
                    ));
                    return;
                }
                for (argument, spec) in args[1..].iter().zip(specs) {
                    let ty = self.infer_expr_type(argument, scopes, method_context);
                    if spec.conversion == FormatConversion::Display
                        && matches!(
                            self.display_conversion_kind(ty),
                            DisplayConversionKind::NonDisplayableClass
                        )
                    {
                        self.report_non_displayable_class(ty, argument.span());
                        continue;
                    }
                    let valid = match spec.conversion {
                        FormatConversion::Display => self.is_display_convertible_type(ty),
                        FormatConversion::Decimal
                        | FormatConversion::HexLower
                        | FormatConversion::HexUpper
                        | FormatConversion::Octal
                        | FormatConversion::Binary => {
                            matches!(
                                self.types.kind(ty),
                                TypeKind::Integer(_) | TypeKind::Unknown
                            )
                        }
                        FormatConversion::Float => {
                            matches!(self.types.kind(ty), TypeKind::Float(_) | TypeKind::Unknown)
                        }
                    };
                    if !valid {
                        self.diagnostics.push(Diagnostic::new(
                            "E0457",
                            format!(
                                "format conversion `{}` does not accept `{}`",
                                spec.conversion.specifier(),
                                self.types.display(ty)
                            ),
                            argument.span(),
                        ));
                    }
                }
            }
            Builtin::ReadLine | Builtin::Panic => {}
        }
    }

    fn require_builtin_string_arg(
        &mut self,
        builtin: Builtin,
        argument: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let ty = self.infer_expr_type(argument, scopes, method_context);
        if !matches!(self.types.kind(ty), TypeKind::String | TypeKind::Unknown) {
            self.diagnostics.push(Diagnostic::new(
                "E0453",
                format!(
                    "{} expects `string`, got `{}`",
                    builtin.name(),
                    self.types.display(ty)
                ),
                argument.span(),
            ));
        }
    }

    fn check_panic_call(
        &mut self,
        args: &[Expr],
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        if args.len() != 1 {
            self.diagnostics.push(Diagnostic::new(
                "E0434",
                format!("panic expects exactly 1 argument, got {}", args.len()),
                span,
            ));
            return;
        }

        let message = &args[0];
        let message_ty = self.infer_expr_type(message, scopes, method_context);
        if !matches!(
            self.types.kind(message_ty),
            TypeKind::String | TypeKind::Unknown
        ) {
            self.diagnostics.push(Diagnostic::new(
                "E0435",
                format!(
                    "panic message must be `string`, got `{}`",
                    self.types.display(message_ty)
                ),
                message.span(),
            ));
        }
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

        if method_info.is_static {
            self.diagnostics.push(Diagnostic::new(
                "E0487",
                format!("static method `{class_name}::{method}` must be called with `::`"),
                span,
            ));
            return;
        }

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

        if method_info
            .receiver_mode
            .is_some_and(ReceiverMode::is_writable)
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
        access: StaticAccess<'_>,
        args: &[Expr],
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let Some(class_name) = self.resolve_static_qualifier(access, method_context) else {
            return;
        };
        let class_name = class_name.as_str();
        if class_name.contains('\\') {
            self.report_deferred_qualified_name(class_name, access.span);
            return;
        }
        if class_name == "Int" && access.member == "toFloat" {
            self.check_cross_kind_intrinsic_argument(
                "Int::toFloat",
                args,
                TypeKind::Integer(IntegerType::Int64),
                access.span,
                scopes,
                method_context,
            );
            return;
        }
        if class_name == "Float" && access.member == "toInt" {
            self.check_cross_kind_intrinsic_argument(
                "Float::toInt",
                args,
                TypeKind::Float(FloatType::Float64),
                access.span,
                scopes,
                method_context,
            );
            return;
        }

        if let Some(target) = IntegerType::from_companion_name(class_name) {
            if access.member != "from" {
                self.diagnostics.push(Diagnostic::new(
                    "E0304",
                    format!(
                        "unknown integer companion intrinsic `{class_name}::{}`; only `{class_name}::from(...)` is available",
                        access.member
                    ),
                    access.span,
                ));
                return;
            }
            if args.len() != 1 {
                self.diagnostics.push(Diagnostic::new(
                    "E0443",
                    format!(
                        "{}::from expects exactly 1 argument, got {}",
                        target.companion_name(),
                        args.len()
                    ),
                    access.span,
                ));
                return;
            }

            let argument = &args[0];
            self.contextualize_integer_literals(argument, IntegerType::Int64);
            let argument_ty = self.infer_expr_type(argument, scopes, method_context);
            if !matches!(
                self.types.kind(argument_ty),
                TypeKind::Integer(_) | TypeKind::Unknown
            ) {
                self.diagnostics.push(Diagnostic::new(
                    "E0443",
                    format!(
                        "{}::from requires an integer argument, got `{}`",
                        target.companion_name(),
                        self.types.display(argument_ty)
                    ),
                    argument.span(),
                ));
            }
            return;
        }

        let Some(class_info) = self.classes.get(class_name).cloned() else {
            self.diagnostics.push(Diagnostic::new(
                "E0305",
                format!("unknown class `{class_name}`"),
                access.span,
            ));
            return;
        };
        let Some(method_info) = class_info.methods.get(access.member) else {
            self.diagnostics.push(Diagnostic::new(
                "E0304",
                format!("unknown method `{class_name}::{}`", access.member),
                access.span,
            ));
            return;
        };

        if self.check_direct_lifecycle_method_call(class_name, access.member, access.span) {
            return;
        }

        if !method_info.is_static {
            self.diagnostics.push(Diagnostic::new(
                "E0487",
                format!(
                    "instance method `{class_name}::{}` requires an object receiver",
                    access.member
                ),
                access.span,
            ));
            return;
        }

        if matches!(method_info.access, MemberAccess::Internal)
            && !self.can_access_internal_member(class_name, method_context)
        {
            self.diagnostics.push(Diagnostic::new(
                "E0307",
                format!("method `{class_name}::{}` is internal", access.member),
                access.span,
            ));
        }

        self.check_call_arguments(
            &format!("method `{class_name}::{}`", access.member),
            &method_info.params,
            args,
            access.span,
            scopes,
            method_context,
        );
    }

    fn check_static_member(
        &mut self,
        access: StaticAccess<'_>,
        method_context: Option<&MethodContext>,
    ) {
        let Some(class_name) = self.resolve_static_qualifier(access, method_context) else {
            return;
        };
        let class_name = class_name.as_str();
        if class_name.contains('\\') {
            self.report_deferred_qualified_name(class_name, access.span);
            return;
        }
        let Some(class_info) = self.classes.get(class_name) else {
            self.diagnostics.push(Diagnostic::new(
                "E0305",
                format!("unknown class `{class_name}`"),
                access.span,
            ));
            return;
        };
        let member_access = class_info
            .constants
            .get(access.member)
            .map(|constant| constant.access.clone())
            .or_else(|| {
                class_info
                    .static_properties
                    .get(access.member)
                    .map(|property| property.access.clone())
            });
        let Some(member_access) = member_access else {
            self.diagnostics.push(Diagnostic::new(
                "E0488",
                format!("unknown static member `{class_name}::{}`", access.member),
                access.span,
            ));
            return;
        };
        if member_access == MemberAccess::Internal
            && !self.can_access_internal_member(class_name, method_context)
        {
            self.diagnostics.push(Diagnostic::new(
                "E0307",
                format!(
                    "static member `{class_name}::{}` is internal",
                    access.member
                ),
                access.span,
            ));
        }
    }

    fn check_static_assignment_target(
        &mut self,
        access: StaticAccess<'_>,
        method_context: Option<&MethodContext>,
    ) -> Option<AssignmentTarget> {
        let class_name = self.resolve_static_qualifier(access, method_context)?;
        self.check_resolved_static_member(&class_name, access.member, access.span, method_context);
        let class_info = self.classes.get(&class_name)?;
        if class_info.constants.contains_key(access.member) {
            self.diagnostics.push(Diagnostic::new(
                "E0489",
                format!(
                    "cannot assign to constant `{class_name}::{}`",
                    access.member
                ),
                access.span,
            ));
            return None;
        }
        let property = class_info.static_properties.get(access.member)?.clone();
        if !property.writable {
            self.diagnostics.push(Diagnostic::new(
                "E0202",
                format!(
                    "cannot assign to readonly static property `{class_name}::{}`",
                    access.member
                ),
                access.span,
            ));
        }
        Some(AssignmentTarget {
            ty: property.ty,
            destination: AssignmentDestination::Type,
        })
    }

    fn resolve_static_qualifier(
        &mut self,
        access: StaticAccess<'_>,
        method_context: Option<&MethodContext>,
    ) -> Option<String> {
        if let Some(sigil_span) = access.member_sigil_span {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0494",
                    "Doria static member access is sigil-free; remove `$`",
                    sigil_span,
                )
                .with_help("declarations carry `$`; member accesses do not")
                .with_fix(sigil_span, ""),
            );
            return None;
        }

        match access.qualifier {
            StaticQualifier::Class(name) => Some(name.clone()),
            StaticQualifier::SelfType => method_context
                .map(|context| context.class_name.clone())
                .or_else(|| {
                    self.diagnostics.push(Diagnostic::new(
                        "E0492",
                        "`self` is only available in a declaring or composing class context",
                        access.qualifier_span,
                    ));
                    None
                }),
            StaticQualifier::Parent => {
                self.diagnostics.push(Diagnostic::unsupported_stage(
                    "E0496",
                    "generalized `parent::member()` syntax is accepted; parent implementation semantics land in Stage 34",
                    access.span,
                ));
                None
            }
            StaticQualifier::InvalidStatic => {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0495",
                        "Doria does not support late static binding; use `self::`",
                        access.qualifier_span,
                    )
                    .with_help(
                        "replace the qualifier with `self` and keep the member access unchanged",
                    )
                    .with_fix(access.qualifier_span, "self"),
                );
                None
            }
        }
    }

    fn check_resolved_static_member(
        &mut self,
        class_name: &str,
        member: &str,
        span: Span,
        method_context: Option<&MethodContext>,
    ) {
        if class_name.contains('\\') {
            self.report_deferred_qualified_name(class_name, span);
            return;
        }
        let Some(class_info) = self.classes.get(class_name) else {
            self.diagnostics.push(Diagnostic::new(
                "E0305",
                format!("unknown class `{class_name}`"),
                span,
            ));
            return;
        };
        let access = class_info
            .constants
            .get(member)
            .map(|constant| constant.access.clone())
            .or_else(|| {
                class_info
                    .static_properties
                    .get(member)
                    .map(|property| property.access.clone())
            });
        let Some(access) = access else {
            self.diagnostics.push(Diagnostic::new(
                "E0488",
                format!("unknown static member `{class_name}::{member}`"),
                span,
            ));
            return;
        };
        if access == MemberAccess::Internal
            && !self.can_access_internal_member(class_name, method_context)
        {
            self.diagnostics.push(Diagnostic::new(
                "E0307",
                format!("static member `{class_name}::{member}` is internal"),
                span,
            ));
        }
    }

    fn check_cross_kind_intrinsic_argument(
        &mut self,
        name: &str,
        args: &[Expr],
        expected_kind: TypeKind,
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        if args.len() != 1 {
            self.diagnostics.push(Diagnostic::new(
                "E0443",
                format!("{name} expects exactly 1 argument, got {}", args.len()),
                span,
            ));
            return;
        }
        let expected = self.types.intern(expected_kind);
        let actual = self.infer_expr_type(&args[0], scopes, method_context);
        if actual != expected && !self.is_unknown_type(actual) {
            self.diagnostics.push(Diagnostic::new(
                "E0443",
                format!(
                    "{name} requires a `{}` argument, got `{}`",
                    self.types.display(expected),
                    self.types.display(actual)
                ),
                args[0].span(),
            ));
        }
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
                if param.writable
                    && matches!(self.types.kind(param.ty), TypeKind::Class(_))
                    && !self.is_writable_object_path(arg, scopes, method_context)
                {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0204",
                            format!(
                                "argument {} of {callee} must be a writable class value",
                                index + 1
                            ),
                            arg.span(),
                        )
                        .with_help(
                            "declare the argument binding `writable` before passing it for mutation",
                        ),
                    );
                }
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
                .map(|context| {
                    context.this_available
                        && context.receiver_mode.is_some_and(ReceiverMode::is_writable)
                })
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
                format!("unknown property `{class_name}::{property}`"),
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
                format!("property `{class_name}::{property}` is internal"),
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

    fn report_deferred_qualified_name(&mut self, name: &str, span: Span) {
        self.diagnostics.push(Diagnostic::new(
            "E0475",
            format!(
                "qualified name `{name}` is accepted syntax but namespace resolution is not available in this compiler version"
            ),
            span,
        ));
    }

    fn resolve_type_ref(&mut self, ty: &TypeRef, span: Span) -> TypeId {
        self.resolve_type_ref_in_position(ty, span, TypePosition::Value, None)
    }

    fn const_type_id(&mut self, ty: crate::const_eval::ConstType) -> TypeId {
        let kind = match ty {
            crate::const_eval::ConstType::Integer(ty) => TypeKind::Integer(ty),
            crate::const_eval::ConstType::Float(ty) => TypeKind::Float(ty),
            crate::const_eval::ConstType::String => TypeKind::String,
            crate::const_eval::ConstType::Bool => TypeKind::Bool,
            crate::const_eval::ConstType::Null => TypeKind::Null,
            crate::const_eval::ConstType::NullableString => TypeKind::NullableString,
        };
        self.types.intern(kind)
    }

    fn resolve_type_ref_with_class(
        &mut self,
        ty: &TypeRef,
        span: Span,
        declaring_class: Option<&str>,
    ) -> TypeId {
        self.resolve_type_ref_in_position(ty, span, TypePosition::Value, declaring_class)
    }

    fn resolve_type_ref_in_position(
        &mut self,
        ty: &TypeRef,
        span: Span,
        position: TypePosition,
        declaring_class: Option<&str>,
    ) -> TypeId {
        if ty.name.contains('\\') {
            for arg in &ty.args {
                self.resolve_type_ref_in_position(arg, span, TypePosition::Value, declaring_class);
            }
            self.report_deferred_qualified_name(&ty.name, span);
            return self.types.unknown();
        }
        if ty.nullable {
            if ty.name == "string" && ty.args.is_empty() {
                return self.types.intern(TypeKind::NullableString);
            }
            return self.reject_type_ref_with_help(
                ty,
                span,
                "E0454",
                format!("nullable type `{ty}` is not supported by this compiler"),
                "only `?string` is currently available as a nullable type",
            );
        }
        if let Some(integer) = IntegerType::from_source_name(&ty.name) {
            return self.resolve_zero_arg_type(ty, span, TypeKind::Integer(integer));
        }

        if let Some(float) = FloatType::from_source_name(&ty.name) {
            return self.resolve_zero_arg_type(ty, span, TypeKind::Float(float));
        }

        match ty.name.as_str() {
            "self" if ty.args.is_empty() => match declaring_class {
                Some(class_name) => self.types.intern(TypeKind::Class(class_name.to_string())),
                None => self.reject_type_ref(
                    ty,
                    span,
                    "E0492",
                    "`self` is reserved for the declaring or composing class context",
                ),
            },
            "void" if position == TypePosition::Return => {
                self.resolve_zero_arg_type(ty, span, TypeKind::Void)
            }
            "void" => {
                self.reject_type_ref(ty, span, "E0430", "`void` is only valid as a return type")
            }
            "string" => self.resolve_zero_arg_type(ty, span, TypeKind::String),
            "bool" => self.resolve_zero_arg_type(ty, span, TypeKind::Bool),
            "null" => self.reject_type_ref_with_help(
                ty,
                span,
                "E0431",
                "`null` is a literal, not a type name",
                "nullable type syntax like `?T` is planned but not implemented yet; use a supported non-null type or `mixed` for now",
            ),
            "mixed" => self.resolve_zero_arg_type(ty, span, TypeKind::Mixed),
            "object" => self.reject_type_ref_with_help(
                ty,
                span,
                "E0401",
                "unknown type `object`",
                "Doria has no `object` type; use `mixed` for dynamic boundaries until narrowing syntax is implemented",
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
            "uint" => self.reject_type_ref_with_help(
                ty,
                span,
                "E0401",
                "Doria has no bare `uint`; use an explicit width such as `uint64`",
                "choose `uint8`, `uint16`, `uint32`, or `uint64`",
            ),
            "i8" | "i16" | "i32" | "i64" => {
                let width = &ty.name[1..];
                self.reject_type_ref_with_help(
                    ty,
                    span,
                    "E0401",
                    format!("Doria uses `int{width}`, not `{}`", ty.name),
                    "use the Doria fixed-width integer spelling",
                )
            }
            "u8" | "u16" | "u32" | "u64" => {
                let width = &ty.name[1..];
                self.reject_type_ref_with_help(
                    ty,
                    span,
                    "E0401",
                    format!("Doria uses `uint{width}`, not `{}`", ty.name),
                    "use the Doria fixed-width integer spelling",
                )
            }
            "[]" => {
                if !self.expect_type_arg_count(ty, 1, span) {
                    for arg in &ty.args {
                        self.resolve_type_ref_in_position(arg, span, TypePosition::Value, declaring_class);
                    }
                    return self.types.unknown();
                }
                let element =
                    self.resolve_type_ref_in_position(&ty.args[0], span, TypePosition::Value, declaring_class);
                self.types.intern(TypeKind::TypedArray(element))
            }
            "List" => {
                if !self.expect_type_arg_count(ty, 1, span) {
                    for arg in &ty.args {
                        self.resolve_type_ref_in_position(arg, span, TypePosition::Value, declaring_class);
                    }
                    return self.types.unknown();
                }
                let element =
                    self.resolve_type_ref_in_position(&ty.args[0], span, TypePosition::Value, declaring_class);
                self.types.intern(TypeKind::List(element))
            }
            "Dictionary" => {
                if !self.expect_type_arg_count(ty, 2, span) {
                    for arg in &ty.args {
                        self.resolve_type_ref_in_position(arg, span, TypePosition::Value, declaring_class);
                    }
                    return self.types.unknown();
                }
                let key = self.resolve_type_ref_in_position(&ty.args[0], span, TypePosition::Value, declaring_class);
                let value =
                    self.resolve_type_ref_in_position(&ty.args[1], span, TypePosition::Value, declaring_class);
                self.types.intern(TypeKind::Dictionary(key, value))
            }
            "Set" => {
                if !self.expect_type_arg_count(ty, 1, span) {
                    for arg in &ty.args {
                        self.resolve_type_ref_in_position(arg, span, TypePosition::Value, declaring_class);
                    }
                    return self.types.unknown();
                }
                let element =
                    self.resolve_type_ref_in_position(&ty.args[0], span, TypePosition::Value, declaring_class);
                self.types.intern(TypeKind::Set(element))
            }
            name if self.classes.contains_key(name) => {
                if !self.expect_type_arg_count(ty, 0, span) {
                    for arg in &ty.args {
                        self.resolve_type_ref_in_position(arg, span, TypePosition::Value, declaring_class);
                    }
                }
                self.types.intern(TypeKind::Class(name.to_string()))
            }
            name => {
                for arg in &ty.args {
                    self.resolve_type_ref_in_position(arg, span, TypePosition::Value, declaring_class);
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
            self.resolve_type_ref_in_position(arg, span, TypePosition::Value, None);
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
            self.resolve_type_ref_in_position(arg, span, TypePosition::Value, None);
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
                "cannot assign value of type `{value_name}` to property `{class_name}::{name}` of type `{target_name}`"
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
        match *self.types.kind(target) {
            TypeKind::Integer(integer) => {
                if let Some(fits) = self.check_contextual_integer_literal(value_expr, integer) {
                    return fits;
                }
                self.contextualize_integer_literals(value_expr, integer);
            }
            TypeKind::Float(float) => self.contextualize_float_literals(value_expr, float),
            _ => {}
        }

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

    fn update_nullable_assignment_flow_type(
        &self,
        target: &Expr,
        value_ty: TypeId,
        scopes: &mut ScopeStack,
    ) {
        let Some(name) = Self::assignment_target_variable_name(target) else {
            return;
        };
        let Some(binding) = scopes.lookup_mut(name) else {
            return;
        };
        if !matches!(
            self.types.kind(binding.declared_ty),
            TypeKind::NullableString
        ) {
            return;
        }
        binding.ty = if matches!(self.types.kind(value_ty), TypeKind::String) {
            value_ty
        } else {
            binding.declared_ty
        };
    }

    fn is_expr_assignable(
        &mut self,
        target: TypeId,
        value_expr: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> bool {
        match *self.types.kind(target) {
            TypeKind::Integer(integer) => {
                if let Some(fits) = self.check_contextual_integer_literal(value_expr, integer) {
                    return fits;
                }
                self.contextualize_integer_literals(value_expr, integer);
            }
            TypeKind::Float(float) => self.contextualize_float_literals(value_expr, float),
            _ => {}
        }

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
                        let implicit_key = self.types.intern(TypeKind::Integer(IntegerType::Int64));
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
            (TypeKind::NullableString, TypeKind::String | TypeKind::Null) => true,
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
        let ty = self.infer_expr_type_unrecorded(expr, scopes, method_context);
        match self.types.kind(ty) {
            TypeKind::Integer(integer) => {
                self.integer_expression_types
                    .insert((expr.span().start, expr.span().end), *integer);
            }
            TypeKind::Float(float) => {
                self.float_expression_types
                    .insert((expr.span().start, expr.span().end), *float);
            }
            _ => {}
        }
        ty
    }

    fn infer_expr_type_unrecorded(
        &mut self,
        expr: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> TypeId {
        match expr {
            Expr::String { .. } | Expr::InterpolatedString { .. } => {
                self.types.intern(TypeKind::String)
            }
            Expr::Int { span, .. } => {
                let integer = self
                    .integer_expression_types
                    .get(&(span.start, span.end))
                    .copied()
                    .unwrap_or(IntegerType::Int64);
                self.types.intern(TypeKind::Integer(integer))
            }
            Expr::Float { span, .. } => {
                let float = self
                    .float_expression_types
                    .get(&(span.start, span.end))
                    .copied()
                    .unwrap_or(FloatType::Float64);
                self.types.intern(TypeKind::Float(float))
            }
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
            Expr::Identifier { name, .. } => {
                let key = crate::const_eval::ConstKey::TopLevel(name.clone());
                let ty = self.const_evaluation.values.get(&key).map(|value| value.ty);
                ty.map(|ty| self.const_type_id(ty))
                    .unwrap_or_else(|| self.types.unknown())
            }
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
            Expr::FunctionCall { name, .. } => {
                if let Some(builtin) = Builtin::from_name(name) {
                    match builtin {
                        Builtin::ReadLine => self.types.intern(TypeKind::NullableString),
                        Builtin::Sprintf | Builtin::ReadFile => self.types.intern(TypeKind::String),
                        Builtin::Printf
                        | Builtin::WriteFile
                        | Builtin::WriteStderr
                        | Builtin::Panic => self.types.intern(TypeKind::Void),
                    }
                } else {
                    self.functions
                        .get(name)
                        .map(|function| function.return_ty)
                        .unwrap_or_else(|| self.types.unknown())
                }
            }
            Expr::StaticCall {
                qualifier, method, ..
            } => {
                let Some(class_name) = Self::static_qualifier_class_name(qualifier, method_context)
                else {
                    return self.types.unknown();
                };
                if class_name == "Int" && method == "toFloat" {
                    return self.types.intern(TypeKind::Float(FloatType::Float64));
                }
                if class_name == "Float" && method == "toInt" {
                    return self.types.intern(TypeKind::Integer(IntegerType::Int64));
                }
                if method == "from" {
                    if let Some(integer) = IntegerType::from_companion_name(&class_name) {
                        return self.types.intern(TypeKind::Integer(integer));
                    }
                }
                self.classes
                    .get(&class_name)
                    .and_then(|class_info| class_info.methods.get(method))
                    .map(|method| method.return_ty)
                    .unwrap_or_else(|| self.types.unknown())
            }
            Expr::StaticMember {
                qualifier, member, ..
            } => {
                let Some(class_name) = Self::static_qualifier_class_name(qualifier, method_context)
                else {
                    return self.types.unknown();
                };
                let ty = self.classes.get(&class_name).and_then(|class_info| {
                    class_info
                        .constants
                        .get(member)
                        .map(|constant| constant.ty)
                        .or_else(|| {
                            class_info
                                .static_properties
                                .get(member)
                                .map(|property| property.ty)
                        })
                });
                ty.unwrap_or_else(|| {
                    let key = crate::const_eval::ConstKey::Class {
                        class_name,
                        name: member.clone(),
                    };
                    let ty = self.const_evaluation.values.get(&key).map(|value| value.ty);
                    ty.map(|ty| self.const_type_id(ty))
                        .unwrap_or_else(|| self.types.unknown())
                })
            }
            Expr::Grouped { expr, .. } => self.infer_expr_type(expr, scopes, method_context),
            Expr::Unary { op, expr, span } => {
                self.infer_unary_type(op, expr, *span, scopes, method_context)
            }
            Expr::Binary {
                left, op, right, ..
            } => self.infer_binary_type(left, op, right, scopes, method_context),
            Expr::Range { .. } => self.types.unknown(),
        }
    }

    fn static_qualifier_class_name(
        qualifier: &StaticQualifier,
        method_context: Option<&MethodContext>,
    ) -> Option<String> {
        match qualifier {
            StaticQualifier::Class(name) => Some(name.clone()),
            StaticQualifier::SelfType => method_context.map(|context| context.class_name.clone()),
            StaticQualifier::Parent | StaticQualifier::InvalidStatic => None,
        }
    }

    fn infer_unary_type(
        &mut self,
        op: &UnaryOp,
        expr: &Expr,
        span: Span,
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
            UnaryOp::Negate => {
                if Self::integer_literal_parts(expr).is_some() {
                    self.check_contextual_integer_literal(
                        &Expr::Unary {
                            op: UnaryOp::Negate,
                            expr: Box::new(expr.clone()),
                            span,
                        },
                        IntegerType::Int64,
                    );
                }
                match self.types.kind(ty) {
                    TypeKind::Integer(integer) if integer.is_signed() => {
                        self.types.intern(TypeKind::Integer(*integer))
                    }
                    TypeKind::Float(float) => self.types.intern(TypeKind::Float(*float)),
                    TypeKind::Unknown => self.types.unknown(),
                    _ => self.types.intern(TypeKind::Heterogeneous),
                }
            }
            UnaryOp::BitwiseNot => match self.types.kind(ty) {
                TypeKind::Integer(integer) => self.types.intern(TypeKind::Integer(*integer)),
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
        let (left_ty, right_ty) =
            self.infer_contextual_binary_operand_types(left, right, scopes, method_context);

        match op {
            BinaryOp::Add
            | BinaryOp::Sub
            | BinaryOp::Mul
            | BinaryOp::Div
            | BinaryOp::Mod
            | BinaryOp::ShiftLeft
            | BinaryOp::ShiftRight
            | BinaryOp::BitwiseAnd
            | BinaryOp::BitwiseXor
            | BinaryOp::BitwiseOr => self.infer_numeric_binary_type(left_ty, right_ty),
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

    fn infer_contextual_binary_operand_types(
        &mut self,
        left: &Expr,
        right: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> (TypeId, TypeId) {
        let mut left_ty = self.infer_expr_type(left, scopes, method_context);
        let mut right_ty = self.infer_expr_type(right, scopes, method_context);

        let left_literal = Self::integer_literal_parts(left).is_some();
        let right_literal = Self::integer_literal_parts(right).is_some();
        let left_integer = match self.types.kind(left_ty) {
            TypeKind::Integer(integer) => Some(*integer),
            _ => None,
        };
        let right_integer = match self.types.kind(right_ty) {
            TypeKind::Integer(integer) => Some(*integer),
            _ => None,
        };

        if left_literal && !right_literal {
            if let Some(integer) = right_integer {
                self.check_contextual_integer_literal(left, integer);
                left_ty = self.types.intern(TypeKind::Integer(integer));
            }
        } else if right_literal && !left_literal {
            if let Some(integer) = left_integer {
                self.check_contextual_integer_literal(right, integer);
                right_ty = self.types.intern(TypeKind::Integer(integer));
            }
        } else if left_literal && right_literal {
            let integer = match (left_integer, right_integer) {
                (Some(left), Some(right)) if left == right => left,
                _ => IntegerType::Int64,
            };
            self.check_contextual_integer_literal(left, integer);
            self.check_contextual_integer_literal(right, integer);
            left_ty = self.types.intern(TypeKind::Integer(integer));
            right_ty = self.types.intern(TypeKind::Integer(integer));
        }

        let left_float_literal = Self::is_float_literal(left);
        let right_float_literal = Self::is_float_literal(right);
        let left_float = match self.types.kind(left_ty) {
            TypeKind::Float(float) => Some(*float),
            _ => None,
        };
        let right_float = match self.types.kind(right_ty) {
            TypeKind::Float(float) => Some(*float),
            _ => None,
        };

        if left_float_literal && !right_float_literal {
            if let Some(float) = right_float {
                self.record_float_expression_type(left, float);
                left_ty = self.types.intern(TypeKind::Float(float));
            }
        } else if right_float_literal && !left_float_literal {
            if let Some(float) = left_float {
                self.record_float_expression_type(right, float);
                right_ty = self.types.intern(TypeKind::Float(float));
            }
        }

        (left_ty, right_ty)
    }

    fn infer_numeric_binary_type(&mut self, left: TypeId, right: TypeId) -> TypeId {
        if let Some(recovery) = self.recovery_binary_type(left, right) {
            return recovery;
        }

        let left_kind = self.types.kind(left).clone();
        let right_kind = self.types.kind(right).clone();
        match (left_kind, right_kind) {
            (TypeKind::Integer(left), TypeKind::Integer(right)) if left == right => {
                self.types.intern(TypeKind::Integer(left))
            }
            (TypeKind::Float(left), TypeKind::Float(right)) if left == right => {
                self.types.intern(TypeKind::Float(left))
            }
            _ => self.types.intern(TypeKind::Heterogeneous),
        }
    }

    fn infer_concat_binary_type(&mut self, left: TypeId, right: TypeId) -> TypeId {
        if let Some(recovery) = self.recovery_binary_type(left, right) {
            return recovery;
        }

        let left_is_string = matches!(self.types.kind(left), TypeKind::String);
        let right_is_string = matches!(self.types.kind(right), TypeKind::String);
        if (left_is_string && self.is_display_convertible_type(right))
            || (right_is_string && self.is_display_convertible_type(left))
        {
            self.types.intern(TypeKind::String)
        } else {
            self.types.intern(TypeKind::Heterogeneous)
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
            (TypeKind::Integer(left), TypeKind::Integer(right)) if left == right => {
                self.types.intern(TypeKind::Bool)
            }
            (TypeKind::Float(left), TypeKind::Float(right)) if left == right => {
                self.types.intern(TypeKind::Bool)
            }
            (TypeKind::String, TypeKind::String) => self.types.intern(TypeKind::Bool),
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
            let explicit_keys = elements
                .iter()
                .filter_map(|element| element.key.as_ref())
                .collect::<Vec<_>>();
            let mut key_types =
                self.infer_collection_member_types(&explicit_keys, scopes, method_context);
            key_types.extend(
                elements
                    .iter()
                    .filter(|element| element.key.is_none())
                    .map(|_| self.types.intern(TypeKind::Integer(IntegerType::Int64))),
            );
            let values = elements
                .iter()
                .map(|element| &element.value)
                .collect::<Vec<_>>();
            let value_types = self.infer_collection_member_types(&values, scopes, method_context);
            let key = self.common_clear_type(key_types);
            let value = self.common_clear_type(value_types);
            self.types.intern(TypeKind::Dictionary(key, value))
        } else {
            let values = elements
                .iter()
                .map(|element| &element.value)
                .collect::<Vec<_>>();
            let element_types = self.infer_collection_member_types(&values, scopes, method_context);
            let element = self.common_clear_type(element_types);
            self.types.intern(TypeKind::List(element))
        }
    }

    fn infer_collection_member_types(
        &mut self,
        expressions: &[&Expr],
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> Vec<TypeId> {
        let mut contextual_integer = None;
        for expr in expressions {
            if Self::integer_literal_parts(expr).is_some() {
                continue;
            }
            let ty = self.infer_expr_type(expr, scopes, method_context);
            if let TypeKind::Integer(integer) = self.types.kind(ty) {
                match contextual_integer {
                    None => contextual_integer = Some(*integer),
                    Some(current) if current == *integer => {}
                    Some(_) => {
                        contextual_integer = None;
                        break;
                    }
                }
            }
        }

        if let Some(integer) = contextual_integer {
            for expr in expressions {
                self.contextualize_integer_literals(expr, integer);
            }
        }

        expressions
            .iter()
            .map(|expr| self.infer_expr_type(expr, scopes, method_context))
            .collect()
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
                                common = Some(self.merge_inferred_return_types(common_ty, ty));
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
                if let Some(common) = common {
                    if self.type_contains_mixed(common) {
                        return common;
                    }
                }
                return self.types.intern(TypeKind::Mixed);
            }
            return self.types.intern(TypeKind::Heterogeneous);
        }

        if let Some(common) = common {
            if saw_empty_collection && !self.is_collection_like_type(common) {
                if saw_mixed {
                    return self.types.intern(TypeKind::Mixed);
                }
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

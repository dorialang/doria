use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;

use crate::ast::{self, Argument, BinaryOp, Expr, Item, MemberAccess, StaticQualifier, UnaryOp};
use crate::diagnostics::{Diagnostic, DiagnosticResult};
use crate::enums::{EnumCaseSource, EnumId, EnumType};
use crate::numeric::{parse_decimal_magnitude, FloatType, FloatValue, IntegerType, IntegerValue};
use crate::source::Span;
use crate::types::{ResolvedType, TypeRef, TypeRegistry};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstValue {
    Integer(IntegerValue),
    Float(FloatValue),
    String(String),
    Bool(bool),
    Null,
    Enum(ConstEnumValue),
    PayloadEnum(ConstPayloadEnumValue),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstEnumValue {
    pub enum_type: EnumType,
    pub case: EnumCaseSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstPayloadEnumValue {
    pub value: ConstEnumValue,
    pub fields: Vec<ConstValue>,
    pub field_types: Vec<ResolvedType>,
}

impl ConstValue {
    fn display(&self) -> Option<String> {
        match self {
            Self::Integer(value) => Some(value.display()),
            Self::Float(value) => Some(value.display()),
            Self::String(value) => Some(value.clone()),
            Self::Bool(value) => Some(value.to_string()),
            Self::Null => None,
            Self::Enum(_) | Self::PayloadEnum(_) => None,
        }
    }
}

impl Evaluation {
    pub fn enum_case_name(&self, value: &ConstEnumValue) -> Option<(&str, &str)> {
        self.enum_cases
            .iter()
            .find_map(|((enum_name, case_name), candidate)| {
                (candidate.case == value.case && enum_name == &value.enum_type.name)
                    .then_some((enum_name.as_str(), case_name.as_str()))
            })
    }

    pub fn payload_case_name(&self, value: &ConstEnumValue) -> Option<(&str, &str)> {
        self.payload_cases
            .iter()
            .find_map(|((enum_name, case_name), schema)| {
                (schema.value.case == value.case && enum_name == &value.enum_type.name)
                    .then_some((enum_name.as_str(), case_name.as_str()))
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ConstKey {
    TopLevel(String),
    Class { class_name: String, name: String },
    Static { class_name: String, name: String },
}

impl ConstKey {
    pub fn display(&self) -> String {
        match self {
            Self::TopLevel(name) => name.clone(),
            Self::Class { class_name, name } | Self::Static { class_name, name } => {
                format!("{class_name}::{name}")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluatedDecl {
    pub value: ConstValue,
    pub ty: ConstType,
    pub access: MemberAccess,
    pub writable: bool,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TypedValue {
    value: ConstValue,
    ty: ConstType,
}

impl TypedValue {
    fn new(value: ConstValue) -> Self {
        let ty = ConstType::of(&value);
        Self { value, ty }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Evaluation {
    pub values: HashMap<ConstKey, EvaluatedDecl>,
    pub enum_cases: HashMap<(String, String), ConstEnumValue>,
    enum_names: HashMap<String, (EnumId, usize)>,
    payload_cases: HashMap<(String, String), PayloadCaseSchema>,
    class_parents: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PayloadCaseSchema {
    value: ConstEnumValue,
    parameters: Vec<String>,
    fields: Vec<(String, TypeRef)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ParameterDefaultKey {
    pub function: Span,
    pub parameter_index: usize,
}

pub fn evaluate_parameter_default(
    evaluation: &Evaluation,
    expr: &Expr,
    expected: &TypeRef,
    declaring_class: Option<&str>,
) -> Option<ConstValue> {
    let expected = const_type_with_enums(expected, &evaluation.enum_names)?;
    let requester = EvaluationRequester::parameter_default(declaring_class);
    let mut evaluator = Evaluator {
        nodes: HashMap::new(),
        states: evaluation
            .values
            .iter()
            .map(|(key, value)| (key.clone(), State::Done(Box::new(value.clone()))))
            .collect(),
        stack: Vec::new(),
        diagnostics: Vec::new(),
        enum_cases: evaluation.enum_cases.clone(),
        enum_names: evaluation.enum_names.clone(),
        payload_cases: evaluation.payload_cases.clone(),
        class_parents: evaluation.class_parents.clone(),
    };
    let value = evaluator.evaluate_expr(expr, Some(expected.clone()), &requester)?;
    (evaluator.diagnostics.is_empty() && expected.accepts(&value.ty)).then_some(value.value)
}

pub fn evaluate_string_expression(evaluation: &Evaluation, expr: &Expr) -> Option<String> {
    let requester = EvaluationRequester::parameter_default(None);
    let mut evaluator = Evaluator {
        nodes: HashMap::new(),
        states: evaluation
            .values
            .iter()
            .map(|(key, value)| (key.clone(), State::Done(Box::new(value.clone()))))
            .collect(),
        stack: Vec::new(),
        diagnostics: Vec::new(),
        enum_cases: evaluation.enum_cases.clone(),
        enum_names: evaluation.enum_names.clone(),
        payload_cases: evaluation.payload_cases.clone(),
        class_parents: evaluation.class_parents.clone(),
    };
    let value = evaluator.evaluate_expr(expr, Some(ConstType::String), &requester)?;
    match value.value {
        ConstValue::String(value) if evaluator.diagnostics.is_empty() => Some(value),
        _ => None,
    }
}

pub fn evaluate_attribute_value(
    evaluation: &Evaluation,
    expr: &Expr,
    expected: &TypeRef,
    declaring_class: Option<&str>,
) -> DiagnosticResult<ConstValue> {
    let Some(expected) = const_type_with_enums(expected, &evaluation.enum_names) else {
        return Err(vec![Diagnostic::new(
            "E0692",
            format!("type `{expected}` is not compatible with attribute metadata"),
            expr.span(),
        )
        .with_title("Attribute Schema Type Is Not Metadata Compatible")]);
    };
    let requester = EvaluationRequester::parameter_default(declaring_class);
    let mut evaluator = Evaluator {
        nodes: HashMap::new(),
        states: evaluation
            .values
            .iter()
            .map(|(key, value)| (key.clone(), State::Done(Box::new(value.clone()))))
            .collect(),
        stack: Vec::new(),
        diagnostics: Vec::new(),
        enum_cases: evaluation.enum_cases.clone(),
        enum_names: evaluation.enum_names.clone(),
        payload_cases: evaluation.payload_cases.clone(),
        class_parents: evaluation.class_parents.clone(),
    };
    let value = evaluator.evaluate_expr(expr, Some(expected.clone()), &requester);
    if let Some(value) = &value {
        if !expected.accepts(&value.ty) {
            evaluator.invalid(
                expr.span(),
                format!(
                    "attribute constant expects `{expected}`, found `{}`",
                    value.ty
                ),
            );
        }
    }
    if let Some(value) = value.filter(|_| evaluator.diagnostics.is_empty()) {
        Ok(value.value)
    } else {
        for diagnostic in &mut evaluator.diagnostics {
            if diagnostic.code == "E0485" {
                diagnostic.code = "E0693";
                diagnostic.title = "Attribute Argument Must Be Const Evaluable".to_string();
            }
        }
        if evaluator.diagnostics.is_empty() {
            evaluator.diagnostics.push(
                Diagnostic::new(
                    "E0693",
                    "attribute argument must be a compile-time constant expression",
                    expr.span(),
                )
                .with_title("Attribute Argument Must Be Const Evaluable"),
            );
        }
        Err(evaluator.diagnostics)
    }
}

#[derive(Clone)]
struct Node {
    key: ConstKey,
    annotation: Option<TypeRef>,
    initializer: Expr,
    access: MemberAccess,
    writable: bool,
    span: Span,
}

#[derive(Clone)]
enum State {
    Visiting(usize),
    Done(Box<EvaluatedDecl>),
    Failed,
}

#[derive(Clone)]
struct EvaluationRequester {
    declaring_class: Option<String>,
    static_initializer: bool,
}

impl EvaluationRequester {
    fn declaration(key: &ConstKey) -> Self {
        Self {
            declaring_class: match key {
                ConstKey::Class { class_name, .. } | ConstKey::Static { class_name, .. } => {
                    Some(class_name.clone())
                }
                ConstKey::TopLevel(_) => None,
            },
            static_initializer: matches!(key, ConstKey::Static { .. }),
        }
    }

    fn parameter_default(declaring_class: Option<&str>) -> Self {
        Self {
            declaring_class: declaring_class.map(str::to_string),
            static_initializer: false,
        }
    }
}

pub fn evaluate_program(program: &ast::Program) -> DiagnosticResult<Evaluation> {
    let (evaluation, diagnostics) = evaluate_program_with_diagnostics(program);
    if diagnostics.is_empty() {
        Ok(evaluation)
    } else {
        Err(diagnostics)
    }
}

pub(crate) fn evaluate_program_with_diagnostics(
    program: &ast::Program,
) -> (Evaluation, Vec<Diagnostic>) {
    let mut evaluator = Evaluator::collect(program);
    evaluator.evaluate_all();
    (
        Evaluation {
            values: evaluator
                .states
                .into_iter()
                .filter_map(|(key, state)| match state {
                    State::Done(value) => Some((key, *value)),
                    State::Visiting(_) | State::Failed => None,
                })
                .collect(),
            enum_cases: evaluator.enum_cases,
            enum_names: evaluator.enum_names,
            payload_cases: evaluator.payload_cases,
            class_parents: evaluator.class_parents,
        },
        evaluator.diagnostics,
    )
}

struct Evaluator {
    nodes: HashMap<ConstKey, Node>,
    states: HashMap<ConstKey, State>,
    stack: Vec<ConstKey>,
    diagnostics: Vec<Diagnostic>,
    enum_cases: HashMap<(String, String), ConstEnumValue>,
    enum_names: HashMap<String, (EnumId, usize)>,
    payload_cases: HashMap<(String, String), PayloadCaseSchema>,
    class_parents: HashMap<String, String>,
}

impl Evaluator {
    fn collect(program: &ast::Program) -> Self {
        let mut evaluator = Self {
            nodes: HashMap::new(),
            states: HashMap::new(),
            stack: Vec::new(),
            diagnostics: Vec::new(),
            enum_cases: HashMap::new(),
            enum_names: HashMap::new(),
            payload_cases: HashMap::new(),
            class_parents: HashMap::new(),
        };
        for item in &program.items {
            if let Item::Class(declaration) = item {
                if let Some(parent) = &declaration.parent {
                    evaluator
                        .class_parents
                        .insert(declaration.name.clone(), parent.name.clone());
                }
            }
        }
        for item in &program.items {
            if let Item::Enum(declaration) = item {
                let next = evaluator.enum_names.len();
                let (enum_id, _) = *evaluator
                    .enum_names
                    .entry(declaration.name.clone())
                    .or_insert((EnumId(next), declaration.type_params.len()));
                let payload_enum = declaration
                    .cases
                    .iter()
                    .any(|case| !case.payload.is_empty());
                for (index, case) in declaration.cases.iter().enumerate() {
                    let value = ConstEnumValue {
                        enum_type: EnumType::new(enum_id, declaration.name.clone()),
                        case: EnumCaseSource {
                            declaration: enum_id,
                            index,
                        },
                    };
                    if payload_enum {
                        evaluator.payload_cases.insert(
                            (declaration.name.clone(), case.name.clone()),
                            PayloadCaseSchema {
                                value,
                                parameters: declaration
                                    .type_params
                                    .iter()
                                    .map(|parameter| parameter.name.clone())
                                    .collect(),
                                fields: case
                                    .payload
                                    .iter()
                                    .map(|field| (field.name.clone(), field.ty.clone()))
                                    .collect(),
                            },
                        );
                    } else {
                        evaluator
                            .enum_cases
                            .insert((declaration.name.clone(), case.name.clone()), value);
                    }
                }
            }
        }
        for item in &program.items {
            match item {
                Item::Constant(decl) => evaluator.insert(Node {
                    key: ConstKey::TopLevel(decl.name.clone()),
                    annotation: decl.ty.clone(),
                    initializer: decl.initializer.clone(),
                    access: decl.access,
                    writable: false,
                    span: decl.span,
                }),
                Item::Class(class) => {
                    for member in &class.members {
                        match member {
                            ast::ClassMember::Constant(decl) => evaluator.insert(Node {
                                key: ConstKey::Class {
                                    class_name: class.name.clone(),
                                    name: decl.name.clone(),
                                },
                                annotation: decl.ty.clone(),
                                initializer: decl.initializer.clone(),
                                access: decl.access,
                                writable: false,
                                span: decl.span,
                            }),
                            ast::ClassMember::Property(property)
                                if property.is_static
                                    && crate::property_hooks::declaration_facts(
                                        property,
                                        crate::property_hooks::PropertyHookContext::Class,
                                    ).is_none_or(|facts| facts.storage != Some(crate::property_hooks::PropertyHookStorage::Computed)) => {
                                let Some(initializer) = &property.initializer else {
                                    evaluator.diagnostics.push(Diagnostic::new(
                                        "E0480",
                                        format!(
                                            "static property `{}::{}` requires a const-evaluable initializer",
                                            class.name, property.name
                                        ),
                                        property.span,
                                    ));
                                    continue;
                                };
                                evaluator.insert(Node {
                                    key: ConstKey::Static {
                                        class_name: class.name.clone(),
                                        name: property.name.clone(),
                                    },
                                    annotation: Some(property.ty.clone()),
                                    initializer: initializer.clone(),
                                    access: property.access,
                                    writable: property.writable,
                                    span: property.span,
                                });
                            }
                            _ => {}
                        }
                    }
                }
                Item::Enum(_)
                | Item::Interface(_)
                | Item::Trait(_)
                | Item::Function(_)
                | Item::Statement(_) => {}
            }
        }
        evaluator
    }

    fn insert(&mut self, node: Node) {
        let name = match &node.key {
            ConstKey::TopLevel(name)
            | ConstKey::Class { name, .. }
            | ConstKey::Static { name, .. } => name,
        };
        let declared_name = name.rsplit('\\').next().unwrap_or(name);
        if !matches!(&node.key, ConstKey::Static { .. }) && !is_screaming_snake_case(declared_name)
        {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0490",
                    format!(
                        "constant `{}` must use `SCREAMING_SNAKE_CASE`",
                        node.key.display()
                    ),
                    node.span,
                )
                .with_help("rename the constant using uppercase words separated by underscores"),
            );
        }
        if let Some(previous) = self.nodes.get(&node.key) {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0481",
                    format!(
                        "constant or static `{}` is already declared",
                        node.key.display()
                    ),
                    node.span,
                )
                .with_help(format!(
                    "the previous declaration begins at byte {}",
                    previous.span.start
                )),
            );
        } else {
            self.nodes.insert(node.key.clone(), node);
        }
    }

    fn evaluate_all(&mut self) {
        let keys = self.nodes.keys().cloned().collect::<Vec<_>>();
        for key in keys {
            self.evaluate_key(&key);
        }
    }

    fn evaluate_key(&mut self, key: &ConstKey) -> Option<EvaluatedDecl> {
        match self.states.get(key) {
            Some(State::Done(value)) => return Some(value.as_ref().clone()),
            Some(State::Failed) => return None,
            Some(State::Visiting(index)) => {
                let mut chain = self.stack[*index..]
                    .iter()
                    .map(ConstKey::display)
                    .collect::<Vec<_>>();
                chain.push(key.display());
                let span = self
                    .nodes
                    .get(key)
                    .map_or(Span::default(), |node| node.span);
                self.diagnostics.push(Diagnostic::new(
                    "E0482",
                    format!("constant initialization cycle: {}", chain.join(" -> ")),
                    span,
                ));
                return None;
            }
            None => {}
        }

        let node = self.nodes.get(key)?.clone();
        let index = self.stack.len();
        self.states.insert(key.clone(), State::Visiting(index));
        self.stack.push(key.clone());
        let expected = node
            .annotation
            .as_ref()
            .and_then(|ty| const_type_with_enums(ty, &self.enum_names));
        if let Some(annotation) = &node.annotation {
            if expected.is_none() && !matches!(node.key, ConstKey::Static { .. }) {
                self.diagnostics.push(Diagnostic::new(
                    "E0483",
                    format!("`{annotation}` is not a supported constant or static type"),
                    node.span,
                ));
            }
        }
        let requester = EvaluationRequester::declaration(&node.key);
        let evaluated = if node.annotation.is_some() && expected.is_none() {
            None
        } else {
            self.evaluate_expr(&node.initializer, expected.clone(), &requester)
        };
        self.stack.pop();

        let result = evaluated.and_then(|evaluated| {
            if expected
                .as_ref()
                .is_some_and(|expected| !expected.accepts(&evaluated.ty))
            {
                self.diagnostics.push(Diagnostic::new(
                    "E0484",
                    format!(
                        "initializer for `{}` has type `{}`, expected `{}`",
                        key.display(),
                        evaluated.ty,
                        node.annotation
                            .as_ref()
                            .expect("expected type came from annotation")
                    ),
                    node.initializer.span(),
                ));
                return None;
            }
            Some(EvaluatedDecl {
                ty: expected.unwrap_or(evaluated.ty),
                value: evaluated.value,
                access: node.access,
                writable: node.writable,
                span: node.span,
            })
        });
        self.states.insert(
            key.clone(),
            result
                .clone()
                .map_or(State::Failed, |value| State::Done(Box::new(value))),
        );
        result
    }

    fn evaluate_expr(
        &mut self,
        expr: &Expr,
        expected: Option<ConstType>,
        requester: &EvaluationRequester,
    ) -> Option<TypedValue> {
        match expr {
            Expr::Int { value, span } => self.integer_literal(value, false, expected, *span),
            Expr::Float { value, span } => {
                let ty = expected
                    .as_ref()
                    .and_then(ConstType::float)
                    .unwrap_or(FloatType::Float64);
                FloatValue::parse_decimal(ty, value)
                    .map(ConstValue::Float)
                    .map(TypedValue::new)
                    .or_else(|| {
                        self.invalid(*span, format!("invalid `{ty}` constant literal"));
                        None
                    })
            }
            Expr::String { value, .. } => Some(TypedValue::new(ConstValue::String(value.clone()))),
            Expr::Bool { value, .. } => Some(TypedValue::new(ConstValue::Bool(*value))),
            Expr::Null { .. } => Some(TypedValue::new(ConstValue::Null)),
            Expr::Grouped { expr, .. } => self.evaluate_expr(expr, expected, requester),
            Expr::Identifier { name, span } => {
                self.reference(&ConstKey::TopLevel(name.clone()), *span, requester)
            }
            Expr::StaticMember {
                qualifier,
                member,
                span,
                ..
            } => {
                let class_name = self.qualifier_class_name(qualifier, requester)?;
                if let Some(schema) = self
                    .payload_cases
                    .get(&(class_name.clone(), member.clone()))
                    .filter(|schema| schema.fields.is_empty())
                    .cloned()
                {
                    return self.payload_enum(
                        &class_name,
                        member,
                        &[],
                        schema,
                        *span,
                        requester,
                        expected.as_ref(),
                    );
                }
                if let Some(value) = self
                    .enum_cases
                    .get(&(class_name.clone(), member.clone()))
                    .cloned()
                {
                    return Some(TypedValue::new(ConstValue::Enum(value)));
                }
                let constant = self.inherited_key(ConstKey::Class {
                    class_name: class_name.clone(),
                    name: member.clone(),
                });
                if self.nodes.contains_key(&constant) || self.states.contains_key(&constant) {
                    self.reference(&constant, *span, requester)
                } else {
                    let static_key = self.inherited_key(ConstKey::Static {
                        class_name: class_name.clone(),
                        name: member.clone(),
                    });
                    if !requester.static_initializer {
                        self.invalid(*span, "constant expressions cannot read static properties");
                        None
                    } else {
                        self.reference(&static_key, *span, requester)
                    }
                }
            }
            Expr::Unary {
                op: UnaryOp::Negate,
                expr,
                span,
            } if Self::integer_literal_text(expr).is_some() => {
                let value = Self::integer_literal_text(expr).expect("guard ensures a literal");
                self.integer_literal(value, true, expected, *span)
            }
            Expr::Unary { op, expr, span } => {
                let value = self.evaluate_expr(expr, expected, requester)?;
                self.unary(op, value.value, *span).map(TypedValue::new)
            }
            Expr::Binary {
                left,
                op,
                right,
                span,
            } => {
                if *op != BinaryOp::Coalesce && Self::is_contextual_numeric_literal(left) {
                    let right = self.evaluate_expr(right, expected, requester)?;
                    let left = self.evaluate_expr(left, Some(right.ty.clone()), requester)?;
                    return self.binary(op, left, right, *span).map(TypedValue::new);
                }
                let left = self.evaluate_expr(left, expected.clone(), requester)?;
                match (op, &left.value) {
                    (BinaryOp::And, ConstValue::Bool(false)) => {
                        return Some(TypedValue::new(ConstValue::Bool(false)));
                    }
                    (BinaryOp::Or, ConstValue::Bool(true)) => {
                        return Some(TypedValue::new(ConstValue::Bool(true)));
                    }
                    (BinaryOp::Coalesce, ConstValue::Null) => {
                        let expected = left
                            .ty
                            .nullable_payload()
                            .or_else(|| expected.as_ref().and_then(ConstType::nullable_payload))
                            .or(expected);
                        return self.evaluate_expr(right, expected, requester);
                    }
                    (BinaryOp::Coalesce, _) => {
                        return Some(TypedValue {
                            value: left.value,
                            ty: left.ty.nullable_payload().unwrap_or(left.ty),
                        });
                    }
                    _ => {}
                }
                let right = self.evaluate_expr(right, Some(left.ty.clone()), requester)?;
                self.binary(op, left, right, *span).map(TypedValue::new)
            }
            Expr::StaticCall {
                qualifier,
                method,
                args,
                span,
                ..
            } => {
                let class_name = self.qualifier_class_name(qualifier, requester)?;
                if let Some(schema) = self
                    .payload_cases
                    .get(&(class_name.clone(), method.clone()))
                    .cloned()
                {
                    return self.payload_enum(
                        &class_name,
                        method,
                        args,
                        schema,
                        *span,
                        requester,
                        expected.as_ref(),
                    );
                }
                self.convert(&class_name, method, args, *span, requester)
            }
            _ => {
                self.unavailable(requester, expr.span(), "expression");
                None
            }
        }
    }

    fn integer_literal(
        &mut self,
        text: &str,
        negative: bool,
        expected: Option<ConstType>,
        span: Span,
    ) -> Option<TypedValue> {
        let ty = expected
            .as_ref()
            .and_then(ConstType::integer)
            .unwrap_or(IntegerType::Int64);
        let value = parse_decimal_magnitude(text)
            .and_then(|magnitude| IntegerValue::from_literal(ty, magnitude, negative));
        value
            .map(ConstValue::Integer)
            .map(TypedValue::new)
            .or_else(|| {
                self.integer_literal_out_of_range(span, ty);
                None
            })
    }

    fn integer_literal_text(expr: &Expr) -> Option<&str> {
        match expr {
            Expr::Int { value, .. } => Some(value),
            Expr::Grouped { expr, .. } => Self::integer_literal_text(expr),
            _ => None,
        }
    }

    fn is_contextual_numeric_literal(expr: &Expr) -> bool {
        match expr {
            Expr::Int { .. } | Expr::Float { .. } => true,
            Expr::Grouped { expr, .. }
            | Expr::Unary {
                op: UnaryOp::Negate,
                expr,
                ..
            } => Self::is_contextual_numeric_literal(expr),
            _ => false,
        }
    }

    fn integer_literal_out_of_range(&mut self, span: Span, ty: IntegerType) {
        let mut diagnostic = Diagnostic::new(
            "E0417",
            format!(
                "integer literal is outside the Doria `{}` range",
                ty.source_name()
            ),
            span,
        );
        if ty == IntegerType::Int64 {
            diagnostic = diagnostic.with_help(
                "unconstrained integer literals default to `int`; add a `uint64` context when that is intended",
            );
        }
        self.diagnostics.push(diagnostic);
    }

    fn qualifier_class_name(
        &self,
        qualifier: &StaticQualifier,
        requester: &EvaluationRequester,
    ) -> Option<String> {
        match qualifier {
            StaticQualifier::Class(name) => Some(name.clone()),
            StaticQualifier::SelfType => requester.declaring_class.clone(),
            StaticQualifier::Parent => requester
                .declaring_class
                .as_ref()
                .and_then(|class_name| self.class_parents.get(class_name))
                .cloned(),
            StaticQualifier::InvalidStatic => None,
        }
    }

    fn inherited_key(&self, key: ConstKey) -> ConstKey {
        let (kind, class_name, name) = match &key {
            ConstKey::Class { class_name, name } => (false, class_name, name),
            ConstKey::Static { class_name, name } => (true, class_name, name),
            ConstKey::TopLevel(_) => return key,
        };
        let mut current = class_name.clone();
        while let Some(parent) = self.class_parents.get(&current) {
            let candidate = if kind {
                ConstKey::Static {
                    class_name: parent.clone(),
                    name: name.clone(),
                }
            } else {
                ConstKey::Class {
                    class_name: parent.clone(),
                    name: name.clone(),
                }
            };
            if self.nodes.contains_key(&candidate) || self.states.contains_key(&candidate) {
                return candidate;
            }
            current = parent.clone();
        }
        key
    }

    fn reference(
        &mut self,
        key: &ConstKey,
        span: Span,
        requester: &EvaluationRequester,
    ) -> Option<TypedValue> {
        let key = self.inherited_key(key.clone());
        let declaration = self
            .nodes
            .get(&key)
            .map(|node| (node.access, node.writable))
            .or_else(|| match self.states.get(&key) {
                Some(State::Done(value)) => Some((value.access, value.writable)),
                Some(State::Visiting(_) | State::Failed) | None => None,
            });
        let Some((access, writable)) = declaration else {
            self.invalid(
                span,
                format!("unknown constant or static `{}`", key.display()),
            );
            return None;
        };
        if matches!(key, ConstKey::Static { .. }) && writable {
            self.invalid(
                span,
                format!(
                    "constant evaluation cannot read writable static `{}`",
                    key.display()
                ),
            );
            return None;
        }
        let declaring_class = match &key {
            ConstKey::Class { class_name, .. } | ConstKey::Static { class_name, .. } => {
                Some(class_name.as_str())
            }
            ConstKey::TopLevel(_) => None,
        };
        let requester_class = requester.declaring_class.as_deref();
        if declaring_class != requester_class && access == MemberAccess::Internal {
            self.invalid(
                span,
                format!("constant or static `{}` is internal", key.display()),
            );
            return None;
        }
        self.evaluate_key(&key).map(|value| TypedValue {
            value: value.value,
            ty: value.ty,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn payload_enum(
        &mut self,
        enum_name: &str,
        case_name: &str,
        args: &[Argument],
        schema: PayloadCaseSchema,
        span: Span,
        requester: &EvaluationRequester,
        expected_result: Option<&ConstType>,
    ) -> Option<TypedValue> {
        let parameter_names = schema
            .fields
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>();
        let argument_names = args
            .iter()
            .map(|argument| argument.name.as_ref().map(|name| name.text.as_str()))
            .collect::<Vec<_>>();
        let bound = crate::arg_binding::bind_arguments(
            &parameter_names,
            &vec![false; parameter_names.len()],
            &argument_names,
        );
        if !bound.unknown.is_empty()
            || !bound.duplicate.is_empty()
            || !bound.missing.is_empty()
            || bound.overflow != 0
        {
            self.invalid(
                span,
                format!("payload enum constant `{enum_name}::{case_name}` has invalid arguments"),
            );
            return None;
        }

        let patterns = schema
            .parameters
            .iter()
            .map(|name| (name.clone(), ResolvedType::TypeParameter(name.clone())))
            .collect::<HashMap<_, _>>();
        let field_patterns = schema
            .fields
            .iter()
            .map(|(_, ty)| resolve_const_type(ty, &self.enum_names, &patterns))
            .collect::<Option<Vec<_>>>()?;
        let mut value = schema.value;
        let mut types = TypeRegistry::new();
        let mut bindings: HashMap<String, crate::types::TypeId> = HashMap::new();
        let mut context = HashMap::new();
        let callee = format!("enum case `{enum_name}::{case_name}`");
        let mut inference_diagnostics = Vec::new();
        let contextual_application = match expected_result {
            Some(ConstType::Enum(ty) | ConstType::NullableEnum(ty))
                if ty.id == value.enum_type.id && ty.arguments.len() == schema.parameters.len() =>
            {
                Some(ty)
            }
            _ => None,
        };
        if let Some(ty) = contextual_application {
            context.extend(
                schema
                    .parameters
                    .iter()
                    .cloned()
                    .zip(ty.arguments.iter().cloned()),
            );
        }
        let mut fields = vec![None; schema.fields.len()];
        for (argument_index, argument) in args.iter().enumerate() {
            let parameter_index = bound.arg_to_param[argument_index]
                .expect("validated payload enum argument has a parameter");
            let pattern = &field_patterns[parameter_index];
            let mut contextual = context.clone();
            contextual.extend(
                bindings
                    .iter()
                    .map(|(name, ty)| (name.clone(), types.resolved(*ty))),
            );
            let hint = ConstType::from_resolved(crate::types::substitute_resolved_type(
                pattern,
                &contextual,
            ))
            .or_else(|| {
                schema
                    .parameters
                    .is_empty()
                    .then(|| const_type(&schema.fields[parameter_index].1))
                    .flatten()
            });
            let field = self.evaluate_expr(&argument.value, hint, requester)?;
            if !schema.parameters.is_empty() {
                let pattern = types.intern_resolved(pattern);
                let actual = types.intern_resolved(&field.ty.resolved()?);
                crate::generic_inference::infer_type_parameter_bindings(
                    &mut types,
                    &mut inference_diagnostics,
                    &callee,
                    pattern,
                    actual,
                    argument.value.span(),
                    &mut bindings,
                    false,
                );
            }
            fields[parameter_index] = Some((field, argument.value.span()));
        }
        if !inference_diagnostics.is_empty() {
            self.diagnostics.extend(inference_diagnostics);
            return None;
        }
        for (name, ty) in bindings {
            context.insert(name, types.resolved(ty));
        }
        let Some(arguments) = schema
            .parameters
            .iter()
            .map(|name| context.get(name).cloned())
            .collect::<Option<Vec<_>>>()
        else {
            self.invalid(
                span,
                format!("cannot infer all type arguments for {callee}"),
            );
            return None;
        };
        value.enum_type.arguments = arguments;
        let field_types = field_patterns
            .iter()
            .map(|pattern| crate::types::substitute_resolved_type(pattern, &context))
            .collect::<Vec<_>>();
        let mut values = Vec::with_capacity(fields.len());
        for (index, field) in fields.into_iter().enumerate() {
            let (field, field_span) = field.expect("validated payload field is bound");
            let Some(expected) =
                ConstType::from_resolved(field_types[index].clone()).or_else(|| {
                    schema
                        .parameters
                        .is_empty()
                        .then(|| const_type(&schema.fields[index].1))
                        .flatten()
                })
            else {
                self.invalid(
                    field_span,
                    format!(
                        "payload field `${}` is not available in the Copy constant tier",
                        schema.fields[index].0
                    ),
                );
                return None;
            };
            if !expected.accepts(&field.ty) {
                self.invalid(
                    field_span,
                    format!(
                        "payload field `${}` expects `{expected}`, found `{}`",
                        schema.fields[index].0, field.ty
                    ),
                );
                return None;
            }
            values.push(field.value);
        }
        Some(TypedValue::new(ConstValue::PayloadEnum(
            ConstPayloadEnumValue {
                value,
                fields: values,
                field_types,
            },
        )))
    }

    fn unary(&mut self, op: &UnaryOp, value: ConstValue, span: Span) -> Option<ConstValue> {
        let result = match (op, value) {
            (UnaryOp::Negate, ConstValue::Integer(value)) if value.ty.is_signed() => value
                .checked_neg()
                .map(ConstValue::Integer)
                .map_err(|error| error.message()),
            (UnaryOp::Negate, ConstValue::Float(value)) => Ok(ConstValue::Float(value.negate())),
            (UnaryOp::BitwiseNot, ConstValue::Integer(value)) => {
                Ok(ConstValue::Integer(value.bitwise_not()))
            }
            (UnaryOp::Not, ConstValue::Bool(value)) => Ok(ConstValue::Bool(!value)),
            _ => Err("invalid unary operation in constant expression"),
        };
        match result {
            Ok(value) => Some(value),
            Err(message) => {
                self.invalid(span, message);
                None
            }
        }
    }

    fn binary(
        &mut self,
        op: &BinaryOp,
        left: TypedValue,
        right: TypedValue,
        span: Span,
    ) -> Option<ConstValue> {
        if matches!(op, BinaryOp::Equal | BinaryOp::NotEqual)
            && (left.ty.is_nullable() || right.ty.is_nullable())
            && (left.ty.accepts(&right.ty) || right.ty.accepts(&left.ty))
            && (matches!(left.value, ConstValue::Null) || matches!(right.value, ConstValue::Null))
        {
            let equal = matches!(
                (&left.value, &right.value),
                (ConstValue::Null, ConstValue::Null)
            );
            return Some(ConstValue::Bool(if *op == BinaryOp::Equal {
                equal
            } else {
                !equal
            }));
        }

        let (left, right) = (left.value, right.value);
        if *op == BinaryOp::Concat
            && (matches!(&left, ConstValue::String(_)) || matches!(&right, ConstValue::String(_)))
        {
            let result = left
                .display()
                .zip(right.display())
                .map(|(left, right)| ConstValue::String(left + &right));
            if result.is_none() {
                self.invalid(
                    span,
                    "constant expression operand is not display-convertible",
                );
            }
            return result;
        }

        let result = match (left, right) {
            (ConstValue::Integer(left), ConstValue::Integer(right)) if left.ty == right.ty => {
                self.integer_binary(op, left, right)
            }
            (ConstValue::Float(left), ConstValue::Float(right)) if left.ty == right.ty => {
                self.float_binary(op, left, right)
            }
            (ConstValue::Bool(left), ConstValue::Bool(right)) => match op {
                BinaryOp::And => Ok(ConstValue::Bool(left && right)),
                BinaryOp::Or => Ok(ConstValue::Bool(left || right)),
                BinaryOp::Xor => Ok(ConstValue::Bool(left ^ right)),
                BinaryOp::Equal => Ok(ConstValue::Bool(left == right)),
                BinaryOp::NotEqual => Ok(ConstValue::Bool(left != right)),
                // Canonical `false < true` ordering (false = 0, true = 1),
                // matching the concrete-`bool` relational operators in ordinary
                // (non-const) expressions.
                BinaryOp::Less => Ok(ConstValue::Bool((left as u8) < (right as u8))),
                BinaryOp::LessEqual => Ok(ConstValue::Bool((left as u8) <= (right as u8))),
                BinaryOp::Greater => Ok(ConstValue::Bool((left as u8) > (right as u8))),
                BinaryOp::GreaterEqual => Ok(ConstValue::Bool((left as u8) >= (right as u8))),
                _ => Err("invalid boolean operation in constant expression"),
            },
            (ConstValue::String(left), ConstValue::String(right)) => match op {
                BinaryOp::Concat => Ok(ConstValue::String(left + &right)),
                BinaryOp::Equal => Ok(ConstValue::Bool(left == right)),
                BinaryOp::NotEqual => Ok(ConstValue::Bool(left != right)),
                BinaryOp::Less => Ok(ConstValue::Bool(left < right)),
                BinaryOp::LessEqual => Ok(ConstValue::Bool(left <= right)),
                BinaryOp::Greater => Ok(ConstValue::Bool(left > right)),
                BinaryOp::GreaterEqual => Ok(ConstValue::Bool(left >= right)),
                _ => Err("invalid string operation in constant expression"),
            },
            (ConstValue::Enum(left), ConstValue::Enum(right))
                if left.enum_type == right.enum_type =>
            {
                match op {
                    BinaryOp::Equal => Ok(ConstValue::Bool(left.case == right.case)),
                    BinaryOp::NotEqual => Ok(ConstValue::Bool(left.case != right.case)),
                    _ => Err("only equality is defined for enum constants"),
                }
            }
            (ConstValue::PayloadEnum(left), ConstValue::PayloadEnum(right))
                if left.value.enum_type == right.value.enum_type =>
            {
                match op {
                    BinaryOp::Equal => Ok(ConstValue::Bool(
                        left.value.case == right.value.case && left.fields == right.fields,
                    )),
                    BinaryOp::NotEqual => Ok(ConstValue::Bool(
                        left.value.case != right.value.case || left.fields != right.fields,
                    )),
                    _ => Err("only equality is defined for enum constants"),
                }
            }
            (ConstValue::Null, ConstValue::Null) => match op {
                BinaryOp::Equal => Ok(ConstValue::Bool(true)),
                BinaryOp::NotEqual => Ok(ConstValue::Bool(false)),
                _ => Err("invalid null operation in constant expression"),
            },
            _ => Err("constant expression operands must have compatible types"),
        };
        match result {
            Ok(value) => Some(value),
            Err(message) => {
                self.invalid(span, message);
                None
            }
        }
    }

    fn integer_binary(
        &self,
        op: &BinaryOp,
        left: IntegerValue,
        right: IntegerValue,
    ) -> Result<ConstValue, &'static str> {
        let integer = |result: Result<IntegerValue, crate::numeric::IntegerPanic>| {
            result
                .map(ConstValue::Integer)
                .map_err(|error| error.message())
        };
        match op {
            BinaryOp::Add => integer(left.checked_add(right)),
            BinaryOp::Sub => integer(left.checked_sub(right)),
            BinaryOp::Mul => integer(left.checked_mul(right)),
            BinaryOp::Div => integer(left.divide(right)),
            BinaryOp::Mod => integer(left.remainder(right)),
            BinaryOp::ShiftLeft => integer(left.shift_left(right)),
            BinaryOp::ShiftRight => integer(left.shift_right(right)),
            BinaryOp::BitwiseAnd => Ok(ConstValue::Integer(left.bitwise_and(right))),
            BinaryOp::BitwiseOr => Ok(ConstValue::Integer(left.bitwise_or(right))),
            BinaryOp::BitwiseXor => Ok(ConstValue::Integer(left.bitwise_xor(right))),
            BinaryOp::Equal => Ok(ConstValue::Bool(left.compare(right) == Ordering::Equal)),
            BinaryOp::NotEqual => Ok(ConstValue::Bool(left.compare(right) != Ordering::Equal)),
            BinaryOp::Less => Ok(ConstValue::Bool(left.compare(right) == Ordering::Less)),
            BinaryOp::LessEqual => Ok(ConstValue::Bool(left.compare(right) != Ordering::Greater)),
            BinaryOp::Greater => Ok(ConstValue::Bool(left.compare(right) == Ordering::Greater)),
            BinaryOp::GreaterEqual => Ok(ConstValue::Bool(left.compare(right) != Ordering::Less)),
            _ => Err("invalid integer operation in constant expression"),
        }
    }

    fn float_binary(
        &self,
        op: &BinaryOp,
        left: FloatValue,
        right: FloatValue,
    ) -> Result<ConstValue, &'static str> {
        match op {
            BinaryOp::Add => Ok(ConstValue::Float(left.add(right))),
            BinaryOp::Sub => Ok(ConstValue::Float(left.subtract(right))),
            BinaryOp::Mul => Ok(ConstValue::Float(left.multiply(right))),
            BinaryOp::Div => Ok(ConstValue::Float(left.divide(right))),
            BinaryOp::Equal => Ok(ConstValue::Bool(left.compare_equal(right))),
            BinaryOp::NotEqual => Ok(ConstValue::Bool(left.compare_not_equal(right))),
            BinaryOp::Less => Ok(ConstValue::Bool(left.compare_less(right))),
            BinaryOp::LessEqual => Ok(ConstValue::Bool(left.compare_less_equal(right))),
            BinaryOp::Greater => Ok(ConstValue::Bool(left.compare_greater(right))),
            BinaryOp::GreaterEqual => Ok(ConstValue::Bool(left.compare_greater_equal(right))),
            _ => Err("invalid float operation in constant expression"),
        }
    }

    fn convert(
        &mut self,
        class_name: &str,
        method: &str,
        args: &[Argument],
        span: Span,
        requester: &EvaluationRequester,
    ) -> Option<TypedValue> {
        let is_integer_conversion =
            method == "from" && IntegerType::from_companion_name(class_name).is_some();
        let is_scalar_conversion = matches!(
            (class_name, method),
            ("Int", "toFloat") | ("Float", "toInt")
        );
        if !is_integer_conversion && !is_scalar_conversion {
            self.unavailable(requester, span, "call");
            return None;
        }

        let [argument] = args else {
            self.invalid(span, "constant conversion expects exactly one argument");
            return None;
        };
        let argument = &argument.value;
        if method == "from" {
            if let Some(target) = IntegerType::from_companion_name(class_name) {
                let ConstValue::Integer(value) =
                    self.evaluate_expr(argument, None, requester)?.value
                else {
                    self.invalid(span, "integer conversion requires an integer constant");
                    return None;
                };
                return value
                    .convert(target)
                    .map(ConstValue::Integer)
                    .map(TypedValue::new)
                    .ok()
                    .or_else(|| {
                        self.invalid(span, "integer conversion is out of range");
                        None
                    });
            }
        }
        if class_name == "Int" && method == "toFloat" {
            let ConstValue::Integer(value) = self
                .evaluate_expr(
                    argument,
                    Some(ConstType::Integer(IntegerType::Int64)),
                    requester,
                )?
                .value
            else {
                self.invalid(span, "Int::toFloat requires an int constant");
                return None;
            };
            return Some(TypedValue::new(ConstValue::Float(FloatValue::from_f64(
                value.signed_value() as f64,
            ))));
        }
        if class_name == "Float" && method == "toInt" {
            let ConstValue::Float(value) = self
                .evaluate_expr(
                    argument,
                    Some(ConstType::Float(FloatType::Float64)),
                    requester,
                )?
                .value
            else {
                self.invalid(span, "Float::toInt requires a float constant");
                return None;
            };
            return value
                .to_i64_checked()
                .map(|value| {
                    TypedValue::new(ConstValue::Integer(
                        IntegerValue::from_i128(IntegerType::Int64, value as i128)
                            .expect("i64 fits int"),
                    ))
                })
                .or_else(|| {
                    self.invalid(span, "float-to-int constant conversion is out of range");
                    None
                });
        }
        unreachable!("recognized constant conversions return from their matching branch")
    }

    fn unavailable(&mut self, requester: &EvaluationRequester, span: Span, operation: &str) {
        let diagnostic = if requester.static_initializer {
            Diagnostic::new(
                "E0485",
                format!(
                    "{operation} is not const-evaluable; runtime-initialized statics require a future accepted decision record"
                ),
                span,
            )
            .with_help("use a const-evaluable initializer or defer this static until runtime initialization semantics are accepted")
        } else {
            Diagnostic::new(
                "E0485",
                format!("{operation} is not available in constant evaluation"),
                span,
            )
        };
        self.diagnostics.push(diagnostic);
    }

    fn invalid(&mut self, span: Span, message: impl Into<String>) {
        self.diagnostics
            .push(Diagnostic::new("E0485", message, span));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstType {
    Integer(IntegerType),
    NullableInteger(IntegerType),
    Float(FloatType),
    NullableFloat(FloatType),
    String,
    Bool,
    NullableBool,
    Null,
    NullableString,
    Enum(EnumType),
    NullableEnum(EnumType),
    Inline(crate::inline_value::InlineValueType),
    NullableInline(crate::inline_value::InlineValueType),
    NullableInlineNominal(Box<TypeRef>),
}

impl ConstType {
    fn from_resolved(ty: ResolvedType) -> Option<Self> {
        Some(match ty {
            ResolvedType::Integer(ty) => Self::Integer(ty),
            ResolvedType::Float(ty) => Self::Float(ty),
            ResolvedType::String => Self::String,
            ResolvedType::Bool => Self::Bool,
            ResolvedType::Null => Self::Null,
            ResolvedType::Enum(ty) => Self::Enum(ty),
            ResolvedType::Nullable(inner) => match Self::from_resolved(*inner)? {
                Self::Integer(ty) => Self::NullableInteger(ty),
                Self::Float(ty) => Self::NullableFloat(ty),
                Self::String => Self::NullableString,
                Self::Bool => Self::NullableBool,
                Self::Enum(ty) => Self::NullableEnum(ty),
                _ => return None,
            },
            _ => return None,
        })
    }

    fn resolved(&self) -> Option<ResolvedType> {
        Some(match self {
            Self::Integer(ty) => ResolvedType::Integer(*ty),
            Self::Float(ty) => ResolvedType::Float(*ty),
            Self::String => ResolvedType::String,
            Self::Bool => ResolvedType::Bool,
            Self::Null => ResolvedType::Null,
            Self::Enum(ty) => ResolvedType::Enum(ty.clone()),
            ty => crate::types::nullable_of(ty.nullable_payload()?.resolved()?),
        })
    }

    fn of(value: &ConstValue) -> Self {
        match value {
            ConstValue::Integer(value) => Self::Integer(value.ty),
            ConstValue::Float(value) => Self::Float(value.ty),
            ConstValue::String(_) => Self::String,
            ConstValue::Bool(_) => Self::Bool,
            ConstValue::Null => Self::Null,
            ConstValue::Enum(value) => Self::Enum(value.enum_type.clone()),
            ConstValue::PayloadEnum(value) => Self::Enum(value.value.enum_type.clone()),
        }
    }

    fn accepts(&self, actual: &Self) -> bool {
        self == actual
            || match (self, actual) {
                (Self::NullableInlineNominal(expected), Self::NullableInlineNominal(actual)) => {
                    let substitutions = HashMap::new();
                    crate::types::resolved_type_ref_with_substitutions(expected, &substitutions)
                        .is_some_and(|expected| {
                            crate::types::resolved_type_ref_with_substitutions(
                                actual,
                                &substitutions,
                            )
                            .is_some_and(|actual| expected == actual)
                        })
                }
                (Self::NullableString, Self::String | Self::Null)
                | (Self::NullableInteger(_), Self::Null)
                | (Self::NullableFloat(_), Self::Null)
                | (Self::NullableBool, Self::Bool | Self::Null) => true,
                (Self::NullableEnum(expected), Self::Enum(actual)) => expected == actual,
                (Self::NullableEnum(_), Self::Null) => true,
                (Self::NullableInline(_), Self::Null) => true,
                (Self::NullableInlineNominal(_), Self::Null) => true,
                (Self::NullableInline(expected), Self::Inline(actual)) => expected == actual,
                (Self::NullableInteger(expected), Self::Integer(actual)) => expected == actual,
                (Self::NullableFloat(expected), Self::Float(actual)) => expected == actual,
                _ => false,
            }
    }

    const fn is_nullable(&self) -> bool {
        matches!(
            self,
            Self::NullableInteger(_)
                | Self::NullableFloat(_)
                | Self::NullableBool
                | Self::NullableString
                | Self::NullableEnum(_)
                | Self::NullableInline(_)
                | Self::NullableInlineNominal(_)
        )
    }

    fn nullable_payload(&self) -> Option<Self> {
        match self {
            Self::NullableInteger(ty) => Some(Self::Integer(*ty)),
            Self::NullableFloat(ty) => Some(Self::Float(*ty)),
            Self::NullableBool => Some(Self::Bool),
            Self::NullableString => Some(Self::String),
            Self::NullableEnum(ty) => Some(Self::Enum(ty.clone())),
            Self::NullableInline(ty) => Some(Self::Inline(*ty)),
            _ => None,
        }
    }

    fn integer(&self) -> Option<IntegerType> {
        match self {
            Self::Integer(ty) | Self::NullableInteger(ty) => Some(*ty),
            _ => None,
        }
    }

    fn float(&self) -> Option<FloatType> {
        match self {
            Self::Float(ty) | Self::NullableFloat(ty) => Some(*ty),
            _ => None,
        }
    }
}

impl fmt::Display for ConstType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Integer(ty) => formatter.write_str(ty.source_name()),
            Self::NullableInteger(ty) => write!(formatter, "?{}", ty.source_name()),
            Self::Float(ty) => formatter.write_str(ty.source_name()),
            Self::NullableFloat(ty) => write!(formatter, "?{}", ty.source_name()),
            Self::String => formatter.write_str("string"),
            Self::Bool => formatter.write_str("bool"),
            Self::NullableBool => formatter.write_str("?bool"),
            Self::Null => formatter.write_str("null"),
            Self::NullableString => formatter.write_str("?string"),
            Self::Enum(ty) | Self::NullableEnum(ty) => {
                let mut types = crate::types::TypeRegistry::new();
                let id = types.intern_resolved(&crate::types::ResolvedType::Enum(ty.clone()));
                if matches!(self, Self::NullableEnum(_)) {
                    formatter.write_str("?")?;
                }
                formatter.write_str(&types.display(id))
            }
            Self::Inline(ty) => formatter.write_str(ty.name()),
            Self::NullableInline(ty) => write!(formatter, "?{}", ty.name()),
            Self::NullableInlineNominal(ty) => write!(formatter, "{ty}"),
        }
    }
}

fn const_type(ty: &TypeRef) -> Option<ConstType> {
    if ty.nullable
        && matches!(
            crate::inline_value::InlineValueKind::for_nominal(&ty.name, ty.arguments.len()),
            Some(
                crate::inline_value::InlineValueKind::RegistrationKey
                    | crate::inline_value::InlineValueKind::ReadTransferKey
                    | crate::inline_value::InlineValueKind::WriteTransferKey
                    | crate::inline_value::InlineValueKind::CancellationToken
                    | crate::inline_value::InlineValueKind::StandardInput
                    | crate::inline_value::InlineValueKind::StandardOutput
                    | crate::inline_value::InlineValueKind::StandardError
            )
        )
    {
        return Some(ConstType::NullableInlineNominal(Box::new(ty.clone())));
    }
    if ty.arguments.is_empty() {
        if let Some(value) = crate::inline_value::InlineValueType::for_name(&ty.name) {
            return Some(if ty.nullable {
                ConstType::NullableInline(value)
            } else {
                ConstType::Inline(value)
            });
        }
        if let Some(integer) = IntegerType::from_source_name(&ty.name) {
            return Some(if ty.nullable {
                ConstType::NullableInteger(integer)
            } else {
                ConstType::Integer(integer)
            });
        }
        if let Some(float) = FloatType::from_source_name(&ty.name) {
            return Some(if ty.nullable {
                ConstType::NullableFloat(float)
            } else {
                ConstType::Float(float)
            });
        }
    }
    match (ty.nullable, ty.name.as_str(), ty.arguments.is_empty()) {
        (false, "string", true) => Some(ConstType::String),
        (true, "string", true) => Some(ConstType::NullableString),
        (false, "bool", true) => Some(ConstType::Bool),
        (true, "bool", true) => Some(ConstType::NullableBool),
        _ => None,
    }
}

fn const_type_with_enums(
    ty: &TypeRef,
    enum_names: &HashMap<String, (EnumId, usize)>,
) -> Option<ConstType> {
    const_type(ty)
        .or_else(|| ConstType::from_resolved(resolve_const_type(ty, enum_names, &HashMap::new())?))
}

fn resolve_const_type(
    ty: &TypeRef,
    enum_names: &HashMap<String, (EnumId, usize)>,
    substitutions: &HashMap<String, ResolvedType>,
) -> Option<ResolvedType> {
    let ty = crate::types::resolved_type_ref_with_substitutions(ty, substitutions)?;
    Some(crate::types::resolve_nominal_kinds_with(ty, &|class| {
        if let Some(&(id, arity)) = enum_names.get(&class.name) {
            if class.arguments.len() == arity {
                return ResolvedType::Enum(EnumType::with_arguments(
                    id,
                    class.name,
                    class.arguments,
                ));
            }
        }
        ResolvedType::Class(class)
    }))
}

fn is_screaming_snake_case(name: &str) -> bool {
    name.as_bytes().first().is_some_and(u8::is_ascii_uppercase)
        && name
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

#[cfg(test)]
mod enum_constant_tests {
    use super::*;
    use crate::types::ResolvedType;

    #[test]
    fn enum_constants_infer_nested_applications_and_fill_empty_cases_from_context() {
        let program = crate::parse_source(
            "generic-constant.doria",
            r#"
enum Item<T> { case Value(T $value); case Empty; }
enum Outer<T> { case Wrap(Item<T> $item); }
const Outer<bool> PRESENT = Outer::Wrap(Item::Value(true));
const Outer<bool> EMPTY = Outer::Wrap(Item::Empty);
const Item<bool> WRONG = Item::Value("text");
"#,
        )
        .unwrap();
        let (evaluation, diagnostics) = evaluate_program_with_diagnostics(&program);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert!(!evaluation
            .values
            .contains_key(&ConstKey::TopLevel("WRONG".into())));
        for name in ["PRESENT", "EMPTY"] {
            let ConstValue::PayloadEnum(outer) =
                &evaluation.values[&ConstKey::TopLevel(name.into())].value
            else {
                panic!("expected outer payload");
            };
            let ConstValue::PayloadEnum(inner) = &outer.fields[0] else {
                panic!("expected inner payload");
            };
            assert_eq!(outer.value.enum_type.arguments, vec![ResolvedType::Bool]);
            assert_eq!(inner.value.enum_type.arguments, vec![ResolvedType::Bool]);
            assert_eq!(
                outer.field_types,
                vec![ResolvedType::Enum(inner.value.enum_type.clone())]
            );
        }
    }

    #[test]
    fn enum_constants_validate_nested_field_types_after_argument_binding() {
        let program = crate::parse_source(
            "enum-payload-types.doria",
            r#"
enum State { case Ready; }
enum Other { case Ready; }
enum Packet { case Value(?State $state, bool $flag); }
const Packet GOOD = Packet::Value(flag: true, state: null);
const Packet BAD = Packet::Value(flag: true, state: Other::Ready);
"#,
        )
        .unwrap();
        let (evaluation, diagnostics) = evaluate_program_with_diagnostics(&program);
        assert!(evaluation
            .values
            .contains_key(&ConstKey::TopLevel("GOOD".into())));
        assert!(!evaluation
            .values
            .contains_key(&ConstKey::TopLevel("BAD".into())));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert!(diagnostics[0]
            .message
            .contains("expects `?State`, found `Other`"));
    }

    #[test]
    fn enum_constant_types_and_equality_preserve_the_complete_application() {
        let application = |argument| ConstEnumValue {
            enum_type: EnumType::with_arguments(EnumId(17), "Packet", vec![argument]),
            case: EnumCaseSource {
                declaration: EnumId(17),
                index: 0,
            },
        };
        let first = application(ResolvedType::Bool);
        let second = application(ResolvedType::String);
        let expected = ConstType::Enum(first.enum_type.clone());
        let nullable = ConstType::NullableEnum(first.enum_type.clone());
        assert_eq!(expected.to_string(), "Packet<bool>");
        assert_eq!(nullable.to_string(), "?Packet<bool>");
        assert!(nullable.accepts(&expected));
        assert!(nullable.accepts(&ConstType::Null));
        assert!(!nullable.accepts(&ConstType::Enum(second.enum_type.clone())));
        assert_eq!(nullable.nullable_payload(), Some(expected.clone()));

        let program =
            crate::parse_source("enum-constant.doria", "function main(): void {}").unwrap();
        for payload in [false, true] {
            let value = |identity: ConstEnumValue| {
                if payload {
                    ConstValue::PayloadEnum(ConstPayloadEnumValue {
                        value: identity,
                        fields: vec![ConstValue::Bool(true)],
                        field_types: vec![ResolvedType::Bool],
                    })
                } else {
                    ConstValue::Enum(identity)
                }
            };
            assert_eq!(ConstType::of(&value(first.clone())), expected);
            let mut evaluator = Evaluator::collect(&program);
            assert_eq!(
                evaluator.binary(
                    &BinaryOp::Equal,
                    TypedValue::new(value(first.clone())),
                    TypedValue::new(value(first.clone())),
                    Span::default()
                ),
                Some(ConstValue::Bool(true))
            );
            assert_eq!(
                evaluator.binary(
                    &BinaryOp::Equal,
                    TypedValue::new(value(first.clone())),
                    TypedValue::new(value(second.clone())),
                    Span::default()
                ),
                None
            );
            assert_eq!(evaluator.diagnostics.len(), 1);
        }
    }
}

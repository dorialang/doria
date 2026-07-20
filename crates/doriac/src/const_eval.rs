use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;

use crate::ast::{self, BinaryOp, Expr, Item, MemberAccess, StaticQualifier, UnaryOp};
use crate::diagnostics::{Diagnostic, DiagnosticResult};
use crate::numeric::{parse_decimal_magnitude, FloatType, FloatValue, IntegerType, IntegerValue};
use crate::source::Span;
use crate::types::TypeRef;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstValue {
    Integer(IntegerValue),
    Float(FloatValue),
    String(String),
    Bool(bool),
    Null,
}

impl ConstValue {
    fn display(&self) -> Option<String> {
        match self {
            Self::Integer(value) => Some(value.display()),
            Self::Float(value) => Some(value.display()),
            Self::String(value) => Some(value.clone()),
            Self::Bool(value) => Some(value.to_string()),
            Self::Null => None,
        }
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ParameterDefaultKey {
    pub function_start: usize,
    pub parameter_index: usize,
}

pub fn evaluate_parameter_default(
    evaluation: &Evaluation,
    expr: &Expr,
    expected: &TypeRef,
    declaring_class: Option<&str>,
) -> Option<ConstValue> {
    let expected = const_type(expected)?;
    let requester = EvaluationRequester::parameter_default(declaring_class);
    let mut evaluator = Evaluator {
        nodes: HashMap::new(),
        states: evaluation
            .values
            .iter()
            .map(|(key, value)| (key.clone(), State::Done(value.clone())))
            .collect(),
        stack: Vec::new(),
        diagnostics: Vec::new(),
    };
    let value = evaluator.evaluate_expr(expr, Some(expected), &requester)?;
    evaluator.diagnostics.is_empty().then_some(value.value)
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
    Done(EvaluatedDecl),
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
    let mut evaluator = Evaluator::collect(program);
    evaluator.evaluate_all();
    if evaluator.diagnostics.is_empty() {
        Ok(Evaluation {
            values: evaluator
                .states
                .into_iter()
                .filter_map(|(key, state)| match state {
                    State::Done(value) => Some((key, value)),
                    State::Visiting(_) | State::Failed => None,
                })
                .collect(),
        })
    } else {
        Err(evaluator.diagnostics)
    }
}

struct Evaluator {
    nodes: HashMap<ConstKey, Node>,
    states: HashMap<ConstKey, State>,
    stack: Vec<ConstKey>,
    diagnostics: Vec<Diagnostic>,
}

impl Evaluator {
    fn collect(program: &ast::Program) -> Self {
        let mut evaluator = Self {
            nodes: HashMap::new(),
            states: HashMap::new(),
            stack: Vec::new(),
            diagnostics: Vec::new(),
        };
        for item in &program.items {
            match item {
                Item::Constant(decl) => evaluator.insert(Node {
                    key: ConstKey::TopLevel(decl.name.clone()),
                    annotation: decl.ty.clone(),
                    initializer: decl.initializer.clone(),
                    access: decl.access.clone(),
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
                                access: decl.access.clone(),
                                writable: false,
                                span: decl.span,
                            }),
                            ast::ClassMember::Property(property) if property.is_static => {
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
                                    access: property.access.clone(),
                                    writable: property.writable,
                                    span: property.span,
                                });
                            }
                            _ => {}
                        }
                    }
                }
                Item::Interface(_) | Item::Trait(_) | Item::Function(_) | Item::Statement(_) => {}
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
        if !matches!(&node.key, ConstKey::Static { .. }) && !is_screaming_snake_case(name) {
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
            Some(State::Done(value)) => return Some(value.clone()),
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
        let expected = node.annotation.as_ref().and_then(const_type);
        if let Some(annotation) = &node.annotation {
            if expected.is_none() {
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
            self.evaluate_expr(&node.initializer, expected, &requester)
        };
        self.stack.pop();

        let result = evaluated.and_then(|evaluated| {
            if expected.is_some_and(|expected| !expected.accepts(evaluated.ty)) {
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
            result.clone().map_or(State::Failed, State::Done),
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
                let constant = ConstKey::Class {
                    class_name: class_name.clone(),
                    name: member.clone(),
                };
                if self.nodes.contains_key(&constant) || self.states.contains_key(&constant) {
                    self.reference(&constant, *span, requester)
                } else {
                    let static_key = ConstKey::Static {
                        class_name: class_name.clone(),
                        name: member.clone(),
                    };
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
                if Self::is_contextual_numeric_literal(left) {
                    let right = self.evaluate_expr(right, expected, requester)?;
                    let left = self.evaluate_expr(left, Some(right.ty), requester)?;
                    return self.binary(op, left, right, *span).map(TypedValue::new);
                }
                let left = self.evaluate_expr(left, expected, requester)?;
                match (op, &left.value) {
                    (BinaryOp::And, ConstValue::Bool(false)) => {
                        return Some(TypedValue::new(ConstValue::Bool(false)));
                    }
                    (BinaryOp::Or, ConstValue::Bool(true)) => {
                        return Some(TypedValue::new(ConstValue::Bool(true)));
                    }
                    _ => {}
                }
                let right = self.evaluate_expr(right, Some(left.ty), requester)?;
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
            StaticQualifier::Parent | StaticQualifier::InvalidStatic => None,
        }
    }

    fn reference(
        &mut self,
        key: &ConstKey,
        span: Span,
        requester: &EvaluationRequester,
    ) -> Option<TypedValue> {
        let declaration = self
            .nodes
            .get(key)
            .map(|node| (node.access.clone(), node.writable))
            .or_else(|| match self.states.get(key) {
                Some(State::Done(value)) => Some((value.access.clone(), value.writable)),
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
        let declaring_class = match key {
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
        self.evaluate_key(key).map(|value| TypedValue {
            value: value.value,
            ty: value.ty,
        })
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
            && (matches!(left.ty, ConstType::NullableString)
                || matches!(right.ty, ConstType::NullableString))
        {
            let nullable_operand = |ty: ConstType| {
                matches!(
                    ty,
                    ConstType::String | ConstType::Null | ConstType::NullableString
                )
            };
            if nullable_operand(left.ty) && nullable_operand(right.ty) {
                let equal = match (&left.value, &right.value) {
                    (ConstValue::String(left), ConstValue::String(right)) => left == right,
                    (ConstValue::Null, ConstValue::Null) => true,
                    (ConstValue::String(_), ConstValue::Null)
                    | (ConstValue::Null, ConstValue::String(_)) => false,
                    _ => unreachable!("nullable string types carry only string or null values"),
                };
                return Some(ConstValue::Bool(if *op == BinaryOp::Equal {
                    equal
                } else {
                    !equal
                }));
            }
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
        args: &[Expr],
        span: Span,
        requester: &EvaluationRequester,
    ) -> Option<TypedValue> {
        let [argument] = args else {
            self.invalid(span, "constant conversion expects exactly one argument");
            return None;
        };
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
        self.unavailable(requester, span, "call");
        None
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}

impl ConstType {
    fn of(value: &ConstValue) -> Self {
        match value {
            ConstValue::Integer(value) => Self::Integer(value.ty),
            ConstValue::Float(value) => Self::Float(value.ty),
            ConstValue::String(_) => Self::String,
            ConstValue::Bool(_) => Self::Bool,
            ConstValue::Null => Self::Null,
        }
    }

    fn accepts(self, actual: Self) -> bool {
        self == actual
            || match (self, actual) {
                (Self::NullableString, Self::String | Self::Null)
                | (Self::NullableInteger(_), Self::Null)
                | (Self::NullableFloat(_), Self::Null)
                | (Self::NullableBool, Self::Bool | Self::Null) => true,
                (Self::NullableInteger(expected), Self::Integer(actual)) => expected == actual,
                (Self::NullableFloat(expected), Self::Float(actual)) => expected == actual,
                _ => false,
            }
    }

    fn integer(self) -> Option<IntegerType> {
        match self {
            Self::Integer(ty) | Self::NullableInteger(ty) => Some(ty),
            _ => None,
        }
    }

    fn float(self) -> Option<FloatType> {
        match self {
            Self::Float(ty) | Self::NullableFloat(ty) => Some(ty),
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
        }
    }
}

fn const_type(ty: &TypeRef) -> Option<ConstType> {
    if ty.args.is_empty() {
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
    match (ty.nullable, ty.name.as_str(), ty.args.is_empty()) {
        (false, "string", true) => Some(ConstType::String),
        (true, "string", true) => Some(ConstType::NullableString),
        (false, "bool", true) => Some(ConstType::Bool),
        (true, "bool", true) => Some(ConstType::NullableBool),
        _ => None,
    }
}

fn is_screaming_snake_case(name: &str) -> bool {
    name.as_bytes().first().is_some_and(u8::is_ascii_uppercase)
        && name
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

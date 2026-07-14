//! Stage 19 ownership checking for class move values.
//!
//! This pass is intentionally backend-independent. It runs after ordinary
//! semantic/type checking and records errors in source vocabulary before MIR
//! lowering or either native backend can observe an invalid ownership graph.

use std::collections::{HashMap, HashSet};

use crate::ast::{self, AssignOp, ClassMember, Expr, Item, Stmt};
use crate::diagnostics::Diagnostic;
use crate::source::Span;

#[derive(Debug, Clone)]
struct Parameter {
    class: Option<String>,
    take: bool,
}

#[derive(Debug, Clone, Default)]
struct Signature {
    params: Vec<Parameter>,
    returns: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum State {
    Borrowed,
    Owned,
    Given { at: Span },
    MaybeGiven { at: Span },
}

#[derive(Debug, Clone)]
struct Binding {
    class: String,
    writable: bool,
    state: State,
}

#[derive(Debug, Clone, Default)]
struct Scopes(Vec<HashMap<String, Binding>>);

impl Scopes {
    fn new() -> Self {
        Self(vec![HashMap::new()])
    }

    fn push(&mut self) {
        self.0.push(HashMap::new());
    }

    fn pop(&mut self) {
        self.0.pop();
    }

    fn declare(&mut self, name: String, binding: Binding) {
        self.0
            .last_mut()
            .expect("ownership scope")
            .insert(name, binding);
    }

    fn get(&self, name: &str) -> Option<&Binding> {
        self.0.iter().rev().find_map(|scope| scope.get(name))
    }

    fn get_mut(&mut self, name: &str) -> Option<&mut Binding> {
        self.0
            .iter_mut()
            .rev()
            .find_map(|scope| scope.get_mut(name))
    }

    fn merge_from(&mut self, left: &Self, right: &Self) {
        for (index, scope) in self.0.iter_mut().enumerate() {
            for (name, binding) in scope {
                let Some(left_state) = left.0.get(index).and_then(|scope| scope.get(name)) else {
                    continue;
                };
                let Some(right_state) = right.0.get(index).and_then(|scope| scope.get(name)) else {
                    continue;
                };
                binding.state = join_state(&left_state.state, &right_state.state);
            }
        }
    }
}

fn join_state(left: &State, right: &State) -> State {
    match (left, right) {
        (State::Borrowed, State::Borrowed) => State::Borrowed,
        (State::Owned, State::Owned) => State::Owned,
        (State::Given { at: left }, State::Given { at: right }) if left == right => {
            State::Given { at: *left }
        }
        (State::Given { at }, State::Given { .. })
        | (State::MaybeGiven { at }, _)
        | (_, State::MaybeGiven { at })
        | (State::Owned, State::Given { at })
        | (State::Given { at }, State::Owned) => State::MaybeGiven { at: *at },
        (State::Borrowed, _) | (_, State::Borrowed) => {
            unreachable!("a binding cannot change between borrowed and owned")
        }
    }
}

pub fn check_program(program: &ast::Program) -> Vec<Diagnostic> {
    let classes = program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Class(class) => Some(class.name.clone()),
            _ => None,
        })
        .collect::<HashSet<_>>();
    let mut signatures = HashMap::new();
    let mut constructors = HashMap::new();
    let mut methods = HashMap::new();
    let mut properties = HashMap::new();

    for item in &program.items {
        match item {
            Item::Function(function) => {
                signatures.insert(function.name.clone(), signature(function, &classes));
            }
            Item::Class(class) => {
                for member in &class.members {
                    match member {
                        ClassMember::Property(property) => {
                            if classes.contains(&property.ty.name) {
                                properties.insert(
                                    (class.name.clone(), property.name.clone()),
                                    property.ty.name.clone(),
                                );
                            }
                        }
                        ClassMember::Method(method) => {
                            let method_signature = signature(method, &classes);
                            methods.insert(
                                (class.name.clone(), method.name.clone()),
                                method_signature.clone(),
                            );
                            if method.name == "__construct" {
                                constructors.insert(class.name.clone(), method_signature);
                                for param in &method.params {
                                    if param.promoted_access.is_some()
                                        && classes.contains(&param.ty.name)
                                    {
                                        properties.insert(
                                            (class.name.clone(), param.name.clone()),
                                            param.ty.name.clone(),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Item::Statement(_) => {}
        }
    }

    let mut checker = Checker {
        classes,
        signatures,
        constructors,
        methods,
        properties,
        receiver_class: None,
        diagnostics: Vec::new(),
    };
    let mut top_level_scopes = Scopes::new();
    for item in &program.items {
        match item {
            Item::Function(function) => checker.check_function(function, None),
            Item::Class(class) => {
                for member in &class.members {
                    match member {
                        ClassMember::Property(property) => {
                            if let Some(initializer) = &property.initializer {
                                let mut scopes = Scopes::new();
                                checker.use_expr(initializer, &mut scopes, UseMode::Give);
                            }
                        }
                        ClassMember::Method(method) => {
                            checker.check_function(method, Some(&class.name))
                        }
                    }
                }
            }
            Item::Statement(statement) => {
                checker.check_statement(statement, &mut top_level_scopes, None);
            }
        }
    }
    checker.diagnostics
}

fn signature(function: &ast::FunctionDecl, classes: &HashSet<String>) -> Signature {
    Signature {
        params: function
            .params
            .iter()
            .map(|param| Parameter {
                class: classes
                    .contains(&param.ty.name)
                    .then(|| param.ty.name.clone()),
                take: param.take,
            })
            .collect(),
        returns: function
            .return_type
            .as_ref()
            .filter(|ty| classes.contains(&ty.name))
            .map(|ty| ty.name.clone()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UseMode {
    Borrow,
    Give,
}

#[derive(Debug, Clone)]
struct Flow {
    falls_through: bool,
    backedges: Vec<Scopes>,
}

impl Flow {
    fn fallthrough() -> Self {
        Self {
            falls_through: true,
            backedges: Vec::new(),
        }
    }

    fn stops() -> Self {
        Self {
            falls_through: false,
            backedges: Vec::new(),
        }
    }
}

struct Checker {
    classes: HashSet<String>,
    signatures: HashMap<String, Signature>,
    constructors: HashMap<String, Signature>,
    methods: HashMap<(String, String), Signature>,
    properties: HashMap<(String, String), String>,
    receiver_class: Option<String>,
    diagnostics: Vec<Diagnostic>,
}

impl Checker {
    fn check_function(&mut self, function: &ast::FunctionDecl, receiver_class: Option<&str>) {
        let previous_receiver =
            std::mem::replace(&mut self.receiver_class, receiver_class.map(str::to_owned));
        let mut scopes = Scopes::new();
        for param in &function.params {
            if self.classes.contains(&param.ty.name) {
                scopes.declare(
                    param.name.clone(),
                    Binding {
                        class: param.ty.name.clone(),
                        writable: param.writable,
                        state: if param.take {
                            State::Owned
                        } else {
                            State::Borrowed
                        },
                    },
                );
            }
        }
        let return_class = function
            .return_type
            .as_ref()
            .filter(|ty| self.classes.contains(&ty.name))
            .map(|ty| ty.name.as_str());
        self.check_block(&function.body, &mut scopes, return_class, false);
        self.receiver_class = previous_receiver;
    }

    fn check_block(
        &mut self,
        block: &ast::Block,
        scopes: &mut Scopes,
        return_class: Option<&str>,
        nested: bool,
    ) -> Flow {
        if nested {
            scopes.push();
        }
        let mut flow = Flow::fallthrough();
        for statement in &block.statements {
            if !flow.falls_through {
                break;
            }
            let statement_flow = self.check_statement(statement, scopes, return_class);
            flow.falls_through = statement_flow.falls_through;
            flow.backedges.extend(statement_flow.backedges);
        }
        if nested {
            scopes.pop();
            for backedge in &mut flow.backedges {
                backedge.pop();
            }
        }
        flow
    }

    fn check_statement(
        &mut self,
        statement: &Stmt,
        scopes: &mut Scopes,
        return_class: Option<&str>,
    ) -> Flow {
        match statement {
            Stmt::VarDecl(decl) => {
                let class = decl
                    .ty
                    .as_ref()
                    .filter(|ty| self.classes.contains(&ty.name))
                    .map(|ty| ty.name.clone())
                    .or_else(|| self.expr_class(&decl.initializer, scopes));
                self.use_expr(
                    &decl.initializer,
                    scopes,
                    if class.is_some() {
                        UseMode::Give
                    } else {
                        UseMode::Borrow
                    },
                );
                if let Some(class) = class {
                    scopes.declare(
                        decl.name.clone(),
                        Binding {
                            class,
                            writable: decl.writable,
                            state: State::Owned,
                        },
                    );
                }
                Flow::fallthrough()
            }
            Stmt::Assignment(assignment) => {
                if assignment.op != AssignOp::Assign {
                    self.use_expr(&assignment.target, scopes, UseMode::Borrow);
                    self.use_expr(&assignment.value, scopes, UseMode::Borrow);
                    return Flow::fallthrough();
                }
                if let Expr::Variable { name, span } = &assignment.target {
                    let value_class = self.expr_class(&assignment.value, scopes);
                    let target_class = scopes.get(name).map(|binding| binding.class.clone());
                    if value_class.is_some() && value_class == target_class {
                        if variable_name(&assignment.value).is_some_and(|source| source == name) {
                            self.diagnostics.push(
                                Diagnostic::new(
                                    "E0471",
                                    format!("`${name}` cannot be given to itself"),
                                    assignment.span,
                                )
                                .with_help("give the value to a different owning destination"),
                            );
                            return Flow::fallthrough();
                        }
                        let was_owned = scopes
                            .get(name)
                            .is_some_and(|binding| binding.state == State::Owned);
                        let writable = scopes.get(name).is_some_and(|binding| binding.writable);
                        if !was_owned && !writable {
                            self.diagnostics.push(
                                Diagnostic::new(
                                    "E0473",
                                    format!("readonly `${name}` cannot be assigned a new owner"),
                                    *span,
                                )
                                .with_help(
                                    "declare the binding `writable` if it must be reinitialized after giving its value away",
                                ),
                            );
                        }
                        self.use_expr(&assignment.value, scopes, UseMode::Give);
                        if let Some(binding) = scopes.get_mut(name) {
                            binding.state = State::Owned;
                        }
                    } else {
                        self.use_expr(&assignment.value, scopes, UseMode::Borrow);
                    }
                } else {
                    if self.expr_class(&assignment.value, scopes).is_some() {
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0472",
                                "direct moves into owned properties are not supported",
                                assignment.span,
                            )
                            .with_help(
                                "keep the owned class value in a local until writable-path move rules are specified",
                            ),
                        );
                    }
                    self.use_expr(&assignment.target, scopes, UseMode::Borrow);
                    self.use_expr(&assignment.value, scopes, UseMode::Borrow);
                }
                Flow::fallthrough()
            }
            Stmt::Echo { expr, .. } | Stmt::Expr { expr, .. } => {
                self.use_expr(expr, scopes, UseMode::Borrow);
                Flow::fallthrough()
            }
            Stmt::Return { expr, .. } => {
                if let Some(expr) = expr {
                    self.use_expr(
                        expr,
                        scopes,
                        if return_class.is_some() {
                            UseMode::Give
                        } else {
                            UseMode::Borrow
                        },
                    );
                }
                Flow::stops()
            }
            Stmt::If(statement) => {
                self.use_expr(&statement.condition, scopes, UseMode::Borrow);
                let before = scopes.clone();
                let mut then_scopes = before.clone();
                let mut then_flow =
                    self.check_block(&statement.then_block, &mut then_scopes, return_class, true);
                let mut else_scopes = before.clone();
                let mut else_flow = if let Some(branch) = &statement.else_branch {
                    match branch {
                        ast::ElseBranch::If(nested) => self.check_statement(
                            &Stmt::If((**nested).clone()),
                            &mut else_scopes,
                            return_class,
                        ),
                        ast::ElseBranch::Block(block) => {
                            self.check_block(block, &mut else_scopes, return_class, true)
                        }
                    }
                } else {
                    Flow::fallthrough()
                };
                match (then_flow.falls_through, else_flow.falls_through) {
                    (true, true) => scopes.merge_from(&then_scopes, &else_scopes),
                    (true, false) => *scopes = then_scopes,
                    (false, true) => *scopes = else_scopes,
                    (false, false) => *scopes = before,
                }
                then_flow.backedges.append(&mut else_flow.backedges);
                Flow {
                    falls_through: then_flow.falls_through || else_flow.falls_through,
                    backedges: then_flow.backedges,
                }
            }
            Stmt::While(statement) => {
                self.use_expr(&statement.condition, scopes, UseMode::Borrow);
                let before = scopes.clone();
                let mut body = before.clone();
                let mut body_flow =
                    self.check_block(&statement.body, &mut body, return_class, true);
                if body_flow.falls_through {
                    body_flow.backedges.push(body);
                }
                self.check_second_iteration(&statement.body, &body_flow.backedges, return_class);
                merge_loop_exit(scopes, &before, &body_flow.backedges);
                Flow::fallthrough()
            }
            Stmt::For(statement) => {
                scopes.push();
                if let Some(initializer) = &statement.initializer {
                    match initializer {
                        ast::ForInitializer::VarDecl(decl) => {
                            let _ = self.check_statement(
                                &Stmt::VarDecl(decl.clone()),
                                scopes,
                                return_class,
                            );
                        }
                        ast::ForInitializer::Assignment(assignment) => {
                            let _ = self.check_statement(
                                &Stmt::Assignment(assignment.clone()),
                                scopes,
                                return_class,
                            );
                        }
                    }
                }
                if let Some(condition) = &statement.condition {
                    self.use_expr(condition, scopes, UseMode::Borrow);
                }
                let before = scopes.clone();
                let mut body = before.clone();
                let mut body_flow =
                    self.check_block(&statement.body, &mut body, return_class, true);
                if body_flow.falls_through {
                    body_flow.backedges.push(body);
                }
                for repeat in &mut body_flow.backedges {
                    if let Some(increment) = &statement.increment {
                        match increment {
                            ast::ForIncrement::Assignment(assignment) => {
                                let _ = self.check_statement(
                                    &Stmt::Assignment(assignment.clone()),
                                    repeat,
                                    return_class,
                                );
                            }
                            ast::ForIncrement::Increment(increment) => {
                                self.use_expr(&increment.target, repeat, UseMode::Borrow)
                            }
                        }
                    }
                }
                self.check_second_iteration(&statement.body, &body_flow.backedges, return_class);
                merge_loop_exit(scopes, &before, &body_flow.backedges);
                scopes.pop();
                Flow::fallthrough()
            }
            Stmt::Foreach(statement) => {
                self.use_expr(&statement.iterable, scopes, UseMode::Borrow);
                let before = scopes.clone();
                let mut body = before.clone();
                let mut body_flow =
                    self.check_block(&statement.body, &mut body, return_class, true);
                if body_flow.falls_through {
                    body_flow.backedges.push(body);
                }
                self.check_second_iteration(&statement.body, &body_flow.backedges, return_class);
                merge_loop_exit(scopes, &before, &body_flow.backedges);
                Flow::fallthrough()
            }
            Stmt::Increment(increment) => {
                self.use_expr(&increment.target, scopes, UseMode::Borrow);
                Flow::fallthrough()
            }
            Stmt::Break { .. } => Flow::stops(),
            Stmt::Continue { .. } => Flow {
                falls_through: false,
                backedges: vec![scopes.clone()],
            },
        }
    }

    fn check_second_iteration(
        &mut self,
        body: &ast::Block,
        backedges: &[Scopes],
        return_class: Option<&str>,
    ) {
        for backedge in backedges {
            let diagnostics_before = self.diagnostics.len();
            let mut second_iteration = backedge.clone();
            let _ = self.check_block(body, &mut second_iteration, return_class, true);
            self.deduplicate_diagnostics_from(diagnostics_before);
        }
    }

    fn deduplicate_diagnostics_from(&mut self, start: usize) {
        let mut additions = self.diagnostics.split_off(start);
        additions.retain(|candidate| {
            !self.diagnostics.iter().any(|existing| {
                existing.code == candidate.code
                    && existing.message == candidate.message
                    && existing.span == candidate.span
            })
        });
        self.diagnostics.extend(additions);
    }

    fn use_expr(&mut self, expr: &Expr, scopes: &mut Scopes, mode: UseMode) {
        match expr {
            Expr::Variable { name, span } => {
                let Some(binding) = scopes.get_mut(name) else {
                    return;
                };
                match binding.state {
                    State::Borrowed if mode == UseMode::Give => {
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0474",
                                format!("borrowed `${name}` cannot be given away"),
                                *span,
                            )
                            .with_help(
                                "declare the parameter with `take` if this function must receive ownership",
                            ),
                        );
                    }
                    State::Borrowed => {}
                    State::Owned if mode == UseMode::Give => {
                        binding.state = State::Given { at: *span };
                    }
                    State::Owned => {}
                    State::Given { at } | State::MaybeGiven { at } => {
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0470",
                                format!("`${name}` is still being used after its value was given away"),
                                *span,
                            )
                            .with_help(format!(
                                "the value was given away at bytes {}..{} and cannot be used afterward",
                                at.start, at.end
                            )),
                        );
                    }
                }
            }
            Expr::Grouped { expr, .. } => self.use_expr(expr, scopes, mode),
            Expr::PropertyAccess { object, span, .. } => {
                if mode == UseMode::Give && self.expr_class(expr, scopes).is_some() {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0472",
                            "direct moves out of owned properties are not supported",
                            *span,
                        )
                        .with_help(
                            "use the property without transferring it until writable-path move rules are specified",
                        ),
                    );
                }
                self.use_expr(object, scopes, UseMode::Borrow);
            }
            Expr::FunctionCall { name, args, .. } => {
                let signature = self.signatures.get(name).cloned().unwrap_or_default();
                self.use_call_args(args, &signature, scopes);
            }
            Expr::New {
                class_name, args, ..
            } => {
                let signature = self
                    .constructors
                    .get(class_name)
                    .cloned()
                    .unwrap_or_default();
                self.use_call_args(args, &signature, scopes);
            }
            Expr::MethodCall {
                object,
                method,
                args,
                ..
            } => {
                let signature = self
                    .expr_class(object, scopes)
                    .and_then(|class| self.methods.get(&(class, method.clone())).cloned())
                    .unwrap_or_default();
                self.use_expr(object, scopes, UseMode::Borrow);
                self.use_call_args(args, &signature, scopes);
            }
            Expr::StaticCall {
                class_name,
                method,
                args,
                ..
            } => {
                let signature = self
                    .methods
                    .get(&(class_name.clone(), method.clone()))
                    .cloned()
                    .unwrap_or_default();
                self.use_call_args(args, &signature, scopes);
            }
            Expr::InterpolatedString { parts, .. } => {
                for part in parts {
                    if let ast::InterpolatedStringPart::Expr(expr) = part {
                        self.use_expr(expr, scopes, UseMode::Borrow);
                    }
                }
            }
            Expr::Array { elements, .. } => {
                for element in elements {
                    if let Some(key) = &element.key {
                        self.use_expr(key, scopes, UseMode::Borrow);
                    }
                    self.use_expr(&element.value, scopes, UseMode::Borrow);
                }
            }
            Expr::Unary { expr, .. } => self.use_expr(expr, scopes, UseMode::Borrow),
            Expr::Binary { left, right, .. }
            | Expr::Range {
                start: left,
                end: right,
                ..
            } => {
                self.use_expr(left, scopes, UseMode::Borrow);
                self.use_expr(right, scopes, UseMode::Borrow);
            }
            Expr::This { span } => {
                if mode == UseMode::Give {
                    self.diagnostics.push(
                        Diagnostic::new("E0474", "borrowed `$this` cannot be given away", *span)
                            .with_help(
                                "the method receiver is borrowed from its caller and must remain owned by that caller",
                            ),
                    );
                }
            }
            Expr::Identifier { .. }
            | Expr::String { .. }
            | Expr::Int { .. }
            | Expr::Float { .. }
            | Expr::Bool { .. }
            | Expr::Null { .. } => {}
        }
    }

    fn use_call_args(&mut self, args: &[Expr], signature: &Signature, scopes: &mut Scopes) {
        for (index, arg) in args.iter().enumerate() {
            let mode = signature
                .params
                .get(index)
                .filter(|param| param.take && param.class.is_some())
                .map_or(UseMode::Borrow, |_| UseMode::Give);
            self.use_expr(arg, scopes, mode);
        }
    }

    fn expr_class(&self, expr: &Expr, scopes: &Scopes) -> Option<String> {
        match expr {
            Expr::Variable { name, .. } => scopes.get(name).map(|binding| binding.class.clone()),
            Expr::New { class_name, .. } if self.classes.contains(class_name) => {
                Some(class_name.clone())
            }
            Expr::FunctionCall { name, .. } => self
                .signatures
                .get(name)
                .and_then(|signature| signature.returns.clone()),
            Expr::PropertyAccess {
                object, property, ..
            } => {
                let object_class = self.expr_class(object, scopes)?;
                self.properties
                    .get(&(object_class, property.clone()))
                    .cloned()
            }
            Expr::MethodCall { object, method, .. } => {
                let object_class = self.expr_class(object, scopes)?;
                self.methods
                    .get(&(object_class, method.clone()))
                    .and_then(|signature| signature.returns.clone())
            }
            Expr::StaticCall {
                class_name, method, ..
            } => self
                .methods
                .get(&(class_name.clone(), method.clone()))
                .and_then(|signature| signature.returns.clone()),
            Expr::This { .. } => self.receiver_class.clone(),
            Expr::Grouped { expr, .. } => self.expr_class(expr, scopes),
            _ => None,
        }
    }
}

fn variable_name(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::Variable { name, .. } => Some(name),
        Expr::Grouped { expr, .. } => variable_name(expr),
        _ => None,
    }
}

fn merge_loop_exit(scopes: &mut Scopes, before: &Scopes, backedges: &[Scopes]) {
    let Some((first, rest)) = backedges.split_first() else {
        *scopes = before.clone();
        return;
    };
    let mut repeated = first.clone();
    for state in rest {
        let left = repeated.clone();
        repeated.merge_from(&left, state);
    }
    scopes.merge_from(before, &repeated);
}

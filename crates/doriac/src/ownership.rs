//! Stage 19 ownership checking for class move values.
//!
//! This pass is intentionally backend-independent. It runs after ordinary
//! semantic/type checking and records errors in source vocabulary before MIR
//! lowering or either native backend can observe an invalid ownership graph.

use std::collections::{HashMap, HashSet};

use crate::ast::{self, AssignOp, BinaryOp, ClassMember, Expr, Item, Stmt};
use crate::diagnostics::Diagnostic;
use crate::source::Span;

#[derive(Debug, Clone)]
struct Parameter {
    move_type: bool,
    take: bool,
}

#[derive(Debug, Clone, Default)]
struct Signature {
    params: Vec<Parameter>,
    returns: Option<String>,
    returns_move_type: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum State {
    Borrowed,
    BorrowedOrOwned,
    Owned,
    Given { at: Span },
    MaybeGiven { at: Span },
}

#[derive(Debug, Clone)]
struct Binding {
    class: Option<String>,
    mixed: bool,
    borrowed_place: bool,
    writable: bool,
    state: State,
}

#[derive(Debug, Clone)]
struct PropertyInfo {
    class: Option<String>,
    move_type: bool,
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
        (State::BorrowedOrOwned, State::Borrowed)
        | (State::Borrowed, State::BorrowedOrOwned)
        | (State::BorrowedOrOwned, State::BorrowedOrOwned)
        | (State::BorrowedOrOwned, State::Owned)
        | (State::Owned, State::BorrowedOrOwned)
        | (State::Borrowed, State::Owned)
        | (State::Owned, State::Borrowed) => State::BorrowedOrOwned,
        (State::Owned, State::Owned) => State::Owned,
        (State::Given { at: left }, State::Given { at: right }) if left == right => {
            State::Given { at: *left }
        }
        (State::Given { at }, State::Given { .. })
        | (State::MaybeGiven { at }, _)
        | (_, State::MaybeGiven { at })
        | (State::Owned, State::Given { at })
        | (State::Given { at }, State::Owned)
        | (State::Borrowed, State::Given { at })
        | (State::Given { at }, State::Borrowed)
        | (State::BorrowedOrOwned, State::Given { at })
        | (State::Given { at }, State::BorrowedOrOwned) => State::MaybeGiven { at: *at },
    }
}

pub fn check_program(program: &ast::Program) -> Vec<Diagnostic> {
    check_program_with_inferred_move_returns(program, &HashSet::new())
}

pub(crate) fn check_program_with_inferred_move_returns(
    program: &ast::Program,
    inferred_move_returns: &HashSet<usize>,
) -> Vec<Diagnostic> {
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
                signatures.insert(
                    function.name.clone(),
                    signature(function, &classes, inferred_move_returns),
                );
            }
            Item::Class(class) => {
                for member in &class.members {
                    match member {
                        ClassMember::Property(property) => {
                            let property_class = classes
                                .contains(&property.ty.name)
                                .then(|| property.ty.name.clone());
                            let move_type = type_ref_is_move_type(&property.ty, &classes);
                            if move_type {
                                properties.insert(
                                    (class.name.clone(), property.name.clone()),
                                    PropertyInfo {
                                        class: property_class,
                                        move_type,
                                    },
                                );
                            }
                        }
                        ClassMember::Method(method) => {
                            let method_signature =
                                signature(method, &classes, inferred_move_returns);
                            methods.insert(
                                (class.name.clone(), method.name.clone()),
                                method_signature.clone(),
                            );
                            if method.name == "__construct" {
                                constructors.insert(class.name.clone(), method_signature);
                                for param in &method.params {
                                    let property_class = classes
                                        .contains(&param.ty.name)
                                        .then(|| param.ty.name.clone());
                                    let move_type = type_ref_is_move_type(&param.ty, &classes);
                                    if param.promoted_access.is_some() && move_type {
                                        properties.insert(
                                            (class.name.clone(), param.name.clone()),
                                            PropertyInfo {
                                                class: property_class,
                                                move_type,
                                            },
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
        inferred_move_returns: inferred_move_returns.clone(),
        receiver_class: None,
        diagnostics: Vec::new(),
    };
    let mut top_level_scopes = Scopes::new();
    let mut top_level_falls_through = true;
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
                if top_level_falls_through {
                    top_level_falls_through = checker
                        .check_statement(statement, &mut top_level_scopes, false)
                        .falls_through;
                }
            }
        }
    }
    checker.diagnostics
}

fn signature(
    function: &ast::FunctionDecl,
    classes: &HashSet<String>,
    inferred_move_returns: &HashSet<usize>,
) -> Signature {
    Signature {
        params: function
            .params
            .iter()
            .map(|param| Parameter {
                move_type: type_ref_is_move_type(&param.ty, classes),
                take: param.take,
            })
            .collect(),
        returns: function
            .return_type
            .as_ref()
            .filter(|ty| classes.contains(&ty.name))
            .map(|ty| ty.name.clone()),
        returns_move_type: function
            .return_type
            .as_ref()
            .is_some_and(|ty| type_ref_is_move_type(ty, classes))
            || inferred_move_returns.contains(&function.span.start),
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
    breaks: Vec<Scopes>,
}

impl Flow {
    fn fallthrough() -> Self {
        Self {
            falls_through: true,
            backedges: Vec::new(),
            breaks: Vec::new(),
        }
    }

    fn stops() -> Self {
        Self {
            falls_through: false,
            backedges: Vec::new(),
            breaks: Vec::new(),
        }
    }

    fn breaks(scopes: &Scopes) -> Self {
        Self {
            falls_through: false,
            backedges: Vec::new(),
            breaks: vec![scopes.clone()],
        }
    }
}

struct Checker {
    classes: HashSet<String>,
    signatures: HashMap<String, Signature>,
    constructors: HashMap<String, Signature>,
    methods: HashMap<(String, String), Signature>,
    properties: HashMap<(String, String), PropertyInfo>,
    inferred_move_returns: HashSet<usize>,
    receiver_class: Option<String>,
    diagnostics: Vec<Diagnostic>,
}

impl Checker {
    fn check_function(&mut self, function: &ast::FunctionDecl, receiver_class: Option<&str>) {
        let previous_receiver =
            std::mem::replace(&mut self.receiver_class, receiver_class.map(str::to_owned));
        let mut scopes = Scopes::new();
        for param in &function.params {
            let class = self
                .classes
                .contains(&param.ty.name)
                .then(|| param.ty.name.clone());
            let mixed = param.ty.name == "mixed";
            if type_ref_is_move_type(&param.ty, &self.classes) {
                scopes.declare(
                    param.name.clone(),
                    Binding {
                        class,
                        mixed,
                        borrowed_place: !param.take,
                        writable: param.writable,
                        state: if param.take && param.promoted_access.is_some() {
                            State::Given { at: param.span }
                        } else if param.take {
                            State::Owned
                        } else {
                            State::Borrowed
                        },
                    },
                );
            }
        }
        let return_move_type = function
            .return_type
            .as_ref()
            .is_some_and(|ty| type_ref_is_move_type(ty, &self.classes))
            || self.inferred_move_returns.contains(&function.span.start);
        self.check_block(&function.body, &mut scopes, return_move_type, false);
        self.receiver_class = previous_receiver;
    }

    fn check_block(
        &mut self,
        block: &ast::Block,
        scopes: &mut Scopes,
        return_move_type: bool,
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
            let statement_flow = self.check_statement(statement, scopes, return_move_type);
            flow.falls_through = statement_flow.falls_through;
            flow.backedges.extend(statement_flow.backedges);
            flow.breaks.extend(statement_flow.breaks);
        }
        if nested {
            scopes.pop();
            for backedge in &mut flow.backedges {
                backedge.pop();
            }
            for break_exit in &mut flow.breaks {
                break_exit.pop();
            }
        }
        flow
    }

    fn check_statement(
        &mut self,
        statement: &Stmt,
        scopes: &mut Scopes,
        return_move_type: bool,
    ) -> Flow {
        match statement {
            Stmt::VarDecl(decl) => {
                let declared_class = decl
                    .ty
                    .as_ref()
                    .filter(|ty| self.classes.contains(&ty.name))
                    .map(|ty| ty.name.clone());
                let class = declared_class.or_else(|| self.expr_class(&decl.initializer, scopes));
                let initializer_moves = self.expr_is_move_value(&decl.initializer, scopes);
                let mixed = decl.ty.as_ref().is_some_and(|ty| ty.name == "mixed")
                    || (decl.ty.is_none() && class.is_none() && initializer_moves);
                let declared_move_type = decl
                    .ty
                    .as_ref()
                    .is_some_and(|ty| type_ref_is_move_type(ty, &self.classes));
                self.use_expr(
                    &decl.initializer,
                    scopes,
                    if initializer_moves || class.is_some() || mixed || declared_move_type {
                        UseMode::Give
                    } else {
                        UseMode::Borrow
                    },
                );
                if class.is_some() || mixed || declared_move_type {
                    scopes.declare(
                        decl.name.clone(),
                        Binding {
                            class,
                            mixed,
                            borrowed_place: false,
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
                    let value_moves = self.expr_is_move_value(&assignment.value, scopes);
                    let target = scopes.get(name).cloned();
                    let class_assignment = value_class.is_some()
                        && target
                            .as_ref()
                            .is_some_and(|binding| binding.mixed || binding.class == value_class);
                    let mixed_assignment = target.as_ref().is_some_and(|binding| binding.mixed);
                    let move_assignment = target.is_some() && value_moves;
                    if class_assignment || mixed_assignment || move_assignment {
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
                        self.use_expr(
                            &assignment.value,
                            scopes,
                            if value_moves || class_assignment {
                                UseMode::Give
                            } else {
                                UseMode::Borrow
                            },
                        );
                        if let Some(binding) = scopes.get_mut(name) {
                            binding.state = if binding.borrowed_place {
                                State::Borrowed
                            } else {
                                State::Owned
                            };
                            if binding.mixed {
                                binding.class = value_class;
                            }
                        }
                    } else {
                        self.use_expr(&assignment.value, scopes, UseMode::Borrow);
                    }
                } else {
                    if self.expr_is_move_value(&assignment.value, scopes) {
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
            Stmt::Echo { expr, .. } => {
                self.use_expr(expr, scopes, UseMode::Borrow);
                Flow::fallthrough()
            }
            Stmt::Expr { expr, .. } => {
                self.use_expr(expr, scopes, UseMode::Borrow);
                if is_panic_expr(expr) {
                    Flow::stops()
                } else {
                    Flow::fallthrough()
                }
            }
            Stmt::Return { expr, .. } => {
                if let Some(expr) = expr {
                    self.use_expr(
                        expr,
                        scopes,
                        if return_move_type {
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
                if let Some(condition) = constant_bool(&statement.condition) {
                    if condition {
                        return self.check_block(
                            &statement.then_block,
                            scopes,
                            return_move_type,
                            true,
                        );
                    }
                    return if let Some(branch) = &statement.else_branch {
                        match branch {
                            ast::ElseBranch::If(nested) => self.check_statement(
                                &Stmt::If((**nested).clone()),
                                scopes,
                                return_move_type,
                            ),
                            ast::ElseBranch::Block(block) => {
                                self.check_block(block, scopes, return_move_type, true)
                            }
                        }
                    } else {
                        Flow::fallthrough()
                    };
                }
                let before = scopes.clone();
                let mut then_scopes = before.clone();
                let mut then_flow = self.check_block(
                    &statement.then_block,
                    &mut then_scopes,
                    return_move_type,
                    true,
                );
                let mut else_scopes = before.clone();
                let mut else_flow = if let Some(branch) = &statement.else_branch {
                    match branch {
                        ast::ElseBranch::If(nested) => self.check_statement(
                            &Stmt::If((**nested).clone()),
                            &mut else_scopes,
                            return_move_type,
                        ),
                        ast::ElseBranch::Block(block) => {
                            self.check_block(block, &mut else_scopes, return_move_type, true)
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
                then_flow.breaks.append(&mut else_flow.breaks);
                Flow {
                    falls_through: then_flow.falls_through || else_flow.falls_through,
                    backedges: then_flow.backedges,
                    breaks: then_flow.breaks,
                }
            }
            Stmt::While(statement) => {
                self.use_expr(&statement.condition, scopes, UseMode::Borrow);
                if constant_bool(&statement.condition) == Some(false) {
                    return Flow::fallthrough();
                }
                let before = scopes.clone();
                let mut body = before.clone();
                let mut body_flow =
                    self.check_block(&statement.body, &mut body, return_move_type, true);
                if body_flow.falls_through {
                    body_flow.backedges.push(body);
                }
                for repeat in &mut body_flow.backedges {
                    self.use_expr(&statement.condition, repeat, UseMode::Borrow);
                }
                self.check_second_iteration(
                    &statement.body,
                    &body_flow.backedges,
                    return_move_type,
                );
                let mut exits = body_flow.backedges;
                exits.extend(body_flow.breaks);
                merge_loop_exit(scopes, &before, &exits);
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
                                return_move_type,
                            );
                        }
                        ast::ForInitializer::Assignment(assignment) => {
                            let _ = self.check_statement(
                                &Stmt::Assignment(assignment.clone()),
                                scopes,
                                return_move_type,
                            );
                        }
                    }
                }
                if let Some(condition) = &statement.condition {
                    self.use_expr(condition, scopes, UseMode::Borrow);
                    if constant_bool(condition) == Some(false) {
                        scopes.pop();
                        return Flow::fallthrough();
                    }
                }
                let before = scopes.clone();
                let mut body = before.clone();
                let mut body_flow =
                    self.check_block(&statement.body, &mut body, return_move_type, true);
                if body_flow.falls_through {
                    body_flow.backedges.push(body);
                }
                for repeat in &mut body_flow.backedges {
                    self.check_for_tail(statement, repeat, return_move_type);
                }
                self.check_for_second_iteration(statement, &body_flow.backedges, return_move_type);
                let mut exits = body_flow.backedges;
                exits.extend(body_flow.breaks);
                merge_loop_exit(scopes, &before, &exits);
                scopes.pop();
                Flow::fallthrough()
            }
            Stmt::Foreach(statement) => {
                self.use_expr(&statement.iterable, scopes, UseMode::Borrow);
                let before = scopes.clone();
                let mut body = before.clone();
                let mut body_flow =
                    self.check_foreach_iteration(statement, &mut body, return_move_type);
                if body_flow.falls_through {
                    body_flow.backedges.push(body);
                }
                self.check_foreach_second_iteration(
                    statement,
                    &body_flow.backedges,
                    return_move_type,
                );
                let mut exits = body_flow.backedges;
                exits.extend(body_flow.breaks);
                merge_loop_exit(scopes, &before, &exits);
                Flow::fallthrough()
            }
            Stmt::Increment(increment) => {
                self.use_expr(&increment.target, scopes, UseMode::Borrow);
                Flow::fallthrough()
            }
            Stmt::Break { .. } => Flow::breaks(scopes),
            Stmt::Continue { .. } => Flow {
                falls_through: false,
                backedges: vec![scopes.clone()],
                breaks: Vec::new(),
            },
        }
    }

    fn check_foreach_iteration(
        &mut self,
        statement: &ast::ForeachStmt,
        scopes: &mut Scopes,
        return_move_type: bool,
    ) -> Flow {
        scopes.push();
        if let Some(key) = &statement.key {
            self.declare_foreach_binding(key, scopes);
        }
        self.declare_foreach_binding(&statement.value, scopes);
        let mut flow = self.check_block(&statement.body, scopes, return_move_type, false);
        scopes.pop();
        for backedge in &mut flow.backedges {
            backedge.pop();
        }
        for break_exit in &mut flow.breaks {
            break_exit.pop();
        }
        flow
    }

    fn declare_foreach_binding(&self, binding: &ast::ForeachBinding, scopes: &mut Scopes) {
        let Some(ty) = &binding.ty else {
            return;
        };
        if !type_ref_is_move_type(ty, &self.classes) {
            return;
        }
        scopes.declare(
            binding.name.clone(),
            Binding {
                class: self.classes.contains(&ty.name).then(|| ty.name.clone()),
                mixed: ty.name == "mixed",
                borrowed_place: true,
                writable: false,
                state: State::Borrowed,
            },
        );
    }

    fn check_foreach_second_iteration(
        &mut self,
        statement: &ast::ForeachStmt,
        entries: &[Scopes],
        return_move_type: bool,
    ) {
        for entry in entries {
            let diagnostics_before = self.diagnostics.len();
            let mut second_iteration = entry.clone();
            let _ =
                self.check_foreach_iteration(statement, &mut second_iteration, return_move_type);
            self.deduplicate_diagnostics_from(diagnostics_before);
        }
    }

    fn check_second_iteration(
        &mut self,
        body: &ast::Block,
        backedges: &[Scopes],
        return_move_type: bool,
    ) {
        for backedge in backedges {
            let diagnostics_before = self.diagnostics.len();
            let mut second_iteration = backedge.clone();
            let _ = self.check_block(body, &mut second_iteration, return_move_type, true);
            self.deduplicate_diagnostics_from(diagnostics_before);
        }
    }

    fn check_for_second_iteration(
        &mut self,
        statement: &ast::ForStmt,
        entries: &[Scopes],
        return_move_type: bool,
    ) {
        for entry in entries {
            let diagnostics_before = self.diagnostics.len();
            let mut second_iteration = entry.clone();
            let mut flow = self.check_block(
                &statement.body,
                &mut second_iteration,
                return_move_type,
                true,
            );
            if flow.falls_through {
                flow.backedges.push(second_iteration);
            }
            for backedge in &mut flow.backedges {
                self.check_for_tail(statement, backedge, return_move_type);
            }
            self.deduplicate_diagnostics_from(diagnostics_before);
        }
    }

    fn check_for_tail(
        &mut self,
        statement: &ast::ForStmt,
        scopes: &mut Scopes,
        return_move_type: bool,
    ) {
        if let Some(increment) = &statement.increment {
            match increment {
                ast::ForIncrement::Assignment(assignment) => {
                    let _ = self.check_statement(
                        &Stmt::Assignment(assignment.clone()),
                        scopes,
                        return_move_type,
                    );
                }
                ast::ForIncrement::Increment(increment) => {
                    self.use_expr(&increment.target, scopes, UseMode::Borrow);
                }
            }
        }
        if let Some(condition) = &statement.condition {
            self.use_expr(condition, scopes, UseMode::Borrow);
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
                    State::BorrowedOrOwned if mode == UseMode::Give => {
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0474",
                                format!("`${name}` may still be borrowed and cannot be given away"),
                                *span,
                            )
                            .with_help(
                                "keep borrowed and owned values in separate bindings before transferring ownership",
                            ),
                        );
                    }
                    State::BorrowedOrOwned => {}
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
                if mode == UseMode::Give && self.expr_is_move_value(expr, scopes) {
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
                self.use_call_args(None, args, &signature, scopes);
            }
            Expr::New {
                class_name, args, ..
            } => {
                let signature = self
                    .constructors
                    .get(class_name)
                    .cloned()
                    .unwrap_or_default();
                self.use_call_args(None, args, &signature, scopes);
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
                self.use_call_args(Some(object), args, &signature, scopes);
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
                self.use_call_args(None, args, &signature, scopes);
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
                        self.use_owned_expression(key, scopes);
                    }
                    self.use_owned_expression(&element.value, scopes);
                }
            }
            Expr::Unary { expr, .. } => self.use_expr(expr, scopes, UseMode::Borrow),
            Expr::Binary {
                left,
                op: op @ (BinaryOp::And | BinaryOp::Or),
                right,
                ..
            } => {
                self.use_expr(left, scopes, UseMode::Borrow);
                match (op, constant_bool(left)) {
                    (BinaryOp::And, Some(false)) | (BinaryOp::Or, Some(true)) => {}
                    (BinaryOp::And, Some(true)) | (BinaryOp::Or, Some(false)) => {
                        self.use_expr(right, scopes, UseMode::Borrow);
                    }
                    _ => {
                        let without_right = scopes.clone();
                        let mut with_right = without_right.clone();
                        self.use_expr(right, &mut with_right, UseMode::Borrow);
                        scopes.merge_from(&without_right, &with_right);
                    }
                }
            }
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

    fn use_call_args(
        &mut self,
        receiver: Option<&Expr>,
        args: &[Expr],
        signature: &Signature,
        scopes: &mut Scopes,
    ) {
        let mut borrowed = HashSet::new();
        if let Some(name) = receiver.and_then(ownership_root_name) {
            if scopes.get(name).is_some() {
                borrowed.insert(name);
            }
        }
        for (index, arg) in args.iter().enumerate() {
            let mode = call_arg_mode(signature, index);
            if mode == UseMode::Borrow {
                if let Some(name) = ownership_root_name(arg) {
                    if scopes.get(name).is_some() {
                        borrowed.insert(name);
                    }
                }
            }
        }

        if let Some(receiver) = receiver {
            self.use_expr(receiver, scopes, UseMode::Borrow);
        }
        for (index, arg) in args.iter().enumerate() {
            let mode = call_arg_mode(signature, index);
            if mode == UseMode::Give {
                if let Some(name) = ownership_root_name(arg).filter(|name| borrowed.contains(name))
                {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0471",
                            format!("`${name}` cannot be borrowed and given away in the same call"),
                            arg.span(),
                        )
                        .with_help("pass distinct owners for borrowed and ownership-taking inputs"),
                    );
                }
            }
            self.use_expr(arg, scopes, mode);
        }
    }

    fn use_owned_expression(&mut self, expr: &Expr, scopes: &mut Scopes) {
        let mode = if self.expr_is_move_value(expr, scopes) {
            UseMode::Give
        } else {
            UseMode::Borrow
        };
        self.use_expr(expr, scopes, mode);
    }

    fn expr_is_move_value(&self, expr: &Expr, scopes: &Scopes) -> bool {
        match expr {
            Expr::Variable { name, .. } => scopes.get(name).is_some(),
            Expr::Grouped { expr, .. } => self.expr_is_move_value(expr, scopes),
            Expr::Array { .. } => true,
            Expr::New { class_name, .. } => self.classes.contains(class_name),
            Expr::FunctionCall { name, .. } => self
                .signatures
                .get(name)
                .is_some_and(|signature| signature.returns_move_type),
            Expr::MethodCall { object, method, .. } => {
                let Some(class) = self.expr_class(object, scopes) else {
                    return false;
                };
                self.methods
                    .get(&(class, method.clone()))
                    .is_some_and(|signature| signature.returns_move_type)
            }
            Expr::StaticCall {
                class_name, method, ..
            } => self
                .methods
                .get(&(class_name.clone(), method.clone()))
                .is_some_and(|signature| signature.returns_move_type),
            Expr::PropertyAccess {
                object, property, ..
            } => {
                let Some(class) = self.expr_class(object, scopes) else {
                    return false;
                };
                self.properties
                    .get(&(class, property.clone()))
                    .is_some_and(|property| property.move_type)
            }
            Expr::This { .. } => self.receiver_class.is_some(),
            _ => false,
        }
    }

    fn expr_class(&self, expr: &Expr, scopes: &Scopes) -> Option<String> {
        match expr {
            Expr::Variable { name, .. } => {
                scopes.get(name).and_then(|binding| binding.class.clone())
            }
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
                    .and_then(|property| property.class.clone())
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

fn ownership_root_name(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::Variable { name, .. } => Some(name),
        Expr::PropertyAccess { object, .. } | Expr::Grouped { expr: object, .. } => {
            ownership_root_name(object)
        }
        _ => None,
    }
}

fn type_ref_is_move_type(ty: &crate::types::TypeRef, classes: &HashSet<String>) -> bool {
    classes.contains(&ty.name)
        || matches!(
            ty.name.as_str(),
            "mixed" | "[]" | "List" | "Dictionary" | "Set"
        )
}

fn call_arg_mode(signature: &Signature, index: usize) -> UseMode {
    signature
        .params
        .get(index)
        .filter(|param| param.take && param.move_type)
        .map_or(UseMode::Borrow, |_| UseMode::Give)
}

fn constant_bool(expr: &Expr) -> Option<bool> {
    match expr {
        Expr::Bool { value, .. } => Some(*value),
        Expr::Grouped { expr, .. } => constant_bool(expr),
        Expr::Unary {
            op: ast::UnaryOp::Not,
            expr,
            ..
        } => constant_bool(expr).map(|value| !value),
        Expr::Binary {
            left,
            op: BinaryOp::And,
            right,
            ..
        } => match constant_bool(left) {
            Some(false) => Some(false),
            Some(true) => constant_bool(right),
            None if constant_bool(right) == Some(false) => Some(false),
            None => None,
        },
        Expr::Binary {
            left,
            op: BinaryOp::Or,
            right,
            ..
        } => match constant_bool(left) {
            Some(true) => Some(true),
            Some(false) => constant_bool(right),
            None if constant_bool(right) == Some(true) => Some(true),
            None => None,
        },
        Expr::Binary {
            left,
            op: BinaryOp::Xor,
            right,
            ..
        } => Some(constant_bool(left)? ^ constant_bool(right)?),
        _ => None,
    }
}

fn is_panic_expr(expr: &Expr) -> bool {
    match expr {
        Expr::FunctionCall { name, .. } => name == "panic",
        Expr::Grouped { expr, .. } => is_panic_expr(expr),
        _ => false,
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

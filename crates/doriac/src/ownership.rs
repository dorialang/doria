//! Stage 19 ownership checking for class move values.
//!
//! This pass is intentionally backend-independent. It runs after ordinary
//! semantic/type checking and records errors in source vocabulary before MIR
//! lowering or either native backend can observe an invalid ownership graph.

use std::collections::{HashMap, HashSet};

use crate::ast::{self, AssignOp, BinaryOp, ClassMember, Expr, Item, Stmt};
use crate::diagnostics::Diagnostic;
use crate::source::Span;
use crate::symbols::{BorrowSource, ReturnBorrow};

#[derive(Debug, Clone)]
struct Parameter {
    move_type: bool,
    class_type: bool,
    take: bool,
    writable: bool,
}

#[derive(Debug, Clone, Default)]
struct Signature {
    params: Vec<Parameter>,
    returns: Option<String>,
    returns_move_type: bool,
    return_borrow: Option<ReturnBorrow>,
    receiver: Option<UseMode>,
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
    writable: bool,
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
    check_program_with_inferred_move_returns(program, &HashSet::new(), &HashMap::new())
}

pub(crate) fn check_program_with_inferred_move_returns(
    program: &ast::Program,
    inferred_move_returns: &HashSet<usize>,
    return_borrows: &HashMap<usize, ReturnBorrow>,
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
    let mut static_properties = HashMap::new();

    for item in &program.items {
        match item {
            Item::Function(function) => {
                signatures.insert(
                    function.name.clone(),
                    signature(
                        function,
                        &classes,
                        inferred_move_returns,
                        return_borrows,
                        None,
                    ),
                );
            }
            Item::Class(class) => {
                for member in &class.members {
                    match member {
                        ClassMember::Property(property) if property.is_static => {
                            static_properties.insert(
                                (class.name.clone(), property.name.clone()),
                                property.writable,
                            );
                        }
                        ClassMember::Property(property) => {
                            let property_class =
                                type_ref_class_name(&property.ty, &classes, Some(&class.name));
                            let move_type =
                                type_ref_is_move_type(&property.ty, &classes, Some(&class.name));
                            properties.insert(
                                (class.name.clone(), property.name.clone()),
                                PropertyInfo {
                                    class: property_class,
                                    move_type,
                                    writable: property.writable,
                                },
                            );
                        }
                        ClassMember::Constant(_) => {}
                        ClassMember::Method(method) => {
                            let method_signature = signature(
                                method,
                                &classes,
                                inferred_move_returns,
                                return_borrows,
                                Some(&class.name),
                            );
                            methods.insert(
                                (class.name.clone(), method.name.clone()),
                                method_signature.clone(),
                            );
                            if method.name == "__construct" {
                                constructors.insert(class.name.clone(), method_signature);
                                for param in &method.params {
                                    let property_class =
                                        type_ref_class_name(&param.ty, &classes, Some(&class.name));
                                    let move_type = type_ref_is_move_type(
                                        &param.ty,
                                        &classes,
                                        Some(&class.name),
                                    );
                                    if param.promoted_access.is_some() {
                                        properties.insert(
                                            (class.name.clone(), param.name.clone()),
                                            PropertyInfo {
                                                class: property_class,
                                                move_type,
                                                writable: param.writable,
                                            },
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Item::Interface(_) | Item::Trait(_) | Item::Constant(_) | Item::Statement(_) => {}
        }
    }

    let mut checker = Checker {
        classes,
        signatures,
        constructors,
        methods,
        properties,
        static_properties,
        inferred_move_returns: inferred_move_returns.clone(),
        return_borrows: return_borrows.clone(),
        receiver_class: None,
        receiver_writable: false,
        current_return_borrow: None,
        active_assignment_writes: HashSet::new(),
        active_assignment_targets: HashSet::new(),
        active_borrows: Vec::new(),
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
                                let previous_receiver =
                                    checker.receiver_class.replace(class.name.clone());
                                let mut scopes = Scopes::new();
                                if type_ref_is_move_type(
                                    &property.ty,
                                    &checker.classes,
                                    Some(&class.name),
                                ) {
                                    checker.reject_borrowed_result(
                                        initializer,
                                        &scopes,
                                        "borrowed result cannot initialize an owning property",
                                        "initialize the property with an independently owned value",
                                    );
                                }
                                checker.use_expr(initializer, &mut scopes, UseMode::Give);
                                checker.receiver_class = previous_receiver;
                            }
                        }
                        ClassMember::Method(method) => {
                            checker.check_function(method, Some(&class.name))
                        }
                        ClassMember::Constant(_) => {}
                    }
                }
            }
            Item::Interface(_) | Item::Trait(_) | Item::Constant(_) => {}
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

pub(crate) fn function_return_borrow(function: &ast::FunctionDecl) -> Option<ReturnBorrow> {
    function_return_borrow_with_calls(function, &mut |_| None)
}

pub(crate) fn function_return_borrow_with_calls(
    function: &ast::FunctionDecl,
    resolve_call: &mut dyn FnMut(&Expr) -> Option<ReturnBorrow>,
) -> Option<ReturnBorrow> {
    let mut borrow = None;
    if block_return_borrow(
        &function.body,
        function,
        resolve_call,
        &HashSet::new(),
        &mut borrow,
    )
    .is_some()
    {
        borrow
    } else {
        None
    }
}

fn block_return_borrow(
    block: &ast::Block,
    function: &ast::FunctionDecl,
    resolve_call: &mut dyn FnMut(&Expr) -> Option<ReturnBorrow>,
    inherited_shadowed: &HashSet<String>,
    borrow: &mut Option<ReturnBorrow>,
) -> Option<bool> {
    let mut shadowed = inherited_shadowed.clone();
    let mut falls_through = true;
    for statement in &block.statements {
        if !falls_through {
            break;
        }
        falls_through =
            statement_return_borrow(statement, function, resolve_call, &mut shadowed, borrow)?;
    }
    Some(falls_through)
}

fn statement_return_borrow(
    statement: &Stmt,
    function: &ast::FunctionDecl,
    resolve_call: &mut dyn FnMut(&Expr) -> Option<ReturnBorrow>,
    shadowed: &mut HashSet<String>,
    borrow: &mut Option<ReturnBorrow>,
) -> Option<bool> {
    match statement {
        Stmt::Return {
            expr: Some(expr), ..
        } => {
            let candidate = expr_return_borrow(expr, function, resolve_call, shadowed)?;
            match borrow {
                Some(existing) if existing.source != candidate.source => None,
                Some(existing) => {
                    existing.writable &= candidate.writable;
                    Some(false)
                }
                slot @ None => {
                    *slot = Some(candidate);
                    Some(false)
                }
            }
        }
        Stmt::Return { expr: None, .. } => None,
        Stmt::If(statement) => match constant_bool(&statement.condition) {
            Some(true) => block_return_borrow(
                &statement.then_block,
                function,
                resolve_call,
                shadowed,
                borrow,
            ),
            Some(false) => statement.else_branch.as_ref().map_or(Some(true), |branch| {
                else_branch_return_borrow(branch, function, resolve_call, shadowed, borrow)
            }),
            None => {
                let then_falls = block_return_borrow(
                    &statement.then_block,
                    function,
                    resolve_call,
                    shadowed,
                    borrow,
                )?;
                let else_falls = statement
                    .else_branch
                    .as_ref()
                    .map_or(Some(true), |branch| {
                        else_branch_return_borrow(branch, function, resolve_call, shadowed, borrow)
                    })?;
                Some(then_falls || else_falls)
            }
        },
        Stmt::While(statement) => {
            if constant_bool(&statement.condition) != Some(false) {
                block_return_borrow(&statement.body, function, resolve_call, shadowed, borrow)?;
            }
            Some(crate::return_analysis::statement_falls_through(
                &Stmt::While(statement.clone()),
            ))
        }
        Stmt::For(statement) => {
            let mut loop_shadowed = shadowed.clone();
            if let Some(ast::ForInitializer::VarDecl(decl)) = &statement.initializer {
                loop_shadowed.insert(decl.name.clone());
            }
            if statement
                .condition
                .as_ref()
                .is_none_or(|condition| constant_bool(condition) != Some(false))
            {
                block_return_borrow(
                    &statement.body,
                    function,
                    resolve_call,
                    &loop_shadowed,
                    borrow,
                )?;
            }
            Some(crate::return_analysis::statement_falls_through(&Stmt::For(
                statement.clone(),
            )))
        }
        Stmt::Foreach(statement) => {
            let mut loop_shadowed = shadowed.clone();
            if let Some(key) = &statement.key {
                loop_shadowed.insert(key.name.clone());
            }
            loop_shadowed.insert(statement.value.name.clone());
            block_return_borrow(
                &statement.body,
                function,
                resolve_call,
                &loop_shadowed,
                borrow,
            )?;
            Some(crate::return_analysis::statement_falls_through(
                &Stmt::Foreach(statement.clone()),
            ))
        }
        Stmt::VarDecl(decl) => {
            shadowed.insert(decl.name.clone());
            Some(true)
        }
        Stmt::Break { .. } | Stmt::Continue { .. } => Some(false),
        Stmt::Expr { expr, .. } if is_panic_expr(expr) => Some(false),
        Stmt::Assignment(_) | Stmt::Echo { .. } | Stmt::Increment(_) | Stmt::Expr { .. } => {
            Some(true)
        }
    }
}

fn else_branch_return_borrow(
    branch: &ast::ElseBranch,
    function: &ast::FunctionDecl,
    resolve_call: &mut dyn FnMut(&Expr) -> Option<ReturnBorrow>,
    shadowed: &HashSet<String>,
    borrow: &mut Option<ReturnBorrow>,
) -> Option<bool> {
    match branch {
        ast::ElseBranch::If(statement) => {
            let mut branch_shadowed = shadowed.clone();
            statement_return_borrow(
                &Stmt::If((**statement).clone()),
                function,
                resolve_call,
                &mut branch_shadowed,
                borrow,
            )
        }
        ast::ElseBranch::Block(block) => {
            block_return_borrow(block, function, resolve_call, shadowed, borrow)
        }
    }
}

fn expr_return_borrow(
    expr: &Expr,
    function: &ast::FunctionDecl,
    resolve_call: &mut dyn FnMut(&Expr) -> Option<ReturnBorrow>,
    shadowed: &HashSet<String>,
) -> Option<ReturnBorrow> {
    match expr {
        Expr::This { .. } if !function.is_static => Some(ReturnBorrow {
            source: BorrowSource::Receiver,
            writable: function.writable_this,
        }),
        Expr::Variable { name, .. }
            if !shadowed.contains(name)
                && function
                    .params
                    .iter()
                    .filter(|param| !param.take && param.ty.as_class_name().is_some())
                    .count()
                    == 1 =>
        {
            function
                .params
                .iter()
                .enumerate()
                .find(|(_, param)| param.name == *name && !param.take)
                .map(|(index, param)| ReturnBorrow {
                    source: BorrowSource::Parameter(index),
                    writable: param.writable,
                })
        }
        Expr::Grouped { expr, .. } => expr_return_borrow(expr, function, resolve_call, shadowed),
        Expr::Binary {
            left,
            op: BinaryOp::Coalesce,
            right,
            ..
        } => coalesced_return_borrow(left, right, function, resolve_call, shadowed),
        Expr::PropertyAccess { object, .. } => {
            let mut direct_object = object.as_ref();
            while let Expr::Grouped { expr, .. } = direct_object {
                direct_object = expr;
            }
            if !matches!(direct_object, Expr::Variable { .. } | Expr::This { .. }) {
                return None;
            }
            expr_return_borrow(object, function, resolve_call, shadowed).map(|borrow| {
                ReturnBorrow {
                    writable: false,
                    ..borrow
                }
            })
        }
        Expr::FunctionCall { args, .. } | Expr::StaticCall { args, .. } => {
            returned_call_borrow(expr, None, args, function, resolve_call, shadowed)
        }
        Expr::MethodCall { object, args, .. } => {
            returned_call_borrow(expr, Some(object), args, function, resolve_call, shadowed)
        }
        _ => None,
    }
}

fn coalesced_return_borrow(
    left: &Expr,
    right: &Expr,
    function: &ast::FunctionDecl,
    resolve_call: &mut dyn FnMut(&Expr) -> Option<ReturnBorrow>,
    shadowed: &HashSet<String>,
) -> Option<ReturnBorrow> {
    let left_null = matches!(ungroup_expr(left), Expr::Null { .. });
    let right_null = matches!(ungroup_expr(right), Expr::Null { .. });
    let left = (!left_null)
        .then(|| expr_return_borrow(left, function, resolve_call, shadowed))
        .flatten();
    let right = (!right_null)
        .then(|| expr_return_borrow(right, function, resolve_call, shadowed))
        .flatten();

    match (left, right, left_null, right_null) {
        (Some(borrow), None, _, true) | (None, Some(borrow), true, _) => Some(borrow),
        (Some(mut left), Some(right), _, _) if left.source == right.source => {
            left.writable &= right.writable;
            Some(left)
        }
        _ => None,
    }
}

fn ungroup_expr(mut expr: &Expr) -> &Expr {
    while let Expr::Grouped { expr: inner, .. } = expr {
        expr = inner;
    }
    expr
}

fn returned_call_borrow(
    call: &Expr,
    receiver: Option<&Expr>,
    args: &[Expr],
    function: &ast::FunctionDecl,
    resolve_call: &mut dyn FnMut(&Expr) -> Option<ReturnBorrow>,
    shadowed: &HashSet<String>,
) -> Option<ReturnBorrow> {
    let returned = resolve_call(call)?;
    let source = match returned.source {
        BorrowSource::Receiver => receiver?,
        BorrowSource::Parameter(index) => args.get(index)?,
    };
    expr_return_borrow(source, function, resolve_call, shadowed).map(|mut borrow| {
        borrow.writable &= returned.writable;
        borrow
    })
}

fn signature(
    function: &ast::FunctionDecl,
    classes: &HashSet<String>,
    inferred_move_returns: &HashSet<usize>,
    return_borrows: &HashMap<usize, ReturnBorrow>,
    receiver_class: Option<&str>,
) -> Signature {
    let return_borrow = return_borrows
        .get(&function.span.start)
        .copied()
        .or_else(|| function_return_borrow(function))
        .filter(|_| {
            function
                .return_type
                .as_ref()
                .is_some_and(|ty| type_ref_class_name(ty, classes, receiver_class).is_some())
        });
    Signature {
        params: function
            .params
            .iter()
            .map(|param| Parameter {
                move_type: type_ref_is_move_type(&param.ty, classes, receiver_class),
                class_type: type_ref_class_name(&param.ty, classes, receiver_class).is_some(),
                take: param.take,
                writable: param.writable,
            })
            .collect(),
        returns: function
            .return_type
            .as_ref()
            .and_then(|ty| type_ref_class_name(ty, classes, receiver_class)),
        returns_move_type: function.return_type.as_ref().is_some_and(|ty| {
            type_ref_is_move_type(ty, classes, receiver_class) && return_borrow.is_none()
        }) || (function.return_type.is_none()
            && inferred_move_returns.contains(&function.span.start)),
        return_borrow,
        receiver: receiver_class.map(|_| {
            if function.writable_this {
                UseMode::Write
            } else {
                UseMode::Read
            }
        }),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UseMode {
    Read,
    Write,
    Give,
}

#[derive(Debug, Clone)]
struct ActiveBorrow {
    root: String,
    mode: UseMode,
    span: Span,
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
    static_properties: HashMap<(String, String), bool>,
    inferred_move_returns: HashSet<usize>,
    return_borrows: HashMap<usize, ReturnBorrow>,
    receiver_class: Option<String>,
    receiver_writable: bool,
    current_return_borrow: Option<UseMode>,
    active_assignment_writes: HashSet<String>,
    active_assignment_targets: HashSet<String>,
    active_borrows: Vec<ActiveBorrow>,
    diagnostics: Vec<Diagnostic>,
}

impl Checker {
    fn check_function(&mut self, function: &ast::FunctionDecl, receiver_class: Option<&str>) {
        let previous_receiver =
            std::mem::replace(&mut self.receiver_class, receiver_class.map(str::to_owned));
        let previous_receiver_writable =
            std::mem::replace(&mut self.receiver_writable, function.writable_this);
        let previous_return_borrow = self.current_return_borrow;
        self.current_return_borrow = function
            .return_type
            .as_ref()
            .is_some_and(|ty| {
                type_ref_class_name(ty, &self.classes, self.receiver_class.as_deref()).is_some()
            })
            .then(|| {
                self.return_borrows
                    .get(&function.span.start)
                    .copied()
                    .or_else(|| function_return_borrow(function))
            })
            .flatten()
            .map(|borrow| {
                if borrow.writable {
                    UseMode::Write
                } else {
                    UseMode::Read
                }
            });
        let mut scopes = Scopes::new();
        for param in &function.params {
            let class =
                type_ref_class_name(&param.ty, &self.classes, self.receiver_class.as_deref());
            let mixed = param.ty.name == "mixed";
            if type_ref_is_move_type(&param.ty, &self.classes, self.receiver_class.as_deref()) {
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
        let return_move_type = function.return_type.as_ref().is_some_and(|ty| {
            type_ref_is_move_type(ty, &self.classes, self.receiver_class.as_deref())
                && self.current_return_borrow.is_none()
        }) || (function.return_type.is_none()
            && self.inferred_move_returns.contains(&function.span.start));
        self.check_block(&function.body, &mut scopes, return_move_type, false);
        self.current_return_borrow = previous_return_borrow;
        self.receiver_writable = previous_receiver_writable;
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
                let declared_class = decl.ty.as_ref().and_then(|ty| {
                    type_ref_class_name(ty, &self.classes, self.receiver_class.as_deref())
                });
                let class = declared_class.or_else(|| self.expr_class(&decl.initializer, scopes));
                if self.expr_returns_borrow(&decl.initializer, scopes) {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0478",
                            format!(
                                "borrowed result cannot initialize owning `${}`",
                                decl.name
                            ),
                            decl.initializer.span(),
                        )
                        .with_help(
                            "keep using the result in the current expression, or bind an independently owned value",
                        ),
                    );
                }
                let initializer_moves = self.expr_is_move_value(&decl.initializer, scopes);
                let mixed = decl.ty.as_ref().is_some_and(|ty| ty.name == "mixed")
                    || (decl.ty.is_none() && class.is_none() && initializer_moves);
                let declared_move_type = decl.ty.as_ref().is_some_and(|ty| {
                    type_ref_is_move_type(ty, &self.classes, self.receiver_class.as_deref())
                });
                self.use_expr(
                    &decl.initializer,
                    scopes,
                    if initializer_moves || class.is_some() || mixed || declared_move_type {
                        UseMode::Give
                    } else {
                        UseMode::Read
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
                    self.use_assignment_operands(&assignment.target, &assignment.value, scopes);
                    return Flow::fallthrough();
                }
                if let Expr::Variable { name, span } = &assignment.target {
                    if self.expr_returns_borrow(&assignment.value, scopes)
                        && scopes.get(name).is_some()
                    {
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0478",
                                format!("borrowed result cannot replace owning `${name}`"),
                                assignment.value.span(),
                            )
                            .with_help(
                                "keep using the result in the current expression, or assign an independently owned value",
                            ),
                        );
                        return Flow::fallthrough();
                    }
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
                                UseMode::Read
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
                        self.use_expr(&assignment.value, scopes, UseMode::Read);
                    }
                } else {
                    if self.expr_returns_borrow(&assignment.value, scopes) {
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0478",
                                "borrowed result cannot be stored in an owning property",
                                assignment.value.span(),
                            )
                            .with_help(
                                "assign an independently owned value to the property instead",
                            ),
                        );
                    } else if self.expr_is_move_value(&assignment.value, scopes) {
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
                    self.use_assignment_operands(&assignment.target, &assignment.value, scopes);
                }
                Flow::fallthrough()
            }
            Stmt::Echo { expr, .. } => {
                self.use_expr(expr, scopes, UseMode::Read);
                Flow::fallthrough()
            }
            Stmt::Expr { expr, .. } => {
                self.use_expr(expr, scopes, UseMode::Read);
                if is_panic_expr(expr) {
                    Flow::stops()
                } else {
                    Flow::fallthrough()
                }
            }
            Stmt::Return { expr, .. } => {
                if let Some(expr) = expr {
                    if return_move_type
                        && self.current_return_borrow.is_none()
                        && self.expr_returns_borrow(expr, scopes)
                    {
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0478",
                                "borrowed result cannot satisfy an owning return",
                                expr.span(),
                            )
                            .with_help(
                                "return an independently owned value or return the borrowed source directly",
                            ),
                        );
                        self.use_expr(expr, scopes, UseMode::Read);
                        return Flow::stops();
                    }
                    self.use_expr(
                        expr,
                        scopes,
                        if return_move_type {
                            UseMode::Give
                        } else {
                            self.current_return_borrow.unwrap_or(UseMode::Read)
                        },
                    );
                }
                Flow::stops()
            }
            Stmt::If(statement) => {
                self.use_expr(&statement.condition, scopes, UseMode::Read);
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
                self.use_expr(&statement.condition, scopes, UseMode::Read);
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
                    self.use_expr(&statement.condition, repeat, UseMode::Read);
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
                    self.use_expr(condition, scopes, UseMode::Read);
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
                self.use_expr(&statement.iterable, scopes, UseMode::Read);
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
                self.use_expr(&increment.target, scopes, UseMode::Read);
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
        if !type_ref_is_move_type(ty, &self.classes, self.receiver_class.as_deref()) {
            return;
        }
        scopes.declare(
            binding.name.clone(),
            Binding {
                class: type_ref_class_name(ty, &self.classes, self.receiver_class.as_deref()),
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
                    self.use_expr(&increment.target, scopes, UseMode::Read);
                }
            }
        }
        if let Some(condition) = &statement.condition {
            self.use_expr(condition, scopes, UseMode::Read);
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

    fn use_assignment_operands(&mut self, target: &Expr, value: &Expr, scopes: &mut Scopes) {
        self.use_expr(target, scopes, UseMode::Write);
        let mut ungrouped_target = target;
        while let Expr::Grouped { expr, .. } = ungrouped_target {
            ungrouped_target = expr;
        }
        let tracked_target = matches!(
            ungrouped_target,
            Expr::PropertyAccess { .. } | Expr::StaticMember { .. }
        );
        let assignment_root = tracked_target
            .then(|| self.borrow_root_key(target, scopes))
            .flatten();
        let inserted = assignment_root
            .as_ref()
            .is_some_and(|root| self.active_assignment_writes.insert(root.clone()));
        let assignment_target = tracked_target
            .then(|| self.assignment_place_key(target, scopes))
            .flatten();
        let target_inserted = assignment_target
            .as_ref()
            .is_some_and(|target| self.active_assignment_targets.insert(target.clone()));
        self.use_expr(value, scopes, UseMode::Read);
        if target_inserted {
            self.active_assignment_targets
                .remove(assignment_target.as_deref().expect("inserted target"));
        }
        if inserted {
            self.active_assignment_writes
                .remove(assignment_root.as_deref().expect("inserted root"));
        }
    }

    fn use_expr(&mut self, expr: &Expr, scopes: &mut Scopes, mode: UseMode) {
        match expr {
            Expr::Variable { name, span } => {
                if matches!(mode, UseMode::Read | UseMode::Write) {
                    self.check_active_borrow_conflict(name, mode, *span);
                } else if mode == UseMode::Give {
                    self.check_give_against_active_borrows(name, *span);
                }
                if self.check_assignment_write_conflict(name, mode, *span) && mode == UseMode::Give
                {
                    return;
                }
                let Some(binding) = scopes.get_mut(name) else {
                    return;
                };
                if mode == UseMode::Write && !binding.writable {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0479",
                            format!("readonly `${name}` cannot be used as writable"),
                            *span,
                        )
                        .with_help("declare the binding `writable` before passing it for mutation"),
                    );
                }
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
                let assignment_root = (mode == UseMode::Read
                    && self
                        .assignment_place_key(expr, scopes)
                        .is_some_and(|target| self.active_assignment_targets.contains(&target)))
                .then(|| self.borrow_root_key(object, scopes))
                .flatten();
                let suspended_assignment_write = assignment_root
                    .as_ref()
                    .is_some_and(|root| self.active_assignment_writes.remove(root));
                self.use_expr(
                    object,
                    scopes,
                    if mode == UseMode::Write {
                        UseMode::Write
                    } else {
                        UseMode::Read
                    },
                );
                if suspended_assignment_write {
                    self.active_assignment_writes
                        .insert(assignment_root.expect("suspended assignment root"));
                }
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
                qualifier,
                method,
                args,
                ..
            } => {
                let signature = self
                    .qualifier_class(qualifier)
                    .and_then(|class_name| self.methods.get(&(class_name, method.clone())))
                    .cloned()
                    .unwrap_or_default();
                self.use_call_args(None, args, &signature, scopes);
            }
            Expr::InterpolatedString { parts, .. } => {
                let borrow_depth = self.active_borrows.len();
                for part in parts {
                    if let ast::InterpolatedStringPart::Expr(expr) = part {
                        self.use_read_with_place_borrow(expr, scopes);
                    }
                }
                self.active_borrows.truncate(borrow_depth);
            }
            Expr::Array { elements, .. } => {
                let borrow_depth = self.active_borrows.len();
                for element in elements {
                    if let Some(key) = &element.key {
                        let mode = self.use_owned_expression(key, scopes);
                        self.activate_place_input_borrows(key, scopes);
                        if mode == UseMode::Read {
                            self.activate_call_borrow(key, mode, scopes);
                        }
                    }
                    let mode = self.use_owned_expression(&element.value, scopes);
                    self.activate_place_input_borrows(&element.value, scopes);
                    if mode == UseMode::Read {
                        self.activate_call_borrow(&element.value, mode, scopes);
                    }
                }
                self.active_borrows.truncate(borrow_depth);
            }
            Expr::Unary { expr, .. } => self.use_expr(expr, scopes, UseMode::Read),
            Expr::IsType { expr, .. } => self.use_expr(expr, scopes, UseMode::Read),
            Expr::Binary {
                left,
                op: op @ (BinaryOp::And | BinaryOp::Or),
                right,
                ..
            } => {
                let borrow_depth = self.active_borrows.len();
                self.use_read_with_place_borrow(left, scopes);
                match (op, constant_bool(left)) {
                    (BinaryOp::And, Some(false)) | (BinaryOp::Or, Some(true)) => {}
                    (BinaryOp::And, Some(true)) | (BinaryOp::Or, Some(false)) => {
                        self.use_expr(right, scopes, UseMode::Read);
                    }
                    _ => {
                        let without_right = scopes.clone();
                        let mut with_right = without_right.clone();
                        self.use_expr(right, &mut with_right, UseMode::Read);
                        scopes.merge_from(&without_right, &with_right);
                    }
                }
                self.active_borrows.truncate(borrow_depth);
            }
            Expr::Binary { left, right, .. }
            | Expr::Range {
                start: left,
                end: right,
                ..
            } => {
                let borrow_depth = self.active_borrows.len();
                self.use_read_with_place_borrow(left, scopes);
                self.use_expr(right, scopes, UseMode::Read);
                self.active_borrows.truncate(borrow_depth);
            }
            Expr::This { span } => {
                if matches!(mode, UseMode::Read | UseMode::Write) {
                    self.check_active_borrow_conflict("$this", mode, *span);
                } else if mode == UseMode::Give {
                    self.check_give_against_active_borrows("$this", *span);
                }
                if self.check_assignment_write_conflict("$this", mode, *span)
                    && mode == UseMode::Give
                {
                    return;
                }
                if mode == UseMode::Give {
                    self.diagnostics.push(
                        Diagnostic::new("E0474", "borrowed `$this` cannot be given away", *span)
                            .with_help(
                                "the method receiver is borrowed from its caller and must remain owned by that caller",
                            ),
                    );
                }
            }
            Expr::StaticMember { span, .. } => {
                let Some(root) = self.borrow_root_key(expr, scopes) else {
                    return;
                };
                let exact_assignment_target = mode == UseMode::Read
                    && self
                        .assignment_place_key(expr, scopes)
                        .is_some_and(|target| self.active_assignment_targets.contains(&target));
                let suspended_assignment_write =
                    exact_assignment_target && self.active_assignment_writes.remove(&root);
                if matches!(mode, UseMode::Read | UseMode::Write) {
                    self.check_active_borrow_conflict(&root, mode, *span);
                } else {
                    self.check_give_against_active_borrows(&root, *span);
                }
                self.check_assignment_write_conflict(&root, mode, *span);
                if suspended_assignment_write {
                    self.active_assignment_writes.insert(root);
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
        let borrow_depth = self.active_borrows.len();
        if let Some(receiver) = receiver {
            self.use_expr(receiver, scopes, UseMode::Read);
            self.activate_place_input_borrows(receiver, scopes);
            if let Some(mode) = signature.receiver {
                self.activate_call_borrow(receiver, mode, scopes);
            }
        }
        for (index, arg) in args.iter().enumerate() {
            let mode = call_arg_mode(signature, index);
            if mode == UseMode::Write
                && signature
                    .params
                    .get(index)
                    .is_some_and(|param| !param.class_type)
            {
                self.check_writable_move_argument(arg, scopes);
            }
            if mode == UseMode::Give {
                if self.expr_returns_borrow(arg, scopes) {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0474",
                            "borrowed call result cannot be given away",
                            arg.span(),
                        )
                        .with_help(
                            "pass an independently owned value to an ownership-taking parameter",
                        ),
                    );
                    self.use_expr(arg, scopes, UseMode::Read);
                    continue;
                }
                if let Some(name) = ownership_root_name(arg).filter(|name| {
                    self.active_borrows
                        .iter()
                        .skip(borrow_depth)
                        .any(|borrow| borrow.root == *name)
                }) {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0471",
                            format!("`${name}` cannot be borrowed and given away in the same call"),
                            arg.span(),
                        )
                        .with_help("pass distinct owners for borrowed and ownership-taking inputs"),
                    );
                }
                if let Some(name) = ownership_root_name(arg) {
                    self.check_give_against_active_borrows(name, arg.span());
                }
            }
            self.use_expr(arg, scopes, mode);
            self.activate_place_input_borrows(arg, scopes);
            if matches!(mode, UseMode::Read | UseMode::Write) {
                self.activate_call_borrow(arg, mode, scopes);
            }
        }
        self.active_borrows.truncate(borrow_depth);
    }

    fn activate_call_borrow(&mut self, expr: &Expr, mode: UseMode, scopes: &Scopes) {
        let Some(root) = self.borrow_root_key(expr, scopes) else {
            return;
        };
        self.check_active_borrow_conflict(&root, mode, expr.span());
        self.active_borrows.push(ActiveBorrow {
            root,
            mode,
            span: expr.span(),
        });
    }

    fn activate_place_input_borrows(&mut self, expr: &Expr, scopes: &Scopes) {
        match expr {
            Expr::Grouped { expr, .. } | Expr::PropertyAccess { object: expr, .. } => {
                self.activate_place_input_borrows(expr, scopes);
            }
            _ => self.activate_nested_property_borrows(expr, scopes),
        }
    }

    fn activate_nested_property_borrows(&mut self, expr: &Expr, scopes: &Scopes) {
        match expr {
            Expr::PropertyAccess { object, .. } => {
                if self
                    .borrow_root_key(expr, scopes)
                    .is_some_and(|root| !self.active_assignment_writes.contains(&root))
                {
                    self.activate_call_borrow(expr, UseMode::Read, scopes);
                }
                self.activate_nested_property_borrows(object, scopes);
            }
            Expr::Grouped { expr, .. } | Expr::Unary { expr, .. } => {
                self.activate_nested_property_borrows(expr, scopes);
            }
            Expr::Binary {
                left,
                op: op @ (BinaryOp::And | BinaryOp::Or),
                right,
                ..
            } => {
                self.activate_nested_property_borrows(left, scopes);
                if !matches!(
                    (op, constant_bool(left)),
                    (BinaryOp::And, Some(false)) | (BinaryOp::Or, Some(true))
                ) {
                    self.activate_nested_property_borrows(right, scopes);
                }
            }
            Expr::Binary { left, right, .. } => {
                self.activate_nested_property_borrows(left, scopes);
                self.activate_nested_property_borrows(right, scopes);
            }
            Expr::Range { start, end, .. } => {
                self.activate_nested_property_borrows(start, scopes);
                self.activate_nested_property_borrows(end, scopes);
            }
            Expr::InterpolatedString { parts, .. } => {
                for part in parts {
                    if let ast::InterpolatedStringPart::Expr(expr) = part {
                        self.activate_nested_property_borrows(expr, scopes);
                    }
                }
            }
            Expr::Array { elements, .. } => {
                for element in elements {
                    if let Some(key) = &element.key {
                        self.activate_nested_property_borrows(key, scopes);
                    }
                    self.activate_nested_property_borrows(&element.value, scopes);
                }
            }
            Expr::FunctionCall { name, args, .. } => {
                let signature = self.signatures.get(name).cloned().unwrap_or_default();
                self.activate_nested_call_property_borrows(None, args, &signature, scopes);
            }
            Expr::New {
                class_name, args, ..
            } => {
                let signature = self
                    .constructors
                    .get(class_name)
                    .cloned()
                    .unwrap_or_default();
                self.activate_nested_call_property_borrows(None, args, &signature, scopes);
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
                self.activate_nested_call_property_borrows(Some(object), args, &signature, scopes);
            }
            Expr::StaticCall {
                qualifier,
                method,
                args,
                ..
            } => {
                let signature = self
                    .qualifier_class(qualifier)
                    .and_then(|class| self.methods.get(&(class, method.clone())).cloned())
                    .unwrap_or_default();
                self.activate_nested_call_property_borrows(None, args, &signature, scopes);
            }
            _ => {}
        }
    }

    fn activate_nested_call_property_borrows(
        &mut self,
        receiver: Option<&Expr>,
        args: &[Expr],
        signature: &Signature,
        scopes: &Scopes,
    ) {
        if let Some(receiver) = receiver {
            self.activate_place_input_borrows(receiver, scopes);
            let result_continues_receiver_borrow = signature
                .return_borrow
                .is_some_and(|borrow| borrow.source == BorrowSource::Receiver);
            if !result_continues_receiver_borrow {
                if let Some(mode @ (UseMode::Read | UseMode::Write)) = signature.receiver {
                    self.activate_call_borrow(receiver, mode, scopes);
                }
            }
        }
        for (index, arg) in args.iter().enumerate() {
            let mode = call_arg_mode(signature, index);
            self.activate_place_input_borrows(arg, scopes);
            if matches!(mode, UseMode::Read | UseMode::Write) {
                self.activate_call_borrow(arg, mode, scopes);
            }
        }
    }

    fn use_read_with_place_borrow(&mut self, expr: &Expr, scopes: &mut Scopes) {
        self.use_expr(expr, scopes, UseMode::Read);
        self.activate_place_input_borrows(expr, scopes);
        self.activate_call_borrow(expr, UseMode::Read, scopes);
    }

    fn check_writable_move_argument(&mut self, expr: &Expr, scopes: &Scopes) {
        if let Some((subject, span)) = self.readonly_writable_path(expr, scopes) {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0479",
                    format!("readonly {subject} cannot be used as writable"),
                    span,
                )
                .with_help("make every borrowed binding and property in the path `writable` before passing it for mutation"),
            );
        }
    }

    fn readonly_writable_path(&self, expr: &Expr, scopes: &Scopes) -> Option<(String, Span)> {
        match expr {
            Expr::Grouped { expr, .. } => self.readonly_writable_path(expr, scopes),
            // Direct bindings are checked by `use_expr` in write mode. This
            // helper supplies the path-sensitive checks that mode cannot see.
            Expr::Variable { .. } => None,
            Expr::This { span } if !self.receiver_writable => Some(("`$this`".to_string(), *span)),
            Expr::This { .. } => None,
            Expr::PropertyAccess {
                object,
                property,
                span,
                ..
            } => self.readonly_writable_path(object, scopes).or_else(|| {
                self.expr_class(object, scopes)
                    .and_then(|class| self.properties.get(&(class, property.clone())))
                    .is_some_and(|property| !property.writable)
                    .then(|| (format!("property `${property}`"), *span))
            }),
            Expr::StaticMember {
                qualifier,
                member,
                span,
                ..
            } => self.qualifier_class(qualifier).and_then(|class| {
                self.static_properties
                    .get(&(class.clone(), member.clone()))
                    .is_some_and(|writable| !writable)
                    .then(|| (format!("static property `{class}::{member}`"), *span))
            }),
            Expr::FunctionCall { name, span, .. } => self
                .signatures
                .get(name)
                .and_then(|signature| self.readonly_return_borrow_path(signature, *span)),
            Expr::MethodCall {
                object,
                method,
                span,
                ..
            } => self.expr_class(object, scopes).and_then(|class| {
                self.methods
                    .get(&(class, method.clone()))
                    .and_then(|signature| self.readonly_return_borrow_path(signature, *span))
            }),
            Expr::StaticCall {
                qualifier,
                method,
                span,
                ..
            } => self.qualifier_class(qualifier).and_then(|class| {
                self.methods
                    .get(&(class, method.clone()))
                    .and_then(|signature| self.readonly_return_borrow_path(signature, *span))
            }),
            _ => None,
        }
    }

    fn readonly_return_borrow_path(
        &self,
        signature: &Signature,
        span: Span,
    ) -> Option<(String, Span)> {
        let borrow = signature.return_borrow?;
        (!borrow.writable).then(|| ("returned borrow".to_string(), span))
    }

    fn check_assignment_write_conflict(&mut self, root: &str, mode: UseMode, span: Span) -> bool {
        if !self.active_assignment_writes.contains(root) {
            return false;
        }
        match mode {
            UseMode::Give => self.diagnostics.push(
                Diagnostic::new(
                    "E0471",
                    format!(
                        "`{}` cannot be given away while it is the destination of a property assignment",
                        display_borrow_root(root)
                    ),
                    span,
                )
                .with_help(
                    "compute the replacement without giving away the object being assigned",
                ),
            ),
            UseMode::Read | UseMode::Write => {
                let requested = if mode == UseMode::Read {
                    "readonly"
                } else {
                    "writable"
                };
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0477",
                        format!(
                            "`{}` cannot be used as {requested} while it is the destination of a property assignment",
                            display_borrow_root(root)
                        ),
                        span,
                    )
                    .with_help(
                        "finish computing the property value before using the same owner again",
                    ),
                );
            }
        }
        true
    }

    fn borrow_root_key(&self, expr: &Expr, scopes: &Scopes) -> Option<String> {
        match expr {
            Expr::This { .. } if self.receiver_class.is_some() => Some("$this".to_string()),
            Expr::Variable { name, .. } if scopes.get(name).is_some() => Some(name.clone()),
            Expr::PropertyAccess { object, .. } | Expr::Grouped { expr: object, .. } => {
                self.borrow_root_key(object, scopes)
            }
            Expr::StaticMember {
                qualifier, member, ..
            } => self.qualifier_class(qualifier).and_then(|class| {
                self.static_properties
                    .contains_key(&(class.clone(), member.clone()))
                    .then(|| format!("static:{class}::{member}"))
            }),
            Expr::FunctionCall { name, args, .. } => self
                .signatures
                .get(name)
                .and_then(|signature| signature.return_borrow)
                .and_then(|borrow| self.call_borrow_root(borrow, None, args, scopes)),
            Expr::MethodCall {
                object,
                method,
                args,
                ..
            } => {
                let class = self.expr_class(object, scopes)?;
                self.methods
                    .get(&(class, method.clone()))
                    .and_then(|signature| signature.return_borrow)
                    .and_then(|borrow| self.call_borrow_root(borrow, Some(object), args, scopes))
            }
            Expr::StaticCall {
                qualifier,
                method,
                args,
                ..
            } => self
                .qualifier_class(qualifier)
                .and_then(|class| self.methods.get(&(class, method.clone())))
                .and_then(|signature| signature.return_borrow)
                .and_then(|borrow| self.call_borrow_root(borrow, None, args, scopes)),
            _ => None,
        }
    }

    fn assignment_place_key(&self, expr: &Expr, scopes: &Scopes) -> Option<String> {
        match expr {
            Expr::This { .. } if self.receiver_class.is_some() => Some("$this".to_string()),
            Expr::Variable { name, .. } if scopes.get(name).is_some() => Some(name.clone()),
            Expr::Grouped { expr, .. } => self.assignment_place_key(expr, scopes),
            Expr::PropertyAccess {
                object, property, ..
            } => self
                .assignment_place_key(object, scopes)
                .map(|object| format!("{object}->{property}")),
            Expr::StaticMember { .. } => self.borrow_root_key(expr, scopes),
            _ => None,
        }
    }

    fn call_borrow_root(
        &self,
        borrow: ReturnBorrow,
        receiver: Option<&Expr>,
        args: &[Expr],
        scopes: &Scopes,
    ) -> Option<String> {
        match borrow.source {
            BorrowSource::Receiver => self.borrow_root_key(receiver?, scopes),
            BorrowSource::Parameter(index) => self.borrow_root_key(args.get(index)?, scopes),
        }
    }

    fn check_active_borrow_conflict(&mut self, root: &str, mode: UseMode, span: Span) {
        if mode == UseMode::Give {
            self.check_give_against_active_borrows(root, span);
            return;
        }
        let Some(existing) = self
            .active_borrows
            .iter()
            .rev()
            .find(|borrow| borrow.root == root && borrow_modes_conflict(borrow.mode, mode))
            .cloned()
        else {
            return;
        };
        let existing_span = existing.span;
        let requested = match mode {
            UseMode::Read => "readonly",
            UseMode::Write => "writable",
            UseMode::Give => unreachable!("handled above"),
        };
        let existing = match existing.mode {
            UseMode::Read => "readonly",
            UseMode::Write => "writable",
            UseMode::Give => unreachable!("active borrow cannot be a give"),
        };
        let root_display = display_borrow_root(root);
        self.diagnostics.push(
            Diagnostic::new(
                "E0477",
                format!(
                    "`{root_display}` cannot be used as {requested} here because it is already used as {existing} in this call"
                ),
                span,
            )
            .with_help(format!(
                "finish the earlier use at bytes {}..{} before taking the conflicting writable access",
                existing_span.start,
                existing_span.end
            )),
        );
    }

    fn check_give_against_active_borrows(&mut self, root: &str, span: Span) {
        if let Some(existing) = self
            .active_borrows
            .iter()
            .rev()
            .find(|borrow| borrow.root == root)
            .cloned()
        {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0471",
                    format!(
                        "`{}` cannot be borrowed and given away in the same call",
                        display_borrow_root(root)
                    ),
                    span,
                )
                .with_help(format!(
                    "the earlier use at bytes {}..{} must finish before ownership is given away",
                    existing.span.start, existing.span.end
                )),
            );
        }
    }

    fn use_owned_expression(&mut self, expr: &Expr, scopes: &mut Scopes) -> UseMode {
        if self.reject_borrowed_result(
            expr,
            scopes,
            "borrowed result cannot be stored in an owning collection",
            "store an independently owned value in the collection",
        ) {
            self.use_expr(expr, scopes, UseMode::Read);
            return UseMode::Read;
        }
        let mode = if self.expr_is_move_value(expr, scopes) {
            UseMode::Give
        } else {
            UseMode::Read
        };
        self.use_expr(expr, scopes, mode);
        mode
    }

    fn reject_borrowed_result(
        &mut self,
        expr: &Expr,
        scopes: &Scopes,
        message: impl Into<String>,
        help: impl Into<String>,
    ) -> bool {
        if !self.expr_returns_borrow(expr, scopes) {
            return false;
        }
        self.diagnostics
            .push(Diagnostic::new("E0478", message, expr.span()).with_help(help));
        true
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
                qualifier, method, ..
            } => self
                .qualifier_class(qualifier)
                .and_then(|class_name| self.methods.get(&(class_name, method.clone())))
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

    fn expr_returns_borrow(&self, expr: &Expr, scopes: &Scopes) -> bool {
        match expr {
            Expr::Grouped { expr, .. } => self.expr_returns_borrow(expr, scopes),
            Expr::FunctionCall { name, .. } => self
                .signatures
                .get(name)
                .is_some_and(|signature| signature.return_borrow.is_some()),
            Expr::MethodCall { object, method, .. } => {
                let Some(class) = self.expr_class(object, scopes) else {
                    return false;
                };
                self.methods
                    .get(&(class, method.clone()))
                    .is_some_and(|signature| signature.return_borrow.is_some())
            }
            Expr::StaticCall {
                qualifier, method, ..
            } => self
                .qualifier_class(qualifier)
                .and_then(|class| self.methods.get(&(class, method.clone())))
                .is_some_and(|signature| signature.return_borrow.is_some()),
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
                qualifier, method, ..
            } => self
                .qualifier_class(qualifier)
                .and_then(|class_name| self.methods.get(&(class_name, method.clone())))
                .and_then(|signature| signature.returns.clone()),
            Expr::This { .. } => self.receiver_class.clone(),
            Expr::Grouped { expr, .. } => self.expr_class(expr, scopes),
            _ => None,
        }
    }

    fn qualifier_class(&self, qualifier: &ast::StaticQualifier) -> Option<String> {
        match qualifier {
            ast::StaticQualifier::Class(name) => Some(name.clone()),
            ast::StaticQualifier::SelfType => self.receiver_class.clone(),
            ast::StaticQualifier::Parent | ast::StaticQualifier::InvalidStatic => None,
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

fn borrow_modes_conflict(existing: UseMode, requested: UseMode) -> bool {
    matches!(
        (existing, requested),
        (UseMode::Write, UseMode::Read)
            | (UseMode::Read, UseMode::Write)
            | (UseMode::Write, UseMode::Write)
    )
}

fn display_borrow_root(root: &str) -> String {
    if root == "$this" {
        root.to_string()
    } else if let Some(member) = root.strip_prefix("static:") {
        member.to_string()
    } else {
        format!("${root}")
    }
}

fn type_ref_class_name(
    ty: &crate::types::TypeRef,
    classes: &HashSet<String>,
    receiver_class: Option<&str>,
) -> Option<String> {
    let name = if ty.name == "self" {
        receiver_class?
    } else {
        &ty.name
    };
    classes.contains(name).then(|| name.to_string())
}

pub(crate) fn type_ref_is_move_type(
    ty: &crate::types::TypeRef,
    classes: &HashSet<String>,
    receiver_class: Option<&str>,
) -> bool {
    type_ref_class_name(ty, classes, receiver_class).is_some()
        || matches!(
            ty.name.as_str(),
            "mixed" | "[]" | "List" | "Dictionary" | "Set"
        )
}

fn call_arg_mode(signature: &Signature, index: usize) -> UseMode {
    let Some(param) = signature.params.get(index) else {
        return UseMode::Read;
    };
    if param.take && param.move_type {
        UseMode::Give
    } else if param.writable && param.move_type {
        UseMode::Write
    } else {
        UseMode::Read
    }
}

pub(crate) fn constant_bool(expr: &Expr) -> Option<bool> {
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

use std::collections::{BTreeMap, HashMap};

use crate::ast::{
    Block, ElseBranch, Expr, ForIncrement, ForInitializer, FunctionDecl, Item, Param, Program, Stmt,
};
use crate::control_flow::{build_function_cfg, Node, NodeAction};
use crate::dataflow::{solve_forward, ForwardAnalysis};
use crate::source::Span;
use crate::types::TypeRef;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fact {
    NonNull,
    Null,
    Exact(TypeRef),
}

pub type FactsByUse = HashMap<(usize, usize), Fact>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct BindingId(usize);

#[derive(Debug, Clone, PartialEq, Eq)]
struct State {
    reachable: bool,
    facts: BTreeMap<BindingId, Fact>,
}

#[derive(Default)]
struct Resolution {
    uses: HashMap<(usize, usize), BindingId>,
    declarations: HashMap<usize, BindingId>,
}

#[derive(Default)]
struct MutationCatalog {
    functions: HashMap<String, Vec<bool>>,
    methods: HashMap<String, Vec<bool>>,
    qualified_methods: HashMap<(String, String), Vec<bool>>,
    constructors: HashMap<String, Vec<bool>>,
}

impl MutationCatalog {
    fn from_program(program: &Program) -> Self {
        let mut catalog = Self::default();
        for item in &program.items {
            match item {
                Item::Function(function) => merge_parameter_modes(
                    catalog.functions.entry(function.name.clone()).or_default(),
                    &function.params,
                ),
                Item::Class(class) => {
                    for member in &class.members {
                        let crate::ast::ClassMember::Method(method) = member else {
                            continue;
                        };
                        let modes = method
                            .params
                            .iter()
                            .map(|parameter| parameter.writable || parameter.take)
                            .collect::<Vec<_>>();
                        merge_modes(
                            catalog.methods.entry(method.name.clone()).or_default(),
                            &modes,
                        );
                        merge_modes(
                            catalog
                                .qualified_methods
                                .entry((class.name.clone(), method.name.clone()))
                                .or_default(),
                            &modes,
                        );
                        if method.name == "__construct" {
                            merge_modes(
                                catalog.constructors.entry(class.name.clone()).or_default(),
                                &modes,
                            );
                        }
                    }
                }
                Item::Interface(_) | Item::Trait(_) | Item::Constant(_) | Item::Statement(_) => {}
            }
        }
        catalog
    }

    fn function_modes(&self, name: &str) -> Option<&[bool]> {
        self.functions.get(name).map(Vec::as_slice)
    }

    fn method_modes(&self, method: &str) -> Option<&[bool]> {
        self.methods.get(method).map(Vec::as_slice)
    }

    fn static_method_modes(
        &self,
        qualifier: &crate::ast::StaticQualifier,
        method: &str,
    ) -> Option<&[bool]> {
        match qualifier {
            crate::ast::StaticQualifier::Class(class) => self
                .qualified_methods
                .get(&(class.clone(), method.to_string()))
                .or_else(|| self.methods.get(method))
                .map(Vec::as_slice),
            crate::ast::StaticQualifier::SelfType
            | crate::ast::StaticQualifier::Parent
            | crate::ast::StaticQualifier::InvalidStatic => self.method_modes(method),
        }
    }

    fn constructor_modes(&self, class: &str) -> Option<&[bool]> {
        self.constructors.get(class).map(Vec::as_slice)
    }
}

fn merge_parameter_modes(target: &mut Vec<bool>, parameters: &[Param]) {
    let modes = parameters
        .iter()
        .map(|parameter| parameter.writable || parameter.take)
        .collect::<Vec<_>>();
    merge_modes(target, &modes);
}

fn merge_modes(target: &mut Vec<bool>, incoming: &[bool]) {
    if target.len() < incoming.len() {
        target.resize(incoming.len(), false);
    }
    for (index, mutable) in incoming.iter().enumerate() {
        target[index] |= mutable;
    }
}

pub fn analyze_program(program: &Program) -> FactsByUse {
    let mut facts = HashMap::new();
    let mutations = MutationCatalog::from_program(program);
    let top_level_span = program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Statement(statement) => Some(statement_span(statement)),
            _ => None,
        })
        .reduce(Span::merge)
        .unwrap_or_default();
    let top_level = Block {
        statements: program
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Statement(statement) => Some(statement.clone()),
                _ => None,
            })
            .collect(),
        span: top_level_span,
    };
    if !top_level.statements.is_empty() {
        analyze_body(&top_level, &[], top_level_span, &mut facts, &mutations);
    }

    for item in &program.items {
        match item {
            Item::Function(function) => analyze_function(function, &mut facts, &mutations),
            Item::Class(class) => {
                for member in &class.members {
                    if let crate::ast::ClassMember::Method(method) = member {
                        analyze_function(method, &mut facts, &mutations);
                    }
                }
            }
            Item::Interface(_) | Item::Trait(_) | Item::Constant(_) | Item::Statement(_) => {}
        }
    }
    facts
}

fn statement_span(statement: &Stmt) -> Span {
    match statement {
        Stmt::VarDecl(declaration) => declaration.span,
        Stmt::Assignment(assignment) => assignment.span,
        Stmt::Echo { span, .. }
        | Stmt::Return { span, .. }
        | Stmt::Break { span }
        | Stmt::Continue { span }
        | Stmt::Expr { span, .. } => *span,
        Stmt::If(statement) => statement.span,
        Stmt::While(statement) => statement.span,
        Stmt::For(statement) => statement.span,
        Stmt::Foreach(statement) => statement.span,
        Stmt::Increment(statement) => statement.span,
    }
}

fn analyze_function(function: &FunctionDecl, facts: &mut FactsByUse, mutations: &MutationCatalog) {
    analyze_body(
        &function.body,
        &function.params,
        function.span,
        facts,
        mutations,
    );
}

fn analyze_body(
    body: &Block,
    params: &[Param],
    span: Span,
    facts: &mut FactsByUse,
    mutations: &MutationCatalog,
) {
    let resolution = Resolver::resolve(body, params);
    let graph = build_function_cfg(body, span);
    let analysis = NarrowingAnalysis {
        resolution: &resolution,
        mutations,
    };
    let result = solve_forward(&graph, &analysis);

    for node_id in result.traversal_order {
        let input = &result.inputs[node_id.0];
        if !input.reachable {
            continue;
        }
        collect_action_facts(
            &graph.nodes[node_id.0].action,
            input,
            &resolution,
            mutations,
            facts,
        );
    }
}

struct NarrowingAnalysis<'a> {
    resolution: &'a Resolution,
    mutations: &'a MutationCatalog,
}

impl ForwardAnalysis for NarrowingAnalysis<'_> {
    type State = State;

    fn bottom(&self) -> Self::State {
        State {
            reachable: false,
            facts: BTreeMap::new(),
        }
    }

    fn entry_state(&self) -> Self::State {
        State {
            reachable: true,
            facts: BTreeMap::new(),
        }
    }

    fn transfer(&self, node: &Node, input: &Self::State) -> Self::State {
        if !input.reachable {
            return input.clone();
        }
        let mut output = input.clone();
        match &node.action {
            NodeAction::Assume { condition, truth } => apply_condition_with_effects(
                condition,
                *truth,
                &mut output,
                self.resolution,
                self.mutations,
            ),
            NodeAction::Statement(statement) => {
                transfer_statement(statement, &mut output, self.resolution, self.mutations)
            }
            NodeAction::ForInitializer(initializer) => match initializer {
                ForInitializer::VarDecl(declaration) => transfer_declaration(
                    declaration.span,
                    &declaration.initializer,
                    &mut output,
                    self.resolution,
                    self.mutations,
                ),
                ForInitializer::Assignment(assignment) => transfer_assignment(
                    &assignment.target,
                    &assignment.value,
                    &mut output,
                    self.resolution,
                    self.mutations,
                ),
            },
            NodeAction::ForIncrement(increment) => match increment {
                ForIncrement::Assignment(assignment) => transfer_assignment(
                    &assignment.target,
                    &assignment.value,
                    &mut output,
                    self.resolution,
                    self.mutations,
                ),
                ForIncrement::Increment(increment) => {
                    kill_mutated_call_arguments(
                        &increment.target,
                        &mut output,
                        self.resolution,
                        self.mutations,
                    );
                    kill_target(&increment.target, &mut output, self.resolution)
                }
            },
            NodeAction::Expression(expression) => kill_mutated_call_arguments(
                expression,
                &mut output,
                self.resolution,
                self.mutations,
            ),
            NodeAction::None => {}
        }
        output
    }

    fn join(&self, state: &mut Self::State, incoming: &Self::State) -> bool {
        if !incoming.reachable {
            return false;
        }
        if !state.reachable {
            *state = incoming.clone();
            return true;
        }

        let merged = state
            .facts
            .iter()
            .filter(|(binding, fact)| incoming.facts.get(binding) == Some(*fact))
            .map(|(binding, fact)| (*binding, fact.clone()))
            .collect();
        if state.facts == merged {
            false
        } else {
            state.facts = merged;
            true
        }
    }
}

fn transfer_statement(
    statement: &Stmt,
    state: &mut State,
    resolution: &Resolution,
    mutations: &MutationCatalog,
) {
    match statement {
        Stmt::VarDecl(declaration) => transfer_declaration(
            declaration.span,
            &declaration.initializer,
            state,
            resolution,
            mutations,
        ),
        Stmt::Assignment(assignment) => transfer_assignment(
            &assignment.target,
            &assignment.value,
            state,
            resolution,
            mutations,
        ),
        Stmt::Increment(increment) => {
            kill_mutated_call_arguments(&increment.target, state, resolution, mutations);
            kill_target(&increment.target, state, resolution);
        }
        Stmt::Echo { expr, .. } | Stmt::Expr { expr, .. } => {
            kill_mutated_call_arguments(expr, state, resolution, mutations);
        }
        Stmt::Return { expr, .. } => {
            if let Some(expr) = expr {
                kill_mutated_call_arguments(expr, state, resolution, mutations);
            }
        }
        Stmt::If(_)
        | Stmt::While(_)
        | Stmt::For(_)
        | Stmt::Foreach(_)
        | Stmt::Break { .. }
        | Stmt::Continue { .. } => {}
    }
}

fn transfer_declaration(
    span: Span,
    initializer: &Expr,
    state: &mut State,
    resolution: &Resolution,
    mutations: &MutationCatalog,
) {
    kill_mutated_call_arguments(initializer, state, resolution, mutations);
    if let Some(binding) = resolution.declarations.get(&span.start) {
        set_from_value(*binding, initializer, state);
    }
}

fn transfer_assignment(
    target: &Expr,
    value: &Expr,
    state: &mut State,
    resolution: &Resolution,
    mutations: &MutationCatalog,
) {
    kill_mutated_call_arguments(target, state, resolution, mutations);
    kill_mutated_call_arguments(value, state, resolution, mutations);
    if let Some(binding) = variable_binding(target, resolution) {
        set_from_value(binding, value, state);
    }
}

fn kill_target(target: &Expr, state: &mut State, resolution: &Resolution) {
    if let Some(binding) = variable_binding(target, resolution) {
        state.facts.remove(&binding);
    }
}

fn apply_condition_with_effects(
    condition: &Expr,
    truth: bool,
    state: &mut State,
    resolution: &Resolution,
    mutations: &MutationCatalog,
) {
    match ungroup(condition) {
        Expr::Unary {
            op: crate::ast::UnaryOp::Not,
            expr,
            ..
        } => apply_condition_with_effects(expr, !truth, state, resolution, mutations),
        Expr::Binary {
            left,
            op: crate::ast::BinaryOp::And,
            right,
            ..
        } if truth => {
            apply_condition_with_effects(left, true, state, resolution, mutations);
            apply_condition_with_effects(right, true, state, resolution, mutations);
        }
        Expr::Binary {
            left,
            op: crate::ast::BinaryOp::Or,
            right,
            ..
        } if !truth => {
            apply_condition_with_effects(left, false, state, resolution, mutations);
            apply_condition_with_effects(right, false, state, resolution, mutations);
        }
        _ => {
            kill_mutated_call_arguments(condition, state, resolution, mutations);
            apply_condition(condition, truth, state, resolution);
        }
    }
}

fn kill_mutated_call_arguments(
    expr: &Expr,
    state: &mut State,
    resolution: &Resolution,
    mutations: &MutationCatalog,
) {
    match expr {
        Expr::PropertyAccess { object, .. }
        | Expr::Grouped { expr: object, .. }
        | Expr::Unary { expr: object, .. }
        | Expr::IsType { expr: object, .. } => {
            kill_mutated_call_arguments(object, state, resolution, mutations)
        }
        Expr::MethodCall {
            object,
            method,
            args,
            ..
        } => {
            kill_mutated_call_arguments(object, state, resolution, mutations);
            kill_calls_in_arguments(args, state, resolution, mutations);
            kill_arguments_for_modes(args, mutations.method_modes(method), state, resolution);
        }
        Expr::FunctionCall { name, args, .. } => {
            kill_calls_in_arguments(args, state, resolution, mutations);
            kill_arguments_for_modes(args, mutations.function_modes(name), state, resolution);
        }
        Expr::StaticCall {
            qualifier,
            method,
            args,
            ..
        } => {
            kill_calls_in_arguments(args, state, resolution, mutations);
            kill_arguments_for_modes(
                args,
                mutations.static_method_modes(qualifier, method),
                state,
                resolution,
            );
        }
        Expr::New {
            class_name, args, ..
        } => {
            kill_calls_in_arguments(args, state, resolution, mutations);
            kill_arguments_for_modes(
                args,
                mutations.constructor_modes(class_name),
                state,
                resolution,
            );
        }
        Expr::InterpolatedString { parts, .. } => {
            for part in parts {
                if let crate::ast::InterpolatedStringPart::Expr(expr) = part {
                    kill_mutated_call_arguments(expr, state, resolution, mutations);
                }
            }
        }
        Expr::Array { elements, .. } => {
            for element in elements {
                if let Some(key) = &element.key {
                    kill_mutated_call_arguments(key, state, resolution, mutations);
                }
                kill_mutated_call_arguments(&element.value, state, resolution, mutations);
            }
        }
        Expr::Binary { left, right, .. }
        | Expr::Range {
            start: left,
            end: right,
            ..
        } => {
            kill_mutated_call_arguments(left, state, resolution, mutations);
            kill_mutated_call_arguments(right, state, resolution, mutations);
        }
        Expr::Variable { .. }
        | Expr::This { .. }
        | Expr::Identifier { .. }
        | Expr::String { .. }
        | Expr::Int { .. }
        | Expr::Float { .. }
        | Expr::Bool { .. }
        | Expr::Null { .. }
        | Expr::StaticMember { .. } => {}
    }
}

fn kill_calls_in_arguments(
    args: &[Expr],
    state: &mut State,
    resolution: &Resolution,
    mutations: &MutationCatalog,
) {
    for argument in args {
        kill_mutated_call_arguments(argument, state, resolution, mutations);
    }
}

fn kill_arguments_for_modes(
    args: &[Expr],
    modes: Option<&[bool]>,
    state: &mut State,
    resolution: &Resolution,
) {
    let Some(modes) = modes else {
        return;
    };
    for (argument, mutable) in args.iter().zip(modes) {
        if *mutable {
            kill_target(argument, state, resolution);
        }
    }
}

fn set_from_value(binding: BindingId, value: &Expr, state: &mut State) {
    let fact = match ungroup(value) {
        Expr::Null { .. } => Some(Fact::Null),
        Expr::String { .. }
        | Expr::Int { .. }
        | Expr::Float { .. }
        | Expr::Bool { .. }
        | Expr::New { .. } => Some(Fact::NonNull),
        _ => None,
    };
    if let Some(fact) = fact {
        state.facts.insert(binding, fact);
    } else {
        state.facts.remove(&binding);
    }
}

fn apply_condition(condition: &Expr, truth: bool, state: &mut State, resolution: &Resolution) {
    match ungroup(condition) {
        Expr::Unary {
            op: crate::ast::UnaryOp::Not,
            expr,
            ..
        } => apply_condition(expr, !truth, state, resolution),
        Expr::Binary {
            left,
            op: crate::ast::BinaryOp::And,
            right,
            ..
        } if truth => {
            apply_condition(left, true, state, resolution);
            apply_condition(right, true, state, resolution);
        }
        Expr::Binary {
            left,
            op: crate::ast::BinaryOp::Or,
            right,
            ..
        } if !truth => {
            apply_condition(left, false, state, resolution);
            apply_condition(right, false, state, resolution);
        }
        Expr::Binary {
            left,
            op: crate::ast::BinaryOp::Equal | crate::ast::BinaryOp::NotEqual,
            right,
            ..
        } => {
            let equality = matches!(
                ungroup(condition),
                Expr::Binary {
                    op: crate::ast::BinaryOp::Equal,
                    ..
                }
            );
            let non_null = truth != equality;
            let variable = match (ungroup(left), ungroup(right)) {
                (Expr::Variable { .. }, Expr::Null { .. }) => variable_binding(left, resolution),
                (Expr::Null { .. }, Expr::Variable { .. }) => variable_binding(right, resolution),
                _ => None,
            };
            if let Some(variable) = variable {
                state
                    .facts
                    .insert(variable, if non_null { Fact::NonNull } else { Fact::Null });
            }
        }
        Expr::IsType { expr, ty, .. } if truth => {
            if let Some(variable) = variable_binding(expr, resolution) {
                state.facts.insert(variable, Fact::Exact(ty.clone()));
            }
        }
        _ => {}
    }
}

fn collect_action_facts(
    action: &NodeAction,
    state: &State,
    resolution: &Resolution,
    mutations: &MutationCatalog,
    facts: &mut FactsByUse,
) {
    match action {
        NodeAction::Statement(statement) => {
            collect_statement(statement, state, resolution, mutations, facts)
        }
        NodeAction::Expression(expression)
        | NodeAction::Assume {
            condition: expression,
            ..
        } => {
            collect_expr(expression, state, resolution, mutations, facts);
        }
        NodeAction::ForInitializer(initializer) => match initializer {
            ForInitializer::VarDecl(declaration) => {
                collect_expr(
                    &declaration.initializer,
                    state,
                    resolution,
                    mutations,
                    facts,
                );
            }
            ForInitializer::Assignment(assignment) => {
                let state = collect_expr(&assignment.target, state, resolution, mutations, facts);
                collect_expr(&assignment.value, &state, resolution, mutations, facts);
            }
        },
        NodeAction::ForIncrement(increment) => match increment {
            ForIncrement::Increment(increment) => {
                collect_expr(&increment.target, state, resolution, mutations, facts);
            }
            ForIncrement::Assignment(assignment) => {
                let state = collect_expr(&assignment.target, state, resolution, mutations, facts);
                collect_expr(&assignment.value, &state, resolution, mutations, facts);
            }
        },
        NodeAction::None => {}
    }
}

fn collect_statement(
    statement: &Stmt,
    state: &State,
    resolution: &Resolution,
    mutations: &MutationCatalog,
    facts: &mut FactsByUse,
) {
    match statement {
        Stmt::VarDecl(declaration) => {
            collect_expr(
                &declaration.initializer,
                state,
                resolution,
                mutations,
                facts,
            );
        }
        Stmt::Assignment(assignment) => {
            let state = collect_expr(&assignment.target, state, resolution, mutations, facts);
            collect_expr(&assignment.value, &state, resolution, mutations, facts);
        }
        Stmt::Echo { expr, .. } | Stmt::Expr { expr, .. } => {
            collect_expr(expr, state, resolution, mutations, facts);
        }
        Stmt::Return { expr, .. } => {
            if let Some(expr) = expr {
                collect_expr(expr, state, resolution, mutations, facts);
            }
        }
        Stmt::Increment(increment) => {
            collect_expr(&increment.target, state, resolution, mutations, facts);
        }
        Stmt::If(_)
        | Stmt::While(_)
        | Stmt::For(_)
        | Stmt::Foreach(_)
        | Stmt::Break { .. }
        | Stmt::Continue { .. } => {}
    }
}

fn collect_expr(
    expr: &Expr,
    state: &State,
    resolution: &Resolution,
    mutations: &MutationCatalog,
    facts: &mut FactsByUse,
) -> State {
    if let Expr::Variable { span, .. } = expr {
        if let Some(binding) = resolution.uses.get(&(span.start, span.end)) {
            if let Some(fact) = state.facts.get(binding) {
                facts.insert((span.start, span.end), fact.clone());
            }
        }
        return state.clone();
    }

    match expr {
        Expr::PropertyAccess { object, .. }
        | Expr::Grouped { expr: object, .. }
        | Expr::Unary { expr: object, .. }
        | Expr::IsType { expr: object, .. } => {
            collect_expr(object, state, resolution, mutations, facts)
        }
        Expr::MethodCall {
            object,
            method,
            args,
            ..
        } => {
            let state = collect_expr(object, state, resolution, mutations, facts);
            let mut state = collect_expr_sequence(args, &state, resolution, mutations, facts);
            kill_arguments_for_modes(args, mutations.method_modes(method), &mut state, resolution);
            state
        }
        Expr::FunctionCall { name, args, .. } => {
            let mut state = collect_expr_sequence(args, state, resolution, mutations, facts);
            kill_arguments_for_modes(args, mutations.function_modes(name), &mut state, resolution);
            state
        }
        Expr::StaticCall {
            qualifier,
            method,
            args,
            ..
        } => {
            let mut state = collect_expr_sequence(args, state, resolution, mutations, facts);
            kill_arguments_for_modes(
                args,
                mutations.static_method_modes(qualifier, method),
                &mut state,
                resolution,
            );
            state
        }
        Expr::New {
            class_name, args, ..
        } => {
            let mut state = collect_expr_sequence(args, state, resolution, mutations, facts);
            kill_arguments_for_modes(
                args,
                mutations.constructor_modes(class_name),
                &mut state,
                resolution,
            );
            state
        }
        Expr::InterpolatedString { parts, .. } => {
            let mut state = state.clone();
            for part in parts {
                if let crate::ast::InterpolatedStringPart::Expr(expr) = part {
                    state = collect_expr(expr, &state, resolution, mutations, facts);
                }
            }
            state
        }
        Expr::Array { elements, .. } => {
            let mut state = state.clone();
            for element in elements {
                if let Some(key) = &element.key {
                    state = collect_expr(key, &state, resolution, mutations, facts);
                }
                state = collect_expr(&element.value, &state, resolution, mutations, facts);
            }
            state
        }
        Expr::Binary {
            left,
            op: crate::ast::BinaryOp::And,
            right,
            ..
        } => {
            collect_expr(left, state, resolution, mutations, facts);
            let mut right_state = state.clone();
            apply_condition_with_effects(left, true, &mut right_state, resolution, mutations);
            collect_expr(right, &right_state, resolution, mutations, facts);
            let mut result = state.clone();
            kill_mutated_call_arguments(expr, &mut result, resolution, mutations);
            result
        }
        Expr::Binary {
            left,
            op: crate::ast::BinaryOp::Or,
            right,
            ..
        } => {
            collect_expr(left, state, resolution, mutations, facts);
            let mut right_state = state.clone();
            apply_condition_with_effects(left, false, &mut right_state, resolution, mutations);
            collect_expr(right, &right_state, resolution, mutations, facts);
            let mut result = state.clone();
            kill_mutated_call_arguments(expr, &mut result, resolution, mutations);
            result
        }
        Expr::Binary { left, right, .. }
        | Expr::Range {
            start: left,
            end: right,
            ..
        } => {
            let state = collect_expr(left, state, resolution, mutations, facts);
            collect_expr(right, &state, resolution, mutations, facts)
        }
        Expr::This { .. }
        | Expr::Identifier { .. }
        | Expr::String { .. }
        | Expr::Int { .. }
        | Expr::Float { .. }
        | Expr::Bool { .. }
        | Expr::Null { .. }
        | Expr::StaticMember { .. }
        | Expr::Variable { .. } => state.clone(),
    }
}

fn collect_expr_sequence(
    expressions: &[Expr],
    state: &State,
    resolution: &Resolution,
    mutations: &MutationCatalog,
    facts: &mut FactsByUse,
) -> State {
    expressions.iter().fold(state.clone(), |state, expression| {
        collect_expr(expression, &state, resolution, mutations, facts)
    })
}

fn variable_binding(expr: &Expr, resolution: &Resolution) -> Option<BindingId> {
    let Expr::Variable { span, .. } = ungroup(expr) else {
        return None;
    };
    resolution.uses.get(&(span.start, span.end)).copied()
}

fn ungroup(mut expr: &Expr) -> &Expr {
    while let Expr::Grouped { expr: inner, .. } = expr {
        expr = inner;
    }
    expr
}

struct Resolver {
    next_binding: usize,
    scopes: Vec<HashMap<String, BindingId>>,
    resolution: Resolution,
}

impl Resolver {
    fn resolve(body: &Block, params: &[Param]) -> Resolution {
        let mut resolver = Self {
            next_binding: 0,
            scopes: vec![HashMap::new()],
            resolution: Resolution::default(),
        };
        for parameter in params {
            resolver.declare(&parameter.name, parameter.span.start);
        }
        resolver.resolve_statements(&body.statements);
        resolver.resolution
    }

    fn declare(&mut self, name: &str, span_start: usize) -> BindingId {
        let id = BindingId(self.next_binding);
        self.next_binding += 1;
        self.scopes
            .last_mut()
            .expect("scope")
            .insert(name.to_string(), id);
        self.resolution.declarations.insert(span_start, id);
        id
    }

    fn resolve_statements(&mut self, statements: &[Stmt]) {
        for statement in statements {
            self.resolve_statement(statement);
        }
    }

    fn resolve_block(&mut self, block: &Block) {
        self.scopes.push(HashMap::new());
        self.resolve_statements(&block.statements);
        self.scopes.pop();
    }

    fn resolve_statement(&mut self, statement: &Stmt) {
        match statement {
            Stmt::VarDecl(declaration) => {
                self.resolve_expr(&declaration.initializer);
                self.declare(&declaration.name, declaration.span.start);
            }
            Stmt::Assignment(assignment) => {
                self.resolve_expr(&assignment.target);
                self.resolve_expr(&assignment.value);
            }
            Stmt::Echo { expr, .. } | Stmt::Expr { expr, .. } => self.resolve_expr(expr),
            Stmt::Return { expr, .. } => {
                if let Some(expr) = expr {
                    self.resolve_expr(expr);
                }
            }
            Stmt::If(statement) => {
                self.resolve_expr(&statement.condition);
                self.resolve_block(&statement.then_block);
                if let Some(branch) = &statement.else_branch {
                    match branch {
                        ElseBranch::If(statement) => {
                            self.resolve_statement(&Stmt::If((**statement).clone()))
                        }
                        ElseBranch::Block(block) => self.resolve_block(block),
                    }
                }
            }
            Stmt::While(statement) => {
                self.resolve_expr(&statement.condition);
                self.resolve_block(&statement.body);
            }
            Stmt::For(statement) => {
                self.scopes.push(HashMap::new());
                if let Some(initializer) = &statement.initializer {
                    match initializer {
                        ForInitializer::VarDecl(declaration) => {
                            self.resolve_expr(&declaration.initializer);
                            self.declare(&declaration.name, declaration.span.start);
                        }
                        ForInitializer::Assignment(assignment) => {
                            self.resolve_expr(&assignment.target);
                            self.resolve_expr(&assignment.value);
                        }
                    }
                }
                if let Some(condition) = &statement.condition {
                    self.resolve_expr(condition);
                }
                self.resolve_block(&statement.body);
                if let Some(increment) = &statement.increment {
                    match increment {
                        ForIncrement::Increment(increment) => self.resolve_expr(&increment.target),
                        ForIncrement::Assignment(assignment) => {
                            self.resolve_expr(&assignment.target);
                            self.resolve_expr(&assignment.value);
                        }
                    }
                }
                self.scopes.pop();
            }
            Stmt::Foreach(statement) => {
                self.resolve_expr(&statement.iterable);
                self.scopes.push(HashMap::new());
                if let Some(key) = &statement.key {
                    self.declare(&key.name, statement.span.start);
                }
                self.declare(
                    &statement.value.name,
                    statement.span.start.saturating_add(1),
                );
                self.resolve_statements(&statement.body.statements);
                self.scopes.pop();
            }
            Stmt::Increment(increment) => self.resolve_expr(&increment.target),
            Stmt::Break { .. } | Stmt::Continue { .. } => {}
        }
    }

    fn resolve_expr(&mut self, expr: &Expr) {
        if let Expr::Variable { name, span } = expr {
            if let Some(binding) = self
                .scopes
                .iter()
                .rev()
                .find_map(|scope| scope.get(name))
                .copied()
            {
                self.resolution.uses.insert((span.start, span.end), binding);
            }
            return;
        }
        match expr {
            Expr::PropertyAccess { object, .. }
            | Expr::Grouped { expr: object, .. }
            | Expr::Unary { expr: object, .. }
            | Expr::IsType { expr: object, .. } => self.resolve_expr(object),
            Expr::MethodCall { object, args, .. } => {
                self.resolve_expr(object);
                for argument in args {
                    self.resolve_expr(argument);
                }
            }
            Expr::FunctionCall { args, .. }
            | Expr::StaticCall { args, .. }
            | Expr::New { args, .. } => {
                for argument in args {
                    self.resolve_expr(argument);
                }
            }
            Expr::InterpolatedString { parts, .. } => {
                for part in parts {
                    if let crate::ast::InterpolatedStringPart::Expr(expr) = part {
                        self.resolve_expr(expr);
                    }
                }
            }
            Expr::Array { elements, .. } => {
                for element in elements {
                    if let Some(key) = &element.key {
                        self.resolve_expr(key);
                    }
                    self.resolve_expr(&element.value);
                }
            }
            Expr::Binary { left, right, .. }
            | Expr::Range {
                start: left,
                end: right,
                ..
            } => {
                self.resolve_expr(left);
                self.resolve_expr(right);
            }
            Expr::This { .. }
            | Expr::Identifier { .. }
            | Expr::String { .. }
            | Expr::Int { .. }
            | Expr::Float { .. }
            | Expr::Bool { .. }
            | Expr::Null { .. }
            | Expr::StaticMember { .. }
            | Expr::Variable { .. } => {}
        }
    }
}

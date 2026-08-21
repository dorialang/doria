use std::collections::HashMap;

use crate::ast::{
    AssignOp, ClassDecl, ClassMember, Expr, ForIncrement, ForInitializer, InterpolatedStringPart,
    Item, Program, Stmt,
};
use crate::control_flow::{
    build_function_cfg_with_checked_effects, GivenSemanticInfoMap, Node, NodeAction, NodeKind,
};
use crate::dataflow::{solve_forward, ForwardAnalysis};
use crate::diagnostics::Diagnostic;
use crate::semantics::PropertyWriteKind;
use crate::source::Span;

#[derive(Debug, Default)]
pub(crate) struct Analysis {
    pub diagnostics: Vec<Diagnostic>,
    pub property_writes: HashMap<(usize, usize), PropertyWriteKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InitState {
    Uninitialized,
    Initialized,
    MaybeInitialized,
}

impl InitState {
    fn join(self, other: Self) -> Self {
        if self == other {
            self
        } else {
            Self::MaybeInitialized
        }
    }
}

#[derive(Debug, Clone)]
struct Property {
    name: String,
    writable: bool,
    preinitialized: bool,
}

struct AssignmentSite<'a> {
    property: &'a str,
    operation: &'a AssignOp,
    span: Span,
    repeatable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct State {
    reachable: bool,
    properties: Vec<InitState>,
}

impl State {
    fn bottom(property_count: usize) -> Self {
        Self {
            reachable: false,
            properties: vec![InitState::Uninitialized; property_count],
        }
    }

    fn join(&mut self, incoming: &Self) -> bool {
        if !incoming.reachable {
            return false;
        }
        if !self.reachable {
            *self = incoming.clone();
            return true;
        }
        let joined = self
            .properties
            .iter()
            .zip(&incoming.properties)
            .map(|(current, incoming)| current.join(*incoming))
            .collect::<Vec<_>>();
        if joined == self.properties {
            return false;
        }
        self.properties = joined;
        true
    }
}

struct ConstructorAnalysis<'a> {
    properties: &'a [Property],
    entry: State,
}

impl ForwardAnalysis for ConstructorAnalysis<'_> {
    type State = State;

    fn bottom(&self) -> Self::State {
        State::bottom(self.properties.len())
    }

    fn entry_state(&self) -> Self::State {
        self.entry.clone()
    }

    fn transfer(&self, node: &Node, input: &Self::State) -> Self::State {
        let mut output = input.clone();
        if output.reachable {
            transfer_action(self.properties, &node.action, &mut output);
        }
        output
    }

    fn join(&self, state: &mut Self::State, incoming: &Self::State) -> bool {
        state.join(incoming)
    }
}

pub(crate) fn check_program(
    program: &Program,
    given_preludes: &GivenSemanticInfoMap,
    checked_effect_sites: &crate::checked_effects::EffectSiteMap,
    catch_error_types: &crate::checked_effects::CatchTypeMap,
) -> Analysis {
    let mut analysis = Analysis::default();
    for class in program.items.iter().filter_map(|item| match item {
        Item::Class(class) => Some(class),
        _ => None,
    }) {
        check_class(
            class,
            given_preludes,
            checked_effect_sites,
            catch_error_types,
            &mut analysis,
        );
    }
    analysis
}

fn check_class(
    class: &ClassDecl,
    given_preludes: &GivenSemanticInfoMap,
    checked_effect_sites: &crate::checked_effects::EffectSiteMap,
    catch_error_types: &crate::checked_effects::CatchTypeMap,
    analysis: &mut Analysis,
) {
    let mut properties = class
        .members
        .iter()
        .filter_map(|member| match member {
            ClassMember::Property(property) if !property.is_static => Some(Property {
                name: property.name.clone(),
                writable: property.writable,
                preinitialized: property.initializer.is_some(),
            }),
            _ => None,
        })
        .collect::<Vec<_>>();
    let constructor = class.members.iter().find_map(|member| match member {
        ClassMember::Method(method) if method.name == "__construct" => Some(method),
        _ => None,
    });
    if let Some(constructor) = constructor {
        properties.extend(constructor.params.iter().filter_map(|parameter| {
            parameter.promoted_access.as_ref().map(|_| Property {
                name: parameter.name.clone(),
                writable: parameter.writable,
                preinitialized: true,
            })
        }));
    }
    if properties.is_empty() {
        return;
    }

    let entry = State {
        reachable: true,
        properties: properties
            .iter()
            .map(|property| {
                if property.preinitialized {
                    InitState::Initialized
                } else {
                    InitState::Uninitialized
                }
            })
            .collect(),
    };
    let Some(constructor) = constructor else {
        report_incomplete_exit(
            class,
            &properties,
            &entry,
            class.span,
            "implicit constructor",
            &mut analysis.diagnostics,
        );
        return;
    };

    let graph = build_function_cfg_with_checked_effects(
        &constructor.body,
        constructor.span,
        given_preludes,
        checked_effect_sites,
        catch_error_types,
    );
    let result = solve_forward(
        &graph,
        &ConstructorAnalysis {
            properties: &properties,
            entry,
        },
    );
    for node in &graph.nodes {
        let state = &result.inputs[node.id.0];
        if !state.reachable {
            continue;
        }
        inspect_action(class, &properties, state, node, analysis);
        match node.kind {
            NodeKind::ReturnExit => report_incomplete_exit(
                class,
                &properties,
                state,
                node.span,
                "explicit return",
                &mut analysis.diagnostics,
            ),
            NodeKind::FallthroughExit => report_incomplete_exit(
                class,
                &properties,
                state,
                constructor.body.span,
                "constructor fallthrough",
                &mut analysis.diagnostics,
            ),
            _ => {}
        }
    }
}

fn transfer_action(properties: &[Property], action: &NodeAction, state: &mut State) {
    match action {
        NodeAction::Statement(Stmt::Assignment(assignment)) => {
            transfer_assignment(properties, state, &assignment.target, &assignment.op)
        }
        NodeAction::ForInitializer(ForInitializer::Assignment(assignment))
        | NodeAction::ForIncrement(ForIncrement::Assignment(assignment)) => {
            transfer_assignment(properties, state, &assignment.target, &assignment.op)
        }
        _ => {}
    }
}

fn transfer_assignment(
    properties: &[Property],
    state: &mut State,
    target: &Expr,
    operation: &AssignOp,
) {
    if !matches!(operation, AssignOp::Assign) {
        return;
    }
    let Some((property, _)) = direct_this_property(target) else {
        return;
    };
    let Some(index) = property_index(properties, property) else {
        return;
    };
    state.properties[index] = if properties[index].writable {
        InitState::Initialized
    } else {
        match state.properties[index] {
            InitState::Uninitialized => InitState::Initialized,
            current => current,
        }
    };
}

fn inspect_action(
    class: &ClassDecl,
    properties: &[Property],
    input: &State,
    node: &Node,
    analysis: &mut Analysis,
) {
    let mut state = input.clone();
    match &node.action {
        NodeAction::None | NodeAction::Assume { .. } => {}
        NodeAction::Expression(expression) => {
            inspect_expr(class, properties, &state, expression, analysis)
        }
        NodeAction::Statement(statement) => inspect_statement(
            class,
            properties,
            &mut state,
            statement,
            node.repeatable,
            analysis,
        ),
        NodeAction::ForInitializer(initializer) => match initializer {
            ForInitializer::VarDecl(declaration) => inspect_expr(
                class,
                properties,
                &state,
                &declaration.initializer,
                analysis,
            ),
            ForInitializer::Assignment(assignment) => {
                inspect_assignment(class, properties, &mut state, assignment, false, analysis)
            }
        },
        NodeAction::ForIncrement(increment) => match increment {
            ForIncrement::Increment(increment) => inspect_increment(
                class,
                properties,
                &mut state,
                &increment.target,
                increment.span,
                true,
                analysis,
            ),
            ForIncrement::Assignment(assignment) => {
                inspect_assignment(class, properties, &mut state, assignment, true, analysis)
            }
        },
    }
}

fn inspect_statement(
    class: &ClassDecl,
    properties: &[Property],
    state: &mut State,
    statement: &Stmt,
    repeatable: bool,
    analysis: &mut Analysis,
) {
    match statement {
        Stmt::Block(block) => {
            for statement in &block.statements {
                inspect_statement(class, properties, state, statement, repeatable, analysis);
            }
        }
        Stmt::VarDecl(declaration) => {
            inspect_expr(class, properties, state, &declaration.initializer, analysis)
        }
        Stmt::Assignment(assignment) => {
            inspect_assignment(class, properties, state, assignment, repeatable, analysis)
        }
        Stmt::Echo { expr, .. } | Stmt::Expr { expr, .. } => {
            inspect_expr(class, properties, state, expr, analysis)
        }
        Stmt::Return { expr, .. } => {
            if let Some(expr) = expr {
                inspect_expr(class, properties, state, expr, analysis);
            }
        }
        Stmt::Throw(statement) => inspect_expr(class, properties, state, &statement.expr, analysis),
        Stmt::Increment(increment) => inspect_increment(
            class,
            properties,
            state,
            &increment.target,
            increment.span,
            repeatable,
            analysis,
        ),
        Stmt::If(_)
        | Stmt::Try(_)
        | Stmt::While(_)
        | Stmt::DoWhile(_)
        | Stmt::For(_)
        | Stmt::Foreach(_)
        | Stmt::Break { .. }
        | Stmt::Continue { .. } => {}
    }
}

fn inspect_assignment(
    class: &ClassDecl,
    properties: &[Property],
    state: &mut State,
    assignment: &crate::ast::Assignment,
    repeatable: bool,
    analysis: &mut Analysis,
) {
    inspect_expr(class, properties, state, &assignment.value, analysis);
    if let Some((property, span)) = direct_this_property(&assignment.target) {
        apply_assignment(
            class,
            properties,
            state,
            AssignmentSite {
                property,
                operation: &assignment.op,
                span,
                repeatable,
            },
            analysis,
        );
    } else {
        inspect_expr(class, properties, state, &assignment.target, analysis);
    }
}

fn inspect_increment(
    class: &ClassDecl,
    properties: &[Property],
    state: &mut State,
    target: &Expr,
    span: Span,
    repeatable: bool,
    analysis: &mut Analysis,
) {
    if let Some((property, property_span)) = direct_this_property(target) {
        apply_assignment(
            class,
            properties,
            state,
            AssignmentSite {
                property,
                operation: &AssignOp::AddAssign,
                span: property_span,
                repeatable,
            },
            analysis,
        );
    } else {
        let _ = span;
        inspect_expr(class, properties, state, target, analysis);
    }
}

fn apply_assignment(
    class: &ClassDecl,
    properties: &[Property],
    state: &mut State,
    site: AssignmentSite<'_>,
    analysis: &mut Analysis,
) {
    let Some(index) = property_index(properties, site.property) else {
        return;
    };
    let property = &properties[index];
    if !matches!(site.operation, AssignOp::Assign) {
        analysis
            .property_writes
            .insert((site.span.start, site.span.end), PropertyWriteKind::Replace);
        observe_property(
            class,
            properties,
            state,
            index,
            site.span,
            &mut analysis.diagnostics,
        );
        return;
    }
    let write_kind = match state.properties[index] {
        InitState::Uninitialized => PropertyWriteKind::Initialize,
        InitState::Initialized => PropertyWriteKind::Replace,
        InitState::MaybeInitialized => PropertyWriteKind::InitializeOrReplace,
    };
    analysis
        .property_writes
        .insert((site.span.start, site.span.end), write_kind);
    if property.writable {
        state.properties[index] = InitState::Initialized;
        return;
    }
    if site.repeatable {
        return;
    }
    match state.properties[index] {
        InitState::Uninitialized => state.properties[index] = InitState::Initialized,
        InitState::Initialized => analysis.diagnostics.push(Diagnostic::new(
            "E0412",
            format!(
                "readonly property `{}::{}` is already initialized on this constructor path",
                class.name, site.property
            ),
            site.span,
        )),
        InitState::MaybeInitialized => analysis.diagnostics.push(
            Diagnostic::new(
                "E0502",
                format!(
                    "readonly property `{}::{}` is initialized on only some incoming paths, so this assignment would initialize it twice on another path",
                    class.name, site.property
                ),
                site.span,
            )
            .with_help("initialize the readonly property exactly once in every branch"),
        ),
    }
}

fn inspect_expr(
    class: &ClassDecl,
    properties: &[Property],
    state: &State,
    expression: &Expr,
    analysis: &mut Analysis,
) {
    match expression {
        Expr::This { span } => {
            report_incomplete_this(class, properties, state, *span, &mut analysis.diagnostics)
        }
        Expr::PropertyAccess {
            object,
            property,
            span,
            ..
        } if is_this(object) => {
            if let Some(index) = property_index(properties, property) {
                observe_property(
                    class,
                    properties,
                    state,
                    index,
                    *span,
                    &mut analysis.diagnostics,
                );
            }
        }
        Expr::PropertyAccess { object, .. }
        | Expr::Grouped { expr: object, .. }
        | Expr::Unary { expr: object, .. } => {
            inspect_expr(class, properties, state, object, analysis)
        }
        Expr::MethodCall {
            object, args, span, ..
        } => {
            if is_this(object) {
                report_incomplete_this(class, properties, state, *span, &mut analysis.diagnostics);
            } else {
                inspect_expr(class, properties, state, object, analysis);
            }
            for argument in args {
                inspect_expr(class, properties, state, &argument.value, analysis);
            }
        }
        Expr::FunctionCall { args, .. }
        | Expr::StaticCall { args, .. }
        | Expr::New { args, .. } => {
            for argument in args {
                inspect_expr(class, properties, state, &argument.value, analysis);
            }
        }
        Expr::InterpolatedString { parts, .. } => {
            for part in parts {
                if let InterpolatedStringPart::Expr(expression) = part {
                    inspect_expr(class, properties, state, expression, analysis);
                }
            }
        }
        Expr::Array { elements, .. } => {
            for element in elements {
                if let Some(key) = &element.key {
                    inspect_expr(class, properties, state, key, analysis);
                }
                inspect_expr(class, properties, state, &element.value, analysis);
            }
        }
        Expr::ArrayRepeat { value, count, .. } => {
            inspect_expr(class, properties, state, value, analysis);
            inspect_expr(class, properties, state, count, analysis);
        }
        Expr::Index {
            collection, index, ..
        } => {
            inspect_expr(class, properties, state, collection, analysis);
            inspect_expr(class, properties, state, index, analysis);
        }
        Expr::IsType { expr, .. } => inspect_expr(class, properties, state, expr, analysis),
        Expr::Binary {
            left,
            op: crate::ast::BinaryOp::And,
            ..
        } if constant_bool(left) == Some(false) => {
            inspect_expr(class, properties, state, left, analysis)
        }
        Expr::Binary {
            left,
            op: crate::ast::BinaryOp::Or,
            ..
        } if constant_bool(left) == Some(true) => {
            inspect_expr(class, properties, state, left, analysis)
        }
        Expr::Binary { left, right, .. }
        | Expr::Range {
            start: left,
            end: right,
            ..
        } => {
            inspect_expr(class, properties, state, left, analysis);
            inspect_expr(class, properties, state, right, analysis);
        }
        Expr::When(when) => {
            let mut nested = state.clone();
            if let Some(given) = &when.given {
                for statement in &given.block.statements {
                    inspect_statement(class, properties, &mut nested, statement, false, analysis);
                }
            }
            for branch in &when.branches {
                if let Some(condition) = &branch.condition {
                    inspect_expr(class, properties, &nested, condition, analysis);
                }
                let mut branch_state = nested.clone();
                for statement in &branch.block.statements {
                    inspect_statement(
                        class,
                        properties,
                        &mut branch_state,
                        statement,
                        false,
                        analysis,
                    );
                }
            }
            if let Some(finally) = &when.finally {
                let mut final_state = nested;
                for statement in &finally.block.statements {
                    inspect_statement(
                        class,
                        properties,
                        &mut final_state,
                        statement,
                        false,
                        analysis,
                    );
                }
            }
        }
        Expr::Variable { .. }
        | Expr::Closure(_)
        | Expr::CallableCall { .. }
        | Expr::Identifier { .. }
        | Expr::String { .. }
        | Expr::Int { .. }
        | Expr::Float { .. }
        | Expr::Bool { .. }
        | Expr::Null { .. }
        | Expr::Match { .. }
        | Expr::StaticMember { .. } => {}
    }
}

fn observe_property(
    class: &ClassDecl,
    properties: &[Property],
    state: &State,
    index: usize,
    span: Span,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let property = &properties[index];
    match state.properties[index] {
        InitState::Initialized => {}
        InitState::Uninitialized => diagnostics.push(Diagnostic::new(
            "E0501",
            format!(
                "property `{}::{}` is read before it is initialized",
                class.name, property.name
            ),
            span,
        )),
        InitState::MaybeInitialized => diagnostics.push(Diagnostic::new(
            "E0501",
            format!(
                "property `{}::{}` may be read on a path where it is not initialized",
                class.name, property.name
            ),
            span,
        )),
    }
}

fn report_incomplete_this(
    class: &ClassDecl,
    properties: &[Property],
    state: &State,
    span: Span,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let missing = properties
        .iter()
        .zip(&state.properties)
        .filter_map(|(property, state)| {
            (*state != InitState::Initialized).then_some(format!("`${}`", property.name))
        })
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        diagnostics.push(
            Diagnostic::new(
                "E0503",
                format!(
                    "`$this` cannot be observed or passed from `{}::__construct` before {} {} initialized",
                    class.name,
                    missing.join(", "),
                    if missing.len() == 1 { "is" } else { "are" }
                ),
                span,
            )
            .with_help("initialize every property before exposing the object under construction"),
        );
    }
}

fn report_incomplete_exit(
    class: &ClassDecl,
    properties: &[Property],
    state: &State,
    span: Span,
    exit: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for (property, init) in properties.iter().zip(&state.properties) {
        match init {
            InitState::Initialized => {}
            InitState::Uninitialized => diagnostics.push(Diagnostic::new(
                "E0500",
                format!(
                    "property `{}::{}` is not initialized before {exit} completes",
                    class.name, property.name
                ),
                span,
            )),
            InitState::MaybeInitialized => diagnostics.push(Diagnostic::new(
                "E0500",
                format!(
                    "property `{}::{}` is not initialized on every path before {exit} completes",
                    class.name, property.name
                ),
                span,
            )),
        }
    }
}

fn direct_this_property(expression: &Expr) -> Option<(&str, Span)> {
    match expression {
        Expr::Grouped { expr, .. } => direct_this_property(expr),
        Expr::PropertyAccess {
            object,
            property,
            span,
            ..
        } if is_this(object) => Some((property, *span)),
        _ => None,
    }
}

fn is_this(expression: &Expr) -> bool {
    match expression {
        Expr::This { .. } => true,
        Expr::Grouped { expr, .. } => is_this(expr),
        _ => false,
    }
}

fn constant_bool(expression: &Expr) -> Option<bool> {
    match expression {
        Expr::Bool { value, .. } => Some(*value),
        Expr::Grouped { expr, .. } => constant_bool(expr),
        Expr::Unary {
            op: crate::ast::UnaryOp::Not,
            expr,
            ..
        } => constant_bool(expr).map(|value| !value),
        _ => None,
    }
}

fn property_index(properties: &[Property], name: &str) -> Option<usize> {
    properties.iter().position(|property| property.name == name)
}

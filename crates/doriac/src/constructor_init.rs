use std::collections::{HashMap, HashSet};

use crate::ast::{
    AssignOp, ClassDecl, ClassMember, Expr, ForIncrement, ForInitializer, InterpolatedStringPart,
    Item, Program, Stmt,
};
use crate::control_flow::{
    build_function_cfg_with_checked_effects, GivenSemanticInfoMap, Node, NodeAction, NodeKind,
};
use crate::dataflow::{solve_forward, ForwardAnalysis};
use crate::diagnostics::Diagnostic;
use crate::property_hooks::{declaration_facts, PropertyHookContext, PropertyHookStorage};
use crate::semantics::PropertyWriteKind;
use crate::source::Span;

#[derive(Debug, Default)]
pub(crate) struct Analysis {
    pub diagnostics: Vec<Diagnostic>,
    pub property_writes: HashMap<Span, PropertyWriteKind>,
    pub partial_cleanups: Vec<ConstructorPartialCleanup>,
}

/// Resolved inheritance/storage facts, supplied by member selection rather than
/// reconstructed from source names by constructor dataflow.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConstructorCleanupContext {
    pub parent_class: Option<Span>,
    pub parent_constructor: Option<Span>,
    pub inherited_stored_properties: Vec<Span>,
}

/// Fields which may have been initialized when construction exits through a
/// checked error. These are canonical backing declarations, not accessor views.
/// Consumers use checked property ownership/type facts to select owned payloads;
/// failure never invokes the incomplete object's own destructor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstructorPartialCleanup {
    pub class: Span,
    pub constructor: Option<Span>,
    pub site: Span,
    pub initialized_properties: Vec<Span>,
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
    declaration: Span,
    writable: bool,
    preinitialized: bool,
    same_named_constructor_only_parameter: bool,
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

#[cfg(test)]
pub(crate) fn check_program(
    program: &Program,
    backing_fields: &HashMap<Span, crate::property_hooks::PropertyBackingField>,
    given_preludes: &GivenSemanticInfoMap,
    checked_effect_sites: &crate::checked_effects::EffectSiteMap,
    catch_error_types: &crate::checked_effects::CatchTypeMap,
    catch_coverage: &crate::checked_effects::CatchCoverageMap,
) -> Analysis {
    check_program_with_cleanup_context(
        program,
        backing_fields,
        given_preludes,
        checked_effect_sites,
        catch_error_types,
        catch_coverage,
        &HashMap::new(),
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn check_program_with_cleanup_context(
    program: &Program,
    backing_fields: &HashMap<Span, crate::property_hooks::PropertyBackingField>,
    given_preludes: &GivenSemanticInfoMap,
    checked_effect_sites: &crate::checked_effects::EffectSiteMap,
    catch_error_types: &crate::checked_effects::CatchTypeMap,
    catch_coverage: &crate::checked_effects::CatchCoverageMap,
    cleanup_contexts: &HashMap<Span, ConstructorCleanupContext>,
) -> Analysis {
    let mut analysis = Analysis::default();
    for class in program.items.iter().filter_map(|item| match item {
        Item::Class(class) => Some(class),
        _ => None,
    }) {
        check_class(
            class,
            backing_fields,
            given_preludes,
            checked_effect_sites,
            catch_error_types,
            catch_coverage,
            cleanup_contexts.get(&class.span),
            &mut analysis,
        );
    }
    inherit_parent_partial_cleanups(program, cleanup_contexts, &mut analysis.partial_cleanups);
    analysis
}

#[allow(clippy::too_many_arguments)]
fn check_class(
    class: &ClassDecl,
    backing_fields: &HashMap<Span, crate::property_hooks::PropertyBackingField>,
    given_preludes: &GivenSemanticInfoMap,
    checked_effect_sites: &crate::checked_effects::EffectSiteMap,
    catch_error_types: &crate::checked_effects::CatchTypeMap,
    catch_coverage: &crate::checked_effects::CatchCoverageMap,
    cleanup_context: Option<&ConstructorCleanupContext>,
    analysis: &mut Analysis,
) {
    let constructor = class.members.iter().find_map(|member| match member {
        ClassMember::Method(method) if method.name == "__construct" => Some(method),
        _ => None,
    });
    let constructor_only_names = constructor
        .into_iter()
        .flat_map(|constructor| &constructor.params)
        .filter(|parameter| parameter.constructor_role.is_constructor_only())
        .map(|parameter| parameter.name.as_str())
        .collect::<HashSet<_>>();
    let mut properties = class
        .members
        .iter()
        .filter_map(|member| match member {
            ClassMember::Property(property)
                if !property.is_static
                    && !backing_fields
                        .get(&property.span)
                        .is_some_and(|field| field.declaration != property.span)
                    && !declaration_facts(property, PropertyHookContext::Class).is_some_and(
                        |facts| facts.storage == Some(PropertyHookStorage::Computed),
                    ) =>
            {
                Some(Property {
                    name: property.name.clone(),
                    declaration: property.span,
                    writable: property.writable,
                    preinitialized: property.initializer.is_some(),
                    same_named_constructor_only_parameter: constructor_only_names
                        .contains(property.name.as_str()),
                })
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if let Some(constructor) = constructor {
        properties.extend(
            constructor
                .params
                .iter()
                .filter(|parameter| parameter.constructor_role.is_promoted())
                .map(|parameter| Property {
                    name: parameter.name.clone(),
                    declaration: parameter.span,
                    writable: parameter.writable,
                    preinitialized: true,
                    same_named_constructor_only_parameter: false,
                }),
        );
    }
    project_partial_cleanups(
        class,
        &properties,
        cleanup_context,
        given_preludes,
        checked_effect_sites,
        catch_error_types,
        catch_coverage,
        &mut analysis.partial_cleanups,
    );
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

    let Some(body) = constructor.body.as_block() else {
        return;
    };
    let graph = build_function_cfg_with_checked_effects(
        body,
        constructor.span,
        given_preludes,
        checked_effect_sites,
        catch_error_types,
        catch_coverage,
        &std::collections::HashSet::new(),
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
                body.span,
                "constructor fallthrough",
                &mut analysis.diagnostics,
            ),
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn project_partial_cleanups(
    class: &ClassDecl,
    properties: &[Property],
    cleanup_context: Option<&ConstructorCleanupContext>,
    given_preludes: &GivenSemanticInfoMap,
    checked_effect_sites: &crate::checked_effects::EffectSiteMap,
    catch_error_types: &crate::checked_effects::CatchTypeMap,
    catch_coverage: &crate::checked_effects::CatchCoverageMap,
    cleanups: &mut Vec<ConstructorPartialCleanup>,
) {
    let constructor = class.members.iter().find_map(|member| match member {
        ClassMember::Method(method) if method.name == "__construct" => Some(method),
        _ => None,
    });
    let mut statements = Vec::new();
    // Construction phases follow declarations, not the physical field list:
    // backed overrides have no new field but their initializer still executes.
    let explicit = class.members.iter().filter_map(|member| match member {
        ClassMember::Property(property)
            if !property.is_static
                && !declaration_facts(property, PropertyHookContext::Class)
                    .is_some_and(|facts| facts.storage == Some(PropertyHookStorage::Computed)) =>
        {
            property
                .initializer
                .clone()
                .map(|value| (property.name.clone(), property.span, value))
        }
        _ => None,
    });
    let promoted = constructor.into_iter().flat_map(|constructor| {
        constructor
            .params
            .iter()
            .filter(|parameter| parameter.constructor_role.is_promoted())
            .map(|parameter| {
                (
                    parameter.name.clone(),
                    parameter.span,
                    Expr::Variable {
                        name: parameter.name.clone(),
                        span: parameter.span,
                    },
                )
            })
    });
    for (name, declaration, value) in explicit.chain(promoted) {
        // These assignments model the existing construction phase, using
        // authored initializer spans so checked effects precede the store.
        statements.push(Stmt::Assignment(crate::ast::Assignment {
            target: Expr::PropertyAccess {
                object: Box::new(Expr::This { span: declaration }),
                property: name,
                member_span: declaration,
                null_safe: false,
                span: declaration,
            },
            op: AssignOp::Assign,
            value,
            span: declaration,
        }));
    }
    if let Some(body) = constructor.and_then(|constructor| constructor.body.as_block()) {
        let mut remaining = body.statements.as_slice();
        if cleanup_context.is_some_and(|context| context.parent_class.is_some())
            && remaining
                .first()
                .is_some_and(is_parent_constructor_statement)
        {
            // Parent failure is projected from the parent's own dataflow below.
            // Its successful path precedes this class's initialization phase.
            remaining = &remaining[1..];
        }
        statements.extend_from_slice(remaining);
    }
    let owner = constructor.map_or(class.span, |constructor| constructor.span);
    let body = crate::ast::Block {
        statements,
        span: owner,
    };
    let graph = build_function_cfg_with_checked_effects(
        &body,
        owner,
        given_preludes,
        checked_effect_sites,
        catch_error_types,
        catch_coverage,
        &HashSet::new(),
    );
    let result = solve_forward(
        &graph,
        &ConstructorAnalysis {
            properties,
            entry: State {
                reachable: true,
                properties: vec![InitState::Uninitialized; properties.len()],
            },
        },
    );
    for node in &graph.nodes {
        let input = &result.inputs[node.id.0];
        if !input.reachable
            || node.kind != NodeKind::DivergeExit
            || !matches!(node.action, NodeAction::None)
            || !checked_effect_sites
                .get(&node.span)
                .is_some_and(|effects| !effects.is_empty())
        {
            continue;
        }
        let mut initialized_properties = cleanup_context
            .map(|context| context.inherited_stored_properties.clone())
            .unwrap_or_default();
        initialized_properties.extend(properties.iter().zip(&input.properties).filter_map(
            |(property, state)| {
                (*state != InitState::Uninitialized).then_some(property.declaration)
            },
        ));
        insert_partial_cleanup(
            cleanups,
            ConstructorPartialCleanup {
                class: class.span,
                constructor: constructor.map(|constructor| constructor.span),
                site: node.span,
                initialized_properties,
            },
        );
    }
}

fn is_parent_constructor_statement(statement: &Stmt) -> bool {
    matches!(statement, Stmt::Expr {
        expr: Expr::StaticCall {
            qualifier: crate::ast::StaticQualifier::Parent,
            method,
            ..
        },
        ..
    } if method == "__construct")
}

fn insert_partial_cleanup(
    cleanups: &mut Vec<ConstructorPartialCleanup>,
    mut incoming: ConstructorPartialCleanup,
) -> bool {
    if let Some(existing) = cleanups.iter_mut().find(|existing| {
        existing.class == incoming.class
            && existing.constructor == incoming.constructor
            && existing.site == incoming.site
    }) {
        let before = existing.initialized_properties.len();
        for property in incoming.initialized_properties {
            if !existing.initialized_properties.contains(&property) {
                existing.initialized_properties.push(property);
            }
        }
        return existing.initialized_properties.len() != before;
    }
    incoming.initialized_properties.dedup();
    cleanups.push(incoming);
    true
}

fn inherit_parent_partial_cleanups(
    program: &Program,
    contexts: &HashMap<Span, ConstructorCleanupContext>,
    cleanups: &mut Vec<ConstructorPartialCleanup>,
) {
    loop {
        let mut changed = false;
        let previous = cleanups.clone();
        for class in program.items.iter().filter_map(|item| match item {
            Item::Class(class) => Some(class),
            _ => None,
        }) {
            let Some(context) = contexts.get(&class.span) else {
                continue;
            };
            let constructor = class.members.iter().find_map(|member| match member {
                ClassMember::Method(method) if method.name == "__construct" => Some(method.span),
                _ => None,
            });
            for parent in previous.iter().filter(|cleanup| {
                Some(cleanup.class) == context.parent_class
                    && cleanup.constructor == context.parent_constructor
            }) {
                changed |= insert_partial_cleanup(
                    cleanups,
                    ConstructorPartialCleanup {
                        class: class.span,
                        constructor,
                        site: parent.site,
                        initialized_properties: parent.initialized_properties.clone(),
                    },
                );
            }
        }
        if !changed {
            break;
        }
    }
    cleanups.sort_by_key(|cleanup| (cleanup.class, cleanup.constructor, cleanup.site));
}

fn transfer_action(properties: &[Property], action: &NodeAction, state: &mut State) {
    match action {
        NodeAction::Statement(Stmt::Assignment(assignment)) => {
            transfer_assignment(properties, state, &assignment.target, &assignment.op)
        }
        NodeAction::ForInitializer(ForInitializer::Assignment(assignment)) => {
            transfer_assignment(properties, state, &assignment.target, &assignment.op)
        }
        NodeAction::ForIncrement(ForIncrement::Assignment(assignment)) => {
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
            .insert(site.span, PropertyWriteKind::Replace);
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
    analysis.property_writes.insert(site.span, write_kind);
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
        // Its statements now have their own CFG nodes and incoming states.
        Expr::When(_) => {}
        Expr::Closure(closure) => {
            if let Some(capture) = closure.captures.as_ref().and_then(|clause| {
                clause
                    .captures
                    .iter()
                    .find(|capture| capture.name == "this")
            }) {
                report_incomplete_this(
                    class,
                    properties,
                    state,
                    capture.span,
                    &mut analysis.diagnostics,
                );
            }
        }
        Expr::Variable { .. }
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
            InitState::Uninitialized => diagnostics.push(property_init_diagnostic(
                class,
                property,
                span,
                format!(
                    "property `{}::{}` is not initialized before {exit} completes",
                    class.name, property.name
                ),
            )),
            InitState::MaybeInitialized => diagnostics.push(property_init_diagnostic(
                class,
                property,
                span,
                format!(
                    "property `{}::{}` is not initialized on every path before {exit} completes",
                    class.name, property.name
                ),
            )),
        }
    }
}

fn property_init_diagnostic(
    class: &ClassDecl,
    property: &Property,
    span: Span,
    message: String,
) -> Diagnostic {
    let diagnostic = Diagnostic::new("E0500", message, span);
    if property.same_named_constructor_only_parameter {
        diagnostic
            .with_note(format!(
                "`${}` is a constructor-only parameter and does not initialize `{}::{}`",
                property.name, class.name, property.name
            ))
            .with_help(format!(
                "initialize the property explicitly with `$this->{} = ${}`, or use ordinary promotion when the parameter should declare the property",
                property.name, property.name
            ))
    } else {
        diagnostic
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

#[cfg(test)]
mod cleanup_tests {
    use super::*;
    use crate::checked_effects::{CatchCoverage, CatchCoverageMap, CatchTypeMap, EffectSiteMap};
    use crate::types::ResolvedType;

    fn source_span(source: &str, text: &str) -> Span {
        let start = source.find(text).expect("fixture text exists");
        Span::new(start, start + text.len())
    }

    fn class<'a>(program: &'a Program, name: &str) -> &'a ClassDecl {
        program
            .items
            .iter()
            .find_map(|item| match item {
                Item::Class(class) if class.name == name => Some(class),
                _ => None,
            })
            .expect("fixture class exists")
    }

    fn property(class: &ClassDecl, name: &str) -> Span {
        class
            .members
            .iter()
            .find_map(|member| match member {
                ClassMember::Property(property) if property.name == name => Some(property.span),
                _ => None,
            })
            .expect("fixture property exists")
    }

    fn analyze(source: &str, calls: &[&str]) -> (Program, Analysis) {
        let program = crate::parse_source("constructor-cleanup.doria", source).unwrap();
        let effects = calls
            .iter()
            .map(|call| (source_span(source, call), vec![ResolvedType::Error]))
            .collect();
        let analysis = check_program(
            &program,
            &HashMap::new(),
            &GivenSemanticInfoMap::new(),
            &effects,
            &CatchTypeMap::new(),
            &CatchCoverageMap::new(),
        );
        (program, analysis)
    }

    #[test]
    fn initializer_failure_releases_only_completed_stores_before_promotion() {
        let source = r#"
class Example {
    Token $first = new Token();
    Token $second = fail();
    function __construct(Token $promoted) {}
}
"#;
        let (program, analysis) = analyze(source, &["fail()"]);
        assert_eq!(analysis.partial_cleanups.len(), 1);
        assert_eq!(
            analysis.partial_cleanups[0].initialized_properties,
            vec![property(class(&program, "Example"), "first")]
        );
    }

    #[test]
    fn body_failure_joins_maybe_initialized_fields_without_committing_throwing_store() {
        let source = r#"
class Example {
    Token $first;
    Token $second;
    function __construct() {
        if (choose()) { $this->first = new Token(); }
        $this->second = fail();
    }
}
"#;
        let (program, analysis) = analyze(source, &["fail()"]);
        assert_eq!(analysis.partial_cleanups.len(), 1);
        assert_eq!(
            analysis.partial_cleanups[0].initialized_properties,
            vec![property(class(&program, "Example"), "first")]
        );
    }

    #[test]
    fn when_failure_observes_prior_inner_stores_but_not_the_enclosing_store() {
        let source = r#"
class Example {
    Token $before;
    Token $result;
    function __construct() {
        $this->result = when (true): Token {
            $this->before = new Token();
            fail();
            return new Token();
        } else { return new Token(); };
    }
}
"#;
        let (program, analysis) = analyze(source, &["fail()"]);
        assert_eq!(analysis.partial_cleanups.len(), 1);
        assert_eq!(
            analysis.partial_cleanups[0].initialized_properties,
            vec![property(class(&program, "Example"), "before")]
        );
        assert!(!analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0412"));
    }

    #[test]
    fn later_argument_failure_observes_completed_when_and_its_finalizer() {
        let source = r#"
class Example {
    Token $before;
    Token $finalized;
    Token $result;
    function __construct() {
        $this->result = combine(when (true): int {
            $this->before = new Token();
            return 1;
        } else { return 0; } finally {
            $this->finalized = new Token();
        }, fail());
    }
}
"#;
        let (program, analysis) = analyze(source, &["fail()"]);
        let owner = class(&program, "Example");
        assert_eq!(analysis.partial_cleanups.len(), 1);
        assert_eq!(
            analysis.partial_cleanups[0].initialized_properties,
            vec![property(owner, "before"), property(owner, "finalized")]
        );
    }

    #[test]
    fn caught_failures_do_not_release_fields_and_escaping_finalizers_are_included() {
        let source = r#"
class Example {
    Token $caught;
    Token $finalized;
    function __construct() {
        try { caughtFailure(); }
        catch (Error $error) { $this->caught = new Token(); }
        try { escapingFailure(); }
        finally { $this->finalized = new Token(); }
    }
}
"#;
        let program = crate::parse_source("constructor-cleanup.doria", source).unwrap();
        let owner = class(&program, "Example");
        let catch = owner
            .members
            .iter()
            .find_map(|member| match member {
                ClassMember::Method(method) => method.body.as_block().and_then(|body| {
                    body.statements
                        .iter()
                        .find_map(|statement| match statement {
                            Stmt::Try(statement) => {
                                statement.catches.first().map(|catch| catch.span)
                            }
                            _ => None,
                        })
                }),
                _ => None,
            })
            .unwrap();
        let effects = EffectSiteMap::from([
            (
                source_span(source, "caughtFailure()"),
                vec![ResolvedType::Error],
            ),
            (
                source_span(source, "escapingFailure()"),
                vec![ResolvedType::Error],
            ),
        ]);
        let analysis = check_program(
            &program,
            &HashMap::new(),
            &GivenSemanticInfoMap::new(),
            &effects,
            &CatchTypeMap::from([(catch, ResolvedType::Error)]),
            &CatchCoverageMap::from([(
                catch,
                HashMap::from([(ResolvedType::Error, CatchCoverage::Complete)]),
            )]),
        );
        assert_eq!(analysis.partial_cleanups.len(), 1);
        assert_eq!(
            analysis.partial_cleanups[0].site,
            source_span(source, "escapingFailure()")
        );
        assert_eq!(
            analysis.partial_cleanups[0].initialized_properties,
            vec![property(owner, "caught"), property(owner, "finalized")]
        );
    }

    #[test]
    fn callback_creation_does_not_execute_its_latent_checked_effects() {
        let source = r#"
class Example {
    Token $first = new Token();
    function(): int throws Error $callback = function(): int {
        fail();
        return 1;
    };
}
"#;
        let (_, analysis) = analyze(source, &["fail()"]);
        assert!(analysis.partial_cleanups.is_empty());
    }

    #[test]
    fn parent_failure_preserves_parent_partial_state_before_derived_initializers() {
        let source = r#"
class Parent {
    Token $first = new Token();
    Token $second = failBase();
}
class Child extends Parent {
    Token $third = failChild();
}
"#;
        let program = crate::parse_source("constructor-cleanup.doria", source).unwrap();
        let parent = class(&program, "Parent");
        let child = class(&program, "Child");
        let effects = EffectSiteMap::from([
            (source_span(source, "failBase()"), vec![ResolvedType::Error]),
            (
                source_span(source, "failChild()"),
                vec![ResolvedType::Error],
            ),
        ]);
        let analysis = check_program_with_cleanup_context(
            &program,
            &HashMap::new(),
            &GivenSemanticInfoMap::new(),
            &effects,
            &CatchTypeMap::new(),
            &CatchCoverageMap::new(),
            &HashMap::from([(
                child.span,
                ConstructorCleanupContext {
                    parent_class: Some(parent.span),
                    parent_constructor: None,
                    inherited_stored_properties: vec![
                        property(parent, "first"),
                        property(parent, "second"),
                    ],
                },
            )]),
        );
        let child_failures = analysis
            .partial_cleanups
            .iter()
            .filter(|cleanup| cleanup.class == child.span)
            .collect::<Vec<_>>();
        assert_eq!(child_failures.len(), 2);
        assert_eq!(
            child_failures[0].initialized_properties,
            vec![property(parent, "first")]
        );
        assert_eq!(
            child_failures[1].initialized_properties,
            vec![property(parent, "first"), property(parent, "second")]
        );
    }
}

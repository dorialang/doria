//! Cleanup facts produced while checking ownership, not by rescanning syntax.
//!
//! A binding and its owned value have different identities: a transfer can
//! empty a binding while a pending argument or return still owns that value.

use std::collections::{HashMap, HashSet};

use crate::source::Span;
use crate::symbols::BindingId;
use crate::types::ResolvedType;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ValueId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    Expression(Span),
    Binding(BindingId),
    EnumPayload {
        scrutinee: Span,
        case: crate::enums::EnumCaseId,
        field: usize,
    },
    CallbackReturn {
        callback: Span,
    },
    CollectionElement {
        collection: Span,
        key: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Value {
    pub owner: Span,
    pub source: Source,
    pub ty: ResolvedType,
    /// Values transferred into an aggregate or closure environment. Borrowed
    /// captures never enter this set.
    pub contents: HashSet<ValueId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cause {
    ScopeExit,
    StatementEnd,
    Replacement,
    CheckedError,
    SupersededReturn,
    EnvironmentRelease,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub owner: Span,
    pub site: Span,
    pub values: HashSet<ValueId>,
    pub cause: Cause,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Analysis {
    pub values: HashMap<ValueId, Value>,
    pub releases: Vec<Release>,
    pub value_flows: HashSet<(Span, Span)>,
    /// Value origins with their aggregate role intact. A Dictionary key drop
    /// must not inherit the origins of its stored values (or vice versa).
    pub source_flows: HashSet<(Source, Source)>,
    identities: HashMap<(Span, Source, ResolvedType), ValueId>,
    release_sites: HashMap<(Span, Span, Cause), usize>,
}

impl Analysis {
    pub fn acquire(
        &mut self,
        owner: Span,
        source: Source,
        ty: ResolvedType,
        contents: HashSet<ValueId>,
    ) -> ValueId {
        let key = (owner, source, ty.clone());
        if let Some(id) = self.identities.get(&key).copied() {
            self.values
                .get_mut(&id)
                .expect("registered cleanup value")
                .contents
                .extend(contents);
            return id;
        }
        let id = ValueId(self.values.len());
        self.values.insert(
            id,
            Value {
                owner,
                source,
                ty,
                contents,
            },
        );
        self.identities.insert(key, id);
        id
    }

    pub fn release(&mut self, owner: Span, site: Span, values: HashSet<ValueId>, cause: Cause) {
        if values.is_empty() {
            return;
        }
        let key = (owner, site, cause);
        if let Some(index) = self.release_sites.get(&key).copied() {
            self.releases[index].values.extend(values);
        } else {
            self.release_sites.insert(key, self.releases.len());
            self.releases.push(Release {
                owner,
                site,
                values,
                cause,
            });
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Slot {
    Binding(BindingId),
    Temporary(Span),
    PendingArgument { call: Span, index: usize },
    PendingCallee(Span),
    PendingReturn(Span),
    PendingYield(Span),
    PendingError(Span),
    CaughtError(Span),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    values: HashSet<ValueId>,
    depth: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct State {
    slots: HashMap<Slot, Entry>,
}

pub(super) struct CallableContext {
    owner: Option<Span>,
    exception_scopes: Vec<super::ExceptionScope>,
    yield_regions: Vec<(Span, usize)>,
    when_result_modes: Vec<super::UseMode>,
}

impl State {
    pub fn values(&self, slot: &Slot) -> HashSet<ValueId> {
        self.slots
            .get(slot)
            .map(|entry| entry.values.clone())
            .unwrap_or_default()
    }

    pub fn take(&mut self, slot: &Slot) -> HashSet<ValueId> {
        self.slots
            .remove(slot)
            .map(|entry| entry.values)
            .unwrap_or_default()
    }

    pub fn put(&mut self, slot: Slot, values: HashSet<ValueId>, depth: usize) {
        if values.is_empty() {
            self.slots.remove(&slot);
        } else {
            self.slots.insert(slot, Entry { values, depth });
        }
    }

    pub fn merge_from(&mut self, left: &Self, right: &Self) {
        self.slots = left.slots.clone();
        for (slot, incoming) in &right.slots {
            self.slots
                .entry(*slot)
                .and_modify(|entry| {
                    entry.values.extend(&incoming.values);
                    entry.depth = entry.depth.min(incoming.depth);
                })
                .or_insert_with(|| incoming.clone());
        }
    }

    pub fn drain_from_depth(&mut self, depth: usize) -> HashSet<ValueId> {
        self.drain_matching(|_, entry| entry.depth >= depth)
    }

    pub fn drain_for_exception(&mut self, handler_depth: usize) -> HashSet<ValueId> {
        self.drain_matching(|slot, entry| {
            entry.depth > handler_depth
                || (handler_depth == 0
                    && matches!(slot, Slot::PendingReturn(_) | Slot::PendingError(_)))
        })
    }

    pub fn drain_temporaries(&mut self) -> HashSet<ValueId> {
        self.drain_temporaries_from_depth(0)
    }

    pub fn drain_temporaries_from_depth(&mut self, depth: usize) -> HashSet<ValueId> {
        self.drain_matching(|slot, entry| {
            matches!(slot, Slot::Temporary(_)) && entry.depth >= depth
        })
    }

    pub fn take_pending_returns(&mut self) -> HashSet<ValueId> {
        self.drain_matching(|slot, _| matches!(slot, Slot::PendingReturn(_)))
    }

    pub fn take_pending_errors(&mut self) -> HashSet<ValueId> {
        self.drain_matching(|slot, _| matches!(slot, Slot::PendingError(_)))
    }

    fn drain_matching(
        &mut self,
        mut predicate: impl FnMut(&Slot, &Entry) -> bool,
    ) -> HashSet<ValueId> {
        let mut values = HashSet::new();
        self.slots.retain(|slot, entry| {
            if predicate(slot, entry) {
                values.extend(&entry.values);
                false
            } else {
                true
            }
        });
        values
    }
}

impl super::Checker<'_> {
    pub(super) fn cleanup_acquire_value(
        &mut self,
        source: Source,
        ty: ResolvedType,
        contents: HashSet<ValueId>,
    ) -> HashSet<ValueId> {
        let Some(owner) = self.cleanup_owner else {
            return HashSet::new();
        };
        if !super::resolved_type_is_move_type(&ty, &self.move_enum_names)
            && !super::resolved_type_requires_conservative_move(&ty)
        {
            return HashSet::new();
        }
        HashSet::from([self.cleanup_analysis.acquire(owner, source, ty, contents)])
    }

    pub(super) fn cleanup_collection_roots(
        &self,
        expression: &crate::ast::Expr,
        scopes: &super::Scopes,
    ) -> HashSet<ValueId> {
        match super::ungroup_expr(expression) {
            crate::ast::Expr::Variable { name, .. } => scopes
                .get(name)
                .and_then(|binding| binding.canonical_id)
                .map(|binding| scopes.1.values(&Slot::Binding(binding)))
                .unwrap_or_default(),
            expression => scopes.1.values(&Slot::Temporary(expression.span())),
        }
    }

    pub(super) fn cleanup_begin_callable(&mut self, owner: Span) -> CallableContext {
        let context = CallableContext {
            owner: self.cleanup_owner.replace(owner),
            exception_scopes: std::mem::take(&mut self.exception_scopes),
            yield_regions: std::mem::take(&mut self.cleanup_yield_regions),
            when_result_modes: std::mem::take(&mut self.when_result_modes),
        };
        self.exception_scopes.push(super::ExceptionScope {
            lexical_depth: 0,
            exits: Vec::new(),
        });
        context
    }

    pub(super) fn cleanup_end_callable(
        &mut self,
        context: CallableContext,
        scopes: &mut super::Scopes,
        flow: &mut super::Flow,
        site: Span,
    ) {
        self.cleanup_finish_callable(scopes, flow, site);
        self.cleanup_owner = context.owner;
        self.exception_scopes = context.exception_scopes;
        self.cleanup_yield_regions = context.yield_regions;
        self.when_result_modes = context.when_result_modes;
    }

    pub(super) fn cleanup_release(&mut self, site: Span, values: HashSet<ValueId>, cause: Cause) {
        if let Some(owner) = self.cleanup_owner {
            self.cleanup_analysis.release(owner, site, values, cause);
        }
    }

    pub(super) fn cleanup_release_temporaries(&mut self, scopes: &mut super::Scopes, site: Span) {
        let values = scopes
            .1
            .drain_temporaries_from_depth(scopes.lexical_depth());
        self.cleanup_release(site, values, Cause::StatementEnd);
    }

    pub(super) fn cleanup_take_expr(
        &mut self,
        expression: &crate::ast::Expr,
        scopes: &mut super::Scopes,
    ) -> HashSet<ValueId> {
        scopes.1.take(&Slot::Temporary(expression.span()))
    }

    pub(super) fn cleanup_bind_result(
        &mut self,
        binding: BindingId,
        expression: &crate::ast::Expr,
        scopes: &mut super::Scopes,
        site: Span,
    ) {
        let values = self.cleanup_take_expr(expression, scopes);
        let previous = scopes.1.take(&Slot::Binding(binding));
        self.cleanup_release(site, previous, Cause::Replacement);
        let depth = scopes
            .get_by_canonical(binding)
            .map_or(scopes.lexical_depth(), |(_, binding)| binding.scope_depth);
        scopes.1.put(Slot::Binding(binding), values, depth);
    }

    pub(super) fn cleanup_store_place(
        &mut self,
        target: &crate::ast::Expr,
        expression: &crate::ast::Expr,
        scopes: &mut super::Scopes,
        site: Span,
        replaces: bool,
    ) {
        let contents = self.cleanup_take_expr(expression, scopes);
        self.cleanup_analysis
            .value_flows
            .insert((expression.span(), target.span()));
        if let crate::ast::Expr::PropertyAccess { object, .. }
        | crate::ast::Expr::Index {
            collection: object, ..
        } = super::ungroup_expr(target)
        {
            let roots = match super::ungroup_expr(object) {
                crate::ast::Expr::Variable { name, .. } => scopes
                    .get(name)
                    .and_then(|binding| binding.canonical_id)
                    .map(|binding| scopes.1.values(&Slot::Binding(binding)))
                    .unwrap_or_default(),
                _ => scopes.1.values(&Slot::Temporary(object.span())),
            };
            for root in roots {
                if let Some(value) = self.cleanup_analysis.values.get_mut(&root) {
                    value.contents.extend(&contents);
                }
            }
        }
        if !replaces {
            return;
        }
        if let (Some(owner), Some(ty)) = (self.cleanup_owner, self.resolved_type(target).cloned()) {
            if super::resolved_type_is_move_type(&ty, &self.move_enum_names)
                || super::resolved_type_requires_conservative_move(&ty)
            {
                let old = self.cleanup_analysis.acquire(
                    owner,
                    Source::Expression(target.span()),
                    ty,
                    HashSet::new(),
                );
                self.cleanup_release(site, HashSet::from([old]), Cause::Replacement);
            }
        }
    }

    pub(super) fn cleanup_owned_parameter(
        &mut self,
        binding: BindingId,
        scopes: &mut super::Scopes,
    ) {
        let Some(owner) = self.cleanup_owner else {
            return;
        };
        let Some(ty) = self
            .binding_resolution
            .declarations_by_id
            .get(&binding)
            .and_then(|binding| binding.source_type.clone())
        else {
            return;
        };
        if !super::resolved_type_is_move_type(&ty, &self.move_enum_names)
            && !super::resolved_type_requires_conservative_move(&ty)
        {
            return;
        }
        let value =
            self.cleanup_analysis
                .acquire(owner, Source::Binding(binding), ty, HashSet::new());
        scopes.1.put(
            Slot::Binding(binding),
            HashSet::from([value]),
            scopes.lexical_depth(),
        );
    }

    pub(super) fn cleanup_stage_return(
        &mut self,
        expression: Option<&crate::ast::Expr>,
        scopes: &mut super::Scopes,
        site: Span,
    ) {
        let values = expression
            .map(|expression| self.cleanup_take_expr(expression, scopes))
            .unwrap_or_default();
        let previous = scopes.1.take_pending_returns();
        self.cleanup_release(site, previous, Cause::SupersededReturn);
        let previous_error = scopes.1.take_pending_errors();
        self.cleanup_release(site, previous_error, Cause::SupersededReturn);
        scopes.1.put(Slot::PendingReturn(site), values, 0);
    }

    pub(super) fn cleanup_superseded_control_exit(&mut self, flow: &mut super::Flow, site: Span) {
        for exit in flow.backedges.iter_mut().chain(&mut flow.breaks) {
            let mut values = exit.1.take_pending_returns();
            values.extend(exit.1.take_pending_errors());
            self.cleanup_release(site, values, Cause::SupersededReturn);
        }
    }

    pub(super) fn cleanup_stage_yield(
        &mut self,
        expression: &crate::ast::Expr,
        scopes: &mut super::Scopes,
    ) {
        let Some((region, depth)) = self.cleanup_yield_regions.last().copied() else {
            return;
        };
        let values = self.cleanup_take_expr(expression, scopes);
        scopes.1.put(Slot::PendingYield(region), values, depth);
    }

    pub(super) fn cleanup_pop_scope(
        &mut self,
        scopes: &mut super::Scopes,
        site: Span,
        cause: Cause,
    ) {
        let values = scopes.1.drain_from_depth(scopes.lexical_depth());
        self.cleanup_release(site, values, cause);
        scopes.pop();
    }

    pub(super) fn cleanup_pop_flow_scope(
        &mut self,
        scopes: &mut super::Scopes,
        flow: &mut super::Flow,
        site: Span,
    ) {
        if flow.falls_through {
            self.cleanup_pop_scope(scopes, site, Cause::ScopeExit);
        } else {
            scopes.pop();
        }
        for exit in flow
            .backedges
            .iter_mut()
            .chain(&mut flow.breaks)
            .chain(&mut flow.returns)
            .chain(&mut flow.yields)
        {
            self.cleanup_pop_scope(exit, site, Cause::ScopeExit);
        }
    }

    pub(super) fn cleanup_finish_callable(
        &mut self,
        scopes: &mut super::Scopes,
        flow: &mut super::Flow,
        site: Span,
    ) {
        if flow.falls_through {
            let values = scopes.1.drain_from_depth(1);
            self.cleanup_release(site, values, Cause::ScopeExit);
        }
        for exit in &mut flow.returns {
            // Only a completed return transfers the result to the caller.
            // Finalizers have already had their opportunity to supersede it.
            exit.1.take_pending_returns();
            let values = exit.1.drain_from_depth(1);
            self.cleanup_release(site, values, Cause::ScopeExit);
        }
    }
}

pub(super) fn statement_span(statement: &crate::ast::Stmt) -> Span {
    use crate::ast::Stmt;
    match statement {
        Stmt::Block(block) => block.span,
        Stmt::VarDecl(declaration) => declaration.span,
        Stmt::Assignment(assignment) => assignment.span,
        Stmt::Echo { span, .. } | Stmt::Return { span, .. } | Stmt::Expr { span, .. } => *span,
        Stmt::If(statement) => statement.span,
        Stmt::While(statement) => statement.span,
        Stmt::DoWhile(statement) => statement.span,
        Stmt::For(statement) => statement.span,
        Stmt::Foreach(statement) => statement.span,
        Stmt::Increment(statement) => statement.span,
        Stmt::Throw(statement) => statement.span,
        Stmt::Try(statement) => statement.span,
        Stmt::Break { span } | Stmt::Continue { span } => *span,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moved_value_remains_owned_by_its_pending_destination() {
        let mut state = State::default();
        let binding = Slot::Binding(BindingId(1));
        let pending = Slot::PendingArgument {
            call: Span::new(10, 20),
            index: 0,
        };
        state.put(binding, HashSet::from([ValueId(0)]), 1);
        let transferred = state.take(&binding);
        state.put(pending, transferred, 1);
        assert!(state.values(&binding).is_empty());
        assert_eq!(state.drain_from_depth(1), HashSet::from([ValueId(0)]));
    }

    #[test]
    fn pending_return_survives_lexical_exit_but_not_exceptional_unwind() {
        let mut state = State::default();
        state.put(
            Slot::PendingReturn(Span::new(10, 20)),
            HashSet::from([ValueId(0)]),
            0,
        );
        state.put(Slot::Binding(BindingId(1)), HashSet::from([ValueId(1)]), 1);
        assert_eq!(state.drain_from_depth(1), HashSet::from([ValueId(1)]));
        assert_eq!(state.drain_from_depth(0), HashSet::from([ValueId(0)]));
    }

    #[test]
    fn repeatable_environment_survives_an_invocation_error() {
        let capture = Slot::Binding(BindingId(1));
        let mut state = State::default();
        state.put(capture, HashSet::from([ValueId(0)]), 0);
        state.put(Slot::Binding(BindingId(2)), HashSet::from([ValueId(1)]), 1);
        state.put(
            Slot::PendingReturn(Span::new(10, 20)),
            HashSet::from([ValueId(2)]),
            0,
        );
        assert_eq!(
            state.drain_for_exception(0),
            HashSet::from([ValueId(1), ValueId(2)])
        );
        assert_eq!(state.values(&capture), HashSet::from([ValueId(0)]));
    }

    #[test]
    fn branch_merge_keeps_only_possibly_remaining_owners() {
        let slot = Slot::Binding(BindingId(1));
        let mut left = State::default();
        left.put(slot, HashSet::from([ValueId(0)]), 1);
        let right = State::default();
        let mut joined = State::default();
        joined.merge_from(&left, &right);
        assert_eq!(joined.take(&slot), HashSet::from([ValueId(0)]));
        joined.merge_from(&right, &right);
        assert!(joined.take(&slot).is_empty());
    }
}

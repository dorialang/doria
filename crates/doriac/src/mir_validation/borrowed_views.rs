//! Preserve the owner or access lease behind borrowed nominal views, including
//! views materialized in separate locals for interface dispatch and narrowing.
use super::*;
use retained::{Place, Source};

#[path = "stored_owners.rs"]
mod stored_owners;

fn temporary_conflict(
    loans: &HashMap<Place, bool>,
    place: &Place,
    writable: bool,
) -> Result<(), BackendError> {
    if loans
        .iter()
        .any(|(source, exclusive)| place.overlaps(source) && (writable || *exclusive))
    {
        return Err(malformed_mir(
            "temporary value retains conflicting source access",
        ));
    }
    Ok(())
}

/// How a carrier still uses one of its stored fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FieldUse {
    Read,
    /// A pending property store: the field is an assignment destination.
    Store,
}

/// Decision 0136: lowering materializes stored property paths into synthetic
/// carriers before the operands that use them are evaluated. A carrier that is
/// still used only for field access claims only those fields, not its whole
/// object; any other use claims the whole object as before. A pending store
/// reserves its destination field: reading exactly that field is the
/// assignment's own read-modify-write, while any other overlapping access
/// (an ancestor used as a whole, a descendant, or a write) conflicts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Hold {
    Whole,
    Fields(HashMap<crate::class_layout::PropertyId, FieldUse>),
}

impl Hold {
    fn merge(&mut self, incoming: &Self) {
        match (&mut *self, incoming) {
            (Self::Whole, _) => {}
            (_, Self::Whole) => *self = Self::Whole,
            (Self::Fields(fields), Self::Fields(incoming)) => {
                for (field, used) in incoming {
                    let entry = fields.entry(*field).or_insert(*used);
                    if *used == FieldUse::Store {
                        *entry = FieldUse::Store;
                    }
                }
            }
        }
    }
}

/// The remaining uses of every tracked local that is live at one point.
pub(super) type Holds = HashMap<mir::LocalId, Hold>;

fn merge_holds(target: &mut Holds, incoming: &Holds) {
    for (local, hold) in incoming {
        target
            .entry(*local)
            .and_modify(|existing| existing.merge(hold))
            .or_insert_with(|| hold.clone());
    }
}

/// A writable borrow of `written` can restructure storage below it but cannot
/// replace the object at `written` or any of its ancestors.
fn identity_survives_write(root: &Place, written: &Place) -> bool {
    !root.opaque && root.local == written.local && written.fields.starts_with(&root.fields)
}

type PayloadLoans = HashMap<(crate::enums::EnumCaseId, usize), PayloadFieldLoans>;

#[derive(Clone, Default, PartialEq, Eq)]
struct PayloadFieldLoans {
    sources: HashSet<Source>,
    nested: Option<PayloadLoans>,
}

fn merge_payload_loans(target: &mut PayloadLoans, incoming: &PayloadLoans) {
    use std::collections::hash_map::Entry;
    for (field, incoming) in incoming {
        match target.entry(*field) {
            Entry::Vacant(entry) => {
                entry.insert(incoming.clone());
            }
            Entry::Occupied(mut entry) => {
                let field = entry.get_mut();
                field.sources.extend(incoming.sources.iter().cloned());
                if let (Some(left), Some(right)) = (&mut field.nested, &incoming.nested) {
                    merge_payload_loans(left, right);
                } else {
                    field.nested = None;
                }
            }
        }
    }
}

fn retain_payload_cleanup_loans(fields: &mut PayloadLoans) {
    for field in fields.values_mut() {
        field
            .sources
            .retain(|source| source.lifetime == mir::RetainedLifetime::Cleanup);
        if let Some(nested) = &mut field.nested {
            retain_payload_cleanup_loans(nested);
        }
    }
}

fn widen_sources_on_cycle(old: &HashSet<Source>, next: &mut HashSet<Source>) {
    *next = next
        .drain()
        .map(|mut source| {
            source
                .place
                .widen_on_cycle(old.iter().map(|loan| &loan.place));
            source.contract.owned_path = source.place.fields.clone();
            source
        })
        .collect();
}

fn widen_payload_loans_on_cycle(old: &HashSet<Source>, next: &mut PayloadLoans) {
    for field in next.values_mut() {
        widen_sources_on_cycle(old, &mut field.sources);
        if let Some(nested) = &mut field.nested {
            widen_payload_loans_on_cycle(old, nested);
        }
    }
}

/// Whether this argument borrow is the receiver of a child pipe getter call.
/// The call's own access check covers the slot the getter lends, so the
/// argument must not claim the whole owner.
fn is_child_pipe_receiver(
    program: &mir::Program,
    accesses: &ClassLocalAccesses<'_>,
    position: usize,
    local: mir::LocalId,
) -> Result<bool, BackendError> {
    let Some(ClassLocalAccess::Call(CallTarget::Direct { function, .. }, args)) =
        accesses.iter().nth(position + 1)
    else {
        return Ok(false);
    };
    Ok(child_pipe_place(program, function)?.is_some()
        && args.first().and_then(mir::Rvalue::direct_place_local) == Some(local))
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct State {
    roots: Vec<HashSet<Place>>,
    loans: Vec<HashSet<Source>>,
    payload_loans: HashMap<mir::LocalId, PayloadLoans>,
    parents: Vec<HashSet<byte_regions::ParentLoan>>,
    view_parents: Vec<HashSet<mir::LocalId>>,
    view_access: Vec<Option<bool>>,
    /// Field-only claims of live synthetic carriers at the current point; a
    /// live view absent here claims its whole object.
    carrier_fields: HashMap<mir::LocalId, HashMap<crate::class_layout::PropertyId, FieldUse>>,
    ended: HashSet<mir::LocalId>,
    storage: stored_owners::Storage,
}

impl State {
    pub(super) fn new(program: &mir::Program, function: &mir::Function) -> Self {
        Self {
            storage: stored_owners::Storage::new(program, function),
            payload_loans: HashMap::new(),
            roots: function
                .locals
                .iter()
                .map(|local| HashSet::from([Place::local(local.id)]))
                .collect(),
            loans: function
                .locals
                .iter()
                .map(|local| {
                    if (function.params.contains(&local.id)
                        || function.closure.as_ref().is_some_and(|closure| {
                            closure.capture_locals.iter().any(|(_, id)| *id == local.id)
                        }))
                        && retained::carries_source(program, local.ty)
                    {
                        HashSet::from([Source::input(
                            local.id,
                            byte_regions::kind(local.ty)
                                .is_some_and(mir::ByteRegionKind::is_writable),
                            if byte_regions::kind(local.ty).is_some() {
                                mir::RetainedLifetime::LastUse
                            } else {
                                mir::RetainedLifetime::Cleanup
                            },
                        )])
                    } else {
                        HashSet::new()
                    }
                })
                .collect(),
            parents: vec![HashSet::new(); function.locals.len()],
            view_parents: vec![HashSet::new(); function.locals.len()],
            view_access: vec![None; function.locals.len()],
            carrier_fields: HashMap::new(),
            ended: function
                .locals
                .iter()
                .filter(|local| {
                    byte_regions::kind(local.ty).is_some()
                        && !function.params.contains(&local.id)
                        && !function.closure.as_ref().is_some_and(|closure| {
                            closure.capture_locals.iter().any(|(_, id)| *id == local.id)
                        })
                })
                .map(|local| local.id)
                .collect(),
        }
    }

    fn end_carrier(&mut self, local: mir::LocalId) {
        self.ended.insert(local);
        // Keep provenance available for a checked call's outgoing result edge.
        // Ended carriers no longer contribute live loans and cannot be read.
    }

    fn expire_dead_last_use_sources(
        &mut self,
        program: &mir::Program,
        function: &mir::Function,
        holds: &Holds,
    ) {
        for local in &function.locals {
            if !holds.contains_key(&local.id) {
                self.view_access[local.id.0] = None;
            }
        }
        self.carrier_fields = holds
            .iter()
            .filter_map(|(local, hold)| match hold {
                Hold::Fields(fields) => Some((*local, fields.clone())),
                Hold::Whole => None,
            })
            .collect();
        // An alias or borrowed capture can be the only later use of a carrier.
        // Follow the ownership graph before expiring its underlying loans,
        // rather than copying those loans into a different source contract.
        let mut live = holds.keys().copied().collect::<HashSet<_>>();
        live.extend(
            function
                .params
                .iter()
                .copied()
                .filter(|id| !function.locals[id.0].owned),
        );
        loop {
            let before = live.len();
            for local in &function.locals {
                if self.ended.contains(&local.id) {
                    continue;
                }
                let used = live.contains(&local.id);
                if used {
                    live.extend(self.roots[local.id.0].iter().map(|place| place.local));
                    live.extend(self.parents[local.id.0].iter().map(|source| source.local));
                }
                live.extend(self.loans[local.id.0].iter().filter_map(|source| {
                    ((used || source.lifetime == mir::RetainedLifetime::Cleanup)
                        && source.local != local.id)
                        .then_some(source.local)
                }));
            }
            if before == live.len() {
                break;
            }
        }
        for local in &function.locals {
            if LastUseLiveness::tracks(program, local.ty) && !live.contains(&local.id) {
                self.loans[local.id.0]
                    .retain(|source| source.lifetime == mir::RetainedLifetime::Cleanup);
                if let Some(fields) = self.payload_loans.get_mut(&local.id) {
                    retain_payload_cleanup_loans(fields);
                }
                self.storage.retain_cleanup_sources(local.id);
                if byte_regions::kind(local.ty).is_some() && self.loans[local.id.0].is_empty() {
                    self.end_carrier(local.id);
                }
            }
        }
    }

    fn end(&mut self, local: mir::LocalId) {
        self.ended.insert(local);
        for (index, roots) in self.roots.iter().enumerate() {
            if roots.iter().any(|place| place.local == local)
                || self.loans[index]
                    .iter()
                    .any(|loan| !loan.inherited && loan.local == local)
            {
                self.ended.insert(mir::LocalId(index));
            }
        }
    }

    fn assign(&mut self, function: &mir::Function, local: mir::LocalId, roots: HashSet<Place>) {
        self.end(local);
        self.loans[local.0].clear();
        self.payload_loans.remove(&local);
        self.parents[local.0].clear();
        self.view_parents[local.0].clear();
        self.view_access[local.0] = None;
        self.storage.reset(local, function.locals[local.0].ty);
        // Owned and Copy locals have independent storage, including absent
        // nullable values and region descriptors. A borrowed Move carrier can
        // instead have no owner: preserve its neutral root set through aliases
        // and joins instead of inventing a temporary lifetime.
        let definition = &function.locals[local.0];
        self.roots[local.0] =
            if roots.is_empty() && (definition.owned || !definition.ty.has_move_ownership()) {
                HashSet::from([Place::local(local)])
            } else {
                roots
            };
        self.ended.remove(&local);
        if !definition.owned
            && definition.ty.has_move_ownership()
            && byte_regions::kind(definition.ty).is_none()
            && !self.roots[local.0].is_empty()
        {
            self.view_access[local.0] = Some(definition.writable);
        }
    }

    fn ancestors(&self, through: impl IntoIterator<Item = mir::LocalId>) -> HashSet<mir::LocalId> {
        let mut pending: Vec<_> = through.into_iter().collect();
        let mut ancestors = HashSet::new();
        while let Some(local) = pending.pop() {
            if ancestors.insert(local) {
                pending.extend(self.view_parents[local.0].iter().copied());
            }
        }
        ancestors
    }

    fn value_parents(
        program: &mir::Program,
        value: &mir::Rvalue,
    ) -> Result<HashSet<mir::LocalId>, BackendError> {
        let mut parents = escaping_class_local_borrows(program, value)?;
        if value.borrows_move_value() {
            parents.extend(value.direct_place_local());
        }
        Ok(parents.into_iter().collect())
    }

    fn sources(
        &self,
        program: &mir::Program,
        value: &mir::Rvalue,
    ) -> Result<HashSet<Place>, BackendError> {
        let mut sources = escaping_class_local_borrows(program, value)?;
        if value.borrows_move_value() {
            sources.extend(value.direct_place_local());
        }
        if sources.is_empty() {
            return Ok(HashSet::new());
        }
        retained::source_roots(program, value, &self.roots)
    }

    fn conflicts(&self, local: mir::LocalId) -> Result<(), BackendError> {
        self.conflicting_access(local, true)
    }

    fn exact_payload_loans(
        &self,
        program: &mir::Program,
        value: &mir::Rvalue,
    ) -> Result<Option<PayloadLoans>, BackendError> {
        match value {
            mir::Rvalue::PayloadEnum(value) => self.exact_payload_expression(program, value),
            mir::Rvalue::NullablePayloadEnum(value) => self.exact_nullable_payload(program, value),
            mir::Rvalue::Mixed(mir::MixedExpression::BoxPayloadEnum { value }) => {
                self.exact_payload_expression(program, value)
            }
            mir::Rvalue::NullableMixed(mir::NullableMixedExpression::BoxNullablePayloadEnum(
                value,
            )) => self.exact_nullable_payload(program, value),
            mir::Rvalue::NullableMixed(mir::NullableMixedExpression::Mixed(
                mir::MixedExpression::BoxPayloadEnum { value },
            )) => self.exact_payload_expression(program, value),
            mir::Rvalue::NullableMixed(mir::NullableMixedExpression::Mixed(_)) => Ok(None),
            mir::Rvalue::Mixed(_) | mir::Rvalue::NullableMixed(_) => Ok(value
                .direct_place_local()
                .and_then(|local| self.payload_loans.get(&local).cloned())),
            _ => Ok(None),
        }
    }

    fn exact_payload_expression(
        &self,
        program: &mir::Program,
        value: &mir::PayloadEnumExpression,
    ) -> Result<Option<PayloadLoans>, BackendError> {
        use mir::PayloadEnumExpression as E;
        match value {
            E::Construct { case, fields, .. } => {
                let mut result = HashMap::new();
                for (index, field) in fields.iter().enumerate() {
                    let mut sources =
                        retained::value_sources(program, field, &self.roots, &self.loans)?;
                    sources.extend(
                        self.storage
                            .value(program, field, &self.roots, &self.loans)?
                            .sources(),
                    );
                    let nested = self.exact_payload_loans(program, field)?;
                    if !sources.is_empty() || nested.is_some() {
                        result.insert((*case, index), PayloadFieldLoans { sources, nested });
                    }
                }
                Ok(Some(result))
            }
            E::Use { place, .. } => Ok(self.exact_payload_place(place)),
            E::Coalesce { left, right, .. } => {
                match nullable_payload_enum_expression_presence(left) {
                    Some(true) => return self.exact_nullable_payload(program, left),
                    Some(false) => return self.exact_payload_expression(program, right),
                    None => {}
                }
                let Some(mut left) = self.exact_nullable_payload(program, left)? else {
                    return Ok(None);
                };
                let Some(right) = self.exact_payload_expression(program, right)? else {
                    return Ok(None);
                };
                merge_payload_loans(&mut left, &right);
                Ok(Some(left))
            }
            E::Call { .. } => Ok(None),
        }
    }

    fn exact_nullable_payload(
        &self,
        program: &mir::Program,
        value: &mir::NullablePayloadEnumExpression,
    ) -> Result<Option<PayloadLoans>, BackendError> {
        use mir::NullablePayloadEnumExpression as E;
        match value {
            E::Null(_) => Ok(Some(HashMap::new())),
            E::Value(value) => self.exact_payload_expression(program, value),
            E::Use { place, .. } => Ok(self.exact_payload_place(place)),
            E::Coalesce { left, right, .. } => {
                match nullable_payload_enum_expression_presence(left) {
                    Some(true) => return self.exact_nullable_payload(program, left),
                    Some(false) => return self.exact_nullable_payload(program, right),
                    None => {}
                }
                let Some(mut left) = self.exact_nullable_payload(program, left)? else {
                    return Ok(None);
                };
                let Some(right) = self.exact_nullable_payload(program, right)? else {
                    return Ok(None);
                };
                merge_payload_loans(&mut left, &right);
                Ok(Some(left))
            }
            E::Call { .. } | E::CollectionGet { .. } => Ok(None),
        }
    }

    fn exact_payload_place(&self, place: &mir::PayloadEnumPlace) -> Option<PayloadLoans> {
        match place {
            mir::PayloadEnumPlace::Local(local)
            | mir::PayloadEnumPlace::NullableLocalAssumeNonNull(local)
            | mir::PayloadEnumPlace::MixedPayload { mixed: local } => {
                self.payload_loans.get(local).cloned()
            }
            _ => None,
        }
    }

    fn narrow_absent_payload(&mut self, condition: &mir::BoolExpression, mut selected: bool) {
        let mut condition = condition;
        while let mir::BoolExpression::Not(inner) = condition {
            condition = inner;
            selected = !selected;
        }
        if selected {
            return;
        }
        if let mir::BoolExpression::NullablePayloadEnumIsPresent(value) = condition {
            if let mir::NullablePayloadEnumExpression::Use {
                place: mir::PayloadEnumPlace::Local(local),
                ..
            } = value.as_ref()
            {
                self.loans[local.0].clear();
                self.storage.retain_sources(*local, |_| false);
                self.payload_loans.insert(*local, HashMap::new());
            }
        }
    }

    fn conflicting_definition(
        &self,
        function: &mir::Function,
        local: mir::LocalId,
    ) -> Result<(), BackendError> {
        // Redefining a view changes its carrier, not the borrowed referent.
        // A writable borrowed binding writes through its original home.
        if function.is_writable_borrowed_place(local) {
            self.conflicts(local)
        } else {
            self.conflicting_place(&Place::local(local), true, &HashSet::from([local]))
        }
    }

    /// Whether an access of `place` conflicts with what live view `local`
    /// still claims (see [`Hold`]).
    fn view_conflicts(
        &self,
        local: mir::LocalId,
        writable: bool,
        place: &Place,
        exclusive: bool,
    ) -> bool {
        let roots = &self.roots[local.0];
        let Some(fields) = self.carrier_fields.get(&local) else {
            return (exclusive || writable) && roots.iter().any(|root| root.overlaps(place));
        };
        roots.iter().any(|root| {
            fields.iter().any(|(field, used)| {
                let claimed = root.project(*field);
                claimed.overlaps(place)
                    && (exclusive
                        || *used == FieldUse::Store && (claimed.opaque || claimed != *place))
            })
        })
    }

    fn conflicting_access(&self, local: mir::LocalId, exclusive: bool) -> Result<(), BackendError> {
        for place in &self.roots[local.0] {
            self.conflicting_place(place, exclusive, &self.ancestors([local]))?;
        }
        Ok(())
    }

    fn conflicting_place(
        &self,
        place: &Place,
        exclusive: bool,
        through: &HashSet<mir::LocalId>,
    ) -> Result<(), BackendError> {
        if let Some((index, _)) = self.view_access.iter().enumerate().find(|(index, access)| {
            let local = mir::LocalId(*index);
            !through.contains(&local)
                && !self.ended.contains(&local)
                && access
                    .is_some_and(|writable| self.view_conflicts(local, writable, place, exclusive))
        }) {
            return Err(malformed_mir(format!(
                "source access conflicts with a live borrowed view: source {place:?}, view local{index}"
            )));
        }
        if self.parents.iter().enumerate().any(|(index, parents)| {
            !self.ended.contains(&mir::LocalId(index))
                && parents.iter().any(|parent| {
                    place.overlaps(&Place::local(parent.local)) && (exclusive || parent.writable)
                })
        }) {
            return Err(malformed_mir(
                "byte region parent is accessed while its reborrow is live",
            ));
        }
        if self.loans.iter().enumerate().any(|(index, loans)| {
            !self.ended.contains(&mir::LocalId(index))
                && loans.iter().any(|loan| {
                    !loan.inherited && place.overlaps(&loan.place) && (exclusive || loan.writable)
                })
        }) {
            return Err(malformed_mir(format!(
                "source local{} is mutated or ended while a live value retains it",
                place.local.0
            )));
        }
        Ok(())
    }

    fn accesses(
        &mut self,
        program: &mir::Program,
        function: &mir::Function,
        accesses: &ClassLocalAccesses<'_>,
        checking: bool,
    ) -> Result<(), BackendError> {
        let mut storage = accesses.storage_accesses.iter().peekable();
        let mut acquired = HashMap::<Place, bool>::new();
        let mut call_entries = Vec::new();
        let mut conditional_entries = Vec::new();
        for (position, access) in accesses.iter().map(Some).chain([None]).enumerate() {
            while let Some((at, _)) = storage.peek() {
                if *at != position {
                    break;
                }
                let (_, access) = storage.next().expect("borrowed storage access position");
                self.storage_access(program, function, *access, &mut acquired, checking)?;
            }
            let Some(access) = access else {
                continue;
            };
            match access {
                ClassLocalAccess::BeginConditional => {
                    conditional_entries.push((self.clone(), acquired.clone()));
                }
                ClassLocalAccess::EndConditional => {
                    let (state, loans) = conditional_entries.pop().ok_or_else(|| {
                        malformed_mir("conditional access markers are unbalanced")
                    })?;
                    self.join(state);
                    for (place, writable) in loans {
                        *acquired.entry(place).or_default() |= writable;
                    }
                }
                ClassLocalAccess::PropertyBorrow(local, property) => {
                    if checking && self.ended.contains(&local) {
                        return Err(malformed_mir(
                            "property view is used after its source ownership ended",
                        ));
                    }
                    let property = property_in(program, property.class, property)?;
                    if checking && !property.ty.has_move_ownership() {
                        for place in &self.roots[local.0] {
                            self.conflicting_place(
                                &place.project(property.id),
                                false,
                                &self.ancestors([local]),
                            )?;
                            temporary_conflict(&acquired, &place.project(property.id), false)?;
                        }
                    }
                }
                ClassLocalAccess::Borrow(local) | ClassLocalAccess::Transfer(local) => {
                    if checking && self.ended.contains(&local) {
                        return Err(malformed_mir(format!(
                            "view local{} is used after its source ownership ended",
                            local.0
                        )));
                    }
                    if matches!(access, ClassLocalAccess::Transfer(_)) {
                        if checking {
                            self.conflicts(local)?;
                            for place in &self.roots[local.0] {
                                temporary_conflict(&acquired, place, true)?;
                            }
                        }
                        self.end(local);
                    } else if checking
                        && !is_child_pipe_receiver(program, accesses, position, local)?
                    {
                        self.conflicting_access(local, false)?;
                    }
                }
                ClassLocalAccess::Call(callee, args) => {
                    let entry = call_entries
                        .pop()
                        .ok_or_else(|| malformed_mir("call access markers are unbalanced"))?;
                    let returned =
                        self.temporary_call_sources(program, accesses, position, callee, args)?;
                    if checking {
                        retained::validate_call_inputs(
                            program,
                            function,
                            callee,
                            args,
                            &self.roots,
                            &self.loans,
                        )?;
                    }
                    let mut borrows = Vec::<(Place, ClassBorrowMode)>::new();
                    let mut mutated = Vec::<Place>::new();
                    let mut direct_inputs = HashSet::new();
                    for (value, mode, retains_writable_place) in
                        borrowed_call_inputs(program, callee, args)?
                    {
                        let mut parents = Self::value_parents(program, value)?;
                        let places = if retains_writable_place {
                            parents.extend(value.direct_place_local());
                            parents.extend(
                                retained::source_property_place(value).map(|(object, _)| object),
                            );
                            retained::source_roots(program, value, &self.roots)?
                        } else {
                            self.sources(program, value)?
                        };
                        // A child pipe getter reaches only the slot it lends.
                        let places = match callee {
                            CallTarget::Direct { function, .. } => {
                                match child_pipe_place(program, function)? {
                                    Some(slot) => {
                                        places.iter().map(|place| place.project(slot)).collect()
                                    }
                                    None => places,
                                }
                            }
                            CallTarget::Indirect(_) => places,
                        };
                        let through = self.ancestors(parents);
                        direct_inputs.extend(through.iter().copied());
                        for place in &places {
                            if checking {
                                self.conflicting_place(
                                    place,
                                    matches!(mode, ClassBorrowMode::Writable),
                                    &through,
                                )?;
                                temporary_conflict(
                                    &entry,
                                    place,
                                    matches!(mode, ClassBorrowMode::Writable),
                                )?;
                                if borrows.iter().any(|(previous, previous_mode)| {
                                    place.overlaps(previous) && mode.conflicts_with(*previous_mode)
                                }) {
                                    return Err(class_access_error(
                                        "call through borrowed views",
                                        "takes overlapping writable borrows of",
                                        place.local,
                                    ));
                                }
                            }
                            if matches!(mode, ClassBorrowMode::Writable) {
                                mutated.push(place.clone());
                            }
                        }
                        // Alternative origins of one argument are not separate
                        // simultaneous borrows. Compare only prior arguments.
                        borrows.extend(places.into_iter().map(|place| (place, mode)));
                    }
                    // A writable argument may replace storage below it, ending
                    // views into that storage. Views of the argument's own
                    // object or its ancestors keep their object (Decision 0136).
                    for local in &function.locals {
                        if !local.owned
                            && !function.params.contains(&local.id)
                            && !direct_inputs.contains(&local.id)
                            && self.roots[local.id.0].iter().any(|root| {
                                mutated.iter().any(|place| {
                                    root.overlaps(place) && !identity_survives_write(root, place)
                                })
                            })
                        {
                            self.ended.insert(local.id);
                        }
                    }
                    if let Some((_, span)) = accesses
                        .immediate_calls
                        .iter()
                        .find(|(at, _)| *at == position)
                    {
                        self.stored_call(program, accesses, position, callee, args, *span)?;
                    }
                    // Argument-only loans retire with this call; loans carried
                    // by its result remain live through the containing call.
                    acquired = entry;
                    for (place, writable) in returned {
                        *acquired.entry(place).or_default() |= writable;
                    }
                }
                ClassLocalAccess::BeginCall => call_entries.push(acquired.clone()),
            }
        }
        if !conditional_entries.is_empty() {
            return Err(malformed_mir("conditional access markers are unbalanced"));
        }
        if !call_entries.is_empty() {
            return Err(malformed_mir("call access markers are unbalanced"));
        }
        Ok(())
    }

    fn storage_access(
        &mut self,
        program: &mir::Program,
        function: &mir::Function,
        access: StorageAccess<'_>,
        acquired: &mut HashMap<Place, bool>,
        checking: bool,
    ) -> Result<(), BackendError> {
        match access {
            StorageAccess::Read(local) | StorageAccess::Contents(local, _) => {
                if checking {
                    if self.ended.contains(&local) {
                        return Err(malformed_mir(
                            "resource view is used after its source ownership ended",
                        ));
                    }
                    let writable = matches!(access, StorageAccess::Contents(_, true));
                    self.conflicting_access(local, writable)?;
                    for place in &self.roots[local.0] {
                        temporary_conflict(acquired, place, writable)?;
                    }
                }
            }
            StorageAccess::ContentsValue(value) if checking => {
                let mut places = self.sources(program, value)?;
                if let Some(local) = value.direct_place_local() {
                    places.extend(self.roots[local.0].iter().cloned());
                }
                for place in places {
                    self.conflicting_place(
                        &place,
                        false,
                        &self.ancestors(Self::value_parents(program, value)?),
                    )?;
                    temporary_conflict(acquired, &place, false)?;
                }
            }
            StorageAccess::Acquire(value, writable) => {
                let places = retained::source_roots(program, value, &self.roots)?;
                for place in &places {
                    if checking {
                        self.conflicting_place(
                            place,
                            writable,
                            &self.ancestors(Self::value_parents(program, value)?),
                        )?;
                        temporary_conflict(acquired, place, writable)?;
                    }
                }
                for place in places {
                    *acquired.entry(place).or_default() |= writable;
                }
            }
            StorageAccess::Transfer(local) => {
                if checking {
                    self.conflicts(local)?;
                    for place in &self.roots[local.0] {
                        temporary_conflict(acquired, place, true)?;
                    }
                }
                if byte_regions::kind(local_in(function, local)?.ty).is_some() {
                    self.end_carrier(local);
                } else {
                    self.end(local);
                }
            }
            StorageAccess::Independent(value) if checking => {
                // Resolve the result's obligations before evaluating a call
                // which may retire its selected source's stored membership.
                self.independent_value(program, function, value)?;
            }
            StorageAccess::ContentsValue(_) | StorageAccess::Independent(_) => {}
        }
        Ok(())
    }

    fn independent_value(
        &self,
        program: &mir::Program,
        function: &mir::Function,
        value: &mir::Rvalue,
    ) -> Result<(), BackendError> {
        let stored = self
            .storage
            .value(program, value, &self.roots, &self.loans)?;
        let sources = match stored.normalized_sources() {
            Some(sources) => sources,
            None => retained::value_sources(program, value, &self.roots, &self.loans)?,
        };
        retained::validate_independent(function, &sources)
    }

    fn temporary_call_sources(
        &self,
        program: &mir::Program,
        accesses: &ClassLocalAccesses<'_>,
        position: usize,
        target: CallTarget,
        args: &[mir::Rvalue],
    ) -> Result<HashMap<Place, bool>, BackendError> {
        let mut result = HashMap::<Place, bool>::new();
        match target {
            CallTarget::Direct {
                function,
                parameter_offset,
            } => {
                let receiver = accesses
                    .method_receivers
                    .iter()
                    .find_map(|(at, _, receiver)| (*at == position).then_some(*receiver));
                let mut inputs = Vec::new();
                if let Some(receiver) = receiver {
                    inputs.push(mir::Rvalue::NullableClass(receiver.clone()));
                    inputs.extend_from_slice(args);
                }
                let args = if receiver.is_some() { &inputs } else { args };
                let constructing = parameter_offset == 1 && receiver.is_none();
                for source in retained::call_sources(
                    program,
                    function,
                    args,
                    constructing,
                    &self.roots,
                    &self.loans,
                )? {
                    *result.entry(source.place).or_default() |= source.writable;
                }
                if !constructing {
                    for callee in retained::possible_callees(
                        program,
                        function,
                        retained::receiver_class(args),
                    )? {
                        if let Some(borrow) = callee.return_borrow {
                            if borrow.kind == mir::ReturnBorrowKind::Value {
                                let source =
                                    borrowed_call_rvalue_source(program, callee.id, args, borrow)?;
                                let pipe = child_pipe_place(program, callee.id)?;
                                for place in retained::source_roots(program, source, &self.roots)? {
                                    let place =
                                        pipe.map_or(place.clone(), |slot| place.project(slot));
                                    *result.entry(place).or_default() |= borrow.writable;
                                }
                            }
                        }
                    }
                }
            }
            CallTarget::Indirect(function_type) => {
                if let Some(borrow) = function_type_in(program, function_type)?.return_borrow {
                    let mir::BorrowSource::Parameter(index) = borrow.source else {
                        return Err(malformed_mir(
                            "indirect returned borrow has no parameter source",
                        ));
                    };
                    let source = args.get(index).ok_or_else(|| {
                        malformed_mir("indirect returned borrow source is missing")
                    })?;
                    for place in retained::source_roots(program, source, &self.roots)? {
                        *result.entry(place).or_default() |= borrow.writable;
                    }
                }
            }
        }
        Ok(result)
    }

    fn stores(statement: &mir::Statement) -> bool {
        match statement {
            mir::Statement::CoreCollection { operation, .. } => operation.mutates(),
            mir::Statement::CollectionAdd { .. }
            | mir::Statement::CollectionSet { .. }
            | mir::Statement::AssignCollectionIndex { .. }
            | mir::Statement::CollectionClear { .. }
            | mir::Statement::AssignProperty { .. }
            | mir::Statement::AssignByteRegionIndex { .. }
            | mir::Statement::CopyWithinByteRegion { .. } => true,
            _ => false,
        }
    }

    /// The effect of a store, checked once its operands have been evaluated.
    fn store_conflicts(&self, statement: &mir::Statement) -> Result<(), BackendError> {
        match statement {
            mir::Statement::CoreCollection { collection, .. }
            | mir::Statement::CollectionAdd { collection, .. }
            | mir::Statement::CollectionSet { collection, .. }
            | mir::Statement::AssignCollectionIndex { collection, .. }
            | mir::Statement::CollectionClear { collection, .. } => self.conflicts(*collection),
            mir::Statement::AssignProperty {
                object, property, ..
            } => {
                for place in &self.roots[object.0] {
                    self.conflicting_place(
                        &place.project(*property),
                        true,
                        &self.ancestors([*object]),
                    )?;
                }
                Ok(())
            }
            mir::Statement::AssignByteRegionIndex { region, .. }
            | mir::Statement::CopyWithinByteRegion { region, .. } => self.conflicts(*region),
            _ => Ok(()),
        }
    }

    /// `after` holds the remaining uses once the statement completes. A store
    /// evaluates its operands before its effect, so the effect conflicts only
    /// with what remains live afterwards; without `after`, conservatively with
    /// everything live before the statement.
    pub(super) fn statement(
        &mut self,
        program: &mir::Program,
        function: &mir::Function,
        statement: &mir::Statement,
        checking: bool,
        after: Option<&Holds>,
    ) -> Result<(), BackendError> {
        if let mir::Statement::EndBorrow { local } = statement {
            self.end_carrier(*local);
            return Ok(());
        }
        if checking {
            if let mir::Statement::AssignLocal { value, .. } = statement {
                for local in escaping_class_local_borrows(program, value)? {
                    if self.ended.contains(&local) {
                        return Err(malformed_mir(format!(
                            "view local{} is used after its source ownership ended",
                            local.0
                        )));
                    }
                }
            }
        }
        let assignment = match statement {
            mir::Statement::AssignLocal { target, .. }
                if function.is_writable_borrowed_place(*target) =>
            {
                // Replacement changes the value, never the caller-owned home.
                Some((*target, self.roots[target.0].clone()))
            }
            mir::Statement::AssignLocal { target, value }
                if byte_regions::kind(value.ty()).is_some() =>
            {
                Some((*target, HashSet::from([Place::local(*target)])))
            }
            mir::Statement::CoreCollection {
                collection,
                operation:
                    mir::CoreCollectionOperation::KeyAt { target, .. }
                    | mir::CoreCollectionOperation::ValueAt { target, .. },
            } => Some((*target, self.roots[collection.0].clone())),
            mir::Statement::AssignLocal { target, value }
                if !local_in(function, *target)?.owned =>
            {
                Some((*target, self.sources(program, value)?))
            }
            mir::Statement::AssignLocal { target, value } => {
                let mut roots = if value.mixed_ownership() == mir::MixedOwnership::ShellOnly {
                    self.sources(program, value)?
                } else if let Some(source) = value.direct_place_local() {
                    // Moving an owned shell changes its home, not the lifetime
                    // of any payload it borrows from another owner or lease.
                    self.roots[source.0]
                        .iter()
                        .filter(|root| root.local != source)
                        .cloned()
                        .collect()
                } else {
                    HashSet::new()
                };
                roots.insert(Place::local(*target));
                Some((*target, roots))
            }
            _ => None,
        };
        let assigned_loans = if let mir::Statement::AssignLocal { value, .. }
        | mir::Statement::AssignLocalGroup { value, .. } = statement
        {
            retained::value_sources(program, value, &self.roots, &self.loans)?
        } else {
            HashSet::new()
        };
        let assigned_payload = if let mir::Statement::AssignLocal { value, .. } = statement {
            self.exact_payload_loans(program, value)?
        } else {
            None
        };
        let assigned_parents = if let mir::Statement::AssignLocal { value, .. }
        | mir::Statement::AssignLocalGroup { value, .. } = statement
        {
            byte_regions::value_parents(program, function, value, &self.parents)?
        } else {
            HashSet::new()
        };
        let assigned_view_parents = match statement {
            mir::Statement::AssignLocal { value, .. } => Self::value_parents(program, value)?,
            mir::Statement::CoreCollection { collection, .. } => HashSet::from([*collection]),
            _ => HashSet::new(),
        };
        let assigned_storage = match statement {
            mir::Statement::AssignLocal { value, .. }
            | mir::Statement::AssignLocalGroup { value, .. } => {
                Some(
                    self.storage
                        .value(program, value, &self.roots, &self.loans)?,
                )
            }
            _ => None,
        };
        let collection_result = match statement {
            mir::Statement::CoreCollection {
                collection,
                operation:
                    mir::CoreCollectionOperation::KeyAt { .. }
                    | mir::CoreCollectionOperation::ValueAt { .. }
                    | mir::CoreCollectionOperation::Remove { .. }
                    | mir::CoreCollectionOperation::Exchange { .. },
            } => Some((
                self.loans[collection.0].clone(),
                self.storage.collection_value(*collection),
            )),
            _ => None,
        };
        let stored_collection = match statement {
            mir::Statement::CollectionAdd {
                span,
                collection,
                value,
                op,
                ..
            } if *op != mir::CollectionMutationOp::Remove => {
                Some((*collection, Some(*span), vec![value], Vec::new()))
            }
            mir::Statement::CollectionSet {
                index_span,
                collection,
                key,
                value,
            } => Some((*collection, Some(*index_span), vec![key, value], Vec::new())),
            mir::Statement::AssignCollectionIndex {
                index_span,
                collection,
                value,
                ..
            } => Some((*collection, Some(*index_span), vec![value], Vec::new())),
            mir::Statement::CoreCollection {
                collection,
                operation,
            } => {
                let origin = match operation {
                    mir::CoreCollectionOperation::Insert { storage_origin, .. }
                    | mir::CoreCollectionOperation::Exchange { storage_origin, .. } => {
                        *storage_origin
                    }
                    _ => None,
                };
                let inputs = operation.transfers();
                (!inputs.is_empty()).then_some((*collection, origin, Vec::new(), inputs))
            }
            _ => None,
        }
        .map(|(collection, origin, values, locals)| {
            let incoming = self.storage.collection_inputs(
                program,
                &values,
                &locals,
                &self.roots,
                &self.loans,
            )?;
            Ok::<_, BackendError>((collection, origin, incoming))
        })
        .transpose()?;
        if checking {
            for source in &assigned_loans {
                if !source.inherited && self.ended.contains(&source.local) {
                    return Err(malformed_mir("iterator retains an ended source owner"));
                }
            }
            let stored = match statement {
                mir::Statement::AssignProperty {
                    object,
                    property,
                    value,
                    ..
                } if property_in(program, property.class, *property)?
                    .borrowed_source
                    .is_none() =>
                {
                    Some((Some(*object), value))
                }
                mir::Statement::AssignStatic { value, .. } => Some((None, value)),
                _ => None,
            };
            if let Some((object, value)) = stored {
                if let Some(object) = object {
                    let stored = self
                        .storage
                        .value(program, value, &self.roots, &self.loans)?;
                    let sources = match stored.normalized_sources() {
                        Some(sources) => sources,
                        None => retained::value_sources(program, value, &self.roots, &self.loans)?,
                    };
                    retained::validate_property_storage(program, function, object, &sources)?;
                } else {
                    self.independent_value(program, function, value)?;
                }
            }
        }
        self.accesses(
            program,
            function,
            &collect_statement_class_local_accesses(statement),
            checking,
        )?;
        if checking && Self::stores(statement) {
            let completed = after.map(|after| {
                let mut completed = self.clone();
                completed.expire_dead_last_use_sources(program, function, after);
                completed
            });
            completed
                .as_ref()
                .unwrap_or(self)
                .store_conflicts(statement)?;
        }
        if let Some((collection, origin, incoming)) = stored_collection {
            if checking
                && incoming
                    .sources()
                    .iter()
                    .any(|source| !source.inherited && self.ended.contains(&source.local))
            {
                return Err(malformed_mir("collection retains an ended source owner"));
            }
            self.store_collection_sources(collection, origin, incoming, function)?;
        }
        if let mir::Statement::CollectionClear { collection, .. } = statement {
            self.clear_collection_sources(*collection);
        }
        if let Some((target, roots)) = assignment {
            if checking {
                self.conflicting_definition(function, target)?;
            }
            self.assign(function, target, roots);
            self.view_parents[target.0] = assigned_view_parents;
            self.loans[target.0] = assigned_loans.clone();
            if let Some(payload) = assigned_payload {
                self.payload_loans.insert(target, payload);
            }
            self.parents[target.0] = assigned_parents.clone();
            if let Some(value) = &assigned_storage {
                self.storage
                    .assign(target, value, local_in(function, target)?.owned);
                if let Some(sources) = value.normalized_sources() {
                    self.loans[target.0] = sources;
                }
            }
        }
        match statement {
            mir::Statement::CoreCollection { operation, .. } => {
                if !matches!(
                    operation,
                    mir::CoreCollectionOperation::KeyAt { .. }
                        | mir::CoreCollectionOperation::ValueAt { .. }
                ) {
                    for target in operation.outputs() {
                        if checking {
                            self.conflicting_definition(function, target)?;
                        }
                        let roots = if local_in(function, target)?.owned {
                            HashSet::from([Place::local(target)])
                        } else {
                            HashSet::new()
                        };
                        self.assign(function, target, roots);
                        self.loans[target.0].clear();
                    }
                }
            }
            mir::Statement::AssignLocalGroup { targets, .. } => {
                for target in targets {
                    if checking {
                        self.conflicting_definition(function, *target)?;
                    }
                    self.assign(function, *target, HashSet::new());
                    self.loans[target.0] = assigned_loans.clone();
                    self.parents[target.0] = assigned_parents.clone();
                    if let Some(value) = &assigned_storage {
                        self.storage
                            .assign(*target, value, local_in(function, *target)?.owned);
                        if let Some(sources) = value.normalized_sources() {
                            self.loans[target.0] = sources;
                        }
                    }
                }
            }
            mir::Statement::ExtractErrorObject { target, .. } => {
                self.assign(function, *target, HashSet::new())
            }
            // The moved field may hold any of its receiver's loans.
            mir::Statement::TakeProperty { target, object, .. } => {
                let loans = self.loans[object.0].clone();
                self.assign(function, *target, HashSet::new());
                self.loans[target.0] = loans;
            }
            mir::Statement::BindPayloadEnumFields {
                source,
                case,
                mode,
                targets,
                ..
            } => {
                let source_loans = self.loans[source.0].clone();
                let source_roots = self.roots[source.0].clone();
                let borrows_payload = *mode != mir::MatchBindingMode::ConsumedArm;
                if checking && !borrows_payload {
                    self.conflicts(*source)?;
                }
                for (index, target) in targets.iter().enumerate() {
                    let definition = local_in(function, *target)?;
                    if checking {
                        self.conflicting_definition(function, *target)?;
                    }
                    let borrows_storage = borrows_payload && definition.ty.has_move_ownership();
                    let roots = if borrows_storage {
                        source_roots.clone()
                    } else {
                        HashSet::new()
                    };
                    self.assign(function, *target, roots);
                    let field = self.payload_loans.get(source).map_or_else(
                        || PayloadFieldLoans {
                            sources: if retained::carries_source(program, definition.ty) {
                                source_loans.clone()
                            } else {
                                HashSet::new()
                            },
                            nested: None,
                        },
                        |fields| fields.get(&(*case, index)).cloned().unwrap_or_default(),
                    );
                    self.loans[target.0] = field.sources;
                    if let Some(nested) = field.nested {
                        self.payload_loans.insert(*target, nested);
                    }
                    if borrows_storage {
                        self.view_parents[target.0].insert(*source);
                    }
                }
                if !borrows_payload {
                    let moved = targets
                        .iter()
                        .flat_map(|target| &self.loans[target.0])
                        .cloned()
                        .collect::<HashSet<_>>();
                    self.loans[source.0].retain(|loan| !moved.contains(loan));
                    self.payload_loans.remove(source);
                }
            }
            mir::Statement::CleanupConstructorPhase { object: local, .. }
            | mir::Statement::DropClass { local, .. }
            | mir::Statement::DropError { local }
            | mir::Statement::DropMixed { local }
            | mir::Statement::DropFunction { local, .. }
            | mir::Statement::DropCollection { local, .. }
            | mir::Statement::DropPayloadEnum { local, .. }
            | mir::Statement::DropSharedReference { local, .. }
            | mir::Statement::DropWeakReference { local, .. }
            | mir::Statement::DropWritableSharedReference { local, .. }
            | mir::Statement::DropWritableWeakReference { local, .. }
            | mir::Statement::DropSharedReferenceAccess { local, .. } => {
                if checking {
                    self.conflicts(*local)?;
                }
                self.end(*local);
            }
            _ => {}
        }
        if let (Some((sources, storage)), mir::Statement::CoreCollection { operation, .. }) =
            (collection_result, statement)
        {
            for target in operation.outputs() {
                let local = local_in(function, target)?;
                if retained::carries_source(program, local.ty) {
                    self.loans[target.0] = storage
                        .normalized_sources()
                        .unwrap_or_else(|| sources.clone());
                    self.storage.assign(target, &storage, local.owned);
                }
            }
        }
        Ok(())
    }

    pub(super) fn edge(
        &self,
        program: &mir::Program,
        function: &mir::Function,
        block: mir::BlockId,
        terminator: &mir::Terminator,
        target: mir::BlockId,
        callback_origins: &call_origins::CallbackOrigins,
    ) -> Result<Self, BackendError> {
        let mut state = self.clone();
        if let mir::Terminator::Branch {
            condition,
            then_block,
            else_block,
        } = terminator
        {
            if then_block != else_block {
                state.narrow_absent_payload(condition, target == *then_block);
            }
        }
        if let mir::Terminator::Branch {
            condition:
                mir::BoolExpression::PayloadEnumIsCase {
                    local, ty, case, ..
                },
            then_block,
            else_block,
        } = terminator
        {
            if then_block != else_block {
                let selected = target == *then_block;
                if let Some(fields) = state.payload_loans.get_mut(local) {
                    fields.retain(|(candidate, _), _| (*candidate == *case) == selected);
                    state.loans[local.0] = fields
                        .values()
                        .flat_map(|field| &field.sources)
                        .cloned()
                        .collect();
                    state
                        .storage
                        .retain_sources(*local, |source| state.loans[local.0].contains(source));
                } else if program.enums.get(ty.id.0).is_some_and(|definition| {
                    definition
                        .cases
                        .get(case.index)
                        .is_some_and(|tested| tested.id == *case)
                        && definition
                            .cases
                            .iter()
                            .filter(|candidate| (candidate.id == *case) == selected)
                            .all(|candidate| {
                                candidate
                                    .payload
                                    .iter()
                                    .all(|field| !retained::carries_source(program, field.ty))
                            })
                }) {
                    state.loans[local.0].clear();
                    state.storage.retain_sources(*local, |_| false);
                    state.payload_loans.insert(*local, HashMap::new());
                }
            }
        }
        let returned_storage = self.returned_storage_on_edge(
            program,
            terminator,
            target,
            callback_origins.get(&(function.id, block)),
        )?;
        let (result, borrow) = match terminator {
            mir::Terminator::CheckedCall {
                function: callee,
                args,
                result,
                success,
                ..
            } if *success == target => {
                let borrow = function_in(program, *callee)?
                    .return_borrow
                    .map(|borrow| borrowed_call_rvalue_source(program, *callee, args, borrow))
                    .transpose()?;
                (*result, borrow)
            }
            mir::Terminator::IndirectCall {
                function_type,
                args,
                result,
                continuation: success,
                ..
            }
            | mir::Terminator::CheckedIndirectCall {
                function_type,
                args,
                result,
                success,
                ..
            } if *success == target => {
                let borrow = match function_type_in(program, *function_type)?.return_borrow {
                    Some(mir::ReturnBorrow {
                        source: mir::BorrowSource::Parameter(index),
                        ..
                    }) => args.get(index),
                    _ => None,
                };
                (*result, borrow)
            }
            mir::Terminator::CheckedCall { error, failure, .. }
            | mir::Terminator::CheckedIndirectCall { error, failure, .. }
            | mir::Terminator::CheckedConstruct { error, failure, .. }
            | mir::Terminator::CheckedIo { error, failure, .. }
            | mir::Terminator::CheckedDeadlineFromNow { error, failure, .. }
            | mir::Terminator::CheckedCancellationRequest { error, failure, .. }
                if *failure == target =>
            {
                (Some(*error), None)
            }
            mir::Terminator::CheckedConstruct {
                result, success, ..
            } if *success == target => (Some(*result), None),
            mir::Terminator::CheckedIo {
                result, success, ..
            }
            | mir::Terminator::CheckedDeadlineFromNow {
                result, success, ..
            }
            | mir::Terminator::CheckedCancellationRequest {
                result, success, ..
            } if *success == target => (*result, None),
            _ => (None, None),
        };
        let readiness_source = match terminator {
            mir::Terminator::CheckedIo {
                operation: mir::CheckedIoOperation::BorrowReadinessStream { set, .. },
                success,
                ..
            } if *success == target => Some(*set),
            _ => None,
        };
        if let Some(result) = result {
            let roots = if byte_regions::kind(local_in(function, result)?.ty).is_some() {
                HashSet::from([Place::local(result)])
            } else if !local_in(function, result)?.owned {
                if let Some(source) = readiness_source {
                    state.roots[source.0].clone()
                } else {
                    borrow
                        .map(|value| state.sources(program, value))
                        .transpose()?
                        .unwrap_or_default()
                }
            } else {
                HashSet::new()
            };
            let loans = match terminator {
                mir::Terminator::CheckedIo {
                    operation: mir::CheckedIoOperation::TransferGroupReadRegion { group, .. },
                    success,
                    ..
                } if *success == target => state.roots[group.0]
                    .iter()
                    .cloned()
                    .map(|place| Source::new(place, false, false, mir::RetainedLifetime::LastUse))
                    .collect(),
                mir::Terminator::CheckedIo {
                    operation: mir::CheckedIoOperation::BeginTransferGroup { set, plan, .. },
                    success,
                    ..
                } if *success == target => {
                    let mut sources = state.loans[plan.0].clone();
                    sources.extend(state.roots[set.0].iter().cloned().map(|place| {
                        Source::new(place, false, true, mir::RetainedLifetime::Cleanup)
                    }));
                    sources
                }
                mir::Terminator::CheckedIo {
                    operation:
                        mir::CheckedIoOperation::ConfigureTransferPlan {
                            previous, region, ..
                        },
                    success,
                    ..
                } if *success == target => {
                    let mut sources = previous
                        .map(|previous| state.loans[previous.0].clone())
                        .unwrap_or_default();
                    sources.extend(state.loans[region.0].iter().cloned().map(|mut source| {
                        source.lifetime = mir::RetainedLifetime::Cleanup;
                        source
                    }));
                    sources
                }
                mir::Terminator::CheckedConstruct {
                    constructor,
                    args,
                    success,
                    ..
                } if *success == target => retained::call_sources(
                    program,
                    *constructor,
                    args,
                    true,
                    &state.roots,
                    &state.loans,
                )?,
                mir::Terminator::CheckedCall {
                    function: callee,
                    args,
                    success,
                    ..
                } if *success == target => retained::call_sources(
                    program,
                    *callee,
                    args,
                    false,
                    &state.roots,
                    &state.loans,
                )?,
                mir::Terminator::IndirectCall {
                    callee,
                    args,
                    continuation: success,
                    ..
                }
                | mir::Terminator::CheckedIndirectCall {
                    callee,
                    args,
                    success,
                    ..
                } if *success == target => retained::indirect_sources(
                    program,
                    callee,
                    args,
                    &state.roots,
                    &state.loans,
                    callback_origins.get(&(function.id, block)),
                )?,
                _ => HashSet::new(),
            };
            let parents = match terminator {
                mir::Terminator::CheckedCall {
                    function: callee,
                    args,
                    success,
                    ..
                } if *success == target => {
                    byte_regions::call_parents(program, function, *callee, args, &state.parents)?
                }
                mir::Terminator::IndirectCall {
                    function_type,
                    args,
                    continuation: success,
                    ..
                }
                | mir::Terminator::CheckedIndirectCall {
                    function_type,
                    args,
                    success,
                    ..
                } if *success == target => {
                    let signature = function_type_in(program, *function_type)?;
                    if let Some(mir::ReturnBorrow {
                        source: mir::BorrowSource::Parameter(index),
                        writable,
                        ..
                    }) = signature.return_borrow
                    {
                        let value = args.get(index).ok_or_else(|| {
                            malformed_mir("retained indirect parent argument is missing")
                        })?;
                        if signature.parameters[index].mode == mir::FunctionParameterMode::Take {
                            byte_regions::value_parents(program, function, value, &state.parents)?
                        } else {
                            byte_regions::reborrow_parents(
                                program,
                                function,
                                value,
                                writable,
                                &state.parents,
                            )?
                        }
                    } else {
                        HashSet::new()
                    }
                }
                _ => HashSet::new(),
            };
            let view_parents = if let Some(source) = readiness_source {
                HashSet::from([source])
            } else {
                borrow
                    .map(|value| Self::value_parents(program, value))
                    .transpose()?
                    .unwrap_or_default()
            };
            state.assign(function, result, roots);
            state.view_parents[result.0] = view_parents;
            state.loans[result.0] = loans;
            state.parents[result.0] = parents;
            if let Some(storage) = &returned_storage {
                state
                    .storage
                    .assign(result, storage, local_in(function, result)?.owned);
                // Unnormalized container contents do not replace a view's loan.
                if let Some(sources) = storage.normalized_sources() {
                    state.loans[result.0] = sources;
                }
            }
        }
        state.stored_call_edge(program, terminator, target)?;
        Ok(state)
    }

    /// Shared monotone join for ordinary views and whole-program pending proofs.
    pub(super) fn join(&mut self, incoming: Self) -> bool {
        let before = self.clone();
        self.storage
            .join(&incoming.storage, &self.loans, &incoming.loans);
        self.payload_loans.retain(|local, fields| {
            let Some(other) = incoming.payload_loans.get(local) else {
                return false;
            };
            merge_payload_loans(fields, other);
            true
        });
        for (roots, incoming) in self.roots.iter_mut().zip(incoming.roots) {
            roots.extend(incoming);
        }
        for (loans, incoming) in self.loans.iter_mut().zip(incoming.loans) {
            loans.extend(incoming);
        }
        for (parents, incoming) in self.parents.iter_mut().zip(incoming.parents) {
            parents.extend(incoming);
        }
        for (parents, incoming) in self.view_parents.iter_mut().zip(incoming.view_parents) {
            parents.extend(incoming);
        }
        for (access, incoming) in self.view_access.iter_mut().zip(incoming.view_access) {
            *access = match (*access, incoming) {
                (Some(left), Some(right)) => Some(left || right),
                (left, right) => left.or(right),
            };
        }
        self.ended.extend(incoming.ended);
        *self != before
    }

    pub(super) fn join_edge(&mut self, mut incoming: Self, cyclic: bool) -> bool {
        if cyclic {
            for (old, next) in self.roots.iter().zip(&mut incoming.roots) {
                *next = next
                    .drain()
                    .map(|mut place| {
                        place.widen_on_cycle(old.iter());
                        place
                    })
                    .collect();
            }
            for (old, next) in self.loans.iter().zip(&mut incoming.loans) {
                widen_sources_on_cycle(old, next);
            }
            for (local, fields) in &mut incoming.payload_loans {
                widen_payload_loans_on_cycle(&self.loans[local.0], fields);
            }
        }
        self.join(incoming)
    }
}

/// Reverse postorder avoids repeatedly propagating unfinished forward joins.
/// DFS backedges still govern widening on every reachable CFG cycle.
/// Reverse-postorder ranks and back edges of the reachable blocks.
pub(super) struct BlockOrder {
    back_edges: HashSet<(mir::BlockId, mir::BlockId)>,
    ranks: Vec<usize>,
}

impl BlockOrder {
    pub(super) fn new(function: &mir::Function) -> Result<Self, BackendError> {
        let mut marks = vec![0; function.blocks.len()];
        let mut edges = HashSet::new();
        let mut ranks = vec![usize::MAX; function.blocks.len()];
        let mut finished = 0;
        let mut stack = vec![(
            function.entry_block,
            analysis_terminator_targets(
                &block_in(function, function.entry_block)?.terminator,
                true,
            ),
        )];
        marks[function.entry_block.0] = 1;
        while let Some((id, targets)) = stack.last_mut() {
            if let Some(target) = targets.pop() {
                block_in(function, target)?;
                match marks[target.0] {
                    0 => {
                        marks[target.0] = 1;
                        stack.push((
                            target,
                            analysis_terminator_targets(
                                &block_in(function, target)?.terminator,
                                true,
                            ),
                        ));
                    }
                    1 => {
                        edges.insert((*id, target));
                    }
                    _ => {}
                }
            } else {
                marks[id.0] = 2;
                ranks[id.0] = finished;
                finished += 1;
                stack.pop();
            }
        }
        for rank in ranks.iter_mut().filter(|rank| **rank != usize::MAX) {
            *rank = finished - 1 - *rank;
        }
        Ok(Self {
            back_edges: edges,
            ranks,
        })
    }

    pub(super) fn is_back_edge(&self, from: mir::BlockId, to: mir::BlockId) -> bool {
        self.back_edges.contains(&(from, to))
    }
}

/// A worklist in reverse postorder whose nodes also carry the finalizer exits
/// selected on the way in (see [`super::FinalizerExits`]), so a shared
/// finalizer does not merge one exit's state into another exit's
/// continuation. Pushing a queued node again does not duplicate it.
pub(super) struct NodeWorklist {
    blocks: BlockOrder,
    pending: std::collections::BTreeSet<NodeKey>,
}

/// A node ordered by its block's rank, then block and selections.
type NodeKey = (usize, usize, Vec<(usize, usize)>);

pub(super) type Node = (mir::BlockId, super::FinalizerSelections);

pub(super) fn node_key(selections: &super::FinalizerSelections) -> Vec<(usize, usize)> {
    selections
        .iter()
        .map(|(completion, index)| (completion.0, *index))
        .collect()
}

impl NodeWorklist {
    pub(super) fn new(function: &mir::Function) -> Result<Self, BackendError> {
        let blocks = BlockOrder::new(function)?;
        let mut worklist = Self {
            blocks,
            pending: std::collections::BTreeSet::new(),
        };
        worklist.push((function.entry_block, Vec::new()));
        Ok(worklist)
    }

    pub(super) fn push(&mut self, (block, selections): Node) {
        self.pending
            .insert((self.blocks.ranks[block.0], block.0, node_key(&selections)));
    }

    pub(super) fn pop(&mut self) -> Option<Node> {
        self.pending.pop_first().map(|(_, block, selections)| {
            (
                mir::BlockId(block),
                selections
                    .into_iter()
                    .map(|(completion, index)| (mir::BlockId(completion), index))
                    .collect(),
            )
        })
    }

    pub(super) fn is_back_edge(&self, from: mir::BlockId, to: mir::BlockId) -> bool {
        self.blocks.is_back_edge(from, to)
    }
}

/// The successors of a node: the selected exit's continuation at a finalizer
/// completion entered through a known exit, otherwise every analysis target.
pub(super) fn node_successors(
    finalizers: &super::FinalizerExits<'_>,
    block: &mir::BasicBlock,
    incoming: &super::FinalizerSelections,
) -> Vec<Node> {
    let mut selections = incoming.clone();
    finalizers.enter(block.id, &mut selections);
    match finalizers.selected(block.id, &selections) {
        Some(selected) => vec![selected],
        None => analysis_terminator_targets(&block.terminator, true)
            .into_iter()
            .map(|target| (target, selections.clone()))
            .collect(),
    }
}

/// Only actual later uses extend LastUse loans. Lexical EndBorrow and
/// DropFunction bookkeeping does not use borrowed captures; cleanup-dependent
/// loans remain live independently until the owned environment is destroyed.
/// Reuse the canonical MIR access and CFG walkers, including checked edge defs.
/// Each live local also records what it still uses (see [`Hold`]).
struct LastUseLiveness {
    statements: Vec<Vec<Holds>>,
    terminators: Vec<Holds>,
}

impl LastUseLiveness {
    /// A Copy payload enum can hold a region, so its loans also end at last use.
    fn tracks(program: &mir::Program, ty: mir::Type) -> bool {
        byte_regions::kind(ty).is_some()
            || ty.has_move_ownership()
            || matches!(
                ty,
                mir::Type::PayloadEnum(_) | mir::Type::NullablePayloadEnum(_)
            ) && program.carries_retained_sources(ty)
    }

    /// What each tracked local is used for by one statement or terminator.
    /// Only synthetic carriers distinguish field uses; a named binding or
    /// owner is always used as a whole.
    fn uses(
        program: &mir::Program,
        function: &mir::Function,
        accesses: &ClassLocalAccesses<'_>,
        store: Option<(mir::LocalId, crate::class_layout::PropertyId)>,
    ) -> Holds {
        let definition = |local: mir::LocalId| {
            function
                .locals
                .get(local.0)
                .filter(|definition| Self::tracks(program, definition.ty))
        };
        let mut holds = Holds::new();
        let mut field_use =
            |local: mir::LocalId, field: Option<(crate::class_layout::PropertyId, FieldUse)>| {
                let Some(definition) = definition(local) else {
                    return;
                };
                let hold = match field {
                    Some((field, used)) if definition.synthetic && !definition.owned => {
                        Hold::Fields(HashMap::from([(field, used)]))
                    }
                    _ => Hold::Whole,
                };
                merge_holds(&mut holds, &HashMap::from([(local, hold)]));
            };
        for local in accesses
            .resource_reads
            .iter()
            .chain(&accesses.resource_transfers)
            .chain(&accesses.collection_mutations)
        {
            field_use(*local, None);
        }
        for access in accesses.iter() {
            match access {
                ClassLocalAccess::Borrow(local) | ClassLocalAccess::Transfer(local) => {
                    field_use(local, None)
                }
                ClassLocalAccess::PropertyBorrow(local, property) => {
                    field_use(local, Some((property, FieldUse::Read)))
                }
                ClassLocalAccess::BeginCall
                | ClassLocalAccess::Call(_, _)
                | ClassLocalAccess::BeginConditional
                | ClassLocalAccess::EndConditional => {}
            }
        }
        if let Some((object, property)) = store {
            field_use(object, Some((property, FieldUse::Store)));
        }
        holds
    }

    fn before_block(
        program: &mir::Program,
        function: &mir::Function,
        block: &mir::BasicBlock,
        entries: &[Holds],
    ) -> Result<(Vec<Holds>, Holds), BackendError> {
        let mut live = Holds::new();
        for target in analysis_terminator_targets(&block.terminator, true) {
            block_in(function, target)?;
            let mut incoming = entries[target.0].clone();
            let defined = match &block.terminator {
                mir::Terminator::CheckedCall {
                    result, success, ..
                }
                | mir::Terminator::CheckedIndirectCall {
                    result, success, ..
                }
                | mir::Terminator::CheckedIo {
                    result, success, ..
                }
                | mir::Terminator::CheckedDeadlineFromNow {
                    result, success, ..
                }
                | mir::Terminator::CheckedCancellationRequest {
                    result, success, ..
                } if *success == target => *result,
                mir::Terminator::CheckedConstruct {
                    result, success, ..
                } if *success == target => Some(*result),
                mir::Terminator::CheckedCall { error, failure, .. }
                | mir::Terminator::CheckedIndirectCall { error, failure, .. }
                | mir::Terminator::CheckedConstruct { error, failure, .. }
                | mir::Terminator::CheckedIo { error, failure, .. }
                | mir::Terminator::CheckedDeadlineFromNow { error, failure, .. }
                | mir::Terminator::CheckedCancellationRequest { error, failure, .. }
                    if *failure == target =>
                {
                    Some(*error)
                }
                mir::Terminator::IndirectCall {
                    result,
                    continuation,
                    ..
                } if *continuation == target => *result,
                _ => None,
            };
            if let Some(result) = defined {
                incoming.remove(&result);
            }
            merge_holds(&mut live, &incoming);
        }
        merge_holds(
            &mut live,
            &Self::uses(
                program,
                function,
                &collect_terminator_class_local_accesses(&block.terminator),
                None,
            ),
        );
        let terminator = live.clone();
        let mut statements = vec![Holds::new(); block.statements.len()];
        for (index, statement) in block.statements.iter().enumerate().rev() {
            match statement {
                mir::Statement::AssignLocal { target, .. }
                | mir::Statement::ExtractErrorObject { target, .. }
                | mir::Statement::TakeProperty { target, .. } => {
                    live.remove(target);
                }
                mir::Statement::AssignLocalGroup { targets, .. }
                | mir::Statement::BindPayloadEnumFields { targets, .. } => {
                    for target in targets {
                        live.remove(target);
                    }
                }
                mir::Statement::CoreCollection { operation, .. } => {
                    for target in operation.outputs() {
                        live.remove(&target);
                    }
                }
                _ => {}
            }
            let store = match statement {
                mir::Statement::AssignProperty {
                    object, property, ..
                } => Some((*object, *property)),
                _ => None,
            };
            merge_holds(
                &mut live,
                &Self::uses(
                    program,
                    function,
                    &collect_statement_class_local_accesses(statement),
                    store,
                ),
            );
            statements[index] = live.clone();
        }
        Ok((statements, terminator))
    }

    fn new(program: &mir::Program, function: &mir::Function) -> Result<Self, BackendError> {
        let mut entries = vec![Holds::new(); function.blocks.len()];
        loop {
            let mut changed = false;
            for block in function.blocks.iter().rev() {
                let (statements, terminator) =
                    Self::before_block(program, function, block, &entries)?;
                let incoming = statements.first().unwrap_or(&terminator);
                if entries[block.id.0] != *incoming {
                    entries[block.id.0] = incoming.clone();
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        let mut result = Self {
            statements: Vec::new(),
            terminators: Vec::new(),
        };
        for block in &function.blocks {
            let (statements, terminator) = Self::before_block(program, function, block, &entries)?;
            result.statements.push(statements);
            result.terminators.push(terminator);
        }
        Ok(result)
    }
}

pub(super) fn validate(
    program: &mir::Program,
    function: &mir::Function,
    callback_origins: &call_origins::CallbackOrigins,
) -> Result<(), BackendError> {
    let mut returned_selectors = HashSet::new();
    let mut returned_storage = Vec::new();
    let mut normal_removals = Vec::new();
    let mut exceptional_removals = Vec::new();
    let liveness = LastUseLiveness::new(program, function)?;
    let finalizers = super::FinalizerExits::of(function);
    let mut pending = NodeWorklist::new(function)?;
    let mut entries = HashMap::<Node, State>::new();
    entries.insert(
        (function.entry_block, Vec::new()),
        State::new(program, function),
    );
    while let Some(node) = pending.pop() {
        let id = node.0;
        let mut state = entries[&node].clone();
        let block = block_in(function, id)?;
        for (index, statement) in block.statements.iter().enumerate() {
            state.expire_dead_last_use_sources(
                program,
                function,
                &liveness.statements[id.0][index],
            );
            state.statement(program, function, statement, false, None)?;
        }
        state.expire_dead_last_use_sources(program, function, &liveness.terminators[id.0]);
        state.accesses(
            program,
            function,
            &collect_terminator_class_local_accesses(&block.terminator),
            false,
        )?;
        for successor in node_successors(&finalizers, block, &node.1) {
            let target = successor.0;
            block_in(function, target)?;
            let outgoing = state.edge(
                program,
                function,
                id,
                &block.terminator,
                target,
                callback_origins,
            )?;
            let changed = match entries.get_mut(&successor) {
                None => {
                    entries.insert(successor.clone(), outgoing);
                    true
                }
                Some(existing) => existing.join_edge(outgoing, pending.is_back_edge(id, target)),
            };
            if changed {
                pending.push(successor);
            }
        }
    }
    let mut nodes = entries.into_iter().collect::<Vec<_>>();
    nodes.sort_by_key(|((block, selections), _)| (block.0, node_key(selections)));
    for ((block_id, _), mut state) in nodes {
        let block = block_in(function, block_id)?;
        for (index, statement) in block.statements.iter().enumerate() {
            state.expire_dead_last_use_sources(
                program,
                function,
                &liveness.statements[block.id.0][index],
            );
            let after = liveness.statements[block.id.0]
                .get(index + 1)
                .unwrap_or(&liveness.terminators[block.id.0]);
            state
                .statement(program, function, statement, true, Some(after))
                .map_err(|mut error| {
                    error.message.push_str(&format!(
                        " in `{}` block{} statement{index}",
                        function.name, block.id.0
                    ));
                    error
                })?;
        }
        state.expire_dead_last_use_sources(program, function, &liveness.terminators[block.id.0]);
        if let mir::Terminator::Return(value) = &block.terminator {
            returned_selectors.extend(state.returned_storage_selectors(program, value)?);
            returned_storage.push(state.returned_storage(program, function, value)?);
            let sources = retained::value_sources(program, value, &state.roots, &state.loans)?;
            if matches!(value.ty(), mir::Type::ByteRegion(_)) && sources.is_empty() {
                return Err(malformed_mir(
                    "byte region return has no retained source provenance",
                ));
            }
            retained::validate_return(program, function, &sources)?;
        }
        retained::validate_indirect_inputs(
            program,
            function,
            &block.terminator,
            &state.roots,
            &state.loans,
        )?;
        state
            .accesses(
                program,
                function,
                &collect_terminator_class_local_accesses(&block.terminator),
                true,
            )
            .map_err(|mut error| {
                error.message.push_str(&format!(
                    " in `{}` block{} terminator",
                    function.name, block.id.0
                ));
                error
            })?;
        match &block.terminator {
            mir::Terminator::Return(_) | mir::Terminator::ReturnVoid => {
                state.validate_stored_exit(function, false)?;
                normal_removals.push(state.removed_storage(function));
            }
            mir::Terminator::PropagateError { .. } => {
                state.validate_stored_exit(function, true)?;
                exceptional_removals.push(state.removed_storage(function));
            }
            _ => {}
        }
    }
    let declared = retained::contract(function)
        .map(|plan| plan.returned_selectors.iter().cloned().collect())
        .unwrap_or_default();
    if returned_selectors != declared {
        return Err(malformed_mir(format!(
            "returned ownership selectors disagree with executable storage provenance in `{}`: inferred {returned_selectors:?}, declared {declared:?}",
            function.name
        )));
    }
    stored_owners::validate_returned_storage(function, returned_storage)?;
    stored_owners::validate_removals(function, normal_removals, exceptional_removals)?;
    Ok(())
}

#[cfg(test)]
mod worklist_tests {
    use super::*;

    #[test]
    fn nested_payload_loan_paths_widen_with_the_carrier_on_loop_edges() {
        use crate::class_layout::{ClassId, PropertyId};
        use crate::enums::{EnumCaseId, EnumId};

        let owner = mir::LocalId(0);
        let property = PropertyId {
            class: ClassId(0),
            index: 0,
        };
        let prefix = Place::local(owner).project(property);
        let previous = HashSet::from([Source::new(
            prefix.clone(),
            false,
            true,
            mir::RetainedLifetime::Cleanup,
        )]);
        let grown = Source::new(
            prefix.project(property),
            false,
            true,
            mir::RetainedLifetime::Cleanup,
        );
        let case = (
            EnumCaseId {
                enum_id: EnumId(0),
                index: 0,
            },
            0,
        );
        let mut payload = HashMap::from([(
            case,
            PayloadFieldLoans {
                sources: HashSet::from([grown.clone()]),
                nested: Some(HashMap::from([(
                    case,
                    PayloadFieldLoans {
                        sources: HashSet::from([grown]),
                        nested: None,
                    },
                )])),
            },
        )]);
        widen_payload_loans_on_cycle(&previous, &mut payload);
        let field = &payload[&case];
        let nested = &field.nested.as_ref().unwrap()[&case];
        assert_eq!(field.sources, nested.sources);
        let loan = nested.sources.iter().next().unwrap();
        assert!(loan.place.opaque);
        assert_eq!(loan.contract.owned_path, loan.place.fields);
    }

    #[test]
    fn consumed_payload_bindings_keep_their_own_field_loans() {
        let program = crate::lower_source_to_mir(
            "split-payload-loans.doria",
            r#"
enum Split { case Both(function(): int $first, function(): int $second); }
function split(take Split $pair, function(): int $a, function(): int $b): int {
    return match (take $pair) { Split::Both($first, $second) => $first() + $second(), };
}
function main(): void {}
"#,
        )
        .unwrap();
        let function = program
            .functions
            .iter()
            .find(|f| f.name == "split")
            .unwrap();
        let binding = function
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .find(|statement| {
                matches!(
                    statement,
                    mir::Statement::BindPayloadEnumFields {
                        mode: mir::MatchBindingMode::ConsumedArm,
                        ..
                    }
                )
            })
            .unwrap();
        let mir::Statement::BindPayloadEnumFields {
            source,
            ty,
            case,
            targets,
            ..
        } = binding
        else {
            unreachable!();
        };
        let mut state = State::new(&program, function);
        let [_, a, b] = function.params.as_slice() else {
            panic!("expected two loan inputs");
        };
        let fields = [*a, *b]
            .into_iter()
            .map(|local| {
                let mir::Type::Function(function_type) = function.locals[local.0].ty else {
                    panic!("loan input must be a callback");
                };
                mir::Rvalue::Function(mir::FunctionExpression::Local {
                    function_type,
                    local,
                    transfer: false,
                })
            })
            .collect();
        let value = mir::Rvalue::PayloadEnum(mir::PayloadEnumExpression::Construct {
            ty: *ty,
            case: *case,
            fields,
            span: Span::default(),
        });
        let exact = state
            .exact_payload_loans(&program, &value)
            .unwrap()
            .unwrap();
        state.loans[source.0] = exact
            .values()
            .flat_map(|field| &field.sources)
            .cloned()
            .collect();
        state.payload_loans.insert(*source, exact);
        let mut guarded = state.clone();
        guarded.roots[a.0] = HashSet::from([Place::local(*source)]);
        guarded.view_access[a.0] = Some(false);
        let error = guarded
            .statement(&program, function, binding, true, None)
            .expect_err("a live readonly payload view prevents a consuming bind");
        assert!(error
            .message
            .contains("conflicts with a live borrowed view"));

        state
            .statement(&program, function, binding, false, None)
            .unwrap();
        assert_eq!(state.loans[targets[0].0], state.loans[a.0]);
        assert_eq!(state.loans[targets[1].0], state.loans[b.0]);
        assert_ne!(state.loans[targets[0].0], state.loans[targets[1].0]);
        assert!(state.loans[source.0].is_empty());
        assert!(!state.payload_loans.contains_key(source));
    }

    #[test]
    fn payload_case_edges_release_only_impossible_loans() {
        let program = crate::lower_source_to_mir(
            "payload-case-loans.doria",
            r#"
enum Retry { case Complete; case Pending(function(): int $resume); }
function classify(Retry $retry): void {}
function classifyNullable(?Retry $retry): void { if ($retry == null) { return; } }
function main(): void {}
"#,
        )
        .unwrap();
        let function = program
            .functions
            .iter()
            .find(|f| f.name == "classify")
            .unwrap();
        let local = function.params[0];
        let mir::Type::PayloadEnum(ty) = function.locals[local.0].ty else {
            panic!("retry must be a payload enum");
        };
        let cases = &program.enums[ty.id.0].cases;
        let state = State::new(&program, function);
        assert_eq!(state.loans[local.0].len(), 1);
        let origins = call_origins::CallbackOrigins::new();
        let then_block = mir::BlockId(1);
        let else_block = mir::BlockId(2);
        for (index, case) in cases.iter().enumerate() {
            let branch = mir::Terminator::Branch {
                condition: mir::BoolExpression::PayloadEnumIsCase {
                    local,
                    ty,
                    case: case.id,
                    nullable: false,
                },
                then_block,
                else_block,
            };
            for (target, selected) in [(then_block, true), (else_block, false)] {
                let next = state
                    .edge(
                        &program,
                        function,
                        mir::BlockId(0),
                        &branch,
                        target,
                        &origins,
                    )
                    .unwrap();
                assert_eq!(next.loans[local.0].is_empty(), (index == 0) == selected);
            }
        }

        let function = program
            .functions
            .iter()
            .find(|function| function.name == "classifyNullable")
            .unwrap();
        let local = function.params[0];
        let state = State::new(&program, function);
        assert_eq!(state.loans[local.0].len(), 1);
        let entry = &function.blocks[function.entry_block.0];
        let mir::Terminator::Branch {
            then_block,
            else_block,
            ..
        } = &entry.terminator
        else {
            panic!("nullable comparison should branch");
        };
        for (target, absent) in [(*then_block, true), (*else_block, false)] {
            let next = state
                .edge(
                    &program,
                    function,
                    entry.id,
                    &entry.terminator,
                    target,
                    &origins,
                )
                .unwrap();
            assert_eq!(next.loans[local.0].is_empty(), absent);
        }
    }

    fn graph(edges: &[Vec<usize>]) -> mir::Function {
        let program = crate::lower_source_to_mir(
            "block-worklist.doria",
            "function graph(bool $condition): void {} function main(): void {}",
        )
        .unwrap();
        let mut function = program
            .functions
            .iter()
            .find(|f| f.name == "graph")
            .unwrap()
            .clone();
        function.entry_block = mir::BlockId(0);
        function.blocks = edges
            .iter()
            .enumerate()
            .map(|(id, targets)| mir::BasicBlock {
                id: mir::BlockId(id),
                statements: Vec::new(),
                terminator: match targets.as_slice() {
                    [] => mir::Terminator::ReturnVoid,
                    [target] => mir::Terminator::Jump(mir::BlockId(*target)),
                    [then_block, else_block] => mir::Terminator::Branch {
                        condition: mir::BoolExpression::Use {
                            operand: mir::Operand::Local(function.params[0]),
                        },
                        then_block: mir::BlockId(*then_block),
                        else_block: mir::BlockId(*else_block),
                    },
                    _ => unreachable!(),
                },
            })
            .collect();
        function
    }

    fn solve(function: &mir::Function, ordered: bool) -> (Vec<u64>, usize) {
        assert!(function.blocks.len() < 64);
        let mut worklist = NodeWorklist::new(function).unwrap();
        let mut fifo = VecDeque::from([function.entry_block]);
        let mut queued = vec![false; function.blocks.len()];
        queued[function.entry_block.0] = true;
        let mut facts = vec![0; function.blocks.len()];
        let mut visits = 0;
        while let Some(id) = if ordered {
            worklist.pop().map(|(block, _)| block)
        } else {
            fifo.pop_front()
        } {
            queued[id.0] = false;
            visits += 1;
            facts[id.0] |= 1 << id.0;
            for target in analysis_terminator_targets(&function.blocks[id.0].terminator, true) {
                let incoming = facts[target.0] | facts[id.0];
                if incoming == facts[target.0] {
                    continue;
                }
                facts[target.0] = incoming;
                if ordered {
                    worklist.push((target, Vec::new()));
                } else if !queued[target.0] {
                    queued[target.0] = true;
                    fifo.push_back(target);
                }
            }
        }
        (facts, visits)
    }

    #[test]
    fn forward_joins_converge_once_per_block_without_changing_facts() {
        let mut edges = vec![Vec::new(); 33];
        for start in (0..32).step_by(4) {
            edges[start] = vec![start + 1, start + 2];
            edges[start + 1] = vec![start + 4];
            edges[start + 2] = vec![start + 3];
            edges[start + 3] = vec![start + 4];
        }
        let function = graph(&edges);
        let (ordered, visits) = solve(&function, true);
        let (fifo, fifo_visits) = solve(&function, false);
        assert_eq!(ordered, fifo);
        assert_eq!(visits, function.blocks.len());
        assert!(fifo_visits > visits, "{fifo_visits} vs {visits}");
    }

    #[test]
    fn cycles_revisit_changed_blocks_and_retain_the_fifo_fixed_point() {
        let mut revisited = false;
        for edges in [
            vec![vec![0, 1], vec![]],
            vec![vec![1, 2], vec![3], vec![3], vec![1, 4], vec![]],
            vec![vec![1], vec![2, 5], vec![3, 4], vec![2], vec![1], vec![]],
        ] {
            let function = graph(&edges);
            let order = BlockOrder::new(&function).unwrap();
            assert!(!order.back_edges.is_empty());
            let (ordered, visits) = solve(&function, true);
            assert_eq!(ordered, solve(&function, false).0);
            revisited |= visits > function.blocks.len();
        }
        assert!(revisited);
    }

    #[test]
    fn checked_constant_aware_reachability_and_queue_deduplication_are_preserved() {
        let mut function = graph(&[vec![1, 2], vec![], vec![]]);
        let mir::Terminator::Branch { condition, .. } = &mut function.blocks[0].terminator else {
            unreachable!()
        };
        *condition = mir::BoolExpression::Use {
            operand: mir::Operand::Scalar(mir::ScalarValue::Bool(true)),
        };
        let mut worklist = NodeWorklist::new(&function).unwrap();
        assert_eq!(worklist.blocks.ranks[2], usize::MAX);
        let block = |id| (mir::BlockId(id), Vec::new());
        worklist.push(block(1));
        worklist.push(block(1));
        assert_eq!(worklist.pop(), Some(block(0)));
        assert_eq!(worklist.pop(), Some(block(1)));
        assert_eq!(worklist.pop(), None);
        worklist.push(block(1));
        assert_eq!(worklist.pop(), Some(block(1)));
        function.blocks[0].terminator = mir::Terminator::Jump(mir::BlockId(99));
        assert!(BlockOrder::new(&function).is_err());

        let program = crate::lower_source_to_mir(
            "checked-block-worklist.doria",
            r#"class Failure implements Error { function __construct(string $message) {} }
function prepare(): void throws Failure {}
function graph(): void { try { prepare(); } catch (Failure) {} }
function main(): void {}"#,
        )
        .unwrap();
        let function = program
            .functions
            .iter()
            .find(|f| f.name == "graph")
            .unwrap();
        let worklist = BlockOrder::new(function).unwrap();
        let mut checked_calls = 0;
        for block in &function.blocks {
            if let mir::Terminator::CheckedCall {
                success, failure, ..
            } = block.terminator
            {
                checked_calls += 1;
                assert_ne!(worklist.ranks[success.0], usize::MAX);
                assert_ne!(worklist.ranks[failure.0], usize::MAX);
            }
        }
        assert_eq!(checked_calls, 1);
    }
}

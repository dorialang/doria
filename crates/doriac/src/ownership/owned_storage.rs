//! Stored ownership and its non-owning selectors have separate loan lifetimes.

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub origin: Span,
    pub callers: Vec<Span>,
}

impl Identity {
    fn at_call(&self, site: Span) -> Self {
        let mut identity = self.clone();
        if !identity.callers.contains(&site) {
            identity.callers.push(site);
        }
        identity
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selector {
    Member(Identity),
    Input(BorrowSource),
    Capture(BindingId),
    All,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Store {
    pub destination: BorrowSource,
    pub path: Vec<crate::class_layout::PropertyId>,
    pub identity: Identity,
    pub sources: Vec<RetainedSource>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub destination: BorrowSource,
    pub path: Vec<crate::class_layout::PropertyId>,
    pub selectors: Vec<Selector>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReturnedStorage {
    pub inputs: Vec<BorrowSource>,
    pub direct: Vec<RetainedSource>,
    pub members: Vec<ReturnedMember>,
    pub selections: Vec<Selection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReturnedMember {
    pub identity: Identity,
    pub path: Vec<crate::class_layout::PropertyId>,
    pub sources: Vec<RetainedSource>,
    pub repeated: bool,
}

impl ReturnedStorage {
    pub(super) fn retain_whole_receivers(&mut self) {
        retained::whole_receiver_sources(&mut self.direct);
        for member in &mut self.members {
            retained::whole_receiver_sources(&mut member.sources);
        }
    }

    pub(super) fn merge(&mut self, incoming: Self) {
        for selection in incoming.selections {
            if !self.selections.contains(&selection) {
                self.selections.push(selection);
            }
        }
        for input in incoming.inputs {
            if !self.inputs.contains(&input) {
                self.inputs.push(input);
            }
        }
        retained::append_sources(&mut self.direct, incoming.direct);
        for incoming in incoming.members {
            if let Some(member) = self
                .members
                .iter_mut()
                .find(|member| member.identity == incoming.identity && member.path == incoming.path)
            {
                retained::append_sources(&mut member.sources, incoming.sources);
                member.repeated |= incoming.repeated;
            } else {
                self.members.push(incoming);
            }
        }
    }
}

pub(super) fn append_stores(
    destination: &mut Vec<Store>,
    incoming: impl IntoIterator<Item = Store>,
) {
    for incoming in incoming {
        if let Some(store) = destination.iter_mut().find(|store| {
            store.destination == incoming.destination
                && store.path == incoming.path
                && store.identity == incoming.identity
        }) {
            retained::append_sources(&mut store.sources, incoming.sources);
        } else {
            destination.push(incoming);
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct Contents {
    direct: Option<Vec<RetainedLoan>>,
    members: Vec<Member>,
    selectors: Vec<Selector>,
    inputs: Vec<BorrowSource>,
    selections: Vec<(Selection, Vec<RetainedLoan>)>,
}

#[derive(Debug, Clone)]
struct Member {
    identity: Identity,
    path: Vec<crate::class_layout::PropertyId>,
    loans: Vec<RetainedLoan>,
    repeated: bool,
}

impl Contents {
    pub(super) fn retain_cleanup_loans(&mut self) {
        for loans in self
            .direct
            .iter_mut()
            .chain(self.members.iter_mut().map(|member| &mut member.loans))
            .chain(self.selections.iter_mut().map(|(_, loans)| loans))
        {
            loans.retain(|loan| loan.lifetime == RetainedLifetime::Cleanup);
        }
    }

    pub(super) fn input(&mut self, source: BorrowSource) {
        self.inputs.push(source);
    }

    pub(super) fn capture_selector(&mut self, binding: BindingId) {
        self.selectors = vec![Selector::Capture(binding)];
    }

    pub(super) fn merge_from(
        &mut self,
        incoming: &Self,
        loans: &[RetainedLoan],
        incoming_loans: &[RetainedLoan],
    ) {
        // A returned carrier can have loans without known member partitions.
        // Normalize both predecessors before joining their storage metadata.
        retained::append_loans(
            self.direct.get_or_insert_with(|| loans.to_vec()),
            incoming.direct.as_deref().unwrap_or(incoming_loans),
        );
        for input in &incoming.inputs {
            if !self.inputs.contains(input) {
                self.inputs.push(*input);
            }
        }
        for (selection, loans) in &incoming.selections {
            self.select(selection.clone(), loans.clone());
        }
        for incoming in &incoming.members {
            if let Some(member) = self
                .members
                .iter_mut()
                .find(|member| member.identity == incoming.identity && member.path == incoming.path)
            {
                retained::append_loans(&mut member.loans, &incoming.loans);
                member.repeated |= incoming.repeated;
            } else {
                self.members.push(incoming.clone());
            }
        }
        for selector in &incoming.selectors {
            if !self.selectors.contains(selector) {
                self.selectors.push(selector.clone());
            }
        }
    }

    fn insert(
        &mut self,
        identity: Identity,
        path: Vec<crate::class_layout::PropertyId>,
        loans: Vec<RetainedLoan>,
        direct: &[RetainedLoan],
    ) {
        self.direct.get_or_insert_with(|| direct.to_vec());
        if let Some(member) = self
            .members
            .iter_mut()
            .find(|member| member.identity == identity && member.path == path)
        {
            // A repeated acquisition site can denote several live members.
            // Removing one key must not discharge the others' obligations.
            member.repeated = true;
            retained::append_loans(&mut member.loans, &loans);
        } else {
            self.members.push(Member {
                identity,
                path,
                loans,
                repeated: false,
            });
        }
    }

    fn remove(
        &mut self,
        selectors: &[Selector],
        path: &[crate::class_layout::PropertyId],
        direct: &[RetainedLoan],
    ) -> Vec<RetainedLoan> {
        if selectors == [Selector::All] {
            let mut loans = Vec::new();
            if path.is_empty() {
                loans = self.loans();
                *self = Self {
                    direct: Some(Vec::new()),
                    ..Self::default()
                };
            } else {
                self.members.retain(|member| {
                    if member.path.starts_with(path) {
                        retained::append_loans(&mut loans, &member.loans);
                        false
                    } else {
                        true
                    }
                });
            }
            return loans;
        }
        // Incoming owners can contain members whose acquisition is outside
        // this callable. Selection preserves those unpartitioned obligations.
        let mut loans = self.direct.get_or_insert_with(|| direct.to_vec()).clone();
        let unknown = selectors.is_empty() || selectors.contains(&Selector::Unknown);
        self.members.retain(|member| {
            if member.path != path
                || (!unknown && !selectors.contains(&Selector::Member(member.identity.clone())))
            {
                return true;
            }
            retained::append_loans(&mut loans, &member.loans);
            // Unknown or joined selectors cannot prove which single owner left.
            unknown || selectors.len() != 1 || member.repeated
        });
        loans
    }

    fn select(&mut self, selection: Selection, loans: Vec<RetainedLoan>) {
        if let Some((_, existing)) = self.selections.iter_mut().find(|(s, _)| *s == selection) {
            retained::append_loans(existing, &loans);
        } else {
            self.selections.push((selection, loans));
        }
    }

    pub(super) fn loans(&self) -> Vec<RetainedLoan> {
        let mut loans = self.direct.clone().unwrap_or_default();
        for member in &self.members {
            retained::append_loans(&mut loans, &member.loans);
        }
        for (_, selected) in &self.selections {
            retained::append_loans(&mut loans, selected);
        }
        loans
    }
}

impl Checker<'_> {
    pub(super) fn validate_collection_destination(
        &mut self,
        collection: &Expr,
        value: &RetainedValueState,
        span: Span,
        scopes: &mut Scopes,
    ) -> bool {
        let places = self.borrow_root_keys(collection, scopes);
        if !places.is_empty() {
            let mut valid = true;
            for place in places {
                valid &= if let Some(binding) = storage_binding(scopes, &place) {
                    self.validate_collection_element(value, span, binding.scope_depth)
                } else {
                    self.validate_owned_function_storage(value, span, "static collection storage")
                };
            }
            return valid;
        }
        // An owned receiver expression is a statement temporary, not an
        // independently escaping destination such as a static or shared owner.
        self.validate_collection_element(value, span, scopes.lexical_depth())
    }

    fn store_retained_sources(
        &mut self,
        place: BorrowPlace,
        identity: Identity,
        loans: Vec<RetainedLoan>,
        site: Span,
        scopes: &mut Scopes,
    ) {
        let Some(binding) = storage_binding(scopes, &place) else {
            return;
        };
        if !self.validate_local_function_destination(
            &retained::iterator_value(loans.clone()),
            binding.scope_depth,
            site,
        ) {
            return;
        }
        let value = binding
            .retained_value
            .get_or_insert_with(|| retained::iterator_value(Vec::new()));
        value
            .storage
            .insert(identity, place.fields, loans, &value.leases);
        value.refresh_stored_loans();
    }

    pub(super) fn store_collection_sources(
        &mut self,
        collection: &Expr,
        incoming: &RetainedValueState,
        site: Span,
        scopes: &mut Scopes,
    ) {
        if !self.validate_collection_destination(collection, incoming, site, scopes)
            || incoming.leases.is_empty()
        {
            return;
        }
        let places = self.borrow_root_keys(collection, scopes);
        if !places.is_empty() {
            scopes.apply_place_alternatives(places, |place, branch| {
                self.store_retained_sources(
                    place,
                    Identity {
                        origin: site,
                        callers: Vec::new(),
                    },
                    incoming.leases.clone(),
                    site,
                    branch,
                )
            });
        } else {
            let mut value = self
                .retained_value_from_expr(collection, scopes)
                .unwrap_or_else(|| retained::iterator_value(Vec::new()));
            value
                .storage
                .merge_from(&incoming.storage, &value.leases, &incoming.leases);
            value.refresh_stored_loans();
            self.retained
                .expression_values
                .insert(collection.span(), value);
        }
    }

    pub(super) fn clear_collection_sources(&mut self, collection: &Expr, scopes: &mut Scopes) {
        let places = self.borrow_root_keys(collection, scopes);
        scopes.apply_place_alternatives(places, |place, branch| {
            if !place.projectable {
                return;
            }
            let selection = self.storage_selection(collection, &place, vec![Selector::All], branch);
            if let Some(value) =
                storage_binding(branch, &place).and_then(|b| b.retained_value.as_mut())
            {
                value
                    .storage
                    .remove(&[Selector::All], &place.fields, &value.leases);
                value.refresh_stored_loans();
            }
            if let Some(selection) = selection {
                if !branch.3.contains(&selection) {
                    branch.3.push(selection);
                }
            }
        });
    }

    pub(super) fn unknown_storage_selector(
        &self,
        expr: &Expr,
        scopes: &Scopes,
    ) -> Option<RetainedValueState> {
        if matches!(ungroup_expr(expr), Expr::Null { .. })
            || self.flow_fact(expr) == Some(&Fact::Null)
            || matches!(ungroup_expr(expr), Expr::Variable { name, .. }
            if scopes.get(name).is_some_and(|binding| {
                binding.borrow_root.as_ref() == Some(&BorrowSourceRoot::ImmutableNull)
            }))
        {
            return None;
        }
        if !is_selector_type(self.resolved_type(expr)?) {
            return None;
        }
        let mut value = retained::iterator_value(Vec::new());
        value.storage.selectors.push(Selector::Unknown);
        Some(value)
    }

    pub(super) fn parameter_storage_selector(
        &self,
        id: Option<BindingId>,
    ) -> Option<RetainedValueState> {
        let id = id?;
        let declaration = self.binding_resolution.declarations_by_id.get(&id)?;
        if !is_selector_type(declaration.source_type.as_ref()?) {
            return None;
        }
        let index = self
            .retained
            .current
            .as_ref()?
            .parameters
            .iter()
            .position(|(parameter, _)| *parameter == Some(id))?;
        let mut value = retained::iterator_value(Vec::new());
        value
            .storage
            .selectors
            .push(Selector::Input(BorrowSource::Parameter(index)));
        Some(value)
    }

    pub(super) fn summarize_escaping_stored_sources(&mut self, scopes: &Scopes) {
        let Some(declaration) = self
            .retained
            .current
            .as_ref()
            .map(|current| current.declaration)
        else {
            return;
        };
        let stores = self.stored_sources(scopes);
        let info = self.retained.callables.entry(declaration).or_default();
        append_stores(&mut info.exceptional_stored_sources, stores);
    }

    pub(super) fn record_returned_storage_selectors(&mut self, value: &RetainedValueState) {
        let storage = ReturnedStorage {
            inputs: value.storage.inputs.clone(),
            direct: value
                .storage
                .direct
                .as_deref()
                .unwrap_or(&value.leases)
                .iter()
                .filter_map(|loan| self.parameter_source(loan))
                .filter(|source| {
                    !(source.inherited
                        && source.owned_path.is_empty()
                        && value.storage.inputs.contains(&source.source))
                })
                .collect(),
            members: value
                .storage
                .members
                .iter()
                .map(|member| ReturnedMember {
                    identity: member.identity.clone(),
                    path: member.path.clone(),
                    sources: member
                        .loans
                        .iter()
                        .filter_map(|loan| self.parameter_source(loan))
                        .collect(),
                    repeated: member.repeated,
                })
                .collect(),
            selections: value
                .storage
                .selections
                .iter()
                .map(|(s, _)| s.clone())
                .collect(),
        };
        let Some(current) = &mut self.retained.current else {
            return;
        };
        current
            .returned_storage
            .get_or_insert_with(ReturnedStorage::default)
            .merge(storage);
        for identity in &value.storage.selectors {
            if !current.returned_selectors.contains(identity) {
                current.returned_selectors.push(identity.clone());
            }
        }
    }

    pub(super) fn returned_storage_from_call(
        &self,
        storage: &ReturnedStorage,
        site: Span,
        receiver: Option<&Expr>,
        args: &[Argument],
        signature: &Signature,
        scopes: &Scopes,
    ) -> Option<RetainedValueState> {
        let direct =
            self.retained_call_loans(&storage.direct, site, receiver, args, signature, scopes)?;
        let mut value = retained::iterator_value(direct.clone());
        value.storage.direct = Some(direct);
        for source in &storage.inputs {
            let input = self.storage_input(*source, receiver, args, signature)?;
            if let Some(incoming) = self.retained_value_from_expr(input, scopes) {
                value
                    .storage
                    .merge_from(&incoming.storage, &value.leases, &incoming.leases);
            }
        }
        for selection in &storage.selections {
            let input = self.storage_input(selection.destination, receiver, args, signature)?;
            for mut place in self.borrow_root_keys(input, scopes) {
                self.extend_retained_place(&mut place, &selection.path, signature);
                let selectors = self.call_storage_selectors(
                    &selection.selectors,
                    site,
                    receiver,
                    args,
                    signature,
                    scopes,
                );
                let Some(mut incoming) = self.retained_value_from_expr(input, scopes) else {
                    continue;
                };
                let inherited = incoming
                    .storage
                    .direct
                    .as_deref()
                    .unwrap_or(&incoming.leases)
                    .to_vec();
                let loans = incoming
                    .storage
                    .remove(&selectors, &place.fields, &incoming.leases);
                if let Some(selection) = self.storage_selection(input, &place, selectors, scopes) {
                    let known = loans
                        .iter()
                        .filter(|loan| !inherited.iter().any(|input| input.same_source(loan)))
                        .cloned()
                        .collect::<Vec<_>>();
                    retained::append_loans(
                        value.storage.direct.as_mut().expect("normalized storage"),
                        &known,
                    );
                    value.storage.select(selection, inherited);
                } else {
                    retained::append_loans(
                        value.storage.direct.as_mut().expect("normalized storage"),
                        &loans,
                    );
                }
            }
        }
        for member in &storage.members {
            let loans =
                self.retained_call_loans(&member.sources, site, receiver, args, signature, scopes)?;
            let incoming = Contents {
                direct: Some(Vec::new()),
                members: vec![Member {
                    identity: member.identity.at_call(site),
                    path: member.path.clone(),
                    loans,
                    repeated: member.repeated,
                }],
                ..Contents::default()
            };
            value.storage.merge_from(&incoming, &[], &[]);
        }
        value.refresh_stored_loans();
        Some(value)
    }

    pub(super) fn summarize_stored_call_sources(&mut self, scopes: &Scopes, flow: &Flow) {
        let Some(declaration) = self
            .retained
            .current
            .as_ref()
            .map(|current| current.declaration)
        else {
            return;
        };
        let mut normal = Vec::new();
        let normal_exits = flow
            .returns
            .iter()
            .chain(flow.falls_through.then_some(scopes));
        let removals = definite_removals(normal_exits);
        if flow.falls_through {
            append_stores(&mut normal, self.stored_sources(scopes));
        }
        for scopes in &flow.returns {
            append_stores(&mut normal, self.stored_sources(scopes));
        }
        let failures = self
            .exception_scopes
            .first()
            .map(|scope| {
                scope
                    .exits
                    .iter()
                    .map(|exit| exit.scopes.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut exceptional = Vec::new();
        for scopes in &failures {
            append_stores(&mut exceptional, self.stored_sources(scopes));
        }
        let info = self.retained.callables.entry(declaration).or_default();
        info.removals = removals;
        info.exceptional_removals = definite_removals(failures.iter());
        append_stores(&mut info.stored_sources, normal);
        append_stores(&mut info.exceptional_stored_sources, exceptional);
    }

    fn stored_sources(&mut self, scopes: &Scopes) -> Vec<Store> {
        let mut stores = Vec::new();
        for binding in scopes.0.iter().flat_map(|scope| scope.values()).flatten() {
            if !matches!(binding.state, State::Borrowed | State::BorrowedOrOwned) {
                continue;
            }
            let Some(destination) = binding.canonical_id.and_then(|id| {
                self.retained
                    .current
                    .as_ref()?
                    .parameters
                    .iter()
                    .enumerate()
                    .find(|(_, (parameter, take))| *parameter == Some(id) && !*take)
                    .map(|(index, _)| BorrowSource::Parameter(index))
            }) else {
                continue;
            };
            for member in binding
                .retained_value
                .iter()
                .flat_map(|value| &value.storage.members)
            {
                let mut sources = Vec::new();
                for loan in &member.loans {
                    if let Some(source) = self.parameter_source(loan) {
                        retained::append_sources(&mut sources, [source]);
                    } else {
                        self.diagnostics.push(retained::iterator_escape(
                            member.identity.origin,
                            loan.capture_span,
                            "stored owner would outlive this call's borrowed source",
                        ));
                    }
                }
                stores.push(Store {
                    destination,
                    path: member.path.clone(),
                    identity: member.identity.clone(),
                    sources,
                });
            }
        }
        stores
    }

    fn apply_stored_sources(
        &mut self,
        site: Span,
        receiver: Option<&Expr>,
        args: &[Argument],
        signature: &Signature,
        stores: &[Store],
        scopes: &mut Scopes,
    ) {
        for store in stores {
            let Some(destination) =
                self.storage_input(store.destination, receiver, args, signature)
            else {
                continue;
            };
            let places = self.borrow_root_keys(destination, scopes);
            let mut loans = Vec::new();
            for source in &store.sources {
                let Some(input) = self.storage_input(source.source, receiver, args, signature)
                else {
                    continue;
                };
                let incoming = if source.inherited {
                    self.retained_value_from_expr(input, scopes)
                        .map(|value| value.leases)
                        .unwrap_or_default()
                } else {
                    self.retained_source_loans(
                        input,
                        scopes,
                        if source.writable {
                            BorrowAccess::Writable
                        } else {
                            BorrowAccess::Readonly
                        },
                    )
                };
                for mut loan in incoming {
                    self.extend_retained_place(&mut loan.root_key, &source.owned_path, signature);
                    loan.lifetime = source.lifetime;
                    retained::append_loans(&mut loans, &[loan]);
                }
            }
            scopes.apply_place_alternatives(places, |mut place, branch| {
                self.extend_retained_place(&mut place, &store.path, signature);
                self.store_retained_sources(
                    place,
                    store.identity.at_call(site),
                    loans.clone(),
                    site,
                    branch,
                );
            });
        }
    }

    fn storage_input<'a>(
        &self,
        source: BorrowSource,
        receiver: Option<&'a Expr>,
        args: &'a [Argument],
        signature: &Signature,
    ) -> Option<&'a Expr> {
        self.call_borrow_source_expr(
            ReturnBorrow {
                kind: ReturnBorrowKind::Retained,
                source,
                writable: false,
                lifetime: RetainedLifetime::Cleanup,
            },
            receiver,
            signature,
            args,
        )
    }

    fn storage_selection(
        &self,
        input: &Expr,
        place: &BorrowPlace,
        selectors: Vec<Selector>,
        scopes: &Scopes,
    ) -> Option<Selection> {
        let mut loan = self
            .retained_loans_at_source(
                Some(BorrowSourceRoot::Place(place.clone())),
                input.span(),
                scopes,
                BorrowAccess::Readonly,
            )
            .into_iter()
            .next()?;
        loan.inherited = true;
        let source = self.parameter_source(&loan)?;
        Some(Selection {
            destination: source.source,
            path: place.fields.clone(),
            selectors,
        })
    }

    fn call_storage_selectors(
        &self,
        selectors: &[Selector],
        site: Span,
        receiver: Option<&Expr>,
        args: &[Argument],
        signature: &Signature,
        scopes: &Scopes,
    ) -> Vec<Selector> {
        let mut result = Vec::new();
        for selector in selectors {
            let incoming = match selector {
                Selector::All => vec![Selector::All],
                Selector::Member(identity) => vec![Selector::Member(identity.at_call(site))],
                Selector::Input(source) => self
                    .storage_input(*source, receiver, args, signature)
                    .and_then(|input| self.retained_value_from_expr(input, scopes))
                    .map(|value| value.storage.selectors)
                    .unwrap_or_else(|| vec![Selector::Unknown]),
                Selector::Capture(_) | Selector::Unknown => vec![Selector::Unknown],
            };
            for selector in incoming {
                if !result.contains(&selector) {
                    result.push(selector);
                }
            }
        }
        result
    }

    fn apply_removals(
        &self,
        selections: &[Selection],
        site: Span,
        receiver: Option<&Expr>,
        args: &[Argument],
        signature: &Signature,
        scopes: &mut Scopes,
    ) {
        for selection in selections {
            let Some(input) = self.storage_input(selection.destination, receiver, args, signature)
            else {
                continue;
            };
            let places = self.borrow_root_keys(input, scopes);
            scopes.apply_place_alternatives(places, |mut place, branch| {
                self.extend_retained_place(&mut place, &selection.path, signature);
                if selection.selectors == [Selector::All] && !place.projectable {
                    return;
                }
                let selectors = self.call_storage_selectors(
                    &selection.selectors,
                    site,
                    receiver,
                    args,
                    signature,
                    branch,
                );
                let symbolic = self.storage_selection(input, &place, selectors.clone(), branch);
                if let Some(value) = storage_binding(branch, &place)
                    .and_then(|binding| binding.retained_value.as_mut())
                {
                    value
                        .storage
                        .remove(&selectors, &place.fields, &value.leases);
                    value.refresh_stored_loans();
                }
                if let Some(selection) = symbolic {
                    if !branch.3.contains(&selection) {
                        branch.3.push(selection);
                    }
                }
            });
        }
    }

    pub(super) fn transfer_stored_call_sources(
        &mut self,
        site: Span,
        receiver: Option<&Expr>,
        args: &[Argument],
        signature: &Signature,
        scopes: &mut Scopes,
    ) {
        use crate::compiler_known_stream::ReadinessFunction;
        let operation = signature
            .declaration
            .and_then(crate::compiler_known_stream::readiness_function);
        if !matches!(
            operation,
            Some(ReadinessFunction::Register | ReadinessFunction::Unregister)
        ) {
            let Some(mut info) = signature
                .declaration
                .and_then(|declaration| self.retained.callables.get(&declaration))
                .map(RetainedCallableInfo::for_callers)
                .cloned()
            else {
                return;
            };
            for implementation in signature
                .declaration
                .and_then(|declaration| self.retained.implementations.get(&declaration))
                .into_iter()
                .flatten()
            {
                if let Some(candidate) = self.retained.callables.get(implementation) {
                    for selector in &candidate.returned_selectors {
                        if !info.returned_selectors.contains(selector) {
                            info.returned_selectors.push(selector.clone());
                        }
                    }
                }
            }
            let returns_selector = self.resolved_types.get(&site).is_some_and(is_selector_type);
            if returns_selector
                && info.returned_selectors.is_empty()
                && signature.declaration.is_none_or(|declaration| {
                    !self.retained.analyzed_callables.contains(&declaration)
                })
            {
                info.returned_selectors.push(Selector::Unknown);
            }
            if info.stored_sources.is_empty()
                && info.exceptional_stored_sources.is_empty()
                && info.removals.is_empty()
                && info.exceptional_removals.is_empty()
                && !returns_selector
            {
                return;
            }
            let mut failed = scopes.clone();
            // Select returned members before applying the caller-visible removal.
            if let Some(storage) = info
                .returned_storage
                .as_ref()
                .filter(|s| !s.selections.is_empty())
            {
                if let Some(value) = self
                    .returned_storage_from_call(storage, site, receiver, args, signature, scopes)
                {
                    self.retained.expression_values.insert(site, value);
                }
            }
            self.apply_stored_sources(
                site,
                receiver,
                args,
                signature,
                &info.exceptional_stored_sources,
                &mut failed,
            );
            self.apply_stored_sources(
                site,
                receiver,
                args,
                signature,
                &info.stored_sources,
                scopes,
            );
            self.apply_removals(
                &info.exceptional_removals,
                site,
                receiver,
                args,
                signature,
                &mut failed,
            );
            self.apply_removals(&info.removals, site, receiver, args, signature, scopes);
            self.retained.failed_storage_calls.insert(site, failed);
            if returns_selector {
                // Generic results can be keys or source-bearing owners. Adding
                // selector facts must preserve their returned storage and loans.
                let mut selector = self
                    .retained
                    .expression_values
                    .get(&site)
                    .cloned()
                    .or_else(|| {
                        let storage =
                            info.returned_storage
                                .clone()
                                .unwrap_or_else(|| ReturnedStorage {
                                    direct: info.returns.clone(),
                                    ..Default::default()
                                });
                        self.returned_storage_from_call(
                            &storage, site, receiver, args, signature, scopes,
                        )
                    })
                    .unwrap_or_else(|| retained::iterator_value(Vec::new()));
                for returned in &info.returned_selectors {
                    let incoming = match returned {
                        Selector::All => vec![Selector::All],
                        Selector::Member(identity) => {
                            vec![Selector::Member(identity.at_call(site))]
                        }
                        Selector::Input(source) => self
                            .storage_input(*source, receiver, args, signature)
                            .and_then(|input| self.retained_value_from_expr(input, scopes))
                            .map(|value| value.storage.selectors)
                            .unwrap_or_default(),
                        Selector::Capture(_) | Selector::Unknown => vec![Selector::Unknown],
                    };
                    for incoming in incoming {
                        if !selector.storage.selectors.contains(&incoming) {
                            selector.storage.selectors.push(incoming);
                        }
                    }
                }
                self.retained.expression_values.insert(site, selector);
            }
            return;
        }
        let Some(receiver) = receiver else { return };
        let places = self.borrow_root_keys(receiver, scopes);
        if places.is_empty() {
            return;
        }
        let Some(input) = self.call_borrow_source_expr(
            ReturnBorrow {
                kind: ReturnBorrowKind::Retained,
                source: BorrowSource::Parameter(0),
                writable: false,
                lifetime: RetainedLifetime::Cleanup,
            },
            Some(receiver),
            signature,
            args,
        ) else {
            return;
        };
        let input_value = self.retained_value_from_expr(input, scopes);
        let selectors = input_value
            .as_ref()
            .map(|value| value.storage.selectors.clone())
            .unwrap_or_default();
        let failed = scopes.clone();
        let mut returned = None;
        scopes.apply_place_alternatives(places, |place, branch| {
            let selection = self.storage_selection(receiver, &place, selectors.clone(), branch);
            if operation == Some(ReadinessFunction::Register) {
                self.store_retained_sources(
                    place,
                    Identity {
                        origin: site,
                        callers: Vec::new(),
                    },
                    input_value
                        .as_ref()
                        .map(|value| value.leases.clone())
                        .unwrap_or_default(),
                    site,
                    branch,
                );
                let mut selector = retained::iterator_value(Vec::new());
                selector.storage.selectors.push(Selector::Member(Identity {
                    origin: site,
                    callers: Vec::new(),
                }));
                returned = join_retained_value(returned.as_ref(), Some(&selector));
                return;
            }
            let Some(binding) = storage_binding(branch, &place) else {
                return;
            };
            let value = binding
                .retained_value
                .get_or_insert_with(|| retained::iterator_value(Vec::new()));
            let result = match operation.expect("checked storage operation") {
                ReadinessFunction::Register => unreachable!(),
                ReadinessFunction::Unregister => {
                    let inherited = value
                        .storage
                        .direct
                        .as_deref()
                        .unwrap_or(&value.leases)
                        .to_vec();
                    let loans = value
                        .storage
                        .remove(&selectors, &place.fields, &value.leases);
                    value.refresh_stored_loans();
                    let retains_sources = self.resolved_types.get(&site).is_none_or(|ty| {
                        resolved_type_requires_conservative_move(ty)
                            || resolved_type_is_move_type(ty, &self.move_enum_names)
                    });
                    let mut result = retained::iterator_value(if retains_sources {
                        loans.clone()
                    } else {
                        Vec::new()
                    });
                    if let Some(selection) = selection {
                        if retains_sources {
                            result.storage.direct = Some(
                                loans
                                    .into_iter()
                                    .filter(|loan| {
                                        !inherited.iter().any(|input| input.same_source(loan))
                                    })
                                    .collect(),
                            );
                            result.storage.select(selection.clone(), inherited);
                        }
                        if !branch.3.contains(&selection) {
                            branch.3.push(selection);
                        }
                    }
                    result
                }
                ReadinessFunction::New
                | ReadinessFunction::Begin
                | ReadinessFunction::GetStream
                | ReadinessFunction::GetWritableStream => unreachable!(),
            };
            returned = join_retained_value(returned.as_ref(), Some(&result));
        });
        if let Some(result) = returned {
            self.retained.expression_values.insert(site, result);
        }
        self.retained.failed_storage_calls.insert(site, failed);
    }
}

fn definite_removals<'a>(mut exits: impl Iterator<Item = &'a Scopes>) -> Vec<Selection> {
    let Some(first) = exits.next() else {
        return Vec::new();
    };
    let mut selections = first.3.clone();
    for exit in exits {
        selections.retain(|selection| exit.3.contains(selection));
    }
    selections
}

pub(super) fn is_selector_type(ty: &ResolvedType) -> bool {
    match ty {
        ResolvedType::Class(class) => class.name == crate::compiler_known_stream::REGISTRATION_KEY,
        ResolvedType::TypeParameter(_) => true,
        ResolvedType::Nullable(ty) => is_selector_type(ty),
        _ => false,
    }
}

fn storage_binding<'a>(scopes: &'a mut Scopes, place: &BorrowPlace) -> Option<&'a mut Binding> {
    scopes.0.iter_mut().rev().find_map(|scope| {
        scope.iter_mut().find_map(|(name, binding)| {
            binding.as_mut().filter(|binding| {
                binding_identity_key(binding.id, name) == place.owner
                    || (name == "this" && place.owner == "$this")
            })
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loan(id: usize) -> RetainedLoan {
        RetainedLoan {
            root: BorrowRoot::Binding(BindingId(id)),
            root_key: format!("source:{id}").into(),
            access: BorrowAccess::Readonly,
            capture_span: Span::new(id, id + 1),
            source_depth: 1,
            inherited: false,
            lifetime: RetainedLifetime::Cleanup,
            reborrows: Vec::new(),
            pending_capture: None,
        }
    }

    #[test]
    fn enum_branch_join_preserves_case_and_field_partitions() {
        use crate::enums::{EnumCaseSource, EnumId};

        let case = |index| EnumCaseSource {
            declaration: EnumId(0),
            index,
        };
        let left_key = (case(0), 0);
        let right_key = (case(1), 0);
        let sibling_key = (case(0), 1);
        let mut left = retained::iterator_value(vec![loan(1), loan(2)]);
        left.enum_payloads = Some(HashMap::from([
            (left_key, retained::iterator_value(vec![loan(1)])),
            (sibling_key, retained::iterator_value(vec![loan(2)])),
        ]));
        let mut right = retained::iterator_value(vec![loan(3), loan(4)]);
        right.enum_payloads = Some(HashMap::from([
            (left_key, retained::iterator_value(vec![loan(3)])),
            (right_key, retained::iterator_value(vec![loan(4)])),
        ]));

        let joined = join_retained_value(Some(&left), Some(&right)).unwrap();
        let fields = joined.enum_payloads.as_ref().unwrap();
        let roots = |key| provenance_roots(&fields[&key].provenance);
        assert_eq!(
            roots(left_key),
            vec![
                BorrowRoot::Binding(BindingId(1)),
                BorrowRoot::Binding(BindingId(3))
            ]
        );
        assert_eq!(roots(sibling_key), vec![BorrowRoot::Binding(BindingId(2))]);
        assert_eq!(roots(right_key), vec![BorrowRoot::Binding(BindingId(4))]);

        let opaque = retained::iterator_value(vec![loan(5)]);
        let joined = join_retained_value(Some(&joined), Some(&opaque)).unwrap();
        assert!(joined.enum_payloads.is_none());
        assert_eq!(joined.leases.len(), 5);
        let projected = joined
            .project_enum_payload(
                left_key.0,
                left_key.1,
                Some(RetainedValueKind::SourceCarrier),
            )
            .unwrap();
        assert_eq!(projected.leases.len(), 5);
        assert!(joined
            .project_enum_payload(left_key.0, left_key.1, None)
            .is_none());
    }

    #[test]
    fn enum_payload_projection_does_not_restore_last_use_loans() {
        use crate::enums::{EnumCaseSource, EnumId};

        let case = EnumCaseSource {
            declaration: EnumId(0),
            index: 0,
        };
        let mut last_use = loan(1);
        last_use.lifetime = RetainedLifetime::LastUse;
        let loans = vec![last_use, loan(2)];
        let mut value = retained::iterator_value(loans.clone());
        let mut nested = retained::iterator_value(loans.clone());
        nested.enum_payloads = Some(HashMap::from([(
            (case, 0),
            retained::iterator_value(loans),
        )]));
        value.enum_payloads = Some(HashMap::from([((case, 0), nested)]));
        value.retain_cleanup_loans();

        let outer = value.project_enum_payload(case, 0, None).unwrap();
        let inner = outer.project_enum_payload(case, 0, None).unwrap();
        for value in [value, outer, inner] {
            assert_eq!(value.leases.len(), 1);
            assert_eq!(value.leases[0].root, BorrowRoot::Binding(BindingId(2)));
        }
    }

    #[test]
    fn selecting_one_member_preserves_other_and_direct_dependencies() {
        let mut contents = Contents::default();
        let first = Identity {
            origin: Span::new(10, 11),
            callers: Vec::new(),
        };
        let second = Identity {
            origin: Span::new(20, 21),
            callers: Vec::new(),
        };
        contents.insert(first.clone(), vec![], vec![loan(1)], &[loan(0)]);
        contents.insert(second, vec![], vec![loan(2)], &[]);
        let removed = contents.remove(&[Selector::Member(first)], &[], &[]);
        assert_eq!(removed.len(), 2);
        assert!(removed
            .iter()
            .any(|loan| loan.root == BorrowRoot::Binding(BindingId(1))));
        assert!(removed
            .iter()
            .any(|loan| loan.root == BorrowRoot::Binding(BindingId(0))));
        let remaining = contents.loans();
        assert_eq!(remaining.len(), 2);
        assert!(remaining
            .iter()
            .all(|loan| loan.root != BorrowRoot::Binding(BindingId(1))));
    }

    #[test]
    fn stored_owner_provenance_and_branch_joins_follow_the_current_loans() {
        let empty = retained::iterator_value(Vec::new());
        let mut owned = empty.clone();
        let identity = Identity {
            origin: Span::new(10, 11),
            callers: Vec::new(),
        };
        owned
            .storage
            .insert(identity.clone(), vec![], vec![loan(1)], &[]);
        owned.refresh_stored_loans();
        let joined = join_retained_value(Some(&empty), Some(&owned)).unwrap();
        assert_eq!(
            joined.provenance,
            ValueProvenance::BorrowBound(vec![BorrowRoot::Binding(BindingId(1))])
        );
        owned
            .storage
            .remove(&[Selector::Member(identity)], &[], &owned.leases);
        owned.refresh_stored_loans();
        assert!(owned.leases.is_empty());
        assert_eq!(owned.provenance, ValueProvenance::Owned);
    }

    #[test]
    fn joined_opaque_sources_survive_removal_of_a_known_member() {
        let identity = Identity {
            origin: Span::new(10, 11),
            callers: Vec::new(),
        };
        let mut known = retained::iterator_value(Vec::new());
        known
            .storage
            .insert(identity.clone(), vec![], vec![loan(1)], &[]);
        known.refresh_stored_loans();
        let opaque = retained::iterator_value(vec![loan(2)]);
        for reversed in [false, true] {
            let (left, right) = if reversed {
                (&opaque, &known)
            } else {
                (&known, &opaque)
            };
            let mut joined = join_retained_value(Some(left), Some(right)).unwrap();
            assert_eq!(joined.leases.len(), 2);
            let removed =
                joined
                    .storage
                    .remove(&[Selector::Member(identity.clone())], &[], &joined.leases);
            assert_eq!(removed.len(), 2);
            joined.refresh_stored_loans();
            assert_eq!(joined.leases.len(), 1);
            assert_eq!(joined.leases[0].root, BorrowRoot::Binding(BindingId(2)));
            assert_eq!(
                joined.provenance,
                ValueProvenance::BorrowBound(vec![BorrowRoot::Binding(BindingId(2))])
            );
        }
    }

    #[test]
    fn repeated_and_unknown_selection_do_not_release_other_live_instances() {
        let mut contents = Contents::default();
        let site = Identity {
            origin: Span::new(10, 11),
            callers: Vec::new(),
        };
        contents.insert(site.clone(), vec![], vec![loan(1)], &[]);
        contents.insert(site.clone(), vec![], vec![loan(2)], &[]);
        assert_eq!(
            contents.remove(&[Selector::Member(site)], &[], &[]).len(),
            2
        );
        assert_eq!(contents.loans().len(), 2);
        assert_eq!(contents.remove(&[], &[], &[]).len(), 2);
        assert_eq!(contents.loans().len(), 2);
    }

    #[test]
    fn selection_preserves_incoming_unpartitioned_member_obligations() {
        let mut contents = Contents::default();
        let incoming = vec![loan(1)];
        assert_eq!(contents.remove(&[], &[], &incoming).len(), 1);
        assert_eq!(contents.loans().len(), 1);
        assert_eq!(contents.remove(&[], &[], &[]).len(), 1);
    }

    #[test]
    fn joining_known_and_unknown_keys_preserves_every_possible_member() {
        let mut contents = Contents::default();
        let first = Identity {
            origin: Span::new(10, 11),
            callers: Vec::new(),
        };
        let second = Identity {
            origin: Span::new(20, 21),
            callers: Vec::new(),
        };
        contents.insert(first.clone(), vec![], vec![loan(1)], &[]);
        contents.insert(second, vec![], vec![loan(2)], &[]);
        let removed = contents.remove(&[Selector::Member(first), Selector::Unknown], &[], &[]);
        assert_eq!(removed.len(), 2);
        assert_eq!(contents.loans().len(), 2);
    }
}

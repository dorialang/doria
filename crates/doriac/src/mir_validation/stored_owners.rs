//! Stored owners retain loans; Copy selectors identify owners but lend nothing.

use super::*;
use crate::Span;
use retained::CallInputs;

#[derive(Clone, Default, PartialEq, Eq)]
pub(super) struct Storage {
    contents: Vec<Contents>,
    selectors: Vec<HashSet<mir::RetainedStorageSelector>>,
    removals: HashSet<mir::RetainedStorageSelection>,
    forwarding_site: Option<Span>,
}

#[derive(Clone, Default, PartialEq, Eq)]
struct Contents {
    direct: Option<HashSet<Source>>,
    members: Vec<Member>,
    inputs: HashSet<mir::LocalId>,
    selections: Vec<(mir::RetainedStorageSelection, HashSet<Source>)>,
}

#[derive(Clone, PartialEq, Eq)]
struct Member {
    identity: mir::RetainedStorageIdentity,
    path: Vec<crate::class_layout::PropertyId>,
    sources: HashSet<Source>,
    repeated: bool,
}

#[derive(Clone)]
pub(super) struct Assignment {
    contents: Contents,
    selectors: HashSet<mir::RetainedStorageSelector>,
}

impl Assignment {
    pub(super) fn sources(&self) -> HashSet<Source> {
        self.contents.sources()
    }

    pub(super) fn normalized_sources(&self) -> Option<HashSet<Source>> {
        self.contents.direct.as_ref().map(|_| self.sources())
    }
}

impl Contents {
    fn join(
        &mut self,
        incoming: &Self,
        sources: &HashSet<Source>,
        incoming_sources: &HashSet<Source>,
    ) {
        self.direct.get_or_insert_with(|| sources.clone()).extend(
            incoming
                .direct
                .as_ref()
                .unwrap_or(incoming_sources)
                .iter()
                .cloned(),
        );
        self.inputs.extend(&incoming.inputs);
        for (selection, sources) in &incoming.selections {
            self.select(selection.clone(), sources.clone());
        }
        for incoming in &incoming.members {
            if let Some(member) = self
                .members
                .iter_mut()
                .find(|member| member.identity == incoming.identity && member.path == incoming.path)
            {
                member.sources.extend(incoming.sources.iter().cloned());
                member.repeated |= incoming.repeated;
            } else {
                self.members.push(incoming.clone());
            }
        }
    }

    fn sources(&self) -> HashSet<Source> {
        self.direct
            .iter()
            .flatten()
            .cloned()
            .chain(
                self.members
                    .iter()
                    .flat_map(|member| member.sources.iter().cloned()),
            )
            .chain(
                self.selections
                    .iter()
                    .flat_map(|(_, sources)| sources.iter().cloned()),
            )
            .collect()
    }

    fn select(&mut self, selection: mir::RetainedStorageSelection, sources: HashSet<Source>) {
        if let Some((_, existing)) = self.selections.iter_mut().find(|(s, _)| *s == selection) {
            existing.extend(sources);
        } else {
            self.selections.push((selection, sources));
        }
    }

    fn remove(
        &mut self,
        path: &[crate::class_layout::PropertyId],
        selectors: &HashSet<mir::RetainedStorageSelector>,
        direct: &HashSet<Source>,
    ) -> HashSet<Source> {
        if selectors.len() == 1 && selectors.contains(&mir::RetainedStorageSelector::All) {
            let mut sources = HashSet::new();
            if path.is_empty() {
                sources = self.sources();
                *self = Self {
                    direct: Some(HashSet::new()),
                    ..Self::default()
                };
            } else {
                self.members.retain(|member| {
                    if member.path.starts_with(path) {
                        sources.extend(member.sources.iter().cloned());
                        false
                    } else {
                        true
                    }
                });
            }
            return sources;
        }
        let mut sources = self.direct.get_or_insert_with(|| direct.clone()).clone();
        let unknown =
            selectors.is_empty() || selectors.contains(&mir::RetainedStorageSelector::Unknown);
        self.members.retain(|member| {
            if member.path != path
                || (!unknown
                    && !selectors.contains(&mir::RetainedStorageSelector::Member(
                        member.identity.clone(),
                    )))
            {
                return true;
            }
            sources.extend(member.sources.iter().cloned());
            unknown || selectors.len() != 1 || member.repeated
        });
        sources
    }
}

impl Storage {
    pub(super) fn collection_inputs(
        &self,
        program: &mir::Program,
        values: &[&mir::Rvalue],
        locals: &[mir::LocalId],
        roots: &[HashSet<Place>],
        loans: &[HashSet<Source>],
    ) -> Result<Assignment, BackendError> {
        let mut contents = Contents {
            direct: Some(HashSet::new()),
            ..Default::default()
        };
        for value in values {
            let incoming = self.value(program, value, roots, loans)?.contents;
            let sources = retained::value_sources(program, value, roots, loans)?;
            contents.join(&incoming, &HashSet::new(), &sources);
        }
        for local in locals {
            contents.join(&self.contents[local.0], &HashSet::new(), &loans[local.0]);
        }
        Ok(Assignment {
            contents,
            selectors: HashSet::new(),
        })
    }

    pub(super) fn retain_cleanup_sources(&mut self, local: mir::LocalId) {
        self.retain_sources(local, |source| {
            source.lifetime == mir::RetainedLifetime::Cleanup
        });
    }

    pub(super) fn retain_sources(&mut self, local: mir::LocalId, keep: impl Fn(&Source) -> bool) {
        let contents = &mut self.contents[local.0];
        for sources in contents
            .direct
            .iter_mut()
            .chain(
                contents
                    .members
                    .iter_mut()
                    .map(|member| &mut member.sources),
            )
            .chain(contents.selections.iter_mut().map(|(_, sources)| sources))
        {
            sources.retain(&keep);
        }
    }

    pub(super) fn collection_value(&self, source: mir::LocalId) -> Assignment {
        Assignment {
            contents: self.contents[source.0].clone(),
            selectors: HashSet::new(),
        }
    }

    pub(super) fn new(program: &mir::Program, function: &mir::Function) -> Self {
        let count = function.locals.len();
        let mut storage = Self {
            contents: vec![Contents::default(); count],
            selectors: vec![HashSet::new(); count],
            removals: HashSet::new(),
            forwarding_site: program
                .interface_vtables
                .iter()
                .any(|table| table.methods.contains(&function.id))
                .then_some(function.source_span),
        };
        for local in &function.locals {
            if function.params.contains(&local.id)
                && local.owned
                && retained::carries_source(program, local.ty)
                && !matches!(
                    local.ty,
                    mir::Type::Function(_)
                        | mir::Type::NullableFunction(_)
                        | mir::Type::ByteRegion(_)
                        | mir::Type::NullableByteRegion(_)
                )
            {
                storage.contents[local.id.0].inputs.insert(local.id);
            }
            if is_key(local.ty) {
                storage.selectors[local.id.0].insert(
                    if function.params.contains(&local.id)
                        || function.closure.as_ref().is_some_and(|closure| {
                            closure
                                .capture_locals
                                .iter()
                                .any(|(_, capture)| *capture == local.id)
                        })
                    {
                        mir::RetainedStorageSelector::Input(local.id)
                    } else {
                        mir::RetainedStorageSelector::Unknown
                    },
                );
            }
        }
        storage
    }

    fn call_identity(
        &self,
        identity: &mir::RetainedStorageIdentity,
        span: Span,
    ) -> mir::RetainedStorageIdentity {
        // Generated interface forwarding is not another source-level call.
        // Its stores, selections, and returned owners keep the same identity.
        if self.forwarding_site == Some(span) {
            identity.clone()
        } else {
            identity.at_call(span)
        }
    }

    fn aggregate_contents<'a>(
        &self,
        program: &mir::Program,
        values: impl Iterator<Item = &'a mir::Rvalue>,
        roots: &[HashSet<Place>],
        loans: &[HashSet<Source>],
    ) -> Result<Contents, BackendError> {
        let empty = HashSet::new();
        let mut contents = Contents {
            direct: Some(HashSet::new()),
            ..Default::default()
        };
        for value in values {
            let incoming = self.value(program, value, roots, loans)?.contents;
            let sources = retained::value_sources(program, value, roots, loans)?;
            contents.join(&incoming, &empty, &sources);
        }
        Ok(contents)
    }

    fn coalesced_payload_contents(
        &self,
        program: &mir::Program,
        left: &mir::NullablePayloadEnumExpression,
        right: impl FnOnce() -> mir::Rvalue,
        roots: &[HashSet<Place>],
        loans: &[HashSet<Source>],
    ) -> Result<Contents, BackendError> {
        match nullable_payload_enum_expression_presence(left) {
            Some(true) => Ok(self
                .value(
                    program,
                    &mir::Rvalue::NullablePayloadEnum(left.clone()),
                    roots,
                    loans,
                )?
                .contents),
            Some(false) => Ok(self.value(program, &right(), roots, loans)?.contents),
            None => self.aggregate_contents(
                program,
                [&mir::Rvalue::NullablePayloadEnum(left.clone()), &right()].into_iter(),
                roots,
                loans,
            ),
        }
    }

    pub(super) fn value(
        &self,
        program: &mir::Program,
        value: &mir::Rvalue,
        roots: &[HashSet<Place>],
        loans: &[HashSet<Source>],
    ) -> Result<Assignment, BackendError> {
        match value {
            mir::Rvalue::Class(mir::ClassExpression::Upcast { value, .. }) => {
                return self.value(
                    program,
                    &mir::Rvalue::Class((**value).clone()),
                    roots,
                    loans,
                );
            }
            mir::Rvalue::NullableClass(mir::NullableClassExpression::Upcast { value, .. }) => {
                return self.value(
                    program,
                    &mir::Rvalue::NullableClass((**value).clone()),
                    roots,
                    loans,
                );
            }
            _ => {}
        }
        let source = value.direct_place_local();
        let contents = if let Some(source) = source {
            self.contents[source.0].clone()
        } else if let Some(source) = retained::collection_element_source(value)
            .filter(|_| retained::carries_source(program, value.ty()))
        {
            self.contents[source.0].clone()
        } else {
            match value {
                mir::Rvalue::PayloadEnum(mir::PayloadEnumExpression::Construct {
                    fields, ..
                }) => self.aggregate_contents(program, fields.iter(), roots, loans)?,
                mir::Rvalue::PayloadEnum(mir::PayloadEnumExpression::Coalesce {
                    left,
                    right,
                    ..
                }) => self.coalesced_payload_contents(
                    program,
                    left,
                    || mir::Rvalue::PayloadEnum((**right).clone()),
                    roots,
                    loans,
                )?,
                mir::Rvalue::NullablePayloadEnum(
                    mir::NullablePayloadEnumExpression::Coalesce { left, right, .. },
                ) => self.coalesced_payload_contents(
                    program,
                    left,
                    || mir::Rvalue::NullablePayloadEnum((**right).clone()),
                    roots,
                    loans,
                )?,
                mir::Rvalue::Collection(mir::CollectionExpression::Literal { entries, .. }) => self
                    .aggregate_contents(
                        program,
                        entries.iter().flat_map(|entry| {
                            entry.key.iter().chain(std::iter::once(&entry.value))
                        }),
                        roots,
                        loans,
                    )?,
                mir::Rvalue::Collection(mir::CollectionExpression::Fill { value, .. }) => {
                    let mut contents = self.value(program, value, roots, loans)?.contents;
                    contents
                        .direct
                        .get_or_insert(retained::value_sources(program, value, roots, loans)?);
                    contents
                }
                mir::Rvalue::Class(
                    mir::ClassExpression::InterfaceReceiver {
                        receiver: local, ..
                    }
                    | mir::ClassExpression::InterfacePayload { local, .. }
                    | mir::ClassExpression::MixedPayload { mixed: local, .. },
                )
                | mir::Rvalue::Collection(
                    mir::CollectionExpression::InterfaceReceiver {
                        receiver: local, ..
                    }
                    | mir::CollectionExpression::IteratorSource {
                        receiver: local, ..
                    },
                )
                | mir::Rvalue::Interface(mir::InterfaceExpression {
                    value: mir::InterfaceValue::NarrowedLocal { local, .. },
                    ..
                })
                | mir::Rvalue::Function(mir::FunctionExpression::MixedPayload {
                    mixed: local,
                    ..
                }) => self.contents[local.0].clone(),
                mir::Rvalue::Class(mir::ClassExpression::Call {
                    function,
                    args,
                    span,
                    ..
                })
                | mir::Rvalue::NullableClass(mir::NullableClassExpression::Call {
                    function,
                    args,
                    span,
                    ..
                })
                | mir::Rvalue::Collection(mir::CollectionExpression::Call {
                    function,
                    args,
                    span,
                    ..
                })
                | mir::Rvalue::NullableCollection(mir::NullableCollectionExpression::Call {
                    function,
                    args,
                    span,
                    ..
                })
                | mir::Rvalue::Function(mir::FunctionExpression::Call {
                    function,
                    args,
                    span,
                    ..
                })
                | mir::Rvalue::NullableFunction(mir::NullableFunctionExpression::Call {
                    function,
                    args,
                    span,
                    ..
                })
                | mir::Rvalue::PayloadEnum(mir::PayloadEnumExpression::Call {
                    function,
                    args,
                    span,
                    ..
                })
                | mir::Rvalue::NullablePayloadEnum(mir::NullablePayloadEnumExpression::Call {
                    function,
                    args,
                    span,
                    ..
                })
                | mir::Rvalue::Interface(mir::InterfaceExpression {
                    value:
                        mir::InterfaceValue::Call {
                            function,
                            args,
                            span,
                            ..
                        },
                    ..
                })
                | mir::Rvalue::NullableInterface(mir::NullableInterfaceExpression {
                    value:
                        mir::NullableInterfaceValue::Call {
                            function,
                            args,
                            span,
                            ..
                        },
                    ..
                }) => self.call_contents(
                    program,
                    function_in(program, *function)?,
                    args,
                    *span,
                    roots,
                    loans,
                )?,
                mir::Rvalue::NullableClass(mir::NullableClassExpression::Class(value)) => {
                    self.value(program, &mir::Rvalue::Class(value.clone()), roots, loans)?
                        .contents
                }
                mir::Rvalue::NullableFunction(mir::NullableFunctionExpression::Present(value)) => {
                    self.value(program, &mir::Rvalue::Function(value.clone()), roots, loans)?
                        .contents
                }
                mir::Rvalue::NullablePayloadEnum(mir::NullablePayloadEnumExpression::Value(
                    value,
                )) => {
                    self.value(
                        program,
                        &mir::Rvalue::PayloadEnum(value.clone()),
                        roots,
                        loans,
                    )?
                    .contents
                }
                mir::Rvalue::Function(mir::FunctionExpression::AssumePresent { value, .. }) => {
                    self.value(
                        program,
                        &mir::Rvalue::NullableFunction((**value).clone()),
                        roots,
                        loans,
                    )?
                    .contents
                }
                mir::Rvalue::NullableCollection(mir::NullableCollectionExpression::Collection(
                    value,
                )) => {
                    self.value(
                        program,
                        &mir::Rvalue::Collection(value.clone()),
                        roots,
                        loans,
                    )?
                    .contents
                }
                mir::Rvalue::NullableMixed(mir::NullableMixedExpression::Mixed(value)) => {
                    self.value(program, &mir::Rvalue::Mixed(value.clone()), roots, loans)?
                        .contents
                }
                mir::Rvalue::Interface(mir::InterfaceExpression {
                    value: mir::InterfaceValue::FromClass { object: value, .. },
                    ..
                }) => {
                    self.value(
                        program,
                        &mir::Rvalue::Class((**value).clone()),
                        roots,
                        loans,
                    )?
                    .contents
                }
                mir::Rvalue::Mixed(mir::MixedExpression::BoxClass { value, .. }) => {
                    self.value(program, &mir::Rvalue::Class(value.clone()), roots, loans)?
                        .contents
                }
                mir::Rvalue::Mixed(mir::MixedExpression::BoxPayloadEnum { value }) => {
                    self.value(
                        program,
                        &mir::Rvalue::PayloadEnum((**value).clone()),
                        roots,
                        loans,
                    )?
                    .contents
                }
                mir::Rvalue::NullableMixed(
                    mir::NullableMixedExpression::BoxNullablePayloadEnum(value),
                ) => {
                    self.value(
                        program,
                        &mir::Rvalue::NullablePayloadEnum((**value).clone()),
                        roots,
                        loans,
                    )?
                    .contents
                }
                mir::Rvalue::Mixed(mir::MixedExpression::BoxFunction { value, .. }) => {
                    self.value(
                        program,
                        &mir::Rvalue::Function((**value).clone()),
                        roots,
                        loans,
                    )?
                    .contents
                }
                mir::Rvalue::Interface(mir::InterfaceExpression {
                    value: mir::InterfaceValue::FromNullableClass { object, .. },
                    ..
                }) => {
                    self.value(
                        program,
                        &mir::Rvalue::NullableClass((**object).clone()),
                        roots,
                        loans,
                    )?
                    .contents
                }
                mir::Rvalue::Interface(mir::InterfaceExpression {
                    value: mir::InterfaceValue::FromValue { value, .. },
                    ..
                }) => self.value(program, value, roots, loans)?.contents,
                mir::Rvalue::Interface(mir::InterfaceExpression {
                    value: mir::InterfaceValue::Upcast { source, .. },
                    ..
                })
                | mir::Rvalue::Mixed(mir::MixedExpression::BoxInterface {
                    value: source, ..
                }) => {
                    self.value(
                        program,
                        &mir::Rvalue::Interface((**source).clone()),
                        roots,
                        loans,
                    )?
                    .contents
                }
                mir::Rvalue::NullableInterface(mir::NullableInterfaceExpression {
                    value: mir::NullableInterfaceValue::Upcast { source, .. },
                    ..
                }) => {
                    self.value(
                        program,
                        &mir::Rvalue::NullableInterface((**source).clone()),
                        roots,
                        loans,
                    )?
                    .contents
                }
                mir::Rvalue::NullableInterface(mir::NullableInterfaceExpression {
                    interface,
                    value: mir::NullableInterfaceValue::Present(value),
                }) => {
                    self.value(
                        program,
                        &mir::Rvalue::Interface(mir::InterfaceExpression {
                            interface: *interface,
                            value: value.clone(),
                        }),
                        roots,
                        loans,
                    )?
                    .contents
                }
                mir::Rvalue::Class(mir::ClassExpression::Coalesce { left, right, .. }) => {
                    let left = mir::Rvalue::NullableClass((**left).clone());
                    let right = mir::Rvalue::Class((**right).clone());
                    let mut contents = self.value(program, &left, roots, loans)?.contents;
                    let incoming = self.value(program, &right, roots, loans)?.contents;
                    contents.join(
                        &incoming,
                        &retained::value_sources(program, &left, roots, loans)?,
                        &retained::value_sources(program, &right, roots, loans)?,
                    );
                    contents
                }
                _ => Contents::default(),
            }
        };
        let selectors = self.value_selectors(program, value)?;
        Ok(Assignment {
            contents,
            selectors,
        })
    }

    fn call_contents(
        &self,
        program: &mir::Program,
        callee: &mir::Function,
        args: &[mir::Rvalue],
        span: Span,
        roots: &[HashSet<Place>],
        loans: &[HashSet<Source>],
    ) -> Result<Contents, BackendError> {
        let mut contents = Contents {
            direct: Some(HashSet::new()),
            ..Default::default()
        };
        for candidate in
            retained::possible_callees(program, callee.id, retained::receiver_class(args))?
        {
            let incoming =
                self.instantiate_contents(program, candidate, args.into(), span, roots, loans)?;
            contents.join(&incoming, &HashSet::new(), &HashSet::new());
        }
        Ok(contents)
    }

    fn instantiate_contents(
        &self,
        program: &mir::Program,
        callee: &mir::Function,
        args: CallInputs<'_>,
        span: Span,
        roots: &[HashSet<Place>],
        loans: &[HashSet<Source>],
    ) -> Result<Contents, BackendError> {
        let Some(plan) = retained::contract(callee).and_then(|plan| plan.returned_storage.as_ref())
        else {
            return Ok(Contents {
                direct: Some(retained::direct_call_sources(
                    program, callee, args, false, roots, loans,
                )?),
                ..Default::default()
            });
        };
        let mut contents = Contents {
            direct: Some(map_call_sources(
                program,
                callee,
                args,
                &plan.direct,
                roots,
                loans,
            )?),
            ..Default::default()
        };
        for input in &plan.inputs {
            let value = argument(callee, args, *input)?;
            let mut incoming = self.value(program, value, roots, loans)?.contents;
            let input_loans = retained::value_sources(program, value, roots, loans)?;
            for removal in retained::contract(callee)
                .into_iter()
                .flat_map(|p| &p.removals)
                .filter(|r| r.destination == *input)
            {
                let selectors = self.selection_selectors(program, callee, args, removal, span)?;
                incoming.remove(&removal.path, &selectors, &input_loans);
            }
            contents.join(&incoming, &HashSet::new(), &input_loans);
        }
        for selection in &plan.selections {
            let input = argument(callee, args, selection.destination)?;
            let places = retained::source_roots(program, input, roots)?;
            let selectors = self.selection_selectors(program, callee, args, selection, span)?;
            for mut place in places {
                for property in &selection.path {
                    place = place.project(*property);
                }
                let mut incoming = self.contents[place.local.0].clone();
                let inherited = incoming
                    .direct
                    .as_ref()
                    .unwrap_or(&loans[place.local.0])
                    .clone();
                let selected = incoming.remove(&place.fields, &selectors, &loans[place.local.0]);
                contents
                    .direct
                    .as_mut()
                    .expect("normalized storage")
                    .extend(selected.difference(&inherited).cloned());
                contents.select(selection_at_place(&place, &selectors), inherited);
            }
        }
        for member in &plan.members {
            let incoming = Contents {
                direct: Some(HashSet::new()),
                members: vec![Member {
                    identity: self.call_identity(&member.identity, span),
                    path: member.path.clone(),
                    sources: map_call_sources(
                        program,
                        callee,
                        args,
                        &member.sources,
                        roots,
                        loans,
                    )?,
                    repeated: member.repeated,
                }],
                ..Default::default()
            };
            contents.join(&incoming, &HashSet::new(), &HashSet::new());
        }
        Ok(contents)
    }

    fn selection_selectors(
        &self,
        program: &mir::Program,
        callee: &mir::Function,
        inputs: CallInputs<'_>,
        selection: &mir::RetainedStorageSelection,
        span: Span,
    ) -> Result<HashSet<mir::RetainedStorageSelector>, BackendError> {
        let mut result = HashSet::new();
        for selector in &selection.selectors {
            match selector {
                mir::RetainedStorageSelector::All => {
                    result.insert(mir::RetainedStorageSelector::All);
                }
                mir::RetainedStorageSelector::Member(identity) => {
                    result.insert(mir::RetainedStorageSelector::Member(
                        self.call_identity(identity, span),
                    ));
                }
                mir::RetainedStorageSelector::Input(local) => {
                    if let Some(input) = inputs.get(callee, *local)? {
                        result.extend(self.value_selectors(program, input)?);
                    }
                }
                mir::RetainedStorageSelector::Unknown => {
                    result.insert(mir::RetainedStorageSelector::Unknown);
                }
            }
        }
        Ok(result)
    }

    fn value_selectors(
        &self,
        program: &mir::Program,
        value: &mir::Rvalue,
    ) -> Result<HashSet<mir::RetainedStorageSelector>, BackendError> {
        match value {
            mir::Rvalue::Class(mir::ClassExpression::Upcast { value, .. }) => {
                return self.value_selectors(program, &mir::Rvalue::Class((**value).clone()));
            }
            mir::Rvalue::NullableClass(mir::NullableClassExpression::Upcast { value, .. }) => {
                return self
                    .value_selectors(program, &mir::Rvalue::NullableClass((**value).clone()));
            }
            _ => {}
        }
        if let Some(local) = value.direct_place_local() {
            return Ok(self.selectors[local.0].clone());
        }
        match value {
            mir::Rvalue::Value(mir::ValueExpression::Inline(
                mir::InlineValueExpression::Call {
                    function,
                    args,
                    span,
                    ..
                },
            ))
            | mir::Rvalue::NullableScalar(mir::NullableScalarExpression::Call {
                function,
                args,
                span,
                ..
            }) => self.call_selectors(program, function_in(program, *function)?, args, *span),
            mir::Rvalue::NullableScalar(mir::NullableScalarExpression::NullSafeCall {
                object,
                function,
                args,
                span,
                ..
            }) => {
                let mut inputs = vec![mir::Rvalue::NullableClass(*object.clone())];
                inputs.extend(args.iter().cloned());
                self.call_selectors(program, function_in(program, *function)?, &inputs, *span)
            }
            mir::Rvalue::NullableScalar(mir::NullableScalarExpression::Value(value)) => {
                self.value_selectors(program, &mir::Rvalue::Value(value.clone()))
            }
            mir::Rvalue::Value(mir::ValueExpression::Inline(
                mir::InlineValueExpression::Coalesce { left, right, .. },
            )) => {
                let mut selectors =
                    self.value_selectors(program, &mir::Rvalue::NullableScalar(*left.clone()))?;
                selectors.extend(self.value_selectors(
                    program,
                    &mir::Rvalue::Value(mir::ValueExpression::Inline(*right.clone())),
                )?);
                Ok(selectors)
            }
            mir::Rvalue::NullableScalar(mir::NullableScalarExpression::Coalesce {
                left,
                right,
                ..
            }) => {
                let mut selectors =
                    self.value_selectors(program, &mir::Rvalue::NullableScalar(*left.clone()))?;
                selectors.extend(
                    self.value_selectors(program, &mir::Rvalue::NullableScalar(*right.clone()))?,
                );
                Ok(selectors)
            }
            mir::Rvalue::NullableScalar(mir::NullableScalarExpression::Null(_)) => {
                Ok(HashSet::new())
            }
            _ if is_key(value.ty()) => Ok(HashSet::from([mir::RetainedStorageSelector::Unknown])),
            _ => Ok(HashSet::new()),
        }
    }

    fn call_selectors(
        &self,
        program: &mir::Program,
        callee: &mir::Function,
        args: &[mir::Rvalue],
        span: Span,
    ) -> Result<HashSet<mir::RetainedStorageSelector>, BackendError> {
        self.call_selectors_with_inputs(program, callee, args.into(), span)
    }

    /// Expands a call site's static callee into every executable candidate.
    fn call_selectors_with_inputs(
        &self,
        program: &mir::Program,
        callee: &mir::Function,
        args: CallInputs<'_>,
        span: Span,
    ) -> Result<HashSet<mir::RetainedStorageSelector>, BackendError> {
        let mut selectors = HashSet::new();
        for candidate in retained::possible_callees(
            program,
            callee.id,
            (args.implicit == 0)
                .then(|| retained::receiver_class(args.args))
                .flatten(),
        )? {
            selectors.extend(self.candidate_selectors(program, candidate, args, span)?);
        }
        Ok(selectors)
    }

    /// Selectors returned by one executable body. A dispatch or vtable entry
    /// is already a candidate; expanding it again would ask its own override
    /// set about the caller's receiver domain.
    fn candidate_selectors(
        &self,
        program: &mir::Program,
        candidate: &mir::Function,
        args: CallInputs<'_>,
        span: Span,
    ) -> Result<HashSet<mir::RetainedStorageSelector>, BackendError> {
        let mut selectors = HashSet::new();
        {
            for selector in retained::contract(candidate)
                .into_iter()
                .flat_map(|plan| &plan.returned_selectors)
            {
                match selector {
                    mir::RetainedStorageSelector::All => {
                        selectors.insert(mir::RetainedStorageSelector::All);
                    }
                    mir::RetainedStorageSelector::Member(identity) => {
                        selectors.insert(mir::RetainedStorageSelector::Member(
                            self.call_identity(identity, span),
                        ));
                    }
                    mir::RetainedStorageSelector::Input(local) => {
                        if candidate.params.contains(local) {
                            selectors.extend(
                                self.value_selectors(program, argument(candidate, args, *local)?)?,
                            );
                        } else {
                            selectors.insert(mir::RetainedStorageSelector::Unknown);
                        }
                    }
                    mir::RetainedStorageSelector::Unknown => {
                        selectors.insert(mir::RetainedStorageSelector::Unknown);
                    }
                }
            }
        }
        Ok(selectors)
    }

    pub(super) fn assign(&mut self, target: mir::LocalId, value: &Assignment, owned: bool) {
        self.contents[target.0] = if owned {
            value.contents.clone()
        } else {
            Contents::default()
        };
        self.selectors[target.0] = value.selectors.clone();
    }

    pub(super) fn reset(&mut self, target: mir::LocalId, ty: mir::Type) {
        self.contents[target.0] = Contents::default();
        self.selectors[target.0].clear();
        if is_key(ty) {
            self.selectors[target.0].insert(mir::RetainedStorageSelector::Unknown);
        }
    }

    pub(super) fn join(
        &mut self,
        incoming: &Self,
        sources: &[HashSet<Source>],
        incoming_sources: &[HashSet<Source>],
    ) {
        for (index, (contents, incoming)) in
            self.contents.iter_mut().zip(&incoming.contents).enumerate()
        {
            contents.join(incoming, &sources[index], &incoming_sources[index]);
        }
        for (selectors, incoming) in self.selectors.iter_mut().zip(&incoming.selectors) {
            selectors.extend(incoming.iter().cloned());
        }
        self.removals
            .retain(|selection| incoming.removals.contains(selection));
    }
}

impl State {
    pub(super) fn clear_collection_sources(&mut self, collection: mir::LocalId) {
        let places = self.roots[collection.0].clone();
        if places.len() != 1 {
            return;
        }
        for place in places.into_iter().filter(|place| !place.opaque) {
            let selectors = HashSet::from([mir::RetainedStorageSelector::All]);
            self.remove_sources(&place, &selectors);
            self.storage
                .removals
                .insert(selection_at_place(&place, &selectors));
        }
    }

    pub(super) fn stored_call(
        &mut self,
        program: &mir::Program,
        accesses: &ClassLocalAccesses<'_>,
        position: usize,
        target: CallTarget,
        args: &[mir::Rvalue],
        span: Span,
    ) -> Result<(), BackendError> {
        let CallTarget::Direct {
            function,
            parameter_offset,
        } = target
        else {
            return Ok(());
        };
        let receiver = accesses
            .method_receivers
            .iter()
            .find_map(|(at, _, receiver)| (*at == position).then_some(*receiver));
        let mut inputs = Vec::new();
        if let Some(receiver) = receiver {
            inputs.push(mir::Rvalue::NullableClass(receiver.clone()));
            inputs.extend_from_slice(args);
        }
        self.dispatch_storage_call(
            program,
            retained::possible_callees(
                program,
                function,
                receiver
                    .map(mir::NullableClassExpression::static_receiver_class)
                    .or_else(|| {
                        (parameter_offset == 0)
                            .then(|| retained::receiver_class(args))
                            .flatten()
                    }),
            )?,
            CallInputs {
                args: if receiver.is_some() { &inputs } else { args },
                implicit: if receiver.is_some() {
                    0
                } else {
                    parameter_offset
                },
            },
            None,
            true,
            span,
        )?;
        Ok(())
    }

    pub(super) fn returned_storage_on_edge(
        &self,
        program: &mir::Program,
        terminator: &mir::Terminator,
        target: mir::BlockId,
        origin: Option<&call_origins::CallbackOrigin>,
    ) -> Result<Option<Assignment>, BackendError> {
        match terminator {
            mir::Terminator::CheckedCall {
                function,
                args,
                span,
                success,
                ..
            } if *success == target => {
                let callee = function_in(program, *function)?;
                Ok(Some(Assignment {
                    contents: self.storage.call_contents(
                        program,
                        callee,
                        args,
                        *span,
                        &self.roots,
                        &self.loans,
                    )?,
                    selectors: self.storage.call_selectors(program, callee, args, *span)?,
                }))
            }
            mir::Terminator::IndirectCall {
                callee,
                args,
                span,
                continuation: success,
                ..
            }
            | mir::Terminator::CheckedIndirectCall {
                callee,
                args,
                span,
                success,
                ..
            } if *success == target => {
                if let mir::IndirectCallee::Closure(value) = callee {
                    if let Some(origin) = origin.filter(|origin| !origin.unknown) {
                        let mut assignment = Assignment {
                            contents: Contents {
                                direct: Some(HashSet::new()),
                                ..Default::default()
                            },
                            selectors: HashSet::new(),
                        };
                        for target in &origin.targets {
                            let callee = function_in(program, *target)?;
                            let inputs = CallInputs { args, implicit: 1 };
                            let incoming = self.storage.instantiate_contents(
                                program,
                                callee,
                                inputs,
                                *span,
                                &self.roots,
                                &self.loans,
                            )?;
                            assignment
                                .contents
                                .join(&incoming, &HashSet::new(), &HashSet::new());
                            assignment.selectors.extend(
                                self.storage
                                    .candidate_selectors(program, callee, inputs, *span)?,
                            );
                        }
                        return Ok(Some(assignment));
                    }
                    let contract = function_type_in(program, value.function_type())?;
                    if let Some(borrow) = contract.return_borrow {
                        let mir::BorrowSource::Parameter(index) = borrow.source else {
                            return Err(malformed_mir(
                                "returned callback storage has no explicit parameter source",
                            ));
                        };
                        let forwards_contents = match borrow.kind {
                            mir::ReturnBorrowKind::Value => matches!(contract.return_type,
                                mir::ReturnType::Value(ty) if retained::carries_source(program, ty)),
                            mir::ReturnBorrowKind::Retained => {
                                contract.parameters[index].mode == mir::FunctionParameterMode::Take
                            }
                        };
                        if forwards_contents {
                            let source = args.get(index).ok_or_else(|| {
                                malformed_mir("returned callback storage source is missing")
                            })?;
                            return self
                                .storage
                                .value(program, source, &self.roots, &self.loans)
                                .map(Some);
                        }
                    }
                    return Ok(None);
                }
                let mir::IndirectCallee::InterfaceMethod {
                    interface, slot, ..
                } = callee
                else {
                    return Ok(None);
                };
                let mut assignment = Assignment {
                    contents: Contents {
                        direct: Some(HashSet::new()),
                        ..Default::default()
                    },
                    selectors: HashSet::new(),
                };
                for table in &program.interface_vtables {
                    if table.interface == *interface {
                        let entry = table.methods.get(*slot).ok_or_else(|| {
                            malformed_mir("returned storage interface call has no method entry")
                        })?;
                        // A vtable entry is already the executable implementation.
                        let callee = function_in(program, *entry)?;
                        let incoming = self.storage.instantiate_contents(
                            program,
                            callee,
                            args.as_slice().into(),
                            *span,
                            &self.roots,
                            &self.loans,
                        )?;
                        assignment
                            .contents
                            .join(&incoming, &HashSet::new(), &HashSet::new());
                        assignment
                            .selectors
                            .extend(self.storage.candidate_selectors(
                                program,
                                callee,
                                args.as_slice().into(),
                                *span,
                            )?);
                    }
                }
                Ok(Some(assignment))
            }
            _ => Ok(None),
        }
    }

    fn store_sources(
        &mut self,
        place: Place,
        identity: mir::RetainedStorageIdentity,
        sources: HashSet<Source>,
    ) {
        let contents = &mut self.storage.contents[place.local.0];
        contents
            .direct
            .get_or_insert_with(|| self.loans[place.local.0].clone());
        if let Some(member) = contents
            .members
            .iter_mut()
            .find(|member| member.identity == identity && member.path == place.fields)
        {
            member.repeated = true;
            member.sources.extend(sources);
        } else {
            contents.members.push(Member {
                identity,
                path: place.fields,
                sources,
                repeated: false,
            });
        }
        self.loans[place.local.0] = contents.sources();
    }

    pub(super) fn store_collection_sources(
        &mut self,
        collection: mir::LocalId,
        origin: Option<Span>,
        incoming: Assignment,
        function: &mir::Function,
    ) -> Result<(), BackendError> {
        let sources = incoming.sources();
        if sources.is_empty() {
            return Ok(());
        }
        for place in self.roots[collection.0].clone() {
            if let Some(origin) = origin {
                self.store_sources(
                    place,
                    mir::RetainedStorageIdentity {
                        origin,
                        callers: Vec::new(),
                    },
                    sources.clone(),
                );
            } else {
                if function.params.contains(&place.local) && !function.locals[place.local.0].owned {
                    return Err(malformed_mir(
                        "collection mutation loses its stored owner origin",
                    ));
                }
                self.storage.contents[place.local.0].join(
                    &incoming.contents,
                    &self.loans[place.local.0],
                    &sources,
                );
                self.loans[place.local.0] = self.storage.contents[place.local.0].sources();
            }
        }
        Ok(())
    }

    fn remove_sources(
        &mut self,
        place: &Place,
        selectors: &HashSet<mir::RetainedStorageSelector>,
    ) -> HashSet<Source> {
        let contents = &mut self.storage.contents[place.local.0];
        let sources = contents.remove(&place.fields, selectors, &self.loans[place.local.0]);
        if contents.direct.is_some() {
            self.loans[place.local.0] = contents.sources();
        }
        sources
    }

    fn apply_call_removals(
        &mut self,
        program: &mir::Program,
        callee: &mir::Function,
        inputs: CallInputs<'_>,
        selections: &[mir::RetainedStorageSelection],
        span: Span,
    ) -> Result<(), BackendError> {
        for selection in selections {
            let Some(input) = inputs.get(callee, selection.destination)? else {
                continue;
            };
            let places = retained::source_roots(program, input, &self.roots)?;
            let mut selectors = self
                .storage
                .selection_selectors(program, callee, inputs, selection, span)?;
            // An ambiguous owner cannot prove which predecessor lost a member.
            if places.len() != 1 {
                selectors.insert(mir::RetainedStorageSelector::Unknown);
            }
            for mut place in places {
                for property in &selection.path {
                    place = place.project(*property);
                }
                if selectors.contains(&mir::RetainedStorageSelector::All) && place.opaque {
                    continue;
                }
                self.remove_sources(&place, &selectors);
                self.storage
                    .removals
                    .insert(selection_at_place(&place, &selectors));
            }
        }
        Ok(())
    }

    pub(super) fn stored_call_edge(
        &mut self,
        program: &mir::Program,
        terminator: &mir::Terminator,
        target: mir::BlockId,
    ) -> Result<(), BackendError> {
        let indirect = match terminator {
            mir::Terminator::IndirectCall {
                callee,
                args,
                result,
                continuation,
                span,
                ..
            } if *continuation == target => Some((callee, args, result, span, true)),
            mir::Terminator::CheckedIndirectCall {
                callee,
                args,
                result,
                success,
                span,
                ..
            } => Some((callee, args, result, span, *success == target)),
            _ => None,
        };
        if let Some((
            mir::IndirectCallee::InterfaceMethod {
                interface, slot, ..
            },
            args,
            result,
            span,
            succeeded,
        )) = indirect
        {
            let mut candidates = Vec::new();
            for table in &program.interface_vtables {
                if table.interface == *interface {
                    let implementation = table.methods.get(*slot).ok_or_else(|| {
                        malformed_mir("ownership selector interface call has no method entry")
                    })?;
                    candidates.push(function_in(program, *implementation)?);
                }
            }
            return self.dispatch_storage_call(
                program,
                candidates,
                args.as_slice().into(),
                *result,
                succeeded,
                *span,
            );
        }
        if let mir::Terminator::CheckedIo {
            operation,
            result,
            success,
            span,
            ..
        } = terminator
        {
            if *success == target {
                let (set, stream, key) = match operation {
                    mir::CheckedIoOperation::RegisterReadinessStream { set, stream, .. } => {
                        (*set, Some(*stream), None)
                    }
                    mir::CheckedIoOperation::UnregisterReadinessStream { set, key, .. } => {
                        (*set, None, Some(*key))
                    }
                    _ => return Ok(()),
                };
                let places = self.roots[set.0].clone();
                let sources = stream
                    .map(|stream| self.loans[stream.0].clone())
                    .unwrap_or_default();
                let selectors = key
                    .map(|key| self.storage.selectors[key.0].clone())
                    .unwrap_or_default();
                self.membership_update(places, sources, selectors, operation, *result, *span);
            }
            return Ok(());
        }
        if let mir::Terminator::CheckedConstruct {
            constructor,
            args,
            success,
            span,
            ..
        } = terminator
        {
            return self.dispatch_storage_call(
                program,
                retained::possible_callees(program, *constructor, None)?,
                CallInputs { args, implicit: 1 },
                None,
                *success == target,
                *span,
            );
        }
        let mir::Terminator::CheckedCall {
            function,
            args,
            result,
            success,
            span,
            ..
        } = terminator
        else {
            return Ok(());
        };
        self.dispatch_storage_call(
            program,
            retained::possible_callees(program, *function, retained::receiver_class(args))?,
            args.as_slice().into(),
            *result,
            *success == target,
            *span,
        )
    }

    fn dispatch_storage_call(
        &mut self,
        program: &mir::Program,
        candidates: Vec<&mir::Function>,
        inputs: CallInputs<'_>,
        result: Option<mir::LocalId>,
        succeeded: bool,
        span: Span,
    ) -> Result<(), BackendError> {
        let mut joined = None::<Self>;
        for callee in candidates {
            let mut branch = self.clone();
            branch.storage_call(program, callee, inputs, result, succeeded, span)?;
            if let Some(joined) = &mut joined {
                joined.join(branch);
            } else {
                joined = Some(branch);
            }
        }
        if let Some(joined) = joined {
            *self = joined;
        }
        Ok(())
    }

    fn storage_call(
        &mut self,
        program: &mir::Program,
        callee: &mir::Function,
        inputs: CallInputs<'_>,
        result: Option<mir::LocalId>,
        succeeded: bool,
        span: Span,
    ) -> Result<(), BackendError> {
        // Identify the compiler-owned operation from its executable body. User
        // helpers use their checked effects, never a method-name heuristic.
        if crate::compiler_known_stream::readiness_function(callee.source_span).is_some() {
            if let Some(operation) =
                callee
                    .blocks
                    .iter()
                    .find_map(|block| match &block.terminator {
                        mir::Terminator::CheckedIo { operation, .. } => Some(operation),
                        _ => None,
                    })
            {
                if succeeded {
                    return self.membership_edge(program, callee, operation, inputs, result, span);
                }
                return Ok(());
            }
        }
        let Some(plan) = retained::contract(callee) else {
            return Ok(());
        };
        for store in if succeeded {
            &plan.stores
        } else {
            &plan.exceptional_stores
        } {
            let Some(input) = inputs.get(callee, store.destination)? else {
                continue;
            };
            let sources = map_call_sources(
                program,
                callee,
                inputs,
                &store.sources,
                &self.roots,
                &self.loans,
            )?;
            for mut place in retained::source_roots(program, input, &self.roots)? {
                for property in &store.path {
                    place = place.project(*property);
                }
                self.store_sources(
                    place,
                    self.storage.call_identity(&store.identity, span),
                    sources.clone(),
                );
            }
        }
        self.apply_call_removals(
            program,
            callee,
            inputs,
            if succeeded {
                &plan.removals
            } else {
                &plan.exceptional_removals
            },
            span,
        )?;
        if succeeded {
            if let Some(result) = result {
                self.storage.selectors[result.0] = self
                    .storage
                    .candidate_selectors(program, callee, inputs, span)?;
            }
        }
        Ok(())
    }

    fn membership_edge(
        &mut self,
        program: &mir::Program,
        callee: &mir::Function,
        operation: &mir::CheckedIoOperation,
        inputs: CallInputs<'_>,
        result: Option<mir::LocalId>,
        span: Span,
    ) -> Result<(), BackendError> {
        let (set, stream, key) = match operation {
            mir::CheckedIoOperation::RegisterReadinessStream { set, stream, .. } => {
                (*set, Some(*stream), None)
            }
            mir::CheckedIoOperation::UnregisterReadinessStream { set, key, .. } => {
                (*set, None, Some(*key))
            }
            _ => return Ok(()),
        };
        let Some(set) = inputs.get(callee, set)? else {
            return Ok(());
        };
        let places = retained::source_roots(program, set, &self.roots)?;
        let sources = if let Some(stream) = stream {
            retained::value_sources(
                program,
                inputs
                    .get(callee, stream)?
                    .ok_or_else(|| malformed_mir("registered stream has no caller input"))?,
                &self.roots,
                &self.loans,
            )?
        } else {
            HashSet::new()
        };
        let selectors = if let Some(key) = key {
            self.storage.value_selectors(
                program,
                inputs
                    .get(callee, key)?
                    .ok_or_else(|| malformed_mir("ownership selector has no caller input"))?,
            )?
        } else {
            HashSet::new()
        };
        self.membership_update(places, sources, selectors, operation, result, span);
        Ok(())
    }

    fn membership_update(
        &mut self,
        places: HashSet<Place>,
        sources: HashSet<Source>,
        selectors: HashSet<mir::RetainedStorageSelector>,
        operation: &mir::CheckedIoOperation,
        result: Option<mir::LocalId>,
        span: Span,
    ) {
        if matches!(
            operation,
            mir::CheckedIoOperation::RegisterReadinessStream { .. }
        ) {
            let identity = mir::RetainedStorageIdentity {
                origin: span,
                callers: Vec::new(),
            };
            for place in places {
                self.store_sources(place, identity.clone(), sources.clone());
            }
            if let Some(result) = result {
                self.storage.selectors[result.0] =
                    HashSet::from([mir::RetainedStorageSelector::Member(identity)]);
            }
        } else {
            let mut sources = HashSet::new();
            let mut selected = Contents {
                direct: Some(HashSet::new()),
                ..Default::default()
            };
            let mut selectors = selectors;
            if places.len() != 1 {
                selectors.insert(mir::RetainedStorageSelector::Unknown);
            }
            for place in places {
                let inherited = self.storage.contents[place.local.0]
                    .direct
                    .as_ref()
                    .unwrap_or(&self.loans[place.local.0])
                    .clone();
                let removed = self.remove_sources(&place, &selectors);
                sources.extend(removed.iter().cloned());
                let selection = selection_at_place(&place, &selectors);
                self.storage.removals.insert(selection.clone());
                selected
                    .direct
                    .as_mut()
                    .expect("normalized storage")
                    .extend(removed.difference(&inherited).cloned());
                selected.select(selection, inherited);
            }
            if let Some(result) = result {
                if matches!(operation,
                    mir::CheckedIoOperation::UnregisterReadinessStream { stream_type, .. }
                    if stream_type.has_move_ownership())
                {
                    self.loans[result.0] = sources;
                    self.storage.contents[result.0] = selected;
                }
            }
        }
    }

    pub(super) fn validate_stored_exit(
        &self,
        function: &mir::Function,
        exceptional: bool,
    ) -> Result<(), BackendError> {
        let declared = retained::contract(function).map(|plan| {
            if exceptional {
                &plan.exceptional_stores
            } else {
                &plan.stores
            }
        });
        for parameter in &function.params {
            if function.locals[parameter.0].owned {
                continue;
            }
            for member in &self.storage.contents[parameter.0].members {
                let store = declared.and_then(|stores| {
                    stores.iter().find(|store| {
                        store.destination == *parameter
                            && store.path == member.path
                            && store.identity == member.identity
                    })
                });
                if member.sources.iter().any(|source| {
                    !function.params.contains(&source.local)
                        || (!source.inherited && function.locals[source.local.0].owned)
                        || store.is_none_or(|store| {
                            !store.sources.iter().any(|declared| {
                                declared.local == source.local
                                    && declared.inherited == source.inherited
                                    && source.place.fields.starts_with(&declared.owned_path)
                                    && (!source.writable || declared.writable)
                                    && declared.lifetime.accepts(source.lifetime)
                            })
                        })
                }) {
                    return Err(malformed_mir(
                        "stored owner loses or outlives a retained input source",
                    ));
                }
            }
        }
        Ok(())
    }

    pub(super) fn returned_storage_selectors(
        &self,
        program: &mir::Program,
        value: &mir::Rvalue,
    ) -> Result<HashSet<mir::RetainedStorageSelector>, BackendError> {
        self.storage.value_selectors(program, value)
    }

    pub(super) fn returned_storage(
        &self,
        program: &mir::Program,
        function: &mir::Function,
        value: &mir::Rvalue,
    ) -> Result<mir::RetainedReturnedStorage, BackendError> {
        let contents = self
            .storage
            .value(program, value, &self.roots, &self.loans)?
            .contents;
        let loans = retained::value_sources(program, value, &self.roots, &self.loans)?;
        let mut direct = contents.direct.as_ref().unwrap_or(&loans).clone();
        for (selection, sources) in &contents.selections {
            if !function.params.contains(&selection.destination) {
                direct.extend(sources.iter().cloned());
            }
        }
        Ok(mir::RetainedReturnedStorage {
            inputs: contents.inputs.iter().copied().collect(),
            direct: direct
                .iter()
                .filter(|source| {
                    !(source.inherited
                        && source.place.fields.is_empty()
                        && contents.inputs.contains(&source.local))
                })
                .map(|source| source.contract.clone())
                .collect(),
            selections: contents
                .selections
                .iter()
                .filter(|(s, _)| function.params.contains(&s.destination))
                .map(|(s, _)| s.clone())
                .collect(),
            members: contents
                .members
                .iter()
                .map(|member| mir::RetainedReturnedMember {
                    identity: member.identity.clone(),
                    path: member.path.clone(),
                    sources: member
                        .sources
                        .iter()
                        .map(|source| source.contract.clone())
                        .collect(),
                    repeated: member.repeated,
                })
                .collect(),
        })
    }

    pub(super) fn removed_storage(
        &self,
        function: &mir::Function,
    ) -> HashSet<mir::RetainedStorageSelection> {
        self.storage
            .removals
            .iter()
            .filter(|selection| function.params.contains(&selection.destination))
            .cloned()
            .collect()
    }
}

fn selection_at_place(
    place: &Place,
    selectors: &HashSet<mir::RetainedStorageSelector>,
) -> mir::RetainedStorageSelection {
    let mut selectors = selectors.iter().cloned().collect::<Vec<_>>();
    selectors.sort();
    mir::RetainedStorageSelection {
        destination: place.local,
        path: place.fields.clone(),
        selectors,
    }
}

pub(super) fn validate_removals(
    function: &mir::Function,
    normal: Vec<HashSet<mir::RetainedStorageSelection>>,
    exceptional: Vec<HashSet<mir::RetainedStorageSelection>>,
) -> Result<(), BackendError> {
    let plan = retained::contract(function);
    for (exits, declared) in [
        (normal, plan.map(|p| &p.removals)),
        (exceptional, plan.map(|p| &p.exceptional_removals)),
    ] {
        let mut exits = exits.into_iter();
        let mut inferred = exits.next().unwrap_or_default();
        for exit in exits {
            inferred.retain(|selection| exit.contains(selection));
        }
        let declared = declared
            .into_iter()
            .flatten()
            .cloned()
            .collect::<HashSet<_>>();
        if inferred != declared {
            return Err(malformed_mir(format!(
                "removed storage disagrees with executable ownership effects in `{}`: inferred {inferred:?}, declared {declared:?}",
                function.name
            )));
        }
    }
    Ok(())
}

fn map_call_sources(
    program: &mir::Program,
    callee: &mir::Function,
    inputs: CallInputs<'_>,
    sources: &[mir::RetainedSource],
    roots: &[HashSet<Place>],
    loans: &[HashSet<Source>],
) -> Result<HashSet<Source>, BackendError> {
    let mut result = HashSet::new();
    for source in sources {
        let Some(input) = inputs.get(callee, source.local)? else {
            continue;
        };
        let incoming = if source.inherited {
            retained::value_sources(program, input, roots, loans)?
        } else {
            retained::borrowed_sources(
                program,
                input,
                source.writable,
                source.lifetime,
                roots,
                loans,
            )?
        };
        for mut incoming in incoming {
            for property in &source.owned_path {
                incoming.place = incoming.place.project(*property);
            }
            incoming.contract.owned_path = incoming.place.fields.clone();
            incoming.lifetime = source.lifetime;
            result.insert(incoming);
        }
    }
    Ok(result)
}

fn argument<'a>(
    callee: &mir::Function,
    args: CallInputs<'a>,
    local: mir::LocalId,
) -> Result<&'a mir::Rvalue, BackendError> {
    args.get(callee, local)?
        .ok_or_else(|| malformed_mir("stored owner contract has no actual input"))
}

pub(super) fn validate_returned_storage(
    function: &mir::Function,
    inferred: Vec<mir::RetainedReturnedStorage>,
) -> Result<(), BackendError> {
    let declared = retained::contract(function).and_then(|plan| plan.returned_storage.as_ref());
    if declared.is_none()
        && inferred.iter().all(|storage| {
            storage.inputs.is_empty() && storage.members.is_empty() && storage.selections.is_empty()
        })
    {
        return Ok(());
    }
    let summarize = |storages: &[mir::RetainedReturnedStorage]| {
        let mut inputs = HashSet::<mir::LocalId>::new();
        let mut direct = HashSet::new();
        let mut members = std::collections::HashMap::new();
        let mut selections = HashSet::new();
        for storage in storages {
            inputs.extend(storage.inputs.iter().copied());
            selections.extend(storage.selections.iter().cloned());
            direct.extend(storage.direct.iter().cloned());
            for member in &storage.members {
                let (sources, repeated) = members
                    .entry((member.identity.clone(), member.path.clone()))
                    .or_insert_with(|| (HashSet::new(), false));
                sources.extend(member.sources.iter().cloned());
                *repeated |= member.repeated;
            }
        }
        (inputs, direct, members, selections)
    };
    let inferred = summarize(&inferred);
    let declared = summarize(&declared.into_iter().cloned().collect::<Vec<_>>());
    if inferred != declared {
        return Err(malformed_mir(format!(
            "returned storage disagrees with executable ownership provenance in `{}`: inferred {inferred:?}, declared {declared:?}",
            function.name
        )));
    }
    Ok(())
}

fn is_key(ty: mir::Type) -> bool {
    matches!(
        ty,
        mir::Type::Scalar(mir::ScalarType::Inline(
            crate::inline_value::InlineValueType::RegistrationKey(_)
        )) | mir::Type::NullableScalar(mir::ScalarType::Inline(
            crate::inline_value::InlineValueType::RegistrationKey(_)
        ))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(contents: Contents, loans: HashSet<Source>) -> State {
        State {
            roots: vec![HashSet::from([Place::local(mir::LocalId(0))])],
            loans: vec![loans],
            payload_loans: HashMap::new(),
            parents: vec![HashSet::new()],
            view_parents: vec![HashSet::new()],
            view_access: vec![None],
            carrier_fields: HashMap::new(),
            ended: HashSet::new(),
            storage: Storage {
                contents: vec![contents],
                selectors: vec![HashSet::new()],
                removals: HashSet::new(),
                forwarding_site: None,
            },
        }
    }

    #[test]
    fn payload_views_preserve_storage_metadata_and_source_loans() {
        let program =
            crate::lower_source_to_mir("views.doria", "function main(): void {}").unwrap();
        let local = mir::LocalId(0);
        let class = crate::class_layout::ClassId(0);
        let interface = mir::InterfaceTypeId(0);
        let vtable = mir::InterfaceVtableId(0);
        let source = Source::input(local, false, mir::RetainedLifetime::Cleanup);
        let state = state(
            Contents {
                direct: Some(HashSet::new()),
                inputs: HashSet::from([local]),
                selections: Vec::new(),
                members: vec![Member {
                    identity: mir::RetainedStorageIdentity {
                        origin: Span::new(10, 11),
                        callers: Vec::new(),
                    },
                    path: Vec::new(),
                    sources: HashSet::from([source.clone()]),
                    repeated: false,
                }],
            },
            HashSet::from([source.clone()]),
        );
        let payload = mir::ClassExpression::InterfacePayload {
            class,
            local,
            transfer: true,
        };
        let erased = mir::InterfaceExpression {
            interface,
            value: mir::InterfaceValue::FromClass {
                object: Box::new(payload.clone()),
                vtable,
            },
        };
        let nullable = mir::NullableInterfaceExpression {
            interface,
            value: mir::NullableInterfaceValue::Present(erased.value.clone()),
        };
        let boxed = mir::MixedExpression::BoxInterface {
            value: Box::new(erased.clone()),
            payload_owned: true,
        };
        let function_type = mir::FunctionTypeId(0);
        let callable = mir::FunctionExpression::Local {
            function_type,
            local,
            transfer: true,
        };
        let nullable_callable = mir::NullableFunctionExpression::Present(callable.clone());
        for value in [
            mir::Rvalue::NullableFunction(nullable_callable.clone()),
            mir::Rvalue::Function(mir::FunctionExpression::AssumePresent {
                function_type,
                value: Box::new(nullable_callable),
            }),
            mir::Rvalue::Function(mir::FunctionExpression::MixedPayload {
                function_type,
                mixed: local,
                transfer: true,
            }),
            mir::Rvalue::Mixed(mir::MixedExpression::BoxFunction {
                value: Box::new(callable),
                payload_owned: true,
            }),
            mir::Rvalue::Class(payload.clone()),
            mir::Rvalue::Class(mir::ClassExpression::InterfaceReceiver {
                class,
                receiver: local,
                vtable,
                transfer: true,
            }),
            mir::Rvalue::NullableClass(mir::NullableClassExpression::Class(payload.clone())),
            mir::Rvalue::Interface(erased.clone()),
            mir::Rvalue::Interface(mir::InterfaceExpression {
                interface,
                value: mir::InterfaceValue::FromNullableClass {
                    object: Box::new(mir::NullableClassExpression::Class(payload.clone())),
                    vtable,
                },
            }),
            mir::Rvalue::Interface(mir::InterfaceExpression {
                interface,
                value: mir::InterfaceValue::Upcast {
                    source: Box::new(erased),
                    interface,
                },
            }),
            mir::Rvalue::NullableInterface(nullable.clone()),
            mir::Rvalue::NullableInterface(mir::NullableInterfaceExpression {
                interface,
                value: mir::NullableInterfaceValue::Upcast {
                    source: Box::new(nullable),
                    interface,
                },
            }),
            mir::Rvalue::Interface(mir::InterfaceExpression {
                interface,
                value: mir::InterfaceValue::NarrowedLocal {
                    local,
                    interface,
                    transfer: true,
                },
            }),
            mir::Rvalue::Mixed(mir::MixedExpression::BoxClass {
                value: payload.clone(),
                payload_owned: true,
            }),
            mir::Rvalue::Mixed(boxed.clone()),
            mir::Rvalue::NullableMixed(mir::NullableMixedExpression::Mixed(boxed)),
            mir::Rvalue::Class(mir::ClassExpression::MixedPayload {
                class,
                mixed: local,
                transfer: true,
            }),
            mir::Rvalue::Class(mir::ClassExpression::Coalesce {
                class,
                left: Box::new(mir::NullableClassExpression::Class(payload.clone())),
                right: Box::new(payload),
                transfer: true,
            }),
        ] {
            let assignment = state
                .storage
                .value(&program, &value, &state.roots, &state.loans)
                .unwrap();
            assert!(
                assignment.contents == state.storage.contents[0],
                "{value:?}"
            );
            assert_eq!(
                retained::value_sources(&program, &value, &state.roots, &state.loans).unwrap(),
                HashSet::from([source.clone()]),
                "{value:?}"
            );
        }
    }

    #[test]
    fn joined_opaque_sources_survive_removal_of_a_known_member() {
        let identity = mir::RetainedStorageIdentity {
            origin: Span::new(10, 11),
            callers: Vec::new(),
        };
        let known_source = Source::input(mir::LocalId(1), false, mir::RetainedLifetime::Cleanup);
        let opaque_source = Source::input(mir::LocalId(2), false, mir::RetainedLifetime::Cleanup);
        let known = state(
            Contents {
                direct: Some(HashSet::new()),
                members: vec![Member {
                    identity: identity.clone(),
                    path: Vec::new(),
                    sources: HashSet::from([known_source.clone()]),
                    repeated: false,
                }],
                ..Default::default()
            },
            HashSet::from([known_source.clone()]),
        );
        let opaque = state(Contents::default(), HashSet::from([opaque_source.clone()]));
        for reversed in [false, true] {
            let (mut joined, incoming) = if reversed {
                (opaque.clone(), known.clone())
            } else {
                (known.clone(), opaque.clone())
            };
            joined.join(incoming);
            let removed = joined.remove_sources(
                &Place::local(mir::LocalId(0)),
                &HashSet::from([mir::RetainedStorageSelector::Member(identity.clone())]),
            );
            assert_eq!(
                removed,
                HashSet::from([known_source.clone(), opaque_source.clone()])
            );
            assert_eq!(joined.loans[0], HashSet::from([opaque_source.clone()]));
        }
    }
}

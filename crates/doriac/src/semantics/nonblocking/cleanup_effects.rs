//! Release sites come from ownership and definite-initialization analysis. This
//! consumer selects destructor effects; it does not decide who owns a value.

use super::*;
use crate::ownership::cleanup::{Analysis, Source, ValueId};

#[derive(Clone, Default)]
pub(super) struct Effects {
    pub targets: HashSet<(Span, ClassType<ResolvedType>)>,
    pub callbacks: HashSet<Value>,
    pub unknown: bool,
    pub generic: Vec<(ResolvedType, Value)>,
}

impl From<Source> for Value {
    fn from(source: Source) -> Self {
        match source {
            Source::Expression(span) => Self::Expression(span),
            Source::Binding(binding) => Self::Binding(binding),
            Source::CallbackReturn { callback } => Self::CallbackReturned(callback),
            Source::EnumPayload { .. } | Source::CollectionElement { .. } => {
                Self::Projection(source)
            }
        }
    }
}

impl Checker<'_> {
    pub(super) fn blocking_cleanups(
        &self,
        graph: &mut Graph,
        cleanup: &Analysis,
        partial_cleanups: &[crate::constructor_init::ConstructorPartialCleanup],
    ) {
        for (source, target) in &cleanup.value_flows {
            graph.flow(Value::Expression(*source), Value::Expression(*target));
        }
        for (source, target) in &cleanup.source_flows {
            for source in [source, target] {
                if let Source::EnumPayload {
                    scrutinee,
                    case,
                    field,
                } = source
                {
                    graph.unpacked_fields.insert((
                        Value::Expression(*scrutinee),
                        (*source).into(),
                        PayloadField {
                            case: *case,
                            index: *field,
                        },
                    ));
                }
            }
            graph.flow((*source).into(), (*target).into());
        }
        for value in cleanup.values.values() {
            if let Source::EnumPayload {
                scrutinee,
                case,
                field,
            } = value.source
            {
                graph.unpacked_fields.insert((
                    Value::Expression(scrutinee),
                    value.source.into(),
                    PayloadField { case, index: field },
                ));
            }
        }
        for release in &cleanup.releases {
            let mut effects = Effects::default();
            let mut visited = HashSet::new();
            for value in &release.values {
                self.released_value_effects(cleanup, *value, &mut visited, &mut effects);
            }
            self.connect_cleanup(graph, release.owner, release.site, effects);
        }
        for release in partial_cleanups {
            let mut effects = Effects::default();
            for declaration in &release.initialized_properties {
                if let Some(property) = self
                    .classes
                    .values()
                    .flat_map(|class| class.properties.values())
                    .find(|property| property.declaration_span == *declaration)
                {
                    if !property.borrowed_source {
                        self.drop_effects(
                            &self.types.resolved(property.ty),
                            Value::Property(*declaration),
                            &mut HashSet::new(),
                            &mut effects,
                        );
                    }
                }
            }
            self.connect_cleanup(
                graph,
                release.constructor.unwrap_or(release.class),
                release.site,
                effects,
            );
        }
        for closure in self.closures.values() {
            let mut effects = Effects::default();
            for capture in &closure.captures {
                if capture.mode == ClosureCaptureMode::Take {
                    self.drop_effects(
                        &capture.source_type,
                        Value::Binding(capture.source_binding_id),
                        &mut HashSet::new(),
                        &mut effects,
                    );
                }
            }
            if let Some(span) = graph
                .closures
                .keys()
                .find(|span| ClosureId::from_span(**span) == closure.closure_id)
            {
                graph.closure_cleanups.insert(*span, effects);
            }
        }
    }

    fn connect_cleanup(&self, graph: &mut Graph, owner: Span, site: Span, effects: Effects) {
        for (target, class) in effects.targets {
            graph.calls.push(Call {
                owner,
                site,
                targets: vec![target],
                callee: None,
                arguments: Vec::new(),
                bindings: self.drop_target_bindings(&class),
            });
        }
        graph.cleanups.extend(
            effects
                .callbacks
                .into_iter()
                .map(|value| (owner, site, value)),
        );
        graph.generic_cleanups.extend(
            effects
                .generic
                .into_iter()
                .map(|(ty, source)| (owner, site, ty, source)),
        );
        if effects.unknown {
            graph
                .bodies
                .entry(owner)
                .or_default()
                .blocking
                .get_or_insert(site);
        }
    }

    pub(super) fn drop_target_bindings(
        &self,
        class: &ClassType<ResolvedType>,
    ) -> HashMap<String, ResolvedType> {
        self.classes
            .get(&class.name)
            .into_iter()
            .flat_map(|declaration| declaration.type_params.iter().zip(&class.arguments))
            .map(|(parameter, ty)| (parameter.name.clone(), ty.clone()))
            .collect()
    }

    fn released_value_effects(
        &self,
        analysis: &Analysis,
        id: ValueId,
        visited: &mut HashSet<ValueId>,
        effects: &mut Effects,
    ) {
        if !visited.insert(id) {
            return;
        }
        let Some(value) = analysis.values.get(&id) else {
            return;
        };
        for content in &value.contents {
            self.released_value_effects(analysis, *content, visited, effects);
        }
        if !value.contents.is_empty()
            && matches!(value.ty, ResolvedType::Function(_) | ResolvedType::Mixed)
        {
            return;
        }
        let source = value.source.into();
        self.drop_effects(&value.ty, source, &mut HashSet::new(), effects);
    }

    pub(super) fn blocking_specializations(
        &self,
        owner: Span,
    ) -> Vec<HashMap<String, ResolvedType>> {
        // Generic bodies carry conditional cleanup obligations. Check every
        // concrete specialization recorded by the shared monomorphization facts.
        let mut bindings = Vec::new();
        let mut generic_owner = false;
        for (name, class) in &self.classes {
            if !class.type_params.is_empty()
                && class.declaration.source == owner.source
                && class.declaration.expansion == owner.expansion
                && class.declaration.start <= owner.start
                && class.declaration.end >= owner.end
            {
                generic_owner = true;
                for instance in self
                    .class_instantiations
                    .iter()
                    .filter(|instance| instance.name == *name)
                {
                    bindings.push(
                        self.class_type_substitutions(instance)
                            .into_iter()
                            .map(|(name, ty)| (name, self.types.resolved(ty)))
                            .collect::<HashMap<_, _>>(),
                    );
                }
            }
        }
        let mut method_instances = Vec::new();
        for (site, pending) in &self.pending_generic_calls {
            if self
                .call_targets
                .get(site)
                .is_some_and(|target| self.blocking_target(target).contains(&owner))
            {
                generic_owner = true;
                let method_bindings = pending
                    .bindings
                    .iter()
                    .map(|(name, ty)| (name.clone(), self.types.resolved(*ty)))
                    .collect::<HashMap<_, _>>();
                method_instances.push(method_bindings);
            }
        }
        if !method_instances.is_empty() {
            bindings = if bindings.is_empty() {
                method_instances
            } else {
                bindings
                    .iter()
                    .flat_map(|class| {
                        method_instances.iter().map(|method| {
                            let mut binding = class.clone();
                            binding.extend(method.clone());
                            binding
                        })
                    })
                    .collect()
            };
        }
        if !generic_owner {
            return vec![HashMap::new()];
        }
        bindings
    }

    pub(super) fn drop_effects(
        &self,
        ty: &ResolvedType,
        source: Value,
        visited: &mut HashSet<ResolvedType>,
        effects: &mut Effects,
    ) {
        if crate::types::resolved_type_is_symbolic(ty) {
            effects.generic.push((ty.clone(), source));
            return;
        }
        if !visited.insert(ty.clone()) {
            return;
        }
        match ty {
            ResolvedType::Function(_) => {
                effects.callbacks.insert(source);
            }
            ResolvedType::Nullable(inner)
            | ResolvedType::TypedArray(inner)
            | ResolvedType::List(inner)
            | ResolvedType::Set(inner)
            | ResolvedType::SortedSet(inner)
            | ResolvedType::PriorityQueue(inner)
            | ResolvedType::Deque(inner) => self.drop_effects(inner, source, visited, effects),
            ResolvedType::Dictionary(key, value) | ResolvedType::SortedDictionary(key, value) => {
                self.drop_effects(key, source, visited, effects);
                self.drop_effects(value, source, visited, effects);
            }
            ResolvedType::SharedHandle(kind, inner) => {
                if !matches!(
                    kind,
                    SharedHandleKind::WeakReference | SharedHandleKind::WritableWeakReference
                ) {
                    self.drop_effects(inner, source, visited, effects);
                }
            }
            ResolvedType::Class(class_type) => {
                let Some(class) = self.classes.get(&class_type.name) else {
                    effects.unknown = true;
                    return;
                };
                if let Some(destructor) = class.methods.get("__destruct") {
                    effects
                        .targets
                        .insert((destructor.declaration, class_type.clone()));
                }
                let substitutions = class
                    .type_params
                    .iter()
                    .map(|param| param.name.clone())
                    .zip(class_type.arguments.iter().cloned())
                    .collect::<HashMap<_, _>>();
                for property in class.properties.values() {
                    if property.borrowed_source
                        || property.hooks.as_ref().is_some_and(|hooks| {
                            hooks.storage
                                == Some(crate::property_hooks::PropertyHookStorage::Computed)
                        })
                    {
                        continue;
                    }
                    let ty = crate::types::substitute_resolved_type(
                        &self.types.resolved(property.ty),
                        &substitutions,
                    );
                    self.drop_effects(
                        &ty,
                        Value::Property(property.declaration_span),
                        visited,
                        effects,
                    );
                }
                if let Some(parent) = &class.parent {
                    let parent = ResolvedType::Class(ClassType {
                        name: parent.name.clone(),
                        arguments: parent
                            .arguments
                            .iter()
                            .map(|ty| self.types.resolved(*ty))
                            .collect(),
                    });
                    self.drop_effects(
                        &crate::types::substitute_resolved_type(&parent, &substitutions),
                        source,
                        visited,
                        effects,
                    );
                }
            }
            ResolvedType::Interface(interface) => {
                let implementations = self
                    .contracts
                    .conformances
                    .iter()
                    .filter(|conformance| conformance.interface == *interface)
                    .collect::<Vec<_>>();
                if implementations.is_empty() {
                    effects.unknown = true;
                }
                for conformance in implementations {
                    self.drop_effects(&conformance.implementing_type, source, visited, effects);
                }
            }
            ResolvedType::Enum(enum_type) => {
                if let Some(declaration) = self.enums.get(&enum_type.name) {
                    for case in &declaration.cases {
                        for field in &case.payload {
                            self.drop_effects(
                                &self.types.resolved(field.ty),
                                source,
                                visited,
                                effects,
                            );
                        }
                    }
                }
            }
            ResolvedType::Mixed
            | ResolvedType::Error
            | ResolvedType::TypeParameter(_)
            | ResolvedType::InterfaceSelf(_)
            | ResolvedType::TraitSelf(_)
            | ResolvedType::Unsupported => effects.unknown = true,
            ResolvedType::Void
            | ResolvedType::Integer(_)
            | ResolvedType::Float(_)
            | ResolvedType::String
            | ResolvedType::Bytes
            | ResolvedType::Bool
            | ResolvedType::Null => {}
        }
        visited.remove(ty);
    }
}

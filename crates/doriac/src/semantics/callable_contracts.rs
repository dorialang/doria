//! Refine callable contracts before either ordinary or guarded body diagnostics.

use super::*;

pub(super) type MemberDeclarations<'a> =
    HashMap<Span, (std::borrow::Cow<'a, FunctionDecl>, &'a ClassDecl)>;

pub(super) fn member_declarations(program: &Program) -> MemberDeclarations<'_> {
    let mut declarations = HashMap::new();
    for item in &program.items {
        let Item::Class(class) = item else { continue };
        for member in &class.members {
            for method in crate::property_hooks::member_callables(
                member,
                crate::property_hooks::PropertyHookContext::Class,
            ) {
                declarations.insert(method.span, (method, class));
            }
        }
    }
    declarations
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Instance {
    declaration: Span,
    bindings: Vec<(String, ResolvedType)>,
}

impl Instance {
    pub(crate) fn new(declaration: Span, substitutions: &HashMap<String, ResolvedType>) -> Self {
        let mut bindings = substitutions
            .iter()
            .map(|(name, ty)| (name.clone(), ty.clone()))
            .collect::<Vec<_>>();
        bindings.sort_by(|left, right| left.0.cmp(&right.0));
        Self {
            declaration,
            bindings,
        }
    }

    pub(crate) fn declaration(&self) -> Span {
        self.declaration
    }

    pub(crate) fn binding(&self, name: &str) -> Option<&ResolvedType> {
        self.bindings
            .binary_search_by(|(key, _)| key.as_str().cmp(name))
            .ok()
            .map(|index| &self.bindings[index].1)
    }

    pub(crate) fn binding_count(&self) -> usize {
        self.bindings.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InterfaceGenericCall {
    pub interface: crate::types::InterfaceType<ResolvedType>,
    pub requirement: Span,
    pub arguments: Vec<GenericArgument>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GenericCall {
    pub site: Span,
    pub targets: Vec<Instance>,
    pub interface: Option<InterfaceGenericCall>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum CallGraph {
    #[default]
    Unprepared,
    Checked(HashMap<Option<Instance>, Vec<GenericCall>>),
}

impl CallGraph {
    pub(super) fn begin() -> Self {
        Self::Checked(HashMap::new())
    }

    pub(super) fn scope(&mut self, caller: Option<Instance>) {
        if let Self::Checked(scopes) = self {
            scopes.entry(caller).or_default();
        }
    }

    pub(super) fn record(&mut self, caller: Option<Instance>, call: GenericCall) {
        if let Self::Checked(scopes) = self {
            let calls = scopes.entry(caller).or_default();
            if let Some(existing) = calls.iter_mut().find(|existing| existing.site == call.site) {
                *existing = call;
            } else {
                calls.push(call);
                calls.sort_by_key(|call| call.site);
            }
        }
    }

    pub(crate) fn calls(
        &self,
        caller: Option<Instance>,
        site: Span,
    ) -> DiagnosticResult<&[GenericCall]> {
        let Self::Checked(scopes) = self else {
            return Err(vec![Diagnostic::new(
                "I2401",
                "checked generic-call graph is unavailable",
                site,
            )]);
        };
        scopes.get(&caller).map(Vec::as_slice).ok_or_else(|| {
            vec![Diagnostic::new(
                "I2401",
                "checked callable has no generic-call scope",
                site,
            )]
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum Admissions {
    #[default]
    Unprepared,
    Checked(HashSet<Instance>),
}

impl Admissions {
    pub(crate) fn checked(
        instances: impl IntoIterator<Item = (Span, HashMap<String, ResolvedType>)>,
    ) -> Self {
        Self::Checked(
            instances
                .into_iter()
                .map(|(declaration, bindings)| Instance::new(declaration, &bindings))
                .collect(),
        )
    }

    fn contains(
        &self,
        declaration: Span,
        bindings: &HashMap<String, ResolvedType>,
    ) -> Option<bool> {
        match self {
            Self::Unprepared => None,
            Self::Checked(instances) => {
                Some(instances.contains(&Instance::new(declaration, bindings)))
            }
        }
    }
}

impl SemanticInfo {
    pub(crate) fn callable_available(
        &self,
        declaration: Span,
        bindings: &HashMap<String, ResolvedType>,
    ) -> Option<bool> {
        if !self
            .contracts
            .callable_conditions
            .contains_key(&declaration)
        {
            return Some(true);
        }
        self.callable_admissions.contains(declaration, bindings)
    }
}

impl Checker<'_> {
    pub(super) fn checked_callable_admissions(&self) -> Admissions {
        if self.contracts.conditions.is_empty() && self.contracts.callable_conditions.is_empty() {
            return Admissions::Unprepared;
        }
        Admissions::checked(
            self.callable_instances
                .iter()
                .filter(|instance| {
                    self.contracts
                        .callable_conditions
                        .contains_key(&instance.declaration)
                })
                .map(|instance| {
                    (
                        instance.declaration,
                        instance
                            .bindings
                            .iter()
                            .map(|(name, ty)| (name.clone(), self.types.resolved(*ty)))
                            .collect(),
                    )
                }),
        )
    }
}

/// Keep authored binding/span identities while applying an effective instance.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct BodyInstance {
    pub(super) declaration: Span,
    pub(super) substitutions: HashMap<String, ResolvedType>,
    receiver: Option<ResolvedType>,
}

impl BodyInstance {
    pub(super) fn new(
        program: CallableBodyProgram<'_>,
        declaration: Span,
        substitutions: HashMap<String, ResolvedType>,
        composition: &crate::trait_composition::CompositionPlan,
    ) -> Self {
        let template = match program {
            CallableBodyProgram::Expanded(_) => None,
            CallableBodyProgram::Authored(_) => composition.callable_templates.get(&declaration),
        };
        match template {
            Some(template) => Self {
                declaration: template.declaration,
                substitutions: template
                    .bindings
                    .iter()
                    .map(|(name, ty)| {
                        (
                            name.clone(),
                            crate::types::substitute_resolved_type(ty, &substitutions),
                        )
                    })
                    .collect(),
                receiver: Some(crate::types::substitute_resolved_type(
                    &template.receiver,
                    &substitutions,
                )),
            },
            None => Self {
                declaration,
                substitutions,
                receiver: None,
            },
        }
    }

    pub(super) fn substitute(&self, ty: &ResolvedType) -> ResolvedType {
        match &self.receiver {
            Some(receiver) => {
                crate::types::substitute_trait_type(ty, &self.substitutions, receiver)
            }
            None => crate::types::substitute_resolved_type(ty, &self.substitutions),
        }
    }

    pub(super) fn substitute_id(&self, ty: TypeId, types: &mut TypeRegistry) -> TypeId {
        types.intern_resolved(&self.substitute(&types.resolved(ty)))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EffectDependency {
    Exact(Span),
    Virtual(Span),
}

#[derive(Debug, Clone, Default)]
pub(super) struct AmbientProfiles {
    pub(super) exact: HashMap<Span, Vec<ResolvedType>>,
    pub(super) virtual_calls: HashMap<Span, Vec<ResolvedType>>,
}

impl AmbientProfiles {
    pub(super) fn effects(&self, dependency: EffectDependency) -> &[ResolvedType] {
        match dependency {
            EffectDependency::Exact(declaration) => self.exact.get(&declaration),
            EffectDependency::Virtual(declaration) => self.virtual_calls.get(&declaration),
        }
        .map(Vec::as_slice)
        .unwrap_or_default()
    }
}

pub(super) struct Seeds {
    effects: AmbientProfiles,
    reach: AmbientProfiles,
    callbacks: HashMap<Span, Vec<ResolvedType>>,
    borrows: HashMap<Span, Option<ReturnBorrow>>,
    specialized_borrows: SpecializedReturnBorrows,
    sources: crate::ownership::SourceTypes,
}

impl Seeds {
    pub(super) fn collect(checker: &mut Checker<'_>, discovery: bool) -> Self {
        let (effects, reach) = if discovery {
            (
                checker.inferred_ambient_effects(),
                checker.inferred_ambient_reach(),
            )
        } else {
            (
                checker.escaping_ambient_effects(),
                checker.escaping_ambient_reach(),
            )
        };
        let borrows = checker.inferred_return_borrows();
        let specialized_borrows = checker.infer_specialized_return_borrows();
        let sources = checker.discover_retained_source_types();
        let fields = checker.property_backing_fields();
        let provenance = checker.prepare_call_provenance(&fields);
        let callbacks =
            checker.callback_ambient_effects(&checker.known_callback_targets(&provenance));
        Self {
            effects,
            reach,
            callbacks,
            borrows,
            specialized_borrows,
            sources,
        }
    }

    pub(super) fn apply(&self, checker: &mut Checker<'_>) {
        checker.ambient_effect_seed = self.effects.clone();
        checker.ambient_reach_seed = self.reach.clone();
        checker.callback_ambient_effect_seed = self.callbacks.clone();
        checker.return_borrow_seed = self.borrows.clone();
        checker.specialized_return_borrow_seed = self.specialized_borrows.clone();
        checker.retained_source_seed = Some(self.sources.clone());
    }

    fn equivalent(&self, other: &Self) -> bool {
        ambient_effect_maps_equal(&self.effects.exact, &other.effects.exact)
            && ambient_effect_maps_equal(&self.effects.virtual_calls, &other.effects.virtual_calls)
            && ambient_effect_maps_equal(&self.reach.exact, &other.reach.exact)
            && ambient_effect_maps_equal(&self.reach.virtual_calls, &other.reach.virtual_calls)
            && ambient_effect_maps_equal(&self.callbacks, &other.callbacks)
            && self.borrows == other.borrows
            && self.specialized_borrows == other.specialized_borrows
            && self.sources == other.sources
    }
}

pub(super) fn refined_checker<'source>(
    mut fresh: impl FnMut() -> Checker<'source>,
    check: impl Fn(&mut Checker<'source>),
) -> Checker<'source> {
    let mut discovery = fresh();
    check(&mut discovery);
    let mut seeds = Seeds::collect(&mut discovery, true);
    let generated_effects = discovery
        .callable_effective_checked_effects
        .iter()
        .filter(|(span, _)| discovery.test_semantics.is_generated_function(**span))
        .map(|(span, effects)| (*span, effects.clone()))
        .collect::<HashMap<_, _>>();

    // Each pass rebuilds callable types from complete loans and effects. Catch
    // coverage narrows the initial over-approximation until all contracts agree.
    loop {
        let mut checker = fresh();
        seeds.apply(&mut checker);
        checker.generated_test_effect_seed = generated_effects.clone();
        check(&mut checker);
        let refined = Seeds::collect(&mut checker, false);
        if seeds.equivalent(&refined) {
            let mut final_checker = fresh();
            refined.apply(&mut final_checker);
            final_checker.generated_test_effect_seed = generated_effects;
            return final_checker;
        }
        seeds = refined;
    }
}

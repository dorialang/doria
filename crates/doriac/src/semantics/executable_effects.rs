//! Executable effect contracts of each callable specialization.
//!
//! A generic or trait body is checked once for every implementation it may
//! select (Decision 0123), and its authored reach decides diagnostics, so they
//! never depend on callers. A specialization's executable transport belongs to
//! what that specialization selects (Decisions 0130 and 0139). This pass replays
//! the checked effect regions of every callable instance with the callees that
//! instance selects, as a least fixpoint over the instance call graph. MIR
//! lowering and PHP emission read its overlays for signatures, catch routing
//! and origin guards. It narrows transport only.

use super::callable_contracts::Instance;
use super::*;
use crate::ast::PropertyHookKind;
use crate::checked_effects::{is_automatic_effect, CatchCoverage, EffectSiteMap};
use crate::types::resolved_type_is_symbolic;
use std::sync::Arc;

/// Where one contribution to a checked effect site comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SiteSource {
    /// The operation raises these itself in every specialization.
    Operation(Vec<ResolvedType>),
    /// A call. Its executable effects are those of the callee each
    /// specialization selects; `effects` is the template's transport.
    Call {
        callee: Callee,
        effects: Vec<ResolvedType>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Callee {
    /// A declaration the check resolved. `owner` is its declaring class as
    /// written in the template; `slot` says MIR dispatches through the virtual
    /// slot rather than the exact body.
    Named {
        declaration: Span,
        owner: Option<ClassType<ResolvedType>>,
        slot: bool,
    },
    /// The implementation of `member` on `receiver`, selected per
    /// specialization. `call` says the author wrote this call, so a generic
    /// method's specialization comes from the instance call graph.
    Selected {
        receiver: ResolvedType,
        member: String,
        call: bool,
    },
    /// The accessor a constrained property use selects.
    Accessor {
        receiver: ResolvedType,
        property: String,
        kind: PropertyHookKind,
    },
    /// A step of the iterator that `iterable::iterator()` returns.
    Iteration {
        iterable: ResolvedType,
        member: String,
    },
}

/// How a specialization finds a contract-selected member again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SelectedUse {
    Call,
    /// A compiler-inserted use: a core operation or `iterator()`.
    Implicit,
    Accessor(PropertyHookKind),
    IterationStep {
        iterable: ResolvedType,
        member: &'static str,
    },
}

/// The provenance of a contract-selected use. An erased call has none: it
/// dispatches over an open implementer set at runtime, in every specialization.
pub(super) fn call_source(
    target: &CallableTarget,
    method: &MethodInfo,
    slot: bool,
    selected: SelectedUse,
) -> Option<Callee> {
    if let SelectedUse::IterationStep { iterable, member } = selected {
        return Some(Callee::Iteration {
            iterable,
            member: member.to_string(),
        });
    }
    match target {
        CallableTarget::Method { class_type, .. } => Some(Callee::Named {
            declaration: method.declaration,
            owner: Some(class_type.clone()),
            slot,
        }),
        CallableTarget::ConstrainedMethod {
            receiver,
            method_name,
            ..
        } => Some(match selected {
            SelectedUse::Accessor(kind) => Callee::Accessor {
                receiver: receiver.clone(),
                property: method_name.clone(),
                kind,
            },
            SelectedUse::Implicit => Callee::Selected {
                receiver: receiver.clone(),
                member: method_name.clone(),
                call: false,
            },
            SelectedUse::Call | SelectedUse::IterationStep { .. } => Callee::Selected {
                receiver: receiver.clone(),
                member: method_name.clone(),
                call: true,
            },
        }),
        CallableTarget::InterfaceMethod { .. } | CallableTarget::Function { .. } => None,
    }
}

/// Merge a re-checked body's contribution into what the site already records.
pub(super) fn merge_site_source(sources: &mut Vec<SiteSource>, source: SiteSource) {
    match source {
        SiteSource::Operation(effects) => {
            for existing in sources.iter_mut() {
                if let SiteSource::Operation(recorded) = existing {
                    extend(recorded, effects);
                    return;
                }
            }
            sources.push(SiteSource::Operation(effects));
        }
        SiteSource::Call { callee, effects } => {
            for existing in sources.iter_mut() {
                if let SiteSource::Call {
                    callee: recorded,
                    effects: recorded_effects,
                } = existing
                {
                    if *recorded == callee {
                        extend(recorded_effects, effects);
                        return;
                    }
                }
            }
            sources.push(SiteSource::Call { callee, effects });
        }
    }
}

/// An effect scope of a checked body.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Region {
    /// Effects the scope starts with: a constructor's property initializers,
    /// the entry source's top-level statements.
    pub(super) seed: Vec<ResolvedType>,
    pub(super) members: Vec<Member>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Member {
    Site(Span),
    Try(Span),
    Finalizer(Span),
    /// A closure body: its own root, contributing nothing to this scope.
    Closure(Span),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TryRegion {
    pub(super) body: Region,
    pub(super) catches: Vec<CatchRegion>,
    pub(super) finally: Option<Region>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CatchRegion {
    pub(super) span: Span,
    pub(super) catches_all: bool,
    pub(super) body: Region,
}

/// Region structure recorded while bodies are checked.
#[derive(Debug, Clone, Default)]
pub(super) struct Regions {
    pub(super) roots: HashMap<Span, Region>,
    pub(super) tries: HashMap<Span, TryRegion>,
    pub(super) finalizers: HashMap<Span, Region>,
}

/// One specialization's executable view. Absent entries equal the template.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct InstanceEffects {
    contract: Option<Vec<ResolvedType>>,
    sites: HashMap<Span, Vec<ResolvedType>>,
    tries: HashMap<Span, Vec<ResolvedType>>,
    unreachable_catches: HashSet<Span>,
}

impl InstanceEffects {
    /// The complete effect profile this specialization's ABI carries, or
    /// `None` when that is the template's.
    pub(crate) fn contract(&self) -> Option<&[ResolvedType]> {
        self.contract.as_deref()
    }

    /// The effects an operation can raise here, or `None` for the template's.
    pub(crate) fn site(&self, span: Span) -> Option<&[ResolvedType]> {
        self.sites.get(&span).map(Vec::as_slice)
    }

    /// What escapes a `try` statement's catches, or `None` for the template's.
    pub(crate) fn try_uncovered(&self, span: Span) -> Option<&[ResolvedType]> {
        self.tries.get(&span).map(Vec::as_slice)
    }

    /// Whether a catch can receive an Error in this specialization.
    pub(crate) fn catch_reachable(&self, span: Span) -> bool {
        !self.unreachable_catches.contains(&span)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ExecutableEffects {
    instances: HashMap<Instance, Arc<InstanceEffects>>,
}

impl ExecutableEffects {
    pub(crate) fn instance(
        &self,
        declaration: Span,
        substitutions: &HashMap<String, ResolvedType>,
    ) -> Option<&Arc<InstanceEffects>> {
        self.instances
            .get(&Instance::new(declaration, substitutions))
    }
}

/// One site's contributions, resolved once for a specialization.
#[derive(Debug, Clone, Default)]
struct SitePlan {
    /// Effects no specialization changes: required effects and every automatic
    /// effect not explained by a resolved call.
    fixed: Vec<ResolvedType>,
    /// Callee instances whose executable automatic effects reach this site.
    callees: Vec<Instance>,
}

#[derive(Debug, Clone, Default)]
struct Selection {
    callees: Vec<Instance>,
    fixed: Vec<ResolvedType>,
}

struct InstancePlan {
    instance: Instance,
    /// The body's own declaration, which keys its regions.
    root: Span,
    sites: HashMap<Span, SitePlan>,
}

/// What selection needs that does not change between sites.
struct SelectionContext<'a> {
    caller: &'a Instance,
    instance: &'a SemanticCallableInstance,
    body: &'a callable_contracts::BodyInstance,
    /// Declarations MIR reaches through a virtual slot unless a call is exact.
    virtual_methods: &'a HashSet<Span>,
}

/// Evaluation of one instance's regions against the current fixpoint.
struct Replay<'a> {
    regions: &'a Regions,
    sites: &'a HashMap<Span, SitePlan>,
    automatic: &'a HashMap<Instance, Vec<ResolvedType>>,
    coverage: &'a crate::checked_effects::CatchCoverageMap,
    template: &'a EffectSiteMap,
    record: bool,
    site_effects: HashMap<Span, Vec<ResolvedType>>,
    try_effects: HashMap<Span, Vec<ResolvedType>>,
    unreachable_catches: HashSet<Span>,
}

impl<'a> Replay<'a> {
    fn new(
        regions: &'a Regions,
        sites: &'a HashMap<Span, SitePlan>,
        automatic: &'a HashMap<Instance, Vec<ResolvedType>>,
        coverage: &'a crate::checked_effects::CatchCoverageMap,
        template: &'a EffectSiteMap,
        record: bool,
    ) -> Self {
        Self {
            regions,
            sites,
            automatic,
            coverage,
            template,
            record,
            site_effects: HashMap::new(),
            try_effects: HashMap::new(),
            unreachable_catches: HashSet::new(),
        }
    }

    fn template_site(&self, span: Span) -> Vec<ResolvedType> {
        crate::checked_effects::effects_at(self.template, span).to_vec()
    }

    fn region(&mut self, region: &Region) -> Vec<ResolvedType> {
        let mut effects = region.seed.clone();
        for member in &region.members {
            match *member {
                Member::Site(span) => extend(&mut effects, self.site(span)),
                Member::Try(span) => extend(&mut effects, self.try_statement(span)),
                Member::Finalizer(span) => {
                    let regions = self.regions;
                    let escaped = match regions.finalizers.get(&span) {
                        Some(region) => self.region(region),
                        None => self.template_site(span),
                    };
                    if self.record {
                        self.site_effects.insert(span, escaped.clone());
                    }
                    extend(&mut effects, escaped);
                }
                Member::Closure(span) => {
                    let regions = self.regions;
                    if let Some(root) = regions.roots.get(&span) {
                        let _ = self.region(root);
                    }
                }
            }
        }
        effects
    }

    fn site(&mut self, span: Span) -> Vec<ResolvedType> {
        let Some(plan) = self.sites.get(&span) else {
            return self.template_site(span);
        };
        let mut effects = plan.fixed.clone();
        for callee in &plan.callees {
            extend(
                &mut effects,
                self.automatic.get(callee).into_iter().flatten().cloned(),
            );
        }
        if self.record {
            self.site_effects.insert(span, effects.clone());
        }
        effects
    }

    fn try_statement(&mut self, span: Span) -> Vec<ResolvedType> {
        let regions = self.regions;
        let Some(statement) = regions.tries.get(&span) else {
            return self.template_site(span);
        };
        let protected = self.region(&statement.body);
        let mut uncovered = protected.clone();
        let mut escaped = Vec::new();
        for catch in &statement.catches {
            let recorded = self.coverage.get(&catch.span);
            let coverage = |effect: &ResolvedType| {
                if catch.catches_all {
                    return Some(CatchCoverage::Complete);
                }
                recorded.and_then(|coverage| coverage.get(effect)).copied()
            };
            // An effect the template never protected here has no recorded
            // coverage: the catch stays reachable and the effect uncovered.
            let reachable = protected
                .iter()
                .any(|effect| coverage(effect) != Some(CatchCoverage::None));
            uncovered.retain(|effect| coverage(effect) != Some(CatchCoverage::Complete));
            if reachable {
                extend(&mut escaped, self.region(&catch.body));
            } else if self.record {
                self.unreachable_catches.insert(catch.span);
            }
        }
        if self.record {
            self.try_effects.insert(span, uncovered.clone());
        }
        extend(&mut escaped, uncovered);
        if let Some(finally) = &statement.finally {
            extend(&mut escaped, self.region(finally));
        }
        if self.record {
            self.site_effects.insert(span, escaped.clone());
        }
        escaped
    }
}

fn extend(target: &mut Vec<ResolvedType>, effects: impl IntoIterator<Item = ResolvedType>) {
    for effect in effects {
        if !target.contains(&effect) {
            target.push(effect);
        }
    }
}

fn automatic(effects: &[ResolvedType]) -> impl Iterator<Item = ResolvedType> + '_ {
    effects
        .iter()
        .filter(|effect| is_automatic_effect(effect))
        .cloned()
}

fn same_set(left: &[ResolvedType], right: &[ResolvedType]) -> bool {
    left.len() == right.len() && left.iter().all(|effect| right.contains(effect))
}

fn collect_region_sites(regions: &Regions, region: &Region, spans: &mut Vec<Span>) {
    for member in &region.members {
        match *member {
            Member::Site(span) => spans.push(span),
            Member::Try(span) => {
                if let Some(statement) = regions.tries.get(&span) {
                    collect_region_sites(regions, &statement.body, spans);
                    for catch in &statement.catches {
                        collect_region_sites(regions, &catch.body, spans);
                    }
                    if let Some(finally) = &statement.finally {
                        collect_region_sites(regions, finally, spans);
                    }
                }
            }
            Member::Finalizer(span) => {
                if let Some(region) = regions.finalizers.get(&span) {
                    collect_region_sites(regions, region, spans);
                }
            }
            Member::Closure(span) => {
                if let Some(region) = regions.roots.get(&span) {
                    collect_region_sites(regions, region, spans);
                }
            }
        }
    }
}

impl Checker<'_> {
    /// Each specialization's executable effect view.
    pub(super) fn collect_executable_effects(&mut self) -> ExecutableEffects {
        if self
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == crate::diagnostics::DiagnosticSeverity::Error)
        {
            return ExecutableEffects::default();
        }
        let virtual_methods = self
            .classes
            .values()
            .flat_map(|class| class.methods.values())
            .filter(|method| method.virtual_root.is_some())
            .map(|method| method.declaration)
            .collect::<HashSet<_>>();

        // Every instance is known before any call is resolved against it. An
        // instance without a checked body (an intrinsic or a requirement) has
        // its declared contract as its executable one.
        let mut automatic_effects = HashMap::<Instance, Vec<ResolvedType>>::new();
        let mut planned = Vec::new();
        for instance in self.callable_instances.clone() {
            let resolved = instance.resolved(&self.types);
            let substitutions = instance
                .bindings
                .iter()
                .map(|(name, ty)| (name.clone(), self.types.resolved(*ty)))
                .collect::<HashMap<_, _>>();
            let body = callable_contracts::BodyInstance::new(
                self.callable_body_program,
                instance.declaration,
                substitutions,
                &self.composition,
            );
            if self.effect_regions.roots.contains_key(&body.declaration) {
                // A declared automatic entry is authored contract: the ABI
                // carries it, so callers must see it whatever the body does.
                let declared = self.declared_automatic_effects(instance.declaration);
                automatic_effects.insert(resolved.clone(), declared);
                planned.push((instance, resolved, body));
            } else {
                automatic_effects.insert(
                    resolved,
                    self.callable_effective_checked_effects
                        .get(&instance.declaration)
                        .map(|effects| automatic(effects).collect())
                        .unwrap_or_default(),
                );
            }
        }

        let mut plans = Vec::with_capacity(planned.len());
        for (instance, resolved, body) in &planned {
            let mut spans = Vec::new();
            collect_region_sites(
                &self.effect_regions,
                &self.effect_regions.roots[&body.declaration],
                &mut spans,
            );
            let context = SelectionContext {
                caller: resolved,
                instance,
                body,
                virtual_methods: &virtual_methods,
            };
            let mut sites = HashMap::with_capacity(spans.len());
            for span in spans {
                let plan = self.site_plan(&context, span, &automatic_effects);
                sites.insert(span, plan);
            }
            plans.push(InstancePlan {
                instance: resolved.clone(),
                root: body.declaration,
                sites,
            });
        }

        // Least fixpoint: every instance starts with only what it declares and
        // grows monotonically through what it selects, so recursion that never
        // reaches an operation stays empty.
        loop {
            let mut changed = false;
            for plan in &plans {
                let escaped = Replay::new(
                    &self.effect_regions,
                    &plan.sites,
                    &automatic_effects,
                    &self.catch_coverage,
                    &self.checked_effect_sites,
                    false,
                )
                .region(&self.effect_regions.roots[&plan.root])
                .into_iter()
                .filter(is_automatic_effect)
                .collect::<Vec<_>>();
                let current = automatic_effects
                    .get_mut(&plan.instance)
                    .expect("planned instance has an automatic set");
                let before = current.len();
                extend(current, escaped);
                changed |= current.len() != before;
            }
            if !changed {
                break;
            }
        }

        let mut instances = HashMap::with_capacity(plans.len());
        for plan in &plans {
            let declaration = plan.instance.declaration();
            let mut replay = Replay::new(
                &self.effect_regions,
                &plan.sites,
                &automatic_effects,
                &self.catch_coverage,
                &self.checked_effect_sites,
                true,
            );
            let _ = replay.region(&self.effect_regions.roots[&plan.root]);
            // A composed copy checked through its authored template reports
            // spans in the template; the backend lowers the copy's own.
            let key = |span: Span| {
                if plan.root.expansion != declaration.expansion {
                    span.in_expansion(declaration.expansion)
                } else {
                    span
                }
            };
            let mut effects = InstanceEffects {
                contract: self.executable_contract(declaration, &automatic_effects[&plan.instance]),
                unreachable_catches: replay
                    .unreachable_catches
                    .iter()
                    .copied()
                    .map(key)
                    .collect(),
                ..InstanceEffects::default()
            };
            for (span, site) in replay.site_effects {
                let template = crate::checked_effects::effects_at(&self.checked_effect_sites, span);
                if !same_set(template, &site) {
                    effects.sites.insert(key(span), site);
                }
            }
            for (span, uncovered) in replay.try_effects {
                let template = self
                    .try_uncovered_effects
                    .get(&span)
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                if !same_set(template, &uncovered) {
                    effects.tries.insert(key(span), uncovered);
                }
            }
            instances.insert(plan.instance.clone(), Arc::new(effects));
        }
        ExecutableEffects { instances }
    }

    /// The template contract with its automatic entries narrowed to `escaped`,
    /// which holds what this specialization's body lets out and what the author
    /// declared. Required entries are authored contract and stay.
    fn executable_contract(
        &self,
        declaration: Span,
        escaped: &[ResolvedType],
    ) -> Option<Vec<ResolvedType>> {
        let template = self
            .callable_effective_checked_effects
            .get(&declaration)
            .cloned()
            .unwrap_or_default();
        let mut contract = template
            .iter()
            .filter(|effect| !is_automatic_effect(effect) || escaped.contains(effect))
            .cloned()
            .collect::<Vec<_>>();
        extend(&mut contract, escaped.iter().cloned());
        (contract != template).then_some(contract)
    }

    /// Automatic entries the author wrote in a `throws` clause, and the ones a
    /// generated test declares through its seed.
    fn declared_automatic_effects(&self, declaration: Span) -> Vec<ResolvedType> {
        let mut declared = Vec::new();
        for effects in self
            .callable_declared_checked_effects
            .get(&declaration)
            .into_iter()
            .chain(self.generated_test_effect_seed.get(&declaration))
        {
            extend(&mut declared, automatic(effects));
        }
        declared
    }

    fn site_plan(
        &mut self,
        context: &SelectionContext<'_>,
        span: Span,
        known: &HashMap<Instance, Vec<ResolvedType>>,
    ) -> SitePlan {
        let template =
            crate::checked_effects::effects_at(&self.checked_effect_sites, span).to_vec();
        let mut plan = SitePlan {
            fixed: template
                .iter()
                .filter(|effect| !is_automatic_effect(effect))
                .cloned()
                .collect(),
            callees: Vec::new(),
        };
        let mut explained = Vec::new();
        let sources = self
            .effect_site_sources
            .get(&span)
            .cloned()
            .unwrap_or_default();
        for source in sources {
            match source {
                SiteSource::Operation(effects) => {
                    extend(&mut plan.fixed, automatic(&effects));
                    extend(&mut explained, automatic(&effects));
                }
                SiteSource::Call { callee, effects } => {
                    extend(&mut explained, automatic(&effects));
                    let selection =
                        self.select_callees(context, span, &callee)
                            .filter(|selection| {
                                selection
                                    .callees
                                    .iter()
                                    .all(|callee| known.contains_key(callee))
                            });
                    match selection {
                        Some(selection) => {
                            extend(&mut plan.fixed, automatic(&selection.fixed));
                            for callee in selection.callees {
                                if !plan.callees.contains(&callee) {
                                    plan.callees.push(callee);
                                }
                            }
                        }
                        None => extend(&mut plan.fixed, automatic(&effects)),
                    }
                }
            }
        }
        // A record that bypassed provenance stays exactly as checked.
        extend(
            &mut plan.fixed,
            automatic(&template).filter(|effect| !explained.contains(effect)),
        );
        plan
    }

    fn select_callees(
        &mut self,
        context: &SelectionContext<'_>,
        span: Span,
        callee: &Callee,
    ) -> Option<Selection> {
        let generic_call = self.generic_call_specializations.contains_key(&span);
        match callee {
            Callee::Named {
                declaration,
                owner,
                slot,
            } => {
                if generic_call {
                    return self.graph_selection(context, span);
                }
                let bindings = match owner {
                    Some(owner) if owner.arguments.is_empty() => HashMap::new(),
                    Some(owner) => {
                        let ResolvedType::Class(owner) =
                            context.body.substitute(&ResolvedType::Class(owner.clone()))
                        else {
                            return None;
                        };
                        self.owner_bindings(&owner)?
                    }
                    None => HashMap::new(),
                };
                let mut selection = Selection::default();
                if *slot {
                    self.add_slot_profile(&mut selection, *declaration);
                }
                selection
                    .callees
                    .push(Instance::new(*declaration, &bindings));
                Some(selection)
            }
            Callee::Selected {
                receiver,
                member,
                call,
            } => {
                if *call && generic_call {
                    return self.graph_selection(context, span);
                }
                self.select_member(context.body.substitute(receiver), member, span)
            }
            Callee::Accessor {
                receiver,
                property,
                kind,
            } => {
                let receiver = context.body.substitute(receiver);
                if resolved_type_is_symbolic(&receiver) {
                    return None;
                }
                let receiver = self.types.intern_resolved(&receiver);
                let TypeKind::Class(class) = self.types.kind(receiver).clone() else {
                    return None;
                };
                let (declaring, info) = self.lookup_instance_property(&class, property)?;
                let method = self.property_accessor_method(&info, &declaring, *kind)?;
                self.method_selection(&declaring, &method, span)
            }
            Callee::Iteration { iterable, member } => {
                let iterable = context.body.substitute(iterable);
                if resolved_type_is_symbolic(&iterable) {
                    return None;
                }
                let iterable = self.types.intern_resolved(&iterable);
                let TypeKind::Class(class) = self.types.kind(iterable).clone() else {
                    return None;
                };
                let (_, method) = self.lookup_instance_method(&class, "iterator")?;
                let iterator = self.types.resolved(method.return_ty);
                self.select_member(iterator, member, span)
            }
        }
    }

    /// The instance collector's own selection for a generic call.
    fn graph_selection(&self, context: &SelectionContext<'_>, span: Span) -> Option<Selection> {
        let site = if context.body.declaration.expansion != context.instance.declaration.expansion {
            span.in_expansion(context.instance.declaration.expansion)
        } else {
            span
        };
        let calls = self
            .generic_call_graph
            .calls(Some(context.caller.clone()), site)
            .ok()?;
        let call = calls.iter().find(|call| call.site == site)?;
        if call.interface.is_some() || call.targets.is_empty() {
            return None;
        }
        let mut selection = Selection::default();
        for target in &call.targets {
            let declaration = target.declaration();
            if context.virtual_methods.contains(&declaration)
                && !self.exact_method_calls.contains(&(span, declaration))
            {
                self.add_slot_profile(&mut selection, declaration);
            }
            selection.callees.push(target.clone());
        }
        Some(selection)
    }

    /// What a member use selects on a concrete receiver.
    fn select_member(
        &mut self,
        receiver: ResolvedType,
        member: &str,
        span: Span,
    ) -> Option<Selection> {
        if matches!(
            receiver,
            ResolvedType::Integer(_)
                | ResolvedType::Float(_)
                | ResolvedType::Bool
                | ResolvedType::String
        ) {
            // Primitive core operations are intrinsics with no checked Error.
            return Some(Selection::default());
        }
        if resolved_type_is_symbolic(&receiver) {
            return None;
        }
        let receiver = self.types.intern_resolved(&receiver);
        let TypeKind::Class(class) = self.types.kind(receiver).clone() else {
            return None;
        };
        let (declaring, method) = self.lookup_instance_method(&class, member)?;
        self.method_selection(&declaring, &method, span)
    }

    /// A concrete method as its callable instance. MIR reaches a method with a
    /// virtual root through its slot unless the call is proven exact.
    fn method_selection(
        &self,
        declaring: &ClassType<TypeId>,
        method: &MethodInfo,
        span: Span,
    ) -> Option<Selection> {
        if !method.type_params.is_empty() {
            // Generic methods are selected by the instance call graph.
            return None;
        }
        let bindings = self
            .class_type_substitutions(declaring)
            .into_iter()
            .map(|(name, ty)| (name, self.types.resolved(ty)))
            .collect::<HashMap<_, _>>();
        let mut selection = Selection::default();
        if method.virtual_root.is_some()
            && !self
                .exact_method_calls
                .contains(&(span, method.declaration))
        {
            self.add_slot_profile(&mut selection, method.declaration);
        }
        selection
            .callees
            .push(Instance::new(method.declaration, &bindings));
        Some(selection)
    }

    /// A concrete declaring class's type-parameter bindings.
    fn owner_bindings(
        &mut self,
        owner: &ClassType<ResolvedType>,
    ) -> Option<HashMap<String, ResolvedType>> {
        let ty = ResolvedType::Class(owner.clone());
        if resolved_type_is_symbolic(&ty) {
            return None;
        }
        let ty = self.types.intern_resolved(&ty);
        let TypeKind::Class(class) = self.types.kind(ty).clone() else {
            return None;
        };
        Some(
            self.class_type_substitutions(&class)
                .into_iter()
                .map(|(name, ty)| (name, self.types.resolved(ty)))
                .collect(),
        )
    }

    fn add_slot_profile(&self, selection: &mut Selection, declaration: Span) {
        if let Some(slot) = self.ambient_effect_seed.virtual_calls.get(&declaration) {
            extend(&mut selection.fixed, slot.iter().cloned());
        }
    }
}

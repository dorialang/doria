//! Declaration-owned dependencies become executable only under their guards.

use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum NominalInstance {
    Class(ClassType<TypeId>),
    Enum(EnumType<TypeId>),
}

pub(super) trait Application {
    fn name(&self) -> &str;
    fn arguments(&self) -> &[TypeId];
}

impl Application for ClassType<TypeId> {
    fn name(&self) -> &str {
        &self.name
    }
    fn arguments(&self) -> &[TypeId] {
        &self.arguments
    }
}

impl Application for NominalInstance {
    fn name(&self) -> &str {
        match self {
            Self::Class(class) => &class.name,
            Self::Enum(enumeration) => &enumeration.name,
        }
    }

    fn arguments(&self) -> &[TypeId] {
        match self {
            Self::Class(class) => &class.arguments,
            Self::Enum(enumeration) => &enumeration.arguments,
        }
    }
}

impl NominalInstance {
    pub(super) fn resolved(&self, types: &TypeRegistry) -> ResolvedType {
        match self {
            Self::Class(class) => ResolvedType::Class(ClassType::new(
                &class.name,
                class
                    .arguments
                    .iter()
                    .map(|argument| types.resolved(*argument))
                    .collect(),
            )),
            Self::Enum(enumeration) => {
                ResolvedType::Enum(enumeration.map_arguments(|argument| types.resolved(*argument)))
            }
        }
    }

    pub(super) fn substitute(
        &self,
        bindings: &HashMap<String, TypeId>,
        checker: &mut Checker<'_>,
    ) -> Self {
        self.map_arguments(|argument| checker.substitute_type_id(*argument, bindings))
    }

    pub(super) fn map_arguments(&self, mut map: impl FnMut(&TypeId) -> TypeId) -> Self {
        match self {
            Self::Class(class) => Self::Class(ClassType::new(
                &class.name,
                class.arguments.iter().map(map).collect(),
            )),
            Self::Enum(enumeration) => Self::Enum(enumeration.map_arguments(&mut map)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct NominalDependency {
    pub(super) instance: NominalInstance,
    pub(super) condition: Option<Span>,
}

impl Checker<'_> {
    pub(super) fn record_class_instantiation(
        &mut self,
        class: ClassType<TypeId>,
        declaring_class: Option<&str>,
    ) {
        self.record_nominal_instantiation(NominalInstance::Class(class), declaring_class);
    }

    pub(super) fn record_nominal_instantiation(
        &mut self,
        instance: NominalInstance,
        declaring_class: Option<&str>,
    ) {
        if self.nominal_identity_depth != 0 {
            return;
        }
        if let Some(callable) = self.current_callable {
            self.callable_nominal_instantiation_templates
                .entry(callable)
                .or_default()
                .insert(instance);
            return;
        }
        let condition = self.instantiation_condition;
        let symbolic = instance
            .arguments()
            .iter()
            .any(|argument| self.type_is_symbolic(*argument));
        if symbolic || condition.is_some() || self.instantiation_declaration.is_some() {
            if let Some(owner) = self
                .instantiation_declaration
                .as_deref()
                .or(declaring_class)
            {
                self.nominal_instantiation_templates
                    .entry(owner.to_owned())
                    .or_default()
                    .insert(NominalDependency {
                        instance,
                        condition,
                    });
            }
        } else {
            match instance {
                NominalInstance::Class(class) => {
                    self.class_instantiations.insert(class);
                }
                NominalInstance::Enum(enumeration) => {
                    self.enum_instantiations.insert(enumeration);
                }
            }
        }
    }

    pub(super) fn instantiation_condition_available(
        &mut self,
        declaration: Span,
        bindings: &HashMap<String, TypeId>,
    ) -> bool {
        let Some(condition) = self.contracts.conditions.get(&declaration).cloned() else {
            return false;
        };
        let Ok(steps) = self.condition_queries(&condition, bindings) else {
            return false;
        };
        let mut context = nominal_proof::Context::default();
        steps.into_iter().all(|step| {
            self.prove_nominal(step.query, &mut context, step.origin, &[])
                .is_ok()
        })
    }

    pub(super) fn callable_instance_available(
        &mut self,
        instance: &SemanticCallableInstance,
    ) -> bool {
        self.contracts
            .callable_conditions
            .get(&instance.declaration)
            .copied()
            .is_none_or(|condition| {
                self.instantiation_condition_available(condition, &instance.substitutions())
            })
    }

    pub(super) fn materialize_nominal_dependency(
        &mut self,
        parent: Option<&NominalInstance>,
        instance: NominalInstance,
        span: Span,
    ) -> bool {
        if instance
            .arguments()
            .iter()
            .any(|argument| self.type_is_symbolic(*argument))
        {
            return false;
        }
        let present = match &instance {
            NominalInstance::Class(class) => self.class_instantiations.contains(class),
            NominalInstance::Enum(enumeration) => self.enum_instances.contains_key(enumeration),
        };
        if present {
            return false;
        }
        if let Some(parent) = parent {
            if let Some(cycle) = expanding_specialization_cycle(
                &self.nominal_instantiation_parents,
                parent,
                &instance,
                &self.types,
            ) {
                self.report_expanding_nominal_specialization(&cycle);
                return false;
            }
        }
        match &instance {
            NominalInstance::Class(class) => {
                self.check_concrete_class_constraints(class, span);
                self.class_instantiations.insert(class.clone());
            }
            NominalInstance::Enum(enumeration) => {
                let Some(mut definition) = self.enums.get(&enumeration.name).cloned() else {
                    return false;
                };
                if definition.source_type.id != enumeration.id
                    || definition.type_params.len() != enumeration.arguments.len()
                    || !self.check_nominal_type_constraints(
                        &enumeration.name,
                        &definition.type_params,
                        &enumeration.arguments,
                        span,
                    )
                {
                    return false;
                }
                let bindings = definition
                    .type_params
                    .iter()
                    .zip(&enumeration.arguments)
                    .map(|(parameter, argument)| (parameter.name.clone(), *argument))
                    .collect();
                definition.id = EnumId(self.enum_instances.len());
                definition.source_type = enumeration.clone();
                let ty = self.types.intern(TypeKind::Enum(enumeration.clone()));
                definition.name = self.types.display(ty);
                for case in &mut definition.cases {
                    case.id.enum_id = definition.id;
                    for field in &mut case.payload {
                        field.ty = self.substitute_type_id(field.ty, &bindings);
                    }
                }
                definition.layout = None;
                self.enum_instantiations.insert(enumeration.clone());
                self.enum_instances.insert(enumeration.clone(), definition);
                self.enum_metadata_dirty = true;
            }
        }
        if let Some(parent) = parent {
            self.nominal_instantiation_parents
                .insert(instance, parent.clone());
        }
        true
    }
}

pub(super) fn expanding_specialization_cycle<T: Application + Eq + std::hash::Hash>(
    parents: &HashMap<T, T>,
    current: &T,
    target: &T,
    types: &TypeRegistry,
) -> Option<Vec<String>> {
    let mut matching_ancestors = Vec::new();
    let mut cursor = Some(current);
    while let Some(instance) = cursor {
        if instance.name() == target.name() {
            matching_ancestors.push(instance);
            if matching_ancestors.len() == 2 {
                break;
            }
        }
        cursor = parents.get(instance);
    }
    let [nearest, previous] = matching_ancestors.as_slice() else {
        return None;
    };
    let complexity = |instance: &T| {
        instance
            .arguments()
            .iter()
            .map(|argument| resolved_type_complexity(&types.resolved(*argument)))
            .sum::<usize>()
    };
    if complexity(target) <= complexity(nearest) || complexity(nearest) <= complexity(previous) {
        return None;
    }
    let mut cycle = vec![target.name().to_owned()];
    let mut cursor = Some(current);
    while let Some(instance) = cursor {
        cycle.push(instance.name().to_owned());
        if instance.name() == target.name() {
            break;
        }
        cursor = parents.get(instance);
    }
    Some(cycle)
}

pub(super) fn expand_nominal_instantiations(
    checker: &mut Checker<'_>,
    expanded: &mut HashSet<NominalInstance>,
) {
    let mut instances = checker
        .class_instantiations
        .iter()
        .cloned()
        .map(NominalInstance::Class)
        .chain(
            checker
                .enum_instantiations
                .iter()
                .cloned()
                .map(NominalInstance::Enum),
        )
        .filter(|instance| {
            !expanded.contains(instance)
                && !instance
                    .arguments()
                    .iter()
                    .any(|argument| checker.type_is_symbolic(*argument))
        })
        .collect::<Vec<_>>();
    instances.sort_by_key(|instance| instance.resolved(&checker.types));
    let mut cursor = 0;
    while cursor < instances.len() {
        let instance = instances[cursor].clone();
        cursor += 1;
        if !expanded.insert(instance.clone()) {
            continue;
        }
        let (span, bindings, stored) = match &instance {
            NominalInstance::Class(class) => {
                let span = checker
                    .classes
                    .get(&class.name)
                    .map(|class| class.declaration)
                    .unwrap_or_default();
                checker.check_specialized_class_shared_payloads(class, span);
                (
                    span,
                    checker.class_type_substitutions(class),
                    checker.specialized_stored_types(class),
                )
            }
            NominalInstance::Enum(enumeration) => {
                let Some(declaration) = checker.enums.get(&enumeration.name) else {
                    continue;
                };
                let span = declaration.span;
                let bindings = declaration
                    .type_params
                    .iter()
                    .zip(&enumeration.arguments)
                    .map(|(parameter, argument)| (parameter.name.clone(), *argument))
                    .collect();
                checker.materialize_nominal_dependency(None, instance.clone(), span);
                let Some(definition) = checker.enum_instances.get(enumeration) else {
                    continue;
                };
                (
                    span,
                    bindings,
                    definition
                        .cases
                        .iter()
                        .flat_map(|case| case.payload.iter().map(|field| field.ty))
                        .collect(),
                )
            }
        };
        let templates = checker
            .nominal_instantiation_templates
            .get(instance.name())
            .cloned()
            .unwrap_or_default();
        let mut dependencies = Vec::new();
        for template in templates {
            if template.condition.is_some_and(|condition| {
                !checker.instantiation_condition_available(condition, &bindings)
            }) {
                continue;
            }
            dependencies.push(template.instance.substitute(&bindings, checker));
        }
        let mut visited = HashSet::new();
        for ty in stored
            .into_iter()
            .chain(instance.arguments().iter().copied())
        {
            checker.formed_nominal_types(ty, &mut visited, &mut dependencies);
        }
        dependencies.sort_by_key(|dependency| dependency.resolved(&checker.types));
        dependencies.dedup();
        for dependency in dependencies {
            if checker.materialize_nominal_dependency(Some(&instance), dependency.clone(), span) {
                instances.push(dependency);
            }
        }
    }
}

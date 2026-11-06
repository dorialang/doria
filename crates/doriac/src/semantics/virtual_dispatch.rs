//! Canonical physical slot families, independent of executable body admission.

use super::*;
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct Family {
    pub declaration: Span,
    pub owner: ClassType<ResolvedType>,
    pub arguments: Vec<ResolvedType>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Slot {
    pub family: Family,
    pub receiver_mode: ReceiverMode,
    pub implementation: CallableInstanceKey,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum Plan {
    #[default]
    Unprepared,
    Checked {
        classes: HashMap<ClassType<ResolvedType>, Vec<Slot>>,
        instances: HashMap<CallableInstanceKey, Family>,
    },
}

impl Plan {
    pub(crate) fn slots(
        &self,
        receiver: &ClassType<ResolvedType>,
        site: Span,
    ) -> DiagnosticResult<&[Slot]> {
        let Self::Checked { classes, .. } = self else {
            return Err(failure(site, "checked virtual-slot plan is unavailable"));
        };
        classes
            .get(receiver)
            .map(Vec::as_slice)
            .ok_or_else(|| failure(site, "checked class has no canonical virtual-slot plan"))
    }

    pub(crate) fn family(
        &self,
        instance: &CallableInstanceKey,
        site: Span,
    ) -> DiagnosticResult<&Family> {
        let Self::Checked { instances, .. } = self else {
            return Err(failure(site, "checked virtual-slot plan is unavailable"));
        };
        instances
            .get(instance)
            .ok_or_else(|| failure(site, "checked virtual method has no canonical family"))
    }
}

fn failure(site: Span, message: &str) -> Vec<Diagnostic> {
    vec![Diagnostic::new("I2401", message, site)]
}

pub(super) fn collect(
    declarations: &callable_contracts::MemberDeclarations<'_>,
    checker: &Checker<'_>,
    classes: &[ClassSemanticInfo],
    methods: &HashMap<Span, MethodHierarchySemanticInfo>,
) -> DiagnosticResult<Plan> {
    let mut families = BTreeSet::new();
    for method in methods.values() {
        if method.virtual_root != Some(method.declaration) {
            continue;
        }
        let (declaration, _) = declarations.get(&method.declaration).ok_or_else(|| {
            failure(
                method.declaration,
                "virtual root has no checked declaration",
            )
        })?;
        if !declaration.type_params.is_empty() {
            continue;
        }
        for class in classes
            .iter()
            .filter(|class| class.declaration_name == method.declaring_class)
        {
            families.insert(Family {
                declaration: method.declaration,
                owner: ClassType::new(class.declaration_name.clone(), class.arguments.clone()),
                arguments: Vec::new(),
            });
        }
    }
    for instance in &checker.callable_instances {
        let Some(method) = methods.get(&instance.declaration) else {
            continue;
        };
        let Some(root) = method.virtual_root else {
            continue;
        };
        let root_method = methods
            .get(&root)
            .ok_or_else(|| failure(root, "virtual method has no checked root"))?;
        let (declaration, owner) = declarations.get(&instance.declaration).ok_or_else(|| {
            failure(
                instance.declaration,
                "virtual method has no checked declaration",
            )
        })?;
        let bindings = instance.substitutions();
        let resolve = |name: &str| {
            bindings
                .get(name)
                .map(|ty| checker.types.resolved(*ty))
                .ok_or_else(|| {
                    failure(
                        instance.declaration,
                        "virtual method has incomplete checked bindings",
                    )
                })
        };
        let owner_arguments = owner
            .type_params
            .iter()
            .map(|parameter| resolve(&parameter.name))
            .collect::<DiagnosticResult<Vec<_>>>()?;
        let owner = ClassType::new(owner.name.clone(), owner_arguments);
        let class = classes
            .iter()
            .find(|class| {
                class.declaration_name == owner.name && class.arguments == owner.arguments
            })
            .ok_or_else(|| failure(instance.declaration, "virtual method has no concrete owner"))?;
        let root_owner = std::iter::once(&owner)
            .chain(&class.ancestors)
            .find(|owner| owner.name == root_method.declaring_class)
            .ok_or_else(|| failure(root, "virtual root is outside its checked hierarchy"))?;
        families.insert(Family {
            declaration: root,
            owner: root_owner.clone(),
            arguments: declaration
                .type_params
                .iter()
                .map(|parameter| resolve(&parameter.name))
                .collect::<DiagnosticResult<Vec<_>>>()?,
        });
    }
    let mut class_plans = HashMap::new();
    let mut instances = HashMap::new();
    for class in classes {
        let receiver = ClassType::new(class.declaration_name.clone(), class.arguments.clone());
        let hierarchy = class
            .ancestors
            .iter()
            .rev()
            .chain(std::iter::once(&receiver))
            .collect::<Vec<_>>();
        let mut slots = Vec::new();
        for owner in &hierarchy {
            for family in families.iter().filter(|family| &family.owner == *owner) {
                let selected = hierarchy
                    .iter()
                    .rev()
                    .find_map(|owner| {
                        methods
                            .values()
                            .find(|method| {
                                method.declaring_class == owner.name
                                    && method.virtual_root == Some(family.declaration)
                            })
                            .map(|method| (*owner, method))
                    })
                    .ok_or_else(|| {
                        failure(
                            family.declaration,
                            "virtual family has no checked implementation identity",
                        )
                    })?;
                let (function, declaration) =
                    declarations.get(&selected.1.declaration).ok_or_else(|| {
                        failure(
                            selected.1.declaration,
                            "virtual implementation has no declaration",
                        )
                    })?;
                if declaration.type_params.len() != selected.0.arguments.len()
                    || function.type_params.len() != family.arguments.len()
                {
                    return Err(failure(
                        selected.1.declaration,
                        "virtual family has inconsistent generic arity",
                    ));
                }
                let bindings =
                    declaration
                        .type_params
                        .iter()
                        .zip(&selected.0.arguments)
                        .map(|(parameter, argument)| (parameter.name.clone(), argument.clone()))
                        .chain(function.type_params.iter().zip(&family.arguments).map(
                            |(parameter, argument)| (parameter.name.clone(), argument.clone()),
                        ))
                        .collect();
                let implementation = CallableInstanceKey::new(selected.1.declaration, &bindings);
                if instances
                    .insert(implementation.clone(), family.clone())
                    .is_some_and(|previous| previous != *family)
                {
                    return Err(failure(
                        selected.1.declaration,
                        "virtual instance belongs to conflicting canonical families",
                    ));
                }
                slots.push(Slot {
                    family: family.clone(),
                    receiver_mode: selected.1.receiver_mode.ok_or_else(|| {
                        failure(
                            selected.1.declaration,
                            "virtual implementation has no checked receiver contract",
                        )
                    })?,
                    implementation,
                });
            }
        }
        class_plans.insert(receiver, slots);
    }
    Ok(Plan::Checked {
        classes: class_plans,
        instances,
    })
}

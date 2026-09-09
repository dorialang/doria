//! Validate final class-plan inputs independently of member selection.

use super::*;

impl CompositionPlan {
    pub fn validate(&self, program: &Program) -> Result<(), String> {
        for name in self.classes.keys() {
            if !program.items.iter().any(|item| matches!(item, Item::Class(class)
                if &class.name == name && class.members.iter().any(|member| matches!(member, ClassMember::Uses(_))))) {
                return Err("composition replaced a class without authored trait uses".into());
            }
        }
        let mut identities = HashSet::new();
        for origin in &self.origins {
            if origin.id == ExpansionId::default()
                || !identities.insert(origin.id)
                || origin.declaration.expansion != origin.id
            {
                return Err("duplicate or missing effective member identity".into());
            }
            if !self.classes.contains_key(&origin.composing_class) {
                return Err("effective origin has no composing class".into());
            }
        }
        for item in &program.items {
            let Item::Class(authored) = item else {
                continue;
            };
            if !authored
                .members
                .iter()
                .any(|member| matches!(member, ClassMember::Uses(_)))
            {
                continue;
            }
            let composed = self
                .classes
                .get(&authored.name)
                .ok_or("unresolved trait use")?;
            if self.invalid_classes.contains(&authored.name) {
                return Err("invalid composition entered executable lowering".into());
            }
            let mut header = composed.clone();
            header.members = authored.members.clone();
            if &header != authored {
                return Err("composition changed the class or its hierarchy".into());
            }
            let mut names = HashSet::new();
            let mut previous_property = Vec::new();
            let mut physical_origins = Vec::new();
            let mut represented = HashSet::new();
            for member in &composed.members {
                if matches!(member, ClassMember::Uses(_)) {
                    return Err("unresolved trait use in final member set".into());
                }
                if !names.insert(member_name(member)) {
                    return Err("duplicate effective member name".into());
                }
                if matches!(member, ClassMember::Method(method) if method.body.as_block().is_none())
                {
                    return Err("unresolved method requirement in final member set".into());
                }
                let span = member_span(member);
                let mut property_order = vec![span.start];
                if span.expansion == ExpansionId::default() {
                    if !authored.members.contains(member) {
                        return Err("composition changed an authored class member".into());
                    }
                } else {
                    let origin = self
                        .origin(span)
                        .ok_or("effective member has no authored origin")?;
                    self.validate_member(program, authored, origin, member)?;
                    represented.insert(origin.id);
                    if let ClassMember::Property(property) = member {
                        if !property.is_static {
                            if physical_origins
                                .iter()
                                .any(|prior: &&EffectiveMemberOrigin| {
                                    prior.authored_declaration == origin.authored_declaration
                                        && same_type(&prior.trait_type, &origin.trait_type)
                                })
                            {
                                return Err("duplicate physical trait property".into());
                            }
                            physical_origins.push(origin);
                            property_order =
                                origin.paths[0].iter().map(|span| span.start).collect();
                            property_order.push(origin.authored_declaration.start);
                        }
                    }
                }
                if matches!(member, ClassMember::Property(property) if !property.is_static) {
                    if property_order < previous_property {
                        return Err(
                            "trait property order differs from lexical expansion order".into()
                        );
                    }
                    previous_property = property_order;
                }
            }
            for obligation in self
                .obligations
                .iter()
                .filter(|obligation| obligation.class == authored.name)
            {
                if obligation.implementation.is_none() || !obligation.failures.is_empty() {
                    return Err("unsatisfied trait method obligation".into());
                }
                let mut owner = Some(authored.name.as_str());
                let mut visited = HashSet::new();
                let mut implementation = None;
                while let Some(name) = owner {
                    if !visited.insert(name) {
                        break;
                    }
                    let declaration = self
                        .class(name)
                        .or_else(|| {
                            program.items.iter().find_map(|item| match item {
                                Item::Class(class) if class.name == name => Some(class),
                                _ => None,
                            })
                        })
                        .ok_or("unknown trait requirement implementation owner")?;
                    if let Some(method) = declaration.members.iter().find_map(|member| match member
                    {
                        ClassMember::Method(method)
                            if method.name == obligation.requirement.name
                                && (name == authored.name
                                    || method.access == MemberAccess::External) =>
                        {
                            Some(method)
                        }
                        _ => None,
                    }) {
                        if method.body.as_block().is_some() {
                            implementation = Some(method.span);
                        }
                        break;
                    }
                    owner = declaration
                        .parent
                        .as_ref()
                        .map(|parent| parent.name.as_str());
                }
                if implementation != obligation.implementation {
                    return Err("trait requirement points to a different implementation".into());
                }
                if self.origin(obligation.origin.declaration) != Some(&obligation.origin) {
                    return Err("trait obligation has a mismatched effective identity".into());
                }
                self.validate_member(
                    program,
                    authored,
                    &obligation.origin,
                    &ClassMember::Method(obligation.requirement.clone()),
                )?;
                if !represented.insert(obligation.origin.id) {
                    return Err("requirement also occupies an executable member".into());
                }
            }
            if self
                .origins
                .iter()
                .filter(|origin| origin.composing_class == authored.name)
                .any(|origin| !represented.contains(&origin.id))
            {
                return Err("effective origin was lost from its class".into());
            }
            for member in &authored.members {
                if !matches!(member, ClassMember::Uses(_)) && !composed.members.contains(member) {
                    return Err("composition dropped an authored class member".into());
                }
            }
        }
        Ok(())
    }

    fn validate_member(
        &self,
        program: &Program,
        class: &ClassDecl,
        origin: &EffectiveMemberOrigin,
        actual: &ClassMember,
    ) -> Result<(), String> {
        if origin.composing_class != class.name || origin.declaration != member_span(actual) {
            return Err("effective member belongs to a different composer".into());
        }
        let declaration = program
            .items
            .iter()
            .find_map(|item| match item {
                Item::Trait(declaration) if declaration.name == origin.trait_type.name => {
                    Some(declaration)
                }
                _ => None,
            })
            .ok_or("unknown authored trait origin")?;
        let member = declaration
            .members
            .iter()
            .find(|member| member_span(member) == origin.authored_declaration)
            .ok_or("unknown authored member origin")?;
        if member_name_span(member) != origin.authored_name
            || declaration.type_params.len() != origin.trait_type.arguments.len()
        {
            return Err("invalid trait specialization or authored name".into());
        }
        let mut expected = member.clone();
        let substitutions = declaration
            .type_params
            .iter()
            .enumerate()
            .map(|(index, parameter)| {
                origin
                    .trait_type
                    .type_argument(index)
                    .cloned()
                    .map(|argument| (parameter.name.clone(), argument))
                    .ok_or("invalid trait type argument")
            })
            .collect::<Result<HashMap<_, _>, _>>()?;
        expected.transform(&mut Instantiate {
            substitutions,
            expansion: None,
        });
        for (path_index, path) in origin.paths.iter().enumerate() {
            if !path.first().is_some_and(|first| {
                class.members.iter().any(|member|
                matches!(member, ClassMember::Uses(uses) if uses.type_spans.contains(first)))
            }) {
                return Err("trait origin has no composing-class use path".into());
            }
            let mut members = &class.members;
            let mut bindings = HashMap::new();
            let mut leaf = None;
            let mut adaptations = Vec::new();
            let mut name = member_name(member).to_owned();
            let mut alias = None;
            let mut internal = false;
            for segment in path {
                let edge = members.iter().find_map(|member| match member {
                    ClassMember::Uses(uses) => uses
                        .type_spans
                        .iter()
                        .position(|span| span == segment)
                        .map(|index| (uses, &uses.traits[index])),
                    _ => None,
                });
                if let Some((uses, ty)) = edge {
                    let ty = substitute(ty, &bindings);
                    adaptations.extend(uses.adaptations.iter().filter(|adaptation| {
                        same_type(&substitute(&adaptation.origin, &bindings), &ty)
                    }));
                    let declaration = program
                        .items
                        .iter()
                        .find_map(|item| match item {
                            Item::Trait(declaration) if declaration.name == ty.name => {
                                Some(declaration)
                            }
                            _ => None,
                        })
                        .ok_or("trait path reaches an unknown declaration")?;
                    if declaration.type_params.len() != ty.arguments.len() {
                        return Err("trait path has incomplete generic substitution".into());
                    }
                    bindings = declaration
                        .type_params
                        .iter()
                        .zip(ty.type_arguments())
                        .map(|(parameter, argument)| (parameter.name.clone(), argument.clone()))
                        .collect();
                    members = &declaration.members;
                    leaf = Some(ty);
                } else {
                    let adaptation = adaptations
                        .iter()
                        .find(|adaptation| adaptation.span == *segment)
                        .ok_or("trait origin contains an unrelated use or adaptation")?;
                    if adaptation.method.text != name {
                        return Err("trait adaptation selects a different method origin".into());
                    }
                    if let TraitAdaptationKind::Alias {
                        alias: next_alias,
                        internal_span,
                        ..
                    } = &adaptation.kind
                    {
                        if let Some(next_alias) = next_alias {
                            name.clone_from(&next_alias.text);
                            alias = Some(next_alias.span);
                        }
                        internal |= internal_span.is_some();
                    }
                }
            }
            if !leaf
                .as_ref()
                .is_some_and(|ty| same_type(ty, &origin.trait_type))
            {
                return Err(
                    "trait origin specialization differs from its authored use path".into(),
                );
            }
            if name != origin.name || alias != origin.alias {
                return Err("effective alias differs from its authored adaptation path".into());
            }
            if path_index == 0 {
                if let ClassMember::Method(method) = &mut expected {
                    method.name = name;
                    if internal {
                        method.access = MemberAccess::Internal;
                    }
                }
            }
        }
        if origin.paths.is_empty() {
            return Err("trait origin has no expansion path".into());
        }
        expected.transform(&mut Instantiate {
            substitutions: HashMap::new(),
            expansion: Some(origin.id),
        });
        if member_name(&expected) != origin.name || &expected != actual {
            return Err(
                "effective member changed its authored body, signature, substitution, or access"
                    .into(),
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_class_plans_do_not_cross_executable_lowering() {
        let source = r#"
trait Fields<T> {
    T $first;
    int $second = 2;
    function required(): int;
    function read(): int { return $this->second; }
    function other(): int { return 4; }
}
class Box {
    uses Fields<int> { Fields<int>::read as alias; }
    function __construct() { $this->first = 1; }
    function required(): int { return 3; }
}
"#;
        let (program, analysis) = crate::analyze_source_for_ide("plan.doria", source).unwrap();
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
        let plan = analysis.info.composition;
        assert_eq!(plan.validate(&program), Ok(()));
        let mutations: &[fn(&mut CompositionPlan)] = &[
            |plan| {
                plan.classes.clear();
            },
            |plan| {
                let class = plan.classes.get_mut("Box").unwrap();
                class.members.push(class.members[0].clone());
            },
            |plan| {
                plan.classes.get_mut("Box").unwrap().members.swap(0, 1);
            },
            |plan| {
                plan.classes.get_mut("Box").unwrap().name = "Other".into();
            },
            |plan| {
                plan.origins[0].composing_class = "Other".into();
            },
            |plan| {
                plan.origins[0].trait_type =
                    TypeRef::generic("Fields", vec![TypeRef::named("string")]);
            },
            |plan| {
                plan.origins[0].paths.clear();
            },
            |plan| {
                let alias = plan
                    .origins
                    .iter_mut()
                    .find(|origin| origin.alias.is_some())
                    .unwrap();
                alias.paths[0].pop();
            },
            |plan| {
                let other = plan
                    .origins
                    .iter()
                    .find(|origin| origin.name == "other")
                    .unwrap()
                    .clone();
                let alias = plan
                    .origins
                    .iter_mut()
                    .find(|origin| origin.alias.is_some())
                    .unwrap();
                alias.authored_declaration = other.authored_declaration;
                alias.authored_name = other.authored_name;
                alias.declaration = other.authored_declaration.in_expansion(alias.id);
                let mut replacement = plan.classes["Box"]
                    .members
                    .iter()
                    .find(|member| member_name(member) == "other")
                    .unwrap()
                    .clone();
                replacement.transform(&mut Instantiate {
                    substitutions: HashMap::new(),
                    expansion: Some(alias.id),
                });
                if let ClassMember::Method(method) = &mut replacement {
                    method.name.clone_from(&alias.name);
                }
                let class = plan.classes.get_mut("Box").unwrap();
                *class
                    .members
                    .iter_mut()
                    .find(|member| member_name(member) == "alias")
                    .unwrap() = replacement;
            },
            |plan| {
                plan.origins[1].id = plan.origins[0].id;
            },
            |plan| {
                plan.obligations[0].implementation = None;
            },
            |plan| {
                plan.obligations[0].implementation = Some(plan.origins[0].declaration);
            },
            |plan| {
                plan.invalid_classes.insert("Box".into());
            },
            |plan| {
                let mut extra = plan.classes["Box"].clone();
                extra.name = "Extra".into();
                plan.classes.insert("Extra".into(), extra);
            },
            |plan| {
                if let ClassMember::Property(property) =
                    &mut plan.classes.get_mut("Box").unwrap().members[0]
                {
                    property.writable = !property.writable;
                }
            },
            |plan| {
                let class = plan.classes.get_mut("Box").unwrap();
                if let ClassMember::Method(method) = class
                    .members
                    .iter_mut()
                    .find(|member| member_name(member) == "alias")
                    .unwrap()
                {
                    method.return_type = Some(TypeRef::named("bool"));
                }
            },
            |plan| {
                plan.classes
                    .get_mut("Box")
                    .unwrap()
                    .members
                    .push(ClassMember::Method(plan.obligations[0].requirement.clone()));
            },
        ];
        for (index, mutate) in mutations.iter().enumerate() {
            let mut broken = plan.clone();
            mutate(&mut broken);
            assert!(
                broken.validate(&program).is_err(),
                "mutation {index} was accepted"
            );
        }
    }
}

//! Compiler-owned trait expansion. Backends consume the checked final members.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::ast::transform::{Transform, Transformable};
use crate::ast::*;
use crate::diagnostics::Diagnostic;
use crate::semantics::contracts::ContractFacts;
use crate::source::{ExpansionId, Span};
use crate::types::TypeRef;

mod validation;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveMemberOrigin {
    pub id: ExpansionId,
    pub composing_class: String,
    pub trait_type: TypeRef,
    pub authored_declaration: Span,
    pub authored_name: Span,
    pub declaration: Span,
    pub name: String,
    pub alias: Option<Span>,
    /// All authored paths, including duplicate diamond paths, in lexical order.
    pub paths: Vec<Vec<Span>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodObligation {
    pub class: String,
    pub requirement: FunctionDecl,
    pub origin: EffectiveMemberOrigin,
    pub implementation: Option<Span>,
    pub failures: Vec<crate::semantics::contracts::ContractMismatch>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompositionPlan {
    pub(crate) classes: BTreeMap<String, ClassDecl>,
    pub origins: Vec<EffectiveMemberOrigin>,
    pub obligations: Vec<MethodObligation>,
    pub invalid_classes: HashSet<String>,
    pub(crate) candidates: Vec<AdaptationCandidate>,
    pub adaptation_references: Vec<crate::semantics::contracts::ContractMemberReference>,
    pub adaptation_subjects: Vec<AdaptationSubjects>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdaptationSubjects {
    pub span: Span,
    pub selected: Span,
    pub excluded: Vec<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AdaptationCandidate {
    pub class: String,
    pub uses: Span,
    pub edge: TypeRef,
    pub applicable_origins: Vec<Span>,
    pub authored_declaration: Span,
    pub method: FunctionDecl,
}

impl CompositionPlan {
    pub fn origin(&self, span: Span) -> Option<&EffectiveMemberOrigin> {
        self.origins
            .iter()
            .find(|origin| origin.id == span.expansion)
    }

    pub fn class(&self, name: &str) -> Option<&ClassDecl> {
        self.classes.get(name)
    }

    pub(crate) fn apply(&self, program: &Program) -> Program {
        let mut program = program.clone();
        for item in &mut program.items {
            if let Item::Class(class) = item {
                if let Some(composed) = self.classes.get(&class.name) {
                    *class = composed.clone();
                }
            }
        }
        program
    }
}

#[derive(Clone)]
struct Candidate {
    member: ClassMember,
    trait_type: TypeRef,
    authored_declaration: Span,
    authored_name: Span,
    alias: Option<Span>,
    paths: Vec<Vec<Span>>,
}

pub(crate) fn prepare(
    program: &Program,
    facts: &ContractFacts,
) -> (CompositionPlan, Vec<Diagnostic>) {
    let traits = program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Trait(trait_decl) => Some((trait_decl.name.clone(), trait_decl)),
            _ => None,
        })
        .collect();
    let mut composer = Composer {
        traits,
        facts,
        plan: CompositionPlan::default(),
        diagnostics: Vec::new(),
        current_class: String::new(),
        next_expansion: 1,
    };
    for item in &program.items {
        if let Item::Class(class) = item {
            if class
                .members
                .iter()
                .any(|member| matches!(member, ClassMember::Uses(_)))
            {
                composer.compose_class(class);
            }
        }
    }
    (composer.plan, composer.diagnostics)
}

struct Composer<'a> {
    traits: HashMap<String, &'a TraitDecl>,
    facts: &'a ContractFacts,
    plan: CompositionPlan,
    diagnostics: Vec<Diagnostic>,
    current_class: String,
    next_expansion: u32,
}

impl Composer<'_> {
    fn compose_class(&mut self, class: &ClassDecl) {
        self.current_class = class.name.clone();
        let before = self.diagnostics.len();
        let valid = self
            .facts
            .compositions
            .iter()
            .filter(|fact| fact.class == class.name)
            .all(|fact| fact.valid);
        let mut output = class.clone();
        output.members.clear();
        let mut candidates = Vec::new();
        let mut order = Vec::new();
        for member in &class.members {
            match member {
                ClassMember::Uses(uses) => {
                    for candidate in self.expand_uses(uses, &HashMap::new(), &[], &mut Vec::new()) {
                        if !valid {
                            continue;
                        }
                        let index = candidates.len();
                        candidates.push(candidate);
                        order.push(Err(index));
                    }
                }
                _ => order.push(Ok(member.clone())),
            }
        }
        deduplicate(&mut candidates, &mut order);
        let authored: HashMap<_, _> = class
            .members
            .iter()
            .filter(|member| !matches!(member, ClassMember::Uses(_)))
            .map(|member| (member_name(member).to_owned(), member))
            .collect();
        let mut selected = HashMap::<String, Span>::new();
        for entry in order {
            let candidate = match entry {
                Ok(member) => {
                    output.members.push(member);
                    continue;
                }
                Err(index) => &candidates[index],
            };
            let name = member_name(&candidate.member).to_owned();
            let requirement = matches!(&candidate.member, ClassMember::Method(method) if method.body.as_block().is_none());
            let displaced = matches!(
                (authored.get(&name), &candidate.member),
                (Some(ClassMember::Method(_)), ClassMember::Method(_))
            );
            if !requirement {
                if let Some(previous) = selected.get(&name).copied().or_else(|| {
                    (!displaced)
                        .then(|| authored.get(&name).map(|member| member_span(member)))
                        .flatten()
                }) {
                    self.conflict(&name, member_span(&candidate.member), previous);
                    continue;
                }
                selected.insert(name.clone(), member_span(&candidate.member));
            }
            let id = self.expansion_id();
            let mut member = candidate.member.clone();
            member.transform(&mut Instantiate {
                substitutions: HashMap::new(),
                expansion: Some(id),
            });
            let origin = EffectiveMemberOrigin {
                id,
                composing_class: class.name.clone(),
                trait_type: candidate.trait_type.clone(),
                authored_declaration: candidate.authored_declaration,
                authored_name: candidate.authored_name,
                declaration: member_span(&member),
                name,
                alias: candidate.alias,
                paths: candidate.paths.clone(),
            };
            self.plan.origins.push(origin.clone());
            if requirement || displaced {
                if let ClassMember::Method(requirement) = member {
                    self.plan.obligations.push(MethodObligation {
                        class: class.name.clone(),
                        requirement,
                        origin,
                        implementation: None,
                        failures: Vec::new(),
                    });
                }
            } else {
                output.members.push(member);
            }
        }
        if !valid || self.diagnostics.len() != before {
            self.plan.invalid_classes.insert(class.name.clone());
        }
        self.plan.classes.insert(class.name.clone(), output);
    }

    fn expansion_id(&mut self) -> ExpansionId {
        let id = ExpansionId(self.next_expansion);
        self.next_expansion += 1;
        id
    }

    fn expand_uses(
        &mut self,
        uses: &TraitUse,
        bindings: &HashMap<String, TypeRef>,
        path: &[Span],
        stack: &mut Vec<String>,
    ) -> Vec<Candidate> {
        let mut candidates = Vec::<(TypeRef, Candidate)>::new();
        for (index, ty) in uses.traits.iter().enumerate() {
            let ty = substitute(ty, bindings);
            let Some(declaration) = self.traits.get(&ty.name).copied() else {
                continue;
            };
            if stack.contains(&ty.name)
                || !self
                    .facts
                    .traits
                    .iter()
                    .any(|fact| fact.name == ty.name && fact.valid)
            {
                continue;
            }
            let mut bindings = HashMap::new();
            for (index, parameter) in declaration.type_params.iter().enumerate() {
                if let Some(argument) = ty.type_argument(index) {
                    bindings.insert(parameter.name.clone(), argument.clone());
                } else if let Some(default) = &parameter.default_type {
                    bindings.insert(parameter.name.clone(), substitute(default, &bindings));
                }
            }
            let canonical_type = TypeRef::generic(
                &declaration.name,
                declaration
                    .type_params
                    .iter()
                    .filter_map(|parameter| bindings.get(&parameter.name).cloned())
                    .collect(),
            );
            let mut child_path = path.to_vec();
            child_path.push(uses.type_spans.get(index).copied().unwrap_or(uses.span));
            stack.push(ty.name.clone());
            for member in &declaration.members {
                if let ClassMember::Uses(nested) = member {
                    candidates.extend(
                        self.expand_uses(nested, &bindings, &child_path, stack)
                            .into_iter()
                            .map(|candidate| (ty.clone(), candidate)),
                    );
                } else {
                    let mut instantiated = member.clone();
                    instantiated.transform(&mut Instantiate {
                        substitutions: bindings.clone(),
                        expansion: None,
                    });
                    candidates.push((
                        ty.clone(),
                        Candidate {
                            member: instantiated,
                            trait_type: canonical_type.clone(),
                            authored_declaration: member_span(member),
                            authored_name: member_name_span(member),
                            alias: None,
                            paths: vec![child_path.clone()],
                        },
                    ));
                }
            }
            stack.pop();
        }
        let mut unique: Vec<(TypeRef, Candidate)> = Vec::new();
        for (edge, candidate) in candidates {
            if let Some((_, previous)) = unique.iter_mut().find(|(prior_edge, prior)| {
                same_type(prior_edge, &edge) && same_origin(prior, &candidate)
            }) {
                previous.paths.extend(candidate.paths);
            } else {
                unique.push((edge, candidate));
            }
        }
        let mut candidates = unique;
        let mut excluded = HashSet::new();
        let mut aliases = Vec::new();
        for (edge, candidate) in &candidates {
            if let ClassMember::Method(method) = &candidate.member {
                if method.body.as_block().is_some() {
                    let mut method = method.clone();
                    method.transform(&mut Instantiate {
                        substitutions: HashMap::new(),
                        expansion: Some(self.expansion_id()),
                    });
                    self.plan.candidates.push(AdaptationCandidate {
                        class: self.current_class.clone(),
                        uses: uses.span,
                        edge: edge.clone(),
                        applicable_origins: uses
                            .adaptations
                            .iter()
                            .filter(|adaptation| {
                                same_type(edge, &substitute(&adaptation.origin, bindings))
                            })
                            .map(|adaptation| adaptation.origin_span)
                            .collect(),
                        authored_declaration: candidate.authored_declaration,
                        method,
                    });
                }
            }
        }
        for adaptation in &uses.adaptations {
            let origin = substitute(&adaptation.origin, bindings);
            let matches = candidates.iter().enumerate().filter(|(_, (edge, candidate))| {
                same_type(edge, &origin) && member_name(&candidate.member) == adaptation.method.text
                    && matches!(&candidate.member, ClassMember::Method(method) if method.body.as_block().is_some())
            }).map(|(index, _)| index).collect::<Vec<_>>();
            if matches.len() != 1 {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0757",
                        format!(
                            "adaptation must select one implemented method `{origin}::{}`",
                            adaptation.method.text
                        ),
                        adaptation.method.span,
                    )
                    .with_title("Trait Adaptation Has No Unique Method"),
                );
                continue;
            }
            let chosen = matches[0];
            let mut subjects = AdaptationSubjects {
                span: adaptation.method.span,
                selected: candidates[chosen]
                    .1
                    .alias
                    .unwrap_or(candidates[chosen].1.authored_name),
                excluded: Vec::new(),
            };
            self.plan.adaptation_references.push(
                crate::semantics::contracts::ContractMemberReference {
                    span: adaptation.method.span,
                    origins: vec![candidates[chosen].1.authored_declaration],
                },
            );
            match &adaptation.kind {
                TraitAdaptationKind::InsteadOf {
                    excluded: edges, ..
                } => {
                    for path in &mut candidates[chosen].1.paths {
                        path.push(adaptation.span);
                    }
                    for edge in edges {
                        let edge = substitute(edge, bindings);
                        let mut found = false;
                        for (index, (candidate_edge, candidate)) in candidates.iter().enumerate() {
                            if same_type(candidate_edge, &edge)
                                && member_name(&candidate.member) == adaptation.method.text
                                && matches!(&candidate.member, ClassMember::Method(method) if method.body.as_block().is_some())
                            {
                                found = true;
                                let subject = candidate.alias.unwrap_or(candidate.authored_name);
                                if !subjects.excluded.contains(&subject) {
                                    subjects.excluded.push(subject);
                                }
                                if index != chosen {
                                    excluded.insert(index);
                                }
                            }
                        }
                        if !found {
                            self.diagnostics.push(
                                Diagnostic::new(
                                    "E0757",
                                    format!(
                                        "excluded trait `{edge}` does not supply `{}`",
                                        adaptation.method.text
                                    ),
                                    adaptation.span,
                                )
                                .with_title("Invalid Trait Exclusion"),
                            );
                        }
                    }
                }
                TraitAdaptationKind::Alias {
                    internal_span,
                    alias,
                    ..
                } => {
                    if let Some(alias) = alias {
                        let mut candidate = candidates[chosen].1.clone();
                        if let ClassMember::Method(method) = &mut candidate.member {
                            method.name = alias.text.clone();
                            if internal_span.is_some() {
                                method.access = MemberAccess::Internal;
                            }
                        }
                        candidate.alias = Some(alias.span);
                        for path in &mut candidate.paths {
                            path.push(adaptation.span);
                        }
                        aliases.push(candidate);
                    } else if internal_span.is_some() {
                        if let ClassMember::Method(method) = &mut candidates[chosen].1.member {
                            method.access = MemberAccess::Internal;
                        }
                        for path in &mut candidates[chosen].1.paths {
                            path.push(adaptation.span);
                        }
                    }
                }
            }
            self.plan.adaptation_subjects.push(subjects);
        }
        let mut result = candidates
            .into_iter()
            .enumerate()
            .filter_map(|(index, (_, member))| (!excluded.contains(&index)).then_some(member))
            .collect::<Vec<_>>();
        result.extend(aliases);
        result
    }

    fn conflict(&mut self, name: &str, here: Span, previous: Span) {
        self.diagnostics.push(
            Diagnostic::new(
                "E0757",
                format!("trait composition supplies conflicting member `{name}`"),
                here,
            )
            .with_title("Trait Member Conflict")
            .with_related(previous, "the other member is declared here"),
        );
    }
}

fn deduplicate(candidates: &mut [Candidate], order: &mut Vec<Result<ClassMember, usize>>) {
    let mut first: Vec<usize> = Vec::new();
    order.retain(|entry| {
        let Err(index) = entry else {
            return true;
        };
        let duplicate = first.iter().copied().find(|previous| {
            let a = &candidates[*previous];
            let b = &candidates[*index];
            same_origin(a, b)
        });
        if let Some(previous) = duplicate {
            let paths = candidates[*index].paths.clone();
            candidates[previous].paths.extend(paths);
            false
        } else {
            first.push(*index);
            true
        }
    });
}

fn same_origin(a: &Candidate, b: &Candidate) -> bool {
    a.authored_declaration == b.authored_declaration
        && same_type(&a.trait_type, &b.trait_type)
        && a.alias == b.alias
        && member_name(&a.member) == member_name(&b.member)
}

fn same_type(a: &TypeRef, b: &TypeRef) -> bool {
    let bindings = HashMap::new();
    match (
        crate::types::resolved_type_ref_with_substitutions(a, &bindings),
        crate::types::resolved_type_ref_with_substitutions(b, &bindings),
    ) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

pub(crate) fn member_span(member: &ClassMember) -> Span {
    match member {
        ClassMember::Method(value) => value.span,
        ClassMember::Property(value) => value.span,
        ClassMember::Constant(value) => value.span,
        ClassMember::Uses(value) => value.span,
    }
}
pub(crate) fn member_name(member: &ClassMember) -> &str {
    match member {
        ClassMember::Method(value) => &value.name,
        ClassMember::Property(value) => &value.name,
        ClassMember::Constant(value) => &value.name,
        ClassMember::Uses(_) => "",
    }
}
fn member_name_span(member: &ClassMember) -> Span {
    match member {
        ClassMember::Method(value) => value.name_span,
        ClassMember::Constant(value) => value.name_span,
        ClassMember::Property(value) => value.name_span,
        _ => member_span(member),
    }
}

struct Instantiate {
    substitutions: HashMap<String, TypeRef>,
    expansion: Option<ExpansionId>,
}
impl Transform for Instantiate {
    fn span(&mut self, span: &mut Span) {
        if let Some(expansion) = self.expansion {
            *span = span.in_expansion(expansion);
        }
    }
    fn type_ref(&mut self, ty: &mut TypeRef) {
        if ty.arguments.is_empty() && ty.function.is_none() && ty.grouped.is_none() {
            if let Some(replacement) = self.substitutions.get(&ty.name) {
                let mut replacement = replacement.clone();
                replacement.nullable |= ty.nullable;
                *ty = replacement;
                // Substitution is simultaneous, not recursive through the replacement.
                let saved = std::mem::take(&mut self.substitutions);
                ty.walk(self);
                self.substitutions = saved;
                return;
            }
        }
        ty.walk(self);
    }
    fn function(&mut self, function: &mut FunctionDecl) {
        let saved = self.substitutions.clone();
        for parameter in &function.type_params {
            self.substitutions.remove(&parameter.name);
        }
        if !self.substitutions.is_empty() {
            let renamed = function
                .type_params
                .iter()
                .filter(|parameter| !parameter.name.contains('#'))
                .map(|parameter| {
                    (
                        parameter.name.clone(),
                        TypeRef::named(format!(
                            "{}#trait{}.{}",
                            parameter.name, parameter.span.source.0, parameter.span.start
                        )),
                    )
                })
                .collect::<HashMap<_, _>>();
            if !renamed.is_empty() {
                function.walk(&mut Instantiate {
                    substitutions: renamed.clone(),
                    expansion: None,
                });
                for parameter in &mut function.type_params {
                    if let Some(name) = renamed.get(&parameter.name) {
                        parameter.name = name.name.clone();
                    }
                }
            }
        }
        function.walk(self);
        self.substitutions = saved;
    }
}

pub fn authored_type_parameter_name(name: &str) -> &str {
    name.split_once("#trait")
        .map_or(name, |(authored, _)| authored)
}
fn substitute(ty: &TypeRef, bindings: &HashMap<String, TypeRef>) -> TypeRef {
    let mut ty = ty.clone();
    ty.transform(&mut Instantiate {
        substitutions: bindings.clone(),
        expansion: None,
    });
    ty
}

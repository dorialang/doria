//! Source-edit proofs for checked members. Identities remain analysis-local;
//! clients must additionally prove that the containing source graph is editable.

use std::collections::{BTreeMap, BTreeSet};

use crate::ast::{self, ClassMember, Expr, FunctionDecl, Item, Program, TraitAdaptationKind};
use crate::source::Span;
use crate::types::{ClassType, ResolvedType, SharedHandleKind};

use super::{CallableTarget, SemanticInfo};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompositionRenameFacts {
    pub targets: Vec<MemberRenameTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberRenameTarget {
    pub name: String,
    pub name_span: Span,
    /// Effective declarations, retaining expansion identity, including aliases.
    pub declarations: Vec<Span>,
    /// Exact authored member tokens, excluding the target's declaration token.
    pub references: Vec<Span>,
    pub forbidden_names: Vec<String>,
    /// `None` proves the occurrence set only within this analyzed source graph.
    pub refusal: Option<MemberRenameRefusal>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberRenameRefusal {
    InvalidComposition,
    IncompleteOccurrences,
    AmbiguousSourceToken,
    RelatedContract,
    SpecialMethod,
}

/// Consume the exact authored and expanded inputs to this semantic analysis.
/// This performs no name resolution, composition, or source-text matching.
pub fn composition_rename_facts(
    authored: &Program,
    expanded: &Program,
    info: &SemanticInfo,
) -> CompositionRenameFacts {
    let mut proof = Proof {
        info,
        targets: BTreeMap::new(),
        declarations: BTreeMap::new(),
        observations: BTreeMap::new(),
        unresolved_names: BTreeSet::new(),
    };
    for item in &authored.items {
        let members = match item {
            Item::Class(class) => &class.members,
            Item::Trait(declaration) => &declaration.members,
            _ => continue,
        };
        let mut names = Vec::new();
        let mut local_targets = Vec::new();
        for member in members {
            match member {
                ClassMember::Uses(uses) => {
                    for adaptation in &uses.adaptations {
                        if let TraitAdaptationKind::Alias {
                            alias: Some(alias), ..
                        } = &adaptation.kind
                        {
                            proof.add_target(&alias.text, alias.span, None);
                            names.push(alias.text.clone());
                            local_targets.push(alias.span.authored());
                        }
                    }
                }
                _ => {
                    let (name, name_span, declaration) = member_identity(member).unwrap();
                    proof.add_target(name, name_span, Some(declaration));
                    names.push(name.to_owned());
                    local_targets.push(name_span.authored());
                    if let ClassMember::Method(method) = member {
                        if matches!(method.name.as_str(), "__construct" | "__destruct") {
                            proof.refuse(name_span, MemberRenameRefusal::SpecialMethod);
                        }
                    }
                }
            }
        }
        for target in local_targets {
            proof
                .targets
                .get_mut(&target)
                .unwrap()
                .forbidden_names
                .extend(names.clone());
        }
    }

    for origin in &info.composition.origins {
        let key = origin.alias.unwrap_or(origin.authored_name).authored();
        if let Some(target) = proof.targets.get_mut(&key) {
            target.declarations.push(origin.declaration);
            proof.declarations.insert(origin.declaration, key);
        }
    }

    // A shared authored token may have several checked expansion contexts. Keep
    // every observation until agreement has been proved, including unknowns.
    visit_program(expanded, false, &mut |expression| {
        proof.observe_expression(expression)
    });
    visit_program(authored, true, &mut |expression| {
        if let Some((name, token)) = member_token(expression) {
            if !proof.observations.contains_key(&token.authored()) {
                proof.observe(name, token, None);
            }
        }
    });
    for subject in &info.composition.adaptation_subjects {
        let key = subject.selected.authored();
        let selected = proof.targets.contains_key(&key).then_some(key);
        proof
            .observations
            .entry(subject.span.authored())
            .or_default()
            .insert(selected);
        if !subject.excluded.is_empty() {
            // Renaming only one side can change which method an exclusion selects.
            proof.refuse(key, MemberRenameRefusal::RelatedContract);
            for excluded in &subject.excluded {
                proof.refuse(*excluded, MemberRenameRefusal::RelatedContract);
            }
        }
    }
    for item in &authored.items {
        let members = match item {
            Item::Class(class) => &class.members,
            Item::Trait(declaration) => &declaration.members,
            _ => continue,
        };
        for member in members {
            let ClassMember::Uses(uses) = member else {
                continue;
            };
            for adaptation in &uses.adaptations {
                if !proof
                    .observations
                    .contains_key(&adaptation.method.span.authored())
                {
                    proof.observe(&adaptation.method.text, adaptation.method.span, None);
                }
            }
        }
    }

    for surface in &info.class_member_surfaces {
        let mut affected = surface
            .members
            .iter()
            .filter_map(|member| proof.declarations.get(&member.declaration).copied())
            .collect::<BTreeSet<_>>();
        let candidates = info
            .trait_adaptation_surfaces
            .iter()
            .filter(|candidate| candidate.composing_class == surface.receiver.name)
            .collect::<Vec<_>>();
        affected.extend(
            candidates
                .iter()
                .filter_map(|candidate| proof.declarations.get(&candidate.declaration).copied()),
        );
        let names = surface
            .members
            .iter()
            .map(|member| member.name.clone())
            .chain(candidates.iter().map(|candidate| candidate.name.clone()))
            .collect::<Vec<_>>();
        for key in affected {
            let target = proof.targets.get_mut(&key).unwrap();
            target.forbidden_names.extend(names.clone());
            if !surface.valid {
                target.refusal = Some(MemberRenameRefusal::InvalidComposition);
            }
        }
    }
    // Invalid expansion can omit candidates completely; no occurrence closure
    // is claimed for any member until the graph's composition is checked.
    if !info.composition.invalid_classes.is_empty() {
        for target in proof.targets.values_mut() {
            target.refusal = Some(MemberRenameRefusal::InvalidComposition);
        }
    }
    for hierarchy in info.method_hierarchy.values() {
        if hierarchy.is_open
            || hierarchy.is_override
            || hierarchy.virtual_root.is_some()
            || hierarchy.overridden_declaration.is_some()
        {
            proof.refuse_declaration(hierarchy.declaration, MemberRenameRefusal::RelatedContract);
        }
    }
    for family in info.property_families.values() {
        if !family.override_parameters.is_empty() {
            proof.refuse_declaration(
                family.root_declaration,
                MemberRenameRefusal::RelatedContract,
            );
        }
    }
    for conformance in &info.contracts.conformances {
        for implementation in &conformance.implementations {
            if let Some(declaration) = implementation.implementation {
                proof.refuse_declaration(declaration, MemberRenameRefusal::RelatedContract);
            }
        }
    }
    for obligation in &info.composition.obligations {
        proof.refuse_declaration(
            obligation.requirement.span,
            MemberRenameRefusal::RelatedContract,
        );
        if let Some(declaration) = obligation.implementation {
            proof.refuse_declaration(declaration, MemberRenameRefusal::RelatedContract);
        }
    }
    for (token, keys) in &proof.observations {
        for key in keys.iter().flatten() {
            let target = proof.targets.get_mut(key).unwrap();
            if keys.len() != 1 {
                target.refusal = Some(MemberRenameRefusal::AmbiguousSourceToken);
            }
            if *token != target.name_span {
                target.references.push(*token);
            }
        }
    }
    for target in proof.targets.values_mut() {
        if target.declarations.is_empty() || proof.unresolved_names.contains(&target.name) {
            target
                .refusal
                .get_or_insert(MemberRenameRefusal::IncompleteOccurrences);
        }
        target
            .forbidden_names
            .extend(proof.unresolved_names.iter().cloned());
        target.forbidden_names.retain(|name| name != &target.name);
        target.forbidden_names.sort();
        target.forbidden_names.dedup();
        target.declarations.sort();
        target.declarations.dedup();
        target.references.sort();
        target.references.dedup();
    }
    CompositionRenameFacts {
        targets: proof.targets.into_values().collect(),
    }
}

struct Proof<'a> {
    info: &'a SemanticInfo,
    targets: BTreeMap<Span, MemberRenameTarget>,
    declarations: BTreeMap<Span, Span>,
    observations: BTreeMap<Span, BTreeSet<Option<Span>>>,
    unresolved_names: BTreeSet<String>,
}

impl Proof<'_> {
    fn add_target(&mut self, name: &str, name_span: Span, declaration: Option<Span>) {
        let key = name_span.authored();
        self.targets
            .entry(key)
            .or_insert_with(|| MemberRenameTarget {
                name: name.to_owned(),
                name_span: key,
                declarations: declaration.into_iter().collect(),
                references: Vec::new(),
                forbidden_names: Vec::new(),
                refusal: None,
            });
        if let Some(declaration) = declaration {
            self.declarations.insert(declaration, key);
        }
    }

    fn refuse(&mut self, name_span: Span, reason: MemberRenameRefusal) {
        if let Some(target) = self.targets.get_mut(&name_span.authored()) {
            target.refusal.get_or_insert(reason);
        }
    }

    fn refuse_declaration(&mut self, declaration: Span, reason: MemberRenameRefusal) {
        if let Some(key) = self.declarations.get(&declaration).copied() {
            self.refuse(key, reason);
        }
    }

    fn observe(&mut self, name: &str, token: Span, declaration: Option<Span>) {
        let key = declaration
            .and_then(|declaration| self.declarations.get(&declaration))
            .copied();
        self.observations
            .entry(token.authored())
            .or_default()
            .insert(key);
        if declaration.is_none() {
            self.unresolved_names.insert(name.to_owned());
        }
    }

    fn observe_expression(&mut self, expression: &Expr) {
        let Some((name, token)) = member_token(expression) else {
            return;
        };
        let declaration = match expression {
            Expr::MethodCall { span, .. } | Expr::StaticCall { span, .. } => self
                .info
                .method_call_targets
                .get(span)
                .map(|target| target.declaration)
                .or_else(|| match self.info.call_targets.get(span) {
                    Some(CallableTarget::Method {
                        class_type,
                        method_name,
                        ..
                    }) => self.member(class_type, method_name),
                    _ => None,
                }),
            Expr::PropertyAccess { object, span, .. } => {
                self.info.expression_types.get(span).and_then(|_| {
                    self.info
                        .expression_types
                        .get(&object.span())
                        .and_then(receiver_class)
                        .and_then(|receiver| self.member(receiver, name))
                })
            }
            Expr::StaticMember {
                span,
                qualifier_span,
                ..
            } => self.info.expression_types.get(span).and_then(|_| {
                self.info
                    .static_receiver_types
                    .get(qualifier_span)
                    .and_then(|receiver| self.member(receiver, name))
            }),
            _ => None,
        };
        self.observe(name, token, declaration);
    }

    fn member(&self, receiver: &ClassType<ResolvedType>, name: &str) -> Option<Span> {
        self.info
            .class_member_surface(receiver)
            .filter(|surface| surface.valid)?
            .members
            .iter()
            .find(|member| member.name == name)
            .map(|member| member.declaration)
    }
}

fn receiver_class(ty: &ResolvedType) -> Option<&ClassType<ResolvedType>> {
    match ty {
        ResolvedType::Class(class) => Some(class),
        ResolvedType::Nullable(inner) => receiver_class(inner),
        ResolvedType::SharedHandle(
            SharedHandleKind::SharedReference
            | SharedHandleKind::ReadonlySharedReferenceAccess
            | SharedHandleKind::WritableSharedReferenceAccess,
            inner,
        ) => receiver_class(inner),
        _ => None,
    }
}

fn member_token(expression: &Expr) -> Option<(&str, Span)> {
    match expression {
        Expr::MethodCall {
            method,
            member_span,
            ..
        }
        | Expr::StaticCall {
            method,
            member_span,
            ..
        } => Some((method, *member_span)),
        Expr::PropertyAccess {
            property,
            member_span,
            ..
        } => Some((property, *member_span)),
        Expr::StaticMember {
            member,
            member_span,
            ..
        } => Some((member, *member_span)),
        _ => None,
    }
}

fn member_identity(member: &ClassMember) -> Option<(&str, Span, Span)> {
    match member {
        ClassMember::Method(method) => Some((&method.name, method.name_span, method.span)),
        ClassMember::Property(property) => {
            Some((&property.name, property.name_span, property.span))
        }
        ClassMember::Constant(constant) => {
            Some((&constant.name, constant.name_span, constant.span))
        }
        ClassMember::Uses(_) => None,
    }
}

fn visit_function(function: &FunctionDecl, visitor: &mut dyn FnMut(&Expr)) {
    for parameter in &function.params {
        if let Some(default) = &parameter.default {
            ast::visit::expr(default, visitor);
        }
    }
    if let Some(body) = function.body.as_block() {
        ast::visit::block(body, visitor);
    }
}

fn visit_members(members: &[ClassMember], visitor: &mut dyn FnMut(&Expr)) {
    for member in members {
        match member {
            ClassMember::Method(method) => visit_function(method, visitor),
            ClassMember::Property(property) => {
                if let Some(initializer) = &property.initializer {
                    ast::visit::expr(initializer, visitor);
                }
            }
            ClassMember::Constant(constant) => ast::visit::expr(&constant.initializer, visitor),
            ClassMember::Uses(_) => {}
        }
    }
}

fn visit_program(program: &Program, traits_only: bool, visitor: &mut dyn FnMut(&Expr)) {
    if !traits_only {
        for attachment in &program.attributes {
            for group in &attachment.groups {
                for attribute in &group.attributes {
                    if let Some(arguments) = &attribute.argument_list {
                        for argument in &arguments.arguments {
                            ast::visit::expr(&argument.value, visitor);
                        }
                    }
                }
            }
        }
    }
    for item in &program.items {
        if traits_only {
            if let Item::Trait(declaration) = item {
                visit_members(&declaration.members, visitor);
            }
            continue;
        }
        match item {
            Item::Class(class) => visit_members(&class.members, visitor),
            Item::Function(function) => visit_function(function, visitor),
            Item::Constant(constant) => ast::visit::expr(&constant.initializer, visitor),
            Item::Statement(statement) => ast::visit::stmt(statement, visitor),
            Item::Enum(declaration) => {
                for case in &declaration.cases {
                    if let Some(value) = &case.backing_value {
                        ast::visit::expr(value, visitor);
                    }
                }
            }
            Item::Interface(declaration) => {
                for requirement in &declaration.requirements {
                    visit_function(requirement, visitor);
                }
            }
            Item::Trait(_) => {}
        }
    }
}

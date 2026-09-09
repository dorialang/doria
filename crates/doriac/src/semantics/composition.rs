//! Checked member surfaces for compiler consumers, including editor tooling.

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassMemberSurface {
    pub receiver: ClassType<ResolvedType>,
    pub valid: bool,
    pub members: Vec<ClassMemberSurfaceEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassMemberSurfaceEntry {
    pub name: String,
    pub kind: MemberKind,
    pub declaration: Span,
    pub name_span: Span,
    pub declaring_class: ClassType<ResolvedType>,
    pub access: MemberAccess,
    pub writable: bool,
    pub is_open: bool,
    pub is_override: bool,
    pub ty: Option<ResolvedType>,
    pub signature: Option<CallableSignatureSemanticInfo>,
    pub generic_parameters: Vec<TypeParamDecl>,
    pub checked_effects: Vec<ResolvedType>,
    pub automatic_effects: Vec<ResolvedType>,
    pub return_borrow: Option<ReturnBorrow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraitAdaptationSurface {
    pub composing_class: String,
    pub uses: Span,
    pub trait_type: TypeRef,
    pub applicable_origins: Vec<Span>,
    pub name: String,
    pub declaration: Span,
    pub is_static: bool,
    pub access: MemberAccess,
    pub signature: CallableSignatureSemanticInfo,
    pub writable: bool,
    pub generic_parameters: Vec<TypeParamDecl>,
    pub checked_effects: Vec<ResolvedType>,
    pub automatic_effects: Vec<ResolvedType>,
    pub return_borrow: Option<ReturnBorrow>,
}

impl SemanticInfo {
    pub fn class_member_surface(
        &self,
        receiver: &ClassType<ResolvedType>,
    ) -> Option<&ClassMemberSurface> {
        self.class_member_surfaces
            .iter()
            .find(|surface| &surface.receiver == receiver)
    }
}

impl Checker<'_> {
    pub(super) fn collect_trait_adaptation_surfaces(&mut self) -> Vec<TraitAdaptationSurface> {
        self.composition
            .candidates
            .clone()
            .into_iter()
            .map(|candidate| {
                let parameters = self
                    .program
                    .items
                    .iter()
                    .find_map(|item| match item {
                        Item::Class(class) if class.name == candidate.class => {
                            Some(class.type_params.clone())
                        }
                        _ => None,
                    })
                    .unwrap_or_default();
                self.type_parameter_scopes
                    .push(type_parameter_scope(&parameters));
                let signature =
                    self.resolve_function_signature(&candidate.method, Some(&candidate.class));
                let (automatic_effects, checked_effects) = signature
                    .checked_effects
                    .iter()
                    .map(|effect| self.types.resolved(*effect))
                    .partition(crate::checked_effects::is_automatic_effect);
                let surface = TraitAdaptationSurface {
                    composing_class: candidate.class,
                    uses: candidate.uses,
                    trait_type: candidate.edge,
                    applicable_origins: candidate.applicable_origins,
                    writable: candidate.method.writable_this,
                    generic_parameters: candidate.method.type_params,
                    checked_effects,
                    automatic_effects,
                    return_borrow: signature.return_borrow,
                    name: candidate.method.name,
                    declaration: candidate.authored_declaration,
                    is_static: candidate.method.is_static,
                    access: candidate.method.access,
                    signature: CallableSignatureSemanticInfo {
                        generic_parameter_count: signature.type_params.len(),
                        parameters: signature
                            .params
                            .iter()
                            .map(|parameter| CallableParameterSemanticInfo {
                                name: parameter.name.clone(),
                                r#type: self.types.resolved(parameter.ty),
                                take: parameter.take,
                                writable: parameter.writable,
                                borrow: false,
                                has_default: parameter.has_default,
                            })
                            .collect(),
                        return_type: self.types.resolved(signature.return_ty),
                    },
                };
                self.type_parameter_scopes.pop();
                surface
            })
            .collect()
    }

    pub(super) fn collect_class_member_surfaces(
        &mut self,
        classes: &[ClassSemanticInfo],
        return_borrows: &HashMap<Span, ReturnBorrow>,
    ) -> Vec<ClassMemberSurface> {
        let mut receivers = classes
            .iter()
            .map(|class| ClassType::new(class.declaration_name.clone(), class.arguments.clone()))
            .collect::<Vec<_>>();
        let mut names = self.classes.keys().cloned().collect::<Vec<_>>();
        names.sort();
        for name in names {
            let ty = self.symbolic_class_type(&name);
            if let ResolvedType::Class(class) = self.types.resolved(ty) {
                if !receivers.contains(&class) {
                    receivers.push(class);
                }
            }
        }
        receivers.into_iter().map(|receiver| {
            let class = ClassType::new(receiver.name.clone(), receiver.arguments.iter().map(|argument| self.types.intern_resolved(argument)).collect());
            let mut names = std::collections::BTreeSet::new();
            let mut current = Some(class.clone());
            let mut visited = HashSet::new();
            while let Some(owner) = current {
                if !visited.insert(owner.name.clone()) { break; }
                if let Some(info) = self.classes.get(&owner.name) { names.extend(info.members.keys().cloned()); }
                current = self.specialized_parent_type(&owner);
            }
            let mut members = Vec::new();
            for name in names {
                let selected = self.classes.get(&class.name).and_then(|info| info.members.get(&name)).cloned().map(|member| (class.clone(), member))
                    .or_else(|| self.specialized_parent_type(&class).and_then(|parent| self.lookup_inherited_member(&parent, &name)));
                let Some((owner, member)) = selected else { continue; };
                let info = self.classes[&owner.name].clone();
                let substitutions = self.class_type_substitutions(&owner);
                let syntax = self.program.items.iter().find_map(|item| match item {
                    Item::Class(class) if class.name == owner.name => class.members.iter().find(|entry| crate::trait_composition::member_name(entry) == name),
                    _ => None,
                }).cloned();
                let name_span = match &syntax {
                    Some(ClassMember::Method(method)) => method.name_span,
                    Some(ClassMember::Property(property)) => property.name_span,
                    Some(ClassMember::Constant(constant)) => constant.name_span,
                    _ => member.span,
                };
                let mut entry = ClassMemberSurfaceEntry { name: name.clone(), kind: member.kind, declaration: member.span, name_span,
                    declaring_class: ClassType::new(owner.name.clone(), owner.arguments.iter().map(|argument| self.types.resolved(*argument)).collect()),
                    access: MemberAccess::External, writable: false, is_open: false, is_override: false, ty: None, signature: None,
                    generic_parameters: Vec::new(), checked_effects: Vec::new(), automatic_effects: Vec::new(), return_borrow: None };
                match member.kind {
                    MemberKind::InstanceMethod | MemberKind::StaticMethod => {
                        let method = self.specialize_method_for_class(&info.methods[&name], &owner);
                        entry.access = method.access;
                        entry.is_open = method.is_open;
                        entry.is_override = method.is_override;
                        entry.writable = method.receiver_mode == Some(ReceiverMode::Writable);
                        entry.signature = Some(CallableSignatureSemanticInfo {
                            generic_parameter_count: method.type_params.len(),
                            parameters: method.params.iter().enumerate().map(|(index, parameter)| CallableParameterSemanticInfo {
                                name: parameter.name.clone(), r#type: self.types.resolved(parameter.ty), take: parameter.take,
                                writable: parameter.writable, has_default: parameter.has_default,
                                borrow: matches!(&syntax, Some(ClassMember::Method(method)) if method.params.get(index).is_some_and(|parameter| parameter.borrow_span.is_some())),
                            }).collect(), return_type: self.types.resolved(method.return_ty),
                        });
                        if let Some(ClassMember::Method(method)) = syntax { entry.generic_parameters = method.type_params; }
                        (entry.automatic_effects, entry.checked_effects) = method.checked_effects.iter().map(|effect| self.types.resolved(*effect)).partition(crate::checked_effects::is_automatic_effect);
                        entry.return_borrow = return_borrows.get(&method.declaration).copied().or(method.return_borrow);
                    }
                    MemberKind::InstanceProperty | MemberKind::PromotedProperty => {
                        let property = self.specialize_property_for_class(&info.properties[&name], &owner);
                        entry.access = property.access; entry.writable = property.writable; entry.ty = Some(self.types.resolved(property.ty));
                    }
                    MemberKind::StaticProperty => {
                        let property = &info.static_properties[&name];
                        entry.access = property.access; entry.writable = property.writable;
                        let ty = self.substitute_type_id(property.ty, &substitutions); entry.ty = Some(self.types.resolved(ty));
                    }
                    MemberKind::Constant => {
                        let constant = &info.constants[&name]; entry.access = constant.access;
                        let ty = self.substitute_type_id(constant.ty, &substitutions); entry.ty = Some(self.types.resolved(ty));
                    }
                }
                members.push(entry);
            }
            let valid = self.class_composition_is_valid(&class.name);
            ClassMemberSurface { receiver, members, valid }
        }).collect()
    }
}

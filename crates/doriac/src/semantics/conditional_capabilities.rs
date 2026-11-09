use super::contracts::{NominalConditionFacts, NominalObligation, NominalTypeParameter};
use super::{type_parameter_scope, Checker};
use crate::ast::{
    ClassMember, FunctionDecl, Item, NominalCondition, Program, PropertyDecl, TypeParamDecl,
};
use crate::diagnostics::Diagnostic;
use crate::property_hooks::{declaration_facts, PropertyHookContext, PropertyHookStorage};
use crate::source::Span;
use std::collections::{HashMap, HashSet};

mod bodies;
pub(super) use bodies::AvailabilityCall;
#[cfg(test)]
mod tests;

#[cfg(test)]
pub(super) fn with_checked_program<R>(
    declarations: Checker<'_>,
    inspect: impl FnOnce(&mut Checker<'_>) -> R,
) -> R {
    with_source_checked_program(declarations, |mut checker, initial_diagnostics| {
        let program = checker.program;
        let authored = checker.callable_body_program();
        let fields = checker.property_backing_fields();
        let partial_cleanups = checker.check_constructor_initialization(&fields);
        let classes = super::collect_ordered_class_semantics_once(
            program,
            &mut checker,
            &fields,
            super::ClassFactsPurpose::SourceOwnership,
        );
        let enums = super::collect_ordered_enum_semantics(&checker);
        let conversions = checker.interface_conversion_types();
        let mut provenance = checker.prepare_call_provenance(&fields);
        let (_, mut ownership) =
            checker.analyze_ownership(authored, &classes, &enums, &conversions, &provenance);
        checker.check_post_ownership_contracts(&mut ownership, &mut provenance, &partial_cleanups);
        for diagnostic in initial_diagnostics {
            if !checker.diagnostics.contains(&diagnostic) {
                checker.diagnostics.push(diagnostic);
            }
        }
        inspect(&mut checker)
    })
}

pub(super) fn checked_analysis(declarations: Checker<'_>) -> super::SemanticAnalysis {
    with_source_checked_program(declarations, |mut checker, diagnostics| {
        let authored = checker.callable_body_program();
        let program = checker.program;
        let const_evaluation = checker.const_evaluation.clone();
        let source_texts = checker.source_texts.clone();
        let compilation_context = checker.compilation_context.clone();
        let compilation_contexts = checker.compilation_contexts.clone();
        let global_symbols = checker.global_symbols.clone();
        let source_semantic_contexts = checker.source_semantic_contexts.clone();
        let test_semantics = checker.test_semantics.clone();
        let composition = checker.composition.clone();
        let conditions = checker.contracts.conditions.clone();
        let callable_conditions = checker.contracts.callable_conditions.clone();
        let contracts = super::callable_contracts::Seeds::collect(&mut checker, false);
        let source = super::finish_checked_analysis(checker, authored, diagnostics);
        if source
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == crate::diagnostics::DiagnosticSeverity::Error)
        {
            return source;
        }

        // Declarations and refined contracts remain complete. Only admitted
        // bodies acquire executable facts under their effective identities.
        let bodies = checked_body_program(program, &source.info);
        let mut executable = super::callable_contracts::refined_checker(
            || {
                let mut executable = Checker::new(
                    program,
                    const_evaluation.clone(),
                    &source_texts,
                    compilation_context.clone(),
                    compilation_contexts.clone(),
                    global_symbols.clone(),
                    source_semantic_contexts.clone(),
                    test_semantics.clone(),
                );
                executable.callable_body_program = super::CallableBodyProgram::Expanded(&bodies);
                executable.composition = composition.clone();
                executable.contracts.conditions = conditions.clone();
                executable.contracts.callable_conditions = callable_conditions.clone();
                contracts.apply(&mut executable);
                executable.retained_callable_seed = source.info.retained_callables.clone();
                executable
            },
            |checker| {
                checker.check_conditional_program();
                super::collect_concrete_instances(program, checker);
            },
        );
        executable.check_conditional_program();
        super::collect_concrete_instances(program, &mut executable);
        let mut result = super::finish_checked_analysis(executable, authored, source.diagnostics);
        // Editor occurrences belong to authored caller scopes, including bodies
        // that acquire no executable specialization in this compilation.
        result.info.class_member_availability = source.info.class_member_availability;
        result.info.static_receiver_types = source.info.static_receiver_types;
        for surface in source.info.class_member_surfaces {
            if !result
                .info
                .class_member_surfaces
                .iter()
                .any(|existing| existing.receiver == surface.receiver)
            {
                result.info.class_member_surfaces.push(surface);
            }
        }
        result
    })
}

fn checked_body_program(program: &Program, facts: &super::SemanticInfo) -> Program {
    let admitted = match &facts.callable_admissions {
        super::callable_contracts::Admissions::Checked(instances) => instances
            .iter()
            .map(super::callable_contracts::Instance::declaration)
            .collect::<HashSet<_>>(),
        super::callable_contracts::Admissions::Unprepared => HashSet::new(),
    };
    let selected =
        |span| !facts.contracts.callable_conditions.contains_key(&span) || admitted.contains(&span);
    let mut bodies = program.clone();
    bodies.items.retain_mut(|item| match item {
        // Their authored bodies have already established the source contracts.
        Item::Trait(_) => false,
        Item::Class(class) => {
            class.members.retain(|member| {
                crate::property_hooks::member_callables(member, PropertyHookContext::Class)
                    .all(|callable| selected(callable.span))
            });
            true
        }
        _ => true,
    });
    bodies
}

pub(super) fn remove_source_view_conditions(
    contracts: &mut super::contracts::ContractFacts,
    composition: &crate::trait_composition::CompositionPlan,
) {
    for member in composition.source_trait_members.values().flatten() {
        contracts
            .conditions
            .remove(&crate::trait_composition::member_span(member));
        for callable in crate::property_hooks::member_callables(member, PropertyHookContext::Trait)
        {
            contracts.callable_conditions.remove(&callable.span);
        }
    }
}

fn with_source_checked_program<R>(
    mut declarations: Checker<'_>,
    inspect: impl FnOnce(Checker<'_>, Vec<Diagnostic>) -> R,
) -> R {
    let composition = declarations.collect_conditional_declaration_facts();
    let conditions = std::mem::take(&mut declarations.contracts.conditions);
    let callable_conditions = std::mem::take(&mut declarations.contracts.callable_conditions);
    let mut initial_diagnostics = std::mem::take(&mut declarations.diagnostics);
    let Checker {
        program: authored,
        source_texts,
        compilation_context,
        compilation_contexts,
        global_symbols,
        source_semantic_contexts,
        test_semantics,
        ..
    } = declarations;
    let expanded = composition.apply(authored);
    let (const_evaluation, const_diagnostics) =
        crate::const_eval::evaluate_program_with_diagnostics(&expanded);
    initial_diagnostics.extend(const_diagnostics);
    let make_checker = || {
        let mut checker = Checker::new(
            &expanded,
            const_evaluation.clone(),
            &source_texts,
            compilation_context.clone(),
            compilation_contexts.clone(),
            global_symbols.clone(),
            source_semantic_contexts.clone(),
            test_semantics.clone(),
        );
        checker.callable_body_program = super::CallableBodyProgram::Authored(authored);
        checker.composition = composition.clone();
        checker.contracts.conditions = conditions.clone();
        checker
            .contracts
            .conditions
            .extend(composition.conditions.clone());
        checker.contracts.callable_conditions = callable_conditions.clone();
        checker
            .contracts
            .callable_conditions
            .extend(composition.callable_conditions.clone());
        checker
    };

    let mut checker = super::callable_contracts::refined_checker(
        make_checker,
        Checker::check_conditional_program,
    );
    checker.check_conditional_program();
    super::collect_concrete_instances(&expanded, &mut checker);
    inspect(checker, initial_diagnostics)
}

impl Checker<'_> {
    fn collect_conditional_declaration_facts(
        &mut self,
    ) -> crate::trait_composition::CompositionPlan {
        self.predeclare_classes();
        self.collect_enums();
        for item in &self.program.items {
            let (owner, parameters, members, context) = match item {
                Item::Class(class) => {
                    self.type_parameter_scopes
                        .push(type_parameter_scope(&class.type_params));
                    for (entry, span) in class
                        .implements
                        .iter()
                        .zip(&class.syntax.inheritance_type_spans)
                    {
                        if let Some(condition) = &entry.condition {
                            self.resolve_nominal_condition(
                                *span,
                                condition,
                                &class.type_params,
                                &class.name,
                            );
                        }
                    }
                    self.type_parameter_scopes.pop();
                    (
                        &class.name,
                        &class.type_params,
                        &class.members,
                        PropertyHookContext::Class,
                    )
                }
                Item::Trait(declaration) => (
                    &declaration.name,
                    &declaration.type_params,
                    &declaration.members,
                    PropertyHookContext::Trait,
                ),
                _ => continue,
            };
            self.type_parameter_scopes
                .push(type_parameter_scope(parameters));
            for member in members {
                match member {
                    ClassMember::Method(method) if method_condition_allowed(method, true) => {
                        if let Some(condition) = &method.availability {
                            self.contracts
                                .callable_conditions
                                .insert(method.span, method.span);
                            let mut lexical_parameters = parameters.to_vec();
                            lexical_parameters.extend_from_slice(&method.type_params);
                            self.type_parameter_scopes
                                .push(type_parameter_scope(&method.type_params));
                            self.resolve_nominal_condition(
                                method.span,
                                condition,
                                &lexical_parameters,
                                owner,
                            );
                            self.type_parameter_scopes.pop();
                        }
                    }
                    ClassMember::Property(property)
                        if property_condition_allowed(property, context) =>
                    {
                        if let Some(condition) = &property.availability {
                            for hook in &property.hooks {
                                self.contracts
                                    .callable_conditions
                                    .insert(hook.span, property.span);
                            }
                            self.resolve_nominal_condition(
                                property.span,
                                condition,
                                parameters,
                                owner,
                            );
                        }
                    }
                    _ => {}
                }
            }
            self.type_parameter_scopes.pop();
        }
        let checked_conditions = self.check_condition_type_formations();
        self.check_class_conformance_types();
        self.collect_classes();
        self.collect_interface_declarations();
        self.collect_trait_declarations();
        self.check_condition_conformances(checked_conditions);
        self.check_pending_class_conformances();
        let (composition, diagnostics) =
            crate::trait_composition::prepare_authored_bodies(self.program, &self.contracts);
        self.diagnostics.extend(diagnostics);
        composition
    }

    fn check_conditional_program(&mut self) {
        self.predeclare_classes();
        self.collect_enums();
        self.collect_classes();
        self.collect_interface_declarations();
        self.collect_trait_declarations();
        self.check_class_conformance_types();
        self.collect_source_trait_signatures();
        self.validate_class_hierarchies();
        self.check_conditional_availability();
        self.check_callable_bodies();
        self.validate_checked_callable_contracts();
    }

    fn collect_source_trait_signatures(&mut self) {
        let declarations = self
            .composition
            .source_trait_members
            .iter()
            .map(|(owner, members)| (owner.clone(), members.clone()))
            .collect::<Vec<_>>();
        for (owner, members) in declarations {
            let Some(declaration) = self.trait_declaration(&owner) else {
                continue;
            };
            self.type_parameter_scopes
                .push(type_parameter_scope(&declaration.type_params));
            for member in members {
                match member {
                    ClassMember::Method(method) => {
                        let signature = self.resolve_function_signature(&method, Some(&owner));
                        self.function_signatures.insert(method.span, signature);
                    }
                    ClassMember::Property(property) => {
                        self.collect_property_hook_signatures(
                            &property,
                            &owner,
                            PropertyHookContext::Trait,
                        );
                    }
                    _ => {}
                }
            }
            self.type_parameter_scopes.pop();
        }
    }

    #[cfg(test)]
    pub(super) fn take_conditional_declaration_contracts(
        &mut self,
    ) -> super::contracts::ContractFacts {
        let mut contracts = super::contracts::ContractFacts {
            conditions: std::mem::take(&mut self.contracts.conditions),
            callable_conditions: std::mem::take(&mut self.contracts.callable_conditions),
            ..super::contracts::ContractFacts::default()
        };
        remove_source_view_conditions(&mut contracts, &self.composition);
        contracts
    }

    pub(super) fn project_composed_ambient_effects(
        &self,
        effects: &mut HashMap<Span, Vec<crate::types::ResolvedType>>,
    ) -> bool {
        if !matches!(
            self.callable_body_program,
            super::CallableBodyProgram::Authored(_)
        ) {
            return false;
        }
        self.project_composed_templates(effects)
    }

    /// Give each composed copy its authored trait member's profile.
    ///
    /// Transport projects only in the authored source view; executable checks
    /// recheck each copy against its composer's exact members. Authored reach
    /// projects in every view, because a composer may narrow a copy's
    /// transport but never what the trait body it copies may raise.
    pub(super) fn project_composed_templates(
        &self,
        effects: &mut HashMap<Span, Vec<crate::types::ResolvedType>>,
    ) -> bool {
        let mut changed = false;
        for (effective, template) in &self.composition.callable_templates {
            let projected = effects
                .get(&template.declaration)
                .into_iter()
                .flatten()
                .map(|effect| {
                    crate::types::substitute_trait_type(
                        effect,
                        &template.bindings,
                        &template.receiver,
                    )
                })
                .collect::<Vec<_>>();
            if projected.is_empty() {
                continue;
            }
            let target = effects.entry(*effective).or_default();
            for effect in projected {
                if !target.contains(&effect) {
                    target.push(effect);
                    changed = true;
                }
            }
        }
        changed
    }

    pub(super) fn project_composed_return_borrows(&mut self) -> bool {
        if !matches!(
            self.callable_body_program,
            super::CallableBodyProgram::Authored(_)
        ) {
            return false;
        }
        let projections = self
            .composition
            .callable_templates
            .iter()
            .filter_map(|(effective, template)| {
                Some((
                    *effective,
                    self.function_signatures
                        .get(&template.declaration)?
                        .return_borrow,
                    self.function_signatures.get(effective)?.clone(),
                ))
            })
            .collect::<Vec<_>>();
        let mut changed = false;
        for (effective, borrow, signature) in projections {
            let borrow =
                self.specialize_return_borrow(borrow, &signature.params, signature.return_ty);
            if signature.return_borrow == borrow {
                continue;
            }
            changed = true;
            self.function_signatures
                .get_mut(&effective)
                .unwrap()
                .return_borrow = borrow;
            for class in self.classes.values_mut() {
                for method in class.methods.values_mut() {
                    if method.declaration == effective {
                        method.return_borrow = borrow;
                    }
                }
            }
        }
        changed
    }

    fn resolve_nominal_condition(
        &mut self,
        declaration: Span,
        condition: &NominalCondition,
        parameters: &[TypeParamDecl],
        owner: &str,
    ) {
        let before = self.diagnostics.len();
        let parameters = parameters
            .iter()
            .map(|parameter| NominalTypeParameter {
                name: parameter.name.clone(),
                declaration: parameter.span,
            })
            .collect::<Vec<_>>();
        let mut names = HashSet::new();
        let unambiguous = parameters
            .iter()
            .all(|parameter| names.insert(parameter.name.as_str()));
        let mut obligations = Vec::new();
        for bound in &condition.bounds {
            let subject = parameters
                .iter()
                .find(|parameter| parameter.name == bound.subject.text);
            if subject.is_none() {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0778",
                        format!(
                            "`{}` must name a type parameter of this class, trait or method",
                            bound.subject.text
                        ),
                        bound.subject.span,
                    )
                    .with_title("Availability Subject Must Be A Type Parameter"),
                );
            }
            for (interface, span) in bound.interfaces.iter().zip(&bound.type_spans) {
                let interface = self.with_nominal_identity(|checker| {
                    checker.resolve_interface_edge_in_owner(interface, *span, Some(owner))
                });
                if let (Some(subject), Some(interface)) = (subject, interface) {
                    obligations.push(NominalObligation {
                        subject: subject.clone(),
                        subject_type: crate::types::ResolvedType::TypeParameter(
                            subject.name.clone(),
                        ),
                        subject_span: bound.subject.span,
                        interface: self.resolved_interface(&interface),
                        interface_span: *span,
                    });
                }
            }
        }
        if unambiguous && self.diagnostics.len() == before {
            let condition = NominalConditionFacts {
                clause: condition.span,
                parameters,
                obligations,
            };
            self.defer_condition_type_check(declaration, condition.clone());
            self.contracts.conditions.insert(declaration, condition);
        }
    }
}

#[derive(Default)]
pub(crate) struct DeclarationCheck {
    pub first_condition: Option<Span>,
    pub diagnostics: Vec<Diagnostic>,
}

pub(crate) fn check_declarations(program: &Program) -> DeclarationCheck {
    let mut check = DeclarationCheck::default();
    for item in &program.items {
        match item {
            Item::Class(class) => {
                for entry in &class.implements {
                    if let Some(condition) = &entry.condition {
                        check.first_condition.get_or_insert(condition.span);
                    }
                }
                members(&class.members, PropertyHookContext::Class, &mut check);
            }
            Item::Trait(declaration) => {
                members(&declaration.members, PropertyHookContext::Trait, &mut check);
            }
            Item::Interface(declaration) => {
                for method in &declaration.requirements {
                    method_condition(method, false, &mut check);
                }
                for property in &declaration.properties {
                    property_condition(property, PropertyHookContext::Interface, &mut check);
                }
            }
            Item::Function(function) => method_condition(function, false, &mut check),
            _ => {}
        }
    }
    check
}

fn members(members: &[ClassMember], context: PropertyHookContext, check: &mut DeclarationCheck) {
    for member in members {
        match member {
            ClassMember::Method(method) => method_condition(method, true, check),
            ClassMember::Property(property) => property_condition(property, context, check),
            _ => {}
        }
    }
}

fn method_condition(function: &FunctionDecl, member: bool, check: &mut DeclarationCheck) {
    let Some(condition) = &function.availability else {
        return;
    };
    check.first_condition.get_or_insert(condition.span);
    if !method_condition_allowed(function, member) {
        invalid(condition, "availability clauses apply to ordinary class or trait members; free functions, lifecycle methods and interface requirements remain unconditional", &mut check.diagnostics);
    }
}

fn property_condition(
    property: &PropertyDecl,
    context: PropertyHookContext,
    check: &mut DeclarationCheck,
) {
    let Some(condition) = &property.availability else {
        return;
    };
    check.first_condition.get_or_insert(condition.span);
    if !property_condition_allowed(property, context) {
        invalid(condition, "availability clauses require a computed class or trait property with no initializer or backing storage; stored properties and interface requirements remain unconditional", &mut check.diagnostics);
    }
}

fn method_condition_allowed(function: &FunctionDecl, member: bool) -> bool {
    member && !matches!(function.name.as_str(), "__construct" | "__destruct")
}

fn property_condition_allowed(property: &PropertyDecl, context: PropertyHookContext) -> bool {
    property.initializer.is_none()
        && declaration_facts(property, context)
            .is_some_and(|facts| facts.storage == Some(PropertyHookStorage::Computed))
}

fn invalid(condition: &NominalCondition, message: &str, diagnostics: &mut Vec<Diagnostic>) {
    diagnostics.push(
        Diagnostic::new("E0777", message, condition.span)
            .with_title("Availability Clause Is Not Allowed Here"),
    );
}

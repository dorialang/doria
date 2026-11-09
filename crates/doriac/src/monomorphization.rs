//! Reachable callable specializations shared by native and compatibility lowering.

use crate::class_layout::ClassId;
use crate::diagnostics::{Diagnostic, DiagnosticResult};
use crate::hir;
pub(crate) use crate::semantics::InterfaceGenericCall;
use crate::semantics::{GenericArgument, SemanticInfo};
use crate::source::Span;
use crate::types::{substitute_resolved_type, ClassType, ResolvedType};
use std::collections::HashMap;

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct CallableInstance {
    pub(crate) declaration: usize,
    pub(crate) arguments: Vec<GenericArgument>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CallableOrigin {
    Source,
    SyntheticConstructor,
}

#[derive(Clone, Copy)]
pub(crate) struct CallableDecl<'a> {
    pub(crate) origin: CallableOrigin,
    pub(crate) function: &'a hir::FunctionDecl,
    pub(crate) class: Option<ClassId>,
    pub(crate) receiver: Option<ClassId>,
    pub(crate) class_type_params: &'a [hir::TypeParamDecl],
    pub(crate) class_arguments: &'a [ResolvedType],
}

impl CallableDecl<'_> {
    pub(crate) fn is_top_level(self) -> bool {
        self.class.is_none()
    }
}

pub(crate) fn synthetic_constructors(program: &hir::Program) -> Vec<(String, hir::FunctionDecl)> {
    program
        .items
        .iter()
        .filter_map(|item| {
            let hir::Item::Class(class) = item else {
                return None;
            };
            if class.members.iter().any(|member| {
                matches!(member, hir::ClassMember::Method(method) if method.name == "__construct")
            }) {
                return None;
            }

            let mut checked_effects = Vec::new();
            for initializer in class.members.iter().filter_map(|member| match member {
                hir::ClassMember::Property(property) if !property.is_static => {
                    property.initializer.as_ref()
                }
                _ => None,
            }) {
                for (span, effects) in &program.semantic_info.checked_effect_sites {
                    if initializer.span().contains(*span) {
                        for effect in effects {
                            if !checked_effects.contains(effect) {
                                checked_effects.push(effect.clone());
                            }
                        }
                    }
                }
            }
            let effect_profile =
                crate::checked_effects::CheckedEffectProfile::classify(checked_effects.clone());
            let span = Span::in_source(class.span.source, class.span.start, class.span.start);
            Some((
                class.name.clone(),
                hir::FunctionDecl {
                    global_id: None,
                    source_identity: class.source_identity.clone(),
                    package: class.package.clone(),
                    access: hir::MemberAccess::External,
                    access_span: None,
                    is_open: false,
                    open_span: None,
                    is_override: false,
                    override_span: None,
                    writable_this: false,
                    consuming_this: false,
                    is_static: false,
                    name: "__construct".to_string(),
                    type_params: Vec::new(),
                    params: Vec::new(),
                    return_type: None,
                    throws: None,
                    checked_effects,
                    required_checked_effects: effect_profile.required,
                    ambient_checked_effects: effect_profile.ambient,
                    test_assertion_checked_effects: effect_profile.test_assertion,
                    body: hir::Block {
                        statements: Vec::new(),
                        span,
                    },
                    intrinsic: None,
                    modifier_prefix_span: span,
                    span,
                },
            ))
        })
        .collect::<Vec<_>>()
}

pub(crate) fn callable_declarations<'a>(
    program: &'a hir::Program,
    synthetic_constructors: &'a [(String, hir::FunctionDecl)],
) -> Vec<CallableDecl<'a>> {
    let mut declarations = Vec::new();

    for item in &program.items {
        match item {
            hir::Item::Function(function) => declarations.push(CallableDecl {
                origin: CallableOrigin::Source,
                function,
                class: None,
                receiver: None,
                class_type_params: &[],
                class_arguments: &[],
            }),
            hir::Item::Class(class_decl) => {
                for class_info in program
                    .semantic_info
                    .classes
                    .iter()
                    .filter(|info| info.declaration_name == class_decl.name)
                {
                    for member in &class_decl.members {
                        for method in member.callables() {
                            declarations.push(CallableDecl {
                                origin: CallableOrigin::Source,
                                function: method,
                                class: Some(class_info.id),
                                receiver: (!method.is_static).then_some(class_info.id),
                                class_type_params: &class_decl.type_params,
                                class_arguments: &class_info.arguments,
                            });
                        }
                    }
                    if let Some((_, constructor)) = synthetic_constructors
                        .iter()
                        .find(|(name, _)| name == &class_decl.name)
                    {
                        declarations.push(CallableDecl {
                            origin: CallableOrigin::SyntheticConstructor,
                            function: constructor,
                            class: Some(class_info.id),
                            receiver: Some(class_info.id),
                            class_type_params: &class_decl.type_params,
                            class_arguments: &class_info.arguments,
                        });
                    }
                }
            }
            hir::Item::Statement(_) => {}
            hir::Item::Enum(_) | hir::Item::Constant(_) => {}
        }
    }
    declarations.retain(|declaration| {
        !matches!(
            declaration.function.name.as_str(),
            "__construct" | "__destruct"
        ) || declaration.class.is_none_or(|id| {
            crate::inline_value::InlineValueKind::for_name(
                &program.semantic_info.classes[id.0].declaration_name,
            )
            .is_none()
        })
    });
    declarations
}

fn checked_instance(
    target: &crate::semantics::CallableInstanceKey,
    declarations: &[CallableDecl<'_>],
    site: Span,
) -> DiagnosticResult<CallableInstance> {
    let mut selected = None;
    for (index, declaration) in declarations.iter().enumerate() {
        if declaration.function.span != target.declaration()
            || declaration.origin != CallableOrigin::Source
        {
            continue;
        }
        if declaration.class_type_params.len() != declaration.class_arguments.len() {
            return Err(vec![Diagnostic::new(
                "I2401",
                "checked class has incomplete specialization arguments",
                site,
            )]);
        }
        if !declaration
            .class_type_params
            .iter()
            .zip(declaration.class_arguments)
            .all(|(parameter, argument)| target.binding(&parameter.name) == Some(argument))
        {
            continue;
        }
        let Some(arguments) = declaration
            .function
            .type_params
            .iter()
            .map(|parameter| {
                target
                    .binding(&parameter.name)
                    .cloned()
                    .map(GenericArgument::Type)
            })
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        if target.binding_count()
            != declaration.class_type_params.len() + declaration.function.type_params.len()
        {
            continue;
        }
        if selected.is_some() {
            return Err(vec![Diagnostic::new(
                "I2401",
                "checked callable instance has ambiguous declarations",
                site,
            )]);
        }
        selected = Some(CallableInstance {
            declaration: index,
            arguments,
        });
    }
    selected.ok_or_else(|| {
        vec![Diagnostic::new(
            "I2401",
            "checked callable instance has no declaration with its exact bindings",
            site,
        )]
    })
}

fn enqueue_checked_calls(
    calls: &[crate::semantics::CheckedGenericCall],
    declarations: &[CallableDecl<'_>],
    semantic_info: &SemanticInfo,
    instances: &mut Vec<CallableInstance>,
    ids: &mut HashMap<CallableInstance, usize>,
    interface_calls: &mut Vec<InterfaceGenericCall>,
) -> DiagnosticResult<()> {
    for call in calls {
        if let Some(interface) = &call.interface {
            if !interface_calls.contains(interface) {
                interface_calls.push(interface.clone());
            }
        } else if call.targets.is_empty() {
            return Err(vec![Diagnostic::new(
                "I2401",
                "checked generic call has no concrete callable target",
                call.site,
            )]);
        }
        for target in &call.targets {
            let target = checked_instance(target, declarations, call.site)?;
            if !callable_available(
                declarations[target.declaration],
                &target.arguments,
                semantic_info,
                call.site,
            )? {
                let message = if call.interface.is_some() {
                    "checked interface call selects an unavailable implementation"
                } else {
                    "checked generic call selects an unavailable callable"
                };
                return Err(vec![Diagnostic::new("I2401", message, call.site)]);
            }
            if !ids.contains_key(&target) {
                ids.insert(target.clone(), instances.len());
                instances.push(target);
            }
        }
    }
    Ok(())
}

pub(crate) fn collect_callable_instances(
    program: &hir::Program,
    declarations: &[CallableDecl<'_>],
    class_ids: &HashMap<ClassType<ResolvedType>, ClassId>,
    semantic_info: &SemanticInfo,
) -> DiagnosticResult<(Vec<CallableInstance>, Vec<InterfaceGenericCall>)> {
    validate_witness_admissions(declarations, class_ids, semantic_info)?;
    let mut instances = Vec::new();
    let mut interface_calls = Vec::new();
    let mut ids = HashMap::new();
    for (declaration, callable) in declarations.iter().enumerate() {
        if callable.function.type_params.is_empty() {
            if !callable_available(*callable, &[], semantic_info, callable.function.span)? {
                continue;
            }
            let instance = CallableInstance {
                declaration,
                arguments: Vec::new(),
            };
            ids.insert(instance.clone(), instances.len());
            instances.push(instance);
        }
    }
    enqueue_checked_calls(
        semantic_info
            .generic_call_graph
            .calls(None, Span::default())?,
        declarations,
        semantic_info,
        &mut instances,
        &mut ids,
        &mut interface_calls,
    )?;
    for item in &program.items {
        let hir::Item::Class(class) = item else {
            continue;
        };
        if !class.members.iter().any(|member| matches!(member, hir::ClassMember::Property(property) if property.initializer.is_some())) {
            continue;
        }
        for class_info in semantic_info
            .classes
            .iter()
            .filter(|info| info.declaration_name == class.name)
        {
            let substitutions = class
                .type_params
                .iter()
                .zip(&class_info.arguments)
                .map(|(parameter, argument)| (parameter.name.clone(), argument.clone()))
                .collect();
            let scope = crate::semantics::CallableInstanceKey::new(class.span, &substitutions);
            enqueue_checked_calls(
                semantic_info
                    .generic_call_graph
                    .calls(Some(scope), class.span)?,
                declarations,
                semantic_info,
                &mut instances,
                &mut ids,
                &mut interface_calls,
            )?;
        }
    }
    let mut cursor = 0;
    while cursor < instances.len() {
        let instance = instances[cursor].clone();
        cursor += 1;
        let declaration = declarations[instance.declaration];
        if declaration.origin == CallableOrigin::SyntheticConstructor {
            continue;
        }
        let substitutions = type_substitutions(declaration, &instance.arguments)?;
        let scope =
            crate::semantics::CallableInstanceKey::new(declaration.function.span, &substitutions);
        enqueue_checked_calls(
            semantic_info
                .generic_call_graph
                .calls(Some(scope), declaration.function.span)?,
            declarations,
            semantic_info,
            &mut instances,
            &mut ids,
            &mut interface_calls,
        )?;
    }
    Ok((instances, interface_calls))
}

fn validate_witness_admissions(
    declarations: &[CallableDecl<'_>],
    class_ids: &HashMap<ClassType<ResolvedType>, ClassId>,
    semantic_info: &SemanticInfo,
) -> DiagnosticResult<()> {
    for conformance in &semantic_info.contracts.conformances {
        if conformance.status != crate::semantics::contracts::ConformanceStatus::Checked {
            continue;
        }
        let ResolvedType::Class(class) = &conformance.implementing_type else {
            continue;
        };
        if !class_ids.contains_key(class) {
            continue;
        }
        for witness in semantic_info.interface_witnesses.conformance(conformance)? {
            // Generic implementations require the concrete call's method arguments.
            if !witness.parameters.is_empty() {
                continue;
            }
            let instance = witness.instance(&[], conformance.origin)?;
            let target = checked_instance(&instance, declarations, conformance.origin)?;
            if !callable_available(
                declarations[target.declaration],
                &[],
                semantic_info,
                conformance.origin,
            )? {
                return Err(vec![Diagnostic::new(
                    "I2401",
                    "checked interface witness selects an unavailable callable",
                    conformance.origin,
                )
                .with_related(
                    witness.declaration,
                    "the implementation is declared here",
                )]);
            }
        }
    }
    Ok(())
}

fn callable_available(
    declaration: CallableDecl<'_>,
    arguments: &[GenericArgument],
    semantic_info: &SemanticInfo,
    site: Span,
) -> DiagnosticResult<bool> {
    let guarded = semantic_info
        .contracts
        .callable_conditions
        .contains_key(&declaration.function.span);
    let virtual_method = semantic_info
        .method_hierarchy
        .get(&declaration.function.span)
        .is_some_and(|method| method.virtual_root.is_some());
    if !guarded && !virtual_method {
        return Ok(true);
    }
    let bindings = type_substitutions(declaration, arguments)?;
    let available = if guarded {
        semantic_info
            .callable_available(declaration.function.span, &bindings)
            .ok_or_else(|| {
                vec![Diagnostic::new(
                    "I2401",
                    "guarded callable has no checked admission inventory",
                    site,
                )]
            })?
    } else {
        true
    };
    if available && virtual_method {
        semantic_info.virtual_dispatch.family(
            &crate::semantics::CallableInstanceKey::new(declaration.function.span, &bindings),
            site,
        )?;
    }
    Ok(available)
}

pub(crate) fn type_substitutions(
    callable: CallableDecl<'_>,
    arguments: &[GenericArgument],
) -> DiagnosticResult<HashMap<String, crate::types::ResolvedType>> {
    let function = callable.function;
    if function.type_params.len() != arguments.len() {
        return Err(vec![Diagnostic::new(
            "I2401",
            format!(
                "generic function `{}` expected {} specialization arguments but received {}",
                function.name,
                function.type_params.len(),
                arguments.len()
            ),
            function.span,
        )]);
    }
    let mut substitutions = callable
        .class_type_params
        .iter()
        .zip(callable.class_arguments)
        .map(|(parameter, argument)| (parameter.name.clone(), argument.clone()))
        .collect::<HashMap<_, _>>();
    substitutions.extend(function.type_params.iter().zip(arguments).map(
        |(parameter, argument)| {
            let GenericArgument::Type(ty) = argument;
            (parameter.name.clone(), ty.clone())
        },
    ));
    Ok(substitutions)
}

pub(crate) fn substitute_generic_argument(
    argument: &GenericArgument,
    substitutions: &HashMap<String, crate::types::ResolvedType>,
) -> GenericArgument {
    match argument {
        GenericArgument::Type(ty) => {
            GenericArgument::Type(substitute_resolved_type(ty, substitutions))
        }
    }
}

//! Reachable callable specializations shared by native and compatibility lowering.

use crate::class_layout::ClassId;
use crate::diagnostics::{Diagnostic, DiagnosticResult};
use crate::hir;
use crate::semantics::{CallableTarget, GenericArgument, GenericSpecialization, SemanticInfo};
use crate::source::Span;
use crate::types::{
    resolved_type_complexity, resolved_type_is_symbolic, substitute_resolved_type, ClassType,
    ResolvedType,
};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct CallableInstance {
    pub(crate) declaration: usize,
    pub(crate) arguments: Vec<GenericArgument>,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct InterfaceGenericCall {
    pub interface: crate::types::InterfaceType<ResolvedType>,
    pub requirement: Span,
    pub arguments: Vec<GenericArgument>,
}

#[derive(Clone, Copy)]
pub(crate) struct CallableDecl<'a> {
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
                    if span.source == initializer.span().source
                        && span.start >= initializer.span().start
                        && span.end <= initializer.span().end
                    {
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
                        if let hir::ClassMember::Method(method) = member {
                            declarations.push(CallableDecl {
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
    declarations
}

#[allow(clippy::too_many_arguments)]
fn specialize_callable_instances(
    span: &Span,
    specialization: &GenericSpecialization,
    substitutions: &HashMap<String, ResolvedType>,
    functions: &HashMap<String, usize>,
    methods: &HashMap<(ClassId, String), usize>,
    class_ids: &HashMap<ClassType<ResolvedType>, ClassId>,
    semantic_info: &SemanticInfo,
    declarations: &[CallableDecl<'_>],
    interface_calls: &mut Vec<InterfaceGenericCall>,
) -> DiagnosticResult<Vec<CallableInstance>> {
    let Some(target) = semantic_info
        .call_targets
        .get(span)
        .and_then(|target| target.specialize(|ty| substitute_resolved_type(ty, substitutions)))
    else {
        return Err(vec![Diagnostic::new(
            "I2401",
            "checked generic call has no callable target",
            *span,
        )]);
    };
    let direct = match &target {
        CallableTarget::Function { name } => functions.get(name).copied(),
        CallableTarget::Method {
            class_type,
            method_name,
            ..
        } => class_ids
            .get(class_type)
            .and_then(|class| methods.get(&(*class, method_name.clone())))
            .copied(),
        CallableTarget::ConstrainedMethod { .. } => None,
        CallableTarget::InterfaceMethod { .. } => None,
    };
    let arguments = specialization
        .arguments
        .iter()
        .map(|argument| substitute_generic_argument(argument, substitutions))
        .collect::<Vec<_>>();
    if arguments.iter().any(generic_argument_is_symbolic) {
        return Err(vec![Diagnostic::new(
            "I2401",
            "generic specialization retained an unresolved type parameter",
            *span,
        )]);
    }
    if let Some(declaration) = direct {
        return Ok(vec![CallableInstance {
            declaration,
            arguments,
        }]);
    }
    let CallableTarget::InterfaceMethod {
        interface,
        requirement,
        method_name,
    } = target
    else {
        return Err(vec![Diagnostic::new(
            "I2401",
            "checked generic call has no callable declaration",
            *span,
        )]);
    };
    let mut instances = Vec::new();
    let call = InterfaceGenericCall {
        interface: interface.clone(),
        requirement,
        arguments: arguments.clone(),
    };
    if !interface_calls.contains(&call) {
        interface_calls.push(call);
    }
    for conformance in &semantic_info.contracts.conformances {
        if conformance.interface != interface
            || conformance.status != crate::semantics::contracts::ConformanceStatus::Checked
            || resolved_type_is_symbolic(&conformance.implementing_type)
        {
            continue;
        }
        let ResolvedType::Class(class) = &conformance.implementing_type else {
            continue;
        };
        let Some(class) = class_ids.get(class).copied() else {
            continue;
        };
        let implementation = conformance
            .implementations
            .iter()
            .find(|implementation| {
                implementation
                    .requirement_origins
                    .iter()
                    .any(|origin| origin.declaration == requirement)
            })
            .and_then(|implementation| implementation.implementation);
        let declaration = std::iter::once(class)
            .chain(
                semantic_info.classes[class.0]
                    .ancestors
                    .iter()
                    .filter_map(|class| class_ids.get(class).copied()),
            )
            .find_map(|class| methods.get(&(class, method_name.clone())).copied())
            .filter(|index| Some(declarations[*index].function.span) == implementation)
            .ok_or_else(|| {
                vec![Diagnostic::new(
                    "I2401",
                    "checked interface method has no concrete implementation declaration",
                    *span,
                )]
            })?;
        let instance = CallableInstance {
            declaration,
            arguments: arguments.clone(),
        };
        if !instances.contains(&instance) {
            instances.push(instance);
        }
    }
    Ok(instances)
}

pub(crate) fn collect_callable_instances(
    program: &hir::Program,
    declarations: &[CallableDecl<'_>],
    class_ids: &HashMap<ClassType<ResolvedType>, ClassId>,
    semantic_info: &SemanticInfo,
) -> DiagnosticResult<(Vec<CallableInstance>, Vec<InterfaceGenericCall>)> {
    let functions = declarations
        .iter()
        .enumerate()
        .filter(|(_, declaration)| declaration.is_top_level())
        .map(|(index, declaration)| (declaration.function.name.clone(), index))
        .collect::<HashMap<_, _>>();
    let methods = declarations
        .iter()
        .enumerate()
        .filter_map(|(index, declaration)| {
            declaration
                .class
                .map(|class| ((class, declaration.function.name.clone()), index))
        })
        .collect::<HashMap<_, _>>();

    let mut instances = Vec::new();
    let mut interface_calls = Vec::new();
    let mut parents = Vec::new();
    let mut ids = HashMap::new();
    for (declaration, callable) in declarations.iter().enumerate() {
        if callable.function.type_params.is_empty() {
            let instance = CallableInstance {
                declaration,
                arguments: Vec::new(),
            };
            ids.insert(instance.clone(), instances.len());
            instances.push(instance);
            parents.push(None);
        }
    }

    let mut calls = semantic_info
        .generic_call_specializations
        .iter()
        .collect::<Vec<_>>();
    calls.sort_by_key(|(span, _)| **span);

    for item in &program.items {
        let hir::Item::Class(class) = item else {
            continue;
        };
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
                .collect::<HashMap<_, _>>();
            for member in &class.members {
                let hir::ClassMember::Property(hir::PropertyDecl {
                    is_static: false,
                    initializer: Some(initializer),
                    ..
                }) = member
                else {
                    continue;
                };
                for (span, specialization) in &calls {
                    if span.source != initializer.span().source
                        || span.start < initializer.span().start
                        || span.end > initializer.span().end
                    {
                        continue;
                    }
                    for target in specialize_callable_instances(
                        span,
                        specialization,
                        &substitutions,
                        &functions,
                        &methods,
                        class_ids,
                        semantic_info,
                        declarations,
                        &mut interface_calls,
                    )? {
                        if !ids.contains_key(&target) {
                            ids.insert(target.clone(), instances.len());
                            instances.push(target);
                            parents.push(None);
                        }
                    }
                }
            }
        }
    }

    let mut cursor = 0;
    while cursor < instances.len() {
        let instance_index = cursor;
        let instance = instances[cursor].clone();
        cursor += 1;
        let callable = declarations[instance.declaration];
        let substitutions = type_substitutions(callable, &instance.arguments)?;
        for (span, specialization) in &calls {
            let in_function = span.source == callable.function.span.source
                && span.start >= callable.function.span.start
                && span.end <= callable.function.span.end;
            if !in_function {
                continue;
            }
            for target in specialize_callable_instances(
                span,
                specialization,
                &substitutions,
                &functions,
                &methods,
                class_ids,
                semantic_info,
                declarations,
                &mut interface_calls,
            )? {
                if !ids.contains_key(&target) {
                    if specialization_expands_recursively(
                        &instances,
                        &parents,
                        instance_index,
                        &target,
                    ) {
                        let name = &declarations[target.declaration].function.name;
                        return Err(vec![Diagnostic::new(
                        "E0539",
                        format!(
                            "generic specialization of `{name}` recursively expands its type arguments and has no finite monomorphization"
                        ),
                        **span,
                    )
                    .with_help(
                        "keep recursive generic calls at the same concrete type, or move the type-changing step outside the recursion",
                    )]);
                    }
                    ids.insert(target.clone(), instances.len());
                    instances.push(target);
                    parents.push(Some(instance_index));
                }
            }
        }
    }

    Ok((instances, interface_calls))
}

fn specialization_expands_recursively(
    instances: &[CallableInstance],
    parents: &[Option<usize>],
    current: usize,
    target: &CallableInstance,
) -> bool {
    // One type-changing recursive step can still converge (for example, T -> int).
    // Two consecutive increases for the same declaration establish an expanding
    // specialization chain while keeping bounded type changes valid.
    let mut matching_ancestors = Vec::new();
    let mut cursor = Some(current);
    while let Some(index) = cursor {
        if instances[index].declaration == target.declaration {
            matching_ancestors.push(&instances[index]);
            if matching_ancestors.len() == 2 {
                break;
            }
        }
        cursor = parents[index];
    }
    let [nearest, previous] = matching_ancestors.as_slice() else {
        return false;
    };
    specialization_complexity(&target.arguments) > specialization_complexity(&nearest.arguments)
        && specialization_complexity(&nearest.arguments)
            > specialization_complexity(&previous.arguments)
}

fn specialization_complexity(arguments: &[GenericArgument]) -> usize {
    arguments
        .iter()
        .map(|argument| {
            let GenericArgument::Type(ty) = argument;
            resolved_type_complexity(ty)
        })
        .sum()
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

fn generic_argument_is_symbolic(argument: &GenericArgument) -> bool {
    let GenericArgument::Type(ty) = argument;
    resolved_type_is_symbolic(ty)
}

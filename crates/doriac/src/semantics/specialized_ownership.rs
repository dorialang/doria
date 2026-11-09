//! Recheck ownership and provenance for concrete generic bodies.

use super::*;
use crate::ownership::{self, OwnershipAnalysisContext};
use std::borrow::Cow;
use std::sync::Arc;

pub(crate) use super::callable_contracts::Instance;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Facts {
    bindings: HashMap<BindingId, BindingOwnership>,
    closures: HashMap<ClosureId, ownership::ClosureOwnershipInfo>,
    retained_callables: HashMap<Span, ownership::RetainedCallableInfo>,
    writable_borrow_sources: HashSet<Span>,
}

pub(crate) type FactsByInstance = HashMap<Instance, Arc<Facts>>;

impl SemanticInfo {
    pub(crate) fn with_callable_ownership(
        &self,
        declaration: Span,
        substitutions: &HashMap<String, ResolvedType>,
    ) -> Cow<'_, Self> {
        let instance = Instance::new(declaration, substitutions);
        let facts = self.specialized_ownership.get(&instance);
        let returned = self.specialized_return_borrows.get(&instance);
        if facts.is_none() && returned.is_none() {
            return Cow::Borrowed(self);
        }
        let mut info = self.clone();
        if let Some(returned) = returned {
            match returned {
                Some(borrow) => {
                    info.return_borrows.insert(declaration, *borrow);
                }
                None => {
                    info.return_borrows.remove(&declaration);
                }
            }
        }
        let Some(facts) = facts else {
            return Cow::Owned(info);
        };
        for (id, ownership) in &facts.bindings {
            if let Some(binding) = info.binding_resolution.declarations_by_id.get_mut(id) {
                binding.ownership = *ownership;
            }
        }
        info.closure_ownership.extend(facts.closures.clone());
        info.retained_callables
            .extend(facts.retained_callables.clone());
        info.writable_borrow_sources
            .retain(|span| !declaration.contains(*span));
        info.writable_borrow_sources
            .extend(&facts.writable_borrow_sources);
        Cow::Owned(info)
    }
}

fn is_opaque_runtime_class(ty: &ResolvedType) -> bool {
    let ResolvedType::Class(class) = ty else {
        return false;
    };
    // Compiler-known identities cannot be redeclared, so the canonical name
    // identifies the runtime payload.
    class.arguments.is_empty()
        && [
            crate::compiler_known_stream::SOURCE_ID,
            crate::compiler_known_concurrency::SOURCE_ID,
        ]
        .into_iter()
        .any(|source| {
            crate::mir::RuntimeClassPayload::for_declaration(&class.name, source).is_some()
        })
}

pub(super) fn analyze(
    program: &Program,
    body_program: CallableBodyProgram<'_>,
    context: &OwnershipAnalysisContext<'_>,
    instances: &[SemanticCallableInstance],
    types: &TypeRegistry,
) -> (FactsByInstance, ownership::OwnershipAnalysis) {
    let mut functions = HashMap::new();
    for item in &program.items {
        match item {
            Item::Function(function) => {
                functions.insert(function.span, Cow::Borrowed(function));
            }
            Item::Class(class) => {
                for member in &class.members {
                    for callable in crate::property_hooks::member_callables(
                        member,
                        crate::property_hooks::PropertyHookContext::Class,
                    ) {
                        functions.insert(callable.span, callable);
                    }
                }
            }
            Item::Trait(declaration) => {
                for member in &declaration.members {
                    for callable in crate::property_hooks::member_callables(
                        member,
                        crate::property_hooks::PropertyHookContext::Trait,
                    ) {
                        functions.insert(callable.span, callable);
                    }
                }
            }
            _ => {}
        }
    }
    let mut facts = HashMap::new();
    let mut result = ownership::OwnershipAnalysis::default();
    for instance in instances {
        let substitutions = instance
            .bindings
            .iter()
            .map(|(name, ty)| (name.clone(), types.resolved(*ty)))
            .collect::<HashMap<_, _>>();
        let copy = |ty: &ResolvedType| {
            !ownership::resolved_type_requires_conservative_move(ty)
                && !ownership::resolved_type_is_move_type(ty, context.move_enum_names)
        };
        let body = callable_contracts::BodyInstance::new(
            body_program,
            instance.declaration,
            substitutions.clone(),
            context.composition,
        );
        let declaration = body.declaration;
        if substitutions.is_empty() && declaration == instance.declaration {
            continue;
        }
        // A compiler-owned runtime payload (a file, readiness set, transfer
        // plan or group, cancellation source) is an opaque owned value with
        // no Doria-visible fields: exactly how the template already treats a
        // type parameter. Specializing for it changes no ownership fact, and
        // lookups fall back to the template's facts.
        if declaration == instance.declaration
            && substitutions.values().all(is_opaque_runtime_class)
        {
            continue;
        }
        let Some(function) = functions.get(&declaration) else {
            continue;
        };

        let substitute = |ty: &ResolvedType| body.substitute(ty);
        let mut resolved_types = context.resolved_types.clone();
        let mut conversion_types = context.interface_conversion_types.clone();
        for map in [&mut resolved_types, &mut conversion_types] {
            for (span, ty) in map {
                if declaration.contains(*span) {
                    *ty = substitute(ty);
                }
            }
        }
        let mut bindings = context.binding_resolution.clone();
        for binding in bindings.declarations_by_id.values_mut() {
            if binding.span.is_some_and(|span| declaration.contains(span)) {
                if let Some(ty) = &mut binding.source_type {
                    *ty = substitute(ty);
                }
            }
        }
        let mut return_borrows = context.return_borrows.clone();
        if let Some(borrow) = super::specialized_return_borrow(
            context.specialized_return_borrows,
            instance.declaration,
            &substitutions,
        ) {
            return_borrows.insert(declaration, borrow);
        } else if let Some(Some(borrow)) = return_borrows.get_mut(&declaration) {
            if borrow.kind == ReturnBorrowKind::Value {
                if let BorrowSource::Parameter(index) = borrow.source {
                    let source = function
                        .params
                        .get(index)
                        .and_then(|param| bindings.declaration_by_span.get(&param.span))
                        .and_then(|id| bindings.declarations_by_id.get(id))
                        .and_then(|binding| binding.source_type.as_ref());
                    if source.is_some_and(copy) {
                        return_borrows.insert(declaration, None);
                    }
                }
            }
        }
        let mut closures = context.closures.clone();
        for closure in closures
            .values_mut()
            .filter(|closure| declaration.contains(closure_span(closure.closure_id)))
        {
            closure.function_type = substitute(&closure.function_type);
            closure.execution_function_type = substitute(&closure.execution_function_type);
            closure.inferred_return_type = substitute(&closure.inferred_return_type);
            for capture in &mut closure.captures {
                capture.source_type = substitute(&capture.source_type);
            }
            for effects in [
                &mut closure.inferred_checked_effects,
                &mut closure.required_checked_effects,
                &mut closure.ambient_checked_effects,
                &mut closure.test_assertion_checked_effects,
            ] {
                for effect in effects {
                    *effect = substitute(effect);
                }
            }
        }
        let mut calls = context.callable_value_calls.clone();
        for (span, call) in &mut calls {
            if declaration.contains(*span) {
                call.function_type = substitute(&call.function_type);
                call.return_type = substitute(&call.return_type);
                for effects in [
                    &mut call.checked_effects,
                    &mut call.required_checked_effects,
                    &mut call.ambient_checked_effects,
                    &mut call.test_assertion_checked_effects,
                ] {
                    for effect in effects {
                        *effect = substitute(effect);
                    }
                }
            }
        }
        let mut targets = context.call_targets.clone();
        for (span, target) in &mut targets {
            if declaration.contains(*span) {
                if let Some(specialized) = target.specialize(substitute) {
                    *target = specialized;
                }
            }
        }
        let mut accessors = context.property_accessor_calls.clone();
        for (span, calls) in &mut accessors {
            if !declaration.contains(*span) {
                continue;
            }
            for accessor in calls.getter.iter_mut().chain(calls.setter.iter_mut()) {
                accessor.declaring_type = substitute(&accessor.declaring_type);
                accessor.return_type = substitute(&accessor.return_type);
                if copy(&accessor.return_type)
                    && accessor
                        .return_borrow
                        .is_some_and(|borrow| borrow.kind == ReturnBorrowKind::Value)
                {
                    accessor.return_borrow = None;
                }
                if let Some(target) = accessor.target.specialize(substitute) {
                    accessor.target = target;
                }
                if let Some(parameter) = &mut accessor.parameter {
                    parameter.r#type = substitute(&parameter.r#type);
                }
                for effect in &mut accessor.checked_effects {
                    *effect = substitute(effect);
                }
            }
        }
        let mut algorithms = context.list_algorithm_calls.clone();
        for (span, call) in &mut algorithms {
            if !declaration.contains(*span) {
                continue;
            }
            for ty in [
                &mut call.receiver_type,
                &mut call.element_type,
                &mut call.result_type,
                &mut call.callback_type,
            ] {
                *ty = substitute(ty);
            }
            if let Some(ty) = &mut call.accumulator_type {
                *ty = substitute(ty);
            }
            for effects in [
                &mut call.checked_effects,
                &mut call.required_checked_effects,
                &mut call.ambient_checked_effects,
                &mut call.test_assertion_checked_effects,
            ] {
                for effect in effects {
                    *effect = substitute(effect);
                }
            }
        }
        let mut generic_calls = context.generic_call_specializations.clone();
        for (span, call) in &mut generic_calls {
            if declaration.contains(*span) {
                for GenericArgument::Type(ty) in &mut call.arguments {
                    *ty = substitute(ty);
                }
            }
        }
        // Authored identities are shared by different composing receivers. Each
        // body owns its local summaries; only callee summaries cross instances.
        let retained = context
            .retained_callable_seed
            .iter()
            .filter(|(span, _)| !declaration.contains(**span))
            .map(|(span, summary)| (*span, summary.clone()))
            .collect();
        let analysis = ownership::check_callable_with_inferred_move_returns(
            program,
            &OwnershipAnalysisContext {
                resolved_types: &resolved_types,
                interface_conversion_types: &conversion_types,
                binding_resolution: &bindings,
                return_borrows: &return_borrows,
                closures: &closures,
                callable_value_calls: &calls,
                call_targets: &targets,
                property_accessor_calls: &accessors,
                list_algorithm_calls: &algorithms,
                generic_call_specializations: &generic_calls,
                retained_callable_seed: &retained,
                ..*context
            },
            declaration,
        );
        for diagnostic in analysis.diagnostics {
            if !result.diagnostics.contains(&diagnostic) {
                result.diagnostics.push(diagnostic);
            }
        }
        result
            .writeback_constraints
            .extend(analysis.writeback_constraints);
        let binding_facts = analysis
            .binding_ownership
            .into_iter()
            .filter(|(id, _)| {
                bindings.declarations_by_id.get(id).is_some_and(|binding| {
                    binding.span.is_some_and(|span| declaration.contains(span))
                })
            })
            .collect();
        facts.insert(
            Instance::new(instance.declaration, &substitutions),
            Arc::new(Facts {
                bindings: binding_facts,
                closures: analysis
                    .closures
                    .into_iter()
                    .filter(|(id, _)| declaration.contains(closure_span(*id)))
                    .collect(),
                retained_callables: analysis
                    .retained_callables
                    .into_iter()
                    .filter(|(span, _)| declaration.contains(*span))
                    .collect(),
                writable_borrow_sources: analysis
                    .writable_borrow_sources
                    .into_iter()
                    .filter(|span| declaration.contains(*span))
                    .collect(),
            }),
        );
    }
    (facts, result)
}

pub(super) fn composed_retained_summaries(
    program: &Program,
    body_program: CallableBodyProgram<'_>,
    context: &OwnershipAnalysisContext<'_>,
    types: &TypeRegistry,
) -> HashMap<Span, ownership::RetainedCallableInfo> {
    let mut instances = context
        .composition
        .callable_templates
        .keys()
        .map(|declaration| SemanticCallableInstance {
            declaration: *declaration,
            bindings: Vec::new(),
        })
        .collect::<Vec<_>>();
    instances.sort_by_key(|instance| instance.declaration);
    let (facts, _) = analyze(program, body_program, context, &instances, types);
    instances
        .into_iter()
        .filter_map(|instance| {
            let declaration =
                context.composition.callable_templates[&instance.declaration].declaration;
            facts
                .get(&Instance::new(instance.declaration, &HashMap::new()))
                .and_then(|facts| facts.retained_callables.get(&declaration))
                .cloned()
                .map(|summary| (instance.declaration, summary))
        })
        .collect()
}

fn closure_span(id: ClosureId) -> Span {
    Span {
        source: id.source,
        expansion: id.expansion,
        start: id.start,
        end: id.end,
    }
}

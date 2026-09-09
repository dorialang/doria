use std::collections::{HashMap, HashSet};

use crate::ast::ClosureCaptureMode;
use crate::hir::{self, ClassMember, Expr, Item};
use crate::mir;
use crate::ownership::CaptureAcquisitionKind;
use crate::source::Span;
use crate::symbols::{BindingId, BorrowSource, ClosureId, LexicalOwner};
use crate::types::{FunctionTypeParameterMode, ResolvedType};

#[derive(Debug, Clone)]
pub(crate) struct PhpClosureDescriptor {
    pub(crate) closure_id: ClosureId,
    pub(crate) source_instance: mir::ClosureOwner,
    pub(crate) descriptor: mir::ClosureDescriptorId,
    pub(crate) function_type: mir::FunctionTypeId,
    pub(crate) environment_layout: Option<mir::ClosureEnvironmentLayoutId>,
    pub(crate) invocation_mode: mir::FunctionInvocationMode,
    pub(crate) helper_name: String,
    pub(crate) carrier_name: String,
    pub(crate) environment_name: Option<String>,
    pub(crate) owner_class: Option<String>,
    pub(crate) debug_identity: String,
}

#[derive(Debug, Clone)]
pub(crate) struct PhpClosurePlan {
    pub(crate) descriptors: HashMap<mir::ClosureDescriptorId, PhpClosureDescriptor>,
    pub(crate) layouts: HashMap<mir::ClosureEnvironmentLayoutId, mir::ClosureEnvironmentLayout>,
    pub(crate) function_types: HashMap<mir::FunctionTypeId, mir::FunctionType>,
    pub(crate) cell_bindings: HashSet<BindingId>,
    pub(crate) binding_homes: HashSet<BindingId>,
    pub(crate) binding_resolution: crate::symbols::BindingResolution,
    pub(crate) closures: HashMap<ClosureId, hir::ClosureExpression>,
    pub(crate) semantic_closures: HashMap<ClosureId, crate::semantics::ClosureSemanticInfo>,
    pub(crate) ownership: HashMap<ClosureId, crate::ownership::ClosureOwnershipInfo>,
    pub(crate) callable_value_calls: HashMap<Span, crate::semantics::CallableValueCallInfo>,
    pub(crate) property_write_types: HashMap<Span, ResolvedType>,
    pub(crate) callables: HashMap<Span, PhpCallablePlan>,
    pub(crate) source_callables: HashMap<Span, PhpCallablePlan>,
    pub(crate) call_targets: HashMap<Span, Span>,
    pub(crate) call_site_plans: HashMap<Span, PhpCallablePlan>,
}

#[derive(Debug, Clone)]
pub(crate) struct PhpCallableParameter {
    pub(crate) name: String,
    pub(crate) cell: bool,
    pub(crate) take: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct PhpCallablePlan {
    pub(crate) parameters: Vec<PhpCallableParameter>,
    pub(crate) returns_borrow: bool,
}

impl PhpClosurePlan {
    pub(crate) fn build(program: &hir::Program, mir: Option<&mir::Program>) -> Self {
        let callable_classes = callable_classes(program);
        let owner_classes = closure_owner_classes(program, &callable_classes);
        let used_class_names = program
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Class(class) => Some(class.name.clone()),
                _ => None,
            })
            .collect::<HashSet<_>>();
        let mut allocated_names = used_class_names;
        let mut used_helpers = callable_names(program);
        let mut descriptors = HashMap::new();

        for descriptor in mir
            .into_iter()
            .flat_map(|program| program.closure_descriptors.iter())
        {
            let suffix = descriptor.id.0;
            let carrier_name =
                allocate_class_name(format!("__DoriaClosureValue{suffix}"), &mut allocated_names);
            let environment_name = descriptor.environment_layout.map(|_| {
                allocate_class_name(
                    format!("__DoriaClosureEnvironment{suffix}"),
                    &mut allocated_names,
                )
            });
            let owner_class = owner_classes
                .get(&descriptor.source_closure)
                .cloned()
                .flatten();
            let helper_name = allocate_helper_name(
                format!("__doriaClosureEntry{suffix}"),
                owner_class.as_deref(),
                &mut used_helpers,
            );
            descriptors.insert(
                descriptor.id,
                PhpClosureDescriptor {
                    closure_id: descriptor.source_closure,
                    source_instance: descriptor.source_instance,
                    descriptor: descriptor.id,
                    function_type: descriptor.function_type,
                    environment_layout: descriptor.environment_layout,
                    invocation_mode: descriptor.invocation_mode,
                    helper_name,
                    carrier_name,
                    environment_name,
                    owner_class,
                    debug_identity: descriptor.debug_identity.clone(),
                },
            );
        }

        let mut cell_bindings = program
            .semantic_info
            .binding_resolution
            .declarations_by_id
            .values()
            .filter(|declaration| {
                declaration
                    .source_type
                    .as_ref()
                    .is_some_and(requires_owned_cell)
                    && (declaration.ownership == crate::symbols::BindingOwnership::Owned
                        || declaration.writable)
            })
            .map(|declaration| declaration.id)
            .collect::<HashSet<_>>();

        for closure in program.semantic_info.closures.values() {
            let ownership = program
                .semantic_info
                .closure_ownership
                .get(&closure.closure_id);
            for capture in &closure.captures {
                let needs_place = ownership
                    .and_then(|ownership| {
                        ownership.acquisitions.iter().find(|acquisition| {
                            acquisition.environment_binding_id == capture.environment_binding_id
                        })
                    })
                    .is_some_and(|acquisition| {
                        matches!(
                            acquisition.kind,
                            CaptureAcquisitionKind::ReadonlyLease
                                | CaptureAcquisitionKind::WritableLease
                                | CaptureAcquisitionKind::MoveIntoEnvironment
                        )
                    })
                    || capture.mode != ClosureCaptureMode::Take;
                if needs_place {
                    cell_bindings.insert(capture.source_binding_id);
                }
            }
        }

        mark_parameter_home_bindings(program, &mut cell_bindings);
        let binding_homes = cell_bindings.clone();
        let (source_callables, callables) = collect_callable_plans(program, &cell_bindings);
        let call_targets = collect_call_targets(program);
        mark_call_argument_places(program, &callables, &call_targets, &mut cell_bindings);

        let property_write_types = program
            .semantic_info
            .property_writes
            .iter()
            .filter_map(|(span, write)| {
                program
                    .semantic_info
                    .classes
                    .iter()
                    .find(|class| class.name == write.class_name)
                    .and_then(|class| {
                        class
                            .properties
                            .iter()
                            .find(|property| property.name == write.property_name)
                    })
                    .map(|property| (*span, property.ty.clone()))
            })
            .collect();

        Self {
            descriptors,
            layouts: mir
                .into_iter()
                .flat_map(|program| program.closure_environment_layouts.iter())
                .cloned()
                .map(|layout| (layout.id, layout))
                .collect(),
            function_types: mir
                .into_iter()
                .flat_map(|program| program.function_types.iter())
                .cloned()
                .map(|function_type| (function_type.id, function_type))
                .collect(),
            cell_bindings,
            binding_homes,
            binding_resolution: program.semantic_info.binding_resolution.clone(),
            closures: collect_closures(program),
            semantic_closures: program.semantic_info.closures.clone(),
            ownership: program.semantic_info.closure_ownership.clone(),
            callable_value_calls: program.semantic_info.callable_value_calls.clone(),
            property_write_types,
            callables,
            source_callables,
            call_targets,
            call_site_plans: HashMap::new(),
        }
    }

    pub(crate) fn descriptor(
        &self,
        closure: ClosureId,
        owner: Option<mir::ClosureOwner>,
    ) -> &PhpClosureDescriptor {
        let mut matches = self.descriptors.values().filter(|descriptor| {
            descriptor.closure_id == closure
                && owner.is_none_or(|owner| descriptor.source_instance == owner)
        });
        let descriptor = matches
            .next()
            .expect("validated MIR must describe every checked closure instance");
        assert!(
            matches.next().is_none(),
            "closure selection requires its concrete owner instance"
        );
        descriptor
    }

    pub(crate) fn callable_at(&self, span: crate::source::Span) -> Option<&PhpCallablePlan> {
        if let Some(plan) = self.call_site_plans.get(&span) {
            return Some(plan);
        }
        self.call_targets
            .get(&span)
            .and_then(|target| self.callables.get(target))
    }

    pub(crate) fn callable_definition(&self, span: Span) -> Option<&PhpCallablePlan> {
        self.callables.get(&span)
    }

    pub(crate) fn function_type(&self, id: mir::FunctionTypeId) -> &mir::FunctionType {
        self.function_types
            .get(&id)
            .expect("validated MIR must describe every closure function type")
    }

    pub(crate) fn layout(
        &self,
        id: mir::ClosureEnvironmentLayoutId,
    ) -> &mir::ClosureEnvironmentLayout {
        self.layouts
            .get(&id)
            .expect("validated MIR must describe every closure environment")
    }
}

fn collect_closures(program: &hir::Program) -> HashMap<ClosureId, hir::ClosureExpression> {
    let mut closures = HashMap::new();
    hir::visit::expressions(program, &mut |expr| {
        if let Expr::Closure(closure) = expr {
            closures.insert(closure.closure_id, (**closure).clone());
        }
    });
    closures
}

fn allocate_class_name(base: String, used: &mut HashSet<String>) -> String {
    if used.insert(base.clone()) {
        return base;
    }
    for suffix in 1.. {
        let candidate = format!("{base}_{suffix}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!("a collision-free generated PHP class name must exist")
}

fn callable_names(program: &hir::Program) -> HashMap<Option<String>, HashSet<String>> {
    let mut names = HashMap::new();
    for item in &program.items {
        match item {
            Item::Function(function) => {
                names
                    .entry(None)
                    .or_insert_with(HashSet::new)
                    .insert(function.name.clone());
            }
            Item::Class(class) => {
                let members = names
                    .entry(Some(class.name.clone()))
                    .or_insert_with(HashSet::new);
                for member in &class.members {
                    if let ClassMember::Method(method) = member {
                        members.insert(method.name.clone());
                    }
                }
            }
            Item::Enum(_) | Item::Constant(_) | Item::Statement(_) => {}
        }
    }
    names
}

fn allocate_helper_name(
    base: String,
    owner: Option<&str>,
    used: &mut HashMap<Option<String>, HashSet<String>>,
) -> String {
    let namespace = used.entry(owner.map(str::to_string)).or_default();
    if namespace.insert(base.clone()) {
        return base;
    }
    for suffix in 1.. {
        let candidate = format!("{base}_{suffix}");
        if namespace.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!("a collision-free generated PHP helper name must exist")
}

fn callable_classes(program: &hir::Program) -> HashMap<crate::source::Span, String> {
    program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Class(class) => Some(class),
            _ => None,
        })
        .flat_map(|class| {
            class.members.iter().filter_map(|member| match member {
                ClassMember::Method(method) => Some((method.span, class.name.clone())),
                _ => None,
            })
        })
        .collect()
}

fn closure_owner_classes(
    program: &hir::Program,
    callable_classes: &HashMap<crate::source::Span, String>,
) -> HashMap<ClosureId, Option<String>> {
    program
        .semantic_info
        .binding_resolution
        .closure_owners
        .keys()
        .copied()
        .map(|closure| {
            let mut owner = LexicalOwner::Closure(closure);
            let class = loop {
                owner = match program
                    .semantic_info
                    .binding_resolution
                    .lexical_parents
                    .get(&owner)
                    .copied()
                {
                    Some(parent) => parent,
                    None => break None,
                };
                match owner {
                    LexicalOwner::Callable(span) => {
                        break callable_classes.get(&span).cloned();
                    }
                    LexicalOwner::TopLevel => break None,
                    LexicalOwner::Closure(_) => {}
                }
            };
            (closure, class)
        })
        .collect()
}

fn mark_parameter_home_bindings(program: &hir::Program, cells: &mut HashSet<BindingId>) {
    let resolution = &program.semantic_info.binding_resolution;
    for item in &program.items {
        match item {
            Item::Function(function) => {
                mark_callable_parameter_homes(function, program, cells);
            }
            Item::Class(class) => {
                for member in &class.members {
                    if let ClassMember::Method(method) = member {
                        mark_callable_parameter_homes(method, program, cells);
                    }
                }
            }
            Item::Enum(_) | Item::Constant(_) | Item::Statement(_) => {}
        }
    }

    let _ = resolution;
}

fn mark_callable_parameter_homes(
    function: &hir::FunctionDecl,
    program: &hir::Program,
    cells: &mut HashSet<BindingId>,
) {
    let Some(borrow) = program.semantic_info.return_borrows.get(&function.span) else {
        return;
    };
    let BorrowSource::Parameter(index) = borrow.source else {
        return;
    };
    let Some(param) = function.params.get(index) else {
        return;
    };
    if let Some(binding) = binding_declared_in_span(program, &param.name, param.span) {
        cells.insert(binding);
    }
}

fn mark_call_argument_places(
    program: &hir::Program,
    callables: &HashMap<Span, PhpCallablePlan>,
    call_targets: &HashMap<Span, Span>,
    cells: &mut HashSet<BindingId>,
) {
    hir::visit::expressions(program, &mut |expr| match expr {
        Expr::CallableCall(call) => {
            if let Some(ResolvedType::Function(function_type)) = program
                .semantic_info
                .callable_value_calls
                .get(&call.span)
                .map(|call| &call.function_type)
            {
                mark_mode_arguments(&call.args, &function_type.parameters, program, cells);
            }
        }
        Expr::FunctionCall { args, span, .. }
        | Expr::MethodCall { args, span, .. }
        | Expr::StaticCall { args, span, .. }
        | Expr::New { args, span, .. } => {
            if let Some(callable) = call_targets
                .get(span)
                .and_then(|target| callables.get(target))
            {
                mark_callable_arguments(args, callable, program, cells);
            }
        }
        _ => {}
    });
}

fn collect_callable_plans(
    program: &hir::Program,
    cells: &HashSet<BindingId>,
) -> (
    HashMap<Span, PhpCallablePlan>,
    HashMap<Span, PhpCallablePlan>,
) {
    let mut callables = HashMap::new();
    for item in &program.items {
        match item {
            Item::Function(function) => {
                callables.insert(function.span, callable_plan(function, program, cells));
            }
            Item::Class(class) => {
                for member in &class.members {
                    if let ClassMember::Method(method) = member {
                        callables.insert(method.span, callable_plan(method, program, cells));
                    }
                }
            }
            Item::Enum(_) | Item::Constant(_) | Item::Statement(_) => {}
        }
    }
    let source_callables = callables.clone();
    // An erased requirement and every checked implementation must agree on
    // the parameter home ABI, including homes needed for returned borrows.
    for interface in &program.semantic_info.contracts.interface_specializations {
        for requirement in &interface.requirements {
            let plan = PhpCallablePlan {
                returns_borrow: requirement.return_borrow.is_some(),
                parameters: requirement
                    .signature
                    .parameters
                    .iter()
                    .enumerate()
                    .map(|(index, parameter)| PhpCallableParameter {
                        name: parameter.name.clone(),
                        cell: (requires_owned_cell(&parameter.r#type)
                            && (parameter.take || parameter.writable))
                            || requirement.return_borrow.is_some_and(|borrow| {
                                borrow.source == BorrowSource::Parameter(index)
                            }),
                        take: parameter.take,
                    })
                    .collect(),
            };
            for origin in &requirement.origins {
                callables.insert(origin.declaration, plan.clone());
            }
        }
    }
    // A concrete body may require a parameter home even when the contract
    // doesn't. Propagate that representation throughout the conformance family.
    loop {
        let mut changed = false;
        for conformance in program
            .semantic_info
            .contracts
            .conformances
            .iter()
            .filter(|fact| fact.status == crate::semantics::contracts::ConformanceStatus::Checked)
        {
            for implementation in &conformance.implementations {
                let Some(body) = implementation.implementation else {
                    continue;
                };
                for origin in &implementation.requirement_origins {
                    let (Some(body_plan), Some(requirement_plan)) =
                        (callables.get(&body), callables.get(&origin.declaration))
                    else {
                        continue;
                    };
                    let cells = body_plan
                        .parameters
                        .iter()
                        .zip(&requirement_plan.parameters)
                        .map(|(body, requirement)| body.cell || requirement.cell)
                        .collect::<Vec<_>>();
                    for target in [body, origin.declaration] {
                        for (parameter, cell) in callables
                            .get_mut(&target)
                            .unwrap()
                            .parameters
                            .iter_mut()
                            .zip(&cells)
                        {
                            changed |= parameter.cell != *cell;
                            parameter.cell = *cell;
                        }
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    (source_callables, callables)
}

fn callable_plan(
    function: &hir::FunctionDecl,
    program: &hir::Program,
    cells: &HashSet<BindingId>,
) -> PhpCallablePlan {
    PhpCallablePlan {
        returns_borrow: program
            .semantic_info
            .return_borrows
            .contains_key(&function.span),
        parameters: function
            .params
            .iter()
            .map(|parameter| {
                let binding = binding_declared_in_span(program, &parameter.name, parameter.span);
                PhpCallableParameter {
                    name: parameter.name.clone(),
                    cell: binding.is_some_and(|binding| cells.contains(&binding)),
                    take: parameter.take,
                }
            })
            .collect(),
    }
}

fn collect_call_targets(program: &hir::Program) -> HashMap<Span, Span> {
    let functions = program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Function(function) => Some((function.name.clone(), function.span)),
            _ => None,
        })
        .collect::<HashMap<_, _>>();
    let methods = program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Class(class) => Some(class),
            _ => None,
        })
        .flat_map(|class| {
            class.members.iter().filter_map(|member| match member {
                ClassMember::Method(method) => {
                    Some(((class.name.clone(), method.name.clone()), method.span))
                }
                _ => None,
            })
        })
        .collect::<HashMap<_, _>>();

    program
        .semantic_info
        .call_targets
        .iter()
        .filter_map(|(span, target)| {
            let start = match target {
                crate::semantics::CallableTarget::Function { name } => functions.get(name),
                crate::semantics::CallableTarget::Method {
                    class_type,
                    method_name,
                    ..
                } => methods.get(&(class_type.name.clone(), method_name.clone())),
                // Generic specialization remains outside PHP compatibility coverage.
                crate::semantics::CallableTarget::ConstrainedMethod { .. } => None,
                crate::semantics::CallableTarget::InterfaceMethod { requirement, .. } => {
                    Some(requirement)
                }
            }?;
            Some((*span, *start))
        })
        .collect()
}

fn mark_callable_arguments(
    args: &[hir::Argument],
    callable: &PhpCallablePlan,
    program: &hir::Program,
    cells: &mut HashSet<BindingId>,
) {
    let mut next_positional = 0;
    for argument in args {
        let index = argument
            .name
            .as_ref()
            .and_then(|name| {
                callable
                    .parameters
                    .iter()
                    .position(|parameter| parameter.name == name.text)
            })
            .unwrap_or_else(|| {
                let index = next_positional;
                next_positional += 1;
                index
            });
        if callable
            .parameters
            .get(index)
            .is_some_and(|parameter| parameter.cell)
        {
            if let Some(binding) = binding_used_at(program, argument.value.span()) {
                cells.insert(binding);
            }
        }
    }
}

fn mark_mode_arguments(
    args: &[hir::Argument],
    parameters: &[crate::types::SemanticFunctionParameter<ResolvedType>],
    program: &hir::Program,
    cells: &mut HashSet<BindingId>,
) {
    for (argument, parameter) in args.iter().zip(parameters) {
        if parameter.ownership_mode == FunctionTypeParameterMode::Writable {
            if let Some(binding) = binding_used_at(program, argument.value.span()) {
                cells.insert(binding);
            }
        }
    }
}

pub(crate) fn binding_used_at(
    program: &hir::Program,
    span: crate::source::Span,
) -> Option<BindingId> {
    program
        .semantic_info
        .binding_resolution
        .uses_by_span
        .get(&span)
        .copied()
}

pub(crate) fn binding_declared_in_span(
    program: &hir::Program,
    name: &str,
    span: crate::source::Span,
) -> Option<BindingId> {
    program
        .semantic_info
        .binding_resolution
        .declarations_by_id
        .values()
        .find(|declaration| {
            declaration.name == name
                && declaration
                    .span
                    .is_some_and(|declared| span.contains(declared))
        })
        .map(|declaration| declaration.id)
}

pub(crate) fn requires_owned_cell(ty: &ResolvedType) -> bool {
    match ty {
        ResolvedType::Interface(_)
        | ResolvedType::InterfaceSelf(_)
        | ResolvedType::Error
        | ResolvedType::Class(_)
        | ResolvedType::Enum(_)
        | ResolvedType::Mixed
        | ResolvedType::SharedHandle(_, _)
        | ResolvedType::Function(_) => true,
        ResolvedType::Nullable(inner)
        | ResolvedType::TypedArray(inner)
        | ResolvedType::List(inner)
        | ResolvedType::Set(inner)
        | ResolvedType::SortedSet(inner)
        | ResolvedType::PriorityQueue(inner)
        | ResolvedType::Deque(inner) => requires_owned_cell(inner),
        ResolvedType::Dictionary(key, value) | ResolvedType::SortedDictionary(key, value) => {
            requires_owned_cell(key) || requires_owned_cell(value)
        }
        _ => false,
    }
}

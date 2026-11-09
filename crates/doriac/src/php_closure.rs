use std::collections::{HashMap, HashSet};

use crate::ast::ClosureCaptureMode;
use crate::hir::{self, ClassMember, Expr, Item};
use crate::mir;
use crate::ownership::CaptureAcquisitionKind;
use crate::source::Span;
use crate::symbols::{BindingId, BorrowSource, ClosureId, LexicalOwner};
use crate::types::{FunctionTypeParameterMode, ResolvedType};

mod argument_places;
pub(crate) use argument_places::argument_place_bindings;

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
    indirect_call_types: HashMap<(mir::ClosureOwner, Span), mir::FunctionTypeId>,
    pub(crate) callable_return_borrows: HashMap<mir::FunctionId, Option<mir::ReturnBorrow>>,
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
    pub(crate) consumes_receiver: bool,
    // True only when the returned carrier is borrowed. An owned closure may
    // retain source borrows without lending its carrier or environment.
    pub(crate) returns_borrow: bool,
    pub(crate) receiver_home: bool,
    pub(crate) lifecycle_receiver: bool,
}

impl PhpCallablePlan {
    pub(crate) fn caller_receiver_home(&self) -> bool {
        self.receiver_home && !self.lifecycle_receiver
    }
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
            .filter(|declaration| requires_binding_cell(declaration, &program.semantic_info.enums))
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

        // Returned parameter homes are selected again for each concrete callable.
        let binding_homes = cell_bindings.clone();
        mark_parameter_home_bindings(program, &mut cell_bindings);
        let (source_callables, callables) = collect_callable_plans(program, &cell_bindings);
        let call_targets = collect_call_targets(program);
        mark_call_argument_places(program, &callables, &call_targets, &mut cell_bindings);

        let property_initializers = program
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Class(class) => Some(class),
                _ => None,
            })
            .flat_map(|class| {
                class.members.iter().filter_map(move |member| match member {
                    ClassMember::Property(property) => property
                        .initializer
                        .as_ref()
                        .map(|value| ((class.name.as_str(), property.name.as_str()), value.span())),
                    _ => None,
                })
            })
            .collect::<HashMap<_, _>>();
        let mut indirect_call_types = HashMap::new();
        for function in mir.into_iter().flat_map(|program| &program.functions) {
            let owner = function
                .closure
                .as_ref()
                .map_or(mir::ClosureOwner::Callable(function.id), |closure| {
                    descriptors[&closure.descriptor].source_instance
                });
            for block in &function.blocks {
                let (span, function_type) = match &block.terminator {
                    mir::Terminator::IndirectCall {
                        callee: mir::IndirectCallee::Closure(_),
                        span,
                        function_type,
                        ..
                    }
                    | mir::Terminator::CheckedIndirectCall {
                        callee: mir::IndirectCallee::Closure(_),
                        span,
                        function_type,
                        ..
                    } => (*span, *function_type),
                    _ => continue,
                };
                indirect_call_types.insert((owner, span), function_type);
                if let Some(method) = function
                    .method
                    .as_ref()
                    .filter(|method| method.name == "__construct")
                {
                    for property in &program.semantic_info.classes[method.class.0].properties {
                        if property_initializers
                            .get(&(property.declaring_class.as_str(), property.name.as_str()))
                            .is_some_and(|initializer| {
                                initializer.source == span.source
                                    && initializer.start <= span.start
                                    && span.end <= initializer.end
                            })
                        {
                            indirect_call_types.insert(
                                (mir::ClosureOwner::PropertyInitializer(property.id), span),
                                function_type,
                            );
                        }
                    }
                }
            }
        }

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
            indirect_call_types,
            callable_return_borrows: mir
                .into_iter()
                .flat_map(|program| &program.functions)
                .map(|function| (function.id, function.return_borrow))
                .collect(),
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

    pub(crate) fn parameter_uses_cell(&self, function: mir::FunctionTypeId, index: usize) -> bool {
        parameter_uses_cell(self.function_type(function), index)
    }

    pub(crate) fn callable_value_type(
        &self,
        owner: Option<mir::ClosureOwner>,
        span: Span,
    ) -> &mir::FunctionType {
        let owner = owner.expect("callable invocation requires its concrete owner");
        let id = self
            .indirect_call_types
            .get(&(owner, span))
            .expect("validated MIR must describe every callable invocation");
        self.function_type(*id)
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

pub(crate) fn parameter_uses_cell(function: &mir::FunctionType, index: usize) -> bool {
    function.parameters[index].mode == mir::FunctionParameterMode::Writable
        || (function.parameters[index].mode == mir::FunctionParameterMode::Take
            && function.parameters[index].ty.requires_owned_cleanup())
        || function.return_borrow.is_some_and(|borrow| {
            borrow.kind == mir::ReturnBorrowKind::Value
                && !matches!(
                    function.return_type,
                    mir::ReturnType::Value(
                        mir::Type::ByteRegion(_) | mir::Type::NullableByteRegion(_)
                    )
                )
                && borrow.source == mir::BorrowSource::Parameter(index)
        })
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
                    for method in member.callables() {
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
            class
                .members
                .iter()
                .flat_map(ClassMember::callables)
                .map(|method| (method.span, class.name.clone()))
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
                    for method in member.callables() {
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
    if borrow.kind != crate::types::ReturnBorrowKind::Value
        || !borrowed_value_uses_cell(
            &program.semantic_info.callable_signatures[&function.span].return_type,
        )
    {
        return;
    }
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
                mark_mode_arguments(&call.args, function_type, program, cells);
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
                    for method in member.callables() {
                        callables.insert(method.span, callable_plan(method, program, cells));
                    }
                }
            }
            Item::Enum(_) | Item::Constant(_) | Item::Statement(_) => {}
        }
    }
    let mut source_callables = callables.clone();
    // An erased requirement and every checked implementation must agree on
    // the parameter home ABI, including homes needed for returned borrows.
    for interface in &program.semantic_info.contracts.interface_specializations {
        for requirement in &interface.requirements {
            let returns_borrow = requirement.return_borrow.is_some_and(|borrow| {
                borrow.kind == crate::types::ReturnBorrowKind::Value
                    && borrowed_value_uses_cell(&requirement.signature.return_type)
            });
            let plan = PhpCallablePlan {
                consumes_receiver: requirement.receiver_mode.is_consuming(),
                lifecycle_receiver: false,
                returns_borrow,
                receiver_home: requirement.return_borrow.is_some_and(|borrow| {
                    returns_borrow && borrow.source == BorrowSource::Receiver
                }),
                parameters: requirement
                    .signature
                    .parameters
                    .iter()
                    .enumerate()
                    .map(|(index, parameter)| PhpCallableParameter {
                        name: parameter.name.clone(),
                        cell: requires_parameter_cell(
                            &parameter.r#type,
                            parameter.take,
                            parameter.writable,
                            &program.semantic_info.enums,
                        ) || requirement.return_borrow.is_some_and(|borrow| {
                            returns_borrow && borrow.source == BorrowSource::Parameter(index)
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
                    let receiver_home = body_plan.receiver_home || requirement_plan.receiver_home;
                    for target in [body, origin.declaration] {
                        let plan = callables.get_mut(&target).unwrap();
                        changed |= plan.receiver_home != receiver_home;
                        plan.receiver_home = receiver_home;
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
        for method in program.semantic_info.method_hierarchy.values() {
            let Some(root) = method.virtual_root else {
                continue;
            };
            let (Some(body), Some(root_plan)) =
                (callables.get(&method.declaration), callables.get(&root))
            else {
                continue;
            };
            let home = body.receiver_home || root_plan.receiver_home;
            for target in [method.declaration, root] {
                let plan = callables.get_mut(&target).unwrap();
                changed |= plan.receiver_home != home;
                plan.receiver_home = home;
            }
        }
        let targets = collect_call_targets(program);
        let mut receiver_callers = HashSet::new();
        hir::visit::expressions(program, &mut |expr| {
            let (object, target) = match expr {
                Expr::MethodCall { object, span, .. } => {
                    (Some(object.as_ref()), targets.get(span).copied())
                }
                Expr::PropertyAccess { object, span, .. } => (
                    Some(object.as_ref()),
                    program
                        .semantic_info
                        .property_accessor_calls
                        .get(span)
                        .and_then(|calls| calls.getter.as_ref())
                        .map(|getter| getter.declaration),
                ),
                Expr::StaticCall { span, .. }
                    if program
                        .semantic_info
                        .method_call_targets
                        .get(span)
                        .is_some_and(|call| call.direct_parent) =>
                {
                    (None, targets.get(span).copied())
                }
                _ => return,
            };
            if !target
                .and_then(|target| callables.get(&target))
                .is_some_and(|plan| plan.receiver_home)
            {
                return;
            }
            let Some(mut object) = object else {
                if let Some(owner) = program
                    .semantic_info
                    .method_hierarchy
                    .keys()
                    .filter(|owner| owner.contains(expr.span()))
                    .min_by_key(|owner| owner.end - owner.start)
                {
                    receiver_callers.insert(*owner);
                }
                return;
            };
            while let Expr::Grouped { expr, .. } = object {
                object = expr;
            }
            if !matches!(object, Expr::This { .. }) {
                return;
            }
            let Some(binding) = binding_used_at(program, object.span()) else {
                return;
            };
            if let Some(declaration) = program
                .semantic_info
                .binding_resolution
                .declarations_by_id
                .get(&binding)
            {
                if let LexicalOwner::Callable(owner) = declaration.owner {
                    receiver_callers.insert(owner);
                }
            }
        });
        for caller in receiver_callers {
            if let Some(plan) = callables.get_mut(&caller) {
                changed |= !plan.receiver_home;
                plan.receiver_home = true;
            }
        }
        if !changed {
            break;
        }
    }
    for (span, source) in &mut source_callables {
        source.receiver_home = callables[span].receiver_home;
    }
    (source_callables, callables)
}

fn callable_plan(
    function: &hir::FunctionDecl,
    program: &hir::Program,
    cells: &HashSet<BindingId>,
) -> PhpCallablePlan {
    let returns_borrow = program
        .semantic_info
        .return_borrows
        .get(&function.span)
        .is_some_and(|borrow| {
            borrow.kind == crate::types::ReturnBorrowKind::Value
                && borrowed_value_uses_cell(
                    &program.semantic_info.callable_signatures[&function.span].return_type,
                )
        });
    PhpCallablePlan {
        consumes_receiver: function.consuming_this,
        lifecycle_receiver: matches!(function.name.as_str(), "__construct" | "__destruct")
            && program
                .semantic_info
                .binding_resolution
                .receiver_for_callable(function.span)
                .is_some(),
        receiver_home: program
            .semantic_info
            .return_borrows
            .get(&function.span)
            .is_some_and(|borrow| returns_borrow && borrow.source == BorrowSource::Receiver),
        returns_borrow,
        parameters: function
            .params
            .iter()
            .enumerate()
            .map(|(index, parameter)| {
                let binding = binding_declared_in_span(program, &parameter.name, parameter.span);
                let copies = program
                    .semantic_info
                    .callable_signatures
                    .get(&function.span)
                    .and_then(|signature| signature.parameters.get(index))
                    .is_some_and(|parameter| {
                        parameter_copies_value(&parameter.r#type, &program.semantic_info.enums)
                    });
                let lends = program
                    .semantic_info
                    .return_borrows
                    .get(&function.span)
                    .is_some_and(|borrow| {
                        returns_borrow && borrow.source == BorrowSource::Parameter(index)
                    });
                let retains_writable_place = program
                    .semantic_info
                    .retained_callables
                    .get(&function.span)
                    .is_some_and(|plan| {
                        plan.constructs.iter().any(|source| {
                            source.source == BorrowSource::Parameter(index)
                                && !source.inherited
                                && source.writable
                        })
                    });
                PhpCallableParameter {
                    name: parameter.name.clone(),
                    cell: lends
                        || retains_writable_place
                        || ((!copies || parameter.take)
                            && binding.is_some_and(|binding| cells.contains(&binding))),
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
            class
                .members
                .iter()
                .flat_map(ClassMember::callables)
                .map(|method| ((class.name.clone(), method.name.clone()), method.span))
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
            cells.extend(argument_place_bindings(program, &argument.value));
        }
    }
}

fn mark_mode_arguments(
    args: &[hir::Argument],
    function: &crate::types::SemanticFunctionType<ResolvedType>,
    program: &hir::Program,
    cells: &mut HashSet<BindingId>,
) {
    for (index, (argument, parameter)) in args.iter().zip(&function.parameters).enumerate() {
        if parameter.ownership_mode == FunctionTypeParameterMode::Writable
            || function_parameter_requires_return_home(function, index)
        {
            cells.extend(argument_place_bindings(program, &argument.value));
        }
    }
}

pub(crate) fn function_parameter_requires_return_home(
    function: &crate::types::SemanticFunctionType<ResolvedType>,
    index: usize,
) -> bool {
    function.return_borrow.is_some_and(|borrow| {
        borrow.kind == crate::types::ReturnBorrowKind::Value
            && borrowed_value_uses_cell(&function.return_type)
            && borrow.source == crate::types::FunctionBorrowSource::Parameter(index)
    })
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

pub(crate) fn place_root_binding(program: &hir::Program, expr: &Expr) -> Option<BindingId> {
    match expr {
        Expr::Grouped { expr, .. } => place_root_binding(program, expr),
        Expr::Index { collection, .. } => place_root_binding(program, collection),
        Expr::PropertyAccess { object, .. } => place_root_binding(program, object),
        _ => binding_used_at(program, expr.span()),
    }
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

pub(crate) fn requires_binding_cell(
    declaration: &crate::symbols::BindingDeclaration,
    enums: &[crate::semantics::EnumSemanticInfo],
) -> bool {
    let Some(ty) = &declaration.source_type else {
        return false;
    };
    let parameter = matches!(
        declaration.kind,
        crate::symbols::BindingKind::FunctionParameter
            | crate::symbols::BindingKind::MethodParameter
            | crate::symbols::BindingKind::ClosureParameter
    );
    let mut inner = ty;
    while let ResolvedType::Nullable(value) = inner {
        inner = value;
    }
    // Applications are classified per instance; a symbolic one keeps a cell.
    if let ResolvedType::Enum(ty) = inner {
        // Trivial Copy bindings, including named parameters, are independent
        // values. Capture and loan consumers mark actual shared homes below.
        if enums
            .iter()
            .find(|definition| definition.source_type == *ty)
            .is_some_and(|definition| definition.capabilities.trivial_copy)
        {
            return false;
        }
    }
    (requires_owned_cell(ty) && declaration.ownership == crate::symbols::BindingOwnership::Owned)
        || (declaration.ownership != crate::symbols::BindingOwnership::Owned
            && !parameter
            && requires_writable_cell(ty))
        || (declaration.writable && requires_writable_cell(ty))
}

pub(crate) fn requires_owned_cell(ty: &ResolvedType) -> bool {
    match ty {
        ResolvedType::Interface(_)
        | ResolvedType::InterfaceSelf(_)
        | ResolvedType::Error
        | ResolvedType::Enum(_)
        | ResolvedType::Mixed
        | ResolvedType::SharedHandle(_, _)
        | ResolvedType::Function(_) => true,
        ResolvedType::Class(class) => class
            .inline_copy_type()
            .is_none_or(|kind| kind.requires_owned_cleanup()),
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

pub(crate) fn parameter_copies_value(
    ty: &ResolvedType,
    enums: &[crate::semantics::EnumSemanticInfo],
) -> bool {
    match ty {
        ResolvedType::Nullable(inner) => parameter_copies_value(inner, enums),
        // Applications are classified per instance; a symbolic one is not Copy.
        ResolvedType::Enum(ty) => enums
            .iter()
            .find(|definition| definition.source_type == *ty)
            .is_some_and(|definition| definition.capabilities.copy),
        ResolvedType::TypeParameter(_)
        | ResolvedType::InterfaceSelf(_)
        | ResolvedType::TraitSelf(_) => false,
        _ => !crate::ownership::resolved_type_is_move_type(
            ty,
            &crate::ownership::MoveEnums::default(),
        ),
    }
}

pub(crate) fn borrowed_value_uses_cell(ty: &ResolvedType) -> bool {
    match ty {
        ResolvedType::Nullable(inner) => borrowed_value_uses_cell(inner),
        // Regions retain storage provenance in their inline descriptor, not an
        // address of the caller's descriptor binding.
        ResolvedType::ByteRegion(_) => false,
        _ => true,
    }
}

pub(crate) fn requires_parameter_cell(
    ty: &ResolvedType,
    take: bool,
    writable: bool,
    enums: &[crate::semantics::EnumSemanticInfo],
) -> bool {
    let mut payload = ty;
    while let ResolvedType::Nullable(inner) = payload {
        payload = inner;
    }
    let ownership_cell = match payload {
        ResolvedType::Enum(ty) => !enums
            .iter()
            .find(|definition| definition.source_type == *ty)
            .is_some_and(|definition| definition.capabilities.trivial_copy),
        _ => requires_owned_cell(ty),
    };
    (take && ownership_cell)
        || (writable && !parameter_copies_value(ty, enums) && requires_writable_cell(ty))
}

pub(crate) fn requires_writable_cell(ty: &ResolvedType) -> bool {
    match ty {
        // PHP arrays have value semantics. Their writable Doria loans must
        // share a stable home even when every element is a Copy scalar.
        ResolvedType::Bytes
        | ResolvedType::TypedArray(_)
        | ResolvedType::List(_)
        | ResolvedType::Dictionary(_, _)
        | ResolvedType::SortedDictionary(_, _)
        | ResolvedType::Set(_)
        | ResolvedType::SortedSet(_)
        | ResolvedType::PriorityQueue(_)
        | ResolvedType::Deque(_) => true,
        ResolvedType::Nullable(inner) => requires_writable_cell(inner),
        _ => requires_owned_cell(ty),
    }
}

#[cfg(test)]
mod parameter_home_tests {
    use super::*;

    #[test]
    fn region_returns_keep_provenance_without_descriptor_parameter_homes() {
        let program = crate::lower_source(
            "region-parameter-homes.doria",
            r#"
use Doria\Std\Io\{ByteRegion, WritableByteRegion};
class Token {}
function forward(ByteRegion $value): ByteRegion { return $value; }
function optional(?ByteRegion $value): ?ByteRegion { return $value; }
function writableForward(writable WritableByteRegion $value): WritableByteRegion {
    let $length = $value->length;
    return $value->getWritableRegion(0, $length);
}
function objectBorrow(Token $value): Token { return $value; }
"#,
        )
        .unwrap();
        let plans = PhpClosurePlan::build(&program, None);
        for item in &program.items {
            let Item::Function(function) = item else {
                continue;
            };
            let home = &plans.source_callables[&function.span];
            let expected = function.name == "objectBorrow";
            assert_eq!(home.returns_borrow, expected, "{}", function.name);
            assert_eq!(home.parameters[0].cell, expected, "{}", function.name);
        }
    }

    #[test]
    fn taken_trivial_copy_enums_do_not_acquire_move_ownership_homes() {
        let program = crate::lower_source(
            "parameter-homes.doria",
            r#"
class Token {}
enum Scalar { case Item(int $value); }
enum Managed { case Item(string $value); }
enum Linear { case Item(Token $value); }
"#,
        )
        .unwrap();
        let enums = &program.semantic_info.enums;
        for (name, owned) in [("Scalar", false), ("Managed", true), ("Linear", true)] {
            let facts = enums.iter().find(|value| value.name == name).unwrap();
            let ty = ResolvedType::Enum(facts.source_type.clone());
            assert_eq!(
                requires_parameter_cell(&ty, true, false, enums),
                owned,
                "{name}"
            );
            let nullable = ResolvedType::Nullable(Box::new(ty));
            assert_eq!(
                requires_parameter_cell(&nullable, true, false, enums),
                owned,
                "?{name}"
            );
        }
    }
}

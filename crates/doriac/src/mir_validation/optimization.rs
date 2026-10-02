//! Shared optimizer proofs derived from validated MIR, never source modifiers alone.

use super::*;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ParameterFacts {
    pub readonly: bool,
    pub nocapture: bool,
    pub noalias: bool,
    pub nonnull: bool,
    pub dereferenceable: u32,
    pub alignment: u32,
}

#[derive(Debug, Clone, Default)]
pub struct FunctionOptimizationFacts {
    pub parameters: Vec<ParameterFacts>,
    pub stack_classes: HashMap<mir::LocalId, ClassId>,
    pub constructed_closures: HashSet<mir::ClosureDescriptorId>,
}

#[derive(Debug, Clone)]
pub struct OptimizationFacts {
    pub functions: Vec<FunctionOptimizationFacts>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OptimizationMetrics {
    pub direct_call_site_count: usize,
    pub virtual_call_site_count: usize,
    pub interface_call_site_count: usize,
    pub closure_call_site_count: usize,
    pub stack_class_allocation_count: usize,
    pub stack_closure_environment_count: usize,
    pub pointer_readonly_parameter_count: usize,
    pub pointer_nocapture_parameter_count: usize,
    pub pointer_noalias_parameter_count: usize,
}

impl OptimizationMetrics {
    fn calls(&mut self, program: &mir::Program, accesses: &ClassLocalAccesses<'_>) {
        for access in &accesses.accesses {
            if let ClassLocalAccess::Call(CallTarget::Direct { function, .. }, _) = access {
                if program.functions[function.0].virtual_slot.is_some() {
                    self.virtual_call_site_count += 1;
                } else {
                    self.direct_call_site_count += 1;
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Effects {
    captures: bool,
    writes: bool,
}

impl Effects {
    const UNKNOWN: Self = Self {
        captures: true,
        writes: true,
    };

    fn include(&mut self, other: Self) {
        self.captures |= other.captures;
        self.writes |= other.writes;
    }
}

/// Call only after `validate_program`. Backend ABI facts and storage choices
/// must use the same proof, including its conservative unknown-call boundary.
pub fn optimization_facts(program: &mir::Program) -> OptimizationFacts {
    optimization_facts_with_metrics(program, None)
}

pub(crate) fn optimization_facts_with_metrics(
    program: &mir::Program,
    mut metrics: Option<&mut OptimizationMetrics>,
) -> OptimizationFacts {
    let mut effects: Vec<Vec<Effects>> = program
        .functions
        .iter()
        .map(|function| vec![Effects::default(); function.params.len()])
        .collect();
    // Start with no observed effects and propagate them through direct calls.
    // This finite lattice also handles mutually recursive noncapturing calls.
    loop {
        let previous = effects.clone();
        for function in &program.functions {
            for (index, parameter) in function.params.iter().enumerate() {
                effects[function.id.0][index] =
                    root_effects(program, function, *parameter, &previous);
            }
        }
        if effects == previous {
            break;
        }
    }
    let functions = program
        .functions
        .iter()
        .map(|function| {
            let parameters = function
                .params
                .iter()
                .enumerate()
                .map(|(index, local)| {
                    let mir::Type::Class(class) = function.locals[local.0].ty else {
                        return ParameterFacts::default();
                    };
                    let class = &program.classes[class.0];
                    // Open classes and erased values use aggregate carriers, not one
                    // pointer parameter. Their payload facts must not annotate a carrier.
                    if class.is_open
                        || function.closure.is_some()
                        || (index == 0 && function.uses_virtual_receiver_abi())
                    {
                        return ParameterFacts::default();
                    }
                    let effect = effects[function.id.0][index];
                    let scalar_payload = class.properties.iter().all(|property| {
                        matches!(
                            property.ty,
                            mir::Type::Scalar(_) | mir::Type::NullableScalar(_)
                        )
                    });
                    let facts = ParameterFacts {
                        // Copying a string or inspecting shared access can write through
                        // a payload pointer even when the source receiver is readonly.
                        readonly: !effect.writes && scalar_payload,
                        nocapture: !effect.captures,
                        noalias: !effect.captures
                            && function.parameter_modes[index]
                                == mir::FunctionParameterMode::Writable
                            && scalar_payload,
                        nonnull: true,
                        dereferenceable: class.layout.size,
                        alignment: class.layout.align,
                    };
                    if let Some(metrics) = metrics.as_deref_mut() {
                        metrics.pointer_readonly_parameter_count += usize::from(facts.readonly);
                        metrics.pointer_nocapture_parameter_count += usize::from(facts.nocapture);
                        metrics.pointer_noalias_parameter_count += usize::from(facts.noalias);
                    }
                    facts
                })
                .collect();
            let mut stack_classes = HashMap::new();
            // Bound the total added frame, not each allocation independently. Large
            // objects and repeated allocations keep the existing heap path.
            let mut stack_bytes = 0_u32;
            for block in &function.blocks {
                if iterator_storage::repeated_block(function, block.id) {
                    continue;
                }
                for statement in &block.statements {
                    let mir::Statement::AssignLocal {
                        target,
                        value:
                            mir::Rvalue::Class(mir::ClassExpression::New {
                                class,
                                concrete_class,
                                constructor,
                                ..
                            }),
                    } = statement
                    else {
                        continue;
                    };
                    if class != concrete_class
                        || program.classes[class.0].is_open
                        || !function.locals[target.0].owned
                        || assignment_count(function, *target) != 1
                        || root_effects(program, function, *target, &effects).captures
                        || constructor.is_some_and(|id| effects[id.0][0].captures)
                    {
                        continue;
                    }
                    let mut phase = Some(*class);
                    let mut safe_drop = true;
                    while let Some(id) = phase {
                        let definition = &program.classes[id.0];
                        safe_drop &= definition
                            .destructor
                            .is_none_or(|id| !effects[id.0][0].captures);
                        phase = definition.parent;
                    }
                    let layout = &program.classes[class.0].layout;
                    let end = stack_bytes
                        .checked_add(layout.align - 1)
                        .map(|value| value & !(layout.align - 1))
                        .and_then(|value| value.checked_add(layout.size.max(1)));
                    if safe_drop && end.is_some_and(|end| end <= 4096) {
                        stack_bytes = end.unwrap();
                        stack_classes.insert(*target, *class);
                        if let Some(metrics) = metrics.as_deref_mut() {
                            metrics.stack_class_allocation_count += 1;
                        }
                    }
                }
            }
            let mut constructed_closures = HashSet::new();
            for block in &function.blocks {
                for statement in &block.statements {
                    let accesses = collect_statement_class_local_accesses(statement);
                    if let Some(metrics) = metrics.as_deref_mut() {
                        metrics.calls(program, &accesses);
                    }
                    constructed_closures.extend(accesses.closure_constructions);
                }
                let accesses = collect_terminator_class_local_accesses(&block.terminator);
                if let Some(metrics) = metrics.as_deref_mut() {
                    metrics.calls(program, &accesses);
                    if let mir::Terminator::IndirectCall { callee, .. }
                    | mir::Terminator::CheckedIndirectCall { callee, .. } = &block.terminator
                    {
                        match callee {
                            mir::IndirectCallee::Closure(_) => metrics.closure_call_site_count += 1,
                            mir::IndirectCallee::InterfaceMethod { .. } => {
                                metrics.interface_call_site_count += 1
                            }
                        }
                    }
                }
                constructed_closures.extend(accesses.closure_constructions);
            }
            if let Some(metrics) = metrics.as_deref_mut() {
                metrics.stack_closure_environment_count += constructed_closures
                    .iter()
                    .filter(|id| {
                        program.closure_descriptors[id.0].environment_placement
                            == mir::ClosureEnvironmentPlacement::Stack
                    })
                    .count();
            }
            FunctionOptimizationFacts {
                parameters,
                stack_classes,
                constructed_closures,
            }
        })
        .collect();
    OptimizationFacts { functions }
}

fn assignment_count(function: &mir::Function, local: mir::LocalId) -> usize {
    function
        .blocks
        .iter()
        .map(|block| {
            let statements = block
                .statements
                .iter()
                .filter(|statement| match statement {
                    mir::Statement::AssignLocal { target, .. } => *target == local,
                    mir::Statement::AssignLocalGroup { targets, .. }
                    | mir::Statement::BindPayloadEnumFields { targets, .. } => {
                        targets.contains(&local)
                    }
                    mir::Statement::BindClosureEnvironment { bindings, .. } => {
                        bindings.iter().any(|(_, target)| *target == local)
                    }
                    mir::Statement::CoreCollection { operation, .. } => {
                        operation.outputs().contains(&local)
                    }
                    mir::Statement::ExtractErrorObject { target, .. } => *target == local,
                    _ => false,
                })
                .count();
            let result = match &block.terminator {
                mir::Terminator::CheckedCall { result, error, .. }
                | mir::Terminator::CheckedIndirectCall { result, error, .. }
                | mir::Terminator::CheckedIo { result, error, .. } => {
                    *result == Some(local) || *error == local
                }
                mir::Terminator::IndirectCall { result, .. } => *result == Some(local),
                mir::Terminator::CheckedConstruct { result, error, .. } => {
                    *result == local || *error == local
                }
                _ => false,
            };
            statements + usize::from(result)
        })
        .sum()
}

fn local_alias(value: &mir::Rvalue) -> Option<mir::LocalId> {
    match value {
        mir::Rvalue::Class(mir::ClassExpression::Local {
            local,
            transfer: false,
            ..
        }) => Some(*local),
        _ => None,
    }
}

fn touches(accesses: &ClassLocalAccesses<'_>, roots: &HashSet<mir::LocalId>) -> bool {
    accesses
        .borrowed()
        .chain(accesses.transferred())
        .chain(accesses.resource_reads.iter().copied())
        .chain(accesses.resource_transfers.iter().copied())
        .any(|local| roots.contains(&local))
}

fn value_touches(value: &mir::Rvalue, roots: &HashSet<mir::LocalId>) -> bool {
    let mut accesses = ClassLocalAccesses::default();
    collect_rvalue_class_local_accesses(value, &mut accesses);
    touches(&accesses, roots)
}

fn copied_result(value: &mir::Rvalue) -> bool {
    matches!(
        value.ty(),
        mir::Type::Scalar(_)
            | mir::Type::NullableScalar(_)
            | mir::Type::String
            | mir::Type::NullableString
    )
}

fn call_effects(
    program: &mir::Program,
    accesses: &ClassLocalAccesses<'_>,
    roots: &HashSet<mir::LocalId>,
    summaries: &[Vec<Effects>],
) -> Effects {
    let mut result = Effects::default();
    for (callee, receiver) in &accesses.method_receivers {
        let mut receiver_accesses = ClassLocalAccesses::default();
        collect_nullable_class_local_accesses(receiver, &mut receiver_accesses);
        if touches(&receiver_accesses, roots) {
            // Null-safe calls record the receiver separately from explicit arguments.
            result.include(
                if program.functions[callee.0].virtual_slot.is_none()
                    && program.functions[callee.0].return_borrow.is_none()
                {
                    summaries[callee.0][0]
                } else {
                    Effects::UNKNOWN
                },
            );
        }
    }
    if accesses
        .transferred()
        .chain(accesses.resource_transfers.iter().copied())
        .any(|local| roots.contains(&local))
    {
        result.include(Effects::UNKNOWN);
    }
    for access in &accesses.accesses {
        let ClassLocalAccess::Call(target, args) = access else {
            continue;
        };
        for (index, value) in args.iter().enumerate() {
            if !value_touches(value, roots) {
                continue;
            }
            match target {
                CallTarget::Direct {
                    function,
                    parameter_offset,
                } if program.functions[function.0].virtual_slot.is_none()
                    && program.functions[function.0].return_borrow.is_none() =>
                {
                    result.include(summaries[function.0][index + parameter_offset]);
                }
                _ => result.include(Effects::UNKNOWN),
            }
        }
    }
    result
}

fn root_effects(
    program: &mir::Program,
    function: &mir::Function,
    root: mir::LocalId,
    summaries: &[Vec<Effects>],
) -> Effects {
    let mut roots = HashSet::from([root]);
    loop {
        let count = roots.len();
        for statement in function.blocks.iter().flat_map(|block| &block.statements) {
            if let mir::Statement::AssignLocal { target, value } = statement {
                if local_alias(value).is_some_and(|source| roots.contains(&source))
                    && !function.locals[target.0].owned
                {
                    roots.insert(*target);
                }
            }
        }
        if count == roots.len() {
            break;
        }
    }
    if roots.iter().any(|local| {
        assignment_count(function, *local) > usize::from(!function.params.contains(local))
    }) {
        return Effects::UNKNOWN;
    }
    let mut result = Effects::default();
    for block in &function.blocks {
        for statement in &block.statements {
            if let mir::Statement::DropClass { local, class } = statement {
                if roots.contains(local) {
                    result.writes = true;
                    let mut phase = Some(*class);
                    while let Some(class) = phase {
                        let definition = &program.classes[class.0];
                        if let Some(destructor) = definition.destructor {
                            result.include(summaries[destructor.0][0]);
                        }
                        phase = definition.parent;
                    }
                }
                continue;
            }
            let accesses = collect_statement_class_local_accesses(statement);
            if !touches(&accesses, &roots) {
                continue;
            }
            result.include(call_effects(program, &accesses, &roots, summaries));
            match statement {
                mir::Statement::AssignLocal { target, value }
                    if roots.contains(target)
                        && local_alias(value).is_some_and(|local| roots.contains(&local)) => {}
                mir::Statement::AssignLocal { value, .. }
                | mir::Statement::AssignLocalGroup { value, .. }
                | mir::Statement::AssignStatic { value, .. }
                    if copied_result(value) => {}
                mir::Statement::AssignProperty { object, value, .. } => {
                    result.writes |= roots.contains(object);
                    if value_touches(value, &roots) && !copied_result(value) {
                        result.include(Effects::UNKNOWN);
                    }
                }
                mir::Statement::CallVoid { .. }
                | mir::Statement::EchoString(_)
                | mir::Statement::WriteStderr(_)
                | mir::Statement::Printf(_)
                | mir::Statement::WriteFile { .. }
                | mir::Statement::AppendFile { .. } => {}
                _ => result.include(Effects::UNKNOWN),
            }
        }
        let accesses = collect_terminator_class_local_accesses(&block.terminator);
        if !touches(&accesses, &roots) {
            continue;
        }
        result.include(call_effects(program, &accesses, &roots, summaries));
        match &block.terminator {
            mir::Terminator::Return(value) if copied_result(value) => {}
            mir::Terminator::Branch { .. }
            | mir::Terminator::Panic { .. }
            | mir::Terminator::CheckedCall { .. }
            | mir::Terminator::CheckedIo { .. } => {}
            _ => result.include(Effects::UNKNOWN),
        }
    }
    result
}

//! Check replacement against the physical slot, independently of source proofs.

use super::*;

pub(super) fn validate(program: &mir::Program) -> Result<Proof<'_>, BackendError> {
    let mut proof = Proof::new(program, Purpose::Writeback)?;
    if let Some(entry) = program.selected_entry {
        seed_root(&mut proof, entry)?;
        for boundary in runtime_entries(program)? {
            if boundary != entry {
                seed_root(&mut proof, boundary)?;
            }
        }
        converge(&mut proof)?;
        let graph = call_graph(program)?;
        loop {
            let roots = uncovered_roots(&proof, &graph);
            if roots.is_empty() {
                break;
            }
            // Solve callers before their helpers, including mutually recursive
            // components. Never add an arbitrary unknown callback context in
            // parallel with a helper's established executable call sites.
            for root in roots {
                seed_root(&mut proof, root)?;
            }
            converge(&mut proof)?;
        }
    } else {
        // Library MIR does not establish a closed exported-call boundary. An
        // internal caller cannot discharge the independent public obligation.
        for function in &program.functions {
            if function.closure.is_none() {
                seed_root(&mut proof, function.id)?;
            }
        }
        converge(&mut proof)?;
    }
    proof.check()?;
    Ok(proof)
}

fn seed_root(proof: &mut Proof<'_>, id: mir::FunctionId) -> Result<(), BackendError> {
    let function = function_in(proof.program, id)?;
    if function.closure.is_some() {
        return Err(malformed_mir(
            "independent writeback root requires a nonclosure body",
        ));
    }
    let context = proof.contexts.len();
    let inputs = function
        .params
        .iter()
        .map(|local| {
            let ty = function.locals[local.0].ty;
            // Physical owners remain distinct from unknown callable provenance.
            let mut value = Value::owner(Root::Local(OWN, Some(*local)));
            value.homes.insert(Some(ty));
            value.types.insert(Some(ty));
            value.mark_unknown_callable(ty);
            value
        })
        .collect();
    proof
        .contexts
        .push(Context::new(id, None, Kind::Root, OWN, context, inputs));
    proof.changed = true;
    Ok(())
}

fn converge(proof: &mut Proof<'_>) -> Result<(), BackendError> {
    loop {
        while proof.changed {
            proof.changed = false;
            proof.pass()?;
        }
        // Construction environments are definition proofs, never fabricated
        // executable calls. Their bodies may construct further environments.
        if !proof.seed_callable_definitions()? {
            return Ok(());
        }
        proof.changed = true;
    }
}
fn runtime_entries(program: &mir::Program) -> Result<Vec<mir::FunctionId>, BackendError> {
    // These bodies are also called by native drop glue and nominal stream
    // thunks, not only by authored MIR calls. Declaration identity owns the
    // stream boundary; an authored method spelling is never evidence.
    let mut entries = program
        .classes
        .iter()
        .filter_map(|class| class.destructor)
        .collect::<HashSet<_>>();
    for table in &program.interface_vtables {
        let interface = interface_in(program, table.interface)?;
        for (slot, entry) in table.methods.iter().enumerate() {
            if interface.stream_operation(slot).is_some() {
                entries.insert(*entry);
            }
        }
    }
    let mut entries = entries.into_iter().collect::<Vec<_>>();
    entries.sort_unstable_by_key(|entry| entry.0);
    Ok(entries)
}

fn call_graph(program: &mir::Program) -> Result<Vec<HashSet<usize>>, BackendError> {
    let mut graph = vec![HashSet::new(); program.functions.len()];
    for function in &program.functions {
        let (reachable, _) = reachable_blocks_and_predecessors(function, true)?;
        for block in function.blocks.iter().filter(|block| reachable[block.id.0]) {
            for statement in &block.statements {
                graph_targets(
                    program,
                    &collect_statement_class_local_accesses(statement),
                    &mut graph[function.id.0],
                )?;
            }
            graph_targets(
                program,
                &collect_terminator_class_local_accesses(&block.terminator),
                &mut graph[function.id.0],
            )?;
        }
    }
    Ok(graph)
}

fn graph_targets(
    program: &mir::Program,
    accesses: &ClassLocalAccesses<'_>,
    targets: &mut HashSet<usize>,
) -> Result<(), BackendError> {
    for (position, access) in accesses.iter().enumerate() {
        if let ClassLocalAccess::Call(
            CallTarget::Direct {
                function,
                parameter_offset,
            },
            args,
        ) = access
        {
            let receiver = accesses
                .method_receivers
                .iter()
                .find_map(|(at, _, receiver)| {
                    (*at == position).then_some(receiver.static_receiver_class())
                })
                .or_else(|| {
                    (parameter_offset == 0)
                        .then(|| retained::receiver_class(args))
                        .flatten()
                });
            targets.extend(
                possible_callees(program, function, receiver)?
                    .into_iter()
                    .map(|callee| callee.id.0),
            );
        }
    }
    for (_, callee) in &accesses.indirect_callees {
        if let mir::IndirectCallee::InterfaceMethod {
            interface, slot, ..
        } = callee
        {
            targets.extend(
                program
                    .interface_vtables
                    .iter()
                    .filter(|table| table.interface == *interface)
                    .map(|table| table.methods[*slot].0),
            );
        }
    }
    for descriptor in &accesses.closure_constructions {
        targets.insert(
            closure_descriptor_in(program, *descriptor)?
                .entry_function
                .0,
        );
    }
    Ok(())
}

/// Strongly connected components of the call graph, including every closure
/// body an indirect call could execute. Closure creation is checked against
/// its descriptor's function type, and MIR has no function-type conversion,
/// so equal function types name every possible callback body.
pub(super) fn components(program: &mir::Program) -> Result<Vec<usize>, BackendError> {
    let mut graph = call_graph(program)?;
    for function in &program.functions {
        for block in &function.blocks {
            if let mir::Terminator::IndirectCall {
                callee: mir::IndirectCallee::Closure(expression),
                ..
            }
            | mir::Terminator::CheckedIndirectCall {
                callee: mir::IndirectCallee::Closure(expression),
                ..
            } = &block.terminator
            {
                let ty = expression.function_type();
                graph[function.id.0].extend(
                    program
                        .closure_descriptors
                        .iter()
                        .filter(|descriptor| descriptor.function_type == ty)
                        .map(|descriptor| descriptor.entry_function.0),
                );
            }
        }
    }
    // Iterative Kosaraju keeps recursion in the analyzed program off the host stack.
    let mut visited = vec![false; graph.len()];
    let mut finished = Vec::with_capacity(graph.len());
    for root in 0..graph.len() {
        let mut pending = vec![(root, false)];
        while let Some((node, exiting)) = pending.pop() {
            if exiting {
                finished.push(node);
            } else if !std::mem::replace(&mut visited[node], true) {
                pending.push((node, true));
                pending.extend(graph[node].iter().map(|next| (*next, false)));
            }
        }
    }
    let mut reverse = vec![Vec::new(); graph.len()];
    for (caller, targets) in graph.iter().enumerate() {
        for target in targets {
            reverse[*target].push(caller);
        }
    }
    let mut components = vec![None; graph.len()];
    let mut count = 0;
    for root in finished.into_iter().rev() {
        if components[root].is_some() {
            continue;
        }
        let mut pending = vec![root];
        while let Some(node) = pending.pop() {
            if components[node].is_none() {
                components[node] = Some(count);
                pending.extend(reverse[node].iter().copied());
            }
        }
        count += 1;
    }
    // Every node is finished above. A shared fallback would only merge cycles.
    Ok(components
        .into_iter()
        .map(|component| component.unwrap_or(count))
        .collect())
}

fn uncovered_roots(proof: &Proof<'_>, calls: &[HashSet<usize>]) -> Vec<mir::FunctionId> {
    let live = proof.live();
    let covered = proof
        .contexts
        .iter()
        .zip(&live)
        .filter(|(_, live)| **live)
        .map(|(context, _)| context.function)
        .collect::<HashSet<_>>();
    let uncovered = proof
        .program
        .functions
        .iter()
        .map(|function| function.closure.is_none() && !covered.contains(&function.id))
        .collect::<Vec<_>>();
    // Closure bodies require their actual descriptor/environment context. For
    // root ordering only, retain their edges between nonclosure components.
    // A construction edge never counts as executable body coverage.
    let mut graph = vec![HashSet::new(); calls.len()];
    for (caller, missing) in uncovered.iter().enumerate() {
        if !missing {
            continue;
        }
        let mut pending = calls[caller].iter().copied().collect::<Vec<_>>();
        let mut visited = HashSet::new();
        while let Some(callee) = pending.pop() {
            if !visited.insert(callee) {
                continue;
            }
            if proof.program.functions[callee].closure.is_some() {
                pending.extend(calls[callee].iter().copied());
            } else if uncovered[callee] {
                graph[caller].insert(callee);
            }
        }
    }
    source_components(&graph, &uncovered)
        .into_iter()
        .map(mir::FunctionId)
        .collect()
}

fn source_components(graph: &[HashSet<usize>], included: &[bool]) -> Vec<usize> {
    // Iterative Kosaraju keeps recursion in the analyzed program off the host
    // stack. Only source SCCs receive independent inputs; every later body is
    // reached with the actual caller's facts, or seeded on the next coverage pass.
    let mut visited = HashSet::new();
    let mut finished = Vec::new();
    for (root, included) in included.iter().enumerate() {
        if !included {
            continue;
        }
        let mut pending = vec![(root, false)];
        while let Some((node, exiting)) = pending.pop() {
            if exiting {
                finished.push(node);
            } else if visited.insert(node) {
                pending.push((node, true));
                pending.extend(graph[node].iter().map(|next| (*next, false)));
            }
        }
    }
    let mut reverse = vec![Vec::new(); graph.len()];
    for (caller, targets) in graph.iter().enumerate() {
        for target in targets {
            reverse[*target].push(caller);
        }
    }
    let mut components = vec![None; graph.len()];
    let mut representatives = Vec::new();
    for root in finished.into_iter().rev() {
        if components[root].is_some() {
            continue;
        }
        let component = representatives.len();
        let mut representative = root;
        let mut pending = vec![root];
        while let Some(node) = pending.pop() {
            if components[node].is_some() {
                continue;
            }
            components[node] = Some(component);
            representative = representative.min(node);
            pending.extend(reverse[node].iter().copied());
        }
        representatives.push(representative);
    }
    let mut incoming = vec![false; representatives.len()];
    for (caller, targets) in graph.iter().enumerate() {
        for target in targets {
            if components[caller] != components[*target] {
                incoming[components[*target].expect("covered call component")] = true;
            }
        }
    }
    let mut roots = representatives
        .into_iter()
        .enumerate()
        .filter_map(|(component, root)| (!incoming[component]).then_some(root))
        .collect::<Vec<_>>();
    roots.sort_unstable();
    roots
}

pub(super) fn assign(
    program: &mir::Program,
    function: &mir::Function,
    target: mir::LocalId,
    previous: &Value,
    replacement: &mut Value,
    checking: bool,
) -> Result<(), BackendError> {
    let local = local_in(function, target)?;
    if function.is_writable_borrowed_place(target) {
        if checking {
            validate_replacement(program, &previous.homes, &replacement.types).map_err(
                |mut error| {
                    error.message.push_str(&format!(
                        " in `{}` at local{}: homes {:?}, replacement types {:?}",
                        function.name, target.0, previous.homes, replacement.types,
                    ));
                    error
                },
            )?;
        }
        replacement.homes.clone_from(&previous.homes);
    } else if local.owned || !local.ty.has_move_ownership() {
        replacement.homes = HashSet::from([Some(local.ty)]);
    }
    Ok(())
}

pub(super) fn unknown_call(
    program: &mir::Program,
    signature: &mir::FunctionType,
    inputs: &[Value],
) -> Result<(), BackendError> {
    for (parameter, input) in signature.parameters.iter().zip(inputs) {
        if parameter.mode == mir::FunctionParameterMode::Writable {
            validate_replacement(program, &input.homes, &HashSet::from([Some(parameter.ty)]))?;
        }
    }
    Ok(())
}

fn validate_replacement(
    program: &mir::Program,
    homes: &HashSet<Option<mir::Type>>,
    replacements: &HashSet<Option<mir::Type>>,
) -> Result<(), BackendError> {
    if homes.is_empty() || replacements.is_empty() {
        return Err(malformed_mir(
            "writable replacement lacks proven storage origins",
        ));
    }
    for home in homes {
        let Some(home) = home else {
            return Err(malformed_mir("writable replacement has unresolved storage"));
        };
        for replacement in replacements {
            let compatible = match replacement {
                Some(ty) => fits(program, *ty, *home),
                None => *home == mir::Type::Mixed || nullable_value_type(*home).is_some(),
            };
            if !compatible {
                return Err(malformed_mir(format!(
                    "writable replacement {replacement:?} cannot fit original storage {home}"
                )));
            }
        }
    }
    Ok(())
}

fn fits(program: &mir::Program, source: mir::Type, target: mir::Type) -> bool {
    if source == target || target == mir::Type::Mixed || target == mir::Type::NullableMixed {
        return true;
    }
    if let Some(target) = nullable_value_type(target) {
        return fits(
            program,
            nullable_value_type(source).unwrap_or(source),
            target,
        );
    }
    if nullable_value_type(source).is_some() {
        return false;
    }
    if let (Some(source), mir::Type::Interface(target)) =
        (source.interface_implementing_type(), target)
    {
        return program.interface_vtable(source, target).is_some();
    }
    nominal_type_implies(program, source, target)
}

#[cfg(test)]
#[test]
fn inline_replacement_requires_the_checked_interface_conformance() {
    let program = crate::lower_source_to_mir(
        "inline-replacement.doria",
        r#"
use Doria\Std\Io\{StandardStreams, ReadinessStream};
interface Other {}
class Heap implements Other {}
function observe(ReadinessStream $stream): void {}
function other(Other $value): void {}
function main(): void { observe(StandardStreams::input); other(new Heap()); }
"#,
    )
    .unwrap();
    let inline = program
        .classes
        .iter()
        .find_map(|class| {
            class
                .inline_copy_type()
                .filter(|ty| matches!(ty, crate::inline_value::InlineValueType::StandardInput(_)))
        })
        .unwrap();
    let source = mir::Type::Scalar(mir::ScalarType::Inline(inline));
    let nullable = mir::Type::NullableScalar(mir::ScalarType::Inline(inline));
    let mut accepted = 0;
    let mut rejected = 0;
    for interface in &program.interface_types {
        let conforms = program
            .interface_vtable(source.interface_implementing_type().unwrap(), interface.id)
            .is_some();
        assert_eq!(
            fits(&program, source, mir::Type::Interface(interface.id)),
            conforms
        );
        assert_eq!(
            fits(
                &program,
                nullable,
                mir::Type::NullableInterface(interface.id)
            ),
            conforms
        );
        assert!(!fits(
            &program,
            nullable,
            mir::Type::Interface(interface.id)
        ));
        assert!(!fits(
            &program,
            mir::Type::Scalar(mir::ScalarType::Bool),
            mir::Type::Interface(interface.id)
        ));
        if conforms {
            accepted += 1;
        } else {
            rejected += 1;
        }
    }
    assert!(accepted > 0 && rejected > 0);
}

fn nullable_value_type(ty: mir::Type) -> Option<mir::Type> {
    // Payload enums have program-dependent layouts and are deliberately absent
    // from the fixed-width ABI helper. Their nullable value domain still exists.
    match ty {
        mir::Type::NullablePayloadEnum(ty) => Some(mir::Type::PayloadEnum(ty)),
        _ => crate::native_abi::nullable_payload_type(ty),
    }
}

pub(super) fn complete_value(
    program: &mir::Program,
    expression: &mir::Rvalue,
    value: &mut Value,
) -> Result<(), BackendError> {
    match expression {
        mir::Rvalue::Class(mir::ClassExpression::Upcast {
            value: expression, ..
        }) => {
            return complete_value(program, &mir::Rvalue::Class((**expression).clone()), value);
        }
        mir::Rvalue::NullableClass(mir::NullableClassExpression::Upcast {
            value: expression,
            ..
        }) => {
            return complete_value(
                program,
                &mir::Rvalue::NullableClass((**expression).clone()),
                value,
            );
        }
        _ => {}
    }
    if expression.is_null_value()
        || matches!(
            expression,
            mir::Rvalue::NullableScalar(mir::NullableScalarExpression::Null(_))
                | mir::Rvalue::NullableString(mir::NullableStringExpression::Null)
        )
    {
        value.types = HashSet::from([None]);
        return Ok(());
    }
    if matches!(
        expression,
        mir::Rvalue::ByteRegion(mir::ByteRegionExpression::Region { .. })
    ) {
        // Formation preserves the byte-storage owners, but produces a new
        // inline descriptor. Its replacement domain and home are the region,
        // not the Bytes or parent descriptor from which it borrows storage.
        value.types = HashSet::from([Some(expression.ty())]);
        value.homes = HashSet::from([Some(expression.ty())]);
        return validate_type(program, expression.ty());
    }
    if expression.ty() != mir::Type::Mixed && nullable_value_type(expression.ty()).is_none() {
        value.types = value
            .types
            .iter()
            .filter_map(|ty| ty.map(|ty| Some(nullable_value_type(ty).unwrap_or(ty))))
            .collect();
    }
    // Forwarded operands keep bottom until their actual executable origin is
    // known. A visible widened type must never stand in for physical storage.
    if expression.direct_place_local().is_some() || forwarded(expression) {
        return Ok(());
    }
    if let mir::Rvalue::Class(mir::ClassExpression::New { concrete_class, .. }) = expression {
        value.types = HashSet::from([Some(mir::Type::Class(*concrete_class))]);
    } else if value.types.is_empty() {
        value.types.insert(Some(expression.ty()));
    }
    if value.homes.is_empty() {
        // A borrowed value's visible type is not a proof of its physical home.
        // Unsupported origin transport must remain unresolved, not silently
        // permit widened replacements into an invented slot.
        value.homes.insert(if expression.borrows_move_value() {
            None
        } else {
            Some(expression.ty())
        });
    }
    validate_type(program, expression.ty())
}

fn forwarded(value: &mir::Rvalue) -> bool {
    use mir::*;
    if let Some(value) = crate::native_shared::Expression::from_rvalue(value) {
        return matches!(
            value.operation(),
            crate::native_shared::Operation::Present(_)
                | crate::native_shared::Operation::Call { .. }
                | crate::native_shared::Operation::Coalesce { .. }
        );
    }
    matches!(
        value,
        Rvalue::NullableScalar(
            NullableScalarExpression::Value(_)
                | NullableScalarExpression::Call { .. }
                | NullableScalarExpression::Coalesce { .. }
        ) | Rvalue::NullableString(
            NullableStringExpression::String(_)
                | NullableStringExpression::Call { .. }
                | NullableStringExpression::Coalesce { .. }
        ) | Rvalue::String(StringExpression::Call { .. } | StringExpression::Coalesce { .. })
            | Rvalue::Value(
                ValueExpression::Integer(IntegerExpression::Call { .. })
                    | ValueExpression::Float(FloatExpression::Call { .. })
                    | ValueExpression::Bool(BoolExpression::Call { .. })
                    | ValueExpression::Enum(EnumExpression::Call { .. })
                    | ValueExpression::Inline(InlineValueExpression::Call { .. })
            )
            | Rvalue::Class(ClassExpression::Call { .. } | ClassExpression::Coalesce { .. })
            | Rvalue::NullableClass(
                NullableClassExpression::Class(_)
                    | NullableClassExpression::Call { .. }
                    | NullableClassExpression::Coalesce { .. }
            )
            | Rvalue::Collection(CollectionExpression::Call { .. })
            | Rvalue::NullableCollection(
                NullableCollectionExpression::Collection(_)
                    | NullableCollectionExpression::Call { .. }
                    | NullableCollectionExpression::Coalesce { .. }
            )
            | Rvalue::Function(
                FunctionExpression::Call { .. } | FunctionExpression::AssumePresent { .. }
            )
            | Rvalue::NullableFunction(
                NullableFunctionExpression::Present(_) | NullableFunctionExpression::Call { .. }
            )
            | Rvalue::Interface(InterfaceExpression {
                value: InterfaceValue::Call { .. }
                    | InterfaceValue::FromClass { .. }
                    | InterfaceValue::FromNullableClass { .. }
                    | InterfaceValue::FromValue { .. }
                    | InterfaceValue::Upcast { .. },
                ..
            })
            | Rvalue::NullableInterface(NullableInterfaceExpression {
                value: NullableInterfaceValue::Call { .. }
                    | NullableInterfaceValue::Present(_)
                    | NullableInterfaceValue::Upcast { .. },
                ..
            })
            | Rvalue::Mixed(MixedExpression::Call { .. })
            | Rvalue::NullableMixed(
                NullableMixedExpression::Mixed(_)
                    | NullableMixedExpression::Call { .. }
                    | NullableMixedExpression::Coalesce { .. }
            )
            | Rvalue::PayloadEnum(
                PayloadEnumExpression::Call { .. } | PayloadEnumExpression::Coalesce { .. }
            )
            | Rvalue::NullablePayloadEnum(
                NullablePayloadEnumExpression::Value(_)
                    | NullablePayloadEnumExpression::Call { .. }
                    | NullablePayloadEnumExpression::Coalesce { .. }
            )
            | Rvalue::ByteRegion(
                ByteRegionExpression::Call { .. }
                    | ByteRegionExpression::AssumePresent { .. }
                    | ByteRegionExpression::Coalesce { .. }
            )
            | Rvalue::NullableByteRegion(
                NullableByteRegionExpression::Present(_)
                    | NullableByteRegionExpression::Call { .. }
            )
    )
}

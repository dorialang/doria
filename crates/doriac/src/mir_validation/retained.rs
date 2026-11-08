//! Validate retained source contracts independently of source ownership analysis.

use super::*;

/// A physical place is distinct from the input that supplies its lifetime.
/// Only resolved stored owning properties subdivide that place. An opaque
/// suffix covers any storage below the known prefix and cannot be subdivided.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct Place {
    pub local: mir::LocalId,
    pub fields: Vec<crate::class_layout::PropertyId>,
    pub opaque: bool,
}

impl Place {
    pub(super) fn local(local: mir::LocalId) -> Self {
        Self {
            local,
            fields: Vec::new(),
            opaque: false,
        }
    }

    pub(super) fn project(&self, property: crate::class_layout::PropertyId) -> Self {
        let mut place = self.clone();
        if !place.opaque {
            place.fields.push(property);
        }
        place
    }

    pub(super) fn overlaps(&self, other: &Self) -> bool {
        self.local == other.local
            && (self.fields.starts_with(&other.fields) || other.fields.starts_with(&self.fields))
    }

    /// Widen only path growth across an actual analysis cycle. Keep the newly
    /// reached prefix, but stop subdividing its unknown recursive descendants.
    pub(super) fn widen_on_cycle<'a>(&mut self, mut previous: impl Iterator<Item = &'a Self>) {
        self.opaque |= previous.any(|place| {
            self.local == place.local
                && self.fields.len() > place.fields.len()
                && self.fields.starts_with(&place.fields)
        });
    }
}

/// Executable provenance preserves physical paths and opaque subtrees; it is
/// derived from MIR operands, not accepted from source ownership annotations.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct Source {
    pub contract: mir::RetainedSource,
    pub place: Place,
}

impl Source {
    pub(super) fn new(
        place: Place,
        inherited: bool,
        writable: bool,
        lifetime: mir::RetainedLifetime,
    ) -> Self {
        Self {
            contract: mir::RetainedSource {
                local: place.local,
                inherited,
                writable,
                lifetime,
                owned_path: place.fields.clone(),
            },
            place,
        }
    }

    pub(super) fn input(
        local: mir::LocalId,
        writable: bool,
        lifetime: mir::RetainedLifetime,
    ) -> Self {
        Self::new(Place::local(local), true, writable, lifetime)
    }
}

impl std::ops::Deref for Source {
    type Target = mir::RetainedSource;
    fn deref(&self) -> &Self::Target {
        &self.contract
    }
}

impl std::ops::DerefMut for Source {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.contract
    }
}

/// Called after all function bodies have passed structural MIR validation.
pub(super) fn validate_pending_selections(program: &mir::Program) -> Result<(), BackendError> {
    call_origins::validate_pending(program)
}

pub(super) fn contract(function: &mir::Function) -> Option<&mir::RetainedSourcesPlan> {
    function.retained_sources_plan()
}

fn validate_receiver_source(
    program: &mir::Program,
    function: &mir::Function,
    source: &mir::RetainedSource,
) -> Result<(), BackendError> {
    if function.receiver_mode == Some(mir::ReceiverMode::Consuming)
        && function.params.first() == Some(&source.local)
        // A consumed carrier can forward its existing loans, but cannot lend
        // its own storage. The inherited bit alone is not proof of a carrier.
        && !(source.inherited && carries_source(program, local_in(function, source.local)?.ty))
    {
        return Err(malformed_mir(
            "retained return cannot borrow from a consuming receiver",
        ));
    }
    Ok(())
}

pub(super) fn validate_contract(
    program: &mir::Program,
    function: &mir::Function,
    plan: &mir::RetainedSourcesPlan,
) -> Result<(), BackendError> {
    if function
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter(|statement| {
            matches!(
                statement,
                mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::RetainedSources(_))
            )
        })
        .count()
        != 1
    {
        return Err(malformed_mir("retained source contract is duplicated"));
    }
    let mut sources = HashSet::new();
    let mut selectors = HashSet::new();
    for selector in &plan.returned_selectors {
        if !selectors.insert(selector) {
            return Err(malformed_mir("returned ownership selector is duplicated"));
        }
        if !matches!(
            function.return_type,
            mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Inline(
                crate::inline_value::InlineValueType::RegistrationKey(_)
            ))) | mir::ReturnType::Value(mir::Type::NullableScalar(mir::ScalarType::Inline(
                crate::inline_value::InlineValueType::RegistrationKey(_)
            )))
        ) {
            return Err(malformed_mir(
                "returned ownership selector requires a key result",
            ));
        }
        if let mir::RetainedStorageSelector::Input(local) = selector {
            let input = local_in(function, *local)?;
            let key_type = |ty| match ty {
                mir::Type::Scalar(mir::ScalarType::Inline(key))
                | mir::Type::NullableScalar(mir::ScalarType::Inline(key))
                    if matches!(
                        key,
                        crate::inline_value::InlineValueType::RegistrationKey(_)
                    ) =>
                {
                    Some(key)
                }
                _ => None,
            };
            let capture = function.closure.as_ref().is_some_and(|closure| {
                closure
                    .capture_locals
                    .iter()
                    .any(|(_, capture)| capture == local)
            });
            if (!function.params.contains(local) && !capture)
                || key_type(input.ty).is_none()
                || !matches!(function.return_type, mir::ReturnType::Value(ty) if key_type(ty) == key_type(input.ty))
            {
                return Err(malformed_mir(
                    "returned ownership selector must name an exact key input",
                ));
            }
        }
    }
    for stores in [&plan.stores, &plan.exceptional_stores] {
        let mut destinations = HashSet::new();
        for store in stores {
            let destination = local_in(function, store.destination)?;
            validate_source_path(
                program,
                function,
                &mir::RetainedSource {
                    local: store.destination,
                    inherited: false,
                    writable: true,
                    lifetime: mir::RetainedLifetime::Cleanup,
                    owned_path: store.path.clone(),
                },
            )?;
            if !function.params.contains(&store.destination)
                || destination.owned
                || !destination.writable
                || !destinations.insert((store.destination, &store.path, &store.identity))
            {
                return Err(malformed_mir(
                    "stored source contract must name a unique writable borrowed input",
                ));
            }
            let mut inputs = HashSet::new();
            for source in &store.sources {
                validate_source_path(program, function, source)?;
                let input = local_in(function, source.local)?;
                if !function.params.contains(&source.local)
                    || (!source.inherited && input.owned)
                    || (!source.inherited && source.writable && !input.writable)
                    || !inputs.insert(source)
                {
                    return Err(malformed_mir(
                        "stored source contract must retain live input loans",
                    ));
                }
            }
        }
    }
    for selections in [&plan.removals, &plan.exceptional_removals] {
        let mut unique = HashSet::new();
        for selection in selections {
            validate_storage_selection(program, function, selection)?;
            if !local_in(function, selection.destination)?.writable || !unique.insert(selection) {
                return Err(malformed_mir(
                    "storage removal must name a unique writable input",
                ));
            }
        }
    }
    for source in &plan.returns {
        validate_receiver_source(program, function, source)?;
        validate_source_path(program, function, source)?;
        let local = local_in(function, source.local)?;
        if !function.params.contains(&source.local)
            || !sources.insert((source.local, source.inherited, source.owned_path.clone()))
            || (!source.inherited && local.owned)
            || (!source.inherited && source.writable && !local.writable)
            || !matches!(function.return_type, mir::ReturnType::Value(ty) if ty.has_move_ownership() || byte_regions::kind(ty).is_some())
        {
            return Err(malformed_mir(
                "retained return must name a unique live input source",
            ));
        }
    }
    if let Some(storage) = &plan.returned_storage {
        let mut selections = HashSet::new();
        for selection in &storage.selections {
            validate_storage_selection(program, function, selection)?;
            if !selections.insert(selection) {
                return Err(malformed_mir("returned storage selection is duplicated"));
            }
        }
        let mut inputs = HashSet::new();
        for input in &storage.inputs {
            let local = local_in(function, *input)?;
            if !function.params.contains(input)
                || !local.owned
                || !carries_source(program, local.ty)
                || !inputs.insert(*input)
            {
                return Err(malformed_mir(
                    "returned storage must name unique owning carrier inputs",
                ));
            }
        }
        let mut members = HashSet::new();
        for member in &storage.members {
            if !members.insert((&member.identity, &member.path)) {
                return Err(malformed_mir("returned storage member is duplicated"));
            }
        }
        for source in storage
            .direct
            .iter()
            .chain(storage.members.iter().flat_map(|member| &member.sources))
        {
            validate_source_path(program, function, source)?;
            let local = local_in(function, source.local)?;
            if !function.params.contains(&source.local)
                || (!source.inherited && local.owned)
                || (!source.inherited && source.writable && !local.writable)
            {
                return Err(malformed_mir(
                    "returned storage must retain live input loans",
                ));
            }
        }
    }
    if let Some(borrow) = function
        .return_borrow
        .filter(|borrow| borrow.kind == mir::ReturnBorrowKind::Retained)
    {
        let index = match borrow.source {
            mir::BorrowSource::Receiver => 0,
            mir::BorrowSource::Parameter(index) => {
                index
                    + usize::from(function.receiver_mode.is_some())
                    + usize::from(function.closure.is_some())
            }
        };
        let parameter = function
            .params
            .get(index)
            .ok_or_else(|| malformed_mir("retained return signature has no source parameter"))?;
        if plan
            .returns
            .iter()
            .any(|source| source.local == *parameter && !borrow.lifetime.accepts(source.lifetime))
        {
            return Err(malformed_mir(
                "retained return signature shortens a cleanup-dependent source loan",
            ));
        }
    }
    let mut independent = HashSet::new();
    let mut constructed = HashSet::new();
    for source in &plan.constructs {
        validate_source_path(program, function, source)?;
        let local = local_in(function, source.local)?;
        let inherited_owner = source.inherited && local.owned && carries_source(program, local.ty);
        let borrowed_input =
            !source.inherited && !local.owned && (!source.writable || local.writable);
        if function
            .method
            .as_ref()
            .is_none_or(|method| method.name != "__construct")
            || !function.params.contains(&source.local)
            || function.params.first() == Some(&source.local)
            || !(inherited_owner || borrowed_input)
            || source.lifetime != mir::RetainedLifetime::Cleanup
            || !constructed.insert((source.local, source.inherited, source.owned_path.clone()))
        {
            return Err(malformed_mir(
                "constructed source must borrow an input or inherit an owned carrier's cleanup loans",
            ));
        }
    }
    for parameter in &plan.independent_parameters {
        if !function.params.contains(parameter) || !independent.insert(*parameter) {
            return Err(malformed_mir(
                "retained source precondition names an invalid or duplicate parameter",
            ));
        }
    }
    let mut properties = HashSet::new();
    for (property, parameter) in &plan.promotions {
        let definition = class_in(program, property.class)?;
        let property = property_in(program, property.class, *property)?;
        let parameter = local_in(function, *parameter)?;
        let Some(access) = property.borrowed_source else {
            return Err(malformed_mir(
                "retained promotion must name a borrowed field",
            ));
        };
        if definition.constructor != Some(function.id)
            || !function.params.contains(&parameter.id)
            || parameter.owned
            || parameter.writable != access.is_writable()
            || parameter.ty != property.ty
            || !properties.insert(property.id)
            || !plan.constructs.contains(&mir::RetainedSource {
                local: parameter.id,
                inherited: false,
                writable: access.is_writable(),
                lifetime: mir::RetainedLifetime::Cleanup,
                owned_path: Vec::new(),
            })
        {
            return Err(malformed_mir(
                "retained promotion must match its constructor input's type and access",
            ));
        }
    }
    Ok(())
}

fn validate_storage_selection(
    program: &mir::Program,
    function: &mir::Function,
    selection: &mir::RetainedStorageSelection,
) -> Result<(), BackendError> {
    validate_source_path(
        program,
        function,
        &mir::RetainedSource {
            local: selection.destination,
            inherited: false,
            writable: false,
            lifetime: mir::RetainedLifetime::Cleanup,
            owned_path: selection.path.clone(),
        },
    )?;
    if !function.params.contains(&selection.destination) {
        return Err(malformed_mir("storage selection must name an input owner"));
    }
    let mut previous = None;
    for selector in &selection.selectors {
        if previous.is_some_and(|previous| previous >= selector) {
            return Err(malformed_mir(
                "storage selection keys must be unique and ordered",
            ));
        }
        previous = Some(selector);
        if let mir::RetainedStorageSelector::Input(local) = selector {
            let input = local_in(function, *local)?;
            let capture = function.closure.as_ref().is_some_and(|closure| {
                closure
                    .capture_locals
                    .iter()
                    .any(|(_, capture)| capture == local)
            });
            if (!function.params.contains(local) && !capture)
                || !matches!(
                    input.ty,
                    mir::Type::Scalar(mir::ScalarType::Inline(
                        crate::inline_value::InlineValueType::RegistrationKey(_)
                    )) | mir::Type::NullableScalar(mir::ScalarType::Inline(
                        crate::inline_value::InlineValueType::RegistrationKey(_)
                    ))
                )
            {
                return Err(malformed_mir("storage selection must name a key input"));
            }
        }
    }
    Ok(())
}

fn validate_source_path(
    program: &mir::Program,
    function: &mir::Function,
    source: &mir::RetainedSource,
) -> Result<(), BackendError> {
    let mut ty = local_in(function, source.local)?.ty;
    for field in &source.owned_path {
        let valid_receiver = match ty {
            mir::Type::Class(class) | mir::Type::NullableClass(class) => {
                class_is_subtype(program, class, field.class)
            }
            mir::Type::Interface(interface) | mir::Type::NullableInterface(interface) => {
                program.interface_vtables.iter().any(|table| {
                    table.interface == interface
                        && matches!(table.implementing_type,
                            mir::ImplementingType::Class(class)
                                if class_is_subtype(program, class, field.class))
                })
            }
            _ => false,
        };
        let property = property_in(program, field.class, *field)?;
        if !valid_receiver || property.borrowed_source.is_some() {
            return Err(malformed_mir(
                "retained source path must name stored owning properties",
            ));
        }
        ty = property.ty;
    }
    Ok(())
}

pub(super) fn carries_source(program: &mir::Program, ty: mir::Type) -> bool {
    program.carries_retained_sources(ty)
}

pub(super) fn iteration_operation(
    program: &mir::Program,
    interface: mir::InterfaceTypeId,
    slot: usize,
) -> Option<crate::compiler_known_contracts::IterationOperation> {
    let interface = program.interface_types.get(interface.0)?;
    interface.iteration_operation(slot)
}

pub(super) fn indirect_sources(
    program: &mir::Program,
    callee: &mir::IndirectCallee,
    args: &[mir::Rvalue],
    roots: &[HashSet<Place>],
    loans: &[HashSet<Source>],
    origin: Option<&call_origins::CallbackOrigin>,
) -> Result<HashSet<Source>, BackendError> {
    if let mir::IndirectCallee::Closure(value) = callee {
        let contract = function_type_in(program, value.function_type())?;
        if let Some(origin) = origin.filter(|origin| !origin.unknown) {
            if origin.targets.is_empty() {
                return Err(malformed_mir(
                    "callback has no executable return-source origin",
                ));
            }
            let mut sources = HashSet::new();
            for target in &origin.targets {
                let function = function_in(program, *target)?;
                sources.extend(direct_call_sources(
                    program,
                    function,
                    CallInputs { args, implicit: 1 },
                    false,
                    roots,
                    loans,
                )?);
            }
            return Ok(sources);
        }
        let Some(borrow) = contract.return_borrow else {
            return Ok(HashSet::new());
        };
        let mir::BorrowSource::Parameter(index) = borrow.source else {
            return Err(malformed_mir(
                "retained callback return has no explicit parameter source",
            ));
        };
        let source = args
            .get(index)
            .ok_or_else(|| malformed_mir("retained callback return source is missing"))?;
        return match borrow.kind {
            // A returned carrier can inherit its existing loans. A borrowed
            // leaf (for example a cursor's element) only inherits ordinary
            // view roots, not every retained loan held by the source carrier.
            mir::ReturnBorrowKind::Value => {
                if matches!(contract.return_type, mir::ReturnType::Value(ty)
                    if carries_source(program, ty)
                        || matches!(ty, mir::Type::Function(_) | mir::Type::NullableFunction(_)))
                {
                    value_sources(program, source, roots, loans)
                } else {
                    Ok(HashSet::new())
                }
            }
            mir::ReturnBorrowKind::Retained => {
                if contract.parameters[index].mode == mir::FunctionParameterMode::Take {
                    // Taking a carrier forwards its existing retained sources;
                    // it cannot borrow the callee's consumed parameter storage.
                    value_sources(program, source, roots, loans)
                } else {
                    borrowed_sources(
                        program,
                        source,
                        borrow.writable,
                        borrow.lifetime,
                        roots,
                        loans,
                    )
                }
            }
        };
    }
    let mir::IndirectCallee::InterfaceMethod {
        interface, slot, ..
    } = callee
    else {
        return Ok(HashSet::new());
    };
    let mut sources = HashSet::new();
    if iteration_operation(program, *interface, *slot)
        == Some(crate::compiler_known_contracts::IterationOperation::Acquire)
    {
        let receiver = args
            .first()
            .ok_or_else(|| malformed_mir("iterator acquisition has no receiver"))?;
        sources.extend(
            source_roots(program, receiver, roots)?
                .into_iter()
                .map(|place| Source::new(place, false, false, mir::RetainedLifetime::Cleanup)),
        );
    }
    for table in &program.interface_vtables {
        if table.interface == *interface {
            let entry = table
                .methods
                .get(*slot)
                .ok_or_else(|| malformed_mir("retained interface call has no method entry"))?;
            sources.extend(call_sources(program, *entry, args, false, roots, loans)?);
        }
    }
    Ok(sources)
}

pub(super) fn possible_callees(
    program: &mir::Program,
    callee: mir::FunctionId,
    receiver: Option<crate::class_layout::ClassId>,
) -> Result<Vec<&mir::Function>, BackendError> {
    let callee = function_in(program, callee)?;
    let mut candidates = vec![callee];
    if let (Some(slot), Some(method)) = (callee.virtual_slot, &callee.method) {
        let receiver =
            receiver.ok_or_else(|| malformed_mir("virtual call has no checked receiver domain"))?;
        if !class_is_subtype(program, receiver, method.class) {
            return Err(malformed_mir(
                "virtual receiver domain is outside its method hierarchy",
            ));
        }
        let family = method
            .virtual_family
            .as_ref()
            .ok_or_else(|| malformed_mir("virtual callable has no canonical family"))?;
        for class in &program.classes {
            if class.id == receiver || class.ancestors.contains(&receiver) {
                let entry = class
                    .virtual_methods
                    .get(slot as usize)
                    .ok_or_else(|| malformed_mir("retained virtual call has no method entry"))?;
                if &entry.family != family {
                    return Err(malformed_mir(
                        "virtual call selects another canonical family",
                    ));
                }
                let target = entry.implementation.ok_or_else(|| {
                    malformed_mir("virtual call can enter an inactive method slot")
                })?;
                if !candidates.iter().any(|candidate| candidate.id == target) {
                    candidates.push(function_in(program, target)?);
                }
            }
        }
    }
    Ok(candidates)
}

pub(super) fn receiver_class(args: &[mir::Rvalue]) -> Option<crate::class_layout::ClassId> {
    match args.first()? {
        mir::Rvalue::Class(value) => Some(value.static_receiver_class()),
        mir::Rvalue::NullableClass(value) => Some(value.static_receiver_class()),
        _ => None,
    }
}

pub(super) fn call_sources(
    program: &mir::Program,
    callee: mir::FunctionId,
    args: &[mir::Rvalue],
    constructing: bool,
    roots: &[HashSet<Place>],
    loans: &[HashSet<Source>],
) -> Result<HashSet<Source>, BackendError> {
    let mut result = HashSet::new();
    for callee in possible_callees(program, callee, receiver_class(args))? {
        result.extend(direct_call_sources(
            program,
            callee,
            CallInputs {
                args,
                implicit: usize::from(constructing),
            },
            constructing,
            roots,
            loans,
        )?);
    }
    Ok(result)
}

#[derive(Clone, Copy)]
pub(super) struct CallInputs<'a> {
    pub(super) args: &'a [mir::Rvalue],
    pub(super) implicit: usize,
}

impl<'a> CallInputs<'a> {
    pub(super) fn get(
        self,
        callee: &mir::Function,
        local: mir::LocalId,
    ) -> Result<Option<&'a mir::Rvalue>, BackendError> {
        let index = callee
            .params
            .iter()
            .position(|parameter| *parameter == local)
            .ok_or_else(|| malformed_mir("stored owner contract has no actual input"))?;
        let Some(index) = index.checked_sub(self.implicit) else {
            return Ok(None);
        };
        self.args
            .get(index)
            .map(Some)
            .ok_or_else(|| malformed_mir("stored owner contract has no actual input"))
    }
}

impl<'a> From<&'a [mir::Rvalue]> for CallInputs<'a> {
    fn from(args: &'a [mir::Rvalue]) -> Self {
        Self { args, implicit: 0 }
    }
}

pub(super) fn direct_call_sources(
    program: &mir::Program,
    callee: &mir::Function,
    args: CallInputs<'_>,
    constructing: bool,
    roots: &[HashSet<Place>],
    loans: &[HashSet<Source>],
) -> Result<HashSet<Source>, BackendError> {
    let Some(plan) = contract(callee) else {
        return Ok(HashSet::new());
    };
    let borrows_carrier = !constructing
        && callee
            .return_borrow
            .is_some_and(|borrow| borrow.kind == mir::ReturnBorrowKind::Value);
    let sources = if constructing {
        plan.constructs.clone()
    } else {
        plan.returns.clone()
    };
    let mut result = HashSet::new();
    for source in sources {
        // A Value contract tracks the returned carrier's lifetime through view
        // roots. Only inherited plan entries describe loans inside that
        // carrier; noninherited entries must not manufacture an owned carrier.
        if borrows_carrier && !source.inherited {
            continue;
        }
        let value = args
            .get(callee, source.local)?
            .ok_or_else(|| malformed_mir("retained call source names an implicit input"))?;
        if source.inherited {
            let consumed = local_in(callee, source.local)?.owned;
            result.extend(
                value_sources(program, value, roots, loans)?
                    .into_iter()
                    .map(|mut inherited| {
                        if !consumed {
                            inherited.lifetime = inherited.lifetime.join(source.lifetime);
                        }
                        inherited
                    }),
            );
        } else if source.writable || value.ty().has_move_ownership() {
            result.extend(
                source_roots(program, value, roots)?
                    .into_iter()
                    .map(|mut place| {
                        if !place.opaque {
                            place.fields.extend(&source.owned_path);
                        }
                        Source::new(place, false, source.writable, source.lifetime)
                    }),
            );
        }
    }
    Ok(result)
}

pub(super) fn source_roots(
    program: &mir::Program,
    value: &mir::Rvalue,
    roots: &[HashSet<Place>],
) -> Result<HashSet<Place>, BackendError> {
    match value {
        mir::Rvalue::Class(mir::ClassExpression::Upcast { value, .. }) => {
            return source_roots(program, &mir::Rvalue::Class((**value).clone()), roots)
        }
        mir::Rvalue::NullableClass(mir::NullableClassExpression::Upcast { value, .. }) => {
            return source_roots(
                program,
                &mir::Rvalue::NullableClass((**value).clone()),
                roots,
            )
        }
        _ => {}
    }
    // Literal absence has no payload owner to retain. All non-null sources
    // still require a materialized owner; nullable carrier locals keep theirs.
    if value.is_null_value() {
        return Ok(HashSet::new());
    }
    // A Value-return call freezes subdivision below its actual input place,
    // not below that input's containing owner. Resolve the argument first.
    let call = match value {
        mir::Rvalue::Class(mir::ClassExpression::Call { function, args, .. })
        | mir::Rvalue::NullableClass(mir::NullableClassExpression::Call {
            function, args, ..
        })
        | mir::Rvalue::Collection(mir::CollectionExpression::Call { function, args, .. })
        | mir::Rvalue::NullableCollection(mir::NullableCollectionExpression::Call {
            function,
            args,
            ..
        })
        | mir::Rvalue::Function(mir::FunctionExpression::Call { function, args, .. })
        | mir::Rvalue::NullableFunction(mir::NullableFunctionExpression::Call {
            function,
            args,
            ..
        })
        | mir::Rvalue::Mixed(mir::MixedExpression::Call { function, args, .. })
        | mir::Rvalue::Interface(mir::InterfaceExpression {
            value: mir::InterfaceValue::Call { function, args, .. },
            ..
        })
        | mir::Rvalue::NullableInterface(mir::NullableInterfaceExpression {
            value: mir::NullableInterfaceValue::Call { function, args, .. },
            ..
        }) => Some((*function, args.as_slice())),
        _ => None,
    };
    if let Some((callee, args)) = call {
        if let Some(borrow) = function_in(program, callee)?.return_borrow {
            let input = borrowed_call_rvalue_source(program, callee, args, borrow)?;
            let pipe = child_pipe_place(program, callee)?;
            return Ok(source_roots(program, input, roots)?
                .into_iter()
                .map(|place| {
                    let mut place = pipe.map_or(place.clone(), |slot| place.project(slot));
                    place.opaque = true;
                    place
                })
                .collect());
        }
    }
    match value {
        // Validated payload forwarding and interface upcasts preserve the
        // physical allocation, including any known containing-field prefix.
        mir::Rvalue::Class(mir::ClassExpression::InterfaceReceiver { receiver, .. })
        | mir::Rvalue::Collection(mir::CollectionExpression::InterfaceReceiver {
            receiver, ..
        })
        | mir::Rvalue::Value(mir::ValueExpression::Inline(
            mir::InlineValueExpression::InterfaceReceiver { receiver, .. },
        )) => return Ok(roots[receiver.0].clone()),
        mir::Rvalue::Class(mir::ClassExpression::InterfacePayload { local, .. })
        | mir::Rvalue::Interface(mir::InterfaceExpression {
            value: mir::InterfaceValue::NarrowedLocal { local, .. },
            ..
        }) => return Ok(roots[local.0].clone()),
        mir::Rvalue::Interface(mir::InterfaceExpression {
            value: mir::InterfaceValue::Upcast { source, .. },
            ..
        }) => return source_roots(program, &mir::Rvalue::Interface((**source).clone()), roots),
        mir::Rvalue::NullableInterface(mir::NullableInterfaceExpression {
            value: mir::NullableInterfaceValue::Upcast { source, .. },
            ..
        }) => {
            return source_roots(
                program,
                &mir::Rvalue::NullableInterface((**source).clone()),
                roots,
            );
        }
        mir::Rvalue::NullableInterface(mir::NullableInterfaceExpression {
            interface,
            value: mir::NullableInterfaceValue::Present(value),
        }) => {
            return source_roots(
                program,
                &mir::Rvalue::Interface(mir::InterfaceExpression {
                    interface: *interface,
                    value: value.clone(),
                }),
                roots,
            );
        }
        mir::Rvalue::Interface(mir::InterfaceExpression {
            value: mir::InterfaceValue::FromValue { value, .. },
            ..
        }) => return source_roots(program, value, roots),
        mir::Rvalue::NullableClass(mir::NullableClassExpression::Class(value)) => {
            return source_roots(program, &mir::Rvalue::Class(value.clone()), roots);
        }
        mir::Rvalue::NullableCollection(mir::NullableCollectionExpression::Collection(value)) => {
            return source_roots(program, &mir::Rvalue::Collection(value.clone()), roots);
        }
        mir::Rvalue::Interface(mir::InterfaceExpression {
            value: mir::InterfaceValue::FromClass { object, .. },
            ..
        }) => return source_roots(program, &mir::Rvalue::Class((**object).clone()), roots),
        mir::Rvalue::Interface(mir::InterfaceExpression {
            value: mir::InterfaceValue::FromNullableClass { object, .. },
            ..
        }) => {
            return source_roots(
                program,
                &mir::Rvalue::NullableClass((**object).clone()),
                roots,
            );
        }
        _ => {}
    }
    if let Some((object, property)) = source_property_place(value) {
        let borrowed = property_in(program, property.class, property)?
            .borrowed_source
            .is_some();
        return Ok(roots[object.0]
            .iter()
            .map(|place| {
                // A retained field is its own slot of the owner; the caller's
                // source behind it has no structure visible here.
                let mut place = place.project(property);
                place.opaque |= borrowed;
                place
            })
            .collect());
    }
    let locals = if let Some(local) = value.direct_place_local() {
        vec![local]
    } else {
        escaping_class_local_borrows(program, value)?
            .into_iter()
            .collect()
    };
    if locals.is_empty() {
        return Err(malformed_mir(
            "retained source has no materialized lifetime owner",
        ));
    }
    let opaque = value.direct_place_local().is_none();
    Ok(locals
        .into_iter()
        .flat_map(|local| roots[local.0].iter().cloned())
        .map(|mut place| {
            place.opaque |= opaque;
            place
        })
        .collect())
}

/// Structural checks do not have CFG alias facts. Keep syntactically proven
/// fields exact here; the ordinary view proof subsequently resolves aliases.
pub(super) fn static_places(
    program: &mir::Program,
    value: &mir::Rvalue,
    retains_writable_place: bool,
) -> Result<HashSet<Place>, BackendError> {
    let mut locals = escaping_class_local_borrows(program, value)?;
    if retains_writable_place {
        locals.extend(
            value
                .direct_place_local()
                .or_else(|| source_property_place(value).map(|(object, _)| object)),
        );
    }
    let Some(last) = locals.iter().map(|local| local.0).max() else {
        return Ok(HashSet::new());
    };
    let roots = (0..=last)
        .map(|index| HashSet::from([Place::local(mir::LocalId(index))]))
        .collect::<Vec<_>>();
    source_roots(program, value, &roots)
}

pub(super) fn source_property_place(
    value: &mir::Rvalue,
) -> Option<(mir::LocalId, crate::class_layout::PropertyId)> {
    use crate::native_closure_abi::{direct_borrow_home_place, BorrowHomePlace};
    stored_property(value).or_else(|| match direct_borrow_home_place(value) {
        Some(BorrowHomePlace::Property { object, property }) => Some((object, property)),
        _ => None,
    })
}

pub(super) fn stored_property(
    value: &mir::Rvalue,
) -> Option<(mir::LocalId, crate::class_layout::PropertyId)> {
    match value {
        mir::Rvalue::Class(mir::ClassExpression::Upcast { value, .. }) => {
            stored_property(&mir::Rvalue::Class((**value).clone()))
        }
        mir::Rvalue::NullableClass(mir::NullableClassExpression::Upcast { value, .. }) => {
            stored_property(&mir::Rvalue::NullableClass((**value).clone()))
        }
        mir::Rvalue::Class(mir::ClassExpression::Property {
            object, property, ..
        })
        | mir::Rvalue::NullableClass(mir::NullableClassExpression::Property {
            object,
            property,
            ..
        })
        | mir::Rvalue::Collection(mir::CollectionExpression::Property {
            object, property, ..
        })
        | mir::Rvalue::NullableCollection(mir::NullableCollectionExpression::Property {
            object,
            property,
            ..
        })
        | mir::Rvalue::Function(mir::FunctionExpression::Property {
            object, property, ..
        })
        | mir::Rvalue::NullableFunction(mir::NullableFunctionExpression::Property {
            object,
            property,
            ..
        })
        | mir::Rvalue::Mixed(mir::MixedExpression::Property { object, property })
        | mir::Rvalue::NullableMixed(mir::NullableMixedExpression::Property { object, property })
        | mir::Rvalue::Interface(mir::InterfaceExpression {
            value:
                mir::InterfaceValue::Property {
                    object, property, ..
                },
            ..
        })
        | mir::Rvalue::NullableInterface(mir::NullableInterfaceExpression {
            value:
                mir::NullableInterfaceValue::Property {
                    object, property, ..
                },
            ..
        }) => Some((*object, *property)),
        _ => None,
    }
}

pub(super) fn borrowed_sources(
    program: &mir::Program,
    value: &mir::Rvalue,
    writable: bool,
    lifetime: mir::RetainedLifetime,
    roots: &[HashSet<Place>],
    loans: &[HashSet<Source>],
) -> Result<HashSet<Source>, BackendError> {
    if byte_regions::kind(value.ty()).is_some() {
        // A reborrow preserves the original loan, including its exclusivity.
        // Access to the immediate descriptor is tracked separately.
        Ok(value_sources(program, value, roots, loans)?
            .into_iter()
            .map(|mut source| {
                source.lifetime = source.lifetime.join(lifetime);
                source
            })
            .collect())
    } else {
        Ok(source_roots(program, value, roots)?
            .into_iter()
            .map(|place| Source::new(place, false, writable, lifetime))
            .collect())
    }
}

fn region_sources(
    program: &mir::Program,
    value: &mir::ByteRegionExpression,
    roots: &[HashSet<Place>],
    loans: &[HashSet<Source>],
) -> Result<HashSet<Source>, BackendError> {
    use mir::ByteRegionExpression as E;
    match value {
        E::CollectionIndex { collection, .. } => Ok(loans[collection.0].clone()),
        E::Local { local, .. } => Ok(loans[local.0].clone()),
        E::Region { kind, source, .. } => borrowed_sources(
            program,
            source,
            kind.is_writable(),
            mir::RetainedLifetime::LastUse,
            roots,
            loans,
        ),
        E::Call { function, args, .. } => {
            call_sources(program, *function, args, false, roots, loans)
        }
        E::AssumePresent { value, .. } => nullable_region_sources(program, value, roots, loans),
        E::Coalesce { left, right, .. } => {
            let mut sources = nullable_region_sources(program, left, roots, loans)?;
            sources.extend(region_sources(program, right, roots, loans)?);
            Ok(sources)
        }
    }
}

fn nullable_region_sources(
    program: &mir::Program,
    value: &mir::NullableByteRegionExpression,
    roots: &[HashSet<Place>],
    loans: &[HashSet<Source>],
) -> Result<HashSet<Source>, BackendError> {
    use mir::NullableByteRegionExpression as E;
    match value {
        E::CollectionIndex { collection, .. } | E::CollectionGet { collection, .. } => {
            Ok(loans[collection.0].clone())
        }
        E::Null { .. } => Ok(HashSet::new()),
        E::Present(value) => region_sources(program, value, roots, loans),
        E::Local { local, .. } => Ok(loans[local.0].clone()),
        E::Call { function, args, .. } => {
            call_sources(program, *function, args, false, roots, loans)
        }
    }
}

pub(super) fn collection_element_source(value: &mir::Rvalue) -> Option<mir::LocalId> {
    collection_element_place(value).map(|(local, _)| local)
}

pub(super) fn collection_element_place(
    value: &mir::Rvalue,
) -> Option<(mir::LocalId, &mir::Rvalue)> {
    if let mir::Rvalue::Class(value) = value {
        return match value.without_upcasts() {
            mir::ClassExpression::CollectionIndex {
                collection, index, ..
            } => Some((*collection, index)),
            _ => None,
        };
    }
    if let mir::Rvalue::NullableClass(value) = value {
        return match value.without_upcasts() {
            mir::NullableClassExpression::DictionaryGet {
                collection, key, ..
            } => Some((*collection, key)),
            _ => None,
        };
    }
    match value {
        mir::Rvalue::ByteRegion(value) => region_collection_element_place(value),
        mir::Rvalue::NullableByteRegion(value) => nullable_region_collection_element_place(value),
        mir::Rvalue::Function(mir::FunctionExpression::CollectionIndex {
            collection,
            index,
            ..
        })
        | mir::Rvalue::NullableFunction(
            mir::NullableFunctionExpression::CollectionIndex {
                collection, index, ..
            }
            | mir::NullableFunctionExpression::DictionaryGet {
                collection,
                key: index,
                ..
            },
        )
        | mir::Rvalue::Interface(mir::InterfaceExpression {
            value:
                mir::InterfaceValue::CollectionIndex {
                    collection, index, ..
                },
            ..
        })
        | mir::Rvalue::NullableInterface(mir::NullableInterfaceExpression {
            value:
                mir::NullableInterfaceValue::CollectionIndex {
                    collection, index, ..
                }
                | mir::NullableInterfaceValue::DictionaryGet {
                    collection,
                    key: index,
                    ..
                },
            ..
        })
        | mir::Rvalue::Mixed(mir::MixedExpression::CollectionIndex {
            collection, index, ..
        })
        | mir::Rvalue::NullableMixed(
            mir::NullableMixedExpression::CollectionIndex {
                collection, index, ..
            }
            | mir::NullableMixedExpression::CollectionGet {
                collection,
                key: index,
                ..
            },
        ) => Some((*collection, index)),
        mir::Rvalue::Collection(mir::CollectionExpression::Index { source, index, .. })
        | mir::Rvalue::NullableCollection(
            mir::NullableCollectionExpression::Index { source, index, .. }
            | mir::NullableCollectionExpression::CollectionGet {
                source, key: index, ..
            },
        ) => Some((*source, index)),
        _ => None,
    }
}

fn region_collection_element_place(
    value: &mir::ByteRegionExpression,
) -> Option<(mir::LocalId, &mir::Rvalue)> {
    match value {
        mir::ByteRegionExpression::CollectionIndex {
            collection, index, ..
        } => Some((*collection, index)),
        mir::ByteRegionExpression::AssumePresent { value, .. } => {
            nullable_region_collection_element_place(value)
        }
        _ => None,
    }
}

fn nullable_region_collection_element_place(
    value: &mir::NullableByteRegionExpression,
) -> Option<(mir::LocalId, &mir::Rvalue)> {
    match value {
        mir::NullableByteRegionExpression::CollectionIndex {
            collection, index, ..
        }
        | mir::NullableByteRegionExpression::CollectionGet {
            collection,
            key: index,
            ..
        } => Some((*collection, index)),
        mir::NullableByteRegionExpression::Present(value) => region_collection_element_place(value),
        _ => None,
    }
}

pub(super) fn value_sources(
    program: &mir::Program,
    value: &mir::Rvalue,
    roots: &[HashSet<Place>],
    loans: &[HashSet<Source>],
) -> Result<HashSet<Source>, BackendError> {
    match value {
        mir::Rvalue::Class(mir::ClassExpression::Upcast { value, .. }) => {
            return value_sources(
                program,
                &mir::Rvalue::Class((**value).clone()),
                roots,
                loans,
            );
        }
        mir::Rvalue::NullableClass(mir::NullableClassExpression::Upcast { value, .. }) => {
            return value_sources(
                program,
                &mir::Rvalue::NullableClass((**value).clone()),
                roots,
                loans,
            );
        }
        _ => {}
    }
    // A payload enum is a value, not a view of its place: it holds only the
    // retained sources its fields can carry, wherever it is read from.
    if matches!(
        value,
        mir::Rvalue::PayloadEnum(_) | mir::Rvalue::NullablePayloadEnum(_)
    ) && !carries_source(program, value.ty())
    {
        return Ok(HashSet::new());
    }
    if let Some(local) = value.direct_place_local() {
        return Ok(loans[local.0].clone());
    }
    if carries_source(program, value.ty()) {
        if let Some(source) = collection_element_source(value) {
            return Ok(loans[source.0].clone());
        }
    }
    match value {
        mir::Rvalue::PayloadEnum(value) => payload_enum_sources(program, value, roots, loans),
        mir::Rvalue::NullablePayloadEnum(value) => {
            nullable_payload_enum_sources(program, value, roots, loans)
        }
        mir::Rvalue::Collection(mir::CollectionExpression::Literal { entries, .. }) => {
            let mut sources = HashSet::new();
            for entry in entries {
                for value in entry.key.iter().chain(std::iter::once(&entry.value)) {
                    sources.extend(value_sources(program, value, roots, loans)?);
                }
            }
            Ok(sources)
        }
        mir::Rvalue::Collection(mir::CollectionExpression::Fill { value, .. }) => {
            value_sources(program, value, roots, loans)
        }
        mir::Rvalue::ByteRegion(value) => region_sources(program, value, roots, loans),
        mir::Rvalue::NullableByteRegion(value) => {
            nullable_region_sources(program, value, roots, loans)
        }
        mir::Rvalue::Interface(mir::InterfaceExpression {
            value: mir::InterfaceValue::NewCollectionIterator { source, .. },
            ..
        }) => Ok(roots[source.0]
            .iter()
            .map(|place| Source::new(place.clone(), false, false, mir::RetainedLifetime::Cleanup))
            .collect()),
        mir::Rvalue::Interface(mir::InterfaceExpression {
            value: mir::InterfaceValue::FromValue { value, .. },
            ..
        }) => value_sources(program, value, roots, loans),
        mir::Rvalue::Collection(
            mir::CollectionExpression::InterfaceReceiver { receiver, .. }
            | mir::CollectionExpression::IteratorSource { receiver, .. },
        ) => Ok(loans[receiver.0].clone()),
        mir::Rvalue::Class(mir::ClassExpression::New {
            constructor: Some(callee),
            args,
            ..
        }) => call_sources(program, *callee, args, true, roots, loans),
        mir::Rvalue::Class(mir::ClassExpression::Call {
            function: callee,
            args,
            ..
        })
        | mir::Rvalue::Function(mir::FunctionExpression::Call {
            function: callee,
            args,
            ..
        })
        | mir::Rvalue::NullableFunction(mir::NullableFunctionExpression::Call {
            function: callee,
            args,
            ..
        })
        | mir::Rvalue::NullableClass(mir::NullableClassExpression::Call {
            function: callee,
            args,
            ..
        })
        | mir::Rvalue::Collection(mir::CollectionExpression::Call {
            function: callee,
            args,
            ..
        })
        | mir::Rvalue::NullableCollection(mir::NullableCollectionExpression::Call {
            function: callee,
            args,
            ..
        })
        | mir::Rvalue::Interface(mir::InterfaceExpression {
            value:
                mir::InterfaceValue::Call {
                    function: callee,
                    args,
                    ..
                },
            ..
        })
        | mir::Rvalue::NullableInterface(mir::NullableInterfaceExpression {
            value:
                mir::NullableInterfaceValue::Call {
                    function: callee,
                    args,
                    ..
                },
            ..
        }) => call_sources(program, *callee, args, false, roots, loans),
        mir::Rvalue::NullableClass(mir::NullableClassExpression::Class(value)) => {
            value_sources(program, &mir::Rvalue::Class(value.clone()), roots, loans)
        }
        mir::Rvalue::NullableCollection(mir::NullableCollectionExpression::Collection(value)) => {
            value_sources(
                program,
                &mir::Rvalue::Collection(value.clone()),
                roots,
                loans,
            )
        }
        mir::Rvalue::NullableMixed(mir::NullableMixedExpression::Mixed(value)) => {
            value_sources(program, &mir::Rvalue::Mixed(value.clone()), roots, loans)
        }
        mir::Rvalue::Interface(mir::InterfaceExpression {
            value: mir::InterfaceValue::FromClass { object, .. },
            ..
        }) => value_sources(
            program,
            &mir::Rvalue::Class((**object).clone()),
            roots,
            loans,
        ),
        mir::Rvalue::Class(
            mir::ClassExpression::InterfaceReceiver {
                receiver: local, ..
            }
            | mir::ClassExpression::InterfacePayload { local, .. }
            | mir::ClassExpression::MixedPayload { mixed: local, .. },
        )
        | mir::Rvalue::Interface(mir::InterfaceExpression {
            value: mir::InterfaceValue::NarrowedLocal { local, .. },
            ..
        })
        | mir::Rvalue::Function(mir::FunctionExpression::MixedPayload { mixed: local, .. }) => {
            Ok(loans[local.0].clone())
        }
        mir::Rvalue::Value(mir::ValueExpression::Inline(
            mir::InlineValueExpression::InterfaceReceiver { receiver, .. },
        )) => Ok(loans[receiver.0].clone()),
        mir::Rvalue::Class(mir::ClassExpression::Coalesce { left, right, .. }) => {
            let mut sources = value_sources(
                program,
                &mir::Rvalue::NullableClass((**left).clone()),
                roots,
                loans,
            )?;
            sources.extend(value_sources(
                program,
                &mir::Rvalue::Class((**right).clone()),
                roots,
                loans,
            )?);
            Ok(sources)
        }
        mir::Rvalue::Interface(mir::InterfaceExpression {
            value: mir::InterfaceValue::FromNullableClass { object, .. },
            ..
        }) => value_sources(
            program,
            &mir::Rvalue::NullableClass((**object).clone()),
            roots,
            loans,
        ),
        mir::Rvalue::Interface(mir::InterfaceExpression {
            value: mir::InterfaceValue::Upcast { source, .. },
            ..
        }) => value_sources(
            program,
            &mir::Rvalue::Interface((**source).clone()),
            roots,
            loans,
        ),
        mir::Rvalue::NullableInterface(mir::NullableInterfaceExpression {
            value: mir::NullableInterfaceValue::Upcast { source, .. },
            ..
        }) => value_sources(
            program,
            &mir::Rvalue::NullableInterface((**source).clone()),
            roots,
            loans,
        ),
        mir::Rvalue::NullableInterface(mir::NullableInterfaceExpression {
            interface,
            value: mir::NullableInterfaceValue::Present(value),
        }) => value_sources(
            program,
            &mir::Rvalue::Interface(mir::InterfaceExpression {
                interface: *interface,
                value: value.clone(),
            }),
            roots,
            loans,
        ),
        mir::Rvalue::Function(mir::FunctionExpression::Create { captures, .. }) => {
            let mut sources = HashSet::new();
            for capture in captures {
                match capture {
                    mir::ClosureCaptureOperand::BorrowLocal { local, writable } => {
                        sources.extend(roots[local.0].iter().map(|place| {
                            Source::new(
                                place.clone(),
                                false,
                                *writable,
                                mir::RetainedLifetime::LastUse,
                            )
                        }));
                        // This environment borrows the carrier, not its owned
                        // contents. The carrier keeps its own loans while this
                        // dependency is live; do not manufacture an inherited
                        // ownership contract for a borrowed capture.
                    }
                    mir::ClosureCaptureOperand::CopyValue(value)
                    | mir::ClosureCaptureOperand::MoveValue(value) => {
                        sources.extend(value_sources(program, value, roots, loans)?)
                    }
                }
            }
            Ok(sources)
        }
        mir::Rvalue::NullableFunction(mir::NullableFunctionExpression::Present(value)) => {
            value_sources(program, &mir::Rvalue::Function(value.clone()), roots, loans)
        }
        mir::Rvalue::Function(mir::FunctionExpression::AssumePresent { value, .. }) => {
            value_sources(
                program,
                &mir::Rvalue::NullableFunction((**value).clone()),
                roots,
                loans,
            )
        }
        mir::Rvalue::Mixed(mir::MixedExpression::BoxClass { value, .. }) => {
            value_sources(program, &mir::Rvalue::Class(value.clone()), roots, loans)
        }
        mir::Rvalue::Mixed(mir::MixedExpression::BoxInterface { value, .. }) => value_sources(
            program,
            &mir::Rvalue::Interface((**value).clone()),
            roots,
            loans,
        ),
        mir::Rvalue::Mixed(mir::MixedExpression::BoxFunction { value, .. }) => value_sources(
            program,
            &mir::Rvalue::Function((**value).clone()),
            roots,
            loans,
        ),
        mir::Rvalue::Mixed(mir::MixedExpression::BoxPayloadEnum { value }) => {
            payload_enum_sources(program, value, roots, loans)
        }
        mir::Rvalue::NullableMixed(mir::NullableMixedExpression::BoxNullablePayloadEnum(value)) => {
            nullable_payload_enum_sources(program, value, roots, loans)
        }
        _ => Ok(HashSet::new()),
    }
}

fn payload_enum_sources(
    program: &mir::Program,
    value: &mir::PayloadEnumExpression,
    roots: &[HashSet<Place>],
    loans: &[HashSet<Source>],
) -> Result<HashSet<Source>, BackendError> {
    use mir::PayloadEnumExpression as E;
    match value {
        E::Construct { fields, .. } => {
            let mut sources = HashSet::new();
            for field in fields {
                sources.extend(value_sources(program, field, roots, loans)?);
            }
            Ok(sources)
        }
        E::Call { function, args, .. } => {
            call_sources(program, *function, args, false, roots, loans)
        }
        E::Coalesce { left, right, .. } => {
            match nullable_payload_enum_expression_presence(left) {
                Some(true) => return nullable_payload_enum_sources(program, left, roots, loans),
                Some(false) => return payload_enum_sources(program, right, roots, loans),
                None => {}
            }
            let mut sources = nullable_payload_enum_sources(program, left, roots, loans)?;
            sources.extend(payload_enum_sources(program, right, roots, loans)?);
            Ok(sources)
        }
        E::Use { place, .. } => Ok(payload_enum_place_sources(place, loans)),
    }
}

fn nullable_payload_enum_sources(
    program: &mir::Program,
    value: &mir::NullablePayloadEnumExpression,
    roots: &[HashSet<Place>],
    loans: &[HashSet<Source>],
) -> Result<HashSet<Source>, BackendError> {
    use mir::NullablePayloadEnumExpression as E;
    match value {
        E::Null(_) => Ok(HashSet::new()),
        E::Value(value) => payload_enum_sources(program, value, roots, loans),
        E::Call { function, args, .. } => {
            call_sources(program, *function, args, false, roots, loans)
        }
        E::Use { place, .. } => Ok(payload_enum_place_sources(place, loans)),
        E::CollectionGet { collection, .. } => Ok(loans[collection.0].clone()),
        E::Coalesce { left, right, .. } => {
            match nullable_payload_enum_expression_presence(left) {
                Some(true) => return nullable_payload_enum_sources(program, left, roots, loans),
                Some(false) => return nullable_payload_enum_sources(program, right, roots, loans),
                None => {}
            }
            let mut sources = nullable_payload_enum_sources(program, left, roots, loans)?;
            sources.extend(nullable_payload_enum_sources(program, right, roots, loans)?);
            Ok(sources)
        }
    }
}

fn payload_enum_place_sources(
    place: &mir::PayloadEnumPlace,
    loans: &[HashSet<Source>],
) -> HashSet<Source> {
    match place {
        mir::PayloadEnumPlace::Local(local)
        | mir::PayloadEnumPlace::NullableLocalAssumeNonNull(local) => loans[local.0].clone(),
        mir::PayloadEnumPlace::Property { object, .. } => loans[object.0].clone(),
        mir::PayloadEnumPlace::CollectionIndex { collection, .. } => loans[collection.0].clone(),
        mir::PayloadEnumPlace::MixedPayload { mixed } => loans[mixed.0].clone(),
        mir::PayloadEnumPlace::Static(_) => HashSet::new(),
    }
}

pub(super) fn validate_return(
    program: &mir::Program,
    function: &mir::Function,
    sources: &HashSet<Source>,
) -> Result<(), BackendError> {
    for source in sources {
        validate_receiver_source(program, function, source)?;
        let local = local_in(function, source.local)?;
        if !function.params.contains(&source.local)
            || (!source.inherited && local.owned)
            || contract(function).is_none_or(|plan| {
                !plan.returns.iter().any(|declared| {
                    declared.local == source.local
                        && declared.inherited == source.inherited
                        && source.place.fields.starts_with(&declared.owned_path)
                        && (!source.writable || declared.writable)
                        && declared.lifetime.accepts(source.lifetime)
                })
            })
        {
            return Err(malformed_mir(format!(
                "return loses or outlives a retained source loan in {}: {:?}",
                function.name, source
            )));
        }
    }
    Ok(())
}

pub(super) fn validate_independent(
    function: &mir::Function,
    sources: &HashSet<Source>,
) -> Result<(), BackendError> {
    for source in sources {
        if !source.inherited
            || contract(function)
                .is_none_or(|plan| !plan.independent_parameters.contains(&source.local))
        {
            return Err(malformed_mir(
                "retained source loan escapes into independent ownership",
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_property_storage(
    program: &mir::Program,
    function: &mir::Function,
    object: mir::LocalId,
    sources: &HashSet<Source>,
) -> Result<(), BackendError> {
    if is_direct_constructor_receiver(program, function, local_in(function, object)?)?
        && sources.iter().all(|source| {
            source.inherited
                && contract(function).is_some_and(|plan| plan.constructs.contains(&source.contract))
        })
    {
        return Ok(());
    }
    validate_independent(function, sources)
}

pub(super) fn validate_call_inputs(
    program: &mir::Program,
    function: &mir::Function,
    target: CallTarget,
    args: &[mir::Rvalue],
    roots: &[HashSet<Place>],
    loans: &[HashSet<Source>],
) -> Result<(), BackendError> {
    let CallTarget::Direct {
        function: callee,
        parameter_offset,
    } = target
    else {
        return Ok(());
    };
    for callee in possible_callees(program, callee, receiver_class(args))? {
        let Some(plan) = contract(callee) else {
            continue;
        };
        if parameter_offset == 0 && parent_constructor_target(program, function) == Some(callee.id)
        {
            let inputs = args
                .get(1..)
                .ok_or_else(|| malformed_mir("parent construction has no receiver"))?;
            let sources = direct_call_sources(
                program,
                callee,
                CallInputs {
                    args: inputs,
                    implicit: 1,
                },
                true,
                roots,
                loans,
            )?;
            if sources.iter().any(|source| {
                contract(function).is_none_or(|plan| !plan.constructs.contains(&source.contract))
            }) {
                return Err(malformed_mir(
                    "child construction loses its parent's retained source loans",
                ));
            }
        }
        for source in &plan.independent_parameters {
            let index = callee
                .params
                .iter()
                .position(|parameter| parameter == source)
                .and_then(|index| index.checked_sub(parameter_offset))
                .ok_or_else(|| malformed_mir("independent input has no caller argument"))?;
            let value = args
                .get(index)
                .ok_or_else(|| malformed_mir("independent input is missing"))?;
            validate_independent(function, &value_sources(program, value, roots, loans)?)?;
        }
    }
    Ok(())
}

pub(super) fn validate_indirect_inputs(
    program: &mir::Program,
    function: &mir::Function,
    terminator: &mir::Terminator,
    roots: &[HashSet<Place>],
    loans: &[HashSet<Source>],
) -> Result<(), BackendError> {
    let (callee, args) = match terminator {
        mir::Terminator::IndirectCall { callee, args, .. }
        | mir::Terminator::CheckedIndirectCall { callee, args, .. } => (callee, args),
        _ => return Ok(()),
    };
    let mir::IndirectCallee::InterfaceMethod {
        interface, slot, ..
    } = callee
    else {
        return Ok(());
    };
    for table in &program.interface_vtables {
        if table.interface == *interface {
            let entry = table
                .methods
                .get(*slot)
                .ok_or_else(|| malformed_mir("retained interface call has no method entry"))?;
            validate_call_inputs(
                program,
                function,
                CallTarget::Direct {
                    function: *entry,
                    parameter_offset: 0,
                },
                args,
                roots,
                loans,
            )?;
        }
    }
    Ok(())
}

//! Backend-independent structural and type validation for native MIR.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::backend::BackendError;
use crate::class_layout::{append_hidden_pointer, compute_class_layout, ClassId, FieldType};
use crate::mir;
use crate::numeric::{FloatType, IntegerType};

pub fn validate_program(program: &mir::Program) -> Result<(), BackendError> {
    validate_graph_metadata(program)?;
    validate_error_metadata(program)?;
    validate_closure_metadata(program)?;
    for (index, definition) in program.enums.iter().enumerate() {
        if definition.id != crate::enums::EnumId(index) {
            return Err(malformed_mir(format!(
                "enum table slot {index} contains enum#{}",
                definition.id.0
            )));
        }
        if definition.cases.is_empty() {
            return Err(malformed_mir(format!(
                "enum#{} has no cases",
                definition.id.0
            )));
        }
        for (case_index, case) in definition.cases.iter().enumerate() {
            if case.id.enum_id != definition.id || case.id.index != case_index {
                return Err(malformed_mir(format!(
                    "enum#{} case table slot {case_index} has invalid identity",
                    definition.id.0
                )));
            }
            if case.tag != case_index as u32 {
                return Err(malformed_mir(format!(
                    "enum#{} case {case_index} has invalid declaration-order tag",
                    definition.id.0
                )));
            }
            match (definition.backing_type, &case.backing_value) {
                (None, None)
                | (
                    Some(crate::enums::EnumBackingType::Int),
                    Some(crate::enums::EnumBackingValue::Int(_)),
                )
                | (
                    Some(crate::enums::EnumBackingType::String),
                    Some(crate::enums::EnumBackingValue::String(_)),
                ) => {}
                _ => {
                    return Err(malformed_mir(format!(
                        "enum#{} case {case_index} has invalid backing metadata",
                        definition.id.0
                    )));
                }
            }
            if definition.backing_type.is_some() && !case.payload.is_empty() {
                return Err(malformed_mir(format!(
                    "backed enum#{} case {case_index} has a payload",
                    definition.id.0
                )));
            }
            for field in &case.payload {
                validate_type(program, field.ty)?;
            }
        }
    }
    for (index, collection) in program.collection_types.iter().enumerate() {
        if collection.id != mir::CollectionTypeId(index) {
            return Err(malformed_mir(format!(
                "collection type table slot {index} contains collection#{}",
                collection.id.0
            )));
        }
        if let Some(key) = collection.key {
            validate_type(program, key)?;
        }
        validate_type(program, collection.value)?;
        if matches!(
            collection.kind,
            mir::CollectionKind::Dictionary | mir::CollectionKind::SortedDictionary
        ) != collection.key.is_some()
        {
            return Err(malformed_mir(format!(
                "collection#{} has an invalid key-type shape",
                collection.id.0
            )));
        }
        let ordered_type = match collection.kind {
            mir::CollectionKind::SortedDictionary => collection.key,
            mir::CollectionKind::SortedSet | mir::CollectionKind::PriorityQueue => {
                Some(collection.value)
            }
            _ => None,
        };
        if ordered_type.is_none() && collection.comparator.is_some() {
            return Err(malformed_mir(format!(
                "collection#{} must not carry a comparator identity",
                collection.id.0
            )));
        }
        let expected_comparator = ordered_type.and_then(valid_collection_comparator);
        if collection.comparator != expected_comparator {
            return Err(malformed_mir(format!(
                "collection#{} has an invalid or missing comparator identity",
                collection.id.0
            )));
        }
        if collection.kind == mir::CollectionKind::Bytes
            && (collection.key.is_some()
                || collection.value
                    != mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::UInt8)))
        {
            return Err(malformed_mir(format!(
                "Bytes collection#{} does not use packed uint8 elements",
                collection.id.0
            )));
        }
    }
    for (index, class) in program.classes.iter().enumerate() {
        validate_class(program, index, class)?;
    }
    for (index, property) in program.statics.iter().enumerate() {
        if property.id != mir::StaticId(index) {
            return Err(malformed_mir(format!(
                "static table slot {index} contains static{}",
                property.id.0
            )));
        }
        class_in(program, property.class)?;
        validate_type(program, property.ty)?;
        if matches!(
            property.ty,
            mir::Type::Class(_) | mir::Type::NullableClass(_)
        ) {
            return Err(malformed_mir(format!(
                "static{} uses an owned class type before owned static lifetime support",
                property.id.0
            )));
        }
        if !static_value_matches(program, &property.initializer, property.ty)? {
            return Err(malformed_mir(format!(
                "static{} initializer does not match {}",
                property.id.0, property.ty
            )));
        }
    }

    if let Some(entry_id) = program.selected_entry {
        if program.entry != entry_id {
            return Err(malformed_mir(
                "the compatibility entry does not match the selected graph entry",
            ));
        }
        let entry = program
            .functions
            .get(entry_id.0)
            .ok_or_else(|| malformed_mir("entry function does not exist"))?;
        if entry.id != entry_id {
            return Err(malformed_mir(
                "entry FunctionId does not match its table slot",
            ));
        }
        validate_entry(program, entry)?;
    }

    for (index, function) in program.functions.iter().enumerate() {
        if function.id != mir::FunctionId(index) {
            return Err(malformed_mir(format!(
                "function table slot {index} contains function{}",
                function.id.0
            )));
        }
        validate_method_identity(program, function)?;
        validate_function(program, function)?;
    }
    Ok(())
}

fn validate_entry(program: &mir::Program, entry: &mir::Function) -> Result<(), BackendError> {
    // Decision 0099: the entry takes either no parameters or exactly one
    // borrowed `List<string>` of program arguments, which the entry glue owns
    // for the duration of the call.
    match entry.params.as_slice() {
        [] => {}
        [parameter] => {
            let local = entry
                .locals
                .get(parameter.0)
                .ok_or_else(|| malformed_mir("entry parameter local does not exist"))?;
            let mir::Type::Collection(collection) = local.ty else {
                return Err(malformed_mir(
                    "entry parameter must be the `List<string>` argument list",
                ));
            };
            let definition = program
                .collection_types
                .get(collection.0)
                .ok_or_else(|| malformed_mir("entry argument collection does not exist"))?;
            if definition.kind != mir::CollectionKind::List || definition.value != mir::Type::String
            {
                return Err(malformed_mir(
                    "entry parameter must be the `List<string>` argument list",
                ));
            }
            if local.owned {
                return Err(malformed_mir(
                    "the entry argument list is borrowed from the entry glue, not owned by `main`",
                ));
            }
            if local.writable {
                return Err(malformed_mir(
                    "the entry argument list is a readonly borrow from the entry glue",
                ));
            }
        }
        _ => {
            return Err(malformed_mir(
                "entry function declares more than one parameter",
            ));
        }
    }
    if entry.method.is_some() || entry.receiver_mode.is_some() {
        return Err(malformed_mir("entry function cannot be a method"));
    }
    if !matches!(
        entry.return_type,
        mir::ReturnType::Void
            | mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Integer(
                IntegerType::Int64
            )))
    ) {
        return Err(malformed_mir(
            "entry function must return void or int/int64",
        ));
    }

    Ok(())
}

fn validate_graph_metadata(program: &mir::Program) -> Result<(), BackendError> {
    let mut source_ids = HashSet::new();
    let mut source_identities = HashSet::new();
    let mut packages = HashSet::new();
    for package in &program.packages {
        if !packages.insert(package.identity.clone()) {
            return Err(malformed_mir(format!(
                "package `{}` appears more than once",
                package.identity.display_name()
            )));
        }
    }
    for package in &program.packages {
        for dependency in package
            .normal_dependencies
            .iter()
            .chain(&package.development_dependencies)
        {
            if !packages.contains(dependency) {
                return Err(malformed_mir(format!(
                    "package `{}` references missing dependency `{}`",
                    package.identity.display_name(),
                    dependency.display_name()
                )));
            }
        }
    }
    for source in &program.sources {
        if !source_ids.insert(source.id) {
            return Err(malformed_mir(format!(
                "source{} appears more than once",
                source.id.0
            )));
        }
        if source.source.id != source.id {
            return Err(malformed_mir(format!(
                "source{} does not match its source-file identity",
                source.id.0
            )));
        }
        if !source_identities.insert(source.identity.clone()) {
            return Err(malformed_mir(format!(
                "source identity `{}` appears more than once",
                source.identity.0
            )));
        }
        if !packages.contains(&source.package) {
            return Err(malformed_mir(format!(
                "source `{}` belongs to missing package `{}`",
                source.identity.0,
                source.package.display_name()
            )));
        }
    }
    validate_global_graph_facts(program)?;
    if !packages.contains(&program.selected_target.package) {
        return Err(malformed_mir("selected target package does not exist"));
    }
    let selected_source = program
        .selected_target
        .entry_source
        .as_ref()
        .map(|identity| {
            program
                .sources
                .iter()
                .find(|source| &source.identity == identity)
                .ok_or_else(|| malformed_mir("selected entry source does not exist"))
        });
    match program.selected_target.kind {
        crate::build_plan::TargetKind::Binary => {
            let selected_source = selected_source
                .ok_or_else(|| malformed_mir("binary graph has no selected entry source"))??;
            if program.selected_entry.is_none() {
                return Err(malformed_mir("binary graph has no selected entry function"));
            }
            let entry = program
                .selected_entry
                .and_then(|entry| program.functions.get(entry.0))
                .ok_or_else(|| malformed_mir("selected entry function does not exist"))?;
            if entry.source_span.source != selected_source.id {
                return Err(malformed_mir(
                    "selected entry function belongs to the wrong source",
                ));
            }
        }
        crate::build_plan::TargetKind::Library => {
            if program.selected_target.entry_source.is_some() || program.selected_entry.is_some() {
                return Err(malformed_mir("library graph carries a process entry"));
            }
        }
    }
    let validate_source = |span: crate::source::Span, kind: &str| {
        if span.source == crate::compiler_known_io::SYNTHETIC_SOURCE_ID {
            return Ok(());
        }
        let source = program
            .sources
            .iter()
            .find(|source| source.id == span.source)
            .ok_or_else(|| malformed_mir(format!("{kind} references a missing source")))?;
        if !source.active {
            return Err(malformed_mir(format!(
                "{kind} references an inactive source"
            )));
        }
        Ok(())
    };
    for function in &program.functions {
        validate_source(function.source_span, "function")?;
    }
    for class in &program.classes {
        validate_source(class.source_span, "class")?;
    }
    for definition in &program.enums {
        validate_source(definition.source_span, "enum")?;
    }
    for property in &program.statics {
        validate_source(property.source_span, "static property")?;
    }
    for origin in &program.error_origins {
        validate_source(origin.span, "Error origin")?;
    }
    for closure in &program.closure_descriptors {
        validate_source(closure.source_span, "closure")?;
    }
    Ok(())
}

fn validate_global_graph_facts(program: &mir::Program) -> Result<(), BackendError> {
    use crate::ast::MemberAccess;
    use crate::names::{GlobalReferenceRole, GlobalSymbolOwner};

    let sources = program
        .sources
        .iter()
        .map(|source| (source.identity.clone(), source))
        .collect::<HashMap<_, _>>();
    let packages = program
        .packages
        .iter()
        .map(|package| (package.identity.clone(), package))
        .collect::<HashMap<_, _>>();
    let mut declarations = HashMap::new();
    let mut qualified_names = HashSet::new();
    for declaration in &program.global_symbols.declarations {
        let source = sources.get(&declaration.source_identity).ok_or_else(|| {
            malformed_mir(format!(
                "global declaration `{}` references a missing source",
                declaration.qualified_name
            ))
        })?;
        if declaration.name_span.source != source.id
            || declaration.declaration_span.source != source.id
        {
            return Err(malformed_mir(format!(
                "global declaration `{}` carries a span from the wrong source",
                declaration.qualified_name
            )));
        }
        let GlobalSymbolOwner::Package(owner) = &declaration.id.owner else {
            return Err(malformed_mir(format!(
                "user declaration `{}` has a compiler-known owner",
                declaration.qualified_name
            )));
        };
        if owner != &source.package {
            return Err(malformed_mir(format!(
                "global declaration `{}` belongs to the wrong package",
                declaration.qualified_name
            )));
        }
        if declaration.id.qualified_name != declaration.qualified_name {
            return Err(malformed_mir(format!(
                "global declaration `{}` does not match its symbol identity",
                declaration.qualified_name
            )));
        }
        if declarations
            .insert(declaration.id.clone(), declaration)
            .is_some()
            || !qualified_names.insert(declaration.qualified_name.as_str())
        {
            return Err(malformed_mir(format!(
                "global identity `{}` appears more than once",
                declaration.qualified_name
            )));
        }
    }

    let compiler_known = program
        .global_symbols
        .compiler_known
        .iter()
        .map(|fact| fact.id.clone())
        .collect::<HashSet<_>>();
    for reference in &program.global_symbols.references {
        let source = sources.get(&reference.source_identity).ok_or_else(|| {
            malformed_mir(format!(
                "global reference `{}` references a missing source",
                reference.source_spelling
            ))
        })?;
        if reference.source_span.source != source.id {
            return Err(malformed_mir(format!(
                "global reference `{}` carries a span from the wrong source",
                reference.source_spelling
            )));
        }
        let Some(declaration) = declarations.get(&reference.symbol_id) else {
            if compiler_known.contains(&reference.symbol_id) {
                continue;
            }
            return Err(malformed_mir(format!(
                "global reference `{}` has no declaration",
                reference.symbol_id.qualified_name
            )));
        };
        let GlobalSymbolOwner::Package(owner) = &declaration.id.owner else {
            continue;
        };
        if owner == &source.package {
            continue;
        }
        let source_package = packages
            .get(&source.package)
            .ok_or_else(|| malformed_mir("global reference belongs to a missing package"))?;
        let development = source.scope == crate::build_plan::SourceScope::Development
            || source.generated_for == Some(crate::build_plan::GeneratedFor::Development);
        let directly_visible = source_package.normal_dependencies.contains(owner)
            || (development && source_package.development_dependencies.contains(owner));
        if !directly_visible {
            return Err(malformed_mir(format!(
                "global reference `{}` crosses a non-direct package edge",
                reference.symbol_id.qualified_name
            )));
        }
        if declaration.access == MemberAccess::Internal {
            return Err(malformed_mir(format!(
                "global reference `{}` crosses an internal package boundary",
                reference.symbol_id.qualified_name
            )));
        }
    }

    if let Some(unresolved) = program.global_symbols.unresolved.first() {
        let subject = if unresolved.role == GlobalReferenceRole::Include {
            "include"
        } else {
            "global reference"
        };
        return Err(malformed_mir(format!(
            "unresolved {subject} `{}` remains in MIR",
            unresolved.source_spelling
        )));
    }
    Ok(())
}

fn validate_closure_metadata(program: &mir::Program) -> Result<(), BackendError> {
    for (index, function_type) in program.function_types.iter().enumerate() {
        if function_type.id != mir::FunctionTypeId(index) {
            return Err(malformed_mir(format!(
                "function type table slot {index} contains type#{}",
                function_type.id.0
            )));
        }
        for parameter in &function_type.parameters {
            validate_type_reference(program, parameter.ty)?;
        }
        if let mir::ReturnType::Value(ty) = function_type.return_type {
            validate_type_reference(program, ty)?;
        }
        validate_checked_effects(program, &function_type.checked_effects)?;
        if let Some(return_borrow) = function_type.return_borrow {
            match return_borrow.source {
                mir::BorrowSource::Receiver => {
                    return Err(malformed_mir(
                        "structural function type uses a receiver return-borrow source",
                    ));
                }
                mir::BorrowSource::Parameter(parameter)
                    if parameter >= function_type.parameters.len() =>
                {
                    return Err(malformed_mir(
                        "structural function type return-borrow parameter does not exist",
                    ));
                }
                mir::BorrowSource::Parameter(_) => {}
            }
        }
    }

    for (index, layout) in program.closure_environment_layouts.iter().enumerate() {
        if layout.id != mir::ClosureEnvironmentLayoutId(index) {
            return Err(malformed_mir(format!(
                "closure environment table slot {index} contains layout#{}",
                layout.id.0
            )));
        }
        let field_count = layout.fields.len();
        let mut logical = HashSet::new();
        let mut physical = HashSet::new();
        let mut environment_bindings = HashSet::new();
        for (field_index, field) in layout.fields.iter().enumerate() {
            if field.id != mir::ClosureEnvironmentFieldId(field_index)
                || field.logical_index >= field_count
                || field.physical_index >= field_count
                || !logical.insert(field.logical_index)
                || !physical.insert(field.physical_index)
                || !environment_bindings.insert(field.environment_binding)
            {
                return Err(malformed_mir(format!(
                    "closure environment layout#{} has invalid field identity or ordering",
                    layout.id.0
                )));
            }
            validate_type_reference(program, field.ty)?;
            if matches!(field.ty, mir::Type::ClosureEnvironment(_)) {
                return Err(malformed_mir(
                    "closure environment fields cannot contain a raw environment handle",
                ));
            }
        }
        let expected_release = (0..field_count).rev().collect::<Vec<_>>();
        if layout.logical_release_order != expected_release {
            return Err(malformed_mir(format!(
                "closure environment layout#{} does not use reverse logical release order",
                layout.id.0
            )));
        }
    }

    for (index, descriptor) in program.closure_descriptors.iter().enumerate() {
        if descriptor.id != mir::ClosureDescriptorId(index) {
            return Err(malformed_mir(format!(
                "closure descriptor table slot {index} contains descriptor#{}",
                descriptor.id.0
            )));
        }
        let function_type = function_type_in(program, descriptor.function_type)?;
        if descriptor.invocation_mode != function_type.invocation_mode {
            return Err(malformed_mir(format!(
                "closure descriptor#{} invocation mode disagrees with its function type",
                descriptor.id.0
            )));
        }
        if let Some(layout) = descriptor.environment_layout {
            closure_environment_layout_in(program, layout)?;
            if descriptor.environment_placement == mir::ClosureEnvironmentPlacement::None {
                return Err(malformed_mir(format!(
                    "closure descriptor#{} has an environment without native placement",
                    descriptor.id.0
                )));
            }
        } else if descriptor.environment_placement != mir::ClosureEnvironmentPlacement::None {
            return Err(malformed_mir(format!(
                "closure descriptor#{} has native environment placement without a layout",
                descriptor.id.0
            )));
        }
        let source = program
            .source_for_span(descriptor.source_span)
            .ok_or_else(|| malformed_mir("closure descriptor references a missing source"))?;
        if descriptor.source_span.start > descriptor.source_span.end
            || descriptor.source_span.end > source.text.len()
            || descriptor.debug_identity.is_empty()
        {
            return Err(malformed_mir(format!(
                "closure descriptor#{} has invalid source identity",
                descriptor.id.0
            )));
        }
    }
    Ok(())
}

fn validate_type_reference(program: &mir::Program, ty: mir::Type) -> Result<(), BackendError> {
    match ty {
        mir::Type::Function(function_type) | mir::Type::NullableFunction(function_type) => {
            function_type_in(program, function_type)?;
        }
        mir::Type::ClosureEnvironment(Some(layout)) => {
            closure_environment_layout_in(program, layout)?;
        }
        mir::Type::ClosureEnvironment(None) => {}
        _ => validate_type(program, ty)?,
    }
    Ok(())
}

fn validate_checked_effects(
    program: &mir::Program,
    effects: &[mir::CheckedEffect],
) -> Result<(), BackendError> {
    let mut seen = HashSet::new();
    for effect in effects {
        if !seen.insert(*effect) {
            return Err(malformed_mir("checked effect set contains a duplicate"));
        }
        if let mir::CheckedEffect::Concrete(descriptor) = effect {
            error_descriptor_in(program, *descriptor)?;
        }
    }
    Ok(())
}

fn validate_error_metadata(program: &mir::Program) -> Result<(), BackendError> {
    for (index, descriptor) in program.error_descriptors.iter().enumerate() {
        if descriptor.id != mir::ErrorDescriptorId(index) {
            return Err(malformed_mir(format!(
                "Error descriptor table slot {index} contains descriptor#{}",
                descriptor.id.0
            )));
        }
        let class = class_in(program, descriptor.class)?;
        if class.error_descriptor != Some(descriptor.id) {
            return Err(malformed_mir(format!(
                "Error descriptor#{} is not bound to class#{}",
                descriptor.id.0, descriptor.class.0
            )));
        }
        if class.error_origin_offset.is_none() {
            return Err(malformed_mir(format!(
                "Error class#{} has no hidden origin slot",
                descriptor.class.0
            )));
        }
        let message = property_in(program, descriptor.class, descriptor.message_property)?;
        if message.name != "message" || message.ty != mir::Type::String {
            return Err(malformed_mir(format!(
                "Error descriptor#{} has an invalid message projection",
                descriptor.id.0
            )));
        }
        if descriptor.type_name != class.name {
            return Err(malformed_mir(format!(
                "Error descriptor#{} type name does not match class#{}",
                descriptor.id.0, descriptor.class.0
            )));
        }
    }
    for class in &program.classes {
        match (class.error_descriptor, class.error_origin_offset) {
            (Some(descriptor), Some(_)) => {
                let found = program
                    .error_descriptors
                    .get(descriptor.0)
                    .filter(|entry| entry.id == descriptor && entry.class == class.id);
                if found.is_none() {
                    return Err(malformed_mir(format!(
                        "class#{} names an unknown Error descriptor",
                        class.id.0
                    )));
                }
            }
            (None, None) => {}
            _ => {
                return Err(malformed_mir(format!(
                    "class#{} has incomplete Error metadata",
                    class.id.0
                )));
            }
        }
    }
    for (index, origin) in program.error_origins.iter().enumerate() {
        if origin.id != mir::ErrorOriginId(index) {
            return Err(malformed_mir(format!(
                "Error origin table slot {index} contains origin#{}",
                origin.id.0
            )));
        }
        let source = program
            .source_for_span(origin.span)
            .ok_or_else(|| malformed_mir("Error origin references a missing source"))?;
        if origin.span.start > origin.span.end || origin.span.end > source.text.len() {
            return Err(malformed_mir(format!(
                "Error origin#{} is outside the source file",
                origin.id.0
            )));
        }
        if origin.callable.is_empty() {
            return Err(malformed_mir(format!(
                "Error origin#{} has no source callable identity",
                origin.id.0
            )));
        }
    }
    Ok(())
}

fn static_value_matches(
    program: &mir::Program,
    value: &mir::StaticValue,
    ty: mir::Type,
) -> Result<bool, BackendError> {
    match (value, ty) {
        (
            mir::StaticValue::Scalar(value),
            mir::Type::Scalar(expected) | mir::Type::NullableScalar(expected),
        ) => Ok(value.ty() == expected),
        (mir::StaticValue::String(_), mir::Type::String | mir::Type::NullableString) => Ok(true),
        (
            mir::StaticValue::Null,
            mir::Type::NullableScalar(_)
            | mir::Type::NullableString
            | mir::Type::NullablePayloadEnum(_),
        ) => Ok(true),
        (
            mir::StaticValue::PayloadEnum(value),
            mir::Type::PayloadEnum(expected) | mir::Type::NullablePayloadEnum(expected),
        ) if value.ty == expected && value.case.enum_id == expected.id => {
            let definition = enum_in(program, expected.id)?;
            let case = definition
                .cases
                .get(value.case.index)
                .filter(|case| case.id == value.case)
                .ok_or_else(|| malformed_mir("payload enum static names an unknown case"))?;
            if case.payload.len() != value.fields.len() {
                return Ok(false);
            }
            for (field, expected) in value.fields.iter().zip(&case.payload) {
                if !static_value_matches(program, field, expected.ty)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn valid_collection_comparator(ty: mir::Type) -> Option<mir::CollectionComparator> {
    match ty {
        mir::Type::Scalar(mir::ScalarType::Integer(integer)) if integer.is_signed() => Some(
            mir::CollectionComparator::SignedInteger(integer.bit_width() as u8),
        ),
        mir::Type::Scalar(mir::ScalarType::Integer(integer)) => Some(
            mir::CollectionComparator::UnsignedInteger(integer.bit_width() as u8),
        ),
        mir::Type::Scalar(mir::ScalarType::Bool) => Some(mir::CollectionComparator::Bool),
        mir::Type::String => Some(mir::CollectionComparator::StringBytes),
        _ => None,
    }
}

fn collection_type_is_copy(ty: mir::Type) -> bool {
    matches!(
        ty,
        mir::Type::Scalar(_)
            | mir::Type::String
            | mir::Type::NullableScalar(_)
            | mir::Type::NullableString
    ) || matches!(
        ty,
        mir::Type::PayloadEnum(payload) | mir::Type::NullablePayloadEnum(payload)
            if payload.capabilities.copy
    )
}

fn validate_method_identity(
    program: &mir::Program,
    function: &mir::Function,
) -> Result<(), BackendError> {
    match (&function.method, function.receiver_mode) {
        (None, None) => Ok(()),
        (None, Some(_)) => Err(malformed_mir(format!(
            "free function {} declares a receiver mode",
            function.name
        ))),
        (Some(method), None) => {
            class_in(program, method.class)?;
            Ok(())
        }
        (Some(method), Some(mir::ReceiverMode::UnsupportedConsuming)) => {
            Err(malformed_mir(format!(
                "method class#{}::{} uses the unsupported consuming receiver mode",
                method.class.0, method.name
            )))
        }
        (Some(method), Some(_)) => {
            class_in(program, method.class)?;
            let receiver = function.params.first().ok_or_else(|| {
                malformed_mir(format!(
                    "method class#{}::{} has no receiver parameter",
                    method.class.0, method.name
                ))
            })?;
            let receiver = local_in(function, *receiver)?;
            if receiver.ty != mir::Type::Class(method.class) || receiver.owned {
                return Err(malformed_mir(format!(
                    "method class#{}::{} has an invalid receiver parameter",
                    method.class.0, method.name
                )));
            }
            Ok(())
        }
    }
}

fn validate_class(
    program: &mir::Program,
    index: usize,
    class: &mir::Class,
) -> Result<(), BackendError> {
    let expected_id = ClassId(index);
    if class.id != expected_id {
        return Err(malformed_mir(format!(
            "class table slot {index} contains class#{}",
            class.id.0
        )));
    }

    for (property_index, property) in class.properties.iter().enumerate() {
        if property.id.class != class.id || property.id.index != property_index {
            return Err(malformed_mir(format!(
                "class#{} property slot {property_index} contains property#{}:{}",
                class.id.0, property.id.class.0, property.id.index
            )));
        }
        validate_type(program, property.ty)?;
    }

    let pointer_size = std::mem::size_of::<usize>() as u32;
    let mut expected_layout = compute_class_layout(
        class.id,
        class
            .properties
            .iter()
            .map(|property| (property.id, field_type(program, property.ty))),
        pointer_size,
    );
    let expected_origin_offset = class
        .error_descriptor
        .map(|_| append_hidden_pointer(&mut expected_layout, pointer_size));
    if class.error_origin_offset != expected_origin_offset {
        return Err(malformed_mir(format!(
            "class#{} has an invalid hidden Error-origin slot",
            class.id.0
        )));
    }
    if class.layout != expected_layout {
        return Err(malformed_mir(format!(
            "class#{} layout does not match its property table",
            class.id.0
        )));
    }

    if let Some(constructor) = class.constructor {
        validate_lifecycle(program, class.id, constructor, "constructor", false)?;
    }
    if let Some(destructor) = class.destructor {
        validate_lifecycle(program, class.id, destructor, "destructor", true)?;
    }
    Ok(())
}

fn validate_lifecycle(
    program: &mir::Program,
    class: ClassId,
    function: mir::FunctionId,
    kind: &str,
    receiver_only: bool,
) -> Result<(), BackendError> {
    let function = function_in(program, function)?;
    if function.return_type != mir::ReturnType::Void {
        return Err(malformed_mir(format!(
            "class#{} {kind} {} does not return void",
            class.0, function.name
        )));
    }
    let Some((receiver, parameters)) = function.params.split_first() else {
        return Err(malformed_mir(format!(
            "class#{} {kind} {} has no implicit receiver",
            class.0, function.name
        )));
    };
    let receiver_definition = local_in(function, *receiver)?;
    if receiver_definition.ty != mir::Type::Class(class) {
        return Err(malformed_mir(format!(
            "class#{} {kind} {} has an incompatible implicit receiver",
            class.0, function.name
        )));
    }
    if receiver_definition.owned {
        return Err(malformed_mir(format!(
            "class#{} {kind} {} marks its implicit receiver as owned",
            class.0, function.name
        )));
    }
    if receiver_only && !parameters.is_empty() {
        return Err(malformed_mir(format!(
            "class#{} destructor {} declares parameters",
            class.0, function.name
        )));
    }
    Ok(())
}

fn field_type(program: &mir::Program, ty: mir::Type) -> FieldType {
    match ty {
        mir::Type::Scalar(mir::ScalarType::Integer(integer)) => FieldType::Integer(integer),
        mir::Type::Scalar(mir::ScalarType::Float(float)) => FieldType::Float(float),
        mir::Type::Scalar(mir::ScalarType::Bool) => FieldType::Bool,
        mir::Type::Scalar(mir::ScalarType::Enum(_)) => FieldType::Integer(IntegerType::UInt32),
        mir::Type::String => FieldType::String,
        mir::Type::Mixed => FieldType::Mixed,
        mir::Type::NullableScalar(mir::ScalarType::Integer(integer)) => {
            FieldType::NullableInteger(integer)
        }
        mir::Type::NullableScalar(mir::ScalarType::Float(float)) => FieldType::NullableFloat(float),
        mir::Type::NullableScalar(mir::ScalarType::Bool) => FieldType::NullableBool,
        mir::Type::NullableScalar(mir::ScalarType::Enum(_)) => {
            FieldType::NullableInteger(IntegerType::UInt32)
        }
        mir::Type::NullableString => FieldType::NullableString,
        mir::Type::NullableMixed => FieldType::NullableMixed,
        mir::Type::Error => FieldType::Error,
        mir::Type::NullableError => FieldType::NullableError,
        mir::Type::Class(class) => FieldType::Class(class),
        mir::Type::NullableClass(class) => FieldType::NullableClass(class),
        mir::Type::SharedReference(class) => FieldType::SharedReference(class),
        mir::Type::WeakReference(class) => FieldType::WeakReference(class),
        mir::Type::NullableSharedReference(class) => FieldType::NullableSharedReference(class),
        mir::Type::NullableWeakReference(class) => FieldType::WeakReference(class),
        mir::Type::WritableSharedReference(_) => FieldType::WritableSharedReference,
        mir::Type::WritableWeakReference(_) => FieldType::WritableWeakReference,
        mir::Type::NullableWritableSharedReference(_) => FieldType::NullableWritableSharedReference,
        mir::Type::NullableWritableWeakReference(_) => FieldType::NullableWritableWeakReference,
        mir::Type::ReadonlySharedReferenceAccess(_)
        | mir::Type::WritableSharedReferenceAccess(_)
        | mir::Type::NullableReadonlySharedReferenceAccess(_)
        | mir::Type::NullableWritableSharedReferenceAccess(_) => FieldType::SharedReferenceAccess,
        mir::Type::Collection(_) | mir::Type::NullableCollection(_) => FieldType::Collection,
        mir::Type::Function(_) => FieldType::Function,
        mir::Type::NullableFunction(_) => FieldType::NullableFunction,
        mir::Type::ClosureEnvironment(_) => FieldType::SharedReferenceAccess,
        mir::Type::PayloadEnum(ty) => {
            let layout = &program.enums[ty.id.0].layout;
            FieldType::Aggregate {
                size: layout.size,
                align: layout.align,
            }
        }
        mir::Type::NullablePayloadEnum(ty) => {
            let layout = &program.enums[ty.id.0].layout;
            FieldType::Aggregate {
                size: layout.size + layout.align,
                align: layout.align,
            }
        }
    }
}

fn validate_function(program: &mir::Program, function: &mir::Function) -> Result<(), BackendError> {
    if let mir::ReturnType::Value(ty) = function.return_type {
        validate_type(program, ty)?;
    }
    if let Some(return_borrow) = function.return_borrow {
        let source_index = match return_borrow.source {
            mir::BorrowSource::Receiver => {
                if function.receiver_mode.is_none() {
                    return Err(malformed_mir(format!(
                        "function {} has a receiver return borrow without a receiver",
                        function.name
                    )));
                }
                0
            }
            mir::BorrowSource::Parameter(index) => {
                index
                    + usize::from(function.receiver_mode.is_some())
                    + usize::from(function.closure.is_some())
            }
        };
        let source = *function.params.get(source_index).ok_or_else(|| {
            malformed_mir(format!(
                "function {} return-borrow source parameter does not exist",
                function.name
            ))
        })?;
        let source = local_in(function, source)?;
        if source.owned || (return_borrow.writable && !source.writable) {
            return Err(malformed_mir(format!(
                "function {} has an invalid return-borrow source",
                function.name
            )));
        }
    }
    for (index, local) in function.locals.iter().enumerate() {
        if local.id != mir::LocalId(index) {
            return Err(malformed_mir(format!(
                "function {} local slot {index} contains local{}",
                function.name, local.id.0
            )));
        }
        validate_type(program, local.ty)?;
    }
    if function.parameter_modes.len() != function.params.len() {
        return Err(malformed_mir(format!(
            "function {} parameter modes do not match its parameters",
            function.name
        )));
    }
    for parameter in &function.params {
        let local = local_in(function, *parameter)?;
        let _ = local;
    }
    validate_checked_effects(program, &function.checked_effects)?;
    match &function.closure {
        Some(closure) => {
            let descriptor = closure_descriptor_in(program, closure.descriptor)?;
            let function_type = function_type_in(program, closure.function_type)?;
            if descriptor.entry_function != function.id
                || descriptor.function_type != closure.function_type
                || descriptor.environment_layout != closure.environment_layout
                || function.return_type != function_type.return_type
                || function.return_borrow != function_type.return_borrow
                || function.checked_effects != function_type.checked_effects
                || function.params.len() != function_type.parameters.len() + 1
                || function.params.first() != Some(&closure.hidden_environment)
            {
                return Err(malformed_mir(format!(
                    "synthetic closure function {} disagrees with its descriptor or structural type",
                    function.name
                )));
            }
            let hidden = local_in(function, closure.hidden_environment)?;
            if hidden.ty != mir::Type::ClosureEnvironment(closure.environment_layout)
                || hidden.writable
                    != matches!(
                        function_type.invocation_mode,
                        mir::FunctionInvocationMode::Writable
                    )
                || hidden.owned
                    != matches!(
                        function_type.invocation_mode,
                        mir::FunctionInvocationMode::Once
                    )
            {
                return Err(malformed_mir(
                    "synthetic closure function has an invalid hidden environment parameter",
                ));
            }
            for ((parameter, mode), expected) in function
                .params
                .iter()
                .skip(1)
                .zip(function.parameter_modes.iter().skip(1))
                .zip(&function_type.parameters)
            {
                if local_in(function, *parameter)?.ty != expected.ty || *mode != expected.mode {
                    return Err(malformed_mir(
                        "synthetic closure source parameter does not match its structural type",
                    ));
                }
            }
            let expected_hidden_mode = match function_type.invocation_mode {
                mir::FunctionInvocationMode::Readonly => mir::FunctionParameterMode::Readonly,
                mir::FunctionInvocationMode::Writable => mir::FunctionParameterMode::Writable,
                mir::FunctionInvocationMode::Once => mir::FunctionParameterMode::Take,
            };
            if function.parameter_modes.first() != Some(&expected_hidden_mode) {
                return Err(malformed_mir(
                    "synthetic closure hidden environment mode is invalid",
                ));
            }
            match closure.environment_layout {
                Some(layout) => {
                    let layout = closure_environment_layout_in(program, layout)?;
                    if closure.capture_locals.len() != layout.fields.len() {
                        return Err(malformed_mir(
                            "synthetic closure capture bindings do not match its environment",
                        ));
                    }
                    for ((field, local), expected) in
                        closure.capture_locals.iter().zip(&layout.fields)
                    {
                        if *field != expected.id || local_in(function, *local)?.ty != expected.ty {
                            return Err(malformed_mir(
                                "synthetic closure capture local has the wrong field or type",
                            ));
                        }
                    }
                }
                None if !closure.capture_locals.is_empty() => {
                    return Err(malformed_mir(
                        "no-capture closure declares environment capture locals",
                    ));
                }
                None => {}
            }
        }
        None => {
            if program
                .closure_descriptors
                .iter()
                .any(|descriptor| descriptor.entry_function == function.id)
            {
                return Err(malformed_mir(
                    "closure descriptor entry function lacks closure metadata",
                ));
            }
        }
    }
    block_in(function, function.entry_block)?;
    for (index, block) in function.blocks.iter().enumerate() {
        if block.id != mir::BlockId(index) {
            return Err(malformed_mir(format!(
                "function {} block slot {index} contains block{}",
                function.name, block.id.0
            )));
        }
    }
    let (reachable, _) = reachable_blocks_and_predecessors(function, true)?;
    for block in &function.blocks {
        for statement in &block.statements {
            validate_statement(program, function, statement)?;
        }
        validate_terminator(program, function, &block.terminator, reachable[block.id.0])?;
    }
    validate_borrowed_user_locals(function, &reachable)?;
    validate_nullable_presence(program, function)?;
    validate_mixed_tag_proofs(program, function)?;
    validate_payload_case_proofs(program, function)?;
    validate_match_result_plans(function)?;
    validate_match_binding_plans(function)?;
    validate_control_flow_plans(program, function)?;
    validate_class_local_lifetimes(function)
}

fn validate_borrowed_user_locals(
    function: &mir::Function,
    reachable: &[bool],
) -> Result<(), BackendError> {
    for local in function.locals.iter().filter(|local| {
        !local.owned
            && !local.synthetic
            && !local.writable
            && !function.params.contains(&local.id)
            && matches!(local.ty, mir::Type::Class(_) | mir::Type::NullableClass(_))
    }) {
        let assignments = function
            .blocks
            .iter()
            .filter(|block| reachable[block.id.0])
            .flat_map(|block| &block.statements)
            .filter(|statement| {
                matches!(
                    statement,
                    mir::Statement::AssignLocal { target, .. } if *target == local.id
                )
            })
            .count();
        if assignments != 1 {
            return Err(malformed_mir(format!(
                "borrowed user local local{} must have exactly one reachable initializer",
                local.id.0
            )));
        }
    }
    Ok(())
}

fn validate_type(program: &mir::Program, ty: mir::Type) -> Result<(), BackendError> {
    if let mir::Type::Scalar(mir::ScalarType::Enum(enum_id))
    | mir::Type::NullableScalar(mir::ScalarType::Enum(enum_id)) = ty
    {
        let definition = enum_in(program, enum_id)?;
        if definition.cases.iter().any(|case| !case.payload.is_empty()) {
            return Err(malformed_mir(format!(
                "payload enum#{} is represented as a scalar enum",
                enum_id.0
            )));
        }
        return Ok(());
    }
    if let mir::Type::PayloadEnum(payload) | mir::Type::NullablePayloadEnum(payload) = ty {
        validate_payload_enum_type(program, payload)?;
        return Ok(());
    }
    if let mir::Type::Class(class)
    | mir::Type::NullableClass(class)
    | mir::Type::SharedReference(class)
    | mir::Type::WeakReference(class)
    | mir::Type::NullableSharedReference(class)
    | mir::Type::NullableWeakReference(class) = ty
    {
        class_in(program, class)?;
    } else {
        match ty {
            mir::Type::Collection(collection) | mir::Type::NullableCollection(collection) => {
                collection_in(program, collection)?;
            }
            mir::Type::Function(function_type) | mir::Type::NullableFunction(function_type) => {
                function_type_in(program, function_type)?;
            }
            mir::Type::ClosureEnvironment(Some(layout)) => {
                closure_environment_layout_in(program, layout)?;
            }
            mir::Type::ClosureEnvironment(None) => {}
            mir::Type::WritableSharedReference(payload)
            | mir::Type::WritableWeakReference(payload)
            | mir::Type::NullableWritableSharedReference(payload)
            | mir::Type::NullableWritableWeakReference(payload)
            | mir::Type::ReadonlySharedReferenceAccess(payload)
            | mir::Type::WritableSharedReferenceAccess(payload)
            | mir::Type::NullableReadonlySharedReferenceAccess(payload)
            | mir::Type::NullableWritableSharedReferenceAccess(payload) => {
                validate_writable_shared_payload(program, payload)?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn enum_in(
    program: &mir::Program,
    id: crate::enums::EnumId,
) -> Result<&mir::EnumDefinition, BackendError> {
    program
        .enums
        .get(id.0)
        .filter(|definition| definition.id == id)
        .ok_or_else(|| malformed_mir(format!("enum#{} does not exist", id.0)))
}

fn validate_payload_enum_type(
    program: &mir::Program,
    ty: mir::PayloadEnumType,
) -> Result<&mir::EnumDefinition, BackendError> {
    let definition = enum_in(program, ty.id)?;
    if !definition.cases.iter().any(|case| !case.payload.is_empty()) {
        return Err(malformed_mir(format!(
            "scalar enum#{} is represented as a payload enum",
            ty.id.0
        )));
    }
    if definition.capabilities != ty.capabilities {
        return Err(malformed_mir(format!(
            "payload enum#{} capabilities disagree with its definition",
            ty.id.0
        )));
    }
    if definition.layout.enum_id != ty.id {
        return Err(malformed_mir(format!(
            "payload enum#{} layout has another enum identity",
            ty.id.0
        )));
    }
    let nullable = crate::enums::nullable_enum_layout(&definition.layout)
        .map_err(|_| malformed_mir("payload enum has an invalid nullable layout"))?;
    if ty.size != definition.layout.size
        || ty.align != definition.layout.align
        || ty.nullable_size != nullable.size
        || ty.nullable_payload_offset != nullable.payload_offset
    {
        return Err(malformed_mir(format!(
            "payload enum#{} type layout disagrees with its definition",
            ty.id.0
        )));
    }
    Ok(definition)
}

fn validate_writable_shared_payload(
    program: &mir::Program,
    payload: mir::WritableSharedPayload,
) -> Result<(), BackendError> {
    match payload {
        mir::WritableSharedPayload::Class(class) => {
            class_in(program, class)?;
        }
        mir::WritableSharedPayload::Collection(collection) => {
            collection_in(program, collection)?;
        }
    }
    Ok(())
}

fn is_writable_closure_parameter(function: &mir::Function, local: mir::LocalId) -> bool {
    function.closure.is_some()
        && function
            .params
            .iter()
            .position(|parameter| *parameter == local)
            .is_some_and(|index| {
                function.parameter_modes.get(index) == Some(&mir::FunctionParameterMode::Writable)
            })
}

fn validate_statement(
    program: &mir::Program,
    function: &mir::Function,
    statement: &mir::Statement,
) -> Result<(), BackendError> {
    match statement {
        mir::Statement::BindClosureEnvironment {
            environment,
            bindings,
        } => {
            let closure = function
                .closure
                .as_ref()
                .ok_or_else(|| malformed_mir("non-closure function binds a closure environment"))?;
            if *environment != closure.hidden_environment || bindings != &closure.capture_locals {
                return Err(malformed_mir(
                    "closure environment binding disagrees with synthetic function metadata",
                ));
            }
            let layout_id = closure
                .environment_layout
                .ok_or_else(|| malformed_mir("no-capture closure binds a closure environment"))?;
            let layout = closure_environment_layout_in(program, layout_id)?;
            if bindings.len() != layout.fields.len() {
                return Err(malformed_mir(
                    "closure environment binding count does not match its layout",
                ));
            }
            for ((field, target), expected) in bindings.iter().zip(&layout.fields) {
                let local = local_in(function, *target)?;
                let expected_owned = match expected.storage {
                    mir::ClosureEnvironmentStorage::Owned => expected.ty.has_move_ownership(),
                    mir::ClosureEnvironmentStorage::WritableBorrow => {
                        expected.ty.transfers_writable_capture_ownership()
                    }
                    mir::ClosureEnvironmentStorage::ReadonlyBorrow => false,
                };
                if *field != expected.id
                    || local.ty != expected.ty
                    || local.owned != expected_owned
                    || local.writable
                        != matches!(
                            expected.storage,
                            mir::ClosureEnvironmentStorage::WritableBorrow
                        )
                {
                    return Err(malformed_mir(
                        "closure environment field binding has incompatible type or access",
                    ));
                }
            }
            Ok(())
        }
        mir::Statement::BindPayloadEnumFields {
            source,
            ty,
            case,
            nullable,
            mode,
            targets,
        } => {
            let expected = if *nullable {
                mir::Type::NullablePayloadEnum(*ty)
            } else {
                mir::Type::PayloadEnum(*ty)
            };
            if local_in(function, *source)?.ty != expected {
                return Err(malformed_mir(
                    "payload binding source has an incompatible enum type",
                ));
            }
            let definition = validate_payload_enum_type(program, *ty)?;
            let case_definition = definition
                .cases
                .get(case.index)
                .filter(|definition| definition.id == *case)
                .ok_or_else(|| malformed_mir("payload binding case does not exist"))?;
            if targets.len() != case_definition.payload.len() {
                return Err(malformed_mir(
                    "payload binding arity does not match its case",
                ));
            }
            let mut unique_targets = HashSet::new();
            for (target, field) in targets.iter().zip(&case_definition.payload) {
                let target = local_in(function, *target)?;
                let copy_payload_owned = matches!(
                    field.ty,
                    mir::Type::PayloadEnum(payload)
                        | mir::Type::NullablePayloadEnum(payload)
                        if payload.capabilities.copy && payload.capabilities.needs_drop
                );
                let expected_owned = match mode {
                    mir::MatchBindingMode::GuardView => false,
                    mir::MatchBindingMode::BorrowedArm => copy_payload_owned,
                    mir::MatchBindingMode::ConsumedArm => {
                        field.ty.has_move_ownership() || copy_payload_owned
                    }
                };
                if target.ty != field.ty || target.writable || target.owned != expected_owned {
                    return Err(malformed_mir(
                        "payload binding target has incompatible readonly copy/borrow ownership",
                    ));
                }
                if target.id == *source || !unique_targets.insert(target.id) {
                    return Err(malformed_mir("payload binding targets overlap"));
                }
            }
            Ok(())
        }
        mir::Statement::MatchResultPlan {
            scrutinee,
            mode,
            result,
            arms,
            merge,
        } => {
            let scrutinee = local_in(function, *scrutinee)?;
            if !scrutinee.synthetic {
                return Err(malformed_mir(
                    "match result plan must use a synthetic scrutinee local",
                ));
            }
            if matches!(mode, mir::MatchOwnershipMode::Consumed)
                && (!scrutinee.owned || !scrutinee.ty.has_move_ownership())
            {
                return Err(malformed_mir(
                    "consuming match must own a Move scrutinee temporary",
                ));
            }
            let result = local_in(function, *result)?;
            if !result.synthetic || result.writable {
                return Err(malformed_mir(
                    "match result plan must target a readonly synthetic local",
                ));
            }
            block_in(function, *merge)?;
            if arms.is_empty() {
                return Err(malformed_mir("match result plan has no arm blocks"));
            }
            let mut unique = HashSet::new();
            for arm in arms {
                block_in(function, arm.binding)?;
                if arm.binding == *merge || !unique.insert(arm.binding) {
                    return Err(malformed_mir(
                        "match result plan has an invalid or repeated arm block",
                    ));
                }
                if let Some(guard) = arm.guard {
                    block_in(function, guard)?;
                    if guard == *merge || guard == arm.binding || !unique.insert(guard) {
                        return Err(malformed_mir(
                            "match result plan has an invalid or repeated guard block",
                        ));
                    }
                }
            }
            Ok(())
        }
        mir::Statement::AssignLocal { target, value } => {
            let declared_local = local_in(function, *target)?;
            // A writable closure parameter is a non-owning frame alias to an
            // owning caller place. Assignment replaces that caller-owned value
            // without making the final value a lexical cleanup obligation here.
            let replacement_owner =
                is_writable_closure_parameter(function, *target).then(|| mir::Local {
                    owned: true,
                    ..declared_local.clone()
                });
            let local = replacement_owner.as_ref().unwrap_or(declared_local);
            match (local.ty, value) {
                (mir::Type::PayloadEnum(expected), mir::Rvalue::PayloadEnum(expression))
                    if expression.ty() == expected =>
                {
                    validate_payload_enum_expression(program, function, expression)?;
                    validate_payload_enum_assignment_ownership(local, expression.use_mode())
                }
                (
                    mir::Type::NullablePayloadEnum(expected),
                    mir::Rvalue::NullablePayloadEnum(expression),
                ) if expression.ty() == expected => {
                    validate_nullable_payload_enum_expression(program, function, expression)?;
                    validate_payload_enum_assignment_ownership(
                        local,
                        nullable_payload_enum_use_mode(expression),
                    )
                }
                (mir::Type::PayloadEnum(_) | mir::Type::NullablePayloadEnum(_), _) => {
                    Err(malformed_mir(format!(
                        "payload enum local local{} receives a mismatched rvalue",
                        target.0
                    )))
                }
                (_, mir::Rvalue::PayloadEnum(_) | mir::Rvalue::NullablePayloadEnum(_)) => {
                    Err(malformed_mir(format!(
                        "non-payload-enum local local{} receives a payload enum rvalue",
                        target.0
                    )))
                }
                (mir::Type::String, mir::Rvalue::String(expression)) => {
                    validate_string_expression(program, function, expression)
                }
                (mir::Type::String, mir::Rvalue::NullableString(_)) => Err(malformed_mir(format!(
                    "string local local{} receives a nullable-string rvalue",
                    target.0
                ))),
                (mir::Type::String, mir::Rvalue::Value(value)) => Err(malformed_mir(format!(
                    "string local local{} receives a {} rvalue",
                    target.0,
                    value.ty()
                ))),
                (mir::Type::NullableString, mir::Rvalue::NullableString(expression)) => {
                    validate_nullable_string_expression(program, function, expression)
                }
                (mir::Type::NullableScalar(expected), mir::Rvalue::NullableScalar(expression))
                    if expression.ty() == expected =>
                {
                    validate_nullable_scalar_expression(program, function, expression)
                }
                (mir::Type::NullableClass(expected), mir::Rvalue::NullableClass(expression))
                    if expression.class() == expected =>
                {
                    validate_nullable_class_expression(program, function, expression)?;
                    if !local.owned {
                        if matches!(expression, mir::NullableClassExpression::Null(_)) {
                            return Ok(());
                        }
                        if nullable_class_expression_accesses_local(expression, *target) {
                            return Err(malformed_mir(format!(
                                "borrowed nullable class local{} reads its own uninitialized value",
                                target.0
                            )));
                        }
                        if !expression.borrows_class_value() {
                            return Err(malformed_mir(format!(
                                "borrowed nullable class local{} receives an owning value",
                                target.0
                            )));
                        }
                        return Ok(());
                    }
                    require_owned_nullable_class_expression(
                        expression,
                        &format!("nullable class assignment to local{}", target.0),
                    )
                }
                (mir::Type::NullableString, mir::Rvalue::Class(_)) => Err(malformed_mir(format!(
                    "nullable-string local local{} receives a class rvalue",
                    target.0
                ))),
                (mir::Type::NullableString, _) => Err(malformed_mir(format!(
                    "nullable-string local local{} receives another rvalue type",
                    target.0
                ))),
                (mir::Type::Error, mir::Rvalue::Error(expression)) => {
                    validate_error_expression(program, function, expression)?;
                    validate_error_assignment_ownership(
                        local,
                        error_expression_is_borrowed(expression),
                    )
                }
                (mir::Type::Error, _) => Err(malformed_mir(format!(
                    "Error local local{} receives a mismatched rvalue",
                    target.0
                ))),
                (mir::Type::NullableError, mir::Rvalue::NullableError(expression)) => {
                    validate_nullable_error_expression(program, function, expression)?;
                    let borrowed = nullable_error_expression_is_borrowed(expression);
                    if matches!(expression, mir::NullableErrorExpression::Null) {
                        Ok(())
                    } else {
                        validate_error_assignment_ownership(local, borrowed)
                    }
                }
                (mir::Type::NullableError, _) => Err(malformed_mir(format!(
                    "nullable Error local local{} receives a mismatched rvalue",
                    target.0
                ))),
                (_, mir::Rvalue::Error(_) | mir::Rvalue::NullableError(_)) => Err(malformed_mir(
                    format!("non-Error local local{} receives an Error rvalue", target.0),
                )),
                (mir::Type::Mixed, mir::Rvalue::Mixed(expression)) => {
                    validate_mixed_expression(program, function, expression)?;
                    let borrowed = is_borrowed_mixed_expression(expression);
                    if local.owned && borrowed {
                        return Err(malformed_mir(format!(
                            "owned mixed local local{} receives a borrowed value",
                            target.0
                        )));
                    }
                    if !local.owned && !borrowed {
                        return Err(malformed_mir(format!(
                            "borrowed mixed local local{} receives an owning value",
                            target.0
                        )));
                    }
                    Ok(())
                }
                (mir::Type::Mixed, _) => Err(malformed_mir(format!(
                    "mixed local local{} receives a mismatched rvalue",
                    target.0
                ))),
                (mir::Type::NullableMixed, mir::Rvalue::NullableMixed(expression)) => {
                    validate_nullable_mixed_expression(program, function, expression)?;
                    let borrowed = is_borrowed_nullable_mixed_expression(expression);
                    if local.owned
                        && borrowed
                        && !matches!(expression, mir::NullableMixedExpression::Null)
                    {
                        return Err(malformed_mir(format!(
                            "owned nullable mixed local local{} receives a borrowed value",
                            target.0
                        )));
                    }
                    if !local.owned && !borrowed {
                        return Err(malformed_mir(format!(
                            "borrowed nullable mixed local local{} receives an owning value",
                            target.0
                        )));
                    }
                    Ok(())
                }
                (mir::Type::NullableMixed, _) => Err(malformed_mir(format!(
                    "nullable mixed local local{} receives a mismatched rvalue",
                    target.0
                ))),
                (mir::Type::Scalar(_), mir::Rvalue::String(_) | mir::Rvalue::NullableString(_)) => {
                    Err(malformed_mir(format!(
                        "scalar local local{} receives a string rvalue",
                        target.0
                    )))
                }
                (mir::Type::Scalar(ty), mir::Rvalue::Value(expression)) => {
                    if expression.ty() != ty {
                        return Err(malformed_mir(format!(
                            "{} local local{} receives {} expression",
                            ty,
                            target.0,
                            expression.ty()
                        )));
                    }
                    validate_value_expression(program, function, expression)
                }
                (mir::Type::Class(expected), mir::Rvalue::Class(expression))
                    if expression.class() == expected =>
                {
                    if !local.owned {
                        if matches!(
                            expression,
                            mir::ClassExpression::CollectionIndex {
                                transfer: false,
                                ..
                            }
                        ) {
                            return validate_class_expression(program, function, expression);
                        }
                        if class_expression_accesses_local(expression, *target) {
                            return Err(malformed_mir(format!(
                                "borrowed class local{} reads its own uninitialized value",
                                target.0
                            )));
                        }
                        validate_class_expression(program, function, expression)?;
                        if !expression.borrows_class_value() {
                            return Err(malformed_mir(format!(
                                "borrowed class local{} receives an owning value",
                                target.0
                            )));
                        }
                        return Ok(());
                    }
                    validate_class_expression(program, function, expression)?;
                    require_owned_class_expression(
                        expression,
                        &format!("class assignment to local{}", target.0),
                    )
                }
                (mir::Type::Class(expected), _) => Err(malformed_mir(format!(
                    "class#{} local local{} receives a mismatched rvalue",
                    expected.0, target.0
                ))),
                (mir::Type::String | mir::Type::Scalar(_), mir::Rvalue::Class(_)) => {
                    Err(malformed_mir(format!(
                        "non-class local local{} receives a class rvalue",
                        target.0
                    )))
                }
                (
                    mir::Type::String | mir::Type::Scalar(_),
                    mir::Rvalue::Mixed(_) | mir::Rvalue::NullableMixed(_),
                ) => Err(malformed_mir(format!(
                    "non-mixed local local{} receives a mixed rvalue",
                    target.0
                ))),
                (mir::Type::NullableScalar(_) | mir::Type::NullableClass(_), _) => {
                    Err(malformed_mir(format!(
                        "nullable local local{} receives a mismatched rvalue",
                        target.0
                    )))
                }
                (
                    mir::Type::String | mir::Type::Scalar(_),
                    mir::Rvalue::NullableScalar(_) | mir::Rvalue::NullableClass(_),
                ) => Err(malformed_mir(format!(
                    "non-nullable local local{} receives a nullable rvalue",
                    target.0
                ))),
                (mir::Type::Collection(expected), mir::Rvalue::Collection(expression))
                    if expression.collection() == expected =>
                {
                    if !local.owned
                        && !matches!(
                            expression,
                            mir::CollectionExpression::Local {
                                transfer: false,
                                ..
                            } | mir::CollectionExpression::Index {
                                transfer: false,
                                ..
                            } | mir::CollectionExpression::Property { .. }
                                | mir::CollectionExpression::SharedAccessPayload { .. }
                        )
                    {
                        return Err(malformed_mir(format!(
                            "borrowed collection local local{} receives an owning value",
                            target.0
                        )));
                    }
                    if !local.owned {
                        validate_collection_borrow_writability(
                            program,
                            function,
                            expression,
                            local.writable,
                        )?;
                    }
                    validate_collection_expression(program, function, expression)
                }
                (mir::Type::Collection(_), _) => Err(malformed_mir(format!(
                    "collection local local{} receives a mismatched rvalue",
                    target.0
                ))),
                (_, mir::Rvalue::Collection(_)) => Err(malformed_mir(format!(
                    "non-collection local local{} receives a collection rvalue",
                    target.0
                ))),
                (
                    mir::Type::NullableCollection(expected),
                    mir::Rvalue::NullableCollection(expression),
                ) if expression.collection() == expected => {
                    validate_nullable_collection_expression(program, function, expression)
                }
                (mir::Type::NullableCollection(_), _) => Err(malformed_mir(format!(
                    "nullable collection local local{} receives a mismatched rvalue",
                    target.0
                ))),
                (_, mir::Rvalue::NullableCollection(_)) => Err(malformed_mir(format!(
                    "non-nullable-collection local local{} receives a nullable collection rvalue",
                    target.0
                ))),
                (
                    mir::Type::SharedReference(expected),
                    mir::Rvalue::SharedReference(expression),
                ) if expression.class() == expected => {
                    validate_shared_reference_expression(program, function, expression)
                }
                (mir::Type::WeakReference(expected), mir::Rvalue::WeakReference(expression))
                    if expression.class() == expected =>
                {
                    validate_weak_reference_expression(program, function, expression)
                }
                (
                    mir::Type::NullableSharedReference(expected),
                    mir::Rvalue::NullableSharedReference(expression),
                ) if expression.class() == expected => {
                    validate_nullable_shared_reference_expression(program, function, expression)
                }
                (
                    mir::Type::NullableWeakReference(expected),
                    mir::Rvalue::NullableWeakReference(expression),
                ) if expression.class() == expected => {
                    validate_nullable_weak_reference_expression(program, function, expression)
                }
                (
                    mir::Type::WritableSharedReference(expected),
                    mir::Rvalue::WritableSharedReference(expression),
                ) if expression.payload() == expected => {
                    validate_writable_shared_reference_expression(program, function, expression)
                }
                (
                    mir::Type::WritableWeakReference(expected),
                    mir::Rvalue::WritableWeakReference(expression),
                ) if expression.payload() == expected => {
                    validate_writable_weak_reference_expression(program, function, expression)
                }
                (
                    mir::Type::NullableWritableSharedReference(expected),
                    mir::Rvalue::NullableWritableSharedReference(expression),
                ) if expression.payload() == expected => {
                    validate_nullable_writable_shared_reference_expression(
                        program, function, expression,
                    )
                }
                (
                    mir::Type::NullableWritableWeakReference(expected),
                    mir::Rvalue::NullableWritableWeakReference(expression),
                ) if expression.payload() == expected => {
                    validate_nullable_writable_weak_reference_expression(
                        program, function, expression,
                    )
                }
                (
                    mir::Type::ReadonlySharedReferenceAccess(expected),
                    mir::Rvalue::SharedReferenceAccess(expression),
                ) if expression.payload() == expected && !expression.writable() => {
                    validate_shared_reference_access_expression(program, function, expression)
                }
                (
                    mir::Type::WritableSharedReferenceAccess(expected),
                    mir::Rvalue::SharedReferenceAccess(expression),
                ) if expression.payload() == expected && expression.writable() => {
                    validate_shared_reference_access_expression(program, function, expression)
                }
                (
                    mir::Type::NullableReadonlySharedReferenceAccess(expected),
                    mir::Rvalue::NullableSharedReferenceAccess(expression),
                ) if expression.payload() == expected && !expression.writable() => {
                    validate_nullable_shared_reference_access_expression(
                        program, function, expression,
                    )
                }
                (
                    mir::Type::NullableWritableSharedReferenceAccess(expected),
                    mir::Rvalue::NullableSharedReferenceAccess(expression),
                ) if expression.payload() == expected && expression.writable() => {
                    validate_nullable_shared_reference_access_expression(
                        program, function, expression,
                    )
                }
                (
                    mir::Type::SharedReference(_)
                    | mir::Type::WeakReference(_)
                    | mir::Type::NullableSharedReference(_)
                    | mir::Type::NullableWeakReference(_)
                    | mir::Type::WritableSharedReference(_)
                    | mir::Type::WritableWeakReference(_)
                    | mir::Type::NullableWritableSharedReference(_)
                    | mir::Type::NullableWritableWeakReference(_)
                    | mir::Type::ReadonlySharedReferenceAccess(_)
                    | mir::Type::WritableSharedReferenceAccess(_)
                    | mir::Type::NullableReadonlySharedReferenceAccess(_)
                    | mir::Type::NullableWritableSharedReferenceAccess(_),
                    _,
                )
                | (
                    _,
                    mir::Rvalue::SharedReference(_)
                    | mir::Rvalue::WeakReference(_)
                    | mir::Rvalue::NullableSharedReference(_)
                    | mir::Rvalue::NullableWeakReference(_)
                    | mir::Rvalue::WritableSharedReference(_)
                    | mir::Rvalue::WritableWeakReference(_)
                    | mir::Rvalue::NullableWritableSharedReference(_)
                    | mir::Rvalue::NullableWritableWeakReference(_)
                    | mir::Rvalue::SharedReferenceAccess(_)
                    | mir::Rvalue::NullableSharedReferenceAccess(_),
                ) => Err(malformed_mir(format!(
                    "local local{} receives a mismatched shared-handle rvalue",
                    target.0
                ))),
                (mir::Type::Function(expected), mir::Rvalue::Function(expression))
                    if expression.function_type() == expected =>
                {
                    validate_function_expression(program, function, expression)?;
                    validate_function_assignment_ownership(
                        local,
                        function_expression_is_borrowed(expression),
                    )
                }
                (
                    mir::Type::NullableFunction(expected),
                    mir::Rvalue::NullableFunction(expression),
                ) if expression.function_type() == expected => {
                    validate_nullable_function_expression(program, function, expression)?;
                    if matches!(expression, mir::NullableFunctionExpression::Null { .. }) {
                        Ok(())
                    } else {
                        validate_function_assignment_ownership(
                            local,
                            nullable_function_expression_is_borrowed(expression),
                        )
                    }
                }
                (mir::Type::Function(_) | mir::Type::NullableFunction(_), _)
                | (_, mir::Rvalue::Function(_) | mir::Rvalue::NullableFunction(_)) => {
                    Err(malformed_mir(format!(
                        "local local{} receives a mismatched function-value rvalue",
                        target.0
                    )))
                }
                (mir::Type::ClosureEnvironment(_), _) => Err(malformed_mir(
                    "closure environments may only enter through hidden parameters",
                )),
            }
        }
        mir::Statement::AssignLocalGroup { targets, value } => {
            if targets.len() < 2 {
                return Err(malformed_mir(
                    "grouped local assignment must contain at least two targets",
                ));
            }
            let mut unique = HashSet::with_capacity(targets.len());
            let first = local_in(function, targets[0])?;
            if first.synthetic {
                return Err(malformed_mir(
                    "grouped local assignment cannot target a synthetic local",
                ));
            }
            for (index, target) in targets.iter().enumerate() {
                let local = local_in(function, *target)?;
                if !unique.insert(*target) {
                    return Err(malformed_mir(format!(
                        "grouped local assignment repeats local{}",
                        target.0
                    )));
                }
                if local.synthetic {
                    return Err(malformed_mir(format!(
                        "grouped local assignment targets synthetic local{}",
                        target.0
                    )));
                }
                if local.ty != first.ty
                    || local.writable != first.writable
                    || local.owned != first.owned
                {
                    return Err(malformed_mir(
                        "grouped local assignment targets must share one type, mutability mode, and ownership mode",
                    ));
                }
                if index > 0 && targets[index - 1].0 >= target.0 {
                    return Err(malformed_mir(
                        "grouped local assignment targets must follow declaration order",
                    ));
                }
            }
            if first.ty.has_move_ownership() {
                if !first.owned || !grouped_move_rvalue_is_null(first.ty, value) {
                    return Err(malformed_mir(
                        "grouped move-type locals may only be initialized from a matching nullable null",
                    ));
                }
            } else if !matches!(
                first.ty,
                mir::Type::Scalar(_)
                    | mir::Type::String
                    | mir::Type::NullableScalar(_)
                    | mir::Type::NullableString
                    | mir::Type::PayloadEnum(mir::PayloadEnumType {
                        capabilities: crate::enums::EnumCapabilities { copy: true, .. },
                        ..
                    })
                    | mir::Type::NullablePayloadEnum(mir::PayloadEnumType {
                        capabilities: crate::enums::EnumCapabilities { copy: true, .. },
                        ..
                    })
            ) {
                return Err(malformed_mir(
                    "grouped local assignment requires a Copy type",
                ));
            }
            validate_statement(
                program,
                function,
                &mir::Statement::AssignLocal {
                    target: targets[0],
                    value: value.clone(),
                },
            )
        }
        mir::Statement::EchoStringLiteral(_) => Ok(()),
        mir::Statement::EchoString(expression) => {
            validate_string_expression(program, function, expression)
        }
        mir::Statement::CallVoid {
            function: callee,
            args,
            ..
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type != mir::ReturnType::Void {
                return Err(malformed_mir(format!(
                    "void call targets integer function {}",
                    callee.name
                )));
            }
            validate_call_args(program, function, callee, args)
        }
        mir::Statement::CallBorrowed {
            function: callee,
            args,
            ..
        } => {
            let callee = function_in(program, *callee)?;
            if !matches!(
                callee.return_type,
                mir::ReturnType::Value(
                    mir::Type::Class(_)
                        | mir::Type::NullableClass(_)
                        | mir::Type::Collection(_)
                        | mir::Type::Mixed
                        | mir::Type::NullableMixed
                )
            ) || infer_function_return_borrow(program, callee)?.is_none()
            {
                return Err(malformed_mir(format!(
                    "borrowed call targets function {} without a borrowed move-value return",
                    callee.name
                )));
            }
            validate_call_args(program, function, callee, args)
        }
        mir::Statement::CallNullSafe {
            object,
            function: callee,
            args,
            ..
        } => validate_null_safe_statement_call(program, function, object, *callee, args),
        mir::Statement::Printf(format) => validate_format_expression(program, function, format),
        mir::Statement::WriteFile { path, contents }
        | mir::Statement::AppendFile { path, contents } => {
            validate_string_expression(program, function, path)?;
            validate_string_expression(program, function, contents)
        }
        mir::Statement::WriteFileBytes { path, contents, .. } => {
            validate_string_expression(program, function, path)?;
            validate_bytes_local(program, function, *contents)
        }
        mir::Statement::WriteStreamBytes { contents, .. } => {
            validate_bytes_local(program, function, *contents)
        }
        mir::Statement::WriteStderr(value) => validate_string_expression(program, function, value),
        mir::Statement::AssignProperty {
            object,
            property,
            value,
            kind,
            ..
        } => {
            let object = local_in(function, *object)?;
            let mir::Type::Class(class) = object.ty else {
                return Err(malformed_mir(
                    "property assignment targets a non-class local",
                ));
            };
            let property_definition = property_in(program, class, *property)?;
            if value.ty() != property_definition.ty {
                return Err(malformed_mir(format!(
                    "property{} receives {} but has type {}",
                    property.index,
                    value.ty(),
                    property_definition.ty
                )));
            }
            if rvalue_transfers_class_local(value, object.id) {
                return Err(malformed_mir(format!(
                    "assignment to property{} consumes its receiver local{}",
                    property.index, object.id.0
                )));
            }
            if rvalue_borrows_class_local_outside_property(value, object.id, *property) {
                return Err(malformed_mir(format!(
                    "assignment to property{} borrows its receiver local{} through another access",
                    property.index, object.id.0
                )));
            }
            let constructor_receiver = class_in(program, class)?.constructor == Some(function.id)
                && function.params.first() == Some(&object.id);
            if matches!(kind, mir::PropertyWriteKind::Initialize) && !constructor_receiver {
                return Err(malformed_mir(format!(
                    "property{} initialization does not target the direct constructor receiver",
                    property.index
                )));
            }
            if matches!(kind, mir::PropertyWriteKind::InitializeOrReplace)
                && (!constructor_receiver || !property_definition.writable)
            {
                return Err(malformed_mir(format!(
                    "property{} conditional initialization must target a writable property on the direct constructor receiver",
                    property.index
                )));
            }
            if !matches!(kind, mir::PropertyWriteKind::Initialize)
                && !property_definition.writable
                && !constructor_receiver
            {
                return Err(malformed_mir(format!(
                    "assignment mutates readonly property{} outside its constructor initializer",
                    property.index
                )));
            }
            if !constructor_receiver && !object.writable {
                return Err(malformed_mir(format!(
                    "assignment to property{} uses readonly receiver local{}",
                    property.index, object.id.0
                )));
            }
            validate_rvalue(program, function, value)?;
            if let (mir::Type::Class(_), mir::Rvalue::Class(expression)) =
                (property_definition.ty, value)
            {
                require_owned_class_expression(
                    expression,
                    &format!("assignment to property{}", property.index),
                )?;
            } else if let (mir::Type::NullableClass(_), mir::Rvalue::NullableClass(expression)) =
                (property_definition.ty, value)
            {
                require_owned_nullable_class_expression(
                    expression,
                    &format!("assignment to property{}", property.index),
                )?;
            } else if property_definition.ty.has_move_ownership() && value.borrows_move_value() {
                return Err(malformed_mir(format!(
                    "assignment to property{} stores a borrowed move value",
                    property.index
                )));
            }
            Ok(())
        }
        mir::Statement::AssignStatic { target, value } => {
            let property = static_in(program, *target)?;
            if !property.writable {
                return Err(malformed_mir(format!(
                    "assignment targets readonly static{}",
                    target.0
                )));
            }
            if value.ty() != property.ty {
                return Err(malformed_mir(format!(
                    "static{} receives {} but has type {}",
                    target.0,
                    value.ty(),
                    property.ty
                )));
            }
            if matches!(property.ty, mir::Type::Mixed | mir::Type::NullableMixed)
                && value.mixed_ownership() == mir::MixedOwnership::None
                && !matches!(
                    value,
                    mir::Rvalue::NullableMixed(mir::NullableMixedExpression::Null)
                )
            {
                return Err(malformed_mir(format!(
                    "assignment to static{} stores a borrowed mixed value",
                    target.0
                )));
            }
            validate_rvalue(program, function, value)
        }
        mir::Statement::DropClass { local, class } => {
            let definition = local_in(function, *local)?;
            if !matches!(
                definition.ty,
                mir::Type::Class(found) | mir::Type::NullableClass(found) if found == *class
            ) {
                return Err(malformed_mir(format!(
                    "drop class#{} references local{} with type {}",
                    class.0, local.0, definition.ty
                )));
            }
            if !definition.owned {
                return Err(malformed_mir(format!(
                    "drop class#{} references borrowed local{}",
                    class.0, local.0
                )));
            }
            class_in(program, *class).map(|_| ())
        }
        mir::Statement::DropSharedReference { local, class } => {
            let definition = local_in(function, *local)?;
            if !matches!(
                definition.ty,
                mir::Type::SharedReference(found)
                    | mir::Type::NullableSharedReference(found) if found == *class
            ) || !definition.owned
            {
                return Err(malformed_mir(format!(
                    "shared-reference drop references local{} with type {}",
                    local.0, definition.ty
                )));
            }
            class_in(program, *class).map(|_| ())
        }
        mir::Statement::DropWeakReference { local, class } => {
            let definition = local_in(function, *local)?;
            if !matches!(
                definition.ty,
                mir::Type::WeakReference(found)
                    | mir::Type::NullableWeakReference(found) if found == *class
            ) || !definition.owned
            {
                return Err(malformed_mir(format!(
                    "weak-reference drop references local{} with type {}",
                    local.0, definition.ty
                )));
            }
            class_in(program, *class).map(|_| ())
        }
        mir::Statement::DropWritableSharedReference { local, payload } => {
            let definition = local_in(function, *local)?;
            if !matches!(
                definition.ty,
                mir::Type::WritableSharedReference(found)
                    | mir::Type::NullableWritableSharedReference(found) if found == *payload
            ) || !definition.owned
            {
                return Err(malformed_mir(format!(
                    "writable shared drop references local{} with type {}",
                    local.0, definition.ty
                )));
            }
            validate_writable_shared_payload(program, *payload)
        }
        mir::Statement::DropWritableWeakReference { local, payload } => {
            let definition = local_in(function, *local)?;
            if !matches!(
                definition.ty,
                mir::Type::WritableWeakReference(found)
                    | mir::Type::NullableWritableWeakReference(found) if found == *payload
            ) || !definition.owned
            {
                return Err(malformed_mir(format!(
                    "writable weak drop references local{} with type {}",
                    local.0, definition.ty
                )));
            }
            validate_writable_shared_payload(program, *payload)
        }
        mir::Statement::DropSharedReferenceAccess {
            local,
            payload,
            writable,
        } => {
            let definition = local_in(function, *local)?;
            let expected = if *writable {
                mir::Type::WritableSharedReferenceAccess(*payload)
            } else {
                mir::Type::ReadonlySharedReferenceAccess(*payload)
            };
            let nullable_expected = if *writable {
                mir::Type::NullableWritableSharedReferenceAccess(*payload)
            } else {
                mir::Type::NullableReadonlySharedReferenceAccess(*payload)
            };
            if (definition.ty != expected && definition.ty != nullable_expected)
                || !definition.owned
            {
                return Err(malformed_mir(format!(
                    "shared access drop references local{} with type {}",
                    local.0, definition.ty
                )));
            }
            validate_writable_shared_payload(program, *payload)
        }
        mir::Statement::DropString { local } => {
            let local = local_in(function, *local)?;
            if !matches!(local.ty, mir::Type::String | mir::Type::NullableString)
                || !local.synthetic
            {
                return Err(malformed_mir(
                    "string drop must reference a synthetic string local",
                ));
            }
            Ok(())
        }
        mir::Statement::DropMixed { local } => {
            let local = local_in(function, *local)?;
            if !matches!(local.ty, mir::Type::Mixed | mir::Type::NullableMixed) || !local.owned {
                return Err(malformed_mir(
                    "mixed drop must reference an owned mixed local",
                ));
            }
            Ok(())
        }
        mir::Statement::EnsureErrorOrigin { error, origin } => {
            let error = local_in(function, *error)?;
            if error.ty != mir::Type::Error || !error.owned {
                return Err(malformed_mir(
                    "Error origin assignment requires an owned Error carrier",
                ));
            }
            program
                .error_origins
                .get(origin.0)
                .filter(|entry| entry.id == *origin)
                .ok_or_else(|| {
                    malformed_mir(format!("Error origin#{} does not exist", origin.0))
                })?;
            Ok(())
        }
        mir::Statement::ExtractErrorObject {
            target,
            error,
            descriptor,
        } => {
            let target = local_in(function, *target)?;
            let error = local_in(function, *error)?;
            let descriptor = error_descriptor_in(program, *descriptor)?;
            if target.ty != mir::Type::Class(descriptor.class) || !target.owned {
                return Err(malformed_mir(
                    "exact catch target does not own the descriptor's concrete class",
                ));
            }
            if error.ty != mir::Type::Error || !error.owned {
                return Err(malformed_mir(
                    "exact catch extraction requires an owned Error carrier",
                ));
            }
            Ok(())
        }
        mir::Statement::DropError { local } => {
            let local = local_in(function, *local)?;
            if !matches!(local.ty, mir::Type::Error | mir::Type::NullableError) || !local.owned {
                return Err(malformed_mir(
                    "Error drop must reference an owned Error carrier",
                ));
            }
            Ok(())
        }
        mir::Statement::CollectionAdd {
            collection,
            value,
            index,
            op,
        } => {
            let local = local_in(function, *collection)?;
            let mir::Type::Collection(collection_type) = local.ty else {
                return Err(malformed_mir(
                    "collection add targets a non-collection local",
                ));
            };
            let definition = collection_in(program, collection_type)?;
            if !local.writable {
                return Err(malformed_mir(format!(
                    "collection add uses readonly local{}",
                    local.id.0
                )));
            }
            if value.ty() != definition.value {
                return Err(malformed_mir("collection add value type mismatch"));
            }
            match (op, definition.kind, index) {
                (
                    mir::CollectionMutationOp::Add,
                    mir::CollectionKind::List
                    | mir::CollectionKind::Set
                    | mir::CollectionKind::SortedSet,
                    None,
                )
                | (
                    mir::CollectionMutationOp::Remove,
                    mir::CollectionKind::Set | mir::CollectionKind::SortedSet,
                    None,
                )
                | (mir::CollectionMutationOp::Push, mir::CollectionKind::PriorityQueue, None)
                | (
                    mir::CollectionMutationOp::PushFront | mir::CollectionMutationOp::PushBack,
                    mir::CollectionKind::Deque,
                    None,
                ) => {}
                (mir::CollectionMutationOp::InsertAt, mir::CollectionKind::List, Some(index)) => {
                    if index.ty() != mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64))
                    {
                        return Err(malformed_mir("List::insertAt index is not int"));
                    }
                    validate_rvalue(program, function, index)?;
                }
                _ => {
                    return Err(malformed_mir(
                        "collection mutation does not match its collection kind",
                    ));
                }
            }
            validate_rvalue(program, function, value)
        }
        mir::Statement::CollectionSet {
            collection,
            key,
            value,
        } => {
            let local = local_in(function, *collection)?;
            let mir::Type::Collection(collection_type) = local.ty else {
                return Err(malformed_mir(
                    "collection set targets a non-collection local",
                ));
            };
            let definition = collection_in(program, collection_type)?;
            let Some(key_type) = definition.key else {
                return Err(malformed_mir(
                    "collection set targets a non-keyed collection",
                ));
            };
            if !local.writable {
                return Err(malformed_mir(format!(
                    "collection set uses readonly local{}",
                    local.id.0
                )));
            }
            if key.ty() != key_type || value.ty() != definition.value {
                return Err(malformed_mir("collection set key/value type mismatch"));
            }
            validate_rvalue(program, function, key)?;
            validate_rvalue(program, function, value)
        }
        mir::Statement::AssignCollectionIndex {
            positional,
            collection,
            index,
            value,
        } => {
            let local = local_in(function, *collection)?;
            let mir::Type::Collection(collection_type) = local.ty else {
                return Err(malformed_mir(
                    "indexed assignment targets a non-collection local",
                ));
            };
            let definition = collection_in(program, collection_type)?;
            if !local.writable {
                return Err(malformed_mir(format!(
                    "indexed assignment uses readonly local{}",
                    local.id.0
                )));
            }
            // A positional assignment addresses a slot, so its index is an
            // offset even when the collection is keyed.
            let index_type = if *positional {
                mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64))
            } else {
                definition
                    .key
                    .unwrap_or(mir::Type::Scalar(mir::ScalarType::Integer(
                        IntegerType::Int64,
                    )))
            };
            if index.ty() != index_type || value.ty() != definition.value {
                return Err(malformed_mir("indexed assignment type mismatch"));
            }
            validate_rvalue(program, function, index)?;
            validate_rvalue(program, function, value)
        }
        mir::Statement::CollectionClear {
            collection,
            collection_type,
        } => {
            let local = local_in(function, *collection)?;
            let mir::Type::Collection(found) = local.ty else {
                return Err(malformed_mir(
                    "collection clear targets a non-collection or nullable local",
                ));
            };
            if found != *collection_type {
                return Err(malformed_mir("collection clear type mismatch"));
            }
            if !local.writable {
                return Err(malformed_mir(format!(
                    "collection clear uses readonly local{}",
                    local.id.0
                )));
            }
            let definition = collection_in(program, *collection_type)?;
            if matches!(
                definition.kind,
                mir::CollectionKind::Bytes | mir::CollectionKind::TypedArray
            ) {
                return Err(malformed_mir(
                    "collection clear requires a named growable collection",
                ));
            }
            Ok(())
        }
        mir::Statement::DropCollection { local, collection } => {
            let definition = local_in(function, *local)?;
            if !matches!(
                definition.ty,
                mir::Type::Collection(found) | mir::Type::NullableCollection(found)
                    if found == *collection
            ) || !definition.owned
            {
                return Err(malformed_mir(format!(
                    "drop collection#{} references incompatible local{}",
                    collection.0, local.0
                )));
            }
            collection_in(program, *collection).map(|_| ())
        }
        mir::Statement::DropPayloadEnum {
            local,
            ty,
            nullable,
        } => {
            let definition = local_in(function, *local)?;
            let expected = if *nullable {
                mir::Type::NullablePayloadEnum(*ty)
            } else {
                mir::Type::PayloadEnum(*ty)
            };
            if definition.ty != expected || !definition.owned || !ty.capabilities.needs_drop {
                return Err(malformed_mir(format!(
                    "payload enum drop references incompatible local{}",
                    local.0
                )));
            }
            validate_payload_enum_type(program, *ty).map(|_| ())
        }
        mir::Statement::DropFunction {
            local,
            function_type,
            nullable,
        } => {
            let definition = local_in(function, *local)?;
            let expected = if *nullable {
                mir::Type::NullableFunction(*function_type)
            } else {
                mir::Type::Function(*function_type)
            };
            if definition.ty != expected || !definition.owned {
                return Err(malformed_mir(format!(
                    "function-value drop references incompatible local{}",
                    local.0
                )));
            }
            function_type_in(program, *function_type).map(|_| ())
        }
        mir::Statement::ControlFlowPlan(plan) => match plan {
            mir::ControlFlowPlan::Given(plan) => {
                block_in(function, plan.setup_entry)?;
                block_in(function, plan.setup_exit)?;
                block_in(function, plan.condition)?;
                if let Some(gate_failed) = plan.gate_failed {
                    block_in(function, gate_failed)?;
                }
                for predicate in &plan.predicates {
                    block_in(function, predicate.block)?;
                }
                for block in &plan.continue_sources {
                    block_in(function, *block)?;
                }
                Ok(())
            }
            mir::ControlFlowPlan::When(plan) => {
                let result = local_in(function, plan.result)?;
                if !result.synthetic || result.writable {
                    return Err(malformed_mir(
                        "when result plan must target a readonly synthetic local",
                    ));
                }
                let expected_owned = matches!(plan.ownership, mir::WhenResultOwnership::Owned);
                if result.owned != expected_owned
                    || (expected_owned && !result.ty.has_move_ownership())
                {
                    return Err(malformed_mir(
                        "when result local has incompatible ownership",
                    ));
                }
                block_in(function, plan.merge)?;
                if plan.branches.len() < 2 {
                    return Err(malformed_mir(
                        "when result plan must include a head branch and mandatory else",
                    ));
                }
                let mut unique = HashSet::new();
                for branch in &plan.branches {
                    block_in(function, *branch)?;
                    if *branch == plan.merge || !unique.insert(*branch) {
                        return Err(malformed_mir(
                            "when result plan has an invalid or repeated branch block",
                        ));
                    }
                }
                Ok(())
            }
            mir::ControlFlowPlan::DoWhile(plan) => {
                block_in(function, plan.entry)?;
                block_in(function, plan.body)?;
                block_in(function, plan.condition)?;
                block_in(function, plan.exit)?;
                for source in &plan.continue_sources {
                    block_in(function, *source)?;
                }
                Ok(())
            }
            mir::ControlFlowPlan::Finalizer(plan) => {
                block_in(function, plan.activation)?;
                block_in(function, plan.entry)?;
                block_in(function, plan.completion)?;
                let discriminator = local_in(function, plan.discriminator)?;
                if !discriminator.synthetic
                    || !discriminator.writable
                    || discriminator.ty
                        != mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64))
                {
                    return Err(malformed_mir(
                        "finalizer discriminator must be a writable synthetic int local",
                    ));
                }
                if plan.body_blocks.is_empty() || !plan.body_blocks.contains(&plan.entry) {
                    return Err(malformed_mir(
                        "finalizer body does not identify its entry block",
                    ));
                }
                for body in &plan.body_blocks {
                    block_in(function, *body)?;
                }
                for exit in &plan.exits {
                    block_in(function, exit.source)?;
                    block_in(function, exit.continuation)?;
                    match exit.kind {
                        mir::StructuredExitKind::WhenYield { result } => {
                            local_in(function, result)?;
                        }
                        mir::StructuredExitKind::FunctionReturn { value: Some(value) } => {
                            local_in(function, value)?;
                        }
                        mir::StructuredExitKind::CheckedError { error } => {
                            let error = local_in(function, error)?;
                            if error.ty != mir::Type::Error || !error.owned {
                                return Err(malformed_mir(
                                    "checked-error finalizer exit does not own an Error carrier",
                                ));
                            }
                        }
                        mir::StructuredExitKind::Normal
                        | mir::StructuredExitKind::FunctionReturn { value: None }
                        | mir::StructuredExitKind::Break
                        | mir::StructuredExitKind::Continue => {}
                    }
                }
                Ok(())
            }
            mir::ControlFlowPlan::ListAlgorithm(plan) => {
                validate_list_algorithm_types(program, function, plan)
            }
        },
    }
}

fn grouped_move_rvalue_is_null(ty: mir::Type, value: &mir::Rvalue) -> bool {
    match (ty, value) {
        (
            mir::Type::NullableError,
            mir::Rvalue::NullableError(mir::NullableErrorExpression::Null),
        ) => true,
        (mir::Type::NullableMixed, mir::Rvalue::NullableMixed(value)) => {
            matches!(value, mir::NullableMixedExpression::Null)
        }
        (
            mir::Type::NullablePayloadEnum(expected),
            mir::Rvalue::NullablePayloadEnum(mir::NullablePayloadEnumExpression::Null(actual)),
        ) => *actual == expected,
        (mir::Type::NullableClass(expected), mir::Rvalue::NullableClass(value)) => {
            matches!(value, mir::NullableClassExpression::Null(actual) if *actual == expected)
        }
        (
            mir::Type::NullableSharedReference(expected),
            mir::Rvalue::NullableSharedReference(value),
        ) => {
            matches!(value, mir::NullableSharedReferenceExpression::Null(actual) if *actual == expected)
        }
        (mir::Type::NullableWeakReference(expected), mir::Rvalue::NullableWeakReference(value)) => {
            matches!(value, mir::NullableWeakReferenceExpression::Null(actual) if *actual == expected)
        }
        (
            mir::Type::NullableWritableSharedReference(expected),
            mir::Rvalue::NullableWritableSharedReference(value),
        ) => {
            matches!(value, mir::NullableWritableSharedReferenceExpression::Null(actual) if *actual == expected)
        }
        (
            mir::Type::NullableWritableWeakReference(expected),
            mir::Rvalue::NullableWritableWeakReference(value),
        ) => {
            matches!(value, mir::NullableWritableWeakReferenceExpression::Null(actual) if *actual == expected)
        }
        (
            mir::Type::NullableReadonlySharedReferenceAccess(expected),
            mir::Rvalue::NullableSharedReferenceAccess(value),
        ) => {
            matches!(value, mir::NullableSharedReferenceAccessExpression::Null { payload, writable: false } if *payload == expected)
        }
        (
            mir::Type::NullableWritableSharedReferenceAccess(expected),
            mir::Rvalue::NullableSharedReferenceAccess(value),
        ) => {
            matches!(value, mir::NullableSharedReferenceAccessExpression::Null { payload, writable: true } if *payload == expected)
        }
        (mir::Type::NullableCollection(expected), mir::Rvalue::NullableCollection(value)) => {
            matches!(value, mir::NullableCollectionExpression::Null(actual) if *actual == expected)
        }
        (mir::Type::NullableFunction(expected), mir::Rvalue::NullableFunction(value)) => {
            matches!(value, mir::NullableFunctionExpression::Null { function_type } if *function_type == expected)
        }
        _ => false,
    }
}

fn validate_terminator(
    program: &mir::Program,
    function: &mir::Function,
    terminator: &mir::Terminator,
    validate_return_ownership: bool,
) -> Result<(), BackendError> {
    match terminator {
        mir::Terminator::Return(expression) => {
            let mir::ReturnType::Value(return_type) = function.return_type else {
                return Err(malformed_mir(format!(
                    "void function {} has an integer return",
                    function.name
                )));
            };
            if expression.ty() != return_type {
                return Err(malformed_mir(format!(
                    "function {} returns {} expression from {} function",
                    function.name,
                    expression.ty(),
                    return_type
                )));
            }
            validate_rvalue(program, function, expression)?;
            if validate_return_ownership {
                if let (mir::Type::Class(_), mir::Rvalue::Class(class)) = (return_type, expression)
                {
                    let expected = infer_function_return_borrow(program, function)?;
                    let actual = infer_expression_return_borrow(program, function, class)?;
                    if !return_borrow_is_compatible(actual, expected) {
                        return Err(malformed_mir(format!(
                            "return from {} has inconsistent class ownership",
                            function.name
                        )));
                    }
                    if expected.is_none() {
                        require_owned_class_expression(
                            class,
                            &format!("return from {}", function.name),
                        )?;
                    }
                } else if let (mir::Type::NullableClass(_), mir::Rvalue::NullableClass(class)) =
                    (return_type, expression)
                {
                    let expected = infer_function_return_borrow(program, function)?;
                    let actual = infer_nullable_expression_return_borrow(program, function, class)?;
                    if !return_borrow_is_compatible(actual, expected) {
                        return Err(malformed_mir(format!(
                            "return from {} has inconsistent nullable class ownership",
                            function.name
                        )));
                    }
                    if expected.is_none() {
                        require_owned_nullable_class_expression(
                            class,
                            &format!("return from {}", function.name),
                        )?;
                    }
                } else if let (mir::Type::Collection(_), mir::Rvalue::Collection(collection)) =
                    (return_type, expression)
                {
                    let expected = infer_function_return_borrow(program, function)?;
                    let actual =
                        infer_collection_expression_return_borrow(program, function, collection)?;
                    if !return_borrow_is_compatible(actual, expected) {
                        return Err(malformed_mir(format!(
                            "return from {} has inconsistent collection ownership",
                            function.name
                        )));
                    }
                    if expected.is_none() && collection.owned_temporary_collection().is_none() {
                        return Err(malformed_mir(format!(
                            "return from {} receives a borrowed collection value",
                            function.name
                        )));
                    }
                } else if let (mir::Type::Mixed, mir::Rvalue::Mixed(mixed)) =
                    (return_type, expression)
                {
                    let expected = infer_function_return_borrow(program, function)?;
                    let actual = infer_mixed_expression_return_borrow(program, function, mixed)?;
                    if !return_borrow_is_compatible(actual, expected) {
                        return Err(malformed_mir(format!(
                            "return from {} has inconsistent mixed ownership",
                            function.name
                        )));
                    }
                    if expected.is_none() && !mixed.ownership().has_shell() {
                        return Err(malformed_mir(format!(
                            "return from {} receives a borrowed mixed value",
                            function.name
                        )));
                    }
                } else if let (mir::Type::Function(_), mir::Rvalue::Function(value)) =
                    (return_type, expression)
                {
                    if function_expression_is_borrowed(value) {
                        return Err(malformed_mir(format!(
                            "return from {} receives a borrowed function carrier",
                            function.name
                        )));
                    }
                } else if let (
                    mir::Type::NullableFunction(_),
                    mir::Rvalue::NullableFunction(value),
                ) = (return_type, expression)
                {
                    if nullable_function_expression_is_borrowed(value) {
                        return Err(malformed_mir(format!(
                            "return from {} receives a borrowed nullable function carrier",
                            function.name
                        )));
                    }
                }
            }
            Ok(())
        }
        mir::Terminator::ReturnVoid => {
            if function.return_type != mir::ReturnType::Void {
                return Err(malformed_mir(format!(
                    "scalar function {} has a void return",
                    function.name
                )));
            }
            Ok(())
        }
        mir::Terminator::Panic { message, .. } => {
            validate_string_expression(program, function, message)
        }
        mir::Terminator::Unreachable => Ok(()),
        mir::Terminator::Jump(target) => block_in(function, *target).map(|_| ()),
        mir::Terminator::Branch {
            condition,
            then_block,
            else_block,
        } => {
            block_in(function, *then_block)?;
            block_in(function, *else_block)?;
            validate_condition(program, function, condition)
        }
        mir::Terminator::CheckedCall {
            function: callee,
            args,
            result,
            error,
            success,
            failure,
            ..
        } => {
            let callee = function_in(program, *callee)?;
            validate_checked_call_args(program, function, callee, args)?;
            match (callee.return_type, result) {
                (mir::ReturnType::Void, None) => {}
                (mir::ReturnType::Value(expected), Some(result)) => {
                    let result = local_in(function, *result)?;
                    if result.ty != expected || !result.synthetic {
                        return Err(malformed_mir(
                            "checked call has an incompatible success slot",
                        ));
                    }
                }
                _ => {
                    return Err(malformed_mir(
                        "checked call has the wrong success-slot shape",
                    ));
                }
            }
            let error = local_in(function, *error)?;
            if error.ty != mir::Type::Error || !error.owned || !error.synthetic {
                return Err(malformed_mir("checked call has an incompatible Error slot"));
            }
            block_in(function, *success)?;
            block_in(function, *failure)?;
            if success == failure {
                return Err(malformed_mir(
                    "checked call success and error edges are identical",
                ));
            }
            Ok(())
        }
        mir::Terminator::IndirectCall {
            callee,
            function_type,
            invocation_mode,
            args,
            result,
            continuation,
            ..
        } => {
            validate_indirect_call(
                program,
                function,
                IndirectCallValidation {
                    callee,
                    function_type: *function_type,
                    invocation_mode: *invocation_mode,
                    args,
                    result: *result,
                    error: None,
                },
            )?;
            block_in(function, *continuation)?;
            Ok(())
        }
        mir::Terminator::CheckedIndirectCall {
            callee,
            function_type,
            invocation_mode,
            args,
            result,
            error,
            success,
            failure,
            ..
        } => {
            validate_indirect_call(
                program,
                function,
                IndirectCallValidation {
                    callee,
                    function_type: *function_type,
                    invocation_mode: *invocation_mode,
                    args,
                    result: *result,
                    error: Some(*error),
                },
            )?;
            block_in(function, *success)?;
            block_in(function, *failure)?;
            if success == failure {
                return Err(malformed_mir(
                    "checked indirect call success and error edges are identical",
                ));
            }
            Ok(())
        }
        mir::Terminator::CheckedConstruct {
            class,
            properties,
            constructor,
            args,
            result,
            error,
            success,
            failure,
            ..
        } => {
            let class_definition = class_in(program, *class)?;
            if class_definition.constructor != Some(*constructor) {
                return Err(malformed_mir(
                    "checked construction names the wrong class constructor",
                ));
            }
            let constructor_definition = function_in(program, *constructor)?;
            if constructor_definition.checked_effects.is_empty() {
                return Err(malformed_mir(
                    "checked construction targets a nonthrowing constructor",
                ));
            }
            let result_definition = local_in(function, *result)?;
            if result_definition.ty != mir::Type::Class(*class)
                || !result_definition.owned
                || !result_definition.synthetic
            {
                return Err(malformed_mir(
                    "checked construction has an incompatible success slot",
                ));
            }
            let error_definition = local_in(function, *error)?;
            if error_definition.ty != mir::Type::Error
                || !error_definition.owned
                || !error_definition.synthetic
            {
                return Err(malformed_mir(
                    "checked construction has an incompatible Error slot",
                ));
            }
            validate_class_expression(
                program,
                function,
                &mir::ClassExpression::New {
                    class: *class,
                    properties: properties.clone(),
                    constructor: Some(*constructor),
                    args: args.clone(),
                },
            )?;
            block_in(function, *success)?;
            block_in(function, *failure)?;
            if success == failure {
                return Err(malformed_mir(
                    "checked construction success and error edges are identical",
                ));
            }
            Ok(())
        }
        mir::Terminator::CheckedIo {
            operation,
            result,
            error,
            success,
            failure,
            ..
        } => {
            validate_checked_io_operation(program, function, operation)?;
            let expected = match operation {
                mir::CheckedIoOperation::ReadLine { .. } => Some(mir::Type::NullableString),
                mir::CheckedIoOperation::ReadFile { bytes: false, .. } => Some(mir::Type::String),
                mir::CheckedIoOperation::ReadFile { bytes: true, .. }
                | mir::CheckedIoOperation::ReadStdinBytes => result
                    .map(|local| local_in(function, local).map(|definition| definition.ty))
                    .transpose()?,
                mir::CheckedIoOperation::WriteFile { .. }
                | mir::CheckedIoOperation::WriteStream { .. } => None,
            };
            match (expected, result) {
                (None, None) => {}
                (Some(expected), Some(result)) => {
                    let definition = local_in(function, *result)?;
                    if definition.ty != expected || !definition.synthetic {
                        return Err(malformed_mir(
                            "checked I/O has an incompatible success slot",
                        ));
                    }
                    if matches!(
                        operation,
                        mir::CheckedIoOperation::ReadFile { bytes: true, .. }
                            | mir::CheckedIoOperation::ReadStdinBytes
                    ) {
                        let mir::Type::Collection(collection) = expected else {
                            return Err(malformed_mir(
                                "checked byte input does not return a collection",
                            ));
                        };
                        if collection_in(program, collection)?.kind != mir::CollectionKind::Bytes {
                            return Err(malformed_mir("checked byte input does not return Bytes"));
                        }
                    }
                }
                _ => {
                    return Err(malformed_mir(
                        "checked I/O has the wrong success-slot shape",
                    ))
                }
            }
            let error = local_in(function, *error)?;
            if error.ty != mir::Type::Error || !error.owned || !error.synthetic {
                return Err(malformed_mir("checked I/O has an incompatible Error slot"));
            }
            block_in(function, *success)?;
            block_in(function, *failure)?;
            if success == failure {
                return Err(malformed_mir(
                    "checked I/O success and error edges are identical",
                ));
            }
            Ok(())
        }
        mir::Terminator::ErrorSwitch {
            error,
            cases,
            catch_all,
            fallback,
        } => {
            let error = local_in(function, *error)?;
            if error.ty != mir::Type::Error || !error.owned {
                return Err(malformed_mir(
                    "Error dispatch does not own an Error carrier",
                ));
            }
            let mut descriptors = HashSet::new();
            for (descriptor, target) in cases {
                error_descriptor_in(program, *descriptor)?;
                block_in(function, *target)?;
                if !descriptors.insert(*descriptor) {
                    return Err(malformed_mir(
                        "Error dispatch repeats a concrete descriptor",
                    ));
                }
            }
            if let Some(target) = catch_all {
                block_in(function, *target)?;
            }
            block_in(function, *fallback)?;
            Ok(())
        }
        mir::Terminator::PropagateError { error } => {
            let error = local_in(function, *error)?;
            if error.ty != mir::Type::Error || !error.owned {
                return Err(malformed_mir(
                    "checked propagation does not own an Error carrier",
                ));
            }
            if function.checked_effects.is_empty() {
                return Err(malformed_mir(
                    "nonthrowing function propagates a checked Error",
                ));
            }
            Ok(())
        }
    }
}

fn validate_checked_io_operation(
    program: &mir::Program,
    function: &mir::Function,
    operation: &mir::CheckedIoOperation,
) -> Result<(), BackendError> {
    match operation {
        mir::CheckedIoOperation::ReadLine { prompt } => {
            validate_string_expression(program, function, prompt)
        }
        mir::CheckedIoOperation::ReadFile { path, .. } => {
            validate_string_expression(program, function, path)
        }
        mir::CheckedIoOperation::ReadStdinBytes => Ok(()),
        mir::CheckedIoOperation::WriteFile { path, contents, .. } => {
            validate_string_expression(program, function, path)?;
            validate_io_contents(program, function, contents)
        }
        mir::CheckedIoOperation::WriteStream { contents, .. } => {
            validate_io_contents(program, function, contents)
        }
    }
}

struct IndirectCallValidation<'a> {
    callee: &'a mir::FunctionExpression,
    function_type: mir::FunctionTypeId,
    invocation_mode: mir::FunctionInvocationMode,
    args: &'a [mir::Rvalue],
    result: Option<mir::LocalId>,
    error: Option<mir::LocalId>,
}

fn validate_indirect_call(
    program: &mir::Program,
    caller: &mir::Function,
    call: IndirectCallValidation<'_>,
) -> Result<(), BackendError> {
    let definition = function_type_in(program, call.function_type)?;
    if call.callee.function_type() != call.function_type
        || definition.invocation_mode != call.invocation_mode
    {
        return Err(malformed_mir(
            "indirect call disagrees with its structural function type",
        ));
    }
    validate_function_expression(program, caller, call.callee)?;
    match call.invocation_mode {
        mir::FunctionInvocationMode::Readonly => {}
        mir::FunctionInvocationMode::Writable => {
            let local = match call.callee {
                mir::FunctionExpression::Local {
                    local,
                    transfer: false,
                    ..
                }
                | mir::FunctionExpression::MixedPayload {
                    mixed: local,
                    transfer: false,
                    ..
                } => *local,
                _ => {
                    return Err(malformed_mir(
                        "writable indirect call does not use a borrowed local carrier",
                    ));
                }
            };
            if !local_in(caller, local)?.writable {
                return Err(malformed_mir(
                    "writable indirect call uses a readonly function carrier",
                ));
            }
        }
        mir::FunctionInvocationMode::Once => {
            if function_expression_is_borrowed(call.callee) {
                return Err(malformed_mir(
                    "once indirect call does not consume its function carrier",
                ));
            }
        }
    }
    if call.args.len() != definition.parameters.len() {
        return Err(malformed_mir(format!(
            "indirect call expects {} arguments, got {}",
            definition.parameters.len(),
            call.args.len()
        )));
    }
    for (index, (argument, parameter)) in call.args.iter().zip(&definition.parameters).enumerate() {
        if argument.ty() != parameter.ty {
            return Err(malformed_mir(format!(
                "indirect call passes {} argument {} to {} parameter",
                argument.ty(),
                index + 1,
                parameter.ty
            )));
        }
        validate_rvalue(program, caller, argument)?;
        if parameter.mode == mir::FunctionParameterMode::Take
            && parameter.ty.has_move_ownership()
            && argument.borrows_move_value()
        {
            return Err(malformed_mir(format!(
                "indirect call borrows argument {} for a take parameter",
                index + 1
            )));
        }
    }
    match (definition.return_type, call.result) {
        (mir::ReturnType::Void, None) => {}
        (mir::ReturnType::Value(expected), Some(result)) => {
            let result = local_in(caller, result)?;
            if result.ty != expected || !result.synthetic {
                return Err(malformed_mir(
                    "indirect call has an incompatible result slot",
                ));
            }
            if expected.has_move_ownership() && !result.owned {
                return Err(malformed_mir(
                    "indirect call move result does not own its result slot",
                ));
            }
        }
        _ => {
            return Err(malformed_mir(
                "indirect call has the wrong result-slot shape",
            ))
        }
    }
    match (definition.checked_effects.is_empty(), call.error) {
        (true, None) => {}
        (false, Some(error)) => {
            let error = local_in(caller, error)?;
            if error.ty != mir::Type::Error || !error.owned || !error.synthetic {
                return Err(malformed_mir(
                    "checked indirect call has an incompatible Error slot",
                ));
            }
        }
        (true, Some(_)) => {
            return Err(malformed_mir(
                "checked indirect call uses a nonthrowing function type",
            ));
        }
        (false, None) => {
            return Err(malformed_mir(
                "ordinary indirect call uses a throwing function type",
            ));
        }
    }
    Ok(())
}

fn validate_io_contents(
    program: &mir::Program,
    function: &mir::Function,
    contents: &mir::IoContents,
) -> Result<(), BackendError> {
    match contents {
        mir::IoContents::String(value) => validate_string_expression(program, function, value),
        mir::IoContents::Format(value) => validate_format_expression(program, function, value),
        mir::IoContents::Bytes(local) => {
            let local = local_in(function, *local)?;
            let mir::Type::Collection(collection) = local.ty else {
                return Err(malformed_mir("checked byte I/O uses another local type"));
            };
            if collection_in(program, collection)?.kind != mir::CollectionKind::Bytes {
                return Err(malformed_mir("checked byte I/O local is not Bytes"));
            }
            Ok(())
        }
    }
}

fn validate_integer_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::IntegerExpression,
) -> Result<(), BackendError> {
    match expression {
        mir::IntegerExpression::Use { ty, operand } => {
            validate_integer_operand(program, function, *ty, operand)
        }
        mir::IntegerExpression::Unary {
            ty, op, operand, ..
        } => {
            if operand.ty() != *ty {
                return Err(malformed_mir(format!(
                    "{ty} unary expression contains {} operand",
                    operand.ty()
                )));
            }
            if matches!(op, mir::IntegerUnaryOp::Negate) && !ty.is_signed() {
                return Err(malformed_mir(format!(
                    "unsigned {ty} expression uses unary negation"
                )));
            }
            validate_integer_expression(program, function, operand)
        }
        mir::IntegerExpression::Binary {
            ty, left, right, ..
        } => {
            if left.ty() != *ty || right.ty() != *ty {
                return Err(malformed_mir(format!(
                    "{ty} binary expression has {} and {} operands",
                    left.ty(),
                    right.ty()
                )));
            }
            validate_integer_expression(program, function, left)?;
            validate_integer_expression(program, function, right)
        }
        mir::IntegerExpression::Convert { value, .. } => {
            validate_integer_expression(program, function, value)
        }
        mir::IntegerExpression::FloatToInt { value, .. } => {
            validate_float_expression(program, function, value)
        }
        mir::IntegerExpression::Call {
            ty,
            function: callee,
            args,
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type
                != mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Integer(*ty)))
            {
                return Err(malformed_mir(format!(
                    "{ty} call targets function {} returning {}",
                    callee.name, callee.return_type
                )));
            }
            validate_call_args(program, function, callee, args)
        }
        mir::IntegerExpression::Coalesce { ty, left, right } => {
            if left.ty() != mir::ScalarType::Integer(*ty) || right.ty() != *ty {
                return Err(malformed_mir("integer coalesce has incompatible operands"));
            }
            validate_nullable_scalar_expression(program, function, left)?;
            validate_integer_expression(program, function, right)
        }
        mir::IntegerExpression::EnumBacking { enum_id, value } => {
            let definition = enum_in(program, *enum_id)?;
            if definition.backing_type != Some(crate::enums::EnumBackingType::Int) {
                return Err(malformed_mir(
                    "integer backing projection targets a non-int-backed enum",
                ));
            }
            if value.enum_id() != *enum_id {
                return Err(malformed_mir(
                    "integer backing projection uses a different enum type",
                ));
            }
            validate_enum_expression(program, function, value)
        }
    }
}

fn validate_value_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::ValueExpression,
) -> Result<(), BackendError> {
    match expression {
        mir::ValueExpression::Integer(value) => {
            validate_integer_expression(program, function, value)
        }
        mir::ValueExpression::Float(value) => validate_float_expression(program, function, value),
        mir::ValueExpression::Bool(value) => validate_condition(program, function, value),
        mir::ValueExpression::Enum(value) => validate_enum_expression(program, function, value),
    }
}

fn validate_enum_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::EnumExpression,
) -> Result<(), BackendError> {
    let definition = enum_in(program, expression.enum_id())?;
    match expression {
        mir::EnumExpression::Case(value) => {
            if value.case_id.enum_id != value.enum_id {
                return Err(malformed_mir("enum case identity names another enum"));
            }
            let case = definition
                .cases
                .get(value.case_id.index)
                .filter(|case| case.id == value.case_id)
                .ok_or_else(|| malformed_mir("enum case does not exist"))?;
            if !case.payload.is_empty() {
                return Err(malformed_mir(
                    "unit enum expression constructs a payload case",
                ));
            }
            Ok(())
        }
        mir::EnumExpression::Use { enum_id, operand } => {
            validate_enum_operand(program, function, *enum_id, operand)
        }
        mir::EnumExpression::Call {
            enum_id,
            function: callee,
            args,
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type
                != mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Enum(*enum_id)))
            {
                return Err(malformed_mir(
                    "enum call targets a function returning another type",
                ));
            }
            validate_call_args(program, function, callee, args)
        }
        mir::EnumExpression::Coalesce {
            enum_id,
            left,
            right,
        } => {
            if left.ty() != mir::ScalarType::Enum(*enum_id) || right.enum_id() != *enum_id {
                return Err(malformed_mir("enum coalesce has incompatible operands"));
            }
            validate_nullable_scalar_expression(program, function, left)?;
            validate_enum_expression(program, function, right)
        }
    }
}

fn validate_enum_operand(
    program: &mir::Program,
    function: &mir::Function,
    enum_id: crate::enums::EnumId,
    operand: &mir::Operand,
) -> Result<(), BackendError> {
    let expected = mir::Type::Scalar(mir::ScalarType::Enum(enum_id));
    match operand {
        mir::Operand::Scalar(mir::ScalarValue::Enum(value)) => {
            if value.enum_id != enum_id {
                return Err(malformed_mir("enum expression contains another enum value"));
            }
            validate_enum_expression(program, function, &mir::EnumExpression::Case(*value))
        }
        mir::Operand::Scalar(_) => Err(malformed_mir(
            "enum expression contains a non-enum scalar constant",
        )),
        mir::Operand::Local(local) => {
            if local_in(function, *local)?.ty != expected {
                return Err(malformed_mir("enum expression uses another local type"));
            }
            Ok(())
        }
        mir::Operand::NullablePayload(local) => {
            if local_in(function, *local)?.ty
                != mir::Type::NullableScalar(mir::ScalarType::Enum(enum_id))
            {
                return Err(malformed_mir(
                    "enum expression uses another nullable payload type",
                ));
            }
            Ok(())
        }
        mir::Operand::Static(id) => validate_static_operand(program, *id, expected),
        mir::Operand::Property { object, property } => {
            validate_property_operand(program, function, *object, *property, expected)
        }
        mir::Operand::CollectionIndex {
            positional,
            collection,
            index,
            remove,
        } => {
            let local = local_in(function, *collection)?;
            let mir::Type::Collection(collection_type) = local.ty else {
                return Err(malformed_mir("enum index source is not a collection"));
            };
            let collection_type = collection_in(program, collection_type)?;
            if collection_type.value != expected {
                return Err(malformed_mir("enum collection element type mismatch"));
            }
            validate_collection_element_access(
                program,
                function,
                local,
                collection_type,
                index,
                *remove,
                *positional,
            )
        }
        mir::Operand::MixedPayload { mixed, tag } => {
            validate_mixed_payload_operand(function, *mixed, *tag, expected)
        }
        mir::Operand::CollectionLength(_)
        | mir::Operand::CollectionKeyAt { .. }
        | mir::Operand::StringIntrinsic(_) => Err(malformed_mir(
            "enum expression uses an incompatible operand",
        )),
    }
}

fn validate_rvalue(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::Rvalue,
) -> Result<(), BackendError> {
    match expression {
        mir::Rvalue::Value(value) => validate_value_expression(program, function, value),
        mir::Rvalue::String(value) => validate_string_expression(program, function, value),
        mir::Rvalue::Mixed(value) => validate_mixed_expression(program, function, value),
        mir::Rvalue::NullableScalar(value) => {
            validate_nullable_scalar_expression(program, function, value)
        }
        mir::Rvalue::NullableString(value) => {
            validate_nullable_string_expression(program, function, value)
        }
        mir::Rvalue::NullableMixed(value) => {
            validate_nullable_mixed_expression(program, function, value)
        }
        mir::Rvalue::Error(value) => validate_error_expression(program, function, value),
        mir::Rvalue::NullableError(value) => {
            validate_nullable_error_expression(program, function, value)
        }
        mir::Rvalue::Class(value) => validate_class_expression(program, function, value),
        mir::Rvalue::NullableClass(value) => {
            validate_nullable_class_expression(program, function, value)
        }
        mir::Rvalue::SharedReference(value) => {
            validate_shared_reference_expression(program, function, value)
        }
        mir::Rvalue::WeakReference(value) => {
            validate_weak_reference_expression(program, function, value)
        }
        mir::Rvalue::NullableSharedReference(value) => {
            validate_nullable_shared_reference_expression(program, function, value)
        }
        mir::Rvalue::NullableWeakReference(value) => {
            validate_nullable_weak_reference_expression(program, function, value)
        }
        mir::Rvalue::WritableSharedReference(value) => {
            validate_writable_shared_reference_expression(program, function, value)
        }
        mir::Rvalue::WritableWeakReference(value) => {
            validate_writable_weak_reference_expression(program, function, value)
        }
        mir::Rvalue::NullableWritableSharedReference(value) => {
            validate_nullable_writable_shared_reference_expression(program, function, value)
        }
        mir::Rvalue::NullableWritableWeakReference(value) => {
            validate_nullable_writable_weak_reference_expression(program, function, value)
        }
        mir::Rvalue::SharedReferenceAccess(value) => {
            validate_shared_reference_access_expression(program, function, value)
        }
        mir::Rvalue::NullableSharedReferenceAccess(value) => {
            validate_nullable_shared_reference_access_expression(program, function, value)
        }
        mir::Rvalue::Collection(value) => validate_collection_expression(program, function, value),
        mir::Rvalue::NullableCollection(value) => {
            validate_nullable_collection_expression(program, function, value)
        }
        mir::Rvalue::PayloadEnum(value) => {
            validate_payload_enum_expression(program, function, value)
        }
        mir::Rvalue::NullablePayloadEnum(value) => {
            validate_nullable_payload_enum_expression(program, function, value)
        }
        mir::Rvalue::Function(value) => validate_function_expression(program, function, value),
        mir::Rvalue::NullableFunction(value) => {
            validate_nullable_function_expression(program, function, value)
        }
    }?;
    let mut accesses = ClassLocalAccesses::default();
    collect_rvalue_class_local_accesses(expression, &mut accesses);
    validate_ordered_class_accesses(
        program,
        "rvalue",
        &accesses,
        &HashMap::new(),
        &mut HashSet::new(),
    )?;
    Ok(())
}

fn validate_function_assignment_ownership(
    local: &mir::Local,
    borrowed: bool,
) -> Result<(), BackendError> {
    if local.owned == borrowed {
        return Err(malformed_mir(format!(
            "function-value local local{} has inconsistent ownership",
            local.id.0
        )));
    }
    Ok(())
}

fn function_expression_is_borrowed(value: &mir::FunctionExpression) -> bool {
    mir::function_expression_is_borrowed(value)
}

fn nullable_function_expression_is_borrowed(value: &mir::NullableFunctionExpression) -> bool {
    mir::nullable_function_expression_is_borrowed(value)
}

fn validate_function_expression(
    program: &mir::Program,
    function: &mir::Function,
    value: &mir::FunctionExpression,
) -> Result<(), BackendError> {
    let expected = value.function_type();
    function_type_in(program, expected)?;
    match value {
        mir::FunctionExpression::Create {
            descriptor,
            captures,
            span,
            ..
        } => {
            let descriptor = closure_descriptor_in(program, *descriptor)?;
            if descriptor.function_type != expected || descriptor.source_span != *span {
                return Err(malformed_mir(
                    "closure construction disagrees with its descriptor",
                ));
            }
            match descriptor.environment_layout {
                None if captures.is_empty() => Ok(()),
                None => Err(malformed_mir(
                    "no-capture closure construction supplies capture operands",
                )),
                Some(layout) => {
                    let layout = closure_environment_layout_in(program, layout)?;
                    if captures.len() != layout.fields.len() {
                        return Err(malformed_mir(
                            "closure construction capture count does not match its layout",
                        ));
                    }
                    for (capture, field) in captures.iter().zip(&layout.fields) {
                        match (capture, field.storage) {
                            (
                                mir::ClosureCaptureOperand::BorrowLocal { local, writable },
                                mir::ClosureEnvironmentStorage::ReadonlyBorrow
                                | mir::ClosureEnvironmentStorage::WritableBorrow,
                            ) => {
                                let source = local_in(function, *local)?;
                                let expected_writable = matches!(
                                    field.storage,
                                    mir::ClosureEnvironmentStorage::WritableBorrow
                                );
                                if source.ty != field.ty
                                    || *writable != expected_writable
                                    || (expected_writable && !source.writable)
                                {
                                    return Err(malformed_mir(
                                        "closure borrow capture has incompatible type or access",
                                    ));
                                }
                            }
                            (
                                mir::ClosureCaptureOperand::CopyValue(value)
                                | mir::ClosureCaptureOperand::MoveValue(value),
                                mir::ClosureEnvironmentStorage::Owned,
                            ) => {
                                if value.ty() != field.ty {
                                    return Err(malformed_mir(
                                        "closure owned capture has an incompatible type",
                                    ));
                                }
                                validate_rvalue(program, function, value)?;
                                if matches!(capture, mir::ClosureCaptureOperand::MoveValue(_))
                                    && value.borrows_move_value()
                                {
                                    return Err(malformed_mir(
                                        "closure move capture uses a borrowed value",
                                    ));
                                }
                            }
                            _ => {
                                return Err(malformed_mir(
                                    "closure capture operand does not match environment storage",
                                ));
                            }
                        }
                    }
                    Ok(())
                }
            }
        }
        mir::FunctionExpression::Local {
            local, transfer, ..
        } => {
            let local = local_in(function, *local)?;
            if local.ty != mir::Type::Function(expected) || (*transfer && !local.owned) {
                return Err(malformed_mir(format!(
                    "function `{}` local expression uses local{} with type `{}` and owned={}, but expected `{}` with transfer={transfer}",
                    function.name,
                    local.id.0,
                    local.ty,
                    local.owned,
                    mir::Type::Function(expected),
                )));
            }
            Ok(())
        }
        mir::FunctionExpression::Property {
            object, property, ..
        } => validate_property_operand(
            program,
            function,
            *object,
            *property,
            mir::Type::Function(expected),
        ),
        mir::FunctionExpression::Call {
            function: callee,
            args,
            return_borrow,
            ..
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type != mir::ReturnType::Value(mir::Type::Function(expected))
                || *return_borrow != infer_function_return_borrow(program, callee)?
            {
                return Err(malformed_mir(
                    "direct call function-value result disagrees with its callee",
                ));
            }
            validate_call_args(program, function, callee, args)
        }
        mir::FunctionExpression::CollectionIndex {
            collection,
            index,
            positional,
            remove,
            ..
        } => {
            let local = local_in(function, *collection)?;
            let mir::Type::Collection(collection_type) = local.ty else {
                return Err(malformed_mir(
                    "function collection access uses a non-collection local",
                ));
            };
            let collection_type = collection_in(program, collection_type)?;
            if collection_type.value != mir::Type::Function(expected) {
                return Err(malformed_mir(
                    "function collection element type does not match",
                ));
            }
            validate_collection_element_access(
                program,
                function,
                local,
                collection_type,
                index,
                *remove,
                *positional,
            )
        }
        mir::FunctionExpression::MixedPayload { mixed, .. } => validate_mixed_payload_operand(
            function,
            *mixed,
            mir::MixedTag::Function(expected),
            mir::Type::Function(expected),
        ),
        mir::FunctionExpression::AssumePresent { value, .. } => {
            if value.function_type() != expected {
                return Err(malformed_mir(
                    "narrowed nullable function uses another structural type",
                ));
            }
            validate_nullable_function_expression(program, function, value)
        }
    }
}

fn validate_nullable_function_expression(
    program: &mir::Program,
    function: &mir::Function,
    value: &mir::NullableFunctionExpression,
) -> Result<(), BackendError> {
    let expected = value.function_type();
    function_type_in(program, expected)?;
    match value {
        mir::NullableFunctionExpression::Null { .. } => Ok(()),
        mir::NullableFunctionExpression::Present(value) => {
            if value.function_type() != expected {
                return Err(malformed_mir(
                    "present nullable function uses another structural type",
                ));
            }
            validate_function_expression(program, function, value)
        }
        mir::NullableFunctionExpression::Local {
            local, transfer, ..
        } => {
            let local = local_in(function, *local)?;
            if local.ty != mir::Type::NullableFunction(expected) || (*transfer && !local.owned) {
                return Err(malformed_mir(format!(
                    "function `{}` nullable function local expression uses local{} with type `{}` and owned={}, but expected `{}` with transfer={transfer}",
                    function.name,
                    local.id.0,
                    local.ty,
                    local.owned,
                    mir::Type::NullableFunction(expected),
                )));
            }
            Ok(())
        }
        mir::NullableFunctionExpression::Property {
            object, property, ..
        } => validate_property_operand(
            program,
            function,
            *object,
            *property,
            mir::Type::NullableFunction(expected),
        ),
        mir::NullableFunctionExpression::Call {
            function: callee,
            args,
            return_borrow,
            ..
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type != mir::ReturnType::Value(mir::Type::NullableFunction(expected))
                || *return_borrow != infer_function_return_borrow(program, callee)?
            {
                return Err(malformed_mir(
                    "direct call nullable-function result disagrees with its callee",
                ));
            }
            validate_call_args(program, function, callee, args)
        }
        mir::NullableFunctionExpression::DictionaryGet {
            collection,
            key,
            access,
            ..
        } => validate_dictionary_get(
            program,
            function,
            *collection,
            key,
            mir::Type::Function(expected),
            *access,
        ),
        mir::NullableFunctionExpression::CollectionIndex {
            collection,
            index,
            positional,
            remove,
            ..
        } => {
            let local = local_in(function, *collection)?;
            let mir::Type::Collection(collection_type) = local.ty else {
                return Err(malformed_mir(
                    "nullable function collection access uses a non-collection local",
                ));
            };
            let collection_type = collection_in(program, collection_type)?;
            if collection_type.value != mir::Type::NullableFunction(expected) {
                return Err(malformed_mir(
                    "nullable function collection element type does not match",
                ));
            }
            validate_collection_element_access(
                program,
                function,
                local,
                collection_type,
                index,
                *remove,
                *positional,
            )
        }
    }
}

fn validate_error_assignment_ownership(
    local: &mir::Local,
    borrowed: bool,
) -> Result<(), BackendError> {
    if local.owned == borrowed {
        return Err(malformed_mir(format!(
            "Error local local{} has inconsistent ownership",
            local.id.0
        )));
    }
    Ok(())
}

fn error_expression_is_borrowed(expression: &mir::ErrorExpression) -> bool {
    expression.is_borrowed()
}

fn nullable_error_expression_is_borrowed(expression: &mir::NullableErrorExpression) -> bool {
    expression.is_borrowed()
}

fn error_descriptor_in(
    program: &mir::Program,
    id: mir::ErrorDescriptorId,
) -> Result<&mir::ErrorDescriptor, BackendError> {
    program
        .error_descriptors
        .get(id.0)
        .filter(|descriptor| descriptor.id == id)
        .ok_or_else(|| malformed_mir(format!("Error descriptor#{} does not exist", id.0)))
}

fn validate_error_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::ErrorExpression,
) -> Result<(), BackendError> {
    match expression {
        mir::ErrorExpression::Local { local, transfer } => {
            let definition = local_in(function, *local)?;
            if definition.ty != mir::Type::Error || (*transfer && !definition.owned) {
                return Err(malformed_mir("Error expression uses an incompatible local"));
            }
            Ok(())
        }
        mir::ErrorExpression::NullableLocalAssumeNonNull { local, transfer } => {
            let definition = local_in(function, *local)?;
            if definition.ty != mir::Type::NullableError || (*transfer && !definition.owned) {
                return Err(malformed_mir(
                    "nonnull Error expression uses an incompatible nullable local",
                ));
            }
            Ok(())
        }
        mir::ErrorExpression::FromClass { object, descriptor } => {
            let descriptor = error_descriptor_in(program, *descriptor)?;
            if object.class() != descriptor.class {
                return Err(malformed_mir(
                    "Error erasure descriptor does not match its concrete class",
                ));
            }
            validate_class_expression(program, function, object)
        }
        mir::ErrorExpression::FromNullableClass { object, descriptor } => {
            let descriptor = error_descriptor_in(program, *descriptor)?;
            if object.class() != descriptor.class {
                return Err(malformed_mir(
                    "nullable Error erasure descriptor does not match its concrete class",
                ));
            }
            validate_nullable_class_expression(program, function, object)
        }
        mir::ErrorExpression::Property {
            object,
            property,
            transfer,
        } => {
            let definition = local_in(function, *object)?;
            if *transfer && !definition.writable {
                return Err(malformed_mir(
                    "Error property transfer uses a readonly receiver",
                ));
            }
            validate_property_operand(program, function, *object, *property, mir::Type::Error)
        }
        mir::ErrorExpression::Call {
            function: callee,
            args,
            return_borrow,
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type != mir::ReturnType::Value(mir::Type::Error)
                || *return_borrow != infer_function_return_borrow(program, callee)?
            {
                return Err(malformed_mir("Error call has an incompatible result"));
            }
            validate_call_args(program, function, callee, args)
        }
        mir::ErrorExpression::CollectionIndex {
            collection,
            index,
            positional,
            remove,
        } => {
            let local = local_in(function, *collection)?;
            let mir::Type::Collection(collection) = local.ty else {
                return Err(malformed_mir("Error index source is not a collection"));
            };
            let collection = collection_in(program, collection)?;
            if collection.value != mir::Type::Error {
                return Err(malformed_mir("Error collection element type mismatch"));
            }
            validate_collection_element_access(
                program,
                function,
                local,
                collection,
                index,
                *remove,
                *positional,
            )
        }
        mir::ErrorExpression::MixedPayload { mixed, .. } => {
            validate_mixed_payload_operand(function, *mixed, mir::MixedTag::Error, mir::Type::Error)
        }
    }
}

fn validate_nullable_error_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::NullableErrorExpression,
) -> Result<(), BackendError> {
    match expression {
        mir::NullableErrorExpression::Null => Ok(()),
        mir::NullableErrorExpression::Error(value) => {
            validate_error_expression(program, function, value)
        }
        mir::NullableErrorExpression::Local { local, transfer } => {
            let definition = local_in(function, *local)?;
            if definition.ty != mir::Type::NullableError || (*transfer && !definition.owned) {
                return Err(malformed_mir(
                    "nullable Error expression uses an incompatible local",
                ));
            }
            Ok(())
        }
        mir::NullableErrorExpression::Property {
            object,
            property,
            transfer,
        } => {
            let definition = local_in(function, *object)?;
            if *transfer && !definition.writable {
                return Err(malformed_mir(
                    "nullable Error property transfer uses a readonly receiver",
                ));
            }
            validate_property_operand(
                program,
                function,
                *object,
                *property,
                mir::Type::NullableError,
            )
        }
        mir::NullableErrorExpression::Call {
            function: callee,
            args,
            return_borrow,
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type != mir::ReturnType::Value(mir::Type::NullableError)
                || *return_borrow != infer_function_return_borrow(program, callee)?
            {
                return Err(malformed_mir(
                    "nullable Error call has an incompatible result",
                ));
            }
            validate_call_args(program, function, callee, args)
        }
        mir::NullableErrorExpression::DictionaryGet {
            collection,
            key,
            access,
        } => validate_dictionary_get(
            program,
            function,
            *collection,
            key,
            mir::Type::Error,
            *access,
        ),
        mir::NullableErrorExpression::CollectionIndex {
            collection,
            index,
            positional,
            remove,
        } => {
            let local = local_in(function, *collection)?;
            let mir::Type::Collection(collection) = local.ty else {
                return Err(malformed_mir(
                    "nullable Error index source is not a collection",
                ));
            };
            let collection = collection_in(program, collection)?;
            if !matches!(
                collection.value,
                mir::Type::Error | mir::Type::NullableError
            ) {
                return Err(malformed_mir(
                    "nullable Error collection element type mismatch",
                ));
            }
            validate_collection_element_access(
                program,
                function,
                local,
                collection,
                index,
                *remove,
                *positional,
            )
        }
    }
}

fn validate_payload_enum_assignment_ownership(
    local: &mir::Local,
    mode: Option<mir::PayloadEnumUseMode>,
) -> Result<(), BackendError> {
    let ty = match local.ty {
        mir::Type::PayloadEnum(ty) | mir::Type::NullablePayloadEnum(ty) => ty,
        _ => {
            return Err(malformed_mir(
                "payload enum ownership checks a non-enum local",
            ))
        }
    };
    if ty.capabilities.needs_drop && matches!(mode, Some(mir::PayloadEnumUseMode::Borrow)) {
        if local.owned || !local.synthetic {
            return Err(malformed_mir(format!(
                "borrowed payload enum requires a non-owning synthetic local, got local{}",
                local.id.0
            )));
        }
    } else if local.owned != ty.capabilities.needs_drop {
        return Err(malformed_mir(format!(
            "payload enum local local{} has inconsistent drop ownership",
            local.id.0
        )));
    }
    Ok(())
}

fn nullable_payload_enum_use_mode(
    expression: &mir::NullablePayloadEnumExpression,
) -> Option<mir::PayloadEnumUseMode> {
    match expression {
        mir::NullablePayloadEnumExpression::Use { mode, .. }
        | mir::NullablePayloadEnumExpression::CollectionGet { mode, .. }
        | mir::NullablePayloadEnumExpression::Coalesce { mode, .. } => Some(*mode),
        mir::NullablePayloadEnumExpression::Null(_)
        | mir::NullablePayloadEnumExpression::Value(_)
        | mir::NullablePayloadEnumExpression::Call { .. } => None,
    }
}

fn validate_payload_enum_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::PayloadEnumExpression,
) -> Result<(), BackendError> {
    let ty = expression.ty();
    let definition = validate_payload_enum_type(program, ty)?;
    match expression {
        mir::PayloadEnumExpression::Construct { case, fields, .. } => {
            if case.enum_id != ty.id {
                return Err(malformed_mir(
                    "payload enum construction uses another enum case",
                ));
            }
            let case_definition = definition
                .cases
                .get(case.index)
                .filter(|candidate| candidate.id == *case)
                .ok_or_else(|| malformed_mir("payload enum construction uses an unknown case"))?;
            if fields.len() != case_definition.payload.len() {
                return Err(malformed_mir(format!(
                    "payload enum case {} expects {} fields, got {}",
                    case_definition.name,
                    case_definition.payload.len(),
                    fields.len()
                )));
            }
            for (index, (field, expected)) in
                fields.iter().zip(&case_definition.payload).enumerate()
            {
                if field.ty() != expected.ty {
                    return Err(malformed_mir(format!(
                        "payload enum case {} field {} has type {}, expected {}",
                        case_definition.name,
                        index + 1,
                        field.ty(),
                        expected.ty
                    )));
                }
                validate_rvalue(program, function, field)?;
            }
            Ok(())
        }
        mir::PayloadEnumExpression::Use { place, mode, .. } => {
            validate_payload_enum_use_mode(ty, *mode)?;
            validate_payload_enum_place(program, function, place, mir::Type::PayloadEnum(ty), *mode)
        }
        mir::PayloadEnumExpression::Call {
            function: callee,
            args,
            ..
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type != mir::ReturnType::Value(mir::Type::PayloadEnum(ty)) {
                return Err(malformed_mir(
                    "payload enum call targets a function with another return type",
                ));
            }
            validate_call_args(program, function, callee, args)
        }
        mir::PayloadEnumExpression::Coalesce {
            left, right, mode, ..
        } => {
            validate_payload_enum_use_mode(ty, *mode)?;
            if left.ty() != ty || right.ty() != ty {
                return Err(malformed_mir(
                    "payload enum coalesce operands have another enum type",
                ));
            }
            validate_nullable_payload_enum_expression(program, function, left)?;
            validate_payload_enum_expression(program, function, right)
        }
    }
}

fn validate_nullable_payload_enum_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::NullablePayloadEnumExpression,
) -> Result<(), BackendError> {
    let ty = expression.ty();
    validate_payload_enum_type(program, ty)?;
    match expression {
        mir::NullablePayloadEnumExpression::Null(_) => Ok(()),
        mir::NullablePayloadEnumExpression::Value(value) => {
            if value.ty() != ty {
                return Err(malformed_mir(
                    "nullable payload enum wraps another enum type",
                ));
            }
            validate_payload_enum_expression(program, function, value)
        }
        mir::NullablePayloadEnumExpression::Use { place, mode, .. } => {
            validate_payload_enum_use_mode(ty, *mode)?;
            validate_payload_enum_place(
                program,
                function,
                place,
                mir::Type::NullablePayloadEnum(ty),
                *mode,
            )
        }
        mir::NullablePayloadEnumExpression::Call {
            function: callee,
            args,
            ..
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type != mir::ReturnType::Value(mir::Type::NullablePayloadEnum(ty)) {
                return Err(malformed_mir(
                    "nullable payload enum call targets a function with another return type",
                ));
            }
            validate_call_args(program, function, callee, args)
        }
        mir::NullablePayloadEnumExpression::CollectionGet {
            collection,
            key,
            access,
            stored_nullable,
            mode,
            ..
        } => {
            validate_payload_enum_use_mode(ty, *mode)?;
            let local = local_in(function, *collection)?;
            let mir::Type::Collection(collection_type) = local.ty else {
                return Err(malformed_mir(
                    "payload enum nullable access uses a non-collection local",
                ));
            };
            let definition = collection_in(program, collection_type)?;
            let expected_value = if *stored_nullable {
                mir::Type::NullablePayloadEnum(ty)
            } else {
                mir::Type::PayloadEnum(ty)
            };
            if definition.value != expected_value {
                return Err(malformed_mir(
                    "payload enum nullable access has incorrect stored nullability",
                ));
            }
            let mutating = matches!(
                access,
                mir::NullableCollectionAccess::Remove
                    | mir::NullableCollectionAccess::Pop
                    | mir::NullableCollectionAccess::PopFront
                    | mir::NullableCollectionAccess::PopBack
            );
            if mutating == matches!(mode, mir::PayloadEnumUseMode::Borrow) {
                return Err(malformed_mir(
                    "payload enum nullable access transfer mode disagrees with mutation",
                ));
            }
            validate_dictionary_get(
                program,
                function,
                *collection,
                key,
                mir::Type::PayloadEnum(ty),
                *access,
            )
        }
        mir::NullablePayloadEnumExpression::Coalesce {
            left, right, mode, ..
        } => {
            validate_payload_enum_use_mode(ty, *mode)?;
            if left.ty() != ty || right.ty() != ty {
                return Err(malformed_mir(
                    "nullable payload enum coalesce operands have another enum type",
                ));
            }
            validate_nullable_payload_enum_expression(program, function, left)?;
            validate_nullable_payload_enum_expression(program, function, right)
        }
    }
}

fn validate_payload_enum_use_mode(
    ty: mir::PayloadEnumType,
    mode: mir::PayloadEnumUseMode,
) -> Result<(), BackendError> {
    match mode {
        mir::PayloadEnumUseMode::Borrow => Ok(()),
        mir::PayloadEnumUseMode::Copy if ty.capabilities.copy => Ok(()),
        mir::PayloadEnumUseMode::Move if !ty.capabilities.copy => Ok(()),
        mir::PayloadEnumUseMode::Copy => Err(malformed_mir(
            "move payload enum is copied instead of transferred",
        )),
        mir::PayloadEnumUseMode::Move => Err(malformed_mir(
            "copy payload enum is transferred instead of copied",
        )),
    }
}

fn validate_payload_enum_place(
    program: &mir::Program,
    function: &mir::Function,
    place: &mir::PayloadEnumPlace,
    expected: mir::Type,
    mode: mir::PayloadEnumUseMode,
) -> Result<(), BackendError> {
    match place {
        mir::PayloadEnumPlace::Local(local) => {
            let definition = local_in(function, *local)?;
            if definition.ty != expected {
                return Err(malformed_mir(format!(
                    "payload enum place local{} has type {}, expected {}",
                    local.0, definition.ty, expected
                )));
            }
            if matches!(mode, mir::PayloadEnumUseMode::Move) && !definition.owned {
                return Err(malformed_mir(
                    "payload enum move transfers a borrowed local",
                ));
            }
            Ok(())
        }
        mir::PayloadEnumPlace::NullableLocalAssumeNonNull(local) => {
            let definition = local_in(function, *local)?;
            let mir::Type::PayloadEnum(expected) = expected else {
                return Err(malformed_mir(
                    "nonnull nullable payload enum place is used as a nullable value",
                ));
            };
            if definition.ty != mir::Type::NullablePayloadEnum(expected) {
                return Err(malformed_mir(format!(
                    "nonnull payload enum place local{} has type {}, expected ?payload-enum#{}",
                    local.0, definition.ty, expected.id.0
                )));
            }
            if matches!(mode, mir::PayloadEnumUseMode::Move) && !definition.owned {
                return Err(malformed_mir(
                    "payload enum move transfers a borrowed nullable local",
                ));
            }
            Ok(())
        }
        mir::PayloadEnumPlace::Static(id) => validate_static_operand(program, *id, expected),
        mir::PayloadEnumPlace::Property { object, property } => {
            validate_property_operand(program, function, *object, *property, expected)
        }
        mir::PayloadEnumPlace::CollectionIndex {
            collection,
            index,
            positional,
            remove,
        } => {
            let local = local_in(function, *collection)?;
            let mir::Type::Collection(collection_type) = local.ty else {
                return Err(malformed_mir(
                    "payload enum index source is not a collection",
                ));
            };
            let collection_type = collection_in(program, collection_type)?;
            if collection_type.value != expected {
                return Err(malformed_mir(
                    "payload enum collection element type mismatch",
                ));
            }
            if *remove == matches!(mode, mir::PayloadEnumUseMode::Borrow) {
                return Err(malformed_mir(
                    "payload enum collection transfer mode disagrees with removal",
                ));
            }
            validate_collection_element_access(
                program,
                function,
                local,
                collection_type,
                index,
                *remove,
                *positional,
            )
        }
        mir::PayloadEnumPlace::MixedPayload { mixed } => {
            let local = local_in(function, *mixed)?;
            if !matches!(local.ty, mir::Type::Mixed | mir::Type::NullableMixed) {
                return Err(malformed_mir(
                    "payload enum mixed projection uses a non-mixed local",
                ));
            }
            Ok(())
        }
    }
}

fn validate_shared_reference_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::SharedReferenceExpression,
) -> Result<(), BackendError> {
    let class = expression.class();
    class_in(program, class)?;
    match expression {
        mir::SharedReferenceExpression::New {
            class: expected,
            value,
        } => {
            if value.class() != *expected {
                return Err(malformed_mir(
                    "shared construction payload class does not match its handle",
                ));
            }
            validate_class_expression(program, function, value)?;
            require_owned_class_expression(value, "shared construction")
        }
        mir::SharedReferenceExpression::Local {
            local, transfer, ..
        } => {
            let definition = local_in(function, *local)?;
            if definition.ty != mir::Type::SharedReference(class) {
                return Err(malformed_mir(
                    "shared-reference local read has another handle type",
                ));
            }
            if *transfer && !definition.owned {
                return Err(malformed_mir(
                    "shared-reference move reads a borrowed local",
                ));
            }
            Ok(())
        }
        mir::SharedReferenceExpression::NullableLocalAssumeNonNull {
            local, transfer, ..
        } => {
            let definition = local_in(function, *local)?;
            if definition.ty != mir::Type::NullableSharedReference(class) {
                return Err(malformed_mir("nonnull shared read has another handle type"));
            }
            if *transfer && !definition.owned {
                return Err(malformed_mir("nonnull shared move reads a borrowed local"));
            }
            Ok(())
        }
        mir::SharedReferenceExpression::Property {
            object, property, ..
        } => {
            let object = local_in(function, *object)?;
            let mir::Type::Class(object_class) = object.ty else {
                return Err(malformed_mir(
                    "shared-reference property read uses a non-class object",
                ));
            };
            let property = property_in(program, object_class, *property)?;
            (property.ty == mir::Type::SharedReference(class))
                .then_some(())
                .ok_or_else(|| malformed_mir("shared-reference property has another type"))
        }
        mir::SharedReferenceExpression::Call {
            function: callee,
            args,
            ..
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type != mir::ReturnType::Value(mir::Type::SharedReference(class)) {
                return Err(malformed_mir(
                    "shared-reference call returns another handle type",
                ));
            }
            validate_call_args(program, function, callee, args)
        }
        mir::SharedReferenceExpression::Share { value, .. } => {
            if value.class() != class {
                return Err(malformed_mir("share changes payload class"));
            }
            validate_shared_reference_expression(program, function, value)
        }
        mir::SharedReferenceExpression::Coalesce {
            left,
            right,
            transfer,
            ..
        } => {
            if left.class() != class || right.class() != class {
                return Err(malformed_mir("shared coalesce changes payload class"));
            }
            if *transfer && (left.owned_temporary().is_none() || right.owned_temporary().is_none())
            {
                return Err(malformed_mir(
                    "shared coalesce operands must transfer owned handles",
                ));
            }
            validate_nullable_shared_reference_expression(program, function, left)?;
            validate_shared_reference_expression(program, function, right)
        }
        mir::SharedReferenceExpression::CollectionIndex {
            collection,
            index,
            remove,
            ..
        } => {
            let local = local_in(function, *collection)?;
            let mir::Type::Collection(collection) = local.ty else {
                return Err(malformed_mir(
                    "shared-reference index uses a non-collection local",
                ));
            };
            let definition = collection_in(program, collection)?;
            if definition.value != mir::Type::SharedReference(class) {
                return Err(malformed_mir(
                    "shared-reference index collection has another element type",
                ));
            }
            if *remove && !local.writable {
                return Err(malformed_mir(
                    "shared-reference removal uses a readonly collection",
                ));
            }
            validate_rvalue(program, function, index)
        }
    }
}

fn validate_weak_reference_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::WeakReferenceExpression,
) -> Result<(), BackendError> {
    let class = expression.class();
    class_in(program, class)?;
    match expression {
        mir::WeakReferenceExpression::Local {
            local, transfer, ..
        } => {
            let definition = local_in(function, *local)?;
            if definition.ty != mir::Type::WeakReference(class) {
                return Err(malformed_mir(
                    "weak-reference local read has another handle type",
                ));
            }
            if *transfer && !definition.owned {
                return Err(malformed_mir("weak-reference move reads a borrowed local"));
            }
            Ok(())
        }
        mir::WeakReferenceExpression::NullableLocalAssumeNonNull {
            local, transfer, ..
        } => {
            let definition = local_in(function, *local)?;
            if definition.ty != mir::Type::NullableWeakReference(class) {
                return Err(malformed_mir("nonnull weak read has another handle type"));
            }
            if *transfer && !definition.owned {
                return Err(malformed_mir("nonnull weak move reads a borrowed local"));
            }
            Ok(())
        }
        mir::WeakReferenceExpression::Property {
            object, property, ..
        } => {
            let object = local_in(function, *object)?;
            let mir::Type::Class(object_class) = object.ty else {
                return Err(malformed_mir(
                    "weak-reference property read uses a non-class object",
                ));
            };
            let property = property_in(program, object_class, *property)?;
            (property.ty == mir::Type::WeakReference(class))
                .then_some(())
                .ok_or_else(|| malformed_mir("weak-reference property has another type"))
        }
        mir::WeakReferenceExpression::Call {
            function: callee,
            args,
            ..
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type != mir::ReturnType::Value(mir::Type::WeakReference(class)) {
                return Err(malformed_mir(
                    "weak-reference call returns another handle type",
                ));
            }
            validate_call_args(program, function, callee, args)
        }
        mir::WeakReferenceExpression::Create { value, .. } => {
            if value.class() != class {
                return Err(malformed_mir(
                    "weak-reference creation changes payload class",
                ));
            }
            validate_shared_reference_expression(program, function, value)
        }
        mir::WeakReferenceExpression::Coalesce {
            left,
            right,
            transfer,
            ..
        } => {
            if left.class() != class || right.class() != class {
                return Err(malformed_mir("weak coalesce changes payload class"));
            }
            if *transfer && (left.owned_temporary().is_none() || right.owned_temporary().is_none())
            {
                return Err(malformed_mir(
                    "weak coalesce operands must transfer owned handles",
                ));
            }
            validate_nullable_weak_reference_expression(program, function, left)?;
            validate_weak_reference_expression(program, function, right)
        }
        mir::WeakReferenceExpression::CollectionIndex {
            collection,
            index,
            remove,
            ..
        } => {
            let local = local_in(function, *collection)?;
            let mir::Type::Collection(collection) = local.ty else {
                return Err(malformed_mir(
                    "weak-reference index uses a non-collection local",
                ));
            };
            let definition = collection_in(program, collection)?;
            if definition.value != mir::Type::WeakReference(class) {
                return Err(malformed_mir(
                    "weak-reference index collection has another element type",
                ));
            }
            if *remove && !local.writable {
                return Err(malformed_mir(
                    "weak-reference removal uses a readonly collection",
                ));
            }
            validate_rvalue(program, function, index)
        }
    }
}

fn validate_nullable_shared_reference_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::NullableSharedReferenceExpression,
) -> Result<(), BackendError> {
    let class = expression.class();
    class_in(program, class)?;
    match expression {
        mir::NullableSharedReferenceExpression::Null(_) => Ok(()),
        mir::NullableSharedReferenceExpression::Shared(value) => {
            if value.class() != class {
                return Err(malformed_mir("nullable shared value changes payload class"));
            }
            validate_shared_reference_expression(program, function, value)
        }
        mir::NullableSharedReferenceExpression::Local {
            local, transfer, ..
        } => {
            let definition = local_in(function, *local)?;
            if definition.ty != mir::Type::NullableSharedReference(class) {
                return Err(malformed_mir("nullable shared local read has another type"));
            }
            if *transfer && !definition.owned {
                return Err(malformed_mir("nullable shared move reads a borrowed local"));
            }
            Ok(())
        }
        mir::NullableSharedReferenceExpression::Property {
            object, property, ..
        } => {
            let object = local_in(function, *object)?;
            let mir::Type::Class(object_class) = object.ty else {
                return Err(malformed_mir(
                    "nullable shared property read uses a non-class object",
                ));
            };
            let property = property_in(program, object_class, *property)?;
            (property.ty == mir::Type::NullableSharedReference(class))
                .then_some(())
                .ok_or_else(|| malformed_mir("nullable shared property has another type"))
        }
        mir::NullableSharedReferenceExpression::Call {
            function: callee,
            args,
            ..
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type
                != mir::ReturnType::Value(mir::Type::NullableSharedReference(class))
            {
                return Err(malformed_mir(
                    "nullable shared call returns another handle type",
                ));
            }
            validate_call_args(program, function, callee, args)
        }
        mir::NullableSharedReferenceExpression::Acquire { value, .. } => {
            if value.class() != class {
                return Err(malformed_mir("weak acquisition changes payload class"));
            }
            validate_weak_reference_expression(program, function, value)
        }
        mir::NullableSharedReferenceExpression::NullSafeShare { value, .. } => {
            if value.class() != class {
                return Err(malformed_mir("null-safe share changes payload class"));
            }
            validate_nullable_shared_reference_expression(program, function, value)
        }
        mir::NullableSharedReferenceExpression::NullSafeAcquire { value, .. } => {
            if value.class() != class {
                return Err(malformed_mir(
                    "null-safe weak acquisition changes payload class",
                ));
            }
            validate_nullable_weak_reference_expression(program, function, value)
        }
        mir::NullableSharedReferenceExpression::Coalesce {
            left,
            right,
            transfer,
            ..
        } => {
            if left.class() != class || right.class() != class {
                return Err(malformed_mir(
                    "nullable shared coalesce changes payload class",
                ));
            }
            if *transfer
                && (!nullable_shared_transfer_source_is_owned(left)
                    || !nullable_shared_transfer_source_is_owned(right))
            {
                return Err(malformed_mir(
                    "nullable shared coalesce operands must transfer owned handles",
                ));
            }
            validate_nullable_shared_reference_expression(program, function, left)?;
            validate_nullable_shared_reference_expression(program, function, right)
        }
        mir::NullableSharedReferenceExpression::DictionaryGet {
            collection,
            key,
            access,
            stored_nullable,
            ..
        } => validate_dictionary_get(
            program,
            function,
            *collection,
            key,
            if *stored_nullable {
                mir::Type::NullableSharedReference(class)
            } else {
                mir::Type::SharedReference(class)
            },
            *access,
        ),
        mir::NullableSharedReferenceExpression::CollectionIndex {
            collection,
            index,
            remove,
            ..
        } => {
            let local = local_in(function, *collection)?;
            let mir::Type::Collection(collection) = local.ty else {
                return Err(malformed_mir(
                    "nullable shared index uses a non-collection local",
                ));
            };
            let definition = collection_in(program, collection)?;
            if definition.value != mir::Type::NullableSharedReference(class) {
                return Err(malformed_mir(
                    "nullable shared index collection has another element type",
                ));
            }
            if *remove && !local.writable {
                return Err(malformed_mir(
                    "nullable shared removal uses a readonly collection",
                ));
            }
            validate_rvalue(program, function, index)
        }
    }
}

fn validate_nullable_weak_reference_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::NullableWeakReferenceExpression,
) -> Result<(), BackendError> {
    let class = expression.class();
    class_in(program, class)?;
    match expression {
        mir::NullableWeakReferenceExpression::Null(_) => Ok(()),
        mir::NullableWeakReferenceExpression::Weak(value) => {
            if value.class() != class {
                return Err(malformed_mir("nullable weak value changes payload class"));
            }
            validate_weak_reference_expression(program, function, value)
        }
        mir::NullableWeakReferenceExpression::Local {
            local, transfer, ..
        } => {
            let definition = local_in(function, *local)?;
            if definition.ty != mir::Type::NullableWeakReference(class) {
                return Err(malformed_mir("nullable weak local read has another type"));
            }
            if *transfer && !definition.owned {
                return Err(malformed_mir("nullable weak move reads a borrowed local"));
            }
            Ok(())
        }
        mir::NullableWeakReferenceExpression::Property {
            object, property, ..
        } => {
            let object = local_in(function, *object)?;
            let mir::Type::Class(object_class) = object.ty else {
                return Err(malformed_mir(
                    "nullable weak property read uses a non-class object",
                ));
            };
            let property = property_in(program, object_class, *property)?;
            (property.ty == mir::Type::NullableWeakReference(class))
                .then_some(())
                .ok_or_else(|| malformed_mir("nullable weak property has another type"))
        }
        mir::NullableWeakReferenceExpression::Call {
            function: callee,
            args,
            ..
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type != mir::ReturnType::Value(mir::Type::NullableWeakReference(class))
            {
                return Err(malformed_mir(
                    "nullable weak call returns another handle type",
                ));
            }
            validate_call_args(program, function, callee, args)
        }
        mir::NullableWeakReferenceExpression::NullSafeCreate { value, .. } => {
            if value.class() != class {
                return Err(malformed_mir(
                    "null-safe weak creation changes payload class",
                ));
            }
            validate_nullable_shared_reference_expression(program, function, value)
        }
        mir::NullableWeakReferenceExpression::Coalesce {
            left,
            right,
            transfer,
            ..
        } => {
            if left.class() != class || right.class() != class {
                return Err(malformed_mir(
                    "nullable weak coalesce changes payload class",
                ));
            }
            if *transfer
                && (!nullable_weak_transfer_source_is_owned(left)
                    || !nullable_weak_transfer_source_is_owned(right))
            {
                return Err(malformed_mir(
                    "nullable weak coalesce operands must transfer owned handles",
                ));
            }
            validate_nullable_weak_reference_expression(program, function, left)?;
            validate_nullable_weak_reference_expression(program, function, right)
        }
        mir::NullableWeakReferenceExpression::DictionaryGet {
            collection,
            key,
            access,
            stored_nullable,
            ..
        } => validate_dictionary_get(
            program,
            function,
            *collection,
            key,
            if *stored_nullable {
                mir::Type::NullableWeakReference(class)
            } else {
                mir::Type::WeakReference(class)
            },
            *access,
        ),
        mir::NullableWeakReferenceExpression::CollectionIndex {
            collection,
            index,
            remove,
            ..
        } => {
            let local = local_in(function, *collection)?;
            let mir::Type::Collection(collection) = local.ty else {
                return Err(malformed_mir(
                    "nullable weak index uses a non-collection local",
                ));
            };
            let definition = collection_in(program, collection)?;
            if definition.value != mir::Type::NullableWeakReference(class) {
                return Err(malformed_mir(
                    "nullable weak index collection has another element type",
                ));
            }
            if *remove && !local.writable {
                return Err(malformed_mir(
                    "nullable weak removal uses a readonly collection",
                ));
            }
            validate_rvalue(program, function, index)
        }
    }
}

fn writable_payload_type(payload: mir::WritableSharedPayload) -> mir::Type {
    match payload {
        mir::WritableSharedPayload::Class(class) => mir::Type::Class(class),
        mir::WritableSharedPayload::Collection(collection) => mir::Type::Collection(collection),
    }
}

fn validate_writable_shared_property(
    program: &mir::Program,
    function: &mir::Function,
    object: mir::LocalId,
    property: crate::class_layout::PropertyId,
    expected: mir::Type,
) -> Result<(), BackendError> {
    let object = local_in(function, object)?;
    let mir::Type::Class(object_class) = object.ty else {
        return Err(malformed_mir(
            "writable shared property read uses a non-class object",
        ));
    };
    let property = property_in(program, object_class, property)?;
    (property.ty == expected)
        .then_some(())
        .ok_or_else(|| malformed_mir("writable shared property has another type"))
}

fn validate_writable_collection_index(
    program: &mir::Program,
    function: &mir::Function,
    collection: mir::LocalId,
    index: &mir::Rvalue,
    expected: mir::Type,
    remove: bool,
) -> Result<(), BackendError> {
    let local = local_in(function, collection)?;
    let mir::Type::Collection(collection_type) = local.ty else {
        return Err(malformed_mir(
            "writable shared index uses a non-collection local",
        ));
    };
    let definition = collection_in(program, collection_type)?;
    if definition.value != expected {
        return Err(malformed_mir(
            "writable shared index collection has another element type",
        ));
    }
    if remove && !local.writable {
        return Err(malformed_mir(
            "writable shared removal uses a readonly collection",
        ));
    }
    validate_rvalue(program, function, index)
}

fn validate_writable_shared_reference_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::WritableSharedReferenceExpression,
) -> Result<(), BackendError> {
    let payload = expression.payload();
    validate_writable_shared_payload(program, payload)?;
    match expression {
        mir::WritableSharedReferenceExpression::New { value, .. } => {
            if value.ty() != writable_payload_type(payload) {
                return Err(malformed_mir(
                    "writable shared construction payload does not match its handle",
                ));
            }
            validate_rvalue(program, function, value)
        }
        mir::WritableSharedReferenceExpression::Local {
            local, transfer, ..
        } => {
            let definition = local_in(function, *local)?;
            if definition.ty != mir::Type::WritableSharedReference(payload) {
                return Err(malformed_mir(
                    "writable shared local read has another handle type",
                ));
            }
            if *transfer && !definition.owned {
                return Err(malformed_mir("writable shared move reads a borrowed local"));
            }
            Ok(())
        }
        mir::WritableSharedReferenceExpression::NullableLocalAssumeNonNull {
            local,
            transfer,
            ..
        } => {
            let definition = local_in(function, *local)?;
            if definition.ty != mir::Type::NullableWritableSharedReference(payload) {
                return Err(malformed_mir(
                    "nonnull writable shared read has another handle type",
                ));
            }
            if *transfer && !definition.owned {
                return Err(malformed_mir(
                    "nonnull writable shared move reads a borrowed local",
                ));
            }
            Ok(())
        }
        mir::WritableSharedReferenceExpression::Property {
            object, property, ..
        } => validate_writable_shared_property(
            program,
            function,
            *object,
            *property,
            mir::Type::WritableSharedReference(payload),
        ),
        mir::WritableSharedReferenceExpression::Call {
            function: callee,
            args,
            ..
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type
                != mir::ReturnType::Value(mir::Type::WritableSharedReference(payload))
            {
                return Err(malformed_mir(
                    "writable shared call returns another handle type",
                ));
            }
            validate_call_args(program, function, callee, args)
        }
        mir::WritableSharedReferenceExpression::Share { value, .. } => {
            if value.payload() != payload {
                return Err(malformed_mir("writable share changes payload type"));
            }
            validate_writable_shared_reference_expression(program, function, value)
        }
        mir::WritableSharedReferenceExpression::Coalesce {
            left,
            right,
            transfer,
            ..
        } => {
            if left.payload() != payload || right.payload() != payload {
                return Err(malformed_mir(
                    "writable shared coalesce changes payload type",
                ));
            }
            if *transfer && (!left.owned_temporary() || !right.owned_temporary()) {
                return Err(malformed_mir(
                    "writable shared coalesce operands must transfer owned handles",
                ));
            }
            validate_nullable_writable_shared_reference_expression(program, function, left)?;
            validate_writable_shared_reference_expression(program, function, right)
        }
        mir::WritableSharedReferenceExpression::CollectionIndex {
            collection,
            index,
            remove,
            ..
        } => validate_writable_collection_index(
            program,
            function,
            *collection,
            index,
            mir::Type::WritableSharedReference(payload),
            *remove,
        ),
    }
}

fn validate_writable_weak_reference_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::WritableWeakReferenceExpression,
) -> Result<(), BackendError> {
    let payload = expression.payload();
    validate_writable_shared_payload(program, payload)?;
    match expression {
        mir::WritableWeakReferenceExpression::Local {
            local, transfer, ..
        } => {
            let definition = local_in(function, *local)?;
            if definition.ty != mir::Type::WritableWeakReference(payload) {
                return Err(malformed_mir(
                    "writable weak local read has another handle type",
                ));
            }
            if *transfer && !definition.owned {
                return Err(malformed_mir("writable weak move reads a borrowed local"));
            }
            Ok(())
        }
        mir::WritableWeakReferenceExpression::NullableLocalAssumeNonNull {
            local,
            transfer,
            ..
        } => {
            let definition = local_in(function, *local)?;
            if definition.ty != mir::Type::NullableWritableWeakReference(payload) {
                return Err(malformed_mir(
                    "nonnull writable weak read has another handle type",
                ));
            }
            if *transfer && !definition.owned {
                return Err(malformed_mir(
                    "nonnull writable weak move reads a borrowed local",
                ));
            }
            Ok(())
        }
        mir::WritableWeakReferenceExpression::Property {
            object, property, ..
        } => validate_writable_shared_property(
            program,
            function,
            *object,
            *property,
            mir::Type::WritableWeakReference(payload),
        ),
        mir::WritableWeakReferenceExpression::Call {
            function: callee,
            args,
            ..
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type
                != mir::ReturnType::Value(mir::Type::WritableWeakReference(payload))
            {
                return Err(malformed_mir(
                    "writable weak call returns another handle type",
                ));
            }
            validate_call_args(program, function, callee, args)
        }
        mir::WritableWeakReferenceExpression::Create { value, .. } => {
            if value.payload() != payload {
                return Err(malformed_mir("writable weak creation changes payload type"));
            }
            validate_writable_shared_reference_expression(program, function, value)
        }
        mir::WritableWeakReferenceExpression::Coalesce {
            left,
            right,
            transfer,
            ..
        } => {
            if left.payload() != payload || right.payload() != payload {
                return Err(malformed_mir("writable weak coalesce changes payload type"));
            }
            if *transfer && (!left.owned_temporary() || !right.owned_temporary()) {
                return Err(malformed_mir(
                    "writable weak coalesce operands must transfer owned handles",
                ));
            }
            validate_nullable_writable_weak_reference_expression(program, function, left)?;
            validate_writable_weak_reference_expression(program, function, right)
        }
        mir::WritableWeakReferenceExpression::CollectionIndex {
            collection,
            index,
            remove,
            ..
        } => validate_writable_collection_index(
            program,
            function,
            *collection,
            index,
            mir::Type::WritableWeakReference(payload),
            *remove,
        ),
    }
}

fn validate_nullable_writable_shared_reference_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::NullableWritableSharedReferenceExpression,
) -> Result<(), BackendError> {
    let payload = expression.payload();
    validate_writable_shared_payload(program, payload)?;
    match expression {
        mir::NullableWritableSharedReferenceExpression::Null(_) => Ok(()),
        mir::NullableWritableSharedReferenceExpression::Strong(value) => {
            validate_writable_shared_reference_expression(program, function, value)
        }
        mir::NullableWritableSharedReferenceExpression::Local {
            local, transfer, ..
        } => {
            let definition = local_in(function, *local)?;
            if definition.ty != mir::Type::NullableWritableSharedReference(payload) {
                return Err(malformed_mir(
                    "nullable writable shared local read has another type",
                ));
            }
            if *transfer && !definition.owned {
                return Err(malformed_mir(
                    "nullable writable shared move reads a borrowed local",
                ));
            }
            Ok(())
        }
        mir::NullableWritableSharedReferenceExpression::Property {
            object, property, ..
        } => validate_writable_shared_property(
            program,
            function,
            *object,
            *property,
            mir::Type::NullableWritableSharedReference(payload),
        ),
        mir::NullableWritableSharedReferenceExpression::Call {
            function: callee,
            args,
            ..
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type
                != mir::ReturnType::Value(mir::Type::NullableWritableSharedReference(payload))
            {
                return Err(malformed_mir(
                    "nullable writable shared call returns another handle type",
                ));
            }
            validate_call_args(program, function, callee, args)
        }
        mir::NullableWritableSharedReferenceExpression::Acquire { value, .. } => {
            if value.payload() != payload {
                return Err(malformed_mir(
                    "writable weak acquisition changes payload type",
                ));
            }
            validate_writable_weak_reference_expression(program, function, value)
        }
        mir::NullableWritableSharedReferenceExpression::NullSafeShare { value, .. } => {
            if value.payload() != payload {
                return Err(malformed_mir(
                    "null-safe writable share changes payload type",
                ));
            }
            validate_nullable_writable_shared_reference_expression(program, function, value)
        }
        mir::NullableWritableSharedReferenceExpression::NullSafeAcquire { value, .. } => {
            if value.payload() != payload {
                return Err(malformed_mir(
                    "null-safe writable acquisition changes payload type",
                ));
            }
            validate_nullable_writable_weak_reference_expression(program, function, value)
        }
        mir::NullableWritableSharedReferenceExpression::Coalesce {
            left,
            right,
            transfer,
            ..
        } => {
            if left.payload() != payload || right.payload() != payload {
                return Err(malformed_mir(
                    "nullable writable shared coalesce changes payload type",
                ));
            }
            if *transfer && (!left.owned_temporary() || !right.owned_temporary()) {
                return Err(malformed_mir(
                    "nullable writable shared coalesce operands must transfer owned handles",
                ));
            }
            validate_nullable_writable_shared_reference_expression(program, function, left)?;
            validate_nullable_writable_shared_reference_expression(program, function, right)
        }
        mir::NullableWritableSharedReferenceExpression::DictionaryGet {
            collection,
            key,
            access,
            stored_nullable,
            ..
        } => validate_dictionary_get(
            program,
            function,
            *collection,
            key,
            if *stored_nullable {
                mir::Type::NullableWritableSharedReference(payload)
            } else {
                mir::Type::WritableSharedReference(payload)
            },
            *access,
        ),
    }
}

fn validate_nullable_writable_weak_reference_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::NullableWritableWeakReferenceExpression,
) -> Result<(), BackendError> {
    let payload = expression.payload();
    validate_writable_shared_payload(program, payload)?;
    match expression {
        mir::NullableWritableWeakReferenceExpression::Null(_) => Ok(()),
        mir::NullableWritableWeakReferenceExpression::Weak(value) => {
            validate_writable_weak_reference_expression(program, function, value)
        }
        mir::NullableWritableWeakReferenceExpression::Local {
            local, transfer, ..
        } => {
            let definition = local_in(function, *local)?;
            if definition.ty != mir::Type::NullableWritableWeakReference(payload) {
                return Err(malformed_mir(
                    "nullable writable weak local read has another type",
                ));
            }
            if *transfer && !definition.owned {
                return Err(malformed_mir(
                    "nullable writable weak move reads a borrowed local",
                ));
            }
            Ok(())
        }
        mir::NullableWritableWeakReferenceExpression::Property {
            object, property, ..
        } => validate_writable_shared_property(
            program,
            function,
            *object,
            *property,
            mir::Type::NullableWritableWeakReference(payload),
        ),
        mir::NullableWritableWeakReferenceExpression::Call {
            function: callee,
            args,
            ..
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type
                != mir::ReturnType::Value(mir::Type::NullableWritableWeakReference(payload))
            {
                return Err(malformed_mir(
                    "nullable writable weak call returns another handle type",
                ));
            }
            validate_call_args(program, function, callee, args)
        }
        mir::NullableWritableWeakReferenceExpression::NullSafeCreate { value, .. } => {
            if value.payload() != payload {
                return Err(malformed_mir(
                    "null-safe writable weak creation changes payload type",
                ));
            }
            validate_nullable_writable_shared_reference_expression(program, function, value)
        }
        mir::NullableWritableWeakReferenceExpression::Coalesce {
            left,
            right,
            transfer,
            ..
        } => {
            if left.payload() != payload || right.payload() != payload {
                return Err(malformed_mir(
                    "nullable writable weak coalesce changes payload type",
                ));
            }
            if *transfer && (!left.owned_temporary() || !right.owned_temporary()) {
                return Err(malformed_mir(
                    "nullable writable weak coalesce operands must transfer owned handles",
                ));
            }
            validate_nullable_writable_weak_reference_expression(program, function, left)?;
            validate_nullable_writable_weak_reference_expression(program, function, right)
        }
        mir::NullableWritableWeakReferenceExpression::DictionaryGet {
            collection,
            key,
            access,
            stored_nullable,
            ..
        } => validate_dictionary_get(
            program,
            function,
            *collection,
            key,
            if *stored_nullable {
                mir::Type::NullableWritableWeakReference(payload)
            } else {
                mir::Type::WritableWeakReference(payload)
            },
            *access,
        ),
    }
}

fn validate_shared_reference_access_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::SharedReferenceAccessExpression,
) -> Result<(), BackendError> {
    let payload = expression.payload();
    validate_writable_shared_payload(program, payload)?;
    let access_type = expression.ty();
    match expression {
        mir::SharedReferenceAccessExpression::Local {
            local, transfer, ..
        } => {
            let definition = local_in(function, *local)?;
            if definition.ty != access_type {
                return Err(malformed_mir(
                    "shared access local read has another access type",
                ));
            }
            if *transfer && !definition.owned {
                return Err(malformed_mir("shared access move reads a borrowed local"));
            }
            Ok(())
        }
        mir::SharedReferenceAccessExpression::NullableLocalAssumeNonNull {
            local,
            transfer,
            ..
        } => {
            let definition = local_in(function, *local)?;
            let expected = if expression.writable() {
                mir::Type::NullableWritableSharedReferenceAccess(payload)
            } else {
                mir::Type::NullableReadonlySharedReferenceAccess(payload)
            };
            if definition.ty != expected {
                return Err(malformed_mir(
                    "shared access narrowing reads another nullable access type",
                ));
            }
            if *transfer && !definition.owned {
                return Err(malformed_mir(
                    "shared access narrowing moves a borrowed local",
                ));
            }
            Ok(())
        }
        mir::SharedReferenceAccessExpression::Property {
            object, property, ..
        } => validate_writable_shared_property(program, function, *object, *property, access_type),
        mir::SharedReferenceAccessExpression::CollectionIndex {
            collection,
            index,
            remove,
            ..
        } => validate_writable_collection_index(
            program,
            function,
            *collection,
            index,
            access_type,
            *remove,
        ),
        mir::SharedReferenceAccessExpression::Call {
            function: callee,
            args,
            ..
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type != mir::ReturnType::Value(access_type) {
                return Err(malformed_mir(
                    "shared access call returns another access type",
                ));
            }
            validate_call_args(program, function, callee, args)
        }
        mir::SharedReferenceAccessExpression::Acquire { value, .. } => {
            if value.payload() != payload {
                return Err(malformed_mir(
                    "shared access acquisition changes payload type",
                ));
            }
            validate_writable_shared_reference_expression(program, function, value)
        }
    }
}

fn validate_nullable_shared_reference_access_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::NullableSharedReferenceAccessExpression,
) -> Result<(), BackendError> {
    let payload = expression.payload();
    validate_writable_shared_payload(program, payload)?;
    let nullable_ty = expression.ty();
    match expression {
        mir::NullableSharedReferenceAccessExpression::Null { .. } => Ok(()),
        mir::NullableSharedReferenceAccessExpression::Access(value) => {
            if value.payload() != payload || value.writable() != expression.writable() {
                return Err(malformed_mir(
                    "nullable shared access wrapping changes access type",
                ));
            }
            validate_shared_reference_access_expression(program, function, value)
        }
        mir::NullableSharedReferenceAccessExpression::Local {
            local, transfer, ..
        } => {
            let definition = local_in(function, *local)?;
            if definition.ty != nullable_ty {
                return Err(malformed_mir(
                    "nullable shared access local read has another access type",
                ));
            }
            if *transfer && !definition.owned {
                return Err(malformed_mir(
                    "nullable shared access move reads a borrowed local",
                ));
            }
            Ok(())
        }
        mir::NullableSharedReferenceAccessExpression::Property {
            object, property, ..
        } => validate_writable_shared_property(program, function, *object, *property, nullable_ty),
        mir::NullableSharedReferenceAccessExpression::CollectionIndex {
            collection,
            index,
            remove,
            ..
        } => validate_writable_collection_index(
            program,
            function,
            *collection,
            index,
            nullable_ty,
            *remove,
        ),
        mir::NullableSharedReferenceAccessExpression::CollectionGet {
            collection,
            key,
            access,
            stored,
        } => validate_dictionary_get(
            program,
            function,
            *collection,
            key,
            stored.into_type(),
            *access,
        ),
        mir::NullableSharedReferenceAccessExpression::Call {
            function: callee,
            args,
            ..
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type != mir::ReturnType::Value(nullable_ty) {
                return Err(malformed_mir(
                    "nullable shared access call returns another access type",
                ));
            }
            validate_call_args(program, function, callee, args)
        }
        mir::NullableSharedReferenceAccessExpression::NullSafeAcquire { value, .. } => {
            if value.payload() != payload {
                return Err(malformed_mir(
                    "null-safe shared access acquisition changes payload type",
                ));
            }
            validate_nullable_writable_shared_reference_expression(program, function, value)
        }
    }
}

fn is_borrowed_mixed_expression(expression: &mir::MixedExpression) -> bool {
    expression.ownership() == mir::MixedOwnership::None
}

fn is_borrowed_nullable_mixed_expression(expression: &mir::NullableMixedExpression) -> bool {
    expression.ownership() == mir::MixedOwnership::None
}

fn validate_mixed_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::MixedExpression,
) -> Result<(), BackendError> {
    match expression {
        mir::MixedExpression::Local { local, transfer } => {
            let definition = local_in(function, *local)?;
            if !matches!(definition.ty, mir::Type::Mixed | mir::Type::NullableMixed) {
                return Err(malformed_mir(
                    "mixed expression references another local type",
                ));
            }
            if *transfer && (!definition.owned || definition.ty != mir::Type::Mixed) {
                return Err(malformed_mir(
                    "mixed expression transfers a non-owned mixed local",
                ));
            }
            Ok(())
        }
        mir::MixedExpression::Property { object, property } => {
            validate_property_operand(program, function, *object, *property, mir::Type::Mixed)
        }
        mir::MixedExpression::Call {
            function: callee,
            args,
            return_borrow,
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type != mir::ReturnType::Value(mir::Type::Mixed) {
                return Err(malformed_mir("mixed call targets a non-mixed function"));
            }
            if *return_borrow != infer_function_return_borrow(program, callee)? {
                return Err(malformed_mir(format!(
                    "mixed call disagrees with function {} return ownership",
                    callee.name
                )));
            }
            validate_call_args(program, function, callee, args)
        }
        mir::MixedExpression::BoxValue(value) => {
            validate_value_expression(program, function, value)
        }
        mir::MixedExpression::BoxString {
            value,
            payload_owned,
        } => {
            if !payload_owned && !value.is_borrowed_place() {
                return Err(malformed_mir(
                    "shell-only mixed string box uses an owning payload expression",
                ));
            }
            validate_string_expression(program, function, value)
        }
        mir::MixedExpression::BoxClass {
            value,
            payload_owned,
        } => {
            if !payload_owned && value.owned_temporary_class().is_some() {
                return Err(malformed_mir(
                    "shell-only mixed class box uses an owning payload expression",
                ));
            }
            validate_class_expression(program, function, value)
        }
        mir::MixedExpression::BoxError { value } => {
            validate_error_expression(program, function, value)?;
            if error_expression_is_borrowed(value) {
                return Err(malformed_mir("mixed box borrows an Error payload"));
            }
            Ok(())
        }
        mir::MixedExpression::BoxPayloadEnum { value } => {
            validate_payload_enum_expression(program, function, value)
        }
        mir::MixedExpression::BoxFunction {
            value,
            payload_owned,
        } => {
            validate_function_expression(program, function, value)?;
            if *payload_owned == mir::function_expression_is_borrowed(value) {
                return Err(malformed_mir(
                    "mixed function box ownership disagrees with its carrier expression",
                ));
            }
            Ok(())
        }
        mir::MixedExpression::CollectionIndex {
            positional,
            collection,
            index,
            transfer: _,
            remove: _,
        } => {
            let local = local_in(function, *collection)?;
            let mir::Type::Collection(collection_type) = local.ty else {
                return Err(malformed_mir("mixed index source is not a collection"));
            };
            let collection_type = collection_in(program, collection_type)?;
            if collection_type.value != mir::Type::Mixed {
                return Err(malformed_mir("mixed index element type mismatch"));
            }
            validate_collection_element_access(
                program,
                function,
                local,
                collection_type,
                index,
                false,
                *positional,
            )
        }
    }
}

fn validate_nullable_mixed_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::NullableMixedExpression,
) -> Result<(), BackendError> {
    match expression {
        mir::NullableMixedExpression::Null => Ok(()),
        mir::NullableMixedExpression::Mixed(value) => {
            validate_mixed_expression(program, function, value)
        }
        mir::NullableMixedExpression::BoxNullablePayloadEnum(value) => {
            validate_nullable_payload_enum_expression(program, function, value)
        }
        mir::NullableMixedExpression::Local { local, transfer } => {
            let definition = local_in(function, *local)?;
            if definition.ty != mir::Type::NullableMixed {
                return Err(malformed_mir(
                    "nullable mixed expression references another local type",
                ));
            }
            if *transfer && !definition.owned {
                return Err(malformed_mir(
                    "nullable mixed expression transfers a borrowed local",
                ));
            }
            Ok(())
        }
        mir::NullableMixedExpression::Property { object, property } => validate_property_operand(
            program,
            function,
            *object,
            *property,
            mir::Type::NullableMixed,
        ),
        mir::NullableMixedExpression::Call {
            function: callee,
            args,
            return_borrow,
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type != mir::ReturnType::Value(mir::Type::NullableMixed) {
                return Err(malformed_mir(
                    "nullable mixed call targets a non-nullable-mixed function",
                ));
            }
            if *return_borrow != infer_function_return_borrow(program, callee)? {
                return Err(malformed_mir(format!(
                    "nullable mixed call disagrees with function {} return ownership",
                    callee.name
                )));
            }
            validate_call_args(program, function, callee, args)
        }
        mir::NullableMixedExpression::Coalesce { left, right, .. } => {
            validate_nullable_mixed_expression(program, function, left)?;
            validate_nullable_mixed_expression(program, function, right)
        }
    }
}

fn validate_collection_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::CollectionExpression,
) -> Result<(), BackendError> {
    let definition = collection_in(program, expression.collection())?;
    match expression {
        mir::CollectionExpression::Local {
            local, transfer, ..
        } => {
            let local = local_in(function, *local)?;
            if !matches!(
                local.ty,
                mir::Type::Collection(found) | mir::Type::NullableCollection(found)
                    if found == definition.id
            ) {
                return Err(malformed_mir("collection local expression type mismatch"));
            }
            if *transfer && !local.owned {
                return Err(malformed_mir(format!(
                    "collection expression moves borrowed local{}",
                    local.id.0
                )));
            }
            Ok(())
        }
        mir::CollectionExpression::Literal { entries, .. } => {
            for entry in entries {
                match (definition.key, &entry.key) {
                    (Some(expected), Some(key)) if key.ty() == expected => {
                        validate_rvalue(program, function, key)?;
                    }
                    (None, None) => {}
                    _ => return Err(malformed_mir("collection literal key shape mismatch")),
                }
                if entry.value.ty() != definition.value {
                    return Err(malformed_mir("collection literal value type mismatch"));
                }
                validate_rvalue(program, function, &entry.value)?;
            }
            Ok(())
        }
        mir::CollectionExpression::Fill { value, count, .. } => {
            if !matches!(
                definition.kind,
                mir::CollectionKind::List | mir::CollectionKind::TypedArray
            ) || definition.key.is_some()
            {
                return Err(malformed_mir(
                    "collection fill destination is not a sequence",
                ));
            }
            if value.ty() != definition.value {
                return Err(malformed_mir("collection fill value type mismatch"));
            }
            if !matches!(definition.value, mir::Type::Scalar(_) | mir::Type::String) {
                return Err(malformed_mir(
                    "collection fill value is not a Copy scalar or string",
                ));
            }
            if count.ty() != IntegerType::Int64 {
                return Err(malformed_mir("collection fill count is not int"));
            }
            validate_rvalue(program, function, value)?;
            validate_integer_expression(program, function, count)
        }
        mir::CollectionExpression::Index {
            source,
            index,
            transfer,
            positional,
            ..
        } => {
            let source = local_in(function, *source)?;
            let mir::Type::Collection(source_type) = source.ty else {
                return Err(malformed_mir("collection index source is not a collection"));
            };
            let source_type = collection_in(program, source_type)?;
            if source_type.value != mir::Type::Collection(definition.id) {
                return Err(malformed_mir("nested collection index type mismatch"));
            }
            if *transfer && (source_type.kind != mir::CollectionKind::List || !source.writable) {
                return Err(malformed_mir(
                    "collection value transfer requires a writable List source",
                ));
            }
            validate_collection_index(program, function, source_type, index, *positional)
        }
        mir::CollectionExpression::Property {
            object, property, ..
        } => {
            let object = local_in(function, *object)?;
            let class = match object.ty {
                mir::Type::Class(class) | mir::Type::NullableClass(class) => class,
                _ => {
                    return Err(malformed_mir(
                        "collection property access uses a non-class local",
                    ));
                }
            };
            let property = property_in(program, class, *property)?;
            if property.ty != mir::Type::Collection(definition.id) {
                return Err(malformed_mir(
                    "collection property expression type mismatch",
                ));
            }
            Ok(())
        }
        mir::CollectionExpression::SharedAccessPayload {
            access, writable, ..
        } => {
            let expected = if *writable {
                mir::Type::WritableSharedReferenceAccess(mir::WritableSharedPayload::Collection(
                    definition.id,
                ))
            } else {
                mir::Type::ReadonlySharedReferenceAccess(mir::WritableSharedPayload::Collection(
                    definition.id,
                ))
            };
            if local_in(function, *access)?.ty != expected {
                return Err(malformed_mir(
                    "collection access payload projection type mismatch",
                ));
            }
            Ok(())
        }
        mir::CollectionExpression::From {
            source,
            transfer,
            algebra,
            ..
        } => {
            let source = local_in(function, *source)?;
            let mir::Type::Collection(source_type) = source.ty else {
                return Err(malformed_mir("Set::from source is not a collection"));
            };
            let source_type = collection_in(program, source_type)?;
            let source_matches = if algebra.is_some() {
                source_type == definition && definition.kind.is_set()
            } else {
                match definition.kind {
                    mir::CollectionKind::Set
                    | mir::CollectionKind::SortedSet
                    | mir::CollectionKind::PriorityQueue
                    | mir::CollectionKind::Deque => {
                        matches!(
                            source_type.kind,
                            mir::CollectionKind::TypedArray | mir::CollectionKind::List
                        ) && source_type.key.is_none()
                            && source_type.value == definition.value
                    }
                    mir::CollectionKind::SortedDictionary => {
                        source_type.kind == mir::CollectionKind::Dictionary
                            && source_type.key == definition.key
                            && source_type.value == definition.value
                    }
                    _ => false,
                }
            };
            if !source_matches {
                return Err(malformed_mir("collection conversion type mismatch"));
            }
            if *transfer {
                return Err(malformed_mir(
                    "non-consuming collection construction must borrow its source collection",
                ));
            }
            if !collection_type_is_copy(definition.value)
                || definition
                    .key
                    .is_some_and(|key| !collection_type_is_copy(key))
            {
                return Err(malformed_mir(
                    "non-consuming collection construction uses non-Copy values",
                ));
            }
            if let Some((_, other)) = algebra {
                if !matches!(
                    definition.kind,
                    mir::CollectionKind::Set | mir::CollectionKind::SortedSet
                ) {
                    return Err(malformed_mir("set algebra uses another collection kind"));
                }
                let other = local_in(function, *other)?;
                let mir::Type::Collection(other_type) = other.ty else {
                    return Err(malformed_mir("set algebra operand is not a collection"));
                };
                if collection_in(program, other_type)? != definition {
                    return Err(malformed_mir("set algebra collection type mismatch"));
                }
            }
            Ok(())
        }
        mir::CollectionExpression::FromBytes { source, .. } => {
            if definition.kind != mir::CollectionKind::TypedArray
                || definition.value
                    != mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::UInt8))
            {
                return Err(malformed_mir("Bytes::toArray destination is not uint8[]"));
            }
            validate_bytes_local(program, function, *source)
        }
        mir::CollectionExpression::BytesFromArray { source, .. } => {
            if definition.kind != mir::CollectionKind::Bytes {
                return Err(malformed_mir("Bytes::fromArray destination is not Bytes"));
            }
            let source = local_in(function, *source)?;
            let mir::Type::Collection(source_type) = source.ty else {
                return Err(malformed_mir("Bytes::fromArray source is not a collection"));
            };
            let source_type = collection_in(program, source_type)?;
            if source_type.kind != mir::CollectionKind::TypedArray
                || source_type.value
                    != mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::UInt8))
            {
                return Err(malformed_mir("Bytes::fromArray source is not uint8[]"));
            }
            Ok(())
        }
        mir::CollectionExpression::ReadFileBytes { path, .. } => {
            if definition.kind != mir::CollectionKind::Bytes {
                return Err(malformed_mir("read_file_bytes result is not Bytes"));
            }
            validate_string_expression(program, function, path)
        }
        mir::CollectionExpression::ReadStdinBytes { .. } => {
            if definition.kind != mir::CollectionKind::Bytes {
                return Err(malformed_mir("read_stdin_bytes result is not Bytes"));
            }
            Ok(())
        }
        mir::CollectionExpression::StringIntrinsic(call) => validate_string_intrinsic(
            program,
            function,
            call,
            mir::Type::Collection(definition.id),
        ),
        mir::CollectionExpression::Call {
            function: callee,
            args,
            return_borrow,
            ..
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type != mir::ReturnType::Value(mir::Type::Collection(definition.id)) {
                return Err(malformed_mir(
                    "collection call targets a function with another return type",
                ));
            }
            if *return_borrow != infer_function_return_borrow(program, callee)? {
                return Err(malformed_mir(format!(
                    "collection call disagrees with function {} return ownership",
                    callee.name
                )));
            }
            validate_call_args(program, function, callee, args)
        }
    }
}

fn validate_nullable_collection_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::NullableCollectionExpression,
) -> Result<(), BackendError> {
    let collection = expression.collection();
    collection_in(program, collection)?;
    match expression {
        mir::NullableCollectionExpression::Null(_) => Ok(()),
        mir::NullableCollectionExpression::Collection(value) => {
            if value.collection() != collection {
                return Err(malformed_mir(
                    "nullable collection wraps another collection type",
                ));
            }
            validate_collection_expression(program, function, value)
        }
        mir::NullableCollectionExpression::Local {
            local, transfer, ..
        } => {
            let local = local_in(function, *local)?;
            if local.ty != mir::Type::NullableCollection(collection) {
                return Err(malformed_mir(
                    "nullable collection local expression type mismatch",
                ));
            }
            if *transfer && !local.owned {
                return Err(malformed_mir(format!(
                    "nullable collection expression moves borrowed local{}",
                    local.id.0
                )));
            }
            Ok(())
        }
        mir::NullableCollectionExpression::Property {
            object, property, ..
        } => {
            let object = local_in(function, *object)?;
            let class = match object.ty {
                mir::Type::Class(class) | mir::Type::NullableClass(class) => class,
                _ => {
                    return Err(malformed_mir(
                        "nullable collection property access uses a non-class local",
                    ));
                }
            };
            if property_in(program, class, *property)?.ty
                != mir::Type::NullableCollection(collection)
            {
                return Err(malformed_mir(
                    "nullable collection property expression type mismatch",
                ));
            }
            Ok(())
        }
        mir::NullableCollectionExpression::Call {
            function: callee,
            args,
            return_borrow,
            ..
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type
                != mir::ReturnType::Value(mir::Type::NullableCollection(collection))
            {
                return Err(malformed_mir(
                    "nullable collection call targets a function with another return type",
                ));
            }
            if *return_borrow != infer_function_return_borrow(program, callee)? {
                return Err(malformed_mir(format!(
                    "nullable collection call disagrees with function {} return ownership",
                    callee.name
                )));
            }
            validate_call_args(program, function, callee, args)
        }
        mir::NullableCollectionExpression::Coalesce {
            left,
            right,
            collection: nested,
            ..
        } => {
            if *nested != collection {
                return Err(malformed_mir("nullable collection coalesce type mismatch"));
            }
            validate_nullable_collection_expression(program, function, left)?;
            validate_nullable_collection_expression(program, function, right)
        }
    }
}

fn validate_string_intrinsic(
    program: &mir::Program,
    function: &mir::Function,
    call: &mir::StringIntrinsicCall,
    expected_result: mir::Type,
) -> Result<(), BackendError> {
    use mir::StringIntrinsicKind as Kind;

    if call.result != expected_result {
        return Err(malformed_mir(format!(
            "String {} result is {}, expected {expected_result}",
            call.kind, call.result
        )));
    }
    for argument in &call.args {
        validate_rvalue(program, function, argument)?;
    }

    let int = mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64));
    let nullable_int = mir::Type::NullableScalar(mir::ScalarType::Integer(IntegerType::Int64));
    let bool_ty = mir::Type::Scalar(mir::ScalarType::Bool);
    let string = mir::Type::String;

    let exact = |expected: &[mir::Type]| {
        if call.args.len() != expected.len() {
            return Err(malformed_mir(format!(
                "String {} expects {} arguments, got {}",
                call.kind,
                expected.len(),
                call.args.len()
            )));
        }
        for (index, (argument, expected)) in call.args.iter().zip(expected).enumerate() {
            if argument.ty() != *expected {
                return Err(malformed_mir(format!(
                    "String {} argument {} has type {}, expected {expected}",
                    call.kind,
                    index + 1,
                    argument.ty()
                )));
            }
        }
        Ok(())
    };

    match call.kind {
        Kind::GraphemeLength | Kind::ByteLength => {
            exact(&[string])?;
            require_string_intrinsic_result(call, int)
        }
        Kind::IsEmpty => {
            exact(&[string])?;
            require_string_intrinsic_result(call, bool_ty)
        }
        Kind::ToBytes => {
            exact(&[string])?;
            require_collection_shape(program, call.result, mir::CollectionKind::Bytes, None, {
                mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::UInt8))
            })
        }
        Kind::Trim
        | Kind::TrimStart
        | Kind::TrimEnd
        | Kind::Lower
        | Kind::Upper
        | Kind::LowerFirst
        | Kind::UpperFirst => {
            exact(&[string])?;
            require_string_intrinsic_result(call, string)
        }
        Kind::Contains
        | Kind::StartsWith
        | Kind::EndsWith
        | Kind::ContainsIgnoreCase
        | Kind::StartsWithIgnoreCase
        | Kind::EndsWithIgnoreCase
        | Kind::EqualsIgnoreCase => {
            exact(&[string, string])?;
            require_string_intrinsic_result(call, bool_ty)
        }
        Kind::IndexOf
        | Kind::LastIndexOf
        | Kind::IndexOfIgnoreCase
        | Kind::LastIndexOfIgnoreCase => {
            exact(&[string, string])?;
            require_string_intrinsic_result(call, nullable_int)
        }
        Kind::CountOccurrences => {
            exact(&[string, string])?;
            require_string_intrinsic_result(call, int)
        }
        Kind::Replace => {
            exact(&[string, string, string])?;
            require_string_intrinsic_result(call, string)
        }
        Kind::Split => {
            exact(&[string, string])?;
            require_collection_shape(
                program,
                call.result,
                mir::CollectionKind::List,
                None,
                string,
            )
        }
        Kind::Join => {
            if call.args.len() != 2 || call.args[0].ty() != string {
                return Err(malformed_mir(
                    "String join expects separator string and List<string>",
                ));
            }
            require_collection_shape(
                program,
                call.args[1].ty(),
                mir::CollectionKind::List,
                None,
                string,
            )?;
            require_borrowed_collection_argument(&call.args[1], "String join")?;
            require_string_intrinsic_result(call, string)
        }
        Kind::Slice => {
            exact(&[string, int, nullable_int])?;
            require_string_intrinsic_result(call, string)
        }
        Kind::Repeat => {
            exact(&[string, int])?;
            require_string_intrinsic_result(call, string)
        }
        Kind::PadStart | Kind::PadEnd => {
            exact(&[string, int, string])?;
            require_string_intrinsic_result(call, string)
        }
        Kind::FromBytes => {
            if call.args.len() != 1 {
                return Err(malformed_mir(format!(
                    "String fromBytes expects 1 argument, got {}",
                    call.args.len()
                )));
            }
            require_collection_shape(
                program,
                call.args[0].ty(),
                mir::CollectionKind::Bytes,
                None,
                mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::UInt8)),
            )?;
            require_borrowed_collection_argument(&call.args[0], "String fromBytes")?;
            require_string_intrinsic_result(call, mir::Type::NullableString)
        }
    }
}

fn require_string_intrinsic_result(
    call: &mir::StringIntrinsicCall,
    expected: mir::Type,
) -> Result<(), BackendError> {
    (call.result == expected).then_some(()).ok_or_else(|| {
        malformed_mir(format!(
            "String {} has result {}, expected {expected}",
            call.kind, call.result
        ))
    })
}

fn require_collection_shape(
    program: &mir::Program,
    ty: mir::Type,
    kind: mir::CollectionKind,
    key: Option<mir::Type>,
    value: mir::Type,
) -> Result<(), BackendError> {
    let mir::Type::Collection(id) = ty else {
        return Err(malformed_mir(
            "String intrinsic collection type is not a collection",
        ));
    };
    let collection = collection_in(program, id)?;
    if collection.kind != kind || collection.key != key || collection.value != value {
        return Err(malformed_mir(
            "String intrinsic collection argument or result has the wrong shape",
        ));
    }
    Ok(())
}

fn require_borrowed_collection_argument(
    value: &mir::Rvalue,
    operation: &str,
) -> Result<(), BackendError> {
    let mir::Rvalue::Collection(collection) = value else {
        return Err(malformed_mir(format!(
            "{operation} collection argument has another representation"
        )));
    };
    if matches!(
        collection,
        mir::CollectionExpression::Local { transfer: true, .. }
            | mir::CollectionExpression::Index { transfer: true, .. }
    ) {
        return Err(malformed_mir(format!(
            "{operation} consumes a readonly borrowed collection argument"
        )));
    }
    Ok(())
}

fn validate_collection_borrow_writability(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::CollectionExpression,
    writable: bool,
) -> Result<(), BackendError> {
    if !writable {
        return Ok(());
    }

    let source_is_writable = match expression {
        mir::CollectionExpression::Local {
            local,
            transfer: false,
            ..
        } => local_in(function, *local)?.writable,
        mir::CollectionExpression::Index {
            source,
            transfer: false,
            ..
        } => local_in(function, *source)?.writable,
        mir::CollectionExpression::Property {
            object, property, ..
        } => {
            let object = local_in(function, *object)?;
            let class = match object.ty {
                mir::Type::Class(class) | mir::Type::NullableClass(class) => class,
                _ => {
                    return Err(malformed_mir(
                        "collection property borrow uses a non-class local",
                    ));
                }
            };
            object.writable && property_in(program, class, *property)?.writable
        }
        mir::CollectionExpression::SharedAccessPayload { writable, .. } => *writable,
        _ => false,
    };

    if !source_is_writable {
        return Err(malformed_mir(
            "writable collection borrow requires a writable source place",
        ));
    }
    Ok(())
}

fn validate_bytes_local(
    program: &mir::Program,
    function: &mir::Function,
    local: mir::LocalId,
) -> Result<(), BackendError> {
    let local = local_in(function, local)?;
    let mir::Type::Collection(collection) = local.ty else {
        return Err(malformed_mir("Bytes operation uses a non-collection local"));
    };
    if collection_in(program, collection)?.kind != mir::CollectionKind::Bytes {
        return Err(malformed_mir(
            "Bytes operation uses another collection kind",
        ));
    }
    Ok(())
}

fn validate_collection_index(
    program: &mir::Program,
    function: &mir::Function,
    collection: &mir::CollectionType,
    index: &mir::Rvalue,
    positional: bool,
) -> Result<(), BackendError> {
    // A positional index addresses a slot, so it is an offset even when the
    // collection is keyed. Requiring the key type here would reject the only
    // form that reads an element without searching for it.
    let expected = if positional {
        mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64))
    } else {
        collection
            .key
            .unwrap_or(mir::Type::Scalar(mir::ScalarType::Integer(
                IntegerType::Int64,
            )))
    };
    if index.ty() != expected {
        return Err(malformed_mir(format!(
            "collection index has type {}, expected {expected}",
            index.ty()
        )));
    }
    validate_rvalue(program, function, index)
}

fn validate_collection_element_access(
    program: &mir::Program,
    function: &mir::Function,
    local: &mir::Local,
    collection: &mir::CollectionType,
    index: &mir::Rvalue,
    remove: bool,
    positional: bool,
) -> Result<(), BackendError> {
    if remove && (collection.kind != mir::CollectionKind::List || !local.writable) {
        return Err(malformed_mir(
            "element removal requires a writable List source",
        ));
    }
    validate_collection_index(program, function, collection, index, positional)
}

fn validate_collection_key_at(
    program: &mir::Program,
    function: &mir::Function,
    local: mir::LocalId,
    expected: mir::Type,
    offset: &mir::Rvalue,
) -> Result<(), BackendError> {
    let definition = local_in(function, local)?;
    let mir::Type::Collection(collection) = definition.ty else {
        return Err(malformed_mir(
            "collection key access uses a non-collection local",
        ));
    };
    if collection_in(program, collection)?.key != Some(expected) {
        return Err(malformed_mir("collection key access type mismatch"));
    }
    if offset.ty() != mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64)) {
        return Err(malformed_mir("collection key offset is not int/int64"));
    }
    validate_rvalue(program, function, offset)
}

fn validate_dictionary_get(
    program: &mir::Program,
    function: &mir::Function,
    local: mir::LocalId,
    key: &mir::Rvalue,
    expected: mir::Type,
    access: mir::NullableCollectionAccess,
) -> Result<(), BackendError> {
    let definition = local_in(function, local)?;
    let mir::Type::Collection(collection) = definition.ty else {
        return Err(malformed_mir("Dictionary::get uses a non-collection local"));
    };
    let collection = collection_in(program, collection)?;
    let kind_matches = match access {
        mir::NullableCollectionAccess::Get
        | mir::NullableCollectionAccess::Index
        | mir::NullableCollectionAccess::Remove => collection.kind.is_dictionary(),
        mir::NullableCollectionAccess::First | mir::NullableCollectionAccess::Last => matches!(
            collection.kind,
            mir::CollectionKind::List
                | mir::CollectionKind::Set
                | mir::CollectionKind::SortedSet
                | mir::CollectionKind::PriorityQueue
                | mir::CollectionKind::Deque
        ),
        mir::NullableCollectionAccess::Pop => matches!(
            collection.kind,
            mir::CollectionKind::List | mir::CollectionKind::PriorityQueue
        ),
        mir::NullableCollectionAccess::PopFront | mir::NullableCollectionAccess::PopBack => {
            collection.kind == mir::CollectionKind::Deque
        }
        mir::NullableCollectionAccess::At => collection.kind.supports_foreach(),
    };
    if !kind_matches
        || (collection.value != expected
            && crate::native_abi::nullable_payload_type(collection.value) != Some(expected))
    {
        return Err(malformed_mir("nullable collection access type mismatch"));
    }
    let expected_key = match access {
        mir::NullableCollectionAccess::Get
        | mir::NullableCollectionAccess::Index
        | mir::NullableCollectionAccess::Remove => collection.key,
        mir::NullableCollectionAccess::First
        | mir::NullableCollectionAccess::Last
        | mir::NullableCollectionAccess::Pop
        | mir::NullableCollectionAccess::PopFront
        | mir::NullableCollectionAccess::PopBack
        | mir::NullableCollectionAccess::At => Some(mir::Type::Scalar(mir::ScalarType::Integer(
            IntegerType::Int64,
        ))),
    };
    if expected_key != Some(key.ty()) {
        return Err(malformed_mir(
            "nullable collection access key type mismatch",
        ));
    }
    if matches!(
        access,
        mir::NullableCollectionAccess::Remove
            | mir::NullableCollectionAccess::Pop
            | mir::NullableCollectionAccess::PopFront
            | mir::NullableCollectionAccess::PopBack
    ) && !definition.writable
    {
        return Err(malformed_mir(
            "mutating nullable collection access uses a readonly local",
        ));
    }
    validate_rvalue(program, function, key)
}

fn require_owned_class_expression(
    expression: &mir::ClassExpression,
    destination: &str,
) -> Result<(), BackendError> {
    match expression {
        mir::ClassExpression::Local { transfer: true, .. }
        | mir::ClassExpression::Call {
            return_borrow: None,
            ..
        }
        | mir::ClassExpression::New { .. }
        | mir::ClassExpression::NullableLocalAssumeNonNull { transfer: true, .. } => Ok(()),
        mir::ClassExpression::Local {
            local,
            transfer: false,
            ..
        } => Err(malformed_mir(format!(
            "{destination} receives borrowed class local local{}",
            local.0
        ))),
        mir::ClassExpression::Property { property, .. } => Err(malformed_mir(format!(
            "{destination} receives borrowed class property{}",
            property.index
        ))),
        mir::ClassExpression::Call {
            return_borrow: Some(_),
            ..
        } => Err(malformed_mir(format!(
            "{destination} receives a borrowed class call result"
        ))),
        mir::ClassExpression::NullableLocalAssumeNonNull {
            local,
            transfer: false,
            ..
        } => Err(malformed_mir(format!(
            "{destination} receives borrowed nullable class local local{}",
            local.0
        ))),
        mir::ClassExpression::Coalesce {
            left,
            right,
            transfer: true,
            ..
        } => {
            require_owned_nullable_class_expression(left, destination)?;
            require_owned_class_expression(right, destination)
        }
        mir::ClassExpression::Coalesce {
            transfer: false, ..
        } => Err(malformed_mir(format!(
            "{destination} receives a borrowed class coalesce result"
        ))),
        mir::ClassExpression::CollectionIndex { transfer: true, .. } => Ok(()),
        mir::ClassExpression::CollectionIndex {
            transfer: false, ..
        } => Err(malformed_mir(format!(
            "{destination} receives a borrowed indexed class value"
        ))),
        mir::ClassExpression::MixedPayload { transfer: true, .. } => Ok(()),
        mir::ClassExpression::MixedPayload {
            transfer: false, ..
        } => Err(malformed_mir(format!(
            "{destination} receives a borrowed mixed class payload"
        ))),
        mir::ClassExpression::SharedPayload { .. } => Err(malformed_mir(format!(
            "{destination} receives a borrowed shared-reference payload"
        ))),
        mir::ClassExpression::SharedAccessPayload { .. } => Err(malformed_mir(format!(
            "{destination} receives a borrowed shared-access payload"
        ))),
    }
}

fn require_owned_nullable_class_expression(
    expression: &mir::NullableClassExpression,
    destination: &str,
) -> Result<(), BackendError> {
    match expression {
        mir::NullableClassExpression::Null(_) => Ok(()),
        mir::NullableClassExpression::Class(value) => {
            require_owned_class_expression(value, destination)
        }
        mir::NullableClassExpression::SharedPayload { .. } => Err(malformed_mir(format!(
            "{destination} receives a borrowed nullable shared-reference payload"
        ))),
        mir::NullableClassExpression::Call {
            return_borrow: None,
            ..
        }
        | mir::NullableClassExpression::NullSafeCall {
            return_borrow: None,
            ..
        }
        | mir::NullableClassExpression::Local { transfer: true, .. } => Ok(()),
        mir::NullableClassExpression::Local {
            local,
            transfer: false,
            ..
        } => Err(malformed_mir(format!(
            "{destination} receives borrowed nullable class local local{}",
            local.0
        ))),
        mir::NullableClassExpression::Property { property, .. }
        | mir::NullableClassExpression::NullSafeProperty { property, .. } => {
            Err(malformed_mir(format!(
                "{destination} receives borrowed nullable class property{}",
                property.index
            )))
        }
        mir::NullableClassExpression::Call {
            return_borrow: Some(_),
            ..
        }
        | mir::NullableClassExpression::NullSafeCall {
            return_borrow: Some(_),
            ..
        } => Err(malformed_mir(format!(
            "{destination} receives a borrowed nullable class call result"
        ))),
        mir::NullableClassExpression::Coalesce {
            left,
            right,
            transfer: true,
            ..
        } => {
            require_owned_nullable_class_expression(left, destination)?;
            require_owned_nullable_class_expression(right, destination)
        }
        mir::NullableClassExpression::Coalesce {
            transfer: false, ..
        } => Err(malformed_mir(format!(
            "{destination} receives a borrowed nullable class coalesce result"
        ))),
        mir::NullableClassExpression::DictionaryGet {
            access:
                mir::NullableCollectionAccess::Remove
                | mir::NullableCollectionAccess::Pop
                | mir::NullableCollectionAccess::PopFront
                | mir::NullableCollectionAccess::PopBack,
            ..
        } => Ok(()),
        mir::NullableClassExpression::DictionaryGet { .. } => Err(malformed_mir(format!(
            "{destination} receives a borrowed nullable collection result"
        ))),
    }
}

fn infer_function_return_borrow(
    program: &mir::Program,
    function: &mir::Function,
) -> Result<Option<mir::ReturnBorrow>, BackendError> {
    let mut inferred: Option<Option<mir::ReturnBorrow>> = None;
    let (reachable, _) = reachable_blocks_and_predecessors(function, true)?;
    for block in function.blocks.iter().filter(|block| reachable[block.id.0]) {
        let candidate = match &block.terminator {
            mir::Terminator::Return(mir::Rvalue::Class(expression)) => Some(
                infer_expression_return_borrow(program, function, expression)?,
            ),
            mir::Terminator::Return(mir::Rvalue::NullableClass(
                mir::NullableClassExpression::Null(_),
            )) => None,
            mir::Terminator::Return(mir::Rvalue::NullableClass(expression)) => Some(
                infer_nullable_expression_return_borrow(program, function, expression)?,
            ),
            mir::Terminator::Return(mir::Rvalue::Collection(expression)) => Some(
                infer_collection_expression_return_borrow(program, function, expression)?,
            ),
            mir::Terminator::Return(mir::Rvalue::Mixed(expression)) => Some(
                infer_mixed_expression_return_borrow(program, function, expression)?,
            ),
            mir::Terminator::Return(mir::Rvalue::NullableMixed(
                mir::NullableMixedExpression::Null,
            )) => None,
            mir::Terminator::Return(mir::Rvalue::NullableMixed(expression)) => Some(
                infer_nullable_mixed_expression_return_borrow(program, function, expression)?,
            ),
            mir::Terminator::Return(mir::Rvalue::Function(expression)) => Some(
                infer_function_expression_return_borrow(program, function, expression)?,
            ),
            mir::Terminator::Return(mir::Rvalue::NullableFunction(
                mir::NullableFunctionExpression::Null { .. },
            )) => None,
            mir::Terminator::Return(mir::Rvalue::NullableFunction(expression)) => Some(
                infer_nullable_function_expression_return_borrow(program, function, expression)?,
            ),
            _ => continue,
        };
        let Some(candidate) = candidate else {
            continue;
        };
        match (inferred.as_mut(), candidate) {
            (None, candidate) => inferred = Some(candidate),
            (Some(Some(existing)), Some(candidate)) if existing.source == candidate.source => {
                existing.writable &= candidate.writable;
            }
            (Some(None), None) => {}
            _ => {
                return Err(malformed_mir(format!(
                    "function {} mixes owned and borrowed move-value returns",
                    function.name
                )));
            }
        }
    }
    Ok(inferred.flatten())
}

fn infer_function_expression_return_borrow(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::FunctionExpression,
) -> Result<Option<mir::ReturnBorrow>, BackendError> {
    match expression {
        mir::FunctionExpression::Create { captures, .. } => {
            let mut inferred = None;
            for capture in captures {
                let mir::ClosureCaptureOperand::BorrowLocal { local, writable } = capture else {
                    continue;
                };
                let candidate =
                    infer_local_return_borrow(program, function, *local)?.map(|borrow| {
                        mir::ReturnBorrow {
                            writable: borrow.writable && *writable,
                            ..borrow
                        }
                    });
                merge_function_value_borrow(&mut inferred, candidate, "closure construction")?;
            }
            Ok(inferred.flatten())
        }
        mir::FunctionExpression::Local { local, .. } => {
            infer_function_local_return_borrow(program, function, *local)
        }
        mir::FunctionExpression::Call {
            function: callee,
            args,
            return_borrow: Some(return_borrow),
            ..
        } => infer_borrowed_rvalue_source(program, function, *callee, args, *return_borrow),
        mir::FunctionExpression::AssumePresent { value, .. } => {
            infer_nullable_function_expression_return_borrow(program, function, value)
        }
        mir::FunctionExpression::Property { object, .. } => {
            infer_local_return_borrow(program, function, *object)
        }
        mir::FunctionExpression::CollectionIndex { collection, .. } => {
            infer_local_return_borrow(program, function, *collection)
        }
        mir::FunctionExpression::MixedPayload {
            mixed,
            transfer: false,
            ..
        } => infer_local_return_borrow(program, function, *mixed),
        mir::FunctionExpression::MixedPayload { transfer: true, .. } => Ok(None),
        mir::FunctionExpression::Call {
            return_borrow: None,
            ..
        } => Ok(None),
    }
}

fn infer_nullable_function_expression_return_borrow(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::NullableFunctionExpression,
) -> Result<Option<mir::ReturnBorrow>, BackendError> {
    match expression {
        mir::NullableFunctionExpression::Null { .. } => Ok(None),
        mir::NullableFunctionExpression::Present(value) => {
            infer_function_expression_return_borrow(program, function, value)
        }
        mir::NullableFunctionExpression::Local { local, .. } => {
            infer_function_local_return_borrow(program, function, *local)
        }
        mir::NullableFunctionExpression::Call {
            function: callee,
            args,
            return_borrow: Some(return_borrow),
            ..
        } => infer_borrowed_rvalue_source(program, function, *callee, args, *return_borrow),
        mir::NullableFunctionExpression::Property { object, .. } => {
            infer_local_return_borrow(program, function, *object)
        }
        mir::NullableFunctionExpression::DictionaryGet { collection, .. }
        | mir::NullableFunctionExpression::CollectionIndex { collection, .. } => {
            infer_local_return_borrow(program, function, *collection)
        }
        mir::NullableFunctionExpression::Call {
            return_borrow: None,
            ..
        } => Ok(None),
    }
}

fn infer_function_local_return_borrow(
    program: &mir::Program,
    function: &mir::Function,
    local: mir::LocalId,
) -> Result<Option<mir::ReturnBorrow>, BackendError> {
    fn visit(
        program: &mir::Program,
        function: &mir::Function,
        local: mir::LocalId,
        visiting: &mut HashSet<mir::LocalId>,
    ) -> Result<Option<mir::ReturnBorrow>, BackendError> {
        if !visiting.insert(local) {
            return Err(malformed_mir(format!(
                "function-value local{} has a recursive assignment",
                local.0
            )));
        }
        let (reachable, _) = reachable_blocks_and_predecessors(function, true)?;
        let mut inferred = None;
        for block in function.blocks.iter().filter(|block| reachable[block.id.0]) {
            for statement in &block.statements {
                let mir::Statement::AssignLocal { target, value } = statement else {
                    continue;
                };
                if *target != local {
                    continue;
                }
                let candidate = match value {
                    mir::Rvalue::Function(mir::FunctionExpression::Local {
                        local: source, ..
                    }) => visit(program, function, *source, visiting)?,
                    mir::Rvalue::Function(expression) => {
                        infer_function_expression_return_borrow(program, function, expression)?
                    }
                    mir::Rvalue::NullableFunction(mir::NullableFunctionExpression::Null {
                        ..
                    }) => None,
                    mir::Rvalue::NullableFunction(expression) => {
                        infer_nullable_function_expression_return_borrow(
                            program, function, expression,
                        )?
                    }
                    _ => continue,
                };
                merge_function_value_borrow(
                    &mut inferred,
                    candidate,
                    &format!("function-value local{}", local.0),
                )?;
            }
        }
        visiting.remove(&local);
        Ok(inferred.flatten())
    }

    visit(program, function, local, &mut HashSet::new())
}

fn merge_function_value_borrow(
    inferred: &mut Option<Option<mir::ReturnBorrow>>,
    candidate: Option<mir::ReturnBorrow>,
    source: &str,
) -> Result<(), BackendError> {
    match (*inferred, candidate) {
        (None, candidate) => *inferred = Some(candidate),
        (Some(Some(existing)), Some(candidate)) if existing.source == candidate.source => {
            *inferred = Some(Some(mir::ReturnBorrow {
                writable: existing.writable && candidate.writable,
                ..existing
            }));
        }
        (Some(None), None) => {}
        _ => {
            return Err(malformed_mir(format!(
                "{source} mixes owned and borrow-bound function values"
            )));
        }
    }
    Ok(())
}

fn infer_nullable_expression_return_borrow(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::NullableClassExpression,
) -> Result<Option<mir::ReturnBorrow>, BackendError> {
    match expression {
        mir::NullableClassExpression::Null(_) => Ok(None),
        mir::NullableClassExpression::Class(expression) => {
            infer_expression_return_borrow(program, function, expression)
        }
        mir::NullableClassExpression::SharedPayload { .. } => Err(malformed_mir(
            "returning a borrow through a nullable shared-reference payload is not supported",
        )),
        mir::NullableClassExpression::Local {
            local,
            transfer: false,
            ..
        } => match borrow_from_parameter(function, *local) {
            Some(borrow) => Ok(Some(borrow)),
            None => infer_synthetic_local_return_borrow(program, function, *local),
        },
        mir::NullableClassExpression::Property { object, .. } => Ok(infer_local_return_borrow(
            program, function, *object,
        )?
        .map(|borrow| mir::ReturnBorrow {
            writable: false,
            ..borrow
        })),
        mir::NullableClassExpression::Call {
            function: callee,
            args,
            return_borrow: Some(return_borrow),
            ..
        } => infer_borrowed_rvalue_source(program, function, *callee, args, *return_borrow),
        mir::NullableClassExpression::NullSafeProperty { object, .. } => Ok(
            infer_nullable_expression_return_borrow(program, function, object)?.map(|borrow| {
                mir::ReturnBorrow {
                    writable: false,
                    ..borrow
                }
            }),
        ),
        mir::NullableClassExpression::NullSafeCall {
            object,
            function: _,
            args,
            return_borrow: Some(return_borrow),
            ..
        } => {
            let source = match return_borrow.source {
                mir::BorrowSource::Receiver => {
                    return infer_nullable_expression_return_borrow(program, function, object).map(
                        |borrow| {
                            borrow.map(|borrow| mir::ReturnBorrow {
                                writable: borrow.writable && return_borrow.writable,
                                ..borrow
                            })
                        },
                    );
                }
                mir::BorrowSource::Parameter(index) => args.get(index),
            };
            let Some(source) = source else {
                return Err(malformed_mir(
                    "null-safe borrowed call has no source argument",
                ));
            };
            infer_rvalue_return_borrow(program, function, source).map(|borrow| {
                borrow.map(|borrow| mir::ReturnBorrow {
                    writable: borrow.writable && return_borrow.writable,
                    ..borrow
                })
            })
        }
        mir::NullableClassExpression::Local { transfer: true, .. }
        | mir::NullableClassExpression::Call {
            return_borrow: None,
            ..
        }
        | mir::NullableClassExpression::NullSafeCall {
            return_borrow: None,
            ..
        } => Ok(None),
        mir::NullableClassExpression::Coalesce { left, right, .. } => {
            let left_borrow = infer_nullable_expression_return_borrow(program, function, left)?;
            let right_borrow = infer_nullable_expression_return_borrow(program, function, right)?;
            match (left_borrow, right_borrow) {
                (left, right) if left == right => Ok(left),
                (None, right) if matches!(**left, mir::NullableClassExpression::Null(_)) => {
                    Ok(right)
                }
                (left, None) if matches!(**right, mir::NullableClassExpression::Null(_)) => {
                    Ok(left)
                }
                _ => Err(malformed_mir(
                    "nullable class coalesce mixes owned and borrowed results",
                )),
            }
        }
        mir::NullableClassExpression::DictionaryGet { collection, .. } => Ok(
            infer_local_return_borrow(program, function, *collection)?.map(|borrow| {
                mir::ReturnBorrow {
                    writable: false,
                    ..borrow
                }
            }),
        ),
    }
}

fn infer_borrowed_rvalue_source(
    program: &mir::Program,
    function: &mir::Function,
    callee: mir::FunctionId,
    args: &[mir::Rvalue],
    return_borrow: mir::ReturnBorrow,
) -> Result<Option<mir::ReturnBorrow>, BackendError> {
    let callee_definition = function_in(program, callee)?;
    let index = match return_borrow.source {
        mir::BorrowSource::Receiver => 0,
        mir::BorrowSource::Parameter(index) => {
            index + usize::from(callee_definition.receiver_mode.is_some())
        }
    };
    let source = args.get(index).ok_or_else(|| {
        malformed_mir(format!(
            "borrowed class call to {} has no source argument",
            callee_definition.name
        ))
    })?;
    infer_rvalue_return_borrow(program, function, source).map(|borrow| {
        borrow.map(|borrow| mir::ReturnBorrow {
            writable: borrow.writable && return_borrow.writable,
            ..borrow
        })
    })
}

fn infer_rvalue_return_borrow(
    program: &mir::Program,
    function: &mir::Function,
    source: &mir::Rvalue,
) -> Result<Option<mir::ReturnBorrow>, BackendError> {
    match source {
        mir::Rvalue::Class(source) => infer_expression_return_borrow(program, function, source),
        mir::Rvalue::NullableClass(source) => {
            infer_nullable_expression_return_borrow(program, function, source)
        }
        mir::Rvalue::Collection(source) => {
            infer_collection_expression_return_borrow(program, function, source)
        }
        mir::Rvalue::Mixed(source) => {
            infer_mixed_expression_return_borrow(program, function, source)
        }
        mir::Rvalue::NullableMixed(source) => {
            infer_nullable_mixed_expression_return_borrow(program, function, source)
        }
        _ => Err(malformed_mir("borrowed call source is not a move value")),
    }
}

fn infer_collection_expression_return_borrow(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::CollectionExpression,
) -> Result<Option<mir::ReturnBorrow>, BackendError> {
    match expression {
        mir::CollectionExpression::Local {
            local,
            transfer: false,
            ..
        } => match borrow_from_parameter(function, *local) {
            Some(borrow) => Ok(Some(borrow)),
            None => infer_synthetic_local_return_borrow(program, function, *local),
        },
        mir::CollectionExpression::Property { object, .. } => Ok(infer_local_return_borrow(
            program, function, *object,
        )?
        .map(|borrow| mir::ReturnBorrow {
            writable: false,
            ..borrow
        })),
        mir::CollectionExpression::SharedAccessPayload {
            access, writable, ..
        } => Ok(
            infer_local_return_borrow(program, function, *access)?.map(|borrow| {
                mir::ReturnBorrow {
                    writable: borrow.writable && *writable,
                    ..borrow
                }
            }),
        ),
        mir::CollectionExpression::Index {
            source,
            transfer: false,
            ..
        } => Ok(
            infer_local_return_borrow(program, function, *source)?.map(|borrow| {
                mir::ReturnBorrow {
                    writable: false,
                    ..borrow
                }
            }),
        ),
        mir::CollectionExpression::Call {
            function: callee,
            args,
            return_borrow: Some(return_borrow),
            ..
        } => infer_borrowed_rvalue_source(program, function, *callee, args, *return_borrow),
        mir::CollectionExpression::Local { transfer: true, .. }
        | mir::CollectionExpression::Literal { .. }
        | mir::CollectionExpression::Fill { .. }
        | mir::CollectionExpression::Index { transfer: true, .. }
        | mir::CollectionExpression::From { .. }
        | mir::CollectionExpression::FromBytes { .. }
        | mir::CollectionExpression::BytesFromArray { .. }
        | mir::CollectionExpression::ReadFileBytes { .. }
        | mir::CollectionExpression::ReadStdinBytes { .. }
        | mir::CollectionExpression::StringIntrinsic(_)
        | mir::CollectionExpression::Call {
            return_borrow: None,
            ..
        } => Ok(None),
    }
}

fn infer_mixed_expression_return_borrow(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::MixedExpression,
) -> Result<Option<mir::ReturnBorrow>, BackendError> {
    match expression {
        mir::MixedExpression::Local {
            local,
            transfer: false,
        } => match borrow_from_parameter(function, *local) {
            Some(borrow) => Ok(Some(borrow)),
            None => infer_synthetic_local_return_borrow(program, function, *local),
        },
        mir::MixedExpression::Property { object, .. } => Ok(infer_local_return_borrow(
            program, function, *object,
        )?
        .map(|borrow| mir::ReturnBorrow {
            writable: false,
            ..borrow
        })),
        mir::MixedExpression::CollectionIndex {
            collection,
            transfer: false,
            ..
        } => Ok(
            infer_local_return_borrow(program, function, *collection)?.map(|borrow| {
                mir::ReturnBorrow {
                    writable: false,
                    ..borrow
                }
            }),
        ),
        mir::MixedExpression::Call {
            function: callee,
            args,
            return_borrow: Some(return_borrow),
        } => infer_borrowed_rvalue_source(program, function, *callee, args, *return_borrow),
        mir::MixedExpression::Local { transfer: true, .. }
        | mir::MixedExpression::Call {
            return_borrow: None,
            ..
        }
        | mir::MixedExpression::BoxValue(_)
        | mir::MixedExpression::BoxString { .. }
        | mir::MixedExpression::BoxClass { .. }
        | mir::MixedExpression::BoxError { .. }
        | mir::MixedExpression::BoxPayloadEnum { .. }
        | mir::MixedExpression::BoxFunction { .. }
        | mir::MixedExpression::CollectionIndex { transfer: true, .. } => Ok(None),
    }
}

fn infer_nullable_mixed_expression_return_borrow(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::NullableMixedExpression,
) -> Result<Option<mir::ReturnBorrow>, BackendError> {
    match expression {
        mir::NullableMixedExpression::Null => Ok(None),
        mir::NullableMixedExpression::Mixed(value) => {
            infer_mixed_expression_return_borrow(program, function, value)
        }
        mir::NullableMixedExpression::BoxNullablePayloadEnum(_) => Ok(None),
        mir::NullableMixedExpression::Local {
            local,
            transfer: false,
        } => match borrow_from_parameter(function, *local) {
            Some(borrow) => Ok(Some(borrow)),
            None => infer_synthetic_local_return_borrow(program, function, *local),
        },
        mir::NullableMixedExpression::Property { object, .. } => Ok(infer_local_return_borrow(
            program, function, *object,
        )?
        .map(|borrow| mir::ReturnBorrow {
            writable: false,
            ..borrow
        })),
        mir::NullableMixedExpression::Call {
            function: callee,
            args,
            return_borrow: Some(return_borrow),
        } => infer_borrowed_rvalue_source(program, function, *callee, args, *return_borrow),
        mir::NullableMixedExpression::Coalesce { left, right, .. } => {
            let left_borrow =
                infer_nullable_mixed_expression_return_borrow(program, function, left)?;
            let right_borrow =
                infer_nullable_mixed_expression_return_borrow(program, function, right)?;
            if left_borrow == right_borrow {
                Ok(left_borrow)
            } else {
                Err(malformed_mir(
                    "nullable mixed coalesce mixes owned and borrowed results",
                ))
            }
        }
        mir::NullableMixedExpression::Local { transfer: true, .. }
        | mir::NullableMixedExpression::Call {
            return_borrow: None,
            ..
        } => Ok(None),
    }
}

fn return_borrow_is_compatible(
    actual: Option<mir::ReturnBorrow>,
    expected: Option<mir::ReturnBorrow>,
) -> bool {
    match (actual, expected) {
        (Some(actual), Some(expected)) => {
            actual.source == expected.source && (!expected.writable || actual.writable)
        }
        (None, None) => true,
        _ => false,
    }
}

fn infer_expression_return_borrow(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::ClassExpression,
) -> Result<Option<mir::ReturnBorrow>, BackendError> {
    match expression {
        mir::ClassExpression::Local {
            local,
            transfer: false,
            ..
        } => match borrow_from_parameter(function, *local) {
            Some(borrow) => Ok(Some(borrow)),
            None => infer_synthetic_local_return_borrow(program, function, *local),
        },
        mir::ClassExpression::NullableLocalAssumeNonNull {
            local,
            transfer: false,
            ..
        } => match borrow_from_parameter(function, *local) {
            Some(borrow) => Ok(Some(borrow)),
            None => infer_synthetic_local_return_borrow(program, function, *local),
        },
        mir::ClassExpression::Property { object, .. } => Ok(infer_local_return_borrow(
            program, function, *object,
        )?
        .map(|borrow| mir::ReturnBorrow {
            writable: false,
            ..borrow
        })),
        mir::ClassExpression::SharedAccessPayload {
            access, writable, ..
        } => Ok(
            infer_local_return_borrow(program, function, *access)?.map(|borrow| {
                mir::ReturnBorrow {
                    writable: borrow.writable && *writable,
                    ..borrow
                }
            }),
        ),
        mir::ClassExpression::CollectionIndex { collection, .. } => Ok(infer_local_return_borrow(
            program,
            function,
            *collection,
        )?
        .map(|borrow| mir::ReturnBorrow {
            writable: false,
            ..borrow
        })),
        mir::ClassExpression::Call {
            function: callee,
            args,
            return_borrow: Some(return_borrow),
            ..
        } => {
            let source = borrowed_call_source(program, *callee, args, *return_borrow)?;
            Ok(
                infer_expression_return_borrow(program, function, source)?.map(|borrow| {
                    mir::ReturnBorrow {
                        writable: borrow.writable && return_borrow.writable,
                        ..borrow
                    }
                }),
            )
        }
        mir::ClassExpression::Local { transfer: true, .. }
        | mir::ClassExpression::NullableLocalAssumeNonNull { transfer: true, .. }
        | mir::ClassExpression::Call {
            return_borrow: None,
            ..
        }
        | mir::ClassExpression::New { .. }
        | mir::ClassExpression::MixedPayload { .. }
        | mir::ClassExpression::SharedPayload { .. } => Ok(None),
        mir::ClassExpression::Coalesce { .. } => Ok(None),
    }
}

fn infer_synthetic_local_return_borrow(
    program: &mir::Program,
    function: &mir::Function,
    local: mir::LocalId,
) -> Result<Option<mir::ReturnBorrow>, BackendError> {
    let definition = local_in(function, local)?;
    if definition.owned {
        return Ok(None);
    }

    let (reachable, _) = reachable_blocks_and_predecessors(function, true)?;
    let mut inferred = None;
    for block in function.blocks.iter().filter(|block| reachable[block.id.0]) {
        for statement in &block.statements {
            let mir::Statement::AssignLocal { target, value } = statement else {
                continue;
            };
            if *target != local {
                continue;
            }
            let (borrow, recursive) = match value {
                mir::Rvalue::Class(expression) => (
                    infer_expression_return_borrow(program, function, expression)?,
                    class_expression_accesses_local(expression, local),
                ),
                mir::Rvalue::NullableClass(mir::NullableClassExpression::Null(_)) => continue,
                mir::Rvalue::NullableClass(expression) => (
                    infer_nullable_expression_return_borrow(program, function, expression)?,
                    nullable_class_expression_accesses_local(expression, local),
                ),
                mir::Rvalue::Collection(expression) => (
                    infer_collection_expression_return_borrow(program, function, expression)?,
                    matches!(
                        expression,
                        mir::CollectionExpression::Local {
                            local: source,
                            ..
                        } | mir::CollectionExpression::Index {
                            source,
                            ..
                        } if *source == local
                    ),
                ),
                _ => continue,
            };
            if recursive {
                return Err(malformed_mir(format!(
                    "borrowed local{} has a recursive assignment",
                    local.0
                )));
            }
            merge_synthetic_local_borrow(&mut inferred, borrow, local)?;
        }
        if let mir::Terminator::CheckedCall {
            function: callee,
            args,
            result: Some(target),
            ..
        } = &block.terminator
        {
            if *target == local {
                let callee = function_in(program, *callee)?;
                let borrow = match infer_function_return_borrow(program, callee)? {
                    Some(return_borrow) => infer_borrowed_rvalue_source(
                        program,
                        function,
                        callee.id,
                        args,
                        return_borrow,
                    )?,
                    None => None,
                };
                merge_synthetic_local_borrow(&mut inferred, borrow, local)?;
            }
        }
    }
    match inferred {
        Some(Some(borrow)) => Ok(Some(borrow)),
        Some(None) => Err(malformed_mir(format!(
            "borrowed local{} receives an owning value",
            local.0
        ))),
        None => Ok(None),
    }
}

fn merge_synthetic_local_borrow(
    inferred: &mut Option<Option<mir::ReturnBorrow>>,
    candidate: Option<mir::ReturnBorrow>,
    local: mir::LocalId,
) -> Result<(), BackendError> {
    match (*inferred, candidate) {
        (None, candidate) => *inferred = Some(candidate),
        (Some(Some(existing)), Some(candidate)) if existing.source == candidate.source => {
            *inferred = Some(Some(mir::ReturnBorrow {
                writable: existing.writable && candidate.writable,
                ..existing
            }));
        }
        (Some(None), None) => {}
        _ => {
            return Err(malformed_mir(format!(
                "borrowed local{} mixes owned and borrowed assignments",
                local.0
            )));
        }
    }
    Ok(())
}

fn infer_local_return_borrow(
    program: &mir::Program,
    function: &mir::Function,
    local: mir::LocalId,
) -> Result<Option<mir::ReturnBorrow>, BackendError> {
    match borrow_from_parameter(function, local) {
        Some(borrow) => Ok(Some(borrow)),
        None => infer_synthetic_local_return_borrow(program, function, local),
    }
}

fn borrow_from_parameter(
    function: &mir::Function,
    local: mir::LocalId,
) -> Option<mir::ReturnBorrow> {
    let position = function
        .params
        .iter()
        .position(|parameter| *parameter == local)?;
    let definition = function.locals.get(local.0)?;
    if definition.owned {
        return None;
    }
    let has_receiver = function.receiver_mode.is_some();
    let source = if has_receiver && position == 0 {
        mir::BorrowSource::Receiver
    } else {
        mir::BorrowSource::Parameter(position - usize::from(has_receiver))
    };
    Some(mir::ReturnBorrow {
        source,
        writable: definition.writable,
    })
}

fn require_writable_class_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::ClassExpression,
    destination: &str,
) -> Result<(), BackendError> {
    let writable = match expression {
        mir::ClassExpression::Local {
            local,
            transfer: false,
            ..
        } => local_in(function, *local)?.writable,
        mir::ClassExpression::NullableLocalAssumeNonNull {
            local,
            transfer: false,
            ..
        } => local_in(function, *local)?.writable,
        mir::ClassExpression::Property {
            object, property, ..
        } => {
            let object = local_in(function, *object)?;
            let mir::Type::Class(class) = object.ty else {
                return Err(malformed_mir(format!(
                    "{destination} uses a property on non-class local local{}",
                    object.id.0
                )));
            };
            object.writable && property_in(program, class, *property)?.writable
        }
        mir::ClassExpression::Local { transfer: true, .. }
        | mir::ClassExpression::NullableLocalAssumeNonNull { transfer: true, .. } => false,
        mir::ClassExpression::Call { return_borrow, .. } => {
            return_borrow.is_none_or(|borrow| borrow.writable)
        }
        mir::ClassExpression::New { .. } => true,
        mir::ClassExpression::Coalesce { left, right, .. } => {
            require_writable_nullable_class_expression(program, function, left, destination)
                .and_then(|()| {
                    require_writable_class_expression(program, function, right, destination)
                })
                .is_ok()
        }
        mir::ClassExpression::CollectionIndex { collection, .. } => {
            local_in(function, *collection)?.writable
        }
        mir::ClassExpression::SharedAccessPayload { writable, .. } => *writable,
        mir::ClassExpression::MixedPayload { .. } | mir::ClassExpression::SharedPayload { .. } => {
            false
        }
    };
    if writable {
        Ok(())
    } else {
        Err(malformed_mir(format!(
            "{destination} requires a writable class value"
        )))
    }
}

fn require_writable_nullable_class_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::NullableClassExpression,
    destination: &str,
) -> Result<(), BackendError> {
    let writable = match expression {
        mir::NullableClassExpression::Null(_) => false,
        mir::NullableClassExpression::SharedPayload { .. } => false,
        mir::NullableClassExpression::Class(value) => {
            require_writable_class_expression(program, function, value, destination).is_ok()
        }
        mir::NullableClassExpression::Local {
            local,
            transfer: false,
            ..
        } => local_in(function, *local)?.writable,
        mir::NullableClassExpression::Property {
            object, property, ..
        } => {
            let object = local_in(function, *object)?;
            let mir::Type::Class(class) = object.ty else {
                return Err(malformed_mir(format!(
                    "{destination} uses a property on non-class local local{}",
                    object.id.0
                )));
            };
            object.writable && property_in(program, class, *property)?.writable
        }
        mir::NullableClassExpression::Call { return_borrow, .. }
        | mir::NullableClassExpression::NullSafeCall { return_borrow, .. } => {
            return_borrow.is_none_or(|borrow| borrow.writable)
        }
        mir::NullableClassExpression::Coalesce { left, right, .. } => {
            require_writable_nullable_class_expression(program, function, left, destination)
                .and_then(|()| {
                    require_writable_nullable_class_expression(
                        program,
                        function,
                        right,
                        destination,
                    )
                })
                .is_ok()
        }
        mir::NullableClassExpression::Local { transfer: true, .. }
        | mir::NullableClassExpression::NullSafeProperty { .. }
        | mir::NullableClassExpression::DictionaryGet { .. } => false,
    };
    if writable {
        Ok(())
    } else {
        Err(malformed_mir(format!(
            "{destination} requires a writable nullable class value"
        )))
    }
}

fn class_expression_transfers_receiver(expression: &mir::ClassExpression) -> bool {
    match expression {
        mir::ClassExpression::Local { transfer, .. }
        | mir::ClassExpression::NullableLocalAssumeNonNull { transfer, .. }
        | mir::ClassExpression::Coalesce { transfer, .. } => *transfer,
        mir::ClassExpression::Property { .. }
        | mir::ClassExpression::Call { .. }
        | mir::ClassExpression::New { .. }
        | mir::ClassExpression::CollectionIndex { .. }
        | mir::ClassExpression::MixedPayload { .. }
        | mir::ClassExpression::SharedPayload { .. }
        | mir::ClassExpression::SharedAccessPayload { .. } => false,
    }
}

fn nullable_class_expression_transfers_receiver(expression: &mir::NullableClassExpression) -> bool {
    match expression {
        mir::NullableClassExpression::Class(value) => class_expression_transfers_receiver(value),
        mir::NullableClassExpression::Local { transfer, .. }
        | mir::NullableClassExpression::Coalesce { transfer, .. } => *transfer,
        mir::NullableClassExpression::Null(_)
        | mir::NullableClassExpression::SharedPayload { .. }
        | mir::NullableClassExpression::Property { .. }
        | mir::NullableClassExpression::Call { .. }
        | mir::NullableClassExpression::NullSafeProperty { .. }
        | mir::NullableClassExpression::NullSafeCall { .. }
        | mir::NullableClassExpression::DictionaryGet { .. } => false,
    }
}

fn validate_null_safe_method_receiver(
    program: &mir::Program,
    caller: &mir::Function,
    callee: &mir::Function,
    object: &mir::NullableClassExpression,
) -> Result<(), BackendError> {
    if nullable_class_expression_transfers_receiver(object) {
        return Err(malformed_mir(format!(
            "null-safe call to {} transfers its receiver",
            callee.name
        )));
    }
    if callee.receiver_mode == Some(mir::ReceiverMode::Writable) {
        require_writable_nullable_class_expression(
            program,
            caller,
            object,
            &format!("null-safe call to {}", callee.name),
        )?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum ClassLocalAccess<'a> {
    Borrow(mir::LocalId),
    PropertyBorrow(mir::LocalId, crate::class_layout::PropertyId),
    Transfer(mir::LocalId),
    BeginCall,
    Call(mir::FunctionId, &'a [mir::Rvalue], usize),
}

#[derive(Default)]
struct ClassLocalAccesses<'a> {
    accesses: Vec<ClassLocalAccess<'a>>,
    nullable_assumptions: Vec<mir::LocalId>,
    mixed_assumptions: Vec<(mir::LocalId, mir::MixedTag)>,
}

impl<'a> ClassLocalAccesses<'a> {
    fn borrow(&mut self, local: mir::LocalId) {
        self.accesses.push(ClassLocalAccess::Borrow(local));
    }

    fn borrow_property(&mut self, local: mir::LocalId, property: crate::class_layout::PropertyId) {
        self.accesses
            .push(ClassLocalAccess::PropertyBorrow(local, property));
    }

    fn transfer(&mut self, local: mir::LocalId) {
        self.accesses.push(ClassLocalAccess::Transfer(local));
    }

    fn call(&mut self, function: mir::FunctionId, args: &'a [mir::Rvalue]) {
        self.accesses
            .push(ClassLocalAccess::Call(function, args, 0));
    }

    fn constructor_call(&mut self, function: mir::FunctionId, args: &'a [mir::Rvalue]) {
        self.accesses
            .push(ClassLocalAccess::Call(function, args, 1));
    }

    fn method_call(&mut self, function: mir::FunctionId, args: &'a [mir::Rvalue]) {
        self.accesses
            .push(ClassLocalAccess::Call(function, args, 1));
    }

    fn begin_call(&mut self) {
        self.accesses.push(ClassLocalAccess::BeginCall);
    }

    fn assume_nullable_present(&mut self, local: mir::LocalId) {
        self.nullable_assumptions.push(local);
    }

    fn assume_mixed_tag(&mut self, local: mir::LocalId, tag: mir::MixedTag) {
        self.mixed_assumptions.push((local, tag));
    }

    fn iter(&self) -> impl Iterator<Item = ClassLocalAccess<'a>> + '_ {
        self.accesses.iter().copied()
    }

    fn nullable_assumptions(&self) -> impl Iterator<Item = mir::LocalId> + '_ {
        self.nullable_assumptions.iter().copied()
    }

    fn mixed_assumptions(&self) -> impl Iterator<Item = (mir::LocalId, mir::MixedTag)> + '_ {
        self.mixed_assumptions.iter().copied()
    }

    fn borrowed(&self) -> impl Iterator<Item = mir::LocalId> + '_ {
        self.iter().filter_map(|access| match access {
            ClassLocalAccess::Borrow(local) | ClassLocalAccess::PropertyBorrow(local, _) => {
                Some(local)
            }
            ClassLocalAccess::Transfer(_)
            | ClassLocalAccess::BeginCall
            | ClassLocalAccess::Call(_, _, _) => None,
        })
    }

    fn transferred(&self) -> impl Iterator<Item = mir::LocalId> + '_ {
        self.iter().filter_map(|access| match access {
            ClassLocalAccess::Transfer(local) => Some(local),
            ClassLocalAccess::Borrow(_)
            | ClassLocalAccess::PropertyBorrow(_, _)
            | ClassLocalAccess::BeginCall
            | ClassLocalAccess::Call(_, _, _) => None,
        })
    }

    fn property_borrowed(
        &self,
    ) -> impl Iterator<Item = (mir::LocalId, crate::class_layout::PropertyId)> + '_ {
        self.iter().filter_map(|access| match access {
            ClassLocalAccess::PropertyBorrow(local, property) => Some((local, property)),
            ClassLocalAccess::Borrow(_)
            | ClassLocalAccess::Transfer(_)
            | ClassLocalAccess::BeginCall
            | ClassLocalAccess::Call(_, _, _) => None,
        })
    }
}

#[derive(Clone, Copy)]
struct PropertyAliasInvalidation {
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
    alias: mir::LocalId,
}

fn rvalue_transfers_class_local(value: &mir::Rvalue, local: mir::LocalId) -> bool {
    let mut accesses = ClassLocalAccesses::default();
    collect_rvalue_class_local_accesses(value, &mut accesses);
    let transfers_local = accesses
        .transferred()
        .any(|transferred| transferred == local);
    transfers_local
}

fn rvalue_borrows_class_local_outside_property(
    value: &mir::Rvalue,
    local: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    let mut accesses = ClassLocalAccesses::default();
    collect_rvalue_class_local_accesses(value, &mut accesses);
    let receiver_borrows = accesses
        .borrowed()
        .filter(|borrowed| *borrowed == local)
        .count();
    let exact_target_borrows = accesses
        .property_borrowed()
        .filter(|borrowed| *borrowed == (local, property))
        .count();
    receiver_borrows != exact_target_borrows
}

fn collect_rvalue_class_local_accesses<'a>(
    value: &'a mir::Rvalue,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::Rvalue::Value(value) => collect_value_class_local_accesses(value, accesses),
        mir::Rvalue::String(value) => collect_string_class_local_accesses(value, accesses),
        mir::Rvalue::Mixed(value) => collect_mixed_class_local_accesses(value, accesses),
        mir::Rvalue::NullableScalar(value) => {
            collect_nullable_scalar_class_local_accesses(value, accesses)
        }
        mir::Rvalue::NullableString(value) => {
            collect_nullable_string_class_local_accesses(value, accesses)
        }
        mir::Rvalue::NullableMixed(value) => {
            collect_nullable_mixed_class_local_accesses(value, accesses)
        }
        mir::Rvalue::Error(value) => collect_error_class_local_accesses(value, accesses),
        mir::Rvalue::NullableError(value) => {
            collect_nullable_error_class_local_accesses(value, accesses)
        }
        mir::Rvalue::Class(value) => collect_class_expression_local_accesses(value, accesses),
        mir::Rvalue::NullableClass(value) => collect_nullable_class_local_accesses(value, accesses),
        mir::Rvalue::Collection(value) => collect_collection_class_local_accesses(value, accesses),
        mir::Rvalue::NullableCollection(value) => {
            collect_nullable_collection_class_local_accesses(value, accesses)
        }
        mir::Rvalue::SharedReference(value) => {
            collect_shared_reference_class_local_accesses(value, accesses)
        }
        mir::Rvalue::WeakReference(value) => {
            collect_weak_reference_class_local_accesses(value, accesses)
        }
        mir::Rvalue::NullableSharedReference(value) => {
            collect_nullable_shared_reference_class_local_accesses(value, accesses)
        }
        mir::Rvalue::NullableWeakReference(value) => {
            collect_nullable_weak_reference_class_local_accesses(value, accesses)
        }
        mir::Rvalue::WritableSharedReference(value) => {
            collect_writable_shared_class_local_accesses(value, accesses)
        }
        mir::Rvalue::WritableWeakReference(value) => {
            collect_writable_weak_class_local_accesses(value, accesses)
        }
        mir::Rvalue::NullableWritableSharedReference(value) => {
            collect_nullable_writable_shared_class_local_accesses(value, accesses)
        }
        mir::Rvalue::NullableWritableWeakReference(value) => {
            collect_nullable_writable_weak_class_local_accesses(value, accesses)
        }
        mir::Rvalue::SharedReferenceAccess(value) => {
            collect_shared_access_class_local_accesses(value, accesses)
        }
        mir::Rvalue::NullableSharedReferenceAccess(value) => {
            collect_nullable_shared_access_class_local_accesses(value, accesses)
        }
        mir::Rvalue::PayloadEnum(value) => {
            collect_payload_enum_class_local_accesses(value, accesses)
        }
        mir::Rvalue::NullablePayloadEnum(value) => {
            collect_nullable_payload_enum_class_local_accesses(value, accesses)
        }
        mir::Rvalue::Function(value) => collect_function_class_local_accesses(value, accesses),
        mir::Rvalue::NullableFunction(value) => {
            collect_nullable_function_class_local_accesses(value, accesses)
        }
    }
}

fn collect_function_class_local_accesses<'a>(
    value: &'a mir::FunctionExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::FunctionExpression::Create { captures, .. } => {
            for capture in captures {
                match capture {
                    mir::ClosureCaptureOperand::BorrowLocal { local, .. } => {
                        accesses.borrow(*local)
                    }
                    mir::ClosureCaptureOperand::CopyValue(value) => {
                        collect_rvalue_class_local_accesses(value, accesses)
                    }
                    mir::ClosureCaptureOperand::MoveValue(value) => {
                        collect_rvalue_class_local_accesses(value, accesses)
                    }
                }
            }
        }
        mir::FunctionExpression::Call { function, args, .. } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::FunctionExpression::CollectionIndex { index, .. } => {
            collect_rvalue_class_local_accesses(index, accesses)
        }
        mir::FunctionExpression::AssumePresent { value, .. } => {
            collect_nullable_function_class_local_accesses(value, accesses)
        }
        mir::FunctionExpression::MixedPayload {
            function_type,
            mixed,
            ..
        } => accesses.assume_mixed_tag(*mixed, mir::MixedTag::Function(*function_type)),
        mir::FunctionExpression::Local { .. } | mir::FunctionExpression::Property { .. } => {}
    }
}

fn collect_nullable_function_class_local_accesses<'a>(
    value: &'a mir::NullableFunctionExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::NullableFunctionExpression::Present(value) => {
            collect_function_class_local_accesses(value, accesses)
        }
        mir::NullableFunctionExpression::Call { function, args, .. } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::NullableFunctionExpression::DictionaryGet { key, .. }
        | mir::NullableFunctionExpression::CollectionIndex { index: key, .. } => {
            collect_rvalue_class_local_accesses(key, accesses)
        }
        mir::NullableFunctionExpression::Null { .. }
        | mir::NullableFunctionExpression::Local { .. }
        | mir::NullableFunctionExpression::Property { .. } => {}
    }
}

fn collect_payload_enum_class_local_accesses<'a>(
    value: &'a mir::PayloadEnumExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::PayloadEnumExpression::Construct { fields, .. } => {
            collect_rvalue_args_class_local_accesses(fields, accesses)
        }
        mir::PayloadEnumExpression::Use { place, .. } => {
            collect_payload_enum_place_class_local_accesses(place, accesses)
        }
        mir::PayloadEnumExpression::Call { function, args, .. } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::PayloadEnumExpression::Coalesce { left, right, .. } => {
            collect_nullable_payload_enum_class_local_accesses(left, accesses);
            collect_payload_enum_class_local_accesses(right, accesses);
        }
    }
}

fn collect_nullable_payload_enum_class_local_accesses<'a>(
    value: &'a mir::NullablePayloadEnumExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::NullablePayloadEnumExpression::Null(_) => {}
        mir::NullablePayloadEnumExpression::Value(value) => {
            collect_payload_enum_class_local_accesses(value, accesses)
        }
        mir::NullablePayloadEnumExpression::Use { place, .. } => {
            collect_payload_enum_place_class_local_accesses(place, accesses)
        }
        mir::NullablePayloadEnumExpression::Call { function, args, .. } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::NullablePayloadEnumExpression::CollectionGet { key, .. } => {
            collect_rvalue_class_local_accesses(key, accesses)
        }
        mir::NullablePayloadEnumExpression::Coalesce { left, right, .. } => {
            collect_nullable_payload_enum_class_local_accesses(left, accesses);
            collect_nullable_payload_enum_class_local_accesses(right, accesses);
        }
    }
}

fn collect_payload_enum_place_class_local_accesses<'a>(
    place: &'a mir::PayloadEnumPlace,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match place {
        mir::PayloadEnumPlace::Property { object, .. } => accesses.borrow(*object),
        mir::PayloadEnumPlace::CollectionIndex { index, .. } => {
            collect_rvalue_class_local_accesses(index, accesses)
        }
        mir::PayloadEnumPlace::NullableLocalAssumeNonNull(local) => {
            accesses.assume_nullable_present(*local)
        }
        mir::PayloadEnumPlace::Local(_)
        | mir::PayloadEnumPlace::Static(_)
        | mir::PayloadEnumPlace::MixedPayload { .. } => {}
    }
}

fn collect_shared_reference_class_local_accesses<'a>(
    value: &'a mir::SharedReferenceExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::SharedReferenceExpression::New { value, .. } => {
            collect_class_expression_local_accesses(value, accesses)
        }
        mir::SharedReferenceExpression::Call { function, args, .. } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::SharedReferenceExpression::Share { value, .. } => {
            collect_shared_reference_class_local_accesses(value, accesses)
        }
        mir::SharedReferenceExpression::Coalesce { left, right, .. } => {
            collect_nullable_shared_reference_class_local_accesses(left, accesses);
            collect_shared_reference_class_local_accesses(right, accesses);
        }
        mir::SharedReferenceExpression::CollectionIndex { index, .. } => {
            collect_rvalue_class_local_accesses(index, accesses)
        }
        mir::SharedReferenceExpression::NullableLocalAssumeNonNull { local, .. } => {
            accesses.assume_nullable_present(*local)
        }
        mir::SharedReferenceExpression::Local { .. }
        | mir::SharedReferenceExpression::Property { .. } => {}
    }
}

fn collect_weak_reference_class_local_accesses<'a>(
    value: &'a mir::WeakReferenceExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::WeakReferenceExpression::Call { function, args, .. } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::WeakReferenceExpression::Create { value, .. } => {
            collect_shared_reference_class_local_accesses(value, accesses)
        }
        mir::WeakReferenceExpression::Coalesce { left, right, .. } => {
            collect_nullable_weak_reference_class_local_accesses(left, accesses);
            collect_weak_reference_class_local_accesses(right, accesses);
        }
        mir::WeakReferenceExpression::CollectionIndex { index, .. } => {
            collect_rvalue_class_local_accesses(index, accesses)
        }
        mir::WeakReferenceExpression::NullableLocalAssumeNonNull { local, .. } => {
            accesses.assume_nullable_present(*local)
        }
        mir::WeakReferenceExpression::Local { .. }
        | mir::WeakReferenceExpression::Property { .. } => {}
    }
}

fn collect_nullable_shared_reference_class_local_accesses<'a>(
    value: &'a mir::NullableSharedReferenceExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::NullableSharedReferenceExpression::Shared(value) => {
            collect_shared_reference_class_local_accesses(value, accesses)
        }
        mir::NullableSharedReferenceExpression::Call { function, args, .. } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::NullableSharedReferenceExpression::Acquire { value, .. } => {
            collect_weak_reference_class_local_accesses(value, accesses)
        }
        mir::NullableSharedReferenceExpression::NullSafeShare { value, .. } => {
            collect_nullable_shared_reference_class_local_accesses(value, accesses)
        }
        mir::NullableSharedReferenceExpression::NullSafeAcquire { value, .. } => {
            collect_nullable_weak_reference_class_local_accesses(value, accesses)
        }
        mir::NullableSharedReferenceExpression::Coalesce { left, right, .. } => {
            collect_nullable_shared_reference_class_local_accesses(left, accesses);
            collect_nullable_shared_reference_class_local_accesses(right, accesses);
        }
        mir::NullableSharedReferenceExpression::DictionaryGet { key, .. } => {
            collect_rvalue_class_local_accesses(key, accesses)
        }
        mir::NullableSharedReferenceExpression::CollectionIndex { index, .. } => {
            collect_rvalue_class_local_accesses(index, accesses)
        }
        mir::NullableSharedReferenceExpression::Null(_)
        | mir::NullableSharedReferenceExpression::Local { .. }
        | mir::NullableSharedReferenceExpression::Property { .. } => {}
    }
}

fn collect_nullable_weak_reference_class_local_accesses<'a>(
    value: &'a mir::NullableWeakReferenceExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::NullableWeakReferenceExpression::Weak(value) => {
            collect_weak_reference_class_local_accesses(value, accesses)
        }
        mir::NullableWeakReferenceExpression::Call { function, args, .. } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::NullableWeakReferenceExpression::NullSafeCreate { value, .. } => {
            collect_nullable_shared_reference_class_local_accesses(value, accesses)
        }
        mir::NullableWeakReferenceExpression::Coalesce { left, right, .. } => {
            collect_nullable_weak_reference_class_local_accesses(left, accesses);
            collect_nullable_weak_reference_class_local_accesses(right, accesses);
        }
        mir::NullableWeakReferenceExpression::DictionaryGet { key, .. } => {
            collect_rvalue_class_local_accesses(key, accesses)
        }
        mir::NullableWeakReferenceExpression::CollectionIndex { index, .. } => {
            collect_rvalue_class_local_accesses(index, accesses)
        }
        mir::NullableWeakReferenceExpression::Null(_)
        | mir::NullableWeakReferenceExpression::Local { .. }
        | mir::NullableWeakReferenceExpression::Property { .. } => {}
    }
}

fn collect_writable_shared_class_local_accesses<'a>(
    value: &'a mir::WritableSharedReferenceExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::WritableSharedReferenceExpression::New { value, .. } => {
            collect_rvalue_class_local_accesses(value, accesses)
        }
        mir::WritableSharedReferenceExpression::Call { function, args, .. } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::WritableSharedReferenceExpression::Share { value, .. } => {
            collect_writable_shared_class_local_accesses(value, accesses)
        }
        mir::WritableSharedReferenceExpression::Coalesce { left, right, .. } => {
            collect_nullable_writable_shared_class_local_accesses(left, accesses);
            collect_writable_shared_class_local_accesses(right, accesses);
        }
        mir::WritableSharedReferenceExpression::CollectionIndex { index, .. } => {
            collect_rvalue_class_local_accesses(index, accesses)
        }
        mir::WritableSharedReferenceExpression::NullableLocalAssumeNonNull { local, .. } => {
            accesses.assume_nullable_present(*local)
        }
        mir::WritableSharedReferenceExpression::Local { .. }
        | mir::WritableSharedReferenceExpression::Property { .. } => {}
    }
}

fn collect_writable_weak_class_local_accesses<'a>(
    value: &'a mir::WritableWeakReferenceExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::WritableWeakReferenceExpression::Call { function, args, .. } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::WritableWeakReferenceExpression::Create { value, .. } => {
            collect_writable_shared_class_local_accesses(value, accesses)
        }
        mir::WritableWeakReferenceExpression::Coalesce { left, right, .. } => {
            collect_nullable_writable_weak_class_local_accesses(left, accesses);
            collect_writable_weak_class_local_accesses(right, accesses);
        }
        mir::WritableWeakReferenceExpression::CollectionIndex { index, .. } => {
            collect_rvalue_class_local_accesses(index, accesses)
        }
        mir::WritableWeakReferenceExpression::NullableLocalAssumeNonNull { local, .. } => {
            accesses.assume_nullable_present(*local)
        }
        mir::WritableWeakReferenceExpression::Local { .. }
        | mir::WritableWeakReferenceExpression::Property { .. } => {}
    }
}

fn collect_nullable_writable_shared_class_local_accesses<'a>(
    value: &'a mir::NullableWritableSharedReferenceExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::NullableWritableSharedReferenceExpression::Strong(value) => {
            collect_writable_shared_class_local_accesses(value, accesses)
        }
        mir::NullableWritableSharedReferenceExpression::Call { function, args, .. } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::NullableWritableSharedReferenceExpression::Acquire { value, .. } => {
            collect_writable_weak_class_local_accesses(value, accesses)
        }
        mir::NullableWritableSharedReferenceExpression::NullSafeShare { value, .. } => {
            collect_nullable_writable_shared_class_local_accesses(value, accesses)
        }
        mir::NullableWritableSharedReferenceExpression::NullSafeAcquire { value, .. } => {
            collect_nullable_writable_weak_class_local_accesses(value, accesses)
        }
        mir::NullableWritableSharedReferenceExpression::Coalesce { left, right, .. } => {
            collect_nullable_writable_shared_class_local_accesses(left, accesses);
            collect_nullable_writable_shared_class_local_accesses(right, accesses);
        }
        mir::NullableWritableSharedReferenceExpression::DictionaryGet { key, .. } => {
            collect_rvalue_class_local_accesses(key, accesses)
        }
        mir::NullableWritableSharedReferenceExpression::Null(_)
        | mir::NullableWritableSharedReferenceExpression::Local { .. }
        | mir::NullableWritableSharedReferenceExpression::Property { .. } => {}
    }
}

fn collect_nullable_writable_weak_class_local_accesses<'a>(
    value: &'a mir::NullableWritableWeakReferenceExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::NullableWritableWeakReferenceExpression::Weak(value) => {
            collect_writable_weak_class_local_accesses(value, accesses)
        }
        mir::NullableWritableWeakReferenceExpression::Call { function, args, .. } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::NullableWritableWeakReferenceExpression::NullSafeCreate { value, .. } => {
            collect_nullable_writable_shared_class_local_accesses(value, accesses)
        }
        mir::NullableWritableWeakReferenceExpression::Coalesce { left, right, .. } => {
            collect_nullable_writable_weak_class_local_accesses(left, accesses);
            collect_nullable_writable_weak_class_local_accesses(right, accesses);
        }
        mir::NullableWritableWeakReferenceExpression::DictionaryGet { key, .. } => {
            collect_rvalue_class_local_accesses(key, accesses)
        }
        mir::NullableWritableWeakReferenceExpression::Null(_)
        | mir::NullableWritableWeakReferenceExpression::Local { .. }
        | mir::NullableWritableWeakReferenceExpression::Property { .. } => {}
    }
}

fn collect_shared_access_class_local_accesses<'a>(
    value: &'a mir::SharedReferenceAccessExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::SharedReferenceAccessExpression::Call { function, args, .. } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::SharedReferenceAccessExpression::Acquire { value, .. } => {
            collect_writable_shared_class_local_accesses(value, accesses)
        }
        mir::SharedReferenceAccessExpression::CollectionIndex { index, .. } => {
            collect_rvalue_class_local_accesses(index, accesses)
        }
        mir::SharedReferenceAccessExpression::NullableLocalAssumeNonNull { local, .. } => {
            accesses.assume_nullable_present(*local)
        }
        mir::SharedReferenceAccessExpression::Local { .. }
        | mir::SharedReferenceAccessExpression::Property { .. } => {}
    }
}

fn collect_nullable_shared_access_class_local_accesses<'a>(
    value: &'a mir::NullableSharedReferenceAccessExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::NullableSharedReferenceAccessExpression::Access(value) => {
            collect_shared_access_class_local_accesses(value, accesses)
        }
        mir::NullableSharedReferenceAccessExpression::Call { function, args, .. } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::NullableSharedReferenceAccessExpression::NullSafeAcquire { value, .. } => {
            collect_nullable_writable_shared_class_local_accesses(value, accesses)
        }
        mir::NullableSharedReferenceAccessExpression::CollectionIndex { index, .. } => {
            collect_rvalue_class_local_accesses(index, accesses)
        }
        mir::NullableSharedReferenceAccessExpression::CollectionGet { key, .. } => {
            collect_rvalue_class_local_accesses(key, accesses)
        }
        mir::NullableSharedReferenceAccessExpression::Null { .. }
        | mir::NullableSharedReferenceAccessExpression::Local { .. }
        | mir::NullableSharedReferenceAccessExpression::Property { .. } => {}
    }
}

fn collect_rvalue_args_class_local_accesses<'a>(
    args: &'a [mir::Rvalue],
    accesses: &mut ClassLocalAccesses<'a>,
) {
    for value in args {
        collect_rvalue_class_local_accesses(value, accesses);
    }
}

fn collect_collection_class_local_accesses<'a>(
    value: &'a mir::CollectionExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::CollectionExpression::Literal { entries, .. } => {
            for entry in entries {
                if let Some(key) = &entry.key {
                    collect_rvalue_class_local_accesses(key, accesses);
                }
                collect_rvalue_class_local_accesses(&entry.value, accesses);
            }
        }
        mir::CollectionExpression::Fill { value, count, .. } => {
            collect_rvalue_class_local_accesses(value, accesses);
            collect_integer_class_local_accesses(count, accesses);
        }
        mir::CollectionExpression::Index { index, .. } => {
            collect_rvalue_class_local_accesses(index, accesses)
        }
        mir::CollectionExpression::Property {
            object, property, ..
        } => accesses.borrow_property(*object, *property),
        mir::CollectionExpression::ReadFileBytes { path, .. } => {
            collect_string_class_local_accesses(path, accesses)
        }
        mir::CollectionExpression::Call { function, args, .. } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::CollectionExpression::StringIntrinsic(call) => {
            collect_rvalue_args_class_local_accesses(&call.args, accesses);
        }
        mir::CollectionExpression::Local { .. }
        | mir::CollectionExpression::SharedAccessPayload { .. }
        | mir::CollectionExpression::From { .. }
        | mir::CollectionExpression::FromBytes { .. }
        | mir::CollectionExpression::BytesFromArray { .. }
        | mir::CollectionExpression::ReadStdinBytes { .. } => {}
    }
}

fn collect_nullable_collection_class_local_accesses<'a>(
    value: &'a mir::NullableCollectionExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::NullableCollectionExpression::Collection(value) => {
            collect_collection_class_local_accesses(value, accesses)
        }
        mir::NullableCollectionExpression::Property {
            object, property, ..
        } => accesses.borrow_property(*object, *property),
        mir::NullableCollectionExpression::Call { function, args, .. } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::NullableCollectionExpression::Coalesce { left, right, .. } => {
            collect_nullable_collection_class_local_accesses(left, accesses);
            collect_nullable_collection_class_local_accesses(right, accesses);
        }
        mir::NullableCollectionExpression::Null(_)
        | mir::NullableCollectionExpression::Local { .. } => {}
    }
}

fn collect_mixed_class_local_accesses<'a>(
    value: &'a mir::MixedExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::MixedExpression::BoxValue(value) => {
            collect_value_class_local_accesses(value, accesses)
        }
        mir::MixedExpression::BoxString { value, .. } => {
            collect_string_class_local_accesses(value, accesses)
        }
        mir::MixedExpression::BoxClass { value, .. } => {
            collect_class_expression_local_accesses(value, accesses)
        }
        mir::MixedExpression::BoxError { value } => {
            collect_error_class_local_accesses(value, accesses)
        }
        mir::MixedExpression::BoxPayloadEnum { value } => {
            collect_payload_enum_class_local_accesses(value, accesses)
        }
        mir::MixedExpression::BoxFunction { value, .. } => {
            collect_function_class_local_accesses(value, accesses)
        }
        mir::MixedExpression::Call { function, args, .. } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::MixedExpression::CollectionIndex { index, .. } => {
            collect_rvalue_class_local_accesses(index, accesses)
        }
        mir::MixedExpression::Local { .. } | mir::MixedExpression::Property { .. } => {}
    }
}

fn collect_error_class_local_accesses<'a>(
    value: &'a mir::ErrorExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::ErrorExpression::FromClass { object, .. } => {
            collect_class_expression_local_accesses(object, accesses)
        }
        mir::ErrorExpression::FromNullableClass { object, .. } => {
            collect_nullable_class_local_accesses(object, accesses)
        }
        mir::ErrorExpression::Property {
            object, property, ..
        } => accesses.borrow_property(*object, *property),
        mir::ErrorExpression::Call { function, args, .. } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::ErrorExpression::CollectionIndex { index, .. } => {
            collect_rvalue_class_local_accesses(index, accesses)
        }
        mir::ErrorExpression::NullableLocalAssumeNonNull { local, transfer } => {
            accesses.assume_nullable_present(*local);
            if *transfer {
                accesses.transfer(*local);
            } else {
                accesses.borrow(*local);
            }
        }
        mir::ErrorExpression::Local { .. } | mir::ErrorExpression::MixedPayload { .. } => {}
    }
}

fn collect_nullable_error_class_local_accesses<'a>(
    value: &'a mir::NullableErrorExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::NullableErrorExpression::Error(value) => {
            collect_error_class_local_accesses(value, accesses)
        }
        mir::NullableErrorExpression::Property {
            object, property, ..
        } => accesses.borrow_property(*object, *property),
        mir::NullableErrorExpression::Call { function, args, .. } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::NullableErrorExpression::DictionaryGet { key, .. }
        | mir::NullableErrorExpression::CollectionIndex { index: key, .. } => {
            collect_rvalue_class_local_accesses(key, accesses)
        }
        mir::NullableErrorExpression::Null | mir::NullableErrorExpression::Local { .. } => {}
    }
}

fn collect_nullable_mixed_class_local_accesses<'a>(
    value: &'a mir::NullableMixedExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::NullableMixedExpression::Mixed(value) => {
            collect_mixed_class_local_accesses(value, accesses)
        }
        mir::NullableMixedExpression::BoxNullablePayloadEnum(value) => {
            collect_nullable_payload_enum_class_local_accesses(value, accesses)
        }
        mir::NullableMixedExpression::Call { function, args, .. } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::NullableMixedExpression::Coalesce { left, right, .. } => {
            collect_nullable_mixed_class_local_accesses(left, accesses);
            collect_nullable_mixed_class_local_accesses(right, accesses);
        }
        mir::NullableMixedExpression::Null
        | mir::NullableMixedExpression::Local { .. }
        | mir::NullableMixedExpression::Property { .. } => {}
    }
}

fn collect_value_class_local_accesses<'a>(
    value: &'a mir::ValueExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::ValueExpression::Integer(value) => {
            collect_integer_class_local_accesses(value, accesses)
        }
        mir::ValueExpression::Float(value) => collect_float_class_local_accesses(value, accesses),
        mir::ValueExpression::Bool(value) => collect_bool_class_local_accesses(value, accesses),
        mir::ValueExpression::Enum(value) => collect_enum_class_local_accesses(value, accesses),
    }
}

fn collect_enum_class_local_accesses<'a>(
    value: &'a mir::EnumExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::EnumExpression::Use { operand, .. } => {
            collect_operand_class_local_accesses(operand, accesses)
        }
        mir::EnumExpression::Case(_) => {}
        mir::EnumExpression::Call { function, args, .. } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::EnumExpression::Coalesce { left, right, .. } => {
            collect_nullable_scalar_class_local_accesses(left, accesses);
            collect_enum_class_local_accesses(right, accesses);
        }
    }
}

fn collect_operand_class_local_accesses<'a>(
    operand: &'a mir::Operand,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match operand {
        mir::Operand::Property { object, property } => {
            accesses.borrow_property(*object, *property);
        }
        mir::Operand::NullablePayload(local) => accesses.assume_nullable_present(*local),
        mir::Operand::CollectionIndex { index, .. } => {
            collect_rvalue_class_local_accesses(index, accesses)
        }
        mir::Operand::CollectionKeyAt { offset, .. } => {
            collect_rvalue_class_local_accesses(offset, accesses)
        }
        mir::Operand::MixedPayload { mixed, tag } => accesses.assume_mixed_tag(*mixed, *tag),
        mir::Operand::StringIntrinsic(call) => {
            collect_rvalue_args_class_local_accesses(&call.args, accesses);
        }
        mir::Operand::Scalar(_)
        | mir::Operand::Local(_)
        | mir::Operand::Static(_)
        | mir::Operand::CollectionLength(_) => {}
    }
}

fn collect_integer_class_local_accesses<'a>(
    value: &'a mir::IntegerExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::IntegerExpression::Use { operand, .. } => {
            collect_operand_class_local_accesses(operand, accesses);
        }
        mir::IntegerExpression::Unary { operand, .. }
        | mir::IntegerExpression::Convert { value: operand, .. } => {
            collect_integer_class_local_accesses(operand, accesses);
        }
        mir::IntegerExpression::Binary { left, right, .. } => {
            collect_integer_class_local_accesses(left, accesses);
            collect_integer_class_local_accesses(right, accesses);
        }
        mir::IntegerExpression::FloatToInt { value, .. } => {
            collect_float_class_local_accesses(value, accesses);
        }
        mir::IntegerExpression::Call { function, args, .. } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::IntegerExpression::Coalesce { left, right, .. } => {
            collect_nullable_scalar_class_local_accesses(left, accesses);
            collect_integer_class_local_accesses(right, accesses);
        }
        mir::IntegerExpression::EnumBacking { value, .. } => {
            collect_enum_class_local_accesses(value, accesses)
        }
    }
}

fn collect_float_class_local_accesses<'a>(
    value: &'a mir::FloatExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::FloatExpression::Use { operand, .. } => {
            collect_operand_class_local_accesses(operand, accesses);
        }
        mir::FloatExpression::Negate { operand, .. } => {
            collect_float_class_local_accesses(operand, accesses);
        }
        mir::FloatExpression::Binary { left, right, .. } => {
            collect_float_class_local_accesses(left, accesses);
            collect_float_class_local_accesses(right, accesses);
        }
        mir::FloatExpression::IntToFloat { value } => {
            collect_integer_class_local_accesses(value, accesses);
        }
        mir::FloatExpression::Call { function, args, .. } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::FloatExpression::Coalesce { left, right, .. } => {
            collect_nullable_scalar_class_local_accesses(left, accesses);
            collect_float_class_local_accesses(right, accesses);
        }
    }
}

fn collect_string_class_local_accesses<'a>(
    value: &'a mir::StringExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::StringExpression::Concat(parts) => {
            for part in parts {
                collect_string_class_local_accesses(part, accesses);
            }
        }
        mir::StringExpression::Display(value) => {
            collect_value_class_local_accesses(value, accesses);
        }
        mir::StringExpression::Call { function, args } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::StringExpression::ReadFile { path, .. } => {
            collect_string_class_local_accesses(path, accesses);
        }
        mir::StringExpression::Format(format) => {
            collect_format_class_local_accesses(format, accesses);
        }
        mir::StringExpression::Coalesce { left, right } => {
            collect_nullable_string_class_local_accesses(left, accesses);
            collect_string_class_local_accesses(right, accesses);
        }
        mir::StringExpression::CollectionIndex { index, .. } => {
            collect_rvalue_class_local_accesses(index, accesses)
        }
        mir::StringExpression::CollectionKeyAt { offset, .. } => {
            collect_rvalue_class_local_accesses(offset, accesses)
        }
        mir::StringExpression::Intrinsic(call) => {
            collect_rvalue_args_class_local_accesses(&call.args, accesses)
        }
        mir::StringExpression::NullableLocalAssumeNonNull(local) => {
            accesses.assume_nullable_present(*local)
        }
        mir::StringExpression::MixedPayload(local) => {
            accesses.assume_mixed_tag(*local, mir::MixedTag::String);
        }
        mir::StringExpression::EnumBacking { value, .. } => {
            collect_enum_class_local_accesses(value, accesses)
        }
        mir::StringExpression::Literal(_)
        | mir::StringExpression::Local(_)
        | mir::StringExpression::Static(_) => {}
        mir::StringExpression::Property { object, property } => {
            accesses.borrow_property(*object, *property)
        }
        mir::StringExpression::ErrorMessage(error) => {
            collect_error_class_local_accesses(error, accesses)
        }
    }
}

fn collect_nullable_string_class_local_accesses<'a>(
    value: &'a mir::NullableStringExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::NullableStringExpression::String(value) => {
            collect_string_class_local_accesses(value, accesses);
        }
        mir::NullableStringExpression::Call { function, args } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::NullableStringExpression::EnumBacking { value, .. } => {
            collect_nullable_scalar_class_local_accesses(value, accesses)
        }
        mir::NullableStringExpression::ReadLine { prompt, .. } => {
            collect_string_class_local_accesses(prompt, accesses);
        }
        mir::NullableStringExpression::Null
        | mir::NullableStringExpression::Local(_)
        | mir::NullableStringExpression::Static(_) => {}
        mir::NullableStringExpression::Property { object, property } => {
            accesses.borrow_property(*object, *property);
        }
        mir::NullableStringExpression::NullSafeProperty { object, .. } => {
            collect_nullable_class_local_accesses(object, accesses);
        }
        mir::NullableStringExpression::NullSafeCall {
            object,
            function,
            args,
        } => {
            collect_nullable_class_local_accesses(object, accesses);
            if !nullable_class_expression_is_definitely_null(object) {
                accesses.begin_call();
                collect_rvalue_args_class_local_accesses(args, accesses);
                accesses.method_call(*function, args);
            }
        }
        mir::NullableStringExpression::Coalesce { left, right } => {
            collect_nullable_string_class_local_accesses(left, accesses);
            collect_nullable_string_class_local_accesses(right, accesses);
        }
        mir::NullableStringExpression::DictionaryGet { key, .. } => {
            collect_rvalue_class_local_accesses(key, accesses)
        }
        mir::NullableStringExpression::Intrinsic(call) => {
            collect_rvalue_args_class_local_accesses(&call.args, accesses)
        }
    }
}

fn collect_class_expression_local_accesses<'a>(
    value: &'a mir::ClassExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::ClassExpression::Local {
            local,
            transfer: true,
            ..
        } => accesses.transfer(*local),
        mir::ClassExpression::Local {
            local,
            transfer: false,
            ..
        } => accesses.borrow(*local),
        mir::ClassExpression::NullableLocalAssumeNonNull {
            local,
            transfer: true,
            ..
        } => {
            accesses.assume_nullable_present(*local);
            accesses.transfer(*local);
        }
        mir::ClassExpression::NullableLocalAssumeNonNull {
            local,
            transfer: false,
            ..
        } => {
            accesses.assume_nullable_present(*local);
            accesses.borrow(*local);
        }
        mir::ClassExpression::Property {
            object, property, ..
        } => accesses.borrow_property(*object, *property),
        mir::ClassExpression::Call { function, args, .. } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::ClassExpression::New {
            properties,
            constructor,
            args,
            ..
        } => {
            for property in properties {
                if let mir::PropertyValueSource::Expression(value) = &property.source {
                    collect_rvalue_class_local_accesses(value, accesses);
                }
            }
            if constructor.is_some() {
                accesses.begin_call();
            }
            collect_rvalue_args_class_local_accesses(args, accesses);
            if let Some(function) = constructor {
                accesses.constructor_call(*function, args);
            }
        }
        mir::ClassExpression::Coalesce { left, right, .. } => {
            collect_nullable_class_local_accesses(left, accesses);
            collect_class_expression_local_accesses(right, accesses);
        }
        mir::ClassExpression::CollectionIndex { index, .. } => {
            collect_rvalue_class_local_accesses(index, accesses)
        }
        mir::ClassExpression::MixedPayload { class, mixed, .. } => {
            accesses.assume_mixed_tag(*mixed, mir::MixedTag::Class(*class));
        }
        mir::ClassExpression::SharedPayload { reference, .. } => {
            collect_shared_reference_class_local_accesses(reference, accesses)
        }
        mir::ClassExpression::SharedAccessPayload { .. } => {}
    }
}

fn class_expression_accesses_local(expression: &mir::ClassExpression, local: mir::LocalId) -> bool {
    let mut accesses = ClassLocalAccesses::default();
    collect_class_expression_local_accesses(expression, &mut accesses);
    let accesses_local = accesses.iter().any(|access| match access {
        ClassLocalAccess::Borrow(accessed)
        | ClassLocalAccess::Transfer(accessed)
        | ClassLocalAccess::PropertyBorrow(accessed, _) => accessed == local,
        ClassLocalAccess::BeginCall | ClassLocalAccess::Call(_, _, _) => false,
    });
    accesses_local
}

fn nullable_class_expression_accesses_local(
    expression: &mir::NullableClassExpression,
    local: mir::LocalId,
) -> bool {
    let mut accesses = ClassLocalAccesses::default();
    collect_nullable_class_local_accesses(expression, &mut accesses);
    let accesses_local = accesses.iter().any(|access| match access {
        ClassLocalAccess::Borrow(accessed)
        | ClassLocalAccess::Transfer(accessed)
        | ClassLocalAccess::PropertyBorrow(accessed, _) => accessed == local,
        ClassLocalAccess::BeginCall | ClassLocalAccess::Call(_, _, _) => false,
    });
    accesses_local
}

fn collect_bool_class_local_accesses<'a>(
    value: &'a mir::BoolExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::BoolExpression::Use { operand } => {
            collect_operand_class_local_accesses(operand, accesses);
        }
        mir::BoolExpression::Compare { left, right, .. } => {
            collect_value_class_local_accesses(left, accesses);
            collect_value_class_local_accesses(right, accesses);
        }
        mir::BoolExpression::StringCompare { left, right, .. } => {
            collect_string_class_local_accesses(left, accesses);
            collect_string_class_local_accesses(right, accesses);
        }
        mir::BoolExpression::NullableStringCompare { left, right, .. } => {
            collect_nullable_string_class_local_accesses(left, accesses);
            collect_nullable_string_class_local_accesses(right, accesses);
        }
        mir::BoolExpression::NullableScalarIsPresent(value) => {
            collect_nullable_scalar_class_local_accesses(value, accesses);
        }
        mir::BoolExpression::NullableErrorIsPresent(value) => {
            collect_nullable_error_class_local_accesses(value, accesses);
        }
        mir::BoolExpression::NullableClassIsPresent(value) => {
            collect_nullable_class_local_accesses(value, accesses);
        }
        mir::BoolExpression::NullableCollectionIsPresent(value) => {
            collect_nullable_collection_class_local_accesses(value, accesses);
        }
        mir::BoolExpression::NullableSharedReferenceIsPresent(value) => {
            collect_nullable_shared_reference_class_local_accesses(value, accesses);
        }
        mir::BoolExpression::NullableWeakReferenceIsPresent(value) => {
            collect_nullable_weak_reference_class_local_accesses(value, accesses);
        }
        mir::BoolExpression::NullableWritableSharedReferenceIsPresent(value) => {
            collect_nullable_writable_shared_class_local_accesses(value, accesses);
        }
        mir::BoolExpression::NullableWritableWeakReferenceIsPresent(value) => {
            collect_nullable_writable_weak_class_local_accesses(value, accesses);
        }
        mir::BoolExpression::NullableSharedReferenceAccessIsPresent(value) => {
            collect_nullable_shared_access_class_local_accesses(value, accesses);
        }
        mir::BoolExpression::NullableMixedIsPresent(value) => {
            collect_nullable_mixed_class_local_accesses(value, accesses);
        }
        mir::BoolExpression::NullablePayloadEnumIsPresent(value) => {
            collect_nullable_payload_enum_class_local_accesses(value, accesses);
        }
        mir::BoolExpression::NullableFunctionIsPresent(value) => {
            collect_nullable_function_class_local_accesses(value, accesses);
        }
        mir::BoolExpression::PayloadEnumCompare { left, right, .. } => {
            collect_payload_enum_class_local_accesses(left, accesses);
            collect_payload_enum_class_local_accesses(right, accesses);
        }
        mir::BoolExpression::PayloadEnumIsCase { .. } => {}
        mir::BoolExpression::NullablePayloadEnumCompare { left, right, .. } => {
            collect_nullable_payload_enum_class_local_accesses(left, accesses);
            collect_nullable_payload_enum_class_local_accesses(right, accesses);
        }
        mir::BoolExpression::MixedIs { mixed, .. } => {
            collect_mixed_class_local_accesses(mixed, accesses);
        }
        mir::BoolExpression::Not(value) => {
            collect_bool_class_local_accesses(value, accesses);
        }
        mir::BoolExpression::Binary { op, left, right } => {
            collect_bool_class_local_accesses(left, accesses);
            if !matches!(
                (op, constant_bool_expression(left)),
                (mir::BoolBinaryOp::And, Some(false)) | (mir::BoolBinaryOp::Or, Some(true))
            ) {
                collect_bool_class_local_accesses(right, accesses);
            }
        }
        mir::BoolExpression::Call { function, args } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::BoolExpression::CollectionEqual { .. } => {}
        mir::BoolExpression::Coalesce { left, right } => {
            collect_nullable_scalar_class_local_accesses(left, accesses);
            collect_bool_class_local_accesses(right, accesses);
        }
        mir::BoolExpression::CollectionHas { value, .. } => {
            collect_rvalue_class_local_accesses(value, accesses)
        }
        mir::BoolExpression::CollectionIsEmpty { .. } => {}
    }
}

fn collect_nullable_scalar_class_local_accesses<'a>(
    value: &'a mir::NullableScalarExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::NullableScalarExpression::Value(value) => {
            collect_value_class_local_accesses(value, accesses)
        }
        mir::NullableScalarExpression::Property {
            object, property, ..
        } => accesses.borrow_property(*object, *property),
        mir::NullableScalarExpression::Call { function, args, .. } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::NullableScalarExpression::EnumBacking { value, .. } => {
            collect_nullable_scalar_class_local_accesses(value, accesses)
        }
        mir::NullableScalarExpression::NullSafeProperty { object, .. } => {
            collect_nullable_class_local_accesses(object, accesses)
        }
        mir::NullableScalarExpression::NullSafeCall {
            object,
            function,
            args,
            ..
        } => {
            collect_nullable_class_local_accesses(object, accesses);
            if !nullable_class_expression_is_definitely_null(object) {
                accesses.begin_call();
                collect_rvalue_args_class_local_accesses(args, accesses);
                accesses.method_call(*function, args);
            }
        }
        mir::NullableScalarExpression::Coalesce { left, right, .. } => {
            collect_nullable_scalar_class_local_accesses(left, accesses);
            collect_nullable_scalar_class_local_accesses(right, accesses);
        }
        mir::NullableScalarExpression::DictionaryGet { key, .. } => {
            collect_rvalue_class_local_accesses(key, accesses)
        }
        mir::NullableScalarExpression::CollectionIndexOf { value, .. } => {
            collect_rvalue_class_local_accesses(value, accesses)
        }
        mir::NullableScalarExpression::StringIntrinsic(call) => {
            collect_rvalue_args_class_local_accesses(&call.args, accesses)
        }
        mir::NullableScalarExpression::Parse { .. }
        | mir::NullableScalarExpression::Null(_)
        | mir::NullableScalarExpression::Local { .. }
        | mir::NullableScalarExpression::Static { .. } => {}
    }
}

fn collect_nullable_class_local_accesses<'a>(
    value: &'a mir::NullableClassExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match value {
        mir::NullableClassExpression::Class(value) => {
            collect_class_expression_local_accesses(value, accesses)
        }
        mir::NullableClassExpression::SharedPayload { reference, .. } => {
            collect_nullable_shared_reference_class_local_accesses(reference, accesses)
        }
        mir::NullableClassExpression::Local {
            local,
            transfer: true,
            ..
        } => accesses.transfer(*local),
        mir::NullableClassExpression::Local {
            local,
            transfer: false,
            ..
        } => accesses.borrow(*local),
        mir::NullableClassExpression::Property {
            object, property, ..
        } => accesses.borrow_property(*object, *property),
        mir::NullableClassExpression::Call { function, args, .. } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, accesses);
            accesses.call(*function, args);
        }
        mir::NullableClassExpression::NullSafeProperty { object, .. } => {
            collect_nullable_class_local_accesses(object, accesses)
        }
        mir::NullableClassExpression::NullSafeCall {
            object,
            function,
            args,
            ..
        } => {
            collect_nullable_class_local_accesses(object, accesses);
            if !nullable_class_expression_is_definitely_null(object) {
                accesses.begin_call();
                collect_rvalue_args_class_local_accesses(args, accesses);
                accesses.method_call(*function, args);
            }
        }
        mir::NullableClassExpression::Coalesce { left, right, .. } => {
            collect_nullable_class_local_accesses(left, accesses);
            collect_nullable_class_local_accesses(right, accesses);
        }
        mir::NullableClassExpression::DictionaryGet { key, .. } => {
            collect_rvalue_class_local_accesses(key, accesses)
        }
        mir::NullableClassExpression::Null(_) => {}
    }
}

fn collect_format_class_local_accesses<'a>(
    format: &'a mir::FormatExpression,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    for argument in format.pieces.iter().filter_map(|piece| match piece {
        crate::format_string::FormatPiece::Argument { index, .. } => {
            format.arguments.get(*index as usize)
        }
        crate::format_string::FormatPiece::Literal(_) => None,
    }) {
        match argument {
            mir::FormatArgument::Value(value) => {
                collect_value_class_local_accesses(value, accesses)
            }
            mir::FormatArgument::String(value) | mir::FormatArgument::ClassDisplay(value) => {
                collect_string_class_local_accesses(value, accesses)
            }
        }
    }
}

fn collect_statement_class_local_accesses(statement: &mir::Statement) -> ClassLocalAccesses<'_> {
    let mut accesses = ClassLocalAccesses::default();
    match statement {
        mir::Statement::BindClosureEnvironment { .. }
        | mir::Statement::BindPayloadEnumFields { .. }
        | mir::Statement::MatchResultPlan { .. }
        | mir::Statement::ControlFlowPlan(_) => {}
        mir::Statement::AssignLocal { value, .. }
        | mir::Statement::AssignLocalGroup { value, .. }
        | mir::Statement::AssignStatic { value, .. } => {
            collect_rvalue_class_local_accesses(value, &mut accesses);
        }
        mir::Statement::EchoString(value) | mir::Statement::WriteStderr(value) => {
            collect_string_class_local_accesses(value, &mut accesses);
        }
        mir::Statement::CallVoid { function, args, .. }
        | mir::Statement::CallBorrowed { function, args, .. } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, &mut accesses);
            accesses.call(*function, args);
        }
        mir::Statement::CallNullSafe {
            object,
            function,
            args,
            ..
        } => {
            collect_nullable_class_local_accesses(object, &mut accesses);
            if !nullable_class_expression_is_definitely_null(object) {
                accesses.begin_call();
                collect_rvalue_args_class_local_accesses(args, &mut accesses);
                accesses.method_call(*function, args);
            }
        }
        mir::Statement::Printf(format) => {
            collect_format_class_local_accesses(format, &mut accesses);
        }
        mir::Statement::WriteFile { path, contents }
        | mir::Statement::AppendFile { path, contents } => {
            collect_string_class_local_accesses(path, &mut accesses);
            collect_string_class_local_accesses(contents, &mut accesses);
        }
        mir::Statement::WriteFileBytes { path, .. } => {
            collect_string_class_local_accesses(path, &mut accesses);
        }
        mir::Statement::AssignProperty { object, value, .. } => {
            collect_rvalue_class_local_accesses(value, &mut accesses);
            accesses.borrow(*object);
        }
        mir::Statement::CollectionAdd { value, .. } => {
            collect_rvalue_class_local_accesses(value, &mut accesses)
        }
        mir::Statement::CollectionSet { key, value, .. } => {
            collect_rvalue_class_local_accesses(key, &mut accesses);
            collect_rvalue_class_local_accesses(value, &mut accesses);
        }
        mir::Statement::AssignCollectionIndex { index, value, .. } => {
            collect_rvalue_class_local_accesses(index, &mut accesses);
            collect_rvalue_class_local_accesses(value, &mut accesses);
        }
        mir::Statement::CollectionClear { .. } => {}
        mir::Statement::EchoStringLiteral(_)
        | mir::Statement::DropClass { .. }
        | mir::Statement::DropString { .. }
        | mir::Statement::DropMixed { .. }
        | mir::Statement::EnsureErrorOrigin { .. }
        | mir::Statement::ExtractErrorObject { .. }
        | mir::Statement::DropError { .. }
        | mir::Statement::DropCollection { .. }
        | mir::Statement::DropPayloadEnum { .. }
        | mir::Statement::DropFunction { .. }
        | mir::Statement::DropSharedReference { .. }
        | mir::Statement::DropWeakReference { .. }
        | mir::Statement::DropWritableSharedReference { .. }
        | mir::Statement::DropWritableWeakReference { .. }
        | mir::Statement::DropSharedReferenceAccess { .. }
        | mir::Statement::WriteStreamBytes { .. } => {}
    }
    accesses
}

fn collect_terminator_class_local_accesses(terminator: &mir::Terminator) -> ClassLocalAccesses<'_> {
    let mut accesses = ClassLocalAccesses::default();
    match terminator {
        mir::Terminator::Return(value) => {
            collect_rvalue_class_local_accesses(value, &mut accesses);
        }
        mir::Terminator::Panic { message: value, .. } => {
            collect_string_class_local_accesses(value, &mut accesses);
        }
        mir::Terminator::Branch { condition, .. } => {
            collect_bool_class_local_accesses(condition, &mut accesses);
        }
        mir::Terminator::CheckedCall { function, args, .. } => {
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, &mut accesses);
            accesses.call(*function, args);
        }
        mir::Terminator::IndirectCall { callee, args, .. }
        | mir::Terminator::CheckedIndirectCall { callee, args, .. } => {
            collect_function_class_local_accesses(callee, &mut accesses);
            collect_rvalue_args_class_local_accesses(args, &mut accesses);
        }
        mir::Terminator::CheckedConstruct {
            properties,
            constructor,
            args,
            ..
        } => {
            for property in properties {
                if let mir::PropertyValueSource::Expression(value) = &property.source {
                    collect_rvalue_class_local_accesses(value, &mut accesses);
                }
            }
            accesses.begin_call();
            collect_rvalue_args_class_local_accesses(args, &mut accesses);
            accesses.constructor_call(*constructor, args);
        }
        mir::Terminator::CheckedIo { operation, .. } => {
            collect_checked_io_class_local_accesses(operation, &mut accesses);
        }
        mir::Terminator::ReturnVoid
        | mir::Terminator::Unreachable
        | mir::Terminator::Jump(_)
        | mir::Terminator::ErrorSwitch { .. }
        | mir::Terminator::PropagateError { .. } => {}
    }
    accesses
}

fn collect_checked_io_class_local_accesses<'a>(
    operation: &'a mir::CheckedIoOperation,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match operation {
        mir::CheckedIoOperation::ReadLine { prompt } => {
            collect_string_class_local_accesses(prompt, accesses)
        }
        mir::CheckedIoOperation::ReadFile { path, .. } => {
            collect_string_class_local_accesses(path, accesses)
        }
        mir::CheckedIoOperation::ReadStdinBytes => {}
        mir::CheckedIoOperation::WriteFile { path, contents, .. } => {
            collect_string_class_local_accesses(path, accesses);
            collect_io_contents_class_local_accesses(contents, accesses);
        }
        mir::CheckedIoOperation::WriteStream { contents, .. } => {
            collect_io_contents_class_local_accesses(contents, accesses)
        }
    }
}

fn collect_io_contents_class_local_accesses<'a>(
    contents: &'a mir::IoContents,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match contents {
        mir::IoContents::String(value) => collect_string_class_local_accesses(value, accesses),
        mir::IoContents::Format(value) => collect_format_class_local_accesses(value, accesses),
        mir::IoContents::Bytes(_) => {}
    }
}

fn reachable_blocks_and_predecessors(
    function: &mir::Function,
    fold_constant_branches: bool,
) -> Result<(Vec<bool>, Vec<Vec<mir::BlockId>>), BackendError> {
    let mut reachable = vec![false; function.blocks.len()];
    let mut pending = vec![function.entry_block];
    while let Some(block_id) = pending.pop() {
        let block = block_in(function, block_id)?;
        if std::mem::replace(&mut reachable[block_id.0], true) {
            continue;
        }
        pending.extend(analysis_terminator_targets(
            &block.terminator,
            fold_constant_branches,
        ));
    }

    let mut predecessors = vec![Vec::new(); function.blocks.len()];
    for block in function.blocks.iter().filter(|block| reachable[block.id.0]) {
        for target in analysis_terminator_targets(&block.terminator, fold_constant_branches) {
            block_in(function, target)?;
            predecessors[target.0].push(block.id);
        }
    }
    Ok((reachable, predecessors))
}

fn apply_class_local_state(
    function: &mir::Function,
    statement: &mir::Statement,
    moved: &mut HashSet<mir::LocalId>,
    alias_invalidations: &[PropertyAliasInvalidation],
    validate: bool,
) -> Result<(), BackendError> {
    let accesses = collect_statement_class_local_accesses(statement);
    if validate {
        if let mir::Statement::AssignLocal { target, .. } = statement {
            if accesses
                .transferred()
                .any(|transferred| transferred == *target)
            {
                return Err(malformed_mir(format!(
                    "function {} assigns class local local{} from an overlapping transfer",
                    function.name, target.0
                )));
            }
        }
    }
    apply_class_local_accesses(function, &accesses, moved, validate)?;
    if let mir::Statement::AssignProperty {
        object, property, ..
    } = statement
    {
        for invalidation in alias_invalidations {
            if invalidation.receiver == *object && invalidation.property == *property {
                moved.insert(invalidation.alias);
            }
        }
    }
    match statement {
        mir::Statement::AssignLocal { target, .. }
            if matches!(
                local_in(function, *target)?.ty,
                mir::Type::Class(_) | mir::Type::NullableClass(_)
            ) =>
        {
            moved.remove(target);
        }
        mir::Statement::AssignLocalGroup { targets, .. } => {
            for target in targets {
                if matches!(
                    local_in(function, *target)?.ty,
                    mir::Type::Class(_) | mir::Type::NullableClass(_)
                ) {
                    moved.remove(target);
                }
            }
        }
        mir::Statement::DropClass { local, .. } => {
            moved.insert(*local);
        }
        _ => {}
    }
    Ok(())
}

fn apply_class_local_accesses(
    function: &mir::Function,
    accesses: &ClassLocalAccesses,
    moved: &mut HashSet<mir::LocalId>,
    validate: bool,
) -> Result<(), BackendError> {
    for access in accesses.iter() {
        let (local, action) = match access {
            ClassLocalAccess::Borrow(local) | ClassLocalAccess::PropertyBorrow(local, _) => {
                (local, "uses")
            }
            ClassLocalAccess::Transfer(local) => (local, "transfers"),
            ClassLocalAccess::BeginCall | ClassLocalAccess::Call(_, _, _) => continue,
        };
        if validate && moved.contains(&local) {
            return Err(malformed_mir(format!(
                "function {} {action} class local local{} after its ownership ended",
                function.name, local.0
            )));
        }
        if matches!(access, ClassLocalAccess::Transfer(_)) {
            moved.insert(local);
        }
    }
    Ok(())
}

fn validate_nullable_presence(
    program: &mir::Program,
    function: &mir::Function,
) -> Result<(), BackendError> {
    let mut entries = vec![None; function.blocks.len()];
    entries[function.entry_block.0] = Some(HashSet::new());
    let mut pending = VecDeque::from([function.entry_block]);

    while let Some(block_id) = pending.pop_front() {
        let block = block_in(function, block_id)?;
        let Some(mut present) = entries[block_id.0].clone() else {
            continue;
        };
        for statement in &block.statements {
            apply_nullable_presence_statement(program, function, statement, &mut present)?;
        }

        apply_nullable_class_call_effects(
            program,
            function,
            &collect_terminator_class_local_accesses(&block.terminator),
            &mut present,
        )?;

        match &block.terminator {
            mir::Terminator::Jump(target) => {
                if merge_definitely_present(&mut entries[target.0], &present) {
                    pending.push_back(*target);
                }
            }
            mir::Terminator::IndirectCall { continuation, .. } => {
                if merge_definitely_present(&mut entries[continuation.0], &present) {
                    pending.push_back(*continuation);
                }
            }
            mir::Terminator::Branch {
                condition,
                then_block,
                else_block,
            } => {
                for (target, condition_value) in [(*then_block, true), (*else_block, false)] {
                    if constant_bool_expression(condition)
                        .is_some_and(|value| value != condition_value)
                    {
                        continue;
                    }
                    let mut outgoing = present.clone();
                    apply_nullable_presence_condition(condition, condition_value, &mut outgoing);
                    if merge_definitely_present(&mut entries[target.0], &outgoing) {
                        pending.push_back(target);
                    }
                }
            }
            mir::Terminator::CheckedCall {
                success, failure, ..
            }
            | mir::Terminator::CheckedIndirectCall {
                success, failure, ..
            }
            | mir::Terminator::CheckedConstruct {
                success, failure, ..
            }
            | mir::Terminator::CheckedIo {
                success, failure, ..
            } => {
                for target in [*success, *failure] {
                    if merge_definitely_present(&mut entries[target.0], &present) {
                        pending.push_back(target);
                    }
                }
            }
            mir::Terminator::ErrorSwitch {
                cases,
                catch_all,
                fallback,
                ..
            } => {
                let targets = cases
                    .iter()
                    .map(|(_, target)| *target)
                    .chain(catch_all.iter().copied())
                    .chain(std::iter::once(*fallback));
                for target in targets {
                    if merge_definitely_present(&mut entries[target.0], &present) {
                        pending.push_back(target);
                    }
                }
            }
            mir::Terminator::Return(_)
            | mir::Terminator::ReturnVoid
            | mir::Terminator::Panic { .. }
            | mir::Terminator::Unreachable
            | mir::Terminator::PropagateError { .. } => {}
        }
    }

    for block in &function.blocks {
        let Some(mut present) = entries[block.id.0].clone() else {
            continue;
        };
        for statement in &block.statements {
            validate_nullable_assumptions(
                function,
                &collect_statement_class_local_accesses(statement),
                &present,
            )?;
            apply_nullable_presence_statement(program, function, statement, &mut present)?;
        }
        validate_nullable_assumptions(
            function,
            &collect_terminator_class_local_accesses(&block.terminator),
            &present,
        )?;
    }

    Ok(())
}

fn validate_mixed_tag_proofs(
    _program: &mir::Program,
    function: &mir::Function,
) -> Result<(), BackendError> {
    let mut entries = vec![None; function.blocks.len()];
    entries[function.entry_block.0] = Some(HashMap::new());
    let mut pending = VecDeque::from([function.entry_block]);

    while let Some(block_id) = pending.pop_front() {
        let block = block_in(function, block_id)?;
        let Some(mut tags) = entries[block_id.0].clone() else {
            continue;
        };
        for statement in &block.statements {
            apply_mixed_tag_statement(function, statement, &mut tags)?;
        }

        match &block.terminator {
            mir::Terminator::Jump(target) => {
                if merge_definite_mixed_tags(&mut entries[target.0], &tags) {
                    pending.push_back(*target);
                }
            }
            mir::Terminator::IndirectCall { continuation, .. } => {
                if merge_definite_mixed_tags(&mut entries[continuation.0], &tags) {
                    pending.push_back(*continuation);
                }
            }
            mir::Terminator::Branch {
                condition,
                then_block,
                else_block,
            } => {
                for (target, condition_value) in [(*then_block, true), (*else_block, false)] {
                    if constant_bool_expression(condition)
                        .is_some_and(|value| value != condition_value)
                    {
                        continue;
                    }
                    let mut outgoing = tags.clone();
                    apply_mixed_tag_condition(condition, condition_value, &mut outgoing);
                    if merge_definite_mixed_tags(&mut entries[target.0], &outgoing) {
                        pending.push_back(target);
                    }
                }
            }
            mir::Terminator::CheckedCall {
                success, failure, ..
            }
            | mir::Terminator::CheckedIndirectCall {
                success, failure, ..
            }
            | mir::Terminator::CheckedConstruct {
                success, failure, ..
            }
            | mir::Terminator::CheckedIo {
                success, failure, ..
            } => {
                for target in [*success, *failure] {
                    if merge_definite_mixed_tags(&mut entries[target.0], &tags) {
                        pending.push_back(target);
                    }
                }
            }
            mir::Terminator::ErrorSwitch {
                cases,
                catch_all,
                fallback,
                ..
            } => {
                let targets = cases
                    .iter()
                    .map(|(_, target)| *target)
                    .chain(catch_all.iter().copied())
                    .chain(std::iter::once(*fallback));
                for target in targets {
                    if merge_definite_mixed_tags(&mut entries[target.0], &tags) {
                        pending.push_back(target);
                    }
                }
            }
            mir::Terminator::Return(_)
            | mir::Terminator::ReturnVoid
            | mir::Terminator::Panic { .. }
            | mir::Terminator::Unreachable
            | mir::Terminator::PropagateError { .. } => {}
        }
    }

    for block in &function.blocks {
        let Some(mut tags) = entries[block.id.0].clone() else {
            continue;
        };
        for statement in &block.statements {
            validate_mixed_tag_assumptions(
                function,
                &collect_statement_class_local_accesses(statement),
                &tags,
            )?;
            apply_mixed_tag_statement(function, statement, &mut tags)?;
        }
        validate_mixed_tag_assumptions(
            function,
            &collect_terminator_class_local_accesses(&block.terminator),
            &tags,
        )?;
    }

    Ok(())
}

fn validate_payload_case_proofs(
    _program: &mir::Program,
    function: &mir::Function,
) -> Result<(), BackendError> {
    let mut entries = vec![None; function.blocks.len()];
    entries[function.entry_block.0] = Some(HashMap::new());
    let mut pending = VecDeque::from([function.entry_block]);

    while let Some(block_id) = pending.pop_front() {
        let block = block_in(function, block_id)?;
        let Some(mut cases) = entries[block_id.0].clone() else {
            continue;
        };
        for statement in &block.statements {
            apply_payload_case_statement(function, statement, &mut cases)?;
        }

        match &block.terminator {
            mir::Terminator::Jump(target) => {
                if merge_definite_payload_cases(&mut entries[target.0], &cases) {
                    pending.push_back(*target);
                }
            }
            mir::Terminator::IndirectCall { continuation, .. } => {
                if merge_definite_payload_cases(&mut entries[continuation.0], &cases) {
                    pending.push_back(*continuation);
                }
            }
            mir::Terminator::Branch {
                condition,
                then_block,
                else_block,
            } => {
                for (target, condition_value) in [(*then_block, true), (*else_block, false)] {
                    if constant_bool_expression(condition)
                        .is_some_and(|value| value != condition_value)
                    {
                        continue;
                    }
                    let mut outgoing = cases.clone();
                    apply_payload_case_condition(condition, condition_value, &mut outgoing);
                    if merge_definite_payload_cases(&mut entries[target.0], &outgoing) {
                        pending.push_back(target);
                    }
                }
            }
            mir::Terminator::CheckedCall {
                success, failure, ..
            }
            | mir::Terminator::CheckedIndirectCall {
                success, failure, ..
            }
            | mir::Terminator::CheckedConstruct {
                success, failure, ..
            }
            | mir::Terminator::CheckedIo {
                success, failure, ..
            } => {
                for target in [*success, *failure] {
                    if merge_definite_payload_cases(&mut entries[target.0], &cases) {
                        pending.push_back(target);
                    }
                }
            }
            mir::Terminator::ErrorSwitch {
                cases: dispatch_cases,
                catch_all,
                fallback,
                ..
            } => {
                let targets = dispatch_cases
                    .iter()
                    .map(|(_, target)| *target)
                    .chain(catch_all.iter().copied())
                    .chain(std::iter::once(*fallback));
                for target in targets {
                    if merge_definite_payload_cases(&mut entries[target.0], &cases) {
                        pending.push_back(target);
                    }
                }
            }
            mir::Terminator::Return(_)
            | mir::Terminator::ReturnVoid
            | mir::Terminator::Panic { .. }
            | mir::Terminator::Unreachable
            | mir::Terminator::PropagateError { .. } => {}
        }
    }

    for block in &function.blocks {
        let Some(mut cases) = entries[block.id.0].clone() else {
            continue;
        };
        for statement in &block.statements {
            if let mir::Statement::BindPayloadEnumFields { source, case, .. } = statement {
                if cases.get(source) != Some(case) {
                    return Err(malformed_mir(format!(
                        "payload enum local local{} is destructured without a dominating exact case proof",
                        source.0
                    )));
                }
            }
            apply_payload_case_statement(function, statement, &mut cases)?;
        }
    }

    Ok(())
}

fn validate_match_result_plans(function: &mir::Function) -> Result<(), BackendError> {
    for block in &function.blocks {
        for statement in &block.statements {
            let mir::Statement::MatchResultPlan {
                result,
                arms,
                merge,
                ..
            } = statement
            else {
                continue;
            };
            for arm in arms {
                validate_match_arm_result_path(function, *result, arm.binding, *merge)?;
            }
        }
    }
    Ok(())
}

fn validate_control_flow_plans(
    program: &mir::Program,
    function: &mir::Function,
) -> Result<(), BackendError> {
    let finalizer_plans = function
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter_map(|statement| match statement {
            mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::Finalizer(plan)) => Some(plan),
            _ => None,
        })
        .collect::<Vec<_>>();
    let finalizer_ids = finalizer_plans
        .iter()
        .map(|plan| plan.id)
        .collect::<HashSet<_>>();
    if finalizer_ids.len() != finalizer_plans.len() {
        return Err(malformed_mir(
            "finalizer region identifiers are duplicated within the function",
        ));
    }
    if finalizer_ids.iter().any(|id| id.0 >= finalizer_ids.len()) {
        return Err(malformed_mir(
            "finalizer region identifiers are not dense within the function",
        ));
    }
    for block in &function.blocks {
        for statement in &block.statements {
            let mir::Statement::ControlFlowPlan(plan) = statement else {
                continue;
            };
            match plan {
                mir::ControlFlowPlan::Given(plan) => {
                    if block.id != plan.setup_entry {
                        return Err(malformed_mir(
                            "given execution plan is not anchored in its setup entry",
                        ));
                    }
                    let first_gate = plan
                        .predicates
                        .first()
                        .map(|predicate| predicate.block)
                        .unwrap_or(plan.condition);
                    if !cfg_reaches(function, plan.setup_exit, first_gate)? {
                        return Err(malformed_mir(
                            "given setup does not lead to its predicate phase",
                        ));
                    }
                    if plan.predicates.is_empty() != plan.gate_failed.is_none() {
                        return Err(malformed_mir(
                            "given gate-failure target disagrees with its predicate phase",
                        ));
                    }
                    for (index, predicate) in plan.predicates.iter().enumerate() {
                        if predicate.ty != mir::Type::Scalar(mir::ScalarType::Bool) {
                            return Err(malformed_mir("given predicate does not have bool type"));
                        }
                        let next = plan
                            .predicates
                            .get(index + 1)
                            .map(|predicate| predicate.block)
                            .unwrap_or(plan.condition);
                        let gate_failed = plan
                            .gate_failed
                            .expect("a non-empty given predicate plan has a false target");
                        if !checked_success_reaches_bool_control(function, predicate.block)?
                            || (!cfg_reaches(function, predicate.block, next)?
                                && !cfg_reaches(function, predicate.block, gate_failed)?)
                        {
                            return Err(malformed_mir(
                                "given predicate chain does not preserve source-order short-circuiting",
                            ));
                        }
                    }
                    if plan.condition_type != mir::Type::Scalar(mir::ScalarType::Bool)
                        || !checked_success_reaches_bool_control(function, plan.condition)?
                    {
                        return Err(malformed_mir(
                            "given attached condition is not represented by bool control flow",
                        ));
                    }
                    let continue_target = plan
                        .predicates
                        .first()
                        .map(|predicate| predicate.block)
                        .unwrap_or(plan.condition);
                    if matches!(plan.attachment, mir::GivenAttachment::While) {
                        for source in &plan.continue_sources {
                            if !matches!(
                                block_in(function, *source)?.terminator,
                                mir::Terminator::Jump(target) if target == continue_target
                            ) {
                                return Err(malformed_mir(
                                    "given while continue skips predicate reevaluation",
                                ));
                            }
                        }
                    } else if !plan.continue_sources.is_empty() {
                        return Err(malformed_mir(
                            "non-loop given plan contains continue sources",
                        ));
                    }
                }
                mir::ControlFlowPlan::When(plan) => {
                    for branch in &plan.branches {
                        validate_result_path(
                            function,
                            plan.result,
                            *branch,
                            plan.merge,
                            "when branch",
                        )?;
                    }
                }
                mir::ControlFlowPlan::DoWhile(plan) => {
                    if block.id != plan.entry
                        || !matches!(
                            block_in(function, plan.entry)?.terminator,
                            mir::Terminator::Jump(target) if target == plan.body
                        )
                    {
                        return Err(malformed_mir(
                            "do-while body is not entered before its first condition",
                        ));
                    }
                    let condition = block_in(function, plan.condition)?;
                    if plan.condition_type != mir::Type::Scalar(mir::ScalarType::Bool)
                        || !matches!(
                            condition.terminator,
                            mir::Terminator::Branch {
                                then_block,
                                else_block,
                                ..
                            } if then_block == plan.body && else_block == plan.exit
                        ) && !matches!(
                            condition.terminator,
                            mir::Terminator::Jump(target)
                                if target == plan.body || target == plan.exit
                        )
                    {
                        return Err(malformed_mir(
                            "do-while condition is not bool control flow between its body and exit",
                        ));
                    }
                    for source in &plan.continue_sources {
                        if !matches!(
                            block_in(function, *source)?.terminator,
                            mir::Terminator::Jump(target) if target == plan.condition
                        ) {
                            return Err(malformed_mir(
                                "do-while continue does not target its condition",
                            ));
                        }
                    }
                }
                mir::ControlFlowPlan::Finalizer(plan) => {
                    if let Some(parent) = plan.parent {
                        if parent.0 >= plan.id.0 || !finalizer_ids.contains(&parent) {
                            return Err(malformed_mir(
                                "finalizer region has an invalid lexical parent",
                            ));
                        }
                    }
                    validate_finalizer_plan(function, block.id, plan)?;
                }
                mir::ControlFlowPlan::ListAlgorithm(plan) => {
                    validate_list_algorithm_cfg(program, function, block.id, plan)?;
                }
            }
        }
    }
    Ok(())
}

fn validate_list_algorithm_types(
    program: &mir::Program,
    function: &mir::Function,
    plan: &mir::ListAlgorithmPlan,
) -> Result<(), BackendError> {
    let source_collection = collection_in(program, plan.source_collection)?;
    if source_collection.kind != mir::CollectionKind::List
        || source_collection.key.is_some()
        || source_collection.value != plan.element_type
        || local_in(function, plan.source)?.ty != mir::Type::Collection(plan.source_collection)
    {
        return Err(malformed_mir(
            "List algorithm source metadata does not describe its concrete List",
        ));
    }
    let callback = function_type_in(program, plan.callback_type)?;
    if callback.invocation_mode != plan.callback_access
        || matches!(callback.invocation_mode, mir::FunctionInvocationMode::Once)
        || callback.checked_effects != plan.checked_effects
    {
        return Err(malformed_mir(
            "List algorithm callback mode or checked effects disagree with its function type",
        ));
    }
    let callback_local = local_in(function, plan.callback)?;
    if callback_local.ty != mir::Type::Function(plan.callback_type)
        || callback_local.writable
            != matches!(plan.callback_access, mir::FunctionInvocationMode::Writable)
    {
        return Err(malformed_mir(
            "List algorithm callback local disagrees with its selected access",
        ));
    }
    let count = local_in(function, plan.count)?;
    let index = local_in(function, plan.index)?;
    let doria_int = mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64));
    if count.ty != doria_int
        || index.ty != doria_int
        || !count.synthetic
        || !index.synthetic
        || count.writable
        || !index.writable
    {
        return Err(malformed_mir(
            "List algorithm count and index locals must be concrete Doria int traversal state",
        ));
    }
    validate_checked_effects(program, &plan.checked_effects)?;

    match plan.kind {
        mir::ListAlgorithmKind::Map => {
            let [parameter] = callback.parameters.as_slice() else {
                return Err(malformed_mir("List::map callback must have one parameter"));
            };
            let mir::ReturnType::Value(mapped) = callback.return_type else {
                return Err(malformed_mir("List::map callback cannot return void"));
            };
            let mir::Type::Collection(result_collection) = plan.result_type else {
                return Err(malformed_mir("List::map result must be a List"));
            };
            let result_collection = collection_in(program, result_collection)?;
            if parameter.mode != mir::FunctionParameterMode::Readonly
                || parameter.ty != plan.element_type
                || callback.return_borrow.is_some()
                || result_collection.kind != mir::CollectionKind::List
                || result_collection.key.is_some()
                || result_collection.value != mapped
                || plan.accumulator_type.is_some()
                || plan.accumulator.is_some()
                || plan.callback_result.is_none()
                || plan.filter_selected.is_some()
            {
                return Err(malformed_mir(
                    "List::map specialization has an invalid shape",
                ));
            }
        }
        mir::ListAlgorithmKind::Filter => {
            let [parameter] = callback.parameters.as_slice() else {
                return Err(malformed_mir(
                    "List::filter callback must have one parameter",
                ));
            };
            if parameter.mode != mir::FunctionParameterMode::Readonly
                || parameter.ty != plan.element_type
                || callback.return_type
                    != mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Bool))
                || plan.element_type.has_move_ownership()
                || plan.result_type != mir::Type::Collection(plan.source_collection)
                || plan.accumulator_type.is_some()
                || plan.accumulator.is_some()
                || plan.callback_result.is_none()
                || plan.filter_selected.is_none()
            {
                return Err(malformed_mir(
                    "List::filter specialization has an invalid Copy-preserving shape",
                ));
            }
        }
        mir::ListAlgorithmKind::Reduce => {
            let [accumulator, element] = callback.parameters.as_slice() else {
                return Err(malformed_mir(
                    "List::reduce callback must have two parameters",
                ));
            };
            if accumulator.mode != mir::FunctionParameterMode::Writable
                || accumulator.ty != plan.result_type
                || element.mode != mir::FunctionParameterMode::Readonly
                || element.ty != plan.element_type
                || callback.return_type != mir::ReturnType::Void
                || callback.return_borrow.is_some()
                || plan.accumulator_type != Some(plan.result_type)
                || plan.callback_result.is_some()
                || plan.filter_selected.is_some()
            {
                return Err(malformed_mir(
                    "List::reduce specialization has an invalid shape",
                ));
            }
        }
    }

    match plan.kind {
        mir::ListAlgorithmKind::Map | mir::ListAlgorithmKind::Filter => {
            let output = plan
                .output
                .ok_or_else(|| malformed_mir("List algorithm result local is missing"))?;
            let output = local_in(function, output)?;
            if output.ty != plan.result_type || !output.synthetic || !output.writable {
                return Err(malformed_mir(
                    "List algorithm result local has incompatible type or ownership",
                ));
            }
        }
        mir::ListAlgorithmKind::Reduce => {
            if plan.output.is_some() {
                return Err(malformed_mir("List::reduce cannot declare a result List"));
            }
            let accumulator = plan
                .accumulator
                .ok_or_else(|| malformed_mir("List::reduce accumulator local is missing"))?;
            let accumulator = local_in(function, accumulator)?;
            if accumulator.ty != plan.result_type || !accumulator.synthetic || !accumulator.writable
            {
                return Err(malformed_mir(
                    "List::reduce accumulator local has incompatible type or ownership",
                ));
            }
        }
    }
    Ok(())
}

fn validate_list_algorithm_cfg(
    program: &mir::Program,
    function: &mir::Function,
    anchor: mir::BlockId,
    plan: &mir::ListAlgorithmPlan,
) -> Result<(), BackendError> {
    validate_list_algorithm_types(program, function, plan)?;
    for id in [
        plan.setup,
        plan.header,
        plan.body,
        plan.callback_success,
        plan.update,
        plan.exit,
    ] {
        block_in(function, id)?;
    }
    if let Some(block) = plan.callback_failure {
        block_in(function, block)?;
    }
    if let Some(block) = plan.filter_selected {
        block_in(function, block)?;
    }
    let setup = block_in(function, plan.setup)?;
    let initializes_count = setup.statements.iter().any(|statement| {
        matches!(
            statement,
            mir::Statement::AssignLocal {
                target,
                value: mir::Rvalue::Value(mir::ValueExpression::Integer(
                    mir::IntegerExpression::Use {
                        ty: IntegerType::Int64,
                        operand: mir::Operand::CollectionLength(source),
                    },
                )),
            } if *target == plan.count && *source == plan.source
        )
    });
    let initializes_index = setup.statements.iter().any(|statement| {
        matches!(
            statement,
            mir::Statement::AssignLocal {
                target,
                value: mir::Rvalue::Value(mir::ValueExpression::Integer(
                    mir::IntegerExpression::Use {
                        ty: IntegerType::Int64,
                        operand: mir::Operand::Scalar(mir::ScalarValue::Integer(value)),
                    },
                )),
            } if *target == plan.index && value.bits == 0
        )
    });
    let header_condition = matches!(
        &block_in(function, plan.header)?.terminator,
        mir::Terminator::Branch {
            condition: mir::BoolExpression::Compare {
                op: mir::CompareOp::Less,
                left,
                right,
            },
            then_block,
            else_block,
        } if *then_block == plan.body
            && *else_block == plan.exit
            && value_expression_reads_integer_local(left, plan.index)
            && value_expression_reads_integer_local(right, plan.count)
    );
    let increments_index = block_in(function, plan.update)?
        .statements
        .iter()
        .any(|statement| {
            matches!(
                statement,
                mir::Statement::AssignLocal {
                    target,
                    value: mir::Rvalue::Value(mir::ValueExpression::Integer(
                        mir::IntegerExpression::Binary {
                            ty: IntegerType::Int64,
                            op: mir::IntegerBinaryOp::Add,
                            left,
                            right,
                            ..
                        },
                    )),
                } if *target == plan.index
                    && integer_expression_reads_local(left, plan.index)
                    && integer_expression_is_constant(right, 1)
            )
        });
    if anchor != plan.setup
        || !initializes_count
        || !initializes_index
        || !header_condition
        || !increments_index
        || !matches!(
            block_in(function, plan.setup)?.terminator,
            mir::Terminator::Jump(target) if target == plan.header
        )
        || !matches!(
            block_in(function, plan.update)?.terminator,
            mir::Terminator::Jump(target) if target == plan.header
        )
        || !cfg_reaches(function, plan.callback_success, plan.update)?
    {
        return Err(malformed_mir(
            "List algorithm plan does not describe its traversal CFG",
        ));
    }
    let callback_shape = match &block_in(function, plan.body)?.terminator {
        mir::Terminator::IndirectCall {
            callee,
            function_type,
            invocation_mode,
            args,
            result,
            continuation,
            ..
        } if plan.checked_effects.is_empty() => {
            if plan.callback_failure.is_some() {
                return Err(malformed_mir(
                    "nonthrowing List algorithm declares a checked failure continuation",
                ));
            }
            (
                *function_type,
                *invocation_mode,
                callee,
                args,
                *result,
                *continuation,
                None,
            )
        }
        mir::Terminator::CheckedIndirectCall {
            callee,
            function_type,
            invocation_mode,
            args,
            result,
            success,
            failure,
            ..
        } if !plan.checked_effects.is_empty() => (
            *function_type,
            *invocation_mode,
            callee,
            args,
            *result,
            *success,
            Some(*failure),
        ),
        _ => {
            return Err(malformed_mir(
                "List algorithm body must invoke its callback with matching checkedness",
            ))
        }
    };
    let (function_type, invocation_mode, callee, args, result, success, failure) = callback_shape;
    if function_type != plan.callback_type
        || invocation_mode != plan.callback_access
        || result != plan.callback_result
        || failure != plan.callback_failure
        || success != plan.callback_success
        || !matches!(
            callee,
            mir::FunctionExpression::Local {
                function_type,
                local,
                transfer: false,
            } if *function_type == plan.callback_type && *local == plan.callback
        )
    {
        return Err(malformed_mir(
            "List algorithm callback terminator disagrees with its specialization",
        ));
    }
    match plan.kind {
        mir::ListAlgorithmKind::Map | mir::ListAlgorithmKind::Filter => {
            if args.len() != 1 {
                return Err(malformed_mir(
                    "List map/filter callback must receive exactly one element borrow",
                ));
            }
        }
        mir::ListAlgorithmKind::Reduce => {
            if args.len() != 2 || args[0].direct_place_local() != plan.accumulator {
                return Err(malformed_mir(
                    "List::reduce callback must receive its accumulator place before the element borrow",
                ));
            }
        }
    }
    validate_list_algorithm_success_path(function, plan)?;
    validate_list_algorithm_checked_cleanup(function, plan)?;
    validate_list_algorithm_region_does_not_mutate_sources(function, plan)?;
    Ok(())
}

fn integer_expression_reads_local(
    expression: &mir::IntegerExpression,
    local: mir::LocalId,
) -> bool {
    matches!(
        expression,
        mir::IntegerExpression::Use {
            ty: IntegerType::Int64,
            operand: mir::Operand::Local(candidate),
        } if *candidate == local
    )
}

fn value_expression_reads_integer_local(
    expression: &mir::ValueExpression,
    local: mir::LocalId,
) -> bool {
    matches!(
        expression,
        mir::ValueExpression::Integer(value) if integer_expression_reads_local(value, local)
    )
}

fn integer_expression_is_constant(expression: &mir::IntegerExpression, value: u64) -> bool {
    matches!(
        expression,
        mir::IntegerExpression::Use {
            ty: IntegerType::Int64,
            operand: mir::Operand::Scalar(mir::ScalarValue::Integer(candidate)),
        } if candidate.bits == value
    )
}

fn validate_list_algorithm_success_path(
    function: &mir::Function,
    plan: &mir::ListAlgorithmPlan,
) -> Result<(), BackendError> {
    let success = block_in(function, plan.callback_success)?;
    match plan.kind {
        mir::ListAlgorithmKind::Map => {
            let output = plan.output.expect("validated map output");
            let result = plan.callback_result.expect("validated map callback result");
            if !success.statements.iter().any(|statement| {
                matches!(
                    statement,
                    mir::Statement::CollectionAdd {
                        collection,
                        value,
                        index: None,
                        op: mir::CollectionMutationOp::Add,
                    } if *collection == output && value.direct_place_local() == Some(result)
                )
            }) || !matches!(success.terminator, mir::Terminator::Jump(target) if target == plan.update)
            {
                return Err(malformed_mir(
                    "List::map success path must append the callback result before updating the index",
                ));
            }
        }
        mir::ListAlgorithmKind::Filter => {
            let selected = plan
                .filter_selected
                .expect("validated filter selected block");
            let predicate = plan
                .callback_result
                .expect("validated filter callback result");
            if !matches!(
                success.terminator,
                mir::Terminator::Branch {
                    condition: mir::BoolExpression::Use {
                        operand: mir::Operand::Local(local),
                    },
                    then_block,
                    else_block,
                } if local == predicate && then_block == selected && else_block == plan.update
            ) {
                return Err(malformed_mir(
                    "List::filter success path must branch on its predicate result",
                ));
            }
            let selected = block_in(function, selected)?;
            if !selected.statements.iter().any(|statement| {
                matches!(
                    statement,
                    mir::Statement::CollectionAdd {
                        collection,
                        index: None,
                        op: mir::CollectionMutationOp::Add,
                        ..
                    } if Some(*collection) == plan.output
                )
            }) || !matches!(selected.terminator, mir::Terminator::Jump(target) if target == plan.update)
            {
                return Err(malformed_mir(
                    "List::filter selected path must copy the element before updating the index",
                ));
            }
        }
        mir::ListAlgorithmKind::Reduce => {
            if !success.statements.is_empty()
                || !matches!(success.terminator, mir::Terminator::Jump(target) if target == plan.update)
            {
                return Err(malformed_mir(
                    "List::reduce success path must continue with the same accumulator",
                ));
            }
        }
    }
    Ok(())
}

fn validate_list_algorithm_checked_cleanup(
    function: &mir::Function,
    plan: &mir::ListAlgorithmPlan,
) -> Result<(), BackendError> {
    let Some(failure) = plan.callback_failure else {
        return Ok(());
    };
    let failure = block_in(function, failure)?;
    let owned = plan
        .output
        .or(plan.accumulator)
        .expect("algorithm owns result state");
    let owned_local = local_in(function, owned)?;
    let requires_drop = owned_local.owned
        || matches!(
            owned_local.ty,
            mir::Type::String | mir::Type::NullableString
        );
    let drops = failure
        .statements
        .iter()
        .filter(|statement| statement_drops_local(statement, owned))
        .count();
    let expected_drops = usize::from(requires_drop);
    if drops != expected_drops {
        return Err(malformed_mir(
            "checked List algorithm failure has an invalid partial-result cleanup count",
        ));
    }
    Ok(())
}

fn statement_drops_local(statement: &mir::Statement, expected: mir::LocalId) -> bool {
    matches!(
        statement,
        mir::Statement::DropClass { local, .. }
            | mir::Statement::DropSharedReference { local, .. }
            | mir::Statement::DropWeakReference { local, .. }
            | mir::Statement::DropWritableSharedReference { local, .. }
            | mir::Statement::DropWritableWeakReference { local, .. }
            | mir::Statement::DropSharedReferenceAccess { local, .. }
            | mir::Statement::DropString { local }
            | mir::Statement::DropMixed { local }
            | mir::Statement::DropError { local }
            | mir::Statement::DropPayloadEnum { local, .. }
            | mir::Statement::DropCollection { local, .. }
            | mir::Statement::DropFunction { local, .. }
            if *local == expected
    )
}

fn validate_list_algorithm_region_does_not_mutate_sources(
    function: &mir::Function,
    plan: &mir::ListAlgorithmPlan,
) -> Result<(), BackendError> {
    let setup = block_in(function, plan.setup)?;
    let plan_index = setup
        .statements
        .iter()
        .position(|statement| {
            matches!(
                statement,
                mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::ListAlgorithm(candidate))
                    if candidate.as_ref() == plan
            )
        })
        .ok_or_else(|| malformed_mir("List algorithm setup is missing its validation plan"))?;
    if setup.statements[plan_index + 1..]
        .iter()
        .any(|statement| list_algorithm_statement_mutates_source(statement, plan))
    {
        return Err(malformed_mir(
            "List algorithm setup cannot mutate its source or consume its callback after activation",
        ));
    }

    let mut blocks = vec![plan.header, plan.body, plan.callback_success, plan.update];
    blocks.extend(plan.filter_selected);
    for block in blocks {
        for statement in &block_in(function, block)?.statements {
            if list_algorithm_statement_mutates_source(statement, plan) {
                return Err(malformed_mir(
                    "List algorithm traversal cannot mutate its source or consume its callback",
                ));
            }
        }
    }
    Ok(())
}

fn list_algorithm_statement_mutates_source(
    statement: &mir::Statement,
    plan: &mir::ListAlgorithmPlan,
) -> bool {
    matches!(
        statement,
        mir::Statement::AssignLocal { target, .. } if *target == plan.source
    ) || matches!(
        statement,
        mir::Statement::CollectionAdd { collection, .. }
            | mir::Statement::CollectionSet { collection, .. }
            | mir::Statement::AssignCollectionIndex { collection, .. }
            | mir::Statement::CollectionClear { collection, .. }
            | mir::Statement::DropCollection { local: collection, .. }
            if *collection == plan.source
    ) || matches!(
        statement,
        mir::Statement::DropFunction { local, .. } if *local == plan.callback
    )
}

fn validate_finalizer_plan(
    function: &mir::Function,
    anchor: mir::BlockId,
    plan: &mir::FinalizerRegionPlan,
) -> Result<(), BackendError> {
    if anchor != plan.activation {
        return Err(malformed_mir(
            "finalizer region is not anchored at its activation block",
        ));
    }
    if plan.body_blocks.is_empty()
        || !plan.body_blocks.contains(&plan.entry)
        || plan.body_blocks.contains(&plan.completion)
    {
        return Err(malformed_mir(
            "finalizer region has an invalid body boundary",
        ));
    }
    let mut planned_sources = HashSet::new();
    let mut planned_continuations = HashSet::new();
    for exit in &plan.exits {
        if !planned_sources.insert(exit.source) {
            return Err(malformed_mir(
                "finalizer region repeats a structured-exit source",
            ));
        }
        if !planned_continuations.insert(exit.continuation) {
            return Err(malformed_mir(
                "finalizer region repeats a continuation block",
            ));
        }
    }
    let entry_predecessors = function
        .blocks
        .iter()
        .filter(|block| terminator_targets(&block.terminator).contains(&plan.entry))
        .map(|block| block.id)
        .collect::<HashSet<_>>();
    if entry_predecessors != planned_sources {
        return Err(malformed_mir(
            "finalizer entry edges disagree with its structured-exit table",
        ));
    }
    for (index, exit) in plan.exits.iter().enumerate() {
        let source = block_in(function, exit.source)?;
        if !matches!(source.terminator, mir::Terminator::Jump(target) if target == plan.entry) {
            return Err(malformed_mir(
                "structured exit does not enter its finalizer region",
            ));
        }
        let selected_exits = source
            .statements
            .iter()
            .filter_map(|statement| {
                let mir::Statement::AssignLocal { target, value } = statement else {
                    return None;
                };
                (*target == plan.discriminator).then_some(value)
            })
            .collect::<Vec<_>>();
        if selected_exits.len() != 1
            || !matches!(
                selected_exits[0],
                mir::Rvalue::Value(mir::ValueExpression::Integer(
                    mir::IntegerExpression::Use {
                        ty: IntegerType::Int64,
                        operand: mir::Operand::Scalar(mir::ScalarValue::Integer(value)),
                    }
                )) if value.mathematical_value() == index as i128
            )
        {
            return Err(malformed_mir(
                "structured exit does not select its finalizer continuation",
            ));
        }
        match exit.kind {
            mir::StructuredExitKind::WhenYield { result }
            | mir::StructuredExitKind::FunctionReturn {
                value: Some(result),
            } => {
                if !source
                    .statements
                    .iter()
                    .any(|statement| matches!(statement, mir::Statement::AssignLocal { target, .. } if *target == result))
                {
                    return Err(malformed_mir(
                        "structured value exit enters a finalizer before acquiring its value",
                    ));
                }
            }
            mir::StructuredExitKind::Continue
                if matches!(
                    plan.attachment,
                    mir::FinalizerAttachment::While | mir::FinalizerAttachment::DoWhile
                ) =>
            {
                return Err(malformed_mir(
                    "same-loop continue incorrectly routes through its loop finalizer",
                ));
            }
            mir::StructuredExitKind::CheckedError { error } => {
                let error_id = error;
                let error = local_in(function, error_id)?;
                if error.ty != mir::Type::Error || !error.owned {
                    return Err(malformed_mir(
                        "checked-error finalizer exit does not own an Error carrier",
                    ));
                }
                let acquired_here = source.statements.iter().any(|statement| {
                    matches!(statement, mir::Statement::AssignLocal { target, .. } if *target == error_id)
                });
                if !acquired_here
                    && !checked_error_forwarded_from_child_finalizer(
                        function,
                        plan.id,
                        exit.source,
                        error_id,
                    )
                {
                    return Err(malformed_mir(
                        "checked-error exit enters a finalizer before acquiring its carrier",
                    ));
                }
            }
            mir::StructuredExitKind::Normal
            | mir::StructuredExitKind::FunctionReturn { value: None }
            | mir::StructuredExitKind::Break
            | mir::StructuredExitKind::Continue => {}
        }
    }
    validate_finalizer_dispatch(function, plan)?;
    Ok(())
}

fn checked_error_forwarded_from_child_finalizer(
    function: &mir::Function,
    parent: mir::FinalizerRegionId,
    continuation: mir::BlockId,
    error: mir::LocalId,
) -> bool {
    function.blocks.iter().any(|block| {
        block.statements.iter().any(|statement| {
            let mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::Finalizer(child)) = statement
            else {
                return false;
            };
            child.parent == Some(parent)
                && child.exits.iter().any(|exit| {
                    exit.continuation == continuation
                        && matches!(
                            exit.kind,
                            mir::StructuredExitKind::CheckedError { error: forwarded }
                                if forwarded == error
                        )
                })
        })
    })
}

fn validate_finalizer_dispatch(
    function: &mir::Function,
    plan: &mir::FinalizerRegionPlan,
) -> Result<(), BackendError> {
    if plan.exits.is_empty() {
        if !matches!(
            block_in(function, plan.completion)?.terminator,
            mir::Terminator::Unreachable
        ) {
            return Err(malformed_mir(
                "finalizer without exits has an executable completion",
            ));
        }
        return Ok(());
    }

    let mut dispatch = plan.completion;
    for (index, exit) in plan.exits.iter().enumerate() {
        let terminator = &block_in(function, dispatch)?.terminator;
        if index + 1 == plan.exits.len() {
            if !matches!(terminator, mir::Terminator::Jump(target) if *target == exit.continuation)
            {
                return Err(malformed_mir(
                    "finalizer completion does not select its final continuation",
                ));
            }
            continue;
        }
        let mir::Terminator::Branch {
            condition,
            then_block,
            else_block,
        } = terminator
        else {
            return Err(malformed_mir(
                "finalizer completion has a malformed continuation dispatch",
            ));
        };
        if *then_block != exit.continuation
            || !is_finalizer_discriminator_condition(condition, plan.discriminator, index as i128)
        {
            return Err(malformed_mir(
                "finalizer completion dispatch disagrees with its exit table",
            ));
        }
        dispatch = *else_block;
    }
    Ok(())
}

fn is_finalizer_discriminator_condition(
    condition: &mir::BoolExpression,
    discriminator: mir::LocalId,
    expected: i128,
) -> bool {
    matches!(
        condition,
        mir::BoolExpression::Compare {
            op: mir::CompareOp::Equal,
            left,
            right,
        } if matches!(
            left.as_ref(),
            mir::ValueExpression::Integer(mir::IntegerExpression::Use {
                ty: IntegerType::Int64,
                operand: mir::Operand::Local(local),
            }) if *local == discriminator
        ) && matches!(
            right.as_ref(),
            mir::ValueExpression::Integer(mir::IntegerExpression::Use {
                ty: IntegerType::Int64,
                operand: mir::Operand::Scalar(mir::ScalarValue::Integer(value)),
            }) if value.mathematical_value() == expected
        )
    )
}

fn checked_success_reaches_bool_control(
    function: &mir::Function,
    start: mir::BlockId,
) -> Result<bool, BackendError> {
    let mut current = start;
    let mut visited = HashSet::new();
    while visited.insert(current) {
        match &block_in(function, current)?.terminator {
            mir::Terminator::Branch { .. } | mir::Terminator::Jump(_) => return Ok(true),
            mir::Terminator::CheckedCall { success, .. }
            | mir::Terminator::CheckedIndirectCall { success, .. }
            | mir::Terminator::CheckedConstruct { success, .. }
            | mir::Terminator::CheckedIo { success, .. } => current = *success,
            mir::Terminator::IndirectCall { continuation, .. } => current = *continuation,
            _ => return Ok(false),
        }
    }
    Ok(false)
}

fn cfg_reaches(
    function: &mir::Function,
    start: mir::BlockId,
    target: mir::BlockId,
) -> Result<bool, BackendError> {
    let mut pending = VecDeque::from([start]);
    let mut visited = HashSet::new();
    while let Some(block) = pending.pop_front() {
        if block == target {
            return Ok(true);
        }
        if visited.insert(block) {
            pending.extend(terminator_targets(&block_in(function, block)?.terminator));
        }
    }
    Ok(false)
}

fn validate_match_binding_plans(function: &mir::Function) -> Result<(), BackendError> {
    let mut expected_modes = HashMap::new();

    for block in &function.blocks {
        for statement in &block.statements {
            let mir::Statement::MatchResultPlan {
                mode, arms, merge, ..
            } = statement
            else {
                continue;
            };
            let binding_mode = match mode {
                mir::MatchOwnershipMode::Borrowed => mir::MatchBindingMode::BorrowedArm,
                mir::MatchOwnershipMode::Consumed => mir::MatchBindingMode::ConsumedArm,
            };

            for arm in arms {
                if expected_modes.insert(arm.binding, binding_mode).is_some() {
                    return Err(malformed_mir(
                        "match binding block belongs to more than one match arm",
                    ));
                }
                let Some(guard) = arm.guard else {
                    continue;
                };
                if expected_modes
                    .insert(guard, mir::MatchBindingMode::GuardView)
                    .is_some()
                {
                    return Err(malformed_mir(
                        "match guard block overlaps another match guard or binding",
                    ));
                }
            }

            let arm_boundaries = arms
                .iter()
                .flat_map(|arm| std::iter::once(arm.binding).chain(arm.guard))
                .collect::<HashSet<_>>();
            for arm in arms {
                let Some(guard) = arm.guard else {
                    continue;
                };
                if !match_guard_reaches_binding(
                    function,
                    guard,
                    arm.binding,
                    *merge,
                    &arm_boundaries,
                )? {
                    return Err(malformed_mir(
                        "match guard must branch through a success path to its final binding block",
                    ));
                }
            }
        }
    }

    for block in &function.blocks {
        for statement in &block.statements {
            let mir::Statement::BindPayloadEnumFields { mode, .. } = statement else {
                continue;
            };
            if expected_modes.get(&block.id) != Some(mode) {
                return Err(malformed_mir(
                    "payload match binding does not match its planned guard or arm mode",
                ));
            }
        }
    }

    Ok(())
}

fn match_guard_reaches_binding(
    function: &mir::Function,
    guard: mir::BlockId,
    binding: mir::BlockId,
    merge: mir::BlockId,
    arm_boundaries: &HashSet<mir::BlockId>,
) -> Result<bool, BackendError> {
    let mut pending = VecDeque::from([guard]);
    let mut visited = HashSet::new();
    while let Some(block_id) = pending.pop_front() {
        if block_id == binding {
            return Ok(true);
        }
        if !visited.insert(block_id)
            || block_id == merge
            || (block_id != guard && arm_boundaries.contains(&block_id))
        {
            continue;
        }
        pending.extend(match_guard_success_targets(
            &block_in(function, block_id)?.terminator,
        ));
    }
    Ok(false)
}

fn match_guard_success_targets(terminator: &mir::Terminator) -> Vec<mir::BlockId> {
    match terminator {
        mir::Terminator::CheckedCall { success, .. }
        | mir::Terminator::CheckedIndirectCall { success, .. }
        | mir::Terminator::CheckedConstruct { success, .. }
        | mir::Terminator::CheckedIo { success, .. } => vec![*success],
        mir::Terminator::ErrorSwitch { .. }
        | mir::Terminator::Return(_)
        | mir::Terminator::ReturnVoid
        | mir::Terminator::Panic { .. }
        | mir::Terminator::Unreachable
        | mir::Terminator::PropagateError { .. } => Vec::new(),
        mir::Terminator::Jump(target) => vec![*target],
        mir::Terminator::IndirectCall { continuation, .. } => vec![*continuation],
        mir::Terminator::Branch {
            then_block,
            else_block,
            ..
        } => vec![*then_block, *else_block],
    }
}

fn validate_match_arm_result_path(
    function: &mir::Function,
    result: mir::LocalId,
    arm: mir::BlockId,
    merge: mir::BlockId,
) -> Result<(), BackendError> {
    validate_result_path(function, result, arm, merge, "match arm")
}

fn validate_result_path(
    function: &mir::Function,
    result: mir::LocalId,
    arm: mir::BlockId,
    merge: mir::BlockId,
    path_name: &str,
) -> Result<(), BackendError> {
    let mut pending = VecDeque::from([(arm, 0_u8)]);
    let mut visited = HashSet::new();
    let mut reached_merge = false;
    let mut reached_fatal_panic = false;
    while let Some((block_id, assignments)) = pending.pop_front() {
        if block_id == merge {
            reached_merge = true;
            if assignments != 1 {
                return Err(malformed_mir(format!(
                    "{path_name} reaches its merge with {assignments} result assignments"
                )));
            }
            continue;
        }
        if !visited.insert((block_id, assignments)) {
            continue;
        }
        let block = block_in(function, block_id)?;
        let assignments = block
            .statements
            .iter()
            .fold(assignments, |count, statement| {
                count.saturating_add(u8::from(matches!(
                    statement,
                    mir::Statement::AssignLocal { target, .. } if *target == result
                )))
            });
        if assignments > 1 {
            return Err(malformed_mir(format!(
                "{path_name} assigns its result more than once on one path"
            )));
        }
        match &block.terminator {
            mir::Terminator::Jump(target) => pending.push_back((*target, assignments)),
            mir::Terminator::IndirectCall { continuation, .. } => {
                pending.push_back((*continuation, assignments));
            }
            mir::Terminator::Branch {
                then_block,
                else_block,
                ..
            } => {
                pending.push_back((*then_block, assignments));
                pending.push_back((*else_block, assignments));
            }
            mir::Terminator::CheckedCall {
                success, failure, ..
            }
            | mir::Terminator::CheckedIndirectCall {
                success, failure, ..
            }
            | mir::Terminator::CheckedConstruct {
                success, failure, ..
            }
            | mir::Terminator::CheckedIo {
                success, failure, ..
            } => {
                pending.push_back((*success, assignments));
                pending.push_back((*failure, assignments));
            }
            mir::Terminator::ErrorSwitch {
                cases,
                catch_all,
                fallback,
                ..
            } => {
                for (_, target) in cases {
                    pending.push_back((*target, assignments));
                }
                if let Some(target) = catch_all {
                    pending.push_back((*target, assignments));
                }
                pending.push_back((*fallback, assignments));
            }
            mir::Terminator::Panic { .. } => {
                reached_fatal_panic = true;
            }
            mir::Terminator::Return(_) | mir::Terminator::ReturnVoid => {
                return Err(malformed_mir(format!(
                    "{path_name} terminates before assigning and merging its result"
                )));
            }
            mir::Terminator::Unreachable | mir::Terminator::PropagateError { .. } => {}
        }
    }
    if !reached_merge && !reached_fatal_panic {
        return Err(malformed_mir(format!(
            "{path_name} cannot reach its result merge"
        )));
    }
    Ok(())
}

fn merge_definitely_present(
    destination: &mut Option<HashSet<mir::LocalId>>,
    incoming: &HashSet<mir::LocalId>,
) -> bool {
    match destination {
        Some(current) => {
            let merged = current
                .intersection(incoming)
                .copied()
                .collect::<HashSet<_>>();
            if *current == merged {
                false
            } else {
                *current = merged;
                true
            }
        }
        None => {
            *destination = Some(incoming.clone());
            true
        }
    }
}

fn merge_definite_mixed_tags(
    destination: &mut Option<HashMap<mir::LocalId, mir::MixedTag>>,
    incoming: &HashMap<mir::LocalId, mir::MixedTag>,
) -> bool {
    match destination {
        Some(current) => {
            let merged = current
                .iter()
                .filter_map(|(local, tag)| {
                    (incoming.get(local) == Some(tag)).then_some((*local, *tag))
                })
                .collect::<HashMap<_, _>>();
            if *current == merged {
                false
            } else {
                *current = merged;
                true
            }
        }
        None => {
            *destination = Some(incoming.clone());
            true
        }
    }
}

fn merge_definite_payload_cases(
    destination: &mut Option<HashMap<mir::LocalId, crate::enums::EnumCaseId>>,
    incoming: &HashMap<mir::LocalId, crate::enums::EnumCaseId>,
) -> bool {
    match destination {
        Some(current) => {
            let merged = current
                .iter()
                .filter_map(|(local, case)| {
                    (incoming.get(local) == Some(case)).then_some((*local, *case))
                })
                .collect::<HashMap<_, _>>();
            if *current == merged {
                false
            } else {
                *current = merged;
                true
            }
        }
        None => {
            *destination = Some(incoming.clone());
            true
        }
    }
}

fn apply_payload_case_statement(
    function: &mir::Function,
    statement: &mir::Statement,
    cases: &mut HashMap<mir::LocalId, crate::enums::EnumCaseId>,
) -> Result<(), BackendError> {
    match statement {
        mir::Statement::AssignLocal { target, .. } => {
            if matches!(
                local_in(function, *target)?.ty,
                mir::Type::PayloadEnum(_) | mir::Type::NullablePayloadEnum(_)
            ) {
                cases.remove(target);
            }
        }
        mir::Statement::AssignLocalGroup { targets, .. } => {
            for target in targets {
                if matches!(
                    local_in(function, *target)?.ty,
                    mir::Type::PayloadEnum(_) | mir::Type::NullablePayloadEnum(_)
                ) {
                    cases.remove(target);
                }
            }
        }
        mir::Statement::BindPayloadEnumFields { targets, .. } => {
            for target in targets {
                cases.remove(target);
            }
        }
        mir::Statement::DropPayloadEnum { local, .. } => {
            cases.remove(local);
        }
        _ => {}
    }
    Ok(())
}

fn apply_payload_case_condition(
    condition: &mir::BoolExpression,
    when_true: bool,
    cases: &mut HashMap<mir::LocalId, crate::enums::EnumCaseId>,
) {
    match condition {
        mir::BoolExpression::PayloadEnumIsCase { local, case, .. } => {
            if when_true {
                cases.insert(*local, *case);
            } else {
                cases.remove(local);
            }
        }
        mir::BoolExpression::Not(value) => apply_payload_case_condition(value, !when_true, cases),
        mir::BoolExpression::Binary { op, left, right } => match (op, when_true) {
            (mir::BoolBinaryOp::And, true) | (mir::BoolBinaryOp::Or, false) => {
                apply_payload_case_condition(left, when_true, cases);
                apply_payload_case_condition(right, when_true, cases);
            }
            _ => {}
        },
        _ => {}
    }
}

fn apply_nullable_presence_statement(
    program: &mir::Program,
    function: &mir::Function,
    statement: &mir::Statement,
    present: &mut HashSet<mir::LocalId>,
) -> Result<(), BackendError> {
    apply_nullable_class_call_effects(
        program,
        function,
        &collect_statement_class_local_accesses(statement),
        present,
    )?;
    match statement {
        mir::Statement::AssignLocal { target, value } => {
            let value_is_present = match (local_in(function, *target)?.ty, value) {
                (mir::Type::NullableScalar(_), mir::Rvalue::NullableScalar(value)) => {
                    nullable_scalar_expression_is_present(value, present)
                }
                (mir::Type::NullableString, mir::Rvalue::NullableString(value)) => {
                    nullable_string_expression_is_present(value, present)
                }
                (mir::Type::NullableClass(_), mir::Rvalue::NullableClass(value)) => {
                    nullable_class_expression_is_present(value, present)
                }
                (mir::Type::NullableCollection(_), mir::Rvalue::NullableCollection(value)) => {
                    nullable_collection_expression_is_present(value, present)
                }
                _ => return Ok(()),
            };
            if value_is_present {
                present.insert(*target);
            } else {
                present.remove(target);
            }
        }
        mir::Statement::AssignLocalGroup { targets, value } => {
            let first = local_in(function, targets[0])?;
            let value_is_present = match (first.ty, value) {
                (mir::Type::NullableScalar(_), mir::Rvalue::NullableScalar(value)) => {
                    nullable_scalar_expression_is_present(value, present)
                }
                (mir::Type::NullableString, mir::Rvalue::NullableString(value)) => {
                    nullable_string_expression_is_present(value, present)
                }
                (mir::Type::NullableClass(_), mir::Rvalue::NullableClass(value)) => {
                    nullable_class_expression_is_present(value, present)
                }
                (mir::Type::NullableCollection(_), mir::Rvalue::NullableCollection(value)) => {
                    nullable_collection_expression_is_present(value, present)
                }
                _ => return Ok(()),
            };
            for target in targets {
                if value_is_present {
                    present.insert(*target);
                } else {
                    present.remove(target);
                }
            }
        }
        mir::Statement::DropClass { local, .. } | mir::Statement::DropCollection { local, .. } => {
            present.remove(local);
        }
        _ => {}
    }
    Ok(())
}

fn apply_mixed_tag_statement(
    function: &mir::Function,
    statement: &mir::Statement,
    tags: &mut HashMap<mir::LocalId, mir::MixedTag>,
) -> Result<(), BackendError> {
    match statement {
        mir::Statement::AssignLocal { target, .. } => {
            if matches!(
                local_in(function, *target)?.ty,
                mir::Type::Mixed | mir::Type::NullableMixed
            ) {
                tags.remove(target);
            }
        }
        mir::Statement::AssignLocalGroup { targets, .. } => {
            for target in targets {
                if matches!(
                    local_in(function, *target)?.ty,
                    mir::Type::Mixed | mir::Type::NullableMixed
                ) {
                    tags.remove(target);
                }
            }
        }
        mir::Statement::DropMixed { local } => {
            tags.remove(local);
        }
        _ => {}
    }
    Ok(())
}

fn apply_mixed_tag_condition(
    condition: &mir::BoolExpression,
    when_true: bool,
    tags: &mut HashMap<mir::LocalId, mir::MixedTag>,
) {
    match condition {
        mir::BoolExpression::MixedIs { mixed, tag } => {
            if let Some(local) = mixed_expression_local(mixed) {
                if when_true {
                    tags.insert(local, *tag);
                } else {
                    tags.remove(&local);
                }
            }
        }
        mir::BoolExpression::Not(value) => apply_mixed_tag_condition(value, !when_true, tags),
        mir::BoolExpression::Binary { op, left, right } => match (op, when_true) {
            (mir::BoolBinaryOp::And, true) | (mir::BoolBinaryOp::Or, false) => {
                apply_mixed_tag_condition(left, when_true, tags);
                apply_mixed_tag_condition(right, when_true, tags);
            }
            _ => {}
        },
        _ => {}
    }
}

fn mixed_expression_local(expression: &mir::MixedExpression) -> Option<mir::LocalId> {
    match expression {
        mir::MixedExpression::Local { local, .. } => Some(*local),
        _ => None,
    }
}

fn validate_mixed_tag_assumptions(
    function: &mir::Function,
    accesses: &ClassLocalAccesses<'_>,
    tags: &HashMap<mir::LocalId, mir::MixedTag>,
) -> Result<(), BackendError> {
    for (local, tag) in accesses.mixed_assumptions() {
        if tags.get(&local) != Some(&tag) {
            return Err(malformed_mir(format!(
                "{} local local{} is unboxed as {tag} without a dominating exact `is` proof",
                local_in(function, local)?.ty,
                local.0,
            )));
        }
    }
    Ok(())
}

fn apply_nullable_class_call_effects(
    program: &mir::Program,
    function: &mir::Function,
    accesses: &ClassLocalAccesses<'_>,
    present: &mut HashSet<mir::LocalId>,
) -> Result<(), BackendError> {
    for access in accesses.iter() {
        let ClassLocalAccess::Call(callee, args, parameter_offset) = access else {
            continue;
        };
        for (local, mode) in borrowed_class_call_locals(program, callee, args, parameter_offset)? {
            if matches!(mode, ClassBorrowMode::Writable)
                && matches!(local_in(function, local)?.ty, mir::Type::NullableClass(_))
            {
                present.remove(&local);
            }
        }
    }
    Ok(())
}

fn nullable_class_expression_is_definitely_null(expression: &mir::NullableClassExpression) -> bool {
    matches!(expression, mir::NullableClassExpression::Null(_))
}

fn nullable_class_expression_is_present(
    expression: &mir::NullableClassExpression,
    present: &HashSet<mir::LocalId>,
) -> bool {
    match expression {
        mir::NullableClassExpression::Class(_) => true,
        mir::NullableClassExpression::SharedPayload { .. } => false,
        mir::NullableClassExpression::Local { local, .. } => present.contains(local),
        mir::NullableClassExpression::Coalesce { left, right, .. } => {
            nullable_class_expression_is_present(left, present)
                || nullable_class_expression_is_present(right, present)
        }
        mir::NullableClassExpression::Null(_)
        | mir::NullableClassExpression::Property { .. }
        | mir::NullableClassExpression::Call { .. }
        | mir::NullableClassExpression::NullSafeProperty { .. }
        | mir::NullableClassExpression::NullSafeCall { .. }
        | mir::NullableClassExpression::DictionaryGet { .. } => false,
    }
}

fn nullable_collection_expression_is_present(
    value: &mir::NullableCollectionExpression,
    present: &HashSet<mir::LocalId>,
) -> bool {
    match value {
        mir::NullableCollectionExpression::Null(_) => false,
        mir::NullableCollectionExpression::Collection(_) => true,
        mir::NullableCollectionExpression::Local { local, .. } => present.contains(local),
        mir::NullableCollectionExpression::Property { .. }
        | mir::NullableCollectionExpression::Call { .. } => false,
        mir::NullableCollectionExpression::Coalesce { left, right, .. } => {
            nullable_collection_expression_is_present(left, present)
                || nullable_collection_expression_is_present(right, present)
        }
    }
}

fn nullable_scalar_expression_is_present(
    expression: &mir::NullableScalarExpression,
    present: &HashSet<mir::LocalId>,
) -> bool {
    match expression {
        mir::NullableScalarExpression::Value(_) => true,
        mir::NullableScalarExpression::Local { local, .. } => present.contains(local),
        mir::NullableScalarExpression::Coalesce { left, right, .. } => {
            nullable_scalar_expression_is_present(left, present)
                || nullable_scalar_expression_is_present(right, present)
        }
        mir::NullableScalarExpression::EnumBacking { value, .. } => {
            nullable_scalar_expression_is_present(value, present)
        }
        mir::NullableScalarExpression::Null(_)
        | mir::NullableScalarExpression::Property { .. }
        | mir::NullableScalarExpression::Static { .. }
        | mir::NullableScalarExpression::Call { .. }
        | mir::NullableScalarExpression::NullSafeProperty { .. }
        | mir::NullableScalarExpression::NullSafeCall { .. }
        | mir::NullableScalarExpression::DictionaryGet { .. }
        | mir::NullableScalarExpression::CollectionIndexOf { .. }
        | mir::NullableScalarExpression::Parse { .. }
        | mir::NullableScalarExpression::StringIntrinsic(_) => false,
    }
}

fn nullable_string_expression_is_present(
    expression: &mir::NullableStringExpression,
    present: &HashSet<mir::LocalId>,
) -> bool {
    match expression {
        mir::NullableStringExpression::String(_) => true,
        mir::NullableStringExpression::Local(local) => present.contains(local),
        mir::NullableStringExpression::Coalesce { left, right } => {
            nullable_string_expression_is_present(left, present)
                || nullable_string_expression_is_present(right, present)
        }
        mir::NullableStringExpression::EnumBacking { value, .. } => {
            nullable_scalar_expression_is_present(value, present)
        }
        mir::NullableStringExpression::Null
        | mir::NullableStringExpression::Property { .. }
        | mir::NullableStringExpression::Static(_)
        | mir::NullableStringExpression::ReadLine { .. }
        | mir::NullableStringExpression::Call { .. }
        | mir::NullableStringExpression::NullSafeProperty { .. }
        | mir::NullableStringExpression::NullSafeCall { .. }
        | mir::NullableStringExpression::DictionaryGet { .. }
        | mir::NullableStringExpression::Intrinsic(_) => false,
    }
}

fn apply_nullable_presence_condition(
    condition: &mir::BoolExpression,
    when_true: bool,
    present: &mut HashSet<mir::LocalId>,
) {
    match condition {
        mir::BoolExpression::NullableScalarIsPresent(value) => {
            if let mir::NullableScalarExpression::Local { local, .. } = value.as_ref() {
                set_nullable_presence(*local, when_true, present);
            }
        }
        mir::BoolExpression::NullableErrorIsPresent(value) => {
            if let mir::NullableErrorExpression::Local { local, .. } = value.as_ref() {
                set_nullable_presence(*local, when_true, present);
            }
        }
        mir::BoolExpression::NullableSharedReferenceAccessIsPresent(value) => {
            if let mir::NullableSharedReferenceAccessExpression::Local { local, .. } =
                value.as_ref()
            {
                set_nullable_presence(*local, when_true, present);
            }
        }
        mir::BoolExpression::NullableClassIsPresent(value) => {
            if let mir::NullableClassExpression::Local { local, .. } = value.as_ref() {
                set_nullable_presence(*local, when_true, present);
            }
        }
        mir::BoolExpression::NullableCollectionIsPresent(value) => {
            if let mir::NullableCollectionExpression::Local { local, .. } = value.as_ref() {
                set_nullable_presence(*local, when_true, present);
            }
        }
        mir::BoolExpression::NullableSharedReferenceIsPresent(value) => {
            if let mir::NullableSharedReferenceExpression::Local { local, .. } = value.as_ref() {
                set_nullable_presence(*local, when_true, present);
            }
        }
        mir::BoolExpression::NullableWeakReferenceIsPresent(value) => {
            if let mir::NullableWeakReferenceExpression::Local { local, .. } = value.as_ref() {
                set_nullable_presence(*local, when_true, present);
            }
        }
        mir::BoolExpression::NullableStringCompare { op, left, right } => {
            if let (Some(local), Some(equals_null)) = (
                nullable_string_null_comparison_local(left, right),
                match op {
                    mir::CompareOp::Equal => Some(true),
                    mir::CompareOp::NotEqual => Some(false),
                    mir::CompareOp::Less
                    | mir::CompareOp::LessEqual
                    | mir::CompareOp::Greater
                    | mir::CompareOp::GreaterEqual => None,
                },
            ) {
                set_nullable_presence(local, when_true != equals_null, present);
            }
        }
        mir::BoolExpression::NullablePayloadEnumIsPresent(value) => {
            if let Some(local) = nullable_payload_enum_presence_local(value) {
                set_nullable_presence(local, when_true, present);
            }
        }
        mir::BoolExpression::PayloadEnumIsCase {
            local, nullable, ..
        } if *nullable => {
            set_nullable_presence(*local, when_true, present);
        }
        mir::BoolExpression::Not(value) => {
            apply_nullable_presence_condition(value, !when_true, present)
        }
        mir::BoolExpression::Binary { op, left, right } => match (op, when_true) {
            (mir::BoolBinaryOp::And, true) | (mir::BoolBinaryOp::Or, false) => {
                apply_nullable_presence_condition(left, when_true, present);
                apply_nullable_presence_condition(right, when_true, present);
            }
            _ => {}
        },
        _ => {}
    }
}

fn set_nullable_presence(
    local: mir::LocalId,
    is_present: bool,
    present: &mut HashSet<mir::LocalId>,
) {
    if is_present {
        present.insert(local);
    } else {
        present.remove(&local);
    }
}

fn nullable_string_null_comparison_local(
    left: &mir::NullableStringExpression,
    right: &mir::NullableStringExpression,
) -> Option<mir::LocalId> {
    match (left, right) {
        (mir::NullableStringExpression::Local(local), mir::NullableStringExpression::Null)
        | (mir::NullableStringExpression::Null, mir::NullableStringExpression::Local(local)) => {
            Some(*local)
        }
        _ => None,
    }
}

fn nullable_payload_enum_presence_local(
    value: &mir::NullablePayloadEnumExpression,
) -> Option<mir::LocalId> {
    match value {
        mir::NullablePayloadEnumExpression::Use {
            place: mir::PayloadEnumPlace::Local(local),
            ..
        } => Some(*local),
        _ => None,
    }
}

fn validate_nullable_assumptions(
    function: &mir::Function,
    accesses: &ClassLocalAccesses<'_>,
    present: &HashSet<mir::LocalId>,
) -> Result<(), BackendError> {
    let property_receivers = accesses.property_borrowed().filter_map(|(local, _)| {
        matches!(
            local_in(function, local).ok()?.ty,
            mir::Type::NullableClass(_)
        )
        .then_some(local)
    });
    for local in accesses.nullable_assumptions().chain(property_receivers) {
        if !present.contains(&local) {
            return Err(malformed_mir(format!(
                "{} local local{} is assumed non-null without a dominating presence proof",
                local_in(function, local)?.ty,
                local.0,
            )));
        }
    }
    Ok(())
}

fn validate_class_local_lifetimes(function: &mir::Function) -> Result<(), BackendError> {
    validate_class_local_lifetimes_with_aliases(function, &[])
}

fn validate_class_local_lifetimes_with_aliases(
    function: &mir::Function,
    alias_invalidations: &[PropertyAliasInvalidation],
) -> Result<(), BackendError> {
    let (reachable, predecessors) = reachable_blocks_and_predecessors(function, true)?;
    let mut moved_on_entry = vec![HashSet::new(); function.blocks.len()];
    let mut moved_on_exit = vec![HashSet::new(); function.blocks.len()];

    loop {
        let mut changed = false;
        for block in function.blocks.iter().filter(|block| reachable[block.id.0]) {
            let mut moved_at_entry = HashSet::new();
            for predecessor in &predecessors[block.id.0] {
                moved_at_entry.extend(class_local_state_on_edge(
                    function,
                    *predecessor,
                    block.id,
                    &moved_on_exit[predecessor.0],
                )?);
            }
            let mut moved_at_exit = moved_at_entry.clone();
            for statement in &block.statements {
                apply_class_local_state(
                    function,
                    statement,
                    &mut moved_at_exit,
                    alias_invalidations,
                    false,
                )?;
            }
            apply_class_local_accesses(
                function,
                &collect_terminator_class_local_accesses(&block.terminator),
                &mut moved_at_exit,
                false,
            )?;
            if moved_on_entry[block.id.0] != moved_at_entry
                || moved_on_exit[block.id.0] != moved_at_exit
            {
                moved_on_entry[block.id.0] = moved_at_entry;
                moved_on_exit[block.id.0] = moved_at_exit;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    for block in function.blocks.iter().filter(|block| reachable[block.id.0]) {
        let mut moved = moved_on_entry[block.id.0].clone();
        for statement in &block.statements {
            apply_class_local_state(function, statement, &mut moved, alias_invalidations, true)?;
        }
        let accesses = collect_terminator_class_local_accesses(&block.terminator);
        apply_class_local_accesses(function, &accesses, &mut moved, true)?;
    }
    Ok(())
}

fn class_local_state_on_edge(
    function: &mir::Function,
    predecessor: mir::BlockId,
    target: mir::BlockId,
    moved: &HashSet<mir::LocalId>,
) -> Result<HashSet<mir::LocalId>, BackendError> {
    let mut state = moved.clone();
    let terminator = &block_in(function, predecessor)?.terminator;
    let initialized = match terminator {
        mir::Terminator::IndirectCall {
            result,
            continuation,
            ..
        } if *continuation == target => *result,
        mir::Terminator::CheckedCall {
            result, success, ..
        }
        | mir::Terminator::CheckedIndirectCall {
            result, success, ..
        }
        | mir::Terminator::CheckedIo {
            result, success, ..
        } if *success == target => *result,
        mir::Terminator::CheckedConstruct {
            result, success, ..
        } if *success == target => Some(*result),
        _ => None,
    };
    if let Some(local) = initialized {
        if matches!(
            local_in(function, local)?.ty,
            mir::Type::Class(_) | mir::Type::NullableClass(_)
        ) {
            state.remove(&local);
        }
    }
    Ok(state)
}

fn validate_class_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::ClassExpression,
) -> Result<(), BackendError> {
    let class = expression.class();
    let Some(class_definition) = program
        .classes
        .get(class.0)
        .filter(|definition| definition.id == class)
    else {
        return Err(malformed_mir(format!("unknown class#{}", class.0)));
    };
    match expression {
        mir::ClassExpression::Local {
            local, transfer, ..
        } => {
            let definition = local_in(function, *local)?;
            if definition.ty != mir::Type::Class(class) {
                return Err(malformed_mir(format!(
                    "class rvalue uses non-class local local{}",
                    local.0
                )));
            }
            if *transfer && !definition.owned {
                return Err(malformed_mir(format!(
                    "class rvalue transfers borrowed local local{}",
                    local.0
                )));
            }
            Ok(())
        }
        mir::ClassExpression::NullableLocalAssumeNonNull {
            local, transfer, ..
        } => {
            let definition = local_in(function, *local)?;
            if definition.ty != mir::Type::NullableClass(class) {
                return Err(malformed_mir(
                    "nonnull class expression references another local type",
                ));
            }
            if *transfer && !definition.owned {
                return Err(malformed_mir(
                    "nonnull class expression transfers a borrowed local",
                ));
            }
            Ok(())
        }
        mir::ClassExpression::Property {
            object, property, ..
        } => validate_property_operand(
            program,
            function,
            *object,
            *property,
            mir::Type::Class(class),
        ),
        mir::ClassExpression::SharedPayload { reference, .. } => {
            if reference.class() != class {
                return Err(malformed_mir(
                    "shared-reference payload projection changes class",
                ));
            }
            validate_shared_reference_expression(program, function, reference)
        }
        mir::ClassExpression::SharedAccessPayload {
            access, writable, ..
        } => {
            let expected = if *writable {
                mir::Type::WritableSharedReferenceAccess(mir::WritableSharedPayload::Class(class))
            } else {
                mir::Type::ReadonlySharedReferenceAccess(mir::WritableSharedPayload::Class(class))
            };
            if local_in(function, *access)?.ty != expected {
                return Err(malformed_mir(
                    "class access payload projection type mismatch",
                ));
            }
            Ok(())
        }
        mir::ClassExpression::Call {
            function: callee,
            args,
            return_borrow,
            ..
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type != mir::ReturnType::Value(mir::Type::Class(class)) {
                return Err(malformed_mir(format!(
                    "class#{} call targets a function with another return type",
                    class.0
                )));
            }
            let expected_return_borrow = infer_function_return_borrow(program, callee)?;
            if *return_borrow != expected_return_borrow {
                return Err(malformed_mir(format!(
                    "class#{} call disagrees with function {} return ownership",
                    class.0, callee.name
                )));
            }
            validate_call_args(program, function, callee, args)
        }
        mir::ClassExpression::New {
            properties,
            args,
            constructor,
            ..
        } => {
            if class_definition.constructor != *constructor {
                return Err(malformed_mir(format!(
                    "class#{} new expression names the wrong constructor",
                    class.0
                )));
            }
            let constructor = constructor
                .map(|constructor| function_in(program, constructor))
                .transpose()?;
            let constructor_parameters = if let Some(constructor) = constructor {
                if constructor.return_type != mir::ReturnType::Void {
                    return Err(malformed_mir(format!(
                        "constructor {} does not return void",
                        constructor.name
                    )));
                }
                let Some((receiver, parameters)) = constructor.params.split_first() else {
                    return Err(malformed_mir(format!(
                        "constructor {} has no implicit receiver",
                        constructor.name
                    )));
                };
                if local_in(constructor, *receiver)?.ty != mir::Type::Class(class) {
                    return Err(malformed_mir(format!(
                        "constructor {} has an incompatible implicit receiver",
                        constructor.name
                    )));
                }
                parameters
            } else {
                if !args.is_empty() {
                    return Err(malformed_mir(format!(
                        "class#{} without a constructor receives arguments",
                        class.0
                    )));
                }
                &[]
            };

            let mut initialized = HashSet::new();
            let mut consumed_class_arguments = HashSet::new();
            let mut construction_accesses = ClassLocalAccesses::default();
            for (position, property) in properties.iter().enumerate() {
                if property.property.index != position {
                    return Err(malformed_mir(format!(
                        "class#{} new expression initializes property{} out of construction order",
                        class.0, property.property.index
                    )));
                }
                let Some(definition) = class_definition
                    .properties
                    .get(property.property.index)
                    .filter(|definition| definition.id == property.property)
                else {
                    return Err(malformed_mir(format!(
                        "class#{} new expression initializes an unknown property slot",
                        class.0
                    )));
                };
                if !initialized.insert(property.property) {
                    return Err(malformed_mir(format!(
                        "class#{} new expression initializes property{} more than once",
                        class.0, property.property.index
                    )));
                }
                let source_type = match &property.source {
                    mir::PropertyValueSource::Expression(value) => {
                        validate_rvalue(program, function, value)?;
                        if let (mir::Type::Class(_), mir::Rvalue::Class(expression)) =
                            (definition.ty, value)
                        {
                            require_owned_class_expression(
                                expression,
                                &format!(
                                    "class#{} property{} initializer",
                                    class.0, property.property.index
                                ),
                            )?;
                        }
                        collect_rvalue_class_local_accesses(value, &mut construction_accesses);
                        value.ty()
                    }
                    mir::PropertyValueSource::ConstructorArgument(index) => {
                        let argument = args.get(*index).ok_or_else(|| {
                            malformed_mir(format!(
                                "class#{} property{} references constructor argument {} but only {} exist",
                                class.0,
                                property.property.index,
                                index,
                                args.len()
                            ))
                        })?;
                        if matches!(argument.ty(), mir::Type::Class(_))
                            && !consumed_class_arguments.insert(*index)
                        {
                            return Err(malformed_mir(format!(
                                "class#{} new expression gives constructor argument {} to more than one property",
                                class.0, index
                            )));
                        }
                        argument.ty()
                    }
                    mir::PropertyValueSource::ConstructorBody => {
                        let Some(constructor) = constructor else {
                            return Err(malformed_mir(format!(
                                "class#{} property{} relies on a missing constructor body",
                                class.0, property.property.index
                            )));
                        };
                        let receiver = *constructor.params.first().ok_or_else(|| {
                            malformed_mir(format!(
                                "constructor {} has no implicit receiver",
                                constructor.name
                            ))
                        })?;
                        validate_constructor_body_initializer(
                            constructor,
                            receiver,
                            property.property,
                            definition.writable,
                        )?;
                        definition.ty
                    }
                };
                if !definition.writable
                    && !matches!(property.source, mir::PropertyValueSource::ConstructorBody)
                {
                    if let Some(constructor) = constructor {
                        if constructor_property_assignment_count(
                            constructor,
                            constructor.params[0],
                            property.property,
                        )? > 0
                        {
                            return Err(malformed_mir(format!(
                                "class#{} readonly property{} is initialized before its constructor assigns it",
                                class.0, property.property.index
                            )));
                        }
                    }
                }
                if source_type != definition.ty {
                    return Err(malformed_mir(format!(
                        "class#{} property{} has type {} but its initializer has type {}",
                        class.0, property.property.index, definition.ty, source_type
                    )));
                }
            }
            if initialized.len() != class_definition.properties.len() {
                let missing = class_definition
                    .properties
                    .iter()
                    .find(|property| !initialized.contains(&property.id))
                    .expect("property count differs");
                return Err(malformed_mir(format!(
                    "class#{} new expression does not initialize property{}",
                    class.0, missing.id.index
                )));
            }
            if constructor.is_some() {
                construction_accesses.begin_call();
            }
            collect_rvalue_args_class_local_accesses(args, &mut construction_accesses);
            if let Some(constructor) = constructor {
                construction_accesses.constructor_call(constructor.id, args);
            }
            validate_ordered_class_accesses(
                program,
                &format!("class#{} new expression", class.0),
                &construction_accesses,
                &HashMap::new(),
                &mut HashSet::new(),
            )?;
            if let Some(constructor) = constructor {
                validate_call_args_for_params(
                    program,
                    function,
                    constructor,
                    constructor_parameters,
                    args,
                    Some(
                        &properties
                            .iter()
                            .filter_map(|property| match property.source {
                                mir::PropertyValueSource::ConstructorArgument(index)
                                    if matches!(
                                        class_definition.properties[property.property.index].ty,
                                        mir::Type::Class(_) | mir::Type::NullableClass(_)
                                    ) =>
                                {
                                    Some(index)
                                }
                                _ => None,
                            })
                            .collect(),
                    ),
                )?;
                for index in &consumed_class_arguments {
                    let parameter = constructor_parameters.get(*index).ok_or_else(|| {
                        malformed_mir(format!(
                            "constructor {} has no parameter {}",
                            constructor.name, index
                        ))
                    })?;
                    if local_in(constructor, *parameter)?.owned {
                        return Err(malformed_mir(format!(
                            "class#{} new expression gives constructor argument {} to a property and an owning constructor parameter",
                            class.0, index
                        )));
                    }
                }
                let alias_invalidations = properties
                    .iter()
                    .filter_map(|property| {
                        let mir::PropertyValueSource::ConstructorArgument(index) = property.source
                        else {
                            return None;
                        };
                        let definition = &class_definition.properties[property.property.index];
                        if !matches!(definition.ty, mir::Type::Class(_)) {
                            return None;
                        }
                        Some(PropertyAliasInvalidation {
                            receiver: constructor.params[0],
                            property: property.property,
                            alias: constructor_parameters[index],
                        })
                    })
                    .collect::<Vec<_>>();
                validate_class_local_lifetimes_with_aliases(constructor, &alias_invalidations)?;
            }
            Ok(())
        }
        mir::ClassExpression::Coalesce { left, right, .. } => {
            if left.class() != class || right.class() != class {
                return Err(malformed_mir("class coalesce has incompatible operands"));
            }
            validate_nullable_class_expression(program, function, left)?;
            validate_class_expression(program, function, right)
        }
        mir::ClassExpression::CollectionIndex {
            collection,
            index,
            transfer,
            positional,
            ..
        } => {
            let local = local_in(function, *collection)?;
            let mir::Type::Collection(collection_type) = local.ty else {
                return Err(malformed_mir("class index source is not a collection"));
            };
            let collection_type = collection_in(program, collection_type)?;
            if collection_type.value != mir::Type::Class(class) {
                return Err(malformed_mir("class index element type mismatch"));
            }
            validate_collection_element_access(
                program,
                function,
                local,
                collection_type,
                index,
                *transfer,
                *positional,
            )
        }
        mir::ClassExpression::MixedPayload { mixed, .. } => validate_mixed_payload_operand(
            function,
            *mixed,
            mir::MixedTag::Class(class),
            mir::Type::Class(class),
        ),
    }
}

fn validate_constructor_body_initializer(
    constructor: &mir::Function,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
    writable: bool,
) -> Result<(), BackendError> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum State {
        Uninitialized,
        Initialized,
        MaybeInitialized,
    }
    impl State {
        fn join(self, incoming: Self) -> Self {
            if self == incoming {
                self
            } else {
                Self::MaybeInitialized
            }
        }

        fn transfer_write(self) -> Self {
            // Fixpoint transfer records the write without judging its kind;
            // the converged validation pass below applies the strict rule.
            Self::Initialized
        }

        fn after_write(
            self,
            kind: mir::PropertyWriteKind,
            writable: bool,
            constructor: &mir::Function,
            property: crate::class_layout::PropertyId,
        ) -> Result<Self, BackendError> {
            match (kind, self) {
                (mir::PropertyWriteKind::Initialize, Self::Uninitialized) => {
                    Ok(Self::Initialized)
                }
                (mir::PropertyWriteKind::Initialize, _) => Err(malformed_mir(format!(
                    "constructor {} initializes property{} more than once on one path",
                    constructor.name, property.index
                ))),
                (mir::PropertyWriteKind::Replace, Self::Initialized) if writable => {
                    Ok(Self::Initialized)
                }
                (mir::PropertyWriteKind::Replace, _) => Err(malformed_mir(format!(
                    "constructor {} replaces property{} before it is definitely initialized or while it is readonly",
                    constructor.name, property.index
                ))),
                (mir::PropertyWriteKind::InitializeOrReplace, Self::MaybeInitialized)
                    if writable =>
                {
                    Ok(Self::Initialized)
                }
                (mir::PropertyWriteKind::InitializeOrReplace, _) => {
                    Err(malformed_mir(format!(
                        "constructor {} conditionally initializes property{} without a maybe-initialized writable obligation",
                        constructor.name, property.index
                    )))
                }
            }
        }
    }

    let (reachable, _) = reachable_blocks_and_predecessors(constructor, true)?;
    let mut inputs = vec![None; constructor.blocks.len()];
    let mut outputs = vec![None; constructor.blocks.len()];
    inputs[constructor.entry_block.0] = Some(State::Uninitialized);
    let mut pending = std::collections::VecDeque::from([constructor.entry_block]);
    let mut queued = vec![false; constructor.blocks.len()];
    queued[constructor.entry_block.0] = true;
    while let Some(block_id) = pending.pop_front() {
        queued[block_id.0] = false;
        let block = block_in(constructor, block_id)?;
        let mut state = inputs[block_id.0].expect("queued constructor block has input state");
        for statement in &block.statements {
            if let mir::Statement::AssignProperty {
                object,
                property: assigned,
                ..
            } = statement
            {
                if *object == receiver && *assigned == property {
                    state = state.transfer_write();
                }
            }
        }
        outputs[block_id.0] = Some(state);
        for successor in analysis_terminator_targets(&block.terminator, true) {
            if !reachable[successor.0] {
                continue;
            }
            let changed = match inputs[successor.0] {
                Some(current) => {
                    let joined = current.join(state);
                    if joined == current {
                        false
                    } else {
                        inputs[successor.0] = Some(joined);
                        true
                    }
                }
                None => {
                    inputs[successor.0] = Some(state);
                    true
                }
            };
            if changed && !queued[successor.0] {
                queued[successor.0] = true;
                pending.push_back(successor);
            }
        }
    }

    for block in constructor
        .blocks
        .iter()
        .filter(|block| inputs[block.id.0].is_some())
    {
        let mut state = inputs[block.id.0].expect("reachable constructor block state");
        for statement in &block.statements {
            if state != State::Initialized
                && statement_observes_property(statement, receiver, property)
            {
                return Err(malformed_mir(format!(
                    "constructor {} reads or exposes property{} before it is initialized",
                    constructor.name, property.index
                )));
            }
            if let mir::Statement::AssignProperty {
                object,
                property: assigned,
                kind,
                ..
            } = statement
            {
                if *object == receiver && *assigned == property {
                    state = state.after_write(*kind, writable, constructor, property)?;
                }
            }
        }
        if state != State::Initialized
            && terminator_observes_property(&block.terminator, receiver, property)
        {
            return Err(malformed_mir(format!(
                "constructor {} reads or exposes property{} before it is initialized",
                constructor.name, property.index
            )));
        }
        if state != State::Initialized
            && matches!(
                block.terminator,
                mir::Terminator::Return(_) | mir::Terminator::ReturnVoid
            )
        {
            return Err(malformed_mir(format!(
                "constructor {} can return without initializing property{}",
                constructor.name, property.index
            )));
        }
    }
    Ok(())
}

fn constructor_property_assignment_count(
    constructor: &mir::Function,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> Result<usize, BackendError> {
    let (reachable, _) = reachable_blocks_and_predecessors(constructor, true)?;
    Ok(constructor
        .blocks
        .iter()
        .filter(|block| reachable[block.id.0])
        .flat_map(|block| block.statements.iter())
        .filter(|statement| {
            matches!(
                statement,
                mir::Statement::AssignProperty {
                    object,
                    property: assigned,
                    ..
                } if *object == receiver && *assigned == property
            )
        })
        .count())
}

fn terminator_targets(terminator: &mir::Terminator) -> Vec<mir::BlockId> {
    match terminator {
        mir::Terminator::Jump(target) => vec![*target],
        mir::Terminator::Branch {
            then_block,
            else_block,
            ..
        } => vec![*then_block, *else_block],
        mir::Terminator::CheckedCall {
            success, failure, ..
        }
        | mir::Terminator::CheckedIndirectCall {
            success, failure, ..
        }
        | mir::Terminator::CheckedConstruct {
            success, failure, ..
        }
        | mir::Terminator::CheckedIo {
            success, failure, ..
        } => vec![*success, *failure],
        mir::Terminator::IndirectCall { continuation, .. } => vec![*continuation],
        mir::Terminator::ErrorSwitch {
            cases,
            catch_all,
            fallback,
            ..
        } => cases
            .iter()
            .map(|(_, target)| *target)
            .chain(catch_all.iter().copied())
            .chain(std::iter::once(*fallback))
            .collect(),
        mir::Terminator::Return(_)
        | mir::Terminator::ReturnVoid
        | mir::Terminator::Panic { .. }
        | mir::Terminator::Unreachable
        | mir::Terminator::PropagateError { .. } => Vec::new(),
    }
}

fn analysis_terminator_targets(
    terminator: &mir::Terminator,
    fold_constant_branches: bool,
) -> Vec<mir::BlockId> {
    if !fold_constant_branches {
        return terminator_targets(terminator);
    }
    match terminator {
        mir::Terminator::Branch {
            condition,
            then_block,
            else_block,
        } => match constant_bool_expression(condition) {
            Some(true) => vec![*then_block],
            Some(false) => vec![*else_block],
            None => vec![*then_block, *else_block],
        },
        _ => terminator_targets(terminator),
    }
}

fn constant_bool_expression(expression: &mir::BoolExpression) -> Option<bool> {
    match expression {
        mir::BoolExpression::Use {
            operand: mir::Operand::Scalar(mir::ScalarValue::Bool(value)),
        } => Some(*value),
        mir::BoolExpression::Not(value) => constant_bool_expression(value).map(|value| !value),
        mir::BoolExpression::Binary { op, left, right } => match op {
            mir::BoolBinaryOp::And => match constant_bool_expression(left) {
                Some(false) => Some(false),
                Some(true) => constant_bool_expression(right),
                None if constant_bool_expression(right) == Some(false) => Some(false),
                None => None,
            },
            mir::BoolBinaryOp::Or => match constant_bool_expression(left) {
                Some(true) => Some(true),
                Some(false) => constant_bool_expression(right),
                None if constant_bool_expression(right) == Some(true) => Some(true),
                None => None,
            },
            mir::BoolBinaryOp::Xor => {
                Some(constant_bool_expression(left)? ^ constant_bool_expression(right)?)
            }
        },
        _ => None,
    }
}

fn statement_observes_property(
    statement: &mir::Statement,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match statement {
        mir::Statement::AssignLocal { value, .. }
        | mir::Statement::AssignLocalGroup { value, .. } => {
            rvalue_observes_property(value, receiver, property)
        }
        mir::Statement::EchoStringLiteral(_)
        | mir::Statement::BindPayloadEnumFields { .. }
        | mir::Statement::MatchResultPlan { .. }
        | mir::Statement::ControlFlowPlan(_)
        | mir::Statement::DropClass { .. }
        | mir::Statement::DropString { .. }
        | mir::Statement::DropMixed { .. }
        | mir::Statement::EnsureErrorOrigin { .. }
        | mir::Statement::ExtractErrorObject { .. }
        | mir::Statement::DropError { .. }
        | mir::Statement::CollectionClear { .. }
        | mir::Statement::DropCollection { .. }
        | mir::Statement::DropPayloadEnum { .. }
        | mir::Statement::DropSharedReference { .. }
        | mir::Statement::DropWeakReference { .. }
        | mir::Statement::DropWritableSharedReference { .. }
        | mir::Statement::DropWritableWeakReference { .. }
        | mir::Statement::DropSharedReferenceAccess { .. }
        | mir::Statement::WriteStreamBytes { .. } => false,
        mir::Statement::AssignStatic { value, .. } => {
            rvalue_observes_property(value, receiver, property)
        }
        mir::Statement::EchoString(value) | mir::Statement::WriteStderr(value) => {
            string_observes_property(value, receiver, property)
        }
        mir::Statement::CallVoid { args, .. } | mir::Statement::CallBorrowed { args, .. } => args
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::Statement::CallNullSafe { object, args, .. } => {
            nullable_class_observes_property(object, receiver, property)
                || args
                    .iter()
                    .any(|value| rvalue_observes_property(value, receiver, property))
        }
        mir::Statement::Printf(format) => format_observes_property(format, receiver, property),
        mir::Statement::WriteFile { path, contents }
        | mir::Statement::AppendFile { path, contents } => {
            string_observes_property(path, receiver, property)
                || string_observes_property(contents, receiver, property)
        }
        mir::Statement::WriteFileBytes { path, .. } => {
            string_observes_property(path, receiver, property)
        }
        mir::Statement::AssignProperty { value, .. } => {
            rvalue_observes_property(value, receiver, property)
        }
        mir::Statement::CollectionAdd { value, .. } => {
            rvalue_observes_property(value, receiver, property)
        }
        mir::Statement::CollectionSet { key, value, .. }
        | mir::Statement::AssignCollectionIndex {
            index: key, value, ..
        } => {
            rvalue_observes_property(key, receiver, property)
                || rvalue_observes_property(value, receiver, property)
        }
        mir::Statement::BindClosureEnvironment { .. } | mir::Statement::DropFunction { .. } => {
            false
        }
    }
}

fn terminator_observes_property(
    terminator: &mir::Terminator,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match terminator {
        mir::Terminator::Return(value) => rvalue_observes_property(value, receiver, property),
        mir::Terminator::Panic { message: value, .. } => {
            string_observes_property(value, receiver, property)
        }
        mir::Terminator::Branch { condition, .. } => {
            bool_observes_property(condition, receiver, property)
        }
        mir::Terminator::CheckedCall { args, .. } => args
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::Terminator::IndirectCall { callee, args, .. }
        | mir::Terminator::CheckedIndirectCall { callee, args, .. } => {
            function_observes_property(callee, receiver, property)
                || args
                    .iter()
                    .any(|value| rvalue_observes_property(value, receiver, property))
        }
        mir::Terminator::CheckedConstruct {
            properties, args, ..
        } => {
            properties.iter().any(|property_value| {
                matches!(
                    &property_value.source,
                    mir::PropertyValueSource::Expression(value)
                        if rvalue_observes_property(value, receiver, property)
                )
            }) || args
                .iter()
                .any(|value| rvalue_observes_property(value, receiver, property))
        }
        mir::Terminator::CheckedIo { operation, .. } => {
            checked_io_observes_property(operation, receiver, property)
        }
        mir::Terminator::ReturnVoid | mir::Terminator::Unreachable | mir::Terminator::Jump(_) => {
            false
        }
        mir::Terminator::ErrorSwitch { .. } | mir::Terminator::PropagateError { .. } => false,
    }
}

fn checked_io_observes_property(
    operation: &mir::CheckedIoOperation,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match operation {
        mir::CheckedIoOperation::ReadLine { prompt } => {
            string_observes_property(prompt, receiver, property)
        }
        mir::CheckedIoOperation::ReadFile { path, .. } => {
            string_observes_property(path, receiver, property)
        }
        mir::CheckedIoOperation::ReadStdinBytes => false,
        mir::CheckedIoOperation::WriteFile { path, contents, .. } => {
            string_observes_property(path, receiver, property)
                || io_contents_observes_property(contents, receiver, property)
        }
        mir::CheckedIoOperation::WriteStream { contents, .. } => {
            io_contents_observes_property(contents, receiver, property)
        }
    }
}

fn io_contents_observes_property(
    contents: &mir::IoContents,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match contents {
        mir::IoContents::String(value) => string_observes_property(value, receiver, property),
        mir::IoContents::Format(value) => format_observes_property(value, receiver, property),
        mir::IoContents::Bytes(_) => false,
    }
}

fn rvalue_observes_property(
    value: &mir::Rvalue,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::Rvalue::Value(value) => value_observes_property(value, receiver, property),
        mir::Rvalue::String(value) => string_observes_property(value, receiver, property),
        mir::Rvalue::Mixed(value) => mixed_observes_property(value, receiver, property),
        mir::Rvalue::NullableScalar(value) => {
            nullable_scalar_observes_property(value, receiver, property)
        }
        mir::Rvalue::NullableString(value) => {
            nullable_string_observes_property(value, receiver, property)
        }
        mir::Rvalue::NullableMixed(value) => {
            nullable_mixed_observes_property(value, receiver, property)
        }
        mir::Rvalue::Error(value) => error_observes_property(value, receiver, property),
        mir::Rvalue::NullableError(value) => {
            nullable_error_observes_property(value, receiver, property)
        }
        mir::Rvalue::Class(value) => class_observes_property(value, receiver, property),
        mir::Rvalue::NullableClass(value) => {
            nullable_class_observes_property(value, receiver, property)
        }
        mir::Rvalue::Collection(value) => collection_observes_property(value, receiver, property),
        mir::Rvalue::NullableCollection(value) => {
            nullable_collection_observes_property(value, receiver, property)
        }
        mir::Rvalue::Function(value) => function_observes_property(value, receiver, property),
        mir::Rvalue::NullableFunction(value) => {
            nullable_function_observes_property(value, receiver, property)
        }
        mir::Rvalue::SharedReference(value) => {
            shared_reference_observes_property(value, receiver, property)
        }
        mir::Rvalue::WeakReference(value) => {
            weak_reference_observes_property(value, receiver, property)
        }
        mir::Rvalue::NullableSharedReference(value) => {
            nullable_shared_reference_observes_property(value, receiver, property)
        }
        mir::Rvalue::NullableWeakReference(value) => {
            nullable_weak_reference_observes_property(value, receiver, property)
        }
        mir::Rvalue::WritableSharedReference(value) => {
            writable_shared_reference_observes_property(value, receiver, property)
        }
        mir::Rvalue::WritableWeakReference(value) => {
            writable_weak_reference_observes_property(value, receiver, property)
        }
        mir::Rvalue::NullableWritableSharedReference(value) => {
            nullable_writable_shared_reference_observes_property(value, receiver, property)
        }
        mir::Rvalue::NullableWritableWeakReference(value) => {
            nullable_writable_weak_reference_observes_property(value, receiver, property)
        }
        mir::Rvalue::SharedReferenceAccess(value) => {
            shared_access_observes_property(value, receiver, property)
        }
        mir::Rvalue::NullableSharedReferenceAccess(value) => {
            nullable_shared_access_observes_property(value, receiver, property)
        }
        mir::Rvalue::PayloadEnum(value) => {
            payload_enum_observes_property(value, receiver, property)
        }
        mir::Rvalue::NullablePayloadEnum(value) => {
            nullable_payload_enum_observes_property(value, receiver, property)
        }
    }
}

fn function_observes_property(
    value: &mir::FunctionExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::FunctionExpression::Create { captures, .. } => captures.iter().any(|capture| {
            matches!(
                capture,
                mir::ClosureCaptureOperand::CopyValue(value)
                    | mir::ClosureCaptureOperand::MoveValue(value)
                    if rvalue_observes_property(value, receiver, property)
            )
        }),
        mir::FunctionExpression::Property {
            object,
            property: observed,
            ..
        } => *object == receiver && *observed == property,
        mir::FunctionExpression::Call { args, .. } => args
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::FunctionExpression::CollectionIndex { index, .. } => {
            rvalue_observes_property(index, receiver, property)
        }
        mir::FunctionExpression::AssumePresent { value, .. } => {
            nullable_function_observes_property(value, receiver, property)
        }
        mir::FunctionExpression::Local { .. } | mir::FunctionExpression::MixedPayload { .. } => {
            false
        }
    }
}

fn nullable_function_observes_property(
    value: &mir::NullableFunctionExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::NullableFunctionExpression::Present(value) => {
            function_observes_property(value, receiver, property)
        }
        mir::NullableFunctionExpression::Property {
            object,
            property: observed,
            ..
        } => *object == receiver && *observed == property,
        mir::NullableFunctionExpression::Call { args, .. } => args
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::NullableFunctionExpression::DictionaryGet { key, .. }
        | mir::NullableFunctionExpression::CollectionIndex { index: key, .. } => {
            rvalue_observes_property(key, receiver, property)
        }
        mir::NullableFunctionExpression::Null { .. }
        | mir::NullableFunctionExpression::Local { .. } => false,
    }
}

fn payload_enum_observes_property(
    value: &mir::PayloadEnumExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::PayloadEnumExpression::Construct { fields, .. }
        | mir::PayloadEnumExpression::Call { args: fields, .. } => fields
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::PayloadEnumExpression::Use { place, .. } => {
            payload_enum_place_observes_property(place, receiver, property)
        }
        mir::PayloadEnumExpression::Coalesce { left, right, .. } => {
            nullable_payload_enum_observes_property(left, receiver, property)
                || payload_enum_observes_property(right, receiver, property)
        }
    }
}

fn nullable_payload_enum_observes_property(
    value: &mir::NullablePayloadEnumExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::NullablePayloadEnumExpression::Null(_) => false,
        mir::NullablePayloadEnumExpression::Value(value) => {
            payload_enum_observes_property(value, receiver, property)
        }
        mir::NullablePayloadEnumExpression::Use { place, .. } => {
            payload_enum_place_observes_property(place, receiver, property)
        }
        mir::NullablePayloadEnumExpression::Call { args, .. } => args
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::NullablePayloadEnumExpression::CollectionGet { key, .. } => {
            rvalue_observes_property(key, receiver, property)
        }
        mir::NullablePayloadEnumExpression::Coalesce { left, right, .. } => {
            nullable_payload_enum_observes_property(left, receiver, property)
                || nullable_payload_enum_observes_property(right, receiver, property)
        }
    }
}

fn payload_enum_place_observes_property(
    place: &mir::PayloadEnumPlace,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match place {
        mir::PayloadEnumPlace::Property {
            object,
            property: observed,
        } => *object == receiver && *observed == property,
        mir::PayloadEnumPlace::CollectionIndex { index, .. } => {
            rvalue_observes_property(index, receiver, property)
        }
        mir::PayloadEnumPlace::Local(_)
        | mir::PayloadEnumPlace::NullableLocalAssumeNonNull(_)
        | mir::PayloadEnumPlace::Static(_)
        | mir::PayloadEnumPlace::MixedPayload { .. } => false,
    }
}

fn writable_shared_reference_observes_property(
    value: &mir::WritableSharedReferenceExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::WritableSharedReferenceExpression::New { value, .. } => {
            rvalue_observes_property(value, receiver, property)
        }
        mir::WritableSharedReferenceExpression::Property {
            object,
            property: observed,
            ..
        } => *object == receiver && *observed == property,
        mir::WritableSharedReferenceExpression::Call { args, .. } => args
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::WritableSharedReferenceExpression::Share { value, .. } => {
            writable_shared_reference_observes_property(value, receiver, property)
        }
        mir::WritableSharedReferenceExpression::Coalesce { left, right, .. } => {
            nullable_writable_shared_reference_observes_property(left, receiver, property)
                || writable_shared_reference_observes_property(right, receiver, property)
        }
        mir::WritableSharedReferenceExpression::CollectionIndex { index, .. } => {
            rvalue_observes_property(index, receiver, property)
        }
        mir::WritableSharedReferenceExpression::Local { .. }
        | mir::WritableSharedReferenceExpression::NullableLocalAssumeNonNull { .. } => false,
    }
}

fn writable_weak_reference_observes_property(
    value: &mir::WritableWeakReferenceExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::WritableWeakReferenceExpression::Property {
            object,
            property: observed,
            ..
        } => *object == receiver && *observed == property,
        mir::WritableWeakReferenceExpression::Call { args, .. } => args
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::WritableWeakReferenceExpression::Create { value, .. } => {
            writable_shared_reference_observes_property(value, receiver, property)
        }
        mir::WritableWeakReferenceExpression::Coalesce { left, right, .. } => {
            nullable_writable_weak_reference_observes_property(left, receiver, property)
                || writable_weak_reference_observes_property(right, receiver, property)
        }
        mir::WritableWeakReferenceExpression::CollectionIndex { index, .. } => {
            rvalue_observes_property(index, receiver, property)
        }
        mir::WritableWeakReferenceExpression::Local { .. }
        | mir::WritableWeakReferenceExpression::NullableLocalAssumeNonNull { .. } => false,
    }
}

fn nullable_writable_shared_reference_observes_property(
    value: &mir::NullableWritableSharedReferenceExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::NullableWritableSharedReferenceExpression::Strong(value) => {
            writable_shared_reference_observes_property(value, receiver, property)
        }
        mir::NullableWritableSharedReferenceExpression::Property {
            object,
            property: observed,
            ..
        } => *object == receiver && *observed == property,
        mir::NullableWritableSharedReferenceExpression::Call { args, .. } => args
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::NullableWritableSharedReferenceExpression::Acquire { value, .. } => {
            writable_weak_reference_observes_property(value, receiver, property)
        }
        mir::NullableWritableSharedReferenceExpression::NullSafeShare { value, .. } => {
            nullable_writable_shared_reference_observes_property(value, receiver, property)
        }
        mir::NullableWritableSharedReferenceExpression::NullSafeAcquire { value, .. } => {
            nullable_writable_weak_reference_observes_property(value, receiver, property)
        }
        mir::NullableWritableSharedReferenceExpression::Coalesce { left, right, .. } => {
            nullable_writable_shared_reference_observes_property(left, receiver, property)
                || nullable_writable_shared_reference_observes_property(right, receiver, property)
        }
        mir::NullableWritableSharedReferenceExpression::DictionaryGet { key, .. } => {
            rvalue_observes_property(key, receiver, property)
        }
        mir::NullableWritableSharedReferenceExpression::Null(_)
        | mir::NullableWritableSharedReferenceExpression::Local { .. } => false,
    }
}

fn nullable_writable_weak_reference_observes_property(
    value: &mir::NullableWritableWeakReferenceExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::NullableWritableWeakReferenceExpression::Weak(value) => {
            writable_weak_reference_observes_property(value, receiver, property)
        }
        mir::NullableWritableWeakReferenceExpression::Property {
            object,
            property: observed,
            ..
        } => *object == receiver && *observed == property,
        mir::NullableWritableWeakReferenceExpression::Call { args, .. } => args
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::NullableWritableWeakReferenceExpression::NullSafeCreate { value, .. } => {
            nullable_writable_shared_reference_observes_property(value, receiver, property)
        }
        mir::NullableWritableWeakReferenceExpression::Coalesce { left, right, .. } => {
            nullable_writable_weak_reference_observes_property(left, receiver, property)
                || nullable_writable_weak_reference_observes_property(right, receiver, property)
        }
        mir::NullableWritableWeakReferenceExpression::DictionaryGet { key, .. } => {
            rvalue_observes_property(key, receiver, property)
        }
        mir::NullableWritableWeakReferenceExpression::Null(_)
        | mir::NullableWritableWeakReferenceExpression::Local { .. } => false,
    }
}

fn shared_access_observes_property(
    value: &mir::SharedReferenceAccessExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::SharedReferenceAccessExpression::Property {
            object,
            property: observed,
            ..
        } => *object == receiver && *observed == property,
        mir::SharedReferenceAccessExpression::Call { args, .. } => args
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::SharedReferenceAccessExpression::Acquire { value, .. } => {
            writable_shared_reference_observes_property(value, receiver, property)
        }
        mir::SharedReferenceAccessExpression::CollectionIndex { index, .. } => {
            rvalue_observes_property(index, receiver, property)
        }
        mir::SharedReferenceAccessExpression::Local { .. }
        | mir::SharedReferenceAccessExpression::NullableLocalAssumeNonNull { .. } => false,
    }
}

fn nullable_shared_access_observes_property(
    value: &mir::NullableSharedReferenceAccessExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::NullableSharedReferenceAccessExpression::Access(value) => {
            shared_access_observes_property(value, receiver, property)
        }
        mir::NullableSharedReferenceAccessExpression::Property {
            object,
            property: observed,
            ..
        } => *object == receiver && *observed == property,
        mir::NullableSharedReferenceAccessExpression::Call { args, .. } => args
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::NullableSharedReferenceAccessExpression::NullSafeAcquire { value, .. } => {
            nullable_writable_shared_reference_observes_property(value, receiver, property)
        }
        mir::NullableSharedReferenceAccessExpression::CollectionIndex { index, .. } => {
            rvalue_observes_property(index, receiver, property)
        }
        mir::NullableSharedReferenceAccessExpression::CollectionGet { key, .. } => {
            rvalue_observes_property(key, receiver, property)
        }
        mir::NullableSharedReferenceAccessExpression::Null { .. }
        | mir::NullableSharedReferenceAccessExpression::Local { .. } => false,
    }
}

fn shared_reference_observes_property(
    value: &mir::SharedReferenceExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::SharedReferenceExpression::New { value, .. } => {
            class_observes_property(value, receiver, property)
        }
        mir::SharedReferenceExpression::Property {
            object,
            property: observed,
            ..
        } => *object == receiver && *observed == property,
        mir::SharedReferenceExpression::Call { args, .. } => args
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::SharedReferenceExpression::Share { value, .. } => {
            shared_reference_observes_property(value, receiver, property)
        }
        mir::SharedReferenceExpression::Coalesce { left, right, .. } => {
            nullable_shared_reference_observes_property(left, receiver, property)
                || shared_reference_observes_property(right, receiver, property)
        }
        mir::SharedReferenceExpression::CollectionIndex { index, .. } => {
            rvalue_observes_property(index, receiver, property)
        }
        mir::SharedReferenceExpression::Local { .. }
        | mir::SharedReferenceExpression::NullableLocalAssumeNonNull { .. } => false,
    }
}

fn weak_reference_observes_property(
    value: &mir::WeakReferenceExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::WeakReferenceExpression::Property {
            object,
            property: observed,
            ..
        } => *object == receiver && *observed == property,
        mir::WeakReferenceExpression::Call { args, .. } => args
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::WeakReferenceExpression::Create { value, .. } => {
            shared_reference_observes_property(value, receiver, property)
        }
        mir::WeakReferenceExpression::Coalesce { left, right, .. } => {
            nullable_weak_reference_observes_property(left, receiver, property)
                || weak_reference_observes_property(right, receiver, property)
        }
        mir::WeakReferenceExpression::CollectionIndex { index, .. } => {
            rvalue_observes_property(index, receiver, property)
        }
        mir::WeakReferenceExpression::Local { .. }
        | mir::WeakReferenceExpression::NullableLocalAssumeNonNull { .. } => false,
    }
}

fn nullable_shared_reference_observes_property(
    value: &mir::NullableSharedReferenceExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::NullableSharedReferenceExpression::Shared(value) => {
            shared_reference_observes_property(value, receiver, property)
        }
        mir::NullableSharedReferenceExpression::Property {
            object,
            property: observed,
            ..
        } => *object == receiver && *observed == property,
        mir::NullableSharedReferenceExpression::Call { args, .. } => args
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::NullableSharedReferenceExpression::Acquire { value, .. } => {
            weak_reference_observes_property(value, receiver, property)
        }
        mir::NullableSharedReferenceExpression::NullSafeShare { value, .. } => {
            nullable_shared_reference_observes_property(value, receiver, property)
        }
        mir::NullableSharedReferenceExpression::NullSafeAcquire { value, .. } => {
            nullable_weak_reference_observes_property(value, receiver, property)
        }
        mir::NullableSharedReferenceExpression::Coalesce { left, right, .. } => {
            nullable_shared_reference_observes_property(left, receiver, property)
                || nullable_shared_reference_observes_property(right, receiver, property)
        }
        mir::NullableSharedReferenceExpression::DictionaryGet { key, .. } => {
            rvalue_observes_property(key, receiver, property)
        }
        mir::NullableSharedReferenceExpression::CollectionIndex { index, .. } => {
            rvalue_observes_property(index, receiver, property)
        }
        mir::NullableSharedReferenceExpression::Null(_)
        | mir::NullableSharedReferenceExpression::Local { .. } => false,
    }
}

fn nullable_weak_reference_observes_property(
    value: &mir::NullableWeakReferenceExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::NullableWeakReferenceExpression::Weak(value) => {
            weak_reference_observes_property(value, receiver, property)
        }
        mir::NullableWeakReferenceExpression::Property {
            object,
            property: observed,
            ..
        } => *object == receiver && *observed == property,
        mir::NullableWeakReferenceExpression::Call { args, .. } => args
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::NullableWeakReferenceExpression::NullSafeCreate { value, .. } => {
            nullable_shared_reference_observes_property(value, receiver, property)
        }
        mir::NullableWeakReferenceExpression::Coalesce { left, right, .. } => {
            nullable_weak_reference_observes_property(left, receiver, property)
                || nullable_weak_reference_observes_property(right, receiver, property)
        }
        mir::NullableWeakReferenceExpression::DictionaryGet { key, .. } => {
            rvalue_observes_property(key, receiver, property)
        }
        mir::NullableWeakReferenceExpression::CollectionIndex { index, .. } => {
            rvalue_observes_property(index, receiver, property)
        }
        mir::NullableWeakReferenceExpression::Null(_)
        | mir::NullableWeakReferenceExpression::Local { .. } => false,
    }
}

fn value_observes_property(
    value: &mir::ValueExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::ValueExpression::Integer(value) => {
            integer_observes_property(value, receiver, property)
        }
        mir::ValueExpression::Float(value) => float_observes_property(value, receiver, property),
        mir::ValueExpression::Bool(value) => bool_observes_property(value, receiver, property),
        mir::ValueExpression::Enum(value) => enum_observes_property(value, receiver, property),
    }
}

fn enum_observes_property(
    value: &mir::EnumExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::EnumExpression::Use { operand, .. } => {
            operand_observes_property(operand, receiver, property)
        }
        mir::EnumExpression::Case(_) => false,
        mir::EnumExpression::Call { args, .. } => args
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::EnumExpression::Coalesce { left, right, .. } => {
            nullable_scalar_observes_property(left, receiver, property)
                || enum_observes_property(right, receiver, property)
        }
    }
}

fn collection_observes_property(
    value: &mir::CollectionExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::CollectionExpression::Literal { entries, .. } => entries.iter().any(|entry| {
            entry
                .key
                .as_ref()
                .is_some_and(|key| rvalue_observes_property(key, receiver, property))
                || rvalue_observes_property(&entry.value, receiver, property)
        }),
        mir::CollectionExpression::Fill { value, count, .. } => {
            rvalue_observes_property(value, receiver, property)
                || integer_observes_property(count, receiver, property)
        }
        mir::CollectionExpression::Index { index, .. } => {
            rvalue_observes_property(index, receiver, property)
        }
        mir::CollectionExpression::Property {
            object,
            property: observed,
            ..
        } => *object == receiver && *observed == property,
        mir::CollectionExpression::ReadFileBytes { path, .. } => {
            string_observes_property(path, receiver, property)
        }
        mir::CollectionExpression::Call { args, .. } => args
            .iter()
            .any(|argument| rvalue_observes_property(argument, receiver, property)),
        mir::CollectionExpression::StringIntrinsic(call) => call
            .args
            .iter()
            .any(|argument| rvalue_observes_property(argument, receiver, property)),
        mir::CollectionExpression::Local { .. }
        | mir::CollectionExpression::SharedAccessPayload { .. }
        | mir::CollectionExpression::From { .. }
        | mir::CollectionExpression::FromBytes { .. }
        | mir::CollectionExpression::BytesFromArray { .. }
        | mir::CollectionExpression::ReadStdinBytes { .. } => false,
    }
}

fn nullable_collection_observes_property(
    value: &mir::NullableCollectionExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::NullableCollectionExpression::Collection(value) => {
            collection_observes_property(value, receiver, property)
        }
        mir::NullableCollectionExpression::Property {
            object,
            property: observed,
            ..
        } => *object == receiver && *observed == property,
        mir::NullableCollectionExpression::Call { args, .. } => args
            .iter()
            .any(|argument| rvalue_observes_property(argument, receiver, property)),
        mir::NullableCollectionExpression::Coalesce { left, right, .. } => {
            nullable_collection_observes_property(left, receiver, property)
                || nullable_collection_observes_property(right, receiver, property)
        }
        mir::NullableCollectionExpression::Null(_)
        | mir::NullableCollectionExpression::Local { .. } => false,
    }
}

fn mixed_observes_property(
    value: &mir::MixedExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::MixedExpression::BoxValue(value) => value_observes_property(value, receiver, property),
        mir::MixedExpression::BoxString { value, .. } => {
            string_observes_property(value, receiver, property)
        }
        mir::MixedExpression::BoxClass { value, .. } => {
            class_observes_property(value, receiver, property)
        }
        mir::MixedExpression::BoxError { value } => {
            error_observes_property(value, receiver, property)
        }
        mir::MixedExpression::BoxPayloadEnum { value } => {
            payload_enum_observes_property(value, receiver, property)
        }
        mir::MixedExpression::BoxFunction { value, .. } => {
            function_observes_property(value, receiver, property)
        }
        mir::MixedExpression::Call { args, .. } => args
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::MixedExpression::CollectionIndex { index, .. } => {
            rvalue_observes_property(index, receiver, property)
        }
        mir::MixedExpression::Local { .. } | mir::MixedExpression::Property { .. } => false,
    }
}

fn error_observes_property(
    value: &mir::ErrorExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::ErrorExpression::FromClass { object, .. } => {
            class_observes_property(object, receiver, property)
        }
        mir::ErrorExpression::FromNullableClass { object, .. } => {
            nullable_class_observes_property(object, receiver, property)
        }
        mir::ErrorExpression::Property {
            object,
            property: observed,
            ..
        } => *object == receiver && *observed == property,
        mir::ErrorExpression::Call { args, .. } => args
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::ErrorExpression::CollectionIndex { index, .. } => {
            rvalue_observes_property(index, receiver, property)
        }
        mir::ErrorExpression::Local { .. }
        | mir::ErrorExpression::NullableLocalAssumeNonNull { .. }
        | mir::ErrorExpression::MixedPayload { .. } => false,
    }
}

fn nullable_error_observes_property(
    value: &mir::NullableErrorExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::NullableErrorExpression::Error(value) => {
            error_observes_property(value, receiver, property)
        }
        mir::NullableErrorExpression::Property {
            object,
            property: observed,
            ..
        } => *object == receiver && *observed == property,
        mir::NullableErrorExpression::Call { args, .. } => args
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::NullableErrorExpression::DictionaryGet { key, .. }
        | mir::NullableErrorExpression::CollectionIndex { index: key, .. } => {
            rvalue_observes_property(key, receiver, property)
        }
        mir::NullableErrorExpression::Null | mir::NullableErrorExpression::Local { .. } => false,
    }
}

fn nullable_mixed_observes_property(
    value: &mir::NullableMixedExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::NullableMixedExpression::Mixed(value) => {
            mixed_observes_property(value, receiver, property)
        }
        mir::NullableMixedExpression::BoxNullablePayloadEnum(value) => {
            nullable_payload_enum_observes_property(value, receiver, property)
        }
        mir::NullableMixedExpression::Call { args, .. } => args
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::NullableMixedExpression::Coalesce { left, right, .. } => {
            nullable_mixed_observes_property(left, receiver, property)
                || nullable_mixed_observes_property(right, receiver, property)
        }
        mir::NullableMixedExpression::Null
        | mir::NullableMixedExpression::Local { .. }
        | mir::NullableMixedExpression::Property { .. } => false,
    }
}

fn operand_observes_property(
    operand: &mir::Operand,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    matches!(
        operand,
        mir::Operand::Property {
            object,
            property: observed,
        } if *object == receiver && *observed == property
    )
}

fn integer_observes_property(
    value: &mir::IntegerExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::IntegerExpression::Use { operand, .. } => {
            operand_observes_property(operand, receiver, property)
        }
        mir::IntegerExpression::Unary { operand, .. }
        | mir::IntegerExpression::Convert { value: operand, .. } => {
            integer_observes_property(operand, receiver, property)
        }
        mir::IntegerExpression::Binary { left, right, .. } => {
            integer_observes_property(left, receiver, property)
                || integer_observes_property(right, receiver, property)
        }
        mir::IntegerExpression::FloatToInt { value, .. } => {
            float_observes_property(value, receiver, property)
        }
        mir::IntegerExpression::Call { args, .. } => args
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::IntegerExpression::Coalesce { left, right, .. } => {
            nullable_scalar_observes_property(left, receiver, property)
                || integer_observes_property(right, receiver, property)
        }
        mir::IntegerExpression::EnumBacking { value, .. } => {
            enum_observes_property(value, receiver, property)
        }
    }
}

fn float_observes_property(
    value: &mir::FloatExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::FloatExpression::Use { operand, .. } => {
            operand_observes_property(operand, receiver, property)
        }
        mir::FloatExpression::Negate { operand, .. } => {
            float_observes_property(operand, receiver, property)
        }
        mir::FloatExpression::Binary { left, right, .. } => {
            float_observes_property(left, receiver, property)
                || float_observes_property(right, receiver, property)
        }
        mir::FloatExpression::IntToFloat { value } => {
            integer_observes_property(value, receiver, property)
        }
        mir::FloatExpression::Call { args, .. } => args
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::FloatExpression::Coalesce { left, right, .. } => {
            nullable_scalar_observes_property(left, receiver, property)
                || float_observes_property(right, receiver, property)
        }
    }
}

fn string_observes_property(
    value: &mir::StringExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::StringExpression::Property {
            object,
            property: observed,
        } => *object == receiver && *observed == property,
        mir::StringExpression::Concat(parts) => parts
            .iter()
            .any(|part| string_observes_property(part, receiver, property)),
        mir::StringExpression::Display(value) => value_observes_property(value, receiver, property),
        mir::StringExpression::Call { args, .. } => args
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::StringExpression::ReadFile { path, .. } => {
            string_observes_property(path, receiver, property)
        }
        mir::StringExpression::Format(format) => {
            format_observes_property(format, receiver, property)
        }
        mir::StringExpression::Coalesce { left, right } => {
            nullable_string_observes_property(left, receiver, property)
                || string_observes_property(right, receiver, property)
        }
        mir::StringExpression::CollectionIndex { index, .. } => {
            rvalue_observes_property(index, receiver, property)
        }
        mir::StringExpression::CollectionKeyAt { offset, .. } => {
            rvalue_observes_property(offset, receiver, property)
        }
        mir::StringExpression::Intrinsic(call) => call
            .args
            .iter()
            .any(|argument| rvalue_observes_property(argument, receiver, property)),
        mir::StringExpression::EnumBacking { value, .. } => {
            enum_observes_property(value, receiver, property)
        }
        mir::StringExpression::ErrorMessage(error) => {
            error_observes_property(error, receiver, property)
        }
        mir::StringExpression::Literal(_)
        | mir::StringExpression::Local(_)
        | mir::StringExpression::Static(_)
        | mir::StringExpression::MixedPayload(_)
        | mir::StringExpression::NullableLocalAssumeNonNull(_) => false,
    }
}

fn nullable_string_observes_property(
    value: &mir::NullableStringExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::NullableStringExpression::String(value) => {
            string_observes_property(value, receiver, property)
        }
        mir::NullableStringExpression::Property {
            object,
            property: observed,
        } => *object == receiver && *observed == property,
        mir::NullableStringExpression::Call { args, .. } => args
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::NullableStringExpression::EnumBacking { value, .. } => {
            nullable_scalar_observes_property(value, receiver, property)
        }
        mir::NullableStringExpression::NullSafeProperty { object, .. } => {
            nullable_class_observes_property(object, receiver, property)
        }
        mir::NullableStringExpression::NullSafeCall { object, args, .. } => {
            nullable_class_observes_property(object, receiver, property)
                || args
                    .iter()
                    .any(|value| rvalue_observes_property(value, receiver, property))
        }
        mir::NullableStringExpression::Coalesce { left, right } => {
            nullable_string_observes_property(left, receiver, property)
                || nullable_string_observes_property(right, receiver, property)
        }
        mir::NullableStringExpression::DictionaryGet { key, .. } => {
            rvalue_observes_property(key, receiver, property)
        }
        mir::NullableStringExpression::Intrinsic(call) => call
            .args
            .iter()
            .any(|argument| rvalue_observes_property(argument, receiver, property)),
        mir::NullableStringExpression::ReadLine { prompt, .. } => {
            string_observes_property(prompt, receiver, property)
        }
        mir::NullableStringExpression::Null
        | mir::NullableStringExpression::Local(_)
        | mir::NullableStringExpression::Static(_) => false,
    }
}

fn class_observes_property(
    value: &mir::ClassExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::ClassExpression::Local { local, .. }
        | mir::ClassExpression::NullableLocalAssumeNonNull { local, .. } => *local == receiver,
        mir::ClassExpression::Property {
            object,
            property: observed,
            ..
        } => *object == receiver && *observed == property,
        mir::ClassExpression::Call { args, .. } => args
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::ClassExpression::New {
            properties, args, ..
        } => {
            properties.iter().any(|value| {
                matches!(
                    &value.source,
                    mir::PropertyValueSource::Expression(value)
                        if rvalue_observes_property(value, receiver, property)
                )
            }) || args
                .iter()
                .any(|value| rvalue_observes_property(value, receiver, property))
        }
        mir::ClassExpression::Coalesce { left, right, .. } => {
            nullable_class_observes_property(left, receiver, property)
                || class_observes_property(right, receiver, property)
        }
        mir::ClassExpression::CollectionIndex { index, .. } => {
            rvalue_observes_property(index, receiver, property)
        }
        mir::ClassExpression::MixedPayload { .. } => false,
        mir::ClassExpression::SharedPayload { reference, .. } => {
            shared_reference_observes_property(reference, receiver, property)
        }
        mir::ClassExpression::SharedAccessPayload { .. } => false,
    }
}

fn bool_observes_property(
    value: &mir::BoolExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::BoolExpression::Use { operand } => {
            operand_observes_property(operand, receiver, property)
        }
        mir::BoolExpression::Compare { left, right, .. } => {
            value_observes_property(left, receiver, property)
                || value_observes_property(right, receiver, property)
        }
        mir::BoolExpression::StringCompare { left, right, .. } => {
            string_observes_property(left, receiver, property)
                || string_observes_property(right, receiver, property)
        }
        mir::BoolExpression::NullableStringCompare { left, right, .. } => {
            nullable_string_observes_property(left, receiver, property)
                || nullable_string_observes_property(right, receiver, property)
        }
        mir::BoolExpression::NullableScalarIsPresent(value) => {
            nullable_scalar_observes_property(value, receiver, property)
        }
        mir::BoolExpression::NullableErrorIsPresent(value) => {
            nullable_error_observes_property(value, receiver, property)
        }
        mir::BoolExpression::NullableClassIsPresent(value) => {
            nullable_class_observes_property(value, receiver, property)
        }
        mir::BoolExpression::NullableFunctionIsPresent(value) => {
            nullable_function_observes_property(value, receiver, property)
        }
        mir::BoolExpression::NullableCollectionIsPresent(value) => {
            nullable_collection_observes_property(value, receiver, property)
        }
        mir::BoolExpression::NullableSharedReferenceIsPresent(value) => {
            nullable_shared_reference_observes_property(value, receiver, property)
        }
        mir::BoolExpression::NullableWeakReferenceIsPresent(value) => {
            nullable_weak_reference_observes_property(value, receiver, property)
        }
        mir::BoolExpression::NullableWritableSharedReferenceIsPresent(value) => {
            nullable_writable_shared_reference_observes_property(value, receiver, property)
        }
        mir::BoolExpression::NullableWritableWeakReferenceIsPresent(value) => {
            nullable_writable_weak_reference_observes_property(value, receiver, property)
        }
        mir::BoolExpression::NullableSharedReferenceAccessIsPresent(value) => {
            nullable_shared_access_observes_property(value, receiver, property)
        }
        mir::BoolExpression::NullableMixedIsPresent(value) => {
            nullable_mixed_observes_property(value, receiver, property)
        }
        mir::BoolExpression::NullablePayloadEnumIsPresent(value) => {
            nullable_payload_enum_observes_property(value, receiver, property)
        }
        mir::BoolExpression::PayloadEnumCompare { left, right, .. } => {
            payload_enum_observes_property(left, receiver, property)
                || payload_enum_observes_property(right, receiver, property)
        }
        mir::BoolExpression::PayloadEnumIsCase { .. } => false,
        mir::BoolExpression::NullablePayloadEnumCompare { left, right, .. } => {
            nullable_payload_enum_observes_property(left, receiver, property)
                || nullable_payload_enum_observes_property(right, receiver, property)
        }
        mir::BoolExpression::MixedIs { mixed, .. } => {
            mixed_observes_property(mixed, receiver, property)
        }
        mir::BoolExpression::Not(value) => bool_observes_property(value, receiver, property),
        mir::BoolExpression::Binary { left, right, .. } => {
            bool_observes_property(left, receiver, property)
                || bool_observes_property(right, receiver, property)
        }
        mir::BoolExpression::Call { args, .. } => args
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::BoolExpression::CollectionEqual { .. } => false,
        mir::BoolExpression::Coalesce { left, right } => {
            nullable_scalar_observes_property(left, receiver, property)
                || bool_observes_property(right, receiver, property)
        }
        mir::BoolExpression::CollectionHas { value, .. } => {
            rvalue_observes_property(value, receiver, property)
        }
        mir::BoolExpression::CollectionIsEmpty { .. } => false,
    }
}

fn nullable_scalar_observes_property(
    value: &mir::NullableScalarExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::NullableScalarExpression::Value(value) => {
            value_observes_property(value, receiver, property)
        }
        mir::NullableScalarExpression::Property {
            object,
            property: observed,
            ..
        } => *object == receiver && *observed == property,
        mir::NullableScalarExpression::Call { args, .. } => args
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::NullableScalarExpression::EnumBacking { value, .. } => {
            nullable_scalar_observes_property(value, receiver, property)
        }
        mir::NullableScalarExpression::NullSafeProperty { object, .. } => {
            nullable_class_observes_property(object, receiver, property)
        }
        mir::NullableScalarExpression::NullSafeCall { object, args, .. } => {
            nullable_class_observes_property(object, receiver, property)
                || args
                    .iter()
                    .any(|value| rvalue_observes_property(value, receiver, property))
        }
        mir::NullableScalarExpression::Coalesce { left, right, .. } => {
            nullable_scalar_observes_property(left, receiver, property)
                || nullable_scalar_observes_property(right, receiver, property)
        }
        mir::NullableScalarExpression::DictionaryGet { key, .. } => {
            rvalue_observes_property(key, receiver, property)
        }
        mir::NullableScalarExpression::CollectionIndexOf { value, .. } => {
            rvalue_observes_property(value, receiver, property)
        }
        mir::NullableScalarExpression::Parse { value, .. } => {
            string_observes_property(value, receiver, property)
        }
        mir::NullableScalarExpression::StringIntrinsic(call) => call
            .args
            .iter()
            .any(|argument| rvalue_observes_property(argument, receiver, property)),
        mir::NullableScalarExpression::Null(_)
        | mir::NullableScalarExpression::Local { .. }
        | mir::NullableScalarExpression::Static { .. } => false,
    }
}

fn nullable_class_observes_property(
    value: &mir::NullableClassExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    match value {
        mir::NullableClassExpression::Class(value) => {
            class_observes_property(value, receiver, property)
        }
        mir::NullableClassExpression::SharedPayload { .. } => false,
        mir::NullableClassExpression::Local { local, .. } => *local == receiver,
        mir::NullableClassExpression::Property {
            object,
            property: observed,
            ..
        } => *object == receiver && *observed == property,
        mir::NullableClassExpression::Call { args, .. } => args
            .iter()
            .any(|value| rvalue_observes_property(value, receiver, property)),
        mir::NullableClassExpression::NullSafeProperty { object, .. } => {
            nullable_class_observes_property(object, receiver, property)
        }
        mir::NullableClassExpression::NullSafeCall { object, args, .. } => {
            nullable_class_observes_property(object, receiver, property)
                || args
                    .iter()
                    .any(|value| rvalue_observes_property(value, receiver, property))
        }
        mir::NullableClassExpression::Coalesce { left, right, .. } => {
            nullable_class_observes_property(left, receiver, property)
                || nullable_class_observes_property(right, receiver, property)
        }
        mir::NullableClassExpression::DictionaryGet { key, .. } => {
            rvalue_observes_property(key, receiver, property)
        }
        mir::NullableClassExpression::Null(_) => false,
    }
}

fn format_observes_property(
    format: &mir::FormatExpression,
    receiver: mir::LocalId,
    property: crate::class_layout::PropertyId,
) -> bool {
    format.arguments.iter().any(|argument| match argument {
        mir::FormatArgument::Value(value) => value_observes_property(value, receiver, property),
        mir::FormatArgument::String(value) | mir::FormatArgument::ClassDisplay(value) => {
            string_observes_property(value, receiver, property)
        }
    })
}

fn validate_float_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::FloatExpression,
) -> Result<(), BackendError> {
    match expression {
        mir::FloatExpression::Use { ty, operand } => {
            validate_float_operand(program, function, *ty, operand)
        }
        mir::FloatExpression::Negate { ty, operand } => {
            if operand.ty() != *ty {
                return Err(malformed_mir(format!(
                    "{ty} negate expression contains {} operand",
                    operand.ty()
                )));
            }
            validate_float_expression(program, function, operand)
        }
        mir::FloatExpression::Binary {
            ty, left, right, ..
        } => {
            if left.ty() != *ty || right.ty() != *ty {
                return Err(malformed_mir(format!(
                    "{ty} binary expression has {} and {} operands",
                    left.ty(),
                    right.ty()
                )));
            }
            validate_float_expression(program, function, left)?;
            validate_float_expression(program, function, right)
        }
        mir::FloatExpression::IntToFloat { value } => {
            if value.ty() != IntegerType::Int64 {
                return Err(malformed_mir("Int::toFloat operand is not canonical int"));
            }
            validate_integer_expression(program, function, value)
        }
        mir::FloatExpression::Call {
            ty,
            function: callee,
            args,
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type
                != mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Float(*ty)))
            {
                return Err(malformed_mir(
                    "float call targets a function with another return type",
                ));
            }
            validate_call_args(program, function, callee, args)
        }
        mir::FloatExpression::Coalesce { ty, left, right } => {
            if left.ty() != mir::ScalarType::Float(*ty) || right.ty() != *ty {
                return Err(malformed_mir("float coalesce has incompatible operands"));
            }
            validate_nullable_scalar_expression(program, function, left)?;
            validate_float_expression(program, function, right)
        }
    }
}

fn validate_call_args(
    program: &mir::Program,
    caller: &mir::Function,
    callee: &mir::Function,
    args: &[mir::Rvalue],
) -> Result<(), BackendError> {
    if !callee.checked_effects.is_empty() {
        return Err(malformed_mir(format!(
            "ordinary call targets throwing function {}",
            callee.name
        )));
    }
    validate_call_args_shape(program, caller, callee, args)
}

fn validate_checked_call_args(
    program: &mir::Program,
    caller: &mir::Function,
    callee: &mir::Function,
    args: &[mir::Rvalue],
) -> Result<(), BackendError> {
    if callee.checked_effects.is_empty() {
        return Err(malformed_mir(format!(
            "checked call targets nonthrowing function {}",
            callee.name
        )));
    }
    validate_call_args_shape(program, caller, callee, args)
}

fn validate_call_args_shape(
    program: &mir::Program,
    caller: &mir::Function,
    callee: &mir::Function,
    args: &[mir::Rvalue],
) -> Result<(), BackendError> {
    if program
        .classes
        .iter()
        .any(|class| class.constructor == Some(callee.id) || class.destructor == Some(callee.id))
    {
        return Err(malformed_mir(format!(
            "ordinary call targets lifecycle function {}",
            callee.name
        )));
    }
    if let (Some(method), Some(receiver_mode)) = (&callee.method, callee.receiver_mode) {
        let Some(mir::Rvalue::Class(receiver)) = args.first() else {
            return Err(malformed_mir(format!(
                "call to method class#{}::{} has no explicit borrowed receiver",
                method.class.0, method.name
            )));
        };
        if class_expression_transfers_receiver(receiver) {
            return Err(malformed_mir(format!(
                "call to method class#{}::{} transfers its receiver",
                method.class.0, method.name
            )));
        }
        if receiver.class() != method.class {
            return Err(malformed_mir(format!(
                "call to method class#{}::{} uses class#{} as receiver",
                method.class.0,
                method.name,
                receiver.class().0
            )));
        }
        if receiver_mode == mir::ReceiverMode::Writable {
            require_writable_class_expression(
                program,
                caller,
                receiver,
                &format!("call to method class#{}::{}", method.class.0, method.name),
            )?;
        }
    }
    validate_call_args_for_params(program, caller, callee, &callee.params, args, None)
}

fn validate_call_args_for_params(
    program: &mir::Program,
    caller: &mir::Function,
    callee: &mir::Function,
    params: &[mir::LocalId],
    args: &[mir::Rvalue],
    promoted_transfers: Option<&HashSet<usize>>,
) -> Result<(), BackendError> {
    if args.len() != params.len() {
        return Err(malformed_mir(format!(
            "call to {} expects {} arguments, got {}",
            callee.name,
            params.len(),
            args.len()
        )));
    }
    let mut borrowed_class_locals: HashMap<mir::LocalId, ClassBorrowMode> = HashMap::new();
    let mut transferred_class_locals = HashSet::new();
    let operation = format!("call to {}", callee.name);
    for (index, (argument, parameter)) in args.iter().zip(params).enumerate() {
        let parameter_definition = local_in(callee, *parameter)?;
        let parameter_type = parameter_definition.ty;
        if argument.ty() != parameter_type {
            return Err(malformed_mir(format!(
                "call to {} passes {} argument {} to {} parameter",
                callee.name,
                argument.ty(),
                index + 1,
                parameter_type
            )));
        }
        validate_rvalue(program, caller, argument)?;
        let mut accesses = ClassLocalAccesses::default();
        collect_rvalue_class_local_accesses(argument, &mut accesses);
        let mut argument_borrows = validate_ordered_class_accesses(
            program,
            &operation,
            &accesses,
            &borrowed_class_locals,
            &mut transferred_class_locals,
        )?;
        let class_like_parameter = matches!(
            parameter_type,
            mir::Type::Class(_) | mir::Type::NullableClass(_)
        );
        let promoted_transfer = promoted_transfers.is_some_and(|indices| indices.contains(&index));
        if matches!(parameter_type, mir::Type::Class(_)) {
            let mir::Rvalue::Class(expression) = argument else {
                unreachable!("class parameter type was checked against its argument")
            };
            if parameter_definition.owned
                && matches!(
                    expression,
                    mir::ClassExpression::Local {
                        transfer: false,
                        ..
                    } | mir::ClassExpression::Property { .. }
                )
            {
                return Err(malformed_mir(format!(
                    "call to {} borrows argument {} for an owned parameter",
                    callee.name,
                    index + 1
                )));
            } else if parameter_definition.owned || promoted_transfer {
                require_owned_class_expression(
                    expression,
                    &format!("call to {} argument {}", callee.name, index + 1),
                )?;
            } else if let mir::ClassExpression::Local {
                local,
                transfer: true,
                ..
            } = expression
            {
                require_owned_synthetic_argument_temp(caller, *local, &callee.name, index + 1)?;
            }
            if parameter_definition.writable {
                require_writable_class_expression(
                    program,
                    caller,
                    expression,
                    &format!("call to {} argument {}", callee.name, index + 1),
                )?;
            }
        } else if matches!(parameter_type, mir::Type::NullableClass(_)) {
            let mir::Rvalue::NullableClass(expression) = argument else {
                unreachable!("nullable class parameter type was checked against its argument")
            };
            if parameter_definition.owned || promoted_transfer {
                require_owned_nullable_class_expression(
                    expression,
                    &format!("call to {} argument {}", callee.name, index + 1),
                )?;
            } else if let mir::NullableClassExpression::Local {
                local,
                transfer: true,
                ..
            } = expression
            {
                require_owned_synthetic_argument_temp(caller, *local, &callee.name, index + 1)?;
            }
            if parameter_definition.writable {
                require_writable_nullable_class_expression(
                    program,
                    caller,
                    expression,
                    &format!("call to {} argument {}", callee.name, index + 1),
                )?;
            }
        } else if matches!(parameter_type, mir::Type::Mixed | mir::Type::NullableMixed) {
            let ownership = argument.mixed_ownership();
            if (parameter_definition.owned || promoted_transfer)
                && ownership == mir::MixedOwnership::None
            {
                return Err(malformed_mir(format!(
                    "call to {} borrows mixed argument {} for an owned parameter",
                    callee.name,
                    index + 1
                )));
            }
        } else if matches!(parameter_type, mir::Type::Collection(_))
            && !parameter_definition.owned
            && !promoted_transfer
        {
            if let mir::Rvalue::Collection(mir::CollectionExpression::Local {
                local,
                transfer: true,
                ..
            }) = argument
            {
                require_owned_synthetic_argument_temp(caller, *local, &callee.name, index + 1)?;
            }
        }
        if class_like_parameter && !parameter_definition.owned && !promoted_transfer {
            let mode = if parameter_definition.writable {
                ClassBorrowMode::Writable
            } else {
                ClassBorrowMode::Readonly
            };
            for local in escaping_class_local_borrows(program, argument)? {
                argument_borrows.insert(local, mode);
            }
        }
        for (local, mode) in argument_borrows {
            if transferred_class_locals.contains(&local) {
                return Err(class_access_error(
                    &operation,
                    "both borrows and transfers",
                    local,
                ));
            }
            if borrowed_class_locals
                .get(&local)
                .is_some_and(|previous| previous.conflicts_with(mode))
            {
                return Err(class_access_error(
                    &operation,
                    "takes overlapping writable borrows of",
                    local,
                ));
            }
            borrowed_class_locals.insert(local, mode);
        }
    }
    Ok(())
}

fn require_owned_synthetic_argument_temp(
    caller: &mir::Function,
    local: mir::LocalId,
    callee: &str,
    argument: usize,
) -> Result<(), BackendError> {
    let definition = local_in(caller, local)?;
    if definition.owned && definition.synthetic {
        return Ok(());
    }
    Err(malformed_mir(format!(
        "call to {callee} transfers argument {argument} into a borrowed parameter"
    )))
}

#[derive(Clone, Copy)]
enum ClassBorrowMode {
    Readonly,
    Writable,
}

impl ClassBorrowMode {
    fn conflicts_with(self, other: Self) -> bool {
        matches!(self, Self::Writable) || matches!(other, Self::Writable)
    }
}

fn validate_ordered_class_accesses(
    program: &mir::Program,
    operation: &str,
    accesses: &ClassLocalAccesses<'_>,
    active_borrows: &HashMap<mir::LocalId, ClassBorrowMode>,
    transfers: &mut HashSet<mir::LocalId>,
) -> Result<HashMap<mir::LocalId, ClassBorrowMode>, BackendError> {
    let mut property_borrows = HashMap::new();
    let mut call_entry_borrows = Vec::new();
    for access in accesses.iter() {
        match access {
            ClassLocalAccess::Borrow(local) => {
                if transfers.contains(&local) {
                    return Err(class_access_error(
                        operation,
                        "both borrows and transfers",
                        local,
                    ));
                }
            }
            ClassLocalAccess::PropertyBorrow(local, _) => {
                if transfers.contains(&local) {
                    return Err(class_access_error(
                        operation,
                        "both borrows and transfers",
                        local,
                    ));
                }
                property_borrows.insert(local, ClassBorrowMode::Readonly);
            }
            ClassLocalAccess::Transfer(local) => {
                if active_borrows.contains_key(&local) || property_borrows.contains_key(&local) {
                    return Err(class_access_error(
                        operation,
                        "both borrows and transfers",
                        local,
                    ));
                }
                if !transfers.insert(local) {
                    return Err(duplicate_class_transfer_error(operation, local));
                }
            }
            ClassLocalAccess::BeginCall => {
                call_entry_borrows.push(property_borrows.clone());
            }
            ClassLocalAccess::Call(function, args, parameter_offset) => {
                let entry_borrows = call_entry_borrows
                    .pop()
                    .ok_or_else(|| malformed_mir("class access call marker is unbalanced"))?;
                for (local, mode) in
                    borrowed_class_call_locals(program, function, args, parameter_offset)?
                {
                    if transfers.contains(&local) {
                        return Err(class_access_error(
                            operation,
                            "both borrows and transfers",
                            local,
                        ));
                    }
                    let conflicts = active_borrows
                        .get(&local)
                        .or_else(|| entry_borrows.get(&local))
                        .is_some_and(|previous| previous.conflicts_with(mode));
                    if conflicts {
                        return Err(class_access_error(
                            operation,
                            "takes overlapping writable borrows of",
                            local,
                        ));
                    }
                }
            }
        }
    }
    if !call_entry_borrows.is_empty() {
        return Err(malformed_mir("class access call marker is unbalanced"));
    }
    Ok(property_borrows)
}

fn class_access_error(operation: &str, action: &str, local: mir::LocalId) -> BackendError {
    malformed_mir(format!("{operation} {action} class local local{}", local.0))
}

fn duplicate_class_transfer_error(operation: &str, local: mir::LocalId) -> BackendError {
    malformed_mir(format!(
        "{operation} transfers class local local{} more than once",
        local.0
    ))
}

fn escaping_class_local_borrows(
    program: &mir::Program,
    argument: &mir::Rvalue,
) -> Result<Vec<mir::LocalId>, BackendError> {
    match argument {
        mir::Rvalue::Class(expression) => {
            escaping_class_expression_local_borrows(program, expression)
        }
        mir::Rvalue::NullableClass(expression) => {
            escaping_nullable_class_expression_local_borrows(program, expression)
        }
        _ => Ok(Vec::new()),
    }
}

fn escaping_class_expression_local_borrows(
    program: &mir::Program,
    expression: &mir::ClassExpression,
) -> Result<Vec<mir::LocalId>, BackendError> {
    match expression {
        mir::ClassExpression::Local {
            local,
            transfer: false,
            ..
        }
        | mir::ClassExpression::NullableLocalAssumeNonNull {
            local,
            transfer: false,
            ..
        }
        | mir::ClassExpression::Property { object: local, .. } => Ok(vec![*local]),
        mir::ClassExpression::Call {
            function,
            args,
            return_borrow: Some(return_borrow),
            ..
        } => escaping_class_expression_local_borrows(
            program,
            borrowed_call_source(program, *function, args, *return_borrow)?,
        ),
        mir::ClassExpression::Coalesce {
            left,
            right,
            transfer: false,
            ..
        } => {
            let mut locals = escaping_nullable_class_expression_local_borrows(program, left)?;
            extend_unique_locals(
                &mut locals,
                escaping_class_expression_local_borrows(program, right)?,
            );
            Ok(locals)
        }
        mir::ClassExpression::Local { transfer: true, .. }
        | mir::ClassExpression::NullableLocalAssumeNonNull { transfer: true, .. }
        | mir::ClassExpression::Call {
            return_borrow: None,
            ..
        }
        | mir::ClassExpression::New { .. }
        | mir::ClassExpression::Coalesce { transfer: true, .. }
        | mir::ClassExpression::CollectionIndex { .. }
        | mir::ClassExpression::MixedPayload { .. }
        | mir::ClassExpression::SharedPayload { .. } => Ok(Vec::new()),
        mir::ClassExpression::SharedAccessPayload { access, .. } => Ok(vec![*access]),
    }
}

fn escaping_nullable_class_expression_local_borrows(
    program: &mir::Program,
    expression: &mir::NullableClassExpression,
) -> Result<Vec<mir::LocalId>, BackendError> {
    match expression {
        mir::NullableClassExpression::Null(_) => Ok(Vec::new()),
        mir::NullableClassExpression::Class(expression) => {
            escaping_class_expression_local_borrows(program, expression)
        }
        mir::NullableClassExpression::SharedPayload { .. } => Ok(Vec::new()),
        mir::NullableClassExpression::Local {
            local,
            transfer: false,
            ..
        }
        | mir::NullableClassExpression::Property { object: local, .. } => Ok(vec![*local]),
        mir::NullableClassExpression::Call {
            function,
            args,
            return_borrow: Some(return_borrow),
            ..
        } => escaping_class_local_borrows(
            program,
            borrowed_call_rvalue_source(program, *function, args, *return_borrow)?,
        ),
        mir::NullableClassExpression::NullSafeProperty { object, .. } => {
            escaping_nullable_class_expression_local_borrows(program, object)
        }
        mir::NullableClassExpression::NullSafeCall {
            object,
            args,
            return_borrow: Some(return_borrow),
            ..
        } => match return_borrow.source {
            mir::BorrowSource::Receiver => {
                escaping_nullable_class_expression_local_borrows(program, object)
            }
            mir::BorrowSource::Parameter(index) => {
                let source = args.get(index).ok_or_else(|| {
                    malformed_mir("null-safe borrowed call has no source argument")
                })?;
                escaping_class_local_borrows(program, source)
            }
        },
        mir::NullableClassExpression::Coalesce {
            left,
            right,
            transfer: false,
            ..
        } => {
            let mut locals = escaping_nullable_class_expression_local_borrows(program, left)?;
            extend_unique_locals(
                &mut locals,
                escaping_nullable_class_expression_local_borrows(program, right)?,
            );
            Ok(locals)
        }
        mir::NullableClassExpression::DictionaryGet { collection, .. } => Ok(vec![*collection]),
        mir::NullableClassExpression::Local { transfer: true, .. }
        | mir::NullableClassExpression::Call {
            return_borrow: None,
            ..
        }
        | mir::NullableClassExpression::NullSafeCall {
            return_borrow: None,
            ..
        }
        | mir::NullableClassExpression::Coalesce { transfer: true, .. } => Ok(Vec::new()),
    }
}

fn extend_unique_locals(target: &mut Vec<mir::LocalId>, incoming: Vec<mir::LocalId>) {
    for local in incoming {
        if !target.contains(&local) {
            target.push(local);
        }
    }
}

fn borrowed_call_source<'a>(
    program: &mir::Program,
    function: mir::FunctionId,
    args: &'a [mir::Rvalue],
    return_borrow: mir::ReturnBorrow,
) -> Result<&'a mir::ClassExpression, BackendError> {
    let source = borrowed_call_rvalue_source(program, function, args, return_borrow)?;
    let mir::Rvalue::Class(source) = source else {
        let callee = function_in(program, function)?;
        return Err(malformed_mir(format!(
            "borrowed class call to {} has no class source argument",
            callee.name
        )));
    };
    Ok(source)
}

fn borrowed_call_rvalue_source<'a>(
    program: &mir::Program,
    function: mir::FunctionId,
    args: &'a [mir::Rvalue],
    return_borrow: mir::ReturnBorrow,
) -> Result<&'a mir::Rvalue, BackendError> {
    let callee = function_in(program, function)?;
    let index = match return_borrow.source {
        mir::BorrowSource::Receiver => 0,
        mir::BorrowSource::Parameter(index) => index + usize::from(callee.receiver_mode.is_some()),
    };
    args.get(index).ok_or_else(|| {
        malformed_mir(format!(
            "borrowed class call to {} has no class source argument",
            callee.name
        ))
    })
}

fn validate_condition(
    program: &mir::Program,
    function: &mir::Function,
    condition: &mir::BoolExpression,
) -> Result<(), BackendError> {
    match condition {
        mir::BoolExpression::Use { operand } => validate_bool_operand(program, function, operand),
        mir::BoolExpression::Compare { op, left, right } => {
            if left.ty() != right.ty() {
                return Err(malformed_mir(format!(
                    "comparison has {} and {} operands",
                    left.ty(),
                    right.ty()
                )));
            }
            if matches!(left.ty(), mir::ScalarType::Enum(_))
                && !matches!(op, mir::CompareOp::Equal | mir::CompareOp::NotEqual)
            {
                return Err(malformed_mir("ordered enum comparison is invalid"));
            }
            validate_value_expression(program, function, left)?;
            validate_value_expression(program, function, right)
        }
        mir::BoolExpression::StringCompare { left, right, .. } => {
            validate_string_expression(program, function, left)?;
            validate_string_expression(program, function, right)
        }
        mir::BoolExpression::NullableStringCompare { op, left, right } => {
            if !matches!(op, mir::CompareOp::Equal | mir::CompareOp::NotEqual) {
                return Err(malformed_mir(
                    "ordered nullable-string comparison is invalid",
                ));
            }
            validate_nullable_string_expression(program, function, left)?;
            validate_nullable_string_expression(program, function, right)
        }
        mir::BoolExpression::NullableScalarIsPresent(value) => {
            validate_nullable_scalar_expression(program, function, value)
        }
        mir::BoolExpression::NullableClassIsPresent(value) => {
            validate_nullable_class_expression(program, function, value)
        }
        mir::BoolExpression::NullableFunctionIsPresent(value) => {
            validate_nullable_function_expression(program, function, value)
        }
        mir::BoolExpression::NullableCollectionIsPresent(value) => {
            validate_nullable_collection_expression(program, function, value)
        }
        mir::BoolExpression::NullableSharedReferenceIsPresent(value) => {
            validate_nullable_shared_reference_expression(program, function, value)
        }
        mir::BoolExpression::NullableWeakReferenceIsPresent(value) => {
            validate_nullable_weak_reference_expression(program, function, value)
        }
        mir::BoolExpression::NullableWritableSharedReferenceIsPresent(value) => {
            validate_nullable_writable_shared_reference_expression(program, function, value)
        }
        mir::BoolExpression::NullableWritableWeakReferenceIsPresent(value) => {
            validate_nullable_writable_weak_reference_expression(program, function, value)
        }
        mir::BoolExpression::NullableSharedReferenceAccessIsPresent(value) => {
            validate_nullable_shared_reference_access_expression(program, function, value)
        }
        mir::BoolExpression::NullableMixedIsPresent(value) => {
            validate_nullable_mixed_expression(program, function, value)
        }
        mir::BoolExpression::NullableErrorIsPresent(value) => {
            validate_nullable_error_expression(program, function, value)
        }
        mir::BoolExpression::NullablePayloadEnumIsPresent(value) => {
            validate_nullable_payload_enum_expression(program, function, value)
        }
        mir::BoolExpression::PayloadEnumCompare { op, left, right } => {
            validate_payload_enum_comparison(program, function, *op, left, right)
        }
        mir::BoolExpression::PayloadEnumIsCase {
            local,
            ty,
            case,
            nullable,
        } => {
            let expected = if *nullable {
                mir::Type::NullablePayloadEnum(*ty)
            } else {
                mir::Type::PayloadEnum(*ty)
            };
            if local_in(function, *local)?.ty != expected {
                return Err(malformed_mir(
                    "payload-enum case test uses an incompatible local",
                ));
            }
            let definition = validate_payload_enum_type(program, *ty)?;
            definition
                .cases
                .get(case.index)
                .filter(|definition| definition.id == *case)
                .map(|_| ())
                .ok_or_else(|| malformed_mir("payload-enum case test references no case"))
        }
        mir::BoolExpression::NullablePayloadEnumCompare { op, left, right } => {
            if !matches!(op, mir::CompareOp::Equal | mir::CompareOp::NotEqual) {
                return Err(malformed_mir(
                    "ordered nullable payload-enum comparison is invalid",
                ));
            }
            if left.ty() != right.ty() {
                return Err(malformed_mir(
                    "nullable payload-enum comparison uses different enum types",
                ));
            }
            let definition = validate_payload_enum_type(program, left.ty())?;
            if !definition.capabilities.equality {
                return Err(malformed_mir(
                    "nullable payload-enum comparison requires equality-capable fields",
                ));
            }
            validate_nullable_payload_enum_expression(program, function, left)?;
            validate_nullable_payload_enum_expression(program, function, right)
        }
        mir::BoolExpression::MixedIs { mixed, tag } => {
            validate_mixed_expression(program, function, mixed)?;
            if let mir::MixedTag::Class(class) = tag {
                class_in(program, *class)?;
            }
            if let mir::MixedTag::Enum(enum_id) = tag {
                enum_in(program, *enum_id)?;
            }
            if let mir::MixedTag::PayloadEnum(payload) = tag {
                validate_payload_enum_type(program, *payload)?;
            }
            if let mir::MixedTag::Function(function_type) = tag {
                function_type_in(program, *function_type)?;
            }
            Ok(())
        }
        mir::BoolExpression::Not(condition) => validate_condition(program, function, condition),
        mir::BoolExpression::Binary { left, right, .. } => {
            validate_condition(program, function, left)?;
            validate_condition(program, function, right)
        }
        mir::BoolExpression::Call {
            function: callee,
            args,
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type
                != mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Bool))
            {
                return Err(malformed_mir("bool call targets a non-bool function"));
            }
            validate_call_args(program, function, callee, args)
        }
        mir::BoolExpression::Coalesce { left, right } => {
            if left.ty() != mir::ScalarType::Bool {
                return Err(malformed_mir("bool coalesce has a non-bool left operand"));
            }
            validate_nullable_scalar_expression(program, function, left)?;
            validate_condition(program, function, right)
        }
        mir::BoolExpression::CollectionHas {
            collection,
            value,
            op,
        } => {
            let local = local_in(function, *collection)?;
            let mir::Type::Collection(collection_type) = local.ty else {
                return Err(malformed_mir("collection has source is not a collection"));
            };
            let collection_type = collection_in(program, collection_type)?;
            let expected = if *op == mir::CollectionMembershipOp::Contains {
                collection_type.key.unwrap_or(collection_type.value)
            } else {
                collection_type.value
            };
            if value.ty() != expected {
                return Err(malformed_mir("collection has argument type mismatch"));
            }
            match (op, collection_type.kind) {
                (
                    mir::CollectionMembershipOp::Contains,
                    mir::CollectionKind::List
                    | mir::CollectionKind::TypedArray
                    | mir::CollectionKind::PriorityQueue
                    | mir::CollectionKind::Deque
                    | mir::CollectionKind::Dictionary
                    | mir::CollectionKind::SortedDictionary
                    | mir::CollectionKind::Set
                    | mir::CollectionKind::SortedSet,
                ) => {}
                (
                    mir::CollectionMembershipOp::ContainsValue,
                    mir::CollectionKind::Dictionary | mir::CollectionKind::SortedDictionary,
                ) => {}
                (
                    mir::CollectionMembershipOp::Add,
                    mir::CollectionKind::Set | mir::CollectionKind::SortedSet,
                ) if local.writable => {}
                (
                    mir::CollectionMembershipOp::Remove,
                    mir::CollectionKind::List
                    | mir::CollectionKind::Set
                    | mir::CollectionKind::SortedSet,
                ) if local.writable => {}
                (
                    mir::CollectionMembershipOp::Add,
                    mir::CollectionKind::Set | mir::CollectionKind::SortedSet,
                ) => {
                    return Err(malformed_mir(format!(
                        "set mutation uses readonly local{}",
                        local.id.0
                    )));
                }
                (
                    mir::CollectionMembershipOp::Remove,
                    mir::CollectionKind::List
                    | mir::CollectionKind::Set
                    | mir::CollectionKind::SortedSet,
                ) => {
                    return Err(malformed_mir(format!(
                        "collection removal uses readonly local{}",
                        local.id.0
                    )));
                }
                _ => {
                    return Err(malformed_mir(
                        "collection membership operation does not match its collection kind",
                    ));
                }
            }
            validate_rvalue(program, function, value)
        }
        mir::BoolExpression::CollectionIsEmpty { collection } => {
            let local = local_in(function, *collection)?;
            if !matches!(local.ty, mir::Type::Collection(_)) {
                return Err(malformed_mir("isEmpty source is not a collection"));
            }
            Ok(())
        }
        mir::BoolExpression::CollectionEqual { left, right } => {
            validate_bytes_local(program, function, *left)?;
            validate_bytes_local(program, function, *right)
        }
    }
}

fn validate_payload_enum_comparison(
    program: &mir::Program,
    function: &mir::Function,
    op: mir::CompareOp,
    left: &mir::PayloadEnumExpression,
    right: &mir::PayloadEnumExpression,
) -> Result<(), BackendError> {
    if !matches!(op, mir::CompareOp::Equal | mir::CompareOp::NotEqual) {
        return Err(malformed_mir("ordered payload-enum comparison is invalid"));
    }
    if left.ty() != right.ty() {
        return Err(malformed_mir(
            "payload-enum comparison uses different enum types",
        ));
    }
    let definition = validate_payload_enum_type(program, left.ty())?;
    if !definition.capabilities.equality {
        return Err(malformed_mir(
            "payload-enum comparison requires equality-capable fields",
        ));
    }
    validate_payload_enum_expression(program, function, left)?;
    validate_payload_enum_expression(program, function, right)
}

fn validate_integer_operand(
    program: &mir::Program,
    function: &mir::Function,
    ty: IntegerType,
    operand: &mir::Operand,
) -> Result<(), BackendError> {
    match operand {
        mir::Operand::Scalar(mir::ScalarValue::Integer(value)) if value.ty != ty => Err(
            malformed_mir(format!("{ty} expression contains {} constant", value.ty)),
        ),
        mir::Operand::Scalar(mir::ScalarValue::Integer(_)) => Ok(()),
        mir::Operand::Local(local) => {
            let definition = local_in(function, *local)?;
            if definition.ty != mir::Type::Scalar(mir::ScalarType::Integer(ty)) {
                return Err(malformed_mir(format!(
                    "{ty} expression uses local{} with type {}",
                    local.0, definition.ty
                )));
            }
            Ok(())
        }
        mir::Operand::NullablePayload(local) => {
            let definition = local_in(function, *local)?;
            if definition.ty != mir::Type::NullableScalar(mir::ScalarType::Integer(ty)) {
                return Err(malformed_mir(format!(
                    "{ty} expression uses nullable payload local{} with type {}",
                    local.0, definition.ty
                )));
            }
            Ok(())
        }
        mir::Operand::Static(id) => validate_static_operand(
            program,
            *id,
            mir::Type::Scalar(mir::ScalarType::Integer(ty)),
        ),
        mir::Operand::Property { object, property } => validate_property_operand(
            program,
            function,
            *object,
            *property,
            mir::Type::Scalar(mir::ScalarType::Integer(ty)),
        ),
        mir::Operand::Scalar(_) => Err(malformed_mir(
            "integer expression contains non-integer constant",
        )),
        mir::Operand::CollectionLength(local) => {
            let definition = local_in(function, *local)?;
            if ty != IntegerType::Int64 || !matches!(definition.ty, mir::Type::Collection(_)) {
                return Err(malformed_mir("collection length is not used as int/int64"));
            }
            Ok(())
        }
        mir::Operand::CollectionIndex {
            positional,
            collection,
            index,
            remove,
        } => {
            let local = local_in(function, *collection)?;
            let mir::Type::Collection(collection_type) = local.ty else {
                return Err(malformed_mir("integer index source is not a collection"));
            };
            let collection_type = collection_in(program, collection_type)?;
            if collection_type.value != mir::Type::Scalar(mir::ScalarType::Integer(ty)) {
                return Err(malformed_mir("integer index element type mismatch"));
            }
            validate_collection_element_access(
                program,
                function,
                local,
                collection_type,
                index,
                *remove,
                *positional,
            )
        }
        mir::Operand::CollectionKeyAt { collection, offset } => validate_collection_key_at(
            program,
            function,
            *collection,
            mir::Type::Scalar(mir::ScalarType::Integer(ty)),
            offset,
        ),
        mir::Operand::MixedPayload { mixed, tag } => validate_mixed_payload_operand(
            function,
            *mixed,
            *tag,
            mir::Type::Scalar(mir::ScalarType::Integer(ty)),
        ),
        mir::Operand::StringIntrinsic(call) => validate_string_intrinsic(
            program,
            function,
            call,
            mir::Type::Scalar(mir::ScalarType::Integer(ty)),
        ),
    }
}

/// Validates every operand a `bool` value can be read from.
///
/// Exhaustive for the same reason as [`validate_integer_operand`]: the
/// catch-all arm this replaced is what rejected `Operand::CollectionIndex`
/// bool element reads until they were reported as a bug.
fn validate_bool_operand(
    program: &mir::Program,
    function: &mir::Function,
    operand: &mir::Operand,
) -> Result<(), BackendError> {
    let expected = mir::Type::Scalar(mir::ScalarType::Bool);
    match operand {
        mir::Operand::Scalar(mir::ScalarValue::Bool(_)) => Ok(()),
        mir::Operand::Scalar(_) => Err(malformed_mir(
            "bool expression contains a non-bool constant",
        )),
        mir::Operand::Local(local) => {
            let definition = local_in(function, *local)?;
            if definition.ty != expected {
                return Err(malformed_mir(format!(
                    "bool expression uses local{} with type {}",
                    local.0, definition.ty
                )));
            }
            Ok(())
        }
        mir::Operand::NullablePayload(local) => {
            let definition = local_in(function, *local)?;
            if definition.ty != mir::Type::NullableScalar(mir::ScalarType::Bool) {
                return Err(malformed_mir(format!(
                    "bool expression uses nullable payload local{} with type {}",
                    local.0, definition.ty
                )));
            }
            Ok(())
        }
        mir::Operand::Static(id) => validate_static_operand(program, *id, expected),
        mir::Operand::Property { object, property } => {
            validate_property_operand(program, function, *object, *property, expected)
        }
        mir::Operand::CollectionLength(_) => Err(malformed_mir(
            "collection length is used as bool instead of int/int64",
        )),
        mir::Operand::CollectionIndex {
            positional,
            collection,
            index,
            remove,
        } => {
            let local = local_in(function, *collection)?;
            let mir::Type::Collection(collection_type) = local.ty else {
                return Err(malformed_mir("bool index source is not a collection"));
            };
            let collection_type = collection_in(program, collection_type)?;
            if collection_type.value != expected {
                return Err(malformed_mir("bool index element type mismatch"));
            }
            validate_collection_element_access(
                program,
                function,
                local,
                collection_type,
                index,
                *remove,
                *positional,
            )
        }
        mir::Operand::CollectionKeyAt { collection, offset } => {
            validate_collection_key_at(program, function, *collection, expected, offset)
        }
        mir::Operand::MixedPayload { mixed, tag } => {
            validate_mixed_payload_operand(function, *mixed, *tag, expected)
        }
        mir::Operand::StringIntrinsic(call) => {
            validate_string_intrinsic(program, function, call, expected)
        }
    }
}

/// Validates every operand a `float32`/`float` value can be read from.
///
/// The match is deliberately exhaustive, like [`validate_integer_operand`]: a
/// catch-all arm here reports a well-formed read as malformed MIR, which is how
/// `Operand::CollectionIndex` and `Operand::Static` stayed rejected for floats
/// long after both native backends and the interpreter lowered them.
fn validate_float_operand(
    program: &mir::Program,
    function: &mir::Function,
    ty: FloatType,
    operand: &mir::Operand,
) -> Result<(), BackendError> {
    let expected = mir::Type::Scalar(mir::ScalarType::Float(ty));
    match operand {
        mir::Operand::Scalar(mir::ScalarValue::Float(value)) if value.ty != ty => Err(
            malformed_mir(format!("{ty} expression contains {} constant", value.ty)),
        ),
        mir::Operand::Scalar(mir::ScalarValue::Float(_)) => Ok(()),
        mir::Operand::Scalar(_) => Err(malformed_mir(format!(
            "{ty} expression contains a non-float constant"
        ))),
        mir::Operand::Local(local) => {
            let definition = local_in(function, *local)?;
            if definition.ty != expected {
                return Err(malformed_mir(format!(
                    "{ty} expression uses local{} with type {}",
                    local.0, definition.ty
                )));
            }
            Ok(())
        }
        mir::Operand::NullablePayload(local) => {
            let definition = local_in(function, *local)?;
            if definition.ty != mir::Type::NullableScalar(mir::ScalarType::Float(ty)) {
                return Err(malformed_mir(format!(
                    "{ty} expression uses nullable payload local{} with type {}",
                    local.0, definition.ty
                )));
            }
            Ok(())
        }
        mir::Operand::Static(id) => validate_static_operand(program, *id, expected),
        mir::Operand::Property { object, property } => {
            validate_property_operand(program, function, *object, *property, expected)
        }
        mir::Operand::CollectionLength(_) => Err(malformed_mir(format!(
            "collection length is used as {ty} instead of int/int64"
        ))),
        mir::Operand::CollectionIndex {
            positional,
            collection,
            index,
            remove,
        } => {
            let local = local_in(function, *collection)?;
            let mir::Type::Collection(collection_type) = local.ty else {
                return Err(malformed_mir(format!(
                    "{ty} index source is not a collection"
                )));
            };
            let collection_type = collection_in(program, collection_type)?;
            if collection_type.value != expected {
                return Err(malformed_mir(format!("{ty} index element type mismatch")));
            }
            validate_collection_element_access(
                program,
                function,
                local,
                collection_type,
                index,
                *remove,
                *positional,
            )
        }
        mir::Operand::CollectionKeyAt { collection, offset } => {
            validate_collection_key_at(program, function, *collection, expected, offset)
        }
        mir::Operand::MixedPayload { mixed, tag } => {
            validate_mixed_payload_operand(function, *mixed, *tag, expected)
        }
        mir::Operand::StringIntrinsic(call) => {
            validate_string_intrinsic(program, function, call, expected)
        }
    }
}

fn validate_mixed_payload_operand(
    function: &mir::Function,
    mixed: mir::LocalId,
    tag: mir::MixedTag,
    expected: mir::Type,
) -> Result<(), BackendError> {
    let definition = local_in(function, mixed)?;
    if !matches!(definition.ty, mir::Type::Mixed | mir::Type::NullableMixed) {
        return Err(malformed_mir(format!(
            "mixed payload reads local{} with type {}",
            mixed.0, definition.ty
        )));
    }
    if tag.ty() != expected {
        return Err(malformed_mir(format!(
            "mixed payload tag {tag} is used as {expected}"
        )));
    }
    Ok(())
}

fn validate_string_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::StringExpression,
) -> Result<(), BackendError> {
    match expression {
        mir::StringExpression::Literal(_) => Ok(()),
        mir::StringExpression::Local(local) => {
            let definition = local_in(function, *local)?;
            if definition.ty != mir::Type::String {
                return Err(malformed_mir(format!(
                    "int local local{} is used as a string operand",
                    local.0
                )));
            }
            Ok(())
        }
        mir::StringExpression::Static(id) => {
            validate_static_operand(program, *id, mir::Type::String)
        }
        mir::StringExpression::MixedPayload(local) => validate_mixed_payload_operand(
            function,
            *local,
            mir::MixedTag::String,
            mir::Type::String,
        ),
        mir::StringExpression::Property { object, property } => {
            validate_property_operand(program, function, *object, *property, mir::Type::String)
        }
        mir::StringExpression::ErrorMessage(error) => {
            validate_error_expression(program, function, error)
        }
        mir::StringExpression::CollectionIndex {
            positional,
            collection,
            index,
            remove,
        } => {
            let local = local_in(function, *collection)?;
            let mir::Type::Collection(collection_type) = local.ty else {
                return Err(malformed_mir("string index source is not a collection"));
            };
            let collection_type = collection_in(program, collection_type)?;
            if collection_type.value != mir::Type::String {
                return Err(malformed_mir("string index element type mismatch"));
            }
            validate_collection_element_access(
                program,
                function,
                local,
                collection_type,
                index,
                *remove,
                *positional,
            )
        }
        mir::StringExpression::CollectionKeyAt { collection, offset } => {
            validate_collection_key_at(program, function, *collection, mir::Type::String, offset)
        }
        mir::StringExpression::NullableLocalAssumeNonNull(local) => {
            if local_in(function, *local)?.ty != mir::Type::NullableString {
                return Err(malformed_mir(
                    "nonnull string expression references another local type",
                ));
            }
            Ok(())
        }
        mir::StringExpression::Concat(parts) => {
            for part in parts {
                validate_string_expression(program, function, part)?;
            }
            Ok(())
        }
        mir::StringExpression::Display(value) => {
            validate_value_expression(program, function, value)
        }
        mir::StringExpression::Call {
            function: callee,
            args,
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type != mir::ReturnType::Value(mir::Type::String) {
                return Err(malformed_mir("string call targets a non-string function"));
            }
            validate_call_args(program, function, callee, args)
        }
        mir::StringExpression::ReadFile { path, .. } => {
            validate_string_expression(program, function, path)
        }
        mir::StringExpression::Format(format) => {
            validate_format_expression(program, function, format)
        }
        mir::StringExpression::Coalesce { left, right } => {
            validate_nullable_string_expression(program, function, left)?;
            validate_string_expression(program, function, right)
        }
        mir::StringExpression::Intrinsic(call) => {
            validate_string_intrinsic(program, function, call, mir::Type::String)
        }
        mir::StringExpression::EnumBacking { enum_id, value } => {
            let definition = enum_in(program, *enum_id)?;
            if definition.backing_type != Some(crate::enums::EnumBackingType::String) {
                return Err(malformed_mir(
                    "string backing projection targets a non-string-backed enum",
                ));
            }
            if value.enum_id() != *enum_id {
                return Err(malformed_mir(
                    "string backing projection uses a different enum type",
                ));
            }
            validate_enum_expression(program, function, value)
        }
    }
}

fn validate_nullable_string_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::NullableStringExpression,
) -> Result<(), BackendError> {
    match expression {
        mir::NullableStringExpression::Null => Ok(()),
        mir::NullableStringExpression::ReadLine { prompt, .. } => {
            validate_string_expression(program, function, prompt)
        }
        mir::NullableStringExpression::String(value) => {
            validate_string_expression(program, function, value)
        }
        mir::NullableStringExpression::Local(local) => {
            if local_in(function, *local)?.ty != mir::Type::NullableString {
                return Err(malformed_mir(
                    "nullable-string expression references another local type",
                ));
            }
            Ok(())
        }
        mir::NullableStringExpression::Static(id) => {
            validate_static_operand(program, *id, mir::Type::NullableString)
        }
        mir::NullableStringExpression::Property { object, property } => validate_property_operand(
            program,
            function,
            *object,
            *property,
            mir::Type::NullableString,
        ),
        mir::NullableStringExpression::Call {
            function: callee,
            args,
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type != mir::ReturnType::Value(mir::Type::NullableString) {
                return Err(malformed_mir(
                    "nullable-string call targets another return type",
                ));
            }
            validate_call_args(program, function, callee, args)
        }
        mir::NullableStringExpression::EnumBacking { enum_id, value } => {
            let definition = enum_in(program, *enum_id)?;
            if definition.backing_type != Some(crate::enums::EnumBackingType::String) {
                return Err(malformed_mir(
                    "nullable string backing projection targets a non-string-backed enum",
                ));
            }
            if value.ty() != mir::ScalarType::Enum(*enum_id) {
                return Err(malformed_mir(
                    "nullable string backing projection uses a different enum type",
                ));
            }
            validate_nullable_scalar_expression(program, function, value)
        }
        mir::NullableStringExpression::NullSafeProperty { object, property } => {
            let class = object.class();
            validate_nullable_class_expression(program, function, object)?;
            property_in(program, class, *property).and_then(|definition| {
                (matches!(definition.ty, mir::Type::String | mir::Type::NullableString))
                    .then_some(())
                    .ok_or_else(|| malformed_mir("null-safe property has another type"))
            })
        }
        mir::NullableStringExpression::NullSafeCall {
            object,
            function: callee,
            args,
        } => validate_null_safe_call(program, function, object, *callee, args, mir::Type::String),
        mir::NullableStringExpression::Coalesce { left, right } => {
            validate_nullable_string_expression(program, function, left)?;
            validate_nullable_string_expression(program, function, right)
        }
        mir::NullableStringExpression::DictionaryGet {
            collection,
            key,
            access,
            ..
        } => validate_dictionary_get(
            program,
            function,
            *collection,
            key,
            mir::Type::String,
            *access,
        ),
        mir::NullableStringExpression::Intrinsic(call) => {
            validate_string_intrinsic(program, function, call, mir::Type::NullableString)
        }
    }
}

fn validate_nullable_scalar_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::NullableScalarExpression,
) -> Result<(), BackendError> {
    let ty = expression.ty();
    match expression {
        mir::NullableScalarExpression::Null(_) => Ok(()),
        mir::NullableScalarExpression::Value(value) if value.ty() == ty => {
            validate_value_expression(program, function, value)
        }
        mir::NullableScalarExpression::Local { local, .. } => (local_in(function, *local)?.ty
            == mir::Type::NullableScalar(ty))
        .then_some(())
        .ok_or_else(|| malformed_mir("nullable scalar references another local type")),
        mir::NullableScalarExpression::Property {
            object, property, ..
        } => validate_property_operand(
            program,
            function,
            *object,
            *property,
            mir::Type::NullableScalar(ty),
        ),
        mir::NullableScalarExpression::Static { id, .. } => {
            validate_static_operand(program, *id, mir::Type::NullableScalar(ty))
        }
        mir::NullableScalarExpression::Call {
            function: callee,
            args,
            ..
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type != mir::ReturnType::Value(mir::Type::NullableScalar(ty)) {
                return Err(malformed_mir(
                    "nullable scalar call has another return type",
                ));
            }
            validate_call_args(program, function, callee, args)
        }
        mir::NullableScalarExpression::EnumBacking { enum_id, value } => {
            let definition = enum_in(program, *enum_id)?;
            if definition.backing_type != Some(crate::enums::EnumBackingType::Int) {
                return Err(malformed_mir(
                    "nullable integer backing projection targets a non-int-backed enum",
                ));
            }
            if value.ty() != mir::ScalarType::Enum(*enum_id) {
                return Err(malformed_mir(
                    "nullable integer backing projection uses a different enum type",
                ));
            }
            validate_nullable_scalar_expression(program, function, value)
        }
        mir::NullableScalarExpression::NullSafeProperty {
            object, property, ..
        } => {
            let class = object.class();
            validate_nullable_class_expression(program, function, object)?;
            property_in(program, class, *property).and_then(|definition| {
                (matches!(
                    definition.ty,
                    mir::Type::Scalar(actual) | mir::Type::NullableScalar(actual)
                        if actual == ty
                ))
                .then_some(())
                .ok_or_else(|| malformed_mir("null-safe property has another scalar type"))
            })
        }
        mir::NullableScalarExpression::NullSafeCall {
            object,
            function: callee,
            args,
            ..
        } => validate_null_safe_call(
            program,
            function,
            object,
            *callee,
            args,
            mir::Type::Scalar(ty),
        ),
        mir::NullableScalarExpression::Coalesce { left, right, .. } => {
            if left.ty() != ty || right.ty() != ty {
                return Err(malformed_mir(
                    "nullable scalar coalesce operands have another type",
                ));
            }
            validate_nullable_scalar_expression(program, function, left)?;
            validate_nullable_scalar_expression(program, function, right)
        }
        mir::NullableScalarExpression::DictionaryGet {
            collection,
            key,
            access,
            ..
        } => validate_dictionary_get(
            program,
            function,
            *collection,
            key,
            mir::Type::Scalar(ty),
            *access,
        ),
        mir::NullableScalarExpression::CollectionIndexOf { collection, value } => {
            let local = local_in(function, *collection)?;
            let mir::Type::Collection(collection_type) = local.ty else {
                return Err(malformed_mir("List::indexOf source is not a collection"));
            };
            let definition = collection_in(program, collection_type)?;
            if definition.kind != mir::CollectionKind::List || value.ty() != definition.value {
                return Err(malformed_mir(
                    "List::indexOf source or argument type does not match",
                ));
            }
            validate_rvalue(program, function, value)
        }
        mir::NullableScalarExpression::Parse {
            ty: parse_ty,
            value,
        } => {
            if *parse_ty != ty
                || !matches!(
                    ty,
                    mir::ScalarType::Integer(IntegerType::Int64)
                        | mir::ScalarType::Float(FloatType::Float64)
                )
            {
                return Err(malformed_mir("parse produces only `?int` or `?float`"));
            }
            validate_string_expression(program, function, value)
        }
        mir::NullableScalarExpression::StringIntrinsic(call) => {
            validate_string_intrinsic(program, function, call, mir::Type::NullableScalar(ty))
        }
        mir::NullableScalarExpression::Value(_) => {
            Err(malformed_mir("nullable scalar wraps another scalar type"))
        }
    }
}

fn nullable_shared_transfer_source_is_owned(
    expression: &mir::NullableSharedReferenceExpression,
) -> bool {
    matches!(expression, mir::NullableSharedReferenceExpression::Null(_))
        || expression.owned_temporary().is_some()
}

fn nullable_weak_transfer_source_is_owned(
    expression: &mir::NullableWeakReferenceExpression,
) -> bool {
    matches!(expression, mir::NullableWeakReferenceExpression::Null(_))
        || expression.owned_temporary().is_some()
}

fn validate_nullable_class_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::NullableClassExpression,
) -> Result<(), BackendError> {
    let class = expression.class();
    class_in(program, class)?;
    match expression {
        mir::NullableClassExpression::Null(_) => Ok(()),
        mir::NullableClassExpression::SharedPayload { reference, .. } => {
            if reference.class() != class {
                return Err(malformed_mir(
                    "nullable shared payload projection changes class",
                ));
            }
            validate_nullable_shared_reference_expression(program, function, reference)
        }
        mir::NullableClassExpression::Class(value) if value.class() == class => {
            validate_class_expression(program, function, value)
        }
        mir::NullableClassExpression::Local {
            local, transfer, ..
        } => {
            let definition = local_in(function, *local)?;
            if definition.ty != mir::Type::NullableClass(class) {
                return Err(malformed_mir(
                    "nullable class references another local type",
                ));
            }
            if *transfer && !definition.owned {
                return Err(malformed_mir("nullable class transfers a borrowed local"));
            }
            Ok(())
        }
        mir::NullableClassExpression::Property {
            object, property, ..
        } => validate_property_operand(
            program,
            function,
            *object,
            *property,
            mir::Type::NullableClass(class),
        ),
        mir::NullableClassExpression::Call {
            function: callee,
            args,
            return_borrow,
            ..
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type != mir::ReturnType::Value(mir::Type::NullableClass(class)) {
                return Err(malformed_mir("nullable class call has another return type"));
            }
            if *return_borrow != infer_function_return_borrow(program, callee)? {
                return Err(malformed_mir(
                    "nullable class call has inconsistent ownership",
                ));
            }
            validate_call_args(program, function, callee, args)
        }
        mir::NullableClassExpression::NullSafeProperty {
            object, property, ..
        } => {
            let receiver = object.class();
            validate_nullable_class_expression(program, function, object)?;
            property_in(program, receiver, *property).and_then(|definition| {
                (matches!(
                    definition.ty,
                    mir::Type::Class(actual) | mir::Type::NullableClass(actual)
                        if actual == class
                ))
                .then_some(())
                .ok_or_else(|| malformed_mir("null-safe property has another class type"))
            })
        }
        mir::NullableClassExpression::NullSafeCall {
            object,
            function: callee,
            args,
            return_borrow,
            ..
        } => {
            let callee_definition = function_in(program, *callee)?;
            if *return_borrow != infer_function_return_borrow(program, callee_definition)? {
                return Err(malformed_mir(
                    "null-safe class call has inconsistent ownership",
                ));
            }
            validate_null_safe_call(
                program,
                function,
                object,
                *callee,
                args,
                mir::Type::Class(class),
            )
        }
        mir::NullableClassExpression::Coalesce { left, right, .. } => {
            if left.class() != class || right.class() != class {
                return Err(malformed_mir(
                    "nullable class coalesce operands have another class",
                ));
            }
            validate_nullable_class_expression(program, function, left)?;
            validate_nullable_class_expression(program, function, right)
        }
        mir::NullableClassExpression::DictionaryGet {
            collection,
            key,
            access,
            ..
        } => validate_dictionary_get(
            program,
            function,
            *collection,
            key,
            mir::Type::Class(class),
            *access,
        ),
        mir::NullableClassExpression::Class(_) => {
            Err(malformed_mir("nullable class wraps another class type"))
        }
    }
}

fn validate_null_safe_call(
    program: &mir::Program,
    caller: &mir::Function,
    object: &mir::NullableClassExpression,
    callee: mir::FunctionId,
    args: &[mir::Rvalue],
    return_type: mir::Type,
) -> Result<(), BackendError> {
    validate_nullable_class_expression(program, caller, object)?;
    let callee = function_in(program, callee)?;
    if !callee.checked_effects.is_empty() {
        return Err(malformed_mir(
            "checked null-safe value call was not lowered to explicit checked control flow",
        ));
    }
    let Some(method) = &callee.method else {
        return Err(malformed_mir("null-safe call targets a free function"));
    };
    let nullable_return_type = match return_type {
        mir::Type::Scalar(ty) => mir::Type::NullableScalar(ty),
        mir::Type::String => mir::Type::NullableString,
        mir::Type::Mixed => mir::Type::NullableMixed,
        mir::Type::Error => mir::Type::NullableError,
        mir::Type::Class(class) => mir::Type::NullableClass(class),
        mir::Type::SharedReference(class) => mir::Type::NullableSharedReference(class),
        mir::Type::WeakReference(class) => mir::Type::NullableWeakReference(class),
        mir::Type::WritableSharedReference(payload) => {
            mir::Type::NullableWritableSharedReference(payload)
        }
        mir::Type::WritableWeakReference(payload) => {
            mir::Type::NullableWritableWeakReference(payload)
        }
        mir::Type::ReadonlySharedReferenceAccess(_)
        | mir::Type::WritableSharedReferenceAccess(_)
                | mir::Type::NullableReadonlySharedReferenceAccess(_)
                | mir::Type::NullableWritableSharedReferenceAccess(_) => {
            return Err(malformed_mir(
                "null-safe calls cannot return shared access objects because nullable access types do not exist",
            ))
        }
        mir::Type::Collection(collection) => mir::Type::NullableCollection(collection),
        mir::Type::PayloadEnum(payload) => mir::Type::NullablePayloadEnum(payload),
        mir::Type::Function(function_type) => mir::Type::NullableFunction(function_type),
        mir::Type::ClosureEnvironment(_) => {
            return Err(malformed_mir(
                "null-safe calls cannot expose closure environments",
            ));
        }
        mir::Type::NullableScalar(_)
        | mir::Type::NullableString
        | mir::Type::NullableMixed
        | mir::Type::NullableError
        | mir::Type::NullableClass(_)
        | mir::Type::NullableCollection(_)
        | mir::Type::NullableFunction(_)
        | mir::Type::NullablePayloadEnum(_)
        | mir::Type::NullableSharedReference(_)
        | mir::Type::NullableWeakReference(_)
        | mir::Type::NullableWritableSharedReference(_)
        | mir::Type::NullableWritableWeakReference(_) => {
            return Err(malformed_mir(
                "null-safe call validator requires a non-null result type",
            ))
        }
    };
    if method.class != object.class()
        || !matches!(
            callee.return_type,
            mir::ReturnType::Value(actual)
                if actual == return_type || actual == nullable_return_type
        )
    {
        return Err(malformed_mir(
            "null-safe call has an incompatible signature",
        ));
    }
    let Some((receiver, parameters)) = callee.params.split_first() else {
        return Err(malformed_mir("null-safe method has no receiver"));
    };
    if local_in(callee, *receiver)?.ty != mir::Type::Class(object.class()) {
        return Err(malformed_mir("null-safe method has another receiver type"));
    }
    validate_null_safe_method_receiver(program, caller, callee, object)?;
    validate_call_args_for_params(program, caller, callee, parameters, args, None)
}

fn validate_null_safe_statement_call(
    program: &mir::Program,
    caller: &mir::Function,
    object: &mir::NullableClassExpression,
    callee: mir::FunctionId,
    args: &[mir::Rvalue],
) -> Result<(), BackendError> {
    validate_nullable_class_expression(program, caller, object)?;
    let callee = function_in(program, callee)?;
    if !callee.checked_effects.is_empty() {
        return Err(malformed_mir(
            "checked null-safe statement call was not lowered to explicit checked control flow",
        ));
    }
    let Some(method) = &callee.method else {
        return Err(malformed_mir(
            "null-safe statement call targets a free function",
        ));
    };
    let discards_borrow = matches!(
        callee.return_type,
        mir::ReturnType::Value(mir::Type::Class(_) | mir::Type::NullableClass(_))
    ) && infer_function_return_borrow(program, callee)?.is_some();
    if method.class != object.class()
        || (!matches!(callee.return_type, mir::ReturnType::Void) && !discards_borrow)
    {
        return Err(malformed_mir(
            "null-safe statement call has an incompatible signature",
        ));
    }
    let Some((receiver, parameters)) = callee.params.split_first() else {
        return Err(malformed_mir("null-safe statement method has no receiver"));
    };
    if local_in(callee, *receiver)?.ty != mir::Type::Class(object.class()) {
        return Err(malformed_mir(
            "null-safe statement method has another receiver type",
        ));
    }
    validate_null_safe_method_receiver(program, caller, callee, object)?;
    validate_call_args_for_params(program, caller, callee, parameters, args, None)
}

fn validate_format_expression(
    program: &mir::Program,
    function: &mir::Function,
    format: &mir::FormatExpression,
) -> Result<(), BackendError> {
    use crate::format_string::{FormatConversion, FormatPiece};
    let mut borrowed_class_locals: HashMap<mir::LocalId, ClassBorrowMode> = HashMap::new();
    let mut transferred_class_locals = HashSet::new();
    let mut expected_index = 0_usize;
    for piece in &format.pieces {
        let FormatPiece::Argument { index, spec } = piece else {
            continue;
        };
        if *index as usize != expected_index {
            return Err(malformed_mir(
                "format argument indices are not in canonical evaluation order",
            ));
        }
        let argument = format
            .arguments
            .get(expected_index)
            .ok_or_else(|| malformed_mir("format argument index is out of bounds"))?;
        expected_index += 1;
        let valid = matches!(
            (spec.conversion, argument),
            (FormatConversion::Display, mir::FormatArgument::Value(_))
                | (FormatConversion::Display, mir::FormatArgument::String(_))
                | (
                    FormatConversion::Display,
                    mir::FormatArgument::ClassDisplay(_),
                )
                | (
                    FormatConversion::Decimal
                        | FormatConversion::HexLower
                        | FormatConversion::HexUpper
                        | FormatConversion::Octal
                        | FormatConversion::Binary,
                    mir::FormatArgument::Value(mir::ValueExpression::Integer(_)),
                )
                | (
                    FormatConversion::Float,
                    mir::FormatArgument::Value(mir::ValueExpression::Float(_)),
                )
        );
        if !valid {
            return Err(malformed_mir(
                "format conversion and argument type disagree",
            ));
        }
        match argument {
            mir::FormatArgument::Value(value) => {
                validate_value_expression(program, function, value)?
            }
            mir::FormatArgument::String(value) | mir::FormatArgument::ClassDisplay(value) => {
                validate_string_expression(program, function, value)?
            }
        }
        let mut accesses = ClassLocalAccesses::default();
        collect_format_argument_class_local_accesses(argument, &mut accesses);
        let mut argument_borrows = validate_ordered_class_accesses(
            program,
            "format expression",
            &accesses,
            &borrowed_class_locals,
            &mut transferred_class_locals,
        )?;
        let call = format_argument_call(argument);
        if matches!(argument, mir::FormatArgument::ClassDisplay(_)) && call.is_none() {
            return Err(malformed_mir(
                "class display argument is not lowered through a string call",
            ));
        }
        let call_borrows = call
            .map(|(callee, args)| borrowed_class_call_locals(program, callee, args, 0))
            .transpose()?
            .unwrap_or_default();
        if matches!(argument, mir::FormatArgument::ClassDisplay(_)) {
            for (local, mode) in call_borrows {
                argument_borrows.insert(local, mode);
            }
        }
        for (local, mode) in argument_borrows {
            if transferred_class_locals.contains(&local) {
                return Err(class_access_error(
                    "format expression",
                    "both borrows and transfers",
                    local,
                ));
            }
            if borrowed_class_locals
                .get(&local)
                .is_some_and(|previous| previous.conflicts_with(mode))
            {
                return Err(class_access_error(
                    "format expression",
                    "takes overlapping writable borrows of",
                    local,
                ));
            }
            borrowed_class_locals.insert(local, mode);
        }
    }
    if expected_index != format.arguments.len() {
        return Err(malformed_mir(
            "format expression contains unreferenced arguments",
        ));
    }
    Ok(())
}

fn collect_format_argument_class_local_accesses<'a>(
    argument: &'a mir::FormatArgument,
    accesses: &mut ClassLocalAccesses<'a>,
) {
    match argument {
        mir::FormatArgument::Value(value) => collect_value_class_local_accesses(value, accesses),
        mir::FormatArgument::String(value) | mir::FormatArgument::ClassDisplay(value) => {
            collect_string_class_local_accesses(value, accesses)
        }
    }
}

fn format_argument_call(
    argument: &mir::FormatArgument,
) -> Option<(mir::FunctionId, &[mir::Rvalue])> {
    match argument {
        mir::FormatArgument::Value(mir::ValueExpression::Integer(
            mir::IntegerExpression::Call { function, args, .. },
        ))
        | mir::FormatArgument::Value(mir::ValueExpression::Float(mir::FloatExpression::Call {
            function,
            args,
            ..
        }))
        | mir::FormatArgument::Value(mir::ValueExpression::Bool(mir::BoolExpression::Call {
            function,
            args,
        }))
        | mir::FormatArgument::String(mir::StringExpression::Call { function, args })
        | mir::FormatArgument::ClassDisplay(mir::StringExpression::Call { function, args }) => {
            Some((*function, args))
        }
        _ => None,
    }
}

fn borrowed_class_call_locals(
    program: &mir::Program,
    callee: mir::FunctionId,
    args: &[mir::Rvalue],
    parameter_offset: usize,
) -> Result<Vec<(mir::LocalId, ClassBorrowMode)>, BackendError> {
    let callee = function_in(program, callee)?;
    let mut borrows = Vec::new();
    for (argument, parameter) in args.iter().zip(callee.params.iter().skip(parameter_offset)) {
        let parameter = local_in(callee, *parameter)?;
        if !matches!(
            parameter.ty,
            mir::Type::Class(_) | mir::Type::NullableClass(_)
        ) || parameter.owned
        {
            continue;
        }
        let mode = if parameter.writable {
            ClassBorrowMode::Writable
        } else {
            ClassBorrowMode::Readonly
        };
        for local in escaping_class_local_borrows(program, argument)? {
            if let Some((_, existing)) = borrows.iter_mut().find(|(borrowed, _)| *borrowed == local)
            {
                if mode.conflicts_with(*existing) {
                    *existing = ClassBorrowMode::Writable;
                }
            } else {
                borrows.push((local, mode));
            }
        }
    }
    Ok(borrows)
}

fn function_in(
    program: &mir::Program,
    id: mir::FunctionId,
) -> Result<&mir::Function, BackendError> {
    program
        .functions
        .get(id.0)
        .filter(|function| function.id == id)
        .ok_or_else(|| malformed_mir(format!("FunctionId function{} does not exist", id.0)))
}

fn function_type_in(
    program: &mir::Program,
    id: mir::FunctionTypeId,
) -> Result<&mir::FunctionType, BackendError> {
    program
        .function_types
        .get(id.0)
        .filter(|function_type| function_type.id == id)
        .ok_or_else(|| malformed_mir(format!("function type#{} does not exist", id.0)))
}

fn closure_descriptor_in(
    program: &mir::Program,
    id: mir::ClosureDescriptorId,
) -> Result<&mir::ClosureDescriptor, BackendError> {
    program
        .closure_descriptors
        .get(id.0)
        .filter(|descriptor| descriptor.id == id)
        .ok_or_else(|| malformed_mir(format!("closure descriptor#{} does not exist", id.0)))
}

fn closure_environment_layout_in(
    program: &mir::Program,
    id: mir::ClosureEnvironmentLayoutId,
) -> Result<&mir::ClosureEnvironmentLayout, BackendError> {
    program
        .closure_environment_layouts
        .get(id.0)
        .filter(|layout| layout.id == id)
        .ok_or_else(|| {
            malformed_mir(format!(
                "closure environment layout#{} does not exist",
                id.0
            ))
        })
}

fn class_in(program: &mir::Program, id: ClassId) -> Result<&mir::Class, BackendError> {
    program
        .classes
        .get(id.0)
        .filter(|class| class.id == id)
        .ok_or_else(|| malformed_mir(format!("ClassId class#{} does not exist", id.0)))
}

fn collection_in(
    program: &mir::Program,
    id: mir::CollectionTypeId,
) -> Result<&mir::CollectionType, BackendError> {
    program
        .collection_types
        .get(id.0)
        .filter(|collection| collection.id == id)
        .ok_or_else(|| {
            malformed_mir(format!(
                "collection type collection#{} does not exist",
                id.0
            ))
        })
}

fn static_in(
    program: &mir::Program,
    id: mir::StaticId,
) -> Result<&mir::StaticProperty, BackendError> {
    program
        .statics
        .get(id.0)
        .filter(|property| property.id == id)
        .ok_or_else(|| malformed_mir(format!("static{} does not exist", id.0)))
}

fn validate_static_operand(
    program: &mir::Program,
    id: mir::StaticId,
    expected: mir::Type,
) -> Result<(), BackendError> {
    let property = static_in(program, id)?;
    if property.ty != expected {
        return Err(malformed_mir(format!(
            "static{} has type {} but is used as {}",
            id.0, property.ty, expected
        )));
    }
    Ok(())
}

fn property_in(
    program: &mir::Program,
    class: ClassId,
    id: crate::class_layout::PropertyId,
) -> Result<&mir::Property, BackendError> {
    let class_definition = class_in(program, class)?;
    if id.class != class {
        return Err(malformed_mir(format!(
            "property#{}:{} does not belong to class#{}",
            id.class.0, id.index, class.0
        )));
    }
    class_definition
        .properties
        .get(id.index)
        .filter(|property| property.id == id)
        .ok_or_else(|| malformed_mir(format!("property{} does not exist", id.index)))
}

fn validate_property_operand(
    program: &mir::Program,
    function: &mir::Function,
    object: mir::LocalId,
    property: crate::class_layout::PropertyId,
    expected: mir::Type,
) -> Result<(), BackendError> {
    let object_definition = local_in(function, object)?;
    let class = match object_definition.ty {
        mir::Type::Class(class) | mir::Type::NullableClass(class) => class,
        _ => {
            return Err(malformed_mir(format!(
                "property operand uses non-class local local{}",
                object.0
            )))
        }
    };
    let property_definition = property_in(program, class, property)?;
    if property_definition.ty != expected {
        return Err(malformed_mir(format!(
            "property{} has type {} but expression expects {}",
            property.index, property_definition.ty, expected
        )));
    }
    Ok(())
}

fn local_in(function: &mir::Function, id: mir::LocalId) -> Result<&mir::Local, BackendError> {
    function
        .locals
        .get(id.0)
        .filter(|local| local.id == id)
        .ok_or_else(|| malformed_mir(format!("LocalId local{} does not exist", id.0)))
}

fn block_in(function: &mir::Function, id: mir::BlockId) -> Result<&mir::BasicBlock, BackendError> {
    function
        .blocks
        .get(id.0)
        .filter(|block| block.id == id)
        .ok_or_else(|| malformed_mir(format!("BlockId block{} does not exist", id.0)))
}

fn malformed_mir(message: impl Into<String>) -> BackendError {
    BackendError::new(format!(
        "backend emission failure: malformed MIR: {}",
        message.into()
    ))
}

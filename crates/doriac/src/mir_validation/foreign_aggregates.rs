//! Foreign structs and unions (Decision 0148 section 6): every aggregate
//! type names its definition with that definition's size and alignment,
//! and every field access and construction reaches a field of the C layout
//! with the field's own C type.

use super::*;
use crate::c_layout::{CRecord, CType};

pub(super) fn definition_in(
    program: &mir::Program,
    ty: mir::ForeignAggregateType,
) -> Result<&mir::ForeignAggregateDefinition, BackendError> {
    let definition = program
        .foreign_aggregates
        .get(ty.id.0)
        .filter(|definition| definition.id == ty.id)
        .ok_or_else(|| malformed_mir("a foreign aggregate type names no definition"))?;
    if definition.ty() != ty || definition.layout.align == 0 {
        return Err(malformed_mir(
            "a foreign aggregate type disagrees with its definition's layout",
        ));
    }
    Ok(definition)
}

/// Whether `layout` has a field, or a nested field, at `offset` whose C
/// type is `expected`. Union members share offset zero, and any of them
/// may be the one accessed.
fn has_field(layout: &CRecord, offset: u64, expected: &CType) -> bool {
    layout.fields.iter().any(|field| {
        if field.offset == offset && field.ty == *expected {
            return true;
        }
        match &field.ty {
            CType::Record(inner) => offset
                .checked_sub(field.offset)
                .is_some_and(|inner_offset| has_field(inner, inner_offset, expected)),
            CType::Scalar(_) | CType::Array { .. } => false,
        }
    })
}

/// The field of the aggregate `aggregate` holds at `offset` has type `ty`.
fn validate_field(
    program: &mir::Program,
    aggregate: mir::ForeignAggregateType,
    offset: u64,
    ty: mir::Type,
) -> Result<(), BackendError> {
    let definition = definition_in(program, aggregate)?;
    let expected = mir::foreign_c_type(ty, &program.foreign_aggregates, CTarget::host())
        .ok_or_else(|| malformed_mir("a foreign field has a type without a C layout"))?;
    if !has_field(&definition.layout, offset, &expected) {
        return Err(malformed_mir(format!(
            "foreign aggregate {} has no field of type {ty} at offset {offset}",
            definition.name
        )));
    }
    Ok(())
}

use crate::c_layout::CTarget;

/// A fixed-length array field `T[length]` at `offset`, viewed or supplied
/// through the typed array region type `region`, whose element the
/// targets build.
fn validate_array_field(
    program: &mir::Program,
    aggregate: mir::ForeignAggregateType,
    offset: u64,
    region: mir::Type,
    length: u64,
) -> Result<(), BackendError> {
    let mir::Type::Region(region) = region else {
        return Err(malformed_mir(
            "a fixed-length array field is viewed through a region",
        ));
    };
    let (element, _) = crate::native_abi::array_region_element(program, region)
        .ok_or_else(|| malformed_mir("a fixed-length array field region views built elements"))?;
    let element = mir::foreign_c_type(element, &program.foreign_aggregates, CTarget::host())
        .ok_or_else(|| malformed_mir("a fixed-length array element has a C layout"))?;
    let expected = crate::c_layout::CType::Array {
        element: Box::new(element),
        length,
    };
    if !has_field(
        &definition_in(program, aggregate)?.layout,
        offset,
        &expected,
    ) {
        return Err(malformed_mir(format!(
            "the foreign aggregate has no fixed-length array field of {length} elements at offset {offset}"
        )));
    }
    Ok(())
}

/// A value stored into the field at `offset`, by a construction or a
/// write: of the field's type, or for a fixed-length array field of
/// `copied_elements` elements a readonly region whose elements are copied
/// (section 6).
fn validate_stored_field(
    program: &mir::Program,
    aggregate: mir::ForeignAggregateType,
    offset: u64,
    value: &mir::Rvalue,
    copied_elements: Option<u64>,
) -> Result<(), BackendError> {
    match copied_elements {
        Some(length) => {
            if value.ty().is_writable_region() {
                return Err(malformed_mir(
                    "a fixed-length array field is copied from a readonly region",
                ));
            }
            validate_array_field(program, aggregate, offset, value.ty(), length)
        }
        None => validate_field(program, aggregate, offset, value.ty()),
    }
}

/// The aggregate type `place` holds, and whether a field write may go
/// through it: a writable local, or a property that permits writes through
/// it on a writable receiver or on the constructor's direct receiver, as
/// checking proves of the place's object path (section 6).
fn place_facts(
    program: &mir::Program,
    function: &mir::Function,
    place: mir::ForeignAggregatePlace,
) -> Result<(mir::ForeignAggregateType, bool), BackendError> {
    let (ty, writable) = match place {
        mir::ForeignAggregatePlace::Local(local) => {
            let local = local_in(function, local)?;
            (local.ty, local.writable)
        }
        // Checking proved the `?Name` local present; its payload is the
        // aggregate.
        mir::ForeignAggregatePlace::NullableLocalAssumeNonNull(local) => {
            let local = local_in(function, local)?;
            let mir::Type::NullableForeignAggregate(aggregate) = local.ty else {
                return Err(malformed_mir(
                    "a narrowed foreign field access names a local without a `?Name` value",
                ));
            };
            (mir::Type::ForeignAggregate(aggregate), local.writable)
        }
        mir::ForeignAggregatePlace::Property { object, property } => {
            let object = local_in(function, object)?;
            let mir::Type::Class(class) = object.ty else {
                return Err(malformed_mir(
                    "a foreign field access names a property of a local without a class value",
                ));
            };
            let property = property_in(program, class, property)?;
            let receiver =
                object.writable || is_direct_constructor_receiver(program, function, object)?;
            (property.ty, receiver && property.permits_write_through())
        }
        // A static property is written through only when it is writable.
        mir::ForeignAggregatePlace::Static(id) => {
            let property = static_in(program, id)?;
            (property.ty, property.writable)
        }
    };
    let mir::Type::ForeignAggregate(aggregate) = ty else {
        return Err(malformed_mir(
            "a foreign field access names a place without a foreign aggregate",
        ));
    };
    Ok((aggregate, writable))
}

pub(super) fn validate_field_access(
    program: &mir::Program,
    function: &mir::Function,
    aggregate: mir::ForeignAggregatePlace,
    offset: u64,
    access: &mir::ForeignFieldAccess,
) -> Result<(), BackendError> {
    let (aggregate_type, writable) = place_facts(program, function, aggregate)?;
    match access {
        mir::ForeignFieldAccess::Read { target } => {
            let target = local_in(function, *target)?;
            validate_field(program, aggregate_type, offset, target.ty)
        }
        mir::ForeignFieldAccess::Write {
            value,
            copied_elements,
        } => {
            if !writable {
                return Err(malformed_mir(
                    "a foreign field write names a readonly place",
                ));
            }
            validate_stored_field(program, aggregate_type, offset, value, *copied_elements)?;
            validate_rvalue(program, function, value)
        }
        mir::ForeignFieldAccess::Region { target, length } => {
            let target = local_in(function, *target)?;
            if target.ty.is_writable_region() && !writable {
                return Err(malformed_mir(
                    "a writable fixed-length array field region names a readonly place",
                ));
            }
            validate_array_field(program, aggregate_type, offset, target.ty, *length)
        }
    }
}

/// Whether the constant `value` stores each field at the offset of a field
/// of its definition's layout with that field's C type, and a union at most
/// one member, each field's value being a constant of its type (section 6).
pub(super) fn constant_matches(
    program: &mir::Program,
    value: &mir::ForeignAggregateConstant,
) -> Result<bool, BackendError> {
    let layout = &definition_in(program, value.ty)?.layout;
    if layout.kind == crate::ast::ForeignAggregateKind::Union && value.fields.len() > 1 {
        return Ok(false);
    }
    for field in &value.fields {
        let Some(expected) =
            mir::foreign_c_type(field.ty, &program.foreign_aggregates, CTarget::host())
        else {
            return Ok(false);
        };
        let declared = layout
            .fields
            .iter()
            .any(|candidate| candidate.offset == field.offset && candidate.ty == expected);
        if !declared || !static_value_matches(program, &field.value, field.ty)? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) fn validate_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::ForeignAggregateExpression,
) -> Result<(), BackendError> {
    let ty = expression.ty();
    let definition = definition_in(program, ty)?;
    match expression {
        mir::ForeignAggregateExpression::Construct { fields, .. } => {
            if definition.layout.kind == crate::ast::ForeignAggregateKind::Union && fields.len() > 1
            {
                return Err(malformed_mir(
                    "a union construction stores at most one member",
                ));
            }
            for field in fields {
                validate_stored_field(
                    program,
                    ty,
                    field.offset,
                    &field.value,
                    field.copied_elements,
                )?;
                validate_rvalue(program, function, &field.value)?;
            }
            Ok(())
        }
        mir::ForeignAggregateExpression::Local { local, .. } => {
            if local_in(function, *local)?.ty != mir::Type::ForeignAggregate(ty) {
                return Err(malformed_mir(
                    "a foreign aggregate copy names a local of another type",
                ));
            }
            Ok(())
        }
        mir::ForeignAggregateExpression::Property {
            object, property, ..
        } => validate_property_operand(
            program,
            function,
            *object,
            *property,
            mir::Type::ForeignAggregate(ty),
        ),
        mir::ForeignAggregateExpression::Static { id, .. } => {
            validate_static_operand(program, *id, mir::Type::ForeignAggregate(ty))
        }
        mir::ForeignAggregateExpression::Call {
            function: callee,
            args,
            ..
        } => validate_call(
            program,
            function,
            *callee,
            args,
            mir::Type::ForeignAggregate(ty),
        ),
        mir::ForeignAggregateExpression::NullableLocalAssumeNonNull { local, .. } => {
            if local_in(function, *local)?.ty != mir::Type::NullableForeignAggregate(ty) {
                return Err(malformed_mir(
                    "a narrowed foreign aggregate copy names a local without its `?Name` value",
                ));
            }
            Ok(())
        }
    }
}

fn validate_call(
    program: &mir::Program,
    function: &mir::Function,
    callee: mir::FunctionId,
    args: &[mir::Rvalue],
    result: mir::Type,
) -> Result<(), BackendError> {
    let callee = function_in(program, callee)?;
    if callee.return_type != mir::ReturnType::Value(result) {
        return Err(malformed_mir(
            "a foreign aggregate call targets a function with another return type",
        ));
    }
    validate_call_args(program, function, callee, args)
}

/// A `?Name` value: `null`, a present aggregate value, or a copy of a
/// local, property, static property or call result of the nullable type.
pub(super) fn validate_nullable_expression(
    program: &mir::Program,
    function: &mir::Function,
    expression: &mir::NullableForeignAggregateExpression,
) -> Result<(), BackendError> {
    let ty = expression.ty();
    definition_in(program, ty)?;
    let nullable = mir::Type::NullableForeignAggregate(ty);
    match expression {
        mir::NullableForeignAggregateExpression::Null(_) => Ok(()),
        mir::NullableForeignAggregateExpression::Present(value) => {
            validate_expression(program, function, value)
        }
        mir::NullableForeignAggregateExpression::Local { local, .. } => {
            if local_in(function, *local)?.ty != nullable {
                return Err(malformed_mir(
                    "a `?Name` copy names a local of another type",
                ));
            }
            Ok(())
        }
        mir::NullableForeignAggregateExpression::Property {
            object, property, ..
        } => validate_property_operand(program, function, *object, *property, nullable),
        mir::NullableForeignAggregateExpression::Static { id, .. } => {
            validate_static_operand(program, *id, nullable)
        }
        mir::NullableForeignAggregateExpression::Call {
            function: callee,
            args,
            ..
        } => validate_call(program, function, *callee, args, nullable),
    }
}

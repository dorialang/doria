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

pub(super) fn validate_field_access(
    program: &mir::Program,
    function: &mir::Function,
    aggregate: mir::LocalId,
    offset: u64,
    access: &mir::ForeignFieldAccess,
) -> Result<(), BackendError> {
    let mir::Type::ForeignAggregate(aggregate_type) = local_in(function, aggregate)?.ty else {
        return Err(malformed_mir(
            "a foreign field access names a local without a foreign aggregate",
        ));
    };
    match access {
        mir::ForeignFieldAccess::Read { target } => {
            let target = local_in(function, *target)?;
            validate_field(program, aggregate_type, offset, target.ty)
        }
        mir::ForeignFieldAccess::Write { value } => {
            if !local_in(function, aggregate)?.writable {
                return Err(malformed_mir(
                    "a foreign field write names a readonly local",
                ));
            }
            validate_field(program, aggregate_type, offset, value.ty())?;
            validate_rvalue(program, function, value)
        }
    }
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
                validate_field(program, ty, field.offset, field.value.ty())?;
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
        mir::ForeignAggregateExpression::Call {
            function: callee,
            args,
            ..
        } => {
            let callee = function_in(program, *callee)?;
            if callee.return_type != mir::ReturnType::Value(mir::Type::ForeignAggregate(ty)) {
                return Err(malformed_mir(
                    "a foreign aggregate call targets a function with another return type",
                ));
            }
            validate_call_args(program, function, callee, args)
        }
    }
}

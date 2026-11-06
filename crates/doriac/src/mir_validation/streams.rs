//! Canonical callback contracts are checked before any dispatch consumer uses them.

use super::*;
use crate::compiler_known_stream::{self, StreamOperation};

pub(super) fn validate_transfer_observation(
    program: &mir::Program,
    function: &mir::Function,
    group: mir::LocalId,
    wait: mir::LocalId,
    outcome: mir::PayloadEnumType,
) -> Result<(), BackendError> {
    let types = compiler_known_stream::TransferObservationTypes::for_result(program, outcome)
        .ok_or_else(|| malformed_mir("completion observation has no typed result schema"))?;
    for (key, reading) in types.keys.into_iter().zip([true, false]) {
        if !matches!(
            (key, reading),
            (
                crate::inline_value::InlineValueType::ReadTransferKey(_),
                true
            ) | (
                crate::inline_value::InlineValueType::WriteTransferKey(_),
                false
            )
        ) {
            return Err(malformed_mir(
                "completion observation has the wrong directional keys",
            ));
        }
        validate_transfer_group_key(program, function, group, key)?;
    }
    if !local_in(function, group)?.writable {
        return Err(malformed_mir(
            "completion observation requires writable group access",
        ));
    }
    let mir::Type::PayloadEnum(wait) = local_in(function, wait)?.ty else {
        return Err(malformed_mir(
            "completion observation requires its exact Wait",
        ));
    };
    use mir::{ScalarType, Type};
    let cases = [
        (
            outcome,
            compiler_known_stream::TRANSFER_WAIT_RESULT,
            vec![
                ("Completed", vec![Type::PayloadEnum(types.completion)]),
                ("TimedOut", vec![]),
                ("Exhausted", vec![]),
            ],
        ),
        (
            types.completion,
            compiler_known_stream::TRANSFER_COMPLETION,
            vec![
                (
                    "Read",
                    vec![
                        Type::Scalar(ScalarType::Inline(types.keys[0])),
                        Type::NullablePayloadEnum(types.outcomes[0]),
                    ],
                ),
                (
                    "Write",
                    vec![
                        Type::Scalar(ScalarType::Inline(types.keys[1])),
                        Type::NullablePayloadEnum(types.outcomes[1]),
                    ],
                ),
            ],
        ),
        (
            wait,
            "Doria\\Std\\Io\\Wait",
            vec![
                ("Immediate", vec![]),
                (
                    "For",
                    vec![Type::Scalar(ScalarType::Inline(
                        crate::inline_value::InlineValueType::Duration,
                    ))],
                ),
                (
                    "Until",
                    vec![Type::Scalar(ScalarType::Inline(
                        crate::inline_value::InlineValueType::Deadline,
                    ))],
                ),
                ("Indefinite", vec![]),
            ],
        ),
    ];
    for (ty, name, cases) in cases {
        let definition = enum_in(program, ty.id)?;
        if definition.name != name
            || definition.source_span.source != compiler_known_stream::SOURCE_ID
            || definition.payload_type() != Some(ty)
            || !ty.capabilities.trivial_copy
            || ty.capabilities.needs_drop
            || definition.cases.len() != cases.len()
            || definition.cases.iter().zip(cases).enumerate().any(
                |(index, (case, (name, fields)))| {
                    case.name != name
                        || case.tag != index as u32
                        || case.payload.len() != fields.len()
                        || case
                            .payload
                            .iter()
                            .zip(fields)
                            .any(|(field, ty)| field.ty != ty)
                },
            )
        {
            return Err(malformed_mir(
                "completion observation requires canonical Copy enums",
            ));
        }
    }
    for (outcome, reading) in types.outcomes.into_iter().zip([true, false]) {
        validate_stream_outcome(program, outcome, reading)?;
    }
    Ok(())
}

pub(super) fn validate_transfer_group_begin(
    program: &mir::Program,
    function: &mir::Function,
    class: crate::class_layout::ClassId,
    set: mir::LocalId,
    plan: mir::LocalId,
    controls: [mir::LocalId; 2],
) -> Result<(), BackendError> {
    use crate::inline_value::InlineValueType;
    let deadline = local_in(function, controls[0])?.ty;
    let cancellation = local_in(function, controls[1])?.ty;
    if deadline != mir::Type::NullableScalar(mir::ScalarType::Inline(InlineValueType::Deadline))
        || !matches!(
            cancellation,
            mir::Type::NullableScalar(mir::ScalarType::Inline(InlineValueType::CancellationToken(
                _
            )))
        )
    {
        return Err(malformed_mir(
            "group controls require nullable Deadline and CancellationToken",
        ));
    }
    validate_type(program, deadline)?;
    validate_type(program, cancellation)?;
    canonical_class(
        program,
        mir::Type::Class(class),
        compiler_known_stream::TRANSFER_GROUP,
    )?;
    let set = local_in(function, set)?;
    canonical_class(program, set.ty, compiler_known_stream::READINESS_SET)?;
    let plan = local_in(function, plan)?;
    canonical_class(program, plan.ty, compiler_known_stream::TRANSFER_PLAN)?;
    if class_in(program, class)?.runtime_payload != Some(mir::RuntimeClassPayload::TransferGroup)
        || !set.writable
        || !plan.owned
    {
        return Err(malformed_mir(
            "group setup requires its exact group, writable set and owned plan",
        ));
    }
    for (ty, payload) in [
        (set.ty, mir::RuntimeClassPayload::ReadinessSet),
        (plan.ty, mir::RuntimeClassPayload::TransferPlan),
    ] {
        let mir::Type::Class(class) = ty else {
            unreachable!()
        };
        if class_in(program, class)?.runtime_payload != Some(payload) {
            return Err(malformed_mir(
                "group setup requires compiler-owned source payloads",
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_transfer_plan_key(
    program: &mir::Program,
    function: &mir::Function,
    plan: mir::LocalId,
    position: mir::LocalId,
    key: crate::inline_value::InlineValueType,
) -> Result<(), BackendError> {
    let ty = local_in(function, plan)?.ty;
    canonical_class(program, ty, compiler_known_stream::TRANSFER_PLAN)?;
    let mir::Type::Class(class) = ty else {
        unreachable!()
    };
    if class_in(program, class)?.runtime_payload != Some(mir::RuntimeClassPayload::TransferPlan)
        || local_in(function, position)?.ty
            != mir::Type::Scalar(mir::ScalarType::Integer(crate::numeric::IntegerType::Int64))
    {
        return Err(malformed_mir(
            "transfer key lookup requires its exact plan and int position",
        ));
    }
    validate_directional_key(program, key)
}

pub(super) fn validate_transfer_group_key(
    program: &mir::Program,
    function: &mir::Function,
    group: mir::LocalId,
    key: crate::inline_value::InlineValueType,
) -> Result<(), BackendError> {
    let ty = local_in(function, group)?.ty;
    canonical_class(program, ty, compiler_known_stream::TRANSFER_GROUP)?;
    let mir::Type::Class(class) = ty else {
        unreachable!()
    };
    if class_in(program, class)?.runtime_payload != Some(mir::RuntimeClassPayload::TransferGroup) {
        return Err(malformed_mir(
            "observed key requires its exact compiler-owned group",
        ));
    }
    validate_directional_key(program, key)
}

pub(super) fn validate_transfer_group_read_region(
    program: &mir::Program,
    function: &mir::Function,
    group: mir::LocalId,
    key: mir::LocalId,
) -> Result<(), BackendError> {
    let mir::Type::Scalar(mir::ScalarType::Inline(
        key @ crate::inline_value::InlineValueType::ReadTransferKey(_),
    )) = local_in(function, key)?.ty
    else {
        return Err(malformed_mir(
            "completed read access requires its exact read key",
        ));
    };
    validate_transfer_group_key(program, function, group, key)
}

pub(super) fn validate_transfer_resubmission(
    program: &mir::Program,
    function: &mir::Function,
    group: mir::LocalId,
    key: mir::LocalId,
    reading: bool,
    range: [mir::LocalId; 2],
) -> Result<(), BackendError> {
    let mir::Type::Scalar(mir::ScalarType::Inline(key)) = local_in(function, key)?.ty else {
        return Err(malformed_mir("resubmission requires its directional key"));
    };
    if !matches!(
        (key, reading),
        (
            crate::inline_value::InlineValueType::ReadTransferKey(_),
            true
        ) | (
            crate::inline_value::InlineValueType::WriteTransferKey(_),
            false
        )
    ) || !local_in(function, group)?.writable
        || range.into_iter().any(|local| {
            function.locals.get(local.0).map(|local| local.ty)
                != Some(mir::Type::Scalar(mir::ScalarType::Integer(
                    crate::numeric::IntegerType::Int64,
                )))
        })
    {
        return Err(malformed_mir(
            "resubmission requires writable group access, its exact directional key and int range",
        ));
    }
    validate_transfer_group_key(program, function, group, key)
}

fn validate_directional_key(
    program: &mir::Program,
    key: crate::inline_value::InlineValueType,
) -> Result<(), BackendError> {
    let name = match key {
        crate::inline_value::InlineValueType::ReadTransferKey(_) => {
            compiler_known_stream::READ_TRANSFER_KEY
        }
        crate::inline_value::InlineValueType::WriteTransferKey(_) => {
            compiler_known_stream::WRITE_TRANSFER_KEY
        }
        _ => {
            return Err(malformed_mir(
                "transfer key lookup requires its exact directional key",
            ))
        }
    };
    validate_type(program, mir::Type::Scalar(mir::ScalarType::Inline(key)))?;
    canonical_class(
        program,
        mir::Type::Class(key.nominal_class().expect("transfer key is nominal")),
        name,
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn validate_transfer_plan(
    program: &mir::Program,
    function: &mir::Function,
    class: crate::class_layout::ClassId,
    previous: Option<mir::LocalId>,
    key: mir::LocalId,
    region: mir::LocalId,
    reading: bool,
) -> Result<(), BackendError> {
    canonical_class(
        program,
        mir::Type::Class(class),
        compiler_known_stream::TRANSFER_PLAN,
    )?;
    if class_in(program, class)?.runtime_payload != Some(mir::RuntimeClassPayload::TransferPlan) {
        return Err(malformed_mir(
            "transfer configuration has no compiler-owned plan payload",
        ));
    }
    if let Some(previous) = previous {
        let previous = local_in(function, previous)?;
        if previous.ty != mir::Type::Class(class) || !previous.owned {
            return Err(malformed_mir(
                "transfer append must consume its original owned plan",
            ));
        }
    }
    let mir::Type::Scalar(mir::ScalarType::Inline(
        crate::inline_value::InlineValueType::RegistrationKey(key),
    )) = local_in(function, key)?.ty
    else {
        return Err(malformed_mir(
            "transfer configuration requires an exact registration key",
        ));
    };
    let arguments = &class_in(program, key)?.arguments;
    let [stream] = arguments.as_slice() else {
        return Err(malformed_mir(
            "transfer configuration key has no exact stream type",
        ));
    };
    let operations = if reading {
        [StreamOperation::PrepareRead, StreamOperation::CompleteRead]
    } else {
        [
            StreamOperation::PrepareWrite,
            StreamOperation::CompleteWrite,
        ]
    };
    if operations
        .iter()
        .any(|operation| program.stream_callback(*stream, *operation).is_none())
    {
        return Err(malformed_mir(
            "transfer configuration requires its nominal pending capability",
        ));
    }
    let region = local_in(function, region)?;
    let kind = if reading {
        mir::ByteRegionKind::Writable
    } else {
        mir::ByteRegionKind::Readonly
    };
    if region.ty != mir::Type::ByteRegion(kind) || (reading && !region.writable) {
        return Err(malformed_mir(
            "transfer configuration has an incompatible byte-region loan",
        ));
    }
    Ok(())
}

pub(super) fn validate_membership(
    program: &mir::Program,
    function: &mir::Function,
    operation: &mir::CheckedIoOperation,
) -> Result<(), BackendError> {
    use crate::inline_value::InlineValueType;
    let (set, key, stream_type) = match *operation {
        mir::CheckedIoOperation::RegisterReadinessStream { set, stream, key } => {
            let stream = local_in(function, stream)?;
            if (stream.ty.requires_owned_cleanup() && !stream.owned)
                || program
                    .stream_callback(stream.ty, StreamOperation::ReadinessSource)
                    .is_none()
            {
                return Err(malformed_mir(
                    "registration requires an independently owned nominal readiness value",
                ));
            }
            (set, key, stream.ty)
        }
        mir::CheckedIoOperation::UnregisterReadinessStream {
            set,
            key,
            stream_type,
        }
        | mir::CheckedIoOperation::BorrowReadinessStream {
            set,
            key,
            stream_type,
            ..
        } => {
            let mir::Type::Scalar(mir::ScalarType::Inline(key)) = local_in(function, key)?.ty
            else {
                return Err(malformed_mir(
                    "removal requires an exact inline registration key",
                ));
            };
            validate_type(program, stream_type)?;
            (set, key, stream_type)
        }
        _ => unreachable!("membership operation"),
    };
    let set = local_in(function, set)?;
    let writable = !matches!(
        operation,
        mir::CheckedIoOperation::BorrowReadinessStream {
            writable: false,
            ..
        }
    );
    if writable && !set.writable {
        return Err(malformed_mir(
            "membership requires writable readiness-set access",
        ));
    }
    canonical_class(program, set.ty, compiler_known_stream::READINESS_SET)?;
    let InlineValueType::RegistrationKey(class) = key else {
        return Err(malformed_mir(
            "membership requires its canonical registration key",
        ));
    };
    validate_type(program, mir::Type::Scalar(mir::ScalarType::Inline(key)))?;
    if class_in(program, class)?.arguments.as_slice() != [stream_type] {
        return Err(malformed_mir(
            "membership changes the registered static stream type",
        ));
    }
    Ok(())
}

pub(super) fn validate_requirement(
    program: &mir::Program,
    method: &mir::InterfaceMethod,
    signature: &mir::FunctionType,
) -> Result<(), BackendError> {
    let mut operation = None;
    for origin in &method.requirement_origins {
        if let Some(current) = StreamOperation::from_requirement(*origin) {
            if operation.is_some_and(|previous| previous != current) {
                return Err(malformed_mir(
                    "stream requirement combines distinct nominal operations",
                ));
            }
            operation = Some(current);
        }
    }
    let Some(operation) = operation else {
        return Ok(());
    };
    let getter = operation == StreamOperation::ReadinessSource;
    if !method.arguments.is_empty()
        || method.exact_dynamic_return
        || method.receiver_mode
            != if getter {
                mir::ReceiverMode::Readonly
            } else {
                mir::ReceiverMode::Writable
            }
        || signature.return_borrow
            != getter.then_some(mir::ReturnBorrow {
                source: mir::BorrowSource::Parameter(0),
                writable: false,
                kind: mir::ReturnBorrowKind::Value,
                lifetime: mir::RetainedLifetime::LastUse,
            })
    {
        return Err(malformed_mir(
            "stream requirement changes its canonical receiver or return loan",
        ));
    }
    if getter {
        let mir::ReturnType::Value(ty) = signature.return_type else {
            return Err(malformed_mir("readiness getter has no borrowed source"));
        };
        if signature.parameters.len() != 1 || !signature.checked_effects.is_empty() {
            return Err(malformed_mir(
                "readiness getter changes its canonical signature",
            ));
        }
        return canonical_class(program, ty, compiler_known_stream::READINESS_SOURCE);
    }

    let reading = matches!(
        operation,
        StreamOperation::Read | StreamOperation::PrepareRead | StreamOperation::CompleteRead
    );
    let prepare = matches!(
        operation,
        StreamOperation::PrepareRead | StreamOperation::PrepareWrite
    );
    let complete = matches!(
        operation,
        StreamOperation::CompleteRead | StreamOperation::CompleteWrite
    );
    let expected_parameters = if prepare || complete { 3 } else { 2 };
    if signature.parameters.len() != expected_parameters {
        return Err(malformed_mir(
            "stream callback has incompatible canonical inputs",
        ));
    }
    if prepare || complete {
        let request = &signature.parameters[1];
        if request.mode
            != if prepare {
                mir::FunctionParameterMode::Writable
            } else {
                mir::FunctionParameterMode::Readonly
            }
        {
            return Err(malformed_mir("pending callback changes its request loan"));
        }
        canonical_class(
            program,
            request.ty,
            if reading {
                compiler_known_stream::READ_REQUEST
            } else {
                compiler_known_stream::WRITE_REQUEST
            },
        )?;
    }
    let region = &signature.parameters[expected_parameters - 1];
    if region.ty
        != mir::Type::ByteRegion(if reading {
            mir::ByteRegionKind::Writable
        } else {
            mir::ByteRegionKind::Readonly
        })
        || region.mode
            != if reading {
                mir::FunctionParameterMode::Writable
            } else {
                mir::FunctionParameterMode::Readonly
            }
    {
        return Err(malformed_mir(
            "stream callback changes its canonical byte-region loan",
        ));
    }
    let io_error = program
        .error_descriptors
        .iter()
        .find(|descriptor| {
            program.classes[descriptor.class.0].declaration_name
                == crate::compiler_known_io::IO_ERROR
        })
        .ok_or_else(|| malformed_mir("stream callback has no canonical IoError"))?;
    if !signature.checked_effects.is_empty()
        || !signature
            .ambient_checked_effects
            .contains(&mir::CheckedEffect::Concrete(io_error.id))
    {
        return Err(malformed_mir(
            "stream callback changes its canonical checked effect",
        ));
    }
    if prepare {
        if signature.return_type != mir::ReturnType::Void {
            return Err(malformed_mir("pending preparation must return void"));
        }
    } else {
        let outcome = match signature.return_type {
            mir::ReturnType::Value(mir::Type::NullablePayloadEnum(outcome)) if complete => outcome,
            mir::ReturnType::Value(mir::Type::PayloadEnum(outcome)) if !complete => outcome,
            _ => {
                return Err(malformed_mir(
                    "stream callback changes its canonical outcome",
                ));
            }
        };
        validate_stream_outcome(program, outcome, reading)?;
    }
    Ok(())
}

fn canonical_class(program: &mir::Program, ty: mir::Type, name: &str) -> Result<(), BackendError> {
    let mir::Type::Class(class) = ty else {
        return Err(malformed_mir(
            "stream callback changes its canonical class input or result",
        ));
    };
    let definition = class_in(program, class)?;
    if definition.source_span.source != compiler_known_stream::SOURCE_ID
        || definition.declaration_name != name
        || !definition.arguments.is_empty()
    {
        return Err(malformed_mir(
            "stream callback changes its canonical class identity",
        ));
    }
    Ok(())
}

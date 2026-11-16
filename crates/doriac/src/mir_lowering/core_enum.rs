//! Compiler-known core value operations on Copy enums (Decision 0096).
//!
//! A Copy enum, unit or with Copy payloads, is Equatable, Hashable and
//! Comparable when its payloads are, with no `implements` clause and no
//! boxing. Andrew ruled the order on 2026-11-16 ("Declaration order, then
//! payloads (Recommended)"): cases compare in declaration order, and two values
//! of the same case compare their payloads left to right. Each operation is
//! lowered here as ordinary MIR control flow over case tests and payload
//! bindings, so every backend runs it without an enum-specific runtime call.

use super::*;
use crate::compiler_known_contracts::CoreValueOperation;

pub(super) const fn is_enum(ty: mir::Type) -> bool {
    matches!(
        ty,
        mir::Type::Scalar(mir::ScalarType::Enum(_)) | mir::Type::PayloadEnum(_)
    )
}

struct Case {
    id: crate::enums::EnumCaseId,
    payload: Vec<mir::Type>,
}

fn cases(
    ty: mir::Type,
    span: Span,
    context: &LoweringContext<'_>,
) -> DiagnosticResult<(crate::enums::EnumId, Vec<Case>)> {
    let enum_id = match ty {
        mir::Type::Scalar(mir::ScalarType::Enum(id)) => id,
        mir::Type::PayloadEnum(payload) => payload.id,
        _ => return Err(vec![unsupported(span, "core enum operation on a non-enum")]),
    };
    let definition = context
        .semantic_info
        .enums
        .iter()
        .find(|definition| definition.id == enum_id)
        .ok_or_else(|| {
            vec![unsupported(
                span,
                "core enum operation has no enum definition",
            )]
        })?;
    let cases = definition
        .cases
        .iter()
        .map(|case| {
            let payload = case
                .payload
                .iter()
                .map(|field| {
                    context.mir_resolved_type(&field.ty).ok_or_else(|| {
                        vec![unsupported(
                            span,
                            "enum payload has no native representation",
                        )]
                    })
                })
                .collect::<DiagnosticResult<Vec<_>>>()?;
            Ok(Case {
                id: case.id,
                payload,
            })
        })
        .collect::<DiagnosticResult<Vec<_>>>()?;
    Ok((enum_id, cases))
}

fn is_case(
    local: mir::LocalId,
    ty: mir::Type,
    enum_id: crate::enums::EnumId,
    case: crate::enums::EnumCaseId,
) -> mir::BoolExpression {
    match ty {
        mir::Type::PayloadEnum(payload) => mir::BoolExpression::PayloadEnumIsCase {
            local,
            ty: payload,
            case,
            nullable: false,
        },
        _ => mir::BoolExpression::Compare {
            op: mir::CompareOp::Equal,
            left: Box::new(mir::ValueExpression::Enum(mir::EnumExpression::Use {
                enum_id,
                operand: mir::Operand::Local(local),
            })),
            right: Box::new(mir::ValueExpression::Enum(mir::EnumExpression::Case(
                crate::enums::EnumValue {
                    enum_id,
                    case_id: case,
                },
            ))),
        },
    }
}

fn int64(value: i128) -> mir::Rvalue {
    mir::Rvalue::Value(mir::ValueExpression::Integer(
        mir::IntegerExpression::constant(
            IntegerValue::from_i128(IntegerType::Int64, value).unwrap(),
        ),
    ))
}

/// Runs `body` in a block for the case `local` holds, one block per case in
/// declaration order, each entered through a test of that case (a payload
/// binding needs that proof), and continues after all of them.
fn for_each_case(
    local: mir::LocalId,
    ty: mir::Type,
    enum_id: crate::enums::EnumId,
    cases: &[Case],
    context: &mut LoweringContext<'_>,
    mut body: impl FnMut(usize, &Case, &mut LoweringContext<'_>) -> DiagnosticResult<()>,
) -> DiagnosticResult<()> {
    let exit = context.create_block();
    for (index, case) in cases.iter().enumerate() {
        let arm = context.create_block();
        let next = context.create_block();
        context.terminate_condition(is_case(local, ty, enum_id, case.id), arm, next);
        context.current_block = Some(arm);
        body(index, case, context)?;
        context.terminate_current(mir::Terminator::Jump(exit));
        context.current_block = Some(next);
    }
    // A value is always one of its cases; this edge is never taken.
    context.terminate_current(mir::Terminator::Jump(exit));
    context.current_block = Some(exit);
    Ok(())
}

/// The declaration index of the case `local` holds, as an `int` local.
fn case_index(
    local: mir::LocalId,
    ty: mir::Type,
    enum_id: crate::enums::EnumId,
    cases: &[Case],
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<mir::LocalId> {
    let index = context.declare_temp(true, IntegerType::Int64);
    context.push_statement(mir::Statement::AssignLocal {
        target: index,
        value: int64(0),
    });
    for_each_case(
        local,
        ty,
        enum_id,
        cases,
        context,
        |position, _, context| {
            context.push_statement(mir::Statement::AssignLocal {
                target: index,
                value: int64(position as i128),
            });
            Ok(())
        },
    )?;
    Ok(index)
}

/// A compiler-generated borrowed `match` over the payload enum in
/// `scrutinee`: one planned arm per case in `cases`, each binding that case's
/// payload as the `match` statement binds it, then running `body`, whose value
/// the arm assigns to `result` once. A case outside `cases` cannot occur there.
#[allow(clippy::too_many_arguments)]
fn payload_match(
    scrutinee: mir::LocalId,
    ty: mir::Type,
    enum_id: crate::enums::EnumId,
    cases: &[&Case],
    result: mir::LocalId,
    context: &mut LoweringContext<'_>,
    mut body: impl FnMut(
        &Case,
        &[(mir::LocalId, mir::Type)],
        &mut LoweringContext<'_>,
    ) -> DiagnosticResult<mir::Rvalue>,
) -> DiagnosticResult<()> {
    let mir::Type::PayloadEnum(payload) = ty else {
        return Err(vec![unsupported(
            Span::default(),
            "payload match on a unit enum",
        )]);
    };
    let plan_block = context.current_block();
    let merge = context.create_block();
    let fallback = context.create_block();
    let mut arms = Vec::with_capacity(cases.len());
    for (index, case) in cases.iter().enumerate() {
        let binding = context.create_block();
        arms.push(mir::MatchArmPlan {
            guard: None,
            binding,
        });
        let next = if index + 1 == cases.len() {
            fallback
        } else {
            context.create_block()
        };
        context.terminate_condition(is_case(scrutinee, ty, enum_id, case.id), binding, next);
        context.current_block = Some(binding);
        context.push_scope();
        context.begin_statement_temporaries();
        let fields = case
            .payload
            .iter()
            .enumerate()
            .map(|(position, field)| {
                // As a borrowed `match` arm binds: strings and Copy payloads
                // with cleanup are retained for the arm and released after it.
                let cleanup = matches!(field, mir::Type::String | mir::Type::NullableString)
                    || (field.requires_owned_cleanup() && !field.has_move_ownership());
                let local = context.declare_pattern_local(
                    &format!("<core payload {position}>"),
                    *field,
                    false,
                    cleanup,
                );
                (local, *field)
            })
            .collect::<Vec<_>>();
        if !fields.is_empty() {
            context.push_statement(mir::Statement::BindPayloadEnumFields {
                source: scrutinee,
                ty: payload,
                case: case.id,
                nullable: false,
                mode: mir::MatchBindingMode::BorrowedArm,
                targets: fields.iter().map(|(local, _)| *local).collect(),
            });
        }
        let value = body(case, &fields, context)?;
        context.push_statement(mir::Statement::AssignLocal {
            target: result,
            value,
        });
        context.finish_statement_temporaries(true);
        context.pop_scope();
        if context.current_block.is_some() {
            context.terminate_current(mir::Terminator::Jump(merge));
        }
        context.current_block = Some(next);
    }
    context.current_block = Some(fallback);
    context.terminate_current(mir::Terminator::Unreachable);
    let error_exit = context.conditional_error_exit();
    context.blocks[plan_block.0]
        .statements
        .push(mir::Statement::MatchResultPlan {
            scrutinee,
            mode: mir::MatchOwnershipMode::Borrowed,
            result,
            arms,
            merge,
            error_exit,
        });
    context.current_block = Some(merge);
    Ok(())
}

pub(super) fn operation(
    operation: CoreValueOperation,
    receiver: (mir::LocalId, mir::Type),
    other: Option<(mir::LocalId, mir::Type)>,
    span: Span,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<mir::Rvalue> {
    let (enum_id, cases) = cases(receiver.1, span, context)?;
    match operation {
        CoreValueOperation::Equal => {
            let other = other.expect("checked equals argument");
            let equal = match receiver.1 {
                mir::Type::PayloadEnum(payload) => mir::BoolExpression::PayloadEnumCompare {
                    op: mir::CompareOp::Equal,
                    left: Box::new(mir::PayloadEnumExpression::Use {
                        ty: payload,
                        place: mir::PayloadEnumPlace::Local(receiver.0),
                        mode: mir::PayloadEnumUseMode::Borrow,
                    }),
                    right: Box::new(mir::PayloadEnumExpression::Use {
                        ty: payload,
                        place: mir::PayloadEnumPlace::Local(other.0),
                        mode: mir::PayloadEnumUseMode::Borrow,
                    }),
                },
                _ => mir::BoolExpression::Compare {
                    op: mir::CompareOp::Equal,
                    left: Box::new(mir::ValueExpression::Enum(mir::EnumExpression::Use {
                        enum_id,
                        operand: mir::Operand::Local(receiver.0),
                    })),
                    right: Box::new(mir::ValueExpression::Enum(mir::EnumExpression::Use {
                        enum_id,
                        operand: mir::Operand::Local(other.0),
                    })),
                },
            };
            Ok(mir::Rvalue::Value(mir::ValueExpression::Bool(equal)))
        }
        CoreValueOperation::Hash => hash(receiver, enum_id, &cases, span, context),
        CoreValueOperation::Compare => {
            let other = other.expect("checked compare argument");
            compare(receiver, other, enum_id, &cases, span, context)
        }
        CoreValueOperation::Clone => Err(vec![unsupported(
            span,
            "Copy enums do not select an owned class clone",
        )]),
    }
}

/// The case's declaration index, then each payload's hash folded in left to
/// right: `hash = rotate_left(hash, 5) ^ payload_hash`. Decision 0134 fixes
/// only the law (equal values hash equal); the value is not a stable format.
fn hash(
    receiver: (mir::LocalId, mir::Type),
    enum_id: crate::enums::EnumId,
    cases: &[Case],
    span: Span,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<mir::Rvalue> {
    let uint = IntegerType::UInt64;
    let read = |local| mir::IntegerExpression::use_operand(uint, mir::Operand::Local(local));
    let index = case_index(receiver.0, receiver.1, enum_id, cases, context)?;
    let seed = context.declare_temp(false, uint);
    context.push_statement(mir::Statement::AssignLocal {
        target: seed,
        value: mir::Rvalue::Value(mir::ValueExpression::Integer(
            mir::IntegerExpression::Convert {
                ty: uint,
                value: Box::new(mir::IntegerExpression::use_operand(
                    IntegerType::Int64,
                    mir::Operand::Local(index),
                )),
                span,
                value_span: span,
                boundary: None,
            },
        )),
    });
    if !matches!(receiver.1, mir::Type::PayloadEnum(_)) {
        return Ok(mir::Rvalue::Value(mir::ValueExpression::Integer(read(
            seed,
        ))));
    }
    let folded = context.declare_temp(false, uint);
    let all = cases.iter().collect::<Vec<_>>();
    payload_match(
        receiver.0,
        receiver.1,
        enum_id,
        &all,
        folded,
        context,
        |_, fields, context| {
            let running = context.declare_temp(true, uint);
            context.push_statement(mir::Statement::AssignLocal {
                target: running,
                value: mir::Rvalue::Value(mir::ValueExpression::Integer(read(seed))),
            });
            for field in fields {
                let field_hash = super::core_value::operate(
                    CoreValueOperation::Hash,
                    *field,
                    None,
                    span,
                    context,
                )?;
                let (field_hash, _) = super::core_value::materialize_value(field_hash, context);
                let constant = |value| {
                    mir::IntegerExpression::constant(IntegerValue::from_i128(uint, value).unwrap())
                };
                let binary = |op, left, right| mir::IntegerExpression::Binary {
                    ty: uint,
                    op,
                    left: Box::new(left),
                    right: Box::new(right),
                    span,
                    right_span: span,
                };
                let rotated = binary(
                    mir::IntegerBinaryOp::BitwiseOr,
                    binary(mir::IntegerBinaryOp::ShiftLeft, read(running), constant(5)),
                    binary(
                        mir::IntegerBinaryOp::ShiftRight,
                        read(running),
                        constant(59),
                    ),
                );
                context.push_statement(mir::Statement::AssignLocal {
                    target: running,
                    value: mir::Rvalue::Value(mir::ValueExpression::Integer(binary(
                        mir::IntegerBinaryOp::BitwiseXor,
                        rotated,
                        read(field_hash),
                    ))),
                });
            }
            Ok(mir::Rvalue::Value(mir::ValueExpression::Integer(read(
                running,
            ))))
        },
    )?;
    Ok(mir::Rvalue::Value(mir::ValueExpression::Integer(read(
        folded,
    ))))
}

/// Cases in declaration order; two values of the same case by their payloads
/// left to right.
fn compare(
    receiver: (mir::LocalId, mir::Type),
    other: (mir::LocalId, mir::Type),
    enum_id: crate::enums::EnumId,
    cases: &[Case],
    span: Span,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<mir::Rvalue> {
    let ordering = |name, context: &LoweringContext<'_>| {
        context
            .enum_case_value("Ordering", name)
            .ok_or_else(|| vec![unsupported(span, "canonical Ordering case is missing")])
    };
    let less = ordering("Less", context)?;
    let equal = ordering("Equal", context)?;
    let greater = ordering("Greater", context)?;
    let ordering_type = mir::Type::Scalar(mir::ScalarType::Enum(less.enum_id));
    let case_value =
        |value| mir::Rvalue::Value(mir::ValueExpression::Enum(mir::EnumExpression::Case(value)));
    let is_equal = |local| mir::BoolExpression::Compare {
        op: mir::CompareOp::Equal,
        left: Box::new(mir::ValueExpression::Enum(mir::EnumExpression::Use {
            enum_id: equal.enum_id,
            operand: mir::Operand::Local(local),
        })),
        right: Box::new(mir::ValueExpression::Enum(mir::EnumExpression::Case(equal))),
    };
    let index = |local| {
        Box::new(mir::ValueExpression::Integer(
            mir::IntegerExpression::use_operand(IntegerType::Int64, mir::Operand::Local(local)),
        ))
    };
    let left = case_index(receiver.0, receiver.1, enum_id, cases, context)?;
    let right = case_index(other.0, other.1, enum_id, cases, context)?;
    let result = context.declare_checked_call_slot(ordering_type, false);
    let exit = context.create_block();
    let lower = context.create_block();
    let not_lower = context.create_block();
    context.terminate_condition(
        mir::BoolExpression::Compare {
            op: mir::CompareOp::Less,
            left: index(left),
            right: index(right),
        },
        lower,
        not_lower,
    );
    context.current_block = Some(lower);
    context.push_statement(mir::Statement::AssignLocal {
        target: result,
        value: case_value(less),
    });
    context.terminate_current(mir::Terminator::Jump(exit));

    context.current_block = Some(not_lower);
    let higher = context.create_block();
    let same = context.create_block();
    context.terminate_condition(
        mir::BoolExpression::Compare {
            op: mir::CompareOp::Less,
            left: index(right),
            right: index(left),
        },
        higher,
        same,
    );
    context.current_block = Some(higher);
    context.push_statement(mir::Statement::AssignLocal {
        target: result,
        value: case_value(greater),
    });
    context.terminate_current(mir::Terminator::Jump(exit));

    context.current_block = Some(same);
    if matches!(receiver.1, mir::Type::PayloadEnum(_)) {
        // `outer` and `inner` are the plans' results; `running` holds the
        // order the payloads have decided so far.
        let outer = context.declare_checked_call_slot(ordering_type, false);
        let all = cases.iter().collect::<Vec<_>>();
        payload_match(
            receiver.0,
            receiver.1,
            enum_id,
            &all,
            outer,
            context,
            |case, left_fields, context| {
                if left_fields.is_empty() {
                    return Ok(case_value(equal));
                }
                let left_fields = left_fields.to_vec();
                let inner = context.declare_checked_call_slot(ordering_type, false);
                // The two values hold the same case here; a nested plan binds
                // the right-hand payload.
                payload_match(
                    other.0,
                    other.1,
                    enum_id,
                    &[case],
                    inner,
                    context,
                    |_, right_fields, context| {
                        let running = context.declare_checked_call_slot(ordering_type, false);
                        context.push_statement(mir::Statement::AssignLocal {
                            target: running,
                            value: case_value(equal),
                        });
                        // A field decides only while every field before it is equal.
                        for (left_field, right_field) in left_fields.iter().zip(right_fields) {
                            let decide = context.create_block();
                            let next = context.create_block();
                            context.terminate_condition(is_equal(running), decide, next);
                            context.current_block = Some(decide);
                            let field = super::core_value::operate(
                                CoreValueOperation::Compare,
                                *left_field,
                                Some(*right_field),
                                span,
                                context,
                            )?;
                            context.push_statement(mir::Statement::AssignLocal {
                                target: running,
                                value: field,
                            });
                            context.terminate_current(mir::Terminator::Jump(next));
                            context.current_block = Some(next);
                        }
                        Ok(local_rvalue(running, ordering_type, false))
                    },
                )?;
                Ok(local_rvalue(inner, ordering_type, false))
            },
        )?;
        context.push_statement(mir::Statement::AssignLocal {
            target: result,
            value: local_rvalue(outer, ordering_type, false),
        });
    } else {
        context.push_statement(mir::Statement::AssignLocal {
            target: result,
            value: case_value(equal),
        });
    }
    context.terminate_current(mir::Terminator::Jump(exit));
    context.current_block = Some(exit);
    Ok(local_rvalue(result, ordering_type, false))
}

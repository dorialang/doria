//! Nominal inline Copy transport over the existing scalar evaluation stack.
//! Explicit Time operations preserve the exact signed nanosecond bit pattern.

use super::*;
use crate::inline_value::InlineValueType;

pub(super) fn nominal_parts(
    value: &LocalValue,
) -> Result<Option<(crate::class_layout::ClassId, Option<usize>)>, InterpreterError> {
    let (ty, value) = match value {
        LocalValue::Scalar(mir::ScalarValue::Inline(value)) => (value.ty, Some(value)),
        LocalValue::NullableScalar {
            ty: mir::ScalarType::Inline(ty),
            value,
        } => {
            let value = match value {
                Some(mir::ScalarValue::Inline(value)) => Some(value),
                None => None,
                _ => {
                    return Err(InterpreterError::new(
                        "inline nominal storage has another scalar type",
                    ))
                }
            };
            (*ty, value)
        }
        _ => return Ok(None),
    };
    let Some(class) = ty.nominal_class() else {
        return Ok(None);
    };
    if ty.storage_bytes() as usize > std::mem::size_of::<usize>() {
        return Err(InterpreterError::new(
            "inline interface payload exceeds one carrier word",
        ));
    }
    let bits = value
        .map(|value| {
            usize::try_from(value.bits).map_err(|_| {
                InterpreterError::new("inline interface payload exceeds one carrier word")
            })
        })
        .transpose()?;
    Ok(Some((class, bits)))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CancellationState {
    claims: usize,
    requested: bool,
    #[cfg(test)]
    notification_failure: Option<Option<i64>>,
}

impl CancellationState {
    pub(super) const fn new(claims: usize) -> Self {
        Self {
            claims,
            requested: false,
            #[cfg(test)]
            notification_failure: None,
        }
    }

    fn request(&mut self) -> Result<(), Option<i64>> {
        self.requested = true;
        #[cfg(test)]
        if let Some(code) = self.notification_failure {
            return Err(code);
        }
        Ok(())
    }
}

fn retain_claim(
    claims: &mut BTreeMap<u128, CancellationState>,
    value: crate::inline_value::InlineValue,
) -> Result<(), InterpreterError> {
    if !value.ty.requires_owned_cleanup() {
        return Ok(());
    }
    if !retain_source_claim(claims, value.bits)? {
        return Err(InterpreterError::runtime_panic("P1503"));
    }
    Ok(())
}

pub(super) fn retain_source_claim(
    claims: &mut BTreeMap<u128, CancellationState>,
    identity: u128,
) -> Result<bool, InterpreterError> {
    let state = claims
        .get_mut(&identity)
        .ok_or_else(|| InterpreterError::new("inline Copy state has no live owner"))?;
    let Some(count) = state
        .claims
        .checked_add(1)
        .filter(|count| *count <= isize::MAX as usize)
    else {
        return Ok(false);
    };
    state.claims = count;
    Ok(true)
}

fn release_claim(
    claims: &mut BTreeMap<u128, CancellationState>,
    value: crate::inline_value::InlineValue,
) -> Result<(), InterpreterError> {
    if !value.ty.requires_owned_cleanup() {
        return Ok(());
    }
    release_source_claim(claims, value.bits)
}

pub(super) fn release_source_claim(
    claims: &mut BTreeMap<u128, CancellationState>,
    identity: u128,
) -> Result<(), InterpreterError> {
    let state = claims
        .get_mut(&identity)
        .ok_or_else(|| InterpreterError::new("inline Copy state was released twice"))?;
    state.claims = state
        .claims
        .checked_sub(1)
        .ok_or_else(|| InterpreterError::new("inline Copy state has no ownership claim"))?;
    if state.claims == 0 {
        claims.remove(&identity);
    }
    Ok(())
}

pub(super) fn retain_owned_claims(
    claims: &mut BTreeMap<u128, CancellationState>,
    value: &LocalValue,
) -> Result<(), InterpreterError> {
    match value {
        LocalValue::Scalar(mir::ScalarValue::Inline(value))
        | LocalValue::NullableScalar {
            value: Some(mir::ScalarValue::Inline(value)),
            ..
        } => retain_claim(claims, *value),
        LocalValue::PayloadEnum(value)
        | LocalValue::NullablePayloadEnum {
            value: Some(value), ..
        } => {
            for (field, moved) in value.fields.iter().zip(&value.moved_fields) {
                if !moved {
                    retain_owned_claims(claims, field)?;
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

pub(super) fn retain_collection_claims(
    claims: &mut BTreeMap<u128, CancellationState>,
    entries: &CollectionEntries,
) -> Result<(), InterpreterError> {
    for (key, value) in entries {
        if let Some(key) = key {
            retain_owned_claims(claims, key)?;
        }
        retain_owned_claims(claims, value)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim_states<const N: usize>(
        entries: [(u128, usize); N],
    ) -> BTreeMap<u128, CancellationState> {
        entries
            .into_iter()
            .map(|(id, claims)| (id, CancellationState::new(claims)))
            .collect()
    }

    fn token(bits: u128) -> crate::inline_value::InlineValue {
        crate::inline_value::InlineValue {
            ty: InlineValueType::CancellationToken(crate::class_layout::ClassId(0)),
            bits,
        }
    }

    #[test]
    fn cancellation_notification_failure_preserves_request_code_and_checked_cleanup() {
        let mut program = crate::lower_source_to_mir(
            "notification-failure.doria",
            r#"
use Doria\Std\Concurrency\CancellationError;
use Doria\Std\Concurrency\CancellationSource as Source;
function request(writable Source $source): void throws CancellationError {}
function status(Source $source): bool { return false; }
function main(): void {
    let writable $source = new Source();
    try { request($source); }
    catch (CancellationError $error) {
        echo $error->message . "\n";
        if ($error->systemCode == null) { echo "absent\n"; }
        else { echo "code: {$error->systemCode ?? -1}\n"; }
    }
    echo "requested: {status($source)}\n";
}
"#,
        )
        .unwrap();
        let source = program
            .classes
            .iter_mut()
            .find(|class| class.name == crate::compiler_known_concurrency::CANCELLATION_SOURCE)
            .unwrap();
        source.name = crate::compiler_known_concurrency::CANCELLATION_SOURCE.into();
        source.declaration_name = source.name.clone();
        source.source_span = Span::in_source(crate::compiler_known_concurrency::SOURCE_ID, 0, 0);
        source.runtime_payload = Some(mir::RuntimeClassPayload::CancellationSource);
        let request = program
            .functions
            .iter_mut()
            .find(|function| function.name == "request")
            .unwrap();
        let error = mir::LocalId(request.locals.len());
        request.locals.push(mir::Local {
            id: error,
            name: "request-error".into(),
            ty: mir::Type::ERROR,
            writable: false,
            owned: true,
            synthetic: true,
        });
        request.entry_block = mir::BlockId(0);
        request.blocks = vec![
            mir::BasicBlock {
                id: mir::BlockId(0),
                statements: vec![],
                terminator: mir::Terminator::CheckedCancellationRequest {
                    source: request.params[0],
                    result: None,
                    error,
                    success: mir::BlockId(1),
                    failure: mir::BlockId(2),
                    span: request.source_span,
                },
            },
            mir::BasicBlock {
                id: mir::BlockId(1),
                statements: vec![],
                terminator: mir::Terminator::ReturnVoid,
            },
            mir::BasicBlock {
                id: mir::BlockId(2),
                statements: vec![],
                terminator: mir::Terminator::PropagateError { error },
            },
        ];
        let status = program
            .functions
            .iter_mut()
            .find(|function| function.name == "status")
            .unwrap();
        status.entry_block = mir::BlockId(0);
        status.blocks = vec![mir::BasicBlock {
            id: mir::BlockId(0),
            statements: vec![],
            terminator: mir::Terminator::Return(mir::Rvalue::Value(mir::ValueExpression::Bool(
                mir::BoolExpression::CancellationRequested {
                    source: status.params[0],
                },
            ))),
        }];
        crate::mir_validation::validate_program(&program).unwrap();
        for code in [None, Some(0), Some(-17)] {
            let mut interpreter = Interpreter::new(
                &program,
                InterpreterLimits::default(),
                MirIo::default(),
                doria_time::monotonic_nanoseconds,
            )
            .unwrap();
            interpreter
                .push_frame(program.entry, &[], &[], None, None)
                .unwrap();
            while interpreter.cancellation_states.is_empty() {
                assert!(matches!(interpreter.step().unwrap(), StepOutcome::Continue));
            }
            let state = interpreter.cancellation_states.values_mut().next().unwrap();
            assert!(!state.requested);
            state.notification_failure = Some(code);
            loop {
                match interpreter.step().unwrap() {
                    StepOutcome::Continue => {}
                    StepOutcome::EntryReturned(FunctionOutcome::Void) => break,
                    _ => panic!("the checked notification error must be caught"),
                }
            }
            let code_text =
                code.map_or_else(|| "absent\n".into(), |code| format!("code: {code}\n"));
            assert_eq!(
                interpreter.stdout,
                format!(
                    "{}\n{code_text}requested: true\n",
                    crate::compiler_known_concurrency::CANCELLATION_NOTIFICATION_FAILURE_MESSAGE
                )
                .as_bytes()
            );
            assert!(interpreter.stderr.is_empty());
            assert!(interpreter.heap.is_empty());
            assert!(interpreter.cancellation_states.is_empty());
        }
    }

    #[test]
    fn retrying_notification_does_not_reset_the_shared_request_or_claims() {
        let mut state = CancellationState::new(3);
        state.notification_failure = Some(Some(0));
        assert_eq!(state.request(), Err(Some(0)));
        assert!(state.requested);
        assert_eq!(state.claims, 3);
        state.notification_failure = None;
        assert_eq!(state.request(), Ok(()));
        assert_eq!(state.request(), Ok(()));
        assert!(state.requested);
        assert_eq!(state.claims, 3);
    }

    fn source_lifetime_fixture() -> (mir::Program, mir::LocalId) {
        let mut program = crate::lower_source_to_mir(
            "source-lifetime.doria",
            r#"
use Doria\Std\Concurrency\CancellationError;
use Doria\Std\Concurrency\CancellationSource as Source;
function describe(CancellationError $error): string { return $error->message; }
function main(): void { let $source = new Source(); }
"#,
        )
        .unwrap();
        let source = program
            .classes
            .iter_mut()
            .find(|class| class.name == crate::compiler_known_concurrency::CANCELLATION_SOURCE)
            .unwrap();
        source.name = crate::compiler_known_concurrency::CANCELLATION_SOURCE.to_owned();
        source.declaration_name = source.name.clone();
        source.source_span = Span::in_source(crate::compiler_known_concurrency::SOURCE_ID, 0, 0);
        source.runtime_payload = Some(mir::RuntimeClassPayload::CancellationSource);
        crate::mir_validation::validate_program(&program).unwrap();
        let local = program.functions[program.entry.0]
            .locals
            .iter()
            .find(|local| local.name == "source")
            .unwrap()
            .id;
        (program, local)
    }

    fn interpreter_with_live_source(
        program: &mir::Program,
        source: mir::LocalId,
    ) -> Interpreter<'_> {
        let mut interpreter = Interpreter::new(
            program,
            InterpreterLimits::default(),
            MirIo::default(),
            doria_time::monotonic_nanoseconds,
        )
        .unwrap();
        interpreter
            .push_frame(program.entry, &[], &[], None, None)
            .unwrap();
        while interpreter.current_frame().unwrap().function != program.entry
            || interpreter.read_local_value(source).is_err()
        {
            assert!(matches!(interpreter.step().unwrap(), StepOutcome::Continue));
        }
        interpreter
    }

    #[test]
    fn source_and_token_claims_retire_independently_through_interpreter_cleanup() {
        let (program, source) = source_lifetime_fixture();
        for tokens_first in [false, true] {
            let mut interpreter = interpreter_with_live_source(&program, source);
            let object = *interpreter.heap.keys().next().unwrap();
            let value = token(object as u128);
            assert_eq!(interpreter.cancellation_states[&value.bits].claims, 1);
            for _ in 0..2 {
                interpreter
                    .execute_inline_task(Task::Value(
                        mir::InlineValueExpression::CancellationToken {
                            ty: value.ty,
                            source,
                            span: program.functions[program.entry.0].source_span,
                        },
                    ))
                    .unwrap();
                assert_eq!(
                    interpreter.pop_scalar().unwrap(),
                    mir::ScalarValue::Inline(value)
                );
            }
            assert_eq!(interpreter.cancellation_states.len(), 1);
            assert_eq!(interpreter.cancellation_states[&value.bits].claims, 3);
            if tokens_first {
                release_claim(&mut interpreter.cancellation_states, value).unwrap();
                release_claim(&mut interpreter.cancellation_states, value).unwrap();
                assert_eq!(interpreter.cancellation_states[&value.bits].claims, 1);
            }
            loop {
                match interpreter.step().unwrap() {
                    StepOutcome::Continue => {}
                    StepOutcome::EntryReturned(FunctionOutcome::Void) => break,
                    _ => panic!("source cleanup must return normally"),
                }
            }
            assert!(interpreter.heap.is_empty());
            if !tokens_first {
                assert_eq!(interpreter.cancellation_states[&value.bits].claims, 2);
                release_claim(&mut interpreter.cancellation_states, value).unwrap();
                assert_eq!(interpreter.cancellation_states[&value.bits].claims, 1);
                release_claim(&mut interpreter.cancellation_states, value).unwrap();
            }
            assert!(interpreter.cancellation_states.is_empty());
        }
    }

    #[test]
    fn token_production_count_exhaustion_preserves_source_and_reports_the_authored_site() {
        let (program, source) = source_lifetime_fixture();
        let mut interpreter = interpreter_with_live_source(&program, source);
        let object = *interpreter.heap.keys().next().unwrap();
        let value = token(object as u128);
        let state = interpreter
            .cancellation_states
            .get_mut(&value.bits)
            .unwrap();
        state.claims = isize::MAX as usize;
        state.requested = true;
        let span = program.functions[program.entry.0].source_span;
        interpreter
            .current_frame_mut()
            .unwrap()
            .tasks
            .push(EvaluationTask::Inline(Task::Value(
                mir::InlineValueExpression::CancellationToken {
                    ty: value.ty,
                    source,
                    span,
                },
            )));
        let StepOutcome::RuntimePanic(event) = interpreter.step().unwrap() else {
            panic!("token production exhaustion must abort");
        };
        assert_eq!(interpreter.cancellation_states.len(), 1);
        assert_eq!(
            interpreter.cancellation_states[&value.bits].claims,
            isize::MAX as usize
        );
        assert!(interpreter.cancellation_states[&value.bits].requested);
        assert_eq!(interpreter.heap.len(), 1);
        assert_eq!(interpreter.current_frame().unwrap().values.len(), 0);
        let output = interpreter.runtime_panic_output(event);
        assert_eq!(output.exit_status, 101);
        let diagnostic = output.runtime_diagnostic.unwrap();
        assert_eq!(diagnostic.code, "P1503");
        assert_eq!(diagnostic.span, span);
    }

    fn release_value(claims: &mut BTreeMap<u128, CancellationState>, value: LocalValue) {
        let mut drops = Vec::new();
        collect_owned_objects_from_value(value, &mut drops);
        for drop in drops {
            match drop {
                OwnedDrop::Inline(value) => release_claim(claims, value).unwrap(),
                // These standalone collections have no registered borrow views.
                OwnedDrop::CollectionView(_) => {}
                _ => panic!("claim fixture produced an unrelated owned drop"),
            }
        }
    }

    #[test]
    fn collection_fill_retains_each_element_and_releases_zero_length_seed() {
        for count in [0, 1, 3, 1000] {
            let seed = LocalValue::Scalar(mir::ScalarValue::Inline(token(17)));
            let mut claims = claim_states([(17, 1)]);
            let entries = repeat_collection_entries(seed.clone(), count).unwrap();
            retain_collection_claims(&mut claims, &entries).unwrap();
            assert_eq!(claims, claim_states([(17, count + 1)]));
            release_value(&mut claims, seed);
            if count == 0 {
                assert!(claims.is_empty());
            } else {
                assert_eq!(claims, claim_states([(17, count)]));
            }
            release_value(
                &mut claims,
                LocalValue::Collection(CollectionValue::new(mir::CollectionTypeId(0), entries)),
            );
            assert!(claims.is_empty());
        }
    }

    #[test]
    fn copied_collection_claims_preserve_source_keys_and_nullable_elements() {
        let source = vec![
            (
                Some(LocalValue::Scalar(mir::ScalarValue::Inline(token(23)))),
                LocalValue::NullableScalar {
                    ty: mir::ScalarType::Inline(token(17).ty),
                    value: Some(mir::ScalarValue::Inline(token(17))),
                },
            ),
            (
                Some(LocalValue::Scalar(mir::ScalarValue::Inline(token(29)))),
                LocalValue::NullableScalar {
                    ty: mir::ScalarType::Inline(token(17).ty),
                    value: None,
                },
            ),
        ];
        let mut claims = claim_states([(17, 1), (23, 1), (29, 1)]);
        let copied = source.clone();
        retain_collection_claims(&mut claims, &copied).unwrap();
        assert_eq!(claims, claim_states([(17, 2), (23, 2), (29, 2)]));
        release_value(
            &mut claims,
            LocalValue::Collection(CollectionValue::new(mir::CollectionTypeId(0), source)),
        );
        assert_eq!(claims, claim_states([(17, 1), (23, 1), (29, 1)]));
        release_value(
            &mut claims,
            LocalValue::Collection(CollectionValue::new(mir::CollectionTypeId(0), copied)),
        );
        assert!(claims.is_empty());
    }

    #[test]
    fn discarded_conversion_duplicates_release_only_their_new_claims() {
        let value = LocalValue::Scalar(mir::ScalarValue::Inline(token(17)));
        let source = vec![(None, value.clone()); 3];
        let mut claims = claim_states([(17, 3)]);
        let mut copied = source.clone();
        retain_collection_claims(&mut claims, &copied).unwrap();
        assert_eq!(claims[&17].claims, 6);
        for (_, duplicate) in copied.drain(1..) {
            release_value(&mut claims, duplicate);
        }
        assert_eq!(claims[&17].claims, 4);
        release_value(
            &mut claims,
            LocalValue::Collection(CollectionValue::new(mir::CollectionTypeId(0), source)),
        );
        assert_eq!(claims[&17].claims, 1);
        release_value(
            &mut claims,
            LocalValue::Collection(CollectionValue::new(mir::CollectionTypeId(0), copied)),
        );
        assert!(claims.is_empty());
    }

    #[test]
    fn managed_copy_retention_allocates_no_second_state_and_releases_each_claim() {
        let value = token(17);
        let mut claims = claim_states([(value.bits, 1)]);
        for _ in 0..1000 {
            retain_claim(&mut claims, value).unwrap();
            assert_eq!(claims.len(), 1);
            assert_eq!(claims[&value.bits].claims, 2);
            release_claim(&mut claims, value).unwrap();
            assert_eq!(claims[&value.bits].claims, 1);
        }
        release_claim(&mut claims, value).unwrap();
        assert!(claims.is_empty());
        assert!(release_claim(&mut claims, value).is_err());
        assert!(retain_claim(&mut claims, value).is_err());
        claims.insert(value.bits, CancellationState::new(usize::MAX));
        assert!(retain_claim(&mut claims, value).is_err());
        assert_eq!(claims[&value.bits].claims, usize::MAX);
    }

    #[test]
    fn managed_copy_uses_the_native_count_limit_and_preserves_an_exhausted_claim() {
        let value = token(17);
        let maximum = isize::MAX as usize;
        let mut claims = claim_states([(value.bits, maximum - 1)]);
        retain_claim(&mut claims, value).unwrap();
        assert_eq!(claims[&value.bits].claims, maximum);
        for count in [maximum, maximum + 1, usize::MAX] {
            claims.insert(value.bits, CancellationState::new(count));
            let error = retain_claim(&mut claims, value).unwrap_err();
            assert_eq!(error.runtime_panic, Some("P1503"));
            assert_eq!(claims[&value.bits].claims, count);
        }
    }

    #[test]
    fn nullable_enum_and_collection_copy_preserve_the_typed_count_overflow() {
        let value = LocalValue::NullableScalar {
            ty: mir::ScalarType::Inline(token(17).ty),
            value: Some(mir::ScalarValue::Inline(token(17))),
        };
        let ty = mir::PayloadEnumType {
            id: crate::enums::EnumId(0),
            capabilities: crate::enums::EnumCapabilities {
                copy: true,
                trivial_copy: false,
                needs_drop: true,
                equality: false,
            },
            size: 24,
            align: 8,
            nullable_size: 32,
            nullable_payload_offset: 8,
        };
        let nested = LocalValue::PayloadEnum(PayloadEnumValue {
            ty,
            case: crate::enums::EnumCaseId {
                enum_id: ty.id,
                index: 0,
            },
            fields: vec![value.clone()],
            moved_fields: vec![false],
        });
        let mut claims = claim_states([(17, isize::MAX as usize)]);
        for value in [value, nested] {
            let error = retain_owned_claims(&mut claims, &value).unwrap_err();
            assert_eq!(error.runtime_panic, Some("P1503"));
            let error = retain_collection_claims(&mut claims, &vec![(None, value)]).unwrap_err();
            assert_eq!(error.runtime_panic, Some("P1503"));
            assert_eq!(claims[&17].claims, isize::MAX as usize);
        }
    }

    #[test]
    fn copy_count_overflow_is_an_authored_abort_without_running_pending_cleanup() {
        let source = "function main(): void { let $first = 1; let $second = $first; }";
        let program = crate::lower_source_to_mir("cancellation-copy.doria", source).unwrap();
        let mut interpreter = Interpreter::new(
            &program,
            InterpreterLimits::default(),
            MirIo::default(),
            doria_time::monotonic_nanoseconds,
        )
        .unwrap();
        interpreter
            .push_frame(program.entry, &[], &[], None, None)
            .unwrap();
        let copy = "let $second = $first;";
        let start = source.find(copy).unwrap();
        let span = Span::new(start, start + copy.len());
        interpreter.set_active_panic_site(span).unwrap();
        interpreter.cancellation_states = claim_states([(17, isize::MAX as usize), (23, 1)]);
        interpreter
            .push_scalar(mir::ScalarValue::Inline(token(17)))
            .unwrap();
        let frame = interpreter.current_frame_mut().unwrap();
        frame
            .tasks
            .push(EvaluationTask::ReleaseInlineValue(token(23)));
        frame.tasks.push(EvaluationTask::Inline(Task::Claim {
            ty: token(17).ty,
            mode: mir::InlineValueUseMode::Copy,
            local: None,
        }));
        let StepOutcome::RuntimePanic(event) = interpreter.step().unwrap() else {
            panic!("Copy exhaustion must abort, not return a compiler failure or continue");
        };
        assert_eq!(
            interpreter.cancellation_states[&17].claims,
            isize::MAX as usize
        );
        assert_eq!(interpreter.cancellation_states[&23].claims, 1);
        assert_eq!(interpreter.current_frame().unwrap().tasks.len(), 1);
        let output = interpreter.runtime_panic_output(event);
        assert_eq!(output.exit_status, 101);
        assert!(output.stdout.is_empty());
        let diagnostic = output.runtime_diagnostic.unwrap();
        assert_eq!(diagnostic.code, "P1503");
        assert_eq!(diagnostic.span, span);
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("cancellation-copy.doria"), "{stderr}");
        assert!(stderr.contains(copy), "{stderr}");
    }

    #[test]
    fn missing_inline_state_remains_an_interpreter_invariant_failure() {
        let mut claims = BTreeMap::new();
        let error = retain_claim(&mut claims, token(17)).unwrap_err();
        assert_eq!(error.runtime_panic, None);
        assert!(error.message.contains("no live owner"));
        assert!(claims.is_empty());

        let program =
            crate::lower_source_to_mir("missing-inline-state.doria", "function main(): void {}")
                .unwrap();
        let mut interpreter = Interpreter::new(
            &program,
            InterpreterLimits::default(),
            MirIo::default(),
            doria_time::monotonic_nanoseconds,
        )
        .unwrap();
        interpreter
            .push_frame(program.entry, &[], &[], None, None)
            .unwrap();
        interpreter
            .push_scalar(mir::ScalarValue::Inline(token(17)))
            .unwrap();
        interpreter
            .current_frame_mut()
            .unwrap()
            .tasks
            .push(EvaluationTask::RetainInlineClaims);
        let Err(error) = interpreter.step() else {
            panic!("missing inline state must remain an interpreter invariant failure");
        };
        assert_eq!(error.runtime_panic, None);
        assert!(error.message.contains("no live owner"));
        assert!(interpreter.cancellation_states.is_empty());
    }

    #[test]
    fn enum_copy_retains_nested_present_claims_but_not_moved_or_absent_fields() {
        let ty = mir::PayloadEnumType {
            id: crate::enums::EnumId(0),
            capabilities: crate::enums::EnumCapabilities {
                copy: true,
                trivial_copy: false,
                needs_drop: true,
                equality: false,
            },
            size: 32,
            align: 8,
            nullable_size: 40,
            nullable_payload_offset: 8,
        };
        let nested = LocalValue::PayloadEnum(PayloadEnumValue {
            ty,
            case: crate::enums::EnumCaseId {
                enum_id: ty.id,
                index: 0,
            },
            fields: vec![
                LocalValue::Scalar(mir::ScalarValue::Inline(token(17))),
                LocalValue::NullableScalar {
                    ty: mir::ScalarType::Inline(token(23).ty),
                    value: Some(mir::ScalarValue::Inline(token(23))),
                },
                LocalValue::NullableScalar {
                    ty: mir::ScalarType::Inline(token(29).ty),
                    value: None,
                },
                LocalValue::Scalar(mir::ScalarValue::Inline(token(31))),
            ],
            moved_fields: vec![false, false, false, true],
        });
        let value = LocalValue::NullablePayloadEnum {
            ty,
            value: Some(PayloadEnumValue {
                ty,
                case: crate::enums::EnumCaseId {
                    enum_id: ty.id,
                    index: 0,
                },
                fields: vec![nested],
                moved_fields: vec![false],
            }),
        };
        let mut claims = claim_states([(17, 1), (23, 1), (31, 1)]);
        retain_owned_claims(&mut claims, &value).unwrap();
        assert_eq!(claims, claim_states([(17, 2), (23, 2), (31, 1)]));
        release_claim(&mut claims, token(17)).unwrap();
        release_claim(&mut claims, token(23)).unwrap();
        assert_eq!(claims, claim_states([(17, 1), (23, 1), (31, 1)]));
    }

    const FAILURE_SOURCE: &str = r#"
use Doria\Std\Time\{ClockError, Deadline, Duration};
use Doria\Std\Io\IoError;
class Guard {
    function __construct() {}
    function __destruct() {
        try { echo "cleanup\n"; }
        catch (IoError) {}
    }
}
function create(): Deadline throws ClockError {
    let $guard = new Guard();
    return Deadline::fromNow(Duration::fromNanoseconds(-17));
}
function forward(): Deadline throws ClockError {
    try { return create(); }
    catch (ClockError $error) { throw $error; }
}
function main(): void {
    try { forward(); }
    catch (ClockError $error) { echo $error->message . "|" . ($error->systemCode ?? -1) . "\n"; }
}
"#;

    #[test]
    fn clock_failure_preserves_absent_and_zero_codes_rethrow_and_cleanup() {
        let program = crate::lower_source_to_mir("clock-failure.doria", FAILURE_SOURCE).unwrap();
        for (clock, expected_code) in [
            (
                (|| Err(doria_time::ClockFailure::InvalidReading)) as fn() -> _,
                -1,
            ),
            (
                (|| Err(doria_time::ClockFailure::System { system_code: 0 })) as fn() -> _,
                0,
            ),
            (
                (|| Err(doria_time::ClockFailure::System { system_code: 17 })) as fn() -> _,
                17,
            ),
        ] {
            let (execution, _) = interpret_internal_with_clock(
                &program,
                InterpreterLimits::default(),
                MirIo::default(),
                clock,
            )
            .unwrap();
            assert_eq!(execution.output.exit_status, 0);
            assert_eq!(
                execution.output.stdout,
                format!(
                    "cleanup\n{}|{expected_code}\n",
                    crate::compiler_known_time::CLOCK_FAILURE_MESSAGE
                )
                .as_bytes()
            );
            assert!(execution.output.stderr.is_empty());
        }
    }

    #[test]
    fn unhandled_clock_failure_uses_orderly_checked_outcome_and_authored_origin() {
        let source = FAILURE_SOURCE.replace(
            "function main(): void {\n    try { forward(); }\n    catch (ClockError $error) { echo $error->message . \"|\" . ($error->systemCode ?? -1) . \"\\n\"; }\n}",
            "function main(): void throws ClockError { forward(); }");
        assert_ne!(source, FAILURE_SOURCE);
        let program = crate::lower_source_to_mir("unhandled-clock.doria", source).unwrap();
        let (execution, _) = interpret_internal_with_clock(
            &program,
            InterpreterLimits::default(),
            MirIo::default(),
            || Err(doria_time::ClockFailure::Unsupported),
        )
        .unwrap();
        assert_eq!(execution.output.exit_status, 70);
        assert_eq!(execution.output.stdout, b"cleanup\n");
        let diagnostic = execution.output.runtime_diagnostic.unwrap();
        assert_eq!(diagnostic.code, "R1000");
        assert_ne!(
            diagnostic.span.source,
            crate::compiler_known_time::SOURCE_ID
        );
    }
}

#[derive(Debug, Clone)]
pub(super) enum Task {
    Value(mir::InlineValueExpression),
    CheckType(InlineValueType),
    Claim {
        ty: InlineValueType,
        mode: mir::InlineValueUseMode,
        local: Option<mir::LocalId>,
    },
    NullableClaim {
        ty: InlineValueType,
        mode: mir::InlineValueUseMode,
        local: Option<mir::LocalId>,
    },
    FromNanoseconds,
    Nanoseconds,
    Coalesce {
        ty: InlineValueType,
        right: mir::InlineValueExpression,
    },
}

impl Interpreter<'_> {
    fn cancellation_identity(&self, source: mir::LocalId) -> Result<u128, InterpreterError> {
        let LocalValue::Class { object, class } = self.read_local_value(source)? else {
            return Err(InterpreterError::new(
                "cancellation operation requires a source",
            ));
        };
        if class_in(self.program, class)?.runtime_payload
            != Some(mir::RuntimeClassPayload::CancellationSource)
        {
            return Err(InterpreterError::new(
                "cancellation operation requires runtime-owned source",
            ));
        }
        Ok(object as u128)
    }

    pub(super) fn cancellation_requested(
        &self,
        source: mir::LocalId,
    ) -> Result<bool, InterpreterError> {
        let identity = self.cancellation_identity(source)?;
        self.cancellation_state_requested(identity)
    }

    pub(super) fn cancellation_state_requested(
        &self,
        identity: u128,
    ) -> Result<bool, InterpreterError> {
        self.cancellation_states
            .get(&identity)
            .map(|state| state.requested)
            .ok_or_else(|| InterpreterError::new("cancellation source state has no live claim"))
    }

    pub(super) fn execute_cancellation_request(
        &mut self,
        source: mir::LocalId,
        error: mir::LocalId,
        success: mir::BlockId,
        failure: mir::BlockId,
    ) -> Result<StepOutcome, InterpreterError> {
        let identity = self.cancellation_identity(source)?;
        let request = self
            .cancellation_states
            .get_mut(&identity)
            .ok_or_else(|| InterpreterError::new("cancellation source state has no live claim"))?
            .request();
        let function = function_in(self.program, self.current_frame()?.function)?.clone();
        let target = match request {
            Ok(()) => success,
            Err(code) => {
                let value = self.allocate_error_object(crate::compiler_known_concurrency::CANCELLATION_ERROR,
                    vec![("message", LocalValue::String(crate::compiler_known_concurrency::CANCELLATION_NOTIFICATION_FAILURE_MESSAGE.into())),
                        ("systemCode", LocalValue::NullableScalar {
                            ty: mir::ScalarType::Integer(IntegerType::Int64),
                            value: code.map(|code| mir::ScalarValue::Integer(IntegerValue { ty: IntegerType::Int64, bits: code as u64 })),
                        })])?;
                let replaced = assign_local(
                    self.program,
                    &function.locals,
                    &mut self.current_frame_mut()?.locals,
                    error,
                    LocalValue::Error(value),
                )?;
                if replaced.is_some() {
                    return Err(InterpreterError::new(
                        "cancellation request overwrote an occupied Error slot",
                    ));
                }
                failure
            }
        };
        self.move_to_block(&function, target)?;
        Ok(StepOutcome::Continue)
    }

    pub(super) fn execute_deadline_from_now(
        &mut self,
        duration: mir::LocalId,
        result: Option<mir::LocalId>,
        error: mir::LocalId,
        success: mir::BlockId,
        failure: mir::BlockId,
    ) -> Result<StepOutcome, InterpreterError> {
        let function = function_in(self.program, self.current_frame()?.function)?.clone();
        let mir::ScalarValue::Inline(duration) =
            self.eval_operand(&mir::Operand::Local(duration))?
        else {
            return Err(InterpreterError::new("relative deadline requires Duration"));
        };
        let instant = (self.monotonic_clock)()
            .map(|sample| doria_time::deadline_from_sample(sample, duration.bits as i64));
        let (target, local, value) = match instant {
            Ok(instant) => (
                success,
                result
                    .ok_or_else(|| InterpreterError::new("relative deadline has no result slot"))?,
                LocalValue::Scalar(mir::ScalarValue::Inline(crate::inline_value::InlineValue {
                    ty: InlineValueType::Deadline,
                    bits: instant as u128,
                })),
            ),
            Err(clock_failure) => {
                let value = self.allocate_error_object(
                    crate::compiler_known_time::CLOCK_ERROR,
                    vec![
                        (
                            "message",
                            LocalValue::String(
                                crate::compiler_known_time::CLOCK_FAILURE_MESSAGE.into(),
                            ),
                        ),
                        (
                            "systemCode",
                            LocalValue::NullableScalar {
                                ty: mir::ScalarType::Integer(IntegerType::Int64),
                                value: clock_failure.system_code().map(|code| {
                                    mir::ScalarValue::Integer(IntegerValue {
                                        ty: IntegerType::Int64,
                                        bits: code as u64,
                                    })
                                }),
                            },
                        ),
                    ],
                )?;
                (failure, error, LocalValue::Error(value))
            }
        };
        let replaced = assign_local(
            self.program,
            &function.locals,
            &mut self.current_frame_mut()?.locals,
            local,
            value,
        )?;
        if replaced.is_some() && local_in(&function, local)?.owned {
            return Err(InterpreterError::new(
                "relative deadline overwrote an occupied Error slot",
            ));
        }
        self.move_to_block(&function, target)?;
        Ok(StepOutcome::Continue)
    }

    pub(super) fn execute_inline_task(
        &mut self,
        task: Task,
    ) -> Result<StepOutcome, InterpreterError> {
        match task {
            Task::Value(expression) => match expression {
                mir::InlineValueExpression::InterfaceReceiver {
                    ty,
                    receiver,
                    vtable,
                    transfer,
                } => {
                    let LocalValue::Error(value) = self.read_or_take_local(receiver, transfer)?
                    else {
                        return Err(InterpreterError::new(
                            "inline interface receiver is not a carrier",
                        ));
                    };
                    if value.vtable != vtable {
                        return Err(InterpreterError::new(
                            "inline interface receiver has another dynamic type",
                        ));
                    }
                    self.push_scalar(mir::ScalarValue::Inline(crate::inline_value::InlineValue {
                        ty,
                        bits: value.object as u128,
                    }))?;
                }
                mir::InlineValueExpression::CancellationToken { ty, source, span } => {
                    self.set_active_panic_site(span)?;
                    let value = crate::inline_value::InlineValue {
                        ty,
                        bits: self.cancellation_identity(source)?,
                    };
                    retain_claim(&mut self.cancellation_states, value)?;
                    self.push_scalar(mir::ScalarValue::Inline(value))?;
                }
                mir::InlineValueExpression::DurationFromNanoseconds(value) => {
                    let frame = self.current_frame_mut()?;
                    frame
                        .tasks
                        .push(EvaluationTask::Inline(Task::FromNanoseconds));
                    frame.tasks.push(EvaluationTask::Integer(*value));
                }
                mir::InlineValueExpression::Use { ty, operand, mode } => {
                    let local = match operand {
                        mir::Operand::Local(local) | mir::Operand::NullablePayload(local) => {
                            Some(local)
                        }
                        _ => None,
                    };
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::Inline(Task::Claim { ty, mode, local }));
                    // Collection reads evaluate asynchronously; check their
                    // result too, rather than losing the nominal type boundary.
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::Inline(Task::CheckType(ty)));
                    if !self.queue_collection_scalar_operand(&operand)? {
                        let value = self.eval_operand(&operand)?;
                        self.push_scalar(value)?;
                    }
                }
                mir::InlineValueExpression::Call {
                    ty,
                    function,
                    args,
                    span,
                    ..
                } => {
                    self.queue_call(
                        function,
                        args,
                        ReturnExpectation::Value(mir::Type::Scalar(mir::ScalarType::Inline(ty))),
                        span,
                    )?;
                }
                mir::InlineValueExpression::Coalesce { ty, left, right } => {
                    if left.ty() != mir::ScalarType::Inline(ty) || right.ty() != ty {
                        return Err(InterpreterError::new(
                            "MIR inline coalesce operands changed type identity",
                        ));
                    }
                    let frame = self.current_frame_mut()?;
                    frame
                        .tasks
                        .push(EvaluationTask::Inline(Task::Coalesce { ty, right: *right }));
                    frame.tasks.push(EvaluationTask::NullableScalar(*left));
                }
            },
            Task::FromNanoseconds => {
                let value = self.pop_integer()?;
                if value.ty != IntegerType::Int64 {
                    return Err(InterpreterError::new(
                        "Duration factory requires int64 nanoseconds",
                    ));
                }
                self.push_scalar(mir::ScalarValue::Inline(crate::inline_value::InlineValue {
                    ty: InlineValueType::Duration,
                    bits: u128::from(value.bits),
                }))?;
            }
            Task::Nanoseconds => {
                let mir::ScalarValue::Inline(value) = self.pop_scalar()? else {
                    return Err(InterpreterError::new(
                        "nanoseconds getter requires Duration",
                    ));
                };
                if value.ty != InlineValueType::Duration {
                    return Err(InterpreterError::new(
                        "nanoseconds getter requires Duration",
                    ));
                }
                self.push_scalar(mir::ScalarValue::Integer(IntegerValue {
                    ty: IntegerType::Int64,
                    bits: value.bits as u64,
                }))?;
            }
            Task::Claim { ty, mode, local } => {
                let mir::ScalarValue::Inline(value) = self.pop_scalar()? else {
                    return Err(InterpreterError::new(
                        "inline claim requires an inline value",
                    ));
                };
                if value.ty != ty {
                    return Err(InterpreterError::new("inline claim changed nominal type"));
                }
                self.apply_inline_claim(value, mode, local)?;
                self.push_scalar(mir::ScalarValue::Inline(value))?;
            }
            Task::NullableClaim { ty, mode, local } => {
                let (actual, value) = self.pop_nullable_scalar()?;
                if actual != mir::ScalarType::Inline(ty) {
                    return Err(InterpreterError::new(
                        "nullable inline claim changed nominal type",
                    ));
                }
                if let Some(value) = value {
                    let mir::ScalarValue::Inline(value) = value else {
                        return Err(InterpreterError::new(
                            "nullable inline claim contains a non-inline payload",
                        ));
                    };
                    if value.ty != ty {
                        return Err(InterpreterError::new(
                            "nullable inline claim changed payload type",
                        ));
                    }
                    self.apply_inline_claim(value, mode, None)?;
                }
                if mode == mir::InlineValueUseMode::TransferOwnedCopy {
                    let local = local.ok_or_else(|| {
                        InterpreterError::new("nullable inline handoff requires a local")
                    })?;
                    self.current_frame_mut()?.locals[local.0] = None;
                }
                self.push_nullable_scalar(actual, value)?;
            }
            Task::CheckType(ty) => {
                let value = self.pop_scalar()?;
                if value.ty() != mir::ScalarType::Inline(ty) {
                    return Err(InterpreterError::new(
                        "MIR inline operand changed type identity",
                    ));
                }
                self.push_scalar(value)?;
            }
            Task::Coalesce { ty, right } => {
                let (actual, value) = self.pop_nullable_scalar()?;
                if actual != mir::ScalarType::Inline(ty) {
                    return Err(InterpreterError::new(
                        "MIR inline coalesce payload changed type identity",
                    ));
                }
                if let Some(value) = value {
                    if value.ty() != actual {
                        return Err(InterpreterError::new(
                            "MIR inline coalesce payload changed type identity",
                        ));
                    }
                    self.push_scalar(value)?;
                } else {
                    self.current_frame_mut()?
                        .tasks
                        .push(EvaluationTask::Inline(Task::Value(right)));
                }
            }
        }
        Ok(StepOutcome::Continue)
    }

    fn apply_inline_claim(
        &mut self,
        value: crate::inline_value::InlineValue,
        mode: mir::InlineValueUseMode,
        local: Option<mir::LocalId>,
    ) -> Result<(), InterpreterError> {
        match mode {
            mir::InlineValueUseMode::Borrow => {}
            mir::InlineValueUseMode::Copy => {
                retain_claim(&mut self.cancellation_states, value)?;
            }
            mir::InlineValueUseMode::TransferOwnedCopy => {
                if let Some(local) = local {
                    self.current_frame_mut()?.locals[local.0] = None;
                }
            }
        }
        Ok(())
    }

    pub(super) fn release_inline_claim(
        &mut self,
        value: crate::inline_value::InlineValue,
    ) -> Result<(), InterpreterError> {
        release_claim(&mut self.cancellation_states, value)
    }
}

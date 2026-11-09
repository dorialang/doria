//! Owning registrations over the interpreter's virtual readiness sources.
use super::*;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Source {
    File(usize),
    Standard(mir::StandardIoStream),
}

impl Interpreter<'_> {
    pub(super) fn readiness_source_home(&self, source: Source) -> usize {
        match source {
            Source::File(file) => file,
            Source::Standard(stream) => {
                self.standard_source_homes[usize::from(stream.runtime_tag())]
            }
        }
    }

    pub(super) fn readiness_source_value(
        &self,
        value: &LocalValue,
    ) -> Result<Source, InterpreterError> {
        let LocalValue::Class { object, class } = *value else {
            return Err(InterpreterError::new(
                "readiness getter returned another value shape",
            ));
        };
        let definition = class_in(self.program, class)?;
        if definition.name != crate::compiler_known_stream::READINESS_SOURCE
            || definition.source_span.source != crate::compiler_known_stream::SOURCE_ID
        {
            return Err(InterpreterError::new(
                "readiness getter returned another nominal class",
            ));
        }
        if let Some(index) = self
            .standard_source_homes
            .iter()
            .position(|home| *home == object)
        {
            Ok(Source::Standard(
                [
                    mir::StandardIoStream::Input,
                    mir::StandardIoStream::Output,
                    mir::StandardIoStream::Error,
                ][index],
            ))
        } else if self.file_owners.contains_key(&object) {
            Ok(Source::File(object))
        } else {
            Err(InterpreterError::new(
                "readiness getter returned a destroyed source",
            ))
        }
    }

    pub(super) fn readiness_source_reason(&self, source: Source) -> Option<IoErrorReason> {
        match source {
            Source::Standard(_) => None,
            Source::File(file) => match self.file_owners.get(&file) {
                Some(owner) if owner.open => None,
                Some(_) => Some(IoErrorReason::Closed),
                None => Some(IoErrorReason::InvalidInput),
            },
        }
    }

    pub(super) fn readiness_source_target(
        &self,
        source: Source,
    ) -> Result<IoTarget, InterpreterError> {
        Ok(match source {
            Source::File(file) => self
                .file_owners
                .get(&file)
                .ok_or_else(|| InterpreterError::new("readiness source has no file owner"))?
                .target(),
            Source::Standard(mir::StandardIoStream::Input) => IoTarget::StandardInput,
            Source::Standard(mir::StandardIoStream::Output) => IoTarget::StandardOutput,
            Source::Standard(mir::StandardIoStream::Error) => IoTarget::StandardError,
        })
    }
}
use crate::compiler_known_io::{IoErrorReason, IoOperation, IoTarget};
use crate::compiler_known_stream::StreamOperation;
use crate::inline_value::{InlineValue, InlineValueType};

#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct SetOwner {
    next_registration: u64,
    pub(super) active: bool,
    // Monotonic identities preserve acquisition order even after removal.
    pub(super) members: BTreeMap<u64, (Source, Option<LocalValue>)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Registration {
    set: mir::LocalId,
    stream: mir::LocalId,
    key: InlineValueType,
    result: Option<mir::LocalId>,
    error: mir::LocalId,
    success: mir::BlockId,
    failure: mir::BlockId,
}

impl Registration {
    pub(super) fn new(
        set: mir::LocalId,
        stream: mir::LocalId,
        key: InlineValueType,
        result: Option<mir::LocalId>,
        error: mir::LocalId,
        success: mir::BlockId,
        failure: mir::BlockId,
    ) -> Self {
        Self {
            set,
            stream,
            key,
            result,
            error,
            success,
            failure,
        }
    }
}

impl Interpreter<'_> {
    pub(super) fn begin_readiness_registration(
        &mut self,
        registration: Registration,
        span: Span,
    ) -> Result<StepOutcome, InterpreterError> {
        let set = self.readiness_set_id(registration.set)?;
        if self.readiness_sets[&set].active {
            let error = self.allocate_io_error(
                IoOperation::Register,
                IoTarget::ReadinessSet,
                IoErrorReason::InvalidInput,
                None,
                None,
            )?;
            return self
                .finish_readiness_registration(registration, FunctionOutcome::CheckedError(error));
        }
        let frame = self.current_frame()?;
        let function = function_in(self.program, frame.function)?;
        let ty = local_in(function, registration.stream)?.ty;
        let value = read_local(&frame.locals, registration.stream)?;
        let (carrier, entry) =
            self.stream_callback_entry(ty, value, StreamOperation::ReadinessSource)?;
        // Keep the original owner alive across the ordinary borrowed getter.
        // Both return paths below resolve the operation's take exactly once.
        self.push_checked_frame(
            entry,
            &[LocalValue::Error(carrier)],
            &[None],
            CheckedContinuation::Register(registration),
            span,
        )?;
        Ok(StepOutcome::Continue)
    }

    pub(super) fn stream_callback_entry(
        &self,
        ty: mir::Type,
        value: &LocalValue,
        operation: StreamOperation,
    ) -> Result<(InterfaceCarrier, mir::FunctionId), InterpreterError> {
        let plan = self.program.stream_callback(ty, operation).ok_or_else(|| {
            InterpreterError::new("stream has no nominal callback for this operation")
        })?;
        let (carrier, entry) = match (plan.target, value) {
            (
                mir::StreamCallbackTarget::Static { vtable, entry },
                LocalValue::Scalar(mir::ScalarValue::Inline(value)),
            ) => (
                InterfaceCarrier {
                    object: value.bits as usize,
                    vtable,
                },
                entry,
            ),
            (
                mir::StreamCallbackTarget::Static { vtable, entry },
                LocalValue::Class { object, .. },
            ) => (
                InterfaceCarrier {
                    object: *object,
                    vtable,
                },
                entry,
            ),
            (mir::StreamCallbackTarget::ClassDescriptor, LocalValue::Class { object, .. }) => {
                let dynamic_class = self
                    .heap
                    .get(object)
                    .ok_or_else(|| InterpreterError::new("registered class owner was released"))?
                    .class;
                let vtable = self
                    .program
                    .interface_vtable(mir::ImplementingType::Class(dynamic_class), plan.interface)
                    .ok_or_else(|| {
                        InterpreterError::new("registered class has no readiness method view")
                    })?;
                (
                    InterfaceCarrier {
                        object: *object,
                        vtable,
                    },
                    self.program.interface_vtables[vtable.0].methods[plan.slot],
                )
            }
            (mir::StreamCallbackTarget::InterfaceDescriptor, LocalValue::Error(carrier)) => (
                *carrier,
                self.program.interface_vtables[carrier.vtable.0].methods[plan.slot],
            ),
            _ => {
                return Err(InterpreterError::new(
                    "registration changed its original receiver representation",
                ))
            }
        };
        Ok((carrier, entry))
    }

    pub(super) fn finish_readiness_registration(
        &mut self,
        registration: Registration,
        getter_outcome: FunctionOutcome,
    ) -> Result<StepOutcome, InterpreterError> {
        let set = self.readiness_set_id(registration.set)?;
        let value = self.current_frame_mut()?.locals[registration.stream.0]
            .take()
            .ok_or_else(|| InterpreterError::new("registration lost its taken stream"))?;
        let mut taken = Some(value);
        let outcome = match getter_outcome {
            FunctionOutcome::CheckedError(error) => CheckedIoResult::Error(error),
            FunctionOutcome::Value(value) => {
                let source = self.readiness_source_value(&value)?;
                let reason = self.readiness_source_reason(source);
                if let Some(reason) = reason {
                    self.readiness_membership_error(IoOperation::Register, reason)?
                } else {
                    let owner = self.readiness_sets.get_mut(&set).ok_or_else(|| {
                        InterpreterError::new("registration set owner was released")
                    })?;
                    if let Some(identity) = owner.next_registration.checked_add(1) {
                        owner.next_registration = identity;
                        owner.members.insert(identity, (source, taken.take()));
                        CheckedIoResult::Success(Some(LocalValue::Scalar(
                            mir::ScalarValue::Inline(InlineValue {
                                ty: registration.key,
                                bits: set as u128 | ((identity as u128) << 64),
                            }),
                        )))
                    } else {
                        self.readiness_membership_error(
                            IoOperation::Register,
                            IoErrorReason::ResourceExhausted,
                        )?
                    }
                }
            }
            _ => {
                return Err(InterpreterError::new(
                    "readiness getter returned another value shape",
                ))
            }
        };
        self.finish_checked_io_result(
            outcome,
            registration.result,
            registration.error,
            registration.success,
            registration.failure,
        )?;
        if let Some(value) = taken {
            let mut drops = Vec::new();
            collect_owned_objects_from_value(value, &mut drops);
            for drop in drops.into_iter().rev() {
                self.push_owned_drop_task(drop)?;
            }
        }
        Ok(StepOutcome::Continue)
    }

    pub(super) fn unregister_readiness_stream(
        &mut self,
        set: mir::LocalId,
        key: mir::LocalId,
    ) -> Result<CheckedIoResult, InterpreterError> {
        let set = self.readiness_set_id(set)?;
        if self.readiness_sets[&set].active {
            return self
                .readiness_membership_error(IoOperation::Unregister, IoErrorReason::InvalidInput);
        }
        let LocalValue::Scalar(mir::ScalarValue::Inline(key)) =
            read_local(&self.current_frame()?.locals, key)?
        else {
            return Err(InterpreterError::new(
                "removal has no inline registration key",
            ));
        };
        let identity = (key.bits >> 64) as u64;
        let value = (key.bits & u64::MAX as u128 == set as u128)
            .then(|| {
                self.readiness_sets
                    .get_mut(&set)
                    .and_then(|owner| owner.members.remove(&identity))
            })
            .flatten();
        match value {
            Some((_, Some(value))) => Ok(CheckedIoResult::Success(Some(value))),
            Some((_, None)) => Err(InterpreterError::new(
                "registration source was transferred during removal",
            )),
            None => self
                .readiness_membership_error(IoOperation::Unregister, IoErrorReason::InvalidInput),
        }
    }

    pub(super) fn borrow_readiness_stream(
        &mut self,
        set: mir::LocalId,
        key: mir::LocalId,
        stream_type: mir::Type,
    ) -> Result<(CheckedIoResult, Option<InterpreterPlace>), InterpreterError> {
        let set = self.readiness_set_id(set)?;
        if self.readiness_sets[&set].active {
            return Ok((
                self.readiness_membership_error(IoOperation::Access, IoErrorReason::InvalidInput)?,
                None,
            ));
        }
        let LocalValue::Scalar(mir::ScalarValue::Inline(key)) =
            read_local(&self.current_frame()?.locals, key)?
        else {
            return Err(InterpreterError::new(
                "lookup has no inline registration key",
            ));
        };
        let identity = (key.bits >> 64) as u64;
        let value = (key.bits & u64::MAX as u128 == set as u128)
            .then(|| self.readiness_sets.get(&set)?.members.get(&identity))
            .flatten();
        match value {
            Some((_, Some(value))) => Ok((
                CheckedIoResult::Success(Some(value.clone())),
                Some(InterpreterPlace {
                    storage: InterpreterPlaceStorage::ReadinessMember {
                        set,
                        registration: identity,
                        ty: stream_type,
                    },
                    ty: stream_type,
                    payload_projection: None,
                }),
            )),
            Some((_, None)) => Err(InterpreterError::new(
                "registration source was transferred during lookup",
            )),
            None => Ok((
                self.readiness_membership_error(IoOperation::Access, IoErrorReason::InvalidInput)?,
                None,
            )),
        }
    }

    pub(super) fn readiness_member_slot(
        &self,
        set: usize,
        registration: u64,
    ) -> Result<&Option<LocalValue>, InterpreterError> {
        self.readiness_sets
            .get(&set)
            .and_then(|owner| owner.members.get(&registration))
            .map(|(_, value)| value)
            .ok_or_else(|| InterpreterError::new("borrowed registration source was removed"))
    }

    pub(super) fn readiness_member_slot_mut(
        &mut self,
        set: usize,
        registration: u64,
    ) -> Result<&mut Option<LocalValue>, InterpreterError> {
        self.readiness_sets
            .get_mut(&set)
            .and_then(|owner| owner.members.get_mut(&registration))
            .map(|(_, value)| value)
            .ok_or_else(|| InterpreterError::new("borrowed registration source was removed"))
    }

    pub(super) fn readiness_set_id(&self, local: mir::LocalId) -> Result<usize, InterpreterError> {
        let LocalValue::Class { object, .. } = read_local(&self.current_frame()?.locals, local)?
        else {
            return Err(InterpreterError::new(
                "readiness operation has no set owner",
            ));
        };
        if !self.readiness_sets.contains_key(object) {
            return Err(InterpreterError::new("readiness set owner was released"));
        }
        Ok(*object)
    }

    fn readiness_membership_error(
        &mut self,
        operation: IoOperation,
        reason: IoErrorReason,
    ) -> Result<CheckedIoResult, InterpreterError> {
        Ok(CheckedIoResult::Error(self.allocate_io_error(
            operation,
            IoTarget::ReadinessSet,
            reason,
            None,
            None,
        )?))
    }
}

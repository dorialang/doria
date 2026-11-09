//! Standard transfers share the interpreter's intrinsic input cursor and output
//! captures. Regions are borrowed directly; there is no second byte queue.
use super::*;
use crate::compiler_known_io::{IoOperation, IoTarget};

pub(super) enum Transfer {
    Outcome(file_stream::FileTransferResult),
    CleanExit,
}

impl Interpreter<'_> {
    pub(super) fn prepare_standard_region(
        &mut self,
        request: usize,
    ) -> Result<(), InterpreterError> {
        let Some(file_stream::RequestSelection::Native {
            source: readiness_set::Source::Standard(stream),
            region,
            ..
        }) = self.pending_requests[&request].selection
        else {
            return Ok(());
        };
        if !stream.is_input()
            || (region.length != 0
                && self.stdin_buffered_end == self.stdin_cursor
                && !self.stdin_eof_observed)
        {
            return Ok(());
        }
        let name = if stream.is_input() {
            "Doria\\Std\\Io\\ReadOutcome"
        } else {
            "Doria\\Std\\Io\\WriteOutcome"
        };
        let ty = self
            .program
            .enums
            .iter()
            .find(|definition| definition.name == name)
            .and_then(|definition| definition.payload_type())
            .ok_or_else(|| InterpreterError::new("standard preparation has no outcome type"))?;
        let Transfer::Outcome(Ok(LocalValue::PayloadEnum(outcome))) =
            self.transfer_standard_region(stream, region, ty)?
        else {
            return Err(InterpreterError::new(
                "queued standard preparation did not produce an inline outcome",
            ));
        };
        let view = self.pending_requests.get_mut(&request).unwrap();
        view.outcome = Some(outcome);
        let Some(file_stream::RequestSelection::Native { prepared, .. }) = &mut view.selection
        else {
            unreachable!()
        };
        *prepared = true;
        Ok(())
    }

    pub(super) fn standard_access_error(
        &mut self,
        stream: mir::StandardIoStream,
        operation: IoOperation,
    ) -> Result<CheckedIoResult, InterpreterError> {
        let target = self.readiness_source_target(readiness_set::Source::Standard(stream))?;
        Ok(CheckedIoResult::Error(self.allocate_io_error(
            operation,
            target,
            crate::compiler_known_io::IoErrorReason::InvalidInput,
            None,
            None,
        )?))
    }

    pub(super) fn transfer_standard_bytes(
        &mut self,
        stream: mir::StandardIoStream,
        region: mir::LocalId,
        outcome: mir::PayloadEnumType,
        call: transfer_group::CheckedCall,
    ) -> Result<StepOutcome, InterpreterError> {
        let region = self.region_local(region)?;
        let result = if region.length != 0
            && self.standard_claims[usize::from(stream.runtime_tag())].is_some()
        {
            Transfer::Outcome(Err(file_stream::FileTransferFailure {
                target: self.readiness_source_target(readiness_set::Source::Standard(stream))?,
                reason: crate::compiler_known_io::IoErrorReason::InvalidInput,
                system_code: None,
                count: None,
            }))
        } else {
            self.transfer_standard_region(stream, region, outcome)?
        };
        let Transfer::Outcome(result) = result else {
            return Ok(StepOutcome::CleanExit);
        };
        let result = self.finish_stream_transfer(
            if stream.is_input() {
                IoOperation::Read
            } else {
                IoOperation::Write
            },
            result,
        )?;
        self.finish_checked_io_result(result, call.result, call.error, call.success, call.failure)
    }

    pub(super) fn transfer_standard_region(
        &mut self,
        stream: mir::StandardIoStream,
        region: ByteRegionValue,
        outcome: mir::PayloadEnumType,
    ) -> Result<Transfer, InterpreterError> {
        let reading = stream.is_input();
        let (target, fault) = match stream {
            mir::StandardIoStream::Input => (
                IoTarget::StandardInput,
                self.io_faults
                    .stdin_bytes_read
                    .then_some(MirIoWriteFailure::Other),
            ),
            mir::StandardIoStream::Output => {
                (IoTarget::StandardOutput, self.io_faults.stdout_write)
            }
            mir::StandardIoStream::Error => (IoTarget::StandardError, self.io_faults.stderr_write),
        };
        let queued =
            reading && (self.stdin_buffered_end > self.stdin_cursor || self.stdin_eof_observed);
        let requested = if reading {
            region.length.min(
                if queued {
                    self.stdin_buffered_end
                } else {
                    self.stdin.len()
                } - self.stdin_cursor,
            )
        } else {
            region.length
        };
        let fault = fault.filter(|_| region.length != 0 && !queued);
        let transferred = if fault.is_some() {
            self.fault_transfer_length(requested)?
        } else {
            Some(requested)
        };
        let count = transferred.unwrap_or(0);
        if count != 0 {
            let base = region
                .source
                .ok_or_else(|| InterpreterError::new("standard region has no source"))?;
            let (entries, _) = self.collection_element_storage(base)?;
            if reading {
                let mut entries = entries.borrow_mut();
                for index in 0..count {
                    entries[base.index + index].1 = LocalValue::Scalar(mir::ScalarValue::Integer(
                        IntegerValue::from_i128(
                            IntegerType::UInt8,
                            i128::from(self.stdin[self.stdin_cursor + index]),
                        )
                        .expect("byte fits uint8"),
                    ));
                }
                self.stdin_cursor += count;
                self.stdin_buffered_end = self.stdin_buffered_end.max(self.stdin_cursor);
            } else {
                let entries = entries.borrow();
                let output = if stream == mir::StandardIoStream::Error {
                    &mut self.stderr
                } else {
                    &mut self.stdout
                };
                for index in 0..count {
                    let LocalValue::Scalar(mir::ScalarValue::Integer(value)) =
                        &entries[base.index + index].1
                    else {
                        return Err(InterpreterError::new(
                            "standard region has a nonbyte element",
                        ));
                    };
                    output.push(value.unsigned_value() as u8);
                }
            }
        }
        if fault == Some(MirIoWriteFailure::BrokenPipe) {
            return Ok(Transfer::CleanExit);
        }
        if reading && region.length != 0 && count == 0 && fault.is_none() {
            self.stdin_eof_observed = true;
        }
        let result = if fault.is_some() {
            Err(file_stream::FileTransferFailure {
                target,
                reason: self.io_faults.failure_reason.compiler_reason(),
                system_code: self.io_faults.system_code,
                count: transferred,
            })
        } else {
            let eof = reading && region.length != 0 && count == 0;
            let fields = if eof {
                Vec::new()
            } else {
                vec![LocalValue::Scalar(mir::ScalarValue::Integer(
                    IntegerValue::from_i128(IntegerType::Int64, count as i128)
                        .expect("region length fits int"),
                ))]
            };
            let value = self.payload_enum_value(
                if reading {
                    "Doria\\Std\\Io\\ReadOutcome"
                } else {
                    "Doria\\Std\\Io\\WriteOutcome"
                },
                if eof {
                    "EndOfStream"
                } else if reading {
                    "Data"
                } else {
                    "Accepted"
                },
                fields,
            )?;
            if local_value_type(self.program, &value) != mir::Type::PayloadEnum(outcome) {
                return Err(InterpreterError::new(
                    "standard transfer changed its outcome type",
                ));
            }
            Ok(value)
        };
        Ok(Transfer::Outcome(result))
    }
}

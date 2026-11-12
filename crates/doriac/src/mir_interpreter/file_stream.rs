//! Owned cursors over the interpreter's supplied virtual filesystem.
use super::readiness_set::Source;
use super::*;
use crate::compiler_known_io::{IoErrorReason, IoOperation, IoTarget};

pub(super) struct FileOwner {
    path: String,
    position: usize,
    append: bool,
    pub(super) open: bool,
    pins: usize,
    retired: bool,
    /// Whole-file lock held through a Doria guard: `true` for exclusive.
    lock: Option<bool>,
    /// One end of a scripted child's pipe; `path` is then its program.
    pub(super) pipe: Option<super::child_process::PipeEnd>,
}

impl FileOwner {
    pub(super) fn pipe(program: String, end: super::child_process::PipeEnd) -> Self {
        Self {
            path: program,
            position: 0,
            append: false,
            open: true,
            pins: 0,
            retired: false,
            lock: None,
            pipe: Some(end),
        }
    }

    pub(super) fn target(&self) -> IoTarget {
        match self.pipe {
            Some(end) => end.target(&self.path),
            None => IoTarget::File(self.path.clone()),
        }
    }
}

pub(super) struct FileTransferFailure {
    pub(super) target: IoTarget,
    pub(super) reason: IoErrorReason,
    pub(super) system_code: Option<i64>,
    pub(super) count: Option<usize>,
}

pub(super) type FileTransferResult = Result<LocalValue, FileTransferFailure>;

fn resize_virtual_file(contents: &mut Vec<u8>, length: usize) -> Result<(), IoErrorReason> {
    contents
        .try_reserve(length.saturating_sub(contents.len()))
        .map_err(|_| IoErrorReason::ResourceExhausted)?;
    contents.resize(length, 0);
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PendingFileConfiguration {
    pub(super) request: usize,
    pub(super) source: Source,
    pub(super) region: ByteRegionValue,
    pub(super) call: transfer_group::CheckedCall,
}

pub(super) enum RequestSelection {
    Native {
        source: Source,
        region: ByteRegionValue,
        prepared: bool,
    },
    Immediate(PayloadEnumValue),
}

pub(super) struct RequestView {
    pub(super) selection: Option<RequestSelection>,
    pub(super) invalid: bool,
    pub(super) observing: bool,
    pub(super) outcome: Option<PayloadEnumValue>,
    pub(super) failure: Option<(IoErrorReason, Option<i64>, Option<usize>)>,
    pub(super) target: IoTarget,
    pub(super) reading: bool,
    pub(super) source: Option<Source>,
    pub(super) bound: usize,
    pub(super) owner: Option<(usize, usize)>,
    pub(super) pin: Option<Source>,
}

impl RequestView {
    fn configure(&mut self, selection: RequestSelection) {
        if self.invalid || self.observing || self.selection.is_some() {
            self.invalid = true;
            return;
        }
        let valid = match &selection {
            RequestSelection::Native { source, .. } => Some(*source) == self.source,
            RequestSelection::Immediate(outcome) => match outcome.case.index {
                0 => matches!(outcome.fields.as_slice(),
                    [LocalValue::Scalar(mir::ScalarValue::Integer(count))]
                    if count.signed_value() >= 0 && count.unsigned_value() <= self.bound as u128),
                1 => outcome.fields.is_empty(),
                2 | 3 => self.reading && outcome.fields.is_empty(),
                _ => false,
            },
        };
        if valid {
            self.selection = Some(selection);
        } else {
            self.invalid = true;
        }
    }
}

impl Interpreter<'_> {
    pub(super) fn begin_pending_file_configuration(
        &mut self,
        file: mir::LocalId,
        request: mir::LocalId,
        region: mir::LocalId,
        call: transfer_group::CheckedCall,
    ) -> Result<StepOutcome, InterpreterError> {
        let source = Source::File(self.file_owner_id(file)?);
        self.begin_pending_native_configuration(source, request, region, call)
    }

    pub(super) fn begin_pending_native_configuration(
        &mut self,
        source: Source,
        request: mir::LocalId,
        region: mir::LocalId,
        call: transfer_group::CheckedCall,
    ) -> Result<StepOutcome, InterpreterError> {
        let region = self.region_local(region)?;
        let request = self.pending_request_id(request)?;
        let view = &self.pending_requests[&request];
        let (group, position) = view.owner.ok_or_else(|| {
            InterpreterError::new("pending configuration has no live group operation")
        })?;
        let configuration = PendingFileConfiguration {
            request,
            source,
            region,
            call,
        };
        if view.invalid || view.observing || view.selection.is_some() {
            self.pending_requests.get_mut(&request).unwrap().invalid = true;
            return self.finish_pending_file_configuration(configuration, FunctionOutcome::Void);
        }
        self.push_transfer_source_getter(
            group,
            position,
            CheckedContinuation::PendingFile(configuration),
            call.span,
        )?;
        Ok(StepOutcome::Continue)
    }

    pub(super) fn finish_pending_file_configuration(
        &mut self,
        configuration: PendingFileConfiguration,
        getter: FunctionOutcome,
    ) -> Result<StepOutcome, InterpreterError> {
        let request = configuration.request;
        let reason = match getter {
            FunctionOutcome::CheckedError(error) => {
                let call = configuration.call;
                return self.finish_checked_io_result(
                    CheckedIoResult::Error(error),
                    call.result,
                    call.error,
                    call.success,
                    call.failure,
                );
            }
            FunctionOutcome::Value(value) => {
                let source = self.readiness_source_value(&value)?;
                if !self.pending_requests[&request].invalid {
                    if let Some(reason) = self.replace_request_source(request, source)? {
                        return self.finish_pending_configuration_error(configuration, reason);
                    }
                    self.pending_requests.get_mut(&request).unwrap().configure(
                        RequestSelection::Native {
                            source: configuration.source,
                            region: configuration.region,
                            prepared: false,
                        },
                    );
                }
                if self.pending_requests[&request].invalid {
                    Some(IoErrorReason::InvalidInput)
                } else {
                    self.readiness_source_reason(configuration.source)
                }
            }
            FunctionOutcome::Void if self.pending_requests[&request].invalid => {
                Some(IoErrorReason::InvalidInput)
            }
            _ => {
                return Err(InterpreterError::new(
                    "pending configuration getter has another shape",
                ))
            }
        };
        if let Some(reason) = reason {
            return self.finish_pending_configuration_error(configuration, reason);
        }
        let call = configuration.call;
        self.finish_checked_io_result(
            CheckedIoResult::Success(None),
            call.result,
            call.error,
            call.success,
            call.failure,
        )
    }

    fn finish_pending_configuration_error(
        &mut self,
        configuration: PendingFileConfiguration,
        reason: IoErrorReason,
    ) -> Result<StepOutcome, InterpreterError> {
        let target = self.readiness_source_target(configuration.source)?;
        let error = self.allocate_io_error(IoOperation::Configure, target, reason, None, None)?;
        let call = configuration.call;
        self.finish_checked_io_result(
            CheckedIoResult::Error(error),
            call.result,
            call.error,
            call.success,
            call.failure,
        )
    }

    pub(super) fn pending_request_id(
        &self,
        local: mir::LocalId,
    ) -> Result<usize, InterpreterError> {
        let LocalValue::Class { object, class } = self.read_local_value(local)? else {
            return Err(InterpreterError::new(
                "pending request is not a borrowed request object",
            ));
        };
        self.class_value_type(object, class)?;
        if !self.pending_requests.contains_key(&object) {
            return Err(InterpreterError::new(
                "pending request has no active callback",
            ));
        }
        Ok(object)
    }

    pub(super) fn complete_pending_request(
        &mut self,
        request: mir::LocalId,
        outcome: mir::LocalId,
    ) -> Result<(), InterpreterError> {
        let request = self.pending_request_id(request)?;
        let LocalValue::PayloadEnum(outcome) =
            read_local(&self.current_frame()?.locals, outcome)?.clone()
        else {
            return Err(InterpreterError::new(
                "immediate completion has no canonical outcome",
            ));
        };
        self.pending_requests
            .get_mut(&request)
            .ok_or_else(|| InterpreterError::new("pending request has no active callback"))?
            .configure(RequestSelection::Immediate(outcome));
        Ok(())
    }

    pub(super) fn pending_request_outcome(
        &mut self,
        request: mir::LocalId,
        outcome: mir::PayloadEnumType,
    ) -> Result<CheckedIoResult, InterpreterError> {
        let request = self.pending_request_id(request)?;
        let view = self
            .pending_requests
            .get(&request)
            .ok_or_else(|| InterpreterError::new("pending request has no active callback"))?;
        let operation = if view.reading {
            IoOperation::Read
        } else {
            IoOperation::Write
        };
        let failure = if view.observing {
            view.failure
        } else {
            Some((IoErrorReason::InvalidInput, None, None))
        };
        if let Some((reason, system_code, count)) = failure {
            let target = view.target.clone();
            return Ok(CheckedIoResult::Error(self.allocate_io_error(
                operation,
                target,
                reason,
                system_code,
                count,
            )?));
        }
        let value = match &view.selection {
            Some(RequestSelection::Immediate(value)) => Some(value.clone()),
            Some(RequestSelection::Native { source, region, .. }) => {
                if matches!(source, Source::File(object) if !self.file_owners.contains_key(object))
                {
                    return Err(InterpreterError::new(
                        "pending file owner was released before retirement",
                    ));
                }
                if let Some(value) = &view.outcome {
                    if let Some(LocalValue::Scalar(mir::ScalarValue::Integer(count))) =
                        value.fields.first()
                    {
                        if count.signed_value() < 0
                            || count.unsigned_value() > region.length as u128
                        {
                            return Err(InterpreterError::new(
                                "pending completion exceeds its selected storage",
                            ));
                        }
                    }
                }
                view.outcome.clone()
            }
            None => view.outcome.clone(),
        };
        if value.as_ref().is_some_and(|value| value.ty != outcome) {
            return Err(InterpreterError::new(
                "pending completion changed its outcome type",
            ));
        }
        Ok(CheckedIoResult::Success(Some(
            LocalValue::NullablePayloadEnum { ty: outcome, value },
        )))
    }

    pub(super) fn open_owned_file(
        &mut self,
        class: crate::class_layout::ClassId,
        creation: Option<mir::LocalId>,
        mode: Option<mir::LocalId>,
    ) -> Result<CheckedIoResult, InterpreterError> {
        let path = self.pop_string()?.to_string();
        let option = |local| -> Result<usize, InterpreterError> {
            match read_local(&self.current_frame()?.locals, local)? {
                LocalValue::Scalar(mir::ScalarValue::Enum(value)) => Ok(value.case_id.index),
                _ => Err(InterpreterError::new("MIR file open has a non-enum option")),
            }
        };
        let creation = creation.map(option).transpose()?.unwrap_or(0);
        let append = mode.map(option).transpose()?.unwrap_or(0) == 1;
        let existing = self.files.contains_key(&path);
        let reason = if path.as_bytes().contains(&0) {
            Some(IoErrorReason::InvalidInput)
        } else if !existing && matches!(creation, 0 | 3) {
            Some(IoErrorReason::NotFound)
        } else if existing && creation == 2 {
            Some(IoErrorReason::Other)
        } else {
            None
        };
        if let Some(reason) = reason {
            return Ok(CheckedIoResult::Error(self.allocate_io_error(
                IoOperation::Open,
                IoTarget::File(path),
                reason,
                None,
                None,
            )?));
        }
        if !existing || matches!(creation, 3 | 4) {
            self.files.insert(path.clone(), Vec::new());
        }
        let object = self.next_object;
        self.next_object += 1;
        self.heap
            .insert(object, ObjectValue::new(class, Vec::new()));
        self.file_owners.insert(
            object,
            FileOwner {
                path,
                position: 0,
                append,
                open: true,
                pins: 0,
                retired: false,
                lock: None,
                pipe: None,
            },
        );
        self.io_trace.file_opens += 1;
        Ok(CheckedIoResult::Success(Some(LocalValue::Class {
            object,
            class,
        })))
    }

    pub(super) fn write_owned_file(
        &mut self,
        file: mir::LocalId,
        source: mir::LocalId,
        outcome: mir::PayloadEnumType,
    ) -> Result<CheckedIoResult, InterpreterError> {
        let object = self.file_owner_id(file)?;
        let region = self.region_local(source)?;
        let result = self.write_file_region(object, region, outcome)?;
        self.finish_stream_transfer(IoOperation::Write, result)
    }

    pub(super) fn write_file_region(
        &mut self,
        object: usize,
        region: ByteRegionValue,
        outcome: mir::PayloadEnumType,
    ) -> Result<FileTransferResult, InterpreterError> {
        let owner = &self.file_owners[&object];
        let path = owner.path.clone();
        if !owner.open {
            return Ok(Err(FileTransferFailure {
                target: owner.target(),
                reason: IoErrorReason::Closed,
                system_code: None,
                count: None,
            }));
        }
        if let Some(pipe) = owner.pipe {
            return self.write_child_pipe(pipe, &path, region, outcome);
        }
        let position = if owner.append {
            self.files[&path].len()
        } else {
            owner.position
        };
        let failed = self.io_faults.file_write && region.length != 0;
        let transferred = if failed {
            self.fault_transfer_length(region.length)?
        } else {
            Some(region.length)
        };
        let count = transferred.unwrap_or(0);
        if region.length != 0 {
            self.io_trace.file_writes += 1;
        }
        if count != 0 {
            let base = region.source.expect("validated present region");
            let (entries, _) = self.collection_element_storage(base)?;
            let entries = entries.borrow();
            let contents = self.files.get_mut(&path).expect("open file exists");
            let end = position
                .checked_add(count)
                .ok_or_else(|| InterpreterError::new("file position overflow"))?;
            if end > contents.len() {
                if let Err(reason) = resize_virtual_file(contents, end) {
                    return Ok(Err(FileTransferFailure {
                        target: IoTarget::File(path),
                        reason,
                        system_code: None,
                        count: Some(0),
                    }));
                }
            }
            for index in 0..count {
                let LocalValue::Scalar(mir::ScalarValue::Integer(value)) =
                    entries[base.index + index].1
                else {
                    return Err(InterpreterError::new(
                        "file write region contains a non-byte",
                    ));
                };
                contents[position + index] = value.unsigned_value() as u8;
            }
            self.file_owners
                .get_mut(&object)
                .expect("file owner exists")
                .position = end;
        }
        if failed {
            return Ok(Err(FileTransferFailure {
                target: IoTarget::File(path),
                reason: self.io_faults.failure_reason.compiler_reason(),
                system_code: self.io_faults.system_code,
                count: transferred,
            }));
        }
        let value = self.payload_enum_value(
            "Doria\\Std\\Io\\WriteOutcome",
            "Accepted",
            vec![LocalValue::Scalar(mir::ScalarValue::Integer(
                IntegerValue::from_i128(IntegerType::Int64, count as i128)
                    .expect("write count fits int"),
            ))],
        )?;
        if local_value_type(self.program, &value) != mir::Type::PayloadEnum(outcome) {
            return Err(InterpreterError::new("file write changed its outcome type"));
        }
        Ok(Ok(value))
    }

    fn file_owner_id(&self, local: mir::LocalId) -> Result<usize, InterpreterError> {
        match read_local(&self.current_frame()?.locals, local)? {
            LocalValue::Class { object, .. } if self.file_owners.contains_key(object) => {
                Ok(*object)
            }
            _ => Err(InterpreterError::new(
                "MIR file operation has no owned cursor",
            )),
        }
    }

    pub(super) fn read_owned_file(
        &mut self,
        file: mir::LocalId,
        destination: mir::LocalId,
        outcome: mir::PayloadEnumType,
    ) -> Result<CheckedIoResult, InterpreterError> {
        let object = self.file_owner_id(file)?;
        let region = self.region_local(destination)?;
        let result = self.read_file_region(object, region, outcome)?;
        self.finish_stream_transfer(IoOperation::Read, result)
    }

    pub(super) fn read_file_region(
        &mut self,
        object: usize,
        region: ByteRegionValue,
        outcome: mir::PayloadEnumType,
    ) -> Result<FileTransferResult, InterpreterError> {
        let owner = &self.file_owners[&object];
        if !owner.open {
            return Ok(Err(FileTransferFailure {
                target: owner.target(),
                reason: IoErrorReason::Closed,
                system_code: None,
                count: None,
            }));
        }
        let path = owner.path.clone();
        if let Some(pipe) = owner.pipe {
            return self.read_child_pipe(pipe, &path, region, outcome);
        }
        let position = owner.position;
        if region.length != 0 {
            self.io_trace.file_reads += 1;
        }
        let available = self.files[&path].len().saturating_sub(position);
        let requested = region.length.min(available);
        let failed = self.io_faults.file_read && region.length != 0;
        let transferred = if failed {
            self.fault_transfer_length(requested)?
        } else {
            Some(requested)
        };
        let count = transferred.unwrap_or(0);
        let base = region.source.expect("validated present region");
        let (entries, _) = self.collection_element_storage(base)?;
        let mut entries = entries.borrow_mut();
        for index in 0..count {
            let byte = self.files[&path][position + index];
            entries[base.index + index].1 = LocalValue::Scalar(mir::ScalarValue::Integer(
                IntegerValue::from_i128(IntegerType::UInt8, i128::from(byte))
                    .expect("byte fits uint8"),
            ));
        }
        drop(entries);
        self.file_owners
            .get_mut(&object)
            .expect("file owner exists")
            .position += count;
        if failed {
            return Ok(Err(FileTransferFailure {
                target: IoTarget::File(path),
                reason: self.io_faults.failure_reason.compiler_reason(),
                system_code: self.io_faults.system_code,
                count: transferred,
            }));
        }
        let case = if region.length != 0 && available == 0 {
            "EndOfStream"
        } else {
            "Data"
        };
        let fields = if case == "Data" {
            vec![LocalValue::Scalar(mir::ScalarValue::Integer(
                IntegerValue::from_i128(IntegerType::Int64, count as i128)
                    .expect("read count fits int"),
            ))]
        } else {
            Vec::new()
        };
        let value = self.payload_enum_value("Doria\\Std\\Io\\ReadOutcome", case, fields)?;
        if local_value_type(self.program, &value) != mir::Type::PayloadEnum(outcome) {
            return Err(InterpreterError::new("file read changed its outcome type"));
        }
        Ok(Ok(value))
    }

    pub(super) fn finish_stream_transfer(
        &mut self,
        operation: IoOperation,
        result: FileTransferResult,
    ) -> Result<CheckedIoResult, InterpreterError> {
        Ok(match result {
            Ok(value) => CheckedIoResult::Success(Some(value)),
            Err(failure) => CheckedIoResult::Error(self.allocate_io_error(
                operation,
                failure.target,
                failure.reason,
                failure.system_code,
                failure.count,
            )?),
        })
    }

    pub(super) fn replace_request_source(
        &mut self,
        request: usize,
        source: Source,
    ) -> Result<Option<IoErrorReason>, InterpreterError> {
        if let Some(reason) = self.readiness_source_reason(source) {
            return Ok(Some(reason));
        }
        let group = self.pending_requests[&request]
            .owner
            .ok_or_else(|| InterpreterError::new("pending source has no group owner"))?
            .0;
        match source {
            Source::File(file) => {
                let owner = self.file_owners.get_mut(&file).unwrap();
                owner.pins = owner
                    .pins
                    .checked_add(1)
                    .ok_or_else(|| InterpreterError::new("pending file pin count overflow"))?;
            }
            Source::Standard(stream) => {
                let claim = &mut self.standard_claims[usize::from(stream.runtime_tag())];
                match claim {
                    Some(owner) if *owner != StandardClaim::Group(group) => {
                        return Ok(Some(IoErrorReason::InvalidInput))
                    }
                    Some(_) => (),
                    None => {
                        *claim = Some(StandardClaim::Group(group));
                        self.transfer_groups
                            .get_mut(&group)
                            .unwrap()
                            .standard_claims[usize::from(stream.runtime_tag())] = true;
                    }
                }
            }
        }
        let target = self.readiness_source_target(source)?;
        let view = self.pending_requests.get_mut(&request).unwrap();
        view.source = Some(source);
        view.target = target;
        if let Some(old) = view.pin.replace(source) {
            self.release_request_source(old)?;
        }
        Ok(None)
    }

    pub(super) fn release_request_source(
        &mut self,
        source: Source,
    ) -> Result<(), InterpreterError> {
        match source {
            Source::File(file) => self.release_file_pin(file),
            Source::Standard(_) => Ok(()),
        }
    }

    pub(super) fn release_file_pin(&mut self, file: usize) -> Result<(), InterpreterError> {
        let owner = self
            .file_owners
            .get_mut(&file)
            .ok_or_else(|| InterpreterError::new("pending file pin was released twice"))?;
        owner.pins = owner
            .pins
            .checked_sub(1)
            .ok_or_else(|| InterpreterError::new("pending file pin count underflow"))?;
        if owner.pins == 0 && owner.retired {
            self.release_file_owner(file);
        }
        Ok(())
    }

    pub(super) fn release_file_owner(&mut self, file: usize) {
        if let Some(owner) = self.file_owners.get_mut(&file) {
            if owner.pins != 0 {
                owner.retired = true;
                return;
            }
        }
        match self.file_owners.remove(&file) {
            Some(FileOwner {
                pipe: Some(pipe), ..
            }) => self.close_child_pipe(pipe),
            Some(owner) if owner.open => self.io_trace.file_close_attempts += 1,
            _ => {}
        }
    }

    pub(super) fn control_owned_file(
        &mut self,
        file: mir::LocalId,
        control: mir::FileControlOperation,
    ) -> Result<CheckedIoResult, InterpreterError> {
        let object = self.file_owner_id(file)?;
        let owner = &self.file_owners[&object];
        if control == mir::FileControlOperation::CloseInput {
            // Half-close is idempotent: wait and collect close the input again.
            let pipe = owner
                .pipe
                .ok_or_else(|| InterpreterError::new("closeInput targets a file, not a pipe"))?;
            self.file_owners.get_mut(&object).unwrap().open = false;
            self.close_child_pipe(pipe);
            return Ok(CheckedIoResult::Success(None));
        }
        let operation = match control {
            mir::FileControlOperation::Seek { .. } => IoOperation::Seek,
            mir::FileControlOperation::SynchronizeData => IoOperation::SynchronizeData,
            mir::FileControlOperation::SynchronizeMetadata => IoOperation::SynchronizeMetadata,
            mir::FileControlOperation::SetLength { .. } => IoOperation::Resize,
            mir::FileControlOperation::Lock { .. } | mir::FileControlOperation::TryLock { .. } => {
                IoOperation::Lock
            }
            mir::FileControlOperation::Unlock | mir::FileControlOperation::AbandonLock => {
                IoOperation::Unlock
            }
            _ => IoOperation::Access,
        };
        let target = owner.target();
        let path = owner.path.clone();
        let mut reason = (!owner.open).then_some(IoErrorReason::Closed);
        let mut value = 0;
        if reason.is_none() {
            let length = self.files.get(&owner.path).map_or(0, Vec::len) as i128;
            value = match control {
                mir::FileControlOperation::SynchronizeData
                | mir::FileControlOperation::SynchronizeMetadata => 0,
                mir::FileControlOperation::Position => owner.position as i128,
                mir::FileControlOperation::Length => length,
                mir::FileControlOperation::SetLength { length } => {
                    let LocalValue::Scalar(mir::ScalarValue::Integer(length)) =
                        self.read_local_value(length)?
                    else {
                        return Err(InterpreterError::new(
                            "file resize length is not canonical int",
                        ));
                    };
                    match usize::try_from(length.signed_value()) {
                        Ok(length) => {
                            reason =
                                resize_virtual_file(self.files.get_mut(&path).unwrap(), length)
                                    .err();
                        }
                        Err(_) => reason = Some(IoErrorReason::InvalidInput),
                    }
                    0
                }
                mir::FileControlOperation::Seek { offset, origin } => {
                    let LocalValue::Scalar(mir::ScalarValue::Integer(offset)) =
                        self.read_local_value(offset)?
                    else {
                        return Err(InterpreterError::new(
                            "file seek offset is not canonical int",
                        ));
                    };
                    let LocalValue::Scalar(mir::ScalarValue::Enum(origin)) =
                        self.read_local_value(origin)?
                    else {
                        return Err(InterpreterError::new("file seek origin is not its enum"));
                    };
                    let base = match origin.case_id.index {
                        0 => 0,
                        1 => owner.position as i128,
                        2 => length,
                        _ => return Err(InterpreterError::new("file seek origin has no case")),
                    };
                    base + offset.signed_value()
                }
                mir::FileControlOperation::Lock { mode }
                | mir::FileControlOperation::TryLock { mode } => {
                    let LocalValue::Scalar(mir::ScalarValue::Enum(mode)) =
                        self.read_local_value(mode)?
                    else {
                        return Err(InterpreterError::new("file lock mode is not its enum"));
                    };
                    let exclusive = mode.case_id.index == 1;
                    // Locks belong to each open file, as with flock/LockFileEx:
                    // another open of the same path conflicts.
                    let contended = self.file_owners.iter().any(|(id, other)| {
                        *id != object
                            && other.open
                            && other.path == path
                            && other.lock.is_some_and(|held| held || exclusive)
                    });
                    let owner = self.file_owners.get_mut(&object).unwrap();
                    if owner.lock.is_some() {
                        reason = Some(IoErrorReason::InvalidInput);
                        0
                    } else if contended {
                        if matches!(control, mir::FileControlOperation::Lock { .. }) {
                            // Only this program could release it, so a native
                            // wait here never ends.
                            return Err(InterpreterError::new(
                                "blocking file lock waits on a lock held by this program",
                            ));
                        }
                        0
                    } else {
                        owner.lock = Some(exclusive);
                        1
                    }
                }
                mir::FileControlOperation::Unlock => {
                    let owner = self.file_owners.get_mut(&object).unwrap();
                    if owner.lock.take().is_none() {
                        reason = Some(IoErrorReason::InvalidInput);
                    }
                    0
                }
                mir::FileControlOperation::AbandonLock => {
                    self.file_owners.get_mut(&object).unwrap().lock = None;
                    0
                }
                mir::FileControlOperation::CloseInput => unreachable!("handled before inspection"),
            };
            if !(0..=i64::MAX as i128).contains(&value) {
                reason = Some(IoErrorReason::InvalidInput);
            } else if matches!(control, mir::FileControlOperation::Seek { .. }) {
                self.file_owners.get_mut(&object).unwrap().position = value as usize;
            }
        }
        if let Some(reason) = reason {
            return Ok(CheckedIoResult::Error(
                self.allocate_io_error(operation, target, reason, None, None)?,
            ));
        }
        match control.result_type() {
            None => Ok(CheckedIoResult::Success(None)),
            Some(mir::Type::Scalar(mir::ScalarType::Bool)) => Ok(CheckedIoResult::Success(Some(
                LocalValue::Scalar(mir::ScalarValue::Bool(value == 1)),
            ))),
            Some(_) => Ok(CheckedIoResult::Success(Some(LocalValue::Scalar(
                mir::ScalarValue::Integer(
                    IntegerValue::from_i128(IntegerType::Int64, value)
                        .expect("file control result fits int"),
                ),
            )))),
        }
    }

    pub(super) fn close_owned_file(
        &mut self,
        file: mir::LocalId,
    ) -> Result<CheckedIoResult, InterpreterError> {
        let object = self.file_owner_id(file)?;
        let owner = self
            .file_owners
            .get_mut(&object)
            .expect("file owner exists");
        if !owner.open {
            return Err(InterpreterError::new("MIR closes an already consumed file"));
        }
        // Discharge the obligation before any checked close failure is reported.
        owner.open = false;
        self.io_trace.file_close_attempts += 1;
        if self.io_faults.file_close {
            let path = owner.path.clone();
            return Ok(CheckedIoResult::Error(self.allocate_io_error(
                IoOperation::Close,
                IoTarget::File(path),
                self.io_faults.failure_reason.compiler_reason(),
                self.io_faults.system_code,
                None,
            )?));
        }
        Ok(CheckedIoResult::Success(None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_file_growth_reports_resource_failure_without_losing_existing_bytes() {
        let mut contents = b"abc".to_vec();
        assert_eq!(
            resize_virtual_file(&mut contents, usize::MAX),
            Err(IoErrorReason::ResourceExhausted)
        );
        assert_eq!(contents, b"abc");
        resize_virtual_file(&mut contents, 6).unwrap();
        assert_eq!(contents, b"abc\0\0\0");
        resize_virtual_file(&mut contents, 2).unwrap();
        assert_eq!(contents, b"ab");
    }
    fn request(reading: bool) -> RequestView {
        RequestView {
            selection: None,
            invalid: false,
            observing: false,
            outcome: None,
            failure: None,
            target: IoTarget::File("registered.bin".into()),
            reading,
            source: Some(Source::File(42)),
            bound: 4,
            owner: None,
            pin: None,
        }
    }

    fn program() -> mir::Program {
        crate::lower_source_to_mir(
            "pending-outcomes.doria",
            r#"
use Doria\Std\Io\{ReadOutcome, WriteOutcome};
function read(): ReadOutcome { return ReadOutcome::Data(0); }
function write(): WriteOutcome { return WriteOutcome::Accepted(0); }
function main(): void {}
"#,
        )
        .unwrap()
    }

    fn immediate(
        program: &mir::Program,
        reading: bool,
        case: &str,
        count: Option<i64>,
    ) -> RequestSelection {
        let name = if reading {
            "ReadOutcome"
        } else {
            "WriteOutcome"
        };
        let definition = program
            .enums
            .iter()
            .find(|definition| definition.name == format!("Doria\\Std\\Io\\{name}"))
            .unwrap();
        let case = definition
            .cases
            .iter()
            .find(|candidate| candidate.name == case)
            .unwrap();
        let fields = count
            .into_iter()
            .map(|count| {
                LocalValue::Scalar(mir::ScalarValue::Integer(
                    IntegerValue::from_i128(IntegerType::Int64, i128::from(count)).unwrap(),
                ))
            })
            .collect::<Vec<_>>();
        RequestSelection::Immediate(PayloadEnumValue {
            ty: definition.payload_type().unwrap(),
            case: case.id,
            moved_fields: vec![false; fields.len()],
            fields,
        })
    }

    #[test]
    fn pending_configuration_preserves_the_first_selection_and_latches_rejection() {
        let program = program();
        for reading in [false, true] {
            let mut view = request(reading);
            view.configure(immediate(&program, reading, "WouldBlock", None));
            assert!(!view.invalid);
            view.configure(immediate(&program, reading, "WouldBlock", None));
            assert!(view.invalid);
            let Some(RequestSelection::Immediate(outcome)) = &view.selection else {
                panic!()
            };
            assert_eq!(outcome.case.index, 1);

            let mut view = request(reading);
            view.observing = true;
            view.configure(immediate(&program, reading, "WouldBlock", None));
            view.observing = false;
            view.configure(immediate(&program, reading, "WouldBlock", None));
            assert!(view.invalid);
            assert!(
                view.selection.is_none(),
                "a later call must not revive a rejected request"
            );
        }
    }

    #[test]
    fn pending_immediate_counts_are_bounded_and_unit_outcomes_preserve_presence() {
        let program = program();
        for reading in [false, true] {
            for count in [-1, 0, 4, 5] {
                let mut view = request(reading);
                let case = if reading { "Data" } else { "Accepted" };
                view.configure(immediate(&program, reading, case, Some(count)));
                assert_eq!(
                    view.invalid,
                    !(0..=4).contains(&count),
                    "{reading}: {count}"
                );
                assert_eq!(view.selection.is_some(), !view.invalid);
                if view.invalid {
                    view.configure(immediate(&program, reading, "WouldBlock", None));
                    assert!(view.selection.is_none());
                }
            }
            let cases = if reading {
                &["WouldBlock", "EndOfStream", "TimedOut"][..]
            } else {
                &["WouldBlock"][..]
            };
            for case in cases {
                let mut view = request(reading);
                view.configure(immediate(&program, reading, case, None));
                assert!(!view.invalid, "{case}");
                assert!(view.selection.is_some(), "unit outcomes are not absence");
            }
        }
    }

    #[test]
    fn pending_file_selection_requires_the_registered_physical_owner() {
        for reading in [false, true] {
            for object in [42, 43] {
                let mut view = request(reading);
                view.configure(RequestSelection::Native {
                    source: Source::File(object),
                    prepared: false,
                    region: ByteRegionValue {
                        kind: if reading {
                            crate::types::ByteRegionKind::Writable
                        } else {
                            crate::types::ByteRegionKind::Readonly
                        },
                        nullable: false,
                        source: Some(CollectionElementPlace {
                            collection: 1,
                            generation: 0,
                            index: 0,
                            ty: mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::UInt8)),
                        }),
                        length: 4,
                    },
                });
                assert_eq!(view.invalid, object != 42);
            }
        }
    }
}

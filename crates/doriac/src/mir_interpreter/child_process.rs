//! Scripted children for the interpreter's sandbox (Decision 0110, Child
//! Process Declarations). Programs come from `MirIo::programs`; no host
//! process ever runs. A spawn finds its program by the executable path, or by
//! the script text for `Command::shell`. The pipes are ordinary file owners
//! whose bytes come from, or go to, this table.
use super::file_stream::FileOwner;
use super::*;
use crate::compiler_known_io::{IoErrorReason, IoOperation, IoTarget};
use crate::compiler_known_stream::ProcessFunction;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ChildStream {
    Input,
    Output,
    Errors,
}

/// A file owner that is one end of a child's pipe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PipeEnd {
    pub(super) process: usize,
    pub(super) stream: ChildStream,
}

impl PipeEnd {
    pub(super) fn target(self, program: &str) -> IoTarget {
        match self.stream {
            ChildStream::Input => IoTarget::ProcessInput(program.to_string()),
            ChildStream::Output => IoTarget::ProcessOutput(program.to_string()),
            ChildStream::Errors => IoTarget::ProcessErrors(program.to_string()),
        }
    }
}

pub(super) struct ChildEmulation {
    /// The program named in errors: the executable, or the platform shell.
    program: String,
    id: i64,
    /// `ChildCleanup` case index: 0 Wait, 1 Terminate, 2 Detach.
    cleanup: usize,
    output: Vec<u8>,
    output_read: usize,
    errors: Vec<u8>,
    errors_read: usize,
    echo_input: bool,
    input_open: bool,
    exit: Option<MirExitStatus>,
    killed: bool,
    /// The reaped raw status: an exit code, or a negated signal.
    status: Option<i64>,
    released: bool,
    /// The pipe objects, created on first borrow: input, output, errors.
    pipes: [Option<usize>; 3],
    captured: [Vec<u8>; 2],
    truncated: bool,
}

/// The raw status native code reports after a forced termination.
const TERMINATED: i64 = if cfg!(windows) { 1 } else { -9 };

impl ChildEmulation {
    fn exited(&self) -> bool {
        self.killed || (self.exit.is_some() && (!self.echo_input || !self.input_open))
    }

    fn raw_status(&self) -> i64 {
        if self.killed {
            return TERMINATED;
        }
        match self.exit.expect("an exited child has a status") {
            MirExitStatus::Exited(code) => code,
            MirExitStatus::Signaled(signal) => -signal,
        }
    }
}

fn shell_program() -> &'static str {
    if cfg!(windows) {
        "cmd.exe"
    } else {
        "/bin/sh"
    }
}

impl Interpreter<'_> {
    fn process_string(&self, local: mir::LocalId) -> Result<String, InterpreterError> {
        match self.read_local_value(local)? {
            LocalValue::String(value) => Ok(value.to_string()),
            _ => Err(InterpreterError::new("process request has a non-string")),
        }
    }

    fn process_strings(&self, local: mir::LocalId) -> Result<Vec<String>, InterpreterError> {
        let LocalValue::Collection(list) = self.read_local_value(local)? else {
            return Err(InterpreterError::new("process request has a non-list"));
        };
        let entries = list.entries();
        let strings = entries
            .iter()
            .map(|(_, value)| match value {
                LocalValue::String(value) => Ok(value.to_string()),
                _ => Err(InterpreterError::new(
                    "process request list has a non-string",
                )),
            })
            .collect();
        strings
    }

    fn child(&self, process: Option<mir::LocalId>) -> Result<usize, InterpreterError> {
        let local =
            process.ok_or_else(|| InterpreterError::new("process operation has no owner"))?;
        match self.read_local_value(local)? {
            LocalValue::Class { object, .. } if self.child_processes.contains_key(&object) => {
                Ok(object)
            }
            _ => Err(InterpreterError::new("process operation has no live child")),
        }
    }

    fn process_value(value: i64) -> LocalValue {
        LocalValue::Scalar(mir::ScalarValue::Integer(
            IntegerValue::from_i128(IntegerType::Int64, i128::from(value)).expect("int"),
        ))
    }

    pub(super) fn process_operation(
        &mut self,
        operation: ProcessFunction,
        process: Option<mir::LocalId>,
        arguments: &[mir::LocalId],
        class: Option<crate::class_layout::ClassId>,
        result: Option<mir::LocalId>,
    ) -> Result<CheckedIoResult, InterpreterError> {
        if operation == ProcessFunction::Launch {
            return self.launch_process(arguments, class);
        }
        let object = self.child(process)?;
        let value = match operation {
            ProcessFunction::Launch => unreachable!(),
            ProcessFunction::InputPipe
            | ProcessFunction::OutputPipe
            | ProcessFunction::ErrorsPipe => {
                let (stream, slot) = match operation {
                    ProcessFunction::InputPipe => (ChildStream::Input, 0),
                    ProcessFunction::OutputPipe => (ChildStream::Output, 1),
                    _ => (ChildStream::Errors, 2),
                };
                let class =
                    class.ok_or_else(|| InterpreterError::new("pipe borrow has no class"))?;
                let pipe = match self.child_processes[&object].pipes[slot] {
                    Some(pipe) => pipe,
                    None => {
                        let pipe = self.next_object;
                        self.next_object += 1;
                        self.heap.insert(pipe, ObjectValue::new(class, Vec::new()));
                        let program = self.child_processes[&object].program.clone();
                        self.file_owners.insert(
                            pipe,
                            FileOwner::pipe(
                                program,
                                PipeEnd {
                                    process: object,
                                    stream,
                                },
                            ),
                        );
                        self.child_processes.get_mut(&object).unwrap().pipes[slot] = Some(pipe);
                        pipe
                    }
                };
                Some(LocalValue::Class {
                    object: pipe,
                    class,
                })
            }
            ProcessFunction::Id => Some(Self::process_value(self.child_processes[&object].id)),
            ProcessFunction::Finish => {
                let LocalValue::Scalar(mir::ScalarValue::Integer(limit)) =
                    self.read_local_value(arguments[0])?
                else {
                    return Err(InterpreterError::new("collection limit is not int"));
                };
                let limit = usize::try_from(limit.signed_value()).unwrap_or(0);
                self.close_child_input(object);
                let child = self.child_processes.get_mut(&object).unwrap();
                if !child.exited() {
                    return Err(InterpreterError::new(
                        "waiting for a scripted child that exits only when terminated",
                    ));
                }
                // The pipes are drained to their end: keep at most the limit.
                for (index, (bytes, read)) in [
                    (&child.output, &mut child.output_read),
                    (&child.errors, &mut child.errors_read),
                ]
                .into_iter()
                .enumerate()
                {
                    let remaining = &bytes[*read..];
                    let kept = remaining.len().min(limit);
                    child.truncated |= kept < remaining.len();
                    child.captured[index] = remaining[..kept].to_vec();
                    *read = bytes.len();
                }
                let status = *child.status.get_or_insert(child.raw_status());
                self.close_child_pipes(object);
                Some(Self::process_value(status))
            }
            ProcessFunction::WaitUntil => {
                // The sandbox has no clock-driven children: one that has not
                // exited now never exits before any deadline.
                let child = self.child_processes.get_mut(&object).unwrap();
                let exited = child.exited();
                if exited {
                    child.status.get_or_insert(child.raw_status());
                }
                Some(LocalValue::Scalar(mir::ScalarValue::Bool(exited)))
            }
            ProcessFunction::Status => {
                let status = self.child_processes[&object].status.ok_or_else(|| {
                    InterpreterError::new("exit status read before the child was reaped")
                })?;
                Some(Self::process_value(status))
            }
            ProcessFunction::Terminate => {
                let child = self.child_processes.get_mut(&object).unwrap();
                if child.status.is_none() && !child.exited() {
                    child.killed = true;
                }
                let status = *child.status.get_or_insert(child.raw_status());
                self.close_child_pipes(object);
                Some(Self::process_value(status))
            }
            ProcessFunction::Release => {
                self.child_processes.get_mut(&object).unwrap().released = true;
                None
            }
            ProcessFunction::CapturedOutput | ProcessFunction::CapturedErrors => {
                let index = usize::from(operation == ProcessFunction::CapturedErrors);
                let bytes = std::mem::take(
                    &mut self.child_processes.get_mut(&object).unwrap().captured[index],
                );
                let collection = self.checked_io_collection_result(result)?;
                self.push_byte_collection(collection, &bytes)?;
                Some(self.pop_local_value()?)
            }
            ProcessFunction::Truncated => Some(LocalValue::Scalar(mir::ScalarValue::Bool(
                self.child_processes[&object].truncated,
            ))),
        };
        Ok(CheckedIoResult::Success(value))
    }

    fn close_child_input(&mut self, object: usize) {
        if let Some(pipe) = self.child_processes[&object].pipes[0] {
            if let Some(owner) = self.file_owners.get_mut(&pipe) {
                owner.open = false;
            }
        }
        self.child_processes.get_mut(&object).unwrap().input_open = false;
    }

    /// The owner's pipes close; later reads and writes through them fail.
    fn close_child_pipes(&mut self, object: usize) {
        self.close_child_input(object);
        for pipe in self.child_processes[&object].pipes.into_iter().flatten() {
            if let Some(owner) = self.file_owners.get_mut(&pipe) {
                owner.open = false;
            }
        }
    }

    fn launch_process(
        &mut self,
        arguments: &[mir::LocalId],
        class: Option<crate::class_layout::ClassId>,
    ) -> Result<CheckedIoResult, InterpreterError> {
        let [program, shell, list, names, values, removed, directory, cleanup] = arguments else {
            return Err(InterpreterError::new(
                "process launch has the wrong arguments",
            ));
        };
        let program = self.process_string(*program)?;
        let LocalValue::Scalar(mir::ScalarValue::Bool(shell)) = self.read_local_value(*shell)?
        else {
            return Err(InterpreterError::new(
                "process launch shell flag is not bool",
            ));
        };
        let arguments = self.process_strings(*list)?;
        let names = self.process_strings(*names)?;
        let values = self.process_strings(*values)?;
        let removed = self.process_strings(*removed)?;
        let directory = match self.read_local_value(*directory)? {
            LocalValue::String(value) => Some(value.to_string()),
            LocalValue::NullableString(value) => value.map(|value| value.to_string()),
            _ => return Err(InterpreterError::new("process directory is not ?string")),
        };
        let LocalValue::Scalar(mir::ScalarValue::Enum(cleanup)) =
            self.read_local_value(*cleanup)?
        else {
            return Err(InterpreterError::new("process cleanup is not its enum"));
        };
        let spawn = MirSpawn {
            program: program.clone(),
            shell,
            arguments,
            environment: names.into_iter().zip(values).collect(),
            removed_environment: removed,
            directory,
        };
        let name = if shell {
            shell_program().to_string()
        } else {
            program.clone()
        };
        // As natively: the shell form takes its whole command line as the
        // script, and a set's names and values pair up.
        let invalid = (spawn.shell && !spawn.arguments.is_empty())
            || spawn.program.is_empty()
            || std::iter::once(&spawn.program)
                .chain(&spawn.arguments)
                .chain(
                    spawn
                        .environment
                        .iter()
                        .flat_map(|(name, value)| [name, value]),
                )
                .chain(&spawn.removed_environment)
                .chain(&spawn.directory)
                .any(|text| text.as_bytes().contains(&0))
            || spawn
                .environment
                .iter()
                .map(|(name, _)| name)
                .chain(&spawn.removed_environment)
                .any(|name| name.is_empty() || name.contains('='));
        self.io_trace.spawns.push(spawn);
        let script = self.programs.get(&program).cloned();
        let reason = if invalid {
            Some(IoErrorReason::InvalidInput)
        } else if script.is_none() {
            Some(IoErrorReason::NotFound)
        } else {
            None
        };
        if let Some(reason) = reason {
            return Ok(CheckedIoResult::Error(self.allocate_io_error(
                IoOperation::Spawn,
                IoTarget::Process(name),
                reason,
                None,
                None,
            )?));
        }
        let script = script.unwrap();
        let class = class.ok_or_else(|| InterpreterError::new("process launch has no class"))?;
        let object = self.next_object;
        self.next_object += 1;
        self.heap
            .insert(object, ObjectValue::new(class, Vec::new()));
        let id = self.next_process_id;
        self.next_process_id += 1;
        self.child_processes.insert(
            object,
            ChildEmulation {
                program: name,
                id,
                cleanup: cleanup.case_id.index,
                output: script.output,
                output_read: 0,
                errors: script.errors,
                errors_read: 0,
                echo_input: script.echo_input,
                input_open: true,
                exit: script.exit,
                killed: false,
                status: None,
                released: false,
                pipes: [None; 3],
                captured: [Vec::new(), Vec::new()],
                truncated: false,
            },
        );
        Ok(CheckedIoResult::Success(Some(LocalValue::Class {
            object,
            class,
        })))
    }

    /// Applies the cleanup policy once, unless the child was resolved.
    pub(super) fn release_child_process(&mut self, object: usize) -> Result<(), InterpreterError> {
        if !self.child_processes.contains_key(&object) {
            return Ok(());
        }
        // The owner destroys its pipe objects before applying its policy.
        self.close_child_input(object);
        let mut child = self.child_processes.remove(&object).unwrap();
        for pipe in child.pipes.into_iter().flatten() {
            self.file_owners.remove(&pipe);
            self.heap.remove(&pipe);
        }
        if child.released || child.status.is_some() {
            return Ok(());
        }
        let policy = match child.cleanup {
            0 => "Wait",
            1 => "Terminate",
            _ => "Detach",
        };
        match child.cleanup {
            0 if !child.exited() => {
                return Err(InterpreterError::new(
                    "the Wait cleanup policy waits for a scripted child that never exits",
                ))
            }
            1 => child.killed = !child.exited(),
            _ => {}
        }
        self.io_trace.cleanups.push(policy);
        Ok(())
    }

    /// A pipe owner closed or destroyed: closing the input lets an echoing
    /// child finish.
    pub(super) fn close_child_pipe(&mut self, pipe: PipeEnd) {
        if pipe.stream == ChildStream::Input {
            if let Some(child) = self.child_processes.get_mut(&pipe.process) {
                child.input_open = false;
            }
        }
    }

    pub(super) fn read_child_pipe(
        &mut self,
        pipe: PipeEnd,
        program: &str,
        region: ByteRegionValue,
        outcome: mir::PayloadEnumType,
    ) -> Result<file_stream::FileTransferResult, InterpreterError> {
        let child = self
            .child_processes
            .get_mut(&pipe.process)
            .ok_or_else(|| InterpreterError::new("a child pipe outlived its process owner"))?;
        let exited = child.exited();
        let (bytes, read) = match pipe.stream {
            ChildStream::Output => (&child.output, &mut child.output_read),
            ChildStream::Errors => (&child.errors, &mut child.errors_read),
            ChildStream::Input => {
                return Ok(Err(file_stream::FileTransferFailure {
                    target: pipe.target(program),
                    reason: IoErrorReason::Unsupported,
                    system_code: None,
                    count: None,
                }))
            }
        };
        let available = bytes.len() - *read;
        if region.length != 0 && available == 0 && !exited {
            return Err(InterpreterError::new(
                "reading child output that only this program's own later writes could produce",
            ));
        }
        let count = region.length.min(available);
        let chunk = bytes[*read..*read + count].to_vec();
        *read += count;
        let base = region.source.expect("validated present region");
        let (entries, _) = self.collection_element_storage(base)?;
        let mut entries = entries.borrow_mut();
        for (index, byte) in chunk.into_iter().enumerate() {
            entries[base.index + index].1 = LocalValue::Scalar(mir::ScalarValue::Integer(
                IntegerValue::from_i128(IntegerType::UInt8, i128::from(byte)).expect("byte"),
            ));
        }
        drop(entries);
        let (case, fields) = if region.length != 0 && count == 0 {
            ("EndOfStream", Vec::new())
        } else {
            ("Data", vec![Self::process_value(count as i64)])
        };
        let value = self.payload_enum_value("Doria\\Std\\Io\\ReadOutcome", case, fields)?;
        if local_value_type(self.program, &value) != mir::Type::PayloadEnum(outcome) {
            return Err(InterpreterError::new("pipe read changed its outcome type"));
        }
        Ok(Ok(value))
    }

    pub(super) fn write_child_pipe(
        &mut self,
        pipe: PipeEnd,
        program: &str,
        region: ByteRegionValue,
        outcome: mir::PayloadEnumType,
    ) -> Result<file_stream::FileTransferResult, InterpreterError> {
        let child = self
            .child_processes
            .get(&pipe.process)
            .ok_or_else(|| InterpreterError::new("a child pipe outlived its process owner"))?;
        // A child that has exited closed its end: the write is a broken pipe.
        if pipe.stream != ChildStream::Input || (child.exited() && region.length != 0) {
            return Ok(Err(file_stream::FileTransferFailure {
                target: pipe.target(program),
                reason: if pipe.stream == ChildStream::Input {
                    IoErrorReason::Closed
                } else {
                    IoErrorReason::Unsupported
                },
                system_code: None,
                count: Some(0),
            }));
        }
        let echo = child.echo_input;
        if echo && region.length != 0 {
            let base = region.source.expect("validated present region");
            let (entries, _) = self.collection_element_storage(base)?;
            let bytes = entries.borrow()[base.index..base.index + region.length]
                .iter()
                .map(|(_, value)| match value {
                    LocalValue::Scalar(mir::ScalarValue::Integer(value)) => {
                        Ok(value.unsigned_value() as u8)
                    }
                    _ => Err(InterpreterError::new(
                        "pipe write region contains a non-byte",
                    )),
                })
                .collect::<Result<Vec<_>, _>>()?;
            self.child_processes
                .get_mut(&pipe.process)
                .unwrap()
                .output
                .extend_from_slice(&bytes);
        }
        let value = self.payload_enum_value(
            "Doria\\Std\\Io\\WriteOutcome",
            "Accepted",
            vec![Self::process_value(region.length as i64)],
        )?;
        if local_value_type(self.program, &value) != mir::Type::PayloadEnum(outcome) {
            return Err(InterpreterError::new("pipe write changed its outcome type"));
        }
        Ok(Ok(value))
    }
}

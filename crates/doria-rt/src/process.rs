//! Decision 0110 child processes: the runtime owner behind
//! `Doria\Std\Process\Child`. The payload owns the platform process, its three
//! pipe file objects, the cleanup policy and the reaped status. Destruction
//! closes the pipes first, then applies the policy once unless the child was
//! already resolved (waited, terminated, collected or detached).
use core::{ffi::c_void, mem, ptr, slice};

use crate::checked_io::{
    fail_named_io, IoFailureOutputs, OP_CLOSE, OP_READ, OP_SPAWN, OP_TERMINATE, OP_WAIT,
    TARGET_PROCESS, TARGET_PROCESS_ERRORS, TARGET_PROCESS_INPUT, TARGET_PROCESS_OUTPUT,
};
use crate::file_io::stream::transport::object::{
    adopt_pipe, close_file_object_handle, dr_v1_file_object_drop, dr_v1_file_object_layout,
    file_object_handle,
};
use crate::platform_io::Failure;
use crate::{DrBytesV1, DrCollectionV1, DrStackFrameV2, DrStringV1};

#[cfg(unix)]
#[path = "process/unix.rs"]
mod platform;
#[cfg(windows)]
#[path = "process/windows.rs"]
mod platform;
#[cfg(not(any(unix, windows)))]
#[path = "process/unsupported.rs"]
mod platform;

#[cfg(all(test, unix))]
#[path = "process/tests.rs"]
mod tests;

const LAUNCH: u8 = 0;
const INPUT_PIPE: u8 = 1;
const OUTPUT_PIPE: u8 = 2;
const ERRORS_PIPE: u8 = 3;
const ID: u8 = 4;
const FINISH: u8 = 5;
const WAIT_UNTIL: u8 = 6;
const STATUS: u8 = 7;
const TERMINATE: u8 = 8;
const RELEASE: u8 = 9;
const CAPTURED_OUTPUT: u8 = 10;
const CAPTURED_ERRORS: u8 = 11;
const TRUNCATED: u8 = 12;

const CLEANUP_WAIT: u64 = 0;
const CLEANUP_TERMINATE: u64 = 1;

/// The bytes `collect` keeps from one stream: at most its limit, grown by
/// doubling, so memory stays bounded however much the child writes.
pub(crate) struct Capture {
    data: *mut u8,
    length: usize,
    capacity: usize,
}

impl Capture {
    pub(crate) const fn new() -> Self {
        Self {
            data: ptr::null_mut(),
            length: 0,
            capacity: 0,
        }
    }

    /// Keeps what fits under `limit`; returns false when bytes were dropped.
    pub(crate) unsafe fn append(
        &mut self,
        frame: *const DrStackFrameV2,
        bytes: &[u8],
        limit: usize,
    ) -> bool {
        let kept = bytes.len().min(limit.saturating_sub(self.length));
        if kept != 0 {
            let needed = self.length + kept;
            if needed > self.capacity {
                let capacity = needed
                    .max(self.capacity.saturating_mul(2))
                    .max(4096)
                    .min(limit);
                let grown = crate::allocate(capacity);
                if grown.is_null() {
                    crate::panic_catalogued(frame, b"P1302");
                }
                if self.length != 0 {
                    ptr::copy_nonoverlapping(self.data, grown, self.length);
                }
                if !self.data.is_null() {
                    crate::deallocate(self.data);
                }
                self.data = grown;
                self.capacity = capacity;
            }
            ptr::copy_nonoverlapping(bytes.as_ptr(), self.data.add(self.length), kept);
            self.length = needed;
        }
        kept == bytes.len()
    }

    /// Moves the captured bytes into a new `Bytes`, leaving this capture empty.
    unsafe fn take_bytes(&mut self) -> *mut DrBytesV1 {
        let data = mem::replace(&mut self.data, ptr::null_mut());
        let length = mem::replace(&mut self.length, 0);
        self.capacity = 0;
        crate::bytes::from_owned(data, length)
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        if !self.data.is_null() {
            unsafe { crate::deallocate(self.data) };
        }
    }
}

/// A validated launch request: borrowed byte strings, no allocation yet.
pub(crate) struct Request<'a> {
    pub(crate) program: &'a [u8],
    pub(crate) shell: bool,
    pub(crate) arguments: Strings<'a>,
    pub(crate) names: Strings<'a>,
    pub(crate) values: Strings<'a>,
    pub(crate) removed: Strings<'a>,
    pub(crate) directory: Option<&'a [u8]>,
}

/// A borrowed `List<string>` read in place.
#[derive(Clone, Copy)]
pub(crate) struct Strings<'a> {
    frame: *const DrStackFrameV2,
    list: *const DrCollectionV1,
    marker: core::marker::PhantomData<&'a ()>,
}

impl<'a> Strings<'a> {
    pub(crate) fn len(self) -> usize {
        unsafe { crate::collection::length(self.list) }
    }

    pub(crate) fn get(self, index: usize) -> &'a [u8] {
        unsafe {
            let value = crate::collection::value_at(self.frame, self.list, index);
            string_slice(value as usize as *const DrStringV1)
        }
    }

    pub(crate) fn iter(self) -> impl Iterator<Item = &'a [u8]> + Clone {
        (0..self.len()).map(move |index| self.get(index))
    }
}

unsafe fn string_slice<'a>(string: *const DrStringV1) -> &'a [u8] {
    match (*string).byte_length {
        0 => &[],
        length => slice::from_raw_parts(crate::string_bytes(string), length),
    }
}

impl Request<'_> {
    /// OS strings cannot hold NUL; a variable name cannot be empty or hold
    /// `=`; the shell form takes its whole command line as the script.
    fn validate(&self) -> Result<(), Failure> {
        let no_nul = |text: &[u8]| !text.contains(&0);
        let name = |text: &[u8]| no_nul(text) && !text.is_empty() && !text.contains(&b'=');
        if no_nul(self.program)
            && !self.program.is_empty()
            && self.arguments.iter().all(no_nul)
            && self.names.iter().all(name)
            && self.values.iter().all(no_nul)
            && self.removed.iter().all(name)
            && self.directory.is_none_or(no_nul)
            && !(self.shell && self.arguments.len() != 0)
            && self.names.len() == self.values.len()
        {
            Ok(())
        } else {
            Err(Failure::invalid_input())
        }
    }
}

pub(crate) struct ChildObject {
    process: platform::Process,
    cleanup: u64,
    status: Option<i64>,
    released: bool,
    /// The program named by failures: the executable, or the shell.
    program: *mut DrStringV1,
    /// Input, output and error-output pipe file objects, owned here.
    pipes: [*mut c_void; 3],
    captured: [Capture; 2],
    truncated: bool,
}

const PIPE_TARGETS: [u8; 3] = [
    TARGET_PROCESS_INPUT,
    TARGET_PROCESS_OUTPUT,
    TARGET_PROCESS_ERRORS,
];

impl ChildObject {
    unsafe fn close_pipes(&mut self) -> Result<(), (usize, Failure)> {
        let mut first = Ok(());
        for (index, pipe) in self.pipes.iter().enumerate() {
            if let Err(failure) = close_file_object_handle(*pipe) {
                if first.is_ok() {
                    first = Err((index, failure));
                }
            }
        }
        first
    }

    unsafe fn reap(&mut self) -> Result<i64, Failure> {
        if let Some(status) = self.status {
            return Ok(status);
        }
        let status = platform::wait(&mut self.process)?;
        self.status = Some(status);
        Ok(status)
    }
}

/// The shared checked transport for every `Child` intrinsic.
///
/// Operation tags follow `ProcessFunction::tag`. `operands` holds the
/// operation's words: for launch, the program string, the shell flag, the four
/// request lists, the directory string or 0, and the cleanup tag (0 Wait,
/// 1 Terminate, 2 Detach); for finish, the byte limit; for a deadline wait,
/// the deadline's two words. The result is the new `Child`, the home of a
/// borrowed pipe object, an int, a 0/1 bool or an owned `Bytes`. Returns 0 or 1 for IoError,
/// whose outputs follow the checked-I/O transport and name the program.
///
/// # Safety
/// `child` is a live `Child` payload for every operation but launch; the
/// operand words have the types above; outputs are writable and disjoint.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_checked_process(
    frame: *const DrStackFrameV2,
    operation: u8,
    child: *mut c_void,
    operands: *const u64,
    operand_count: usize,
    result: *mut u64,
    message: *mut *mut DrStringV1,
    error_path: *mut *mut DrStringV1,
    system_code: *mut i64,
    transferred: *mut usize,
    meta: *mut u64,
) -> u8 {
    *result = 0;
    *message = ptr::null_mut();
    *error_path = ptr::null_mut();
    *system_code = 0;
    *transferred = 0;
    *meta = 0;
    let outputs = IoFailureOutputs {
        message,
        path: error_path,
        system_code,
        transferred_byte_count: transferred,
        meta,
    };
    let operands = match operand_count {
        0 => &[][..],
        count => slice::from_raw_parts(operands, count),
    };
    if operation == LAUNCH {
        return launch(frame, operands, result, outputs);
    }
    let child = &mut *child.cast::<ChildObject>();
    let program = child.program;
    let fail = |operation: u8, target: u8, failure: Failure| -> u8 {
        fail_named_io(frame, operation, target, failure, program, outputs)
    };
    match operation {
        // The result is the pipe's home: the owner's slot holding the object,
        // stable for the child's life. Callers load the object through it.
        INPUT_PIPE | OUTPUT_PIPE | ERRORS_PIPE => {
            *result = ptr::addr_of_mut!(child.pipes[usize::from(operation - INPUT_PIPE)]) as u64;
        }
        ID => *result = platform::id(&child.process) as u64,
        FINISH => {
            let limit = usize::try_from(operands[0] as i64).unwrap_or(0);
            if child.status.is_none() {
                if let Err(failure) = close_file_object_handle(child.pipes[0]) {
                    return fail(OP_CLOSE, TARGET_PROCESS_INPUT, failure);
                }
                let handles = [
                    file_object_handle(child.pipes[1]),
                    file_object_handle(child.pipes[2]),
                ];
                // One sink per stream: Windows drains the error output on a
                // helper thread, so the sinks share no mutable state.
                let [output, errors] = &mut child.captured;
                let mut dropped = [false, false];
                let [output_dropped, errors_dropped] = &mut dropped;
                let drained = platform::drain(
                    handles,
                    &mut |bytes: &[u8]| *output_dropped |= !output.append(frame, bytes, limit),
                    &mut |bytes: &[u8]| *errors_dropped |= !errors.append(frame, bytes, limit),
                );
                child.truncated |= dropped[0] || dropped[1];
                if let Err((index, failure)) = drained {
                    return fail(OP_READ, PIPE_TARGETS[index + 1], failure);
                }
                if let Err((index, failure)) = child.close_pipes() {
                    return fail(OP_CLOSE, PIPE_TARGETS[index], failure);
                }
            }
            match child.reap() {
                Ok(status) => *result = status as u64,
                Err(failure) => return fail(OP_WAIT, TARGET_PROCESS, failure),
            }
        }
        WAIT_UNTIL => {
            if child.status.is_none() {
                let budget = crate::wait_budget::DrWaitBudgetV1::from_deadline(
                    crate::deadline::DrDeadlineV1::from_parts(operands[0], operands[1] as i64),
                );
                match platform::wait_until(&mut child.process, &budget) {
                    Ok(status) => child.status = status,
                    Err(failure) => return fail(OP_WAIT, TARGET_PROCESS, failure),
                }
            }
            *result = u64::from(child.status.is_some());
        }
        STATUS => match child.status {
            Some(status) => *result = status as u64,
            None => crate::runtime_invariant_panic(),
        },
        TERMINATE => {
            if child.status.is_none() {
                if let Err(failure) = platform::kill(&mut child.process) {
                    return fail(OP_TERMINATE, TARGET_PROCESS, failure);
                }
                // Never drain after a kill: a grandchild may hold the pipes.
                if let Err((index, failure)) = child.close_pipes() {
                    return fail(OP_CLOSE, PIPE_TARGETS[index], failure);
                }
            }
            match child.reap() {
                Ok(status) => *result = status as u64,
                Err(failure) => return fail(OP_WAIT, TARGET_PROCESS, failure),
            }
        }
        RELEASE => child.released = true,
        CAPTURED_OUTPUT | CAPTURED_ERRORS => {
            *result = child.captured[usize::from(operation - CAPTURED_OUTPUT)].take_bytes() as u64;
        }
        TRUNCATED => *result = u64::from(child.truncated),
        _ => crate::runtime_invariant_panic(),
    }
    0
}

unsafe fn launch(
    frame: *const DrStackFrameV2,
    operands: &[u64],
    result: *mut u64,
    outputs: IoFailureOutputs,
) -> u8 {
    let [program, shell, arguments, names, values, removed, directory, cleanup] = operands else {
        crate::runtime_invariant_panic();
    };
    let strings = |list: u64| Strings {
        frame,
        list: list as usize as *const DrCollectionV1,
        marker: core::marker::PhantomData,
    };
    let program_string = *program as usize as *mut DrStringV1;
    let request = Request {
        program: string_slice(program_string),
        shell: *shell != 0,
        arguments: strings(*arguments),
        names: strings(*names),
        values: strings(*values),
        removed: strings(*removed),
        directory: (*directory != 0)
            .then(|| string_slice(*directory as usize as *const DrStringV1)),
    };
    // Failures and pipes name the program actually launched.
    let name = if request.shell {
        let shell = platform::SHELL;
        let name = crate::allocate_string_with_frame(frame, shell.len());
        ptr::copy_nonoverlapping(shell.as_ptr(), crate::string_bytes_mut(name), shell.len());
        name
    } else {
        crate::dr_v1_string_retain(program_string)
    };
    let spawned = request
        .validate()
        .and_then(|()| platform::spawn(frame, &request));
    let (process, handles) = match spawned {
        Ok(spawned) => spawned,
        Err(failure) => {
            let code = fail_named_io(frame, OP_SPAWN, TARGET_PROCESS, failure, name, outputs);
            crate::dr_v1_string_release(name);
            return code;
        }
    };
    let mut size = 0;
    let mut alignment = 0;
    dr_v1_file_object_layout(&mut size, &mut alignment);
    let mut pipes = [ptr::null_mut(); 3];
    for ((pipe, handle), target) in pipes.iter_mut().zip(handles).zip(PIPE_TARGETS) {
        let storage = crate::dr_v2_class_allocate(frame, size, alignment).cast::<c_void>();
        if !adopt_pipe(handle, target, name, storage) {
            crate::panic_catalogued(frame, b"P1601");
        }
        *pipe = storage;
    }
    let child = crate::dr_v2_class_allocate(
        frame,
        mem::size_of::<ChildObject>(),
        mem::align_of::<ChildObject>(),
    )
    .cast::<ChildObject>();
    child.write(ChildObject {
        process,
        cleanup: *cleanup,
        status: None,
        released: false,
        program: name,
        pipes,
        captured: [Capture::new(), Capture::new()],
        truncated: false,
    });
    *result = child as u64;
    0
}

/// Closes the pipes, then applies the cleanup policy once unless the child
/// was resolved, and frees the payload. Cleanup is best-effort: failures are
/// not observable here, which is why explicit resolution exists.
/// # Safety
/// `child` is a live, unborrowed `Child` payload; it is freed by this call.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_child_process_drop(
    frame: *const DrStackFrameV2,
    child: *mut c_void,
) {
    let payload = child.cast::<ChildObject>();
    let value = &mut *payload;
    let _ = value.close_pipes();
    for pipe in value.pipes {
        dr_v1_file_object_drop(frame, pipe);
        crate::dr_v1_class_free(pipe.cast());
    }
    if !value.released && value.status.is_none() {
        match value.cleanup {
            CLEANUP_WAIT => {
                let _ = value.reap();
            }
            CLEANUP_TERMINATE => {
                let _ = platform::kill(&mut value.process);
                let _ = value.reap();
            }
            _ => {}
        }
    }
    crate::dr_v1_string_release(value.program);
    ptr::drop_in_place(payload);
    crate::dr_v1_class_free(payload.cast());
}

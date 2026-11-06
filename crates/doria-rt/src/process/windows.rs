//! Windows child processes: CreateProcessW with three anonymous pipes whose
//! child ends alone are inherited (PROC_THREAD_ATTRIBUTE_HANDLE_LIST), the
//! program found on the child's own PATH, MSVC command-line quoting, and a
//! drain that reads the error output on a helper thread so neither pipe can
//! fill and stall the child.
use core::{ffi::c_void, mem, ptr};

use super::Request;
use crate::native_handle::{NativeHandle, OwnedHandle};
use crate::platform_io::{last_failure, Failure};
use crate::DrStackFrameV2;

pub(super) const SHELL: &[u8] = b"cmd.exe";

type Handle = *mut c_void;

const INFINITE: u32 = 0xFFFF_FFFF;
const WAIT_OBJECT_0: u32 = 0;
const WAIT_TIMEOUT: u32 = 258;
const STILL_ACTIVE: u32 = 259;
const HANDLE_FLAG_INHERIT: u32 = 1;
const STARTF_USESTDHANDLES: u32 = 0x100;
const CREATE_UNICODE_ENVIRONMENT: u32 = 0x400;
const EXTENDED_STARTUPINFO_PRESENT: u32 = 0x8_0000;
const PROC_THREAD_ATTRIBUTE_HANDLE_LIST: usize = 0x2_0002;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const INVALID_FILE_ATTRIBUTES: u32 = 0xFFFF_FFFF;
const ERROR_BROKEN_PIPE: u32 = 109;
const ERROR_FILE_NOT_FOUND: i64 = 2;

#[repr(C)]
struct SecurityAttributes {
    length: u32,
    descriptor: *mut c_void,
    inherit: i32,
}

#[repr(C)]
struct StartupInfo {
    cb: u32,
    reserved: *mut u16,
    desktop: *mut u16,
    title: *mut u16,
    x: u32,
    y: u32,
    x_size: u32,
    y_size: u32,
    x_count_chars: u32,
    y_count_chars: u32,
    fill_attribute: u32,
    flags: u32,
    show_window: u16,
    reserved2_count: u16,
    reserved2: *mut u8,
    input: Handle,
    output: Handle,
    error: Handle,
}

#[repr(C)]
struct StartupInfoEx {
    startup: StartupInfo,
    attributes: *mut c_void,
}

#[repr(C)]
struct ProcessInformation {
    process: Handle,
    thread: Handle,
    process_id: u32,
    thread_id: u32,
}

unsafe extern "system" {
    fn CreatePipe(
        read: *mut Handle,
        write: *mut Handle,
        attributes: *const SecurityAttributes,
        size: u32,
    ) -> i32;
    fn SetHandleInformation(handle: Handle, mask: u32, flags: u32) -> i32;
    fn CreateProcessW(
        application: *const u16,
        command_line: *mut u16,
        process_attributes: *const SecurityAttributes,
        thread_attributes: *const SecurityAttributes,
        inherit: i32,
        flags: u32,
        environment: *const c_void,
        directory: *const u16,
        startup: *const StartupInfo,
        information: *mut ProcessInformation,
    ) -> i32;
    fn InitializeProcThreadAttributeList(
        list: *mut c_void,
        count: u32,
        flags: u32,
        size: *mut usize,
    ) -> i32;
    fn UpdateProcThreadAttribute(
        list: *mut c_void,
        flags: u32,
        attribute: usize,
        value: *const c_void,
        size: usize,
        previous: *mut c_void,
        returned: *mut usize,
    ) -> i32;
    fn DeleteProcThreadAttributeList(list: *mut c_void);
    fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
    fn GetExitCodeProcess(process: Handle, code: *mut u32) -> i32;
    fn TerminateProcess(process: Handle, code: u32) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
    fn GetEnvironmentStringsW() -> *mut u16;
    fn FreeEnvironmentStringsW(block: *mut u16) -> i32;
    fn GetEnvironmentVariableW(name: *const u16, buffer: *mut u16, size: u32) -> u32;
    fn GetFileAttributesW(path: *const u16) -> u32;
    fn ReadFile(
        file: Handle,
        buffer: *mut c_void,
        length: u32,
        read: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
    // Matches device_io::helper's declaration of the same import.
    fn CreateThread(
        attributes: *mut c_void,
        stack: usize,
        start: extern "system" fn(*mut c_void) -> u32,
        parameter: *mut c_void,
        flags: u32,
        id: *mut u32,
    ) -> Handle;
    fn GetLastError() -> u32;
}

pub(crate) struct Process {
    handle: Handle,
    id: u32,
}

impl Drop for Process {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.handle) };
    }
}

pub(super) fn id(process: &Process) -> i64 {
    i64::from(process.id)
}

unsafe fn exit_code(process: &Process) -> Result<u32, Failure> {
    let mut code = 0;
    if GetExitCodeProcess(process.handle, &mut code) == 0 {
        return Err(last_failure());
    }
    Ok(code)
}

pub(super) unsafe fn wait(process: &mut Process) -> Result<i64, Failure> {
    if WaitForSingleObject(process.handle, INFINITE) != WAIT_OBJECT_0 {
        return Err(last_failure());
    }
    Ok(i64::from(exit_code(process)?))
}

pub(super) unsafe fn wait_until(
    process: &mut Process,
    budget: &crate::wait_budget::DrWaitBudgetV1,
) -> Result<Option<i64>, Failure> {
    loop {
        let (kind, milliseconds) = budget.remaining(
            1_000_000,
            u64::from(INFINITE - 1),
            crate::monotonic_clock::now,
        )?;
        let timeout = match kind {
            1 => INFINITE,
            3 => 0,
            _ => milliseconds as u32,
        };
        match WaitForSingleObject(process.handle, timeout) {
            WAIT_OBJECT_0 => return Ok(Some(i64::from(exit_code(process)?))),
            WAIT_TIMEOUT if kind == 3 || kind == 0 => return Ok(None),
            WAIT_TIMEOUT => continue,
            _ => return Err(last_failure()),
        }
    }
}

/// TerminateProcess with exit code 1; an already exited child is resolved.
pub(super) unsafe fn kill(process: &mut Process) -> Result<(), Failure> {
    if TerminateProcess(process.handle, 1) != 0 {
        return Ok(());
    }
    let failure = last_failure();
    match exit_code(process) {
        Ok(code) if code != STILL_ACTIVE => Ok(()),
        _ => Err(failure),
    }
}

struct ErrorsJob {
    handle: NativeHandle,
    sink: *mut dyn FnMut(&[u8]),
    failure: Option<Failure>,
}

unsafe fn read_to_end(handle: NativeHandle, sink: &mut dyn FnMut(&[u8])) -> Option<Failure> {
    let mut chunk = [0u8; 16384];
    loop {
        let mut count = 0;
        if ReadFile(
            handle,
            chunk.as_mut_ptr().cast(),
            chunk.len() as u32,
            &mut count,
            ptr::null_mut(),
        ) == 0
        {
            let code = GetLastError();
            return (code != ERROR_BROKEN_PIPE).then(|| Failure::from_system_code(i64::from(code)));
        }
        if count == 0 {
            return None;
        }
        sink(&chunk[..count as usize]);
    }
}

extern "system" fn drain_errors(job: *mut c_void) -> u32 {
    // The job and its sink outlive the thread: drain joins before returning.
    unsafe {
        let job = &mut *job.cast::<ErrorsJob>();
        job.failure = read_to_end(job.handle, &mut *job.sink);
    }
    0
}

/// Reads both outputs to their end concurrently: the error output on a
/// helper thread, the output here. Each sink receives its stream's chunks.
pub(super) unsafe fn drain<'a>(
    handles: [Option<NativeHandle>; 2],
    output: &'a mut dyn FnMut(&[u8]),
    errors: &'a mut dyn FnMut(&[u8]),
) -> Result<(), (usize, Failure)> {
    let mut job = ErrorsJob {
        handle: handles[1].unwrap_or(ptr::null_mut()),
        sink: mem::transmute::<*mut (dyn FnMut(&[u8]) + 'a), *mut dyn FnMut(&[u8])>(errors),
        failure: None,
    };
    let thread = if handles[1].is_some() {
        let thread = CreateThread(
            ptr::null_mut(),
            0,
            drain_errors,
            ptr::addr_of_mut!(job).cast(),
            0,
            ptr::null_mut(),
        );
        if thread.is_null() {
            return Err((1, last_failure()));
        }
        Some(thread)
    } else {
        None
    };
    let output_failure = handles[0].and_then(|handle| read_to_end(handle, output));
    if let Some(thread) = thread {
        WaitForSingleObject(thread, INFINITE);
        CloseHandle(thread);
    }
    if let Some(failure) = output_failure {
        return Err((0, failure));
    }
    match job.failure {
        Some(failure) => Err((1, failure)),
        None => Ok(()),
    }
}

/// A growable NUL-terminable UTF-16 buffer.
struct Wide {
    data: *mut u16,
    length: usize,
    capacity: usize,
    frame: *const DrStackFrameV2,
}

impl Wide {
    fn new(frame: *const DrStackFrameV2) -> Self {
        Self {
            data: ptr::null_mut(),
            length: 0,
            capacity: 0,
            frame,
        }
    }

    unsafe fn push(&mut self, unit: u16) {
        if self.length == self.capacity {
            let capacity = (self.capacity * 2).max(64);
            let grown = crate::allocate(capacity * 2).cast::<u16>();
            if grown.is_null() {
                crate::panic_catalogued(self.frame, b"P1302");
            }
            if self.length != 0 {
                ptr::copy_nonoverlapping(self.data, grown, self.length);
            }
            if !self.data.is_null() {
                crate::deallocate(self.data.cast());
            }
            self.data = grown;
            self.capacity = capacity;
        }
        *self.data.add(self.length) = unit;
        self.length += 1;
    }

    unsafe fn text(&mut self, text: &[u8]) {
        for unit in core::str::from_utf8_unchecked(text).encode_utf16() {
            self.push(unit);
        }
    }

    unsafe fn units(&mut self, units: &[u16]) {
        for unit in units {
            self.push(*unit);
        }
    }

    fn as_slice(&self) -> &[u16] {
        if self.data.is_null() {
            &[]
        } else {
            unsafe { core::slice::from_raw_parts(self.data, self.length) }
        }
    }
}

impl Drop for Wide {
    fn drop(&mut self) {
        if !self.data.is_null() {
            unsafe { crate::deallocate(self.data.cast()) };
        }
    }
}

fn fold(unit: u16) -> u16 {
    if (b'a' as u16..=b'z' as u16).contains(&unit) {
        unit - 32
    } else {
        unit
    }
}

/// Variable names compare case-insensitively on Windows.
fn same_name(left: &[u16], right: &[u16]) -> bool {
    left.len() == right.len() && left.iter().zip(right).all(|(a, b)| fold(*a) == fold(*b))
}

fn wide_name(entry: &[u16]) -> &[u16] {
    // A leading '=' belongs to the name (per-drive directory entries).
    let start = usize::from(entry.first() == Some(&(b'=' as u16)));
    match entry[start..].iter().position(|unit| *unit == b'=' as u16) {
        Some(end) => &entry[..start + end],
        None => entry,
    }
}

unsafe fn wide_of(frame: *const DrStackFrameV2, text: &[u8]) -> Wide {
    let mut wide = Wide::new(frame);
    wide.text(text);
    wide
}

/// The child's environment block, sorted by folded name, or None to inherit.
unsafe fn environment(frame: *const DrStackFrameV2, request: &Request<'_>) -> Option<Wide> {
    if request.names.len() == 0 && request.removed.len() == 0 {
        return None;
    }
    let edited = |name: &[u16]| {
        request
            .names
            .iter()
            .chain(request.removed.iter())
            .any(|edit| same_name(wide_of(frame, edit).as_slice(), name))
    };
    // Surviving and new entries, NUL-separated, then sorted by folded name.
    let mut buffer = Wide::new(frame);
    let block = GetEnvironmentStringsW();
    if !block.is_null() {
        let mut cursor = block;
        while *cursor != 0 {
            let mut length = 0;
            while *cursor.add(length) != 0 {
                length += 1;
            }
            let entry = core::slice::from_raw_parts(cursor, length);
            if !edited(wide_name(entry)) {
                buffer.units(entry);
                buffer.push(0);
            }
            cursor = cursor.add(length + 1);
        }
        FreeEnvironmentStringsW(block);
    }
    for index in 0..request.names.len() {
        buffer.text(request.names.get(index));
        buffer.push(b'=' as u16);
        buffer.text(request.values.get(index));
        buffer.push(0);
    }
    let all = buffer.as_slice();
    let count = all.iter().filter(|unit| **unit == 0).count();
    let starts = crate::allocate(count.max(1) * mem::size_of::<usize>()).cast::<usize>();
    if starts.is_null() {
        crate::panic_catalogued(frame, b"P1302");
    }
    let starts = core::slice::from_raw_parts_mut(starts, count);
    let mut next = 0;
    for start in starts.iter_mut() {
        *start = next;
        next += all[next..].iter().position(|unit| *unit == 0).unwrap() + 1;
    }
    let entry = |start: usize| {
        let end = start + all[start..].iter().position(|unit| *unit == 0).unwrap();
        &all[start..end]
    };
    // Insertion sort: environments are small and the block must be ordered.
    for position in 1..count {
        let mut cursor = position;
        while cursor > 0 {
            let left = wide_name(entry(starts[cursor - 1]))
                .iter()
                .map(|unit| fold(*unit));
            let right = wide_name(entry(starts[cursor]))
                .iter()
                .map(|unit| fold(*unit));
            if left.cmp(right) != core::cmp::Ordering::Greater {
                break;
            }
            starts.swap(cursor - 1, cursor);
            cursor -= 1;
        }
    }
    let mut sorted = Wide::new(frame);
    for start in starts.iter() {
        sorted.units(entry(*start));
        sorted.push(0);
    }
    sorted.push(0);
    crate::deallocate(starts.as_mut_ptr().cast());
    Some(sorted)
}

/// MSVC C runtime quoting, so the child's argv equals the request.
unsafe fn quote(command: &mut Wide, argument: &[u8]) {
    let wide = wide_of(command.frame, argument);
    let units = wide.as_slice();
    let plain = !units.is_empty()
        && !units
            .iter()
            .any(|unit| matches!(*unit, 0x20 | 0x09 | 0x0A | 0x0B | 0x22));
    if plain {
        command.units(units);
        return;
    }
    command.push(b'"' as u16);
    let mut backslashes = 0;
    for unit in units {
        if *unit == b'\\' as u16 {
            backslashes += 1;
            continue;
        }
        let repeat = if *unit == b'"' as u16 {
            backslashes * 2 + 1
        } else {
            backslashes
        };
        for _ in 0..repeat {
            command.push(b'\\' as u16);
        }
        backslashes = 0;
        command.push(*unit);
    }
    for _ in 0..backslashes * 2 {
        command.push(b'\\' as u16);
    }
    command.push(b'"' as u16);
}

unsafe fn is_file(path: &Wide) -> bool {
    let mut terminated = Wide::new(path.frame);
    terminated.units(path.as_slice());
    terminated.push(0);
    let attributes = GetFileAttributesW(terminated.data);
    attributes != INVALID_FILE_ATTRIBUTES && attributes & FILE_ATTRIBUTE_DIRECTORY == 0
}

/// A name with a separator is used as written; otherwise the first file on the
/// child's PATH, trying the name and then the name with `.exe`.
unsafe fn resolve(frame: *const DrStackFrameV2, request: &Request<'_>) -> Result<Wide, Failure> {
    let program = if request.shell {
        SHELL
    } else {
        request.program
    };
    let separated = program
        .iter()
        .any(|byte| matches!(*byte, b'\\' | b'/' | b':'));
    let has_extension = program
        .iter()
        .rposition(|byte| *byte == b'.')
        .is_some_and(|dot| {
            !program[dot..]
                .iter()
                .any(|byte| matches!(*byte, b'\\' | b'/'))
        });
    let candidates = |base: &Wide| {
        let mut plain = Wide::new(frame);
        plain.units(base.as_slice());
        let mut exe = Wide::new(frame);
        exe.units(base.as_slice());
        exe.text(b".exe");
        [Some(plain), (!has_extension).then_some(exe)]
    };
    if separated {
        for candidate in candidates(&wide_of(frame, program)).into_iter().flatten() {
            if is_file(&candidate) {
                return Ok(candidate);
            }
        }
        return Err(Failure::from_system_code(ERROR_FILE_NOT_FOUND));
    }
    let path = if let Some(index) = request.names.iter().position(|name| {
        same_name(
            wide_of(frame, name).as_slice(),
            wide_of(frame, b"PATH").as_slice(),
        )
    }) {
        wide_of(frame, request.values.get(index))
    } else if request.removed.iter().any(|name| {
        same_name(
            wide_of(frame, name).as_slice(),
            wide_of(frame, b"PATH").as_slice(),
        )
    }) {
        Wide::new(frame)
    } else {
        let name = [b'P' as u16, b'A' as u16, b'T' as u16, b'H' as u16, 0];
        let mut value = Wide::new(frame);
        let size = GetEnvironmentVariableW(name.as_ptr(), ptr::null_mut(), 0);
        for _ in 0..size {
            value.push(0);
        }
        if size != 0 {
            let written = GetEnvironmentVariableW(name.as_ptr(), value.data, size);
            value.length = written as usize;
        }
        value
    };
    for directory in path.as_slice().split(|unit| *unit == b';' as u16) {
        if directory.is_empty() {
            continue;
        }
        let mut base = Wide::new(frame);
        base.units(directory);
        if directory.last() != Some(&(b'\\' as u16)) {
            base.push(b'\\' as u16);
        }
        base.text(program);
        for candidate in candidates(&base).into_iter().flatten() {
            if is_file(&candidate) {
                return Ok(candidate);
            }
        }
    }
    Err(Failure::from_system_code(ERROR_FILE_NOT_FOUND))
}

unsafe fn pipe(child_reads: bool) -> Result<(OwnedHandle, OwnedHandle), Failure> {
    let attributes = SecurityAttributes {
        length: mem::size_of::<SecurityAttributes>() as u32,
        descriptor: ptr::null_mut(),
        inherit: 1,
    };
    let (mut read, mut write) = (ptr::null_mut(), ptr::null_mut());
    if CreatePipe(&mut read, &mut write, &attributes, 0) == 0 {
        return Err(last_failure());
    }
    let (read, write) = (OwnedHandle::from_raw(read), OwnedHandle::from_raw(write));
    // (parent end, child end): only the child's end stays inheritable.
    let (parent, child) = if child_reads {
        (write, read)
    } else {
        (read, write)
    };
    if SetHandleInformation(parent.get(), HANDLE_FLAG_INHERIT, 0) == 0 {
        return Err(last_failure());
    }
    Ok((parent, child))
}

/// Spawns the child with its own pipe ends as standard handles and returns
/// the parent's ends: input, output, error output.
pub(super) unsafe fn spawn(
    frame: *const DrStackFrameV2,
    request: &Request<'_>,
) -> Result<(Process, [OwnedHandle; 3]), Failure> {
    let mut application = resolve(frame, request)?;
    application.push(0);
    let mut command = Wide::new(frame);
    if request.shell {
        // /s keeps the script verbatim inside the outer quotes.
        command.text(b"cmd.exe /d /s /c \"");
        command.text(request.program);
        command.push(b'"' as u16);
    } else {
        quote(&mut command, request.program);
        for argument in request.arguments.iter() {
            command.push(b' ' as u16);
            quote(&mut command, argument);
        }
    }
    command.push(0);
    let environment = environment(frame, request);
    let directory = request.directory.map(|directory| {
        let mut wide = wide_of(frame, directory);
        wide.push(0);
        wide
    });
    let (input, child_input) = pipe(true)?;
    let (output, child_output) = pipe(false)?;
    let (errors, child_errors) = pipe(false)?;
    let inherited = [child_input.get(), child_output.get(), child_errors.get()];
    let mut size = 0;
    InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut size);
    let list = crate::allocate(size.max(1)).cast::<c_void>();
    if list.is_null() {
        crate::panic_catalogued(frame, b"P1302");
    }
    let result = (|| {
        if InitializeProcThreadAttributeList(list, 1, 0, &mut size) == 0 {
            return Err(last_failure());
        }
        let updated = UpdateProcThreadAttribute(
            list,
            0,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
            inherited.as_ptr().cast(),
            mem::size_of_val(&inherited),
            ptr::null_mut(),
            ptr::null_mut(),
        );
        if updated == 0 {
            let failure = last_failure();
            DeleteProcThreadAttributeList(list);
            return Err(failure);
        }
        let mut startup: StartupInfoEx = mem::zeroed();
        startup.startup.cb = mem::size_of::<StartupInfoEx>() as u32;
        startup.startup.flags = STARTF_USESTDHANDLES;
        startup.startup.input = inherited[0];
        startup.startup.output = inherited[1];
        startup.startup.error = inherited[2];
        startup.attributes = list;
        let mut information: ProcessInformation = mem::zeroed();
        let created = CreateProcessW(
            application.data,
            command.data,
            ptr::null(),
            ptr::null(),
            1,
            CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT,
            environment
                .as_ref()
                .map_or(ptr::null(), |block| block.data.cast_const().cast()),
            directory
                .as_ref()
                .map_or(ptr::null(), |directory| directory.data.cast_const()),
            ptr::addr_of!(startup.startup),
            &mut information,
        );
        let failure = (created == 0).then(last_failure);
        DeleteProcThreadAttributeList(list);
        match failure {
            Some(failure) => Err(failure),
            None => {
                CloseHandle(information.thread);
                Ok(Process {
                    handle: information.process,
                    id: information.process_id,
                })
            }
        }
    })();
    crate::deallocate(list.cast());
    // The child's ends close here in the parent, so EOF follows its exit.
    drop((child_input, child_output, child_errors));
    Ok((result?, [input, output, errors]))
}

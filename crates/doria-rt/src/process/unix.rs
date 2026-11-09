//! Unix child processes: posix_spawn with three close-on-exec pipes, the
//! program found on the child's own PATH, a poll() drain of both outputs,
//! and deadline waits through kqueue (macOS) or a pidfd (Linux).
use core::{ffi::c_char, ptr};

use super::Request;
use crate::native_handle::{NativeHandle, OwnedHandle};
use crate::platform_io::{last_failure, Failure};
use crate::DrStackFrameV2;

pub(super) const SHELL: &[u8] = b"/bin/sh";
const DEFAULT_PATH: &[u8] = b"/usr/bin:/bin";

pub(crate) struct Process {
    pid: libc::pid_t,
    /// A kqueue (macOS) or pidfd (Linux) for deadline waits, made on first use.
    exit: Option<OwnedHandle>,
}

pub(super) fn id(process: &Process) -> i64 {
    i64::from(process.pid)
}

/// Raw status: the exit code, or the negated terminating signal.
fn decode(status: libc::c_int) -> i64 {
    if libc::WIFSIGNALED(status) {
        -i64::from(libc::WTERMSIG(status))
    } else {
        i64::from(libc::WEXITSTATUS(status))
    }
}

pub(super) unsafe fn wait(process: &mut Process) -> Result<i64, Failure> {
    let mut status = 0;
    loop {
        if libc::waitpid(process.pid, &mut status, 0) == process.pid {
            return Ok(decode(status));
        }
        let failure = last_failure();
        if failure.system_code != Some(i64::from(libc::EINTR)) {
            return Err(failure);
        }
    }
}

unsafe fn try_wait(process: &Process) -> Result<Option<i64>, Failure> {
    let mut status = 0;
    match libc::waitpid(process.pid, &mut status, libc::WNOHANG) {
        0 => Ok(None),
        pid if pid == process.pid => Ok(Some(decode(status))),
        _ => Err(last_failure()),
    }
}

pub(super) unsafe fn wait_until(
    process: &mut Process,
    budget: &crate::wait_budget::DrWaitBudgetV1,
) -> Result<Option<i64>, Failure> {
    loop {
        if let Some(status) = try_wait(process)? {
            return Ok(Some(status));
        }
        // Whole milliseconds, rounded up: poll/kevent resolution.
        let (kind, milliseconds) =
            budget.remaining(1_000_000, i32::MAX as u64, crate::monotonic_clock::now)?;
        if kind == 3 {
            return Ok(None);
        }
        let timeout = if kind == 1 { -1 } else { milliseconds as i32 };
        exit_wait(process, timeout)?;
    }
}

#[cfg(target_os = "macos")]
unsafe fn exit_wait(process: &mut Process, timeout: i32) -> Result<(), Failure> {
    if process.exit.is_none() {
        let queue = libc::kqueue();
        if queue < 0 {
            return Err(last_failure());
        }
        let handle = OwnedHandle::from_raw(queue);
        let change = libc::kevent {
            ident: process.pid as usize,
            filter: libc::EVFILT_PROC,
            flags: libc::EV_ADD | libc::EV_CLEAR,
            fflags: libc::NOTE_EXIT,
            data: 0,
            udata: ptr::null_mut(),
        };
        if libc::kevent(queue, &change, 1, ptr::null_mut(), 0, ptr::null()) < 0 {
            let failure = last_failure();
            // Already exited: the next waitpid reaps it.
            if failure.system_code == Some(i64::from(libc::ESRCH)) {
                return Ok(());
            }
            return Err(failure);
        }
        process.exit = Some(handle);
    }
    let queue = process.exit.as_ref().unwrap().get();
    let mut event: libc::kevent = core::mem::zeroed();
    let limit = libc::timespec {
        tv_sec: libc::time_t::from(timeout.max(0) / 1000),
        tv_nsec: libc::c_long::from(timeout.max(0) % 1000) * 1_000_000,
    };
    let limit = if timeout < 0 { ptr::null() } else { &limit };
    if libc::kevent(queue, ptr::null(), 0, &mut event, 1, limit) < 0 {
        let failure = last_failure();
        if failure.system_code != Some(i64::from(libc::EINTR)) {
            return Err(failure);
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
unsafe fn exit_wait(process: &mut Process, timeout: i32) -> Result<(), Failure> {
    if process.exit.is_none() {
        let pidfd = libc::syscall(libc::SYS_pidfd_open, process.pid, 0) as i32;
        if pidfd < 0 {
            let failure = last_failure();
            if failure.system_code != Some(i64::from(libc::ENOSYS)) {
                return Err(failure);
            }
            // Kernels before 5.3 have no pidfd: recheck at a short interval.
            let pause = if timeout < 0 { 10 } else { timeout.min(10) };
            let mut ignored = libc::pollfd {
                fd: -1,
                events: 0,
                revents: 0,
            };
            libc::poll(&mut ignored, 1, pause);
            return Ok(());
        }
        process.exit = Some(OwnedHandle::from_raw(pidfd));
    }
    let mut descriptor = libc::pollfd {
        fd: process.exit.as_ref().unwrap().get(),
        events: libc::POLLIN,
        revents: 0,
    };
    if libc::poll(&mut descriptor, 1, timeout) < 0 {
        let failure = last_failure();
        if failure.system_code != Some(i64::from(libc::EINTR)) {
            return Err(failure);
        }
    }
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
unsafe fn exit_wait(_: &mut Process, timeout: i32) -> Result<(), Failure> {
    let pause = if timeout < 0 { 10 } else { timeout.min(10) };
    let mut ignored = libc::pollfd {
        fd: -1,
        events: 0,
        revents: 0,
    };
    libc::poll(&mut ignored, 1, pause);
    Ok(())
}

/// Forced termination. A child that exited but was not reaped is still a
/// zombie, so this cannot hit an unrelated process.
pub(super) unsafe fn kill(process: &mut Process) -> Result<(), Failure> {
    if libc::kill(process.pid, libc::SIGKILL) == 0 {
        Ok(())
    } else {
        Err(last_failure())
    }
}

/// Reads both outputs to their end, whichever is ready first, so neither
/// pipe can fill and stall the child. Each sink receives its stream's chunks.
pub(super) unsafe fn drain<'a>(
    handles: [Option<NativeHandle>; 2],
    output: &'a mut dyn FnMut(&[u8]),
    errors: &'a mut dyn FnMut(&[u8]),
) -> Result<(), (usize, Failure)> {
    let sinks = [output, errors];
    let mut descriptors = handles.map(|handle| libc::pollfd {
        fd: handle.unwrap_or(-1),
        events: libc::POLLIN,
        revents: 0,
    });
    let mut chunk = [0u8; 16384];
    while descriptors.iter().any(|descriptor| descriptor.fd >= 0) {
        if libc::poll(descriptors.as_mut_ptr(), 2, -1) < 0 {
            let failure = last_failure();
            if failure.system_code == Some(i64::from(libc::EINTR)) {
                continue;
            }
            let index = descriptors.iter().position(|descriptor| descriptor.fd >= 0);
            return Err((index.unwrap_or(0), failure));
        }
        for (index, descriptor) in descriptors.iter_mut().enumerate() {
            if descriptor.fd < 0 || descriptor.revents == 0 {
                continue;
            }
            let count = libc::read(descriptor.fd, chunk.as_mut_ptr().cast(), chunk.len());
            if count > 0 {
                (sinks[index])(&chunk[..count as usize]);
            } else if count == 0 {
                descriptor.fd = -1;
            } else {
                let failure = last_failure();
                if failure.system_code != Some(i64::from(libc::EINTR))
                    && failure.system_code != Some(i64::from(libc::EAGAIN))
                {
                    return Err((index, failure));
                }
            }
        }
    }
    Ok(())
}

/// One allocation holding NUL-terminated copies and a null-terminated
/// pointer array: an argv or envp for posix_spawn.
struct CStrings {
    pointers: *mut *const c_char,
    bytes: *mut u8,
}

impl CStrings {
    unsafe fn new<'a>(
        frame: *const DrStackFrameV2,
        parts: impl Iterator<Item = (&'a [u8], &'a [u8], &'a [u8])> + Clone,
        borrowed: &[*const c_char],
    ) -> Self {
        let count = parts.clone().count() + borrowed.len();
        let length: usize = parts
            .clone()
            .map(|(a, b, c)| a.len() + b.len() + c.len() + 1)
            .sum();
        let pointers = crate::allocate((count + 1) * core::mem::size_of::<*const c_char>())
            .cast::<*const c_char>();
        let bytes = crate::allocate(length.max(1));
        if pointers.is_null() || bytes.is_null() {
            crate::panic_catalogued(frame, b"P1302");
        }
        let mut cursor = bytes;
        let mut index = 0;
        for (a, b, c) in parts {
            *pointers.add(index) = cursor.cast();
            for part in [a, b, c] {
                ptr::copy_nonoverlapping(part.as_ptr(), cursor, part.len());
                cursor = cursor.add(part.len());
            }
            *cursor = 0;
            cursor = cursor.add(1);
            index += 1;
        }
        for pointer in borrowed {
            *pointers.add(index) = *pointer;
            index += 1;
        }
        *pointers.add(index) = ptr::null();
        Self { pointers, bytes }
    }
}

impl Drop for CStrings {
    fn drop(&mut self) {
        unsafe {
            crate::deallocate(self.pointers.cast());
            crate::deallocate(self.bytes);
        }
    }
}

#[cfg(target_os = "macos")]
unsafe fn environment() -> *const *const c_char {
    *libc::_NSGetEnviron() as *const *const c_char
}

#[cfg(not(target_os = "macos"))]
unsafe fn environment() -> *const *const c_char {
    unsafe extern "C" {
        static environ: *const *const c_char;
    }
    environ
}

unsafe fn c_bytes<'a>(text: *const c_char) -> &'a [u8] {
    core::slice::from_raw_parts(text.cast::<u8>(), libc::strlen(text))
}

/// `NAME=value` entry's name.
fn entry_name(entry: &[u8]) -> &[u8] {
    entry
        .iter()
        .position(|byte| *byte == b'=')
        .map_or(entry, |end| &entry[..end])
}

/// The child's PATH: an edit wins over the inherited value.
unsafe fn child_path<'a>(request: &Request<'a>) -> Option<&'a [u8]> {
    if let Some(index) = request.names.iter().position(|name| name == b"PATH") {
        return Some(request.values.get(index));
    }
    if request.removed.iter().any(|name| name == b"PATH") {
        return None;
    }
    let path = libc::getenv(c"PATH".as_ptr());
    Some(if path.is_null() {
        DEFAULT_PATH
    } else {
        c_bytes(path)
    })
}

/// The executable: a name with a slash is used as written; otherwise the
/// first executable regular file on the child's PATH.
unsafe fn resolve(
    frame: *const DrStackFrameV2,
    request: &Request<'_>,
) -> Result<CStrings, Failure> {
    let program = if request.shell {
        SHELL
    } else {
        request.program
    };
    if program.contains(&b'/') {
        return Ok(CStrings::new(
            frame,
            core::iter::once((program, &b""[..], &b""[..])),
            &[],
        ));
    }
    let Some(path) = child_path(request) else {
        return Err(not_found());
    };
    for directory in path.split(|byte| *byte == b':') {
        let directory: &[u8] = if directory.is_empty() {
            b"."
        } else {
            directory
        };
        let candidate = CStrings::new(
            frame,
            core::iter::once((directory, &b"/"[..], program)),
            &[],
        );
        let mut status: libc::stat = core::mem::zeroed();
        let name = *candidate.pointers;
        if libc::stat(name, &mut status) == 0
            && status.st_mode & libc::S_IFMT == libc::S_IFREG
            && libc::access(name, libc::X_OK) == 0
        {
            return Ok(candidate);
        }
    }
    Err(not_found())
}

fn not_found() -> Failure {
    Failure::from_system_code(i64::from(libc::ENOENT))
}

/// A pipe whose two ends are close-on-exec and numbered above the standard
/// descriptors, so dup2 onto 0-2 never leaves an end that exec would close.
unsafe fn pipe() -> Result<[OwnedHandle; 2], Failure> {
    let mut ends = [0; 2];
    #[cfg(target_os = "linux")]
    let created = libc::pipe2(ends.as_mut_ptr(), libc::O_CLOEXEC);
    #[cfg(not(target_os = "linux"))]
    let created = libc::pipe(ends.as_mut_ptr());
    if created < 0 {
        return Err(last_failure());
    }
    let mut owned = ends.map(|end| OwnedHandle::from_raw(end));
    for end in &mut owned {
        let raw = end.get();
        if raw < 3 {
            let moved = libc::fcntl(raw, libc::F_DUPFD_CLOEXEC, 3);
            if moved < 0 {
                return Err(last_failure());
            }
            *end = OwnedHandle::from_raw(moved);
        } else if libc::fcntl(raw, libc::F_SETFD, libc::FD_CLOEXEC) < 0 {
            return Err(last_failure());
        }
    }
    Ok(owned)
}

/// Makes a parent's pipe end nonblocking so it can join a transfer group. The
/// two ends of a pipe are separate open file descriptions, so the child's end,
/// which becomes its standard stream, stays blocking.
unsafe fn set_nonblocking(end: &OwnedHandle) -> Result<(), Failure> {
    let flags = libc::fcntl(end.get(), libc::F_GETFL);
    if flags < 0 || libc::fcntl(end.get(), libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
        return Err(last_failure());
    }
    Ok(())
}

unsafe extern "C" {
    // macOS 10.15 and glibc 2.29 export this; libc binds it for glibc only.
    fn posix_spawn_file_actions_addchdir_np(
        actions: *mut libc::posix_spawn_file_actions_t,
        path: *const c_char,
    ) -> libc::c_int;
}

/// Spawns the child with its own pipe ends on 0, 1 and 2 and returns the
/// parent's ends: input, output, error output.
pub(super) unsafe fn spawn(
    frame: *const DrStackFrameV2,
    request: &Request<'_>,
) -> Result<(Process, [OwnedHandle; 3]), Failure> {
    let executable = resolve(frame, request)?;
    let program = if request.shell {
        SHELL
    } else {
        request.program
    };
    let arguments = request.arguments.iter();
    let head: [&[u8]; 3] = [program, b"-c", request.program];
    let head = &head[..if request.shell { 3 } else { 1 }];
    let argv = CStrings::new(
        frame,
        head.iter()
            .copied()
            .chain(arguments)
            .map(|part| (part, &b""[..], &b""[..])),
        &[],
    );
    let edited = request.names.len() != 0 || request.removed.len() != 0;
    let mut kept = 0;
    let inherited = environment();
    if edited {
        while !(*inherited.add(kept)).is_null() {
            kept += 1;
        }
    }
    // Inherited entries survive unless set or removed; sets are appended.
    let envp = if edited {
        let filtered = crate::allocate((kept + 1) * core::mem::size_of::<*const c_char>())
            .cast::<*const c_char>();
        if filtered.is_null() {
            crate::panic_catalogued(frame, b"P1302");
        }
        let mut count = 0;
        for index in 0..kept {
            let entry = *inherited.add(index);
            let name = entry_name(c_bytes(entry));
            if !request.names.iter().any(|edit| edit == name)
                && !request.removed.iter().any(|edit| edit == name)
            {
                *filtered.add(count) = entry;
                count += 1;
            }
        }
        let sets = (0..request.names.len()).map(|index| {
            (
                request.names.get(index),
                &b"="[..],
                request.values.get(index),
            )
        });
        let strings = CStrings::new(frame, sets, core::slice::from_raw_parts(filtered, count));
        crate::deallocate(filtered.cast());
        Some(strings)
    } else {
        None
    };
    let directory = request.directory.map(|directory| {
        CStrings::new(
            frame,
            core::iter::once((directory, &b""[..], &b""[..])),
            &[],
        )
    });

    let [input_read, input_write] = pipe()?;
    let [output_read, output_write] = pipe()?;
    let [errors_read, errors_write] = pipe()?;
    for parent_end in [&input_write, &output_read, &errors_read] {
        set_nonblocking(parent_end)?;
    }
    let mut actions: libc::posix_spawn_file_actions_t = core::mem::zeroed();
    let mut attributes: libc::posix_spawnattr_t = core::mem::zeroed();
    let mut code = libc::posix_spawn_file_actions_init(&mut actions);
    if code != 0 {
        return Err(Failure::from_system_code(i64::from(code)));
    }
    code = libc::posix_spawnattr_init(&mut attributes);
    if code != 0 {
        libc::posix_spawn_file_actions_destroy(&mut actions);
        return Err(Failure::from_system_code(i64::from(code)));
    }
    let mut pid = 0;
    let result = (|| {
        for (end, target) in [
            (input_read.get(), 0),
            (output_write.get(), 1),
            (errors_write.get(), 2),
        ] {
            let code = libc::posix_spawn_file_actions_adddup2(&mut actions, end, target);
            if code != 0 {
                return code;
            }
        }
        if let Some(directory) = &directory {
            let code = posix_spawn_file_actions_addchdir_np(&mut actions, *directory.pointers);
            if code != 0 {
                return code;
            }
        }
        // A fresh signal state: default SIGPIPE and an empty mask, whatever
        // this program set, as for a child started from a shell.
        let mut defaults: libc::sigset_t = core::mem::zeroed();
        let mut empty: libc::sigset_t = core::mem::zeroed();
        libc::sigemptyset(&mut defaults);
        libc::sigaddset(&mut defaults, libc::SIGPIPE);
        libc::sigemptyset(&mut empty);
        for code in [
            libc::posix_spawnattr_setsigdefault(&mut attributes, &defaults),
            libc::posix_spawnattr_setsigmask(&mut attributes, &empty),
            libc::posix_spawnattr_setflags(
                &mut attributes,
                (libc::POSIX_SPAWN_SETSIGDEF | libc::POSIX_SPAWN_SETSIGMASK) as libc::c_short,
            ),
        ] {
            if code != 0 {
                return code;
            }
        }
        libc::posix_spawn(
            &mut pid,
            *executable.pointers,
            &actions,
            &attributes,
            argv.pointers.cast(),
            envp.as_ref()
                .map_or(environment(), |envp| envp.pointers.cast_const())
                .cast(),
        )
    })();
    libc::posix_spawn_file_actions_destroy(&mut actions);
    libc::posix_spawnattr_destroy(&mut attributes);
    // The child's ends close here in the parent, so EOF follows its exit.
    drop((input_read, output_write, errors_write));
    if result != 0 {
        return Err(Failure::from_system_code(i64::from(result)));
    }
    Ok((
        Process { pid, exit: None },
        [input_write, output_read, errors_read],
    ))
}

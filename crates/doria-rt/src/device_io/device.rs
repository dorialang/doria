//! Process-wide device identity and exclusive access. A view is metadata, not
//! an owner. The one stdin queue and every retained source refer to this same
//! static device; accessing it creates no handle, heap object or second queue.
use core::{
    cell::UnsafeCell,
    ptr,
    sync::atomic::{AtomicBool, Ordering},
};

use super::{InputBuffer, StandardStream};
use crate::native_handle::ReadinessProjection;
use crate::platform_io::Failure;

pub(crate) struct StandardDevice {
    stream: StandardStream,
    borrowed: AtomicBool,
    input: UnsafeCell<InputBuffer>,
    source: ReadinessProjection,
}

// The projection is immutable. Mutable queue access requires the sole device
// loan; the unsafe low-level methods retain that explicit caller precondition.
unsafe impl Sync for StandardDevice {}

impl StandardDevice {
    const fn new(stream: StandardStream, source: *const ReadinessProjection) -> Self {
        Self {
            stream,
            borrowed: AtomicBool::new(false),
            input: UnsafeCell::new(InputBuffer::new()),
            source: ReadinessProjection::standard(source, stream),
        }
    }

    pub(crate) fn source(&self) -> &ReadinessProjection {
        &self.source
    }

    pub(crate) fn try_borrow(&self) -> Option<DeviceLoan<'_>> {
        self.borrowed
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()
            .map(|_| DeviceLoan { device: self })
    }

    pub(crate) fn try_borrow_for_io(&self) -> Result<DeviceLoan<'_>, Failure> {
        self.try_borrow().ok_or_else(Failure::invalid_input)
    }

    /// # Safety
    /// The private caller has already established exclusive canonical-device
    /// access. Fallible admission must use try_borrow and its checked rejection.
    #[cfg(test)]
    pub(crate) unsafe fn borrow(&self) -> DeviceLoan<'_> {
        // This is an unsafe PRIVATE consumer precondition, not a source-level
        // alias policy or a writable-shared allocation's P1501 contract.
        self.try_borrow()
            .unwrap_or_else(|| unsafe { crate::runtime_invariant_panic() })
    }

    /// # Safety
    /// Caller holds this device's exclusive loan for all queue access and any
    /// returned slice's lifetime. A standard-output device has no input queue.
    pub(crate) unsafe fn input(&self) -> *mut InputBuffer {
        debug_assert_eq!(self.stream, StandardStream::Stdin);
        self.input.get()
    }
}

pub(crate) struct DeviceLoan<'a> {
    device: &'a StandardDevice,
}

impl DeviceLoan<'_> {
    pub(crate) unsafe fn write_text(&self, bytes: *const u8, length: usize) -> super::WriteOutcome {
        super::write_unclaimed(self.device.stream, bytes, length)
    }

    pub(crate) unsafe fn write_bytes(
        &self,
        bytes: *const u8,
        length: usize,
    ) -> super::WriteOutcome {
        super::write_bytes_unclaimed(self.device.stream, bytes, length)
    }

    /// Configuration-only observation, never a native read or queue drain.
    pub(crate) fn buffered_input_ready(&self) -> bool {
        self.device.stream == StandardStream::Stdin
            && unsafe {
                let input = &*self.device.input();
                input.start != input.end || input.eof
            }
    }

    /// Uses only bytes already owned by the canonical stdin queue. None means
    /// raw input is needed, not WouldBlock or EOF. Empty storage is Data(0).
    pub(crate) unsafe fn read_buffered(
        &self,
        data: *mut u8,
        length: usize,
    ) -> Option<super::ReadProgress> {
        if self.device.stream != StandardStream::Stdin {
            return None;
        }
        if length != 0 && !self.buffered_input_ready() {
            return None;
        }
        Some(
            (&mut *self.device.input())
                .read_progress_with(data, length, |_, _| crate::runtime_invariant_panic())
                .unwrap_or_else(|_| crate::runtime_invariant_panic()),
        )
    }

    pub(crate) unsafe fn read_bytes(&self, data: *mut u8, length: usize) -> Result<usize, Failure> {
        if self.device.stream != StandardStream::Stdin {
            return Err(Failure::invalid_input());
        }
        (&mut *self.device.input()).read_bytes_with(data, length, |data, length| {
            super::read_raw_stdin(data, length)
        })
    }

    pub(crate) unsafe fn read_attempt(
        &self,
        handle: crate::native_handle::NativeHandle,
        data: *mut u8,
        length: usize,
    ) -> Result<super::ReadProgress, Failure> {
        if self.device.stream != StandardStream::Stdin {
            return Err(Failure::invalid_input());
        }
        (&mut *self.device.input()).read_progress_with(data, length, |data, length| {
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            {
                crate::byte_io::read_attempt(handle, data, length)
            }
            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            {
                #[cfg(windows)]
                crate::native_handle::require_synchronous_device(handle)?;
                crate::byte_io::read(handle, data, length)
            }
        })
    }

    pub(crate) unsafe fn write_attempt(
        &self,
        handle: crate::native_handle::NativeHandle,
        data: *const u8,
        length: usize,
    ) -> Result<super::WriteProgress, Failure> {
        if self.device.stream == StandardStream::Stdin {
            return Err(Failure::invalid_input());
        }
        #[cfg(windows)]
        if length != 0 {
            crate::native_handle::require_synchronous_device(handle)?;
        }
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let result = crate::byte_io::write_attempt(handle, data, length);
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let result = crate::byte_io::write(handle, data, length);
        self.finish_write(result)
    }

    /// Apply standard-output policy on the driving thread after native work
    /// has actually retired, whether inline or through an inherited helper.
    pub(crate) unsafe fn finish_write(
        &self,
        result: Result<super::WriteProgress, Failure>,
    ) -> Result<super::WriteProgress, Failure> {
        if let Err(failure) = result {
            #[cfg(unix)]
            let outcome = super::classify_unix_write_error(Some(failure));
            #[cfg(windows)]
            let outcome = super::classify_windows_write_error(Some(failure));
            #[cfg(not(any(unix, windows)))]
            let outcome = super::WriteOutcome::OtherFailure(failure);
            if outcome == super::WriteOutcome::BrokenPipe {
                crate::exit_process(0);
            }
        }
        result
    }
}

impl Drop for DeviceLoan<'_> {
    fn drop(&mut self) {
        self.device.borrowed.store(false, Ordering::Release);
    }
}

static STDIN: StandardDevice =
    StandardDevice::new(StandardStream::Stdin, ptr::addr_of!(STDIN.source));
static STDOUT: StandardDevice =
    StandardDevice::new(StandardStream::Stdout, ptr::addr_of!(STDOUT.source));
static STDERR: StandardDevice =
    StandardDevice::new(StandardStream::Stderr, ptr::addr_of!(STDERR.source));

pub(crate) fn standard_device(stream: StandardStream) -> &'static StandardDevice {
    match stream {
        StandardStream::Stdin => &STDIN,
        StandardStream::Stdout => &STDOUT,
        StandardStream::Stderr => &STDERR,
    }
}

#[cfg(test)]
pub(crate) fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Test injection replaces only queue contents under the existing device loan.
/// It never replaces a platform handle or creates another persistent queue.
#[cfg(test)]
pub(crate) struct TestInput {
    saved: Option<InputBuffer>,
}

#[cfg(test)]
impl TestInput {
    pub(crate) unsafe fn new(bytes: &[u8], eof: bool) -> Self {
        let _loan = standard_device(StandardStream::Stdin).borrow();
        let input = &mut *standard_device(StandardStream::Stdin).input();
        let saved = core::mem::replace(input, InputBuffer::new());
        input.ensure_capacity(bytes.len()).unwrap();
        if !bytes.is_empty() {
            ptr::copy_nonoverlapping(bytes.as_ptr(), input.bytes, bytes.len());
        }
        input.end = bytes.len();
        input.eof = eof;
        Self { saved: Some(saved) }
    }
}

#[cfg(test)]
impl Drop for TestInput {
    fn drop(&mut self) {
        unsafe {
            let _loan = standard_device(StandardStream::Stdin).borrow();
            let input = &mut *standard_device(StandardStream::Stdin).input();
            let injected = core::mem::replace(input, self.saved.take().unwrap());
            drop(injected);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_handle::{PhysicalOrigin, ReadinessProjection};

    #[test]
    fn standard_handle_lookup_preserves_exclusive_claims_without_allocation() {
        let _serial = test_lock();
        for stream in [
            StandardStream::Stdin,
            StandardStream::Stdout,
            StandardStream::Stderr,
        ] {
            let device = standard_device(stream);
            let expected = super::super::byte_handle(stream);
            let loan = device.try_borrow().unwrap();
            let buffered = loan.buffered_input_ready();
            let (_, allocations) = crate::probe_runtime_allocations(|| {
                assert_eq!(super::super::byte_handle(stream), expected);
                assert!(device.try_borrow().is_none());
                assert_eq!(loan.buffered_input_ready(), buffered);
            });
            assert_eq!(allocations, 0);
            drop(loan);
            assert!(device.try_borrow().is_some());
            assert_eq!(super::super::byte_handle(stream), expected);
        }
    }

    #[test]
    fn standard_sources_forward_one_static_identity_without_owners_or_allocation() {
        let (_, allocations) = crate::probe_runtime_allocations(|| unsafe {
            for stream in [
                StandardStream::Stdin,
                StandardStream::Stdout,
                StandardStream::Stderr,
            ] {
                let device = standard_device(stream);
                let source = device.source();
                let forwarded = *source.home().cast::<*const core::ffi::c_void>();
                assert_eq!(forwarded, source.value());
                let borrowed = ReadinessProjection::borrow(forwarded).unwrap();
                assert!(ptr::eq(borrowed, source));
                assert!(borrowed.origin.file().is_none());
                assert!(matches!(borrowed.origin, PhysicalOrigin::Standard(s) if s == stream));
                assert_eq!(source.value(), standard_device(stream).source().value());
            }
            assert_ne!(
                standard_device(StandardStream::Stdin).source().value(),
                standard_device(StandardStream::Stdout).source().value()
            );
        });
        assert_eq!(allocations, 0);
    }

    #[test]
    fn forged_standard_projection_is_not_a_second_device_identity() {
        let mut fake = core::mem::MaybeUninit::<ReadinessProjection>::uninit();
        unsafe {
            fake.write(ReadinessProjection::standard(
                fake.as_ptr(),
                StandardStream::Stdin,
            ));
            assert!(ReadinessProjection::borrow(fake.as_ptr().cast()).is_err());
        }
    }

    #[test]
    fn already_loaned_legacy_output_and_control_do_not_acquire_a_second_claim() {
        let _serial = test_lock();
        unsafe {
            for stream in [StandardStream::Stdout, StandardStream::Stderr] {
                let device = standard_device(stream);
                let loan = device.borrow();
                assert!(device.try_borrow().is_none());
                let (_, allocations) = crate::probe_runtime_allocations(|| {
                    assert_eq!(
                        loan.write_text(ptr::dangling(), 0),
                        super::super::WriteOutcome::Success
                    );
                    assert_eq!(
                        loan.write_bytes(ptr::dangling(), 0),
                        super::super::WriteOutcome::Success
                    );
                    assert_eq!(
                        super::super::flush(stream),
                        super::super::WriteOutcome::Success
                    );
                    let _ = super::super::is_interactive(stream);
                });
                assert_eq!(allocations, 0);
                assert!(device.try_borrow().is_none());
                drop(loan);
                assert!(device.try_borrow().is_some());
            }
        }
    }

    #[test]
    fn line_primitive_and_legacy_consumers_share_the_queue_and_exclusive_device_loan() {
        let _serial = test_lock();
        unsafe {
            let _injected = TestInput::new(b"first\n\xff\0last\n", true);
            let device = standard_device(StandardStream::Stdin);
            let queue = device.input();
            let loan = device.borrow();
            assert!(device.try_borrow().is_none());
            for _ in 0..3 {
                assert!(loan.buffered_input_ready());
                assert_eq!((*queue).start, 0, "readiness must not consume input");
            }
            let line = crate::line_io::read_line_with(&mut *queue, |_, _| panic!("injected EOF"))
                .unwrap()
                .unwrap();
            assert_eq!(core::slice::from_raw_parts(line.0, line.1), b"first");
            drop(loan);
            let mut raw = [0; 2];
            let (_, allocations) = crate::probe_runtime_allocations(|| {
                assert_eq!(
                    super::super::read_progress(StandardStream::Stdin, raw.as_mut_ptr(), 2),
                    Ok(super::super::ReadProgress::Data(2))
                );
            });
            assert_eq!(allocations, 0);
            assert_eq!(&raw, b"\xff\0");
            let (loan, buffered) = crate::line_io::take_buffered_input().unwrap();
            assert!(
                device.try_borrow().is_none(),
                "returned bytes retain the queue loan"
            );
            assert_eq!(
                core::slice::from_raw_parts(buffered.bytes, buffered.length),
                b"last\n"
            );
            assert!(buffered.eof);
            assert_eq!(loan.read_bytes(ptr::null_mut(), 0), Ok(0));
            drop(loan);
            assert!(device.try_borrow().is_some());
            assert_eq!(queue, device.input(), "one device-owned queue throughout");
        }
    }

    #[cfg(unix)]
    #[test]
    fn inherited_modes_are_unchanged_on_normal_broken_pipe_and_fatal_termination() {
        use std::io::Read;
        use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
        use std::process::{Command, Stdio};
        for case in [
            "legacy_text",
            "legacy_bytes",
            "primitive",
            "normal101",
            "panic",
        ] {
            let mut descriptors = [-1; 2];
            assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
            let reader = unsafe { OwnedFd::from_raw_fd(descriptors[0]) };
            let writer = unsafe { OwnedFd::from_raw_fd(descriptors[1]) };
            let flags = unsafe { libc::fcntl(reader.as_raw_fd(), libc::F_GETFL) };
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "device_io::device::tests::inherited_termination_child",
                    "--nocapture",
                ])
                .env("DORIA_RT_TEST_MODE_TERMINATION", case)
                .stdin(Stdio::from(reader.try_clone().unwrap()))
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            // Let the harness start with stdout intact. The helper signals only
            // after a real helper has started on the inherited stdin description.
            let mut ready = [0; 1];
            child
                .stderr
                .as_mut()
                .unwrap()
                .read_exact(&mut ready)
                .unwrap();
            assert_eq!(ready, [b'R']);
            drop(child.stdout.take()); // close downstream reader, without handle replacement
            assert_eq!(
                unsafe { libc::write(writer.as_raw_fd(), b"A".as_ptr().cast(), 1) },
                1
            );
            // Drain diagnostics while waiting, so a child assertion/panic is
            // visible and cannot block termination on a full stderr pipe.
            let output = child.wait_with_output().unwrap();
            let diagnostics = String::from_utf8_lossy(&output.stderr);
            let observed = unsafe { libc::fcntl(reader.as_raw_fd(), libc::F_GETFL) };
            let expected = if matches!(case, "normal101" | "panic") {
                101
            } else {
                0
            };
            assert_eq!(
                output.status.code(),
                Some(expected),
                "termination case {case}: {diagnostics}"
            );
            assert_eq!(
                observed, flags,
                "helper must never mutate inherited mode: {case}"
            );
            if case == "panic" {
                assert!(
                    diagnostics.contains("P1001"),
                    "fatal termination must report the intended Doria panic: {diagnostics}"
                );
            } else {
                assert!(
                    output.stderr.is_empty(),
                    "ordinary termination must not be a child harness panic ({case}): {diagnostics}"
                );
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn inherited_termination_child() {
        let Ok(case) = std::env::var("DORIA_RT_TEST_MODE_TERMINATION") else {
            return;
        };
        unsafe {
            libc::signal(libc::SIGPIPE, libc::SIG_IGN);
            unsafe fn notice(_: usize) {}
            let input = standard_device(StandardStream::Stdin).borrow();
            let mut helper = crate::device_io::helper::Helper::new();
            let mut ack = [0; 1];
            helper
                .submit(crate::device_io::helper::Job {
                    handle: 0,
                    data: ack.as_mut_ptr(),
                    length: 1,
                    reading: true,
                    notify: notice,
                    notice: 0,
                })
                .unwrap();
            assert_eq!(libc::write(2, b"R".as_ptr().cast(), 1), 1);
            assert!(matches!(
                helper.drain(),
                Some(crate::transfer_group::pending::FileTerminal::Read(Ok(
                    crate::byte_io::ReadProgress::Data(1)
                )))
            ));
            assert_eq!(ack, [b'A']);
            drop(input);
            // A concurrent spawn may briefly retain an inherited pipe reader
            // until exec. The parent's acknowledgement proves its reader closed,
            // not that every reader closed. Wait for the actual broken-pipe
            // condition without probing writes or changing inherited flags.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let mut downstream = libc::pollfd {
                fd: 1,
                // Darwin arms its closure watch only with an explicit interest;
                // Linux reports pipe closure regardless of this output-only bit.
                // Do not request POLLOUT, which is ready while readers remain.
                events: libc::POLLHUP,
                revents: 0,
            };
            loop {
                let remaining = deadline
                    .checked_duration_since(std::time::Instant::now())
                    .expect("stdout pipe readers did not retire before the fixture deadline");
                let timeout = i32::try_from(remaining.as_millis() + 1).unwrap();
                let ready = libc::poll(&mut downstream, 1, timeout);
                if ready < 0
                    && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
                {
                    continue;
                }
                assert_eq!(ready, 1, "stdout pipe did not report downstream closure");
                assert_eq!(downstream.revents & libc::POLLNVAL, 0);
                assert_ne!(downstream.revents & (libc::POLLERR | libc::POLLHUP), 0);
                break;
            }
            match case.as_str() {
                "legacy_text" => crate::dr_v2_write_stdout(ptr::null(), b"x".as_ptr(), 1),
                "legacy_bytes" => {
                    let data = crate::allocate(1);
                    data.write(b'x');
                    let bytes = crate::bytes::from_owned(data, 1);
                    crate::dr_v2_write_stdout_bytes(ptr::null(), bytes);
                }
                "primitive" => {
                    let output = standard_device(StandardStream::Stdout).borrow();
                    let mut writer = crate::device_io::helper::Helper::new();
                    writer
                        .submit(crate::device_io::helper::Job {
                            handle: 1,
                            data: b"x".as_ptr().cast_mut(),
                            length: 1,
                            reading: false,
                            notify: notice,
                            notice: 0,
                        })
                        .unwrap();
                    let Some(crate::transfer_group::pending::FileTerminal::Write(result)) =
                        writer.drain()
                    else {
                        panic!("missing native write result");
                    };
                    let _ = output.finish_write(result);
                }
                // Numerical status 101 is also a legal ordinary exit status;
                // unchanged modes do not depend on a cleanup/status heuristic.
                "normal101" => crate::dr_v1_exit_process(101),
                "panic" => crate::panic_catalogued(ptr::null(), b"P1001"),
                _ => panic!("unknown child fixture"),
            }
            panic!("termination fixture unexpectedly returned");
        }
    }
}

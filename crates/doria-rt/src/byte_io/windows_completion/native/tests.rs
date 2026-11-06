//! Real Windows coverage. Cross-compilation checks these fixtures, but does not
//! execute their completion, cancellation, or kernel-lifetime assertions.

use std::os::windows::io::{AsRawHandle, FromRawHandle, IntoRawHandle, OwnedHandle as StdHandle};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::time::Duration;

use super::*;

#[test]
fn control_owns_the_frozen_budget_after_caller_storage_changes_and_expires() {
    for (kind, nanos) in [(0, 0), (1, 0), (2, u64::MAX), (2, 5_000_000_000)] {
        let (mut control, allocations) = crate::probe_runtime_allocations(|| {
            let mut original = budget(kind, nanos);
            let control = unsafe { WaitControl::new(Some(&original), ptr::null_mut()).unwrap() };
            let expected = original.remaining(1, u64::MAX, || Ok(0));
            original = budget(0, 0);
            assert_eq!(original.remaining(1, u64::MAX, || Ok(0)), Ok((0, 0)));
            assert_eq!(
                control.budget.unwrap().remaining(1, u64::MAX, || Ok(0)),
                expected
            );
            control
        });
        assert_eq!(allocations, 0);
        let next = control.next_wait().unwrap();
        if kind == 2 && nanos == u64::MAX {
            assert!(matches!(next, Err(Stop::BudgetExhausted)));
        } else {
            assert_eq!(next.unwrap().immediate, kind == 0);
        }
    }
}

const TEST_WAIT: Duration = Duration::from_secs(5);
const OVERLAPPED_FLAG: u32 = 0x4000_0000;
static NEXT_PIPE: AtomicUsize = AtomicUsize::new(0);

struct Connection<'a, 'r> {
    handle: &'a StdHandle,
    request: Pin<&'r mut Overlapped>,
    active: bool,
}

impl Connection<'_, '_> {
    fn raw(&mut self) -> *mut Overlapped {
        unsafe { self.request.as_mut().get_unchecked_mut() }
    }
}

impl Drop for Connection<'_, '_> {
    fn drop(&mut self) {
        if self.active {
            unsafe {
                let request = self.raw();
                let _ = CancelIoEx(self.handle.as_raw_handle(), request.cast());
                // Test setup follows the same kernel lifetime obligation: a
                // failed client connection cannot release pending stack data.
                loop {
                    let mut count = 0;
                    let complete = GetOverlappedResult(
                        self.handle.as_raw_handle(),
                        request.cast(),
                        &mut count,
                        1,
                    );
                    if complete != 0
                        || ptr::read_volatile((*request).internal.get()) != STATUS_PENDING
                    {
                        break;
                    }
                    if WaitForSingleObject((*request).event, INFINITE) != 0 {
                        crate::runtime_invariant_panic();
                    }
                }
            }
        }
    }
}

pub(super) fn connected_pair() -> (StdHandle, StdHandle) {
    let name = format!(
        r"\\.\pipe\doria-completion-{}-{}",
        std::process::id(),
        NEXT_PIPE.fetch_add(1, Ordering::Relaxed)
    );
    let wide: Vec<_> = name.encode_utf16().chain(Some(0)).collect();
    unsafe {
        // Duplex, overlapped, first instance; BYTE/READMODE_BYTE/PIPE_WAIT;
        // reject remote clients. Neither handle is attached to a completion
        // port, callback, or external I/O consumer.
        let raw = CreateNamedPipeW(
            wide.as_ptr(),
            3 | OVERLAPPED_FLAG | 0x0008_0000,
            8,
            1,
            4096,
            4096,
            0,
            ptr::null_mut(),
        );
        assert_ne!(
            raw,
            -1_isize as NativeHandle,
            "{}",
            std::io::Error::last_os_error()
        );
        let server = StdHandle::from_raw_handle(raw);
        let event = CreateEventW(ptr::null_mut(), 1, 0, ptr::null());
        assert!(!event.is_null());
        let event = StdHandle::from_raw_handle(event);
        let request = core::pin::pin!(Overlapped::new(event.as_raw_handle()));
        let mut connection = Connection {
            handle: &server,
            request,
            active: true,
        };
        let result = ConnectNamedPipe(server.as_raw_handle(), connection.raw().cast());
        let pending = if result != 0 {
            connection.active = false;
            false
        } else {
            match GetLastError() {
                997 => true,
                535 => {
                    connection.active = false;
                    false
                }
                code => {
                    connection.active = false;
                    panic!("ConnectNamedPipe failed: {code}");
                }
            }
        };
        let raw = CreateFileW(
            wide.as_ptr(),
            0xc000_0000,
            0,
            ptr::null_mut(),
            3,
            OVERLAPPED_FLAG,
            ptr::null_mut(),
        );
        assert_ne!(
            raw,
            -1_isize as NativeHandle,
            "{}",
            std::io::Error::last_os_error()
        );
        let client = StdHandle::from_raw_handle(raw);
        if pending {
            assert_eq!(
                WaitForSingleObject(event.as_raw_handle(), 5000),
                0,
                "named-pipe connection did not complete"
            );
            let mut unused_connect_count = 0;
            assert_ne!(
                GetOverlappedResult(
                    server.as_raw_handle(),
                    connection.raw().cast(),
                    &mut unused_connect_count,
                    0
                ),
                0
            );
            connection.active = false;
        }
        drop(connection);
        (server, client)
    }
}

pub(super) struct TestClaim(*mut DrCancellationV1);

// The claim keeps the all-atomic shared state and its persistent signal alive.
unsafe impl Send for TestClaim {}
unsafe impl Sync for TestClaim {}

impl TestClaim {
    pub(super) fn new() -> Self {
        let state = cancellation::dr_v1_cancellation_new();
        assert!(!state.is_null());
        Self(state)
    }

    pub(super) fn state(&self) -> *mut DrCancellationV1 {
        self.0
    }

    pub(super) fn request(&self) {
        let (mut reason, mut code, mut has_code) = (255, -1, 1);
        assert_eq!(
            unsafe {
                cancellation::dr_v1_cancellation_request(
                    self.0,
                    &mut reason,
                    &mut code,
                    &mut has_code,
                )
            },
            0
        );
        assert_eq!((reason, code, has_code), (0, 0, 0));
    }
}

impl Clone for TestClaim {
    fn clone(&self) -> Self {
        let retained = unsafe { cancellation::dr_v1_cancellation_retain(self.0) };
        assert_eq!(retained, self.0);
        Self(retained)
    }
}

impl Drop for TestClaim {
    fn drop(&mut self) {
        unsafe { cancellation::dr_v1_cancellation_release(self.0) };
    }
}

pub(super) fn budget(kind: u8, nanoseconds: u64) -> DrWaitBudgetV1 {
    let mut budget = mem::MaybeUninit::uninit();
    let (mut reason, mut code, mut has_code) = (255, -1, 1);
    assert_eq!(
        unsafe {
            wait_budget::dr_v1_wait_budget_init(
                kind,
                nanoseconds,
                budget.as_mut_ptr(),
                &mut reason,
                &mut code,
                &mut has_code,
            )
        },
        0
    );
    assert_eq!((reason, code, has_code), (0, 0, 0));
    unsafe { budget.assume_init() }
}

struct AbiOwner {
    storage: mem::MaybeUninit<Pipe<WindowsDriver>>,
    live: bool,
}

impl AbiOwner {
    fn adopt(handle: StdHandle) -> Self {
        let mut owner = Self {
            storage: mem::MaybeUninit::uninit(),
            live: false,
        };
        let mut status = DrIoStatusV1::default();
        assert_eq!(
            unsafe { dr_v1_windows_pipe_adopt(owner.raw(), handle.into_raw_handle(), &mut status) },
            0
        );
        owner.live = true;
        owner
    }

    fn raw(&mut self) -> *mut c_void {
        self.storage.as_mut_ptr().cast()
    }

    fn event(&self) -> NativeHandle {
        unsafe { self.storage.assume_init_ref().driver.completion.get() }
    }

    fn close(&mut self) {
        let mut status = DrIoStatusV1::default();
        self.live = false;
        assert_eq!(
            unsafe { dr_v1_windows_pipe_close(self.raw(), &mut status) },
            0
        );
    }
}

impl Drop for AbiOwner {
    fn drop(&mut self) {
        if self.live {
            unsafe { dr_v1_windows_pipe_drop(self.raw()) };
        }
    }
}

struct Observed {
    driver: WindowsDriver,
    pending: SyncSender<()>,
    notified: bool,
}

// All terminal facts come from the production driver. The notification is
// emitted only after Win32 has actually accepted a pending operation.
unsafe impl Driver for Observed {
    fn event(&self) -> *mut c_void {
        self.driver.event()
    }
    unsafe fn submit(&mut self, request: *mut Overlapped, buffer: &mut Buffer<'_>) -> Submission {
        let submission = self.driver.submit(request, buffer);
        if matches!(submission, Submission::Pending) && !self.notified {
            self.notified = true;
            self.pending.send(()).expect("pending observer was dropped");
        }
        submission
    }
    unsafe fn query(&mut self, request: *mut Overlapped) -> Query {
        self.driver.query(request)
    }
    fn wait(&mut self, timeout: u32, cancellation: Option<usize>) -> Result<Wake, Failure> {
        self.driver.wait(timeout, cancellation)
    }
    unsafe fn cancel(&mut self, request: *mut Overlapped) -> Result<(), Failure> {
        self.driver.cancel(request)
    }
    unsafe fn drain(&mut self, request: *mut Overlapped) -> Result<usize, Failure> {
        self.driver.drain(request)
    }
}

#[test]
fn overlapped_layout_matches_windows_pointer_width_and_native_offsets() {
    let pointer = mem::size_of::<usize>();
    assert_eq!(mem::size_of::<Overlapped>(), 3 * pointer + 8);
    assert_eq!(mem::align_of::<Overlapped>(), mem::align_of::<usize>());
    assert_eq!(mem::offset_of!(Overlapped, internal), 0);
    assert_eq!(mem::offset_of!(Overlapped, internal_high), pointer);
    assert_eq!(mem::offset_of!(Overlapped, offset), 2 * pointer);
    assert_eq!(mem::offset_of!(Overlapped, offset_high), 2 * pointer + 4);
    assert_eq!(mem::offset_of!(Overlapped, event), 2 * pointer + 8);
    let (mut owner_size, mut owner_align, mut status_size, mut status_align) = (0, 0, 0, 0);
    unsafe {
        dr_v1_windows_pipe_layout(
            &mut owner_size,
            &mut owner_align,
            &mut status_size,
            &mut status_align,
        )
    };
    assert_eq!(
        (owner_size, owner_align),
        (
            mem::size_of::<Pipe<WindowsDriver>>(),
            mem::align_of::<Pipe<WindowsDriver>>()
        )
    );
    assert_eq!(
        (status_size, status_align),
        (
            mem::size_of::<DrCompletionStatusV1>(),
            mem::align_of::<DrCompletionStatusV1>()
        )
    );
}

#[test]
fn private_abi_loopback_reuses_event_and_storage_without_replaying_bytes() {
    let (server, client) = connected_pair();
    let mut server = AbiOwner::adopt(server);
    let mut client = AbiOwner::adopt(client);
    let (server_event, client_event) = (server.event(), client.event());
    for event in [server_event, client_event] {
        let mut flags = u32::MAX;
        assert_ne!(unsafe { GetHandleInformation(event, &mut flags) }, 0);
        assert_eq!(flags & 1, 0, "cached completion event is inheritable");
    }
    let operation_budget = budget(2, 5_000_000_000);
    let (_, runtime_allocations) = crate::probe_runtime_allocations(|| {
        for input in [b"abc".as_slice(), b"defgh".as_slice()] {
            let mut status = DrCompletionStatusV1 {
                stop: 255,
                has_cancellation_failure: 1,
                transfer: DrIoStatusV1 {
                    system_code: -1,
                    has_system_code: 1,
                    ..DrIoStatusV1::default()
                },
                ..DrCompletionStatusV1::default()
            };
            assert_eq!(
                unsafe {
                    dr_v1_windows_pipe_write(
                        server.raw(),
                        input.as_ptr(),
                        input.len(),
                        &operation_budget,
                        ptr::null_mut(),
                        (&raw mut status).cast(),
                    )
                },
                0
            );
            assert_eq!(status.transfer.value, input.len() as u64);
            assert_eq!(
                (
                    status.stop,
                    status.has_cancellation_failure,
                    status.transfer.has_system_code
                ),
                (0, 0, 0)
            );
            let mut bytes = [0x55; 12];
            assert_eq!(
                unsafe {
                    dr_v1_windows_pipe_read(
                        client.raw(),
                        bytes[2..].as_mut_ptr(),
                        input.len(),
                        &operation_budget,
                        ptr::null_mut(),
                        (&raw mut status).cast(),
                    )
                },
                0
            );
            assert_eq!(status.transfer.value, input.len() as u64);
            assert_eq!(&bytes[2..2 + input.len()], input);
            assert!(bytes[..2]
                .iter()
                .chain(bytes[2 + input.len()..].iter())
                .all(|byte| *byte == 0x55));
            assert_eq!(
                (server.event(), client.event()),
                (server_event, client_event)
            );
        }
    });
    // This probe observes Doria runtime heap allocations, not kernel resources
    // or arbitrary allocations made by Windows itself.
    assert_eq!(runtime_allocations, 0);
    let mut status = DrCompletionStatusV1::default();
    assert_eq!(
        unsafe {
            dr_v1_windows_pipe_read(
                client.raw(),
                ptr::null_mut(),
                0,
                ptr::null(),
                ptr::null_mut(),
                (&raw mut status).cast(),
            )
        },
        0
    );
    assert_eq!(status.transfer.value, 0);
    let (_, closes) = crate::native_handle::with_close_probe(|| {
        server.close();
        let fresh_budget = budget(2, 5_000_000_000);
        let mut byte = [0x55];
        assert_eq!(
            unsafe {
                dr_v1_windows_pipe_read(
                    client.raw(),
                    byte.as_mut_ptr(),
                    byte.len(),
                    &fresh_budget,
                    ptr::null_mut(),
                    (&raw mut status).cast(),
                )
            },
            2
        );
        assert_eq!(byte, [0x55]);
        assert_eq!(
            (status.stop, status.transfer.has_transferred_byte_count),
            (0, 0)
        );
        client.close();
        drop(server);
        drop(client);
    });
    assert_eq!(
        closes, 4,
        "two data handles and two cached events close once"
    );
}

#[test]
fn wait_control_retains_shared_claim_and_never_resets_cancellation_signal() {
    let source = TestClaim::new();
    let requester = source.clone();
    let mut control = unsafe { WaitControl::new(None, source.state()).unwrap() };
    let signal = control.cancellation_signal().unwrap();
    drop(source);
    assert_eq!(
        control.next_wait(),
        Ok(Ok(Wait {
            timeout: INFINITE,
            immediate: false
        }))
    );
    assert_eq!(
        unsafe { WaitForSingleObject(signal as NativeHandle, 0) },
        258
    );
    requester.request();
    drop(requester);
    for _ in 0..4 {
        assert_eq!(control.next_wait(), Ok(Err(Stop::Cancellation)));
        assert_eq!(control.cancellation_signal(), Some(signal));
        assert_eq!(unsafe { WaitForSingleObject(signal as NativeHandle, 0) }, 0);
    }
    // Only the control's retained claim now owns the signal. No use follows drop.
}

#[test]
fn wait_control_reuses_real_inline_budgets_and_expiry_never_submits() {
    let immediate = budget(0, 0);
    let capped = budget(3, u64::MAX);
    let expired = budget(3, 0);
    for (budget, expected) in [
        (
            &immediate,
            Ok(Wait {
                timeout: 0,
                immediate: true,
            }),
        ),
        (
            &capped,
            Ok(Wait {
                timeout: INFINITE - 1,
                immediate: false,
            }),
        ),
        (&expired, Err(Stop::BudgetExhausted)),
    ] {
        let mut control = unsafe { WaitControl::new(Some(budget), ptr::null_mut()).unwrap() };
        for _ in 0..3 {
            assert_eq!(control.next_wait(), Ok(expected));
            assert_eq!(
                control.budget.unwrap().remaining(1, u64::MAX, || Ok(0)),
                budget.remaining(1, u64::MAX, || Ok(0))
            );
        }
    }
    let (server, client) = connected_pair();
    let mut owner = AbiOwner::adopt(server);
    let mut bytes = [0x55; 4];
    let mut status = DrCompletionStatusV1::default();
    assert_eq!(
        unsafe {
            dr_v1_windows_pipe_read(
                owner.raw(),
                bytes.as_mut_ptr(),
                bytes.len(),
                &expired,
                ptr::null_mut(),
                (&raw mut status).cast(),
            )
        },
        4
    );
    assert_eq!(status.stop, 2);
    assert_eq!(bytes, [0x55; 4]);
    assert_eq!(status.transfer.has_transferred_byte_count, 0);
    owner.close();
    drop(client);
}

#[test]
fn actual_pending_read_cancellation_drains_before_buffer_and_owner_reuse() {
    let (server, client) = connected_pair();
    let source = TestClaim::new();
    let token = source.clone();
    let (pending_tx, pending_rx) = mpsc::sync_channel(1);
    let (completed_tx, completed_rx) = mpsc::sync_channel(1);
    std::thread::scope(|scope| {
        let worker = scope.spawn(move || {
            let driver = unsafe { WindowsDriver::new(server.into_raw_handle()).unwrap() };
            let event = driver.completion.get();
            let mut pipe = Pipe {
                driver: Observed {
                    driver,
                    pending: pending_tx,
                    notified: false,
                },
            };
            let budget = budget(2, 5_000_000_000);
            let mut control = unsafe { WaitControl::new(Some(&budget), token.state()).unwrap() };
            drop(token);
            let mut bytes = [0x55; 8];
            let address = bytes.as_ptr();
            let cancelled = pipe.transfer(Buffer::Read(&mut bytes), &mut control);
            completed_tx.send(cancelled).unwrap();
            drop(control);
            let mut control = unsafe { WaitControl::new(Some(&budget), ptr::null_mut()).unwrap() };
            let read = pipe.transfer(Buffer::Read(&mut bytes), &mut control);
            assert_eq!(bytes.as_ptr(), address);
            assert_eq!(pipe.driver.driver.completion.get(), event);
            (read, bytes)
        });
        let observed = pending_rx.recv_timeout(TEST_WAIT);
        source.request();
        let cancelled = completed_rx
            .recv_timeout(TEST_WAIT)
            .expect("cancelled read did not drain");
        assert!(observed.is_ok(), "fixture never observed ERROR_IO_PENDING");
        assert_eq!(cancelled.stop, Some(Stop::Cancellation));
        let failure = cancelled.transfer.unwrap().unwrap_err();
        assert_eq!(failure.system_code, Some(995));
        assert_eq!(failure.transferred_byte_count, None);
        let mut client = AbiOwner::adopt(client);
        let mut status = DrCompletionStatusV1::default();
        let budget = budget(2, 5_000_000_000);
        assert_eq!(
            unsafe {
                dr_v1_windows_pipe_write(
                    client.raw(),
                    b"reuse".as_ptr(),
                    5,
                    &budget,
                    ptr::null_mut(),
                    (&raw mut status).cast(),
                )
            },
            0
        );
        let (read, bytes) = worker.join().unwrap();
        assert_eq!(read, Awaited::complete(Ok(5)));
        assert_eq!(&bytes[..5], b"reuse");
        assert_eq!(&bytes[5..], &[0x55; 3]);
        client.close();
    });
}

#[test]
fn actual_pending_write_completes_after_peer_drains_exact_borrowed_bytes() {
    let (server, client) = connected_pair();
    let mut quota = 0;
    assert_ne!(
        unsafe {
            GetNamedPipeInfo(
                server.as_raw_handle(),
                ptr::null_mut(),
                &mut quota,
                ptr::null_mut(),
                ptr::null_mut(),
            )
        },
        0
    );
    assert!(
        (1..=1_048_576).contains(&quota),
        "unexpected fixture pipe quota: {quota}"
    );
    let length = quota as usize + 4096;
    let (pending_tx, pending_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    std::thread::scope(|scope| {
        let worker = scope.spawn(move || {
            let driver = unsafe { WindowsDriver::new(server.into_raw_handle()).unwrap() };
            let mut pipe = Pipe {
                driver: Observed {
                    driver,
                    pending: pending_tx,
                    notified: false,
                },
            };
            let bytes: Vec<_> = (0..length).map(|index| (index % 251) as u8).collect();
            let address = bytes.as_ptr();
            let budget = budget(2, 5_000_000_000);
            let mut control = unsafe { WaitControl::new(Some(&budget), ptr::null_mut()).unwrap() };
            let written = pipe.transfer(Buffer::Write(&bytes), &mut control);
            assert_eq!(bytes.as_ptr(), address);
            assert!(bytes
                .iter()
                .enumerate()
                .all(|(index, byte)| *byte == (index % 251) as u8));
            // Keep the server alive until its peer has consumed every accepted
            // byte; close is not a substitute for peer-drain synchronization.
            release_rx.recv_timeout(TEST_WAIT).unwrap();
            written
        });
        let observed = pending_rx.recv_timeout(TEST_WAIT);
        let mut client = AbiOwner::adopt(client);
        let budget = budget(2, 5_000_000_000);
        let mut received = 0;
        let mut bytes = [0; 4096];
        while received < length {
            let requested = (length - received).min(bytes.len());
            let mut status = DrCompletionStatusV1::default();
            assert_eq!(
                unsafe {
                    dr_v1_windows_pipe_read(
                        client.raw(),
                        bytes.as_mut_ptr(),
                        requested,
                        &budget,
                        ptr::null_mut(),
                        (&raw mut status).cast(),
                    )
                },
                0
            );
            let count = status.transfer.value as usize;
            assert!((1..=requested).contains(&count));
            assert!(bytes[..count]
                .iter()
                .enumerate()
                .all(|(index, byte)| *byte == ((received + index) % 251) as u8));
            received += count;
        }
        release_tx.send(()).unwrap();
        assert_eq!(worker.join().unwrap(), Awaited::complete(Ok(length)));
        assert!(
            observed.is_ok(),
            "fixture never observed a quota-blocked pending write"
        );
        client.close();
    });
}

unsafe extern "system" {
    fn GetHandleInformation(handle: NativeHandle, flags: *mut u32) -> i32;
    fn CreateNamedPipeW(
        name: *const u16,
        access: u32,
        mode: u32,
        instances: u32,
        output: u32,
        input: u32,
        timeout: u32,
        attributes: *mut c_void,
    ) -> NativeHandle;
    fn ConnectNamedPipe(handle: NativeHandle, request: *mut c_void) -> i32;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        sharing: u32,
        attributes: *mut c_void,
        creation: u32,
        flags: u32,
        template: NativeHandle,
    ) -> NativeHandle;
    fn GetNamedPipeInfo(
        handle: NativeHandle,
        flags: *mut u32,
        output: *mut u32,
        input: *mut u32,
        instances: *mut u32,
    ) -> i32;
}

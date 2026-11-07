//! Native private transport for explicitly owned, overlapped byte-mode pipes.
//! Adoption requires creation provenance; it never converts synchronous handles.

use core::mem;
use core::ptr;

use super::*;
use crate::cancellation::{self, DrCancellationV1};
use crate::io_status::DrIoStatusV1;
use crate::native_handle::{NativeHandle, OwnedHandle};
use crate::platform_io::{last_failure, Reason};
use crate::transfer_group::DrCompletionStatusV1;
use crate::wait_budget::{self, DrWaitBudgetV1};
use crate::win32::{
    wait_for_multiple_objects, wait_for_single_object, CancelIoEx, CreateEventW, GetLastError,
    GetOverlappedResult, ReadFile, WriteFile, INFINITE,
};

const IO_PENDING: u32 = 997;
const IO_INCOMPLETE: u32 = 996;
const STATUS_PENDING: usize = 0x103;

mod adapter;
mod broker;
pub(crate) mod pending;
pub(crate) use adapter::{AdapterGroup as NativeAdapterGroup, AdapterSet as NativeAdapterSet};
pub(crate) use broker::transport::{NativeTransferGroup, NativeTransferSet};
pub(crate) use pending::{
    PendingGroup as NativePendingGroup, PendingSet as NativePendingSet,
    PendingSetup as NativePendingSetup,
};
#[cfg(test)]
mod tests;

struct WindowsDriver {
    handle: OwnedHandle,
    completion: OwnedHandle,
}

impl WindowsDriver {
    /// Consumes ownership even if creating the completion event fails.
    /// Caller supplies an idle overlapped byte-mode PIPE_WAIT handle, with no
    /// completion-port association, independent I/O, callback or close consumer.
    unsafe fn new(handle: NativeHandle) -> Result<Self, Failure> {
        let handle = OwnedHandle::from_raw(handle);
        let event = CreateEventW(ptr::null_mut(), 1, 0, ptr::null());
        if event.is_null() {
            Err(last_failure())
        } else {
            Ok(Self {
                handle,
                completion: OwnedHandle::from_raw(event),
            })
        }
    }

    fn close(self) -> Result<(), Failure> {
        let data = self.handle.close();
        let event = self.completion.close();
        data.and(event)
    }

    unsafe fn result(&mut self, request: *mut Overlapped, wait: bool) -> Query {
        let mut count = 0;
        if GetOverlappedResult(
            self.handle.get(),
            request.cast(),
            &mut count,
            i32::from(wait),
        ) != 0
        {
            return Query::Terminal(Ok(count as usize));
        }
        let code = GetLastError();
        if code == IO_INCOMPLETE || code == IO_PENDING {
            return Query::Pending;
        }
        // An API failure is not terminal proof when the request still belongs
        // to the kernel. Internal is observed only to preserve that obligation;
        // counts always come from the completion API, never InternalHigh.
        if ptr::read_volatile((*request).internal.get()) == STATUS_PENDING {
            return Query::PendingFailure(
                IoMessageOperation::Wait,
                Failure::from_system_code(code.into()),
            );
        }
        Query::Terminal(Err(Failure::from_system_code(code.into())
            .with_transferred_byte_count((count > 0).then_some(count as usize))))
    }
}

// The handle/event are independently owned. No request escapes transfer(), and
// the exclusive owner borrow excludes another transfer or close during a wait.
unsafe impl Driver for WindowsDriver {
    fn event(&self) -> *mut c_void {
        self.completion.get()
    }

    unsafe fn submit(&mut self, request: *mut Overlapped, buffer: &mut Buffer<'_>) -> Submission {
        let success = match buffer {
            Buffer::Read(bytes) => ReadFile(
                self.handle.get(),
                bytes.as_mut_ptr().cast(),
                request_length(bytes.len()),
                ptr::null_mut(),
                request.cast(),
            ),
            Buffer::Write(bytes) => WriteFile(
                self.handle.get(),
                bytes.as_ptr().cast(),
                request_length(bytes.len()),
                ptr::null_mut(),
                request.cast(),
            ),
        };
        if success != 0 {
            Submission::Complete
        } else {
            let code = GetLastError();
            if code == IO_PENDING || ptr::read_volatile((*request).internal.get()) == STATUS_PENDING
            {
                Submission::Pending
            } else {
                Submission::Failed(Failure::from_system_code(code.into()))
            }
        }
    }

    unsafe fn query(&mut self, request: *mut Overlapped) -> Query {
        self.result(request, false)
    }

    fn wait(&mut self, timeout: u32, cancellation: Option<usize>) -> Result<Wake, Failure> {
        let handles = [
            self.completion.get(),
            cancellation.unwrap_or(0) as NativeHandle,
        ];
        let count = if cancellation.is_some() { 2 } else { 1 };
        match unsafe {
            wait_for_multiple_objects(
                count,
                handles.as_ptr(),
                0,
                timeout,
                "an event-driver request's completion event or its cancellation signal",
            )
        } {
            0 => Ok(Wake::Completion),
            1 if count == 2 => Ok(Wake::Cancellation),
            258 => Ok(Wake::Timeout),
            u32::MAX => Err(last_failure()),
            _ => Err(Failure::other()),
        }
    }

    unsafe fn cancel(&mut self, request: *mut Overlapped) -> Result<(), Failure> {
        if CancelIoEx(self.handle.get(), request.cast()) != 0 {
            Ok(())
        } else {
            Err(last_failure())
        }
    }

    unsafe fn drain(&mut self, request: *mut Overlapped) -> Result<usize, Failure> {
        loop {
            match self.result(request, true) {
                Query::Terminal(result) => return result,
                Query::Pending | Query::PendingFailure(_, _) => {
                    // Keep every loan live even if querying completion failed.
                    // This owned manual-reset event cannot be closed by another
                    // caller. A failed wait on it is a broken runtime invariant,
                    // not permission to release an outstanding kernel request.
                    if wait_for_single_object(
                        self.completion.get(),
                        INFINITE,
                        "a cancelled event-driver request to finish",
                    ) != 0
                    {
                        crate::runtime_invariant_panic();
                    }
                }
            }
        }
    }
}

use crate::cancellation::CancellationClaim;

fn project_completion(
    status: &mut DrCompletionStatusV1,
    mut awaited: Awaited,
    reading: bool,
) -> u8 {
    let eof = reading
        && awaited.transfer.is_some_and(|result| {
            matches!(result,
                Err(failure) if failure.system_code == Some(109)
                    && !failure.transferred_byte_count.is_some_and(|n| n > 0)
            )
        });
    if eof {
        awaited.transfer = Some(Ok(0));
    }
    let tag = status.project(awaited, reading);
    if eof {
        2
    } else {
        tag
    }
}

struct WaitControl {
    budget: Option<DrWaitBudgetV1>,
    claim: Option<CancellationClaim>,
    signal: Option<usize>,
}

impl WaitControl {
    unsafe fn new(
        budget: Option<&DrWaitBudgetV1>,
        cancellation_state: *mut DrCancellationV1,
    ) -> Result<Self, Failure> {
        let claim = CancellationClaim::acquire(cancellation_state)?;
        let mut control = Self {
            budget: budget.copied(),
            claim,
            signal: None,
        };
        if let Some(claim) = &control.claim {
            let (mut signal, mut reason, mut code, mut has_code) = (0, 0, 0, 0);
            if cancellation::dr_v1_cancellation_signal(
                claim.0,
                &mut signal,
                &mut reason,
                &mut code,
                &mut has_code,
            ) != 0
            {
                return Err(private_failure(reason, code, has_code));
            }
            control.signal = Some(signal);
        }
        Ok(control)
    }
}

impl Control for WaitControl {
    fn next_wait(&mut self) -> Result<Result<Wait<u32>, Stop>, Failure> {
        if self.claim.as_ref().is_some_and(|claim| unsafe {
            cancellation::dr_v1_cancellation_is_requested(claim.0) != 0
        }) {
            return Ok(Err(Stop::Cancellation));
        }
        let Some(budget) = self.budget else {
            return Ok(Ok(Wait {
                timeout: INFINITE,
                immediate: false,
            }));
        };
        let (mut kind, mut units, mut reason, mut code, mut has_code) = (0, 0, 0, 0, 0);
        if unsafe {
            wait_budget::dr_v1_wait_budget_remaining(
                &budget,
                1_000_000,
                u64::from(INFINITE - 1),
                &mut kind,
                &mut units,
                &mut reason,
                &mut code,
                &mut has_code,
            )
        } != 0
        {
            return Err(private_failure(reason, code, has_code));
        }
        match kind {
            0 => Ok(Ok(Wait {
                timeout: 0,
                immediate: true,
            })),
            1 => Ok(Ok(Wait {
                timeout: INFINITE,
                immediate: false,
            })),
            2 => Ok(Ok(Wait {
                timeout: units as u32,
                immediate: false,
            })),
            3 => Ok(Err(Stop::BudgetExhausted)),
            _ => unsafe { crate::runtime_invariant_panic() },
        }
    }

    fn cancellation_signal(&self) -> Option<usize> {
        self.signal
    }
}

fn private_failure(reason: u8, code: i64, has_code: u8) -> Failure {
    // These outputs come only from the existing runtime-owned ABI calls above.
    let reason = match reason {
        0 => Reason::NotFound,
        1 => Reason::PermissionDenied,
        2 => Reason::InvalidInput,
        3 => Reason::Interrupted,
        4 => Reason::ResourceExhausted,
        5 => Reason::Unsupported,
        6 => Reason::Closed,
        7 => Reason::Other,
        _ => unsafe { crate::runtime_invariant_panic() },
    };
    Failure {
        reason,
        system_code: (has_code != 0).then_some(code),
        transferred_byte_count: None,
    }
}

/// # Safety
/// Outputs must be disjoint writable usize storage. This opaque inline owner
/// has no outstanding operation between calls and may then move normally.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_windows_pipe_layout(
    owner_size: *mut usize,
    owner_align: *mut usize,
    status_size: *mut usize,
    status_align: *mut usize,
) {
    *owner_size = mem::size_of::<Pipe<WindowsDriver>>();
    *owner_align = mem::align_of::<Pipe<WindowsDriver>>();
    *status_size = mem::size_of::<DrCompletionStatusV1>();
    *status_align = mem::align_of::<DrCompletionStatusV1>();
}

/// # Safety
/// `handle` transfers one independently owned, idle FILE_FLAG_OVERLAPPED, byte-mode,
/// PIPE_WAIT pipe handle. Its creator must establish that provenance; inherited
/// synchronous handles, files and consoles are not accepted by this contract.
/// The handle must not be associated with a completion port or have an external
/// I/O, callback or close consumer. Event completion alone cannot drain queued
/// IOCP packets retaining a request address; that is a separate driver contract.
/// Owner must be uninitialized storage of the queried layout, disjoint from
/// writable status. Failure still consumes and closes the incoming handle.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_windows_pipe_adopt(
    owner: *mut c_void,
    handle: NativeHandle,
    status: *mut DrIoStatusV1,
) -> u8 {
    *status = DrIoStatusV1::default();
    (*status).complete(WindowsDriver::new(handle).map(|driver| {
        owner.cast::<Pipe<WindowsDriver>>().write(Pipe { driver });
        0
    }))
}

unsafe fn transfer(
    owner: *mut c_void,
    buffer: Buffer<'_>,
    budget: *const DrWaitBudgetV1,
    cancellation_state: *mut DrCancellationV1,
    status: *mut c_void,
    reading: bool,
) -> u8 {
    let status = &mut *status.cast::<DrCompletionStatusV1>();
    let awaited = if buffer.len() == 0 {
        Awaited::complete(Ok(0))
    } else {
        match WaitControl::new(budget.as_ref(), cancellation_state) {
            Ok(mut control) => {
                (&mut *owner.cast::<Pipe<WindowsDriver>>()).transfer(buffer, &mut control)
            }
            Err(failure) => Awaited {
                transfer: None,
                stop: Some(Stop::Failure(IoMessageOperation::Wait, failure)),
                cancellation_failure: None,
            },
        }
    };
    project_completion(status, awaited, reading)
}

/// Submits one read and retains every borrow until terminal collection/drain.
/// # Safety
/// Owner must be live/exclusively borrowed; destination is writable for capacity
/// bytes and borrowed until return. Budget is null (untimed) or an initialized
/// borrowed budget. Cancellation is null or a live state during claim acquisition.
/// Status is writable queried completion-status storage; all regions are disjoint.
/// Return tags: 0 data, 2 EOF, 3 terminal failure, 4 not submitted; wait facts are
/// separate. No public timeout/cancellation precedence is encoded by these tags.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_windows_pipe_read(
    owner: *mut c_void,
    destination: *mut u8,
    capacity: usize,
    budget: *const DrWaitBudgetV1,
    cancellation_state: *mut DrCancellationV1,
    status: *mut c_void,
) -> u8 {
    let buffer = if capacity == 0 {
        &mut []
    } else {
        core::slice::from_raw_parts_mut(destination, capacity)
    };
    transfer(
        owner,
        Buffer::Read(buffer),
        budget,
        cancellation_state,
        status,
        true,
    )
}

/// Submits one write, preserving the original region and accepted count.
/// # Safety
/// As for read, except source is a readonly region of length bytes. Return tags:
/// 0 accepted, 3 terminal failure, 4 not submitted. No suffix copy or hidden buffer.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_windows_pipe_write(
    owner: *mut c_void,
    source: *const u8,
    length: usize,
    budget: *const DrWaitBudgetV1,
    cancellation_state: *mut DrCancellationV1,
    status: *mut c_void,
) -> u8 {
    let buffer = if length == 0 {
        &[]
    } else {
        core::slice::from_raw_parts(source, length)
    };
    transfer(
        owner,
        Buffer::Write(buffer),
        budget,
        cancellation_state,
        status,
        false,
    )
}

/// # Safety
/// Consumes a live, unborrowed owner; its storage then becomes uninitialized.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_windows_pipe_close(
    owner: *mut c_void,
    status: *mut DrIoStatusV1,
) -> u8 {
    *status = DrIoStatusV1::default();
    let pipe = ptr::read(owner.cast::<Pipe<WindowsDriver>>());
    (*status).complete(pipe.driver.close().map(|()| 0))
}

/// # Safety
/// Consumes a live, unborrowed owner exactly once without a checked close result.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_windows_pipe_drop(owner: *mut c_void) {
    ptr::drop_in_place(owner.cast::<Pipe<WindowsDriver>>());
}

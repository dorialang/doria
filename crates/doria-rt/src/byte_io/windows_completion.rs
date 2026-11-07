//! Private Windows completion ownership. Single transfers keep their request
//! in a pinned call scope; a transfer group retains all participating buffer
//! loans and stable reusable slots until every completion packet is retired.

use core::ffi::c_void;
use core::pin::Pin;
use doria_diagnostic_catalogue::IoMessageOperation;

use crate::platform_io::Failure;
use crate::transfer_group::{Awaited, Buffer, Control, Query, Stop, Submission, Wait, Wake};
use crate::win32::Overlapped;

#[cfg(windows)]
mod native;
#[cfg(windows)]
pub(crate) use native::{
    NativeAdapterGroup, NativeAdapterSet, NativePendingGroup, NativePendingSet, NativePendingSetup,
    NativeTransferGroup, NativeTransferSet,
};
#[cfg(test)]
mod tests;
// Exercise the exact value-only router on the host too. These are serialized
// model interleavings, not execution evidence for Windows FFI or synchronization.
#[cfg(all(test, not(windows)))]
#[path = "windows_completion/native/broker/routing.rs"]
mod routing_model;

fn request_length(length: usize) -> u32 {
    length.min(u32::MAX as usize) as u32
}

/// # Safety
/// Submission must not retain any storage after a terminal submission failure.
/// Query may report terminal only after the kernel releases the request/buffer.
/// Drain must not unwind or return before terminal completion, even when cancel
/// fails. The driver retains its handle and event through every pending request.
/// No completion packet, callback or external consumer may retain the request
/// after a terminal result; an event driver must exclude external IOCP consumers.
unsafe trait Driver {
    fn event(&self) -> *mut c_void;
    unsafe fn submit(&mut self, request: *mut Overlapped, buffer: &mut Buffer<'_>) -> Submission;
    unsafe fn query(&mut self, request: *mut Overlapped) -> Query;
    fn wait(&mut self, timeout: u32, cancellation: Option<usize>) -> Result<Wake, Failure>;
    unsafe fn cancel(&mut self, request: *mut Overlapped) -> Result<(), Failure>;
    unsafe fn drain(&mut self, request: *mut Overlapped) -> Result<usize, Failure>;
}

struct Pipe<D: Driver> {
    driver: D,
}

impl<D: Driver> Pipe<D> {
    fn transfer(&mut self, buffer: Buffer<'_>, control: &mut impl Control) -> Awaited {
        if buffer.len() == 0 {
            return Awaited::complete(Ok(0));
        }
        match control.next_wait() {
            Ok(Ok(_)) => {}
            Ok(Err(stop)) => {
                return Awaited {
                    transfer: None,
                    stop: Some(stop),
                    cancellation_failure: None,
                };
            }
            Err(failure) => {
                return Awaited {
                    transfer: None,
                    stop: Some(Stop::Failure(IoMessageOperation::Wait, failure)),
                    cancellation_failure: None,
                };
            }
        }
        let request = core::pin::pin!(Overlapped::new(self.driver.event()));
        let mut pending = Pending {
            driver: &mut self.driver,
            request,
            buffer,
            active: false,
        };
        // Arm cleanup before the FFI call can start using caller storage.
        pending.active = true;
        let raw = pending.raw_request();
        match unsafe { pending.driver.submit(raw, &mut pending.buffer) } {
            Submission::Failed(failure) => {
                pending.active = false;
                return Awaited::complete(Err(failure));
            }
            Submission::Complete | Submission::Pending => {}
        }
        loop {
            match unsafe { pending.driver.query(raw) } {
                Query::Terminal(result) => return Awaited::complete(pending.finish(result)),
                Query::PendingFailure(operation, failure) => {
                    return pending.stop(Stop::Failure(operation, failure));
                }
                Query::Pending => {}
            }
            let wait = match control.next_wait() {
                Ok(Ok(wait)) => wait,
                Ok(Err(stop)) => return pending.stop(stop),
                Err(failure) => {
                    return pending.stop(Stop::Failure(IoMessageOperation::Wait, failure))
                }
            };
            match pending
                .driver
                .wait(wait.timeout, control.cancellation_signal())
            {
                Ok(Wake::Timeout) if wait.immediate => {
                    return pending.stop(Stop::BudgetExhausted);
                }
                Ok(Wake::Completion | Wake::Timeout) => {}
                Ok(Wake::Cancellation) => return pending.stop(Stop::Cancellation),
                Err(failure) => {
                    return pending.stop(Stop::Failure(IoMessageOperation::Wait, failure))
                }
            }
        }
    }
}

struct Pending<'a, 'r, D: Driver> {
    driver: &'a mut D,
    request: Pin<&'r mut Overlapped>,
    // Retaining the actual borrow, not just its raw pointer, guards host cleanup.
    buffer: Buffer<'a>,
    active: bool,
}

impl<D: Driver> Pending<'_, '_, D> {
    fn raw_request(&mut self) -> *mut Overlapped {
        // The pointee remains pinned in Pipe::transfer until this guard drops.
        unsafe { self.request.as_mut().get_unchecked_mut() }
    }

    fn finish(&mut self, result: Result<usize, Failure>) -> Result<usize, Failure> {
        self.active = false;
        match result {
            Ok(count) if count <= request_length(self.buffer.len()) as usize => Ok(count),
            Ok(_) => Err(Failure::other()),
            Err(failure) => Err(
                if failure
                    .transferred_byte_count
                    .is_some_and(|count| count > request_length(self.buffer.len()) as usize)
                {
                    failure.with_transferred_byte_count(None)
                } else {
                    failure
                },
            ),
        }
    }

    fn stop(&mut self, stop: Stop) -> Awaited {
        let raw = self.raw_request();
        let cancellation_failure = unsafe { self.driver.cancel(raw) }
            .err()
            .map(|failure| (IoMessageOperation::Wait, failure));
        let terminal = unsafe { self.driver.drain(raw) };
        Awaited {
            transfer: Some(self.finish(terminal)),
            stop: Some(stop),
            cancellation_failure,
        }
    }
}

impl<D: Driver> Drop for Pending<'_, '_, D> {
    fn drop(&mut self) {
        if self.active {
            let raw = self.raw_request();
            let _ = unsafe { self.driver.cancel(raw) };
            let _ = unsafe { self.driver.drain(raw) };
            self.active = false;
        }
    }
}

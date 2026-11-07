//! Test-authored functions that the runtime calls through a non-unwinding ABI.
//!
//! Tests hand the runtime adapter, stream, drop and thread callbacks declared
//! `extern "C"` or `extern "system"`. A panic cannot leave such a function:
//! Rust aborts the process at the boundary. [`guard`] runs a callback body and
//! makes that abort explicit. `crate::test_report` has already reported the
//! panic, with the test's name, location and message, past libtest's capture,
//! which an abort would otherwise discard. Continuing is not sound: the runtime
//! is in the middle of an operation whose calling contract just failed, it
//! still holds raw pointers into the fixture, and the callback has no honest
//! status to return.

use std::cell::Cell;
use std::panic::{self, AssertUnwindSafe};

std::thread_local! {
    /// How many guarded callback bodies this thread is inside.
    static DEPTH: Cell<usize> = const { Cell::new(0) };
}

/// Runs the body of a test callback that the runtime entered through a
/// non-unwinding ABI. A panic in `body` is reported and aborts the test
/// binary; see the module documentation.
pub(crate) fn guard<R>(body: impl FnOnce() -> R) -> R {
    crate::test_report::install();
    DEPTH.with(|depth| depth.set(depth.get() + 1));
    let result = panic::catch_unwind(AssertUnwindSafe(body));
    DEPTH.with(|depth| depth.set(depth.get() - 1));
    match result {
        Ok(value) => value,
        Err(_) => std::process::abort(),
    }
}

/// Whether this thread is inside a guarded callback body.
pub(crate) fn inside() -> bool {
    DEPTH.with(Cell::get) > 0
}

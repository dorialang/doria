//! Test-authored functions that the runtime calls through a non-unwinding ABI.
//!
//! Tests hand the runtime adapter, stream, drop and thread callbacks declared
//! `extern "C"` or `extern "system"`. A panic cannot leave such a function:
//! Rust aborts the process at the boundary. Inside the libtest harness that
//! abort also discards the test's captured output, including the panic's
//! location and message. CI then sees only SIGABRT, or `0xc0000409` on
//! Windows, with no test name, and every later test is lost with it.
//!
//! [`guard`] runs a callback body. If the body panics, the panic is reported
//! straight to the process's standard error, past libtest's capture, naming the
//! thread (libtest names it after the test), the location and the message.
//! Then the process aborts. Continuing is not sound: the runtime is in the
//! middle of an operation whose calling contract just failed, it still holds
//! raw pointers into the fixture, and the callback has no honest status to
//! return.

use std::cell::Cell;
use std::io::Write;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Once;

std::thread_local! {
    /// How many guarded callback bodies this thread is inside.
    static DEPTH: Cell<usize> = const { Cell::new(0) };
}

/// Runs the body of a test callback that the runtime entered through a
/// non-unwinding ABI. A panic in `body` is reported uncaptured and aborts the
/// test binary; see the module documentation.
pub(crate) fn guard<R>(body: impl FnOnce() -> R) -> R {
    install_reporter();
    DEPTH.with(|depth| depth.set(depth.get() + 1));
    let result = panic::catch_unwind(AssertUnwindSafe(body));
    DEPTH.with(|depth| depth.set(depth.get() - 1));
    match result {
        Ok(value) => value,
        Err(_) => std::process::abort(),
    }
}

/// Chains a panic hook that, for a panic inside a guarded callback, writes the
/// report to the real standard error. Other panics go only to the previous
/// hook, so tests that expect a panic keep their ordinary captured output.
fn install_reporter() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            if DEPTH.with(Cell::get) > 0 {
                let thread = std::thread::current();
                let name = thread.name().unwrap_or("<unnamed>");
                let _ = writeln!(
                    std::io::stderr().lock(),
                    "\nA test callback panicked in thread '{name}' ({:?}), {info}\n\
                     The runtime called it through a non-unwinding ABI, so the test \
                     binary aborts.",
                    thread.id(),
                );
            }
            previous(info);
        }));
    });
}

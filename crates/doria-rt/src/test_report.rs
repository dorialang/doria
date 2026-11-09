//! Reports a test's failure when it happens, not when the run ends.
//!
//! libtest captures each test's output and prints a failure's message only in
//! the summary after every test has finished. A later hang, abort or CI
//! timeout therefore loses every earlier failure's message and location. On
//! Windows CI that left only test names marked FAILED.
//!
//! This module installs a panic hook when the test binary starts. Every panic
//! is reported at once to the process's standard error, past libtest's
//! capture. The report names the thread (libtest names it after the test), the
//! location and the message. The previous hook still runs, so libtest's own
//! summary is unchanged. A test that panics on purpose wraps that code in
//! [`expect_panic`], which suppresses the immediate report.
//!
//! [`wait_expired`] reports a test-build wait that outlived its bound; see
//! `crate::win32::TEST_WAIT_LIMIT_MILLISECONDS`.

use std::cell::Cell;
use std::io::Write;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Once;

std::thread_local! {
    /// How many `expect_panic` bodies this thread is inside.
    static EXPECTED: Cell<usize> = const { Cell::new(0) };
}

/// Installs the reporter before `main`. Each platform's loader runs these
/// entries before any test thread starts.
#[used]
#[cfg_attr(all(windows, target_env = "msvc"), link_section = ".CRT$XCU")]
#[cfg_attr(
    any(target_os = "linux", target_os = "android", target_os = "freebsd"),
    link_section = ".init_array"
)]
#[cfg_attr(target_vendor = "apple", link_section = "__DATA,__mod_init_func")]
static INSTALL_AT_START: extern "C" fn() = install_at_start;

extern "C" fn install_at_start() {
    crate::test_callback::guard(install);
}

/// Installs the reporter once. Also called lazily, for a platform whose loader
/// has no entry above.
pub(crate) fn install() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            if EXPECTED.with(Cell::get) == 0 {
                let ending = if crate::test_callback::inside() {
                    "\nThe runtime called this test callback through a non-unwinding ABI, \
                     so the test binary aborts."
                } else {
                    ""
                };
                report(&format!("panicked: {info}{ending}"));
            }
            previous(info);
        }));
    });
}

/// Runs `body`, which a test expects to panic, and returns its result the way
/// `catch_unwind` does. Its panic is not reported at once.
pub(crate) fn expect_panic<R>(body: impl FnOnce() -> R) -> std::thread::Result<R> {
    install();
    EXPECTED.with(|depth| depth.set(depth.get() + 1));
    let result = panic::catch_unwind(AssertUnwindSafe(body));
    EXPECTED.with(|depth| depth.set(depth.get() - 1));
    result
}

/// A test-build wait for native completion outlived its bound. The kernel may
/// still own storage the waiter borrowed, so this reports the test and what it
/// waited for, then aborts instead of unwinding.
#[cfg(windows)]
pub(crate) fn wait_expired(waiting_for: &str, milliseconds: u32) -> ! {
    report(&format!(
        "waited {milliseconds} ms for {waiting_for}, longer than any test may wait. \
         The kernel may still own storage this wait borrows, so the test binary aborts."
    ));
    std::process::abort()
}

fn report(what: &str) {
    let thread = std::thread::current();
    let name = thread.name().unwrap_or("<unnamed>");
    let _ = writeln!(
        std::io::stderr().lock(),
        "\n[doria-rt test report] thread '{name}' ({:?}) {what}",
        thread.id(),
    );
}

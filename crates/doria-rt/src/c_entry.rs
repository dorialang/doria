//! C entries into Doria and the Call Path through foreign frames (Decision
//! 0148 section 13).
//!
//! A program with C-ABI Doria functions records the thread that runs its
//! Doria code when it starts (`dr_v1_record_doria_thread`). Each outbound
//! foreign call stores the calling Doria frame in `dr_v1_foreign_call_frame`
//! for the length of the call, restoring the previous value afterwards, so
//! a nested call inside a callback leaves its enclosing call's record in
//! place. The C entry of a C-ABI function calls `dr_v1_enter_c_function`,
//! which panics with P1706 on any other thread and otherwise links a
//! `[foreign call]` frame to the recorded caller, so the Call Path stays
//! continuous through C. An entry with no outbound call in progress starts
//! a new chain.
//!
//! Only the recorded thread runs Doria code and its foreign calls, and the
//! entry checks the thread before it reads the record, so the record is
//! written and read by that thread alone.

use core::ptr;
use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

use crate::{panic_catalogued, DrStackFrameV2};

const FOREIGN_THREAD: &[u8] = b"P1706";

/// The function name of the frame that stands for the foreign code between
/// an outbound call and the C entry it led to.
static FOREIGN_CALL: &[u8] = b"[foreign call]";

/// The Doria frame of the outbound foreign call in progress, or null.
/// Generated code stores to it directly around every foreign call: one store
/// before the call and one after.
#[no_mangle]
#[allow(non_upper_case_globals)]
pub static dr_v1_foreign_call_frame: AtomicPtr<DrStackFrameV2> = AtomicPtr::new(ptr::null_mut());

/// The identity of the thread running Doria code, 0 until it is recorded.
static DORIA_THREAD: AtomicUsize = AtomicUsize::new(0);

#[cfg(unix)]
extern "C" {
    fn pthread_self() -> usize;
}

fn current_thread() -> usize {
    #[cfg(unix)]
    {
        unsafe { pthread_self() }
    }
    #[cfg(windows)]
    {
        unsafe { crate::win32::GetCurrentThreadId() as usize }
    }
    #[cfg(not(any(unix, windows)))]
    {
        0
    }
}

/// Records the calling thread as the thread running Doria code. The process
/// entry of a program with C-ABI functions calls it before any Doria code.
#[no_mangle]
pub extern "C" fn dr_v1_record_doria_thread() {
    DORIA_THREAD.store(current_thread(), Ordering::Relaxed);
}

/// Enters a C-ABI Doria function from C. Panics with P1706, reported at
/// `entry`, when the calling thread is not the thread running Doria code.
/// Otherwise returns the parent frame of the function's own frame: `marker`,
/// filled as the `[foreign call]` frame of the outbound call in progress, or
/// null when no outbound call is in progress.
///
/// # Safety
///
/// `marker` points to writable storage for one frame that outlives the
/// entered function, and `entry` to a valid frame that describes the
/// function.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_enter_c_function(
    marker: *mut DrStackFrameV2,
    entry: *const DrStackFrameV2,
) -> *const DrStackFrameV2 {
    let thread = DORIA_THREAD.load(Ordering::Relaxed);
    if thread == 0 || thread != current_thread() {
        panic_catalogued(entry, FOREIGN_THREAD);
    }
    let caller = dr_v1_foreign_call_frame.load(Ordering::Relaxed);
    if caller.is_null() {
        return ptr::null();
    }
    marker.write(DrStackFrameV2 {
        parent: caller,
        function_name: FOREIGN_CALL.as_ptr(),
        function_name_length: FOREIGN_CALL.len(),
        source_path: ptr::null(),
        source_path_length: 0,
        source_text: ptr::null(),
        source_text_length: 0,
        function_span_start: 0,
        function_span_end: 0,
        active_span_start: 0,
        active_span_end: 0,
    });
    marker
}

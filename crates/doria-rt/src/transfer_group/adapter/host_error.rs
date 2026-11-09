//! The host retains its real checked Error, never a fabricated native vtable.
//! An inline move-only claim lets the shared core own its disposition. Returning
//! to the host, not Rust destruction, performs ordinary Error cleanup.

use super::AdapterTerminal;
use core::{cell::Cell, ffi::c_void, mem, ptr::NonNull};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum Disposition {
    Empty = 0,
    Owned = 1,
    Discarded = 2,
    Returned = 3,
}

pub(crate) struct HostErrorSlot {
    identity: usize,
    disposition: Cell<Disposition>,
}

impl HostErrorSlot {
    fn new(identity: usize) -> Self {
        Self {
            identity,
            disposition: Cell::new(Disposition::Empty),
        }
    }

    /// # Safety
    /// This pinned slot and its matching real host Error outlive the claim.
    /// Access stays on the host driver thread. The host may acknowledge a
    /// returned/discarded claim only after transferring/dropping that Error.
    unsafe fn claim(&self) -> Option<OwnedHostError> {
        if self.disposition.get() != Disposition::Empty {
            return None;
        }
        self.disposition.set(Disposition::Owned);
        Some(OwnedHostError(NonNull::from(self)))
    }

    fn acknowledge(&self) -> bool {
        if !matches!(
            self.disposition.get(),
            Disposition::Discarded | Disposition::Returned
        ) {
            return false;
        }
        self.disposition.set(Disposition::Empty);
        true
    }
}

#[derive(Debug)]
pub(crate) struct OwnedHostError(NonNull<HostErrorSlot>);

impl OwnedHostError {
    /// Consume the sole core claim. The host retrieves the SAME Error at this
    /// identity, then acknowledges its slot; no native error fields are decoded.
    pub(crate) fn into_identity(self) -> usize {
        let owned = mem::ManuallyDrop::new(self);
        let slot = unsafe { owned.0.as_ref() };
        if slot.disposition.get() != Disposition::Owned {
            unsafe { crate::runtime_invariant_panic() };
        }
        slot.disposition.set(Disposition::Returned);
        slot.identity
    }
}

impl Drop for OwnedHostError {
    fn drop(&mut self) {
        let slot = unsafe { self.0.as_ref() };
        if slot.disposition.get() != Disposition::Owned {
            unsafe { crate::runtime_invariant_panic() };
        }
        slot.disposition.set(Disposition::Discarded);
    }
}

/// # Safety
/// Outputs are disjoint writable locations. Both opaque homes are allocated
/// once by the host group; neither contains the host object or a C callback.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_host_adapter_error_layout(
    slot_size: *mut usize,
    slot_alignment: *mut usize,
    terminal_size: *mut usize,
    terminal_alignment: *mut usize,
) {
    *slot_size = mem::size_of::<HostErrorSlot>();
    *slot_alignment = mem::align_of::<HostErrorSlot>();
    *terminal_size = mem::size_of::<AdapterTerminal>();
    *terminal_alignment = mem::align_of::<AdapterTerminal>();
}

/// # Safety
/// Slot is an aligned, uninitialized HostErrorSlot home, pinned until every
/// claim has returned or been discarded and the real host Error is consumed.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_host_adapter_error_initialize(slot: *mut c_void, identity: usize) {
    slot.cast::<HostErrorSlot>()
        .write(HostErrorSlot::new(identity));
}

/// Returns 0 with a move-only terminal, or 1 without changing either home.
/// Only the safe prefix is bounded; the host's original Error is untouched.
/// # Safety
/// Slot is live and pinned, with its real Error retained by the host. Terminal
/// is aligned EMPTY AdapterTerminal storage disjoint from the slot; ownership moves exactly once to
/// the core or the explicit return/discard functions below. No frame is saved.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_host_adapter_error_claim(
    slot: *const c_void,
    terminal: *mut c_void,
    has_count: u8,
    count: usize,
    bound: usize,
) -> u8 {
    if slot.is_null() || terminal.is_null() || has_count > 1 {
        return 1;
    }
    let Some(error) = (&*slot.cast::<HostErrorSlot>()).claim() else {
        return 1;
    };
    terminal
        .cast::<AdapterTerminal>()
        .write(AdapterTerminal::HostError {
            error,
            progress: (has_count == 1 && count <= bound).then_some(count),
        });
    0
}

/// # Safety
/// Slot is a live initialized home, accessed only by its host driver thread.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_host_adapter_error_disposition(slot: *const c_void) -> u8 {
    (&*slot.cast::<HostErrorSlot>()).disposition.get() as u8
}

/// # Safety
/// Slot is live. The matching real Error has ALREADY been moved to the caller
/// or explicitly dropped by the host. An owned/empty claim is never reset.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_host_adapter_error_acknowledge(slot: *const c_void) -> u8 {
    u8::from(!(&*slot.cast::<HostErrorSlot>()).acknowledge())
}

/// # Safety
/// Terminal is the initialized move-only hosted error terminal, consumed by
/// this call. Its pinned slot and real host Error remain live until retrieval.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_host_adapter_error_return(terminal: *mut c_void) -> usize {
    match terminal.cast::<AdapterTerminal>().read() {
        AdapterTerminal::HostError { error, .. } => error.into_identity(),
        _ => crate::runtime_invariant_panic(),
    }
}

/// # Safety
/// Terminal is a live initialized hosted Error terminal, consumed exactly once. The
/// host processes discarded Error slots after this call returns, outside C.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_host_adapter_error_discard(terminal: *mut c_void) {
    match terminal.cast::<AdapterTerminal>().read() {
        AdapterTerminal::HostError { error, .. } => drop(error),
        _ => crate::runtime_invariant_panic(),
    }
}

#[cfg(test)]
mod tests;

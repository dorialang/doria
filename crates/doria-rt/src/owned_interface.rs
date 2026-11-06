//! Move-only ownership of an ordinary compiler-emitted interface carrier.
//! The payload and descriptor are preserved without boxing or a new close claim.
//! A present descriptor permits a zero inline payload.

use core::{mem, ptr};

use crate::DrInterfaceCarrierV1;

pub(crate) struct OwnedInterfaceValue {
    carrier: DrInterfaceCarrierV1,
}

impl OwnedInterfaceValue {
    /// # Safety
    /// Carrier owns one independent value with its compiler-emitted descriptor.
    /// The caller relinquishes this carrier's words and required cleanup claims.
    pub(crate) unsafe fn from_carrier(carrier: DrInterfaceCarrierV1) -> Self {
        if carrier.vtable.is_null() {
            crate::runtime_invariant_panic();
        }
        Self { carrier }
    }

    pub(crate) fn into_carrier(mut self) -> DrInterfaceCarrierV1 {
        mem::replace(&mut self.carrier, crate::empty_error_carrier())
    }

    /// # Safety
    /// Frame is null or live for this cleanup call only. Ownership never
    /// retains the frame from acquisition or an earlier callback.
    pub(crate) unsafe fn drop_in_frame(mut self, frame: *const crate::DrStackFrameV2) {
        self.release(frame);
    }

    unsafe fn release(&mut self, frame: *const crate::DrStackFrameV2) {
        let carrier = mem::replace(&mut self.carrier, crate::empty_error_carrier());
        if !carrier.vtable.is_null() {
            ((*carrier.vtable).drop)(frame, carrier.object);
        }
    }

    /// Borrows the original carrier metadata without retaining or moving its
    /// value. Any projected access expires before this owner moves or drops.
    #[cfg(test)]
    pub(crate) fn carrier(&self) -> &DrInterfaceCarrierV1 {
        &self.carrier
    }

    /// Projects the actual carrier storage, never a temporary carrier copy.
    /// # Safety
    /// Owner is live and pinned through every use of the returned address.
    /// Compiler loans govern readonly/exclusive access and prevent removal or
    /// destruction. Writable capture writeback preserves the original carrier
    /// representation and registered-stream provenance before owner access.
    pub(crate) unsafe fn carrier_home(owner: *mut Self) -> *mut DrInterfaceCarrierV1 {
        ptr::addr_of_mut!((*owner).carrier)
    }
}

impl core::fmt::Debug for OwnedInterfaceValue {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("OwnedInterfaceValue")
            .field("object", &self.carrier.object)
            .field("descriptor", &self.carrier.vtable)
            .finish()
    }
}

impl Drop for OwnedInterfaceValue {
    fn drop(&mut self) {
        // Rust-owned cleanup has no Doria caller; explicit ABI cleanup forwards
        // the live frame through drop_in_frame without retaining it.
        unsafe { self.release(ptr::null()) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn zero_payload_preserves_one_cleanup_claim_across_transfer_and_drop() {
        static DROPS: AtomicUsize = AtomicUsize::new(0);
        unsafe extern "C" fn release(frame: *const crate::DrStackFrameV2, payload: *mut u8) {
            assert!(frame.is_null());
            assert!(payload.is_null());
            DROPS.fetch_add(1, Ordering::Relaxed);
        }
        let descriptor = crate::DrInterfaceVtableV1 {
            type_name: b"Fixture\\Inline".as_ptr(),
            type_name_length: b"Fixture\\Inline".len(),
            message_offset: 0,
            drop: release,
            class_size: 8,
            origin_offset: 0,
            assertion_offsets: 0,
            assertion_magic: 0,
            class_descriptor: ptr::null(),
            interface_id: 17,
            io_transferred_byte_count_offset: 0,
            io_progress_magic: 0,
        };
        unsafe {
            let value = OwnedInterfaceValue::from_carrier(DrInterfaceCarrierV1 {
                object: ptr::null_mut(),
                vtable: &descriptor,
            });
            let carrier = value.into_carrier();
            assert_eq!(DROPS.load(Ordering::Relaxed), 0);
            OwnedInterfaceValue::from_carrier(carrier).drop_in_frame(ptr::null());
            assert_eq!(DROPS.load(Ordering::Relaxed), 1);
            drop(OwnedInterfaceValue::from_carrier(DrInterfaceCarrierV1 {
                object: ptr::null_mut(),
                vtable: &descriptor,
            }));
            assert_eq!(DROPS.load(Ordering::Relaxed), 2);
        }
    }
}

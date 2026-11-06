use super::*;
use crate::transfer_group::TerminalOutcome;
use core::mem::MaybeUninit;

#[test]
fn move_only_host_claim_returns_identity_without_native_carrier_or_allocation() {
    let slot = HostErrorSlot::new(23);
    let mut terminal = MaybeUninit::<AdapterTerminal>::uninit();
    let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
        assert_eq!(
            dr_v1_host_adapter_error_claim(
                core::ptr::from_ref(&slot).cast(),
                terminal.as_mut_ptr().cast(),
                1,
                3,
                4,
            ),
            0
        );
        let owned = terminal.assume_init_read();
        assert!(owned.is_checked_error());
        assert_eq!(owned.progress(), Some(3));
        assert!(!slot.acknowledge());
        assert!(slot.claim().is_none());
        let moved = owned.bounded(2).into_transfer().unwrap();
        assert_eq!(moved.progress(), None);
        let AdapterTerminal::HostError { error, .. } = moved else {
            panic!("host error was reconstructed as a native value")
        };
        assert_eq!(error.into_identity(), 23);
        assert_eq!(slot.disposition.get(), Disposition::Returned);
        assert!(slot.claim().is_none());
        assert!(slot.acknowledge());
        assert_eq!(slot.disposition.get(), Disposition::Empty);
        assert!(!slot.acknowledge());
    });
    assert_eq!(allocations, 0);
}

#[test]
fn discard_is_deferred_to_host_and_slot_cannot_be_reused_before_acknowledgement() {
    let slot = HostErrorSlot::new(7);
    let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
        for progress in [None, Some(0), Some(4), Some(9)] {
            let terminal = AdapterTerminal::HostError {
                error: slot.claim().unwrap(),
                progress,
            }
            .bounded(4);
            assert!(terminal.is_checked_error());
            assert_eq!(terminal.progress(), progress.filter(|count| *count <= 4));
            assert_eq!(slot.disposition.get(), Disposition::Owned);
            assert!(!slot.acknowledge());
            terminal.drop_in_frame(core::ptr::null());
            assert_eq!(slot.disposition.get(), Disposition::Discarded);
            assert!(slot.claim().is_none());
            assert!(slot.acknowledge());
        }
    });
    assert_eq!(allocations, 0);
}

#[test]
fn rejected_claim_is_unchanged_and_opaque_homes_can_be_reused() {
    let mut slot = MaybeUninit::<HostErrorSlot>::uninit();
    let mut terminal = MaybeUninit::<AdapterTerminal>::uninit();
    let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
        let slot_pointer = slot.as_mut_ptr().cast();
        let terminal_pointer = terminal.as_mut_ptr().cast();
        dr_v1_host_adapter_error_initialize(slot_pointer, 0);
        assert_eq!(
            dr_v1_host_adapter_error_claim(slot_pointer, terminal_pointer, 2, 0, 4),
            1
        );
        assert_eq!(dr_v1_host_adapter_error_disposition(slot_pointer), 0);
        assert_eq!(
            dr_v1_host_adapter_error_claim(slot_pointer, terminal_pointer, 1, 9, 4),
            0
        );
        assert_eq!(dr_v1_host_adapter_error_disposition(slot_pointer), 1);
        let mut rejected = MaybeUninit::<AdapterTerminal>::uninit();
        assert_eq!(
            dr_v1_host_adapter_error_claim(slot_pointer, rejected.as_mut_ptr().cast(), 0, 0, 4),
            1
        );
        assert_eq!(dr_v1_host_adapter_error_acknowledge(slot_pointer), 1);
        assert_eq!(dr_v1_host_adapter_error_return(terminal_pointer), 0);
        assert_eq!(dr_v1_host_adapter_error_disposition(slot_pointer), 3);
        assert_eq!(dr_v1_host_adapter_error_acknowledge(slot_pointer), 0);
        assert_eq!(
            dr_v1_host_adapter_error_claim(slot_pointer, terminal_pointer, 0, 0, 4),
            0
        );
        dr_v1_host_adapter_error_discard(terminal_pointer);
        assert_eq!(dr_v1_host_adapter_error_disposition(slot_pointer), 2);
        assert_eq!(dr_v1_host_adapter_error_acknowledge(slot_pointer), 0);
    });
    assert_eq!(allocations, 0);
}

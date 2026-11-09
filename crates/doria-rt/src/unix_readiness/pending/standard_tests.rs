use super::*;
use crate::device_io::{self, StandardStream};
use crate::transfer_group::TerminalOutcome;

#[test]
fn repeated_standard_registration_uses_one_group_claim_with_distinct_request_records() {
    let _serial = device_io::test_lock();
    unsafe {
        let _input = device_io::TestInput::new(b"abcd", true);
        let device = device_io::standard_device(StandardStream::Stdin);
        let log = Log::new();
        let context = Context::new(0, ptr::from_ref(device).cast(), &log);
        let mut set = PendingSet::new(ptr::null_mut()).unwrap();
        let index = set
            .add_source(17, device.source().value(), context.adapter())
            .unwrap();
        let (mut first, mut second) = ([0xa5; 2], [0x5a; 2]);
        let buffers = BufferList::from_iter(
            [Buffer::Read(&mut first), Buffer::Read(&mut second)].into_iter(),
        )
        .unwrap();
        let wait = budget(1);
        let mut group = set
            .begin_selected(ptr::null(), buffers, &wait, selection(&[index, index]))
            .unwrap();
        assert!(device.try_borrow().is_none());
        let mut seen = [false; 2];
        while let Some(completion) = group.next(ptr::null()) {
            assert_eq!(completion.key, 17);
            assert!(!seen[completion.index]);
            seen[completion.index] = true;
            assert_eq!(completion.outcome.transfer.unwrap().progress(), Some(2));
        }
        assert_eq!(seen, [true; 2]);
        assert!(device.try_borrow().is_none());
        drop(group);
        assert!(device.try_borrow().is_some());
        let mut conserved = [first[0], first[1], second[0], second[1]];
        conserved.sort_unstable();
        assert_eq!(&conserved, b"abcd");
        assert_eq!(
            log.borrow()
                .calls()
                .iter()
                .filter(|call| matches!(call, Call::Complete { .. }))
                .count(),
            2
        );
    }
}

#[test]
fn standard_queue_completes_through_adapter_once_without_native_submission_and_retains_loan() {
    let _serial = device_io::test_lock();
    unsafe {
        let _input = device_io::TestInput::new(b"\xff\0tail", true);
        let device = device_io::standard_device(StandardStream::Stdin);
        let endpoint = ptr::from_ref(device).cast();
        let handle = device_io::byte_handle(StandardStream::Stdin).unwrap();
        let flags = libc::fcntl(handle, libc::F_GETFL);
        assert!(flags >= 0);
        let log = Log::new();
        let context = Context::new(0, endpoint, &log);
        let mut set = PendingSet::new(ptr::null_mut()).unwrap();
        let source = device.source().value();
        let index = set.add_source(7, source, context.adapter()).unwrap();
        assert!(
            device.try_borrow().is_some(),
            "registration is not a consumer/close owner"
        );
        assert!(set.add_source(8, source, context.adapter()).is_err());
        let mut bytes = [0; 2];
        let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
        let unlimited = budget(1);
        let mut group = set
            .begin_selected(ptr::null(), buffers, &unlimited, selection(&[index]))
            .unwrap();
        assert!(device.try_borrow().is_none());
        assert_eq!(
            libc::fcntl(handle, libc::F_GETFL),
            flags,
            "buffered preparation must not change native mode"
        );
        let (first, allocations) =
            crate::probe_runtime_allocations(|| group.next(ptr::null()).unwrap());
        assert_eq!(allocations, 0);
        assert_eq!(first.outcome.transfer.unwrap().progress(), Some(2));
        assert_eq!(group.completed(0).unwrap(), b"\xff\0");
        assert!(
            device.try_borrow().is_none(),
            "completion does not release group loans"
        );
        let (_, allocations) = crate::probe_runtime_allocations(|| {
            group.submit(ptr::null(), 0, 0, 2).unwrap();
            assert_eq!(
                group
                    .next(ptr::null())
                    .unwrap()
                    .outcome
                    .transfer
                    .unwrap()
                    .progress(),
                Some(2)
            );
        });
        assert_eq!(allocations, 0);
        assert_eq!(group.completed(0).unwrap(), b"ta");
        group.submit(ptr::null(), 0, 0, 2).unwrap();
        assert_eq!(
            group
                .next(ptr::null())
                .unwrap()
                .outcome
                .transfer
                .unwrap()
                .progress(),
            Some(2)
        );
        assert_eq!(group.completed(0).unwrap(), b"il");
        group.submit(ptr::null(), 0, 0, 2).unwrap();
        assert!(matches!(
            group.next(ptr::null()).unwrap().outcome.transfer,
            Some(AdapterTerminal::EndOfStream)
        ));
        assert!(group.next(ptr::null()).is_none());
        assert!(device.try_borrow().is_none());
        drop(group);
        assert!(device.try_borrow().is_some());
        assert_eq!(libc::fcntl(handle, libc::F_GETFL), flags);
        let complete: std::vec::Vec<_> = log
            .borrow()
            .calls()
            .iter()
            .copied()
            .filter(|call| matches!(call, Call::Complete { .. }))
            .collect();
        assert_eq!(complete.len(), 4);
        for (position, call) in complete.into_iter().enumerate() {
            assert!(
                matches!(call, Call::Complete { submitted: 0, immediate: Some((tag, count)), .. }
                if (tag, count) == if position == 3 { (2, 0) } else { (0, 2) })
            );
        }
        set.remove(7).unwrap();
        drop(set);
        assert!(
            libc::fcntl(handle, libc::F_GETFD) >= 0,
            "standard handle was never closed"
        );
        assert_eq!(
            device_io::byte_handle(StandardStream::Stdin).unwrap(),
            handle
        );
    }
}

#[test]
fn standard_device_claim_is_exclusive_and_never_mutates_or_closes_the_inherited_descriptor() {
    let _serial = device_io::test_lock();
    unsafe {
        let handle = device_io::byte_handle(StandardStream::Stdin).unwrap();
        let flags = libc::fcntl(handle, libc::F_GETFL);
        assert!(flags >= 0);
        let guard = device_io::standard_device(StandardStream::Stdin).borrow();
        assert!(device_io::standard_device(StandardStream::Stdin)
            .try_borrow()
            .is_none());
        assert_eq!(libc::fcntl(handle, libc::F_GETFL), flags);
        drop(guard);
        assert_eq!(libc::fcntl(handle, libc::F_GETFL), flags);
        assert!(libc::fcntl(handle, libc::F_GETFD) >= 0);
        assert!(device_io::standard_device(StandardStream::Stdin)
            .try_borrow()
            .is_some());
    }
}

#[test]
fn standard_preparation_rejection_releases_group_loan_and_does_not_consume_queue() {
    let _serial = device_io::test_lock();
    unsafe {
        let _input = device_io::TestInput::new(b"preserved", true);
        let device = device_io::standard_device(StandardStream::Stdin);
        let log = Log::new();
        // A copied local handle cell is not a standard-device endpoint.
        let local = device_io::byte_handle(StandardStream::Stdin).unwrap();
        let context = Context::new(0, ptr::from_ref(&local).cast(), &log);
        let mut set = PendingSet::new(ptr::null_mut()).unwrap();
        let index = set
            .add_source(1, device.source().value(), context.adapter())
            .unwrap();
        let mut bytes = [0; 8];
        let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
        let unlimited = budget(1);
        assert!(set
            .begin_selected(ptr::null(), buffers, &unlimited, selection(&[index]))
            .is_err());
        let loan = device
            .try_borrow()
            .expect("failed begin released its device loan");
        assert!(loan.buffered_input_ready());
        assert_eq!((*device_io::stdin_input()).start, 0);
        drop(loan);
        let borrowed = log.borrow();
        assert_eq!(borrowed.calls().len(), 2);
        assert!(matches!(
            borrowed.calls()[1],
            Call::Complete {
                submitted: 0,
                tag: 4,
                ..
            }
        ));
        drop(borrowed);
        set.remove(1).unwrap();
        assert!(libc::fcntl(local, libc::F_GETFD) >= 0);
    }
}

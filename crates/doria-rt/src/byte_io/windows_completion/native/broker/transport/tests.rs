//! Real Windows private-ABI fixtures. Cross-compilation does not execute their
//! native packet, pipe EOF, or ownership assertions.

use core::mem::MaybeUninit;
use std::os::windows::io::{IntoRawHandle, OwnedHandle as StdHandle};

use super::super::super::tests::{budget, connected_pair, TestClaim};
use super::*;
use crate::transfer_group::transport::plan::{
    dr_v1_transfer_key, dr_v1_transfer_key_layout, dr_v1_transfer_plan_append,
    dr_v1_transfer_plan_begin, dr_v1_transfer_plan_drop, dr_v1_transfer_plan_new,
};
use crate::transfer_group::transport::registration::{
    dr_v1_transfer_set_add, dr_v1_transfer_set_new, dr_v1_transfer_set_remove,
};
use crate::transfer_group::transport::{
    dr_v1_transfer_group_begin, dr_v1_transfer_group_drop, dr_v1_transfer_group_next,
    dr_v1_transfer_group_region, dr_v1_transfer_group_stop, dr_v1_transfer_group_submit,
    dr_v1_transfer_outcome_layout, dr_v1_transfer_set_drop, DrTransferBufferV1,
};
use crate::transfer_group::PlanKey;

fn poisoned_io() -> DrIoStatusV1 {
    DrIoStatusV1 {
        value: 911,
        system_code: -17,
        transferred_byte_count: 912,
        reason: 255,
        has_system_code: 1,
        has_transferred_byte_count: 1,
    }
}

fn io_fields(status: &DrIoStatusV1) -> (u64, i64, usize, u8, u8, u8) {
    (
        status.value,
        status.system_code,
        status.transferred_byte_count,
        status.reason,
        status.has_system_code,
        status.has_transferred_byte_count,
    )
}

fn clear_io(status: &DrIoStatusV1) {
    assert_eq!(io_fields(status), (0, 0, 0, 0, 0, 0));
}

fn invalid_io(status: &DrIoStatusV1) {
    assert_eq!(
        io_fields(status),
        (0, 0, 0, Reason::InvalidInput as u8, 0, 0)
    );
}

fn poisoned_outcome() -> DrCompletionStatusV1 {
    DrCompletionStatusV1 {
        transfer: poisoned_io(),
        wait_failure: poisoned_io(),
        cancellation_failure: poisoned_io(),
        stop: 255,
        has_cancellation_failure: 1,
        wait_operation: 255,
        cancellation_operation: 255,
    }
}

fn uninterrupted(status: &DrCompletionStatusV1) {
    assert_eq!((status.stop, status.has_cancellation_failure), (0, 0));
    assert_eq!(
        (status.wait_operation, status.cancellation_operation),
        (0, 0)
    );
    clear_io(&status.wait_failure);
    clear_io(&status.cancellation_failure);
}

// These local guards only pair each exported ownership operation with its
// matching exported cleanup. Buffer/budget lifetimes remain explicit below:
// every group is dropped before any original region is accessed or destroyed.
struct InlineStream {
    storage: MaybeUninit<AttachedStream>,
    live: bool,
}

impl InlineStream {
    fn empty() -> Self {
        Self {
            storage: MaybeUninit::uninit(),
            live: false,
        }
    }

    fn adopt(handle: StdHandle) -> Self {
        let mut owner = Self::empty();
        let mut status = poisoned_io();
        let tag = unsafe {
            dr_v1_windows_transfer_stream_adopt(
                owner.address(),
                handle.into_raw_handle(),
                &mut status,
            )
        };
        owner.live = tag == 0;
        assert_eq!(tag, 0);
        clear_io(&status);
        owner
    }

    fn address(&mut self) -> *mut c_void {
        self.storage.as_mut_ptr().cast()
    }

    fn identity(&self) -> (usize, usize, usize) {
        assert!(self.live);
        unsafe { self.storage.assume_init_ref().identity_for_test() }
    }

    fn close(mut self) -> (u8, DrIoStatusV1) {
        assert!(self.live);
        self.live = false;
        let mut status = poisoned_io();
        let tag = unsafe { dr_v1_windows_transfer_stream_close(self.address(), &mut status) };
        (tag, status)
    }
}

impl Drop for InlineStream {
    fn drop(&mut self) {
        if self.live {
            unsafe { dr_v1_windows_transfer_stream_drop(self.address()) };
        }
    }
}

struct AbiSet(*mut c_void);

impl AbiSet {
    fn borrowed(cancellation: *mut DrCancellationV1) -> Self {
        let mut status = poisoned_io();
        let owner =
            Self(unsafe { dr_v1_transfer_set_new(core::ptr::null(), cancellation, &mut status) });
        assert!(!owner.0.is_null());
        clear_io(&status);
        owner
    }

    fn add(&self, stream: &InlineStream) -> PlanKey {
        assert!(stream.live);
        let mut key = PlanKey { owner: 0, value: 0 };
        let mut status = poisoned_io();
        assert_eq!(
            unsafe {
                dr_v1_transfer_set_add(
                    self.0,
                    stream.storage.as_ptr().cast(),
                    (&mut key as *mut PlanKey).cast(),
                    &mut status,
                )
            },
            0
        );
        clear_io(&status);
        assert_ne!(key.owner, 0);
        assert_ne!(key.value, 0);
        key
    }

    fn remove(&self, key: &PlanKey) -> (u8, DrIoStatusV1) {
        let mut status = poisoned_io();
        let tag = unsafe {
            dr_v1_transfer_set_remove(self.0, (key as *const PlanKey).cast(), &mut status)
        };
        (tag, status)
    }

    fn new(
        keys: &[u64],
        streams: &mut [InlineStream],
        cancellation: *mut DrCancellationV1,
    ) -> Self {
        assert_eq!(keys.len(), streams.len());
        let addresses: Vec<_> = streams
            .iter_mut()
            .map(|stream| {
                assert!(stream.live);
                stream.live = false;
                stream.address()
            })
            .collect();
        let mut status = poisoned_io();
        let owner = Self(unsafe {
            dr_v1_windows_transfer_set_new(
                core::ptr::null(),
                keys.as_ptr(),
                addresses.as_ptr(),
                keys.len(),
                cancellation,
                &mut status,
            )
        });
        assert!(!owner.0.is_null());
        clear_io(&status);
        owner
    }

    fn key(&self, index: usize) -> PlanKey {
        let mut key = PlanKey { owner: 0, value: 0 };
        let mut status = poisoned_io();
        assert_eq!(
            unsafe {
                dr_v1_transfer_key(
                    self.0,
                    index,
                    (&mut key as *mut PlanKey).cast(),
                    &mut status,
                )
            },
            0
        );
        clear_io(&status);
        assert_ne!(key.owner, 0);
        key
    }

    fn release<const N: usize>(mut self) -> [InlineStream; N] {
        // Each call below supplies exactly this set's original configured count.
        let mut streams = core::array::from_fn(|_| InlineStream::empty());
        let destinations: [_; N] = core::array::from_fn(|index| streams[index].address());
        let mut status = poisoned_io();
        let tag = unsafe {
            dr_v1_windows_transfer_set_release(self.0, destinations.as_ptr(), &mut status)
        };
        if tag == 0 {
            self.0 = ptr::null_mut();
            for stream in &mut streams {
                stream.live = true;
            }
        }
        assert_eq!(tag, 0);
        clear_io(&status);
        streams
    }
}

impl Drop for AbiSet {
    fn drop(&mut self) {
        unsafe { dr_v1_transfer_set_drop(self.0) };
    }
}

struct AbiGroup(*mut c_void);

struct Completion {
    tag: u8,
    key: u64,
    index: usize,
    status: DrCompletionStatusV1,
}

impl AbiGroup {
    unsafe fn begin(
        set: &AbiSet,
        descriptors: &[DrTransferBufferV1],
        wait: &DrWaitBudgetV1,
    ) -> Self {
        let mut status = poisoned_io();
        let group = Self(dr_v1_transfer_group_begin(
            core::ptr::null(),
            set.0,
            descriptors.as_ptr(),
            descriptors.len(),
            wait,
            &mut status,
        ));
        assert!(!group.0.is_null());
        clear_io(&status);
        group
    }

    fn next(&mut self) -> Completion {
        let (mut key, mut index) = (u64::MAX, usize::MAX);
        let mut status = poisoned_outcome();
        let tag = unsafe {
            dr_v1_transfer_group_next(
                self.0,
                &mut key,
                &mut index,
                (&mut status as *mut DrCompletionStatusV1).cast(),
            )
        };
        Completion {
            tag,
            key,
            index,
            status,
        }
    }

    fn region(&mut self, index: usize, writable: u8) -> (u8, *mut u8, usize) {
        let (mut data, mut length) = (ptr::NonNull::<u8>::dangling().as_ptr(), usize::MAX);
        let tag =
            unsafe { dr_v1_transfer_group_region(self.0, index, writable, &mut data, &mut length) };
        (tag, data, length)
    }

    fn submit(&mut self, index: usize, offset: usize, length: usize) -> (u8, DrIoStatusV1) {
        let mut status = poisoned_io();
        let tag =
            unsafe { dr_v1_transfer_group_submit(self.0, index, offset, length, &mut status) };
        (tag, status)
    }
}

impl Drop for AbiGroup {
    fn drop(&mut self) {
        unsafe { dr_v1_transfer_group_drop(self.0) };
    }
}

// Like AbiGroup, this guard owns only the opaque runtime descriptor owner.
// Caller regions outlive it; consuming calls clear the old pointer first,
// including rejection, so cleanup never revisits a consumed plan allocation.
struct AbiPlan(*mut c_void);

impl AbiPlan {
    unsafe fn new(key: &PlanKey, descriptor: &DrTransferBufferV1) -> Self {
        let mut status = poisoned_io();
        let plan = Self(dr_v1_transfer_plan_new(
            core::ptr::null(),
            (key as *const PlanKey).cast(),
            descriptor,
            &mut status,
        ));
        assert!(!plan.0.is_null());
        clear_io(&status);
        plan
    }

    unsafe fn append(
        mut self,
        key: &PlanKey,
        descriptor: &DrTransferBufferV1,
    ) -> (Self, DrIoStatusV1) {
        let mut status = poisoned_io();
        let owner = core::mem::replace(&mut self.0, ptr::null_mut());
        let plan = Self(dr_v1_transfer_plan_append(
            core::ptr::null(),
            owner,
            (key as *const PlanKey).cast(),
            descriptor,
            &mut status,
        ));
        (plan, status)
    }

    unsafe fn begin(
        mut self,
        set: &AbiSet,
        wait: *const DrWaitBudgetV1,
    ) -> (AbiGroup, DrIoStatusV1) {
        let mut status = poisoned_io();
        let owner = core::mem::replace(&mut self.0, ptr::null_mut());
        let group = AbiGroup(dr_v1_transfer_plan_begin(
            core::ptr::null(),
            set.0,
            owner,
            wait,
            &mut status,
        ));
        (group, status)
    }
}

impl Drop for AbiPlan {
    fn drop(&mut self) {
        unsafe { dr_v1_transfer_plan_drop(self.0) };
    }
}

fn read(data: *mut u8, length: usize) -> DrTransferBufferV1 {
    DrTransferBufferV1 {
        data,
        length,
        reading: 1,
    }
}

fn peer(handle: StdHandle) -> Pipe<WindowsDriver> {
    Pipe {
        driver: unsafe { WindowsDriver::new(handle.into_raw_handle()).unwrap() },
    }
}

fn write(peer: &mut Pipe<WindowsDriver>, bytes: &[u8]) {
    let wait = budget(2, 5_000_000_000);
    let mut control = unsafe { WaitControl::new(Some(&wait), ptr::null_mut()).unwrap() };
    assert_eq!(
        peer.transfer(Buffer::Write(bytes), &mut control),
        Awaited::complete(Ok(bytes.len()))
    );
}

fn data_completion(completion: &Completion, key: u64, index: usize, count: u64) {
    assert_eq!(
        (completion.tag, completion.key, completion.index),
        (0, key, index)
    );
    assert_eq!(
        io_fields(&completion.status.transfer),
        (count, 0, 0, 0, 0, 0)
    );
    uninterrupted(&completion.status);
}

fn exhausted(group: &mut AbiGroup) {
    let completion = group.next();
    assert_eq!(
        (completion.tag, completion.key, completion.index),
        (5, 0, 0)
    );
    clear_io(&completion.status.transfer);
    uninterrupted(&completion.status);
}

unsafe fn invalid_begin(
    set: *mut c_void,
    descriptors: &[DrTransferBufferV1],
    count: usize,
    wait: &DrWaitBudgetV1,
) {
    let mut status = poisoned_io();
    let (group, allocations) = crate::probe_runtime_allocations(|| {
        AbiGroup(dr_v1_transfer_group_begin(
            core::ptr::null(),
            set,
            descriptors.as_ptr(),
            count,
            wait,
            &mut status,
        ))
    });
    // The guard retains even an unexpectedly accepted group until its cleanup
    // runs; the caller's regions/budget remain alive throughout this call.
    assert!(group.0.is_null());
    assert_eq!(
        allocations, 0,
        "rejection must precede buffer/config allocation and I/O"
    );
    invalid_io(&status);
}

#[test]
fn private_abi_regroups_streams_and_reuses_original_completed_regions() {
    let _serial = TEST_BROKER_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (mut stream_size, mut stream_align, mut outcome_size, mut outcome_align) = (0, 0, 0, 0);
    unsafe {
        dr_v1_windows_transfer_stream_layout(&mut stream_size, &mut stream_align);
        dr_v1_transfer_outcome_layout(&mut outcome_size, &mut outcome_align);
    }
    assert_eq!(
        (stream_size, stream_align),
        (
            mem::size_of::<AttachedStream>(),
            mem::align_of::<AttachedStream>()
        )
    );
    assert_eq!(
        (outcome_size, outcome_align),
        (
            mem::size_of::<DrCompletionStatusV1>(),
            mem::align_of::<DrCompletionStatusV1>()
        )
    );
    let (server_a, client_a) = connected_pair();
    let (server_b, client_b) = connected_pair();
    let mut initial = [InlineStream::adopt(server_a), InlineStream::adopt(server_b)];
    let identities = [initial[0].identity(), initial[1].identity()];
    assert_ne!(identities[0].1, identities[1].1);
    assert_eq!(identities[0].2, identities[1].2);
    let former_a = AbiSet::new(&[11], &mut initial[..1], ptr::null_mut());
    let former_b = AbiSet::new(&[22], &mut initial[1..], ptr::null_mut());
    let mut peer_a = peer(client_a);
    let mut peer_b = peer(client_b);
    let wait = budget(2, 5_000_000_000);
    let (mut former_bytes_a, mut former_bytes_b) = ([0_u8; 8], [0_u8; 8]);
    let descriptors_a = [read(former_bytes_a.as_mut_ptr(), 8)];
    let descriptors_b = [read(former_bytes_b.as_mut_ptr(), 8)];
    let mut group_a = unsafe { AbiGroup::begin(&former_a, &descriptors_a, &wait) };
    let mut group_b = unsafe { AbiGroup::begin(&former_b, &descriptors_b, &wait) };
    write(&mut peer_a, b"old-a");
    write(&mut peer_b, b"old-b");
    data_completion(&group_a.next(), 11, 0, 5);
    data_completion(&group_b.next(), 22, 0, 5);
    drop((group_a, group_b));
    assert_eq!(&former_bytes_a[..5], b"old-a");
    assert_eq!(&former_bytes_b[..5], b"old-b");
    let [recovered_a] = former_a.release::<1>();
    let [recovered_b] = former_b.release::<1>();
    assert_eq!(recovered_a.identity(), identities[0]);
    assert_eq!(recovered_b.identity(), identities[1]);
    let mut recovered = [recovered_a, recovered_b];
    let source = TestClaim::new();
    let combined = AbiSet::new(&[101, 202], &mut recovered, source.state());
    let wait = budget(2, 5_000_000_000);
    let (mut first, mut second) = ([0x55_u8; 16], [0x66_u8; 16]);
    let (first_address, second_address) = (first.as_mut_ptr(), second.as_mut_ptr());
    let descriptors = [read(first_address, 16), read(second_address, 16)];
    // Every allocation, including wake registration, precedes the hot probes.
    let mut group = unsafe { AbiGroup::begin(&combined, &descriptors, &wait) };
    assert_eq!(group.region(0, 0), (1, ptr::null_mut(), 0));
    assert_eq!(group.region(1, 1), (1, ptr::null_mut(), 0));
    write(&mut peer_a, b"one");
    let (completion, allocations) = crate::probe_runtime_allocations(|| group.next());
    assert_eq!(allocations, 0);
    data_completion(&completion, 101, 0, 3);
    let (tag, prefix, length) = group.region(0, 0);
    assert_eq!((tag, prefix, length), (0, first_address, 3));
    assert_eq!(
        unsafe { core::slice::from_raw_parts(prefix, length) },
        b"one"
    );
    assert_eq!(group.region(1, 1), (1, ptr::null_mut(), 0));
    let (tag, idle, length) = group.region(0, 1);
    assert_eq!((tag, idle, length), (0, first_address, 16));
    // Use only the ABI-authorized idle region, never the original caller array.
    unsafe { core::slice::from_raw_parts_mut(idle, length).fill(0x77) };
    let ((tag, status), allocations) = crate::probe_runtime_allocations(|| group.submit(0, 4, 5));
    assert_eq!((tag, allocations), (0, 0));
    clear_io(&status);
    assert_eq!(group.region(0, 0), (1, ptr::null_mut(), 0));
    write(&mut peer_a, b"again");
    let (completion, allocations) = crate::probe_runtime_allocations(|| group.next());
    assert_eq!(allocations, 0);
    data_completion(&completion, 101, 0, 5);
    let (tag, prefix, length) = group.region(0, 0);
    assert_eq!((tag, prefix, length), (0, first_address.wrapping_add(4), 5));
    assert_eq!(
        unsafe { core::slice::from_raw_parts(prefix, length) },
        b"again"
    );
    write(&mut peer_b, b"other");
    let (completion, allocations) = crate::probe_runtime_allocations(|| group.next());
    assert_eq!(allocations, 0);
    data_completion(&completion, 202, 1, 5);
    let (tag, prefix, length) = group.region(1, 0);
    assert_eq!((tag, prefix, length), (0, second_address, 5));
    assert_eq!(
        unsafe { core::slice::from_raw_parts(prefix, length) },
        b"other"
    );
    exhausted(&mut group);
    assert_eq!(unsafe { dr_v1_transfer_group_stop(group.0) }, 0);
    let ((), allocations) = crate::probe_runtime_allocations(|| drop(group));
    assert_eq!(allocations, 0);
    assert_eq!(&first[..4], &[0x77; 4]);
    assert_eq!(&first[4..9], b"again");
    assert_eq!(&first[9..], &[0x77; 7]);
    assert_eq!(&second[..5], b"other");
    assert_eq!(&second[5..], &[0x66; 11]);
    let [released_a, released_b] = combined.release::<2>();
    assert_eq!(released_a.identity(), identities[0]);
    assert_eq!(released_b.identity(), identities[1]);
    let ((tag, status), closes_a) = crate::native_handle::with_close_probe(|| released_a.close());
    assert_eq!((tag, closes_a), (0, 1));
    clear_io(&status);
    let ((), closes_b) = crate::native_handle::with_close_probe(|| drop(released_b));
    // The shared port is a private kernel object, not a stream close.
    assert_eq!(closes_b, 1, "the last inline stream closes once");
}

#[test]
fn private_abi_rejects_before_io_and_distinguishes_zero_data_from_pipe_eof() {
    let _serial = TEST_BROKER_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (server, client) = connected_pair();
    let mut stream = [InlineStream::adopt(server)];
    let source = TestClaim::new();
    let set = AbiSet::new(&[303], &mut stream, source.state());
    let immediate = budget(0, 0);
    let (mut bytes, mut other) = ([0x55_u8; 8], [0x66_u8; 8]);
    let address = bytes.as_mut_ptr();
    let valid = [read(address, 8), read(other.as_mut_ptr(), 8)];
    unsafe {
        invalid_begin(set.0, &valid, 0, &immediate);
        invalid_begin(set.0, &valid, 2, &immediate);
        for descriptor in [
            DrTransferBufferV1 {
                data: address,
                length: 8,
                reading: 2,
            },
            read(ptr::null_mut(), 8),
            read(address, isize::MAX as usize + 1),
        ] {
            invalid_begin(set.0, &[descriptor], 1, &immediate);
        }
    }
    let descriptors = [read(address, 8)];
    let mut group = unsafe { AbiGroup::begin(&set, &descriptors, &immediate) };
    assert_eq!(group.region(0, 1), (1, ptr::null_mut(), 0));
    assert_eq!(group.region(0, 0), (1, ptr::null_mut(), 0));
    assert_eq!(group.region(0, 2), (1, ptr::null_mut(), 0));
    assert_eq!(group.region(1, 0), (1, ptr::null_mut(), 0));
    let (tag, status) = group.submit(0, 0, 8);
    assert_eq!(tag, 3);
    invalid_io(&status);
    unsafe { invalid_begin(set.0, &[], 0, &immediate) };
    // Active-set release must reject without consuming its owner or writing a
    // single destination byte. The destination is aligned uninitialized inline
    // storage filled as bytes solely so preservation can be observed safely.
    let mut destination = InlineStream::empty();
    unsafe {
        ptr::write_bytes(
            destination.storage.as_mut_ptr().cast::<u8>(),
            0xa5,
            mem::size_of::<AttachedStream>(),
        )
    };
    let destinations = [destination.address()];
    let mut status = poisoned_io();
    assert_eq!(
        unsafe { dr_v1_windows_transfer_set_release(set.0, destinations.as_ptr(), &mut status) },
        3
    );
    invalid_io(&status);
    assert!(unsafe {
        core::slice::from_raw_parts(
            destination.storage.as_ptr().cast::<u8>(),
            mem::size_of::<AttachedStream>(),
        )
    }
    .iter()
    .all(|byte| *byte == 0xa5));
    let completion = group.next();
    assert_eq!(
        (
            completion.tag,
            completion.key,
            completion.index,
            completion.status.stop
        ),
        (3, 303, 0, 2)
    );
    assert_eq!(
        io_fields(&completion.status.transfer),
        (0, 995, 0, Reason::Interrupted as u8, 1, 1)
    );
    assert_eq!(completion.status.has_cancellation_failure, 0);
    clear_io(&completion.status.wait_failure);
    clear_io(&completion.status.cancellation_failure);
    assert_eq!(unsafe { dr_v1_transfer_group_stop(group.0) }, 2);
    let (tag, idle, length) = group.region(0, 1);
    assert_eq!((tag, idle, length), (0, address, 8));
    exhausted(&mut group);
    drop(group);
    assert_eq!(bytes, [0x55; 8]);
    assert_eq!(other, [0x66; 8]);
    // Rejection/cancelled immediate wait did not consume or replace the set.
    let mut peer = peer(client);
    write(&mut peer, b"valid");
    let finite = budget(2, 5_000_000_000);
    let mut group = unsafe { AbiGroup::begin(&set, &descriptors, &finite) };
    data_completion(&group.next(), 303, 0, 5);
    let (tag, prefix, length) = group.region(0, 0);
    assert_eq!((tag, prefix, length), (0, address, 5));
    assert_eq!(
        unsafe { core::slice::from_raw_parts(prefix, length) },
        b"valid"
    );
    drop(group);
    assert_eq!(&bytes[..5], b"valid");
    let ((), closes) = crate::native_handle::with_close_probe(|| drop(set));
    assert_eq!(
        closes, 2,
        "shared set drop closes stream and last broker once"
    );

    let (server, peer) = connected_pair();
    let mut stream = [InlineStream::adopt(server)];
    let eof_set = AbiSet::new(&[404], &mut stream, ptr::null_mut());
    drop(peer);
    let mut eof_bytes = [0x88_u8; 8];
    for length in [0, 8] {
        let wait = budget(2, 5_000_000_000);
        let descriptors = [read(
            if length == 0 {
                ptr::null_mut()
            } else {
                eof_bytes.as_mut_ptr()
            },
            length,
        )];
        let mut group = unsafe { AbiGroup::begin(&eof_set, &descriptors, &wait) };
        let completion = group.next();
        assert_eq!(
            (completion.tag, completion.key, completion.index),
            (if length == 0 { 0 } else { 2 }, 404, 0)
        );
        clear_io(&completion.status.transfer);
        uninterrupted(&completion.status);
        exhausted(&mut group);
        drop(group);
    }
    assert_eq!(eof_bytes, [0x88; 8]);
    // Projection regressions, not manufactured native reads: successful zero
    // progress remains Data(0); only the actual broken-pipe fact implies EOF.
    let mut outcome = poisoned_outcome();
    assert_eq!(
        NativeTransferSet::project_completion(&mut outcome, Awaited::complete(Ok(0)), true, 8),
        0
    );
    clear_io(&outcome.transfer);
    uninterrupted(&outcome);
    assert_eq!(
        NativeTransferSet::project_completion(
            &mut outcome,
            Awaited::complete(Err(
                Failure::from_system_code(109).with_transferred_byte_count(Some(0))
            )),
            true,
            8
        ),
        2
    );
    clear_io(&outcome.transfer);
    uninterrupted(&outcome);
    let mut status = poisoned_io();
    assert_eq!(
        unsafe {
            dr_v1_windows_transfer_set_release(ptr::null_mut(), destinations.as_ptr(), &mut status)
        },
        3
    );
    invalid_io(&status);
    assert!(unsafe {
        core::slice::from_raw_parts(
            destination.storage.as_ptr().cast::<u8>(),
            mem::size_of::<AttachedStream>(),
        )
    }
    .iter()
    .all(|byte| *byte == 0xa5));
}

#[test]
fn private_plan_abi_reversed_subset_preserves_native_affinity_and_original_regions() {
    let _serial = TEST_BROKER_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (mut size, mut align) = (0, 0);
    unsafe { dr_v1_transfer_key_layout(&mut size, &mut align) };
    assert_eq!(
        (size, align),
        (mem::size_of::<PlanKey>(), mem::align_of::<PlanKey>())
    );
    let (server_a, client_a) = connected_pair();
    let (server_b, client_b) = connected_pair();
    let (server_c, client_c) = connected_pair();
    let mut streams = [
        InlineStream::adopt(server_a),
        InlineStream::adopt(server_b),
        InlineStream::adopt(server_c),
    ];
    let identities = [
        streams[0].identity(),
        streams[1].identity(),
        streams[2].identity(),
    ];
    let set = AbiSet::new(&[101, 202, 303], &mut streams, ptr::null_mut());
    let (key_a, key_c) = (set.key(0), set.key(2));
    assert_eq!((key_a.value, key_c.value), (101, 303));
    assert_eq!(key_a.owner, key_c.owner);
    let mut peer_a = peer(client_a);
    let mut peer_b = peer(client_b);
    let mut peer_c = peer(client_c);
    let (mut bytes_a, mut bytes_c) = ([0x11_u8; 8], [0x33_u8; 8]);
    let (address_a, address_c) = (bytes_a.as_mut_ptr(), bytes_c.as_mut_ptr());
    let wait = budget(2, 5_000_000_000);
    let plan = unsafe { AbiPlan::new(&key_c, &read(address_c, 8)) };
    let (plan, status) = unsafe { plan.append(&key_a, &read(address_a, 8)) };
    assert!(!plan.0.is_null());
    clear_io(&status);
    write(&mut peer_b, b"idle-b");
    let (mut group, status) = unsafe { plan.begin(&set, &wait) };
    assert!(!group.0.is_null());
    clear_io(&status);
    assert_eq!(group.region(2, 0), (1, ptr::null_mut(), 0));
    write(&mut peer_c, b"third");
    let (completion, allocations) = crate::probe_runtime_allocations(|| group.next());
    assert_eq!(allocations, 0);
    data_completion(&completion, 303, 0, 5);
    assert_eq!(group.region(0, 0), (0, address_c, 5));
    let ((tag, status), allocations) = crate::probe_runtime_allocations(|| group.submit(0, 1, 3));
    assert_eq!((tag, allocations), (0, 0));
    clear_io(&status);
    write(&mut peer_c, b"xyz");
    data_completion(&group.next(), 303, 0, 3);
    assert_eq!(group.region(0, 0), (0, address_c.wrapping_add(1), 3));
    write(&mut peer_a, b"first");
    data_completion(&group.next(), 101, 1, 5);
    assert_eq!(group.region(1, 0), (0, address_a, 5));
    exhausted(&mut group);
    drop(group);
    assert_eq!(&bytes_c[..5], b"txyzd");
    assert_eq!(&bytes_a[..5], b"first");

    // The unselected pipe's original payload survives the entire subset and
    // resubmission. Zero-length neighbours submit no native read.
    let mut bytes_b = [0x22_u8; 8];
    let address_b = bytes_b.as_mut_ptr();
    let descriptors = [
        read(ptr::null_mut(), 0),
        read(address_b, 8),
        read(ptr::null_mut(), 0),
    ];
    let wait = budget(2, 5_000_000_000);
    let mut group = unsafe { AbiGroup::begin(&set, &descriptors, &wait) };
    for _ in 0..3 {
        let completion = group.next();
        match completion.index {
            0 => data_completion(&completion, 101, 0, 0),
            1 => data_completion(&completion, 202, 1, 6),
            2 => data_completion(&completion, 303, 2, 0),
            _ => panic!("unexpected full-set completion index"),
        }
    }
    exhausted(&mut group);
    drop(group);
    assert_eq!(&bytes_b[..6], b"idle-b");
    let recovered = set.release::<3>();
    assert_eq!(recovered[0].identity(), identities[0]);
    assert_eq!(recovered[1].identity(), identities[1]);
    assert_eq!(recovered[2].identity(), identities[2]);
}

#[test]
fn private_plan_abi_drop_and_setup_rejection_leave_pipe_payload_unread() {
    let _serial = TEST_BROKER_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (server, client) = connected_pair();
    let (sibling_server, _sibling_client) = connected_pair();
    let (foreign_server, foreign_client) = connected_pair();
    let mut stream = [
        InlineStream::adopt(server),
        InlineStream::adopt(sibling_server),
    ];
    let mut foreign_stream = [InlineStream::adopt(foreign_server)];
    let set = AbiSet::new(&[404, 606], &mut stream, ptr::null_mut());
    let foreign = AbiSet::new(&[505], &mut foreign_stream, ptr::null_mut());
    let (key, sibling_key, foreign_key) = (set.key(0), set.key(1), foreign.key(0));
    let mut peer = peer(client);
    let (mut bytes, mut incoming) = ([0x55_u8; 8], [0x66_u8; 8]);
    let (address, incoming_address) = (bytes.as_mut_ptr(), incoming.as_mut_ptr());
    write(&mut peer, b"intact");
    let plan = unsafe { AbiPlan::new(&key, &read(address, 8)) };
    drop(plan);
    assert_eq!(bytes, [0x55; 8]);

    // A repeated key is valid plan configuration (transport/plan/tests.rs);
    // a key issued by another set is not, and must leave every loan intact.
    let plan = unsafe { AbiPlan::new(&key, &read(address, 8)) };
    let ((rejected, status), allocations) = crate::probe_runtime_allocations(|| unsafe {
        plan.append(&foreign_key, &read(incoming_address, 8))
    });
    assert!(rejected.0.is_null());
    assert_eq!(allocations, 0);
    invalid_io(&status);
    assert_eq!(bytes, [0x55; 8]);
    assert_eq!(incoming, [0x66; 8]);
    let plan = unsafe { AbiPlan::new(&key, &read(address, 8)) };
    let (repeated, status) = unsafe { plan.append(&key, &read(incoming_address, 8)) };
    assert!(!repeated.0.is_null());
    clear_io(&status);
    drop(repeated);
    assert_eq!(bytes, [0x55; 8]);
    assert_eq!(incoming, [0x66; 8]);
    for malformed in [
        DrTransferBufferV1 {
            data: incoming_address,
            length: 8,
            reading: 2,
        },
        read(ptr::null_mut(), 8),
        read(incoming_address, isize::MAX as usize + 1),
    ] {
        // These metadata forms are explicitly rejected before conversion;
        // no invalid Rust slice or second writable loan is constructed.
        let mut status = poisoned_io();
        let (rejected, allocations) = crate::probe_runtime_allocations(|| unsafe {
            AbiPlan(dr_v1_transfer_plan_new(
                core::ptr::null(),
                (&key as *const PlanKey).cast(),
                &malformed,
                &mut status,
            ))
        });
        assert!(rejected.0.is_null());
        assert_eq!(allocations, 0);
        invalid_io(&status);
        let plan = unsafe { AbiPlan::new(&key, &read(address, 8)) };
        let ((rejected, status), allocations) = crate::probe_runtime_allocations(|| unsafe {
            // A distinct issued key from the same set isolates the malformed
            // incoming descriptor check from the key checks.
            plan.append(&sibling_key, &malformed)
        });
        assert!(rejected.0.is_null());
        assert_eq!(allocations, 0);
        invalid_io(&status);
    }
    assert_eq!(bytes, [0x55; 8]);
    assert_eq!(incoming, [0x66; 8]);
    let wait = budget(2, 5_000_000_000);
    let plan = unsafe { AbiPlan::new(&key, &read(address, 8)) };
    let (mut group, status) = unsafe { plan.begin(&set, &wait) };
    assert!(!group.0.is_null());
    clear_io(&status);
    data_completion(&group.next(), 404, 0, 6);
    drop(group);
    assert_eq!(&bytes[..6], b"intact");
    assert_eq!(incoming, [0x66; 8]);
    drop(foreign_client);
}

#[test]
fn private_plan_abi_stale_and_active_membership_reject_before_budget_and_io() {
    let _serial = TEST_BROKER_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (server, client) = connected_pair();
    let mut initial = [InlineStream::adopt(server)];
    let identity = initial[0].identity();
    let former = AbiSet::new(&[707], &mut initial, ptr::null_mut());
    let stale_key = former.key(0);
    let mut recovered = former.release::<1>();
    assert_eq!(recovered[0].identity(), identity);
    let set = AbiSet::new(&[707], &mut recovered, ptr::null_mut());
    let current_key = set.key(0);
    assert_eq!(current_key.value, stale_key.value);
    assert_ne!(current_key.owner, stale_key.owner);
    let mut peer = peer(client);
    write(&mut peer, b"fresh");
    let (mut stale_bytes, mut active_bytes, mut rejected_bytes) =
        ([0x11_u8; 8], [0x22_u8; 8], [0x33_u8; 8]);
    let stale_plan = unsafe { AbiPlan::new(&stale_key, &read(stale_bytes.as_mut_ptr(), 8)) };
    // A null budget is permitted on this checked rejection path. It proves
    // retired owner stamps reject before budget access/preparation allocation.
    let ((rejected, status), allocations) =
        crate::probe_runtime_allocations(|| unsafe { stale_plan.begin(&set, ptr::null()) });
    assert!(rejected.0.is_null());
    assert_eq!(allocations, 0);
    invalid_io(&status);
    assert_eq!(stale_bytes, [0x11; 8]);
    let wait = budget(2, 5_000_000_000);
    let plan = unsafe { AbiPlan::new(&current_key, &read(active_bytes.as_mut_ptr(), 8)) };
    let (mut group, status) = unsafe { plan.begin(&set, &wait) };
    assert!(!group.0.is_null());
    clear_io(&status);
    let mut key_output = PlanKey {
        owner: usize::MAX,
        value: u64::MAX,
    };
    let mut status = poisoned_io();
    let (tag, allocations) = crate::probe_runtime_allocations(|| unsafe {
        dr_v1_transfer_key(
            set.0,
            0,
            (&mut key_output as *mut PlanKey).cast(),
            &mut status,
        )
    });
    assert_eq!((tag, allocations), (3, 0));
    invalid_io(&status);
    assert_eq!(key_output, PlanKey { owner: 0, value: 0 });
    let second_plan = unsafe { AbiPlan::new(&current_key, &read(rejected_bytes.as_mut_ptr(), 8)) };
    let ((rejected, status), allocations) =
        crate::probe_runtime_allocations(|| unsafe { second_plan.begin(&set, ptr::null()) });
    assert!(rejected.0.is_null());
    assert_eq!(allocations, 0);
    invalid_io(&status);
    assert_eq!(rejected_bytes, [0x33; 8]);
    data_completion(&group.next(), 707, 0, 5);
    drop(group);
    assert_eq!(&active_bytes[..5], b"fresh");
    let [released] = set.release::<1>();
    assert_eq!(released.identity(), identity);
}

#[test]
fn borrowed_common_abi_removal_readd_and_active_rejection_preserve_streams_and_keys() {
    let _serial = TEST_BROKER_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (server_a, client_a) = connected_pair();
    let (server_b, client_b) = connected_pair();
    let (server_c, client_c) = connected_pair();
    let streams = [
        InlineStream::adopt(server_a),
        InlineStream::adopt(server_b),
        InlineStream::adopt(server_c),
    ];
    let identities = [
        streams[0].identity(),
        streams[1].identity(),
        streams[2].identity(),
    ];
    let set = AbiSet::borrowed(ptr::null_mut());
    let (key_a, key_b, key_c) = (
        set.add(&streams[0]),
        set.add(&streams[1]),
        set.add(&streams[2]),
    );
    assert_eq!(key_a.owner, key_b.owner);
    assert_eq!(key_a.owner, key_c.owner);
    assert_eq!(set.key(2), key_c);
    let mut peer_a = peer(client_a);
    let mut peer_b = peer(client_b);
    let mut peer_c = peer(client_c);
    write(&mut peer_b, b"retained");
    let (((tag, status), allocations), closes) = crate::native_handle::with_close_probe(|| {
        crate::probe_runtime_allocations(|| set.remove(&key_b))
    });
    assert_eq!((tag, allocations, closes), (0, 0, 0));
    clear_io(&status);
    assert_eq!(set.key(0), key_a);
    assert_eq!(set.key(2), key_c);
    let ((tag, status), allocations) = crate::probe_runtime_allocations(|| set.remove(&key_b));
    assert_eq!((tag, allocations), (3, 0));
    invalid_io(&status);

    let mut destination = InlineStream::empty();
    unsafe {
        ptr::write_bytes(
            destination.storage.as_mut_ptr().cast::<u8>(),
            0xa5,
            mem::size_of::<AttachedStream>(),
        );
    }
    let destinations = [destination.address()];
    let mut status = poisoned_io();
    let ((tag, allocations), closes) = crate::native_handle::with_close_probe(|| {
        crate::probe_runtime_allocations(|| unsafe {
            dr_v1_windows_transfer_set_release(set.0, destinations.as_ptr(), &mut status)
        })
    });
    assert_eq!((tag, allocations, closes), (3, 0, 0));
    invalid_io(&status);
    assert!(unsafe {
        core::slice::from_raw_parts(
            destination.storage.as_ptr().cast::<u8>(),
            mem::size_of::<AttachedStream>(),
        )
    }
    .iter()
    .all(|byte| *byte == 0xa5));
    let mut status = poisoned_io();
    // Borrowed mode rejects the legacy full-set path before inspecting these
    // explicitly permitted null inputs or forming any descriptor/byte slices.
    let (rejected, allocations) = crate::probe_runtime_allocations(|| unsafe {
        AbiGroup(dr_v1_transfer_group_begin(
            core::ptr::null(),
            set.0,
            ptr::null(),
            usize::MAX,
            ptr::null(),
            &mut status,
        ))
    });
    assert!(rejected.0.is_null());
    assert_eq!(allocations, 0);
    invalid_io(&status);
    let (mut stale_bytes, mut bytes_c, mut bytes_a) = ([0x44_u8; 12], [0x33_u8; 12], [0x11_u8; 12]);
    let stale = unsafe { AbiPlan::new(&key_b, &read(stale_bytes.as_mut_ptr(), 12)) };
    let ((rejected, status), allocations) =
        crate::probe_runtime_allocations(|| unsafe { stale.begin(&set, ptr::null()) });
    assert!(rejected.0.is_null());
    assert_eq!(allocations, 0);
    invalid_io(&status);
    assert_eq!(stale_bytes, [0x44; 12]);

    let (address_c, address_a) = (bytes_c.as_mut_ptr(), bytes_a.as_mut_ptr());
    let plan = unsafe { AbiPlan::new(&key_c, &read(address_c, 12)) };
    let (plan, status) = unsafe { plan.append(&key_a, &read(address_a, 12)) };
    assert!(!plan.0.is_null());
    clear_io(&status);
    let wait = budget(2, 5_000_000_000);
    let (mut group, status) = unsafe { plan.begin(&set, &wait) };
    assert!(!group.0.is_null());
    clear_io(&status);
    let mut key_output = PlanKey {
        owner: usize::MAX,
        value: u64::MAX,
    };
    let mut status = poisoned_io();
    let ((tag, allocations), closes) = crate::native_handle::with_close_probe(|| {
        crate::probe_runtime_allocations(|| unsafe {
            dr_v1_transfer_set_add(
                set.0,
                ptr::null(),
                (&mut key_output as *mut PlanKey).cast(),
                &mut status,
            )
        })
    });
    assert_eq!((tag, allocations, closes), (3, 0, 0));
    invalid_io(&status);
    assert_eq!(key_output, PlanKey { owner: 0, value: 0 });
    let mut status = poisoned_io();
    let (tag, allocations) = crate::probe_runtime_allocations(|| unsafe {
        dr_v1_transfer_set_remove(set.0, ptr::null(), &mut status)
    });
    assert_eq!((tag, allocations), (3, 0));
    invalid_io(&status);
    write(&mut peer_c, b"cee");
    data_completion(&group.next(), key_c.value, 0, 3);
    assert_eq!(group.region(0, 0), (0, address_c, 3));
    write(&mut peer_a, b"aye");
    data_completion(&group.next(), key_a.value, 1, 3);
    assert_eq!(group.region(1, 0), (0, address_a, 3));
    exhausted(&mut group);
    drop(group);
    assert_eq!(&bytes_c[..3], b"cee");
    assert_eq!(&bytes_a[..3], b"aye");
    assert_eq!(set.key(0), key_a);
    assert_eq!(set.key(2), key_c);

    let new_key_b = set.add(&streams[1]);
    assert_eq!(new_key_b.owner, key_b.owner);
    assert!(new_key_b.value > key_c.value);
    assert_ne!(new_key_b, key_b);
    let stale = unsafe { AbiPlan::new(&key_b, &read(stale_bytes.as_mut_ptr(), 12)) };
    let ((rejected, status), allocations) =
        crate::probe_runtime_allocations(|| unsafe { stale.begin(&set, ptr::null()) });
    assert!(rejected.0.is_null());
    assert_eq!(allocations, 0);
    invalid_io(&status);
    let mut bytes_b = [0_u8; 12];
    let plan = unsafe { AbiPlan::new(&new_key_b, &read(bytes_b.as_mut_ptr(), 12)) };
    let wait = budget(2, 5_000_000_000);
    let (mut group, status) = unsafe { plan.begin(&set, &wait) };
    assert!(!group.0.is_null());
    clear_io(&status);
    data_completion(&group.next(), new_key_b.value, 0, 8);
    drop(group);
    assert_eq!(&bytes_b[..8], b"retained");
    let ((), closes) = crate::native_handle::with_close_probe(|| drop(set));
    assert_eq!(closes, 0);
    for (stream, identity) in streams.iter().zip(identities) {
        assert_eq!(stream.identity(), identity);
    }
}

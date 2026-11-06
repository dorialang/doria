//! Windows configuration and immutable stream attachment for the common group
//! transport. Group lifetime, buffer validation and cleanup live in that shared
//! transport, not in a second Windows opaque-owner implementation.

use super::*;
use crate::transfer_group::setup::GroupSetup;
use crate::transfer_group::{transport, BufferList, GroupDriver, SetupFailure, TerminalOutcome};
use crate::DrStackFrameV2;

pub(crate) type NativeTransferGroup<'s, 'b> = driver::NativeGroup<'s, 'b>;
pub(in super::super) type PendingDispatcher<'s> = driver::Dispatcher<'s>;
pub(in super::super) type PendingControl<'s> = driver::BrokerControl<'s>;

#[cfg(test)]
pub(in super::super) fn pending_test_lock() -> std::sync::MutexGuard<'static, ()> {
    TEST_BROKER_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
pub(in super::super) fn pending_native_dequeues() -> usize {
    native_dequeues_for_test()
}

pub(crate) struct NativeTransferSet {
    completion: CompletionSet,
    cancellation: Option<CancellationClaim>,
}

impl NativeTransferSet {
    #[cfg(test)]
    pub(in super::super) fn pending_snapshot(&self, index: usize) -> (usize, usize, usize, usize) {
        self.completion.borrowed_snapshot_for_test(index)
    }

    #[cfg(test)]
    pub(in super::super) fn pending_assert_retired(&self) {
        self.completion.assert_retired_for_test();
    }

    unsafe fn new(
        keys: &[u64],
        streams: FixedSlice<AttachedStream>,
        cancellation: *mut DrCancellationV1,
    ) -> Result<Self, Failure> {
        if keys.is_empty() {
            return Err(Failure::invalid_input());
        }
        Ok(Self {
            completion: CompletionSet::from_attached(keys, streams)?,
            cancellation: CancellationClaim::acquire(cancellation)?,
        })
    }

    /// Dynamic membership borrows compiler-retained complete stream owners.
    /// The set owns only registration/request metadata and its cancellation
    /// claim; stream attachment and the sole native close owner stay elsewhere.
    /// # Safety
    /// Cancellation is null or has a live claim during acquisition.
    pub(crate) unsafe fn borrowed_new(
        cancellation: *mut DrCancellationV1,
    ) -> Result<Self, Failure> {
        Ok(Self {
            completion: CompletionSet::borrowed_new()?,
            cancellation: CancellationClaim::acquire(cancellation)?,
        })
    }

    pub(crate) fn is_borrowed(&self) -> bool {
        self.completion.is_borrowed()
    }

    /// # Safety
    /// Projection names a live AttachedStream during this call only. Its complete
    /// owner remains exclusively retained by the compiler until removal or set
    /// destruction, after all groups finish. Captured native metadata is borrowed;
    /// registration and its rejection neither consume nor close that owner.
    pub(crate) unsafe fn add_borrowed(
        &mut self,
        key: u64,
        projection: *const c_void,
    ) -> Result<usize, Failure> {
        self.completion.add_borrowed(key, projection)
    }

    /// # Safety
    /// File is a stable exclusively retained complete owner through removal and
    /// all group teardown. Admission rejects nonregular handles; metadata owns
    /// no native close claim and never associates this synchronous File to IOCP.
    pub(in super::super) unsafe fn add_file(
        &mut self,
        key: u64,
        file: *mut crate::file_io::stream::OwnedFile,
    ) -> Result<usize, Failure> {
        self.completion.add_file(key, file)
    }

    pub(in super::super) unsafe fn add_standard(
        &mut self,
        key: u64,
        stream: crate::device_io::StandardStream,
    ) -> Result<usize, Failure> {
        self.completion.add_standard(key, stream)
    }

    pub(in super::super) fn validate_pending_registration(
        &self,
        index: usize,
    ) -> Result<(), Failure> {
        self.completion.validate_pending_registration(index)
    }

    pub(in super::super) unsafe fn add_bound_source(
        &mut self,
        key: u64,
        source: *const c_void,
        provider: crate::transfer_group::source::SourceProvider,
    ) -> Result<usize, Failure> {
        self.completion.add_bound_source(key, source, provider)
    }

    pub(in super::super) unsafe fn update_external_source(
        &mut self,
        index: usize,
        source: *const c_void,
    ) -> Result<crate::native_handle::PhysicalOrigin, Failure> {
        self.completion.update_external_source(index, source)
    }

    pub(crate) fn remove_borrowed(&mut self, key: u64) -> Result<(), Failure> {
        self.completion.remove_borrowed(key)
    }

    pub(crate) fn len(&self) -> usize {
        self.completion.len()
    }

    pub(in super::super) fn cached_operation(
        &self,
        registration: usize,
        ordinal: usize,
    ) -> Option<usize> {
        self.completion.cached_operation(registration, ordinal)
    }

    pub(in super::super) fn operation_registration(&self, operation: usize) -> usize {
        self.completion.operation_registration(operation)
    }

    pub(in super::super) fn configure_operations(
        &mut self,
        selection: FixedSlice<usize>,
    ) -> Result<FixedSlice<usize>, Failure> {
        self.completion.configure_operations(selection)
    }

    pub(in super::super) fn remove_borrowed_with(
        &mut self,
        key: u64,
        retire: impl FnMut(usize),
    ) -> Result<(), Failure> {
        self.completion.remove_borrowed_with(key, retire)
    }

    pub(crate) fn index_of(&self, key: u64) -> Option<usize> {
        self.completion.index_of(key)
    }

    pub(crate) fn key(&self, index: usize) -> Option<u64> {
        self.completion.key(index)
    }

    pub(crate) fn begin<'s, 'b>(
        &'s mut self,
        buffers: BufferList<'b>,
        budget: &DrWaitBudgetV1,
    ) -> Result<NativeTransferGroup<'s, 'b>, Failure> {
        self.completion.begin(
            buffers,
            Some(budget),
            self.cancellation.as_ref().map(|claim| unsafe { &*claim.0 }),
        )
    }

    pub(crate) fn begin_selected<'s, 'b>(
        &'s mut self,
        buffers: BufferList<'b>,
        budget: &DrWaitBudgetV1,
        selection: FixedSlice<usize>,
    ) -> Result<NativeTransferGroup<'s, 'b>, Failure> {
        self.completion.begin_selected(
            buffers,
            Some(budget),
            self.cancellation.as_ref().map(|claim| unsafe { &*claim.0 }),
            selection,
        )
    }

    pub(in super::super) fn cancellation_policy(&self) -> *mut DrCancellationV1 {
        self.cancellation
            .as_ref()
            .map_or(ptr::null_mut(), |claim| claim.0)
    }

    /// # Safety
    /// A nonnull state is live for acquisition. Completion machinery retains
    /// its own group-local claim before any operation can be submitted.
    pub(in super::super) unsafe fn configure_selected_prepared_with_driver_controlled<
        's,
        'b,
        D,
        T,
    >(
        &'s mut self,
        buffers: BufferList<'b, T>,
        budget: &DrWaitBudgetV1,
        selection: FixedSlice<usize>,
        cancellation: *mut DrCancellationV1,
        wrap: impl FnOnce(driver::Dispatcher<'s>) -> D,
    ) -> Result<GroupSetup<'s, 'b, D, driver::BrokerControl<'s>, T>, SetupFailure<T>>
    where
        D: GroupDriver<T, Request = Overlapped, Timeout = u32>,
        T: TerminalOutcome,
    {
        self.completion.configure_selected_prepared_with_driver(
            buffers,
            Some(budget),
            cancellation.as_ref(),
            selection,
            wrap,
        )
    }

    pub(crate) fn project_completion(
        status: &mut DrCompletionStatusV1,
        awaited: Awaited,
        reading: bool,
        _length: usize,
    ) -> u8 {
        // Successful zero-count byte-pipe completion remains data. Only the
        // Windows boundary's broken-pipe fact identifies EOF, not Unix length.
        super::super::project_completion(status, awaited, reading)
    }

    fn into_attached(self) -> FixedSlice<AttachedStream> {
        self.completion.into_attached()
    }
}

/// # Safety
/// Outputs are disjoint writable usize storage. Stream owners use this inline
/// layout and may move while unborrowed; their native associations stay pinned.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_windows_transfer_stream_layout(size: *mut usize, align: *mut usize) {
    *size = mem::size_of::<AttachedStream>();
    *align = mem::align_of::<AttachedStream>();
}

/// # Safety
/// Handle transfers an independently owned idle FILE_FLAG_OVERLAPPED, byte-mode
/// PIPE_WAIT pipe, with no IOCP/APC/external I/O or close consumer and no native
/// completion-notification suppression. Every accepted read/write must produce
/// a packet. Owner is uninitialized aligned storage of the queried stream
/// layout, disjoint from writable status. Failure also consumes the handle.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_windows_transfer_stream_adopt(
    owner: *mut c_void,
    handle: NativeHandle,
    status: *mut DrIoStatusV1,
) -> u8 {
    *status = DrIoStatusV1::default();
    (*status).complete(
        AttachedStream::adopt(OwnedHandle::from_raw(handle)).map(|stream| {
            owner.cast::<AttachedStream>().write(stream);
            0
        }),
    )
}

/// # Safety
/// Consumes one live unborrowed inline stream owner. It cannot belong to a set
/// or have a pending request. After this one close attempt storage is uninitialized,
/// including on failure; failure is not a retryable ownership claim.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_windows_transfer_stream_close(
    owner: *mut c_void,
    status: *mut DrIoStatusV1,
) -> u8 {
    *status = DrIoStatusV1::default();
    (*status).complete(owner.cast::<AttachedStream>().read().close().map(|()| 0))
}

/// # Safety
/// Consumes one live unborrowed inline stream owner exactly once, without a
/// checked close result. No group/set may retain it or its native handle.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_windows_transfer_stream_drop(owner: *mut c_void) {
    ptr::drop_in_place(owner.cast::<AttachedStream>());
}

/// Private fixed configuration; transfers all incoming stream owners on either
/// success or failure. No attach/detach/reopen/duplicate occurs during this move.
/// # Safety
/// Arrays contain count keys and distinct pointers to live unborrowed inline
/// stream owners. Input arrays, owners and writable status are disjoint. Each
/// owner is consumed once and its storage becomes uninitialized. Cancellation
/// is null or has a live claim during acquisition. No handle has another consumer.
/// Frame is null or live for owner-allocation failure reporting during this call.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_windows_transfer_set_new(
    frame: *const DrStackFrameV2,
    keys: *const u64,
    streams: *const *mut c_void,
    count: usize,
    cancellation: *mut DrCancellationV1,
    status: *mut DrIoStatusV1,
) -> *mut c_void {
    *status = DrIoStatusV1::default();
    if count == 0 {
        (*status).fail(Failure::invalid_input());
        return ptr::null_mut();
    }
    let keys = core::slice::from_raw_parts(keys, count);
    let streams = core::slice::from_raw_parts(streams, count);
    let mut owners = match FixedSlice::new(count) {
        Ok(owners) => owners,
        Err(failure) => {
            for owner in streams {
                ptr::drop_in_place(owner.cast::<AttachedStream>());
            }
            (*status).fail(failure);
            return ptr::null_mut();
        }
    };
    for owner in streams {
        if owners.push(owner.cast::<AttachedStream>().read()).is_err() {
            crate::runtime_invariant_panic();
        }
    }
    match NativeTransferSet::new(keys, owners, cancellation)
        .and_then(|native| transport::allocate_set(frame, native))
    {
        Ok(owner) => owner,
        Err(failure) => {
            (*status).fail(failure);
            ptr::null_mut()
        }
    }
}

/// Returns the same attached owners to inline destination storage. This is a
/// configuration move, never an I/O submission or native reassociation.
/// # Safety
/// Status is live writable storage disjoint from the set and all inputs. Set is
/// null or live; null/active/borrowed-mode sets are permitted only for checked
/// rejection, which accesses no native state or destinations and preserves
/// ownership. Otherwise set is uniquely
/// owned with no active group, and the destination array has its configured
/// stream count, each pointing to distinct uninitialized aligned stream-owner
/// storage. Destinations and array are disjoint from the set allocation and
/// status. Success consumes the set once; destinations then own the streams,
/// which may be regrouped or closed independently.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_windows_transfer_set_release(
    owner: *mut c_void,
    streams: *const *mut c_void,
    status: *mut DrIoStatusV1,
) -> u8 {
    *status = DrIoStatusV1::default();
    (*status).complete(transport::take_set(owner).map(|set| {
        let destinations = core::slice::from_raw_parts(streams, set.len());
        for (stream, destination) in set.into_attached().into_iter().zip(destinations) {
            destination.cast::<AttachedStream>().write(stream);
        }
        0
    }))
}

#[cfg(test)]
mod tests;

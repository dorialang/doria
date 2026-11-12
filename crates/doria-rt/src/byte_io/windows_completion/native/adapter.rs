//! Borrowed regular-file sources dispatch through the complete adapter. Disk
//! readiness is advisory, as on Unix; it is not an IOCP association or a promise
//! that a synchronous callback cannot block. Completion-capable pipes retain
//! their separate native packet-retirement driver.

use core::{ffi::c_void, ptr};

use super::{CancellationClaim, WaitControl};
use crate::cancellation::{self, DrCancellationV1};
use crate::native_handle::NativeHandle;
use crate::platform_io::{Failure, Reason};
use crate::transfer_group::adapter::{AdapterAttempt, AdapterTerminal, DrTransferAdapterV1};
use crate::transfer_group::source::{
    ResolvedSource, SelectedSource, SourceBinding, SourceProvider, SourceRoutes,
};
use crate::transfer_group::{
    Buffer, BufferList, FixedSlice, GroupDriver, Query, RequestSlots, SetupFailure, StableSlots,
    Stop, Submission, TransferGroup, Wake,
};
use crate::wait_budget::DrWaitBudgetV1;
use crate::DrStackFrameV2;

struct Request {
    registration: usize,
    source: NativeHandle,
    adapter: DrTransferAdapterV1,
    data: *mut u8,
    length: usize,
    reading: bool,
    terminal: Option<AdapterTerminal>,
    selected: Option<SelectedSource<()>>,
}

impl Request {
    fn new(registration: usize, source: NativeHandle, adapter: DrTransferAdapterV1) -> Self {
        Self {
            registration,
            source,
            adapter,
            data: ptr::null_mut(),
            length: 0,
            reading: false,
            terminal: None,
            selected: None,
        }
    }

    fn retire(&mut self) {
        // No native operation has escaped a callback. Only the shared group's
        // retained loan kept this pointer available for a readiness retry.
        self.data = ptr::null_mut();
    }
}

struct Registered {
    key: u64,
    source: NativeHandle,
    adapter: DrTransferAdapterV1,
    binding: Option<SourceBinding>,
}

pub(crate) struct AdapterSet {
    slots: RequestSlots<Request>,
    registrations: StableSlots<Registered>,
    sources: SourceRoutes<()>,
    cancellation: Option<CancellationClaim>,
    last_key: u64,
}

impl AdapterSet {
    /// # Safety
    /// Cancellation is null or a live claim throughout acquisition. The set
    /// retains that claim but acquires no source-handle or adapter close owner.
    pub(crate) unsafe fn new(cancellation: *mut DrCancellationV1) -> Result<Self, Failure> {
        Ok(Self {
            slots: RequestSlots::dynamic()?,
            registrations: StableSlots::new()?,
            sources: SourceRoutes::new()?,
            cancellation: CancellationClaim::acquire(cancellation)?,
            last_key: 0,
        })
    }

    /// # Safety
    /// The candidate source is validated before admission. On admission source
    /// and callback context belong to a complete compiler-retained adapter and
    /// remain live until removal or set destruction, after every group finishes.
    /// Callbacks complete synchronously and never retain the
    /// supplied frame or byte region, unwind, or mutate this set reentrantly.
    /// Admission and rejection neither consume nor close the complete owner.
    pub(crate) unsafe fn add(
        &mut self,
        key: u64,
        source: NativeHandle,
        adapter: DrTransferAdapterV1,
    ) -> Result<usize, Failure> {
        if key == 0
            || key == u64::MAX
            || key <= self.last_key
            || !adapter.valid()
            || (0..self.registrations.len()).any(|index| {
                self.registrations
                    .raw(index)
                    .is_some_and(|record| (*record).binding.is_none() && (*record).source == source)
            })
        {
            return Err(Failure::invalid_input());
        }
        validate_source(source)?;
        // All configuration is staged by the sparse storage before publication.
        // Existing registrations and their request addresses never move.
        let index = self.registrations.vacant_index();
        self.registrations.insert_at(
            index,
            Registered {
                key,
                source,
                adapter,
                binding: None,
            },
        )?;
        if let Err(failure) = self
            .slots
            .add_registration(index, key, |_| Ok(Request::new(index, source, adapter)))
        {
            self.registrations.remove(index);
            return Err(failure);
        }
        self.last_key = key;
        Ok(index)
    }

    /// # Safety
    /// Source is live typed metadata during admission; Native provider retains
    /// the exact complete carrier home. Each later callback/wait uses a freshly
    /// resolved File resource, never the cached wrapper or handle.
    pub(crate) unsafe fn add_bound_source(
        &mut self,
        key: u64,
        source: *const c_void,
        provider: SourceProvider,
        adapter: DrTransferAdapterV1,
    ) -> Result<usize, Failure> {
        let resolved = ResolvedSource::borrow(source, None)?;
        require_file(resolved.origin())?;
        let handle = resolved.origin().handle()?;
        self.sources.reserve(
            self.registrations
                .occupied_len()
                .checked_add(2)
                .ok_or_else(exhausted)?,
        )?;
        let mut binding = SourceBinding::new(provider);
        self.sources
            .rebind(&mut binding, resolved, |_, origin, _, resource| {
                require_file(origin)?;
                if !resource.shared_cursor {
                    return Err(unsupported());
                }
                Ok(())
            })?;
        match self.add(key, handle, adapter) {
            Ok(index) => {
                (*self.registrations.raw(index).unwrap()).binding = Some(binding);
                Ok(index)
            }
            Err(failure) => {
                self.sources.unbind(&mut binding);
                Err(failure)
            }
        }
    }

    pub(crate) fn remove(&mut self, key: u64) -> Result<(), Failure> {
        let index = self.index_of(key).ok_or_else(Failure::invalid_input)?;
        for operation in 0..self.slots.len() {
            if self.slots.contains(operation) && self.slots.registration(operation) == index {
                let request = unsafe { &*self.slots.raw(operation) };
                if !request.data.is_null() || request.selected.is_some() {
                    return Err(Failure::invalid_input());
                }
            }
        }
        self.slots.remove_registration(index)?;
        let mut record = self
            .registrations
            .remove(index)
            .ok_or_else(Failure::invalid_input)?;
        if let Some(binding) = &mut record.binding {
            unsafe { self.sources.unbind(binding) };
        }
        Ok(())
    }

    pub(crate) fn index_of(&self, key: u64) -> Option<usize> {
        (0..self.registrations.len()).find(|index| self.key(*index) == Some(key))
    }

    pub(crate) fn key(&self, index: usize) -> Option<u64> {
        self.registrations
            .raw(index)
            .map(|record| unsafe { (*record).key })
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.registrations.occupied_len()
    }

    /// # Safety
    /// Frame is null or live for this call. The retained adapters obey add's
    /// callback contract; buffers remain exclusively retained by the resulting
    /// shared group. No callback can move or mutate the set during execution.
    pub(crate) unsafe fn begin_selected<'s, 'b>(
        &'s mut self,
        frame: *const DrStackFrameV2,
        buffers: BufferList<'b, AdapterTerminal>,
        budget: &DrWaitBudgetV1,
        selection: FixedSlice<usize>,
    ) -> Result<AdapterGroup<'s, 'b>, SetupFailure<AdapterTerminal>> {
        let cancellation = self
            .cancellation
            .as_ref()
            .map_or(ptr::null_mut(), |claim| claim.0);
        self.begin_selected_controlled(frame, buffers, budget, selection, cancellation)
    }

    /// # Safety
    /// Same frame and loan contract as begin_selected. A nonnull cancellation
    /// state is live for acquisition; only the group retains the new claim.
    pub(crate) unsafe fn begin_selected_controlled<'s, 'b>(
        &'s mut self,
        frame: *const DrStackFrameV2,
        buffers: BufferList<'b, AdapterTerminal>,
        budget: &DrWaitBudgetV1,
        selection: FixedSlice<usize>,
        state: *mut DrCancellationV1,
    ) -> Result<AdapterGroup<'s, 'b>, SetupFailure<AdapterTerminal>> {
        if selection.len() != buffers.len() {
            return Err(Failure::invalid_input().into());
        }
        for (position, index) in selection.as_slice().iter().enumerate() {
            if self.key(*index).is_none() {
                return Err(Failure::invalid_input().into());
            }
            let (reading, _) = buffers
                .request_info(position)
                .ok_or_else(Failure::invalid_input)?;
            (*self.registrations.raw(*index).unwrap())
                .adapter
                .validate(reading)?;
        }
        let bound = (0..self.registrations.len())
            .filter(|index| {
                self.registrations
                    .raw(*index)
                    .is_some_and(|record| (*record).binding.is_some())
            })
            .count();
        if bound != 0 {
            self.sources.reserve(
                bound
                    .checked_add(selection.len())
                    .and_then(|count| count.checked_add(1))
                    .ok_or_else(exhausted)?,
            )?;
        }
        let registrations = &self.registrations;
        let selection = self.slots.configure_operations(selection, |index, _| {
            let registration = &*registrations.raw(index).unwrap();
            Ok(Request::new(
                index,
                registration.source,
                registration.adapter,
            ))
        })?;
        let control = WaitControl::new(Some(budget), state)?;
        let driver = Dispatcher {
            frame,
            cancellation: state,
            registrations: &mut self.registrations,
            sources: &mut self.sources,
        };
        // The same core controls setup, stop checks, typed terminal facts and
        // one cancel-all/drain-all teardown. It clears the initial call frame.
        Ok(AdapterGroup {
            inner: TransferGroup::begin_selected_prepared(
                &mut self.slots,
                buffers,
                driver,
                control,
                selection,
            )?,
        })
    }
}

fn unsupported() -> Failure {
    Failure {
        reason: Reason::Unsupported,
        system_code: None,
        transferred_byte_count: None,
    }
}

fn exhausted() -> Failure {
    Failure {
        reason: Reason::ResourceExhausted,
        system_code: None,
        transferred_byte_count: None,
    }
}

fn require_file(origin: crate::native_handle::PhysicalOrigin) -> Result<(), Failure> {
    if matches!(origin, crate::native_handle::PhysicalOrigin::File { .. }) {
        Ok(())
    } else {
        Err(unsupported())
    }
}

fn validate_source(source: NativeHandle) -> Result<(), Failure> {
    use crate::win32::{GetFileType, GetLastError};
    // https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getfiletype
    // UNKNOWN with a last error is a checked handle failure, not a disk default.
    match unsafe { GetFileType(source) } {
        1 => Ok(()), // FILE_TYPE_DISK: immediate advisory source, not an IOCP.
        0 => {
            let code = unsafe { GetLastError() };
            if code == 0 {
                Err(unsupported())
            } else {
                Err(Failure::from_system_code(i64::from(code)))
            }
        }
        // Synchronous custom pipe/console adapters cannot manufacture a pending
        // native transfer without violating their call-scoped region contract.
        _ => Err(unsupported()),
    }
}

struct Dispatcher<'s> {
    frame: *const DrStackFrameV2,
    cancellation: *mut DrCancellationV1,
    registrations: &'s mut StableSlots<Registered>,
    sources: &'s mut SourceRoutes<()>,
}

impl Dispatcher<'_> {
    unsafe fn resolve_current(
        &mut self,
        request: &mut Request,
        selecting: bool,
    ) -> Result<(), AdapterTerminal> {
        let record = &mut *self.registrations.raw(request.registration).unwrap();
        let Some(binding) = record.binding.as_mut() else {
            return Ok(());
        };
        let resolved = binding.resolve(self.frame, None)?;
        require_file(resolved.origin()).map_err(AdapterTerminal::PlatformFailure)?;
        let route = self
            .sources
            .rebind(binding, resolved, |_, origin, _, resource| {
                require_file(origin)?;
                if !resource.shared_cursor {
                    return Err(unsupported());
                }
                Ok(())
            })
            .map_err(AdapterTerminal::PlatformFailure)?;
        if selecting {
            let selected = self
                .sources
                .select(route)
                .map_err(AdapterTerminal::PlatformFailure)?;
            request.source = selected.handle();
            request.adapter.context = selected
                .context()
                .expect("native synchronous dispatch context");
            request.selected = Some(selected);
        }
        Ok(())
    }

    fn release(request: &mut Request) {
        request.retire();
        request.selected = None;
    }

    unsafe fn attempt(&mut self, request: &mut Request) {
        let result = if request.reading {
            request
                .adapter
                .read(self.frame, request.data, request.length)
        } else {
            request
                .adapter
                .write(self.frame, request.data.cast_const(), request.length)
        };
        if let AdapterAttempt::Terminal(terminal) = result {
            request.retire();
            request.terminal = Some(terminal);
        } else if let Err(terminal) = self.resolve_current(request, true) {
            request.retire();
            request.terminal = Some(terminal);
        }
    }
}

// Disk sources are borrowed, and every callback releases its native/byte use
// before return. Pending is only an advisory-readiness retry in the shared
// group, never escaped kernel I/O. Cancel/drain perform no adapter callback.
unsafe impl GroupDriver<AdapterTerminal> for Dispatcher<'_> {
    type Request = Request;
    type Timeout = u32;
    const MAXIMUM_TRANSFER_BYTES: usize = isize::MAX as usize;
    const QUERY_CAN_TRANSFER: bool = true;
    const QUERY_REFRESHES_SOURCE: bool = true;
    const PREPARE_BEFORE_SUBMISSION: bool = true;
    const PREFLIGHT_EACH_SUBMISSION: bool = true;
    const EMPTY_MUST_DISPATCH: bool = true;

    unsafe fn resource(
        &self,
        _: usize,
        request: *mut Request,
    ) -> Result<crate::transfer_group::TransferResource, Failure> {
        if let Some(selected) = &(*request).selected {
            return Ok(selected.resource());
        }
        if (*self.registrations.raw((*request).registration).unwrap())
            .binding
            .is_some()
        {
            return Err(Failure::invalid_input());
        }
        // Fixed-owner admission supports regular files only.
        Ok(crate::transfer_group::TransferResource {
            identity: (*request).source as usize,
            shared_cursor: true,
        })
    }

    fn set_call_frame(&mut self, frame: *const DrStackFrameV2) {
        self.frame = frame;
    }

    fn call_frame(&self) -> *const DrStackFrameV2 {
        self.frame
    }

    unsafe fn setup_request(
        &mut self,
        _: usize,
        request: *mut Request,
    ) -> Result<(), AdapterTerminal> {
        self.resolve_current(&mut *request, false)
    }

    unsafe fn prepare_request(
        &mut self,
        _: usize,
        request: *mut Request,
        _: &mut Buffer<'_>,
    ) -> Result<(), AdapterTerminal> {
        if let Err(terminal) = self.resolve_current(&mut *request, true) {
            Self::release(&mut *request);
            return Err(terminal);
        }
        Ok(())
    }

    unsafe fn prepare_query(
        &mut self,
        _: usize,
        request: *mut Request,
    ) -> Result<(), AdapterTerminal> {
        if (*request).terminal.is_some() {
            return Ok(());
        }
        if let Err(terminal) = self.resolve_current(&mut *request, true) {
            Self::release(&mut *request);
            return Err(terminal);
        }
        Ok(())
    }

    unsafe fn release_request(&mut self, _: usize, request: *mut Request) {
        Self::release(&mut *request);
    }

    unsafe fn reset_request(&mut self, _: usize, request: *mut Request) {
        let request = &mut *request;
        if !request.data.is_null() || request.terminal.is_some() || request.selected.is_some() {
            crate::runtime_invariant_panic();
        }
        request.length = 0;
        request.reading = false;
    }

    unsafe fn submit(
        &mut self,
        _: usize,
        request: *mut Request,
        buffer: &mut Buffer<'_>,
    ) -> Submission {
        let request = &mut *request;
        match buffer {
            Buffer::Read(bytes) => {
                request.data = bytes.as_mut_ptr();
                request.length = bytes.len();
                request.reading = true;
            }
            Buffer::Write(bytes) => {
                request.data = bytes.as_ptr().cast_mut();
                request.length = bytes.len();
                request.reading = false;
            }
        }
        self.attempt(request);
        if request.terminal.is_some() {
            Submission::Complete
        } else {
            Submission::Pending
        }
    }

    unsafe fn query(&mut self, _: usize, request: *mut Request) -> Query<AdapterTerminal> {
        let request = &mut *request;
        if request.terminal.is_none() {
            self.attempt(request);
        }
        match request.terminal.take() {
            Some(terminal) => {
                request.selected = None;
                Query::Terminal(terminal)
            }
            None => Query::Pending,
        }
    }

    fn wait_any(&mut self, _: u32, _: Option<usize>) -> Result<Wake, Failure> {
        if !self.cancellation.is_null()
            && unsafe { cancellation::dr_v1_cancellation_is_requested(self.cancellation) } != 0
        {
            Ok(Wake::Cancellation)
        } else {
            // A regular-file source has no kernel readiness subscription. The
            // common core rechecks the original budget before callback retry.
            Ok(Wake::Completion)
        }
    }

    unsafe fn cancel(
        &mut self,
        _: usize,
        request: *mut Request,
    ) -> Result<(), (doria_diagnostic_catalogue::IoMessageOperation, Failure)> {
        let request = &mut *request;
        request.retire();
        if request.terminal.is_none() {
            request.terminal = Some(AdapterTerminal::NoTransfer);
        }
        Ok(())
    }

    unsafe fn drain(&mut self, _: usize, request: *mut Request) -> Option<AdapterTerminal> {
        Self::release(&mut *request);
        Some(
            (*request)
                .terminal
                .take()
                .unwrap_or(AdapterTerminal::NoTransfer),
        )
    }
}

pub(crate) struct AdapterGroup<'s, 'b> {
    inner: TransferGroup<'s, 'b, Dispatcher<'s>, WaitControl, AdapterTerminal>,
}

impl AdapterGroup<'_, '_> {
    /// # Safety
    /// Frame is null or live throughout cleanup and is never retained.
    pub(crate) unsafe fn drop_in_frame(self, frame: *const DrStackFrameV2) {
        self.inner.drop_in_frame(frame);
    }

    /// # Safety
    /// Frame is null or live for this call. Registered callbacks obey add's
    /// contract and do not retain the frame beyond this call.
    #[cfg(test)]
    pub(crate) unsafe fn next(
        &mut self,
        frame: *const DrStackFrameV2,
    ) -> Option<crate::transfer_group::Completion<AdapterTerminal>> {
        self.inner.with_call_frame(frame, |group| group.next())
    }

    /// Observes with a call-owned budget. Expiry never retires transfers.
    /// # Safety
    /// Frame is null or live for this call; neither frame nor budget is retained.
    pub(crate) unsafe fn observe(
        &mut self,
        frame: *const DrStackFrameV2,
        mut budget: DrWaitBudgetV1,
    ) -> crate::transfer_group::Observation<AdapterTerminal> {
        self.inner
            .with_call_frame(frame, |group| group.observe(&mut budget))
    }

    /// # Safety
    /// Frame is null or live for this call; callback and byte loans remain valid.
    pub(crate) unsafe fn submit(
        &mut self,
        frame: *const DrStackFrameV2,
        index: usize,
        offset: usize,
        length: usize,
    ) -> Result<(), Failure> {
        self.inner
            .with_call_frame(frame, |group| group.submit(index, offset, length))
    }

    pub(crate) fn request_info(&self, index: usize) -> Option<(bool, usize)> {
        self.inner.request_info(index)
    }

    pub(crate) fn completed(&self, index: usize) -> Option<&[u8]> {
        self.inner.completed(index)
    }

    pub(crate) fn read_storage_mut(&mut self, index: usize) -> Option<&mut [u8]> {
        self.inner.read_storage_mut(index)
    }

    pub(crate) fn stop(&self) -> Option<Stop> {
        self.inner.stop()
    }

    /// The plan's watched sources join this group's wait (Decision 0140).
    pub(crate) fn watch(
        &mut self,
        sources: &[crate::transfer_group::WatchSource],
    ) -> Result<(), Failure> {
        self.inner.watch(sources)
    }

    pub(crate) fn rearm_watch(&mut self, index: usize) -> Result<(), Failure> {
        self.inner.rearm_watch(index)
    }
}

#[cfg(test)]
mod tests;

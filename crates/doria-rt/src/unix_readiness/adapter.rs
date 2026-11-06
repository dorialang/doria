//! Complete adapters perform every transfer. The borrowed source is used only
//! for admission and advisory watches; it never grants another close owner.

use super::*;
use crate::transfer_group::adapter::{AdapterAttempt, AdapterTerminal, DrTransferAdapterV1};
use crate::transfer_group::source::{
    ResolvedSource, SelectedSource, SourceBinding, SourceProvider, SourceRoutes,
};
use crate::transfer_group::{
    Buffer, BufferList, FixedSlice, GroupDriver, Query, RequestSlots, SetupFailure, StableSlots,
    Stop, Submission, TransferGroup, Wake,
};
use crate::DrStackFrameV2;

struct Request {
    registration: usize,
    adapter: DrTransferAdapterV1,
    data: *mut u8,
    length: usize,
    reading: bool,
    watched: bool,
    terminal: Option<AdapterTerminal>,
    control_failure: Option<(doria_diagnostic_catalogue::IoMessageOperation, Failure)>,
    selected: Option<SelectedSource<RegistrationWatch>>,
}

impl Request {
    fn new(registration: usize, adapter: DrTransferAdapterV1) -> Self {
        Self {
            registration,
            adapter,
            data: ptr::null_mut(),
            length: 0,
            reading: false,
            watched: false,
            terminal: None,
            control_failure: None,
            selected: None,
        }
    }
}

struct Registered {
    key: u64,
    source: BorrowedProjection,
    adapter: DrTransferAdapterV1,
    watch: RegistrationWatch,
    binding: Option<SourceBinding>,
}

pub(crate) struct AdapterSet {
    watcher: Set,
    slots: RequestSlots<Request>,
    registrations: StableSlots<Registered>,
    sources: SourceRoutes<RegistrationWatch>,
    events: FixedSlice<DrReadyEventV1>,
    last_key: u64,
}

impl AdapterSet {
    /// # Safety
    /// Cancellation is null or carries a live claim during this call. This set
    /// retains cancellation, but never owns any subsequently supplied source.
    pub(crate) unsafe fn new(cancellation: *mut DrCancellationV1) -> Result<Self, Failure> {
        let watcher = Set::new(1, cancellation)?;
        let mut events = FixedSlice::new(2)?;
        for _ in 0..2 {
            events.push(DrReadyEventV1::default())?;
        }
        Ok(Self {
            watcher,
            slots: RequestSlots::dynamic()?,
            registrations: StableSlots::new()?,
            sources: SourceRoutes::new()?,
            events,
            last_key: 0,
        })
    }

    /// # Safety
    /// The complete source owner and adapter context remain live at stable
    /// addresses until removal or set destruction. Only group callback dispatch
    /// may access that retained adapter. No consumer may close the source or
    /// change its blocking mode while registered. The exclusively borrowed set
    /// has no live group; rejection never claims or closes the supplied source.
    pub(crate) unsafe fn add(
        &mut self,
        key: u64,
        source: BorrowedProjection,
        adapter: DrTransferAdapterV1,
    ) -> Result<usize, Failure> {
        if key <= self.last_key || key == CANCEL || !adapter.valid() {
            return Err(Failure::invalid_input());
        }
        self.watcher.handle()?;
        if (0..self.registrations.len()).any(|index| {
            self.registrations
                .raw(index)
                .is_some_and(|record| (*record).binding.is_none() && (*record).source == source)
        }) {
            return Err(Failure::invalid_input());
        }
        transfer::TransferSet::validate_handle(source)?;
        let required = self.len().checked_add(1).ok_or_else(exhausted)?;
        self.reserve_watches(required)?;
        let index = self.registrations.vacant_index();
        let watch = RegistrationWatch::new(source, index)?;
        self.registrations.insert_at(
            index,
            Registered {
                key,
                source,
                adapter,
                watch,
                binding: None,
            },
        )?;
        if let Err(failure) = self
            .slots
            .add_registration(index, key, |_| Ok(Request::new(index, adapter)))
        {
            self.registrations.remove(index);
            return Err(failure);
        }
        self.last_key = key;
        Ok(index)
    }

    /// # Safety
    /// Source is a live typed projection during admission. The resolver borrows
    /// the actual retained carrier home at each later source-dependent boundary.
    /// Route metadata pins resources, not an admission-time File wrapper.
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
        self.sources
            .reserve(self.len().checked_add(2).ok_or_else(exhausted)?)?;
        let mut binding = SourceBinding::new(provider);
        self.sources
            .rebind(&mut binding, resolved, |index, origin, handle, _| {
                require_file(origin)?;
                RegistrationWatch::source(handle, index)
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

    fn reserve_watches(&mut self, required: usize) -> Result<(), Failure> {
        if required <= self.watcher.capacity {
            return Ok(());
        }
        let maximum = (i32::MAX as usize - 1) / 2;
        if required > maximum {
            return Err(Failure::invalid_input());
        }
        let capacity = self
            .watcher
            .capacity
            .saturating_mul(2)
            .min(maximum)
            .max(required);
        let mut events = FixedSlice::new(capacity * 2)?;
        for _ in 0..capacity * 2 {
            events.push(DrReadyEventV1::default())?;
        }
        self.watcher.reserve(capacity)?;
        self.events = events;
        Ok(())
    }

    pub(crate) fn remove(&mut self, key: u64) -> Result<(), Failure> {
        let index = self.index_of(key).ok_or_else(Failure::invalid_input)?;
        for operation in 0..self.slots.len() {
            if self.slots.contains(operation) && self.slots.registration(operation) == index {
                let request = unsafe { &*self.slots.raw(operation) };
                if request.watched || !request.data.is_null() || request.selected.is_some() {
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

    pub(crate) fn len(&self) -> usize {
        self.registrations.occupied_len()
    }

    pub(crate) fn key(&self, index: usize) -> Option<u64> {
        self.registrations
            .raw(index)
            .map(|record| unsafe { (*record).key })
    }

    pub(crate) fn index_of(&self, key: u64) -> Option<usize> {
        (0..self.registrations.len()).find(|index| self.key(*index) == Some(key))
    }

    /// # Safety
    /// Frame is null or a live generated frame chain for this call only. The
    /// configured callback/source loans remain valid throughout the group.
    pub(crate) unsafe fn begin_selected<'s, 'b>(
        &'s mut self,
        frame: *const DrStackFrameV2,
        buffers: BufferList<'b, AdapterTerminal>,
        budget: &DrWaitBudgetV1,
        selection: FixedSlice<usize>,
    ) -> Result<AdapterGroup<'s, 'b>, SetupFailure<AdapterTerminal>> {
        let cancellation = self.watcher.cancellation;
        self.begin_selected_controlled(frame, buffers, budget, selection, cancellation)
    }

    /// # Safety
    /// Frame and loans obey begin_selected's contract. A nonnull cancellation
    /// state is live for this call; the returned group retains its own claim.
    pub(crate) unsafe fn begin_selected_controlled<'s, 'b>(
        &'s mut self,
        frame: *const DrStackFrameV2,
        buffers: BufferList<'b, AdapterTerminal>,
        budget: &DrWaitBudgetV1,
        selection: FixedSlice<usize>,
        cancellation: *mut DrCancellationV1,
    ) -> Result<AdapterGroup<'s, 'b>, SetupFailure<AdapterTerminal>> {
        self.watcher.handle()?;
        if selection.len() != buffers.len() {
            return Err(Failure::invalid_input().into());
        }
        for (index, registered) in selection.as_slice().iter().enumerate() {
            if self.key(*registered).is_none() {
                return Err(Failure::invalid_input().into());
            }
            let (reading, _) = buffers
                .request_info(index)
                .ok_or_else(Failure::invalid_input)?;
            (*self.registrations.raw(*registered).unwrap())
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
            let capacity = bound
                .checked_add(selection.len())
                .and_then(|count| count.checked_add(1))
                .ok_or_else(exhausted)?;
            self.sources.reserve(capacity)?;
            self.reserve_watches(capacity.checked_add(self.len()).ok_or_else(exhausted)?)?;
        } else {
            self.reserve_watches(selection.len())?;
        }
        let registrations = &self.registrations;
        let selection = self
            .slots
            .configure_operations(selection, |registered, _| {
                let adapter = (*registrations.raw(registered).unwrap()).adapter;
                Ok(Request::new(registered, adapter))
            })?;
        let cancellation = GroupCancellationWatch::install(&mut self.watcher, cancellation)?;
        let control = transfer::WaitControl::new(budget, cancellation.state());
        let driver = Dispatcher {
            watcher: &mut self.watcher,
            events: self.events.as_mut_slice(),
            registrations: &mut self.registrations,
            sources: &mut self.sources,
            frame,
            cancellation,
        };
        TransferGroup::begin_selected_prepared(&mut self.slots, buffers, driver, control, selection)
            .map(|inner| AdapterGroup { inner })
    }
}

fn require_file(origin: crate::native_handle::PhysicalOrigin) -> Result<(), Failure> {
    if matches!(origin, crate::native_handle::PhysicalOrigin::File { .. }) {
        Ok(())
    } else {
        // Synchronous callbacks have no canonical-device loan transport.
        // Preserve the existing boundary rather than erase standard provenance.
        Err(Failure::unsupported())
    }
}

struct Dispatcher<'s> {
    watcher: &'s mut Set,
    events: &'s mut [DrReadyEventV1],
    registrations: &'s mut StableSlots<Registered>,
    sources: &'s mut SourceRoutes<RegistrationWatch>,
    // The shared core clears this immediately after begin/with_call_frame.
    frame: *const DrStackFrameV2,
    cancellation: GroupCancellationWatch,
}

impl Drop for Dispatcher<'_> {
    fn drop(&mut self) {
        self.cancellation.detach(self.watcher);
    }
}

impl Dispatcher<'_> {
    unsafe fn watch(&self, request: &Request) -> *mut RegistrationWatch {
        if let Some(selected) = &request.selected {
            selected.platform()
        } else {
            ptr::addr_of_mut!((*self.registrations.raw(request.registration).unwrap()).watch)
        }
    }

    fn unwatch(&mut self, request: &mut Request) -> Result<(), Failure> {
        if !request.watched {
            return Ok(());
        }
        request.watched = false;
        let result = unsafe { (*self.watch(request)).unsubscribe(self.watcher, request.reading) };
        if result.is_err() {
            // Watches retain no caller bytes, but a failed removal would keep
            // generating irrelevant wakes. Invalidate only this watcher owner.
            drop(self.watcher.watcher.take());
        }
        result
    }

    fn retire(&mut self, request: &mut Request) -> Result<(), Failure> {
        request.data = ptr::null_mut();
        self.unwatch(request)
    }

    fn release(&mut self, request: &mut Request) -> Result<(), Failure> {
        let result = self.retire(request);
        request.selected = None;
        result
    }

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
            .rebind(binding, resolved, |index, origin, handle, _| {
                require_file(origin)?;
                RegistrationWatch::source(handle, index)
            })
            .map_err(AdapterTerminal::PlatformFailure)?;
        if selecting {
            let selected = self
                .sources
                .select(route)
                .map_err(AdapterTerminal::PlatformFailure)?;
            let changed = request.selected.as_ref().is_none_or(|old| {
                !old.origin().same_source(selected.origin()) || old.handle() != selected.handle()
            });
            if changed {
                self.unwatch(request)
                    .map_err(AdapterTerminal::PlatformFailure)?;
            }
            request.adapter.context = selected
                .context()
                .expect("native synchronous dispatch context");
            request.selected = Some(selected);
        }
        Ok(())
    }

    unsafe fn attempt(&mut self, request: &mut Request) {
        if request.terminal.is_some() || request.control_failure.is_some() {
            return;
        }
        let attempt = if request.reading {
            request
                .adapter
                .read(self.frame, request.data, request.length)
        } else {
            request
                .adapter
                .write(self.frame, request.data.cast_const(), request.length)
        };
        match attempt {
            AdapterAttempt::Terminal(terminal) => {
                request.terminal = Some(terminal);
                request.control_failure = self.retire(request).err().map(|failure| {
                    (
                        doria_diagnostic_catalogue::IoMessageOperation::Unregister,
                        failure,
                    )
                });
            }
            AdapterAttempt::Pending => {
                // A synchronous callback can replace its File before returning
                // WouldBlock. Select the current source before advisory wait.
                if let Err(terminal) = self.resolve_current(request, true) {
                    request.terminal = Some(terminal);
                    request.control_failure = self.retire(request).err().map(|failure| {
                        (
                            doria_diagnostic_catalogue::IoMessageOperation::Unregister,
                            failure,
                        )
                    });
                    return;
                }
                if request.watched {
                    return;
                }
                match (*self.watch(request)).subscribe(self.watcher, request.reading) {
                    Ok(()) => request.watched = true,
                    Err(failure) => {
                        request.control_failure = Some((
                            doria_diagnostic_catalogue::IoMessageOperation::Register,
                            failure,
                        ));
                        request.data = ptr::null_mut();
                    }
                }
            }
        }
    }
}

// Callback invocations borrow only the selected bytes for the call. The
// compiler/set retains the complete adapter; no pending kernel I/O exists.
// Cancel/drain retire watches synchronously without another adapter invocation.
unsafe impl GroupDriver<AdapterTerminal> for Dispatcher<'_> {
    type Request = Request;
    type Timeout = DrWaitBudgetV1;
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
        let registration = self.registrations.raw((*request).registration).unwrap();
        if (*registration).binding.is_some() {
            return Err(Failure::invalid_input());
        }
        crate::native_handle::transfer_resource((*registration).source)
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
            let _ = self.release(&mut *request);
            return Err(terminal);
        }
        Ok(())
    }

    unsafe fn prepare_query(
        &mut self,
        _: usize,
        request: *mut Request,
    ) -> Result<(), AdapterTerminal> {
        if (*request).terminal.is_some() || (*request).control_failure.is_some() {
            return Ok(());
        }
        if let Err(terminal) = self.resolve_current(&mut *request, true) {
            let _ = self.release(&mut *request);
            return Err(terminal);
        }
        Ok(())
    }

    unsafe fn release_request(&mut self, _: usize, request: *mut Request) {
        let _ = self.release(&mut *request);
    }

    unsafe fn reset_request(&mut self, _: usize, request: *mut Request) {
        let request = &mut *request;
        if request.watched || !request.data.is_null() || request.selected.is_some() {
            crate::runtime_invariant_panic();
        }
        request.length = 0;
        request.reading = false;
        request.terminal = None;
        request.control_failure = None;
    }

    unsafe fn submit(
        &mut self,
        _: usize,
        request: *mut Request,
        buffer: &mut Buffer<'_>,
    ) -> Submission {
        let request = &mut *request;
        let reading = matches!(buffer, Buffer::Read(_));
        if let Err(failure) = request.adapter.validate(reading) {
            let _ = self.release(request);
            return Submission::Failed(failure);
        }
        request.reading = reading;
        match buffer {
            Buffer::Read(bytes) => {
                request.data = bytes.as_mut_ptr();
                request.length = bytes.len();
            }
            Buffer::Write(bytes) => {
                request.data = bytes.as_ptr().cast_mut();
                request.length = bytes.len();
            }
        }
        self.attempt(request);
        if request.terminal.is_some() && request.control_failure.is_none() {
            Submission::Complete
        } else {
            Submission::Pending
        }
    }

    unsafe fn query(&mut self, _: usize, request: *mut Request) -> Query<AdapterTerminal> {
        let request = &mut *request;
        self.attempt(request);
        if let Some((operation, failure)) = request.control_failure {
            Query::PendingFailure(operation, failure)
        } else if let Some(terminal) = request.terminal.take() {
            request.selected = None;
            Query::Terminal(terminal)
        } else {
            Query::Pending
        }
    }

    fn wait_any(&mut self, budget: Self::Timeout, _: Option<usize>) -> Result<Wake, Failure> {
        match self.watcher.wait(&budget, self.events)? {
            (0, _) => Ok(Wake::Completion),
            (1, _) => Ok(Wake::Timeout),
            (2, _) => Ok(Wake::Cancellation),
            _ => Err(Failure::other()),
        }
    }

    unsafe fn cancel(
        &mut self,
        _: usize,
        request: *mut Request,
    ) -> Result<(), (doria_diagnostic_catalogue::IoMessageOperation, Failure)> {
        let request = &mut *request;
        if request.terminal.is_none() {
            request.terminal = Some(AdapterTerminal::NoTransfer);
        }
        self.retire(request).map_err(|failure| {
            (
                doria_diagnostic_catalogue::IoMessageOperation::Unregister,
                failure,
            )
        })
    }

    unsafe fn drain(&mut self, _: usize, request: *mut Request) -> Option<AdapterTerminal> {
        let _ = self.release(&mut *request);
        Some(
            (*request)
                .terminal
                .take()
                .unwrap_or(AdapterTerminal::NoTransfer),
        )
    }
}

pub(crate) struct AdapterGroup<'s, 'b> {
    inner: TransferGroup<'s, 'b, Dispatcher<'s>, transfer::WaitControl, AdapterTerminal>,
}

impl AdapterGroup<'_, '_> {
    /// # Safety
    /// Frame is null or live throughout cleanup and is never retained.
    pub(crate) unsafe fn drop_in_frame(self, frame: *const DrStackFrameV2) {
        self.inner.drop_in_frame(frame);
    }

    /// # Safety
    /// Frame is null or a live generated frame chain for this call.
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
    /// Frame is null or a live generated frame chain for this call.
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
}

#[cfg(test)]
#[path = "adapter/tests.rs"]
mod tests;

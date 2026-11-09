//! Unix transport for the shared borrowing transfer core. Legacy fixed sets own
//! raw native streams; dynamic sets project compiler-retained stream owners.
//! Native-capable streams use readiness. Inherited synchronous standard devices
//! use Decision 0137's lazy helper under this same group's retained-buffer loan.

use super::*;
use crate::byte_io::{self, ReadProgress, WriteProgress};
use crate::file_io::stream::OwnedFile;
use crate::transfer_group::adapter::AdapterTerminal;
use crate::transfer_group::pending::FileTerminal;
use crate::transfer_group::setup::GroupSetup;
use crate::transfer_group::source::{
    ResolvedSource, SelectedSource, SourceBinding, SourceProvider, SourceRoutes, SourceSnapshot,
};
use crate::transfer_group::{
    Buffer, BufferList, Control, DrCompletionStatusV1, FixedSlice, GroupDriver, Query,
    RequestSlots, SetupFailure, StableSlots, Stop, Submission, TerminalOutcome, TransferGroup,
    Wait, Wake,
};
use crate::DrStackFrameV2;

#[derive(Clone, Copy)]
enum Endpoint {
    Raw,
    HandleHome(*const c_void),
    File(*mut OwnedFile),
    /// A child pipe's parent end. It is nonblocking, so it takes the same
    /// attempt-then-watch path as any readiness stream, not the file path.
    Pipe(*mut OwnedFile),
    Standard(crate::device_io::StandardStream),
}

impl Endpoint {
    /// # Safety
    /// The origin is live under its current owner or the selected use.
    unsafe fn from_origin(origin: crate::native_handle::PhysicalOrigin) -> Result<Self, Failure> {
        Ok(match origin {
            crate::native_handle::PhysicalOrigin::File { file, .. } if origin.is_pipe()? => {
                Self::Pipe(file)
            }
            crate::native_handle::PhysicalOrigin::File { file, .. } => Self::File(file),
            crate::native_handle::PhysicalOrigin::Standard(stream) => Self::Standard(stream),
        })
    }
}

struct PhysicalRoute {
    endpoint: Endpoint,
    watch: RegistrationWatch,
    pending_supported: bool,
}

unsafe fn source_route(
    index: usize,
    origin: crate::native_handle::PhysicalOrigin,
    handle: i32,
    resource: crate::transfer_group::TransferResource,
) -> Result<PhysicalRoute, Failure> {
    Ok(PhysicalRoute {
        endpoint: Endpoint::from_origin(origin)?,
        watch: RegistrationWatch::source(handle, index)?,
        pending_supported: resource.shared_cursor || origin.supports_pending()?,
    })
}

pub(crate) struct Request {
    registration: usize,
    helper_key: u64,
    handle: i32,
    key: u64,
    // Explicit pending-capability projection; legacy/raw registrations do not
    // have one. This is borrowed metadata, not another native close owner.
    endpoint: Endpoint,
    data: *mut u8,
    length: usize,
    reading: bool,
    watched: bool,
    watching_helper: bool,
    terminal: Option<Result<usize, Failure>>,
    file_terminal: Option<FileTerminal>,
    control_failure: Option<(doria_diagnostic_catalogue::IoMessageOperation, Failure)>,
    helper: crate::device_io::helper::Helper,
    helper_signal: Option<crate::cancellation_signal::Signal>,
    helper_active: bool,
    selected: Option<SelectedSource<PhysicalRoute>>,
}

impl Request {
    fn new(handle: i32, key: u64) -> Self {
        Self {
            registration: 0,
            helper_key: 1,
            handle,
            key,
            endpoint: Endpoint::Raw,
            data: ptr::null_mut(),
            length: 0,
            reading: false,
            watched: false,
            watching_helper: false,
            terminal: None,
            file_terminal: None,
            control_failure: None,
            helper: crate::device_io::helper::Helper::new(),
            helper_signal: None,
            helper_active: false,
            selected: None,
        }
    }

    fn configured(
        registration: usize,
        operation: usize,
        handle: i32,
        key: u64,
        endpoint: Endpoint,
    ) -> Result<Self, Failure> {
        let mut request = Self::new(handle, key);
        request.registration = registration;
        request.helper_key = watch_key(operation, true)?;
        request.endpoint = endpoint;
        Ok(request)
    }
}

struct Registered {
    key: u64,
    handle: i32,
    endpoint: Endpoint,
    watch: RegistrationWatch,
    source: Option<SourceBinding>,
    pending_supported: bool,
}

pub(crate) struct TransferSet {
    watcher: Set,
    slots: RequestSlots<Request>,
    registrations: StableSlots<Registered>,
    // Requests (and their selected pins) drop before the route homes they use.
    sources: SourceRoutes<PhysicalRoute>,
    events: FixedSlice<DrReadyEventV1>,
    owners: FixedSlice<OwnedHandle>,
    borrowed: bool,
    last_key: u64,
}

impl TransferSet {
    unsafe fn new(
        keys: &[u64],
        owners: FixedSlice<OwnedHandle>,
        cancellation: *mut DrCancellationV1,
    ) -> Result<Self, Failure> {
        if keys.len() != owners.len() || keys.is_empty() {
            return Err(Failure::invalid_input());
        }
        for (index, owner) in owners.as_slice().iter().enumerate() {
            if keys[..index].contains(&keys[index]) {
                return Err(Failure::invalid_input());
            }
            Self::validate_handle(owner.get())?;
        }
        let watcher = Set::new(keys.len(), cancellation)?;
        let slots = RequestSlots::new_fallible(keys, |index| {
            Request::configured(
                index,
                index,
                owners.as_slice()[index].get(),
                keys[index],
                Endpoint::Raw,
            )
        })?;
        let mut registrations = StableSlots::new()?;
        for (index, key) in keys.iter().copied().enumerate() {
            let handle = owners.as_slice()[index].get();
            registrations.insert_at(
                index,
                Registered {
                    key,
                    handle,
                    endpoint: Endpoint::Raw,
                    watch: RegistrationWatch::new(handle, index)?,
                    source: None,
                    pending_supported: true,
                },
            )?;
        }
        let capacity = keys
            .len()
            .checked_mul(2)
            .ok_or_else(Failure::invalid_input)?;
        let mut events = FixedSlice::new(capacity)?;
        for _ in 0..capacity {
            events.push(DrReadyEventV1::default())?;
        }
        Ok(Self {
            owners,
            slots,
            registrations,
            sources: SourceRoutes::new()?,
            watcher,
            events,
            borrowed: false,
            last_key: 0,
        })
    }

    /// # Safety
    /// Cancellation is null or a live claim for this call. The watcher retains
    /// its own cancellation claim; this mode never owns a projected stream.
    pub(crate) unsafe fn borrowed_new(
        cancellation: *mut DrCancellationV1,
    ) -> Result<Self, Failure> {
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
            owners: FixedSlice::new(0)?,
            borrowed: true,
            last_key: 0,
        })
    }

    pub(crate) fn is_borrowed(&self) -> bool {
        self.borrowed
    }

    pub(super) fn validate_handle(handle: BorrowedProjection) -> Result<(), Failure> {
        let flags = unsafe { libc::fcntl(handle, libc::F_GETFL) };
        if flags < 0 {
            return Err(platform_io::last_failure());
        }
        if flags & libc::O_NONBLOCK == 0 && !platform::is_regular_file(handle)? {
            return Err(Failure {
                reason: Reason::Unsupported,
                system_code: None,
                transferred_byte_count: None,
            });
        }
        Ok(())
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
        // Allocate the caller-facing scratch first. If watcher growth fails,
        // both old allocations stay usable with their original capacity.
        self.watcher.reserve(capacity)?;
        self.events = events;
        Ok(())
    }

    /// # Safety
    /// Invalid/unsupported projections are permitted for checked rejection.
    /// An accepted projection names one idle, live stream owned elsewhere. Its owner must
    /// retain it until successful removal or set destruction, and may not close,
    /// transfer or change its blocking mode while registered. The shared owner
    /// gate excludes active groups. Successful keys strictly increase; zero and
    /// the cancellation sentinel are excluded even for direct private callers.
    /// Failure neither claims nor closes the projection. Reusable configuration
    /// capacity may grow before final request allocation fails; membership does
    /// not change on that path.
    pub(crate) unsafe fn add_borrowed(
        &mut self,
        key: u64,
        projection: BorrowedProjection,
    ) -> Result<usize, Failure> {
        if !self.borrowed || key <= self.last_key || key == CANCEL {
            return Err(Failure::invalid_input());
        }
        self.add_validated(key, projection, Endpoint::Raw)
    }

    // Admission owns keys, duplicate projections, reusable watch capacity and
    // stable requests. Typed file admission supplies an already-classified
    // regular handle; it must not reopen or repeat that classification.
    unsafe fn add_validated(
        &mut self,
        key: u64,
        projection: BorrowedProjection,
        endpoint: Endpoint,
    ) -> Result<usize, Failure> {
        self.watcher.handle()?;
        if (0..self.registrations.len()).any(|index| {
            self.registrations
                .raw(index)
                .is_some_and(|record| (*record).source.is_none() && (*record).handle == projection)
        }) {
            return Err(Failure::invalid_input());
        }
        if !matches!(
            endpoint,
            Endpoint::File(_) | Endpoint::Pipe(_) | Endpoint::Standard(_)
        ) {
            Self::validate_handle(projection)?;
        }
        let required = self
            .registrations
            .occupied_len()
            .checked_add(1)
            .ok_or_else(exhausted)?;
        self.reserve_watches(required)?;
        let index = self.registrations.vacant_index();
        self.registrations.insert_at(
            index,
            Registered {
                key,
                handle: projection,
                endpoint,
                watch: RegistrationWatch::new(projection, index)?,
                source: None,
                pending_supported: true,
            },
        )?;
        if let Err(failure) = self.slots.add_registration(index, key, |operation| {
            Request::configured(index, operation, projection, key, endpoint)
        }) {
            self.registrations.remove(index);
            return Err(failure);
        }
        self.last_key = key;
        Ok(index)
    }

    /// # Safety
    /// Endpoint is null for checked rejection, or the stable aligned home of
    /// the actual i32 handle in its retained sole leaf owner. It is not scalar
    /// wait metadata, an owner-object cast, or a temporary copied handle. The
    /// owner remains live/unmoved and its handle unchanged while registered.
    pub(super) unsafe fn add_pending(
        &mut self,
        key: u64,
        endpoint: *const c_void,
    ) -> Result<usize, Failure> {
        if !self.borrowed
            || key <= self.last_key
            || key == CANCEL
            || endpoint.is_null()
            || !(endpoint as usize).is_multiple_of(mem::align_of::<i32>())
        {
            return Err(Failure::invalid_input());
        }
        let handle = endpoint.cast::<i32>().read();
        self.add_validated(key, handle, Endpoint::HandleHome(endpoint))
    }

    /// # Safety
    /// File is null for checked rejection or one live, stable, exclusively
    /// retained OwnedFile leaf. Admission borrows it and never acquires a close
    /// claim. Its native handle remains unchanged until removal/set destruction.
    /// This private mode currently accepts only regular files, not devices or
    /// pipes accepted by the ordinary File open/transfer surface.
    pub(super) unsafe fn add_file(
        &mut self,
        key: u64,
        file: *mut OwnedFile,
    ) -> Result<usize, Failure> {
        if !self.borrowed
            || key <= self.last_key
            || key == CANCEL
            || file.is_null()
            || !(file as usize).is_multiple_of(mem::align_of::<OwnedFile>())
        {
            return Err(Failure::invalid_input());
        }
        let handle = (*file).pending_regular_handle()?;
        self.add_validated(key, handle, Endpoint::File(file))
    }

    pub(super) unsafe fn add_standard(
        &mut self,
        key: u64,
        stream: crate::device_io::StandardStream,
    ) -> Result<usize, Failure> {
        if !self.borrowed || key <= self.last_key || key == CANCEL {
            return Err(Failure::invalid_input());
        }
        let handle = crate::device_io::byte_handle(stream)?;
        self.add_validated(key, handle, Endpoint::Standard(stream))
    }

    /// Callback-independent membership borrows typed physical provenance. A
    /// source may be watchable without supporting this driver's transfer kind.
    /// # Safety
    /// Origin belongs to a live retained complete owner at a stable address.
    pub(super) unsafe fn add_readiness_origin(
        &mut self,
        key: u64,
        origin: crate::native_handle::PhysicalOrigin,
    ) -> Result<usize, Failure> {
        if !self.borrowed || key <= self.last_key || key == CANCEL {
            return Err(Failure::invalid_input());
        }
        let handle = origin.handle()?;
        let endpoint = Endpoint::from_origin(origin)?;
        // Cache endpoint support once at configuration. It is not an admission
        // requirement and must not add a syscall to every group reuse.
        let pending_supported = origin.supports_pending()?;
        let index = self.add_validated(key, handle, endpoint)?;
        (*self.registrations.raw(index).unwrap()).pending_supported = pending_supported;
        Ok(index)
    }

    /// Owned native/hosted membership binds a logical member to a resolver,
    /// never to an admission-time file pointer. Admission supplies the already
    /// evaluated source; later boundaries resolve through this exact provider.
    pub(super) unsafe fn add_bound_source(
        &mut self,
        key: u64,
        source: *const c_void,
        provider: SourceProvider,
    ) -> Result<usize, Failure> {
        let resolved = ResolvedSource::borrow(source, None)?;
        let origin = resolved.origin();
        self.sources
            .reserve(self.len().checked_add(2).ok_or_else(exhausted)?)?;
        let mut binding = SourceBinding::new(provider);
        self.sources
            .rebind(&mut binding, resolved, |index, origin, handle, resource| {
                source_route(index, origin, handle, resource)
            })?;
        match self.add_readiness_origin(key, origin) {
            Ok(index) => {
                (*self.registrations.raw(index).unwrap()).source = Some(binding);
                Ok(index)
            }
            Err(failure) => {
                self.sources.unbind(&mut binding);
                Err(failure)
            }
        }
    }

    /// A hosted controller supplies a fresh ordinary getter result. This stages
    /// metadata only; operation boundaries still require their own fresh input.
    /// Native bindings reject external substitution before invoking a getter.
    pub(super) unsafe fn update_external_source(
        &mut self,
        index: usize,
        source: *const c_void,
    ) -> Result<crate::native_handle::PhysicalOrigin, Failure> {
        let record = self
            .registrations
            .raw(index)
            .ok_or_else(Failure::invalid_input)?;
        let binding = (*record)
            .source
            .as_mut()
            .ok_or_else(Failure::invalid_input)?;
        let resolved = match binding.resolve(ptr::null(), Some(source)) {
            Ok(source) => source,
            Err(AdapterTerminal::PlatformFailure(failure)) => return Err(failure),
            Err(_) => crate::runtime_invariant_panic(),
        };
        let origin = resolved.origin();
        self.sources
            .rebind(binding, resolved, |index, origin, handle, resource| {
                source_route(index, origin, handle, resource)
            })?;
        Ok(origin)
    }

    /// Transfer-kind restrictions are checked only for selected operations,
    /// before their first preparation; source-only membership remains valid.
    pub(super) fn validate_pending_registration(&self, index: usize) -> Result<(), Failure> {
        let record = self
            .registrations
            .raw(index)
            .ok_or_else(Failure::invalid_input)?;
        let record = unsafe { &*record };
        if record.source.is_some() {
            // The retained member may already have replaced its old file.
            // Validate the fresh source under the group guard, not this cache.
            return Ok(());
        }
        if !record.pending_supported {
            return Err(Failure::unsupported());
        }
        if let Endpoint::File(file) | Endpoint::Pipe(file) = record.endpoint {
            if unsafe { (*file).native_handle() } != record.handle {
                return Err(Failure::invalid_input());
            }
        }
        Ok(())
    }

    /// The exclusively borrowed set has no live group. Removal destroys only
    /// the selected request metadata, never its caller-owned stream projection.
    pub(crate) fn remove_borrowed(&mut self, key: u64) -> Result<(), Failure> {
        self.remove_borrowed_with(key, |_| {})
    }

    pub(super) fn remove_borrowed_with(
        &mut self,
        key: u64,
        mut retire: impl FnMut(usize),
    ) -> Result<(), Failure> {
        if !self.borrowed {
            return Err(Failure::invalid_input());
        }
        let index = self.index_of(key).ok_or_else(Failure::invalid_input)?;
        for operation in 0..self.slots.len() {
            if self.slots.contains(operation) && self.slots.registration(operation) == index {
                let request = unsafe { &*self.slots.raw(operation) };
                if request.watched || !request.data.is_null() || request.selected.is_some() {
                    return Err(Failure::invalid_input());
                }
            }
        }
        for operation in 0..self.slots.len() {
            if self.slots.contains(operation) && self.slots.registration(operation) == index {
                retire(operation);
            }
        }
        self.slots.remove_registration(index)?;
        unsafe {
            if let Some(binding) = (*self.registrations.raw(index).unwrap()).source.as_mut() {
                self.sources.unbind(binding);
            }
        }
        self.registrations
            .remove(index)
            .ok_or_else(Failure::invalid_input)?;
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

    pub(super) fn cached_operation(&self, registration: usize, ordinal: usize) -> Option<usize> {
        self.slots.operation_of(registration, ordinal)
    }

    pub(super) fn operation_registration(&self, operation: usize) -> usize {
        self.slots.registration(operation)
    }

    pub(super) fn configure_operations(
        &mut self,
        selection: FixedSlice<usize>,
    ) -> Result<FixedSlice<usize>, Failure> {
        if selection
            .as_slice()
            .iter()
            .any(|registered| self.key(*registered).is_none())
        {
            return Err(Failure::invalid_input());
        }
        let bound = (0..self.registrations.len())
            .filter(|index| {
                self.registrations
                    .raw(*index)
                    .is_some_and(|record| unsafe { (*record).source.is_some() })
            })
            .count();
        if bound != 0 {
            let source_capacity = bound
                .checked_add(selection.len())
                .and_then(|count| count.checked_add(1))
                .ok_or_else(exhausted)?;
            self.sources.reserve(source_capacity)?;
            self.reserve_watches(
                source_capacity
                    .checked_add(selection.len())
                    .and_then(|count| count.checked_add(self.len()))
                    .ok_or_else(exhausted)?,
            )?;
        } else {
            self.reserve_watches(selection.len().max(self.len()))?;
        }
        let registrations = &self.registrations;
        self.slots
            .configure_operations(selection, |registered, operation| {
                let record = unsafe { &*registrations.raw(registered).unwrap() };
                Request::configured(
                    registered,
                    operation,
                    record.handle,
                    record.key,
                    record.endpoint,
                )
            })
    }

    #[cfg(test)]
    pub(super) fn pending_snapshot(&mut self, index: usize) -> Option<(usize, i32, usize, bool)> {
        let index = self.cached_operation(index, 0)?;
        let watcher = self.watcher.handle().ok()?;
        let watches = self
            .watcher
            .slots()
            .iter()
            .filter(|entry| entry.interests != 0)
            .count();
        let request = unsafe { &*self.slots.raw(index) };
        Some((
            self.slots.raw(index) as usize,
            watcher,
            watches,
            request.data.is_null() && !request.watched,
        ))
    }

    pub(crate) fn project_completion(
        status: &mut DrCompletionStatusV1,
        outcome: crate::transfer_group::Awaited,
        reading: bool,
        length: usize,
    ) -> u8 {
        let eof = reading && length != 0 && outcome.transfer == Some(Ok(0));
        let tag = status.project(outcome, reading);
        if eof {
            2
        } else {
            tag
        }
    }

    pub(crate) fn begin<'s, 'b>(
        &'s mut self,
        buffers: BufferList<'b>,
        budget: &DrWaitBudgetV1,
    ) -> Result<NativeGroup<'s, 'b>, Failure> {
        if self.borrowed {
            return Err(Failure::invalid_input());
        }
        self.begin_with_selection(buffers, budget, None)
    }

    /// Selection names logical registrations in plan order. Repeated members
    /// configure distinct cached operation slots under the same registration.
    /// The shared core validates it before any request is submitted and keeps
    /// caller buffers indexed by plan position, not registered-slot position.
    pub(crate) fn begin_selected<'s, 'b>(
        &'s mut self,
        buffers: BufferList<'b>,
        budget: &DrWaitBudgetV1,
        selection: FixedSlice<usize>,
    ) -> Result<NativeGroup<'s, 'b>, Failure> {
        self.begin_with_selection(buffers, budget, Some(selection))
    }

    fn begin_with_selection<'s, 'b>(
        &'s mut self,
        buffers: BufferList<'b>,
        budget: &DrWaitBudgetV1,
        selection: Option<FixedSlice<usize>>,
    ) -> Result<NativeGroup<'s, 'b>, Failure> {
        self.begin_with_driver(buffers, budget, selection, core::convert::identity)
    }

    pub(crate) fn begin_with_driver<'s, 'b, D, T>(
        &'s mut self,
        buffers: BufferList<'b, T>,
        budget: &DrWaitBudgetV1,
        selection: Option<FixedSlice<usize>>,
        wrap: impl FnOnce(Dispatcher<'s>) -> D,
    ) -> Result<TransferGroup<'s, 'b, D, WaitControl, T>, Failure>
    where
        D: GroupDriver<T, Request = Request, Timeout = DrWaitBudgetV1>,
        T: TerminalOutcome,
    {
        if selection.as_ref().map_or(self.len(), FixedSlice::len) != buffers.len() {
            return Err(Failure::invalid_input());
        }
        let registrations = match selection {
            Some(selection) => selection,
            None => self.slots.primary_registrations()?,
        };
        let selection = self.configure_operations(registrations)?;
        let (slots, native, control) = self.driver_parts(budget)?;
        let driver = wrap(native);
        TransferGroup::begin_selected(slots, buffers, driver, control, selection)
    }

    /// # Safety
    /// A nonnull cancellation state is live for acquisition during this call.
    pub(super) unsafe fn configure_prepared_with_driver_controlled<'s, 'b, D, T>(
        &'s mut self,
        buffers: BufferList<'b, T>,
        budget: &DrWaitBudgetV1,
        selection: FixedSlice<usize>,
        cancellation: *mut DrCancellationV1,
        wrap: impl FnOnce(Dispatcher<'s>) -> D,
    ) -> Result<GroupSetup<'s, 'b, D, WaitControl, T>, SetupFailure<T>>
    where
        D: GroupDriver<T, Request = Request, Timeout = DrWaitBudgetV1>,
        T: TerminalOutcome,
    {
        let (slots, native, control) = self.driver_parts_controlled(budget, cancellation)?;
        let driver = wrap(native);
        GroupSetup::configure(slots, buffers, driver, control, Some(selection))
    }

    fn driver_parts<'s>(
        &'s mut self,
        budget: &DrWaitBudgetV1,
    ) -> Result<(&'s mut RequestSlots<Request>, Dispatcher<'s>, WaitControl), Failure> {
        let cancellation = self.watcher.cancellation;
        unsafe { self.driver_parts_controlled(budget, cancellation) }
    }

    pub(super) fn cancellation_policy(&self) -> *mut DrCancellationV1 {
        self.watcher.cancellation
    }

    unsafe fn driver_parts_controlled<'s>(
        &'s mut self,
        budget: &DrWaitBudgetV1,
        cancellation: *mut DrCancellationV1,
    ) -> Result<(&'s mut RequestSlots<Request>, Dispatcher<'s>, WaitControl), Failure> {
        self.watcher.handle()?;
        let cancellation = GroupCancellationWatch::install(&mut self.watcher, cancellation)?;
        let control = WaitControl::new(budget, cancellation.state());
        let driver = Dispatcher {
            _owners: self.owners.as_slice(),
            watcher: &mut self.watcher,
            events: self.events.as_mut_slice(),
            registrations: &mut self.registrations,
            sources: &mut self.sources,
            devices: [None, None, None],
            cancellation,
        };
        Ok((&mut self.slots, driver, control))
    }
}

pub(crate) struct WaitControl {
    budget: DrWaitBudgetV1,
    // The group driver retains its claim through every control observation.
    cancellation: *const DrCancellationV1,
}

impl WaitControl {
    pub(super) fn new(budget: &DrWaitBudgetV1, cancellation: *const DrCancellationV1) -> Self {
        Self {
            budget: *budget,
            cancellation,
        }
    }
}

impl Control<DrWaitBudgetV1> for WaitControl {
    fn next_wait(&mut self) -> Result<Result<Wait<DrWaitBudgetV1>, Stop>, Failure> {
        if !self.cancellation.is_null()
            && unsafe { cancellation::dr_v1_cancellation_is_requested(self.cancellation) } != 0
        {
            return Ok(Err(Stop::Cancellation));
        }
        match self.budget.remaining(
            platform::UNIT,
            platform::MAXIMUM,
            crate::monotonic_clock::now,
        )? {
            (3, _) => Ok(Err(Stop::BudgetExhausted)),
            (kind, _) => Ok(Ok(Wait {
                timeout: self.budget,
                immediate: kind == 0,
            })),
        }
    }
    fn cancellation_signal(&self) -> Option<usize> {
        None
    }
}

pub(crate) struct Dispatcher<'s> {
    _owners: &'s [OwnedHandle],
    watcher: &'s mut Set,
    events: &'s mut [DrReadyEventV1],
    registrations: &'s mut StableSlots<Registered>,
    sources: &'s mut SourceRoutes<PhysicalRoute>,
    // Canonical device claims belong to the group, not a mutable registration.
    // Rebinding several members to one standard device reuses one claim.
    devices: [Option<crate::device_io::DeviceLoan<'static>>; 3],
    cancellation: GroupCancellationWatch,
}

impl Drop for Dispatcher<'_> {
    fn drop(&mut self) {
        self.cancellation.detach(self.watcher);
    }
}

impl Dispatcher<'_> {
    fn device_loan(&self, request: &Request) -> Option<&crate::device_io::DeviceLoan<'static>> {
        let Endpoint::Standard(stream) = request.endpoint else {
            return None;
        };
        self.devices[stream as usize].as_ref()
    }

    fn acquire_device(&mut self, endpoint: Endpoint, handle: i32) -> Result<(), Failure> {
        if let Endpoint::Standard(stream) = endpoint {
            if crate::device_io::byte_handle(stream)? != handle {
                return Err(Failure::invalid_input());
            }
            if self.devices[stream as usize].is_none() {
                self.devices[stream as usize] =
                    Some(crate::device_io::standard_device(stream).try_borrow_for_io()?);
            }
        }
        Ok(())
    }

    unsafe fn watch(&self, request: &Request) -> *mut RegistrationWatch {
        if let Some(selected) = &request.selected {
            ptr::addr_of_mut!((*selected.platform()).watch)
        } else {
            let registration = self
                .registrations
                .raw(request.registration)
                .unwrap_or_else(|| crate::runtime_invariant_panic());
            ptr::addr_of_mut!((*registration).watch)
        }
    }

    unsafe fn resolve_source(
        &mut self,
        request: *mut Request,
        frame: *const DrStackFrameV2,
        external: Option<*const c_void>,
        selecting: bool,
    ) -> Result<Option<SourceSnapshot>, AdapterTerminal> {
        let registration = self
            .registrations
            .raw((*request).registration)
            .ok_or_else(|| AdapterTerminal::PlatformFailure(Failure::invalid_input()))?;
        let Some(binding) = (*registration).source.as_mut() else {
            if external.is_some() {
                return Err(AdapterTerminal::PlatformFailure(Failure::invalid_input()));
            }
            return Ok(None);
        };
        let resolved = binding.resolve(frame, external)?;
        let route = self
            .sources
            .rebind(binding, resolved, |index, origin, handle, resource| {
                source_route(index, origin, handle, resource)
            })
            .map_err(AdapterTerminal::PlatformFailure)?;
        let mut snapshot = route.snapshot();
        if !snapshot
            .origin
            .supports_pending()
            .map_err(AdapterTerminal::PlatformFailure)?
        {
            return Err(AdapterTerminal::PlatformFailure(Failure::unsupported()));
        }
        if selecting {
            if (*request).watched || !(*request).data.is_null() || (*request).helper_active {
                crate::runtime_invariant_panic();
            }
            let selected = self
                .sources
                .select(route)
                .map_err(AdapterTerminal::PlatformFailure)?;
            snapshot = selected.snapshot();
            let platform = &*selected.platform();
            if !platform.pending_supported {
                return Err(AdapterTerminal::PlatformFailure(Failure::unsupported()));
            }
            self.acquire_device(platform.endpoint, selected.handle())
                .map_err(AdapterTerminal::PlatformFailure)?;
            (*request).handle = selected.handle();
            (*request).endpoint = platform.endpoint;
            // Replacing an unsubmitted selection stages the new pin first.
            (*request).selected = Some(selected);
        }
        Ok(Some(snapshot))
    }

    unsafe fn observe_helper(&mut self, request: &mut Request, terminal: FileTerminal) -> Query {
        let retry = matches!(
            terminal,
            FileTerminal::Read(Ok(ReadProgress::WouldBlock(_)))
                | FileTerminal::Write(Ok(WriteProgress::WouldBlock(_)))
        ) || matches!(terminal, FileTerminal::Read(Err(failure)) | FileTerminal::Write(Err(failure))
            if failure.reason == Reason::Interrupted && failure.transferred_byte_count == Some(0));
        if !retry {
            return self.finish_helper(request, terminal);
        }
        // A nonblocking inherited endpoint can become blocking through a
        // foreign open-description alias. Retry only on the helper, after the
        // group's next preflight; native readiness remains advisory.
        request.helper_active = false;
        if let Err(failure) = self.remove_watch(request) {
            let operation = doria_diagnostic_catalogue::IoMessageOperation::Unregister;
            request.control_failure = Some((operation, failure));
            return Query::PendingFailure(operation, failure);
        }
        self.watch_device(request)
    }

    unsafe fn finish_helper(&mut self, request: &mut Request, terminal: FileTerminal) -> Query {
        // query/drain supplied the actual retired host result. Cancellation
        // alone never clears this obligation or releases the selected source.
        request.helper_active = false;
        let terminal = match terminal {
            FileTerminal::Write(result) => {
                FileTerminal::Write(self.device_loan(request).unwrap().finish_write(result))
            }
            terminal => terminal,
        };
        request.file_terminal = Some(terminal);
        request.terminal = Some(terminal.result());
        match self.retire(request) {
            Ok(()) => Query::Terminal(terminal.result()),
            Err(failure) => {
                let operation = doria_diagnostic_catalogue::IoMessageOperation::Unregister;
                request.control_failure = Some((operation, failure));
                Query::PendingFailure(operation, failure)
            }
        }
    }

    unsafe fn submit_helper(&mut self, request: &mut Request) -> Query {
        if let Err(failure) = self.remove_watch(request) {
            let operation = doria_diagnostic_catalogue::IoMessageOperation::Unregister;
            request.control_failure = Some((operation, failure));
            return Query::PendingFailure(operation, failure);
        }
        let setup = (|| {
            if request.helper_signal.is_none() {
                request.helper_signal = Some(crate::cancellation_signal::Signal::new()?);
            }
            let signal = request.helper_signal.as_ref().unwrap().get();
            crate::cancellation_signal::reset(signal)?;
            self.watcher.add(
                crate::cancellation_signal::native_handle(signal),
                request.helper_key,
                READ,
            )?;
            request.watched = true;
            request.watching_helper = true;
            request.helper.submit(crate::device_io::helper::Job {
                handle: request.handle,
                data: request.data,
                length: request.length,
                reading: request.reading,
                notify: helper_notice,
                notice: signal,
            })?;
            request.helper_active = true;
            Ok::<_, Failure>(())
        })();
        match setup {
            Ok(()) => Query::Pending,
            Err(failure) => {
                // Notice/watch/thread setup did not transfer bytes. Retire only
                // its membership and report readiness infrastructure ownership;
                // adapter staging is completed as NoTransfer during group drain.
                let _ = self.retire(request);
                let operation = doria_diagnostic_catalogue::IoMessageOperation::Register;
                request.control_failure = Some((operation, failure));
                Query::PendingFailure(operation, failure)
            }
        }
    }

    fn retire(&mut self, request: &mut Request) -> Result<(), Failure> {
        request.data = ptr::null_mut();
        self.remove_watch(request)
    }

    fn remove_watch(&mut self, request: &mut Request) -> Result<(), Failure> {
        if !request.watched {
            return Ok(());
        }
        request.watched = false;
        let result = if request.watching_helper {
            self.watcher.remove(request.helper_key)
        } else {
            unsafe { (*self.watch(request)).unsubscribe(self.watcher, request.reading) }
        };
        request.watching_helper = false;
        if result.is_err() {
            // A remaining watch retains no buffer address, but it would cause
            // repeated irrelevant wakes. Invalidate only this failed watcher.
            drop(self.watcher.watcher.take());
        }
        result
    }

    fn watch_device(&mut self, request: &mut Request) -> Query {
        if !request.watched {
            match unsafe { (*self.watch(request)).subscribe(self.watcher, request.reading) } {
                Ok(()) => {
                    request.watched = true;
                    request.watching_helper = false;
                }
                Err(failure) => {
                    request.data = ptr::null_mut();
                    let operation = doria_diagnostic_catalogue::IoMessageOperation::Register;
                    request.control_failure = Some((operation, failure));
                    return Query::PendingFailure(operation, failure);
                }
            }
        }
        Query::Pending
    }

    unsafe fn attempt(&mut self, request: &mut Request) -> Query {
        if let Some((operation, failure)) = request.control_failure {
            return Query::PendingFailure(operation, failure);
        }
        if let Some(result) = request.terminal {
            return Query::Terminal(result);
        }
        if request.helper_active {
            return match request.helper.query() {
                Some(terminal) => self.observe_helper(request, terminal),
                None => Query::Pending,
            };
        }
        // File submit has already performed and cached the sole synchronous
        // owned transfer. Query may never retry it or manufacture progress.
        if matches!(request.endpoint, Endpoint::File(_)) {
            crate::runtime_invariant_panic();
        }
        if let Some(device) = self.device_loan(request) {
            // No inherited open-description mode is exclusively owned here.
            // An F_GETFL snapshot cannot authorize potentially blocking I/O on
            // the driving thread. Queued/empty work alone stays inline.
            if request.length != 0 && (!request.reading || !device.buffered_input_ready()) {
                return self.submit_helper(request);
            }
        }
        // Devices and pipes report EOF and no-progress as typed outcomes, so
        // the retired attempt keeps its exact result for completion.
        let typed =
            self.device_loan(request).is_some() || matches!(request.endpoint, Endpoint::Pipe(_));
        let result = if request.reading {
            let result = if let Some(device) = self.device_loan(request) {
                device.read_attempt(request.handle, request.data, request.length)
            } else {
                byte_io::read_attempt(request.handle, request.data, request.length)
            };
            if typed {
                request.file_terminal = Some(FileTerminal::Read(result));
            }
            match result {
                Ok(ReadProgress::Data(n)) => Some(Ok(n)),
                Ok(ReadProgress::EndOfStream) => Some(Ok(0)),
                Ok(ReadProgress::WouldBlock(_)) => None,
                Err(failure)
                    if failure.reason == Reason::Interrupted
                        && failure.transferred_byte_count == Some(0) =>
                {
                    None
                }
                Err(failure) => Some(Err(failure)),
            }
        } else {
            let result = if let Some(device) = self.device_loan(request) {
                device.write_attempt(request.handle, request.data, request.length)
            } else {
                byte_io::write_attempt(request.handle, request.data, request.length)
            };
            if typed {
                request.file_terminal = Some(FileTerminal::Write(result));
            }
            match result {
                Ok(WriteProgress::Accepted(n)) => Some(Ok(n)),
                Ok(WriteProgress::WouldBlock(_)) => None,
                Err(failure)
                    if failure.reason == Reason::Interrupted
                        && failure.transferred_byte_count == Some(0) =>
                {
                    None
                }
                Err(failure) => Some(Err(failure)),
            }
        };
        if let Some(result) = result {
            request.terminal = Some(result);
            return match self.retire(request) {
                Ok(()) => Query::Terminal(result),
                Err(failure) => {
                    let operation = doria_diagnostic_catalogue::IoMessageOperation::Unregister;
                    request.control_failure = Some((operation, failure));
                    Query::PendingFailure(operation, failure)
                }
            };
        }
        self.watch_device(request)
    }
}

unsafe fn helper_notice(signal: usize) {
    if crate::cancellation_signal::set(signal).is_err() {
        crate::runtime_invariant_panic();
    }
}

// Native readiness retries retain the existing path. Helper submission retains
// the original region until the actual host call returns, even during teardown.
unsafe impl GroupDriver for Dispatcher<'_> {
    type Request = Request;
    type Timeout = DrWaitBudgetV1;
    const MAXIMUM_TRANSFER_BYTES: usize = isize::MAX as usize;

    unsafe fn resource(
        &self,
        _: usize,
        request: *mut Request,
    ) -> Result<crate::transfer_group::TransferResource, Failure> {
        if let Some(selected) = &(*request).selected {
            return Ok(selected.resource());
        }
        crate::native_handle::transfer_resource((*request).handle)
    }
    // Readiness query retries the transfer, rather than only observing a
    // previously retired completion. The shared core must preflight each retry
    // and bound an immediate poll even for permanently ready regular files.
    const QUERY_CAN_TRANSFER: bool = true;

    unsafe fn acquire_request(&mut self, _: usize, request: *mut Request) -> Result<(), Failure> {
        if (*self
            .registrations
            .raw((*request).registration)
            .ok_or_else(Failure::invalid_input)?)
        .source
        .is_some()
        {
            return Ok(());
        }
        self.acquire_device((*request).endpoint, (*request).handle)
    }

    unsafe fn release_request(&mut self, _: usize, request: *mut Request) {
        (*request).helper.shutdown();
        drop((*request).helper_signal.take());
        (*request).helper_active = false;
        drop((*request).selected.take());
    }

    unsafe fn reset_request(&mut self, _: usize, request: *mut Request) {
        if (*request).watched || (*request).selected.is_some() {
            crate::runtime_invariant_panic();
        }
        let endpoint = (*request).endpoint;
        let registration = (*request).registration;
        let helper_key = (*request).helper_key;
        let helper = mem::replace(
            &mut (*request).helper,
            crate::device_io::helper::Helper::new(),
        );
        let helper_signal = (*request).helper_signal.take();
        request.write(Request::new((*request).handle, (*request).key));
        (*request).endpoint = endpoint;
        (*request).registration = registration;
        (*request).helper_key = helper_key;
        (*request).helper = helper;
        (*request).helper_signal = helper_signal;
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
        if let Endpoint::File(file) = request.endpoint {
            let terminal = match buffer {
                Buffer::Read(bytes) => FileTerminal::Read((*file).read(bytes)),
                Buffer::Write(bytes) => FileTerminal::Write((*file).write(bytes)),
            };
            request.terminal = Some(terminal.result());
            request.file_terminal = Some(terminal);
            // No kernel-retained region or watch exists on this path. Even a
            // failed owned syscall is an actual retired submission, not an
            // admission rejection; query exposes its original result once.
            request.data = ptr::null_mut();
            return Submission::Complete;
        }
        match self.attempt(request) {
            Query::Terminal(Ok(_)) => Submission::Complete,
            Query::Terminal(Err(failure)) => Submission::Failed(failure),
            Query::Pending | Query::PendingFailure(_, _) => Submission::Pending,
        }
    }

    unsafe fn query(&mut self, _: usize, request: *mut Request) -> Query {
        self.attempt(&mut *request)
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
        if request.helper_active {
            return request.helper.cancel().map_err(|failure| {
                (
                    doria_diagnostic_catalogue::IoMessageOperation::Wait,
                    failure,
                )
            });
        }
        self.retire(request).map_err(|failure| {
            (
                doria_diagnostic_catalogue::IoMessageOperation::Unregister,
                failure,
            )
        })
    }

    unsafe fn drain(&mut self, _: usize, request: *mut Request) -> Option<Result<usize, Failure>> {
        if (*request).helper_active && (*request).terminal.is_none() {
            if let Some(terminal) = (*request).helper.drain() {
                // Teardown preserves an actual host result, including a retry
                // hint; it never resubmits work after cancellation.
                let _ = self.finish_helper(&mut *request, terminal);
            } else {
                let _ = self.retire(&mut *request);
            }
        }
        (*request).terminal
    }
}

// The core supplies the exact stable configured native request. Match the
// captured projection identity before dereferencing the leaf's handle home;
// equal fd bits in an unrelated/local cell do not establish this loan.
unsafe impl crate::transfer_group::pending::PendingNativeDriver for Dispatcher<'_> {
    unsafe fn setup_source(
        &mut self,
        _: usize,
        request: *mut Request,
        frame: *const DrStackFrameV2,
        external: Option<*const c_void>,
    ) -> Result<Option<SourceSnapshot>, AdapterTerminal> {
        self.resolve_source(request, frame, external, false)
    }

    unsafe fn select_source(
        &mut self,
        _: usize,
        request: *mut Request,
        frame: *const DrStackFrameV2,
        external: Option<*const c_void>,
    ) -> Result<Option<SourceSnapshot>, AdapterTerminal> {
        self.resolve_source(request, frame, external, true)
    }

    unsafe fn release_source(&mut self, _: usize, request: *mut Request) {
        if (*request).watched || !(*request).data.is_null() || (*request).helper_active {
            crate::runtime_invariant_panic();
        }
        drop((*request).selected.take());
    }
    unsafe fn prepare_region(
        &self,
        _: usize,
        request: *mut Request,
        reading: bool,
        region: crate::transfer_group::pending::DrPendingRegionV1,
    ) -> Option<crate::transfer_group::adapter::DrAdapterOutcomeV1> {
        if !reading {
            return None;
        }
        let loan = self.device_loan(&*request)?;
        let outcome = loan.read_buffered(region.data, region.length)?;
        let terminal = FileTerminal::Read(Ok(outcome));
        let (tag, status) = terminal.status();
        Some(crate::transfer_group::adapter::DrAdapterOutcomeV1 {
            count: status.value as usize,
            tag,
            has_count: u8::from(tag == 0),
        })
    }
    unsafe fn terminal_status(
        &self,
        _: usize,
        request: *mut Request,
        result: Result<usize, Failure>,
    ) -> (u8, DrIoStatusV1) {
        let request = &*request;
        if matches!(
            request.endpoint,
            Endpoint::File(_) | Endpoint::Pipe(_) | Endpoint::Standard(_)
        ) {
            let terminal = request
                .file_terminal
                .unwrap_or_else(|| crate::runtime_invariant_panic());
            if request.terminal != Some(terminal.result()) {
                crate::runtime_invariant_panic();
            }
            terminal.bounded_status(result, request.length)
        } else {
            let mut status = DrIoStatusV1::default();
            let tag = status.complete(result.map(|count| count as u64));
            (tag, status)
        }
    }

    unsafe fn validate_endpoint(
        &self,
        _: usize,
        request: *mut Request,
        endpoint: *const c_void,
    ) -> Result<(), Failure> {
        let request = unsafe { &*request };
        if endpoint.is_null() {
            return Err(Failure::invalid_input());
        }
        match request.endpoint {
            Endpoint::HandleHome(home) if endpoint == home => {
                if unsafe { home.cast::<i32>().read() } != request.handle {
                    return Err(Failure::invalid_input());
                }
            }
            Endpoint::File(file) | Endpoint::Pipe(file) if endpoint == file.cast_const().cast() => {
                if unsafe { (*file).native_handle() } != request.handle {
                    return Err(Failure::invalid_input());
                }
            }
            Endpoint::Standard(stream)
                if endpoint == ptr::from_ref(crate::device_io::standard_device(stream)).cast() =>
            {
                if self.device_loan(request).is_none()
                    || crate::device_io::byte_handle(stream)? != request.handle
                {
                    return Err(Failure::invalid_input());
                }
            }
            _ => return Err(Failure::invalid_input()),
        }
        Ok(())
    }
}

pub(crate) type NativeGroup<'s, 'b> = TransferGroup<'s, 'b, Dispatcher<'s>, WaitControl>;

/// Private transport only. Every supplied handle is consumed on success/failure.
/// # Safety
/// Arrays contain count distinct independently owned idle native handles and
/// keys, and are disjoint from writable status. No other consumer may close,
/// transfer, or change the blocking mode of a supplied handle. Cancellation is
/// null or carries a live claim. Handles must not alias one another.
/// Frame is null or live for owner-allocation failure reporting during this call.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_unix_transfer_set_new(
    frame: *const DrStackFrameV2,
    keys: *const u64,
    handles: *const i32,
    count: usize,
    cancellation: *mut DrCancellationV1,
    status: *mut DrIoStatusV1,
) -> *mut c_void {
    *status = DrIoStatusV1::default();
    if count == 0 {
        (*status).fail(Failure::invalid_input());
        return ptr::null_mut();
    }
    let keys = slice::from_raw_parts(keys, count);
    let handles = slice::from_raw_parts(handles, count);
    let mut owners = match FixedSlice::new(count) {
        Ok(owners) => owners,
        Err(failure) => {
            for handle in handles {
                drop(OwnedHandle::from_raw(*handle));
            }
            (*status).fail(failure);
            return ptr::null_mut();
        }
    };
    for handle in handles {
        // Capacity was validated above and exactly count owners are inserted.
        if owners.push(OwnedHandle::from_raw(*handle)).is_err() {
            crate::runtime_invariant_panic();
        }
    }
    match TransferSet::new(keys, owners, cancellation) {
        Ok(set) => match crate::transfer_group::transport::allocate_set(frame, set) {
            Ok(owner) => return owner,
            Err(failure) => {
                (*status).fail(failure);
            }
        },
        Err(failure) => {
            (*status).fail(failure);
        }
    }
    ptr::null_mut()
}

#[cfg(test)]
mod tests;

/// Returns the same native owners, without closing or changing their modes.
/// # Safety
/// Owner is null or a live set. An active or borrowed-mode set is permitted for
/// checked rejection before native state/handles output access. Otherwise owner
/// is a fixed owning set, exclusively owned, and handles is disjoint writable storage for its configured
/// count. Success consumes the set and transfers each returned handle once;
/// failure leaves the set and output storage unchanged.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_unix_transfer_set_release(
    owner: *mut c_void,
    handles: *mut i32,
    status: *mut DrIoStatusV1,
) -> u8 {
    *status = DrIoStatusV1::default();
    (*status).complete(
        crate::transfer_group::transport::take_set(owner).map(|set| {
            let destinations = slice::from_raw_parts_mut(handles, set.len());
            for (stream, destination) in set.owners.into_iter().zip(destinations) {
                *destination = stream.get();
                // Ownership crosses this private ABI; it is not a stream close.
                mem::forget(stream);
            }
            0
        }),
    )
}

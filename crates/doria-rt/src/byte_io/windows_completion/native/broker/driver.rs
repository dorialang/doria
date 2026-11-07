//! Native driver for the shared transfer core. Configuration retains stable
//! request/routing records; submission and terminal reuse allocate nothing.

use super::*;
use crate::cancellation::WakeRegistration;
use crate::transfer_group::adapter::AdapterTerminal;
use crate::transfer_group::pending::{FileTerminal, PendingNativeDriver};
use crate::transfer_group::setup::GroupSetup;
use crate::transfer_group::source::{
    ResolvedSource, SelectedSource, SourceBinding, SourceProvider, SourceRoutes, SourceSnapshot,
};
use crate::transfer_group::{
    BufferList, Control, FixedSlice, GroupDriver, RequestSlots, SetupFailure, StableSlots, Stop,
    TerminalOutcome, TransferGroup, Wait,
};
use core::cell::UnsafeCell;

mod borrowed;
use borrowed::{BorrowedLeaf, BorrowedRoutes, InlineLeaf, InlineOwner, InlineTransfer};

struct ConfiguredRoute {
    registration: usize,
    node: Route,
    request: *mut Overlapped,
    inline: InlineTransfer,
    selected: UnsafeCell<Option<SelectedSource<InlineLeaf>>>,
}

enum ConfiguredRoutes {
    Fixed(FixedSlice<u64>),
    Borrowed(BorrowedRoutes),
}

impl ConfiguredRoutes {
    fn len(&self) -> usize {
        match self {
            Self::Fixed(routes) => routes.len(),
            Self::Borrowed(routes) => routes.records.len(),
        }
    }

    fn key(&self, index: usize) -> Option<u64> {
        match self {
            Self::Fixed(keys) => keys.as_slice().get(index).copied(),
            Self::Borrowed(routes) => routes.records.get(index).map(|record| record.key),
        }
    }

    fn stream(
        &self,
        owners: &[AttachedStream],
        index: usize,
    ) -> Option<(NativeHandle, *mut Association)> {
        match self {
            Self::Fixed(_) => owners
                .get(index)
                .map(|owner| (owner.handle(), owner.association.raw(0))),
            Self::Borrowed(routes) => routes.records.get(index).and_then(|record| {
                if record.source.is_some() {
                    None
                } else {
                    record.leaf.stream()
                }
            }),
        }
    }

    fn file(&self, index: usize) -> Option<&InlineLeaf> {
        let Self::Borrowed(routes) = self else {
            return None;
        };
        let record = routes.records.get(index)?;
        if record.source.is_some() {
            return None;
        }
        match &record.leaf {
            BorrowedLeaf::File(file) => Some(file),
            BorrowedLeaf::Attached { .. } => None,
        }
    }

    fn pending_endpoint(&self, owners: &[AttachedStream], index: usize) -> Option<*const c_void> {
        match self {
            Self::Fixed(_) => owners.get(index).map(|owner| ptr::from_ref(owner).cast()),
            Self::Borrowed(routes) => routes.records.get(index).map(|record| match &record.leaf {
                BorrowedLeaf::Attached { projection, .. } => *projection,
                BorrowedLeaf::File(file) => file.endpoint(),
            }),
        }
    }
}

pub(crate) struct CompletionSet {
    pub(super) owners: FixedSlice<AttachedStream>,
    pub(super) slots: RequestSlots<Overlapped>,
    routes: ConfiguredRoutes,
    operations: StableSlots<ConfiguredRoute>,
    sources: SourceRoutes<InlineLeaf>,
    group: FixedSlice<Group>,
    broker: BrokerClaim,
}

impl CompletionSet {
    /// Consumes the stream owners on success and failure. Their native affinity
    /// was established by the stream owner, never by membership in this set.
    pub(super) fn from_attached(
        keys: &[u64],
        owners: FixedSlice<AttachedStream>,
    ) -> Result<Self, Failure> {
        if keys.len() != owners.len() {
            return Err(Failure::invalid_input());
        }
        for (index, key) in keys.iter().enumerate() {
            if keys[..index].contains(key) {
                return Err(Failure::invalid_input());
            }
        }
        let broker = BrokerClaim::acquire()?;
        if owners
            .as_slice()
            .iter()
            .any(|owner| owner.broker.0 != broker.0)
        {
            return Err(Failure::invalid_input());
        }
        let slots = RequestSlots::new(keys, |_| Overlapped::new(ptr::null_mut()))?;
        let mut routes = FixedSlice::new(keys.len())?;
        let mut operations = StableSlots::new()?;
        for (index, key) in keys.iter().copied().enumerate() {
            routes.push(key)?;
            operations.insert_at(
                index,
                ConfiguredRoute {
                    registration: index,
                    node: Route::new(),
                    request: slots.raw(index),
                    inline: InlineTransfer::new(),
                    selected: UnsafeCell::new(None),
                },
            )?;
        }
        let mut group = FixedSlice::new(1)?;
        group.push(Group::new())?;
        Ok(Self {
            owners,
            slots,
            routes: ConfiguredRoutes::Fixed(routes),
            operations,
            sources: SourceRoutes::new()?,
            group,
            broker,
        })
    }

    /// Returns the same stream owners after group teardown. Consuming this set
    /// never closes/reopens/duplicates/detaches handles or changes their keys.
    pub(super) fn into_attached(self) -> FixedSlice<AttachedStream> {
        self.owners
    }

    pub(super) fn index_of(&self, key: u64) -> Option<usize> {
        (0..self.routes.len()).find(|index| self.routes.key(*index) == Some(key))
    }

    pub(super) fn key(&self, index: usize) -> Option<u64> {
        self.routes.key(index)
    }

    pub(super) fn len(&self) -> usize {
        (0..self.routes.len())
            .filter(|index| self.key(*index).is_some())
            .count()
    }

    pub(super) fn cached_operation(&self, registration: usize, ordinal: usize) -> Option<usize> {
        self.slots.operation_of(registration, ordinal)
    }

    pub(super) fn operation_registration(&self, operation: usize) -> usize {
        self.slots.registration(operation)
    }

    fn cache_routes(&mut self) -> Result<(), Failure> {
        for index in 0..self.slots.len() {
            if self.slots.contains(index) && !self.operations.contains(index) {
                self.operations.insert_at(
                    index,
                    ConfiguredRoute {
                        registration: self.slots.registration(index),
                        node: Route::new(),
                        request: self.slots.raw(index),
                        inline: InlineTransfer::new(),
                        selected: UnsafeCell::new(None),
                    },
                )?;
            }
        }
        Ok(())
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
        if let ConfiguredRoutes::Borrowed(routes) = &self.routes {
            let bound = (0..routes.records.len())
                .filter(|index| {
                    routes
                        .records
                        .get(*index)
                        .is_some_and(|record| record.source.is_some())
                })
                .count();
            if bound != 0 {
                self.sources.reserve(
                    bound
                        .checked_add(selection.len())
                        .and_then(|count| count.checked_add(1))
                        .ok_or_else(Failure::invalid_input)?,
                )?;
            }
        }
        let operations = self
            .slots
            .configure_operations(selection, |_, _| Ok(Overlapped::new(ptr::null_mut())))?;
        self.cache_routes()?;
        Ok(operations)
    }

    #[cfg(test)]
    pub(super) fn snapshot_for_test(&self, index: usize) -> (usize, usize) {
        (
            self.slots.raw(self.cached_operation(index, 0).unwrap()) as usize,
            self.group.raw(0) as usize,
        )
    }

    #[cfg(test)]
    pub(super) fn operation_snapshot_for_test(
        &self,
        registration: usize,
        ordinal: usize,
    ) -> (usize, usize) {
        let index = self.cached_operation(registration, ordinal).unwrap();
        let operation = self.operations.get(index).unwrap();
        (
            operation.request as usize,
            ptr::addr_of!(operation.node) as usize,
        )
    }

    #[cfg(test)]
    pub(super) fn assert_retired_for_test(&self) {
        let mut routing = self.broker.broker().routing().lock();
        assert!(!unsafe { routing.registry.group_registered(self.group.raw(0)) });
        for index in 0..self.operations.len() {
            let Some(route) = self.operations.get(index) else {
                continue;
            };
            let node = ptr::addr_of!(route.node).cast_mut();
            assert!(!unsafe { routing.registry.route_registered(node) });
            assert!(!unsafe { routing.registry.active(node) });
            assert!(unsafe { routing.registry.query(node) }.is_none());
            assert_eq!(route.request, self.slots.raw(index));
            assert_ne!(
                unsafe { ptr::read_volatile((*route.request).internal.get()) },
                STATUS_PENDING
            );
        }
    }

    pub(super) fn begin<'s, 'b>(
        &'s mut self,
        buffers: BufferList<'b>,
        budget: Option<&DrWaitBudgetV1>,
        cancellation: Option<&DrCancellationV1>,
    ) -> Result<NativeGroup<'s, 'b>, Failure> {
        self.begin_with_driver(buffers, budget, cancellation, core::convert::identity)
    }

    pub(super) fn begin_with_driver<'s, 'b, D, T>(
        &'s mut self,
        buffers: BufferList<'b, T>,
        budget: Option<&DrWaitBudgetV1>,
        cancellation: Option<&DrCancellationV1>,
        wrap: impl FnOnce(Dispatcher<'s>) -> D,
    ) -> Result<TransferGroup<'s, 'b, D, BrokerControl<'s>, T>, Failure>
    where
        D: GroupDriver<T, Request = Overlapped, Timeout = u32>,
        T: TerminalOutcome,
    {
        if self.is_borrowed() || buffers.len() != self.owners.len() {
            return Err(Failure::invalid_input());
        }
        let registrations = self.slots.primary_registrations()?;
        let selection = self.configure_operations(registrations)?;
        self.begin_configured(buffers, budget, cancellation, selection, wrap)
            .map_err(SetupFailure::legacy)
    }

    /// Selection names logical registrations in plan order. Repeated members
    /// configure distinct cached routes/requests on the original association.
    /// The shared core maps buffer/completion indices to those operation slots.
    pub(super) fn begin_selected<'s, 'b>(
        &'s mut self,
        buffers: BufferList<'b>,
        budget: Option<&DrWaitBudgetV1>,
        cancellation: Option<&DrCancellationV1>,
        selection: FixedSlice<usize>,
    ) -> Result<NativeGroup<'s, 'b>, Failure> {
        self.begin_selected_with_driver(
            buffers,
            budget,
            cancellation,
            selection,
            core::convert::identity,
        )
    }

    pub(super) fn begin_selected_with_driver<'s, 'b, D, T>(
        &'s mut self,
        buffers: BufferList<'b, T>,
        budget: Option<&DrWaitBudgetV1>,
        cancellation: Option<&DrCancellationV1>,
        selection: FixedSlice<usize>,
        wrap: impl FnOnce(Dispatcher<'s>) -> D,
    ) -> Result<TransferGroup<'s, 'b, D, BrokerControl<'s>, T>, Failure>
    where
        D: GroupDriver<T, Request = Overlapped, Timeout = u32>,
        T: TerminalOutcome,
    {
        if selection.len() != buffers.len() {
            return Err(Failure::invalid_input());
        }
        let selection = self.configure_operations(selection)?;
        self.begin_configured(buffers, budget, cancellation, selection, wrap)
            .map_err(SetupFailure::legacy)
    }

    pub(super) fn configure_selected_prepared_with_driver<'s, 'b, D, T>(
        &'s mut self,
        buffers: BufferList<'b, T>,
        budget: Option<&DrWaitBudgetV1>,
        cancellation: Option<&DrCancellationV1>,
        selection: FixedSlice<usize>,
        wrap: impl FnOnce(Dispatcher<'s>) -> D,
    ) -> Result<GroupSetup<'s, 'b, D, BrokerControl<'s>, T>, SetupFailure<T>>
    where
        D: GroupDriver<T, Request = Overlapped, Timeout = u32>,
        T: TerminalOutcome,
    {
        self.configure_group(buffers, budget, cancellation, selection, wrap)
    }

    fn begin_configured<'s, 'b, D, T>(
        &'s mut self,
        buffers: BufferList<'b, T>,
        budget: Option<&DrWaitBudgetV1>,
        cancellation: Option<&DrCancellationV1>,
        selection: FixedSlice<usize>,
        wrap: impl FnOnce(Dispatcher<'s>) -> D,
    ) -> Result<TransferGroup<'s, 'b, D, BrokerControl<'s>, T>, SetupFailure<T>>
    where
        D: GroupDriver<T, Request = Overlapped, Timeout = u32>,
        T: TerminalOutcome,
    {
        let mut frame = ptr::null();
        let setup = self.configure_group(buffers, budget, cancellation, selection, |native| {
            let driver = wrap(native);
            frame = driver.call_frame();
            driver
        })?;
        unsafe { setup.finish(frame) }
    }

    fn configure_group<'s, 'b, D, T>(
        &'s mut self,
        buffers: BufferList<'b, T>,
        budget: Option<&DrWaitBudgetV1>,
        cancellation: Option<&DrCancellationV1>,
        selection: FixedSlice<usize>,
        wrap: impl FnOnce(Dispatcher<'s>) -> D,
    ) -> Result<GroupSetup<'s, 'b, D, BrokerControl<'s>, T>, SetupFailure<T>>
    where
        D: GroupDriver<T, Request = Overlapped, Timeout = u32>,
        T: TerminalOutcome,
    {
        let broker = self.broker.broker();
        // Register the wake outside the routing lock. That registration owns
        // the state claim used by both control and driver until group teardown.
        let control = BrokerControl::new(broker, budget, cancellation)?;
        let mut driver = Dispatcher {
            owners: self.owners.as_slice(),
            routes: &mut self.routes,
            operations: &self.operations,
            sources: &mut self.sources,
            devices: [None, None, None],
            group: self.group.raw(0),
            broker,
            cancellation: control.state,
            linked: 0,
            registered: false,
        };
        {
            let mut routing = broker.routing().lock();
            unsafe { routing.registry.insert_group(driver.group)? };
            driver.registered = true;
            for index in 0..driver.operations.len() {
                if !driver.operations.contains(index) {
                    continue;
                }
                let Some((_, association)) = driver.stream(index) else {
                    continue;
                };
                unsafe {
                    routing.registry.insert_route(
                        association,
                        driver.group,
                        driver.route(index),
                        driver.configured_route(index).request as usize,
                    )?;
                }
                // This is a sparse prefix extent, not an occupied-entry count.
                // Drop skips holes while removing exactly the successful links.
                driver.linked = index + 1;
            }
        }
        // The shared owner is installed before the first request. Its one
        // cleanup algorithm cancels all active plan slots before draining any.
        // Idle route metadata does not submit or access an unselected stream.
        let driver = wrap(driver);
        GroupSetup::configure(&mut self.slots, buffers, driver, control, Some(selection))
    }
}

pub(crate) type NativeGroup<'s, 'b> = TransferGroup<'s, 'b, Dispatcher<'s>, BrokerControl<'s>>;

pub(crate) struct BrokerControl<'port> {
    budget: Option<DrWaitBudgetV1>,
    state: *const DrCancellationV1,
    _wake: Option<WakeRegistration<'port>>,
}

impl<'port> BrokerControl<'port> {
    fn new(
        broker: &'port Broker,
        budget: Option<&DrWaitBudgetV1>,
        cancellation: Option<&DrCancellationV1>,
    ) -> Result<Self, Failure> {
        let (state, wake) = if let Some(state) = cancellation {
            let wake = unsafe {
                WakeRegistration::register((state as *const DrCancellationV1).cast_mut(), broker)?
            };
            (state as *const DrCancellationV1, Some(wake))
        } else {
            (ptr::null(), None)
        };
        Ok(Self {
            budget: budget.copied(),
            state,
            _wake: wake,
        })
    }
}

impl Control for BrokerControl<'_> {
    fn next_wait(&mut self) -> Result<Result<Wait<u32>, Stop>, Failure> {
        if unsafe { requested(self.state) } {
            return Ok(Err(Stop::Cancellation));
        }
        let Some(budget) = self.budget else {
            return Ok(Ok(Wait {
                timeout: INFINITE,
                immediate: false,
            }));
        };
        match budget.remaining(
            1_000_000,
            u64::from(INFINITE - 1),
            crate::monotonic_clock::now,
        )? {
            (0, _) => Ok(Ok(Wait {
                timeout: 0,
                immediate: true,
            })),
            (1, _) => Ok(Ok(Wait {
                timeout: INFINITE,
                immediate: false,
            })),
            (2, units) => Ok(Ok(Wait {
                timeout: units as u32,
                immediate: false,
            })),
            (3, _) => Ok(Err(Stop::BudgetExhausted)),
            _ => Err(Failure::invalid_input()),
        }
    }

    fn cancellation_signal(&self) -> Option<usize> {
        None
    }
}

pub(crate) struct Dispatcher<'s> {
    owners: &'s [AttachedStream],
    routes: &'s mut ConfiguredRoutes,
    operations: &'s StableSlots<ConfiguredRoute>,
    sources: &'s mut SourceRoutes<InlineLeaf>,
    devices: [Option<crate::device_io::DeviceLoan<'static>>; 3],
    group: *mut Group,
    broker: &'s Broker,
    // BrokerControl owns a synchronized registration claim. Shared TransferGroup
    // finishes all driver use/drop before its control registration is released.
    cancellation: *const DrCancellationV1,
    linked: usize,
    registered: bool,
}

impl Dispatcher<'_> {
    unsafe fn helper_terminal(
        &self,
        file: &InlineLeaf,
        operation: &InlineTransfer,
        terminal: FileTerminal,
    ) -> FileTerminal {
        let terminal = match terminal {
            FileTerminal::Write(result) => {
                FileTerminal::Write(self.device_loan(file).unwrap().finish_write(result))
            }
            terminal => terminal,
        };
        operation.helper_active.set(false);
        operation.terminal.set(Some(terminal));
        terminal
    }

    fn configured_route(&self, index: usize) -> &ConfiguredRoute {
        self.operations
            .get(index)
            .unwrap_or_else(|| unsafe { crate::runtime_invariant_panic() })
    }

    fn handle(&self, index: usize) -> NativeHandle {
        self.stream(index)
            .map(|(handle, _)| handle)
            .unwrap_or_else(|| unsafe { crate::runtime_invariant_panic() })
    }

    fn stream(&self, index: usize) -> Option<(NativeHandle, *mut Association)> {
        let operation = self.operations.get(index)?;
        self.routes.stream(self.owners, operation.registration)
    }

    fn file(&self, index: usize) -> Option<(&InlineLeaf, &InlineTransfer)> {
        let operation = self.operations.get(index)?;
        if let Some(selected) = unsafe { &*operation.selected.get() } {
            return Some((unsafe { &*selected.platform() }, &operation.inline));
        }
        Some((self.routes.file(operation.registration)?, &operation.inline))
    }

    fn bound(&self, index: usize) -> bool {
        let ConfiguredRoutes::Borrowed(routes) = &*self.routes else {
            return false;
        };
        routes
            .records
            .get(self.configured_route(index).registration)
            .is_some_and(|record| record.source.is_some())
    }

    fn device_loan(&self, file: &InlineLeaf) -> Option<&crate::device_io::DeviceLoan<'static>> {
        let InlineOwner::Standard { stream } = file.owner else {
            return None;
        };
        self.devices[stream as usize].as_ref()
    }

    fn acquire_device(
        &mut self,
        stream: crate::device_io::StandardStream,
        handle: NativeHandle,
    ) -> Result<(), Failure> {
        if crate::device_io::byte_handle(stream)? != handle {
            return Err(Failure::invalid_input());
        }
        if self.devices[stream as usize].is_none() {
            self.devices[stream as usize] =
                Some(crate::device_io::standard_device(stream).try_borrow_for_io()?);
        }
        Ok(())
    }

    unsafe fn resolve_source(
        &mut self,
        index: usize,
        frame: *const crate::DrStackFrameV2,
        external: Option<*const c_void>,
        selecting: bool,
    ) -> Result<Option<SourceSnapshot>, AdapterTerminal> {
        let registration = self.configured_route(index).registration;
        let ConfiguredRoutes::Borrowed(routes) = &mut *self.routes else {
            return if external.is_none() {
                Ok(None)
            } else {
                Err(AdapterTerminal::PlatformFailure(Failure::invalid_input()))
            };
        };
        let record = routes
            .records
            .raw(registration)
            .ok_or_else(|| AdapterTerminal::PlatformFailure(Failure::invalid_input()))?;
        let Some(binding) = (*record).source.as_mut() else {
            return if external.is_none() {
                Ok(None)
            } else {
                Err(AdapterTerminal::PlatformFailure(Failure::invalid_input()))
            };
        };
        let resolved = binding.resolve(frame, external)?;
        let route = self
            .sources
            .rebind(binding, resolved, |_, origin, handle, _| {
                Ok(InlineLeaf::from_origin(origin, handle))
            })
            .map_err(AdapterTerminal::PlatformFailure)?;
        let mut snapshot = route.snapshot();
        if !matches!(
            snapshot.origin,
            crate::native_handle::PhysicalOrigin::Standard(_)
        ) && !snapshot
            .origin
            .is_regular()
            .map_err(AdapterTerminal::PlatformFailure)?
        {
            return Err(AdapterTerminal::PlatformFailure(Failure::unsupported()));
        }
        if selecting {
            let selected = self
                .sources
                .select(route)
                .map_err(AdapterTerminal::PlatformFailure)?;
            snapshot = selected.snapshot();
            let file = &*selected.platform();
            if let InlineOwner::Standard { stream } = file.owner {
                self.acquire_device(stream, file.handle)
                    .map_err(AdapterTerminal::PlatformFailure)?;
            }
            let operation = self.configured_route(index);
            if operation.inline.helper_active.get() {
                crate::runtime_invariant_panic();
            }
            *operation.selected.get() = Some(selected);
        }
        Ok(Some(snapshot))
    }

    fn route(&self, index: usize) -> *mut Route {
        // Mutable links/facts are UnsafeCell-owned and only accessed by Registry
        // under the broker lock. No &mut slice aliases foreign packet routing.
        ptr::addr_of!(self.configured_route(index).node).cast_mut()
    }

    fn check_request(&self, index: usize, request: *mut Overlapped) {
        if self
            .operations
            .get(index)
            .is_none_or(|route| route.request != request)
        {
            unsafe { crate::runtime_invariant_panic() };
        }
    }
}

// The explicit configured leaf mode determines the endpoint's private type.
// Neither mode accepts scalar readiness metadata. A complete adapter may select
// another byte region, but cannot redirect the registered native request.
unsafe impl PendingNativeDriver for Dispatcher<'_> {
    unsafe fn setup_source(
        &mut self,
        index: usize,
        request: *mut Overlapped,
        frame: *const crate::DrStackFrameV2,
        external: Option<*const c_void>,
    ) -> Result<Option<SourceSnapshot>, AdapterTerminal> {
        self.check_request(index, request);
        self.resolve_source(index, frame, external, false)
    }

    unsafe fn select_source(
        &mut self,
        index: usize,
        request: *mut Overlapped,
        frame: *const crate::DrStackFrameV2,
        external: Option<*const c_void>,
    ) -> Result<Option<SourceSnapshot>, AdapterTerminal> {
        self.check_request(index, request);
        self.resolve_source(index, frame, external, true)
    }

    unsafe fn release_source(&mut self, index: usize, request: *mut Overlapped) {
        self.check_request(index, request);
        let operation = self.configured_route(index);
        if operation.inline.helper_active.get() {
            crate::runtime_invariant_panic();
        }
        drop((&mut *operation.selected.get()).take());
    }
    unsafe fn prepare_region(
        &self,
        index: usize,
        _: *mut Overlapped,
        reading: bool,
        region: crate::transfer_group::pending::DrPendingRegionV1,
    ) -> Option<crate::transfer_group::adapter::DrAdapterOutcomeV1> {
        if !reading {
            return None;
        }
        let (file, _) = self.file(index)?;
        let loan = self.device_loan(file)?;
        let result = loan.read_buffered(region.data, region.length)?;
        let (tag, status) = FileTerminal::Read(Ok(result)).status();
        Some(crate::transfer_group::adapter::DrAdapterOutcomeV1 {
            count: status.value as usize,
            tag,
            has_count: u8::from(tag == 0),
        })
    }
    unsafe fn validate_endpoint(
        &self,
        index: usize,
        request: *mut Overlapped,
        endpoint: *const c_void,
    ) -> Result<(), Failure> {
        let operation = self
            .operations
            .get(index)
            .ok_or_else(Failure::invalid_input)?;
        let expected = self
            .file(index)
            .map(|(file, _)| file.endpoint())
            .or_else(|| {
                self.routes
                    .pending_endpoint(self.owners, operation.registration)
            });
        if endpoint.is_null()
            || expected != Some(endpoint)
            || self
                .operations
                .get(index)
                .is_none_or(|route| route.request != request)
        {
            return Err(Failure::invalid_input());
        }
        if let Some((file, _)) = self.file(index) {
            // The opaque identity check above precedes interpreting it as a
            // File. The configured owner stays stable through the retained loan.
            return file.validate();
        }
        let Some((handle, association)) = self.stream(index) else {
            return Err(Failure::invalid_input());
        };
        // Exact configured owner identity was checked BEFORE this typed cast.
        // PendingNativeDriver's caller retains that live projection throughout
        // the registration. No owner or kernel request is borrowed mutably here.
        let stream = unsafe { &*endpoint.cast::<AttachedStream>() };
        if !ptr::eq(stream.broker.0.as_ptr(), self.broker)
            || stream.owner.as_ref().map(OwnedHandle::get) != Some(handle)
            || stream.association.raw(0) != association
        {
            return Err(Failure::invalid_input());
        }
        Ok(())
    }

    unsafe fn terminal_status(
        &self,
        index: usize,
        request: *mut Overlapped,
        result: Result<usize, Failure>,
    ) -> (u8, DrIoStatusV1) {
        self.check_request(index, request);
        if let Some((_, operation)) = self.file(index) {
            // EOF, empty Data, WouldBlock and checked failure are the exact
            // synchronous owner's terminal facts, not guesses from count zero.
            return operation
                .terminal
                .get()
                .unwrap_or_else(|| crate::runtime_invariant_panic())
                .bounded_status(result, operation.selected_length.get());
        }
        let mut status = DrIoStatusV1::default();
        let tag = status.complete(result.map(|count| count as u64));
        (tag, status)
    }
}

impl Drop for Dispatcher<'_> {
    fn drop(&mut self) {
        let mut routing = self.broker.routing().lock();
        while self.linked != 0 {
            self.linked -= 1;
            if self.stream(self.linked).is_none() {
                continue;
            }
            if unsafe { routing.registry.remove_route(self.route(self.linked)) }.is_err() {
                unsafe { crate::runtime_invariant_panic() };
            }
        }
        if self.registered && unsafe { routing.registry.remove_group(self.group) }.is_err() {
            unsafe { crate::runtime_invariant_panic() };
        }
        drop(routing);
        // This guard is released before BrokerControl unregisters the wake.
    }
}

// Stable slots/buffers remain owned by the exclusively borrowed shared group.
// Arm precedes FFI, acceptance follows FFI, and a terminal packet is never exposed
// before acceptance finalizes. Every accepted attached-pipe transfer retires one
// packet, including synchronous success. Typed regular File operations use the
// same owner's synchronous methods and cache their actual retired result instead;
// they never arm, route, consume, or fabricate a completion packet.
unsafe impl GroupDriver for Dispatcher<'_> {
    type Request = Overlapped;
    type Timeout = u32;
    const MAXIMUM_TRANSFER_BYTES: usize = u32::MAX as usize;

    unsafe fn resource(
        &self,
        index: usize,
        request: *mut Overlapped,
    ) -> Result<crate::transfer_group::TransferResource, Failure> {
        self.check_request(index, request);
        if let Some(selected) = &*self.configured_route(index).selected.get() {
            return Ok(selected.resource());
        }
        if self.bound(index) {
            return Err(Failure::invalid_input());
        }
        let handle = self
            .file(index)
            .map_or_else(|| self.handle(index), |(file, _)| file.handle);
        crate::native_handle::transfer_resource(handle)
    }

    unsafe fn acquire_request(
        &mut self,
        index: usize,
        request: *mut Overlapped,
    ) -> Result<(), Failure> {
        self.check_request(index, request);
        if self.bound(index) {
            return Ok(());
        }
        if let Some((file, _)) = self.file(index) {
            if let InlineOwner::Standard { stream, .. } = file.owner {
                file.validate()?;
                let handle = file.handle;
                self.acquire_device(stream, handle)?;
            }
        }
        Ok(())
    }

    unsafe fn release_request(&mut self, index: usize, _: *mut Overlapped) {
        let operation = self.configured_route(index);
        (&mut *operation.inline.helper.get()).shutdown();
        operation.inline.helper_active.set(false);
        drop((&mut *operation.selected.get()).take());
    }

    unsafe fn reset_request(&mut self, index: usize, request: *mut Overlapped) {
        self.check_request(index, request);
        if (&*self.configured_route(index).selected.get()).is_some() {
            crate::runtime_invariant_panic();
        }
        if self.bound(index) || self.file(index).is_some() {
            let operation = &self.configured_route(index).inline;
            operation.terminal.set(None);
            operation.selected_length.set(0);
            operation.helper_active.set(false);
            operation.control_failure.set(None);
            request.write(Overlapped::new(ptr::null_mut()));
            return;
        }
        if self
            .broker
            .routing()
            .lock()
            .registry
            .active(self.route(index))
        {
            crate::runtime_invariant_panic();
        }
        request.write(Overlapped::new(ptr::null_mut()));
    }

    unsafe fn submit(
        &mut self,
        index: usize,
        request: *mut Overlapped,
        buffer: &mut Buffer<'_>,
    ) -> Submission {
        self.check_request(index, request);
        if let Some((file, operation)) = self.file(index) {
            if operation.terminal.get().is_some() {
                crate::runtime_invariant_panic();
            }
            // The compiler retains one exclusive complete owner at this stable
            // address. These methods preserve its existing append/cursor/close
            // contract; no broker lock, new native handle, or callback bypass is
            // involved in the selected underlying leaf operation.
            operation.selected_length.set(buffer.len());
            if matches!(file.owner, InlineOwner::Standard { .. }) && buffer.len() != 0 {
                // Opening-mode proof and byte I/O both run on the helper. An
                // inherited alias may serialize native metadata work with an
                // already pending operation; submission must not block here.
                let result = (|| {
                    *operation.helper_notice.get() = borrowed::HelperNotice {
                        broker: self.broker,
                        group: self.group,
                    };
                    let (data, reading) = match buffer {
                        Buffer::Read(bytes) => (bytes.as_mut_ptr(), true),
                        Buffer::Write(bytes) => (bytes.as_ptr().cast_mut(), false),
                    };
                    (&mut *operation.helper.get()).submit(crate::device_io::helper::Job {
                        handle: file.handle,
                        data,
                        length: buffer.len(),
                        reading,
                        notify: borrowed::helper_notice,
                        notice: operation.helper_notice.get() as usize,
                    })?;
                    operation.helper_active.set(true);
                    Ok::<_, Failure>(())
                })();
                return match result {
                    Ok(()) => Submission::Pending,
                    Err(failure) => {
                        operation.control_failure.set(Some(failure));
                        Submission::Pending
                    }
                };
            }
            let terminal = file.transfer(buffer, self.device_loan(file));
            operation.terminal.set(Some(terminal));
            return Submission::Complete;
        }
        let route = self.route(index);
        if self.broker.routing().lock().registry.arm(route).is_err() {
            crate::runtime_invariant_panic();
        }
        let handle = self.handle(index);
        let success = match buffer {
            Buffer::Read(bytes) => ReadFile(
                handle,
                bytes.as_mut_ptr().cast(),
                request_length(bytes.len()),
                ptr::null_mut(),
                request.cast(),
            ),
            Buffer::Write(bytes) => WriteFile(
                handle,
                bytes.as_ptr().cast(),
                request_length(bytes.len()),
                ptr::null_mut(),
                request.cast(),
            ),
        };
        // Synchronous success still queues this handle's one packet; only a
        // synchronous failure queues none (see `issued`).
        let rejection = match issued(success, || GetLastError()) {
            Issued::Completed | Issued::Pending => None,
            Issued::Failed(code) => Some(Failure::from_system_code(code.into())),
        };
        let mut routing = self.broker.routing().lock();
        let finalized = match rejection {
            None => routing.registry.accepted(route),
            Some(failure) => routing.registry.rejected(route, failure),
        };
        if finalized.is_err() {
            crate::runtime_invariant_panic();
        }
        // An early packet was hidden until this finalization. Wake protected
        // followers now, without a second native notification or allocation.
        self.broker.routing().wake_all();
        match rejection {
            None => Submission::Pending,
            Some(failure) => {
                // The kernel never took this request; it is idle and ours again.
                // Rewrite it so its Internal no longer reads STATUS_PENDING.
                request.write(Overlapped::new(ptr::null_mut()));
                Submission::Failed(failure)
            }
        }
    }

    unsafe fn query(&mut self, index: usize, request: *mut Overlapped) -> Query {
        self.check_request(index, request);
        if let Some((file, operation)) = self.file(index) {
            if let Some(failure) = operation.control_failure.get() {
                return Query::PendingFailure(
                    doria_diagnostic_catalogue::IoMessageOperation::Register,
                    failure,
                );
            }
            if operation.helper_active.get() && operation.terminal.get().is_none() {
                match (&*operation.helper.get()).query() {
                    Some(terminal) => {
                        self.helper_terminal(file, operation, terminal);
                    }
                    None => return Query::Pending,
                }
            }
            return Query::Terminal(
                operation
                    .terminal
                    .get()
                    .unwrap_or_else(|| crate::runtime_invariant_panic())
                    .result(),
            );
        }
        self.broker
            .routing()
            .lock()
            .registry
            .query(self.route(index))
            .map_or(Query::Pending, Query::Terminal)
    }

    fn wait_any(&mut self, timeout: u32, _: Option<usize>) -> Result<Wake, Failure> {
        unsafe {
            self.broker
                .wait(self.group, timeout, self.cancellation, None)
        }
    }

    unsafe fn cancel(
        &mut self,
        index: usize,
        request: *mut Overlapped,
    ) -> Result<(), (doria_diagnostic_catalogue::IoMessageOperation, Failure)> {
        self.check_request(index, request);
        if let Some((_, operation)) = self.file(index) {
            if operation.helper_active.get() {
                return (&*operation.helper.get()).cancel().map_err(|failure| {
                    (
                        doria_diagnostic_catalogue::IoMessageOperation::Wait,
                        failure,
                    )
                });
            }
            // The operation already retired in submit. Cancellation cannot
            // erase a won result or undo its cursor/append side effects.
            return Ok(());
        }
        if CancelIoEx(self.handle(index), request.cast()) != 0 {
            Ok(())
        } else {
            Err((
                doria_diagnostic_catalogue::IoMessageOperation::Wait,
                last_failure(),
            ))
        }
    }

    unsafe fn drain(
        &mut self,
        index: usize,
        request: *mut Overlapped,
    ) -> Option<Result<usize, Failure>> {
        self.check_request(index, request);
        if let Some((file, operation)) = self.file(index) {
            if operation.control_failure.get().is_some() {
                return None;
            }
            if operation.helper_active.get() && operation.terminal.get().is_none() {
                let terminal = (&*operation.helper.get()).drain()?;
                self.helper_terminal(file, operation, terminal);
            }
            return Some(
                operation
                    .terminal
                    .get()
                    .unwrap_or_else(|| crate::runtime_invariant_panic())
                    .result(),
            );
        }
        let route = self.route(index);
        loop {
            {
                let mut routing = self.broker.routing().lock();
                // The registry owns each submission's one terminal: a published
                // packet, or the synchronous failure `rejected` recorded.
                if let Some(result) = routing.registry.query(route) {
                    return Some(result);
                }
                // Only an accepted, unpublished submission has a packet coming.
                // A slot that was never armed has nothing to drain.
                if !routing.registry.active(route) {
                    return None;
                }
            }
            if self
                .broker
                .wait(self.group, INFINITE, ptr::null(), Some(route))
                .is_err()
            {
                // A queue failure is not retirement proof. The shared core must
                // never release the kernel's borrowed storage on that assumption.
                crate::runtime_invariant_panic();
            }
        }
    }
}

#[cfg(test)]
#[path = "../port/tests.rs"]
mod legacy_tests;

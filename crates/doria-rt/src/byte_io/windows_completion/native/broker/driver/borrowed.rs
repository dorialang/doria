//! Sparse native registration metadata. These snapshots borrow a complete
//! stream-owned attachment or a typed regular File owner; they never own,
//! close, or reassociate either handle.

use super::*;
use crate::file_io::stream::OwnedFile;
use crate::transfer_group::pending::FileTerminal;
use core::cell::{Cell, UnsafeCell};

pub(super) enum InlineOwner {
    File(*mut OwnedFile),
    Standard {
        stream: crate::device_io::StandardStream,
    },
}

pub(super) struct InlineLeaf {
    pub(super) owner: InlineOwner,
    pub(super) handle: NativeHandle,
}

/// Pinned operation-local facts. Sharing a registration does not share a
/// helper, result, selected region, or cancellation/retirement obligation.
pub(super) struct InlineTransfer {
    // Only the exclusive group caller accesses this cell. No broker or kernel
    // observes it; a synchronous File operation retires before returning.
    pub(super) terminal: Cell<Option<FileTerminal>>,
    pub(super) selected_length: Cell<usize>,
    pub(super) helper: UnsafeCell<crate::device_io::helper::Helper>,
    pub(super) helper_active: Cell<bool>,
    pub(super) control_failure: Cell<Option<Failure>>,
    pub(super) helper_notice: UnsafeCell<HelperNotice>,
}

impl InlineTransfer {
    pub(super) fn new() -> Self {
        Self {
            terminal: Cell::new(None),
            selected_length: Cell::new(0),
            helper: UnsafeCell::new(crate::device_io::helper::Helper::new()),
            helper_active: Cell::new(false),
            control_failure: Cell::new(None),
            helper_notice: UnsafeCell::new(HelperNotice {
                broker: ptr::null(),
                group: ptr::null_mut(),
            }),
        }
    }
}

pub(super) struct HelperNotice {
    pub(super) broker: *const Broker,
    pub(super) group: *mut Group,
}

pub(super) unsafe fn helper_notice(context: usize) {
    let notice = &*(context as *const HelperNotice);
    // The driver/group and broker claim outlive helper retirement AND join.
    // Durable readiness survives a foreign pump consuming the pointer-free poke.
    (*notice.group)
        .helper_ready
        .store(true, core::sync::atomic::Ordering::Release);
    if (*notice.broker).wake().is_err() {
        crate::runtime_invariant_panic();
    }
}

impl InlineLeaf {
    pub(super) fn from_origin(
        origin: crate::native_handle::PhysicalOrigin,
        handle: NativeHandle,
    ) -> Self {
        let owner = match origin {
            crate::native_handle::PhysicalOrigin::File { file, .. } => InlineOwner::File(file),
            crate::native_handle::PhysicalOrigin::Standard(stream) => {
                InlineOwner::Standard { stream }
            }
        };
        Self { owner, handle }
    }
    pub(super) fn endpoint(&self) -> *const c_void {
        match self.owner {
            InlineOwner::File(file) => file.cast_const().cast(),
            InlineOwner::Standard { stream, .. } => {
                ptr::from_ref(crate::device_io::standard_device(stream)).cast()
            }
        }
    }

    pub(super) unsafe fn validate(&self) -> Result<(), Failure> {
        let handle = match self.owner {
            InlineOwner::File(file) => (*file).native_handle(),
            InlineOwner::Standard { stream, .. } => crate::device_io::byte_handle(stream)?,
        };
        if handle != self.handle {
            return Err(Failure::invalid_input());
        }
        Ok(())
    }

    pub(super) unsafe fn transfer(
        &self,
        buffer: &mut crate::transfer_group::Buffer<'_>,
        loan: Option<&crate::device_io::DeviceLoan<'static>>,
    ) -> FileTerminal {
        use crate::transfer_group::Buffer;
        match self.owner {
            InlineOwner::File(file) => match buffer {
                Buffer::Read(bytes) => FileTerminal::Read((*file).read(bytes)),
                Buffer::Write(bytes) => FileTerminal::Write((*file).write(bytes)),
            },
            InlineOwner::Standard { .. } => {
                let loan = loan.unwrap_or_else(|| crate::runtime_invariant_panic());
                // Nonempty raw standard-device work belongs to the helper
                // submission path. Only queued input/empty regions stay inline.
                if buffer.len() != 0 {
                    crate::runtime_invariant_panic();
                }
                match buffer {
                    Buffer::Read(bytes) => FileTerminal::Read(loan.read_attempt(
                        self.handle,
                        bytes.as_mut_ptr(),
                        bytes.len(),
                    )),
                    Buffer::Write(bytes) => FileTerminal::Write(loan.write_attempt(
                        self.handle,
                        bytes.as_ptr(),
                        bytes.len(),
                    )),
                }
            }
        }
    }
}

pub(super) enum BorrowedLeaf {
    Attached {
        projection: *const c_void,
        handle: NativeHandle,
        association: *mut Association,
    },
    File(InlineLeaf),
}

impl BorrowedLeaf {
    fn handle(&self) -> NativeHandle {
        match self {
            Self::Attached { handle, .. } => *handle,
            Self::File(file) => file.handle,
        }
    }

    pub(super) fn stream(&self) -> Option<(NativeHandle, *mut Association)> {
        match self {
            Self::Attached {
                handle,
                association,
                ..
            } => Some((*handle, *association)),
            Self::File(_) => None,
        }
    }
}

pub(super) struct BorrowedRegistration {
    pub(super) key: u64,
    pub(super) leaf: BorrowedLeaf,
    pending_supported: bool,
    pub(super) source: Option<SourceBinding>,
}

pub(super) struct BorrowedRoutes {
    pub(super) records: StableSlots<BorrowedRegistration>,
    last_key: u64,
}

impl CompletionSet {
    pub(in super::super) fn borrowed_new() -> Result<Self, Failure> {
        let broker = BrokerClaim::acquire()?;
        let slots = RequestSlots::dynamic()?;
        let routes = ConfiguredRoutes::Borrowed(BorrowedRoutes {
            records: StableSlots::new()?,
            last_key: 0,
        });
        let mut group = FixedSlice::new(1)?;
        group.push(Group::new())?;
        Ok(Self {
            owners: FixedSlice::new(0)?,
            slots,
            routes,
            operations: StableSlots::new()?,
            sources: SourceRoutes::new()?,
            group,
            broker,
        })
    }

    pub(in super::super) fn is_borrowed(&self) -> bool {
        matches!(self.routes, ConfiguredRoutes::Borrowed(_))
    }

    /// # Safety
    /// Projection is null for checked rejection or points to one live idle
    /// AttachedStream only during this call. Its complete owner remains alive,
    /// exclusively retained by the compiler, until registration removal or set
    /// destruction and every group has finished. No other I/O or close consumer
    /// may use that stream. Moving the inline owner does not move its association.
    /// A pending adapter additionally retains this exact inline projection home;
    /// raw borrowed routing continues to use only the persistent association.
    pub(in super::super) unsafe fn add_borrowed(
        &mut self,
        key: u64,
        projection: *const c_void,
    ) -> Result<usize, Failure> {
        let ConfiguredRoutes::Borrowed(routes) = &mut self.routes else {
            return Err(Failure::invalid_input());
        };
        if projection.is_null() || key == 0 || key == u64::MAX || key <= routes.last_key {
            return Err(Failure::invalid_input());
        }
        let stream = &*projection.cast::<AttachedStream>();
        if stream.broker.0 != self.broker.0 {
            return Err(Failure::invalid_input());
        }
        let handle = stream
            .owner
            .as_ref()
            .map(OwnedHandle::get)
            .ok_or_else(Failure::invalid_input)?;
        let association = stream.association.raw(0);
        if (0..routes.records.len()).any(|index| {
            routes.records.get(index).is_some_and(|record| {
                record.source.is_none()
                    && (record.leaf.handle() == handle
                        || record
                            .leaf
                            .stream()
                            .is_some_and(|(_, current)| current == association))
            })
        }) {
            return Err(Failure::invalid_input());
        }

        self.insert_borrowed(
            key,
            BorrowedLeaf::Attached {
                projection,
                handle,
                association,
            },
        )
    }

    /// # Safety
    /// File is null for rejection or a live complete OwnedFile retained at this
    /// address, exclusively, until removal/set destruction and group teardown.
    /// Registration borrows its existing sole close owner. Classification occurs
    /// only at admission; neither submission nor preparation reopens the file.
    pub(in super::super) unsafe fn add_file(
        &mut self,
        key: u64,
        file: *mut OwnedFile,
    ) -> Result<usize, Failure> {
        let ConfiguredRoutes::Borrowed(routes) = &self.routes else {
            return Err(Failure::invalid_input());
        };
        if file.is_null() || key == 0 || key == u64::MAX || key <= routes.last_key {
            return Err(Failure::invalid_input());
        }
        let handle = (&*file).pending_regular_handle()?;
        if (0..routes.records.len()).any(|index| {
            routes
                .records
                .get(index)
                .is_some_and(|record| record.source.is_none() && record.leaf.handle() == handle)
        }) {
            return Err(Failure::invalid_input());
        }
        self.insert_borrowed(
            key,
            BorrowedLeaf::File(InlineLeaf {
                owner: InlineOwner::File(file),
                handle,
            }),
        )
    }

    /// Borrow the actual standard-device handle, without changing its opening
    /// mode, close obligation or native completion association. Helpers start
    /// only for selected nonempty synchronous operations, never registration.
    pub(in super::super) unsafe fn add_standard(
        &mut self,
        key: u64,
        stream: crate::device_io::StandardStream,
    ) -> Result<usize, Failure> {
        let ConfiguredRoutes::Borrowed(routes) = &self.routes else {
            return Err(Failure::invalid_input());
        };
        if key == 0 || key == u64::MAX || key <= routes.last_key {
            return Err(Failure::invalid_input());
        }
        let handle = crate::device_io::byte_handle(stream)?;
        if (0..routes.records.len()).any(|index| {
            routes
                .records
                .get(index)
                .is_some_and(|record| {
                    // Distinct logical standard channels may share one native
                    // handle. No mode/association mutation requires exclusivity
                    // over that physical alias; each retains its own device loan.
                    let other_standard = matches!(&record.leaf,
                        BorrowedLeaf::File(InlineLeaf { owner: InlineOwner::Standard { stream: other }, .. })
                        if *other != stream);
                    record.source.is_none() && record.leaf.handle() == handle && !other_standard
                })
        }) {
            return Err(Failure::invalid_input());
        }
        self.insert_borrowed(
            key,
            BorrowedLeaf::File(InlineLeaf {
                owner: InlineOwner::Standard { stream },
                handle,
            }),
        )
    }

    /// # Safety
    /// Typed origin borrows the actual retained owner. Membership requires no
    /// transfer callbacks, regular-file restriction or native association.
    pub(in super::super) unsafe fn add_readiness_origin(
        &mut self,
        key: u64,
        origin: crate::native_handle::PhysicalOrigin,
    ) -> Result<usize, Failure> {
        match origin {
            crate::native_handle::PhysicalOrigin::Standard(stream) => {
                self.add_standard(key, stream)
            }
            crate::native_handle::PhysicalOrigin::File { file, .. } => {
                let ConfiguredRoutes::Borrowed(routes) = &self.routes else {
                    return Err(Failure::invalid_input());
                };
                if key == 0 || key == u64::MAX || key <= routes.last_key {
                    return Err(Failure::invalid_input());
                }
                let handle = origin.handle()?;
                let pending_supported = crate::native_handle::is_regular_file(handle)?;
                if (0..routes.records.len()).any(|index| {
                    routes.records.get(index).is_some_and(|record| {
                        record.source.is_none() && record.leaf.handle() == handle
                    })
                }) {
                    return Err(Failure::invalid_input());
                }
                self.insert_borrowed_with_capability(
                    key,
                    BorrowedLeaf::File(InlineLeaf {
                        owner: InlineOwner::File(file),
                        handle,
                    }),
                    pending_supported,
                )
            }
        }
    }

    pub(in super::super) fn validate_pending_registration(
        &self,
        index: usize,
    ) -> Result<(), Failure> {
        let ConfiguredRoutes::Borrowed(routes) = &self.routes else {
            return Err(Failure::invalid_input());
        };
        let record = routes
            .records
            .get(index)
            .ok_or_else(Failure::invalid_input)?;
        if record.source.is_some() {
            return Ok(());
        }
        if !record.pending_supported {
            return Err(Failure::unsupported());
        }
        if let BorrowedLeaf::File(InlineLeaf {
            owner: InlineOwner::File(file),
            handle,
            ..
        }) = &record.leaf
        {
            if unsafe { (**file).native_handle() } != *handle {
                return Err(Failure::invalid_input());
            }
        }
        Ok(())
    }

    fn insert_borrowed(&mut self, key: u64, leaf: BorrowedLeaf) -> Result<usize, Failure> {
        self.insert_borrowed_with_capability(key, leaf, true)
    }

    fn insert_borrowed_with_capability(
        &mut self,
        key: u64,
        leaf: BorrowedLeaf,
        pending_supported: bool,
    ) -> Result<usize, Failure> {
        let ConfiguredRoutes::Borrowed(routes) = &mut self.routes else {
            return Err(Failure::invalid_input());
        };

        // Neither insertion has native side effects. If the independent route
        // allocation fails, remove only this new idle request record. Existing
        // pointer-table growth and every unaffected pinned leaf remain valid.
        let index = routes.records.vacant_index();
        let record = BorrowedRegistration {
            key,
            leaf,
            pending_supported,
            source: None,
        };
        routes.records.insert_at(index, record)?;
        if let Err(failure) = self
            .slots
            .add_registration(index, key, |_| Ok(Overlapped::new(ptr::null_mut())))
        {
            routes.records.remove(index);
            return Err(failure);
        }
        routes.last_key = key;
        if let Err(failure) = self.cache_routes() {
            self.remove_borrowed(key)?;
            return Err(failure);
        }
        Ok(index)
    }

    pub(in super::super) fn remove_borrowed(&mut self, key: u64) -> Result<(), Failure> {
        self.remove_borrowed_with(key, |_| {})
    }

    pub(in super::super) unsafe fn add_bound_source(
        &mut self,
        key: u64,
        source: *const c_void,
        provider: SourceProvider,
    ) -> Result<usize, Failure> {
        let resolved = ResolvedSource::borrow(source, None)?;
        let origin = resolved.origin();
        self.sources.reserve(
            self.len()
                .checked_add(2)
                .ok_or_else(Failure::invalid_input)?,
        )?;
        let mut binding = SourceBinding::new(provider);
        self.sources
            .rebind(&mut binding, resolved, |_, origin, handle, _| {
                Ok(InlineLeaf::from_origin(origin, handle))
            })?;
        match self.add_readiness_origin(key, origin) {
            Ok(index) => {
                let ConfiguredRoutes::Borrowed(routes) = &mut self.routes else {
                    crate::runtime_invariant_panic();
                };
                (*routes.records.raw(index).unwrap()).source = Some(binding);
                Ok(index)
            }
            Err(failure) => {
                self.sources.unbind(&mut binding);
                Err(failure)
            }
        }
    }

    pub(in super::super) unsafe fn update_external_source(
        &mut self,
        index: usize,
        source: *const c_void,
    ) -> Result<crate::native_handle::PhysicalOrigin, Failure> {
        let ConfiguredRoutes::Borrowed(routes) = &mut self.routes else {
            return Err(Failure::invalid_input());
        };
        let record = routes
            .records
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
            .rebind(binding, resolved, |_, origin, handle, _| {
                Ok(InlineLeaf::from_origin(origin, handle))
            })?;
        Ok(origin)
    }

    pub(in super::super) fn remove_borrowed_with(
        &mut self,
        key: u64,
        mut retire: impl FnMut(usize),
    ) -> Result<(), Failure> {
        let index = self.index_of(key).ok_or_else(Failure::invalid_input)?;
        let ConfiguredRoutes::Borrowed(routes) = &mut self.routes else {
            return Err(Failure::invalid_input());
        };
        if !routes.records.contains(index) {
            unsafe { crate::runtime_invariant_panic() };
        }
        // Exclusive set access follows group teardown, which has already retired
        // and unlinked every route. The stream's persistent association survives.
        for operation in 0..self.slots.len() {
            if self.slots.contains(operation) && self.slots.registration(operation) == index {
                retire(operation);
                self.operations.remove(operation);
            }
        }
        unsafe {
            if let Some(binding) = (*routes.records.raw(index).unwrap()).source.as_mut() {
                self.sources.unbind(binding);
            }
        }
        if self.slots.remove_registration(index).is_err() || routes.records.remove(index).is_none()
        {
            unsafe { crate::runtime_invariant_panic() };
        }
        Ok(())
    }

    #[cfg(test)]
    pub(in super::super) fn borrowed_snapshot_for_test(
        &self,
        index: usize,
    ) -> (usize, usize, usize, usize) {
        let ConfiguredRoutes::Borrowed(routes) = &self.routes else {
            panic!("borrowed registration required");
        };
        let record = routes.records.get(index).expect("live registration");
        let operation = self.cached_operation(index, 0).expect("primary operation");
        let route = self
            .operations
            .get(operation)
            .expect("configured operation");
        (
            record.leaf.handle() as usize,
            record
                .leaf
                .stream()
                .map_or(0, |(_, association)| association as usize),
            route.request as usize,
            ptr::addr_of!(route.node) as usize,
        )
    }
}

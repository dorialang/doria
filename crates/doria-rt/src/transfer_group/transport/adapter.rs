//! Borrowed and owned complete adapters use the existing generic transfer core.
//! Sources supply wait metadata only. Synchronous callbacks transfer bytes;
//! pending callbacks prepare/complete around the same native transfer engine.
//! Both modes share identities, loan validation, group gates and cleanup.

use super::*;
use crate::cancellation::DrCancellationV1;
use crate::file_io::stream::OwnedFile;
use crate::native_handle::{PhysicalOrigin, ReadinessProjection, ReadinessSource};
use crate::owned_interface::OwnedInterfaceValue;
use crate::transfer_group::adapter::{AdapterTerminal, DrAdapterOutcomeV1, DrTransferAdapterV1};
use crate::transfer_group::pending::{DrPendingTransferAdapterV1, PendingAdapter};
use crate::transfer_group::source::{ResolveSourceCallback, SourceProvider};
use crate::transfer_group::{SetupFailure, StableSlots};
use crate::{DrInterfaceCarrierV1, DrStackFrameV2};

mod host;

#[cfg(windows)]
use crate::byte_io::{NativeAdapterGroup as AdapterGroup, NativeAdapterSet as AdapterSet};
#[cfg(windows)]
use crate::byte_io::{NativePendingGroup as PendingGroup, NativePendingSet as PendingSet};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use crate::unix_readiness::adapter::{AdapterGroup, AdapterSet};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use crate::unix_readiness::pending::{PendingGroup, PendingSet};

enum AdapterSetMode<'s> {
    Synchronous(&'s mut AdapterSet),
    Pending(&'s mut PendingSet),
}

impl<'s> AdapterSetMode<'s> {
    fn index_of(&self, key: u64) -> Option<usize> {
        match self {
            Self::Synchronous(set) => set.index_of(key),
            Self::Pending(set) => set.index_of(key),
        }
    }

    fn remove(&mut self, key: u64) -> Result<(), Failure> {
        match self {
            Self::Synchronous(set) => set.remove(key),
            Self::Pending(set) => set.remove(key),
        }
    }

    unsafe fn begin_selected<'b>(
        self,
        frame: *const DrStackFrameV2,
        buffers: BufferList<'b, AdapterTerminal>,
        budget: &DrWaitBudgetV1,
        selection: FixedSlice<usize>,
    ) -> Result<AdapterGroupMode<'s, 'b>, SetupFailure<AdapterTerminal>> {
        match self {
            Self::Synchronous(set) => set
                .begin_selected(frame, buffers, budget, selection)
                .map(AdapterGroupMode::Synchronous),
            Self::Pending(set) => set
                .begin_selected(frame, buffers, budget, selection)
                .map(AdapterGroupMode::Pending),
        }
    }

    unsafe fn begin_selected_controlled<'b>(
        self,
        frame: *const DrStackFrameV2,
        buffers: BufferList<'b, AdapterTerminal>,
        budget: &DrWaitBudgetV1,
        selection: FixedSlice<usize>,
        cancellation: *mut DrCancellationV1,
    ) -> Result<AdapterGroupMode<'s, 'b>, SetupFailure<AdapterTerminal>> {
        match self {
            Self::Synchronous(set) => set
                .begin_selected_controlled(frame, buffers, budget, selection, cancellation)
                .map(AdapterGroupMode::Synchronous),
            Self::Pending(set) => set
                .begin_selected_controlled(frame, buffers, budget, selection, cancellation)
                .map(AdapterGroupMode::Pending),
        }
    }
}

enum AdapterGroupMode<'s, 'b> {
    Synchronous(AdapterGroup<'s, 'b>),
    Pending(PendingGroup<'s, 'b>),
}

impl AdapterGroupMode<'_, '_> {
    unsafe fn drop_in_frame(self, frame: *const DrStackFrameV2) {
        match self {
            Self::Synchronous(group) => group.drop_in_frame(frame),
            Self::Pending(group) => group.drop_in_frame(frame),
        }
    }

    unsafe fn observe(
        &mut self,
        frame: *const DrStackFrameV2,
        budget: DrWaitBudgetV1,
    ) -> crate::transfer_group::Observation<AdapterTerminal> {
        match self {
            Self::Synchronous(group) => group.observe(frame, budget),
            Self::Pending(group) => group.observe(frame, budget),
        }
    }

    unsafe fn submit(
        &mut self,
        frame: *const DrStackFrameV2,
        index: usize,
        offset: usize,
        length: usize,
    ) -> Result<(), Failure> {
        match self {
            Self::Synchronous(group) => group.submit(frame, index, offset, length),
            Self::Pending(group) => group.submit(frame, index, offset, length),
        }
    }

    fn watch(&mut self, sources: &[crate::transfer_group::WatchSource]) -> Result<(), Failure> {
        match self {
            Self::Synchronous(group) => group.watch(sources),
            Self::Pending(group) => group.watch(sources),
        }
    }

    fn rearm_watch(&mut self, index: usize) -> Result<(), Failure> {
        match self {
            Self::Synchronous(group) => group.rearm_watch(index),
            Self::Pending(group) => group.rearm_watch(index),
        }
    }

    fn request_info(&self, index: usize) -> Option<(bool, usize)> {
        match self {
            Self::Synchronous(group) => group.request_info(index),
            Self::Pending(group) => group.request_info(index),
        }
    }

    fn completed(&self, index: usize) -> Option<&[u8]> {
        match self {
            Self::Synchronous(group) => group.completed(index),
            Self::Pending(group) => group.completed(index),
        }
    }

    fn read_storage_mut(&mut self, index: usize) -> Option<&mut [u8]> {
        match self {
            Self::Synchronous(group) => group.read_storage_mut(index),
            Self::Pending(group) => group.read_storage_mut(index),
        }
    }

    fn stop(&self) -> Option<Stop> {
        match self {
            Self::Synchronous(group) => group.stop(),
            Self::Pending(group) => group.stop(),
        }
    }
}

#[repr(C)]
struct AdapterSetOwner {
    pending: bool,
    // Capability-neutral public membership uses the same pending/native core,
    // but never requires a member to offer a transfer operation.
    readiness: bool,
    active: bool,
    identity: usize,
    next_registration: u64,
    retained: StableSlots<OwnedRegistration>,
}

struct OwnedRegistration {
    key: u64,
    value: OwnedInterfaceValue,
}

impl AdapterSetOwner {
    fn retained_index(&self, key: u64) -> Option<usize> {
        (0..self.retained.len()).find(|index| {
            self.retained
                .raw(*index)
                .is_some_and(|value| unsafe { (*value).key == key })
        })
    }

    /// Validates the shared membership boundary before reading a retained
    /// value. Lookup and removal use the same identity, liveness and group gate.
    /// # Safety
    /// Set is null or live storage with its matching native mode. Key is null
    /// or readable common key storage, except when an earlier gate rejects it.
    /// No incompatible set mutation may overlap the call or a returned loan.
    unsafe fn member(
        set: *const Self,
        key: *const c_void,
    ) -> Result<(u64, Option<usize>), Failure> {
        if set.is_null() || key.is_null() || (*set).active {
            return Err(Failure::invalid_input());
        }
        let key = *key.cast::<PlanKey>();
        if key.owner != (*set).identity {
            return Err(Failure::invalid_input());
        }
        let index = if (*set).pending {
            let native = &(*set.cast::<AdapterSetStorage<PendingSet>>()).native;
            native.index_of(key.value)
        } else {
            let native = &(*set.cast::<AdapterSetStorage<AdapterSet>>()).native;
            native.index_of(key.value)
        };
        if index.is_none() {
            return Err(Failure::invalid_input());
        }
        Ok((key.value, (*set).retained_index(key.value)))
    }
}

// Each native mode shares one allocation with its header. Dispatch borrows
// that state instead of reserving the largest mode in every set owner.
#[repr(C)]
struct AdapterSetStorage<T> {
    owner: AdapterSetOwner,
    native: ManuallyDrop<T>,
}

impl<T> AdapterSetStorage<T> {
    unsafe fn clear_in_frame(&mut self, frame: *const DrStackFrameV2) {
        // Retire borrowed native metadata before complete values close their
        // underlying resources; cleanup borrows only its current caller frame.
        ManuallyDrop::drop(&mut self.native);
        self.owner
            .retained
            .clear_with(|member| member.value.drop_in_frame(frame));
    }
}

impl<T> Drop for AdapterSetStorage<T> {
    fn drop(&mut self) {
        unsafe { self.clear_in_frame(ptr::null()) };
    }
}

impl AdapterSetOwner {
    /// # Safety
    /// Set is the live base pointer from allocate_adapter_set, exclusively
    /// borrowed for 's. The stored mode matches its native storage type.
    unsafe fn native<'s>(set: *mut Self) -> AdapterSetMode<'s> {
        if (*set).pending {
            AdapterSetMode::Pending(&mut (*set.cast::<AdapterSetStorage<PendingSet>>()).native)
        } else {
            AdapterSetMode::Synchronous(&mut (*set.cast::<AdapterSetStorage<AdapterSet>>()).native)
        }
    }
}

struct AdapterGroupOwner {
    group: ManuallyDrop<AdapterGroupMode<'static, 'static>>,
    set: *mut AdapterSetOwner,
    identity: super::super::plan::GroupIdentity,
}

impl AdapterGroupOwner {
    unsafe fn clear_in_frame(&mut self, frame: *const DrStackFrameV2) {
        // Retire every pending callback/watch and release buffer loans
        // before allowing membership edits or dropping retained owners.
        ManuallyDrop::take(&mut self.group).drop_in_frame(frame);
        (*self.set).active = false;
    }
}

impl Drop for AdapterGroupOwner {
    fn drop(&mut self) {
        unsafe { self.clear_in_frame(ptr::null()) };
    }
}

/// # Safety
/// Outputs are disjoint writable usize locations. The callback record is
/// borrowed/copied at registration; it carries no close claim or saved frame.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_adapter_layout(size: *mut usize, align: *mut usize) {
    *size = mem::size_of::<DrTransferAdapterV1>();
    *align = mem::align_of::<DrTransferAdapterV1>();
}

/// # Safety
/// Outputs are disjoint writable usize locations. Outcomes are inline values;
/// checked errors use the existing two-word interface carrier separately.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_adapter_outcome_layout(
    size: *mut usize,
    align: *mut usize,
) {
    *size = mem::size_of::<DrAdapterOutcomeV1>();
    *align = mem::align_of::<DrAdapterOutcomeV1>();
}

/// # Safety
/// Frame is null or live for this call only. Cancellation is null or live for
/// this call. Status is disjoint writable
/// storage. Compiler drop glue releases this metadata before retained streams.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_adapter_set_new(
    frame: *const DrStackFrameV2,
    cancellation: *mut DrCancellationV1,
    status: *mut DrIoStatusV1,
) -> *mut c_void {
    adapter_set_new(frame, cancellation, false, status)
}

/// # Safety
/// Same ownership and cancellation contract as the synchronous set constructor.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_pending_transfer_adapter_set_new(
    frame: *const DrStackFrameV2,
    cancellation: *mut DrCancellationV1,
    status: *mut DrIoStatusV1,
) -> *mut c_void {
    adapter_set_new(frame, cancellation, true, status)
}

unsafe fn adapter_set_new(
    frame: *const DrStackFrameV2,
    cancellation: *mut DrCancellationV1,
    pending: bool,
    status: *mut DrIoStatusV1,
) -> *mut c_void {
    *status = DrIoStatusV1::default();
    let identity = match next_identity(&NEXT_SET_IDENTITY) {
        Ok(identity) => identity,
        Err(failure) => {
            (*status).fail(failure);
            return ptr::null_mut();
        }
    };
    if pending {
        allocate_adapter_set(frame, PendingSet::new(cancellation), identity, true, status)
    } else {
        allocate_adapter_set(
            frame,
            AdapterSet::new(cancellation),
            identity,
            false,
            status,
        )
    }
}

/// # Safety
/// Pending is true exactly for PendingSet storage, false for AdapterSet.
/// Frame is null or live for this call; status is disjoint writable storage,
/// and native fits the runtime alignment. Owner heap failure is fatal P1601;
/// identity and native-resource exhaustion retain their checked status contract.
unsafe fn allocate_adapter_set<T>(
    frame: *const DrStackFrameV2,
    native: Result<T, Failure>,
    identity: usize,
    pending: bool,
    status: *mut DrIoStatusV1,
) -> *mut c_void {
    let native = match native {
        Ok(native) => native,
        Err(failure) => {
            (*status).fail(failure);
            return ptr::null_mut();
        }
    };
    let retained = match StableSlots::new() {
        Ok(retained) => retained,
        Err(failure) => {
            (*status).fail(failure);
            return ptr::null_mut();
        }
    };
    let owner = allocate_owner::<AdapterSetStorage<T>>(frame);
    owner.write(AdapterSetStorage {
        owner: AdapterSetOwner {
            pending,
            readiness: false,
            active: false,
            identity,
            next_registration: 0,
            retained,
        },
        native: ManuallyDrop::new(native),
    });
    owner.cast()
}

/// Empty owning membership. Native watcher/broker acquisition is deferred to
/// admission; construction chooses no member capabilities and performs no I/O.
/// Owner heap-allocation failure is fatal P1601, not a checked IoError.
/// # Safety
/// Frame is null or live for this call. Status is disjoint writable storage.
/// The result is uniquely owned, and all
/// groups must retire before removal or destruction.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_readiness_set_new(
    frame: *const DrStackFrameV2,
    status: *mut DrIoStatusV1,
) -> *mut c_void {
    *status = DrIoStatusV1::default();
    let identity = match next_identity(&NEXT_SET_IDENTITY) {
        Ok(identity) => identity,
        Err(failure) => {
            (*status).fail(failure);
            return ptr::null_mut();
        }
    };
    let set = allocate_adapter_set(frame, PendingSet::empty(), identity, true, status);
    if !set.is_null() {
        (*set.cast::<AdapterSetOwner>()).readiness = true;
    }
    set
}

/// Retains the original complete value and its typed physical provenance.
/// Pending metadata is optional, compiler-produced static capability evidence;
/// no transfer callback runs during admission. Failure consumes the taken T.
/// # Safety
/// Frame is null or live for this call, never retained. Set is null or live
/// exclusive readiness-set storage. Source is a typed live
/// readiness VALUE forwarded through the taken complete object, not its home.
/// Adapter is null for source-only membership or readable pending metadata with
/// context equal to the original object. Value/key/status follow owned admission.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_readiness_set_add_owned(
    frame: *const DrStackFrameV2,
    set: *mut c_void,
    source: *const c_void,
    resolve: Option<ResolveSourceCallback>,
    adapter: *const DrPendingTransferAdapterV1,
    value: *mut DrInterfaceCarrierV1,
    key: *mut c_void,
    status: *mut DrIoStatusV1,
) -> u8 {
    let context = if value.is_null() {
        ptr::null_mut()
    } else {
        (*value).object.cast()
    };
    retain_admission(frame, set, value, key, status, |receiver_home| {
        let Some(resolve) = resolve else {
            return (*status).fail(Failure::invalid_input());
        };
        let adapter = if adapter.is_null() {
            None
        } else {
            let adapter = *adapter;
            if adapter.context.object != context || !adapter.valid() {
                return (*status).fail(Failure::invalid_input());
            }
            Some(PendingAdapter::Native(adapter))
        };
        readiness_set_add(
            set,
            source,
            SourceProvider::Native {
                receiver_home,
                resolve,
            },
            adapter,
            key,
            status,
        )
    })
}

/// Admission for a complete value retained by an external backend owner.
/// The native set borrows only typed readiness metadata, never the value.
/// # Safety
/// Set is null or live exclusive readiness storage. Source is null or a typed
/// readiness value whose complete owner remains live until removal/set drop.
/// Key has the queried common layout; key/status are disjoint writable storage.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_readiness_set_add_source(
    set: *mut c_void,
    source: *const c_void,
    key: *mut c_void,
    status: *mut DrIoStatusV1,
) -> u8 {
    readiness_set_add(set, source, SourceProvider::External, None, key, status)
}

/// Hosted pending capabilities are recorded without native callbacks or a
/// fabricated receiver. The host retains and dispatches its complete value.
/// # Safety
/// Same external complete-owner, source and output contract as add_source.
/// Direction flags describe the actual nominal pending interfaces of that value.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_readiness_set_add_host_pending_source(
    set: *mut c_void,
    source: *const c_void,
    readable: u8,
    writable: u8,
    key: *mut c_void,
    status: *mut DrIoStatusV1,
) -> u8 {
    if readable > 1 || writable > 1 || readable | writable == 0 {
        *status = DrIoStatusV1::default();
        if !key.is_null() {
            key.cast::<PlanKey>().write(PlanKey { owner: 0, value: 0 });
        }
        return (*status).fail(Failure::invalid_input());
    }
    readiness_set_add(
        set,
        source,
        SourceProvider::External,
        Some(PendingAdapter::Host {
            readable: readable != 0,
            writable: writable != 0,
        }),
        key,
        status,
    )
}

unsafe fn readiness_set_add(
    set: *mut c_void,
    source: *const c_void,
    provider: SourceProvider,
    adapter: Option<PendingAdapter>,
    key: *mut c_void,
    status: *mut DrIoStatusV1,
) -> u8 {
    *status = DrIoStatusV1::default();
    if !key.is_null() {
        key.cast::<PlanKey>().write(PlanKey { owner: 0, value: 0 });
    }
    let set = set.cast::<AdapterSetOwner>();
    if set.is_null() || key.is_null() || (*set).active || !(*set).readiness || source.is_null() {
        return (*status).fail(Failure::invalid_input());
    }
    let value = match registration::next_registration(&mut (*set).next_registration) {
        Ok(value) => value,
        Err(failure) => return (*status).fail(failure),
    };
    let AdapterSetMode::Pending(native) = AdapterSetOwner::native(set) else {
        return (*status).fail(Failure::invalid_input());
    };
    (*status).complete(
        native
            .add_bound_source(value, source, provider, adapter)
            .map(|_| {
                key.cast::<PlanKey>().write(PlanKey {
                    owner: (*set).identity,
                    value,
                });
                0
            }),
    )
}

/// Fresh-source metadata boundary for externally retained members. The hosted
/// controller invokes the ordinary getter before EACH source-dependent step.
/// This does not make a cached update sufficient for later preparation or wait.
/// Native-owned membership cannot be substituted through this bridge.
/// # Safety
/// Set/key follow the external membership gate. Source is a fresh typed live
/// projection, not cached opaque bits; status is disjoint writable storage.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_readiness_set_update_source(
    set: *mut c_void,
    key: *const c_void,
    source: *const c_void,
    status: *mut DrIoStatusV1,
) -> u8 {
    *status = DrIoStatusV1::default();
    let set = set.cast::<AdapterSetOwner>();
    if set.is_null() || !(*set).readiness || source.is_null() {
        return (*status).fail(Failure::invalid_input());
    }
    let value = match AdapterSetOwner::member(set, key) {
        Ok((value, None)) => value,
        Ok((_, Some(_))) => return (*status).fail(Failure::invalid_input()),
        Err(failure) => return (*status).fail(failure),
    };
    let AdapterSetMode::Pending(native) = AdapterSetOwner::native(set) else {
        return (*status).fail(Failure::invalid_input());
    };
    let Some(index) = native.index_of(value) else {
        return (*status).fail(Failure::invalid_input());
    };
    (*status).complete(native.update_external_source(index, source).map(|()| 0))
}

/// Removes borrowed metadata before the external backend returns its owned T.
/// Native-retained members cannot be removed through this nonowning bridge.
/// # Safety
/// Set/key/status follow the common borrowed-metadata removal contract.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_readiness_set_remove_source(
    set: *mut c_void,
    key: *const c_void,
    status: *mut DrIoStatusV1,
) -> u8 {
    *status = DrIoStatusV1::default();
    if set.is_null() || !(*set.cast::<AdapterSetOwner>()).readiness {
        return (*status).fail(Failure::invalid_input());
    }
    dr_v1_transfer_adapter_set_remove(set, key, status)
}

/// Validates an externally retained member without accessing a native owner.
/// The backend lends its original complete value only after this succeeds.
/// No metadata is removed, no callback runs and no native handle is acquired.
/// Native-owned members cannot be accessed through this external-owner bridge.
/// # Safety
/// Set is null or live readiness storage, borrowed for this call and the
/// resulting backend loan. Key follows the common membership lookup contract;
/// status is disjoint writable storage. The backend enforces the loan's access
/// mode and lifetime and prevents incompatible set mutation/group use/drop.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_readiness_set_get_source(
    set: *mut c_void,
    key: *const c_void,
    status: *mut DrIoStatusV1,
) -> u8 {
    *status = DrIoStatusV1::default();
    let set = set.cast::<AdapterSetOwner>();
    if set.is_null() || !(*set).readiness {
        return (*status).fail(Failure::invalid_input());
    }
    match AdapterSetOwner::member(set, key) {
        Ok((_, None)) => 0,
        Ok((_, Some(_))) => (*status).fail(Failure::invalid_input()),
        Err(failure) => (*status).fail(failure),
    }
}

/// Lends the unchanged original carrier; its complete value remains set-owned.
/// Both readonly and writable access use this same projection. Compiler loans
/// preserve registered T, access mode and lifetime; neither word is recast.
/// Value home is the pinned original interface-carrier storage, not the set's
/// binding, this call's output carrier or a reconstructed class representation.
/// There is no allocation, retention, removal, callback or byte I/O.
/// # Safety
/// Set/key obey membership lookup. Value is disjoint EMPTY writable carrier
/// storage; nonempty storage is allowed only for unchanged rejection. Success
/// returns NONOWNING metadata: it must never be dropped or taken as a value.
/// Value home is null for rejection or disjoint writable pointer storage. It
/// is cleared on failure without dereferencing its previous address. The home
/// carries no ownership claim and follows the carrier_home access contract.
/// The set and member remain live and unmoved throughout the compiler-enforced
/// loan, with no incompatible set mutation, group creation or destruction.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_readiness_set_get_owned(
    set: *mut c_void,
    key: *const c_void,
    value: *mut DrInterfaceCarrierV1,
    value_home: *mut *mut DrInterfaceCarrierV1,
    status: *mut DrIoStatusV1,
) -> u8 {
    *status = DrIoStatusV1::default();
    if !value_home.is_null() {
        *value_home = ptr::null_mut();
    }
    let set = set.cast::<AdapterSetOwner>();
    if set.is_null()
        || key.is_null()
        || value.is_null()
        || value_home.is_null()
        || (*set).active
        || !(*set).readiness
        || !(*value).object.is_null()
        || !(*value).vtable.is_null()
    {
        return (*status).fail(Failure::invalid_input());
    }
    let index = match AdapterSetOwner::member(set, key) {
        Ok((_, Some(index))) => index,
        Ok((_, None)) => return (*status).fail(Failure::invalid_input()),
        Err(failure) => return (*status).fail(failure),
    };
    let member = (*set).retained.raw(index).unwrap();
    let original = OwnedInterfaceValue::carrier_home(ptr::addr_of_mut!((*member).value));
    (*value).object = (*original).object;
    (*value).vtable = (*original).vtable;
    *value_home = original;
    0
}

/// # Safety
/// Set/key/value/status obey owned removal. Rejection preserves membership and
/// the taken value; success returns the original carrier into EMPTY storage.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_readiness_set_remove_owned(
    set: *mut c_void,
    key: *const c_void,
    value: *mut DrInterfaceCarrierV1,
    status: *mut DrIoStatusV1,
) -> u8 {
    *status = DrIoStatusV1::default();
    if set.is_null() || !(*set.cast::<AdapterSetOwner>()).readiness {
        return (*status).fail(Failure::invalid_input());
    }
    dr_v1_transfer_adapter_set_remove_owned(set, key, value, status)
}

/// Retires native metadata, drops complete values in reverse acquisition order,
/// and frees the single owner allocation. No additional class-free is needed.
/// # Safety
/// Frame is null or live for this call, never retained. Set is null or uniquely
/// owned readiness-set storage with no live group.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_readiness_set_drop(frame: *const DrStackFrameV2, set: *mut c_void) {
    dr_v1_transfer_adapter_set_drop(frame, set);
}

/// Private raw-fixture source-kind registration. Source is the encoded raw
/// readiness VALUE, NOT a compiler-owned typed source or its home.
/// The callback context denotes the complete registered adapter, which can
/// differ from that underlying file owner. No read/write callback runs here.
/// # Safety
/// Set is null or a live exclusive adapter set. Source/adapter may be null for
/// rejection; otherwise source is a live file loan and adapter readable. Key
/// uses the common queried layout and is disjoint writable storage. Compiler
/// retains the complete adapter and source owner until successful removal or
/// metadata destruction; neither may move, close or be destroyed during use.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_adapter_set_add(
    set: *mut c_void,
    source: *const c_void,
    adapter: *const DrTransferAdapterV1,
    key: *mut c_void,
    status: *mut DrIoStatusV1,
) -> u8 {
    synchronous_set_add(set, source, false, None, adapter, key, status)
}

/// Private fixed-owner typed-source admission. Unlike owning membership this
/// has no logical resolver; its unsafe caller keeps the same source owner live.
/// Its typed physical identity is borrowed; admission performs no callback.
/// # Safety
/// Source is null for rejection or a live typed readiness VALUE, not its home,
/// raw fixture bits, a class pointer, or a guessed platform-handle conversion.
/// The complete adapter and physical owner remain retained and unmoved through
/// removal or metadata destruction. Other gates match raw synchronous add.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_adapter_set_add_source(
    set: *mut c_void,
    source: *const c_void,
    adapter: *const DrTransferAdapterV1,
    key: *mut c_void,
    status: *mut DrIoStatusV1,
) -> u8 {
    synchronous_set_add(set, source, true, None, adapter, key, status)
}

/// Transfers a complete value into the existing synchronous adapter set.
/// Admission invokes no transfer callback; failure drops the taken value.
/// # Safety
/// Frame is null or live for this call, never retained. Value is a disjoint
/// writable owned compiler interface carrier. Source and
/// adapter borrow that complete value's stable payload, preserving the static
/// readiness capability. Resolve is mandatory and borrows the actual retained
/// carrier home at each source-dependent boundary; no cached fallback exists.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_adapter_set_add_owned(
    frame: *const DrStackFrameV2,
    set: *mut c_void,
    source: *const c_void,
    resolve: Option<ResolveSourceCallback>,
    adapter: *const DrTransferAdapterV1,
    value: *mut DrInterfaceCarrierV1,
    key: *mut c_void,
    status: *mut DrIoStatusV1,
) -> u8 {
    retain_admission(frame, set, value, key, status, |receiver_home| {
        let Some(resolve) = resolve else {
            return (*status).fail(Failure::invalid_input());
        };
        if adapter.is_null() || (*adapter).context.object != (*receiver_home).object.cast() {
            return (*status).fail(Failure::invalid_input());
        }
        synchronous_set_add(
            set,
            source,
            true,
            Some(SourceProvider::Native {
                receiver_home,
                resolve,
            }),
            adapter,
            key,
            status,
        )
    })
}

/// # Safety
/// Frame is null or live for this call, never retained. Same complete-value
/// contract as synchronous owned admission. Source is the
/// typed readiness value and adapter has the pending callback layout.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_pending_transfer_adapter_set_add_owned(
    frame: *const DrStackFrameV2,
    set: *mut c_void,
    source: *const c_void,
    resolve: Option<ResolveSourceCallback>,
    adapter: *const DrPendingTransferAdapterV1,
    value: *mut DrInterfaceCarrierV1,
    key: *mut c_void,
    status: *mut DrIoStatusV1,
) -> u8 {
    retain_admission(frame, set, value, key, status, |receiver_home| {
        let Some(resolve) = resolve else {
            return (*status).fail(Failure::invalid_input());
        };
        pending_set_add(
            set,
            PendingAdmission::Bound(
                source,
                SourceProvider::Native {
                    receiver_home,
                    resolve,
                },
            ),
            adapter,
            key,
            status,
        )
    })
}

unsafe fn retain_admission(
    frame: *const DrStackFrameV2,
    set: *mut c_void,
    value: *mut DrInterfaceCarrierV1,
    key: *mut c_void,
    status: *mut DrIoStatusV1,
    admit: impl FnOnce(*const DrInterfaceCarrierV1) -> u8,
) -> u8 {
    *status = DrIoStatusV1::default();
    if !key.is_null() {
        key.cast::<PlanKey>().write(PlanKey { owner: 0, value: 0 });
    }
    if value.is_null() {
        return (*status).fail(Failure::invalid_input());
    }
    let value =
        OwnedInterfaceValue::from_carrier(mem::replace(&mut *value, crate::empty_error_carrier()));
    let set = set.cast::<AdapterSetOwner>();
    if set.is_null() || key.is_null() || (*set).active {
        value.drop_in_frame(frame);
        return (*status).fail(Failure::invalid_input());
    }
    let index = (*set).retained.vacant_index();
    let mut value = Some(value);
    // Stage ownership/table growth before native admission can borrow it.
    if let Err(failure) = (*set).retained.insert_at_with(index, || OwnedRegistration {
        key: 0,
        value: value.take().expect("admission takes one complete value"),
    }) {
        value.unwrap().drop_in_frame(frame);
        return (*status).fail(failure);
    }
    let retained = (*set).retained.raw(index).unwrap();
    let receiver_home = OwnedInterfaceValue::carrier_home(ptr::addr_of_mut!((*retained).value));
    let result = admit(receiver_home);
    if result == 0 {
        (*(*set).retained.raw(index).unwrap()).key = (*key.cast::<PlanKey>()).value;
    } else {
        (*set)
            .retained
            .remove(index)
            .unwrap()
            .value
            .drop_in_frame(frame);
    }
    result
}

unsafe fn synchronous_set_add(
    set: *mut c_void,
    source: *const c_void,
    typed: bool,
    provider: Option<SourceProvider>,
    adapter: *const DrTransferAdapterV1,
    key: *mut c_void,
    status: *mut DrIoStatusV1,
) -> u8 {
    *status = DrIoStatusV1::default();
    if !key.is_null() {
        key.cast::<PlanKey>().write(PlanKey { owner: 0, value: 0 });
    }
    let set = set.cast::<AdapterSetOwner>();
    if set.is_null() || key.is_null() || (*set).active || source.is_null() || adapter.is_null() {
        return (*status).fail(Failure::invalid_input());
    }
    let AdapterSetMode::Synchronous(native) = AdapterSetOwner::native(set) else {
        // Reject wrong-mode inputs before decoding a scalar source or reading
        // a descriptor with a different callback layout.
        return (*status).fail(Failure::invalid_input());
    };
    let value = match registration::next_registration(&mut (*set).next_registration) {
        Ok(value) => value,
        Err(failure) => return (*status).fail(failure),
    };
    if let Some(provider) = provider {
        return (*status).complete(
            native
                .add_bound_source(value, source, provider, *adapter)
                .map(|_| {
                    key.cast::<PlanKey>().write(PlanKey {
                        owner: (*set).identity,
                        value,
                    });
                    0
                }),
        );
    }
    let handle = match if typed {
        ReadinessProjection::borrow(source).and_then(|source| match source.origin {
            PhysicalOrigin::File { .. } => source.origin.handle(),
            // A synchronous callback has no explicit retained device-loan
            // transport yet. Never erase this provenance to raw handle bits
            // and accidentally bypass the canonical access/queue owner.
            PhysicalOrigin::Standard(_) => Err(Failure::unsupported()),
        })
    } else {
        ReadinessSource::handle(source)
    } {
        Ok(handle) => handle,
        Err(failure) => return (*status).fail(failure),
    };
    (*status).complete(native.add(value, handle, *adapter).map(|_| {
        key.cast::<PlanKey>().write(PlanKey {
            owner: (*set).identity,
            value,
        });
        0
    }))
}

/// # Safety
/// Private raw-fixture admission. The endpoint is the typed leaf-owned
/// projection required by PendingSet::add,
/// NEVER a scalar readiness source or an owner reinterpretation. Compiler loans
/// retain that leaf and the complete adapter, unmoved, through group retirement.
/// Other pointer/active-set rejection and key contracts match synchronous add.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_pending_transfer_adapter_set_add(
    set: *mut c_void,
    endpoint: *const c_void,
    adapter: *const DrPendingTransferAdapterV1,
    key: *mut c_void,
    status: *mut DrIoStatusV1,
) -> u8 {
    pending_set_add(
        set,
        PendingAdmission::Native(endpoint),
        adapter,
        key,
        status,
    )
}

enum PendingAdmission {
    Native(*const c_void),
    File(*mut OwnedFile),
    FileObject(*mut c_void),
    Source(*const c_void),
    Bound(*const c_void, SourceProvider),
}

impl PendingAdmission {
    fn is_null(&self) -> bool {
        match self {
            Self::Native(endpoint) => endpoint.is_null(),
            Self::File(file) => file.is_null(),
            Self::FileObject(file) => file.is_null(),
            Self::Source(source) => source.is_null(),
            Self::Bound(source, _) => source.is_null(),
        }
    }
}

/// Private raw-file fixture admission into the SAME pending set/driver. It has
/// no public checked origin; compiler adapters use typed-source admission.
/// File-kind validation happens during configuration, never in preparation.
/// The existing file owner retains its cursor, append state and close claim.
/// # Safety
/// File is null for rejection or points to the actual live OwnedFile storage,
/// never its scalar readiness value or an AttachedStream reinterpretation.
/// The compiler retains that owner and complete adapter exclusively, unmoved,
/// until removal or set destruction after all groups have retired. Remaining
/// descriptor, key and active-set contracts match native pending admission.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_pending_transfer_adapter_set_add_file(
    set: *mut c_void,
    file: *mut c_void,
    adapter: *const DrPendingTransferAdapterV1,
    key: *mut c_void,
    status: *mut DrIoStatusV1,
) -> u8 {
    pending_set_add(
        set,
        PendingAdmission::File(file.cast()),
        adapter,
        key,
        status,
    )
}

/// Admit the actual compiler file payload, preserving its error-origin loan
/// before preparation. This shares native admission/retirement with raw File
/// leaves; raw OwnedFile admission remains available for private fixtures.
/// # Safety
/// File is null for rejection or a live complete file class payload retained
/// exclusively and unmoved through removal and group retirement. Never pass
/// OwnedFile storage, readiness metadata, or a path-derived substitute here.
/// All set/descriptor/key/status contracts match pending_set_add.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_pending_transfer_adapter_set_add_file_object(
    set: *mut c_void,
    file: *mut c_void,
    adapter: *const DrPendingTransferAdapterV1,
    key: *mut c_void,
    status: *mut DrIoStatusV1,
) -> u8 {
    pending_set_add(
        set,
        PendingAdmission::FileObject(file),
        adapter,
        key,
        status,
    )
}

/// Admit the forwarded compiler-owned source with its real physical origin
/// BEFORE any preparation callback. The source does not substitute for the
/// complete adapter: callbacks still dispatch through its retained context.
/// # Safety
/// Source is null for rejection or the live typed opaque VALUE, never its home,
/// raw fixture bits, or an owner reinterpretation. The adapter and physical
/// source owner remain exclusively retained at stable addresses through removal
/// and every group retirement. Remaining gates match pending_set_add.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_pending_transfer_adapter_set_add_source(
    set: *mut c_void,
    source: *const c_void,
    adapter: *const DrPendingTransferAdapterV1,
    key: *mut c_void,
    status: *mut DrIoStatusV1,
) -> u8 {
    pending_set_add(set, PendingAdmission::Source(source), adapter, key, status)
}

unsafe fn pending_set_add(
    set: *mut c_void,
    admission: PendingAdmission,
    adapter: *const DrPendingTransferAdapterV1,
    key: *mut c_void,
    status: *mut DrIoStatusV1,
) -> u8 {
    *status = DrIoStatusV1::default();
    if !key.is_null() {
        key.cast::<PlanKey>().write(PlanKey { owner: 0, value: 0 });
    }
    let set = set.cast::<AdapterSetOwner>();
    if set.is_null() || key.is_null() || (*set).active || admission.is_null() || adapter.is_null() {
        return (*status).fail(Failure::invalid_input());
    }
    let AdapterSetMode::Pending(native) = AdapterSetOwner::native(set) else {
        return (*status).fail(Failure::invalid_input());
    };
    let value = match registration::next_registration(&mut (*set).next_registration) {
        Ok(value) => value,
        Err(failure) => return (*status).fail(failure),
    };
    let added = match admission {
        PendingAdmission::Native(endpoint) => native.add(value, endpoint, *adapter),
        PendingAdmission::File(file) => native.add_file(value, file, *adapter),
        PendingAdmission::FileObject(file) => native.add_file_object(value, file, *adapter),
        PendingAdmission::Source(source) => native.add_source(value, source, *adapter),
        PendingAdmission::Bound(source, provider) => native.add_bound_source(
            value,
            source,
            provider,
            Some(PendingAdapter::Native(*adapter)),
        ),
    };
    (*status).complete(added.map(|_| {
        key.cast::<PlanKey>().write(PlanKey {
            owner: (*set).identity,
            value,
        });
        0
    }))
}

/// # Safety
/// Set is null or a live exclusive adapter set. Key is null or readable common
/// key storage. Success removes metadata only; compiler returns the complete T.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_adapter_set_remove(
    set: *mut c_void,
    key: *const c_void,
    status: *mut DrIoStatusV1,
) -> u8 {
    *status = DrIoStatusV1::default();
    let set = set.cast::<AdapterSetOwner>();
    let key = match AdapterSetOwner::member(set, key) {
        Ok((key, None)) => key,
        Ok((_, Some(_))) => return (*status).fail(Failure::invalid_input()),
        Err(failure) => return (*status).fail(failure),
    };
    (*status).complete(AdapterSetOwner::native(set).remove(key).map(|()| 0))
}

/// Returns the exact owned carrier after native membership has retired.
/// Failure leaves both membership and its complete value in the set.
/// # Safety
/// Set/key follow ordinary removal. Value is disjoint EMPTY writable carrier
/// storage. Compiler checking preserves registered T; neither word is recast.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_adapter_set_remove_owned(
    set: *mut c_void,
    key: *const c_void,
    value: *mut DrInterfaceCarrierV1,
    status: *mut DrIoStatusV1,
) -> u8 {
    *status = DrIoStatusV1::default();
    let set = set.cast::<AdapterSetOwner>();
    if set.is_null()
        || key.is_null()
        || value.is_null()
        || (*set).active
        || !(*value).object.is_null()
        || !(*value).vtable.is_null()
    {
        return (*status).fail(Failure::invalid_input());
    }
    let (key, index) = match AdapterSetOwner::member(set, key) {
        Ok((key, Some(index))) => (key, index),
        Ok((_, None)) => return (*status).fail(Failure::invalid_input()),
        Err(failure) => return (*status).fail(failure),
    };
    if let Err(failure) = AdapterSetOwner::native(set).remove(key) {
        return (*status).fail(failure);
    }
    *value = (*set).retained.remove(index).unwrap().value.into_carrier();
    0
}

/// Consumes the existing opaque TransferPlan on success and failure. All plan
/// checks and group allocation precede I/O. Frames are borrowed for this call,
/// never captured in registration or group metadata after it returns.
/// # Safety
/// Set is null or live exclusive adapter storage. Plan is null or an owned
/// existing plan; its buffer loans outlive successful group execution. Budget
/// is null for rejection or live for this call; its frozen value is copied inline.
/// Frame is null or the current live call frame. Error is a disjoint EMPTY carrier for
/// the original checked source-getter error. Status is disjoint writable storage.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_adapter_plan_begin(
    frame: *const DrStackFrameV2,
    set: *mut c_void,
    plan: *mut c_void,
    budget: *const DrWaitBudgetV1,
    error: *mut DrInterfaceCarrierV1,
    status: *mut DrIoStatusV1,
) -> *mut c_void {
    adapter_plan_begin(frame, set, plan, budget, error, false, status)
}

/// # Safety
/// Same consumed-plan and retained-loan contract as synchronous begin. Error is
/// a disjoint EMPTY carrier; a failed preparation moves its original checked
/// error into it, returns null and leaves native status empty. A platform setup
/// error leaves the carrier empty and fills status. No failed setup submits I/O.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_pending_transfer_adapter_plan_begin(
    frame: *const DrStackFrameV2,
    set: *mut c_void,
    plan: *mut c_void,
    budget: *const DrWaitBudgetV1,
    error: *mut DrInterfaceCarrierV1,
    status: *mut DrIoStatusV1,
) -> *mut c_void {
    adapter_plan_begin(frame, set, plan, budget, error, true, status)
}

unsafe fn adapter_plan_begin(
    frame: *const DrStackFrameV2,
    set: *mut c_void,
    plan: *mut c_void,
    budget: *const DrWaitBudgetV1,
    error: *mut DrInterfaceCarrierV1,
    pending: bool,
    status: *mut DrIoStatusV1,
) -> *mut c_void {
    *status = DrIoStatusV1::default();
    project_plan_begin(
        begin_adapter_plan(frame, set, plan, budget, error, pending),
        error,
        status,
    )
}

unsafe fn project_plan_begin(
    result: Result<*mut c_void, SetupFailure<AdapterTerminal>>,
    error: *mut DrInterfaceCarrierV1,
    status: *mut DrIoStatusV1,
) -> *mut c_void {
    match result {
        Ok(owner) => owner,
        Err(SetupFailure::Platform(failure)) => {
            (*status).fail(failure);
            ptr::null_mut()
        }
        Err(SetupFailure::Preparation(terminal)) => {
            match terminal {
                AdapterTerminal::CheckedError { .. } | AdapterTerminal::PlatformFailure(_) => {
                    terminal.project(&mut DrAdapterOutcomeV1::default(), error, &mut *status);
                }
                _ => crate::runtime_invariant_panic(),
            }
            ptr::null_mut()
        }
    }
}

// Keep an adapter's original owned error until the caller selects its transport.
pub(super) unsafe fn begin_adapter_plan(
    frame: *const DrStackFrameV2,
    set: *mut c_void,
    plan: *mut c_void,
    budget: *const DrWaitBudgetV1,
    error: *const DrInterfaceCarrierV1,
    pending: bool,
) -> Result<*mut c_void, SetupFailure<AdapterTerminal>> {
    if budget.is_null() {
        drop(plan::take_plan(plan));
        return Err(Failure::invalid_input().into());
    }
    begin_adapter_plan_with(
        frame,
        set,
        plan,
        error,
        pending,
        |native, buffers, selection| native.begin_selected(frame, buffers, &*budget, selection),
    )
}

pub(super) unsafe fn begin_adapter_plan_controlled(
    frame: *const DrStackFrameV2,
    set: *mut c_void,
    plan: *mut c_void,
    budget: &DrWaitBudgetV1,
    cancellation: *mut DrCancellationV1,
    error: *const DrInterfaceCarrierV1,
    pending: bool,
) -> Result<*mut c_void, SetupFailure<AdapterTerminal>> {
    begin_adapter_plan_with(
        frame,
        set,
        plan,
        error,
        pending,
        |native, buffers, selection| {
            native.begin_selected_controlled(frame, buffers, budget, selection, cancellation)
        },
    )
}

unsafe fn begin_adapter_plan_with(
    frame: *const DrStackFrameV2,
    set: *mut c_void,
    plan: *mut c_void,
    error: *const DrInterfaceCarrierV1,
    pending: bool,
    begin: impl FnOnce(
        AdapterSetMode<'static>,
        BufferList<'static, AdapterTerminal>,
        FixedSlice<usize>,
    ) -> Result<AdapterGroupMode<'static, 'static>, SetupFailure<AdapterTerminal>>,
) -> Result<*mut c_void, SetupFailure<AdapterTerminal>> {
    let Some(plan) = plan::take_plan(plan) else {
        return Err(Failure::invalid_input().into());
    };
    let set = set.cast::<AdapterSetOwner>();
    if set.is_null() || (*set).active {
        return Err(Failure::invalid_input().into());
    }
    if pending != (*set).pending
        || error.is_null()
        || !(*error).object.is_null()
        || !(*error).vtable.is_null()
    {
        return Err(Failure::invalid_input().into());
    }
    let prepared = plan
        .prepare_as::<AdapterTerminal>((*set).identity, |key| {
            AdapterSetOwner::native(set).index_of(key)
        })
        .map_err(|(failure, _)| SetupFailure::Platform(failure))?;
    let owner = allocate_owner::<AdapterGroupOwner>(frame);
    (*set).active = true;
    let begun = begin(
        AdapterSetOwner::native(set),
        prepared.buffers,
        prepared.selection,
    )
    .and_then(|mut group| {
        // The plan's watches are armed once its transfers have begun. A wait
        // that cannot include a watched source ends the group again.
        match group.watch(prepared.watches.as_slice()) {
            Ok(()) => Ok(group),
            Err(failure) => {
                group.drop_in_frame(frame);
                Err(failure.into())
            }
        }
    });
    match begun {
        Ok(group) => {
            owner.write(AdapterGroupOwner {
                group: ManuallyDrop::new(group),
                set,
                identity: super::super::plan::GroupIdentity::with_watches(
                    prepared.identity,
                    prepared.watch_positions,
                ),
            });
            Ok(owner.cast())
        }
        Err(failure) => {
            (*set).active = false;
            crate::dr_v1_class_free(owner.cast());
            Err(failure)
        }
    }
}

/// Returns 0 explicit outcome, 1 original checked adapter error, 3 platform
/// failure, 4 not submitted, 5 no completion, 7 wait failure before taking a
/// queued completion, or 9 a completed watch, whose key is the observed watch
/// key. Tag 7 has no transfer identity or owned adapter error.
/// Wait/control facts remain in
/// the existing completion-status record; no error is reconstructed from them.
/// # Safety
/// Owner is live exclusive adapter-group storage. Frame is current/null. Key,
/// index, outcome and control are disjoint writable records. Error is a live
/// EMPTY existing interface carrier, disjoint from all other storage; an owned
/// error moves to it only on return 1 and is then caller-owned.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_adapter_group_next(
    frame: *const DrStackFrameV2,
    owner: *mut c_void,
    key: *mut u64,
    index: *mut usize,
    outcome: *mut DrAdapterOutcomeV1,
    error: *mut DrInterfaceCarrierV1,
    control: *mut c_void,
) -> u8 {
    observe_adapter_group(
        frame,
        owner,
        None,
        AdapterObservationOutputs {
            key,
            index,
            outcome,
            error,
            control,
        },
    )
}

/// Observes complete adapters with a call-owned budget. Tag 5 means exhausted;
/// tag 6 means observation timeout, including an immediate poll with no result.
/// Tag 7 reports a wait failure once without taking the queued transfer result.
/// # Safety
/// Owner, frame and outputs obey adapter_group_next's requirements. Budget is
/// initialized, live for this call and disjoint from outputs; never retained.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_adapter_group_observe(
    frame: *const DrStackFrameV2,
    owner: *mut c_void,
    budget: *const DrWaitBudgetV1,
    key: *mut u64,
    index: *mut usize,
    outcome: *mut DrAdapterOutcomeV1,
    error: *mut DrInterfaceCarrierV1,
    control: *mut c_void,
) -> u8 {
    observe_adapter_group(
        frame,
        owner,
        Some(*budget),
        AdapterObservationOutputs {
            key,
            index,
            outcome,
            error,
            control,
        },
    )
}

/// Borrowed output locations, not another error or completion owner.
struct AdapterObservationOutputs {
    key: *mut u64,
    index: *mut usize,
    outcome: *mut DrAdapterOutcomeV1,
    error: *mut DrInterfaceCarrierV1,
    control: *mut c_void,
}

/// Shared projection for both indefinite and budgeted observation.
/// # Safety
/// The same owner, frame, empty error carrier and disjoint output requirements
/// as adapter_group_next apply. A budget is copied, never retained.
unsafe fn observe_adapter_group(
    frame: *const DrStackFrameV2,
    owner: *mut c_void,
    budget: Option<DrWaitBudgetV1>,
    outputs: AdapterObservationOutputs,
) -> u8 {
    let AdapterObservationOutputs {
        key,
        index,
        outcome,
        error,
        control,
    } = outputs;
    let control = &mut *control.cast::<DrCompletionStatusV1>();
    *control = DrCompletionStatusV1::default();
    *key = 0;
    *index = 0;
    *outcome = DrAdapterOutcomeV1::default();
    if !(*error).object.is_null() || !(*error).vtable.is_null() {
        return control.transfer.fail(Failure::invalid_input());
    }
    let owner = &mut *owner.cast::<AdapterGroupOwner>();
    owner.identity.clear();
    let group = &mut owner.group;
    let observation = group.observe(frame, budget.unwrap_or_else(DrWaitBudgetV1::indefinite));
    let completion = match observation {
        crate::transfer_group::Observation::Completion(completion) => completion,
        crate::transfer_group::Observation::NoCompletion => return 5,
        crate::transfer_group::Observation::TimedOut => return 6,
        crate::transfer_group::Observation::WaitFailed(failure) => {
            return control.project_wait_failure(failure)
        }
        crate::transfer_group::Observation::Watch(watch) => {
            *index = owner.identity.observe_watch(watch);
            return 9;
        }
    };
    *key = completion.key;
    *index = completion.index;
    let (reading, _) = group.request_info(completion.index).unwrap();
    // Publish identity before transferring the owned error to the caller.
    owner.identity.observe(completion.index, reading);
    let super::super::Awaited {
        transfer,
        stop,
        cancellation_failure,
    } = completion.outcome.after_wait_failure_observation();
    control.project(
        super::super::Awaited {
            transfer: None,
            stop,
            cancellation_failure,
        },
        reading,
    );
    match transfer {
        None => 4,
        Some(terminal) => terminal.project(&mut *outcome, error, &mut control.transfer),
    }
}

/// Returns 0 with the last fetched operation key for the requested kind,
/// or 1 when absent (including before the first fetch or after a fetch timeout).
/// This observes metadata only and never takes an error, outcome or region loan.
/// # Safety
/// Owner is live shared group storage. Key is disjoint aligned writable
/// OperationKey storage; kind is 0 for writes, 1 for reads or 2 for watches.
/// A rejected fetch with a nonempty error carrier does
/// not change this record; the caller must first release that owned error.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_adapter_group_observed(
    owner: *const c_void,
    reading: u8,
    key: *mut c_void,
) -> u8 {
    project_observed_operation(&(*owner.cast::<AdapterGroupOwner>()).identity, reading, key)
}

pub(super) unsafe fn clear_observed(owner: *mut c_void) {
    (*owner.cast::<AdapterGroupOwner>()).identity.clear();
}

/// Returns 0 with a completed read prefix (including a present empty prefix),
/// 1 while pending or without known progress, or 3 for an invalid operation key.
/// Native, synchronous and pending adapters share key and availability rules.
/// # Safety
/// Owner is live shared storage; key is aligned live OperationKey storage.
/// Outputs are disjoint writable storage, disjoint from inputs and the region.
/// The returned readonly region expires before the next group mutation/drop.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_adapter_group_read_region(
    owner: *const c_void,
    key: *const c_void,
    data: *mut *const u8,
    length: *mut usize,
    status: *mut DrIoStatusV1,
) -> u8 {
    let owner = &*owner.cast::<AdapterGroupOwner>();
    let key = *key.cast::<super::super::plan::OperationKey>();
    let region = owner
        .identity
        .operation_position(key, true, |index| owner.group.request_info(index))
        .map(|index| owner.group.completed(index));
    project_completed_region(region, data, length, status)
}

/// Resubmits the original directional operation without changing its identity.
/// Invalid/foreign keys are rejected before mutation or adapter invocation.
/// # Safety
/// Owner is live exclusive storage with no borrowed completion/region view.
/// Key is aligned live OperationKey storage, disjoint from owner and status.
/// Reading is 0 for writes or 1 for reads; status is disjoint writable storage.
/// Frame is current/null and is not retained.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_adapter_group_submit_key(
    frame: *const DrStackFrameV2,
    owner: *mut c_void,
    key: *const c_void,
    reading: u8,
    offset: usize,
    length: usize,
    status: *mut DrIoStatusV1,
) -> u8 {
    *status = DrIoStatusV1::default();
    if reading > 1 {
        return (*status).fail(Failure::invalid_input());
    }
    let owner = &mut *owner.cast::<AdapterGroupOwner>();
    let key = *key.cast::<super::super::plan::OperationKey>();
    let result = owner
        .identity
        .operation_position(key, reading != 0, |index| owner.group.request_info(index))
        .and_then(|index| owner.group.submit(frame, index, offset, length));
    (*status).complete(result.map(|()| 0))
}

/// Arms a watch again after its completion was fetched. A key that is not a
/// watch of this group, a watch that is still armed or whose completion has
/// not been fetched, and a stopped group are rejected without any change.
/// # Safety
/// Owner is live exclusive storage with no borrowed completion/region view.
/// Key is aligned live OperationKey storage, disjoint from owner and status.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_adapter_group_rearm_watch(
    owner: *mut c_void,
    key: *const c_void,
    status: *mut DrIoStatusV1,
) -> u8 {
    *status = DrIoStatusV1::default();
    let owner = &mut *owner.cast::<AdapterGroupOwner>();
    let key = *key.cast::<super::super::plan::OperationKey>();
    let result = owner
        .identity
        .watch_index(key)
        .and_then(|index| owner.group.rearm_watch(index));
    (*status).complete(result.map(|()| 0))
}

/// # Safety
/// Owner is live exclusive storage with no borrowed completion/region view.
/// Frame is current/null. Status is disjoint writable storage.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_adapter_group_submit(
    frame: *const DrStackFrameV2,
    owner: *mut c_void,
    index: usize,
    offset: usize,
    length: usize,
    status: *mut DrIoStatusV1,
) -> u8 {
    *status = DrIoStatusV1::default();
    (*status).complete(
        (*owner.cast::<AdapterGroupOwner>())
            .group
            .submit(frame, index, offset, length)
            .map(|()| 0),
    )
}

/// # Safety
/// Owner is live exclusive storage. Outputs are disjoint writable locations.
/// The returned group-borrowed view expires before the next mutation/drop.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_adapter_group_region(
    owner: *mut c_void,
    index: usize,
    writable: u8,
    data: *mut *mut u8,
    length: *mut usize,
) -> u8 {
    *data = ptr::null_mut();
    *length = 0;
    let group = &mut (*owner.cast::<AdapterGroupOwner>()).group;
    if writable == 1 {
        if let Some(bytes) = group.read_storage_mut(index) {
            *data = bytes.as_mut_ptr();
            *length = bytes.len();
            return 0;
        }
    } else if writable == 0 {
        if let Some(bytes) = group.completed(index) {
            *data = bytes.as_ptr().cast_mut();
            *length = bytes.len();
            return 0;
        }
    }
    1
}

/// # Safety
/// Owner is live exclusive storage with no borrowed region/completion view.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_adapter_group_stop(owner: *mut c_void) -> u8 {
    match (*owner.cast::<AdapterGroupOwner>()).group.stop() {
        None => 0,
        Some(Stop::Cancellation) => 1,
        Some(Stop::BudgetExhausted) => 2,
        Some(Stop::Failure(_, _)) => 3,
    }
}

/// # Safety
/// Frame is null or live for cleanup, never retained. Owner is null or live
/// exclusive owned group storage, dropped exactly once.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_adapter_group_drop(
    frame: *const DrStackFrameV2,
    owner: *mut c_void,
) {
    if !owner.is_null() {
        (*owner.cast::<AdapterGroupOwner>()).clear_in_frame(frame);
        crate::dr_v1_class_free(owner.cast());
    }
}

/// # Safety
/// Frame is null or live for this call, never retained. Owner is null or live
/// exclusive owned adapter-set storage with no group,
/// dropped exactly once. Native metadata retires before retained complete
/// values drop; externally borrowed fixture values remain caller-owned.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_adapter_set_drop(
    frame: *const DrStackFrameV2,
    owner: *mut c_void,
) {
    if !owner.is_null() {
        if (*owner.cast::<AdapterSetOwner>()).active {
            crate::runtime_invariant_panic();
        }
        if (*owner.cast::<AdapterSetOwner>()).pending {
            drop_set_storage::<PendingSet>(frame, owner);
        } else {
            drop_set_storage::<AdapterSet>(frame, owner);
        }
        crate::dr_v1_class_free(owner.cast());
    }
}

unsafe fn drop_set_storage<T>(frame: *const DrStackFrameV2, owner: *mut c_void) {
    let storage = &mut *owner.cast::<AdapterSetStorage<T>>();
    storage.clear_in_frame(frame);
    // Native state is already retired; do not invoke its Rust Drop a second time.
    ptr::drop_in_place(&mut storage.owner);
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod owned_tests;

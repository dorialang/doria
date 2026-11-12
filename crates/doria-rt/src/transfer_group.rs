//! One exclusively borrowing group retains every caller buffer while requests
//! complete and are reused. Configuration may grow sparse pinned request owners;
//! submission allocates neither request records nor borrowed-buffer descriptors.

use crate::platform_io::Failure;
use core::task::Poll;

pub(crate) mod adapter;
mod observation;
mod outcome;
pub(crate) mod pending;
mod plan;
mod resubmission;
pub(crate) mod setup;
pub(crate) mod source;
mod storage;
#[cfg(test)]
mod tests;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
pub(crate) mod transport;
mod watch;

pub(crate) use observation::{Observation, ObservationBudget, WaitTimeout};
pub(crate) use outcome::DrCompletionStatusV1;
pub(crate) use plan::{PlanKey, TransferPlan};
pub(crate) use storage::{FixedSlice, StableSlots};
pub(crate) use watch::WatchSource;

pub(crate) enum Buffer<'a> {
    Read(&'a mut [u8]),
    Write(&'a [u8]),
}

impl Buffer<'_> {
    fn region(&self) -> (usize, usize, bool) {
        match self {
            Self::Read(bytes) => (bytes.as_ptr() as usize, bytes.len(), true),
            Self::Write(bytes) => (bytes.as_ptr() as usize, bytes.len(), false),
        }
    }

    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Read(bytes) => bytes.len(),
            Self::Write(bytes) => bytes.len(),
        }
    }
}

/// Both address ranges have been checked for overflow before this comparison.
fn regions_conflict(a: (usize, usize, bool), b: (usize, usize, bool)) -> bool {
    a.1 != 0 && b.1 != 0 && (a.2 || b.2) && a.0 < b.0 + b.1 && b.0 < a.0 + a.1
}

/// Borrowed physical access identity, classified by the owning transport for
/// the selected operation. Preparation-capable drivers resolve it after each
/// successful preparation. Duplex directions are independent; a shared file
/// cursor is not.
#[derive(Clone, Copy)]
pub(crate) struct TransferResource {
    pub(crate) identity: usize,
    pub(crate) shared_cursor: bool,
}

impl TransferResource {
    fn competes(self, reading: bool, other: Self, other_reading: bool) -> bool {
        self.identity == other.identity
            && (self.shared_cursor || other.shared_cursor || reading == other_reading)
    }
}

pub(crate) enum Submission {
    Complete,
    Pending,
    Failed(Failure),
}

pub(crate) enum Query<T = Result<usize, Failure>> {
    Terminal(T),
    Pending,
    PendingFailure(doria_diagnostic_catalogue::IoMessageOperation, Failure),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Wake {
    Completion,
    Cancellation,
    Timeout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stop {
    Cancellation,
    BudgetExhausted,
    Failure(doria_diagnostic_catalogue::IoMessageOperation, Failure),
}

/// Keeps both the terminal transfer and the reason waiting stopped. A successful
/// transfer racing cancellation is not replaced with invented zero progress.
/// `None` means no actual transfer outcome exists, not unknown error progress.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Awaited<T = Result<usize, Failure>> {
    pub(crate) transfer: Option<T>,
    pub(crate) stop: Option<Stop>,
    pub(crate) cancellation_failure:
        Option<(doria_diagnostic_catalogue::IoMessageOperation, Failure)>,
}

#[cfg(any(windows, test))]
impl<T> Awaited<T> {
    pub(crate) fn complete(result: T) -> Self {
        Self {
            transfer: Some(result),
            stop: None,
            cancellation_failure: None,
        }
    }
}

/// An owned terminal fact, not a second submission or completion engine.
pub(crate) trait TerminalOutcome: Sized {
    fn success(count: usize, reading: bool) -> Self;
    fn failure(failure: Failure) -> Self;
    fn bounded(self, bound: usize) -> Self;
    fn progress(&self) -> Option<usize>;
    /// A successful preparation's checked cleanup error takes precedence over
    /// infrastructure rejection, preserving its original owned carrier. Numeric
    /// native outcomes do not introduce a checked language error of their own.
    fn is_checked_error(&self) -> bool {
        false
    }
    fn into_transfer(self) -> Option<Self> {
        Some(self)
    }

    /// # Safety
    /// Frame is null or live throughout destruction and is never retained.
    unsafe fn drop_in_frame(self, _: *const crate::DrStackFrameV2) {
        drop(self);
    }
}

impl TerminalOutcome for Result<usize, Failure> {
    fn success(count: usize, _: bool) -> Self {
        Ok(count)
    }

    fn failure(failure: Failure) -> Self {
        Err(failure)
    }

    fn bounded(self, bound: usize) -> Self {
        match self {
            Ok(count) if count > bound => Err(Failure::other()),
            Err(failure)
                if failure
                    .transferred_byte_count
                    .is_some_and(|count| count > bound) =>
            {
                Err(failure.with_transferred_byte_count(None))
            }
            other => other,
        }
    }

    fn progress(&self) -> Option<usize> {
        match self {
            Ok(count) => Some(*count),
            Err(failure) => failure.transferred_byte_count,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Wait<T> {
    pub(crate) timeout: T,
    pub(crate) immediate: bool,
}

pub(crate) trait Control<T = u32> {
    /// Reuses one already-established budget. Immediate is a semantic fact,
    /// independent of the native representation or clamping of the timeout.
    fn next_wait(&mut self) -> Result<Result<Wait<T>, Stop>, Failure>;
    fn cancellation_signal(&self) -> Option<usize>;
}

pub(crate) struct RequestSlot<R> {
    // Registration is borrowed metadata. (registration, ordinal) identifies
    // this cached operation, independently of the registration's public key.
    registration: usize,
    ordinal: usize,
    key: u64,
    request: R,
}

pub(crate) enum RequestSlots<R> {
    Fixed {
        entries: FixedSlice<RequestSlot<R>>,
        extra: StableSlots<RequestSlot<R>>,
    },
    Dynamic(StableSlots<RequestSlot<R>>),
}

impl<R> RequestSlots<R> {
    #[cfg(any(windows, test))]
    pub(crate) fn new(
        keys: &[u64],
        mut initialize: impl FnMut(usize) -> R,
    ) -> Result<Self, Failure> {
        Self::new_fallible(keys, |index| Ok(initialize(index)))
    }

    pub(crate) fn new_fallible(
        keys: &[u64],
        mut initialize: impl FnMut(usize) -> Result<R, Failure>,
    ) -> Result<Self, Failure> {
        let mut entries = FixedSlice::new(keys.len())?;
        for (index, key) in keys.iter().enumerate() {
            entries.push(RequestSlot {
                registration: index,
                ordinal: 0,
                key: *key,
                request: initialize(index)?,
            })?;
        }
        Ok(Self::Fixed {
            entries,
            extra: StableSlots::new()?,
        })
    }

    pub(crate) fn dynamic() -> Result<Self, Failure> {
        Ok(Self::Dynamic(StableSlots::new()?))
    }

    #[cfg(test)]
    pub(crate) fn add(&mut self, key: u64, request: R) -> Result<usize, Failure> {
        if self.index_of(key).is_some() {
            return Err(Failure::invalid_input());
        }
        match self {
            Self::Fixed { .. } => Err(Failure::invalid_input()),
            Self::Dynamic(entries) => {
                let registration = entries.vacant_index();
                self.allocate_operation(registration, 0, key, |_| Ok(request))
            }
        }
    }

    /// Admission creates one reusable default operation, not an I/O owner.
    /// Registration membership and key validity belong to the native set.
    pub(crate) fn add_registration(
        &mut self,
        registration: usize,
        key: u64,
        initialize: impl FnOnce(usize) -> Result<R, Failure>,
    ) -> Result<usize, Failure> {
        if self.index_of(key).is_some() || self.operation_of(registration, 0).is_some() {
            return Err(Failure::invalid_input());
        }
        self.allocate_operation(registration, 0, key, initialize)
    }

    /// Resolve every plan operation to its own pinned cached request. Repeated
    /// registration selections are valid; repeated actual request slots are not.
    /// All growth occurs before preparation or native submission.
    pub(crate) fn configure_operations(
        &mut self,
        mut registrations: FixedSlice<usize>,
        mut initialize: impl FnMut(usize, usize) -> Result<R, Failure>,
    ) -> Result<FixedSlice<usize>, Failure> {
        if registrations
            .as_slice()
            .iter()
            .any(|registration| self.operation_of(*registration, 0).is_none())
        {
            return Err(Failure::invalid_input());
        }
        // Reuse the plan's descriptor allocation. Earlier entries are now
        // operation indices; their registration metadata supplies the ordinal.
        for position in 0..registrations.len() {
            let registration = registrations.as_slice()[position];
            let ordinal = registrations.as_slice()[..position]
                .iter()
                .filter(|prior| self.registration(**prior) == registration)
                .count();
            let operation = match self.operation_of(registration, ordinal) {
                Some(operation) => operation,
                None => {
                    let primary = self.operation_of(registration, 0).unwrap();
                    let key = self.key(primary);
                    self.allocate_operation(registration, ordinal, key, |operation| {
                        initialize(registration, operation)
                    })?
                }
            };
            registrations.as_mut_slice()[position] = operation;
        }
        Ok(registrations)
    }

    pub(crate) fn operation_of(&self, registration: usize, ordinal: usize) -> Option<usize> {
        (0..self.len()).find(|index| {
            self.contains(*index)
                && self.registration(*index) == registration
                && self.ordinal(*index) == ordinal
        })
    }

    pub(crate) fn primary_registrations(&self) -> Result<FixedSlice<usize>, Failure> {
        let count = (0..self.len())
            .filter(|index| self.contains(*index) && self.ordinal(*index) == 0)
            .count();
        let mut registrations = FixedSlice::new(count)?;
        for index in 0..self.len() {
            if self.contains(index) && self.ordinal(index) == 0 {
                registrations.push(self.registration(index))?;
            }
        }
        Ok(registrations)
    }

    pub(crate) fn registration(&self, index: usize) -> usize {
        unsafe { core::ptr::addr_of!((*self.entry_raw(index)).registration).read() }
    }

    fn ordinal(&self, index: usize) -> usize {
        unsafe { core::ptr::addr_of!((*self.entry_raw(index)).ordinal).read() }
    }

    fn allocate_operation(
        &mut self,
        registration: usize,
        ordinal: usize,
        key: u64,
        initialize: impl FnOnce(usize) -> Result<R, Failure>,
    ) -> Result<usize, Failure> {
        let index = match self {
            Self::Fixed { entries, extra } => entries
                .len()
                .checked_add(extra.vacant_index())
                .ok_or_else(Failure::invalid_input)?,
            Self::Dynamic(entries) => entries.vacant_index(),
        };
        let request = initialize(index)?;
        let entry = RequestSlot {
            registration,
            ordinal,
            key,
            request,
        };
        match self {
            Self::Fixed { entries, extra } => extra.insert_at(index - entries.len(), entry)?,
            Self::Dynamic(entries) => entries.insert_at(index, entry)?,
        }
        Ok(index)
    }

    /// The entire registration is inactive and every cached operation retired.
    pub(crate) fn remove_registration(&mut self, registration: usize) -> Result<(), Failure> {
        if matches!(self, Self::Fixed { .. }) || self.operation_of(registration, 0).is_none() {
            return Err(Failure::invalid_input());
        }
        for index in 0..self.len() {
            if self.contains(index) && self.registration(index) == registration {
                self.remove(index)?;
            }
        }
        Ok(())
    }

    /// The caller must first retire every native use of the removed request.
    pub(crate) fn remove(&mut self, index: usize) -> Result<(), Failure> {
        match self {
            Self::Fixed { .. } => Err(Failure::invalid_input()),
            Self::Dynamic(entries) => entries
                .remove(index)
                .map(drop)
                .ok_or_else(Failure::invalid_input),
        }
    }

    pub(crate) fn contains(&self, index: usize) -> bool {
        match self {
            Self::Fixed { entries, extra } => {
                index < entries.len() || extra.contains(index - entries.len())
            }
            Self::Dynamic(entries) => entries.contains(index),
        }
    }

    pub(crate) fn occupied_len(&self) -> usize {
        match self {
            Self::Fixed { entries, extra } => entries.len() + extra.occupied_len(),
            Self::Dynamic(entries) => entries.occupied_len(),
        }
    }

    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Fixed { entries, extra } => entries.len() + extra.len(),
            Self::Dynamic(entries) => entries.len(),
        }
    }

    pub(crate) fn key(&self, index: usize) -> u64 {
        // Read only key metadata, never borrow the kernel-owned request field.
        unsafe { core::ptr::addr_of!((*self.entry_raw(index)).key).read() }
    }

    pub(crate) fn index_of(&self, key: u64) -> Option<usize> {
        (0..self.len()).find(|index| {
            self.contains(*index) && self.ordinal(*index) == 0 && self.key(*index) == key
        })
    }

    pub(crate) fn raw(&self, index: usize) -> *mut R {
        // No &mut reference to a kernel-owned record is formed here. Its address
        // is stable across moves of this storage owner and all resubmissions.
        unsafe { core::ptr::addr_of_mut!((*self.entry_raw(index)).request) }
    }

    fn entry_raw(&self, index: usize) -> *mut RequestSlot<R> {
        match self {
            Self::Fixed { entries, .. } if index < entries.len() => entries.raw(index),
            Self::Fixed { entries, extra } => extra
                .raw(index - entries.len())
                .expect("occupied extra operation slot"),
            Self::Dynamic(entries) => entries.raw(index).expect("occupied request slot"),
        }
    }
}

struct BufferState<'b, T> {
    buffer: Buffer<'b>,
    offset: usize,
    length: usize,
    active: bool,
    preparing: bool,
    prepared: bool,
    preparation_failure: Option<Failure>,
    submitted: bool,
    retiring_submission: bool,
    queued: Option<usize>,
    resource: Option<TransferResource>,
    ready: Option<Awaited<T>>,
    progress: Option<usize>,
    cancellation_failure: Option<(doria_diagnostic_catalogue::IoMessageOperation, Failure)>,
}

pub(crate) struct BufferList<'b, T = Result<usize, Failure>> {
    entries: FixedSlice<BufferState<'b, T>>,
}

impl<'b, T> BufferList<'b, T> {
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn request_info(&self, index: usize) -> Option<(bool, usize)> {
        let entry = self.entries.as_slice().get(index)?;
        Some((matches!(entry.buffer, Buffer::Read(_)), entry.length))
    }

    pub(crate) fn from_iter(
        mut buffers: impl ExactSizeIterator<Item = Buffer<'b>>,
    ) -> Result<Self, Failure> {
        let count = buffers.len();
        let mut list = Self::with_capacity(count)?;
        for buffer in &mut buffers {
            list.push(buffer)?;
        }
        if list.entries.len() != count {
            return Err(Failure::invalid_input());
        }
        Ok(list)
    }

    fn with_capacity(count: usize) -> Result<Self, Failure> {
        Ok(Self {
            entries: FixedSlice::new(count)?,
        })
    }

    fn push(&mut self, buffer: Buffer<'b>) -> Result<(), Failure> {
        let length = buffer.len();
        self.entries.push(BufferState {
            buffer,
            offset: 0,
            length,
            active: false,
            preparing: false,
            prepared: false,
            preparation_failure: None,
            submitted: false,
            retiring_submission: false,
            queued: None,
            resource: None,
            ready: None,
            progress: None,
            cancellation_failure: None,
        })
    }
}

/// # Safety
/// Submitted storage remains borrowed until a terminal query or drain. Terminal
/// means neither the kernel nor a completion packet/callback retains the record
/// or buffer. Submission failure retains neither. The temporary Buffer argument
/// itself may not be retained; only its underlying bytes remain borrowed. Cancel
/// and drain never unwind. Drain preserves other slots' completions and never
/// returns pending, even if cancel fails.
/// Every handle and completion resource stays live until all active slots are
/// terminal. A driver may borrow those resources from complete stream owners
/// retained by the compiler/set projection; it is not another close owner. All
/// request accesses use the configured stable slot address.
pub(crate) unsafe trait GroupDriver<T = Result<usize, Failure>> {
    type Request;
    type Timeout: WaitTimeout;
    const MAXIMUM_TRANSFER_BYTES: usize;
    /// Unix readiness retries and synchronous adapter queries can transfer
    /// bytes and must recheck the original stop. Windows packet queries only
    /// observe retirement. Drivers declare their actual query behavior here.
    const QUERY_CAN_TRANSFER: bool = false;
    /// Synchronous adapters may change their source between call-scoped
    /// attempts. They have no escaped kernel work: refresh before a retry,
    /// then requeue a changed resource through this core's cursor arbitration.
    /// Native pending drivers keep their in-flight selection until retirement.
    const QUERY_REFRESHES_SOURCE: bool = false;
    /// A prepare callback may execute user code before submission. Recheck the
    /// original stop before each preparation without suppressing a terminal
    /// native completion that won cancellation during a later query.
    const PREFLIGHT_EACH_SUBMISSION: bool = false;
    /// Configuration-only user preparation runs for all selected requests
    /// before any native submission. Fixed native drivers keep their original
    /// one-stage path; synchronous bound adapters use this phase to select the
    /// current source without invoking a transfer callback.
    const PREPARE_BEFORE_SUBMISSION: bool = false;
    /// Complete adapters retain observable behavior for empty regions, too.
    const EMPTY_MUST_DISPATCH: bool = false;

    fn set_call_frame(&mut self, _: *const crate::DrStackFrameV2) {}

    fn call_frame(&self) -> *const crate::DrStackFrameV2 {
        core::ptr::null()
    }

    /// Returns the actual stream/cursor identity, not a registration key or a
    /// request address. Fixed-source drivers classify once during group setup;
    /// preparation-capable drivers classify the already-selected snapshot after
    /// each successful preparation, including resubmission. This must not rerun
    /// the getter, mutate the source or query a registration's latest route.
    /// Reads only borrowed owner metadata, even if a sibling request is live.
    unsafe fn resource(
        &self,
        index: usize,
        request: *mut Self::Request,
    ) -> Result<TransferResource, Failure>;

    /// Acquire retained physical-device access before any preparation or I/O.
    /// Unlike an individual request, this claim survives completion/resubmit
    /// and is released only after the whole group has retired its storage.
    unsafe fn acquire_request(&mut self, _: usize, _: *mut Self::Request) -> Result<(), Failure> {
        Ok(())
    }
    unsafe fn release_request(&mut self, _: usize, _: *mut Self::Request) {}

    /// Source-dependent setup runs under the installed group guard, before any
    /// adapter preparation or native submission. It may resolve a checked
    /// getter and retain route metadata, but never selects a buffer or performs
    /// I/O. Failed setup releases its own staging; release_request retires the
    /// earlier setup metadata even when no preparation ever ran.
    unsafe fn setup_request(&mut self, _: usize, _: *mut Self::Request) -> Result<(), T> {
        Ok(())
    }

    /// A hosted controller may hand an ordinary getter call back to its host.
    /// Pending retains only stable request metadata, never this driver's home,
    /// a frame or output slot. Repeated polls consume the SAME staged reply;
    /// they must not invoke the getter again. This is not an OS readiness poll.
    unsafe fn poll_setup_request(
        &mut self,
        index: usize,
        request: *mut Self::Request,
    ) -> Poll<Result<(), T>> {
        Poll::Ready(self.setup_request(index, request))
    }

    /// Called only before submission, after the previous request is terminal.
    /// Reset must not retain any buffer or request address, or unwind.
    unsafe fn reset_request(&mut self, index: usize, request: *mut Self::Request);
    /// Select storage/record an immediate result; never perform I/O. Prepared
    /// records retain only group-owned loans, and cancel/drain also retire them
    /// if setup stops before native submission. Err moves the original terminal
    /// failure out and releases that failed preparation's own staging; its slot
    /// is not active and receives no cancel/drain. Earlier successful slots
    /// still retire through the installed group cleanup guard.
    unsafe fn prepare_request(
        &mut self,
        _: usize,
        _: *mut Self::Request,
        _: &mut Buffer<'_>,
    ) -> Result<(), T> {
        Ok(())
    }

    /// Same preparation contract, with a bounded host handoff. Pending keeps
    /// the backing byte loan, not the temporary Buffer or driver/frame/output
    /// addresses. Ready owns the only success or original checked failure.
    /// An unfinished preparation must be abortable without invoking completion;
    /// a successful reply must be returned even if cancellation raced the host.
    unsafe fn poll_prepare_request(
        &mut self,
        index: usize,
        request: *mut Self::Request,
        buffer: &mut Buffer<'_>,
    ) -> Poll<Result<(), T>> {
        Poll::Ready(self.prepare_request(index, request, buffer))
    }

    /// Retire a yielded preparation that has not returned successfully. No
    /// native work exists and no completion is owed. Release all staged loans,
    /// consume no completed reply, and never unwind or retain call metadata.
    unsafe fn abort_preparation(&mut self, _: usize, _: *mut Self::Request) {}

    /// A host reply that already returned successfully owns completion even
    /// when stop races resumption. Cleanup must consume it instead of aborting
    /// the earlier, still-unreturned preparation handoff.
    unsafe fn preparation_reply_ready(&self, _: usize, _: *mut Self::Request) -> bool {
        false
    }
    unsafe fn submit(
        &mut self,
        index: usize,
        request: *mut Self::Request,
        buffer: &mut Buffer<'_>,
    ) -> Submission;
    /// Moves a terminal result already produced by submission, without I/O,
    /// another callback, or querying native work. Drivers whose Complete tag
    /// only permits a later query leave this absent.
    unsafe fn take_submitted_result(&mut self, _: usize, _: *mut Self::Request) -> Option<T> {
        None
    }
    /// Complete may owe one hosted retirement before submission returns. Pending
    /// retains the same record and loans; resumption never submits again or
    /// queries unrelated pending I/O. Absent preserves drivers' later-query path.
    unsafe fn poll_submitted_result(
        &mut self,
        index: usize,
        request: *mut Self::Request,
    ) -> Poll<Option<T>> {
        Poll::Ready(self.take_submitted_result(index, request))
    }
    /// Configuration-only source selection for a synchronous retry. On Err the
    /// driver has retired its advisory watch and prior selection, without I/O
    /// or another transfer callback. Only QUERY_REFRESHES_SOURCE drivers use it.
    unsafe fn prepare_query(&mut self, _: usize, _: *mut Self::Request) -> Result<(), T> {
        Ok(())
    }
    unsafe fn query(&mut self, index: usize, request: *mut Self::Request) -> Query<T>;
    /// Pending hands a completion call to the host, not an OS readiness wait.
    /// The same record, source pins and byte loans remain live. Resumption must
    /// consume the staged adapter reply without repeating native I/O/retirement.
    unsafe fn poll_query(&mut self, index: usize, request: *mut Self::Request) -> Poll<Query<T>> {
        Poll::Ready(self.query(index, request))
    }
    fn wait_any(
        &mut self,
        timeout: Self::Timeout,
        cancellation: Option<usize>,
    ) -> Result<Wake, Failure>;
    /// Makes `wait_any` also return, as a completion wake, while `handle` is
    /// readable or signaled. The group attaches the handle of a watched
    /// source only while a watch on it is armed, and each handle once. A
    /// driver whose wait cannot include another handle refuses.
    unsafe fn attach_wake(&mut self, _: crate::native_handle::NativeHandle) -> Result<(), Failure> {
        Err(Failure::unsupported())
    }
    /// Takes an attached handle out of the wait. It never fails the caller:
    /// a wait that could not let go of the handle must not be used again.
    unsafe fn detach_wake(&mut self, _: crate::native_handle::NativeHandle) {}
    unsafe fn cancel(
        &mut self,
        index: usize,
        request: *mut Self::Request,
    ) -> Result<(), (doria_diagnostic_catalogue::IoMessageOperation, Failure)>;
    /// Retires storage before returning. None means no actual transfer result,
    /// not a synthetic Interrupted error or zero-byte completion.
    unsafe fn drain(&mut self, index: usize, request: *mut Self::Request) -> Option<T>;
    /// The cancellation fanout has finished before this is called. Pending
    /// retains every loan until the host returns the same completion reply.
    /// Hosted owners must finish poll-based cleanup before dropping the group:
    /// the synchronous drain/Drop contract above never yields or unwinds.
    unsafe fn poll_drain(&mut self, index: usize, request: *mut Self::Request) -> Poll<Option<T>> {
        Poll::Ready(self.drain(index, request))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Completion<T = Result<usize, Failure>> {
    pub(crate) key: u64,
    pub(crate) index: usize,
    pub(crate) outcome: Awaited<T>,
}

/// Setup errors own the same terminal carrier as adapter completion. A checked
/// preparation error must leave begin directly, not be converted into a native
/// status or queued behind requests that must never have been submitted.
#[derive(Debug)]
pub(crate) enum SetupFailure<T> {
    Platform(Failure),
    Preparation(T),
}

impl<T> From<Failure> for SetupFailure<T> {
    fn from(failure: Failure) -> Self {
        Self::Platform(failure)
    }
}

impl<T> SetupFailure<T> {
    pub(crate) fn legacy(self) -> Failure {
        match self {
            Self::Platform(failure) => failure,
            // Preparation-capable drivers must use the typed setup entry.
            Self::Preparation(_) => unsafe { crate::runtime_invariant_panic() },
        }
    }
}

pub(crate) struct TransferGroup<
    's,
    'b,
    D: GroupDriver<T>,
    C: Control<D::Timeout>,
    T: TerminalOutcome = Result<usize, Failure>,
> {
    slots: &'s mut RequestSlots<D::Request>,
    // None retains the allocation-free full-set path. A plan owns the fixed
    // map from its order to stable operation slots, not registration owners.
    selection: Option<FixedSlice<usize>>,
    buffers: BufferList<'b, T>,
    driver: D,
    control: C,
    cursor: usize,
    stopped: Option<Stop>,
    stop_drain: Option<usize>,
    wait_failure_reported: bool,
    // None until a plan with watches arms them; most groups have none.
    watches: Option<FixedSlice<watch::Watch>>,
}

impl<'s, 'b, D: GroupDriver<T>, C: Control<D::Timeout>, T: TerminalOutcome>
    TransferGroup<'s, 'b, D, C, T>
{
    #[cfg(test)]
    pub(crate) fn begin(
        slots: &'s mut RequestSlots<D::Request>,
        buffers: BufferList<'b, T>,
        driver: D,
        control: C,
    ) -> Result<Self, Failure> {
        Self::begin_configured(slots, buffers, driver, control, None).map_err(SetupFailure::legacy)
    }

    #[cfg(any(target_os = "linux", target_os = "macos", test))]
    pub(crate) fn begin_selected(
        slots: &'s mut RequestSlots<D::Request>,
        buffers: BufferList<'b, T>,
        driver: D,
        control: C,
        selection: FixedSlice<usize>,
    ) -> Result<Self, Failure> {
        Self::begin_configured(slots, buffers, driver, control, Some(selection))
            .map_err(SetupFailure::legacy)
    }

    #[cfg(test)]
    pub(crate) fn begin_prepared(
        slots: &'s mut RequestSlots<D::Request>,
        buffers: BufferList<'b, T>,
        driver: D,
        control: C,
    ) -> Result<Self, SetupFailure<T>> {
        Self::begin_configured(slots, buffers, driver, control, None)
    }

    pub(crate) fn begin_selected_prepared(
        slots: &'s mut RequestSlots<D::Request>,
        buffers: BufferList<'b, T>,
        driver: D,
        control: C,
        selection: FixedSlice<usize>,
    ) -> Result<Self, SetupFailure<T>> {
        Self::begin_configured(slots, buffers, driver, control, Some(selection))
    }

    fn begin_configured(
        slots: &'s mut RequestSlots<D::Request>,
        buffers: BufferList<'b, T>,
        driver: D,
        control: C,
        selection: Option<FixedSlice<usize>>,
    ) -> Result<Self, SetupFailure<T>> {
        let frame = driver.call_frame();
        let setup = setup::GroupSetup::configure(slots, buffers, driver, control, selection)?;
        unsafe { setup.finish(frame) }
    }

    pub(crate) fn stop(&self) -> Option<Stop> {
        self.stopped
    }

    /// # Safety
    /// Frame is null or live throughout cleanup; no acquisition frame is used.
    pub(crate) unsafe fn drop_in_frame(mut self, frame: *const crate::DrStackFrameV2) {
        self.driver.set_call_frame(frame);
        drop(self);
    }

    /// # Safety
    /// Frame is null or live throughout the operation. The operation may not
    /// move or swap this group/driver; the guard clears its transient frame even
    /// during host unwinding. No frame is retained after the operation returns.
    pub(crate) unsafe fn with_call_frame<R>(
        &mut self,
        frame: *const crate::DrStackFrameV2,
        operation: impl FnOnce(&mut Self) -> R,
    ) -> R {
        struct Reset<D: GroupDriver<T>, T>(*mut D, core::marker::PhantomData<T>);
        impl<D: GroupDriver<T>, T> Drop for Reset<D, T> {
            fn drop(&mut self) {
                unsafe { (*self.0).set_call_frame(core::ptr::null()) };
            }
        }
        self.driver.set_call_frame(frame);
        let reset = Reset::<D, T>(&mut self.driver, core::marker::PhantomData);
        let result = operation(self);
        drop(reset);
        result
    }

    #[cfg(test)]
    pub(crate) fn next(&mut self) -> Option<Completion<T>> {
        loop {
            match self.observe(&mut observation::Indefinite) {
                Observation::Completion(completion) => return Some(completion),
                Observation::NoCompletion => return None,
                // Internal drain callers receive both facts in Awaited. Public
                // observation reports the wait failure separately, first.
                Observation::WaitFailed(_) => continue,
                Observation::Watch(_) | Observation::TimedOut => unsafe {
                    crate::runtime_invariant_panic()
                },
            }
        }
    }

    pub(crate) fn observe(
        &mut self,
        observation: &mut impl ObservationBudget<D::Timeout>,
    ) -> Observation<T> {
        let frame = self.driver.call_frame();
        let result = {
            let mut call = observation::ObservationCall::new(self, observation);
            unsafe { call.poll(frame) }
        };
        // The surrounding synchronous operation still owns this current frame.
        self.driver.set_call_frame(frame);
        match result {
            Poll::Ready(result) => result,
            // Hosted callers retain ObservationCall and resume it instead.
            Poll::Pending => unsafe { crate::runtime_invariant_panic() },
        }
    }

    pub(crate) fn completed(&self, index: usize) -> Option<&[u8]> {
        let entry = self.buffers.entries.as_slice().get(index)?;
        if entry.active {
            return None;
        }
        let count = entry.progress?;
        let end = entry.offset.checked_add(count)?;
        match &entry.buffer {
            Buffer::Read(bytes) => bytes.get(entry.offset..end),
            Buffer::Write(bytes) => bytes.get(entry.offset..end),
        }
    }

    pub(crate) fn request_info(&self, index: usize) -> Option<(bool, usize)> {
        self.buffers.request_info(index)
    }

    pub(crate) fn read_storage_mut(&mut self, index: usize) -> Option<&mut [u8]> {
        let entry = self.buffers.entries.as_mut_slice().get_mut(index)?;
        if entry.active {
            return None;
        }
        match &mut entry.buffer {
            Buffer::Read(bytes) => Some(bytes),
            Buffer::Write(_) => None,
        }
    }

    pub(crate) fn submit(
        &mut self,
        index: usize,
        offset: usize,
        length: usize,
    ) -> Result<(), Failure> {
        let frame = self.driver.call_frame();
        let result = {
            let mut call = resubmission::ResubmissionCall::new(self, index, offset, length)?;
            unsafe { call.poll(frame) }
        };
        self.driver.set_call_frame(frame);
        if result.is_pending() {
            // Hosted callers retain the same resubmission guard across handoffs.
            unsafe { crate::runtime_invariant_panic() };
        }
        Ok(())
    }

    fn preflight(&mut self) -> Option<Stop> {
        match self.control.next_wait() {
            Ok(Ok(_)) => None,
            Ok(Err(stop)) => Some(stop),
            Err(failure) => Some(Stop::Failure(
                doria_diagnostic_catalogue::IoMessageOperation::Wait,
                failure,
            )),
        }
    }

    fn enqueue(&mut self, index: usize) {
        debug_assert!(self.buffers.entries.as_slice()[index].resource.is_some());
        // Dense bounded ranks avoid both allocation and an ever-growing
        // sequence counter. Reuse joins behind already queued plan operations.
        let rank = self
            .buffers
            .entries
            .as_slice()
            .iter()
            .filter(|entry| entry.queued.is_some())
            .count();
        let entry = &mut self.buffers.entries.as_mut_slice()[index];
        entry.active = true;
        entry.queued = Some(rank);
    }

    /// A changed synchronous source has no kernel operation to cancel. Keep
    /// its prepared byte loan, but return the retry to the existing queue so a
    /// newly shared cursor cannot bypass another operation's selected resource.
    fn refresh_query_resource(&mut self, index: usize) -> bool {
        let (registered, raw) = self.request(index);
        let resource = match unsafe { self.driver.resource(registered, raw) } {
            Ok(resource) => resource,
            Err(failure) => {
                self.begin_stop(Stop::Failure(
                    doria_diagnostic_catalogue::IoMessageOperation::Configure,
                    failure,
                ));
                return true;
            }
        };
        let entry = &mut self.buffers.entries.as_mut_slice()[index];
        let old = entry.resource.expect("selected synchronous source");
        if old.identity == resource.identity && old.shared_cursor == resource.shared_cursor {
            return false;
        }
        entry.resource = Some(resource);
        if entry.queued.is_some() {
            // The operation already has its fair queue rank. Refresh its
            // eligibility without inserting it a second time or moving it.
            return true;
        }
        entry.submitted = false;
        self.enqueue(index);
        true
    }

    fn dequeue(&mut self, index: usize) {
        if let Some(rank) = self.buffers.entries.as_mut_slice()[index].queued.take() {
            for entry in self.buffers.entries.as_mut_slice() {
                if let Some(later) = &mut entry.queued {
                    if *later > rank {
                        *later -= 1;
                    }
                }
            }
        }
    }

    fn eligible(&self, index: usize) -> bool {
        let entry = &self.buffers.entries.as_slice()[index];
        let Some(rank) = entry.queued else {
            return false;
        };
        let resource = entry.resource.unwrap();
        let reading = matches!(entry.buffer, Buffer::Read(_));
        !self
            .buffers
            .entries
            .as_slice()
            .iter()
            .enumerate()
            .any(|(other_index, other)| {
                other_index != index
                    && (other.submitted || other.queued.is_some_and(|prior| prior < rank))
                    && resource.competes(
                        reading,
                        other.resource.unwrap(),
                        matches!(other.buffer, Buffer::Read(_)),
                    )
            })
    }

    fn poll_dispatch_queued(&mut self, check_stop: bool) -> Poll<()> {
        if self.stopped.is_some() {
            return self.poll_stop_all(self.stopped.unwrap());
        }
        for index in 0..self.len() {
            if self.buffers.entries.as_slice()[index].retiring_submission
                && self.poll_submission_retirement(index).is_pending()
            {
                return Poll::Pending;
            }
        }
        // Visit admission order, not physical slot order. Removal compacts the
        // remaining ranks, so even a failed submission advances its successor.
        let mut rank = 0;
        while let Some(index) = (0..self.len())
            .find(|index| self.buffers.entries.as_slice()[*index].queued == Some(rank))
        {
            if D::QUERY_REFRESHES_SOURCE {
                if check_stop && self.buffers.entries.as_slice()[index].length != 0 {
                    if let Some(stop) = self.preflight() {
                        self.begin_stop(stop);
                        break;
                    }
                }
                let (registered, raw) = self.request(index);
                if let Err(terminal) = unsafe { self.driver.prepare_query(registered, raw) } {
                    self.finish(index, Some(terminal), None);
                    continue;
                }
                self.refresh_query_resource(index);
                if self.stopped.is_some() {
                    break;
                }
            }
            if !self.eligible(index) {
                rank += 1;
                continue;
            }
            self.dequeue(index);
            if check_stop && self.buffers.entries.as_slice()[index].length != 0 {
                if let Some(stop) = self.preflight() {
                    self.begin_stop(stop);
                    break;
                }
            }
            if self.start(index).is_pending() {
                return Poll::Pending;
            }
            if self.stopped.is_some() {
                break;
            }
        }
        match self.stopped {
            Some(stop) => self.poll_stop_all(stop),
            None => Poll::Ready(()),
        }
    }

    fn start(&mut self, index: usize) -> Poll<()> {
        if D::PREPARE_BEFORE_SUBMISSION {
            if !self.buffers.entries.as_slice()[index].prepared {
                if let Err(error) = self.prepare(index) {
                    self.finish(index, Some(Self::preparation_terminal(error)), None);
                    return Poll::Ready(());
                }
            }
            if self.buffers.entries.as_slice()[index].length != 0 {
                if let Some(stop) = self.preflight() {
                    self.begin_stop(stop);
                    return Poll::Ready(());
                }
            }
        }
        let (registered, raw) = self.request(index);
        // Only an unsubmitted or fully retired terminal slot reaches this point.
        if !D::PREPARE_BEFORE_SUBMISSION {
            unsafe { self.driver.reset_request(registered, raw) };
        }
        let entry = &mut self.buffers.entries.as_mut_slice()[index];
        entry.active = true;
        entry.submitted = true;
        let end = entry.offset + entry.length;
        let mut selected = match &mut entry.buffer {
            Buffer::Read(bytes) => Buffer::Read(&mut bytes[entry.offset..end]),
            Buffer::Write(bytes) => Buffer::Write(&bytes[entry.offset..end]),
        };
        match unsafe { self.driver.submit(registered, raw, &mut selected) } {
            Submission::Complete => {
                self.buffers.entries.as_mut_slice()[index].retiring_submission = true;
                return self.poll_submission_retirement(index);
            }
            Submission::Pending => {
                if D::QUERY_REFRESHES_SOURCE {
                    self.refresh_query_resource(index);
                }
            }
            Submission::Failed(failure) => self.finish(index, Some(T::failure(failure)), None),
        }
        Poll::Ready(())
    }

    fn poll_submission_retirement(&mut self, index: usize) -> Poll<()> {
        let (registered, raw) = self.request(index);
        let terminal = match unsafe { self.driver.poll_submitted_result(registered, raw) } {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(terminal) => terminal,
        };
        self.buffers.entries.as_mut_slice()[index].retiring_submission = false;
        if let Some(terminal) = terminal {
            self.finish(index, Some(terminal), None);
        }
        Poll::Ready(())
    }

    fn prepare(&mut self, index: usize) -> Result<(), SetupFailure<T>> {
        match self.poll_prepare(index) {
            Poll::Ready(result) => result,
            Poll::Pending => {
                if !self.buffers.entries.as_slice()[index].preparing {
                    // Successful preparation's yielded cleanup is not abortable.
                    unsafe { crate::runtime_invariant_panic() };
                }
                let (registered, raw) = self.request(index);
                unsafe { self.driver.abort_preparation(registered, raw) };
                let entry = &mut self.buffers.entries.as_mut_slice()[index];
                entry.active = false;
                entry.preparing = false;
                Err(SetupFailure::Platform(Failure::unsupported()))
            }
        }
    }

    fn poll_prepare(&mut self, index: usize) -> Poll<Result<(), SetupFailure<T>>> {
        if self.buffers.entries.as_slice()[index]
            .preparation_failure
            .is_some()
        {
            return self.poll_preparation_cleanup(index);
        }
        let (registered, raw) = self.request(index);
        if !self.buffers.entries.as_slice()[index].preparing {
            unsafe { self.driver.reset_request(registered, raw) };
            let entry = &mut self.buffers.entries.as_mut_slice()[index];
            entry.active = true;
            entry.preparing = true;
            entry.resource = None;
        }
        let entry = &mut self.buffers.entries.as_mut_slice()[index];
        let end = entry.offset + entry.length;
        let mut selected = match &mut entry.buffer {
            Buffer::Read(bytes) => Buffer::Read(&mut bytes[entry.offset..end]),
            Buffer::Write(bytes) => Buffer::Write(&bytes[entry.offset..end]),
        };
        let prepared = match unsafe {
            self.driver
                .poll_prepare_request(registered, raw, &mut selected)
        } {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(prepared) => prepared,
        };
        entry.preparing = false;
        if let Err(error) = prepared {
            // Failed preparation has already released its own staging; it is
            // not a live request and must not receive cancel/drain/completion.
            entry.active = false;
            entry.prepared = false;
            return Poll::Ready(Err(SetupFailure::Preparation(error)));
        }
        entry.prepared = true;
        Poll::Ready(match unsafe { self.driver.resource(registered, raw) } {
            Ok(resource) => {
                entry.resource = Some(resource);
                Ok(())
            }
            Err(failure) => {
                // Preparation succeeded and therefore owns one cleanup, even
                // though native work has not been submitted. Retire that staging
                // before reporting the infrastructure failure. Earlier successful
                // records remain protected by the installed group guard.
                entry.preparation_failure = Some(failure);
                return self.poll_preparation_cleanup(index);
            }
        })
    }

    fn poll_preparation_cleanup(&mut self, index: usize) -> Poll<Result<(), SetupFailure<T>>> {
        let (registered, raw) = self.request(index);
        let terminal = match unsafe { self.driver.poll_drain(registered, raw) } {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(terminal) => terminal,
        };
        let entry = &mut self.buffers.entries.as_mut_slice()[index];
        entry.active = false;
        entry.prepared = false;
        let failure = entry.preparation_failure.take().unwrap();
        if let Some(terminal) = terminal {
            if terminal.is_checked_error() {
                return Poll::Ready(Err(SetupFailure::Preparation(terminal)));
            }
            unsafe { terminal.drop_in_frame(self.driver.call_frame()) };
        }
        Poll::Ready(Err(SetupFailure::Platform(failure)))
    }

    fn preparation_terminal(failure: SetupFailure<T>) -> T {
        match failure {
            SetupFailure::Platform(failure) => T::failure(failure),
            SetupFailure::Preparation(terminal) => terminal,
        }
    }

    fn finish(&mut self, index: usize, result: Option<T>, stop: Option<Stop>) {
        self.dequeue(index);
        let entry = &mut self.buffers.entries.as_mut_slice()[index];
        entry.active = false;
        entry.preparing = false;
        entry.prepared = false;
        entry.preparation_failure = None;
        entry.submitted = false;
        entry.retiring_submission = false;
        let bound = entry.length.min(D::MAXIMUM_TRANSFER_BYTES);
        let result = result.map(|result| result.bounded(bound));
        entry.progress = result.as_ref().and_then(TerminalOutcome::progress);
        entry.ready = Some(Awaited {
            transfer: result.and_then(TerminalOutcome::into_transfer),
            stop,
            cancellation_failure: entry.cancellation_failure.take(),
        });
    }

    fn unsubmitted(&mut self, index: usize, stop: Stop) {
        let entry = &mut self.buffers.entries.as_mut_slice()[index];
        entry.ready = Some(Awaited {
            transfer: None,
            stop: Some(stop),
            cancellation_failure: None,
        });
    }

    fn stop_all(&mut self, stop: Stop) {
        if self.poll_stop_all(stop).is_pending() {
            unsafe { crate::runtime_invariant_panic() };
        }
    }

    fn begin_stop(&mut self, stop: Stop) {
        self.stopped.get_or_insert(stop);
        self.stop_watches();
        if self.stop_drain.is_some() || !self.has_active() {
            return;
        }
        self.stop_drain = Some(0);
        // Fan cancellation out before any drain can block on one slot.
        for index in 0..self.len() {
            if self.buffers.entries.as_slice()[index].submitted {
                let (registered, raw) = self.request(index);
                self.buffers.entries.as_mut_slice()[index].cancellation_failure =
                    unsafe { self.driver.cancel(registered, raw) }.err();
            }
        }
    }

    /// Resume cleanup without repeating cancellation or releasing active loans.
    /// The first stop and every original cancellation failure remain unchanged.
    pub(crate) fn poll_stop_all(&mut self, stop: Stop) -> Poll<()> {
        self.begin_stop(stop);
        let stop = self.stopped.unwrap();
        while let Some(index) = self.stop_drain {
            if index == self.len() {
                self.stop_drain = None;
                break;
            }
            if self.buffers.entries.as_slice()[index].active {
                let (registered, raw) = self.request(index);
                if self.buffers.entries.as_slice()[index].preparing
                    && unsafe { self.driver.preparation_reply_ready(registered, raw) }
                {
                    match self.poll_prepare(index) {
                        Poll::Pending => return Poll::Pending,
                        Poll::Ready(Err(error)) => {
                            let terminal = match error {
                                SetupFailure::Preparation(terminal) => terminal,
                                SetupFailure::Platform(failure) => T::failure(failure),
                            };
                            self.finish(index, Some(terminal), Some(stop));
                            self.stop_drain = Some(index + 1);
                            continue;
                        }
                        Poll::Ready(Ok(())) => {}
                    }
                }
                let entry = &self.buffers.entries.as_slice()[index];
                let terminal = if entry.preparing {
                    unsafe { self.driver.abort_preparation(registered, raw) };
                    None
                } else if entry.submitted || entry.prepared {
                    match unsafe { self.driver.poll_drain(registered, raw) } {
                        Poll::Pending => return Poll::Pending,
                        Poll::Ready(terminal) => terminal,
                    }
                } else {
                    None
                };
                self.finish(index, terminal, Some(stop));
            }
            self.stop_drain = Some(index + 1);
        }
        Poll::Ready(())
    }

    fn has_active(&self) -> bool {
        self.buffers
            .entries
            .as_slice()
            .iter()
            .any(|entry| entry.active)
    }

    fn len(&self) -> usize {
        self.buffers.entries.len()
    }

    fn registered_index(&self, index: usize) -> usize {
        self.selection
            .as_ref()
            .map_or(index, |indices| indices.as_slice()[index])
    }

    fn request(&self, index: usize) -> (usize, *mut D::Request) {
        let registered = self.registered_index(index);
        (registered, self.slots.raw(registered))
    }

    fn rotated(&self, step: usize) -> usize {
        let until_wrap = self.len() - self.cursor;
        if step < until_wrap {
            self.cursor + step
        } else {
            step - until_wrap
        }
    }

    fn take_observation(&mut self) -> Option<Observation<T>> {
        self.take_wait_failure()
            .or_else(|| self.take_ready().map(Observation::Completion))
    }

    fn take_wait_failure(&mut self) -> Option<Observation<T>> {
        if !self.wait_failure_reported {
            if let Some(Stop::Failure(
                doria_diagnostic_catalogue::IoMessageOperation::Wait,
                failure,
            )) = self.stopped
            {
                self.wait_failure_reported = true;
                return Some(Observation::WaitFailed(failure));
            }
        }
        None
    }

    fn take_ready(&mut self) -> Option<Completion<T>> {
        for step in 0..self.len() {
            let index = self.rotated(step);
            if let Some(outcome) = self.buffers.entries.as_mut_slice()[index].ready.take() {
                self.cursor = if index + 1 == self.len() {
                    0
                } else {
                    index + 1
                };
                return Some(Completion {
                    key: self.slots.key(self.registered_index(index)),
                    index,
                    outcome,
                });
            }
        }
        None
    }
}

impl<D: GroupDriver<T>, C: Control<D::Timeout>, T: TerminalOutcome> Drop
    for TransferGroup<'_, '_, D, C, T>
{
    fn drop(&mut self) {
        self.release_watches();
        if self.has_active() {
            self.stop_all(Stop::Cancellation);
        }
        for index in 0..self.len() {
            let (registered, request) = self.request(index);
            unsafe {
                self.driver.release_request(registered, request);
            }
        }
        let frame = self.driver.call_frame();
        for entry in self.buffers.entries.as_mut_slice().iter_mut().rev() {
            if let Some(terminal) = entry.ready.as_mut().and_then(|ready| ready.transfer.take()) {
                unsafe { terminal.drop_in_frame(frame) };
            }
        }
        self.driver.set_call_frame(core::ptr::null());
    }
}

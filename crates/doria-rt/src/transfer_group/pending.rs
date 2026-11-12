//! Explicit complete-adapter preparation/finalization around the existing
//! native driver. Native requests, packet retirement, cancellation and loans
//! remain owned by the same group; this layer adds no completion engine.

use core::{ffi::c_void, mem, ptr, task::Poll};

use super::adapter::{
    AdapterAttempt, AdapterTerminal, DrAdapterContextV1, DrAdapterOutcomeV1, DrTransferAdapterV1,
};
use super::source::SourceSnapshot;
use super::{
    Buffer, FixedSlice, GroupDriver, Query, StableSlots, Submission, TerminalOutcome, Wake,
};
use crate::byte_io::{ReadProgress, WriteProgress};
use crate::io_status::DrIoStatusV1;
#[cfg(test)]
use crate::native_handle::FileOrigin;
use crate::native_handle::{PhysicalOrigin, ReadinessProjection};
use crate::platform_io::Failure;
use crate::{DrInterfaceCarrierV1, DrStackFrameV2};

pub(crate) mod host;
mod preparation;
mod request;
use preparation::Preparation;
use request::DrPendingRequestViewV1;
pub(crate) use request::{
    dr_v1_checked_pending_request_configure_file, dr_v1_checked_pending_request_configure_standard,
};
#[cfg(test)]
pub(crate) use request::{
    dr_v1_checked_pending_request_outcome, dr_v1_pending_request_complete,
    dr_v1_pending_request_configure_file, dr_v1_pending_request_configure_native,
    dr_v1_pending_request_configure_standard, pending_callback_facts,
};

/// Inline cached facts from the sole File owner's retired attempt. Both native
/// drivers use this representation so EOF, no-progress and bounds projection
/// have one interpretation; it owns no handle, region, callback or allocation.
#[derive(Clone, Copy)]
pub(crate) enum FileTerminal {
    Read(Result<ReadProgress, Failure>),
    Write(Result<WriteProgress, Failure>),
}

impl FileTerminal {
    pub(crate) fn result(self) -> Result<usize, Failure> {
        match self {
            Self::Read(Ok(ReadProgress::Data(count)))
            | Self::Write(Ok(WriteProgress::Accepted(count))) => Ok(count),
            Self::Read(Ok(ReadProgress::WouldBlock(_) | ReadProgress::EndOfStream))
            | Self::Write(Ok(WriteProgress::WouldBlock(_))) => Ok(0),
            Self::Read(Err(failure)) | Self::Write(Err(failure)) => Err(failure),
        }
    }

    pub(crate) fn status(self) -> (u8, DrIoStatusV1) {
        let mut status = DrIoStatusV1::default();
        let tag = match self {
            Self::Read(result) => status.read(result),
            Self::Write(result) => status.write(result),
        };
        (tag, status)
    }

    pub(crate) fn bounded_status(
        self,
        result: Result<usize, Failure>,
        bound: usize,
    ) -> (u8, DrIoStatusV1) {
        let actual = self.result();
        if actual.bounded(bound) != result {
            unsafe { crate::runtime_invariant_panic() };
        }
        if actual == result {
            self.status()
        } else {
            // Do not restore an oversized count or impossible error prefix
            // after the shared selected-region bounds guard has rejected it.
            numeric_status(result)
        }
    }
}

/// Preparation may select caller/group storage or reusable adapter-owned
/// storage. It is a loan, never a copied buffer or a new allocation/close claim.
/// The compiler retains its ownership root until completion returns. Direction
/// is fixed by the prepareRead/prepareWrite callback, not inferred from bytes.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(crate) struct DrPendingRegionV1 {
    /// Typed leaf-owned native projection. It is validated by the configured
    /// driver, never dereferenced/cast as a readiness source by this wrapper.
    pub(crate) endpoint: *const c_void,
    pub(crate) data: *mut u8,
    pub(crate) length: usize,
}

pub(crate) type PrepareReadCallback = unsafe extern "C" fn(
    *const DrStackFrameV2,
    *const DrAdapterContextV1,
    *mut c_void,
    *mut u8,
    usize,
    *mut DrAdapterOutcomeV1,
    *mut DrInterfaceCarrierV1,
) -> u8;

pub(crate) type PrepareWriteCallback = unsafe extern "C" fn(
    *const DrStackFrameV2,
    *const DrAdapterContextV1,
    *mut c_void,
    *const u8,
    usize,
    *mut DrAdapterOutcomeV1,
    *mut DrInterfaceCarrierV1,
) -> u8;

/// Native tags retain the existing status transport: 0 Data/Accepted, 1
/// WouldBlock, 2 EndOfStream, 3 failure. A retired regular-file attempt keeps
/// its actual typed outcome; zero Data is not inferred to be EOF.
/// Tag 4 is cleanup of successful preparation without a retired native outcome:
/// no native outcome exists, and status is empty rather than an invented result.
/// Would-block probes awaiting readiness are retry hints, not completed results;
/// a cached FileTerminal still preserves its actual WouldBlock/zero/EOF result.
/// Submitted is 0 for rejection, immediate completion or an unretired readiness
/// retry, and 1 only after native retirement. Immediate is null for native facts,
/// otherwise points to the
/// staged immediate outcome. Original caller storage, distinct from selected
/// native storage, remains available for adapter transformation. Both output
/// and frame addresses are call-scoped, never retained in the request.
/// Callback return 4 explicitly projects a nullable absent completion. It is
/// accepted only when there is no actual native/immediate outcome to discard;
/// malformed absence is checked InvalidInput, not an internal-invariant panic.
pub(crate) type CompleteReadCallback = unsafe extern "C" fn(
    *const DrStackFrameV2,
    *const DrAdapterContextV1,
    *mut c_void,
    *mut u8,
    usize,
    *mut DrAdapterOutcomeV1,
    *mut DrInterfaceCarrierV1,
) -> u8;

pub(crate) type CompleteWriteCallback = unsafe extern "C" fn(
    *const DrStackFrameV2,
    *const DrAdapterContextV1,
    *mut c_void,
    *const u8,
    usize,
    *mut DrAdapterOutcomeV1,
    *mut DrInterfaceCarrierV1,
) -> u8;

/// Separate capability: this six-word descriptor and the four-word synchronous
/// descriptor share the same two-word borrowed receiver dispatch context.
/// Preparing does no I/O: helpers configure the opaque view; callback status 0
/// returns normally, 1 moves a preparation checked error and prefix without
/// calling complete. Runtime validates successful configuration. Immediate data traverses the
/// entire complete adapter, so an outer meter/decoder is never bypassed.
/// The native endpoint must be supplied by the explicit pending capability's
/// borrowed projection, NOT derived by casting scalar readiness metadata.
///
/// Context stays live and exclusive for the entire group. Neither callback may
/// retain the context home or frame/output addresses, unwind through C, or reenter this group.
/// Prepared storage stays borrowed until retirement. Native submission receives
/// complete exactly once, including rejection, cancellation failure, and won
/// completion. Every successful preparation also reaches completion when setup
/// stops before submission, releasing adapter staging as NoTransfer (tag 4).
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct DrPendingTransferAdapterV1 {
    pub(crate) context: DrAdapterContextV1,
    pub(crate) prepare_read: Option<PrepareReadCallback>,
    pub(crate) prepare_write: Option<PrepareWriteCallback>,
    pub(crate) complete_read: Option<CompleteReadCallback>,
    pub(crate) complete_write: Option<CompleteWriteCallback>,
}

/// Hosted dispatch has capabilities, not invented native function pointers or
/// receiver carriers. Its complete value stays in the ordinary host owner.
#[derive(Clone, Copy)]
pub(crate) enum PendingAdapter {
    Native(DrPendingTransferAdapterV1),
    Host { readable: bool, writable: bool },
}

impl PendingAdapter {
    fn valid(self) -> bool {
        match self {
            Self::Native(adapter) => adapter.valid(),
            Self::Host { readable, writable } => readable || writable,
        }
    }

    fn validate(self, reading: bool) -> Result<(), Failure> {
        match self {
            Self::Native(adapter) => adapter.validate(reading),
            Self::Host { readable, writable } if if reading { readable } else { writable } => {
                Ok(())
            }
            Self::Host { .. } => Err(Failure::invalid_input()),
        }
    }

    fn native(self) -> DrPendingTransferAdapterV1 {
        match self {
            Self::Native(adapter) => adapter,
            Self::Host { .. } => unsafe { crate::runtime_invariant_panic() },
        }
    }

    fn update_context(&mut self, context: DrAdapterContextV1) {
        match self {
            Self::Native(adapter) => adapter.context = context,
            Self::Host { .. } => unsafe { crate::runtime_invariant_panic() },
        }
    }
}

impl DrPendingTransferAdapterV1 {
    pub(crate) fn valid(&self) -> bool {
        self.context.valid()
            && self.prepare_read.is_some() == self.complete_read.is_some()
            && self.prepare_write.is_some() == self.complete_write.is_some()
            && (self.prepare_read.is_some() || self.prepare_write.is_some())
    }

    pub(crate) fn validate(&self, reading: bool) -> Result<(), Failure> {
        if self.valid()
            && if reading {
                self.prepare_read.is_some()
            } else {
                self.prepare_write.is_some()
            }
        {
            Ok(())
        } else {
            Err(Failure::invalid_input())
        }
    }

    unsafe fn prepare(
        &self,
        frame: *const DrStackFrameV2,
        buffer: &mut Buffer<'_>,
        record: *mut PendingState,
    ) -> Prepared {
        let mut view = DrPendingRequestViewV1::preparation(record);
        let mut progress = DrAdapterOutcomeV1::default();
        let mut error = crate::empty_error_carrier();
        let status = match buffer {
            Buffer::Read(bytes) => self
                .prepare_read
                .unwrap_or_else(|| crate::runtime_invariant_panic())(
                frame,
                &self.context,
                ptr::from_mut(&mut view).cast(),
                bytes.as_mut_ptr(),
                bytes.len(),
                &mut progress,
                &mut error,
            ),
            Buffer::Write(bytes) => self
                .prepare_write
                .unwrap_or_else(|| crate::runtime_invariant_panic())(
                frame,
                &self.context,
                ptr::from_mut(&mut view).cast(),
                bytes.as_ptr(),
                bytes.len(),
                &mut progress,
                &mut error,
            ),
        };
        view.prepared_reply(status, progress, error)
    }

    unsafe fn complete(
        &self,
        frame: *const DrStackFrameV2,
        record: *mut PendingState,
    ) -> AdapterTerminal {
        let mut view = DrPendingRequestViewV1::completion(record);
        let mut outcome = DrAdapterOutcomeV1::default();
        let mut error = crate::empty_error_carrier();
        let status = if (*record).reading {
            self.complete_read
                .unwrap_or_else(|| crate::runtime_invariant_panic())(
                frame,
                &self.context,
                ptr::from_mut(&mut view).cast(),
                (*record).caller_data,
                (*record).bound,
                &mut outcome,
                &mut error,
            )
        } else {
            self.complete_write
                .unwrap_or_else(|| crate::runtime_invariant_panic())(
                frame,
                &self.context,
                ptr::from_mut(&mut view).cast(),
                (*record).caller_data.cast_const(),
                (*record).bound,
                &mut outcome,
                &mut error,
            )
        };
        view.completed_reply(status, outcome, error)
    }
}

unsafe fn decode_terminal(
    status: u8,
    outcome: DrAdapterOutcomeV1,
    error: DrInterfaceCarrierV1,
    reading: bool,
    bound: usize,
) -> AdapterTerminal {
    match DrTransferAdapterV1::decode(status, outcome, error, reading, bound) {
        AdapterAttempt::Terminal(terminal) => terminal,
        AdapterAttempt::Pending => AdapterTerminal::WouldBlock,
    }
}

enum Prepared {
    Native(DrPendingRegionV1),
    Immediate(DrAdapterOutcomeV1),
    Failed(AdapterTerminal),
    Invalid,
}

#[derive(Clone, Copy)]
enum Configuration {
    Native(DrPendingRegionV1),
    Immediate(DrAdapterOutcomeV1),
}

/// Retired numeric facts belong to the stable request, not callback-stack
/// output slots. This phase reuses the record's storage without owning another
/// request, buffer, Error, frame or endpoint.
struct CompletionFacts {
    native: (u8, DrIoStatusV1),
    immediate: Option<DrAdapterOutcomeV1>,
    submitted: bool,
}

impl CompletionFacts {
    fn new(
        submitted: bool,
        native: Option<(u8, DrIoStatusV1)>,
        immediate: Option<&DrAdapterOutcomeV1>,
    ) -> Self {
        Self {
            native: native.unwrap_or_else(|| (4, DrIoStatusV1::default())),
            immediate: immediate.copied(),
            submitted,
        }
    }
}

enum Phase {
    Idle,
    Preparing,
    Prepared(DrPendingRegionV1),
    Immediate(DrAdapterOutcomeV1),
    Native,
    Finalizing(CompletionFacts),
    Ready(AdapterTerminal),
    Taken,
}

/// One configuration-time record per stable native slot. Only native request
/// fields are kernel-owned; this separate record never moves or aliases them.
/// A group exclusively borrows both the native slots and these records.
pub(crate) struct PendingState {
    adapter: PendingAdapter,
    host: Option<host::Exchange>,
    phase: Phase,
    reading: bool,
    caller_data: *mut u8,
    bound: usize,
    native_bound: usize,
    native_region: DrPendingRegionV1,
    configuration: Option<Configuration>,
    invalid_configuration: bool,
    origin: Option<PhysicalOrigin>,
    configurator: Option<SourceConfigurator>,
}

/// Call-only access to the existing native driver's source selector. It never
/// owns/moves the driver, stores a frame, or survives the prepare callback.
type ResolveSource = unsafe fn(
    *mut c_void,
    usize,
    *mut c_void,
    *const DrStackFrameV2,
    Option<*const c_void>,
) -> Result<Option<SourceSnapshot>, AdapterTerminal>;

#[derive(Clone, Copy)]
struct SourceConfigurator {
    driver: *mut c_void,
    index: usize,
    request: *mut c_void,
    resolve: ResolveSource,
}

unsafe fn configure_source<D: PendingNativeDriver>(
    driver: *mut c_void,
    index: usize,
    request: *mut c_void,
    frame: *const DrStackFrameV2,
    external: Option<*const c_void>,
) -> Result<Option<SourceSnapshot>, AdapterTerminal> {
    (&mut *driver.cast::<D>()).select_source(index, request.cast(), frame, external)
}

struct ConfigurationCall(*mut PendingState);

impl Drop for ConfigurationCall {
    fn drop(&mut self) {
        unsafe {
            (*self.0).configurator = None;
        }
    }
}

/// Borrowed registration facts only. Native and callback phase, storage,
/// progress, and owned Error values live in individual operation records.
#[derive(Clone, Copy)]
pub(crate) struct PendingRegistration {
    adapter: PendingAdapter,
    pub(crate) origin: Option<PhysicalOrigin>,
}

impl PendingRegistration {
    pub(crate) fn state(self) -> Result<PendingState, Failure> {
        let mut state = PendingState::with_adapter(self.adapter)?;
        state.origin = self.origin;
        Ok(state)
    }

    pub(crate) fn validate(self, reading: bool) -> Result<(), Failure> {
        self.adapter.validate(reading)
    }
}

impl PendingState {
    pub(crate) fn validate_idle_source_update(&self) -> Result<(), Failure> {
        if !matches!(self.phase, Phase::Idle | Phase::Taken) {
            return Err(Failure::invalid_input());
        }
        Ok(())
    }

    /// # Safety
    /// Every affected record was validated before route staging. Serialized
    /// exclusive set access preserves those phases through this infallible commit.
    pub(crate) unsafe fn set_idle_origin(&mut self, origin: PhysicalOrigin) {
        if self.validate_idle_source_update().is_err() {
            crate::runtime_invariant_panic();
        }
        self.origin = Some(origin);
    }

    pub(crate) fn registration(&self) -> PendingRegistration {
        PendingRegistration {
            adapter: self.adapter,
            origin: self.origin,
        }
    }
    pub(crate) fn new(adapter: DrPendingTransferAdapterV1) -> Result<Self, Failure> {
        Self::with_adapter(PendingAdapter::Native(adapter))
    }

    fn with_adapter(adapter: PendingAdapter) -> Result<Self, Failure> {
        if !adapter.valid() {
            return Err(Failure::invalid_input());
        }
        Ok(Self {
            adapter,
            host: None,
            phase: Phase::Idle,
            reading: false,
            caller_data: ptr::null_mut(),
            bound: 0,
            native_bound: 0,
            native_region: DrPendingRegionV1::default(),
            configuration: None,
            invalid_configuration: false,
            origin: None,
            configurator: None,
        })
    }

    /// # Safety
    /// Payload is a live compiler-retained file class, unmoved through removal
    /// and every group retirement. Only the file runtime knows its layout.
    pub(crate) unsafe fn for_file_object(
        adapter: DrPendingTransferAdapterV1,
        payload: *mut c_void,
    ) -> Result<(Self, *mut crate::file_io::stream::OwnedFile), Failure> {
        let source = crate::file_io::stream::transport::object::pending_source(payload)?;
        let (record, origin) = Self::for_source(adapter, source.value())?;
        Ok((record, origin.file().ok_or_else(Failure::invalid_input)?.0))
    }

    /// # Safety
    /// Source is a live forwarded typed opaque readiness value during formation.
    /// Fixed-owner callers retain that owner unmoved; bound membership instead
    /// retains resource metadata and selects fresh active-use pins at operation
    /// boundaries. Raw fixture source words are not admitted here.
    pub(crate) unsafe fn for_source(
        adapter: DrPendingTransferAdapterV1,
        source: *const c_void,
    ) -> Result<(Self, PhysicalOrigin), Failure> {
        Self::for_bound_source(PendingAdapter::Native(adapter), source)
    }

    pub(crate) unsafe fn for_bound_source(
        adapter: PendingAdapter,
        source: *const c_void,
    ) -> Result<(Self, PhysicalOrigin), Failure> {
        let source = ReadinessProjection::borrow(source)?;
        let mut record = Self::with_adapter(adapter)?;
        record.origin = Some(source.origin);
        Ok((record, source.origin))
    }

    /// Validate every selected direction before the group's first preparation.
    pub(crate) fn validate(&self, reading: bool) -> Result<(), Failure> {
        if !matches!(self.phase, Phase::Idle | Phase::Taken) {
            return Err(Failure::invalid_input());
        }
        self.adapter.validate(reading)
    }

    /// Freeze retirement before dispatching completion. The record retains its
    /// caller/selected loans and source facts while an ordinary host call runs.
    unsafe fn begin_completion(
        &mut self,
        submitted: bool,
        native: Option<(u8, DrIoStatusV1)>,
        immediate: Option<&DrAdapterOutcomeV1>,
    ) {
        // The native driver proves retirement, even when cancel failed. Clear
        // the native phase BEFORE invoking user cleanup so no second query can
        // dispatch completion from a cached copy of the same native result.
        if !matches!(self.phase, Phase::Native | Phase::Preparing) {
            crate::runtime_invariant_panic();
        }
        self.phase = Phase::Finalizing(CompletionFacts::new(submitted, native, immediate));
    }

    unsafe fn finish_completion(&mut self, terminal: AdapterTerminal) {
        if !matches!(self.phase, Phase::Finalizing(_)) {
            crate::runtime_invariant_panic();
        }
        self.native_bound = 0;
        self.native_region = DrPendingRegionV1::default();
        self.caller_data = ptr::null_mut();
        self.configuration = None;
        self.phase = Phase::Ready(terminal);
    }

    unsafe fn finalize(
        &mut self,
        frame: *const DrStackFrameV2,
        submitted: bool,
        native: Option<(u8, DrIoStatusV1)>,
        immediate: Option<&DrAdapterOutcomeV1>,
    ) {
        self.begin_completion(submitted, native, immediate);
        let adapter = self.adapter.native();
        let terminal = adapter.complete(frame, self);
        self.finish_completion(terminal);
    }

    unsafe fn take_ready(&mut self) -> AdapterTerminal {
        match mem::replace(&mut self.phase, Phase::Taken) {
            Phase::Ready(terminal) => terminal,
            _ => crate::runtime_invariant_panic(),
        }
    }
}

/// # Safety
/// Each index maps to a live record exclusively borrowed by this group for
/// its lifetime. Sparse tables may grow only before the group starts, and their
/// pinned records remain stable. No foreign driver/kernel mutates these records.
pub(crate) unsafe trait PendingRecords {
    fn record(&self, index: usize) -> Option<*mut PendingState>;
    fn extent(&self) -> usize;
}

unsafe impl PendingRecords for FixedSlice<PendingState> {
    fn extent(&self) -> usize {
        self.len()
    }
    fn record(&self, index: usize) -> Option<*mut PendingState> {
        (index < self.len()).then(|| self.raw(index))
    }
}

unsafe impl PendingRecords for StableSlots<PendingState> {
    fn extent(&self) -> usize {
        self.len()
    }
    fn record(&self, index: usize) -> Option<*mut PendingState> {
        self.raw(index)
    }
}

/// # Safety
/// The validator checks a typed leaf-owned projection against the existing
/// registered endpoint/association. It performs no I/O, allocation, ownership
/// change or source-to-owner cast, and retains no new pointer claim. Success
/// establishes that submit will use this SAME prepared endpoint and its live
/// native association. Rejection must precede any native buffer access.
pub(crate) unsafe trait PendingNativeDriver: GroupDriver {
    /// Resolve/rebind the current member source without an active native-use
    /// pin. The physical route's metadata keeps these borrowed facts live until
    /// the next binding update or release_request. Raw fixed-owner drivers have
    /// no dynamic binding and retain their explicit unsafe contract.
    unsafe fn setup_source(
        &mut self,
        _: usize,
        _: *mut Self::Request,
        _: *const DrStackFrameV2,
        _: Option<*const c_void>,
    ) -> Result<Option<SourceSnapshot>, AdapterTerminal> {
        Ok(None)
    }

    /// Resolve again at preparation/configuration, retaining the actual source
    /// selected for this operation. This performs no I/O. A previous selection
    /// can be replaced only while still unsubmitted; a submitted selection is
    /// released only after actual retirement AND complete-adapter cleanup.
    unsafe fn select_source(
        &mut self,
        _: usize,
        _: *mut Self::Request,
        _: *const DrStackFrameV2,
        _: Option<*const c_void>,
    ) -> Result<Option<SourceSnapshot>, AdapterTerminal> {
        Ok(None)
    }

    /// No getter is rerun at retirement. Native work and helper storage must
    /// already be retired, and the complete adapter must already have returned.
    unsafe fn release_source(&mut self, _: usize, _: *mut Self::Request) {}

    /// Configuration-time transfer from an already-owned device queue only.
    /// No OS submission, retry, wait, allocation or adapter bypass is permitted.
    unsafe fn prepare_region(
        &self,
        _: usize,
        _: *mut Self::Request,
        _: bool,
        _: DrPendingRegionV1,
    ) -> Option<DrAdapterOutcomeV1> {
        None
    }
    /// The request is the live configured native slot supplied by this group.
    unsafe fn validate_endpoint(
        &self,
        index: usize,
        request: *mut Self::Request,
        endpoint: *const c_void,
    ) -> Result<(), Failure>;

    /// Project the terminal fact AFTER native retirement, using the exact live
    /// configured request. The result has passed the shared selected-region
    /// bounds guard. A typed leaf may recover its cached WouldBlock/EOF fact,
    /// but must preserve that guard rather than restore out-of-range progress.
    /// This performs no I/O, allocation, retry or ownership change. Ordinary
    /// byte-count drivers retain their existing numeric terminal projection.
    unsafe fn terminal_status(
        &self,
        _index: usize,
        _request: *mut Self::Request,
        result: Result<usize, Failure>,
    ) -> (u8, DrIoStatusV1) {
        numeric_status(result)
    }
}

fn numeric_status(result: Result<usize, Failure>) -> (u8, DrIoStatusV1) {
    let mut status = DrIoStatusV1::default();
    let tag = status.complete(result.map(|count| count as u64));
    (tag, status)
}

pub(crate) struct PendingDriver<'s, D: PendingNativeDriver, R: PendingRecords> {
    native: D,
    records: &'s mut R,
    frame: *const DrStackFrameV2,
}

impl<'s, D: PendingNativeDriver, R: PendingRecords> PendingDriver<'s, D, R> {
    /// # Safety
    /// Native and record indices describe the same distinct operation slot. Every
    /// selected direction was validated before begin; the complete adapters,
    /// native endpoint projections, and all selected storage roots remain live
    /// and exclusive until group teardown. Frame is null or call-scoped; the
    /// core clears it when begin/with_call_frame returns. Prepared callbacks do
    /// no I/O and select only compiler-retained storage, never a local scratch
    /// cell. Native provides the existing GroupDriver retirement guarantees.
    pub(crate) unsafe fn new(native: D, records: &'s mut R, frame: *const DrStackFrameV2) -> Self {
        Self {
            native,
            records,
            frame,
        }
    }

    fn record(&self, index: usize) -> *mut PendingState {
        self.records
            .record(index)
            .unwrap_or_else(|| unsafe { crate::runtime_invariant_panic() })
    }

    /// # Safety
    /// The live operation belongs to this driver. A terminal reply only proves
    /// native retirement; its record and all storage/source loans must remain
    /// live until completion is dispatched and accepted. Repeated inspection
    /// of a retired operation never queries the native driver again.
    pub(crate) unsafe fn retire_query(
        &mut self,
        index: usize,
        request: *mut D::Request,
    ) -> Query<()> {
        let record = &mut *self.record(index);
        match record.phase {
            Phase::Native => match self.native.query(index, request) {
                Query::Terminal(result) => {
                    let native = self.native.terminal_status(
                        index,
                        request,
                        result.bounded(record.native_bound),
                    );
                    record.begin_completion(true, Some(native), None);
                }
                Query::PendingFailure(operation, failure) => {
                    return Query::PendingFailure(operation, failure);
                }
                Query::Pending => return Query::Pending,
            },
            Phase::Finalizing(_) | Phase::Ready(_) => {}
            _ => crate::runtime_invariant_panic(),
        }
        Query::Terminal(())
    }

    /// # Safety
    /// Cancellation has already been fanned out to every native operation that
    /// needs it. Drain proves retirement but does not release the source pin or
    /// adapter staging. Those remain borrowed through completion, including
    /// cleanup of successful preparation that never submitted native work.
    pub(crate) unsafe fn retire_drain(&mut self, index: usize, request: *mut D::Request) {
        let record = &mut *self.record(index);
        match record.phase {
            Phase::Native => {
                let result = self.native.drain(index, request);
                let native = result.map(|result| {
                    self.native
                        .terminal_status(index, request, result.bounded(record.native_bound))
                });
                record.begin_completion(native.is_some(), native, None);
            }
            Phase::Immediate(_) => {
                let Phase::Immediate(outcome) = mem::replace(&mut record.phase, Phase::Preparing)
                else {
                    crate::runtime_invariant_panic();
                };
                record.begin_completion(false, Some(numeric_status(Ok(0))), Some(&outcome));
            }
            Phase::Prepared(_) => {
                record.phase = Phase::Preparing;
                record.begin_completion(false, None, None);
            }
            Phase::Finalizing(_) | Phase::Ready(_) => {}
            _ => crate::runtime_invariant_panic(),
        }
    }

    /// # Safety
    /// Terminal is the once-decoded reply from this operation's complete
    /// adapter, not its native result. Retirement was already frozen in the
    /// record; the adapter call has returned and no request view remains live.
    pub(crate) unsafe fn accept_completion(
        &mut self,
        index: usize,
        request: *mut D::Request,
        terminal: AdapterTerminal,
    ) {
        (*self.record(index)).finish_completion(terminal);
        self.native.release_source(index, request);
    }

    unsafe fn complete_retired(&mut self, index: usize, request: *mut D::Request) {
        let record = &mut *self.record(index);
        if matches!(record.phase, Phase::Finalizing(_)) {
            let adapter = record.adapter.native();
            let terminal = adapter.complete(self.frame, record);
            self.accept_completion(index, request, terminal);
        }
    }
}

// Adapter preparation never runs during query. The underlying Unix driver may
// retry a nonblocking transfer, so its existing query fact remains authoritative;
// Windows observes retired packets or a retired inline regular-file attempt.
// Cancel never proves native retirement.
// It preserves the native request type/address and the core's cancel-all before
// drain-all algorithm. Its own state records require no per-transfer allocation.
unsafe impl<D: PendingNativeDriver, R: PendingRecords> GroupDriver<AdapterTerminal>
    for PendingDriver<'_, D, R>
{
    type Request = D::Request;
    type Timeout = D::Timeout;
    // Caller counts may differ from native counts after buffering/decoding.
    const MAXIMUM_TRANSFER_BYTES: usize = isize::MAX as usize;
    const QUERY_CAN_TRANSFER: bool = D::QUERY_CAN_TRANSFER;
    const PREFLIGHT_EACH_SUBMISSION: bool = true;
    const PREPARE_BEFORE_SUBMISSION: bool = true;
    const EMPTY_MUST_DISPATCH: bool = true;

    unsafe fn resource(
        &self,
        index: usize,
        request: *mut Self::Request,
    ) -> Result<super::TransferResource, Failure> {
        self.native.resource(index, request)
    }

    fn set_call_frame(&mut self, frame: *const DrStackFrameV2) {
        self.frame = frame;
        self.native.set_call_frame(frame);
    }

    fn call_frame(&self) -> *const DrStackFrameV2 {
        self.frame
    }

    unsafe fn acquire_request(
        &mut self,
        index: usize,
        request: *mut Self::Request,
    ) -> Result<(), Failure> {
        self.native.acquire_request(index, request)
    }

    unsafe fn release_request(&mut self, index: usize, request: *mut Self::Request) {
        self.native.release_request(index, request);
    }

    unsafe fn setup_request(
        &mut self,
        index: usize,
        request: *mut Self::Request,
    ) -> Result<(), AdapterTerminal> {
        if let Some(source) = self.native.setup_source(index, request, self.frame, None)? {
            let record = &mut *self.record(index);
            record.origin = Some(source.origin);
            if let Some(context) = source.context {
                record.adapter.update_context(context);
            }
        }
        Ok(())
    }

    unsafe fn reset_request(&mut self, index: usize, request: *mut Self::Request) {
        let record = &mut *self.record(index);
        if !matches!(record.phase, Phase::Idle | Phase::Taken) {
            crate::runtime_invariant_panic();
        }
        self.native.reset_request(index, request);
        record.phase = Phase::Idle;
        record.bound = 0;
        record.caller_data = ptr::null_mut();
        record.native_bound = 0;
        record.native_region = DrPendingRegionV1::default();
        record.configuration = None;
        record.invalid_configuration = false;
        if record.host.is_some() {
            crate::runtime_invariant_panic();
        }
    }

    unsafe fn prepare_request(
        &mut self,
        index: usize,
        request: *mut Self::Request,
        buffer: &mut Buffer<'_>,
    ) -> Result<(), AdapterTerminal> {
        self.begin_preparation(index, request, buffer)?;
        let record = &mut *self.record(index);
        let adapter = record.adapter.native();
        record.configurator = Some(SourceConfigurator {
            driver: ptr::from_mut(&mut self.native).cast(),
            index,
            request: request.cast(),
            resolve: configure_source::<D>,
        });
        let call = ConfigurationCall(record);
        let prepared = adapter.prepare(self.frame, buffer, record);
        drop(call);
        match self.accept_preparation(index, request, prepared) {
            Preparation::Ready => Ok(()),
            Preparation::Failed(terminal) => Err(terminal),
            Preparation::Completion(failure) => {
                self.complete_retired(index, request);
                Err(self.preparation_failure(index, failure))
            }
        }
    }

    unsafe fn submit(
        &mut self,
        index: usize,
        request: *mut Self::Request,
        _: &mut Buffer<'_>,
    ) -> Submission {
        let record = &mut *self.record(index);
        let region = match mem::replace(&mut record.phase, Phase::Preparing) {
            Phase::Prepared(region) => region,
            Phase::Immediate(outcome) => {
                if matches!(record.adapter, PendingAdapter::Host { .. }) {
                    record.begin_completion(false, Some(numeric_status(Ok(0))), Some(&outcome));
                    return Submission::Complete;
                }
                record.finalize(
                    self.frame,
                    false,
                    Some(numeric_status(Ok(0))),
                    Some(&outcome),
                );
                self.native.release_source(index, request);
                return Submission::Complete;
            }
            Phase::Ready(terminal) => {
                record.phase = Phase::Ready(terminal);
                return Submission::Complete;
            }
            _ => crate::runtime_invariant_panic(),
        };
        record.native_region = region;
        let data = if region.length == 0 {
            ptr::NonNull::<u8>::dangling().as_ptr()
        } else {
            region.data
        };
        let mut native_buffer = if record.reading {
            Buffer::Read(core::slice::from_raw_parts_mut(data, region.length))
        } else {
            Buffer::Write(core::slice::from_raw_parts(
                data.cast_const(),
                region.length,
            ))
        };
        record.phase = Phase::Native;
        match self.native.submit(index, request, &mut native_buffer) {
            Submission::Failed(failure) => {
                if matches!(record.adapter, PendingAdapter::Host { .. }) {
                    record.begin_completion(
                        false,
                        Some(numeric_status(Err(failure).bounded(record.native_bound))),
                        None,
                    );
                    return Submission::Complete;
                }
                record.finalize(
                    self.frame,
                    false,
                    Some(numeric_status(Err(failure).bounded(record.native_bound))),
                    None,
                );
                self.native.release_source(index, request);
                Submission::Complete
            }
            submitted => submitted,
        }
    }

    unsafe fn take_submitted_result(
        &mut self,
        index: usize,
        _: *mut Self::Request,
    ) -> Option<AdapterTerminal> {
        let record = &mut *self.record(index);
        matches!(record.phase, Phase::Ready(_)).then(|| record.take_ready())
    }

    unsafe fn poll_submitted_result(
        &mut self,
        index: usize,
        request: *mut Self::Request,
    ) -> Poll<Option<AdapterTerminal>> {
        let record = &*self.record(index);
        if matches!(record.adapter, PendingAdapter::Host { .. })
            && matches!(record.phase, Phase::Finalizing(_))
        {
            return self
                .host_complete(index, request, None)
                .map(|()| Some((*self.record(index)).take_ready()));
        }
        Poll::Ready(self.take_submitted_result(index, request))
    }

    unsafe fn query(
        &mut self,
        index: usize,
        request: *mut Self::Request,
    ) -> Query<AdapterTerminal> {
        match self.retire_query(index, request) {
            Query::Terminal(()) => self.complete_retired(index, request),
            Query::PendingFailure(operation, failure) => {
                return Query::PendingFailure(operation, failure);
            }
            Query::Pending => return Query::Pending,
        }
        Query::Terminal((*self.record(index)).take_ready())
    }

    fn wait_any(
        &mut self,
        timeout: Self::Timeout,
        cancellation: Option<usize>,
    ) -> Result<Wake, Failure> {
        self.native.wait_any(timeout, cancellation)
    }

    unsafe fn attach_wake(
        &mut self,
        handle: crate::native_handle::NativeHandle,
    ) -> Result<(), Failure> {
        self.native.attach_wake(handle)
    }

    unsafe fn detach_wake(&mut self, handle: crate::native_handle::NativeHandle) {
        self.native.detach_wake(handle);
    }

    unsafe fn cancel(
        &mut self,
        index: usize,
        request: *mut Self::Request,
    ) -> Result<(), (doria_diagnostic_catalogue::IoMessageOperation, Failure)> {
        match (*self.record(index)).phase {
            Phase::Native => self.native.cancel(index, request),
            Phase::Ready(_) | Phase::Prepared(_) | Phase::Immediate(_) | Phase::Finalizing(_) => {
                Ok(())
            }
            _ => crate::runtime_invariant_panic(),
        }
    }

    unsafe fn drain(
        &mut self,
        index: usize,
        request: *mut Self::Request,
    ) -> Option<AdapterTerminal> {
        self.retire_drain(index, request);
        self.complete_retired(index, request);
        Some((*self.record(index)).take_ready())
    }

    unsafe fn poll_setup_request(
        &mut self,
        index: usize,
        request: *mut Self::Request,
    ) -> Poll<Result<(), AdapterTerminal>> {
        if matches!((*self.record(index)).adapter, PendingAdapter::Host { .. }) {
            self.host_setup(index, request)
        } else {
            Poll::Ready(self.setup_request(index, request))
        }
    }

    unsafe fn poll_prepare_request(
        &mut self,
        index: usize,
        request: *mut Self::Request,
        buffer: &mut Buffer<'_>,
    ) -> Poll<Result<(), AdapterTerminal>> {
        if matches!((*self.record(index)).adapter, PendingAdapter::Host { .. }) {
            self.host_prepare(index, request, buffer)
        } else {
            Poll::Ready(self.prepare_request(index, request, buffer))
        }
    }

    unsafe fn abort_preparation(&mut self, index: usize, request: *mut Self::Request) {
        self.host_abort(index, request);
    }

    unsafe fn preparation_reply_ready(&self, index: usize, _: *mut Self::Request) -> bool {
        matches!(
            (*self.record(index)).host,
            Some(host::Exchange::Prepare(Some(_)) | host::Exchange::Complete { .. })
        )
    }

    unsafe fn poll_query(
        &mut self,
        index: usize,
        request: *mut Self::Request,
    ) -> Poll<Query<AdapterTerminal>> {
        if !matches!((*self.record(index)).adapter, PendingAdapter::Host { .. }) {
            return Poll::Ready(self.query(index, request));
        }
        match self.retire_query(index, request) {
            Query::Pending => Poll::Ready(Query::Pending),
            Query::PendingFailure(operation, failure) => {
                Poll::Ready(Query::PendingFailure(operation, failure))
            }
            Query::Terminal(()) => self
                .host_complete(index, request, None)
                .map(|()| Query::Terminal((*self.record(index)).take_ready())),
        }
    }

    unsafe fn poll_drain(
        &mut self,
        index: usize,
        request: *mut Self::Request,
    ) -> Poll<Option<AdapterTerminal>> {
        if !matches!((*self.record(index)).adapter, PendingAdapter::Host { .. }) {
            return Poll::Ready(self.drain(index, request));
        }
        self.retire_drain(index, request);
        self.host_complete(index, request, None)
            .map(|()| Some((*self.record(index)).take_ready()))
    }
}

/// Private introspection keeps compiler layouts derived from the runtime, not
/// independently restated. The old descriptor/outcome exports are unchanged.
#[no_mangle]
pub extern "C" fn dr_v1_pending_transfer_adapter_layout(alignment: *mut usize) -> usize {
    if !alignment.is_null() {
        unsafe { alignment.write(mem::align_of::<DrPendingTransferAdapterV1>()) };
    }
    mem::size_of::<DrPendingTransferAdapterV1>()
}

#[no_mangle]
pub extern "C" fn dr_v1_pending_region_layout(alignment: *mut usize) -> usize {
    if !alignment.is_null() {
        unsafe { alignment.write(mem::align_of::<DrPendingRegionV1>()) };
    }
    mem::size_of::<DrPendingRegionV1>()
}

#[cfg(test)]
mod tests;

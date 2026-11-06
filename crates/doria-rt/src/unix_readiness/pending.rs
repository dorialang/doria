//! Explicit pending adapters around the existing Unix transfer set. The native
//! request, readiness watcher, budget and cancellation owner are not duplicated.

use core::ffi::c_void;

use super::transfer::{Dispatcher, TransferSet, WaitControl};
use crate::cancellation::DrCancellationV1;
use crate::file_io::stream::OwnedFile;
use crate::native_handle::PhysicalOrigin;
use crate::platform_io::Failure;
use crate::transfer_group::adapter::AdapterTerminal;
use crate::transfer_group::pending::{
    DrPendingTransferAdapterV1, PendingAdapter, PendingDriver, PendingRegistration, PendingState,
};
use crate::transfer_group::setup::GroupSetup;
use crate::transfer_group::source::SourceProvider;
use crate::transfer_group::{
    BufferList, FixedSlice, SetupFailure, StableSlots, Stop, TransferGroup,
};
use crate::wait_budget::DrWaitBudgetV1;
use crate::DrStackFrameV2;

pub(crate) struct PendingSet {
    native: Option<TransferSet>,
    records: StableSlots<PendingState>,
    registrations: StableSlots<Option<PendingRegistration>>,
}

impl PendingSet {
    /// # Safety
    /// Cancellation is null or a live claim for this call. The existing native
    /// watcher retains it; this wrapper acquires no second cancellation owner.
    pub(crate) unsafe fn new(cancellation: *mut DrCancellationV1) -> Result<Self, Failure> {
        Ok(Self {
            native: Some(TransferSet::borrowed_new(cancellation)?),
            records: StableSlots::new()?,
            registrations: StableSlots::new()?,
        })
    }

    pub(crate) fn empty() -> Result<Self, Failure> {
        Ok(Self {
            native: None,
            records: StableSlots::new()?,
            registrations: StableSlots::new()?,
        })
    }

    #[cfg(test)]
    pub(crate) fn initialized(&self) -> bool {
        self.native.is_some()
    }

    /// Native/hosted owning membership retains its logical resolver separately
    /// from the admission-time source. No getter or transfer runs at admission.
    pub(crate) unsafe fn add_bound_source(
        &mut self,
        key: u64,
        source: *const c_void,
        provider: SourceProvider,
        adapter: Option<PendingAdapter>,
    ) -> Result<usize, Failure> {
        let record = adapter
            .map(|adapter| PendingState::for_bound_source(adapter, source).map(|(state, _)| state))
            .transpose()?;
        if self.native.is_none() {
            self.native = Some(TransferSet::borrowed_new(core::ptr::null_mut())?);
        }
        let index = self
            .native
            .as_mut()
            .unwrap()
            .add_bound_source(key, source, provider)?;
        self.finish_optional_admission(key, index, record)
    }

    pub(crate) unsafe fn update_external_source(
        &mut self,
        registration: usize,
        source: *const c_void,
    ) -> Result<(), Failure> {
        let native = self.native.as_mut().ok_or_else(Failure::invalid_input)?;
        let metadata = self
            .registrations
            .raw(registration)
            .ok_or_else(Failure::invalid_input)?;
        // Validate every affected record before committing the physical binding.
        // No callback or fallible record update follows successful route staging.
        for operation in 0..self.records.len() {
            if let Some(record) = self.records.raw(operation) {
                if native.operation_registration(operation) == registration {
                    (*record).validate_idle_source_update()?;
                }
            }
        }
        let origin = native.update_external_source(registration, source)?;
        if let Some(metadata) = (*metadata).as_mut() {
            metadata.origin = Some(origin);
        }
        for operation in 0..self.records.len() {
            if let Some(record) = self.records.raw(operation) {
                if native.operation_registration(operation) == registration {
                    (*record).set_idle_origin(origin);
                }
            }
        }
        Ok(())
    }

    /// # Safety
    /// Endpoint is the aligned, stable home of the actual NativeHandle in its
    /// sole live leaf owner, not scalar readiness metadata or a getter-local
    /// handle copy. The owner and exclusive adapter context remain retained at
    /// stable addresses through successful removal or set destruction. No
    /// other consumer may close, transfer, or change the endpoint's mode while
    /// registered. Rejection neither claims nor closes the endpoint.
    pub(crate) unsafe fn add(
        &mut self,
        key: u64,
        endpoint: *const c_void,
        adapter: DrPendingTransferAdapterV1,
    ) -> Result<usize, Failure> {
        let record = PendingState::new(adapter)?;
        let index = self
            .native
            .as_mut()
            .ok_or_else(Failure::invalid_input)?
            .add_pending(key, endpoint)?;
        self.finish_admission(key, index, record)
    }

    /// # Safety
    /// File is null for checked rejection or an exclusive stable loan of a live
    /// OwnedFile leaf, retained along with adapter context through removal/set
    /// destruction. It may not be closed, moved, or otherwise transferred while
    /// registered. Only regular files are implemented by this private mode;
    /// this does not narrow the ordinary public File open/transfer contract.
    pub(crate) unsafe fn add_file(
        &mut self,
        key: u64,
        file: *mut OwnedFile,
        adapter: DrPendingTransferAdapterV1,
    ) -> Result<usize, Failure> {
        let record = PendingState::new(adapter)?;
        let index = self
            .native
            .as_mut()
            .ok_or_else(Failure::invalid_input)?
            .add_file(key, file)?;
        self.finish_admission(key, index, record)
    }

    /// # Safety
    /// File is the actual live file class payload, retained together with its
    /// adapter through removal and all group retirement. The path loan is
    /// captured before preparation so even unconfigured setup has an origin.
    pub(crate) unsafe fn add_file_object(
        &mut self,
        key: u64,
        file: *mut c_void,
        adapter: DrPendingTransferAdapterV1,
    ) -> Result<usize, Failure> {
        let (record, owner) = PendingState::for_file_object(adapter, file)?;
        self.insert_file_record(key, owner, record)
    }

    /// # Safety
    /// Source is the forwarded typed readiness value, not raw handle bits.
    /// Its physical file owner and complete adapter remain retained and stable
    /// through removal and retirement, including before any preparation.
    pub(crate) unsafe fn add_source(
        &mut self,
        key: u64,
        source: *const c_void,
        adapter: DrPendingTransferAdapterV1,
    ) -> Result<usize, Failure> {
        let (record, origin) = PendingState::for_source(adapter, source)?;
        let native = self.native.as_mut().ok_or_else(Failure::invalid_input)?;
        let index = match origin {
            PhysicalOrigin::File { file, .. } => native.add_file(key, file)?,
            PhysicalOrigin::Standard(stream) => native.add_standard(key, stream)?,
        };
        self.finish_admission(key, index, record)
    }

    unsafe fn insert_file_record(
        &mut self,
        key: u64,
        owner: *mut OwnedFile,
        record: PendingState,
    ) -> Result<usize, Failure> {
        let index = self
            .native
            .as_mut()
            .ok_or_else(Failure::invalid_input)?
            .add_file(key, owner)?;
        self.finish_admission(key, index, record)
    }

    fn finish_admission(
        &mut self,
        key: u64,
        registration: usize,
        record: PendingState,
    ) -> Result<usize, Failure> {
        self.finish_optional_admission(key, registration, Some(record))
    }

    fn finish_optional_admission(
        &mut self,
        key: u64,
        registration: usize,
        record: Option<PendingState>,
    ) -> Result<usize, Failure> {
        let native = self.native.as_mut().ok_or_else(Failure::invalid_input)?;
        let operation = native
            .cached_operation(registration, 0)
            .unwrap_or_else(|| unsafe { crate::runtime_invariant_panic() });
        let metadata = record.as_ref().map(PendingState::registration);
        let result = self
            .registrations
            .insert_at(registration, metadata)
            .and_then(|()| match record {
                Some(record) => self.records.insert_at(operation, record),
                None => Ok(()),
            });
        if let Err(failure) = result {
            self.registrations.remove(registration);
            if native.remove_borrowed(key).is_err() {
                unsafe { crate::runtime_invariant_panic() };
            }
            return Err(failure);
        }
        Ok(registration)
    }

    pub(crate) fn remove(&mut self, key: u64) -> Result<(), Failure> {
        let index = self.index_of(key).ok_or_else(Failure::invalid_input)?;
        let records = &mut self.records;
        self.native
            .as_mut()
            .ok_or_else(Failure::invalid_input)?
            .remove_borrowed_with(key, |operation| {
                records.remove(operation);
            })?;
        if self.registrations.remove(index).is_none() {
            unsafe { crate::runtime_invariant_panic() };
        }
        Ok(())
    }

    pub(crate) fn index_of(&self, key: u64) -> Option<usize> {
        self.native.as_ref().and_then(|native| native.index_of(key))
    }

    pub(crate) fn key(&self, index: usize) -> Option<u64> {
        self.native.as_ref().and_then(|native| native.key(index))
    }

    /// # Safety
    /// Frame is null or live for this call only. Configured adapter/native loans
    /// remain exclusive and live throughout the returned group, including every
    /// prepared region's owner until the complete callback has returned.
    pub(crate) unsafe fn begin_selected<'s, 'b>(
        &'s mut self,
        frame: *const DrStackFrameV2,
        buffers: BufferList<'b, AdapterTerminal>,
        budget: &DrWaitBudgetV1,
        selection: FixedSlice<usize>,
    ) -> Result<PendingGroup<'s, 'b>, SetupFailure<AdapterTerminal>> {
        let setup = self.configure_selected(frame, buffers, budget, selection)?;
        setup.finish(frame).map(|inner| PendingGroup { inner })
    }

    /// # Safety
    /// Same frame, set and retained-loan contract as begin_selected. Cancellation
    /// is null or live for acquisition; only this group retains its claim.
    pub(crate) unsafe fn begin_selected_controlled<'s, 'b>(
        &'s mut self,
        frame: *const DrStackFrameV2,
        buffers: BufferList<'b, AdapterTerminal>,
        budget: &DrWaitBudgetV1,
        selection: FixedSlice<usize>,
        cancellation: *mut DrCancellationV1,
    ) -> Result<PendingGroup<'s, 'b>, SetupFailure<AdapterTerminal>> {
        let setup =
            self.configure_selected_controlled(frame, buffers, budget, selection, cancellation)?;
        setup.finish(frame).map(|inner| PendingGroup { inner })
    }

    /// Retains all native resources and loans before the first ordinary host
    /// call. Host startup resumes the shared guard instead of native callbacks.
    /// # Safety
    /// Same exclusive set, source and buffer contract as begin_selected.
    pub(crate) unsafe fn configure_selected<'s, 'b>(
        &'s mut self,
        frame: *const DrStackFrameV2,
        buffers: BufferList<'b, AdapterTerminal>,
        budget: &DrWaitBudgetV1,
        selection: FixedSlice<usize>,
    ) -> Result<PendingSetup<'s, 'b>, SetupFailure<AdapterTerminal>> {
        let cancellation = self
            .native
            .as_ref()
            .ok_or_else(Failure::invalid_input)?
            .cancellation_policy();
        self.configure_selected_controlled(frame, buffers, budget, selection, cancellation)
    }

    /// # Safety
    /// Same contract as configure_selected, with a call-borrowed cancellation
    /// state whose retained claim belongs to the resulting setup/group.
    pub(crate) unsafe fn configure_selected_controlled<'s, 'b>(
        &'s mut self,
        frame: *const DrStackFrameV2,
        buffers: BufferList<'b, AdapterTerminal>,
        budget: &DrWaitBudgetV1,
        selection: FixedSlice<usize>,
        cancellation: *mut DrCancellationV1,
    ) -> Result<PendingSetup<'s, 'b>, SetupFailure<AdapterTerminal>> {
        if selection.len() != buffers.len() {
            return Err(Failure::invalid_input().into());
        }
        // Validate every direction and selected registration before invoking any
        // adapter. Native group construction retains its own selection checks.
        for (index, registered) in selection.as_slice().iter().enumerate() {
            if self.key(*registered).is_none() {
                return Err(Failure::invalid_input().into());
            }
            let (reading, _) = buffers
                .request_info(index)
                .ok_or_else(Failure::invalid_input)?;
            let record = self
                .registrations
                .raw(*registered)
                .ok_or_else(Failure::invalid_input)?;
            (*record)
                .ok_or_else(Failure::invalid_input)?
                .validate(reading)?;
        }
        let native = self.native.as_mut().ok_or_else(Failure::invalid_input)?;
        for registration in selection.as_slice() {
            native.validate_pending_registration(*registration)?;
        }
        let operations = native.configure_operations(selection)?;
        for (index, operation) in operations.as_slice().iter().copied().enumerate() {
            if !self.records.contains(operation) {
                let metadata = self
                    .registrations
                    .raw(native.operation_registration(operation))
                    .ok_or_else(Failure::invalid_input)?;
                self.records.insert_at(
                    operation,
                    (*metadata).ok_or_else(Failure::invalid_input)?.state()?,
                )?;
            }
            let (reading, _) = buffers
                .request_info(index)
                .ok_or_else(Failure::invalid_input)?;
            (*self.records.raw(operation).unwrap()).validate(reading)?;
        }
        let records = &mut self.records;
        native.configure_prepared_with_driver_controlled(
            buffers,
            budget,
            operations,
            cancellation,
            |native| PendingDriver::new(native, records, frame),
        )
    }
}

type InnerGroup<'s, 'b> = TransferGroup<
    's,
    'b,
    PendingDriver<'s, Dispatcher<'s>, StableSlots<PendingState>>,
    WaitControl,
    AdapterTerminal,
>;

pub(crate) type PendingSetup<'s, 'b> = GroupSetup<
    's,
    'b,
    PendingDriver<'s, Dispatcher<'s>, StableSlots<PendingState>>,
    WaitControl,
    AdapterTerminal,
>;

pub(crate) struct PendingGroup<'s, 'b> {
    inner: InnerGroup<'s, 'b>,
}

impl PendingGroup<'_, '_> {
    /// # Safety
    /// Frame is null or live throughout cleanup and is never retained.
    pub(crate) unsafe fn drop_in_frame(self, frame: *const DrStackFrameV2) {
        self.inner.drop_in_frame(frame);
    }

    /// # Safety
    /// Frame is null or a live generated frame chain during this call only.
    #[cfg(test)]
    pub(crate) unsafe fn next(
        &mut self,
        frame: *const DrStackFrameV2,
    ) -> Option<crate::transfer_group::Completion<AdapterTerminal>> {
        self.inner.with_call_frame(frame, |inner| inner.next())
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
    /// Frame is null or a live generated frame chain during this call only.
    pub(crate) unsafe fn submit(
        &mut self,
        frame: *const DrStackFrameV2,
        index: usize,
        offset: usize,
        length: usize,
    ) -> Result<(), Failure> {
        self.inner
            .with_call_frame(frame, |inner| inner.submit(index, offset, length))
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
mod tests;

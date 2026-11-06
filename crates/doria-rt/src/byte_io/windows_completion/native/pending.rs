//! Explicit pending adapters over the existing shared native completion engine.
//! Attached pipes keep their IOCP lifecycle; typed regular File leaves execute
//! the sole owner's synchronous methods. Both stage adapter state beside the
//! same stable request, without acquiring another handle or close obligation.

use core::ffi::c_void;

use super::broker::transport::{NativeTransferSet, PendingControl, PendingDispatcher};
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
    native: Option<NativeTransferSet>,
    records: StableSlots<PendingState>,
    registrations: StableSlots<Option<PendingRegistration>>,
}

impl PendingSet {
    /// # Safety
    /// Cancellation is null or has a live claim during acquisition. The native
    /// set retains that existing claim; this wrapper creates no second holder.
    pub(crate) unsafe fn new(cancellation: *mut DrCancellationV1) -> Result<Self, Failure> {
        Ok(Self {
            native: Some(NativeTransferSet::borrowed_new(cancellation)?),
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
            self.native = Some(NativeTransferSet::borrowed_new(core::ptr::null_mut())?);
        }
        let index = self
            .native
            .as_mut()
            .unwrap()
            .add_bound_source(key, source, provider)?;
        self.insert_optional_record(key, index, record)
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
    /// Endpoint is null for rejection or a typed live AttachedStream projection
    /// for this call, never a scalar readiness source. This exact owner home and
    /// the complete adapter remain stable and exclusively retained until removal
    /// or metadata destruction, after every group has finished. The stream's
    /// native association remains pinned; raw borrowed routing is unchanged.
    /// Preparation/completion obey the shared pending descriptor contract.
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
            .add_borrowed(key, endpoint)?;
        self.insert_record(key, index, record)
    }

    /// # Safety
    /// File is null for checked rejection or points to a live complete OwnedFile
    /// at a stable, exclusively retained address through removal/set destruction
    /// and all group teardown. The adapter obeys the existing preparation and
    /// completion protocol. This is a typed File leaf, never a source scalar or
    /// an AttachedStream projection; rejection preserves its sole close owner.
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
        self.insert_record(key, index, record)
    }

    /// # Safety
    /// File is the actual compiler-retained complete file payload at a stable
    /// exclusive address. Capture its borrowed path before any preparation;
    /// this acquires no second handle, close claim, or string reference.
    pub(crate) unsafe fn add_file_object(
        &mut self,
        key: u64,
        file: *mut c_void,
        adapter: DrPendingTransferAdapterV1,
    ) -> Result<usize, Failure> {
        let (record, owner) = PendingState::for_file_object(adapter, file)?;
        let index = self
            .native
            .as_mut()
            .ok_or_else(Failure::invalid_input)?
            .add_file(key, owner)?;
        self.insert_record(key, index, record)
    }

    /// # Safety
    /// Source is a forwarded compiler-owned typed readiness value. Its physical
    /// file owner and complete adapter are retained, unmoved and exclusive
    /// through removal and retirement. Raw fixture values are not source records.
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
        self.insert_record(key, index, record)
    }

    fn insert_record(
        &mut self,
        key: u64,
        index: usize,
        record: PendingState,
    ) -> Result<usize, Failure> {
        self.insert_optional_record(key, index, Some(record))
    }

    fn insert_optional_record(
        &mut self,
        key: u64,
        index: usize,
        record: Option<PendingState>,
    ) -> Result<usize, Failure> {
        let native = self.native.as_mut().ok_or_else(Failure::invalid_input)?;
        let operation = native
            .cached_operation(index, 0)
            .unwrap_or_else(|| unsafe { crate::runtime_invariant_panic() });
        let metadata = record.as_ref().map(PendingState::registration);
        let result = self
            .registrations
            .insert_at(index, metadata)
            .and_then(|()| match record {
                Some(record) => self.records.insert_at(operation, record),
                None => Ok(()),
            });
        if let Err(failure) = result {
            self.registrations.remove(index);
            // Only new idle membership is rolled back. Pointer-table capacity
            // and all existing pinned request/route identities remain intact.
            // The consumed registration value is never reused after failure.
            if native.remove_borrowed(key).is_err() {
                unsafe { crate::runtime_invariant_panic() };
            }
            return Err(failure);
        }
        Ok(index)
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
    /// Frame is null or live for this call only. Complete adapter contexts,
    /// prepared storage roots and typed endpoints remain exclusive and live
    /// throughout the group. No callback moves or reenters this set/group.
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
    /// Same set, frame and loan contract as begin_selected. Cancellation is
    /// null or live for acquisition; its new claim belongs to this group.
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

    /// Configures the same native guard before any ordinary hosted calls.
    /// # Safety
    /// Same retained owner, source, buffer and frame contract as begin_selected.
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
    /// Same contract as configure_selected, with a call-borrowed state retained
    /// independently by completion control before any operation is submitted.
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
        for (position, index) in selection.as_slice().iter().enumerate() {
            if self.key(*index).is_none() {
                return Err(Failure::invalid_input().into());
            }
            let record = self
                .registrations
                .get(*index)
                .ok_or_else(Failure::invalid_input)?;
            let (reading, _) = buffers
                .request_info(position)
                .ok_or_else(Failure::invalid_input)?;
            record
                .ok_or_else(Failure::invalid_input)?
                .validate(reading)?;
        }
        let native = self.native.as_mut().ok_or_else(Failure::invalid_input)?;
        for registration in selection.as_slice() {
            native.validate_pending_registration(*registration)?;
        }
        let operations = native.configure_operations(selection)?;
        for (position, operation) in operations.as_slice().iter().copied().enumerate() {
            if !self.records.contains(operation) {
                let metadata = self
                    .registrations
                    .get(native.operation_registration(operation))
                    .ok_or_else(Failure::invalid_input)?;
                self.records.insert_at(
                    operation,
                    metadata.ok_or_else(Failure::invalid_input)?.state()?,
                )?;
            }
            let (reading, _) = buffers
                .request_info(position)
                .ok_or_else(Failure::invalid_input)?;
            self.records.get(operation).unwrap().validate(reading)?;
        }
        let records = &mut self.records;
        native.configure_selected_prepared_with_driver_controlled(
            buffers,
            budget,
            operations,
            cancellation,
            |driver| PendingDriver::new(driver, records, frame),
        )
    }
}

type GroupInner<'s, 'b> = TransferGroup<
    's,
    'b,
    PendingDriver<'s, PendingDispatcher<'s>, StableSlots<PendingState>>,
    PendingControl<'s>,
    AdapterTerminal,
>;

pub(crate) type PendingSetup<'s, 'b> = GroupSetup<
    's,
    'b,
    PendingDriver<'s, PendingDispatcher<'s>, StableSlots<PendingState>>,
    PendingControl<'s>,
    AdapterTerminal,
>;

pub(crate) struct PendingGroup<'s, 'b> {
    inner: GroupInner<'s, 'b>,
}

impl PendingGroup<'_, '_> {
    /// # Safety
    /// Frame is null or live throughout cleanup and is never retained.
    pub(crate) unsafe fn drop_in_frame(self, frame: *const DrStackFrameV2) {
        self.inner.drop_in_frame(frame);
    }

    /// # Safety
    /// Frame is null or live for this call; callbacks cannot retain it or move
    /// the group. The shared frame guard clears it before returning.
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
    /// Frame is null or live for this call. Every newly selected preparation
    /// obeys the retained storage/endpoint contract established by add/begin.
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

// Teardown remains the shared TransferGroup's cancel-all/drain-all path. The
// frame is cleared by begin/next/submit, so destructor completion uses null.

#[cfg(test)]
mod tests;

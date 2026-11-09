//! Owned configuration, not owned bytes or streams. Moving/dropping a plan
//! moves/releases its descriptor loans; beginning transfers them into the one
//! shared group owner after all configuration validation and allocation.

use super::{Buffer, BufferList, FixedSlice, TerminalOutcome};
use crate::platform_io::{Failure, Reason};
use core::sync::atomic::{AtomicU64, Ordering};

/// Private registration ticket. Its set identity is never a dereferenced set
/// pointer and never reused after that set is released or destroyed.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PlanKey {
    pub(crate) owner: usize,
    pub(crate) value: u64,
}

/// An operation identity, not a stream registration or dereferenceable pointer.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OperationKey {
    pub(crate) owner: u64,
    pub(crate) position: u64,
}

static NEXT_PLAN_IDENTITY: AtomicU64 = AtomicU64::new(0);

pub(crate) fn next_plan_identity() -> Result<u64, Failure> {
    NEXT_PLAN_IDENTITY
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |last| {
            last.checked_add(1)
        })
        .map(|last| last + 1)
        .map_err(|_| Failure {
            reason: Reason::ResourceExhausted,
            system_code: None,
            transferred_byte_count: None,
        })
}

/// Group observations keep the consumed plan's stamp, not a registration value.
pub(crate) struct GroupIdentity {
    owner: u64,
    observed: Option<(usize, bool)>,
}

impl GroupIdentity {
    pub(crate) fn new(owner: u64) -> Self {
        Self {
            owner,
            observed: None,
        }
    }

    pub(crate) fn clear(&mut self) {
        self.observed = None;
    }

    pub(crate) fn observe(&mut self, position: usize, reading: bool) {
        self.observed = Some((position, reading));
    }

    pub(crate) fn observed(&self, reading: bool) -> Option<OperationKey> {
        let (position, direction) = self.observed?;
        (direction == reading).then_some(OperationKey {
            owner: self.owner,
            position: position as u64,
        })
    }

    pub(crate) fn operation_position(
        &self,
        key: OperationKey,
        reading: bool,
        request_info: impl FnOnce(usize) -> Option<(bool, usize)>,
    ) -> Result<usize, Failure> {
        if key.owner == 0 || key.owner != self.owner {
            return Err(Failure::invalid_input());
        }
        let position = usize::try_from(key.position).map_err(|_| Failure::invalid_input())?;
        match request_info(position) {
            Some((direction, _)) if direction == reading => Ok(position),
            _ => Err(Failure::invalid_input()),
        }
    }
}

struct Entry<'b> {
    key: PlanKey,
    buffer: Buffer<'b>,
}

pub(crate) struct TransferPlan<'b> {
    identity: u64,
    entries: FixedSlice<Entry<'b>>,
}

pub(crate) struct PreparedPlan<'b, T = Result<usize, Failure>> {
    pub(crate) identity: u64,
    pub(crate) selection: FixedSlice<usize>,
    pub(crate) buffers: BufferList<'b, T>,
}

impl<'b> TransferPlan<'b> {
    pub(crate) fn new(key: PlanKey, buffer: Buffer<'b>) -> Result<Self, Failure> {
        if key.owner == 0 {
            return Err(Failure::invalid_input());
        }
        let identity = next_plan_identity()?;
        let mut entries = FixedSlice::new(1)?;
        entries.push(Entry { key, buffer })?;
        Ok(Self { identity, entries })
    }

    pub(crate) fn operation_key(
        &self,
        position: i64,
        reading: bool,
    ) -> Result<OperationKey, Failure> {
        let position = usize::try_from(position).map_err(|_| Failure::invalid_input())?;
        let entry = self
            .entries
            .as_slice()
            .get(position)
            .ok_or_else(Failure::invalid_input)?;
        if matches!(entry.buffer, Buffer::Read(_)) != reading {
            return Err(Failure::invalid_input());
        }
        Ok(OperationKey {
            owner: self.identity,
            position: position as u64,
        })
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn accepts_key(&self, key: PlanKey) -> bool {
        key.owner != 0
            && self
                .entries
                .as_slice()
                .iter()
                .all(|entry| entry.key.owner == key.owner)
    }

    /// Called after incoming address/length validation and before forming its
    /// slice. Existing loans are only inspected for addresses, never bytes.
    pub(crate) fn conflicts_with(&self, region: (usize, usize, bool)) -> bool {
        self.entries
            .as_slice()
            .iter()
            .any(|entry| super::regions_conflict(entry.buffer.region(), region))
    }

    /// Failure returns the original plan and incoming loan, unchanged. All
    /// construction work is before I/O; a consumed public parameter may drop
    /// these descriptors on failure, but never their callers' byte storage.
    pub(crate) fn append(
        self,
        key: PlanKey,
        buffer: Buffer<'b>,
    ) -> Result<Self, (Failure, Self, Buffer<'b>)> {
        let identity = self.identity;
        if !self.accepts_key(key) {
            return Err((Failure::invalid_input(), self, buffer));
        }
        let Some(count) = self.len().checked_add(1) else {
            return Err((Failure::invalid_input(), self, buffer));
        };
        let mut entries = match FixedSlice::new(count) {
            Ok(entries) => entries,
            Err(failure) => return Err((failure, self, buffer)),
        };
        // The new allocation is complete before any existing loan is moved.
        // The exact fixed capacity makes every insertion below infallible.
        for entry in self.entries {
            if entries.push(entry).is_err() {
                unsafe { crate::runtime_invariant_panic() };
            }
        }
        if entries.push(Entry { key, buffer }).is_err() {
            unsafe { crate::runtime_invariant_panic() };
        }
        Ok(Self { identity, entries })
    }

    /// Lookup is the inactive native set's key lookup under an exclusive loan.
    /// Dynamic membership may change outside that loan; removed values never
    /// resolve to a later registration, even when its slot or stream is reused.
    /// No key grants a separate stream or buffer owner. Rejected preparation
    /// returns the original plan with every loan intact and no native I/O.
    pub(crate) fn prepare(
        self,
        owner: usize,
        lookup: impl Fn(u64) -> Option<usize>,
    ) -> Result<PreparedPlan<'b>, (Failure, Self)> {
        self.prepare_as(owner, lookup)
    }

    /// The same validated plan and loans feed typed adapter terminals without
    /// a second plan owner, selection pass, or configuration allocation.
    pub(crate) fn prepare_as<T: TerminalOutcome>(
        self,
        owner: usize,
        lookup: impl Fn(u64) -> Option<usize>,
    ) -> Result<PreparedPlan<'b, T>, (Failure, Self)> {
        if owner == 0 || self.len() == 0 {
            return Err((Failure::invalid_input(), self));
        }
        for (position, entry) in self.entries.as_slice().iter().enumerate() {
            if entry.key.owner != owner {
                return Err((Failure::invalid_input(), self));
            }
            let Some(index) = lookup(entry.key.value) else {
                return Err((Failure::invalid_input(), self));
            };
            if self.entries.as_slice()[..position].iter().any(|prior| {
                prior.key.value != entry.key.value && lookup(prior.key.value) == Some(index)
            }) {
                return Err((Failure::invalid_input(), self));
            }
        }
        let mut selection = match FixedSlice::new(self.len()) {
            Ok(selection) => selection,
            Err(failure) => return Err((failure, self)),
        };
        let mut buffers = match BufferList::with_capacity(self.len()) {
            Ok(buffers) => buffers,
            Err(failure) => return Err((failure, self)),
        };
        for entry in self.entries {
            // Native membership cannot change through the exclusive set loan.
            let index = lookup(entry.key.value)
                .unwrap_or_else(|| unsafe { crate::runtime_invariant_panic() });
            if selection.push(index).is_err() || buffers.push(entry.buffer).is_err() {
                unsafe { crate::runtime_invariant_panic() };
            }
        }
        Ok(PreparedPlan {
            identity: self.identity,
            selection,
            buffers,
        })
    }
}

#[cfg(test)]
mod tests;

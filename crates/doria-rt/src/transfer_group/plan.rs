//! Owned configuration, not owned bytes or streams. Moving/dropping a plan
//! moves/releases its descriptor loans; beginning transfers them into the one
//! shared group owner after all configuration validation and allocation.

use super::{Buffer, BufferList, FixedSlice, TerminalOutcome, WatchSource};
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

/// What a plan entry is. The byte is shared with compiled code, which names a
/// key's kind when it asks for one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EntryKind {
    Write = 0,
    Read = 1,
    Watch = 2,
}

impl EntryKind {
    pub(crate) fn from_byte(kind: u8) -> Result<Self, Failure> {
        match kind {
            0 => Ok(Self::Write),
            1 => Ok(Self::Read),
            2 => Ok(Self::Watch),
            _ => Err(Failure::invalid_input()),
        }
    }
}

/// A transfer's direction: reading or not.
impl From<bool> for EntryKind {
    fn from(reading: bool) -> Self {
        if reading {
            Self::Read
        } else {
            Self::Write
        }
    }
}

/// Group observations keep the consumed plan's stamp, not a registration value.
/// Keys name plan positions, which count watches; the group's transfers and
/// its watches are each indexed among their own kind, in plan order.
pub(crate) struct GroupIdentity {
    owner: u64,
    observed: Option<(usize, EntryKind)>,
    /// The plan positions that are watches, ascending.
    watches: FixedSlice<usize>,
}

impl GroupIdentity {
    /// A group whose plan holds transfers only.
    pub(crate) fn new(owner: u64) -> Self {
        Self::with_watches(owner, FixedSlice::empty())
    }

    pub(crate) fn with_watches(owner: u64, watches: FixedSlice<usize>) -> Self {
        Self {
            owner,
            observed: None,
            watches,
        }
    }

    pub(crate) fn clear(&mut self) {
        self.observed = None;
    }

    /// The plan position of the transfer at `index` among the transfers.
    pub(crate) fn transfer_position(&self, index: usize) -> usize {
        // Each watch at or before the running position pushes the transfer
        // one place further along the plan.
        self.watches
            .as_slice()
            .iter()
            .fold(index, |position, watch| {
                position + usize::from(*watch <= position)
            })
    }

    /// The index among the transfers of the entry at `position`, unless that
    /// entry is a watch.
    fn transfer_index(&self, position: usize) -> Option<usize> {
        let watches = self.watches.as_slice();
        if watches.contains(&position) {
            return None;
        }
        Some(position - watches.iter().filter(|watch| **watch < position).count())
    }

    pub(crate) fn observe(&mut self, index: usize, reading: bool) {
        self.observed = Some((self.transfer_position(index), reading.into()));
    }

    /// Records the watch at `index` among the watches and returns its plan
    /// position.
    pub(crate) fn observe_watch(&mut self, index: usize) -> usize {
        let position = self.watches.as_slice()[index];
        self.observed = Some((position, EntryKind::Watch));
        position
    }

    pub(crate) fn observed(&self, kind: impl Into<EntryKind>) -> Option<OperationKey> {
        let (position, observed) = self.observed?;
        (observed == kind.into()).then_some(OperationKey {
            owner: self.owner,
            position: position as u64,
        })
    }

    fn position(&self, key: OperationKey) -> Result<usize, Failure> {
        if key.owner == 0 || key.owner != self.owner {
            return Err(Failure::invalid_input());
        }
        usize::try_from(key.position).map_err(|_| Failure::invalid_input())
    }

    /// The index among the group's transfers that a read or write key names.
    pub(crate) fn operation_position(
        &self,
        key: OperationKey,
        reading: bool,
        request_info: impl FnOnce(usize) -> Option<(bool, usize)>,
    ) -> Result<usize, Failure> {
        let index = self
            .transfer_index(self.position(key)?)
            .ok_or_else(Failure::invalid_input)?;
        match request_info(index) {
            Some((direction, _)) if direction == reading => Ok(index),
            _ => Err(Failure::invalid_input()),
        }
    }

    /// The index among the group's watches that a watch key names.
    pub(crate) fn watch_index(&self, key: OperationKey) -> Result<usize, Failure> {
        let position = self.position(key)?;
        self.watches
            .as_slice()
            .iter()
            .position(|watch| *watch == position)
            .ok_or_else(Failure::invalid_input)
    }
}

enum Entry<'b> {
    Transfer {
        key: PlanKey,
        buffer: Buffer<'b>,
    },
    /// A watch belongs to no registration: its source is not in the set.
    Watch(WatchSource),
}

impl Entry<'_> {
    fn kind(&self) -> EntryKind {
        match self {
            Self::Transfer { buffer, .. } => matches!(buffer, Buffer::Read(_)).into(),
            Self::Watch(_) => EntryKind::Watch,
        }
    }

    #[cfg(test)]
    fn buffer(&self) -> &Buffer<'_> {
        match self {
            Self::Transfer { buffer, .. } => buffer,
            Self::Watch(_) => panic!("a watch has no buffer"),
        }
    }

    #[cfg(test)]
    fn key(&self) -> PlanKey {
        match self {
            Self::Transfer { key, .. } => *key,
            Self::Watch(_) => panic!("a watch has no registration key"),
        }
    }
}

pub(crate) struct TransferPlan<'b> {
    identity: u64,
    entries: FixedSlice<Entry<'b>>,
}

pub(crate) struct PreparedPlan<'b, T = Result<usize, Failure>> {
    pub(crate) identity: u64,
    pub(crate) selection: FixedSlice<usize>,
    pub(crate) buffers: BufferList<'b, T>,
    /// The plan's watches in plan order, and the plan position of each.
    pub(crate) watches: FixedSlice<WatchSource>,
    pub(crate) watch_positions: FixedSlice<usize>,
}

impl<'b> TransferPlan<'b> {
    pub(crate) fn new(key: PlanKey, buffer: Buffer<'b>) -> Result<Self, Failure> {
        if key.owner == 0 {
            return Err(Failure::invalid_input());
        }
        Self::starting_with(Entry::Transfer { key, buffer })
    }

    /// A plan whose first entry is a watch. It belongs to no set until a
    /// keyed entry is appended, and may begin on any set if none is.
    pub(crate) fn watching(source: WatchSource) -> Result<Self, Failure> {
        Self::starting_with(Entry::Watch(source))
    }

    fn starting_with(entry: Entry<'b>) -> Result<Self, Failure> {
        let identity = next_plan_identity()?;
        let mut entries = FixedSlice::new(1)?;
        entries.push(entry)?;
        Ok(Self { identity, entries })
    }

    pub(crate) fn operation_key(
        &self,
        position: i64,
        kind: impl Into<EntryKind>,
    ) -> Result<OperationKey, Failure> {
        let position = usize::try_from(position).map_err(|_| Failure::invalid_input())?;
        let entry = self
            .entries
            .as_slice()
            .get(position)
            .ok_or_else(Failure::invalid_input)?;
        if entry.kind() != kind.into() {
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
            && self.entries.as_slice().iter().all(|entry| match entry {
                Entry::Transfer { key: planned, .. } => planned.owner == key.owner,
                Entry::Watch(_) => true,
            })
    }

    /// Called after incoming address/length validation and before forming its
    /// slice. Existing loans are only inspected for addresses, never bytes.
    pub(crate) fn conflicts_with(&self, region: (usize, usize, bool)) -> bool {
        self.entries.as_slice().iter().any(|entry| match entry {
            Entry::Transfer { buffer, .. } => super::regions_conflict(buffer.region(), region),
            Entry::Watch(_) => false,
        })
    }

    /// Failure returns the original plan and incoming loan, unchanged. All
    /// construction work is before I/O; a consumed public parameter may drop
    /// these descriptors on failure, but never their callers' byte storage.
    pub(crate) fn append(
        self,
        key: PlanKey,
        buffer: Buffer<'b>,
    ) -> Result<Self, (Failure, Self, Buffer<'b>)> {
        if !self.accepts_key(key) {
            return Err((Failure::invalid_input(), self, buffer));
        }
        self.append_entry(Entry::Transfer { key, buffer })
            .map_err(|(failure, plan, entry)| match entry {
                Entry::Transfer { buffer, .. } => (failure, plan, buffer),
                Entry::Watch(_) => unsafe { crate::runtime_invariant_panic() },
            })
    }

    /// Failure returns the original plan, unchanged.
    pub(crate) fn append_watch(self, source: WatchSource) -> Result<Self, (Failure, Self)> {
        self.append_entry(Entry::Watch(source))
            .map_err(|(failure, plan, _)| (failure, plan))
    }

    fn append_entry(self, entry: Entry<'b>) -> Result<Self, (Failure, Self, Entry<'b>)> {
        let identity = self.identity;
        let Some(count) = self.len().checked_add(1) else {
            return Err((Failure::invalid_input(), self, entry));
        };
        let mut entries = match FixedSlice::new(count) {
            Ok(entries) => entries,
            Err(failure) => return Err((failure, self, entry)),
        };
        // The new allocation is complete before any existing loan is moved.
        // The exact fixed capacity makes every insertion below infallible.
        for existing in self.entries {
            if entries.push(existing).is_err() {
                unsafe { crate::runtime_invariant_panic() };
            }
        }
        if entries.push(entry).is_err() {
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
        let mut transfers = 0;
        for (position, entry) in self.entries.as_slice().iter().enumerate() {
            let Entry::Transfer { key, .. } = entry else {
                continue;
            };
            transfers += 1;
            if key.owner != owner {
                return Err((Failure::invalid_input(), self));
            }
            let Some(index) = lookup(key.value) else {
                return Err((Failure::invalid_input(), self));
            };
            if self.entries.as_slice()[..position].iter().any(|prior| {
                matches!(prior, Entry::Transfer { key: earlier, .. }
                    if earlier.value != key.value && lookup(earlier.value) == Some(index))
            }) {
                return Err((Failure::invalid_input(), self));
            }
        }
        let watched = self.len() - transfers;
        let mut selection = match FixedSlice::new(transfers) {
            Ok(selection) => selection,
            Err(failure) => return Err((failure, self)),
        };
        let mut buffers = match BufferList::with_capacity(transfers) {
            Ok(buffers) => buffers,
            Err(failure) => return Err((failure, self)),
        };
        let mut watches = match FixedSlice::new(watched) {
            Ok(watches) => watches,
            Err(failure) => return Err((failure, self)),
        };
        let mut watch_positions = match FixedSlice::new(watched) {
            Ok(positions) => positions,
            Err(failure) => return Err((failure, self)),
        };
        for (position, entry) in self.entries.into_iter().enumerate() {
            let stored = match entry {
                Entry::Transfer { key, buffer } => {
                    // Native membership cannot change through the exclusive set loan.
                    let index = lookup(key.value)
                        .unwrap_or_else(|| unsafe { crate::runtime_invariant_panic() });
                    selection.push(index).and_then(|()| buffers.push(buffer))
                }
                Entry::Watch(source) => watches
                    .push(source)
                    .and_then(|()| watch_positions.push(position)),
            };
            if stored.is_err() {
                unsafe { crate::runtime_invariant_panic() };
            }
        }
        Ok(PreparedPlan {
            identity: self.identity,
            selection,
            buffers,
            watches,
            watch_positions,
        })
    }
}

#[cfg(test)]
mod tests;

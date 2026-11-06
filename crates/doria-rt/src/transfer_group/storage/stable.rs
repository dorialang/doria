//! Sparse configuration owners. Growing the owner table never moves a live
//! value; each occupied slot retains its own runtime allocation until removal.

use super::FixedSlice;
use crate::platform_io::Failure;

pub(crate) struct StableSlots<T> {
    entries: FixedSlice<Option<Entry<T>>>,
    occupied: usize,
    newest: Option<usize>,
}

struct Entry<T> {
    value: FixedSlice<T>,
    older: Option<usize>,
    newer: Option<usize>,
}

impl<T> StableSlots<T> {
    pub(crate) fn new() -> Result<Self, Failure> {
        Ok(Self {
            entries: FixedSlice::new(0)?,
            occupied: 0,
            newest: None,
        })
    }

    /// Grows the logical extent within reusable table capacity. Only metadata
    /// relocates during geometric growth, never individually pinned values.
    /// Any failure occurs before existing owners are moved into the new table.
    pub(crate) fn reserve(&mut self, minimum_extent: usize) -> Result<(), Failure> {
        if minimum_extent <= self.len() {
            return Ok(());
        }
        if minimum_extent <= self.entries.capacity {
            for _ in self.len()..minimum_extent {
                self.entries.push(None)?;
            }
            return Ok(());
        }
        let capacity = minimum_extent.max(self.entries.capacity.saturating_mul(2));
        let mut entries = FixedSlice::new(capacity)?;
        for _ in 0..minimum_extent {
            entries.push(None)?;
        }
        for (destination, owner) in entries
            .as_mut_slice()
            .iter_mut()
            .zip(self.entries.as_mut_slice())
        {
            *destination = owner.take();
        }
        self.entries = entries;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn insert(&mut self, value: T) -> Result<usize, Failure> {
        let index = self.vacant_index();
        self.insert_at(index, value)?;
        Ok(index)
    }

    /// Configuration can initialize address-indexed metadata before publishing
    /// the new pinned owner. This does not reserve or mutate the vacant slot.
    pub(crate) fn vacant_index(&self) -> usize {
        self.entries
            .as_slice()
            .iter()
            .position(Option::is_none)
            .unwrap_or(self.len())
    }

    /// Stages both the value owner and any table growth before publishing the
    /// new membership. Reserved table space does not preallocate value owners.
    pub(crate) fn insert_at(&mut self, index: usize, value: T) -> Result<(), Failure> {
        self.insert_at_with(index, || value)
    }

    /// Takes the value only after every fallible storage operation succeeds.
    /// Rejection leaves its caller responsible for context-aware cleanup.
    pub(crate) fn insert_at_with(
        &mut self,
        index: usize,
        value: impl FnOnce() -> T,
    ) -> Result<(), Failure> {
        let extent = index.checked_add(1).ok_or_else(Failure::invalid_input)?;
        if self.contains(index) {
            return Err(Failure::invalid_input());
        }
        let mut owner = FixedSlice::new(1)?;
        self.reserve(extent)?;
        owner.push(value()).expect("one reserved value slot");
        self.entries.as_mut_slice()[index] = Some(Entry {
            value: owner,
            older: self.newest,
            newer: None,
        });
        if let Some(previous) = self.newest {
            self.entries.as_mut_slice()[previous]
                .as_mut()
                .expect("linked slot is occupied")
                .newer = Some(index);
        }
        self.newest = Some(index);
        self.occupied += 1;
        Ok(())
    }

    #[cfg(any(windows, test))]
    pub(crate) fn get(&self, index: usize) -> Option<&T> {
        self.entries
            .as_slice()
            .get(index)?
            .as_ref()?
            .value
            .as_slice()
            .first()
    }

    #[cfg(test)]
    pub(crate) fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        self.entries
            .as_mut_slice()
            .get_mut(index)?
            .as_mut()?
            .value
            .as_mut_slice()
            .first_mut()
    }

    /// Reads only allocation-owner metadata, without borrowing the potentially
    /// kernel-owned T. The pointer remains stable until this slot is removed.
    pub(crate) fn raw(&self, index: usize) -> Option<*mut T> {
        Some(self.entries.as_slice().get(index)?.as_ref()?.value.raw(0))
    }

    pub(crate) fn remove(&mut self, index: usize) -> Option<T> {
        let entry = self.entries.as_mut_slice().get_mut(index)?.take()?;
        if let Some(older) = entry.older {
            self.entries.as_mut_slice()[older]
                .as_mut()
                .expect("linked slot is occupied")
                .newer = entry.newer;
        }
        if let Some(newer) = entry.newer {
            self.entries.as_mut_slice()[newer]
                .as_mut()
                .expect("linked slot is occupied")
                .older = entry.older;
        } else {
            self.newest = entry.older;
        }
        self.occupied -= 1;
        entry.value.into_iter().next()
    }

    pub(crate) fn contains(&self, index: usize) -> bool {
        self.entries
            .as_slice()
            .get(index)
            .is_some_and(Option::is_some)
    }

    /// Includes holes, allowing registered indices to remain stable.
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn occupied_len(&self) -> usize {
        self.occupied
    }

    pub(crate) fn clear_with(&mut self, mut release: impl FnMut(T)) {
        // Reused slots have no relationship to logical acquisition order.
        while let Some(index) = self.newest {
            release(self.remove(index).expect("linked slot is occupied"));
        }
    }
}

impl<T> Drop for StableSlots<T> {
    fn drop(&mut self) {
        self.clear_with(drop);
    }
}

#[cfg(test)]
mod tests;

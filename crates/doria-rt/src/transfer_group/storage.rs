//! Fixed configuration storage using the runtime's allocation owner. Elements
//! never move after initialization and partial construction drops in reverse.

use core::alloc::Layout;
use core::ptr::NonNull;

use crate::platform_io::{Failure, Reason};

mod stable;
pub(crate) use stable::StableSlots;

pub(crate) struct FixedSlice<T> {
    pointer: NonNull<T>,
    allocation: *mut u8,
    capacity: usize,
    initialized: usize,
}

impl<T> FixedSlice<T> {
    pub(crate) fn new(capacity: usize) -> Result<Self, Failure> {
        let layout = Layout::array::<T>(capacity).map_err(|_| Failure::invalid_input())?;
        let mut storage = Self {
            pointer: NonNull::dangling(),
            allocation: core::ptr::null_mut(),
            capacity,
            initialized: 0,
        };
        if layout.size() == 0 {
            return Ok(storage);
        }
        let bytes = layout
            .size()
            .checked_add(layout.align() - 1)
            .filter(|bytes| *bytes <= isize::MAX as usize)
            .ok_or_else(Failure::invalid_input)?;
        let allocation = unsafe { crate::allocate(bytes) };
        if allocation.is_null() {
            return Err(Failure {
                reason: Reason::ResourceExhausted,
                system_code: None,
                transferred_byte_count: None,
            });
        }
        // align_offset retains allocation provenance; the checked over-allocation
        // reserves enough bytes even for an over-aligned element type.
        let offset = allocation.align_offset(layout.align());
        if offset > layout.align() - 1 {
            unsafe { crate::deallocate(allocation) };
            return Err(Failure::invalid_input());
        }
        storage.pointer = unsafe { NonNull::new_unchecked(allocation.add(offset).cast::<T>()) };
        storage.allocation = allocation;
        Ok(storage)
    }

    pub(crate) fn push(&mut self, value: T) -> Result<(), Failure> {
        if self.initialized == self.capacity {
            return Err(Failure::invalid_input());
        }
        unsafe { self.pointer.as_ptr().add(self.initialized).write(value) };
        self.initialized += 1;
        Ok(())
    }

    pub(crate) fn len(&self) -> usize {
        self.initialized
    }

    pub(crate) fn as_slice(&self) -> &[T] {
        unsafe { core::slice::from_raw_parts(self.pointer.as_ptr(), self.initialized) }
    }

    pub(crate) fn as_mut_slice(&mut self) -> &mut [T] {
        unsafe { core::slice::from_raw_parts_mut(self.pointer.as_ptr(), self.initialized) }
    }

    pub(crate) fn raw(&self, index: usize) -> *mut T {
        assert!(index < self.initialized);
        unsafe { self.pointer.as_ptr().add(index) }
    }
}

impl<T> Drop for FixedSlice<T> {
    fn drop(&mut self) {
        while self.initialized != 0 {
            self.initialized -= 1;
            unsafe { self.pointer.as_ptr().add(self.initialized).drop_in_place() };
        }
        if !self.allocation.is_null() {
            unsafe { crate::deallocate(self.allocation) };
        }
    }
}

pub(crate) struct IntoIter<T> {
    storage: FixedSlice<T>,
    front: usize,
}

impl<T> IntoIterator for FixedSlice<T> {
    type Item = T;
    type IntoIter = IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        IntoIter {
            storage: self,
            front: 0,
        }
    }
}

impl<T> Iterator for IntoIter<T> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        if self.front == self.storage.initialized {
            return None;
        }
        let value = unsafe { self.storage.raw(self.front).read() };
        self.front += 1;
        Some(value)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.storage.initialized - self.front;
        (remaining, Some(remaining))
    }
}

impl<T> DoubleEndedIterator for IntoIter<T> {
    fn next_back(&mut self) -> Option<T> {
        if self.front == self.storage.initialized {
            return None;
        }
        self.storage.initialized -= 1;
        Some(unsafe {
            self.storage
                .pointer
                .as_ptr()
                .add(self.storage.initialized)
                .read()
        })
    }
}

impl<T> ExactSizeIterator for IntoIter<T> {}

impl<T> Drop for IntoIter<T> {
    fn drop(&mut self) {
        // Restrict the backing owner before user destructors can unwind. Its
        // field cleanup must never revisit values moved out of either end.
        self.storage.pointer =
            unsafe { NonNull::new_unchecked(self.storage.pointer.as_ptr().add(self.front)) };
        self.storage.initialized -= self.front;
        while self.storage.initialized != 0 {
            self.storage.initialized -= 1;
            unsafe {
                self.storage
                    .pointer
                    .as_ptr()
                    .add(self.storage.initialized)
                    .drop_in_place()
            };
        }
    }
}

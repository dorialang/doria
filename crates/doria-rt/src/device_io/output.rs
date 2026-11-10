//! Console output waiting on the standard-output device (Decision 0140).
//!
//! `Console` renders a frame as many small pieces. They collect here, under
//! the device's exclusive loan, and reach the device as one write the next
//! time anything else touches a standard stream.
use core::ptr;

use crate::platform_io::Failure;

/// Past this many waiting bytes the device is written before more are kept.
pub(crate) const LIMIT: usize = 1 << 20;
const INITIAL_CAPACITY: usize = 4096;

pub(crate) struct OutputBuffer {
    bytes: *mut u8,
    capacity: usize,
    length: usize,
    /// A write that failed while nobody was asking. The next console
    /// operation reports it.
    failure: Option<Failure>,
}

impl OutputBuffer {
    pub(crate) const fn new() -> Self {
        Self {
            bytes: ptr::null_mut(),
            capacity: 0,
            length: 0,
            failure: None,
        }
    }

    pub(crate) fn length(&self) -> usize {
        self.length
    }

    pub(crate) fn waiting(&self) -> (*const u8, usize) {
        (self.bytes, self.length)
    }

    pub(crate) fn clear(&mut self) {
        self.length = 0;
    }

    pub(crate) fn keep_failure(&mut self, failure: Failure) {
        self.failure.get_or_insert(failure);
    }

    pub(crate) fn take_failure(&mut self) -> Option<Failure> {
        self.failure.take()
    }

    /// Keeps `bytes` after what is already waiting. `false` leaves the buffer
    /// unchanged: the bytes would pass [`LIMIT`], or storage was refused.
    pub(crate) unsafe fn append(&mut self, bytes: &[u8]) -> bool {
        let Some(required) = self.length.checked_add(bytes.len()) else {
            return false;
        };
        if required > LIMIT {
            return false;
        }
        if required > self.capacity {
            let mut capacity = self.capacity.max(INITIAL_CAPACITY);
            while capacity < required {
                capacity *= 2;
            }
            let replacement = crate::allocate(capacity.min(LIMIT));
            if replacement.is_null() {
                return false;
            }
            if !self.bytes.is_null() {
                ptr::copy_nonoverlapping(self.bytes, replacement, self.length);
                crate::deallocate(self.bytes);
            }
            self.bytes = replacement;
            self.capacity = capacity.min(LIMIT);
        }
        ptr::copy_nonoverlapping(bytes.as_ptr(), self.bytes.add(self.length), bytes.len());
        self.length = required;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    unsafe fn waiting(buffer: &OutputBuffer) -> &[u8] {
        let (bytes, length) = buffer.waiting();
        if length == 0 {
            &[]
        } else {
            core::slice::from_raw_parts(bytes, length)
        }
    }

    #[test]
    fn pieces_collect_in_order_and_storage_is_reused_after_a_clear() {
        unsafe {
            let mut buffer = OutputBuffer::new();
            assert!(buffer.append(b"\x1b[2;3H"));
            assert!(buffer.append(b"text"));
            assert_eq!(waiting(&buffer), b"\x1b[2;3Htext");
            let storage = buffer.waiting().0;
            buffer.clear();
            let (_, allocations) = crate::probe_runtime_allocations(|| {
                assert!(buffer.append(&[b'x'; INITIAL_CAPACITY]));
            });
            assert_eq!(allocations, 0);
            assert_eq!(buffer.waiting().0, storage);
            assert!(buffer.append(b"y"));
            assert_eq!(buffer.length(), INITIAL_CAPACITY + 1);
            assert_eq!(waiting(&buffer)[INITIAL_CAPACITY], b'y');
            crate::deallocate(buffer.bytes);
        }
    }

    #[test]
    fn bytes_past_the_limit_are_refused_without_losing_what_waits() {
        unsafe {
            let mut buffer = OutputBuffer::new();
            assert!(buffer.append(b"kept"));
            let large = std::vec![b'x'; LIMIT];
            assert!(!buffer.append(&large));
            assert_eq!(waiting(&buffer), b"kept");
            buffer.clear();
            assert!(buffer.append(&large));
            assert!(!buffer.append(b"y"));
            assert_eq!(buffer.length(), LIMIT);
            crate::deallocate(buffer.bytes);
        }
    }

    #[test]
    fn the_first_unreported_failure_is_the_one_kept() {
        let mut buffer = OutputBuffer::new();
        buffer.keep_failure(Failure::from_system_code(5));
        buffer.keep_failure(Failure::from_system_code(28));
        assert_eq!(buffer.take_failure(), Some(Failure::from_system_code(5)));
        assert_eq!(buffer.take_failure(), None);
    }
}

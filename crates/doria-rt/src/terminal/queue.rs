//! Input read from the terminal and not yet handed out as keys. Every
//! platform reads bytes into this queue and takes keys out of it through the
//! shared decoder, so the escape delay and the order of keys are decided in
//! one place.
use core::ptr;

use super::decode::{self, Step};
use super::wire::Observation;
use crate::platform_io::Failure;
use crate::wait_budget::DrWaitBudgetV1;

/// Room for one read of input. Decoding empties the queue of whole keys
/// before the next read, so this only bounds how much type-ahead one read
/// takes.
pub(super) const READ_CAPACITY: usize = 4096;

pub(super) struct Queue {
    bytes: *mut u8,
    capacity: usize,
    start: usize,
    end: usize,
    /// When the bytes still queued stop being the start of a longer
    /// sequence: a lone Escape is a key once this passes.
    escape_deadline: u64,
    /// The terminal has gone away. What is queued is still delivered.
    pub(super) closed: bool,
}

impl Queue {
    /// `None` when storage is refused.
    pub(super) unsafe fn new(capacity: usize) -> Option<Self> {
        let bytes = crate::allocate(capacity);
        (!bytes.is_null()).then_some(Self {
            bytes,
            capacity,
            start: 0,
            end: 0,
            escape_deadline: 0,
            closed: false,
        })
    }

    pub(super) fn is_empty(&self) -> bool {
        self.start == self.end
    }

    pub(super) unsafe fn queued(&self) -> &[u8] {
        if self.is_empty() {
            &[]
        } else {
            core::slice::from_raw_parts(self.bytes.add(self.start), self.end - self.start)
        }
    }

    /// The free tail, after moving what is queued to the front. Empty only
    /// when the queue is full of bytes nothing has taken.
    pub(super) unsafe fn space(&mut self) -> (*mut u8, usize) {
        if self.start != 0 {
            let length = self.end - self.start;
            ptr::copy(self.bytes.add(self.start), self.bytes, length);
            self.start = 0;
            self.end = length;
        }
        (self.bytes.add(self.end), self.capacity - self.end)
    }

    /// `count` bytes were just read into [`Self::space`].
    pub(super) fn filled(&mut self, count: usize) -> Result<(), Failure> {
        self.end += count;
        self.escape_deadline =
            crate::monotonic_clock::now()?.saturating_add(decode::ESCAPE_DELAY_NANOSECONDS);
        Ok(())
    }

    /// Queues bytes that were read earlier. `false` when they do not fit.
    pub(super) unsafe fn push(&mut self, bytes: &[u8]) -> Result<bool, Failure> {
        let (space, available) = self.space();
        if bytes.len() > available {
            return Ok(false);
        }
        ptr::copy_nonoverlapping(bytes.as_ptr(), space, bytes.len());
        self.filled(bytes.len())?;
        Ok(true)
    }

    /// The next whole key, in the order typed. `None` when the queue is empty
    /// or holds only the start of a sequence that may still continue.
    pub(super) unsafe fn next_key(&mut self) -> Result<Option<Observation>, Failure> {
        loop {
            let queued = self.queued();
            if queued.is_empty() {
                return Ok(None);
            }
            let mut step = decode::decode(queued, self.closed);
            if matches!(step, Step::Incomplete)
                && crate::monotonic_clock::now()? >= self.escape_deadline
            {
                step = decode::decode(queued, true);
            }
            match step {
                Step::Key {
                    length,
                    key,
                    modifiers,
                } => {
                    self.start += length;
                    return Ok(Some(Observation::Key { key, modifiers }));
                }
                Step::Ignored { length } => self.start += length,
                Step::Incomplete => return Ok(None),
            }
        }
    }

    /// How long a wait for more input may last: the caller's budget, cut to
    /// the escape delay while a sequence is held back for it.
    pub(super) fn wait_limit(&self, budget: &DrWaitBudgetV1) -> Result<DrWaitBudgetV1, Failure> {
        if self.is_empty() {
            Ok(*budget)
        } else {
            Ok(budget.limited_by(DrWaitBudgetV1::new(3, self.escape_deadline, || Ok(0))?))
        }
    }

    /// A wait under [`Self::wait_limit`] timed out. True when it was the
    /// caller's budget that ended, not the escape delay.
    pub(super) fn caller_expired(&self) -> Result<bool, Failure> {
        Ok(self.is_empty() || crate::monotonic_clock::now()? < self.escape_deadline)
    }

    /// Takes the terminal's answer to a cursor-position query out of the
    /// queue. Keys typed around it stay, in order.
    #[cfg(any(target_os = "linux", target_os = "macos", test))]
    pub(super) unsafe fn take_cursor_report(&mut self) -> Option<(u32, u32)> {
        let report = decode::find_cursor_report(self.queued())?;
        let answer = self.start + report.start;
        let after = answer + report.length;
        ptr::copy(
            self.bytes.add(after),
            self.bytes.add(answer),
            self.end - after,
        );
        self.end -= report.length;
        Some((report.column, report.row))
    }
}

/// UTF-16 units from Windows key records, joined into Unicode scalars. A
/// character outside the Basic Multilingual Plane arrives as two records.
#[cfg(any(windows, test))]
#[derive(Default)]
pub(super) struct Utf16 {
    high: u16,
}

#[cfg(any(windows, test))]
impl Utf16 {
    /// Half a pair with no other half becomes U+FFFD, as broken UTF-8 does.
    pub(super) fn accept(&mut self, unit: u16, mut scalar: impl FnMut(char)) {
        let high = core::mem::take(&mut self.high);
        match unit {
            0xD800..=0xDBFF => {
                if high != 0 {
                    scalar(char::REPLACEMENT_CHARACTER);
                }
                self.high = unit;
            }
            0xDC00..=0xDFFF if high != 0 => {
                let joined =
                    0x1_0000 + ((u32::from(high) - 0xD800) << 10) + (u32::from(unit) - 0xDC00);
                scalar(char::from_u32(joined).unwrap_or(char::REPLACEMENT_CHARACTER));
            }
            _ => {
                if high != 0 {
                    scalar(char::REPLACEMENT_CHARACTER);
                }
                scalar(char::from_u32(u32::from(unit)).unwrap_or(char::REPLACEMENT_CHARACTER));
            }
        }
    }
}

impl Drop for Queue {
    fn drop(&mut self) {
        unsafe { crate::deallocate(self.bytes) }
    }
}

#[cfg(test)]
mod tests {
    use super::super::wire::Key;
    use super::*;

    fn key(character: char) -> Option<Observation> {
        Some(Observation::Key {
            key: Key::Character(character),
            modifiers: 0,
        })
    }

    #[test]
    fn keys_leave_in_the_order_typed_and_a_partial_sequence_waits() {
        unsafe {
            let mut queue = Queue::new(64).unwrap();
            assert_eq!(queue.next_key(), Ok(None));
            assert_eq!(queue.push(b"ab\x1b["), Ok(true));
            assert_eq!(queue.next_key(), Ok(key('a')));
            assert_eq!(queue.next_key(), Ok(key('b')));
            // Within the escape delay the sequence may still continue.
            assert_eq!(queue.next_key(), Ok(None));
            assert!(!queue.is_empty());
            assert_eq!(queue.caller_expired(), Ok(true));
            assert_eq!(queue.push(b"A"), Ok(true));
            assert_eq!(
                queue.next_key(),
                Ok(Some(Observation::Key {
                    key: Key::Up,
                    modifiers: 0
                }))
            );
            assert!(queue.is_empty());
        }
    }

    #[test]
    fn a_lone_escape_is_a_key_once_the_delay_has_passed_or_input_has_closed() {
        unsafe {
            let escape = Some(Observation::Key {
                key: Key::Escape,
                modifiers: 0,
            });
            let mut queue = Queue::new(64).unwrap();
            assert_eq!(queue.push(b"\x1b"), Ok(true));
            assert_eq!(queue.next_key(), Ok(None));
            queue.escape_deadline = 0;
            assert_eq!(queue.caller_expired(), Ok(false));
            assert_eq!(queue.next_key(), Ok(escape));

            assert_eq!(queue.push(b"\x1b"), Ok(true));
            queue.closed = true;
            assert_eq!(queue.next_key(), Ok(escape));
        }
    }

    #[test]
    fn a_cursor_report_leaves_the_keys_around_it_queued() {
        unsafe {
            let mut queue = Queue::new(64).unwrap();
            assert_eq!(queue.push(b"a\x1b[5;10Rb"), Ok(true));
            assert_eq!(queue.take_cursor_report(), Some((9, 4)));
            assert_eq!(queue.take_cursor_report(), None);
            assert_eq!(queue.queued(), b"ab");
        }
    }

    #[test]
    fn utf16_units_join_into_scalars_and_a_lone_half_is_replaced() {
        let mut joined = std::string::String::new();
        let mut units = Utf16::default();
        let typed: std::vec::Vec<u16> = "aé🎮z".encode_utf16().collect();
        for unit in typed {
            units.accept(unit, |scalar| joined.push(scalar));
        }
        assert_eq!(joined, "aé🎮z");

        joined.clear();
        // A low half alone, a high half followed by a letter, two high halves.
        for unit in [0xDC00, 0xD83C, u16::from(b'x'), 0xD83C, 0xD83C, 0xDFAE] {
            units.accept(unit, |scalar| joined.push(scalar));
        }
        assert_eq!(joined, "\u{FFFD}\u{FFFD}x\u{FFFD}🎮");
    }

    #[test]
    fn bytes_that_do_not_fit_are_refused_whole() {
        unsafe {
            let mut queue = Queue::new(4).unwrap();
            assert_eq!(queue.push(b"abc"), Ok(true));
            assert_eq!(queue.push(b"de"), Ok(false));
            assert_eq!(queue.queued(), b"abc");
            assert_eq!(queue.next_key(), Ok(key('a')));
            // Space taken from the front is space again at the back.
            assert_eq!(queue.push(b"de"), Ok(true));
            assert_eq!(queue.queued(), b"bcde");
            assert_eq!(queue.space().1, 0);
        }
    }
}

//! Exact private monotonic instants. Expiry is a wait observation, not a value.

/// Signed two's-complement nanoseconds in two inline words, low word first.
/// This is private compiler/runtime transport, never a public clock epoch.
#[repr(C, align(8))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DrDeadlineV1 {
    low: u64,
    high: i64,
}

impl DrDeadlineV1 {
    pub(crate) const ZERO: Self = Self::from_parts(0, 0);

    pub(crate) const fn from_parts(low: u64, high: i64) -> Self {
        Self { low, high }
    }

    pub(crate) fn from_clock_and_duration(now: u64, nanoseconds: i64) -> Self {
        // Every u64 clock plus every signed int64 interval fits exactly in i128.
        // In particular, past values and sums above u64::MAX retain their bits.
        let instant = doria_time::deadline_from_sample(now, nanoseconds);
        Self::from_parts(instant as u64, (instant >> 64) as i64)
    }

    fn instant(self) -> i128 {
        (i128::from(self.high) << 64) | i128::from(self.low)
    }

    pub(crate) fn remaining_at(self, now: u64) -> Option<u128> {
        let instant = self.instant();
        let now = i128::from(now);
        // Compare before subtracting: even an arbitrary private negative carrier
        // cannot underflow, and no expired instant is rewritten or saturated.
        (instant > now).then(|| (instant - now) as u128)
    }
}

impl Ord for DrDeadlineV1 {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.high
            .cmp(&other.high)
            .then_with(|| self.low.cmp(&other.low))
    }
}

impl PartialOrd for DrDeadlineV1 {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Samples the shared native clock once and writes an exact two-word Deadline.
/// Clock errors carry their original optional system code, never I/O reason tags.
///
/// # Safety
/// Outputs must denote distinct live writable storage of their declared types.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_deadline_from_now(
    duration: i64,
    low: *mut u64,
    high: *mut i64,
    system_code: *mut i64,
    has_system_code: *mut u8,
) -> u8 {
    write_deadline_result(
        doria_time::deadline_from_now(duration),
        low,
        high,
        system_code,
        has_system_code,
    )
}

unsafe fn write_deadline_result(
    result: Result<i128, doria_time::ClockFailure>,
    low: *mut u64,
    high: *mut i64,
    system_code: *mut i64,
    has_system_code: *mut u8,
) -> u8 {
    *low = 0;
    *high = 0;
    *system_code = 0;
    *has_system_code = 0;
    match result {
        Ok(instant) => {
            *low = instant as u64;
            *high = (instant >> 64) as i64;
            0
        }
        Err(failure) => {
            if let Some(code) = failure.system_code() {
                *system_code = code;
                *has_system_code = 1;
            }
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_carrier_has_two_words_and_one_portable_alignment() {
        assert_eq!(core::mem::size_of::<DrDeadlineV1>(), 16);
        assert_eq!(core::mem::align_of::<DrDeadlineV1>(), 8);
        assert_eq!(core::mem::offset_of!(DrDeadlineV1, low), 0);
        assert_eq!(core::mem::offset_of!(DrDeadlineV1, high), 8);
    }

    #[test]
    fn entire_signed_duration_and_unsigned_clock_ranges_fit_without_allocation() {
        let (_, allocations) = crate::probe_runtime_allocations(|| {
            for now in [0, 1, i64::MAX as u64, 1_u64 << 63, u64::MAX] {
                for duration in [i64::MIN, -1, 0, 1, i64::MAX] {
                    let deadline = DrDeadlineV1::from_clock_and_duration(now, duration);
                    let expected = i128::from(now) + i128::from(duration);
                    assert_eq!(deadline.instant(), expected);
                    assert_eq!(
                        DrDeadlineV1::from_parts(deadline.low, deadline.high),
                        deadline
                    );
                }
            }
        });
        assert_eq!(allocations, 0);
    }

    #[test]
    fn distinct_past_instants_keep_identity_order_and_difference_after_observation() {
        let earliest = DrDeadlineV1::from_clock_and_duration(0, i64::MIN);
        let earlier = DrDeadlineV1::from_clock_and_duration(0, -2);
        let later = DrDeadlineV1::from_clock_and_duration(0, -1);
        assert!(earliest < earlier && earlier < later && later < DrDeadlineV1::ZERO);
        assert_ne!(earlier, later);
        assert_eq!(later.instant() - earlier.instant(), 1);
        for now in [0, 1, u64::MAX] {
            assert_eq!(earliest.remaining_at(now), None);
            assert_eq!(earlier.remaining_at(now), None);
            assert_eq!(later.remaining_at(now), None);
        }
        assert_ne!(earlier, later);
        assert_eq!(later.instant() - earlier.instant(), 1);
    }

    #[test]
    fn observation_handles_the_whole_private_carrier_without_wrapping() {
        assert_eq!(
            DrDeadlineV1::from_parts(0, i64::MIN).remaining_at(u64::MAX),
            None
        );
        let latest = DrDeadlineV1::from_parts(u64::MAX, i64::MAX);
        assert_eq!(latest.remaining_at(0), Some(i128::MAX as u128));
        assert_eq!(
            latest.remaining_at(u64::MAX),
            Some(i128::MAX as u128 - u128::from(u64::MAX))
        );
        let maximum_clock = DrDeadlineV1::from_clock_and_duration(u64::MAX, 0);
        let later = DrDeadlineV1::from_clock_and_duration(u64::MAX, 1);
        assert!(maximum_clock < later);
        assert_eq!(later.remaining_at(u64::MAX), Some(1));
    }

    #[test]
    fn factory_word_transport_is_exact_and_clears_stale_error_outputs() {
        for instant in [i128::MIN, -2, -1, 0, 1_i128 << 64, i128::MAX] {
            let (mut low, mut high, mut code, mut has_code) = (u64::MAX, -1, -1, 1);
            let (_, allocations) = crate::probe_runtime_allocations(|| unsafe {
                assert_eq!(
                    write_deadline_result(
                        Ok(instant),
                        &mut low,
                        &mut high,
                        &mut code,
                        &mut has_code,
                    ),
                    0
                );
            });
            assert_eq!(allocations, 0);
            assert_eq!(DrDeadlineV1::from_parts(low, high).instant(), instant);
            assert_eq!((code, has_code), (0, 0));
        }
    }

    #[test]
    fn factory_failure_clears_words_and_preserves_original_optional_code() {
        for failure in [
            doria_time::ClockFailure::System { system_code: 0 },
            doria_time::ClockFailure::System { system_code: 13 },
            doria_time::ClockFailure::System {
                system_code: i64::MAX,
            },
            doria_time::ClockFailure::InvalidReading,
            doria_time::ClockFailure::Unsupported,
        ] {
            let (mut low, mut high, mut code, mut has_code) = (u64::MAX, -1, -1, 1);
            assert_eq!(
                unsafe {
                    write_deadline_result(
                        Err(failure),
                        &mut low,
                        &mut high,
                        &mut code,
                        &mut has_code,
                    )
                },
                1
            );
            assert_eq!((low, high), (0, 0));
            assert_eq!(code, failure.system_code().unwrap_or(0));
            assert_eq!(has_code, u8::from(failure.system_code().is_some()));
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    #[test]
    fn native_factory_retains_the_shared_epoch_without_success_allocation() {
        for duration in [i64::MIN, -1, 0, 1, i64::MAX] {
            let before = doria_time::monotonic_nanoseconds().unwrap();
            let (mut low, mut high, mut code, mut has_code) = (u64::MAX, -1, -1, 1);
            let (_, allocations) = crate::probe_runtime_allocations(|| unsafe {
                assert_eq!(
                    dr_v1_deadline_from_now(
                        duration,
                        &mut low,
                        &mut high,
                        &mut code,
                        &mut has_code,
                    ),
                    0
                );
            });
            let after = doria_time::monotonic_nanoseconds().unwrap();
            let deadline = DrDeadlineV1::from_parts(low, high);
            assert!(DrDeadlineV1::from_clock_and_duration(before, duration) <= deadline);
            assert!(deadline <= DrDeadlineV1::from_clock_and_duration(after, duration));
            assert_eq!((code, has_code), (0, 0));
            assert_eq!(allocations, 0);
        }
    }
}

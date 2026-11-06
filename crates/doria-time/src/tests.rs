use super::*;
use core::cell::Cell;

#[test]
fn clock_failures_preserve_original_optional_codes_without_io_classification() {
    for system_code in [i64::MIN, -1, 0, 4, 13, i64::from(u32::MAX), i64::MAX] {
        let failure = ClockFailure::System { system_code };
        assert_eq!(failure.system_code(), Some(system_code));
    }
    assert_eq!(ClockFailure::InvalidReading.system_code(), None);
    assert_eq!(ClockFailure::Unsupported.system_code(), None);
}

#[test]
fn integer_tick_conversion_keeps_subsecond_precision_and_checks_overflow() {
    assert_eq!(ticks_to_nanoseconds(1, 10_000_000), Ok(100));
    assert_eq!(
        ticks_to_nanoseconds(123_456_789, 10_000_000),
        Ok(12_345_678_900)
    );
    assert_eq!(ticks_to_nanoseconds(1, 3), Ok(333_333_333));
    assert_eq!(
        ticks_to_nanoseconds(u64::MAX, u64::MAX),
        Ok(NANOS_PER_SECOND)
    );
    assert_eq!(
        ticks_to_nanoseconds(u64::MAX, 1),
        Err(ClockFailure::InvalidReading)
    );
    assert_eq!(
        ticks_to_nanoseconds(0, 0),
        Err(ClockFailure::InvalidReading)
    );
}

#[test]
fn timespec_validation_preserves_exact_nanoseconds_and_rejects_invalid_readings() {
    assert_eq!(timespec_nanoseconds(0, 0), Ok(0));
    assert_eq!(timespec_nanoseconds(12, 345_678_900), Ok(12_345_678_900));
    let maximum_seconds = u64::MAX / NANOS_PER_SECOND;
    let maximum_nanos = u64::MAX % NANOS_PER_SECOND;
    assert_eq!(
        timespec_nanoseconds(i128::from(maximum_seconds), i128::from(maximum_nanos)),
        Ok(u64::MAX)
    );
    for (seconds, nanos) in [
        (-1, 0),
        (0, -1),
        (0, i128::from(NANOS_PER_SECOND)),
        (i128::from(maximum_seconds) + 1, 0),
        (i128::from(maximum_seconds), i128::from(maximum_nanos) + 1),
        (i128::MAX, 0),
        (0, i128::MAX),
    ] {
        assert_eq!(
            timespec_nanoseconds(seconds, nanos),
            Err(ClockFailure::InvalidReading)
        );
    }
}

#[test]
fn every_signed_duration_and_unsigned_clock_sample_retains_an_exact_deadline() {
    for now in [0, 1, i64::MAX as u64, 1_u64 << 63, u64::MAX] {
        for duration in [i64::MIN, -1, 0, 1, i64::MAX] {
            let samples = Cell::new(0);
            let deadline = deadline_with_clock(duration, || {
                samples.set(samples.get() + 1);
                Ok(now)
            });
            assert_eq!(deadline, Ok(i128::from(now) + i128::from(duration)));
            assert_eq!(samples.get(), 1);
        }
    }
    assert_ne!(deadline_from_sample(0, -2), deadline_from_sample(0, -1));
    assert!(deadline_from_sample(0, -2) < deadline_from_sample(0, -1));
    assert_eq!(deadline_from_sample(u64::MAX, 1), 1_i128 << 64);
}

#[test]
fn deadline_clock_failures_are_not_retried_or_replaced() {
    for failure in [
        ClockFailure::System { system_code: 0 },
        ClockFailure::System { system_code: 13 },
        ClockFailure::InvalidReading,
        ClockFailure::Unsupported,
    ] {
        for duration in [i64::MIN, -1, 0, i64::MAX] {
            let samples = Cell::new(0);
            assert_eq!(
                deadline_with_clock(duration, || {
                    samples.set(samples.get() + 1);
                    Err(failure)
                }),
                Err(failure)
            );
            assert_eq!(samples.get(), 1);
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[test]
fn native_clock_is_monotonic_and_factory_uses_the_same_epoch() {
    for duration in [i64::MIN, -1, 0, 1, i64::MAX] {
        let before = monotonic_nanoseconds().unwrap();
        let deadline = deadline_from_now(duration).unwrap();
        let after = monotonic_nanoseconds().unwrap();
        assert!(before <= after);
        assert!(deadline_from_sample(before, duration) <= deadline);
        assert!(deadline <= deadline_from_sample(after, duration));
    }
}

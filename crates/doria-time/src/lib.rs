#![no_std]

//! Private native clock substrate shared by the compiler and both runtime
//! profiles. No public clock epoch, allocation, timer, or readiness state.

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios",
    windows,
    test
))]
const NANOS_PER_SECOND: u64 = 1_000_000_000;

pub const CLOCK_FAILURE_MESSAGE: &str = "The monotonic clock could not be read.";

/// Clock facts are independent of I/O classification. A known zero system code
/// is preserved as Some(0), not confused with a missing platform code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockFailure {
    System { system_code: i64 },
    InvalidReading,
    Unsupported,
}

impl ClockFailure {
    pub const fn system_code(self) -> Option<i64> {
        match self {
            Self::System { system_code } => Some(system_code),
            Self::InvalidReading | Self::Unsupported => None,
        }
    }
}

/// Samples the private monotonic clock exactly once, then retains the exact
/// signed instant even for negative intervals and sums above u64::MAX.
pub fn deadline_from_now(duration: i64) -> Result<i128, ClockFailure> {
    deadline_with_clock(duration, monotonic_nanoseconds)
}

/// Exact private arithmetic for an already-sampled clock, including frozen
/// wait budgets. This does not expose the epoch at the Doria language boundary.
pub const fn deadline_from_sample(now: u64, duration: i64) -> i128 {
    now as i128 + duration as i128
}

fn deadline_with_clock(
    duration: i64,
    now: impl FnOnce() -> Result<u64, ClockFailure>,
) -> Result<i128, ClockFailure> {
    now().map(|sample| deadline_from_sample(sample, duration))
}

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios"
))]
pub fn monotonic_nanoseconds() -> Result<u64, ClockFailure> {
    // libc owns time_t width, padding, clock IDs, and time64 symbol selection.
    let mut time = core::mem::MaybeUninit::<libc::timespec>::uninit();
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, time.as_mut_ptr()) } != 0 {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        let system_code = unsafe { i64::from(*libc::__errno_location()) };
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        let system_code = unsafe { i64::from(*libc::__error()) };
        return Err(ClockFailure::System { system_code });
    }
    let time = unsafe { time.assume_init() };
    timespec_nanoseconds(i128::from(time.tv_sec), i128::from(time.tv_nsec))
}

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios",
    test
))]
fn timespec_nanoseconds(seconds: i128, nanos: i128) -> Result<u64, ClockFailure> {
    let seconds = u64::try_from(seconds).map_err(|_| ClockFailure::InvalidReading)?;
    let nanos = u64::try_from(nanos).map_err(|_| ClockFailure::InvalidReading)?;
    if nanos >= NANOS_PER_SECOND {
        return Err(ClockFailure::InvalidReading);
    }
    seconds
        .checked_mul(NANOS_PER_SECOND)
        .and_then(|seconds| seconds.checked_add(nanos))
        .ok_or(ClockFailure::InvalidReading)
}

#[cfg(windows)]
pub fn monotonic_nanoseconds() -> Result<u64, ClockFailure> {
    use core::sync::atomic::{AtomicU64, Ordering};

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn QueryPerformanceCounter(counter: *mut i64) -> i32;
        fn QueryPerformanceFrequency(frequency: *mut i64) -> i32;
        fn GetLastError() -> u32;
    }

    // QPC frequency is fixed for the boot session. Keep the existing lazy,
    // allocation-free initialization; untimed operations do not query it.
    static FREQUENCY: AtomicU64 = AtomicU64::new(0);
    let mut frequency = FREQUENCY.load(Ordering::Relaxed);
    if frequency == 0 {
        let mut measured = 0;
        if unsafe { QueryPerformanceFrequency(&mut measured) } == 0 {
            return Err(ClockFailure::System {
                system_code: unsafe { i64::from(GetLastError()) },
            });
        }
        frequency = u64::try_from(measured)
            .ok()
            .filter(|value| *value != 0)
            .ok_or(ClockFailure::InvalidReading)?;
        FREQUENCY.store(frequency, Ordering::Relaxed);
    }
    let mut ticks = 0;
    if unsafe { QueryPerformanceCounter(&mut ticks) } == 0 {
        return Err(ClockFailure::System {
            system_code: unsafe { i64::from(GetLastError()) },
        });
    }
    let ticks = u64::try_from(ticks).map_err(|_| ClockFailure::InvalidReading)?;
    ticks_to_nanoseconds(ticks, frequency)
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios",
    windows
)))]
pub fn monotonic_nanoseconds() -> Result<u64, ClockFailure> {
    Err(ClockFailure::Unsupported)
}

#[cfg(any(windows, test))]
fn ticks_to_nanoseconds(ticks: u64, frequency: u64) -> Result<u64, ClockFailure> {
    if frequency == 0 {
        return Err(ClockFailure::InvalidReading);
    }
    let nanos = u128::from(ticks) * u128::from(NANOS_PER_SECOND) / u128::from(frequency);
    u64::try_from(nanos).map_err(|_| ClockFailure::InvalidReading)
}

#[cfg(test)]
mod tests;

use crate::platform_io::Failure;

/// Reads the shared operation/deadline clock without initializing any timer or
/// readiness infrastructure. The epoch is private and never denotes wall time.
///
/// # Safety
/// Each output must point to live writable storage of its declared type.
/// Returns zero for success; on failure, `reason` uses the private I/O reason
/// tags, and `has_system_code` distinguishes an OS code from no available code.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_monotonic_clock(
    nanoseconds: *mut u64,
    reason: *mut u8,
    system_code: *mut i64,
    has_system_code: *mut u8,
) -> u8 {
    *nanoseconds = 0;
    *reason = 0;
    *system_code = 0;
    *has_system_code = 0;
    match now() {
        Ok(value) => {
            *nanoseconds = value;
            0
        }
        Err(failure) => {
            *reason = failure.reason as u8;
            if let Some(code) = failure.system_code {
                *system_code = code;
                *has_system_code = 1;
            }
            1
        }
    }
}

pub(crate) fn now() -> Result<u64, Failure> {
    #[cfg(test)]
    crate::io_cost_probe::record(crate::io_cost_probe::Operation::ClockRead);
    doria_time::monotonic_nanoseconds().map_err(map_clock_failure)
}

fn map_clock_failure(failure: doria_time::ClockFailure) -> Failure {
    match failure {
        doria_time::ClockFailure::System { system_code } => Failure::from_system_code(system_code),
        doria_time::ClockFailure::InvalidReading => Failure::other(),
        doria_time::ClockFailure::Unsupported => Failure::unsupported(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_fact_adapter_retains_existing_io_classification_and_codes() {
        for system_code in [i64::MIN, -1, 0, 4, 13, i64::from(u32::MAX), i64::MAX] {
            assert_eq!(
                map_clock_failure(doria_time::ClockFailure::System { system_code }),
                Failure::from_system_code(system_code)
            );
        }
        assert_eq!(
            map_clock_failure(doria_time::ClockFailure::InvalidReading),
            Failure::other()
        );
        assert_eq!(
            map_clock_failure(doria_time::ClockFailure::Unsupported),
            Failure::unsupported()
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    #[test]
    fn native_clock_is_monotonic_and_success_clears_failure_outputs() {
        let before = now().unwrap();
        let (mut time, mut reason, mut code, mut has_code) = (0, 255, -1, 1);
        assert_eq!(
            unsafe { dr_v1_monotonic_clock(&mut time, &mut reason, &mut code, &mut has_code) },
            0
        );
        let after = now().unwrap();
        assert!(before <= time && time <= after);
        assert_eq!((reason, code, has_code), (0, 0, 0));
    }
}

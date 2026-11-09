#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Reason {
    NotFound = 0,
    PermissionDenied = 1,
    InvalidInput = 2,
    Interrupted = 3,
    ResourceExhausted = 4,
    Unsupported = 5,
    Closed = 6,
    Other = 7,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Failure {
    pub(crate) reason: Reason,
    pub(crate) system_code: Option<i64>,
    pub(crate) transferred_byte_count: Option<usize>,
}

impl Failure {
    pub(crate) const fn from_system_code(code: i64) -> Self {
        Self {
            reason: classify(code),
            system_code: Some(code),
            transferred_byte_count: None,
        }
    }

    pub(crate) const fn invalid_input() -> Self {
        Self {
            reason: Reason::InvalidInput,
            system_code: None,
            transferred_byte_count: None,
        }
    }

    pub(crate) const fn unsupported() -> Self {
        Self {
            reason: Reason::Unsupported,
            system_code: None,
            transferred_byte_count: None,
        }
    }

    pub(crate) const fn other() -> Self {
        Self {
            reason: Reason::Other,
            system_code: None,
            transferred_byte_count: None,
        }
    }

    /// Sets progress only when the operation owning the transfer knows it.
    pub(crate) const fn with_transferred_byte_count(mut self, count: Option<usize>) -> Self {
        self.transferred_byte_count = count;
        self
    }

    /// Adds an already transferred prefix without turning unknown progress into zero.
    pub(crate) fn after_transferred(self, prefix: usize) -> Self {
        self.with_transferred_byte_count(
            self.transferred_byte_count
                .and_then(|count| prefix.checked_add(count)),
        )
    }
}

#[cfg(any(unix, windows))]
pub(crate) fn last_failure() -> Failure {
    Failure::from_system_code(last_system_code())
}

#[cfg(unix)]
pub(crate) fn last_system_code() -> i64 {
    unsafe extern "C" {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        fn __errno_location() -> *mut i32;
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        fn __error() -> *mut i32;
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    unsafe {
        i64::from(*__errno_location())
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    unsafe {
        i64::from(*__error())
    }
}

#[cfg(windows)]
pub(crate) fn last_system_code() -> i64 {
    use crate::win32::GetLastError;
    unsafe { i64::from(GetLastError()) }
}

#[cfg(unix)]
const fn classify(code: i64) -> Reason {
    if code == 2 {
        Reason::NotFound
    } else if code == 1 || code == 13 {
        Reason::PermissionDenied
    } else if code == 22 || is_name_too_long(code) {
        Reason::InvalidInput
    } else if code == 4 {
        Reason::Interrupted
    } else if matches!(code, 12 | 23 | 24 | 28) {
        Reason::ResourceExhausted
    } else if is_unsupported(code) {
        Reason::Unsupported
    } else if code == 9 || code == 32 {
        Reason::Closed
    } else {
        Reason::Other
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
const fn is_name_too_long(code: i64) -> bool {
    code == 36
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
const fn is_name_too_long(code: i64) -> bool {
    code == 63
}

#[cfg(all(
    unix,
    not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    ))
))]
const fn is_name_too_long(_code: i64) -> bool {
    false
}

#[cfg(any(target_os = "linux", target_os = "android"))]
const fn is_unsupported(code: i64) -> bool {
    code == 38 || code == 95
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
const fn is_unsupported(code: i64) -> bool {
    code == 45 || code == 78
}

#[cfg(all(
    unix,
    not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    ))
))]
const fn is_unsupported(_code: i64) -> bool {
    false
}

#[cfg(windows)]
const fn classify(code: i64) -> Reason {
    match code {
        2 | 3 => Reason::NotFound,
        5 | 32 => Reason::PermissionDenied,
        1 | 87 | 123 | 206 => Reason::InvalidInput,
        995 => Reason::Interrupted,
        4 | 8 | 14 | 39 | 112 => Reason::ResourceExhausted,
        50 => Reason::Unsupported,
        6 | 109 | 232 => Reason::Closed,
        _ => Reason::Other,
    }
}

#[cfg(not(any(unix, windows)))]
const fn classify(_code: i64) -> Reason {
    Reason::Other
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_failures_have_no_host_code() {
        assert_eq!(Failure::invalid_input().system_code, None);
    }

    #[test]
    fn progress_preserves_absent_zero_positive_and_overflow() {
        let failure = Failure::from_system_code(13);
        assert_eq!(failure.transferred_byte_count, None);
        assert_eq!(failure.after_transferred(9), failure);
        for (raw, prefix, expected) in [
            (0, 0, Some(0)),
            (0, 9, Some(9)),
            (3, 9, Some(12)),
            (1, usize::MAX, None),
        ] {
            let accumulated = failure
                .with_transferred_byte_count(Some(raw))
                .after_transferred(prefix);
            assert_eq!(accumulated.transferred_byte_count, expected);
            assert_eq!(accumulated.reason, failure.reason);
            assert_eq!(accumulated.system_code, failure.system_code);
        }
        assert_eq!(Failure::invalid_input().transferred_byte_count, None);
        assert_eq!(Failure::other().transferred_byte_count, None);
    }

    #[cfg(not(any(unix, windows)))]
    #[test]
    fn unsupported_failures_have_no_host_code() {
        assert_eq!(Failure::unsupported().system_code, None);
    }

    #[cfg(unix)]
    #[test]
    fn unix_reason_families_are_stable() {
        assert_eq!(Failure::from_system_code(2).reason, Reason::NotFound);
        assert_eq!(
            Failure::from_system_code(13).reason,
            Reason::PermissionDenied
        );
        assert_eq!(Failure::from_system_code(4).reason, Reason::Interrupted);
        assert_eq!(
            Failure::from_system_code(28).reason,
            Reason::ResourceExhausted
        );
        assert_eq!(Failure::from_system_code(9).reason, Reason::Closed);
    }

    #[cfg(windows)]
    #[test]
    fn windows_reason_families_are_stable() {
        assert_eq!(Failure::from_system_code(2).reason, Reason::NotFound);
        assert_eq!(
            Failure::from_system_code(5).reason,
            Reason::PermissionDenied
        );
        assert_eq!(Failure::from_system_code(995).reason, Reason::Interrupted);
        assert_eq!(
            Failure::from_system_code(112).reason,
            Reason::ResourceExhausted
        );
        assert_eq!(Failure::from_system_code(6).reason, Reason::Closed);
    }
}

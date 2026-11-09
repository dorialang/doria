//! Test-only observations at shared timing/readiness/cancellation owners.

#[derive(Clone, Copy)]
pub(crate) enum Operation {
    ClockRead,
    ReadinessCreate,
    CompletionBrokerAcquire,
    HelperSubmit,
    CancellationCreate,
    SignalCreate,
}

const COUNT: usize = Operation::SignalCreate as usize + 1;

std::thread_local! {
    static COUNTS: std::cell::Cell<Option<[usize; COUNT]>> = const { std::cell::Cell::new(None) };
}

pub(crate) fn record(operation: Operation) {
    COUNTS.with(|counts| {
        if let Some(mut value) = counts.get() {
            value[operation as usize] += 1;
            counts.set(Some(value));
        }
    });
}

/// Nested intervals preserve their caller's inclusive total. Other test
/// threads and non-test artifacts have no observation state.
pub(crate) fn measure<R>(operation: impl FnOnce() -> R) -> (R, [usize; COUNT]) {
    struct Probe(bool);
    impl Drop for Probe {
        fn drop(&mut self) {
            if self.0 {
                COUNTS.with(|counts| counts.set(None));
            }
        }
    }
    let (baseline, outermost) = COUNTS.with(|counts| match counts.get() {
        Some(value) => (value, false),
        None => {
            counts.set(Some([0; COUNT]));
            ([0; COUNT], true)
        }
    });
    let probe = Probe(outermost);
    let result = operation();
    let total = COUNTS.with(|counts| counts.get().unwrap());
    let interval = std::array::from_fn(|index| total[index] - baseline[index]);
    drop(probe);
    (result, interval)
}

#[test]
fn nested_intervals_and_unwind_preserve_scoped_counts() {
    let ((), total) = measure(|| {
        record(Operation::ClockRead);
        let ((), inner) = measure(|| {
            record(Operation::ReadinessCreate);
            record(Operation::CompletionBrokerAcquire);
            record(Operation::HelperSubmit);
            record(Operation::CancellationCreate);
            record(Operation::SignalCreate);
        });
        assert_eq!(inner, [0, 1, 1, 1, 1, 1]);
        assert!(crate::test_report::expect_panic(|| measure(|| {
            record(Operation::ClockRead);
            panic!("probe cleanup");
        }))
        .is_err());
        assert_eq!(
            std::thread::spawn(|| measure(|| record(Operation::ClockRead)).1)
                .join()
                .unwrap(),
            [1, 0, 0, 0, 0, 0]
        );
    });
    assert_eq!(total, [2, 1, 1, 1, 1, 1]);
    assert!(
        crate::test_report::expect_panic(|| measure(|| panic!("outer probe cleanup"))).is_err()
    );
    assert_eq!(measure(|| ()).1, [0; COUNT]);
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[test]
fn real_clock_cancellation_and_readiness_owners_are_observed() {
    let ((), counts) = measure(|| unsafe {
        crate::monotonic_clock::now().unwrap();
        let state = crate::cancellation::dr_v1_cancellation_new();
        assert!(!state.is_null());
        let signal = crate::cancellation_signal::Signal::new().unwrap();
        drop(signal);
        crate::cancellation::dr_v1_cancellation_release(state);
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let mut status = crate::io_status::DrIoStatusV1::default();
            let set = crate::unix_readiness::dr_v1_unix_readiness_new(
                1,
                core::ptr::null_mut(),
                &mut status,
            );
            assert!(!set.is_null());
            crate::unix_readiness::dr_v1_unix_readiness_drop(set);
        }
    });
    assert_eq!(counts[Operation::ClockRead as usize], 1);
    assert_eq!(counts[Operation::CancellationCreate as usize], 1);
    assert_eq!(counts[Operation::SignalCreate as usize], 1);
    assert_eq!(counts[Operation::HelperSubmit as usize], 0);
    assert_eq!(counts[Operation::CompletionBrokerAcquire as usize], 0);
    assert_eq!(
        counts[Operation::ReadinessCreate as usize],
        usize::from(cfg!(any(target_os = "linux", target_os = "macos")))
    );
}

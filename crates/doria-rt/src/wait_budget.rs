use crate::{deadline::DrDeadlineV1, platform_io::Failure};

const IMMEDIATE: u64 = 0;
const INDEFINITE: u64 = 1;
const DEADLINE: u64 = 2;
const EXPIRED: u64 = 3;

/// Private inline transport, shared by operations and readiness waits. A relative
/// duration becomes one deadline before any retry, never a fresh wait each time.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct DrWaitBudgetV1 {
    kind: u64,
    deadline: DrDeadlineV1,
}

impl DrWaitBudgetV1 {
    pub(crate) fn from_wait(kind: u8, low: u64, high: i64) -> Result<Self, Failure> {
        if kind == 3 {
            Ok(Self::from_deadline(DrDeadlineV1::from_parts(low, high)))
        } else {
            Self::new(kind, low, crate::monotonic_clock::now)
        }
    }

    pub(crate) const fn indefinite() -> Self {
        Self {
            kind: INDEFINITE,
            deadline: DrDeadlineV1::ZERO,
        }
    }

    /// Intersect already-frozen budgets without querying the clock or retaining
    /// either caller's storage. Relative durations have already become deadlines.
    pub(crate) fn limited_by(self, other: Self) -> Self {
        match (self.kind, other.kind) {
            (EXPIRED, _) => self,
            (_, EXPIRED) => other,
            (IMMEDIATE, _) => self,
            (_, IMMEDIATE) => other,
            (INDEFINITE, _) => other,
            (_, INDEFINITE) => self,
            (DEADLINE, DEADLINE) if self.deadline <= other.deadline => self,
            (DEADLINE, DEADLINE) => other,
            _ => unsafe { crate::runtime_invariant_panic() },
        }
    }

    pub(crate) fn new(
        kind: u8,
        nanoseconds: u64,
        now: impl FnOnce() -> Result<u64, Failure>,
    ) -> Result<Self, Failure> {
        let (kind, deadline) = match kind {
            0 => (IMMEDIATE, DrDeadlineV1::ZERO),
            1 => (INDEFINITE, DrDeadlineV1::ZERO),
            2 if (nanoseconds as i64) < 0 => (EXPIRED, DrDeadlineV1::ZERO),
            2 => (
                DEADLINE,
                DrDeadlineV1::from_clock_and_duration(now()?, nanoseconds as i64),
            ),
            3 => (
                DEADLINE,
                DrDeadlineV1::from_clock_and_duration(nanoseconds, 0),
            ),
            _ => return Err(Failure::invalid_input()),
        };
        Ok(Self { kind, deadline })
    }

    pub(crate) fn from_deadline(deadline: DrDeadlineV1) -> Self {
        Self {
            kind: DEADLINE,
            deadline,
        }
    }

    pub(crate) fn remaining(
        &self,
        unit_nanoseconds: u64,
        maximum_units: u64,
        now: impl FnOnce() -> Result<u64, Failure>,
    ) -> Result<(u8, u64), Failure> {
        match self.kind {
            IMMEDIATE => Ok((IMMEDIATE as u8, 0)),
            INDEFINITE => Ok((INDEFINITE as u8, 0)),
            EXPIRED => Ok((EXPIRED as u8, 0)),
            DEADLINE => {
                if unit_nanoseconds == 0 || maximum_units == 0 {
                    return Err(Failure::invalid_input());
                }
                let Some(remaining) = self.deadline.remaining_at(now()?) else {
                    return Ok((EXPIRED as u8, 0));
                };
                // Round up to the platform's resolution without overflowing;
                // a clamped native timeout is not proof the deadline elapsed.
                let unit = u128::from(unit_nanoseconds);
                let units = remaining.div_ceil(unit);
                Ok((DEADLINE as u8, units.min(u128::from(maximum_units)) as u64))
            }
            _ => Err(Failure::invalid_input()),
        }
    }
}

fn write_failure(failure: Failure, reason: &mut u8, code: &mut i64, has_code: &mut u8) -> u8 {
    *reason = failure.reason as u8;
    if let Some(value) = failure.system_code {
        *code = value;
        *has_code = 1;
    }
    1
}

/// Freezes an operation's wait budget. Private input tags are 0 immediate,
/// 1 indefinite, 2 signed relative nanoseconds carried as int64 bits, and
/// 3 unsigned absolute monotonic nanoseconds. Negative relative intervals are
/// already expired, never an immediate poll or an indefinite wait.
///
/// # Safety
/// Output pointers must denote distinct live writable storage of their declared
/// types. On failure the budget is immediate, not partially initialized.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_wait_budget_init(
    kind: u8,
    nanoseconds: u64,
    budget: *mut DrWaitBudgetV1,
    reason: *mut u8,
    system_code: *mut i64,
    has_system_code: *mut u8,
) -> u8 {
    budget.write(DrWaitBudgetV1 {
        kind: IMMEDIATE,
        deadline: DrDeadlineV1::ZERO,
    });
    *reason = 0;
    *system_code = 0;
    *has_system_code = 0;
    match DrWaitBudgetV1::new(kind, nanoseconds, crate::monotonic_clock::now) {
        Ok(value) => {
            budget.write(value);
            0
        }
        Err(failure) => write_failure(
            failure,
            &mut *reason,
            &mut *system_code,
            &mut *has_system_code,
        ),
    }
}

/// Freezes an exact two-word Deadline without reading the clock or changing the
/// supplied value. Words are signed two's-complement nanoseconds, low word first.
/// Its expiry is observed only by `dr_v1_wait_budget_remaining`; distinct past
/// instants and future instants above the clock's u64 range remain distinct.
///
/// # Safety
/// Output pointers must denote distinct live writable storage of their declared
/// types. The private budget layout is 24 bytes with 8-byte alignment.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_wait_budget_init_deadline(
    low: u64,
    high: i64,
    budget: *mut DrWaitBudgetV1,
    reason: *mut u8,
    system_code: *mut i64,
    has_system_code: *mut u8,
) -> u8 {
    *reason = 0;
    *system_code = 0;
    *has_system_code = 0;
    budget.write(DrWaitBudgetV1::from_deadline(DrDeadlineV1::from_parts(
        low, high,
    )));
    0
}

/// Computes the next native wait against the same deadline. Output tags are
/// 0 immediate, 1 indefinite, 2 finite, and 3 expired. Finite units are rounded
/// up and clamped to the native maximum; after an interrupted, spurious, or
/// clamped wait the caller must recheck this budget instead of restarting it.
///
/// # Safety
/// `budget` must reference an initialized budget. Output pointers must denote
/// distinct live writable storage, disjoint from the budget.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_wait_budget_remaining(
    budget: *const DrWaitBudgetV1,
    unit_nanoseconds: u64,
    maximum_units: u64,
    kind: *mut u8,
    units: *mut u64,
    reason: *mut u8,
    system_code: *mut i64,
    has_system_code: *mut u8,
) -> u8 {
    *kind = IMMEDIATE as u8;
    *units = 0;
    *reason = 0;
    *system_code = 0;
    *has_system_code = 0;
    match (*budget).remaining(unit_nanoseconds, maximum_units, crate::monotonic_clock::now) {
        Ok((next_kind, next_units)) => {
            *kind = next_kind;
            *units = next_units;
            0
        }
        Err(failure) => write_failure(
            failure,
            &mut *reason,
            &mut *system_code,
            &mut *has_system_code,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform_io::Reason;

    fn unexpected_clock() -> Result<u64, Failure> {
        panic!("untimed wait queried the clock")
    }

    #[test]
    fn wide_budget_has_one_shared_inline_layout() {
        assert_eq!(core::mem::size_of::<DrWaitBudgetV1>(), 24);
        assert_eq!(core::mem::align_of::<DrWaitBudgetV1>(), 8);
        assert_eq!(core::mem::offset_of!(DrWaitBudgetV1, kind), 0);
        assert_eq!(core::mem::offset_of!(DrWaitBudgetV1, deadline), 8);
    }

    #[test]
    fn relative_clock_plus_maximum_duration_is_exact_without_allocation() {
        let (budget, allocations) = crate::probe_runtime_allocations(|| {
            DrWaitBudgetV1::new(2, i64::MAX as u64, || Ok(u64::MAX)).unwrap()
        });
        assert_eq!(allocations, 0);
        assert_eq!(
            budget.deadline,
            DrDeadlineV1::from_clock_and_duration(u64::MAX, i64::MAX)
        );
        assert_eq!(
            budget.remaining(1, u64::MAX, || Ok(u64::MAX)),
            Ok((2, i64::MAX as u64))
        );
        assert_eq!(budget.remaining(1, u64::MAX, || Ok(0)), Ok((2, u64::MAX)));
        assert_eq!(budget.remaining(u64::MAX, u64::MAX, || Ok(0)), Ok((2, 2)));
        assert_eq!(budget.remaining(1, 100, || Ok(u64::MAX)), Ok((2, 100)));
    }

    #[test]
    fn exact_deadline_initializer_preserves_past_and_wide_future_values() {
        for (low, high) in [(u64::MAX - 1, -1), (u64::MAX, -1), (i64::MAX as u64 - 1, 1)] {
            let mut budget = DrWaitBudgetV1::indefinite();
            let (mut reason, mut code, mut has_code) = (255, -1, 1);
            let (_, allocations) = crate::probe_runtime_allocations(|| unsafe {
                assert_eq!(
                    dr_v1_wait_budget_init_deadline(
                        low,
                        high,
                        &mut budget,
                        &mut reason,
                        &mut code,
                        &mut has_code,
                    ),
                    0
                );
            });
            assert_eq!(allocations, 0);
            assert_eq!((reason, code, has_code), (0, 0, 0));
            let exact = DrDeadlineV1::from_parts(low, high);
            assert_eq!((budget.kind, budget.deadline), (DEADLINE, exact));
            for now in [0, 1, u64::MAX] {
                let expected = if high < 0 { (3, 0) } else { (2, 100) };
                assert_eq!(budget.remaining(1, 100, || Ok(now)), Ok(expected));
                assert_eq!((budget.kind, budget.deadline), (DEADLINE, exact));
            }
        }
    }

    #[test]
    fn wide_observation_caps_native_units_and_keeps_the_original_deadline() {
        let exact = DrDeadlineV1::from_parts(u64::MAX, i64::MAX);
        let budget = DrWaitBudgetV1::from_deadline(exact);
        for unit in [1, 1_000, 1_000_000, u64::MAX] {
            for maximum in [1, 100, u64::MAX] {
                // At u64::MAX, this instant has 2^64 * (2^63 - 1) ns left.
                // Dividing by u64::MAX rounds to exactly 2^63 units; each
                // smaller unit here necessarily exceeds every native u64 cap.
                let expected = if unit == u64::MAX {
                    (1_u64 << 63).min(maximum)
                } else {
                    maximum
                };
                assert_eq!(
                    budget.remaining(unit, maximum, || Ok(u64::MAX)),
                    Ok((2, expected))
                );
                assert_eq!(budget.deadline, exact);
            }
        }
        let failure = Failure::from_system_code(4);
        assert_eq!(budget.remaining(1, 100, || Err(failure)), Err(failure));
        assert_eq!(
            budget.remaining(0, 100, unexpected_clock),
            Err(Failure::invalid_input())
        );
        assert_eq!(
            budget.remaining(1, 0, unexpected_clock),
            Err(Failure::invalid_input())
        );
    }

    #[test]
    fn intersection_orders_exact_deadlines_across_both_words_and_expiry() {
        let deadlines = [
            DrDeadlineV1::from_clock_and_duration(0, i64::MIN),
            DrDeadlineV1::from_clock_and_duration(0, -2),
            DrDeadlineV1::from_clock_and_duration(0, -1),
            DrDeadlineV1::ZERO,
            DrDeadlineV1::from_clock_and_duration(u64::MAX, 0),
            DrDeadlineV1::from_clock_and_duration(u64::MAX, i64::MAX),
        ];
        for (left_index, left) in deadlines.iter().enumerate() {
            for (right_index, right) in deadlines.iter().enumerate() {
                let limited = DrWaitBudgetV1::from_deadline(*left)
                    .limited_by(DrWaitBudgetV1::from_deadline(*right));
                assert_eq!(limited.kind, DEADLINE);
                assert_eq!(limited.deadline, deadlines[left_index.min(right_index)]);
                let _ = limited.remaining(1, u64::MAX, || Ok(u64::MAX)).unwrap();
                assert_eq!(limited.deadline, deadlines[left_index.min(right_index)]);
            }
        }
    }

    #[test]
    fn exact_past_expiry_preserves_budget_identity_in_both_observation_carriers() {
        use crate::transfer_group::ObservationBudget;

        for nanoseconds in [i64::MIN, -2, -1] {
            let exact = DrDeadlineV1::from_clock_and_duration(0, nanoseconds);
            let mut budget = DrWaitBudgetV1::from_deadline(exact);
            assert!(ObservationBudget::<DrWaitBudgetV1>::next_wait(&mut budget)
                .unwrap()
                .is_none());
            assert!(ObservationBudget::<u32>::next_wait(&mut budget)
                .unwrap()
                .is_none());
            assert_eq!((budget.kind, budget.deadline), (DEADLINE, exact));
        }
    }

    #[test]
    fn intersecting_frozen_budgets_preserves_expiry_poll_and_earliest_deadline() {
        let budgets = [
            DrWaitBudgetV1::new(2, (-1_i64) as u64, unexpected_clock).unwrap(),
            DrWaitBudgetV1::new(0, 0, unexpected_clock).unwrap(),
            DrWaitBudgetV1::indefinite(),
            DrWaitBudgetV1::new(3, 10, unexpected_clock).unwrap(),
            DrWaitBudgetV1::new(3, 20, unexpected_clock).unwrap(),
        ];
        for (left_index, left) in budgets.iter().enumerate() {
            for (right_index, right) in budgets.iter().enumerate() {
                let limited = left.limited_by(*right);
                let reverse = right.limited_by(*left);
                assert_eq!(
                    (limited.kind, limited.deadline),
                    (reverse.kind, reverse.deadline)
                );
                let expected = if left_index == 0 || right_index == 0 {
                    &budgets[0]
                } else if left_index == 1 || right_index == 1 {
                    &budgets[1]
                } else if left_index == 2 {
                    right
                } else if right_index == 2 {
                    left
                } else {
                    &budgets[left_index.min(right_index)]
                };
                assert_eq!(
                    (limited.kind, limited.deadline),
                    (expected.kind, expected.deadline)
                );
                if limited.kind != DEADLINE {
                    limited.remaining(1, u64::MAX, unexpected_clock).unwrap();
                } else {
                    assert_eq!(
                        limited.remaining(1, u64::MAX, || Ok(5)).unwrap(),
                        (
                            2,
                            u64::try_from(limited.deadline.remaining_at(5).unwrap()).unwrap()
                        )
                    );
                    assert_eq!(limited.remaining(1, u64::MAX, || Ok(20)).unwrap(), (3, 0));
                }
            }
        }
    }

    #[test]
    fn immediate_and_indefinite_paths_never_query_the_clock() {
        for tag in [0, 1] {
            let budget = DrWaitBudgetV1::new(tag, u64::MAX, unexpected_clock).unwrap();
            assert_eq!(
                budget.remaining(1, u64::MAX, unexpected_clock).unwrap(),
                (tag, 0)
            );
        }
    }

    #[test]
    fn retries_spend_one_relative_duration_without_restarting_it() {
        let budget = DrWaitBudgetV1::new(2, 100, || Ok(10)).unwrap();
        assert_eq!(budget.remaining(1, u64::MAX, || Ok(10)).unwrap(), (2, 100));
        assert_eq!(budget.remaining(1, u64::MAX, || Ok(70)).unwrap(), (2, 40));
        assert_eq!(budget.remaining(1, u64::MAX, || Ok(110)).unwrap(), (3, 0));
        assert_eq!(budget.remaining(1, u64::MAX, || Ok(120)).unwrap(), (3, 0));
    }

    #[test]
    fn negative_relative_intervals_expire_without_clock_or_overflow() {
        for nanoseconds in [i64::MIN, -250_000_000, -1] {
            let (budget, allocations) = crate::probe_runtime_allocations(|| {
                DrWaitBudgetV1::new(2, nanoseconds as u64, unexpected_clock).unwrap()
            });
            assert_eq!(allocations, 0);
            assert_eq!(budget.kind, EXPIRED);
            for _ in 0..2 {
                assert_eq!(
                    budget.remaining(1, u64::MAX, unexpected_clock).unwrap(),
                    (3, 0)
                );
            }
        }
        let maximum = DrWaitBudgetV1::new(2, i64::MAX as u64, || Ok(10)).unwrap();
        assert_eq!(
            maximum.remaining(1, u64::MAX, || Ok(10)).unwrap(),
            (2, i64::MAX as u64)
        );
    }

    #[test]
    fn negative_observation_budgets_expire_in_both_native_timeout_carriers() {
        use crate::transfer_group::ObservationBudget;

        for nanoseconds in [i64::MIN, -250_000_000, -1] {
            let mut budget = DrWaitBudgetV1::new(2, nanoseconds as u64, unexpected_clock).unwrap();
            assert!(ObservationBudget::<DrWaitBudgetV1>::next_wait(&mut budget)
                .unwrap()
                .is_none());
            assert!(ObservationBudget::<u32>::next_wait(&mut budget)
                .unwrap()
                .is_none());
        }
    }

    #[test]
    fn negative_relative_abi_reports_expiry_and_clears_stale_outputs() {
        let mut budget = DrWaitBudgetV1::indefinite();
        for nanoseconds in [i64::MIN, -250_000_000, -1] {
            let (mut reason, mut code, mut has_code) = (255, -1, 1);
            assert_eq!(
                unsafe {
                    dr_v1_wait_budget_init(
                        2,
                        nanoseconds as u64,
                        &mut budget,
                        &mut reason,
                        &mut code,
                        &mut has_code,
                    )
                },
                0
            );
            assert_eq!((reason, code, has_code), (0, 0, 0));
            let (mut kind, mut units) = (255, u64::MAX);
            assert_eq!(
                unsafe {
                    dr_v1_wait_budget_remaining(
                        &budget,
                        1,
                        u64::MAX,
                        &mut kind,
                        &mut units,
                        &mut reason,
                        &mut code,
                        &mut has_code,
                    )
                },
                0
            );
            assert_eq!((kind, units, reason, code, has_code), (3, 0, 0, 0, 0));
        }
    }

    #[test]
    fn absolute_deadlines_and_zero_durations_are_not_immediate_poll_requests() {
        let deadline = DrWaitBudgetV1::new(3, 100, unexpected_clock).unwrap();
        assert_eq!(deadline.remaining(1, u64::MAX, || Ok(99)).unwrap(), (2, 1));
        assert_eq!(deadline.remaining(1, u64::MAX, || Ok(101)).unwrap(), (3, 0));
        let zero = DrWaitBudgetV1::new(2, 0, || Ok(100)).unwrap();
        assert_eq!(zero.remaining(1, u64::MAX, || Ok(100)).unwrap(), (3, 0));
    }

    #[test]
    fn native_resolution_rounds_up_and_caps_do_not_expire_the_budget() {
        let budget = DrWaitBudgetV1::new(3, u64::MAX, unexpected_clock).unwrap();
        assert_eq!(
            budget.remaining(1, u64::MAX, || Ok(0)).unwrap(),
            (2, u64::MAX)
        );
        assert_eq!(budget.remaining(u64::MAX, 1, || Ok(0)).unwrap(), (2, 1));
        assert_eq!(
            budget.remaining(1_000_000, 100, || Ok(0)).unwrap(),
            (2, 100)
        );
        let short = DrWaitBudgetV1::new(3, 1_000_001, unexpected_clock).unwrap();
        assert_eq!(
            short.remaining(1_000_000, u64::MAX, || Ok(0)).unwrap(),
            (2, 2)
        );
        assert_eq!(
            short
                .remaining(1_000_000, u64::MAX, || Ok(1_000_000))
                .unwrap(),
            (2, 1)
        );
    }

    #[test]
    fn relative_u64_addition_overflow_is_exact_and_other_failures_stay_checked() {
        let beyond_clock = DrWaitBudgetV1::new(2, 1, || Ok(u64::MAX)).unwrap();
        assert_eq!(
            beyond_clock.deadline,
            DrDeadlineV1::from_clock_and_duration(u64::MAX, 1)
        );
        assert_eq!(beyond_clock.remaining(1, 10, || Ok(u64::MAX)), Ok((2, 1)));
        assert_eq!(
            DrWaitBudgetV1::new(255, 0, unexpected_clock).err(),
            Some(Failure::invalid_input())
        );
        let failure = Failure::from_system_code(4);
        assert_eq!(
            DrWaitBudgetV1::new(2, 1, || Err(failure)).err(),
            Some(failure)
        );
        let budget = DrWaitBudgetV1::new(3, 100, unexpected_clock).unwrap();
        assert_eq!(budget.remaining(1, 100, || Err(failure)), Err(failure));
        assert_eq!(
            budget.remaining(0, 100, unexpected_clock),
            Err(Failure::invalid_input())
        );
        assert_eq!(
            budget.remaining(1, 0, unexpected_clock),
            Err(Failure::invalid_input())
        );
    }

    #[test]
    fn abi_clears_stale_outputs_and_resets_failed_initialization() {
        let mut budget = DrWaitBudgetV1 {
            kind: 255,
            deadline: DrDeadlineV1::from_clock_and_duration(u64::MAX, 0),
        };
        let (mut reason, mut code, mut has_code) = (255, -1, 1);
        assert_eq!(
            unsafe {
                dr_v1_wait_budget_init(0, 0, &mut budget, &mut reason, &mut code, &mut has_code)
            },
            0
        );
        assert_eq!((reason, code, has_code), (0, 0, 0));
        let (mut kind, mut units) = (255, u64::MAX);
        assert_eq!(
            unsafe {
                dr_v1_wait_budget_remaining(
                    &budget,
                    1,
                    u64::MAX,
                    &mut kind,
                    &mut units,
                    &mut reason,
                    &mut code,
                    &mut has_code,
                )
            },
            0
        );
        assert_eq!((kind, units, reason, code, has_code), (0, 0, 0, 0, 0));
        assert_eq!(
            unsafe {
                dr_v1_wait_budget_init(255, 0, &mut budget, &mut reason, &mut code, &mut has_code)
            },
            1
        );
        assert_eq!(budget.kind, IMMEDIATE);
        assert_eq!((reason, code, has_code), (Reason::InvalidInput as u8, 0, 0));
    }
}

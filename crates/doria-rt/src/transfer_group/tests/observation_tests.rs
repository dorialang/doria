use super::*;

impl ObservationBudget<i64> for ScriptControl {
    fn next_wait(&mut self) -> Result<Option<Wait<i64>>, Failure> {
        match Control::next_wait(self)? {
            Ok(wait) => Ok(Some(wait)),
            Err(Stop::BudgetExhausted) => Ok(None),
            Err(_) => panic!("test observation is not group cancellation"),
        }
    }
}

#[test]
fn expired_and_immediate_fetches_preserve_loans_identity_progress_and_reuse() {
    for progress in [None, Some(0), Some(2)] {
        let ledger = Ledger::new();
        let mut slots = request_slots(&[11, 22]).unwrap();
        let mut read = [0x55; 4];
        let write = [0x66; 4];
        let buffers =
            BufferList::from_iter([Buffer::Read(&mut read), Buffer::Write(&write)].into_iter())
                .unwrap();
        let failure = Failure::other().with_transferred_byte_count(progress);
        let mut driver = ScriptDriver::new(&ledger);
        driver.query_script(
            0,
            &[
                QueryValue::Pending,
                QueryValue::Terminal(Ok(2)),
                QueryValue::Terminal(Err(failure)),
            ],
        );
        driver.query_script(
            1,
            &[
                QueryValue::Pending,
                QueryValue::Pending,
                QueryValue::Pending,
                QueryValue::Terminal(Ok(2)),
            ],
        );
        driver.wakes[0] = Some(Ok(Wake::Timeout));
        let control = ScriptControl::new(&[ControlStep::Wait(100); 4]);
        let mut group = TransferGroup::begin(&mut slots, buffers, driver, control).unwrap();
        let original_request = ledger.borrow().rows[0].request;
        let original_buffer = ledger.borrow().rows[0].buffer;

        let mut expired = ScriptControl::new(&[ControlStep::Stop(Stop::BudgetExhausted)]);
        let (result, allocations) =
            crate::probe_runtime_allocations(|| group.observe(&mut expired));
        assert!(matches!(result, Observation::TimedOut));
        assert_eq!(allocations, 0);
        assert_eq!(
            ledger
                .borrow()
                .count(|event| matches!(event, Event::Query(_) | Event::Wait(..))),
            0
        );

        let mut immediate = ScriptControl::new(&[ControlStep::Wait(0)]);
        let (result, allocations) =
            crate::probe_runtime_allocations(|| group.observe(&mut immediate));
        assert!(matches!(result, Observation::TimedOut));
        assert_eq!(allocations, 0);
        assert_eq!(ledger.borrow().mask(), 3);
        assert_eq!(
            ledger.borrow().count(|event| matches!(
                event,
                Event::Cancel(_) | Event::Drain(_) | Event::Terminal(_)
            )),
            0
        );
        assert_eq!(group.stop(), None);
        for index in 0..2 {
            assert!(group.completed(index).is_none());
            assert!(group.read_storage_mut(index).is_none());
            assert_eq!(group.buffers.entries.as_slice()[index].progress, None);
        }

        let (completion, allocations) = crate::probe_runtime_allocations(|| group.next().unwrap());
        assert_eq!(allocations, 0);
        assert_eq!(
            (
                completion.key,
                completion.index,
                completion.outcome.transfer
            ),
            (11, 0, Some(Ok(2)))
        );
        assert_eq!(completion.outcome.stop, None);
        assert_eq!(group.completed(0), Some(&b"AA"[..]));
        assert_eq!(ledger.borrow().mask(), 2);
        assert_eq!(group.read_storage_mut(0).unwrap(), b"AAUU");
        let (result, allocations) = crate::probe_runtime_allocations(|| group.submit(0, 0, 4));
        assert_eq!(result, Ok(()));
        assert_eq!(allocations, 0);
        assert_eq!(ledger.borrow().rows[0].request, original_request);
        assert_eq!(ledger.borrow().rows[0].buffer, original_buffer);
        assert_eq!(ledger.borrow().rows[0].generation, 2);
        let failed = group.next().unwrap();
        assert_eq!((failed.key, failed.index), (11, 0));
        assert_eq!(failed.outcome.transfer, Some(Err(failure)));
        assert_eq!(failed.outcome.stop, None);
        assert_eq!(group.buffers.entries.as_slice()[0].progress, progress);
        assert_eq!(group.completed(0).map(<[u8]>::len), progress);
        assert_eq!(group.stop(), None);
        assert_eq!(
            ledger.borrow().mask(),
            2,
            "another transfer remains active after the original error"
        );
        let write_completion = group.next().unwrap();
        assert_eq!((write_completion.key, write_completion.index), (22, 1));
        assert!(group.next().is_none());
        let mut exhausted = ScriptControl::new(&[]);
        assert!(matches!(
            group.observe(&mut exhausted),
            Observation::NoCompletion
        ));
        assert_eq!(exhausted.position, 0);
        drop(group);
        assert_eq!(
            ledger
                .borrow()
                .count(|event| matches!(event, Event::Cancel(_) | Event::Drain(_))),
            0
        );
    }
}

#[test]
fn finite_observation_retries_use_remaining_budget_without_retirement() {
    let ledger = Ledger::new();
    let mut slots = request_slots(&[31]).unwrap();
    let mut bytes = [0x55; 4];
    let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
    let mut driver = ScriptDriver::new(&ledger);
    driver.query_script(
        0,
        &[
            QueryValue::Pending,
            QueryValue::Pending,
            QueryValue::Terminal(Ok(3)),
        ],
    );
    driver.wakes[0] = Some(Ok(Wake::Timeout));
    driver.wakes[1] = Some(Ok(Wake::Completion));
    let control = ScriptControl::new(&[ControlStep::Wait(100); 4]);
    let mut group = TransferGroup::begin(&mut slots, buffers, driver, control).unwrap();
    let mut observation = ScriptControl::new(&[
        ControlStep::Wait(9),
        ControlStep::Wait(4),
        ControlStep::Stop(Stop::BudgetExhausted),
    ]);
    assert!(matches!(
        group.observe(&mut observation),
        Observation::TimedOut
    ));
    assert_eq!(
        ledger
            .borrow()
            .count(|event| matches!(event, Event::Wait(9, _, 1) | Event::Wait(4, _, 1))),
        2
    );
    assert_eq!(
        ledger
            .borrow()
            .count(|event| matches!(event, Event::Query(_))),
        2
    );
    assert_eq!(ledger.borrow().mask(), 1);
    assert_eq!(group.stop(), None);
    assert_eq!(group.next().unwrap().outcome.transfer, Some(Ok(3)));
    assert_eq!(
        ledger
            .borrow()
            .count(|event| matches!(event, Event::Cancel(_) | Event::Drain(_))),
        0
    );
}

#[test]
fn immediate_spurious_wake_returns_without_an_unbounded_poll_loop() {
    let ledger = Ledger::new();
    let mut slots = request_slots(&[41]).unwrap();
    let mut bytes = [0x55; 4];
    let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
    let mut driver = ScriptDriver::new(&ledger);
    driver.query_script(
        0,
        &[
            QueryValue::Pending,
            QueryValue::Pending,
            QueryValue::Terminal(Ok(1)),
        ],
    );
    driver.wakes[0] = Some(Ok(Wake::Completion));
    let control = ScriptControl::new(&[ControlStep::Wait(100); 2]);
    let mut group = TransferGroup::begin(&mut slots, buffers, driver, control).unwrap();
    let mut observation = ScriptControl::new(&[ControlStep::Wait(0); 2]);
    assert!(matches!(
        group.observe(&mut observation),
        Observation::TimedOut
    ));
    assert_eq!(
        ledger
            .borrow()
            .count(|event| matches!(event, Event::Wait(..))),
        1
    );
    assert_eq!(group.stop(), None);
    assert_eq!(group.next().unwrap().outcome.transfer, Some(Ok(1)));
}

#[test]
fn explicit_group_stops_still_cancel_all_before_draining_after_fetch_expiry() {
    for stop in [
        Stop::Cancellation,
        Stop::BudgetExhausted,
        Stop::Failure(IoMessageOperation::Wait, Failure::other()),
    ] {
        let ledger = Ledger::new();
        let mut slots = request_slots(&[51, 52]).unwrap();
        let mut bytes = [0x55; 4];
        let write = [0x66; 4];
        let buffers =
            BufferList::from_iter([Buffer::Read(&mut bytes), Buffer::Write(&write)].into_iter())
                .unwrap();
        let mut driver = ScriptDriver::new(&ledger);
        driver.drains = [Ok(2); SLOTS];
        let control = ScriptControl::new(&[ControlStep::Wait(100), ControlStep::Stop(stop)]);
        let mut group = TransferGroup::begin(&mut slots, buffers, driver, control).unwrap();
        let mut expired = ScriptControl::new(&[ControlStep::Stop(Stop::BudgetExhausted)]);
        if let Stop::Failure(IoMessageOperation::Wait, failure) = stop {
            assert!(
                matches!(group.observe(&mut expired), Observation::WaitFailed(observed) if observed == failure)
            );
        }
        let Observation::Completion(completion) = group.observe(&mut expired) else {
            panic!("explicit stop was hidden by fetch expiry")
        };
        assert_eq!(completion.outcome.stop, Some(stop));
        assert_eq!(completion.outcome.transfer, Some(Ok(2)));
        assert_eq!(group.stop(), Some(stop));
        assert_eq!(
            ledger
                .borrow()
                .count(|event| matches!(event, Event::Cancel(_))),
            2
        );
        assert_eq!(
            ledger
                .borrow()
                .count(|event| matches!(event, Event::Drain(_))),
            2
        );
        assert_eq!(group.submit(0, 0, 4), Err(Failure::invalid_input()));
    }
}

#[test]
fn teardown_after_observation_timeout_still_retains_storage_through_drain() {
    let ledger = Ledger::new();
    let mut slots = request_slots(&[61, 62]).unwrap();
    let mut first = BorrowedStorage::new(0, &ledger);
    let mut second = BorrowedStorage::new(1, &ledger);
    let buffers = BufferList::from_iter(
        [
            Buffer::Read(&mut first.bytes),
            Buffer::Read(&mut second.bytes),
        ]
        .into_iter(),
    )
    .unwrap();
    let driver = ScriptDriver::new(&ledger);
    let control = ScriptControl::new(&[ControlStep::Wait(100); 2]);
    let mut group = TransferGroup::begin(&mut slots, buffers, driver, control).unwrap();
    let mut expired = ScriptControl::new(&[ControlStep::Stop(Stop::BudgetExhausted)]);
    assert!(matches!(group.observe(&mut expired), Observation::TimedOut));
    assert_eq!(ledger.borrow().mask(), 3);
    drop(group);
    assert_eq!(ledger.borrow().mask(), 0);
    assert_eq!(
        ledger
            .borrow()
            .count(|event| matches!(event, Event::Cancel(_))),
        2
    );
    assert_eq!(
        ledger
            .borrow()
            .count(|event| matches!(event, Event::Drain(_))),
        2
    );
    drop(first);
    drop(second);
    assert_eq!(
        ledger
            .borrow()
            .count(|event| matches!(event, Event::BufferReleased(_))),
        2
    );
}

#[test]
fn observation_clock_failure_still_drains_actual_terminals_through_group_cleanup() {
    let ledger = Ledger::new();
    let mut slots = request_slots(&[71, 72]).unwrap();
    let mut bytes = [0x55; 4];
    let write = [0x66; 4];
    let buffers =
        BufferList::from_iter([Buffer::Read(&mut bytes), Buffer::Write(&write)].into_iter())
            .unwrap();
    let clock_failure = Failure::other();
    let terminal_failure = Failure::other().with_transferred_byte_count(Some(2));
    let mut driver = ScriptDriver::new(&ledger);
    driver.drains[0] = Err(terminal_failure);
    driver.drains[1] = Ok(3);
    let control = ScriptControl::new(&[ControlStep::Wait(100)]);
    let mut group = TransferGroup::begin(&mut slots, buffers, driver, control).unwrap();
    let mut observation = ScriptControl::new(&[ControlStep::Failed(clock_failure)]);
    let (first_observation, allocations) =
        crate::probe_runtime_allocations(|| group.observe(&mut observation));
    assert_eq!(allocations, 0);
    assert!(
        matches!(first_observation, Observation::WaitFailed(failure) if failure == clock_failure)
    );
    assert!(group
        .buffers
        .entries
        .as_slice()
        .iter()
        .all(|entry| entry.ready.is_some()));
    let Observation::Completion(first) = group.observe(&mut observation) else {
        panic!("infrastructure failure lost actual terminal")
    };
    assert_eq!(first.outcome.transfer, Some(Err(terminal_failure)));
    assert_eq!(
        first.outcome.stop,
        Some(Stop::Failure(IoMessageOperation::Wait, clock_failure))
    );
    assert_eq!(group.next().unwrap().outcome.transfer, Some(Ok(3)));
    assert!(group.next().is_none());
    assert_eq!(
        ledger
            .borrow()
            .count(|event| matches!(event, Event::Cancel(_))),
        2
    );
    assert_eq!(
        ledger
            .borrow()
            .count(|event| matches!(event, Event::Drain(_))),
        2
    );
}

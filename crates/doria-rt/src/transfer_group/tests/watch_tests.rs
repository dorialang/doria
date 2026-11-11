use super::*;
use crate::terminal::Watched;

const QUIET: Watched = Watched::Waiting { until: None };

#[test]
fn a_group_of_one_watch_reports_it_once_per_arming_and_never_waits_while_it_is_ready() {
    let ledger = Ledger::new();
    let mut slots = request_slots(&[]).unwrap();
    let buffers = BufferList::from_iter(core::iter::empty()).unwrap();
    let driver = ScriptDriver::new(&ledger);
    let control = ScriptControl::new(&[ControlStep::Wait(100); 4]);
    let mut group = TransferGroup::begin(&mut slots, buffers, driver, control).unwrap();
    let source = WatchSource::scripted(0, Watched::Ready);
    group.watch(&[source]).unwrap();
    assert_eq!(group.watch_count(), 1);
    // Armed: it cannot be armed again, and no other watch exists.
    assert!(group.rearm_watch(0).is_err());
    assert!(group.rearm_watch(1).is_err());

    assert!(matches!(
        group.observe(&mut observation::Indefinite),
        Observation::Watch(0)
    ));
    // Reported once. Left alone it is retired and the group has nothing.
    assert!(matches!(
        group.observe(&mut observation::Indefinite),
        Observation::NoCompletion
    ));
    group.rearm_watch(0).unwrap();
    assert!(group.rearm_watch(0).is_err());
    assert!(matches!(
        group.observe(&mut observation::Indefinite),
        Observation::Watch(0)
    ));

    // A source with nothing waiting keeps the watch armed through a fetch
    // whose own budget has ended.
    WatchSource::scripted(0, QUIET);
    group.rearm_watch(0).unwrap();
    let mut expired = ScriptControl::new(&[ControlStep::Stop(Stop::BudgetExhausted)]);
    assert!(matches!(group.observe(&mut expired), Observation::TimedOut));
    assert_eq!(group.stop(), None);
    // The source is asked again at the next fetch, not remembered.
    WatchSource::scripted(0, Watched::Ready);
    assert!(matches!(
        group.observe(&mut observation::Indefinite),
        Observation::Watch(0)
    ));
    assert_eq!(
        ledger
            .borrow()
            .count(|event| matches!(event, Event::Wait(..))),
        0
    );
}

#[test]
fn a_quiet_watch_leaves_transfers_alone_and_a_ready_one_is_reported_before_them() {
    let ledger = Ledger::new();
    let mut slots = request_slots(&[11]).unwrap();
    let mut read = [0x55; 4];
    let buffers = BufferList::from_iter([Buffer::Read(&mut read)].into_iter()).unwrap();
    let mut driver = ScriptDriver::new(&ledger);
    driver.query_script(
        0,
        &[
            QueryValue::Pending,
            QueryValue::Terminal(Ok(2)),
            QueryValue::Terminal(Ok(1)),
        ],
    );
    driver.wakes[0] = Some(Ok(Wake::Completion));
    let control = ScriptControl::new(&[ControlStep::Wait(100); 8]);
    let mut group = TransferGroup::begin(&mut slots, buffers, driver, control).unwrap();
    group.watch(&[WatchSource::scripted(1, QUIET)]).unwrap();

    // The transfer completes after one wait; the quiet watch stays armed.
    let Observation::Completion(completion) = group.observe(&mut observation::Indefinite) else {
        panic!("the transfer did not complete");
    };
    assert_eq!(
        (completion.index, completion.outcome.transfer),
        (0, Some(Ok(2)))
    );
    assert_eq!(
        ledger
            .borrow()
            .count(|event| matches!(event, Event::Wait(..))),
        1
    );

    // Both are ready now: the transfer is queued and the source has an
    // event. The watch is asked first.
    group.submit(0, 0, 4).unwrap();
    let mut immediate = ScriptControl::new(&[ControlStep::Wait(0)]);
    let Observation::Completion(queued) = group.observe(&mut immediate) else {
        panic!("the resubmitted transfer did not complete");
    };
    assert_eq!(queued.outcome.transfer, Some(Ok(1)));
    group.submit(0, 0, 4).unwrap();
    WatchSource::scripted(1, Watched::Ready);
    let queries = ledger
        .borrow()
        .count(|event| matches!(event, Event::Query(_)));
    assert!(matches!(
        group.observe(&mut observation::Indefinite),
        Observation::Watch(0)
    ));
    assert_eq!(
        ledger
            .borrow()
            .count(|event| matches!(event, Event::Query(_))),
        queries
    );
}

#[test]
fn a_stopping_group_retires_an_armed_watch_with_its_transfers_and_reports_it_once() {
    let ledger = Ledger::new();
    let mut slots = request_slots(&[11]).unwrap();
    let mut read = [0x55; 4];
    let buffers = BufferList::from_iter([Buffer::Read(&mut read)].into_iter()).unwrap();
    let driver = ScriptDriver::new(&ledger);
    let control = ScriptControl::new(&[ControlStep::Stop(Stop::Cancellation)]);
    let mut group = TransferGroup::begin(&mut slots, buffers, driver, control).unwrap();
    group.watch(&[WatchSource::scripted(2, QUIET)]).unwrap();

    assert!(matches!(
        group.observe(&mut observation::Indefinite),
        Observation::Watch(0)
    ));
    assert_eq!(group.stop(), Some(Stop::Cancellation));
    let Observation::Completion(retired) = group.observe(&mut observation::Indefinite) else {
        panic!("the pending transfer was not retired");
    };
    assert_eq!(retired.outcome.stop, Some(Stop::Cancellation));
    assert!(matches!(
        group.observe(&mut observation::Indefinite),
        Observation::NoCompletion
    ));
    // Nothing can be armed in a stopped group.
    assert!(group.rearm_watch(0).is_err());
}

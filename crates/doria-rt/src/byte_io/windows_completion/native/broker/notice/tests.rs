//! Real Windows lock execution is required; closure injection controls posts.

#![cfg(windows)]

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use super::*;

const TEST_WAIT: Duration = Duration::from_secs(5);

#[test]
fn failed_post_preserves_exact_failure_and_is_retryable() {
    let latch = core::pin::pin!(NotificationLatch::new());
    let calls = AtomicUsize::new(0);
    let failure = Failure::from_system_code(8);
    assert_eq!(
        latch.as_ref().notify(|| {
            calls.fetch_add(1, Ordering::Relaxed);
            Err(failure)
        }),
        Err(failure)
    );
    assert_eq!(
        latch.as_ref().notify(|| {
            calls.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }),
        Ok(())
    );
    assert_eq!(calls.load(Ordering::Relaxed), 2);
}

#[test]
fn one_confirmed_post_coalesces_later_notifications() {
    let latch = core::pin::pin!(NotificationLatch::new());
    let calls = AtomicUsize::new(0);
    for _ in 0..4 {
        assert_eq!(
            latch.as_ref().notify(|| {
                calls.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }),
            Ok(())
        );
    }
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[test]
fn consumption_permits_the_next_confirmed_post() {
    let latch = core::pin::pin!(NotificationLatch::new());
    let calls = AtomicUsize::new(0);
    for expected in 1..=3 {
        assert_eq!(
            latch.as_ref().notify(|| {
                calls.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }),
            Ok(())
        );
        assert_eq!(calls.load(Ordering::Relaxed), expected);
        latch.as_ref().consumed();
    }
}

#[test]
fn in_flight_failed_post_cannot_acknowledge_a_concurrent_notifier() {
    // Pinned Arc ownership keeps both native storage and captured state alive
    // even if a bounded receive fails before joining a spawned caller.
    let latch = Arc::pin(NotificationLatch::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let released = Arc::new(AtomicBool::new(false));
    let failure = Failure::from_system_code(8);
    let (posting_tx, posting_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    let (first_done_tx, first_done_rx) = mpsc::sync_channel(1);
    let first_latch = latch.clone();
    let first_calls = calls.clone();
    let first = thread::spawn(move || {
        let result = first_latch.as_ref().notify(|| {
            first_calls.fetch_add(1, Ordering::Relaxed);
            posting_tx.send(()).unwrap();
            release_rx.recv_timeout(TEST_WAIT).unwrap();
            Err(failure)
        });
        first_done_tx.send(result).unwrap();
    });
    posting_rx.recv_timeout(TEST_WAIT).unwrap();

    let (attempting_tx, attempting_rx) = mpsc::sync_channel(1);
    let (second_done_tx, second_done_rx) = mpsc::sync_channel(1);
    let second_latch = latch.clone();
    let second_calls = calls.clone();
    let second_released = released.clone();
    let second = thread::spawn(move || {
        attempting_tx.send(()).unwrap();
        let result = second_latch.as_ref().notify(|| {
            second_calls.fetch_add(1, Ordering::Relaxed);
            Ok(())
        });
        second_done_tx
            .send((result, second_released.load(Ordering::Acquire)))
            .unwrap();
    });
    attempting_rx.recv_timeout(TEST_WAIT).unwrap();
    assert_eq!(second_done_rx.try_recv(), Err(TryRecvError::Empty));

    released.store(true, Ordering::Release);
    release_tx.send(()).unwrap();
    assert_eq!(first_done_rx.recv_timeout(TEST_WAIT).unwrap(), Err(failure));
    let (result, after_release) = second_done_rx.recv_timeout(TEST_WAIT).unwrap();
    first.join().unwrap();
    second.join().unwrap();
    assert_eq!(result, Ok(()));
    assert!(
        after_release,
        "second notifier acknowledged an in-flight post"
    );
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    assert_eq!(
        latch.as_ref().notify(|| panic!("post was not coalesced")),
        Ok(())
    );
}

#[test]
fn consumer_cannot_clear_before_successful_post_publication() {
    let latch = Arc::pin(NotificationLatch::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let post_ready = Arc::new(AtomicBool::new(false));
    let (posting_tx, posting_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    let (posted_tx, posted_rx) = mpsc::sync_channel(1);
    let producer_latch = latch.clone();
    let producer_calls = calls.clone();
    let producer_ready = post_ready.clone();
    let producer = thread::spawn(move || {
        let result = producer_latch.as_ref().notify(|| {
            producer_calls.fetch_add(1, Ordering::Relaxed);
            // Model the native packet becoming visible before Post returns.
            posting_tx.send(()).unwrap();
            release_rx.recv_timeout(TEST_WAIT).unwrap();
            producer_ready.store(true, Ordering::Release);
            Ok(())
        });
        posted_tx.send(result).unwrap();
    });
    posting_rx.recv_timeout(TEST_WAIT).unwrap();

    let (attempting_tx, attempting_rx) = mpsc::sync_channel(1);
    let (consumed_tx, consumed_rx) = mpsc::sync_channel(1);
    let consumer_latch = latch.clone();
    let consumer_ready = post_ready.clone();
    let consumer = thread::spawn(move || {
        attempting_tx.send(()).unwrap();
        consumer_latch.as_ref().consumed();
        consumed_tx
            .send(consumer_ready.load(Ordering::Acquire))
            .unwrap();
    });
    attempting_rx.recv_timeout(TEST_WAIT).unwrap();
    assert_eq!(consumed_rx.try_recv(), Err(TryRecvError::Empty));
    release_tx.send(()).unwrap();
    assert_eq!(posted_rx.recv_timeout(TEST_WAIT).unwrap(), Ok(()));
    assert!(consumed_rx.recv_timeout(TEST_WAIT).unwrap());
    producer.join().unwrap();
    consumer.join().unwrap();

    assert_eq!(
        latch.as_ref().notify(|| {
            calls.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }),
        Ok(())
    );
    assert_eq!(
        calls.load(Ordering::Relaxed),
        2,
        "consumer's clear was overwritten by post publication"
    );
}

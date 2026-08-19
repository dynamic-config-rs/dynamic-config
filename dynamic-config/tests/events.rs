//! The events stream and the failure hook: refusals reach subscribers.
//!
//! The contract under test, spelled once: a refusal wakes `events()` and
//! never resolves `changed()`; a refusal raced by an install yields both,
//! refusal first; installs collapse latest-wins; and `on_reload_failed`
//! fires on the thread that refused, panic-isolated like its success twin.
//!
//! Cell-level statics, one per test — the same discipline as
//! `async_api.rs`, because a shared cell would let tests race each other.

#![cfg(all(feature = "async", feature = "json"))]

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use dynamic_config::{ConfigCell, Error, Event, ReloadReason};

fn refusal() -> Error {
    Error::invalid("the document was refused; the previous snapshot serves")
}

#[tokio::test]
async fn a_refusal_wakes_the_events_stream() {
    static CELL: ConfigCell<u16> = ConfigCell::new();

    CELL.store(1);

    let mut events = CELL.events();

    CELL.record_failure(&refusal());

    let event = tokio::time::timeout(Duration::from_secs(5), events.next_event())
        .await
        .expect("the refusal wakes the stream");

    assert!(matches!(event, Event::Refused(_)), "{event:?}");
}

#[tokio::test]
async fn a_refusal_raced_by_an_install_yields_both_refusal_first() {
    static CELL: ConfigCell<u16> = ConfigCell::new();

    CELL.store(1);

    let mut events = CELL.events();

    // Both land before anything polls: one wake window, two events.
    CELL.record_failure(&refusal());
    CELL.store(2);

    let first = tokio::time::timeout(Duration::from_secs(5), events.next_event())
        .await
        .expect("first event");
    let second = tokio::time::timeout(Duration::from_secs(5), events.next_event())
        .await
        .expect("the drain-then-park structure delivers the second without another wake");

    assert!(matches!(first, Event::Refused(_)), "{first:?}");

    match second {
        Event::Reloaded { current, .. } => assert_eq!(*current, 2),
        other => panic!("expected the install: {other:?}"),
    }
}

#[tokio::test]
async fn installs_collapse_latest_wins() {
    static CELL: ConfigCell<u16> = ConfigCell::new();

    CELL.store(1);

    let mut events = CELL.events();

    CELL.store(2);
    CELL.store(3);

    let event = tokio::time::timeout(Duration::from_secs(5), events.next_event())
        .await
        .expect("an event");

    match event {
        Event::Reloaded { current, .. } => assert_eq!(*current, 3, "latest wins"),
        other => panic!("expected an install: {other:?}"),
    }
}

#[tokio::test]
async fn changed_is_not_resolved_by_a_refusal() {
    static CELL: ConfigCell<u16> = ConfigCell::new();

    CELL.store(1);

    let mut changes = CELL.changes();

    CELL.record_failure(&refusal());

    // The refusal bumped the wake counter; `changed()` must treat that as
    // at most a re-registration, never a yield of the unchanged snapshot.
    let outcome = tokio::time::timeout(Duration::from_millis(200), changes.changed()).await;
    assert!(outcome.is_err(), "a refusal resolved changed()");

    // And the real install still comes through afterwards.
    CELL.store(2);

    let value = tokio::time::timeout(Duration::from_secs(5), changes.changed())
        .await
        .expect("the install resolves the waiter");
    assert_eq!(*value, 2);
}

#[test]
fn on_reload_failed_fires_and_is_panic_isolated() {
    static CELL: ConfigCell<u16> = ConfigCell::new();
    static CALLS: AtomicU32 = AtomicU32::new(0);

    // The panicking hook registers first; the counter proves the second
    // hook still ran and the calling thread survived.
    CELL.on_reload_failed(|_| panic!("a bug in a hook"));
    CELL.on_reload_failed(|status| {
        assert!(
            !format!("{status:?}").contains("refused; the previous"),
            "a failure hook must never see free text"
        );
        CALLS.fetch_add(1, Ordering::SeqCst);
    });

    CELL.record_failure(&refusal());

    assert_eq!(CALLS.load(Ordering::SeqCst), 1);

    // A success does not fire the failure hooks.
    CELL.store_with(7, ReloadReason::Manual);
    assert_eq!(CALLS.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn refusals_are_monotonic_and_reported() {
    static CELL: ConfigCell<u16> = ConfigCell::new();

    assert_eq!(CELL.refusals(), 0);

    CELL.record_failure(&refusal());
    CELL.store(1);
    CELL.record_failure(&refusal());

    assert_eq!(
        CELL.refusals(),
        2,
        "success must not reset the monotonic count"
    );
    assert_eq!(
        CELL.status().consecutive_failures,
        1,
        "the streak did reset"
    );
}

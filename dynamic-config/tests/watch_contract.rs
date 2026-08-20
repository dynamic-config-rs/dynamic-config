//! Watching a remote store, as a contract every store answers.
//!
//! A store either pushes changes or it does not, and a caller needs to know
//! which before it decides how to wait. What this file pins: the capability
//! a store reports reaches the caller through an erased source, a store's
//! own watch is the one that runs, a store without one still gets watched,
//! and the waits a loop makes are spread and grow after a failure.

#![cfg(feature = "json")]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dynamic_config::{
    Error, Fetched, Format, Pace, Remote, RemoteSource, RemoteWatch, WatchCapability, Watching,
};

/// A store with nothing but `fetch`: the default watch is what it gets.
struct Polled {
    documents: Mutex<Vec<&'static str>>,
    polls: Arc<AtomicUsize>,
}

impl RemoteSource for Polled {
    fn fetch(&self) -> Result<Fetched, Error> {
        self.polls.fetch_add(1, Ordering::SeqCst);

        let mut documents = self.documents.lock().unwrap();
        let text = if documents.len() > 1 {
            documents.remove(0)
        } else {
            documents[0]
        };

        Ok(Fetched::new(text, Format::Json))
    }

    fn describe(&self) -> String {
        "a polled store".to_owned()
    }
}

/// A store that pushes, and says so.
struct Streaming {
    watched: Arc<AtomicUsize>,
}

impl RemoteSource for Streaming {
    fn fetch(&self) -> Result<Fetched, Error> {
        Ok(Fetched::new(r#"{"db":{"port":1}}"#, Format::Json))
    }

    fn describe(&self) -> String {
        "a streaming store".to_owned()
    }

    fn watch_capability(&self) -> WatchCapability {
        WatchCapability::Native
    }

    fn watch(
        &self,
        _watching: &Watching,
        _interval: Duration,
        on_change: &mut dyn FnMut(Fetched) -> Result<(), Error>,
    ) -> Result<(), Error> {
        self.watched.fetch_add(1, Ordering::SeqCst);

        on_change(Fetched::new(r#"{"db":{"port":2}}"#, Format::Json))
    }
}

/// A store whose fetches all fail: the watch must outlive that.
struct Broken;

impl RemoteSource for Broken {
    fn fetch(&self) -> Result<Fetched, Error> {
        Err(Error::remote("the store is down"))
    }

    fn describe(&self) -> String {
        "a broken store".to_owned()
    }
}

#[test]
fn a_store_without_a_watch_still_says_what_it_is() {
    let remote = Remote::new();

    remote.set(Polled {
        documents: Mutex::new(vec![r#"{"db":{"port":1}}"#]),
        polls: Arc::new(AtomicUsize::new(0)),
    });

    assert_eq!(remote.watch_capability(), Some(WatchCapability::Interval));
}

#[test]
fn a_capability_reaches_the_caller_through_an_erased_source() {
    let remote = Remote::new();

    remote.set(Streaming {
        watched: Arc::new(AtomicUsize::new(0)),
    });

    assert_eq!(
        remote.watch_capability(),
        Some(WatchCapability::Native),
        "the store said native, and the erased source must still say so"
    );
    assert_eq!(Remote::new().watch_capability(), None, "nothing installed");
}

/// The store's own mechanism is what runs — not a poll around it.
#[test]
fn a_store_with_a_watch_has_its_own_watch_run() {
    let watched = Arc::new(AtomicUsize::new(0));
    let remote = Remote::new();

    remote.set(Streaming {
        watched: Arc::clone(&watched),
    });

    let handle = RemoteWatch::new();

    remote
        .watch(&handle.watching(), Duration::from_millis(10))
        .expect("the watch runs, and returns when the store's own does");

    assert_eq!(watched.load(Ordering::SeqCst), 1, "the store's watch ran");
    assert_eq!(
        remote.document().map(|document| document.text),
        Some(r#"{"db":{"port":2}}"#.to_owned()),
        "what the store pushed is what was kept"
    );
}

/// The default watch delivers a change and nothing else: a document that
/// has not moved must not be installed again, or every reload hook in the
/// process fires on a timer.
#[test]
fn the_default_watch_delivers_only_what_changed() {
    let polls = Arc::new(AtomicUsize::new(0));
    let remote = Remote::new();

    remote.set(Polled {
        // Read twice before it changes, so a watch that delivered every
        // fetch would install the same document twice.
        documents: Mutex::new(vec![
            r#"{"db":{"port":1}}"#,
            r#"{"db":{"port":1}}"#,
            r#"{"db":{"port":2}}"#,
        ]),
        polls: Arc::clone(&polls),
    });

    let handle = RemoteWatch::new();

    std::thread::scope(|scope| {
        scope.spawn(|| {
            let _ = remote.watch(&handle.watching(), Duration::from_millis(1));
        });

        std::thread::sleep(Duration::from_millis(150));
        handle.stop();
    });

    assert_eq!(
        remote.document().map(|document| document.text),
        Some(r#"{"db":{"port":2}}"#.to_owned()),
        "the last document the store held is the one that stuck"
    );

    let polled = polls.load(Ordering::SeqCst) as u64;
    let delivered = remote.status().fetches;

    assert!(polled > delivered, "{polled} polls, {delivered} deliveries");
    assert_eq!(delivered, 2, "two documents differed; the repeat did not");
}

/// A watch outlives an outage: a failing fetch backs off rather than ending
/// the loop, which is the whole reason a watch exists.
#[test]
fn a_failing_store_does_not_end_the_watch() {
    let remote = Remote::new();

    remote.set(Broken);

    let handle = RemoteWatch::new();
    let watching = handle.watching();

    std::thread::scope(|scope| {
        let running = scope.spawn(|| remote.watch(&watching, Duration::from_millis(1)));

        std::thread::sleep(Duration::from_millis(80));
        handle.stop();

        assert!(
            running.join().unwrap().is_ok(),
            "an outage is a reason to wait, not a reason to stop"
        );
    });

    assert!(remote.document().is_none(), "nothing was ever delivered");
}

/// The waits themselves: spread when things work, growing when they do not.
#[test]
fn the_waits_are_spread_and_grow_after_a_failure() {
    let mut pace = Pace::new(Duration::from_secs(100));

    let healthy: Vec<Duration> = (0..16).map(|_| pace.next_wait()).collect();

    for wait in &healthy {
        assert!(
            *wait >= Duration::from_secs(75) && *wait <= Duration::from_secs(125),
            "a healthy wait stays within a quarter of the interval: {wait:?}"
        );
    }

    assert!(
        healthy.iter().any(|wait| *wait != healthy[0]),
        "sixteen identical waits is a fleet polling in lockstep"
    );

    // Growth is measured where jitter cannot hide it. A quarter either way
    // is a *band*, and two consecutive bands overlap — 2× the interval
    // jittered up can exceed 4× jittered down, and near the ceiling it
    // routinely does. So the bands are compared rather than two samples.
    let mut growing = Pace::new(Duration::from_secs(1));

    growing.failed();
    let first = growing.next_wait();
    growing.failed();
    let second = growing.next_wait();

    assert!(
        first <= Duration::from_millis(2_500),
        "one failure waits about twice the interval: {first:?}"
    );
    assert!(
        second >= Duration::from_secs(3),
        "two failures wait about four times it: {second:?}"
    );

    for _ in 0..40 {
        growing.failed();
    }

    assert!(
        growing.next_wait() <= Duration::from_secs(400),
        "the backoff is capped, and forty failures must not overflow it"
    );

    growing.succeeded();

    assert!(
        growing.next_wait() <= Duration::from_millis(1_250),
        "a round that works puts the wait back to the interval"
    );
}

/// A store that pushes once and then goes quiet, exactly as a dropped
/// subscription does.
struct Stalling {
    polls: Arc<AtomicUsize>,
}

impl RemoteSource for Stalling {
    fn fetch(&self) -> Result<Fetched, Error> {
        let polls = self.polls.fetch_add(1, Ordering::SeqCst);

        Ok(Fetched::new(
            format!(r#"{{"db":{{"port":{}}}}}"#, polls + 10),
            Format::Json,
        ))
    }

    fn describe(&self) -> String {
        "a stalling store".to_owned()
    }

    fn watch_capability(&self) -> WatchCapability {
        WatchCapability::Native
    }

    fn watch(
        &self,
        watching: &Watching,
        _interval: Duration,
        on_change: &mut dyn FnMut(Fetched) -> Result<(), Error>,
    ) -> Result<(), Error> {
        on_change(Fetched::new(r#"{"db":{"port":1}}"#, Format::Json))?;

        // And now nothing, forever — the failure mode a resync exists for.
        while watching.keep_going() {
            std::thread::sleep(Duration::from_millis(10));
        }

        Ok(())
    }
}

/// A native watch is read on the interval as well.
///
/// The failure mode of a stream is silence: a connection that dropped
/// without an error looks exactly like a store where nothing has changed.
/// Only going and asking tells them apart.
#[test]
fn a_native_watch_is_resynced_on_the_interval() {
    let polls = Arc::new(AtomicUsize::new(0));
    let remote = Remote::new();

    remote.set(Stalling {
        polls: Arc::clone(&polls),
    });

    let handle = RemoteWatch::new();

    std::thread::scope(|scope| {
        scope.spawn(|| {
            let _ = remote.watch(&handle.watching(), Duration::from_millis(20));
        });

        std::thread::sleep(Duration::from_millis(400));
        handle.stop();
    });

    assert!(
        polls.load(Ordering::SeqCst) > 0,
        "a store that went quiet was never asked again"
    );
    assert_ne!(
        remote.document().map(|document| document.text),
        Some(r#"{"db":{"port":1}}"#.to_owned()),
        "the resync found a newer document than the stream ever pushed"
    );
}

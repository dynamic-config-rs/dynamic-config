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

/// A store that pushes twice, dropping the document in between.
///
/// The second push is the one under test: it arrives *after* a
/// `Remote::clear()`, which is the case `clear`'s own documentation
/// promises still installs.
struct ClearsBetweenPushes {
    remote: &'static Remote,
}

impl RemoteSource for ClearsBetweenPushes {
    fn fetch(&self) -> Result<Fetched, Error> {
        Ok(Fetched::new(r#"{"db":{"port":1}}"#, Format::Json))
    }

    fn describe(&self) -> String {
        "a store that pushes across a clear".to_owned()
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
        on_change(Fetched::new(r#"{"db":{"port":1}}"#, Format::Json))?;

        // Somebody drops the document — a reconfiguration, a test, an
        // operator taking the store out of the picture for a moment.
        self.remote.clear();

        on_change(Fetched::new(r#"{"db":{"port":2}}"#, Format::Json))
    }
}

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
/// **A `clear()` mid-watch does not end the watch's usefulness.**
///
/// The delivery fence used to be the whole fetch fence — generation *and*
/// the clear counter — captured once when the watch started. One
/// `Remote::clear()` bumped the counter, and from then on every document
/// the store pushed was silently discarded, while the status counters
/// (fenced on the generation alone) went on reporting a healthy store.
///
/// `clear`'s own documentation is the contract: "a watch loop delivering
/// from the same store keeps delivering, and its next push installs
/// normally".
#[test]
fn a_document_pushed_after_a_clear_still_installs() {
    static REMOTE: Remote = Remote::new();

    REMOTE.set(ClearsBetweenPushes { remote: &REMOTE });

    let handle = RemoteWatch::new();

    REMOTE
        .watch(&handle.watching(), Duration::from_millis(10))
        .expect("the watch runs to the end of the store's own");

    assert_eq!(
        REMOTE.document().map(|document| document.text),
        Some(r#"{"db":{"port":2}}"#.to_owned()),
        "the push after the clear is the document that was kept"
    );

    let status = REMOTE.status();

    assert_eq!(
        status.fetches, 2,
        "both pushes counted, and neither was counted without being kept"
    );
}

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

    // After a failure the draw is *full* jitter — anywhere from nothing up
    // to the doubled interval — so a single sample has no lower bound to
    // assert. What grows is the range, and that is what gets measured:
    // sixteen draws at one failure cannot reach where sixteen draws at four
    // failures routinely do.
    let ceiling_of = |failures: u32| {
        let mut pace = Pace::new(Duration::from_secs(1));

        for _ in 0..failures {
            pace.failed();
        }

        (0..16).map(|_| pace.next_wait()).max().unwrap_or_default()
    };

    let after_one = ceiling_of(1);
    let after_four = ceiling_of(4);

    assert!(
        after_one <= Duration::from_secs(2),
        "one failure draws from twice the interval at most: {after_one:?}"
    );
    assert!(
        after_four > after_one,
        "the backoff has to grow: {after_four:?} after four, {after_one:?} after one"
    );
    assert!(
        after_four <= Duration::from_secs(16),
        "four failures draw from sixteen times the interval at most: {after_four:?}"
    );

    // The property the band cannot give: the low half of the range is
    // reachable. This is what decorrelates a fleet that all failed at the
    // same instant, and a ±25% band would make it impossible.
    let mut failing = Pace::new(Duration::from_secs(100));
    failing.failed();

    let draws: Vec<Duration> = (0..64).map(|_| failing.next_wait()).collect();

    assert!(
        draws.iter().any(|wait| *wait < Duration::from_secs(100)),
        "a failed wait must be able to fall below the interval, or the \
         backoff is a band and not full jitter"
    );
    assert!(
        draws.iter().all(|wait| *wait <= Duration::from_secs(200)),
        "and it must never exceed the backoff it was drawn from"
    );

    let mut growing = Pace::new(Duration::from_secs(1));

    for _ in 0..42 {
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
    assert!(
        growing.next_wait() >= Duration::from_millis(750),
        "and back to the *band*: a healthy interval is a promise about how \
         often a store is read, not a range to draw from"
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

/// A watch that ends when the process is winding down, rather than one the
/// caller has to remember to stop.
///
/// Driven here by a plain future on this crate's own tiny executor — no
/// runtime is imposed by `run_until`, which is the property being pinned:
/// the shutdown future is driven by whoever is already driving the caller.
#[test]
fn a_remote_watch_runs_until_the_shutdown_future_completes() {
    use dynamic_config::RemoteWatch;

    let watch = RemoteWatch::new();
    let watching = watch.watching();

    assert!(watching.keep_going());

    // A future that is already complete: the point is the sequencing, not
    // the waiting.
    block_on(watch.run_until(std::future::ready(())));

    assert!(
        !watching.keep_going(),
        "the loop must be told to stop once the shutdown future completes"
    );
}

/// The smallest executor that can drive a future to completion, so this test
/// pins "no runtime is imposed" rather than assuming one.
fn block_on<F: std::future::Future>(future: F) -> F::Output {
    use std::task::{Context, Poll, Waker};

    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    let mut future = Box::pin(future);

    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }

        std::thread::yield_now();
    }
}

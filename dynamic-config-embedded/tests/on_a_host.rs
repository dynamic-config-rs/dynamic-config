//! What a device would do, run on a host.
//!
//! The `std` feature supplies the `critical-section` implementation a device
//! gets from its HAL, and nothing else changes — so these exercise the same
//! code that runs on the part.
//!
//! Without `--features std,async` this whole file compiles to **zero tests**,
//! silently — the `cfg` below cannot be a `compile_error!`, because a plain
//! `cargo test --workspace` must still build. `just embedded` and CI both
//! pass the features; if a "green" run finished suspiciously fast, check the
//! feature flags before trusting it.

#![cfg(feature = "std")]

use dynamic_config_embedded::{ConfigCell, Error, ErrorKind, Format, Validate};
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct Settings {
    interval_ms: u32,
    verbose: bool,
}

impl Validate for Settings {
    fn validate(&self) -> Result<(), &'static str> {
        if self.interval_ms == 0 {
            return Err("interval_ms of zero would spin");
        }

        Ok(())
    }
}

fn defaults() -> Settings {
    Settings {
        interval_ms: 1000,
        verbose: false,
    }
}

#[test]
fn a_device_is_configured_before_anything_arrives() {
    static SETTINGS: ConfigCell<Settings> = ConfigCell::new();

    // Compiled-in defaults: a device that cannot reach its configuration source
    // still has to do something.
    SETTINGS.store(defaults());

    assert_eq!(SETTINGS.get().unwrap().interval_ms, 1000);
}

#[test]
fn a_document_replaces_the_whole_configuration() {
    static SETTINGS: ConfigCell<Settings> = ConfigCell::new();

    SETTINGS.store(defaults());
    SETTINGS
        .apply(br#"{"interval_ms": 250, "verbose": true}"#, Format::Json)
        .expect("the document fits");

    assert_eq!(
        SETTINGS.get().unwrap(),
        Settings {
            interval_ms: 250,
            verbose: true,
        },
        "a device gets one document at a time, not a layer"
    );
}

#[test]
fn nothing_a_link_can_send_takes_the_device_down() {
    static SETTINGS: ConfigCell<Settings> = ConfigCell::new();

    SETTINGS.store(defaults());

    for hostile in [
        b"".as_slice(),
        b"not json",
        b"{",
        br#"{"interval_ms": "not a number", "verbose": true}"#,
        br#"{"interval_ms": 250}"#,
        br#"{"interval_ms": 0, "verbose": true}"#,
        // Deeply nested, in case anything here recurses without a bound.
        br#"{"interval_ms": "x", "extra": [[[[[[[[1]]]]]]]]}"#,
    ] {
        // The assertion is that this *returns*. A serial link is untrusted
        // input, and a device that panics on one is a device that stops.
        let _: Result<(), Error> = SETTINGS.apply(hostile, Format::Json);
    }

    assert_eq!(
        SETTINGS.get().unwrap(),
        defaults(),
        "and none of them displaced the configuration that worked"
    );
}

/// A field the firmware does not know is *not* an error: serde ignores it, and
/// on a device that is the difference between a rolling upgrade and a fleet
/// that stops taking configuration. A firmware that wants the opposite says so
/// with `#[serde(deny_unknown_fields)]`.
#[test]
fn a_field_this_firmware_does_not_know_is_ignored_rather_than_fatal() {
    static SETTINGS: ConfigCell<Settings> = ConfigCell::new();

    SETTINGS.store(defaults());

    SETTINGS
        .apply(
            br#"{"interval_ms": 250, "verbose": true, "added_in_v2": 7}"#,
            Format::Json,
        )
        .expect("a newer sender must not brick an older device");

    assert_eq!(SETTINGS.get().unwrap().interval_ms, 250);
}

#[test]
fn a_rejected_document_names_the_rule_it_broke() {
    static SETTINGS: ConfigCell<Settings> = ConfigCell::new();

    SETTINGS.store(defaults());

    let error = SETTINGS
        .apply(br#"{"interval_ms": 0, "verbose": true}"#, Format::Json)
        .expect_err("zero would spin");

    assert_eq!(error.kind(), ErrorKind::Invalid);
    assert_eq!(error.message(), "interval_ms of zero would spin");
}

#[cfg(feature = "async")]
mod awaiting {
    use core::future::Future;
    use core::pin::Pin;
    use core::task::{Context, Poll, Waker};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::task::Wake;

    use super::*;

    /// A twenty-line executor, so "any runtime drives it" is checked rather
    /// than asserted. Embassy is one; this is another.
    struct Flag(AtomicBool);

    impl Wake for Flag {
        fn wake(self: Arc<Self>) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    /// Polls `future` until it is ready, running `between` after each poll that
    /// is not.
    fn drive<F: Future>(mut future: Pin<&mut F>, mut between: impl FnMut()) -> F::Output {
        let flag = Arc::new(Flag(AtomicBool::new(false)));
        let waker = Waker::from(Arc::clone(&flag));
        let mut context = Context::from_waker(&waker);

        loop {
            if let Poll::Ready(value) = future.as_mut().poll(&mut context) {
                return value;
            }

            between();

            assert!(
                flag.0.swap(false, Ordering::SeqCst),
                "a pending poll that never wakes is a hang, not a test failure"
            );
        }
    }

    static AWAITED: ConfigCell<Settings> = ConfigCell::new();

    #[test]
    fn a_task_wakes_on_the_next_configuration() {
        AWAITED.store(defaults());

        let mut changes = AWAITED.changes();
        // Pinned on the stack: `Box::pin` would need an allocator, which is the
        // whole point of this crate not having one.
        let future = std::pin::pin!(changes.changed());

        let value = drive(future, || {
            AWAITED
                .apply(br#"{"interval_ms": 250, "verbose": true}"#, Format::Json)
                .unwrap();
        });

        assert_eq!(value.interval_ms, 250);
    }

    static CROWDED: ConfigCell<Settings> = ConfigCell::new();

    #[test]
    fn a_fifth_waiter_wakes_the_task_it_evicts() {
        use dynamic_config_embedded::DEFAULT_WAITERS as WAITERS;

        CROWDED.store(defaults());

        // Fill every slot with distinct wakers, each backed by its own flag.
        let flags: Vec<Arc<Flag>> = (0..=WAITERS)
            .map(|_| Arc::new(Flag(AtomicBool::new(false))))
            .collect();

        let mut handles: Vec<_> = (0..=WAITERS).map(|_| CROWDED.changes()).collect();
        let mut futures: Vec<_> = handles
            .iter_mut()
            .map(|handle| Box::pin(handle.changed()))
            .collect();

        for (future, flag) in futures.iter_mut().zip(&flags) {
            let waker = Waker::from(Arc::clone(flag));
            let mut context = Context::from_waker(&waker);

            assert!(future.as_mut().poll(&mut context).is_pending());
        }

        // Registering the fifth evicted the first. The evicted task must be
        // WOKEN — a dropped waker is a task nobody polls again, which on a
        // device is a hang with no backtrace.
        assert!(
            flags[0].0.load(Ordering::SeqCst),
            "the evicted waiter was dropped silently instead of woken"
        );

        // And the wake is not a lie it tells everyone: the newest waiter is
        // still registered, quietly pending.
        assert!(!flags[WAITERS].0.load(Ordering::SeqCst));
    }
}

/// A custom waiter budget through the const parameter, and rejection of
/// trailing bytes — a reused link buffer must not smuggle a stale tail in.
#[test]
fn a_custom_waiter_budget_and_trailing_bytes() {
    use dynamic_config_embedded::ErrorKind;

    static ROOMY: ConfigCell<Settings, 8> = ConfigCell::new();

    ROOMY.store(defaults());

    // Two frames concatenated in one buffer: the first parses, but accepting
    // it would install a configuration nobody sent as "the" document.
    let error = ROOMY
        .apply(
            br#"{"interval_ms": 250, "verbose": true}{"interval_ms": 1}"#,
            Format::Json,
        )
        .expect_err("trailing bytes are a malformed document");

    assert_eq!(error.kind(), ErrorKind::Parse);
    assert_eq!(
        ROOMY.get().unwrap().interval_ms,
        1000,
        "the previous configuration keeps serving"
    );

    // A clean document still applies, custom budget and all.
    ROOMY
        .apply(br#"{"interval_ms": 250, "verbose": true}"#, Format::Json)
        .unwrap();
    assert_eq!(ROOMY.get().unwrap().interval_ms, 250);
}

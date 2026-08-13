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
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier, Mutex};
    use std::task::Wake;

    use dynamic_config_embedded::Changes;

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

        assert_eq!(
            CROWDED.waiter_evictions(),
            1,
            "the cell has to be able to say that the budget was exceeded"
        );
    }

    /// A ready queue, which is the part of an executor the waiter budget is
    /// actually about: a device is asleep while the queue is empty and drawing
    /// current while anything is in it.
    struct Executor {
        ready: Mutex<Vec<usize>>,
        /// Wake-ups per task, so "exactly once per change" is a claim about
        /// each task rather than about a total that could hide a double wake.
        wakes: Mutex<Vec<usize>>,
    }

    struct Task {
        id: usize,
        executor: Arc<Executor>,
    }

    impl Wake for Task {
        fn wake(self: Arc<Self>) {
            self.wake_by_ref();
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.executor.wakes.lock().unwrap()[self.id] += 1;
            self.executor.ready.lock().unwrap().push(self.id);
        }
    }

    struct Budget {
        /// Polls before the ready queue emptied — `tasks` when every waiter
        /// parks, and `cap` when they never stop waking each other.
        polls_to_idle: usize,
        idled: bool,
        /// Per task, for the one configuration change that follows.
        wakes: Vec<usize>,
        completed: usize,
    }

    /// Parks `tasks` waiters on `cell`, then — if the executor ever got to
    /// idle — changes the configuration once and polls everything again.
    fn park<const WAITERS: usize>(
        cell: &'static ConfigCell<Settings, WAITERS>,
        tasks: usize,
        cap: usize,
    ) -> Budget {
        cell.store(defaults());

        let executor = Arc::new(Executor {
            ready: Mutex::new((0..tasks).rev().collect()),
            wakes: Mutex::new(vec![0; tasks]),
        });

        let wakers: Vec<Waker> = (0..tasks)
            .map(|id| {
                Waker::from(Arc::new(Task {
                    id,
                    executor: Arc::clone(&executor),
                }))
            })
            .collect();

        let mut handles: Vec<Changes<Settings, WAITERS>> =
            (0..tasks).map(|_| cell.changes()).collect();
        let mut futures: Vec<_> = handles
            .iter_mut()
            .map(|handle| Box::pin(handle.changed()))
            .collect();

        let mut polls_to_idle = 0;

        let idled = loop {
            let next = executor.ready.lock().unwrap().pop();

            let Some(id) = next else {
                break true;
            };

            if polls_to_idle == cap {
                break false;
            }

            polls_to_idle += 1;

            let mut context = Context::from_waker(&wakers[id]);

            assert!(
                futures[id].as_mut().poll(&mut context).is_pending(),
                "nothing has changed yet, so nothing can be ready"
            );
        };

        if !idled {
            return Budget {
                polls_to_idle,
                idled,
                wakes: executor.wakes.lock().unwrap().clone(),
                completed: 0,
            };
        }

        *executor.wakes.lock().unwrap() = vec![0; tasks];

        cell.apply(br#"{"interval_ms": 250, "verbose": true}"#, Format::Json)
            .expect("the document fits");

        let wakes = executor.wakes.lock().unwrap().clone();
        let mut completed = 0;

        for (id, waker) in wakers.iter().enumerate() {
            let mut context = Context::from_waker(waker);

            if let Poll::Ready(settings) = futures[id].as_mut().poll(&mut context) {
                assert_eq!(settings.interval_ms, 250);
                completed += 1;
            }
        }

        Budget {
            polls_to_idle,
            idled,
            wakes,
            completed,
        }
    }

    static ONE: ConfigCell<Settings, 1> = ConfigCell::new();
    static FULL: ConfigCell<Settings, 4> = ConfigCell::new();
    static SIXTY_FOUR: ConfigCell<Settings, 64> = ConfigCell::new();

    /// Within the budget the executor reaches its idle loop, and one change
    /// wakes every waiter exactly once. Both halves matter: a device that is
    /// woken twice per change wastes power, and one that is never woken hangs.
    #[test]
    fn a_full_budget_parks_every_waiter_and_wakes_each_once() {
        for (report, tasks, evictions) in [
            (park(&ONE, 1, 1_000), 1, ONE.waiter_evictions()),
            (park(&FULL, 4, 1_000), 4, FULL.waiter_evictions()),
            (
                park(&SIXTY_FOUR, 64, 1_000),
                64,
                SIXTY_FOUR.waiter_evictions(),
            ),
        ] {
            assert!(report.idled, "{tasks} waiters within budget must all park");
            assert_eq!(
                report.polls_to_idle, tasks,
                "one poll each and then silence"
            );
            assert_eq!(report.wakes, vec![1; tasks], "exactly once per change");
            assert_eq!(report.completed, tasks);
            assert_eq!(evictions, 0, "nothing was displaced");
        }
    }

    static OVER: ConfigCell<Settings, 4> = ConfigCell::new();

    /// One task past the budget and the executor never idles again — the
    /// waiters trade wake-ups with no configuration change between them. This
    /// is the crate's documented degradation, and the measurement that says a
    /// larger default would only move it: nine tasks on an eight-slot cell do
    /// exactly this too.
    ///
    /// It is a livelock rather than a lost wake-up, which is the trade the
    /// fixed array makes deliberately. `waiter_evictions` is how a firmware
    /// finds out, since the symptom is otherwise a battery that empties early.
    #[test]
    fn one_task_past_the_budget_costs_the_device_its_idle_loop() {
        const CAP: usize = 5_000;

        let report = park(&OVER, 5, CAP);

        assert!(
            !report.idled,
            "five waiters on four slots cannot all park, and pretending otherwise \
             would mean one of them was dropped"
        );
        assert_eq!(report.polls_to_idle, CAP);
        assert!(
            report.wakes.iter().sum::<usize>() >= CAP - 5,
            "nearly every poll is a wake-up nobody asked for: {:?}",
            report.wakes
        );
        assert!(
            OVER.waiter_evictions() > 0,
            "the cell must be able to report that it is over budget"
        );
    }

    static RACED: ConfigCell<Settings, 4> = ConfigCell::new();

    /// The classic lost wake-up: a store that lands between the poll's first
    /// look at the generation and the registration of its waker. Registering
    /// into an array that `bump` has already drained wakes nobody, so without
    /// the re-check after registering, that task waits for a change that has
    /// already happened — on a device, a hang with no backtrace.
    ///
    /// A real thread does the storing, because the whole point is that the
    /// interleaving is not chosen here. Every round asserts the property that
    /// has to hold in all of them: once the store has completed and the poll
    /// has returned, the task is either finished or scheduled.
    #[test]
    fn a_store_that_races_the_registration_is_never_a_lost_wake_up() {
        RACED.store(defaults());

        for round in 0..2_000u32 {
            let flag = Arc::new(Flag(AtomicBool::new(false)));
            let waker = Waker::from(Arc::clone(&flag));
            let mut context = Context::from_waker(&waker);

            let mut changes = RACED.changes();
            let mut future = std::pin::pin!(changes.changed());

            let gate = Arc::new(Barrier::new(2));
            let storer = {
                let gate = Arc::clone(&gate);

                std::thread::spawn(move || {
                    gate.wait();
                    RACED.store(Settings {
                        interval_ms: 1_000 + round,
                        verbose: false,
                    });
                })
            };

            gate.wait();
            let polled = future.as_mut().poll(&mut context);
            storer.join().expect("the storing thread must not panic");

            assert!(
                matches!(polled, Poll::Ready(_)) || flag.0.load(Ordering::SeqCst),
                "round {round}: the store completed and the poll returned pending \
                 without waking anyone — nothing will ever poll that task again"
            );
        }
    }

    /// A waker that reaches back into the cell from inside `wake`.
    ///
    /// That is not a contrived thing to do: an executor may poll the task it
    /// has just woken before `wake` returns, and on a device an interrupt that
    /// stores a configuration can land in the same place. Either re-enters the
    /// cell, and `critical_section::with` nests — so what would break is the
    /// `RefCell` inside it, with a panic, on a target that cannot unwind.
    ///
    /// Once only: a store from inside a wake that the store itself caused
    /// would otherwise recurse forever, which is a different bug.
    struct Reentrant {
        cell: &'static ConfigCell<Settings, 4>,
        wakes: AtomicUsize,
    }

    impl Wake for Reentrant {
        fn wake(self: Arc<Self>) {
            self.wake_by_ref();
        }

        fn wake_by_ref(self: &Arc<Self>) {
            if self.wakes.fetch_add(1, Ordering::SeqCst) == 0 {
                self.cell.store(Settings {
                    interval_ms: 7,
                    verbose: true,
                });

                assert_eq!(self.cell.get().unwrap().interval_ms, 7);

                // Reading the counter re-enters the notify state, which is the
                // borrow that would still be held if a wake happened inside a
                // section.
                let _ = self.cell.waiter_evictions();
            }
        }
    }

    static WOKEN_BY_A_CHANGE: ConfigCell<Settings, 4> = ConfigCell::new();

    /// No critical section is held while a configuration change wakes its
    /// waiters — proved by storing from inside the wake.
    #[test]
    fn a_change_wakes_its_waiters_with_nothing_borrowed() {
        WOKEN_BY_A_CHANGE.store(defaults());

        let reentrant = Arc::new(Reentrant {
            cell: &WOKEN_BY_A_CHANGE,
            wakes: AtomicUsize::new(0),
        });
        let waker = Waker::from(Arc::clone(&reentrant));
        let mut context = Context::from_waker(&waker);

        let mut changes = WOKEN_BY_A_CHANGE.changes();
        let mut future = std::pin::pin!(changes.changed());

        assert!(future.as_mut().poll(&mut context).is_pending());

        WOKEN_BY_A_CHANGE
            .apply(br#"{"interval_ms": 250, "verbose": true}"#, Format::Json)
            .expect("the document fits");

        assert_eq!(reentrant.wakes.load(Ordering::SeqCst), 1);
        assert_eq!(
            WOKEN_BY_A_CHANGE.get().unwrap().interval_ms,
            7,
            "the store from inside the wake is the one that ran last"
        );
    }

    static WOKEN_BY_AN_EVICTION: ConfigCell<Settings, 4> = ConfigCell::new();

    /// The same, for the other place a waker is woken: the eviction inside a
    /// registration, where the wake happens while another task is in the
    /// middle of its own `poll`.
    #[test]
    fn an_eviction_wakes_the_displaced_waiter_with_nothing_borrowed() {
        WOKEN_BY_AN_EVICTION.store(defaults());

        let reentrant = Arc::new(Reentrant {
            cell: &WOKEN_BY_AN_EVICTION,
            wakes: AtomicUsize::new(0),
        });
        let waker = Waker::from(Arc::clone(&reentrant));

        let mut first = WOKEN_BY_AN_EVICTION.changes();
        let mut displaced = std::pin::pin!(first.changed());

        assert!(displaced
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending());

        // Four more distinct wakers: the fourth of them finds every slot taken
        // and displaces the one above, waking it mid-registration.
        let mut handles: Vec<_> = (0..4).map(|_| WOKEN_BY_AN_EVICTION.changes()).collect();
        let mut crowd: Vec<_> = handles
            .iter_mut()
            .map(|handle| Box::pin(handle.changed()))
            .collect();

        for future in &mut crowd {
            let flag = Waker::from(Arc::new(Flag(AtomicBool::new(false))));

            let _ = future.as_mut().poll(&mut Context::from_waker(&flag));
        }

        assert_eq!(
            reentrant.wakes.load(Ordering::SeqCst),
            1,
            "the displaced waiter is woken once, from inside the registration"
        );
        assert_eq!(
            WOKEN_BY_AN_EVICTION.get().unwrap().interval_ms,
            7,
            "and its wake was free to store a configuration of its own"
        );
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

//! Stopping a blocking watch.
//!
//! A watch nobody owns is a leak nobody asked for, so the handle stops the
//! loop when it is dropped and `RemoteWatch::detach` is how a caller says
//! *this one really should run forever*. Only blocking loops need this: an
//! async watch is a future, and dropping it is stopping it.

use std::sync::{Arc, Weak};
use std::time::Duration;

use crate::sync::atomic::{AtomicBool, Ordering};

/// A running blocking watch, from the caller's side.
///
/// Dropping it stops the loop — the same contract the file watcher's
/// `WatchHandle` has, for the same reason: a watch nobody owns is a leak nobody
/// asked for. [`detach`](Self::detach) is the way to say *this one really should
/// run forever*.
///
/// Only blocking loops need this. An async watch is a future: drop it and it is
/// cancelled, on any executor.
///
/// ```no_run
/// # use dynamic_config::RemoteWatch;
/// # struct Consul;
/// # impl Consul {
/// #     fn watch(&self, _: dynamic_config::Watching, _: fn(dynamic_config::Fetched) -> Result<(), dynamic_config::Error>) -> Result<(), dynamic_config::Error> { Ok(()) }
/// # }
/// # fn example(consul: Consul) {
/// # fn apply(_: dynamic_config::Fetched) -> Result<(), dynamic_config::Error> { Ok(()) }
/// let watch = RemoteWatch::new();
/// let watching = watch.watching();
///
/// std::thread::spawn(move || consul.watch(watching, apply));
///
/// // ... and later, or by dropping `watch`:
/// watch.stop();
/// # }
/// ```
#[must_use = "dropping the handle stops the watch; bind it, or call `.detach()` \
              to watch for the rest of the process"]
#[derive(Debug)]
pub struct RemoteWatch {
    running: Arc<AtomicBool>,
}

impl RemoteWatch {
    /// A handle for a watch that has not been handed to a loop yet.
    pub fn new() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(true)),
        }
    }

    /// The loop's half of this handle.
    ///
    /// Hand it to the watch; keep the `RemoteWatch` yourself.
    #[must_use]
    pub fn watching(&self) -> Watching {
        Watching {
            running: Arc::downgrade(&self.running),
        }
    }

    /// Stops the loop at its next check.
    ///
    /// *At its next check* is the whole caveat, and it is not small: a loop
    /// parked in a blocking query does not return until the store answers or
    /// the wait expires, so the store's wait time is the worst-case delay. Each
    /// companion crate documents its own.
    pub fn stop(&self) {
        self.running.store(false, Ordering::Release);
    }

    /// Whether the loop has been told to stop.
    #[must_use]
    pub fn is_stopped(&self) -> bool {
        !self.running.load(Ordering::Acquire)
    }

    /// Watches for the remainder of the process.
    ///
    /// Leaks the handle on purpose, exactly as the file watcher's
    /// `WatchHandle::detach` does: a watch that must never stop has no owner to
    /// hold it, and pretending otherwise is how it ends up stopped at the end of
    /// `main`'s first statement.
    pub fn detach(self) {
        std::mem::forget(self);
    }
}

impl Default for RemoteWatch {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for RemoteWatch {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The loop's half of a [`RemoteWatch`].
///
/// A `Weak`, so a handle that is dropped without anyone remembering to call
/// `stop` still ends the loop: the upgrade fails and
/// [`keep_going`](Self::keep_going) answers `false`.
#[derive(Debug, Clone)]
pub struct Watching {
    running: Weak<AtomicBool>,
}

impl Watching {
    /// Whether the loop should go round again.
    ///
    /// `false` once the caller called [`RemoteWatch::stop`] or dropped the
    /// handle. Check it before every request, not only after one: a loop that
    /// checks only on the way out issues one more query than it was asked to.
    #[must_use]
    pub fn keep_going(&self) -> bool {
        self.running
            .upgrade()
            .is_some_and(|running| running.load(Ordering::Acquire))
    }

    /// Sleeps for `total`, waking early if the watch is stopped.
    ///
    /// The polling loop every blocking store crate writes: sleep a slice,
    /// check [`keep_going`](Self::keep_going), repeat — so a stopped watch
    /// ends within a quarter second instead of at the end of its interval.
    /// Here once, rather than once per store crate.
    pub fn sleep_for(&self, total: Duration) {
        const SLICE: Duration = Duration::from_millis(250);

        let mut slept = Duration::ZERO;

        while slept < total && self.keep_going() {
            std::thread::sleep(SLICE.min(total - slept));
            slept += SLICE;
        }
    }

    /// Sleeps for `total`, jittered, waking early if the watch is stopped.
    ///
    /// A fleet started by one rollout polls in lockstep otherwise: fifty
    /// replicas with a thirty-second interval become fifty simultaneous
    /// requests every thirty seconds, and the store sees a spike rather than
    /// a trickle. The spread is drawn once per loop from the clock, so two
    /// processes on one machine differ and a restart does not land back in
    /// the same phase.
    pub fn sleep_jittered(&self, total: Duration, pace: &mut Pace) {
        self.sleep_for(pace.spread(total));
    }

    /// A token for a watch that should never stop.
    ///
    /// For a loop the caller genuinely wants to outlive everything, so there is
    /// no handle to hold. Prefer [`RemoteWatch::detach`], which says the same
    /// thing at the point where somebody decided it.
    #[must_use]
    pub fn forever() -> Self {
        // A `Weak` that can never upgrade would stop the loop immediately, so
        // this leaks one live flag — one allocation, once, for the life of the
        // process.
        let running = Box::leak(Box::new(Arc::new(AtomicBool::new(true))));

        Self {
            running: Arc::downgrade(running),
        }
    }
}

/// The waits a watch loop makes.
///
/// Two of them, and a loop needs both: the pause between healthy rounds,
/// spread so a fleet does not poll in lockstep, and the growing pause after
/// a failure, so a store that is down is not hammered by everything that
/// depends on it. A loop that sleeps its plain interval after an error is
/// the shape that turns one outage into two.
///
/// ```
/// # use std::time::Duration;
/// # use dynamic_config::{Pace, Watching};
/// # fn example(watching: &Watching) {
/// let mut pace = Pace::new(Duration::from_secs(30));
///
/// while watching.keep_going() {
///     match fetch() {
///         Ok(()) => pace.succeeded(),
///         Err(()) => pace.failed(),
///     }
///
///     pace.wait(watching);
/// }
/// # }
/// # fn fetch() -> Result<(), ()> { Ok(()) }
/// ```
#[derive(Debug, Clone)]
pub struct Pace {
    interval: Duration,
    ceiling: Duration,
    failures: u32,
    entropy: u64,
}

impl Pace {
    /// The default ceiling a backoff grows to.
    const CEILING: Duration = Duration::from_secs(300);

    /// Rounds `interval` apart when things are going well.
    #[must_use]
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            ceiling: Self::CEILING,
            failures: 0,
            entropy: seed(),
        }
    }

    /// Caps how far a backoff grows. Five minutes by default.
    #[must_use]
    pub fn with_ceiling(mut self, ceiling: Duration) -> Self {
        self.ceiling = ceiling;
        self
    }

    /// A round went well: the next wait is the plain interval again.
    pub fn succeeded(&mut self) {
        self.failures = 0;
    }

    /// A round failed: the next wait is longer than the last one.
    pub fn failed(&mut self) {
        self.failures = self.failures.saturating_add(1);
    }

    /// How long to wait before the next round, jittered.
    ///
    /// The interval while things work; doubling from it after each failure,
    /// up to the ceiling.
    #[must_use]
    pub fn next_wait(&mut self) -> Duration {
        let base = if self.failures == 0 {
            self.interval
        } else {
            // Doubling, but never past the ceiling and never past what a
            // `Duration` can hold — a loop that has failed for a week must
            // not overflow its way back down to no wait at all.
            let factor = 1u32.checked_shl(self.failures.min(16)).unwrap_or(u32::MAX);

            self.interval
                .checked_mul(factor)
                .unwrap_or(self.ceiling)
                .min(self.ceiling)
        };

        self.spread(base)
    }

    /// Waits for [`next_wait`](Self::next_wait), waking early if the watch
    /// is stopped.
    pub fn wait(&mut self, watching: &Watching) {
        let wait = self.next_wait();

        watching.sleep_for(wait);
    }

    /// `base`, moved by up to a quarter of itself in either direction.
    ///
    /// Deterministic given the seed, so a test can pin it; drawn from the
    /// clock at construction, so two processes do not share a phase.
    #[must_use]
    pub fn spread(&mut self, base: Duration) -> Duration {
        // An LCG rather than a dependency: the numbers only have to be
        // uncorrelated between processes, which is a far weaker ask than
        // anything a random number generator is built for.
        self.entropy = self
            .entropy
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);

        let quarter = base / 4;
        let offset = quarter
            .checked_mul(u32::try_from(self.entropy >> 33 & 0xFF).unwrap_or(0))
            .unwrap_or(quarter)
            / 255;

        if self.entropy & 1 == 0 {
            base.saturating_add(offset)
        } else {
            base.saturating_sub(offset)
        }
    }
}

/// Something different per process, without a dependency.
fn seed() -> u64 {
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos() as u64);

    since ^ u64::from(std::process::id()).wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

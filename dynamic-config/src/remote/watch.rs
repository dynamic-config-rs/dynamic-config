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

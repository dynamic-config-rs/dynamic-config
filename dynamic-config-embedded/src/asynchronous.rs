//! Awaiting the next configuration, with no allocator and no runtime.
//!
//! The same design as the `std` crate's `changes()`: a generation counter and
//! the tasks waiting on it. What differs is the storage — a `Vec<Waker>` needs
//! an allocator, so this keeps a fixed number of slots in a critical section.
//!
//! [`WAITERS`] is the number. Four is chosen for the shape of the problem
//! rather than as a guess: a device has a handful of tasks that care about
//! configuration, not a handful of thousands. A fifth waiter replaces the
//! oldest rather than being dropped — a task that is never woken is a bug that
//! shows up as a hang, and one that is woken early merely polls again.

use core::cell::RefCell;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};

use critical_section::Mutex;

/// How many tasks can await one configuration at a time.
pub const WAITERS: usize = 4;

/// A generation counter and the tasks waiting on it.
#[derive(Debug)]
pub(crate) struct Notify {
    inner: Mutex<RefCell<State>>,
}

#[derive(Debug)]
struct State {
    generation: u32,
    waiting: [Option<Waker>; WAITERS],
}

impl Notify {
    pub(crate) const fn new() -> Self {
        Self {
            inner: Mutex::new(RefCell::new(State {
                generation: 0,
                waiting: [const { None }; WAITERS],
            })),
        }
    }

    pub(crate) fn generation(&self) -> u32 {
        critical_section::with(|token| self.inner.borrow(token).borrow().generation)
    }

    /// Records a new configuration and wakes everything waiting.
    pub(crate) fn bump(&self) {
        let woken = critical_section::with(|token| {
            let mut state = self.inner.borrow(token).borrow_mut();

            // Wrapping rather than saturating: a saturated counter stops
            // moving, and a counter that stops moving means every `changed()`
            // after reload 4294967295 hangs forever. A wrap can at worst
            // confuse a handle that slept through exactly 2^32 reloads —
            // one missed wake-up on a device that has reloaded four billion
            // times, against a permanent hang for everyone. Not close.
            state.generation = state.generation.wrapping_add(1);

            core::mem::take(&mut state.waiting)
        });

        // Woken outside the section: a waker may poll immediately, on this
        // core, and try to register again.
        for waker in woken.into_iter().flatten() {
            waker.wake();
        }
    }

    fn register(&self, waker: &Waker) {
        let evicted = critical_section::with(|token| {
            let mut state = self.inner.borrow(token).borrow_mut();

            for slot in &mut state.waiting {
                match slot {
                    Some(existing) if existing.will_wake(waker) => return None,
                    Some(_) => {}
                    None => {
                        *slot = Some(waker.clone());

                        return None;
                    }
                }
            }

            // Full. The oldest is *woken*, not dropped: a dropped waker is a
            // task nobody will ever poll again, and that is a hang. Woken, it
            // polls, sees no change, and re-registers — a spurious wake costs
            // one poll.
            state.waiting[0].replace(waker.clone())
        });

        // Woken outside the section, same as `bump`: the wake may poll the
        // evicted task immediately, on this core, and it will want the
        // critical section for its own re-registration.
        if let Some(evicted) = evicted {
            evicted.wake();
        }
    }
}

/// A handle that resolves each time the configuration is replaced.
///
/// Runtime-agnostic, because it is a `Future` and nothing more: Embassy, RTIC
/// and a hand-written `poll` loop all drive it.
///
/// The configuration current when this was created counts as already seen, so
/// the first [`changed`](Self::changed) waits for the *next* one.
pub struct Changes<T: Clone + 'static> {
    cell: &'static crate::ConfigCell<T>,
    seen: u32,
}

impl<T: Clone + 'static> Changes<T> {
    pub(crate) fn new(cell: &'static crate::ConfigCell<T>) -> Self {
        Self {
            seen: cell.notify().generation(),
            cell,
        }
    }

    /// Resolves with the configuration installed by the next change.
    ///
    /// Changes that land while nothing is awaiting are not queued: waking up to
    /// the *latest* configuration is what a reader wants, and a queue would
    /// hand it stale ones first — which on a device means acting on a setting
    /// that has already been superseded.
    pub fn changed(&mut self) -> impl Future<Output = T> + '_ {
        Changed { changes: self }
    }

    /// The generation this handle has already observed.
    #[must_use]
    pub const fn seen(&self) -> u32 {
        self.seen
    }
}

impl<T: Clone + 'static> core::fmt::Debug for Changes<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Changes")
            .field("seen", &self.seen)
            .finish_non_exhaustive()
    }
}

struct Changed<'a, T: Clone + 'static> {
    changes: &'a mut Changes<T>,
}

impl<T: Clone + 'static> Future for Changed<'_, T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<T> {
        let changes = &mut self.get_mut().changes;

        if let Some(value) = take(changes) {
            return Poll::Ready(value);
        }

        changes.cell.notify().register(context.waker());

        // Checked again after registering: a store between the first check and
        // the registration would otherwise be a wake-up nobody receives.
        match take(changes) {
            Some(value) => Poll::Ready(value),
            None => Poll::Pending,
        }
    }
}

fn take<T: Clone + 'static>(changes: &mut Changes<T>) -> Option<T> {
    let current = changes.cell.notify().generation();

    if current == changes.seen {
        return None;
    }

    changes.seen = current;

    // A non-zero generation means `store` ran, so there is a value.
    changes.cell.get()
}

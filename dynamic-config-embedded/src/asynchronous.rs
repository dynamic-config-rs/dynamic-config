//! Awaiting the next configuration, with no allocator and no runtime.
//!
//! The same design as the `std` crate's `changes()`: a generation counter and
//! the tasks waiting on it. What differs is the storage — a `Vec<Waker>` needs
//! an allocator, so this keeps a fixed number of slots in a critical section.
//!
//! [`DEFAULT_WAITERS`] is the number, and it is a type parameter because the
//! right number is a property of the firmware, not of this crate: a device's
//! tasks are known at compile time, so the count of them that can await a
//! configuration is known at compile time too. That is why there is no queue
//! here and no plan for one — see the module's `Beyond the budget` note below.
//!
//! Four is the default for the shape of the problem rather than as a guess: a
//! device has a handful of tasks that care about configuration, not a handful
//! of thousands. A fifth waiter replaces the occupant of a slot rather than
//! being dropped — a task that is never woken is a bug that shows up as a hang,
//! and one that is woken early merely polls again.
//!
//! # Beyond the budget
//!
//! With more waiting tasks than slots there is no honest outcome, only a choice
//! of bad ones: a fixed array cannot park what does not fit. Dropping a waker
//! hangs a task with no diagnostic; refusing the registration hangs it too,
//! because a `Future` that returns `Pending` without a registered waker is one
//! nobody will poll again. So this evicts and wakes, which loses no wake-up —
//! and, measured, costs a device its idle loop: five tasks on a four-slot cell
//! wake each other without end, with no configuration change to show for it.
//!
//! That is a livelock, not mere churn, and the only fix is to not be over
//! budget. [`ConfigCell::waiter_evictions`](crate::ConfigCell::waiter_evictions)
//! is how a firmware finds out that it is — non-zero means `WAITERS` is too
//! small, and it is a number a lab bench can read long before a battery does.
//!
//! # Interrupts
//!
//! Every read and write of the state below happens inside
//! `critical_section::with`, so registering a waker, storing a configuration
//! and reading one are all sound from an interrupt handler. Two rules make
//! that true, and both are load-bearing:
//!
//! - no borrow of the `RefCell` outlives its critical section, so an interrupt
//!   that lands between two of them finds nothing borrowed;
//! - no `Waker` is woken while the section is held. A `wake` implementation
//!   belongs to the executor and may do anything — including storing a
//!   configuration or polling the task it just woke, on this core, before it
//!   returns. Waking inside the section would re-enter the `RefCell` and panic.
//!
//! The cost is the length of the section: a scan of `WAITERS` slots and at most
//! one `Waker` clone, which is two word stores. Nothing here parses, and
//! nothing here waits.

use core::cell::RefCell;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};

use critical_section::Mutex;

use crate::DEFAULT_WAITERS;

/// A generation counter and the tasks waiting on it.
#[derive(Debug)]
pub(crate) struct Notify<const WAITERS: usize> {
    inner: Mutex<RefCell<State<WAITERS>>>,
}

#[derive(Debug)]
struct State<const WAITERS: usize> {
    generation: u32,
    /// Registrations that had to displace another waiter, saturating. Four
    /// bytes per cell, and the only way a firmware learns that `WAITERS` is
    /// too small before the symptom — a device that never idles — reaches a
    /// battery.
    evictions: u32,
    waiting: [Option<Waker>; WAITERS],
}

impl<const WAITERS: usize> Notify<WAITERS> {
    pub(crate) const fn new() -> Self {
        Self {
            inner: Mutex::new(RefCell::new(State {
                generation: 0,
                evictions: 0,
                waiting: [const { None }; WAITERS],
            })),
        }
    }

    pub(crate) fn generation(&self) -> u32 {
        critical_section::with(|token| self.inner.borrow(token).borrow().generation)
    }

    pub(crate) fn evictions(&self) -> u32 {
        critical_section::with(|token| self.inner.borrow(token).borrow().evictions)
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

            // `mem::replace`, not `mem::take`: `Default` for arrays is only
            // implemented up to fixed lengths, and a const-generic length is
            // not among them. The replacement is the same all-`None` array.
            core::mem::replace(&mut state.waiting, [const { None }; WAITERS])
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

            // Full. The occupant of slot 0 — not necessarily the oldest,
            // since `bump` empties every slot and refills happen in scan
            // order — is *woken*, not dropped: a dropped waker is a task
            // nobody will ever poll again, and that is a hang. Woken, it
            // polls, sees no change, and re-registers.
            //
            // Refusing this registration instead would be the same hang with
            // a different name: `poll` would return `Pending` with no waker
            // anywhere, and nothing would ever poll the task again.
            //
            // With a *steady state* of more waiters than slots, that
            // re-registration evicts somebody else and it never settles:
            // measured, five tasks on a four-slot cell trade wake-ups for as
            // long as anyone watches, with no configuration change between
            // them, and the executor never reaches its idle loop. No wake-up
            // is lost — the cost is entirely power. That is why the slot count
            // is a type parameter: size it to the real number of waiting
            // tasks, which on a device is a number the firmware knows.
            //
            // The counter is what makes that diagnosable instead of a mystery
            // brown-out. Saturating, because the question it answers is "did
            // this ever happen", and a wrap could answer "no" to it.
            state.evictions = state.evictions.saturating_add(1);

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
pub struct Changes<T: Clone + 'static, const WAITERS: usize = DEFAULT_WAITERS> {
    cell: &'static crate::ConfigCell<T, WAITERS>,
    seen: u32,
}

impl<T: Clone + 'static, const WAITERS: usize> Changes<T, WAITERS> {
    pub(crate) fn new(cell: &'static crate::ConfigCell<T, WAITERS>) -> Self {
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

impl<T: Clone + 'static, const WAITERS: usize> core::fmt::Debug for Changes<T, WAITERS> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Changes")
            .field("seen", &self.seen)
            .finish_non_exhaustive()
    }
}

struct Changed<'a, T: Clone + 'static, const WAITERS: usize> {
    changes: &'a mut Changes<T, WAITERS>,
}

impl<T: Clone + 'static, const WAITERS: usize> Future for Changed<'_, T, WAITERS> {
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

fn take<T: Clone + 'static, const WAITERS: usize>(changes: &mut Changes<T, WAITERS>) -> Option<T> {
    let current = changes.cell.notify().generation();

    if current == changes.seen {
        return None;
    }

    changes.seen = current;

    // A non-zero generation means `store` ran, so there is a value.
    changes.cell.get()
}

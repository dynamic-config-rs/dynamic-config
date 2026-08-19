//! Async support that does not name a runtime.
//!
//! Two things here needed a runtime, and neither actually does.
//!
//! **Waiting for a reload.** The obvious implementation returns a
//! `tokio::sync::watch::Receiver`, and then the crate only works on tokio. But
//! a change notification is a generation counter and a list of wakers, and both
//! of those are `std`. [`Changes`] is that, and any executor drives it.
//!
//! **Running the load off the async thread.** This one is genuinely
//! runtime-specific — a blocking pool belongs to a runtime. So it is pluggable
//! instead: with the `tokio` feature it uses `spawn_blocking`, with an executor
//! installed by [`set_blocking_executor`] it uses that, and otherwise it spawns
//! a thread. A configuration load happens at startup and on reload, so a thread
//! per call is a real answer rather than a placeholder.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

use crate::sync::atomic::{AtomicU64, Ordering};
use crate::sync::Mutex;
use std::task::{Context, Poll, Waker};

use crate::error::{Error, ErrorKind};

// ---------------------------------------------------------------------------
// Change notification
// ---------------------------------------------------------------------------

/// A generation counter and the tasks waiting on it.
///
/// Const-constructible, so it lives inside a `ConfigCell` in a `static`.
#[derive(Debug)]
pub struct Notify {
    /// Bumped on every store. Zero means nothing has been stored yet.
    generation: AtomicU64,
    waiting: Mutex<Vec<Waker>>,
}

impl Notify {
    #[cfg(not(loom))]
    pub const fn new() -> Self {
        Self {
            generation: AtomicU64::new(0),
            waiting: Mutex::new(Vec::new()),
        }
    }

    /// The same, minus `const`: loom's constructors are not.
    #[cfg(loom)]
    pub fn new() -> Self {
        Self {
            generation: AtomicU64::new(0),
            waiting: Mutex::new(Vec::new()),
        }
    }

    /// The current change counter — each bump is one reload observed.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Records a new snapshot and wakes everything waiting.
    pub fn bump(&self) {
        self.generation.fetch_add(1, Ordering::Release);

        let woken = {
            let mut waiting = self.lock();

            std::mem::take(&mut *waiting)
        };

        // Woken outside the lock: a waker may poll immediately, on this thread,
        // and try to register again.
        for waker in woken {
            waker.wake();
        }
    }

    /// The whole wait step: has the generation moved past `seen`, and did
    /// `load` produce the value it implies?
    ///
    /// Check, register, check again — a bump landing between the first
    /// check and the registration would otherwise be a wake-up nobody
    /// receives. This is the one copy of that protocol; `Changes` polls
    /// through it, and the loom suite drives exactly this function.
    pub fn poll_with<T>(
        &self,
        seen: &mut u64,
        waker: &Waker,
        mut load: impl FnMut() -> Option<T>,
    ) -> std::task::Poll<T> {
        let mut attempt = |seen: &mut u64| -> Option<T> {
            let current = self.generation();

            if current == *seen {
                return None;
            }

            *seen = current;

            load()
        };

        if let Some(value) = attempt(seen) {
            return std::task::Poll::Ready(value);
        }

        self.register(waker);

        match attempt(seen) {
            Some(value) => std::task::Poll::Ready(value),
            None => std::task::Poll::Pending,
        }
    }

    fn register(&self, waker: &Waker) {
        let mut waiting = self.lock();

        if waiting.iter().any(|existing| existing.will_wake(waker)) {
            return;
        }

        waiting.push(waker.clone());
    }

    fn lock(&self) -> crate::sync::MutexGuard<'_, Vec<Waker>> {
        self.waiting
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// A handle that resolves each time the configuration is replaced.
///
/// Runtime-agnostic: tokio, async-std, smol and a hand-written executor all
/// drive it the same way, because it is a `Future` and nothing more.
///
/// The snapshot current when this was created counts as already seen, so the
/// first [`changed`](Self::changed) waits for the *next* reload. Read the value
/// you start from with `current()`.
///
/// A handle created **before** `init()` has seen nothing, so the initial
/// install is its first change — which makes `changes()` double as "wake me
/// when configuration exists". That is contract, not accident: a task can be
/// spawned before the configuration loads and pick up the moment it does.
///
/// # Example
///
/// ```ignore
/// let mut changes = DbConfig::changes();
///
/// while let config = changes.changed().await {
///     pool.resize(config.pool_size);
/// }
/// ```
pub struct Changes<T: Send + Sync + 'static> {
    cell: CellRef<T>,
    seen: u64,
    /// The published install counter last yielded. The wake counter above
    /// also moves for refusals (so the events stream can deliver them);
    /// this one is what keeps a refusal from resolving `changed()` with an
    /// unchanged snapshot.
    seen_generation: u64,
}

/// The cell a `Changes` watches: a type's `static`, or an instance's own.
///
/// Two known shapes, so the type-keyed path stays a bare pointer — the
/// `Arc` exists only where an instance's cell has to outlive the `Dynamic`
/// that handed the `Changes` out.
enum CellRef<T: 'static> {
    Static(&'static crate::ConfigCell<T>),
    Shared(std::sync::Arc<crate::ConfigCell<T>>),
}

impl<T> CellRef<T> {
    fn get(&self) -> &crate::ConfigCell<T> {
        match self {
            Self::Static(cell) => cell,
            Self::Shared(cell) => cell,
        }
    }
}

impl<T: Send + Sync + 'static> Changes<T> {
    pub(crate) fn new(cell: &'static crate::ConfigCell<T>) -> Self {
        Self {
            seen: cell.notify().generation(),
            seen_generation: cell.generation(),
            cell: CellRef::Static(cell),
        }
    }

    /// A `Changes` over an instance's shared cell; what
    /// [`Dynamic::changes`](crate::Dynamic::changes) hands out.
    pub(crate) fn new_shared(cell: std::sync::Arc<crate::ConfigCell<T>>) -> Self {
        Self {
            seen: cell.notify().generation(),
            seen_generation: cell.generation(),
            cell: CellRef::Shared(cell),
        }
    }

    /// Resolves with the snapshot installed by the next reload.
    ///
    /// Reloads that land while nothing is awaiting are not queued: waking up to
    /// the *latest* configuration is what a reader wants, and a queue would
    /// hand it stale ones first.
    pub fn changed(&mut self) -> impl Future<Output = Arc<T>> + '_ {
        Changed { changes: self }
    }

    /// The generation this handle has already observed.
    ///
    /// Zero before anything has been stored.
    #[must_use]
    pub fn seen(&self) -> u64 {
        self.seen
    }
}

impl<T: Send + Sync + 'static> std::fmt::Debug for Changes<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Changes")
            .field("seen", &self.seen)
            // The cell is a `&'static` with no useful rendering.
            .finish_non_exhaustive()
    }
}

struct Changed<'a, T: Send + Sync + 'static> {
    changes: &'a mut Changes<T>,
}

impl<T: Send + Sync + 'static> Future for Changed<'_, T> {
    type Output = Arc<T>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Arc<T>> {
        let changes = &mut self.get_mut().changes;
        let Changes {
            cell,
            seen,
            seen_generation,
        } = changes;
        let cell = cell.get();
        let notify = cell.notify();

        // The check-register-check protocol lives in `poll_with`; the load
        // closure supplies the value a moved generation implies — filtered
        // on the PUBLISHED install counter, because the wake counter also
        // moves for refusals and a refusal must cost a success waiter at
        // most a re-registration, never a spurious yield.
        notify.poll_with(seen, context.waker(), || {
            let generation = cell.generation();

            if generation == *seen_generation {
                return None;
            }

            *seen_generation = generation;

            cell.load()
        })
    }
}

// ---------------------------------------------------------------------------
// Events: installs and refusals, one stream
// ---------------------------------------------------------------------------

/// One thing that happened to a configuration: an install, or a refusal.
///
/// What [`Events`] yields. `Refused` carries the [`FailureStatus`] — the
/// category and the key path, never a message and never a value, the same
/// discipline as every diagnostic surface here.
///
/// [`FailureStatus`]: crate::FailureStatus
#[non_exhaustive]
pub enum Event<T> {
    /// A reload installed a snapshot.
    #[non_exhaustive]
    Reloaded {
        /// The snapshot now serving.
        current: Arc<T>,
        /// The generation it became, and when it landed.
        meta: crate::SnapshotMeta,
        /// What caused the install, when the cell recorded one.
        reason: Option<crate::ReloadReason>,
    },
    /// A reload installed nothing; the previous snapshot keeps serving.
    Refused(crate::FailureStatus),
}

// Hand-written for the same reason `ReloadEvent`'s is: a derive would
// demand `T: Clone`.
impl<T> Clone for Event<T> {
    fn clone(&self) -> Self {
        match self {
            Self::Reloaded {
                current,
                meta,
                reason,
            } => Self::Reloaded {
                current: Arc::clone(current),
                meta: *meta,
                reason: reason.clone(),
            },
            Self::Refused(status) => Self::Refused(status.clone()),
        }
    }
}

impl<T> std::fmt::Debug for Event<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Presence and metadata, never the snapshot: an event is a
        // diagnostic, and a configuration holds passwords.
        match self {
            Self::Reloaded { meta, reason, .. } => f
                .debug_struct("Reloaded")
                .field("generation", &meta.generation)
                .field("reason", reason)
                .finish_non_exhaustive(),
            Self::Refused(status) => f.debug_tuple("Refused").field(status).finish(),
        }
    }
}

/// A handle that resolves for installs **and refusals**, where [`Changes`]
/// resolves for installs alone.
///
/// Created by `events()` on a generated type, a [`Dynamic`](crate::Dynamic)
/// or a [`ConfigCell`](crate::ConfigCell). The push half of the status
/// surface: a supervisor that wants to react to refused reloads no longer
/// polls `status()` on a timer.
///
/// Latest-wins within each kind, and a refusal is never collapsed into a
/// success — a refusal raced by an install yields both, refusal first,
/// mirroring the Python binding's `events()` ordering.
pub struct Events<T: Send + Sync + 'static> {
    cell: CellRef<T>,
    seen_notify: u64,
    seen_generation: u64,
    seen_refusals: u64,
}

impl<T: Send + Sync + 'static> Events<T> {
    pub(crate) fn new(cell: &'static crate::ConfigCell<T>) -> Self {
        Self {
            seen_notify: cell.notify().generation(),
            seen_generation: cell.generation(),
            seen_refusals: cell.refusals(),
            cell: CellRef::Static(cell),
        }
    }

    /// An `Events` over an instance's shared cell; what
    /// [`Dynamic::events`](crate::Dynamic::events) hands out.
    pub(crate) fn new_shared(cell: std::sync::Arc<crate::ConfigCell<T>>) -> Self {
        Self {
            seen_notify: cell.notify().generation(),
            seen_generation: cell.generation(),
            seen_refusals: cell.refusals(),
            cell: CellRef::Shared(cell),
        }
    }

    /// Resolves with the next event.
    pub fn next_event(&mut self) -> impl Future<Output = Event<T>> + '_ {
        NextEvent { events: self }
    }

    /// What has not been delivered yet, refusals first.
    ///
    /// Reading this ADVANCES the corresponding seen-counter, so one bump
    /// that carried both a refusal and an install yields two events across
    /// two polls — the drain-then-park structure in `NextEvent::poll` is
    /// what keeps the second one from waiting for another wake.
    fn pending(&mut self) -> Option<Event<T>> {
        let cell = self.cell.get();

        let refusals = cell.refusals();

        if refusals != self.seen_refusals {
            self.seen_refusals = refusals;

            if let Some(status) = cell.status().last_failure {
                return Some(Event::Refused(status));
            }
        }

        let generation = cell.generation();

        if generation != self.seen_generation {
            self.seen_generation = generation;

            // `meta` is stored before the wake that got us here, so a
            // loaded snapshot always has one; a `None` pair means the
            // install is still mid-flight and the next poll will see it.
            if let (Some(current), Some(meta)) = (cell.load(), cell.meta()) {
                let status = cell.status();

                return Some(Event::Reloaded {
                    current,
                    meta,
                    reason: status.last_reason,
                });
            }
        }

        None
    }
}

impl<T: Send + Sync + 'static> std::fmt::Debug for Events<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Events")
            .field("seen_generation", &self.seen_generation)
            .field("seen_refusals", &self.seen_refusals)
            .finish_non_exhaustive()
    }
}

struct NextEvent<'a, T: Send + Sync + 'static> {
    events: &'a mut Events<T>,
}

impl<T: Send + Sync + 'static> Future for NextEvent<'_, T> {
    type Output = Event<T>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Event<T>> {
        let events = &mut self.get_mut().events;

        // Drain before parking: one notify bump may cover a refusal AND an
        // install (poll_with reads the counter once), and whichever was not
        // yielded on the first poll must not wait for another wake.
        if let Some(event) = events.pending() {
            return Poll::Ready(event);
        }

        let Events {
            cell,
            seen_notify,
            seen_generation,
            seen_refusals,
        } = events;
        let cell = cell.get();

        cell.notify().poll_with(seen_notify, context.waker(), || {
            // Inlined `pending()`: the closure cannot borrow `events`
            // whole while `seen_notify` is borrowed by `poll_with`.
            let refusals = cell.refusals();

            if refusals != *seen_refusals {
                *seen_refusals = refusals;

                if let Some(status) = cell.status().last_failure {
                    return Some(Event::Refused(status));
                }
            }

            let generation = cell.generation();

            if generation != *seen_generation {
                *seen_generation = generation;

                if let (Some(current), Some(meta)) = (cell.load(), cell.meta()) {
                    let status = cell.status();

                    return Some(Event::Reloaded {
                        current,
                        meta,
                        reason: status.last_reason,
                    });
                }
            }

            None
        })
    }
}

// ---------------------------------------------------------------------------
// Running blocking work
// ---------------------------------------------------------------------------

/// Somewhere to run blocking work from an async context.
///
/// Implement this to hand the crate your runtime's blocking pool. Without one
/// it spawns a thread per call, which is correct everywhere and cheap enough
/// for work that happens at startup and on reload.
pub trait BlockingExecutor: Send + Sync + 'static {
    /// Runs `work` somewhere it is allowed to block.
    fn execute(&self, work: Box<dyn FnOnce() + Send + 'static>);
}

static EXECUTOR: OnceLock<Box<dyn BlockingExecutor>> = OnceLock::new();

/// Installs the blocking executor, once per process.
///
/// ```
/// use dynamic_config::BlockingExecutor;
///
/// struct Threads;
///
/// impl BlockingExecutor for Threads {
///     fn execute(&self, work: Box<dyn FnOnce() + Send + 'static>) {
///         // `async_std::task::spawn_blocking(work)` and `smol::unblock`
///         // slot in here just as well.
///         std::thread::spawn(work);
///     }
/// }
///
/// // Once per process; a second call reports that one is already installed.
/// let _ = dynamic_config::set_blocking_executor(Threads);
/// ```
///
/// # Errors
///
/// If one is already installed. The rejected executor is returned rather than
/// dropped, so a caller that wants to can tell "already set" from "failed".
pub fn set_blocking_executor(
    executor: impl BlockingExecutor,
) -> Result<(), Box<dyn BlockingExecutor>> {
    // `OnceLock::set` wants the error type to be `Debug`; a trait object is not,
    // and requiring `Debug` of every executor to satisfy a `Result` would be the
    // tail wagging the dog.
    match EXECUTOR.set(Box::new(executor)) {
        Ok(()) => Ok(()),
        Err(rejected) => Err(rejected),
    }
}

/// Hands `work` to wherever blocking work belongs.
fn dispatch(work: Box<dyn FnOnce() + Send + 'static>) {
    if let Some(executor) = EXECUTOR.get() {
        executor.execute(work);

        return;
    }

    // A pool beats a fresh thread, and a tokio user has one already — but the
    // `tokio` *feature* does not prove there is a tokio *runtime*: a program
    // that enables it and then drives `load_async` from smol would panic
    // inside `spawn_blocking`. Checked, not assumed.
    #[cfg(feature = "tokio")]
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn_blocking(work);

        return;
    }

    // Correct on every runtime. A configuration load is rare enough that the
    // thread is not the expensive part. If even the thread cannot be spawned,
    // `work` is dropped — and dropping it is what runs the `Guard` inside,
    // which wakes the waiter with `ErrorKind::Backend` rather than leaving it
    // pending for the life of the process. No panic on any path.
    if let Err(error) = std::thread::Builder::new()
        .name("dynamic-config-load".to_owned())
        .spawn(work)
    {
        crate::log::warning!("could not spawn a thread to load configuration: {error}");
    }
}

/// Runs blocking configuration work without blocking the caller's executor.
///
/// # Errors
///
/// Whatever `work` returns, plus [`ErrorKind::Backend`] if it never produced a
/// result — a panic inside it, or a runtime shutting down underneath.
pub async fn off_thread<T, F>(work: F) -> Result<T, Error>
where
    F: FnOnce() -> Result<T, Error> + Send + 'static,
    T: Send + 'static,
{
    let slot = Arc::new(Slot::<Result<T, Error>>::default());

    // The guard is *captured*, not created inside the closure: a closure that
    // is dropped without ever running — a thread that could not be spawned, a
    // pool shutting down underneath — never executes its body, so a guard
    // built there would never exist. A captured guard is dropped with the
    // closure, and its drop is what wakes the waiter.
    let guard = Guard {
        slot: Some(Arc::clone(&slot)),
    };

    dispatch(Box::new(move || {
        let mut guard = guard;

        // A panic here drops `guard` during unwinding, which fills the slot
        // with the Backend error instead of leaving the waiter pending for
        // the life of the process.
        let outcome = work();

        guard.disarm().fill(outcome);
    }));

    Awaiting { slot }.await
}

/// A place for one value, and the task waiting for it.
struct Slot<T> {
    value: Mutex<Option<T>>,
    waker: Mutex<Option<Waker>>,
}

impl<T> Default for Slot<T> {
    fn default() -> Self {
        Self {
            value: Mutex::new(None),
            waker: Mutex::new(None),
        }
    }
}

impl<T> Slot<T> {
    fn fill(&self, value: T) {
        *self
            .value
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(value);

        let waker = self
            .waker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();

        if let Some(waker) = waker {
            waker.wake();
        }
    }

    fn take(&self) -> Option<T> {
        self.value
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
}

/// Fills the slot with a failure if the work never got that far.
///
/// `Option` rather than a flag: disarming *takes* the slot, so the drop path
/// cannot fill after a successful hand-off even by mistake.
struct Guard<T> {
    slot: Option<Arc<Slot<Result<T, Error>>>>,
}

impl<T> Guard<T> {
    /// The work finished; the slot is the caller's to fill with the result.
    fn disarm(&mut self) -> Arc<Slot<Result<T, Error>>> {
        self.slot
            .take()
            .expect("a guard is disarmed at most once, right before filling")
    }
}

impl<T> Drop for Guard<T> {
    fn drop(&mut self) {
        if let Some(slot) = self.slot.take() {
            slot.fill(Err(Error::new(
                ErrorKind::Backend,
                "the configuration load did not finish; the task panicked or was cancelled",
            )));
        }
    }
}

struct Awaiting<T> {
    slot: Arc<Slot<T>>,
}

impl<T> Future for Awaiting<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<T> {
        if let Some(value) = self.slot.take() {
            return Poll::Ready(value);
        }

        *self
            .slot
            .waker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(context.waker().clone());

        // Checked again after registering, for the same reason as `Changed`.
        match self.slot.take() {
            Some(value) => Poll::Ready(value),
            None => Poll::Pending,
        }
    }
}

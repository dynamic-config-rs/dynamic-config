//! Process-wide storage for one configuration snapshot.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use arc_swap::{ArcSwap, ArcSwapOption};

use crate::error::Error;
use crate::reload::{ConfigStatus, FailureStatus, ReloadEvent, ReloadReason};

/// A callback run after a reload, with the outgoing and incoming snapshots.
type Hook<T> = Arc<dyn Fn(&Arc<T>, &Arc<T>) + Send + Sync>;

/// A callback run after every install, with the whole event.
type EventHook<T> = Arc<dyn Fn(&ReloadEvent<T>) + Send + Sync>;

/// [`ConfigCell::on_reload_failed`]: the failure's category and key path —
/// deliberately the [`FailureStatus`] and nothing free-text, for the same
/// reason the status surface carries no message.
type FailedHook = Arc<dyn Fn(&crate::reload::FailureStatus) + Send + Sync>;

/// The two hook shapes, in one list.
///
/// One list rather than two, so there is one dispatch loop and the two
/// forms cannot drift on panic isolation, ordering or what counts as an
/// install. The forms differ in exactly one observable way, and it is a
/// property of their *signatures*: the pair form has nowhere to put "there
/// was no previous snapshot", so it does not fire for the first install.
enum Callback<T> {
    /// [`ConfigCell::on_reload`]: `(previous, current)`, reloads only.
    Pair(Hook<T>),
    /// [`ConfigCell::on_reload_with`]: the whole event, first install
    /// included.
    Event(EventHook<T>),
    /// [`ConfigCell::on_reload_failed`]: fires when a reload installs
    /// nothing. Same list as the success forms, so panic isolation and
    /// registration order cannot drift between the two directions.
    Failed(FailedHook),
}

impl<T> Clone for Callback<T> {
    fn clone(&self) -> Self {
        match self {
            Self::Pair(hook) => Self::Pair(Arc::clone(hook)),
            Self::Event(hook) => Self::Event(Arc::clone(hook)),
            Self::Failed(hook) => Self::Failed(Arc::clone(hook)),
        }
    }
}

/// What is true of the snapshot currently installed.
///
/// The operator's questions — *which generation is live, how stale is it* —
/// answered without the program having to record anything itself. Read it
/// through [`ConfigCell::meta`] or [`Dynamic::meta`](crate::Dynamic::meta).
/// (Re-exported at the crate root as `dynamic_config::SnapshotMeta`.)
///
/// **For operators, not for correctness.** Metadata deliberately does not
/// live on the read path: [`load`](ConfigCell::load) is one atomic load and
/// stays that way, so the value and its metadata are two loads and a reload
/// landing between them leaves the pair one install apart. Code that needs
/// the value and its generation to agree should carry a generation *inside*
/// the configuration type.
///
/// [`Instant`] rather than a wall clock, because the question this answers
/// is "how long ago", which is a duration — and a wall clock can go
/// backwards under NTP while a staleness check must not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct SnapshotMeta {
    /// Installs since the process started. Monotonic; zero before the first.
    pub generation: u64,
    /// When this snapshot was installed.
    pub loaded_at: Instant,
}

/// One registered hook: the callback plus the token that identifies it for
/// removal. Permanent hooks get a token too — it is cheaper than two list
/// types, and nothing ever asks to remove them.
struct Registered<T> {
    token: u64,
    callback: Callback<T>,
}

impl<T> Clone for Registered<T> {
    fn clone(&self) -> Self {
        Self {
            token: self.token,
            callback: self.callback.clone(),
        }
    }
}

/// Holds the current configuration snapshot for one type.
///
/// `ConfigCell::new()` is `const`, so this lives in a `static` — which is how
/// `#[dynamic_config]` emits it.
///
/// Reads are lock-free. [`load`](Self::load) clones an `Arc` out of an
/// [`ArcSwap`], so a reload never blocks a request handler and a reader that
/// already holds an `Arc` keeps observing its own generation until it drops it.
/// Call it once per unit of work: calling it twice within one request can
/// straddle a reload and observe two different configurations.
///
/// # Example
///
/// ```
/// use dynamic_config::ConfigCell;
///
/// static PORT: ConfigCell<u16> = ConfigCell::new();
///
/// assert!(PORT.load().is_none());
///
/// PORT.store(8080);
/// assert_eq!(*PORT.load().unwrap(), 8080);
/// ```
pub struct ConfigCell<T> {
    inner: OnceLock<ArcSwap<T>>,

    /// Held as a snapshot rather than behind a lock, so dispatching a reload
    /// takes no lock a callback could deadlock against by storing again.
    hooks: OnceLock<ArcSwap<Vec<Registered<T>>>>,

    /// Hands out hook tokens. Plain counter: 2^64 registrations outlives the
    /// process by some margin.
    next_token: std::sync::atomic::AtomicU64,

    /// The generation and install time of what `inner` holds, in a slot of
    /// its own so that reading configuration stays one atomic load with
    /// nothing to project out of it. Written *after* the value swap, and
    /// allocated by the same compare-and-swap that publishes it, so the
    /// number an observer sees never goes backwards even when two stores
    /// overlap.
    meta: ArcSwapOption<SnapshotMeta>,

    /// Why the installed snapshot was installed. Beside `meta` rather than
    /// inside it: a reason owns a `PathBuf`, and `SnapshotMeta` is `Copy`
    /// precisely so reading it allocates nothing.
    last_reason: ArcSwapOption<ReloadReason>,

    /// The last reload that installed nothing, and how many have failed
    /// since one did. Outside the snapshot because a failed reload has no
    /// snapshot to hang off — that is what makes it a failure.
    last_failure: ArcSwapOption<FailureStatus>,
    consecutive_failures: AtomicU32,

    /// Reloads that installed nothing since the process started. Monotonic
    /// where `consecutive_failures` resets: the events stream needs a
    /// counter a success cannot erase, or a refusal raced by an install
    /// would never be delivered.
    refusals: std::sync::atomic::AtomicU64,

    /// Generation counter and parked wakers, so async tasks can await a reload
    /// instead of polling. No runtime involved: it is an atomic and a list.
    #[cfg(feature = "async")]
    notify: crate::asynchronous::Notify,
}

impl<T> ConfigCell<T> {
    /// An empty cell.
    #[must_use]
    #[cfg(not(loom))]
    pub const fn new() -> Self {
        Self {
            inner: OnceLock::new(),
            hooks: OnceLock::new(),
            next_token: std::sync::atomic::AtomicU64::new(0),
            meta: ArcSwapOption::const_empty(),
            last_reason: ArcSwapOption::const_empty(),
            last_failure: ArcSwapOption::const_empty(),
            consecutive_failures: AtomicU32::new(0),
            refusals: std::sync::atomic::AtomicU64::new(0),
            #[cfg(feature = "async")]
            notify: crate::asynchronous::Notify::new(),
        }
    }

    /// The same, minus `const`: loom's constructors are not.
    #[must_use]
    #[cfg(loom)]
    pub fn new() -> Self {
        Self {
            inner: OnceLock::new(),
            hooks: OnceLock::new(),
            next_token: std::sync::atomic::AtomicU64::new(0),
            meta: ArcSwapOption::const_empty(),
            last_reason: ArcSwapOption::const_empty(),
            last_failure: ArcSwapOption::const_empty(),
            consecutive_failures: AtomicU32::new(0),
            refusals: std::sync::atomic::AtomicU64::new(0),
            #[cfg(feature = "async")]
            notify: crate::asynchronous::Notify::new(),
        }
    }

    /// Atomically installs `value` as the current snapshot.
    ///
    /// Reload callbacks run, and with the `async` feature every waiting task is
    /// woken. Installing the *first* snapshot is not a reload, so
    /// [`on_reload`](Self::on_reload) callbacks do not fire for it — there is
    /// nothing to compare against. [`on_reload_with`](Self::on_reload_with)
    /// does fire, with `previous: None`, which is the difference between
    /// having somewhere to say that and not.
    ///
    /// The install is recorded as [`ReloadReason::Manual`]: something in the
    /// program stored it. Use [`store_with`](Self::store_with) where more is
    /// known.
    pub fn store(&self, value: T) {
        self.store_with(value, ReloadReason::Manual);
    }

    /// [`store`](Self::store), stating why — and handing back what it
    /// installed.
    ///
    /// The reason travels from the call site that knows it — the watcher
    /// knows the file, `init` knows it is the first — to the hooks and to
    /// [`status`](Self::status). Nothing downstream can reconstruct it: by
    /// the time a hook runs, every install is the same swap.
    ///
    /// The returned `Arc` is **this call's** snapshot, not whatever is
    /// current when it returns: a reload landing a moment later would make
    /// a following [`load`](Self::load) answer differently, and the caller
    /// that installed a configuration means the one it installed. It costs
    /// nothing — the `Arc` was allocated here anyway — and ignoring it is
    /// the ordinary case.
    // Not a `#[must_use]`: every call site in the crate before this one
    // discarded it, and a warning on `cell.store_with(v, reason);` would say
    // "you forgot something" about the normal way to use it.
    #[allow(clippy::must_use_candidate)]
    pub fn store_with(&self, value: T, reason: ReloadReason) -> Arc<T> {
        let value = Arc::new(value);

        // `get_or_init` settles the race between two threads installing the
        // very first snapshot: one initializer wins, and the `swap` below
        // applies this call's value either way.
        let slot = self.inner.get_or_init(|| ArcSwap::new(Arc::clone(&value)));
        let previous = slot.swap(Arc::clone(&value));

        // After the swap, so `meta()` never describes an install a reader
        // cannot see yet — the pair can lag, never lead. The generation is
        // allocated inside the compare-and-swap rather than from a counter
        // read beforehand: two overlapping stores would otherwise be free to
        // publish their numbers in the opposite order, and a generation that
        // goes backwards is worse than one that is merely coarse.
        //
        // The winning closure call is the last one, so what it leaves in
        // `installed` is this install's own metadata rather than a
        // neighbour's — which a `load()` afterwards could not promise.
        let mut installed = None;

        self.meta.rcu(|before| {
            let meta = Arc::new(SnapshotMeta {
                generation: before.as_ref().map_or(0, |meta| meta.generation) + 1,
                loaded_at: Instant::now(),
            });

            installed = Some(*meta);

            meta
        });

        let meta = installed.expect("`rcu` runs its closure at least once");

        // Published after the metadata for the same reason the metadata is
        // published after the value: a reader crossing the gap must see a
        // reason that is stale, never one for an install it cannot see.
        self.last_reason.store(Some(Arc::new(reason.clone())));

        // An install is the only kind of success there is — a load that
        // installs nothing is not a reload — so this is where the failure
        // streak ends. `last_failure` stays: it is history, and the counter
        // is the health.
        self.consecutive_failures.store(0, Ordering::Relaxed);

        // If `get_or_init` just installed *our* value, `previous` is the very
        // same `Arc`, and this is the first install: pair-form callbacks do
        // not fire. Two `store`s racing on a cold cell can still both
        // dispatch — the loser's swap sees the winner's value as "previous"
        // — which is the same thing a reload arriving moments after init
        // would do, so callbacks must tolerate it anyway.
        // Waiters are woken *before* the hooks run: a task awaiting
        // `changes()` wants the new snapshot, which is already installed, and
        // making it wait out every hook would hand one slow callback the power
        // to delay every async reader.
        #[cfg(feature = "async")]
        self.notify.bump();

        let previous = if Arc::ptr_eq(&previous, &value) {
            None
        } else {
            Some(previous)
        };

        // Entered across the dispatch, so anything a reload hook logs is
        // attributed to the reload that ran it. Nothing at all without the
        // feature — not even a stderr line, which every install is far too
        // many of.
        #[cfg(feature = "tracing")]
        let _span = crate::telemetry::installed::<T>(&reason, meta.generation);

        // By reference, so returning the snapshot below costs no refcount
        // traffic: `dispatch` clones only after it knows a hook is there to
        // receive an event, which is what it did before this returned
        // anything.
        self.dispatch(previous, &value, reason, meta);

        value
    }

    /// Records a reload that installed nothing.
    ///
    /// Called by whatever decided not to install — a load that failed, a
    /// validation that refused — so that [`status`](Self::status) can answer
    /// *did the last attempt work* and *how many have failed since one did*.
    /// The next successful install resets the streak.
    ///
    /// Only the failure's category and key path are kept; see
    /// [`FailureStatus`].
    pub fn record_failure(&self, error: &Error) {
        // Saturating rather than wrapping: a counter that rolls over to zero
        // reads as "healthy" at the worst possible moment. Four billion
        // consecutive failures is already the alert.
        //
        // Spelled as a compare-exchange loop rather than `fetch_update`,
        // which nightly has deprecated in favour of `try_update` — a name
        // that does not exist at this crate's 1.71 floor. The loop is what
        // `fetch_update` does, and it compiles everywhere.
        let mut count = self.consecutive_failures.load(Ordering::Relaxed);

        while count < u32::MAX {
            match self.consecutive_failures.compare_exchange_weak(
                count,
                count + 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => count = actual,
            }
        }

        let status = FailureStatus::of(error);

        self.last_failure.store(Some(Arc::new(status.clone())));

        // `Release`, paired with the events stream's `Acquire` load: a
        // reader woken by the bump below must see the status stored above.
        self.refusals
            .fetch_add(1, std::sync::atomic::Ordering::Release);

        // Waiters first, hooks second — the same order an install uses, for
        // the same reason: one slow callback must not delay every async
        // reader. `changes()` filters on the *published* generation, which a
        // refusal does not move, so success waiters see at most a spurious
        // re-registration; the events stream is who this wake is for.
        #[cfg(feature = "async")]
        self.notify.bump();

        self.dispatch_failure(&status);

        // The category and the key path, never the value: see `telemetry`.
        #[cfg(feature = "tracing")]
        crate::telemetry::refused::<T>(error);
    }

    /// Refusals since the process started; zero before the first.
    ///
    /// Monotonic — the streak that resets on success is
    /// [`status().consecutive_failures`](Self::status).
    #[must_use]
    pub fn refusals(&self) -> u64 {
        self.refusals.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Runs every [`on_reload_failed`](Self::on_reload_failed) callback.
    ///
    /// The same panic isolation as [`dispatch`](Self::dispatch): a hook that
    /// panics is reported and skipped, the rest still run, the calling
    /// thread survives.
    fn dispatch_failure(&self, status: &FailureStatus) {
        let Some(hooks) = self.hooks.get() else {
            return;
        };

        let hooks = hooks.load();

        for registered in hooks.iter() {
            let Callback::Failed(hook) = &registered.callback else {
                continue;
            };

            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                hook(status);
            }));

            if outcome.is_err() {
                crate::log::warning!(
                    "a reload-failure hook panicked; it stays registered and \
                     the remaining hooks still run"
                );
            }
        }
    }

    /// What is true of this configuration right now.
    ///
    /// A handful of atomic loads and no I/O — nothing is re-read, nothing
    /// is recomputed — so an exporter can call it per scrape. See
    /// [`ConfigStatus`] for what it deliberately does not carry.
    #[must_use]
    pub fn status(&self) -> ConfigStatus {
        let meta = self.meta();

        ConfigStatus {
            generation: meta.map_or(0, |meta| meta.generation),
            loaded_at: meta.map(|meta| meta.loaded_at),
            last_reason: self.last_reason.load_full().map(|reason| (*reason).clone()),
            last_failure: self
                .last_failure
                .load_full()
                .map(|failure| (*failure).clone()),
            consecutive_failures: self.consecutive_failures.load(Ordering::Relaxed),
        }
    }

    /// Registers a callback for every later reload.
    ///
    /// The callback receives the outgoing and incoming snapshots, in that
    /// order, and runs on whichever thread performed the reload — the watcher
    /// thread, usually. Keep it short, and do not store again from inside one:
    /// that recurses rather than deadlocking, which is worse.
    ///
    /// Callbacks registered this way cannot be removed — a hook for the life
    /// of the process, which is what a server wants. Anything with a shorter
    /// life — a test, a plugin, a subsystem that can be torn down — should
    /// use [`on_reload_scoped`](Self::on_reload_scoped) and hold the guard.
    ///
    /// A hook that panics is caught, reported, and skipped for that reload;
    /// the remaining hooks still run and the watcher thread survives. It is
    /// not unregistered — a bug in a hook should be loud on every reload, not
    /// once.
    ///
    /// # Concurrent reloads
    ///
    /// Each call sees a consistent `(previous, current)` pair: both were
    /// installed, and `current` was installed after `previous`.
    ///
    /// The *order of calls* is not defined when two reloads overlap. Two
    /// hooks may observe the same pair, and one hook may see `(A, B)` after
    /// another saw `(B, C)`. A hook that needs a total order should read
    /// [`generation`](Self::generation) — which is monotonic — rather than
    /// infer one from its arguments.
    ///
    /// Reloads are deliberately not serialised against each other. The same
    /// store that dispatches these hooks wakes async waiters *before* running
    /// them, so that one slow callback cannot delay every reader; a lock held
    /// across user callbacks would undo that on purpose, and a hook that
    /// blocked would then block reloads.
    pub fn on_reload(&self, hook: impl Fn(&Arc<T>, &Arc<T>) + Send + Sync + 'static) {
        let _ = self.register(Callback::Pair(Arc::new(hook)));
    }

    /// [`on_reload`](Self::on_reload), told *why*.
    ///
    /// The callback receives a [`ReloadEvent`]: both snapshots, the
    /// [`ReloadReason`], and the [`SnapshotMeta`] of the install. Everything
    /// the pair form promises holds here — same list, same order of
    /// registration, same panic isolation, same absence of an order across
    /// overlapping reloads — with one difference the pair form's signature
    /// makes impossible: **this fires for the first install too**, with
    /// `previous: None`. A hook that only wants reloads matches on that, or
    /// registers through [`on_reload`](Self::on_reload).
    ///
    /// ```
    /// use dynamic_config::{ConfigCell, ReloadReason};
    ///
    /// static PORT: ConfigCell<u16> = ConfigCell::new();
    ///
    /// PORT.on_reload_with(|event| {
    ///     if let ReloadReason::FileChanged(path) = &event.reason {
    ///         println!("generation {} came from {}", event.meta.generation, path.display());
    ///     }
    /// });
    /// ```
    pub fn on_reload_with(&self, hook: impl Fn(&ReloadEvent<T>) + Send + Sync + 'static) {
        let _ = self.register(Callback::Event(Arc::new(hook)));
    }

    /// Registers a callback for every reload that installs nothing.
    ///
    /// The callback receives the [`FailureStatus`] — the category, the key
    /// path, never the message — and runs on whichever thread performed the
    /// failed reload, under the same contract as
    /// [`on_reload`](Self::on_reload): keep it short, panics are caught and
    /// reported, registration is permanent. The next successful install
    /// does not unregister it.
    ///
    /// This is the push half of [`status`](Self::status): a process that
    /// wants to *react* to refusals — page, count, flip a readiness detail —
    /// no longer has to poll for them.
    pub fn on_reload_failed(
        &self,
        hook: impl Fn(&crate::reload::FailureStatus) + Send + Sync + 'static,
    ) {
        let _ = self.register(Callback::Failed(Arc::new(hook)));
    }

    /// [`on_reload`](Self::on_reload), scoped: dropping the returned guard
    /// unregisters the hook.
    ///
    /// For anything whose life is shorter than the process — the permanent
    /// variant would keep a torn-down subsystem's callback firing forever.
    ///
    /// The same concurrency contract as [`on_reload`](Self::on_reload): a
    /// consistent pair every call, in no defined order across overlapping
    /// reloads.
    #[must_use = "dropping the guard unregisters the hook; bind it for as long \
                  as the hook should fire, or use `on_reload` for a permanent one"]
    pub fn on_reload_scoped(
        &'static self,
        hook: impl Fn(&Arc<T>, &Arc<T>) + Send + Sync + 'static,
    ) -> HookGuard<T> {
        HookGuard {
            token: self.register(Callback::Pair(Arc::new(hook))),
            cell: GuardCell::Static(self),
        }
    }

    /// [`on_reload_with`](Self::on_reload_with), scoped: dropping the
    /// returned guard unregisters the hook.
    #[must_use = "dropping the guard unregisters the hook; bind it for as long \
                  as the hook should fire, or use `on_reload_with` for a \
                  permanent one"]
    pub fn on_reload_with_scoped(
        &'static self,
        hook: impl Fn(&ReloadEvent<T>) + Send + Sync + 'static,
    ) -> HookGuard<T> {
        HookGuard {
            token: self.register(Callback::Event(Arc::new(hook))),
            cell: GuardCell::Static(self),
        }
    }

    /// [`on_reload_failed`](Self::on_reload_failed), scoped: dropping the
    /// returned guard unregisters the hook. The same panic isolation and
    /// "short callbacks" contract.
    #[must_use = "dropping the guard unregisters the hook; bind it for as long \
                  as the hook should fire, or use `on_reload_failed` for a \
                  permanent one"]
    pub fn on_reload_failed_scoped(
        &'static self,
        hook: impl Fn(&crate::reload::FailureStatus) + Send + Sync + 'static,
    ) -> HookGuard<T> {
        HookGuard {
            token: self.register(Callback::Failed(Arc::new(hook))),
            cell: GuardCell::Static(self),
        }
    }

    /// The scoped hook over an instance's shared cell; what
    /// [`Dynamic::on_reload_scoped`](crate::Dynamic::on_reload_scoped)
    /// hands out — the guard co-owns the cell, so it outliving the
    /// `Dynamic` is safe rather than subtle.
    pub(crate) fn on_reload_scoped_shared(
        cell: &Arc<Self>,
        hook: impl Fn(&Arc<T>, &Arc<T>) + Send + Sync + 'static,
    ) -> HookGuard<T> {
        HookGuard {
            token: cell.register(Callback::Pair(Arc::new(hook))),
            cell: GuardCell::Shared(Arc::clone(cell)),
        }
    }

    /// [`on_reload_scoped_shared`](Self::on_reload_scoped_shared), event
    /// form; what [`Dynamic::on_reload_with_scoped`] hands out.
    ///
    /// [`Dynamic::on_reload_with_scoped`]: crate::Dynamic::on_reload_with_scoped
    pub(crate) fn on_reload_with_scoped_shared(
        cell: &Arc<Self>,
        hook: impl Fn(&ReloadEvent<T>) + Send + Sync + 'static,
    ) -> HookGuard<T> {
        HookGuard {
            token: cell.register(Callback::Event(Arc::new(hook))),
            cell: GuardCell::Shared(Arc::clone(cell)),
        }
    }

    /// The scoped failure hook over an instance's shared cell; what
    /// `Dynamic::on_reload_failed_scoped` hands out.
    pub(crate) fn on_reload_failed_scoped_shared(
        cell: &Arc<Self>,
        hook: impl Fn(&crate::reload::FailureStatus) + Send + Sync + 'static,
    ) -> HookGuard<T> {
        HookGuard {
            token: cell.register(Callback::Failed(Arc::new(hook))),
            cell: GuardCell::Shared(Arc::clone(cell)),
        }
    }

    fn register(&self, callback: Callback<T>) -> u64 {
        let token = self
            .next_token
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        self.hooks
            .get_or_init(|| ArcSwap::from_pointee(Vec::new()))
            .rcu(|current| {
                let mut next = Vec::with_capacity(current.len() + 1);

                next.extend(current.iter().cloned());
                next.push(Registered {
                    token,
                    callback: callback.clone(),
                });

                next
            });

        token
    }

    fn unregister(&self, token: u64) {
        let Some(hooks) = self.hooks.get() else {
            return;
        };

        hooks.rcu(|current| {
            current
                .iter()
                .filter(|registered| registered.token != token)
                .cloned()
                .collect::<Vec<_>>()
        });
    }

    /// Runs every registered callback for one install.
    ///
    /// `previous` is `None` when this install is the first — see
    /// [`store_with`](Self::store_with).
    fn dispatch(
        &self,
        previous: Option<Arc<T>>,
        current: &Arc<T>,
        reason: ReloadReason,
        meta: SnapshotMeta,
    ) {
        let Some(hooks) = self.hooks.get() else {
            return;
        };

        // A snapshot of the list, so a callback that registers another one does
        // not invalidate the iteration.
        let hooks = hooks.load();

        if hooks.is_empty() {
            return;
        }

        // Built once, after the list is known to be non-empty: an event
        // clones two `Arc`s and a reason's `PathBuf`, and a cell with no
        // hooks — the common case — must not pay for that on every install.
        let event = ReloadEvent::new(previous, Arc::clone(current), reason, meta);

        for registered in hooks.iter() {
            // Caught per hook: a panic in one must neither silence the rest
            // nor unwind into the watcher thread and kill it — a watcher that
            // died with a live-looking handle is the failure mode this exists
            // to prevent. `AssertUnwindSafe` is honest here: the hook gets
            // shared references it cannot leave half-mutated.
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                match &registered.callback {
                    // The pair form has nowhere to put "there was none", so
                    // it does not fire for the first install; that contract
                    // predates the event form and does not move for it.
                    Callback::Pair(hook) => {
                        if let Some(previous) = &event.previous {
                            hook(previous, &event.current);
                        }
                    }
                    Callback::Event(hook) => hook(&event),
                    // Failure hooks have their own dispatch; an install is
                    // exactly what they do not fire for.
                    Callback::Failed(_) => {}
                }
            }));

            if outcome.is_err() {
                crate::log::warning!(
                    "a reload hook panicked; it stays registered and the \
                     remaining hooks still run"
                );
            }
        }
    }

    /// The current snapshot, or `None` if nothing has been stored yet.
    pub fn load(&self) -> Option<Arc<T>> {
        self.inner.get().map(ArcSwap::load_full)
    }

    /// Installs since the process started; zero before the first.
    ///
    /// Monotonic, so it is the number a reload hook should read when it
    /// needs a total order — [`on_reload`](Self::on_reload) does not define
    /// one across overlapping reloads.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.meta.load().as_ref().map_or(0, |meta| meta.generation)
    }

    /// What is true of the installed snapshot, or `None` before the first.
    ///
    /// A load of its own: [`load`](Self::load) is untouched by this and
    /// stays one atomic load, which means the value and its metadata can be
    /// one install apart. See `SnapshotMeta`.
    #[must_use]
    pub fn meta(&self) -> Option<SnapshotMeta> {
        self.meta.load().as_deref().copied()
    }

    /// The current snapshot, panicking if there is none.
    ///
    /// `type_name` is used to build the message; the generated code passes the
    /// annotated struct's name so the panic names the type the caller wrote.
    ///
    /// # Panics
    ///
    /// If nothing has been stored yet.
    pub fn get_or_panic(&self, type_name: &str) -> Arc<T> {
        self.load().unwrap_or_else(|| {
            panic!(
                "{type_name} has no snapshot installed; configure and install \
                 one first: `{type_name}::builder(\"..\")...init()?`"
            )
        })
    }

    /// A handle woken by every later [`store`](Self::store).
    ///
    /// The snapshot current at this call counts as already seen, so the first
    /// `changed()` waits for the *next* store. Read the value you start from
    /// with [`load`](Self::load).
    ///
    /// Runtime-agnostic: it is a `Future`, and any executor drives it.
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    pub fn changes(&'static self) -> crate::Changes<T>
    where
        T: Send + Sync,
    {
        crate::Changes::new(self)
    }

    /// A stream of installs **and refusals**, where [`changes`](Self::changes)
    /// delivers installs alone.
    ///
    /// Each await resolves with the next [`Event`](crate::Event): a
    /// [`Reloaded`](crate::Event::Reloaded) for an install, a
    /// [`Refused`](crate::Event::Refused) for a reload that kept the
    /// previous snapshot. Latest-wins within each kind — ten installs while
    /// nothing awaited collapse to one event carrying the newest snapshot —
    /// but a refusal is never collapsed *into* a success: a refusal raced by
    /// an install yields both, refusal first.
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    pub fn events(&'static self) -> crate::Events<T>
    where
        T: Send + Sync,
    {
        crate::Events::new(self)
    }

    #[cfg(feature = "async")]
    pub(crate) fn notify(&self) -> &crate::asynchronous::Notify {
        &self.notify
    }
}

/// Unregisters its hook when dropped. From
/// [`on_reload_scoped`](ConfigCell::on_reload_scoped).
///
/// `#[must_use]` on the *type* rather than only on the methods that hand one
/// out: a guard is the whole registration, and every producer — the cell's
/// four, [`Dynamic`](crate::Dynamic)'s two, the generated two — has the same
/// silent failure when the result is dropped at the end of the statement.
/// Marking the type is the one place that covers a producer nobody has
/// written yet.
#[must_use = "dropping the guard unregisters the hook immediately; bind it for \
              as long as the hook should fire, or register a permanent hook \
              with `on_reload`"]
pub struct HookGuard<T: 'static> {
    cell: GuardCell<T>,
    token: u64,
}

/// The cell a guard unregisters from: a type's `static`, or an instance's
/// own — the same two shapes `Changes` distinguishes, for the same reason.
enum GuardCell<T: 'static> {
    Static(&'static ConfigCell<T>),
    Shared(Arc<ConfigCell<T>>),
}

impl<T> Drop for HookGuard<T> {
    fn drop(&mut self) {
        match &self.cell {
            GuardCell::Static(cell) => cell.unregister(self.token),
            GuardCell::Shared(cell) => cell.unregister(self.token),
        }
    }
}

impl<T> std::fmt::Debug for HookGuard<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HookGuard")
            .field("token", &self.token)
            .finish_non_exhaustive()
    }
}

impl<T> Default for ConfigCell<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for ConfigCell<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.load() {
            Some(value) => f.debug_tuple("ConfigCell").field(&value).finish(),
            None => f.write_str("ConfigCell(uninitialized)"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::thread;

    #[test]
    fn a_fresh_cell_is_empty() {
        let cell = ConfigCell::<u16>::new();

        assert!(cell.load().is_none());
    }

    #[test]
    fn a_reader_keeps_the_generation_it_took() {
        let cell = ConfigCell::new();
        cell.store(String::from("first"));

        let held = cell.load().unwrap();
        cell.store(String::from("second"));

        assert_eq!(*held, "first");
        assert_eq!(*cell.load().unwrap(), "second");
    }

    #[test]
    fn concurrent_first_writes_do_not_lose_the_cell() {
        let cell: &'static ConfigCell<usize> = Box::leak(Box::new(ConfigCell::new()));

        let writers: Vec<_> = (0..8)
            .map(|value| thread::spawn(move || cell.store(value)))
            .collect();

        for writer in writers {
            writer.join().unwrap();
        }

        let final_value = *cell.load().expect("some writer must have won");
        assert!(final_value < 8);
    }

    #[test]
    fn the_first_store_is_an_initialization_not_a_reload() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let cell = ConfigCell::new();

        let recorder = Arc::clone(&seen);
        cell.on_reload(move |previous, current| {
            recorder.lock().unwrap().push((**previous, **current));
        });

        cell.store(1u16);
        assert!(
            seen.lock().unwrap().is_empty(),
            "there is nothing to compare the first snapshot against"
        );

        cell.store(2u16);
        cell.store(3u16);

        assert_eq!(*seen.lock().unwrap(), [(1, 2), (2, 3)]);
    }

    #[test]
    fn every_registered_callback_runs() {
        let count = Arc::new(Mutex::new(0usize));
        let cell = ConfigCell::new();

        for _ in 0..3 {
            let counter = Arc::clone(&count);
            cell.on_reload(move |_, _| *counter.lock().unwrap() += 1);
        }

        cell.store(1u16);
        cell.store(2u16);

        assert_eq!(*count.lock().unwrap(), 3);
    }

    #[test]
    fn a_panicking_hook_silences_neither_the_rest_nor_the_next_reload() {
        let count = Arc::new(Mutex::new(0usize));
        let cell = ConfigCell::new();

        cell.on_reload(|_, _| panic!("a bug in somebody's hook"));
        {
            let counter = Arc::clone(&count);
            cell.on_reload(move |_, _| *counter.lock().unwrap() += 1);
        }

        cell.store(1u16);
        cell.store(2u16);
        cell.store(3u16);

        assert_eq!(
            *count.lock().unwrap(),
            2,
            "the hook after the panicking one must run on every reload"
        );
    }

    #[test]
    fn dropping_the_guard_unregisters_the_hook() {
        let count = Arc::new(Mutex::new(0usize));
        let cell: &'static ConfigCell<u16> = Box::leak(Box::new(ConfigCell::new()));

        cell.store(1);

        let guard = {
            let counter = Arc::clone(&count);
            cell.on_reload_scoped(move |_, _| *counter.lock().unwrap() += 1)
        };

        cell.store(2);
        assert_eq!(*count.lock().unwrap(), 1);

        drop(guard);
        cell.store(3);
        assert_eq!(
            *count.lock().unwrap(),
            1,
            "an unregistered hook must not fire"
        );
    }

    /// What `store_with` hands back is the snapshot it installed, and it
    /// stays that one: a later store moves `load()` and not the `Arc` an
    /// earlier caller is holding. This is what makes `init_and_current`
    /// answer for its own install rather than for whichever reload won a
    /// race with it.
    #[test]
    fn store_with_hands_back_the_snapshot_it_installed() {
        let cell = ConfigCell::new();

        let first = cell.store_with(1u16, ReloadReason::Initial);
        assert!(Arc::ptr_eq(&first, &cell.load().unwrap()));

        let second = cell.store_with(2u16, ReloadReason::Manual);

        assert_eq!(*first, 1, "the earlier install's snapshot is unmoved");
        assert_eq!(*second, 2);
        assert!(Arc::ptr_eq(&second, &cell.load().unwrap()));
    }

    #[test]
    #[should_panic(expected = "`DbConfig::builder(")]
    fn get_or_panic_points_at_the_builder() {
        ConfigCell::<u16>::new().get_or_panic("DbConfig");
    }

    /// The difference between the two forms, in one test: the pair form has
    /// nowhere to say "there was none", so it stays silent for the first
    /// install; the event form says it with `previous: None`.
    #[test]
    fn the_event_form_sees_the_first_install_and_the_pair_form_does_not() {
        let pairs = Arc::new(Mutex::new(0usize));
        let events = Arc::new(Mutex::new(Vec::new()));
        let cell = ConfigCell::new();

        {
            let counter = Arc::clone(&pairs);
            cell.on_reload(move |_, _| *counter.lock().unwrap() += 1);
        }
        {
            let recorder = Arc::clone(&events);
            cell.on_reload_with(move |event| {
                recorder
                    .lock()
                    .unwrap()
                    .push((event.previous.as_deref().copied(), *event.current));
            });
        }

        cell.store(1u16);
        assert_eq!(*pairs.lock().unwrap(), 0);
        assert_eq!(*events.lock().unwrap(), [(None, 1)]);

        cell.store(2u16);
        assert_eq!(*pairs.lock().unwrap(), 1);
        assert_eq!(*events.lock().unwrap(), [(None, 1), (Some(1), 2)]);
    }

    /// The event carries the install it belongs to, not whatever the cell
    /// happens to hold by the time a hook reads it.
    #[test]
    fn an_event_carries_the_reason_and_the_generation_of_its_own_install() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let cell = ConfigCell::new();

        {
            let recorder = Arc::clone(&seen);
            cell.on_reload_with(move |event| {
                recorder
                    .lock()
                    .unwrap()
                    .push((event.reason.clone(), event.meta.generation));
            });
        }

        cell.store_with(1u16, ReloadReason::Initial);
        cell.store_with(2u16, ReloadReason::RemoteChanged);
        cell.store(3u16);

        assert_eq!(
            *seen.lock().unwrap(),
            [
                (ReloadReason::Initial, 1),
                (ReloadReason::RemoteChanged, 2),
                (ReloadReason::Manual, 3),
            ]
        );
    }

    /// A panicking event hook is isolated exactly like a panicking pair
    /// hook — one list, one dispatch loop, one `catch_unwind`.
    #[test]
    fn a_panicking_event_hook_leaves_the_rest_running() {
        let count = Arc::new(Mutex::new(0usize));
        let cell = ConfigCell::new();

        cell.on_reload_with(|_| panic!("a bug in somebody's hook"));
        {
            let counter = Arc::clone(&count);
            cell.on_reload_with(move |_| *counter.lock().unwrap() += 1);
        }

        cell.store(1u16);
        cell.store(2u16);

        assert_eq!(*count.lock().unwrap(), 2);
    }

    #[test]
    fn a_scoped_event_hook_stops_when_its_guard_drops() {
        let count = Arc::new(Mutex::new(0usize));
        let cell: &'static ConfigCell<u16> = Box::leak(Box::new(ConfigCell::new()));

        let guard = {
            let counter = Arc::clone(&count);
            cell.on_reload_with_scoped(move |_| *counter.lock().unwrap() += 1)
        };

        cell.store(1);
        cell.store(2);
        assert_eq!(*count.lock().unwrap(), 2, "the first install counts too");

        drop(guard);
        cell.store(3);
        assert_eq!(*count.lock().unwrap(), 2);
    }

    /// Failures accumulate, an install clears the streak, and the record of
    /// the last one survives it — the counter is the health, the record is
    /// the history.
    #[test]
    fn failures_count_up_and_an_install_resets_the_streak() {
        let cell = ConfigCell::<u16>::new();

        assert_eq!(cell.status().consecutive_failures, 0);
        assert!(cell.status().is_healthy());
        assert!(cell.status().last_failure.is_none());
        assert!(cell.status().last_reason.is_none());

        for expected in 1..=3 {
            cell.record_failure(&Error::new(
                crate::ErrorKind::Parse,
                "unexpected end of input",
            ));
            assert_eq!(cell.status().consecutive_failures, expected);
        }

        assert!(!cell.status().is_healthy());
        assert_eq!(
            cell.status().last_failure.unwrap().kind,
            crate::ErrorKind::Parse
        );

        cell.store_with(1, ReloadReason::Recovered);

        let status = cell.status();
        assert_eq!(status.consecutive_failures, 0);
        assert!(status.is_healthy());
        assert_eq!(status.generation, 1);
        assert_eq!(status.last_reason, Some(ReloadReason::Recovered));
        assert!(
            status.last_failure.is_some(),
            "the streak resets; the record of what went wrong does not"
        );
        assert!(status.loaded_at.is_some());
    }
}

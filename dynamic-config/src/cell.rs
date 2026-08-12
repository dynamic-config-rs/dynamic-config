//! Process-wide storage for one configuration snapshot.

use std::sync::{Arc, OnceLock};

use arc_swap::ArcSwap;

/// A callback run after a reload, with the outgoing and incoming snapshots.
type Hook<T> = Arc<dyn Fn(&Arc<T>, &Arc<T>) + Send + Sync>;

/// One registered hook: the callback plus the token that identifies it for
/// removal. Permanent hooks get a token too — it is cheaper than two list
/// types, and nothing ever asks to remove them.
struct Registered<T> {
    token: u64,
    hook: Hook<T>,
}

impl<T> Clone for Registered<T> {
    fn clone(&self) -> Self {
        Self {
            token: self.token,
            hook: Arc::clone(&self.hook),
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
            #[cfg(feature = "async")]
            notify: crate::asynchronous::Notify::new(),
        }
    }

    /// Atomically installs `value` as the current snapshot.
    ///
    /// Reload callbacks run, and with the `async` feature every waiting task is
    /// woken. Installing the *first* snapshot is not a reload, so callbacks do
    /// not fire for it — there is nothing to compare against.
    pub fn store(&self, value: T) {
        let value = Arc::new(value);

        // `get_or_init` settles the race between two threads installing the
        // very first snapshot: one initializer wins, and the `swap` below
        // applies this call's value either way.
        let slot = self.inner.get_or_init(|| ArcSwap::new(Arc::clone(&value)));
        let previous = slot.swap(Arc::clone(&value));

        // If `get_or_init` just installed *our* value, `previous` is the very
        // same `Arc`, and no callbacks fire. Two `store`s racing on a cold
        // cell can still both dispatch — the loser's swap sees the winner's
        // value as "previous" — which is the same thing a reload arriving
        // moments after init would do, so callbacks must tolerate it anyway.
        // Waiters are woken *before* the hooks run: a task awaiting
        // `changes()` wants the new snapshot, which is already installed, and
        // making it wait out every hook would hand one slow callback the power
        // to delay every async reader.
        #[cfg(feature = "async")]
        self.notify.bump();

        if !Arc::ptr_eq(&previous, &value) {
            self.dispatch(&previous, &value);
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
    pub fn on_reload(&self, hook: impl Fn(&Arc<T>, &Arc<T>) + Send + Sync + 'static) {
        let _ = self.register(Arc::new(hook));
    }

    /// [`on_reload`](Self::on_reload), scoped: dropping the returned guard
    /// unregisters the hook.
    ///
    /// For anything whose life is shorter than the process — the permanent
    /// variant would keep a torn-down subsystem's callback firing forever.
    #[must_use = "dropping the guard unregisters the hook; bind it for as long \
                  as the hook should fire, or use `on_reload` for a permanent one"]
    pub fn on_reload_scoped(
        &'static self,
        hook: impl Fn(&Arc<T>, &Arc<T>) + Send + Sync + 'static,
    ) -> HookGuard<T> {
        HookGuard {
            token: self.register(Arc::new(hook)),
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
            token: cell.register(Arc::new(hook)),
            cell: GuardCell::Shared(Arc::clone(cell)),
        }
    }

    fn register(&self, hook: Hook<T>) -> u64 {
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
                    hook: Arc::clone(&hook),
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

    fn dispatch(&self, previous: &Arc<T>, current: &Arc<T>) {
        let Some(hooks) = self.hooks.get() else {
            return;
        };

        // A snapshot of the list, so a callback that registers another one does
        // not invalidate the iteration.
        for registered in hooks.load().iter() {
            // Caught per hook: a panic in one must neither silence the rest
            // nor unwind into the watcher thread and kill it — a watcher that
            // died with a live-looking handle is the failure mode this exists
            // to prevent. `AssertUnwindSafe` is honest here: the hook gets
            // shared references it cannot leave half-mutated.
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                (registered.hook)(previous, current);
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

    #[cfg(feature = "async")]
    pub(crate) fn notify(&self) -> &crate::asynchronous::Notify {
        &self.notify
    }
}

/// Unregisters its hook when dropped. From
/// [`on_reload_scoped`](ConfigCell::on_reload_scoped).
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

    #[test]
    #[should_panic(expected = "`DbConfig::builder(")]
    fn get_or_panic_points_at_the_builder() {
        ConfigCell::<u16>::new().get_or_panic("DbConfig");
    }
}

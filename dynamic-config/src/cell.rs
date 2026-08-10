//! Process-wide storage for one configuration snapshot.

use std::sync::{Arc, OnceLock};

use arc_swap::ArcSwap;

/// A callback run after a reload, with the outgoing and incoming snapshots.
type Hook<T> = Arc<dyn Fn(&Arc<T>, &Arc<T>) + Send + Sync>;

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
    hooks: OnceLock<ArcSwap<Vec<Hook<T>>>>,

    /// Generation counter and parked wakers, so async tasks can await a reload
    /// instead of polling. No runtime involved: it is an atomic and a list.
    #[cfg(feature = "async")]
    notify: crate::asynchronous::Notify,
}

impl<T> ConfigCell<T> {
    /// An empty cell.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            inner: OnceLock::new(),
            hooks: OnceLock::new(),
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
        if !Arc::ptr_eq(&previous, &value) {
            self.dispatch(&previous, &value);
        }

        #[cfg(feature = "async")]
        self.notify.bump();
    }

    /// Registers a callback for every later reload.
    ///
    /// The callback receives the outgoing and incoming snapshots, in that
    /// order, and runs on whichever thread performed the reload — the watcher
    /// thread, usually. Keep it short, and do not store again from inside one:
    /// that recurses rather than deadlocking, which is worse.
    ///
    /// Callbacks cannot be removed. A reload hook that should stop firing
    /// should check a flag of its own; the alternative is handing out
    /// registration tokens nobody would remember to drop.
    pub fn on_reload(&self, hook: impl Fn(&Arc<T>, &Arc<T>) + Send + Sync + 'static) {
        let hook: Hook<T> = Arc::new(hook);

        self.hooks
            .get_or_init(|| ArcSwap::from_pointee(Vec::new()))
            .rcu(|current| {
                let mut next = Vec::with_capacity(current.len() + 1);

                next.extend(current.iter().map(Arc::clone));
                next.push(Arc::clone(&hook));

                next
            });
    }

    fn dispatch(&self, previous: &Arc<T>, current: &Arc<T>) {
        let Some(hooks) = self.hooks.get() else {
            return;
        };

        // A snapshot of the list, so a callback that registers another one does
        // not invalidate the iteration.
        for hook in hooks.load().iter() {
            hook(previous, current);
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
            panic!("{type_name} has not been initialized; call `{type_name}::init()` first")
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
    #[should_panic(expected = "`DbConfig::init()`")]
    fn get_or_panic_points_at_init() {
        ConfigCell::<u16>::new().get_or_panic("DbConfig");
    }
}

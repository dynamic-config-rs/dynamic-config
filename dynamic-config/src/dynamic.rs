//! Instance-owned configuration: the engine without the `static`.
//!
//! `#[dynamic_config]` gives a *type* one configuration, stored in statics
//! the macro generates. That is the right default and the wrong ceiling:
//! multi-tenant programs want one configuration per tenant, tests want two
//! side by side without type gymnastics, and a host language binding has no
//! Rust type per user class at all. [`Dynamic<T>`] is the same engine with
//! the storage owned by the value: its own cell, its own hooks, its own
//! watcher identity — nothing shared with the type-level surface, and
//! nothing global.
//!
//! ```no_run
//! # #[cfg(feature = "json")] {
//! use dynamic_config::{Builder, Dynamic};
//! use serde::Deserialize;
//!
//! #[derive(Debug, Deserialize)]
//! struct Tenant { name: String }
//!
//! let acme = Dynamic::new(Builder::new("tenant").file("acme.json"));
//! let umbra = Dynamic::new(Builder::new("tenant").file("umbra.json"));
//!
//! acme.init()?;
//! umbra.init()?;
//!
//! let a: std::sync::Arc<Tenant> = acme.current().expect("initialised above");
//! let u: std::sync::Arc<Tenant> = umbra.current().expect("initialised above");
//! # let _ = (a, u);
//! # }
//! # Ok::<(), dynamic_config::Error>(())
//! ```

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde::de::DeserializeOwned;

use crate::builder::Builder;
use crate::cell::ConfigCell;
use crate::error::Error;

/// One process-unique number per instance, for the watcher registry.
///
/// A type's watcher is keyed by `TypeId`; every `Dynamic<Value>` is the
/// same type, so an instance carries a number instead. Starts at one so
/// zero never names anything — the same "never ambiguous with nothing"
/// convention the reload generation follows.
static NEXT_INSTANCE: AtomicU64 = AtomicU64::new(1);

/// A configuration owned by a value rather than a type.
///
/// Construct one from a [`Builder`] carrying the sources; everything the
/// type-level surface does through generated statics happens here through
/// the instance's own storage. Two instances of the same `T` are fully
/// independent: separate snapshots, separate reload hooks, separate
/// watchers, separate caches if configured.
///
/// Cloning is deliberately absent: a `Dynamic` is an *owner* — share one
/// behind an `Arc` when several places read it, which is also what keeps
/// "who stops the watcher" a question with one answer.
pub struct Dynamic<T> {
    cell: Arc<ConfigCell<T>>,
    builder: Builder<T>,
    id: u64,
    /// The registry wants a `&'static str`; leaked once per instance, on
    /// the first watch, and reused for every stop/start cycle after it.
    #[cfg(feature = "watch")]
    watch_name: std::sync::OnceLock<&'static str>,
}

impl<T: DeserializeOwned + Send + Sync + 'static> Dynamic<T> {
    /// Wraps `builder` around storage this instance owns.
    ///
    /// The builder's sources, cache and validation hook all apply
    /// unchanged; an installer the builder already carried (a generated
    /// `builder()`'s static cell) is replaced by this instance's own.
    #[must_use]
    pub fn new(builder: Builder<T>) -> Self {
        let cell = Arc::new(ConfigCell::new());

        Self {
            builder: builder.with_cell(Arc::clone(&cell)),
            cell,
            id: NEXT_INSTANCE.fetch_add(1, Ordering::Relaxed),
            #[cfg(feature = "watch")]
            watch_name: std::sync::OnceLock::new(),
        }
    }

    /// Loads and installs as this instance's snapshot.
    ///
    /// The same lifecycle as a type's `init()`: validation runs before
    /// anything installs, a configured cache is written after a clean
    /// load and recovered from when the sources will not load.
    ///
    /// # Errors
    ///
    /// Whatever the load reports: a file that will not parse, a missing
    /// required value, a validation refusal with no cache to fall back on.
    pub fn init(&self) -> Result<(), Error> {
        self.builder.init()
    }

    /// The installed snapshot, if [`init`](Self::init) has succeeded.
    ///
    /// One atomic load, no lock — cheap enough per request, but take it
    /// once per request and reuse the `Arc`, or a reload landing
    /// mid-request shows one request two configurations. `None` before the
    /// first successful install: an instance has no place to panic with
    /// the type's name in it, so absence is an answer rather than an
    /// accident.
    #[must_use]
    pub fn current(&self) -> Option<Arc<T>> {
        self.cell.load()
    }

    /// Reads the sources and deserializes, installing nothing.
    ///
    /// # Errors
    ///
    /// The same failures as [`init`](Self::init).
    pub fn load(&self) -> Result<T, Error> {
        self.builder.load()
    }

    /// One reload: load, validate, install, rewrite the cache.
    ///
    /// A failure installs nothing — the previous snapshot keeps serving.
    ///
    /// # Errors
    ///
    /// The same failures as [`load`](Self::load).
    pub fn reload(&self) -> Result<(), Error> {
        self.builder.reload()
    }

    /// Runs `hook` after every later install, for the instance's lifetime.
    ///
    /// The same contract as the type-level `on_reload`: called with the
    /// outgoing and incoming snapshots, on whichever thread performed the
    /// reload — compare, then signal the subsystem that owns the resource.
    pub fn on_reload(&self, hook: impl Fn(&Arc<T>, &Arc<T>) + Send + Sync + 'static) {
        self.cell.on_reload(hook);
    }

    /// [`on_reload`](Self::on_reload), until the returned guard drops.
    pub fn on_reload_scoped(
        &self,
        hook: impl Fn(&Arc<T>, &Arc<T>) + Send + Sync + 'static,
    ) -> crate::HookGuard<T> {
        ConfigCell::on_reload_scoped_shared(&self.cell, hook)
    }

    /// This instance's builder, for the diagnostics that answer without
    /// installing: `source_of`, `is_set`, `check`, `explain`, `snapshot`.
    ///
    /// The instance does not re-wrap them — the builder's answers *are*
    /// the instance's answers, because the builder is where its sources
    /// live.
    #[must_use]
    pub fn builder(&self) -> &Builder<T> {
        &self.builder
    }

    /// The section key this instance reads.
    #[must_use]
    pub fn key(&self) -> &str {
        self.builder.key()
    }
}

#[cfg(feature = "watch")]
#[cfg_attr(docsrs, doc(cfg(feature = "watch")))]
impl<T: DeserializeOwned + Send + Sync + 'static> Dynamic<T> {
    /// Reloads on file changes until the returned handle is dropped.
    ///
    /// The same watcher as everything else — same debounce, same
    /// directory-level watches — registered under this *instance* rather
    /// than the type: two instances of one `T` watch side by side, and a
    /// second watch on the *same* instance is `AlreadyExists`, exactly the
    /// one-watcher-per-owner contract the type-level surface has.
    ///
    /// # Errors
    ///
    /// As the builder's `watch`: no watchable directory, a backend that
    /// cannot start, or this instance already being watched.
    pub fn watch(
        &self,
        debounce: core::time::Duration,
    ) -> std::io::Result<crate::watch::WatchHandle> {
        self.watch_with(debounce, crate::watch::WatchMode::Native)
    }

    /// [`watch`](Self::watch) with the detection strategy chosen
    /// explicitly — polling is what network and overlay filesystems need.
    ///
    /// # Errors
    ///
    /// As [`watch`](Self::watch).
    pub fn watch_with(
        &self,
        debounce: core::time::Duration,
        mode: crate::watch::WatchMode,
    ) -> std::io::Result<crate::watch::WatchHandle> {
        let name = self.watch_name.get_or_init(|| {
            Box::leak(format!("dynamic:{}#{}", self.builder.key(), self.id).into_boxed_str())
        });

        self.builder.watch_as(
            crate::watch::WatchKey::Instance(self.id),
            name,
            debounce,
            mode,
        )
    }
}

#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
impl<T: DeserializeOwned + Send + Sync + 'static> Dynamic<T> {
    /// A handle woken by every later install of *this instance*.
    ///
    /// The same contract as the type-level `changes()`: the snapshot
    /// current at this call counts as already seen, and a handle taken
    /// before [`init`](Self::init) sees the first install as its first
    /// change — "wake me when configuration exists". The handle keeps the
    /// instance's storage alive, so it outliving the `Dynamic` is safe
    /// rather than subtle.
    #[must_use]
    pub fn changes(&self) -> crate::Changes<T> {
        crate::Changes::new_shared(Arc::clone(&self.cell))
    }

    /// [`load`](Self::load), off the async executor.
    ///
    /// # Errors
    ///
    /// The same failures as [`load`](Self::load).
    pub async fn load_async(&self) -> Result<T, Error> {
        self.builder.load_async().await
    }

    /// [`init`](Self::init), off the async executor.
    ///
    /// # Errors
    ///
    /// The same failures as [`init`](Self::init).
    pub async fn init_async(&self) -> Result<(), Error> {
        self.builder.init_async().await
    }
}

impl<T> std::fmt::Debug for Dynamic<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dynamic")
            .field("id", &self.id)
            .field("builder", &self.builder)
            .finish_non_exhaustive()
    }
}

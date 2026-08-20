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
//! let a: std::sync::Arc<Tenant> = acme.init_and_current()?;
//! let u: std::sync::Arc<Tenant> = umbra.init_and_current()?;
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

    /// [`init`](Self::init), handing back the snapshot it installed.
    ///
    /// Worth more here than on the type-level surface: an instance's
    /// [`current`](Self::current) is an `Option` — nothing can panic with a
    /// type's name in it — so the split form ends in an `expect` that this
    /// removes. What comes back is *this* call's snapshot, not whatever a
    /// reload made current a moment later.
    ///
    /// # Errors
    ///
    /// Exactly [`init`](Self::init)'s.
    pub fn init_and_current(&self) -> Result<Arc<T>, Error> {
        self.builder.init_and_current()
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

    /// Installs since this instance was created; zero before the first.
    ///
    /// Monotonic, and the number a reload hook should read when it needs a
    /// total order — [`on_reload`](Self::on_reload) does not define one
    /// across overlapping reloads.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.cell.generation()
    }

    /// What is true of the installed snapshot, or `None` before the first.
    ///
    /// For operators — which generation is live, how long ago it landed —
    /// and deliberately off the read path: [`current`](Self::current) does
    /// not consult it, so the value and its metadata are two loads that a
    /// reload landing between them leaves one install apart. See
    /// `SnapshotMeta`.
    #[must_use]
    pub fn meta(&self) -> Option<crate::SnapshotMeta> {
        self.cell.meta()
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
    /// Reloads are not serialised against each other on purpose: a lock held
    /// across user callbacks would let one slow hook delay every reader, and
    /// a hook that blocked would then block reloads.
    pub fn on_reload(&self, hook: impl Fn(&Arc<T>, &Arc<T>) + Send + Sync + 'static) {
        self.cell.on_reload(hook);
    }

    /// [`on_reload`](Self::on_reload), until the returned guard drops.
    ///
    /// The same concurrency contract: a consistent pair every call, in no
    /// defined order across overlapping reloads.
    pub fn on_reload_scoped(
        &self,
        hook: impl Fn(&Arc<T>, &Arc<T>) + Send + Sync + 'static,
    ) -> crate::HookGuard<T> {
        ConfigCell::on_reload_scoped_shared(&self.cell, hook)
    }

    /// Registers a callback for every reload that installs nothing — the
    /// failure twin of [`on_reload`](Self::on_reload), for the process
    /// lifetime, under the same contract: short callbacks, panics caught,
    /// the watcher survives. The callback receives the
    /// [`FailureStatus`](crate::FailureStatus) the refusal published.
    pub fn on_reload_failed(&self, hook: impl Fn(&crate::FailureStatus) + Send + Sync + 'static) {
        self.cell.on_reload_failed(hook);
    }

    /// [`on_reload_failed`](Self::on_reload_failed), until the returned
    /// guard drops.
    pub fn on_reload_failed_scoped(
        &self,
        hook: impl Fn(&crate::FailureStatus) + Send + Sync + 'static,
    ) -> crate::HookGuard<T> {
        crate::ConfigCell::on_reload_failed_scoped_shared(&self.cell, hook)
    }

    /// [`on_reload`](Self::on_reload), told *why*.
    ///
    /// The callback receives a [`ReloadEvent`](crate::ReloadEvent): both
    /// snapshots, the [`ReloadReason`](crate::ReloadReason), and the
    /// install's [`SnapshotMeta`](crate::SnapshotMeta). Same list, same
    /// registration order, same panic isolation as the pair form — and it
    /// fires for the **first** install too, with `previous: None`, which
    /// the pair form has nowhere to say.
    pub fn on_reload_with(&self, hook: impl Fn(&crate::ReloadEvent<T>) + Send + Sync + 'static) {
        self.cell.on_reload_with(hook);
    }

    /// [`on_reload_with`](Self::on_reload_with), until the returned guard
    /// drops.
    pub fn on_reload_with_scoped(
        &self,
        hook: impl Fn(&crate::ReloadEvent<T>) + Send + Sync + 'static,
    ) -> crate::HookGuard<T> {
        ConfigCell::on_reload_with_scoped_shared(&self.cell, hook)
    }

    /// What is true of this instance right now: generation, when it landed,
    /// why, and how the reloads since have gone.
    ///
    /// A handful of atomic loads and **no I/O** — no source is re-read —
    /// so an exporter can call it per scrape. See
    /// [`ConfigStatus`](crate::ConfigStatus) for what it carries and, as
    /// deliberately, what it does not.
    #[must_use]
    pub fn status(&self) -> crate::ConfigStatus {
        self.cell.status()
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

    /// [`changes`](Self::changes) widened to refusals: a stream of
    /// [`Event`](crate::Event)s — installs *and* reloads that kept the
    /// previous snapshot. The push half of [`status`](Self::status).
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    #[must_use]
    pub fn events(&self) -> crate::Events<T> {
        crate::Events::new_shared(Arc::clone(&self.cell))
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

    /// [`init_and_current`](Self::init_and_current), off the async executor.
    ///
    /// # Errors
    ///
    /// The same failures as [`init`](Self::init).
    pub async fn init_and_current_async(&self) -> Result<Arc<T>, Error> {
        self.builder.init_and_current_async().await
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

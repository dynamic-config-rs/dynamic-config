//! Why a snapshot was installed, and how the installs since have gone.
//!
//! Three questions an operator asks that the crate could always answer and
//! never did: *why did this change*, *did the last attempt work*, and *how
//! many have failed since one did*. [`ReloadReason`] is the first,
//! [`ReloadEvent`] carries it to a hook, and [`ConfigStatus`] is all three
//! in one cheap struct.
//!
//! Nothing here is on the read path. A reason is recorded by the same store
//! that publishes the snapshot, a failure by the same code that decided not
//! to publish one — so reading any of it is a load, never a computation,
//! and `current()` stays the single atomic load it has always been.
//!
//! **Values never appear here.** A reason names a *file*, a failure names a
//! *key path* and an [`ErrorKind`], and a status is counts and timestamps.
//! Every one of these types exists to be printed into a log, which is
//! exactly how a configured value escapes, so none of them can hold one.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use crate::cell::SnapshotMeta;
use crate::error::{Error, ErrorKind};

/// What caused a snapshot to be installed.
///
/// Recorded at the call site that installs, because nothing downstream can
/// reconstruct it: by the time a hook runs, a file change and a manual
/// `reload()` have produced the identical swap.
///
/// `#[non_exhaustive]`: the set of things that can install a configuration
/// grows with the crate, and matching on it must keep compiling when it
/// does.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReloadReason {
    /// A [`Builder::init`](crate::Builder::init) — the call that establishes
    /// a configuration, whether or not one was already installed.
    Initial,
    /// A watched file changed. Carries the path whose event opened the
    /// reload's debounce window.
    ///
    /// One path, not the set: the debounce collapses a flurry into a single
    /// reload, and the window that a `..data` symlink swap or a two-file
    /// edit produces genuinely covers several. The path names *what
    /// triggered this reload*, which is the question a log line asks; the
    /// keys that actually moved are
    /// [`changed_paths`](crate::changed_paths)' job.
    FileChanged(PathBuf),
    /// A remote store pushed a document through
    /// [`RemoteSink::apply`](crate::RemoteSink::apply).
    RemoteChanged,
    /// The program installed it: [`reload`](crate::Builder::reload), a
    /// generated `replace`, [`ConfigCell::store`](crate::ConfigCell::store),
    /// or a [`ReloadGroup`](crate::ReloadGroup) commit.
    Manual,
    /// The last-known-good cache, after the sources refused to load at
    /// [`init`](crate::Builder::init).
    Recovered,
}

impl ReloadReason {
    /// A short, stable label — the category, without the path.
    ///
    /// For a metric dimension or a structured log field, where
    /// [`FileChanged`](Self::FileChanged)'s path is unbounded cardinality
    /// and the category is not.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Initial => "initial",
            Self::FileChanged(_) => "file-changed",
            Self::RemoteChanged => "remote-changed",
            Self::Manual => "manual",
            Self::Recovered => "recovered",
        }
    }
}

/// One install, as an event.
///
/// What [`on_reload_with`](crate::ConfigCell::on_reload_with) hands a hook,
/// and what the two-argument [`on_reload`](crate::ConfigCell::on_reload)
/// cannot say: *why* the snapshot moved, *which* generation it became, and
/// — through `previous` — that there was nothing before it.
///
/// The first install is an event with `previous: None`. The pair form has
/// nowhere to put that and so does not fire for it at all; this form does,
/// which is what makes [`ReloadReason::Initial`] reachable from a hook.
///
/// # A note on `Debug`
///
/// It prints the reason, the metadata and *whether* there was a previous
/// snapshot — never the snapshots themselves, however `T` renders. An event
/// is a diagnostic, a `{:?}` of one lands in a log, and a configuration
/// holds passwords. Reach for the [`current`](Self::current) field when you
/// want the values; that is a deliberate second step.
#[non_exhaustive]
pub struct ReloadEvent<T> {
    /// The snapshot that was serving until now, or `None` when this install
    /// is the first — there was no configuration before it.
    pub previous: Option<Arc<T>>,
    /// The snapshot now serving.
    pub current: Arc<T>,
    /// What caused this install.
    pub reason: ReloadReason,
    /// The generation this install became, and when it landed.
    pub meta: SnapshotMeta,
}

impl<T> ReloadEvent<T> {
    /// Not public API: built by the cell that dispatches it.
    pub(crate) fn new(
        previous: Option<Arc<T>>,
        current: Arc<T>,
        reason: ReloadReason,
        meta: SnapshotMeta,
    ) -> Self {
        Self {
            previous,
            current,
            reason,
            meta,
        }
    }
}

// Hand-written: `#[derive(Clone)]` would demand `T: Clone`, and an `Arc<T>`
// clones whatever `T` is.
impl<T> Clone for ReloadEvent<T> {
    fn clone(&self) -> Self {
        Self {
            previous: self.previous.clone(),
            current: Arc::clone(&self.current),
            reason: self.reason.clone(),
            meta: self.meta,
        }
    }
}

impl<T> std::fmt::Debug for ReloadEvent<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReloadEvent")
            .field("reason", &self.reason)
            .field("generation", &self.meta.generation)
            // Presence, not content: see the type's documentation.
            .field("had_previous", &self.previous.is_some())
            .finish_non_exhaustive()
    }
}

/// A reload that did not install anything.
///
/// The category and the key path, and deliberately not the message: an
/// error's `Display` is value-free by policy and enforced by
/// `tests/security.rs`, but a struct that *stores* free text is one careless
/// `Error::new` away from carrying a value into every log that prints a
/// status. What an operator needs to act — when, what kind, which key — is
/// none of it free text.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FailureStatus {
    /// When the failure was recorded.
    pub at: Instant,
    /// The failure's category.
    pub kind: ErrorKind,
    /// The dotted key path it was reported at; empty when the failure
    /// belongs to the load as a whole rather than to one key.
    pub path: String,
}

impl FailureStatus {
    /// Not public API: built where the failure is recorded.
    pub(crate) fn of(error: &Error) -> Self {
        Self {
            at: Instant::now(),
            kind: error.kind(),
            path: error.path(),
        }
    }
}

/// What is true of a configuration right now, for an operator asking.
///
/// Every field is *recorded* where it happens rather than recomputed here,
/// so building one is a handful of atomic loads: no I/O, no source is
/// re-read, and nothing here can block. That is the constraint — an
/// exporter calling this per scrape must cost nothing.
///
/// It is assembled from several loads, so a reload landing mid-call can
/// leave one field an install ahead of another. The same trade
/// [`SnapshotMeta`] makes, for the same reason: for operators, not for
/// correctness.
///
/// # What it does not carry
///
/// **No values, by construction** — see [`FailureStatus`]. And no *source*
/// list: which sources would be read is a question about the next load, and
/// [`check`](crate::check) already answers it against the sources rather
/// than from a cache of them that could go stale. Nor is there a
/// `last_success`: an install *is* the success, so
/// [`loaded_at`](Self::loaded_at) is when the last one was.
///
/// [`loaded_at`]: Self::loaded_at
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ConfigStatus {
    /// Installs since the process started; zero before the first.
    pub generation: u64,
    /// When the serving snapshot was installed — which is also when the
    /// last successful load was. `None` before the first install.
    pub loaded_at: Option<Instant>,
    /// Why the serving snapshot was installed. `None` before the first.
    pub last_reason: Option<ReloadReason>,
    /// The most recent reload that installed nothing, if there has been
    /// one. Kept after a later success: it is history, and
    /// [`consecutive_failures`](Self::consecutive_failures) is the health.
    pub last_failure: Option<FailureStatus>,
    /// Failures since the last install. **Zero means healthy.**
    pub consecutive_failures: u32,
}

impl ConfigStatus {
    /// Whether the last attempt to load installed something.
    ///
    /// Nothing more than `consecutive_failures == 0`, spelled the way the
    /// question is asked. True before the first load too — nothing has
    /// failed yet.
    #[must_use]
    pub fn is_healthy(&self) -> bool {
        self.consecutive_failures == 0
    }

    /// How long ago the serving snapshot was installed.
    ///
    /// `None` before the first install. This is why the recorded instant is
    /// monotonic: a wall clock going backwards under NTP would make a fresh
    /// configuration look stale.
    #[must_use]
    pub fn stale_for(&self) -> Option<std::time::Duration> {
        self.loaded_at.map(|at| at.elapsed())
    }
}

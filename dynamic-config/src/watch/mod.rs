//! The filesystem watcher behind hot reload.
//!
//! One concern per file: this module holds what a watcher is pointed at
//! ([`Watched`]) and how it detects changes ([`WatchMode`]); `handle.rs`
//! starts and stops one — the one-watcher-per-type registry, the spawn,
//! the [`WatchHandle`] that owns the backend; `debounce.rs` is the
//! background loop that waits out an editor's flurry before reloading;
//! `relevance.rs` decides which events are about our files at all,
//! Kubernetes `..data` swaps included.

mod debounce;
mod handle;
mod relevance;
#[cfg(test)]
mod tests;

pub use debounce::set_atomic_save_grace;
pub use handle::{spawn, spawn_with, WatchHandle, WatchKey};

use std::path::PathBuf;
use std::time::Duration;

use crate::discovery;
use crate::source::LoadSpec;

/// What a watcher looks at, owned.
///
/// The watch used to borrow a `LoadSpec<'static>`, which chained every
/// watcher to statics only the attribute can produce. Owning the three
/// facts the watch actually uses — the explicit file paths, the discovery
/// name, the searched directories — frees the builder (or anything else)
/// to start one from runtime data.
#[derive(Debug, Clone)]
pub struct Watched {
    files: Vec<PathBuf>,
    search_name: Option<String>,
    search_directories: Vec<PathBuf>,
}

impl Watched {
    /// Captures what a watcher needs from `spec`, with any lifetime.
    ///
    /// The searched directories are resolved here, once — the same moment
    /// the directory watches are registered, so the two cannot disagree.
    #[must_use]
    pub fn from_spec(spec: &LoadSpec<'_>) -> Self {
        Self {
            files: spec
                .sources
                .iter()
                .filter_map(|source| source.path())
                .map(PathBuf::from)
                .collect(),
            search_name: spec.search.as_ref().map(|search| search.name.to_owned()),
            search_directories: spec
                .search
                .as_ref()
                .map(|search| discovery::search_directories(search))
                .unwrap_or_default(),
        }
    }
}

/// How to detect changes.
///
/// The native backend is right almost everywhere and wrong in one important
/// place: inotify and its equivalents do not fire on many network and overlay
/// filesystems — NFS, some Docker bind mounts, some CI runners. The failure is
/// silent, because the watch registers successfully and simply never delivers
/// anything, so there is nothing to detect and fall back from. It has to be
/// chosen deliberately.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum WatchMode {
    /// The platform's notification backend. Efficient, and the default.
    #[default]
    Native,
    /// Re-read the files on an interval. Works anywhere, at the cost of the
    /// interval's worth of latency and a periodic wake-up.
    ///
    /// Each tick compares **contents**, not only timestamps. A filesystem
    /// timestamp is compared here in whole seconds, so an edit landing in the
    /// same second as the previous scan would otherwise be invisible — and
    /// stay invisible, because the next scan compares against the value it
    /// just recorded. Configuration files are small and few, and a watcher
    /// that misses edits is the failure polling was chosen to escape.
    Poll {
        /// How often to look.
        interval: Duration,
    },
}

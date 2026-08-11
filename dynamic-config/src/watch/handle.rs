//! Starting a watcher, and the handle that stops it.
//!
//! The registry (one watcher per type, keyed by `TypeId`), the spawn that
//! registers *before* returning so no edit slips through the gap, the
//! rollback that frees the name when a spawn fails partway, and the
//! directory-level watches — directories, not files, because editors and
//! atomic saves replace the inode.

use std::any::TypeId;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Mutex};
use std::thread;
use std::time::Duration;

use notify::{Event, RecursiveMode, Watcher};

use crate::error::Error;
use crate::log::warning;

use super::debounce::run;
use super::{WatchMode, Watched};

/// Types that already have a watcher, keyed by [`TypeId`] — the one identity
/// that survives generics. The display name is kept only for messages: keyed
/// by name, `Db<Postgres>` and `Db<Mysql>` both stringify to `"Db"`, and the
/// second `start_watch()` silently watched nothing.
pub(super) static STARTED: Mutex<BTreeMap<TypeId, &'static str>> = Mutex::new(BTreeMap::new());

/// Keeps a watcher alive. Dropping it stops watching.
///
/// The handle owns the notification backend, and the background thread owns
/// only the receiving end. Dropping the handle closes the channel, which is
/// what ends the thread — no flag to poll, no wake-up latency.
///
/// A server usually wants the watcher to outlive everything, which is what
/// [`detach`](Self::detach) is for. Anything with a lifecycle — a test, a
/// library, a subcommand — should hold the handle instead, so watching stops
/// when the thing being configured goes away.
#[must_use = "dropping the handle stops the watcher; bind it, or call `.detach()` \
              to watch for the rest of the process"]
pub struct WatchHandle {
    key: TypeId,
    name: &'static str,
    /// `None` only while `detach` is dismantling the handle.
    watcher: Option<Backend>,
}

/// The two backends, kept as one owner so the handle is a single type.
enum Backend {
    Native(notify::RecommendedWatcher),
    Poll(notify::PollWatcher),
}

impl WatchHandle {
    /// Watches for the remainder of the process.
    ///
    /// Leaks the backend on purpose: a watcher that must never stop has no
    /// owner to hold it, and pretending otherwise is how the handle ends up
    /// dropped at the end of `main`'s first statement.
    pub fn detach(mut self) {
        if let Some(watcher) = self.watcher.take() {
            std::mem::forget(watcher);
        }

        // The registration stays, so a later `spawn` still reports
        // `AlreadyExists` rather than starting a second watcher.
        std::mem::forget(self);
    }

    /// Stops watching. The same as dropping it, spelled out.
    pub fn stop(self) {}

    /// The type name this watcher was started for.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.name
    }
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        // `None` only mid-`detach`, which forgets the handle before this
        // could run — but belt and braces costs one branch.
        let Some(watcher) = self.watcher.take() else {
            return;
        };

        // Dropping the backend closes the channel and ends the thread. Freeing
        // the registration lets a later `spawn` start a fresh one — which is
        // what makes this usable from tests.
        drop(watcher);

        // Recovered from poisoning rather than skipped: skipping would leak
        // the registration forever, and the map has no invariant a panic
        // could break — the same policy every other lock in the crate follows.
        STARTED
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.key);
    }
}

impl std::fmt::Debug for WatchHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WatchHandle")
            .field("name", &self.name)
            // The backend is a notify watcher, which has no rendering worth
            // printing and would drown the one field that matters.
            .finish_non_exhaustive()
    }
}

/// Starts a background thread that runs `reload` whenever one of `files` changes.
///
/// Calling this twice for the same type is an error (`AlreadyExists`): a
/// second handle could only mislead, and the first watcher keeps running.
///
/// `reload` is expected to swap in a new snapshot. Returning `Some(summary)`
/// replaces the generic "reloaded" line with something more specific — which is
/// how `diff` reports the keys that moved without logging twice.
///
/// Its error is reported and discarded — an invalid or half-written file must
/// degrade to "no change", never to a crash, because the previous snapshot is
/// still perfectly good.
///
/// The watch is registered *before* this function returns, so an edit that
/// lands immediately afterwards cannot slip through the gap. Registering it on
/// the background thread instead would leave a window — short, but reliably hit
/// by anything that writes configuration during startup.
///
/// # Errors
///
/// If the notification backend cannot be created, if none of the directories
/// holding `files` can be watched, or if the thread cannot be spawned. A
/// directory that fails while others succeed is reported and skipped.
///
pub fn spawn(
    key: TypeId,
    name: &'static str,
    watched: Watched,
    debounce: Duration,
    reload: impl Fn() -> Result<Option<String>, Error> + Send + 'static,
) -> std::io::Result<WatchHandle> {
    spawn_with(key, name, watched, debounce, WatchMode::default(), reload)
}

/// [`spawn`], with the detection strategy chosen explicitly.
///
/// # Errors
///
/// As [`spawn`].
///
pub fn spawn_with(
    key: TypeId,
    name: &'static str,
    watched: Watched,
    debounce: Duration,
    mode: WatchMode,
    reload: impl Fn() -> Result<Option<String>, Error> + Send + 'static,
) -> std::io::Result<WatchHandle> {
    // An error, not a quiet no-op handle. The old behaviour returned
    // `Ok(handle-that-owns-nothing)`, which read as "I started watching" and
    // was undetectable at runtime — the worst kind of success.
    if STARTED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(key, name)
        .is_some()
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!(
                "`{name}` is already being watched; hold on to the handle the \
                 first `start_watch()` returned, or drop it before starting \
                 another"
            ),
        ));
    }

    // The insertion above is what makes two concurrent `spawn` calls mutually
    // exclusive, so it has to come first — and therefore a failure below has
    // to undo it. Without the rollback, every later `start_watch()` for this
    // type would find the name taken and return a success handle that owns
    // nothing and watches nothing, silently.
    let registered = Registered { key, armed: true };

    let (sender, receiver) = mpsc::channel::<notify::Result<Event>>();

    let mut backend = match mode {
        WatchMode::Native => Backend::Native(notify::recommended_watcher(sender).map_err(to_io)?),
        WatchMode::Poll { interval } => Backend::Poll(
            notify::PollWatcher::new(
                sender,
                notify::Config::default().with_poll_interval(interval),
            )
            .map_err(to_io)?,
        ),
    };

    match &mut backend {
        Backend::Native(watcher) => watch_directories(name, watcher, &watched)?,
        Backend::Poll(watcher) => watch_directories(name, watcher, &watched)?,
    }

    thread::Builder::new()
        .name(format!("config-watch-{name}"))
        .spawn(move || run(name, &watched, debounce, reload, &receiver))?;

    // Everything that could fail has succeeded; from here the *handle* owns
    // the registration and frees it on drop.
    registered.defuse();

    Ok(WatchHandle {
        key,
        name,
        watcher: Some(backend),
    })
}

/// Rolls the name registration back unless the spawn completed.
///
/// Every `?` between the insertion and the end of `spawn_with` — the backend,
/// the directory watches, the thread — runs through this on the way out.
struct Registered {
    key: TypeId,
    armed: bool,
}

impl Registered {
    /// The spawn completed; the registration now belongs to the handle.
    fn defuse(mut self) {
        self.armed = false;
    }
}

impl Drop for Registered {
    fn drop(&mut self) {
        if self.armed {
            STARTED
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&self.key);
        }
    }
}

fn to_io(error: notify::Error) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, error)
}

/// Watches the *directories* holding the files, not the files themselves.
///
/// Editors and `mv`-based atomic saves replace the inode, which silently
/// detaches a file-level watch. Watching the parent directory survives that —
/// and is also what makes a Kubernetes ConfigMap update, delivered as a `..data`
/// symlink swap, visible at all.
///
/// Fails when nothing could be watched, rather than parking a thread on a
/// channel that will never produce an event.
fn watch_directories(
    name: &'static str,
    watcher: &mut impl Watcher,
    watched: &Watched,
) -> std::io::Result<()> {
    let mut directories = Vec::<PathBuf>::new();

    {
        let mut push = |directory: PathBuf| {
            if !directories.contains(&directory) {
                directories.push(directory);
            }
        };

        for file in &watched.files {
            push(
                file.parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new("."))
                    .to_path_buf(),
            );
        }

        // Every searched directory, whether or not it holds a file today: a
        // config file appearing later is exactly the event worth catching.
        for directory in &watched.search_directories {
            push(directory.clone());
        }
    }

    let mut watched = 0usize;
    let mut last_error = None;

    for directory in &directories {
        match watcher.watch(directory, RecursiveMode::NonRecursive) {
            Ok(()) => watched += 1,
            Err(error) => {
                warning!("{name}: could not watch {}: {error}", directory.display());
                last_error = Some(error);
            }
        }
    }

    if watched == 0 {
        return Err(last_error.map_or_else(
            || {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("{name}: no configuration file to watch"),
                )
            },
            to_io,
        ));
    }

    Ok(())
}

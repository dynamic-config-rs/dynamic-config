//! The filesystem watcher behind hot reload.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use notify::{Event, EventKind, RecursiveMode, Watcher};

use crate::discovery;
use crate::error::Error;
use crate::log::{info, warning};
use crate::source::LoadSpec;

/// Pause after the debounce window, before the files are read back.
///
/// An atomic save writes a temporary file and renames it into place. The rename
/// can be observed a hair before the new inode is visible, so a short grace
/// period avoids reading a file that is about to be replaced.
const ATOMIC_SAVE_GRACE: Duration = Duration::from_millis(25);

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
    /// Re-stat the files on an interval. Works anywhere, at the cost of the
    /// interval's worth of latency and a periodic wake-up.
    Poll {
        /// How often to look.
        interval: Duration,
    },
}

/// Names that already have a watcher, so a second `spawn` is a no-op.
static STARTED: Mutex<BTreeSet<&'static str>> = Mutex::new(BTreeSet::new());

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

        // The name stays registered, so a later `spawn` is still a no-op.
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
        // A handle from a duplicate `spawn` owns nothing; freeing the name here
        // would let a third call start a *second* watcher alongside the one
        // still running.
        let Some(watcher) = self.watcher.take() else {
            return;
        };

        // Dropping the backend closes the channel and ends the thread. Freeing
        // the name lets a later `spawn` start a fresh one — which is what makes
        // this usable from tests.
        drop(watcher);

        // Recovered from poisoning rather than skipped: skipping would leak
        // the name forever, and the set has no invariant a panic could break —
        // the same policy every other lock in the crate follows.
        STARTED
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(self.name);
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
/// Calling this twice with the same `name` is a no-op: the second call returns
/// a handle that owns nothing, so dropping it does not stop the first watcher.
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
    name: &'static str,
    spec: LoadSpec<'static>,
    debounce: Duration,
    reload: impl Fn() -> Result<Option<String>, Error> + Send + 'static,
) -> std::io::Result<WatchHandle> {
    spawn_with(name, spec, debounce, WatchMode::default(), reload)
}

/// [`spawn`], with the detection strategy chosen explicitly.
///
/// # Errors
///
/// As [`spawn`].
///
pub fn spawn_with(
    name: &'static str,
    spec: LoadSpec<'static>,
    debounce: Duration,
    mode: WatchMode,
    reload: impl Fn() -> Result<Option<String>, Error> + Send + 'static,
) -> std::io::Result<WatchHandle> {
    if !STARTED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(name)
    {
        return Ok(WatchHandle {
            name,
            watcher: None,
        });
    }

    // The insertion above is what makes two concurrent `spawn` calls mutually
    // exclusive, so it has to come first — and therefore a failure below has
    // to undo it. Without the rollback, every later `start_watch()` for this
    // type would find the name taken and return a success handle that owns
    // nothing and watches nothing, silently.
    let registered = Registered { name, armed: true };

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
        Backend::Native(watcher) => watch_directories(name, watcher, &spec)?,
        Backend::Poll(watcher) => watch_directories(name, watcher, &spec)?,
    }

    thread::Builder::new()
        .name(format!("config-watch-{name}"))
        .spawn(move || run(name, spec, debounce, reload, &receiver))?;

    // Everything that could fail has succeeded; from here the *handle* owns
    // the registration and frees it on drop.
    registered.defuse();

    Ok(WatchHandle {
        name,
        watcher: Some(backend),
    })
}

/// Rolls the name registration back unless the spawn completed.
///
/// Every `?` between the insertion and the end of `spawn_with` — the backend,
/// the directory watches, the thread — runs through this on the way out.
struct Registered {
    name: &'static str,
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
                .remove(self.name);
        }
    }
}

fn to_io(error: notify::Error) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, error)
}

fn run(
    name: &'static str,
    spec: LoadSpec<'static>,
    debounce: Duration,
    reload: impl Fn() -> Result<Option<String>, Error>,
    receiver: &mpsc::Receiver<notify::Result<Event>>,
) {
    loop {
        let Some(batch) = collect_batch(receiver, name, debounce) else {
            // The watcher was dropped, so no further events can arrive.
            return;
        };

        if !touches_configured_file(&batch, &spec) {
            continue;
        }

        thread::sleep(ATOMIC_SAVE_GRACE);

        match reload() {
            Ok(Some(summary)) => info!("{name}: reloaded, {summary}"),
            Ok(None) => info!("{name}: reloaded"),
            Err(error) => warning!("{name}: reload failed, keeping the previous snapshot: {error}"),
        }
    }
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
    spec: &LoadSpec<'static>,
) -> std::io::Result<()> {
    let mut directories = Vec::<PathBuf>::new();

    {
        let mut push = |directory: PathBuf| {
            if !directories.contains(&directory) {
                directories.push(directory);
            }
        };

        for file in spec.sources.iter().filter_map(|source| source.path()) {
            push(
                Path::new(file)
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new("."))
                    .to_path_buf(),
            );
        }

        // Every searched directory, whether or not it holds a file today: a
        // config file appearing later is exactly the event worth catching.
        if let Some(search) = &spec.search {
            for directory in discovery::search_directories(search) {
                push(directory);
            }
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

/// Blocks for the first event, then drains until the stream goes quiet.
///
/// One editor save typically emits several events; reloading once per event
/// would re-read a file mid-write.
///
/// Returns `None` once the channel is disconnected.
fn collect_batch(
    receiver: &mpsc::Receiver<notify::Result<Event>>,
    name: &'static str,
    debounce: Duration,
) -> Option<Vec<Event>> {
    let mut batch = Vec::new();

    loop {
        match receiver.recv() {
            Ok(Ok(event)) => {
                batch.push(event);
                break;
            }
            Ok(Err(error)) => warning!("{name}: watcher error: {error}"),
            Err(mpsc::RecvError) => return None,
        }
    }

    loop {
        match receiver.recv_timeout(debounce) {
            Ok(Ok(event)) => batch.push(event),
            Ok(Err(error)) => warning!("{name}: watcher error: {error}"),
            Err(mpsc::RecvTimeoutError::Timeout) => return Some(batch),
            Err(mpsc::RecvTimeoutError::Disconnected) => return None,
        }
    }
}

/// Whether a batch is about one of our files.
///
/// The whole directory is watched, so most batches are about something else.
/// Paths are compared in both directions because event paths are absolute while
/// configured paths are usually relative to the working directory; a rare false
/// positive costs one redundant reload, which is harmless.
fn touches_configured_file(batch: &[Event], spec: &LoadSpec<'static>) -> bool {
    batch.iter().any(|event| {
        matches!(
            event.kind,
            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
        ) && event.paths.iter().any(|changed| is_ours(changed, spec))
    })
}

fn is_ours(changed: &Path, spec: &LoadSpec<'static>) -> bool {
    let explicit = spec
        .sources
        .iter()
        .filter_map(|source| source.path())
        .any(|file| {
            let configured = Path::new(file);

            changed == configured || changed.ends_with(configured) || configured.ends_with(changed)
        });

    if explicit {
        return true;
    }

    // Only the searched directories are watched, so matching on the file name
    // alone is enough — and it catches a `config.toml` that did not exist when
    // the watcher started.
    if spec
        .search
        .as_ref()
        .is_some_and(|search| discovery::is_candidate(changed, search.name))
    {
        return true;
    }

    is_mount_marker(changed, spec)
}

/// Whether `changed` is the bookkeeping entry of an atomically remounted
/// directory holding one of our files.
///
/// Kubernetes updates a ConfigMap by writing a new timestamped directory and
/// swinging a `..data` symlink at it. The configuration file's own path never
/// receives an event — only `..data` and `..2026_08_09_12_00_00` do — so
/// matching on the file alone sees a ConfigMap update as silence. Entries
/// beginning with `..` are the kubelet's convention and effectively nothing
/// else's, which keeps this from firing on ordinary files.
fn is_mount_marker(changed: &Path, spec: &LoadSpec<'static>) -> bool {
    let is_marker = changed
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(".."));

    if !is_marker {
        return false;
    }

    let Some(directory) = changed.parent() else {
        return false;
    };

    let mut watched = spec
        .sources
        .iter()
        .filter_map(|source| source.path())
        .filter_map(|file| Path::new(file).parent());

    if watched.any(|parent| directory.ends_with(parent) || parent.ends_with(directory)) {
        return true;
    }

    spec.search.as_ref().is_some_and(|search| {
        discovery::search_directories(search)
            .iter()
            .any(|parent| directory.ends_with(parent) || parent.ends_with(directory))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{CreateKind, ModifyKind};

    /// A spec that names one file explicitly and searches nowhere.
    fn explicit_spec() -> LoadSpec<'static> {
        static SOURCES: &[crate::Source<'static>] =
            &[crate::Source::file("config.toml", crate::Format::Toml)];

        LoadSpec::new("app", SOURCES)
    }

    fn event(kind: EventKind, path: &str) -> Event {
        Event {
            kind,
            paths: vec![PathBuf::from(path)],
            attrs: Default::default(),
        }
    }

    #[test]
    fn an_absolute_event_path_matches_a_relative_configured_path() {
        let batch = [event(
            EventKind::Modify(ModifyKind::Any),
            "/srv/app/config.toml",
        )];

        assert!(touches_configured_file(&batch, &explicit_spec()));
    }

    #[test]
    fn a_discovered_name_matches_even_though_no_file_was_listed() {
        let paths: &'static [&'static str] = &["/srv/app"];
        let spec = LoadSpec::new("db", &[]).with_search("config", paths);

        let batch = [event(
            EventKind::Create(CreateKind::File),
            "/srv/app/config.toml",
        )];
        assert!(touches_configured_file(&batch, &spec));

        let batch = [event(
            EventKind::Create(CreateKind::File),
            "/srv/app/other.toml",
        )];
        assert!(!touches_configured_file(&batch, &spec));
    }

    #[test]
    fn an_unrelated_file_in_the_same_directory_is_ignored() {
        let batch = [event(
            EventKind::Modify(ModifyKind::Any),
            "/srv/app/notes.txt",
        )];

        assert!(!touches_configured_file(&batch, &explicit_spec()));
    }

    #[test]
    fn access_events_do_not_trigger_a_reload() {
        let batch = [event(
            EventKind::Access(notify::event::AccessKind::Read),
            "/srv/app/config.toml",
        )];

        assert!(!touches_configured_file(&batch, &explicit_spec()));
    }

    #[test]
    fn a_duplicate_handle_owns_nothing_and_frees_nothing() {
        let spec = explicit_spec();

        let first = spawn("DuplicateTest", spec, Duration::from_millis(10), || {
            Ok(None)
        })
        .expect("the first spawn should start a watcher");
        let second = spawn("DuplicateTest", spec, Duration::from_millis(10), || {
            Ok(None)
        })
        .expect("the second spawn should be a no-op");

        // Dropping the duplicate must not deregister the name out from under
        // the watcher that is actually running.
        drop(second);
        assert!(
            STARTED.lock().unwrap().contains("DuplicateTest"),
            "the running watcher should still hold its name"
        );

        drop(first);
        assert!(
            !STARTED.lock().unwrap().contains("DuplicateTest"),
            "dropping the owning handle should free the name for a restart"
        );
    }

    /// A failed spawn must free its name, or every retry afterwards returns a
    /// success handle that owns nothing and watches nothing — silently, which
    /// is the exact path a program hits when its config directory does not
    /// exist yet at startup.
    #[test]
    fn a_failed_spawn_frees_its_name_for_a_retry() {
        static BAD: &[crate::Source<'static>] = &[crate::Source::file(
            "/nonexistent-dynamic-config-test-dir/config.toml",
            crate::Format::Toml,
        )];

        let bad = LoadSpec::new("app", BAD);

        assert!(
            spawn("FailedSpawnTest", bad, Duration::from_millis(10), || Ok(
                None
            ))
            .is_err(),
            "watching a directory that does not exist should fail"
        );
        assert!(
            !STARTED.lock().unwrap().contains("FailedSpawnTest"),
            "a failed spawn must not keep its name registered"
        );

        // And the retry gets a *real* watcher, proven by its drop freeing the
        // name — a do-nothing duplicate handle would leave it registered.
        let handle = spawn(
            "FailedSpawnTest",
            explicit_spec(),
            Duration::from_millis(10),
            || Ok(None),
        )
        .expect("the name is free, so the retry starts a watcher");

        drop(handle);

        assert!(
            !STARTED.lock().unwrap().contains("FailedSpawnTest"),
            "the retry owned a real watcher, whose drop frees the name"
        );
    }

    #[test]
    fn creation_and_removal_both_count_as_changes() {
        for kind in [
            EventKind::Create(CreateKind::File),
            EventKind::Remove(notify::event::RemoveKind::File),
        ] {
            let batch = [event(kind, "config.toml")];

            assert!(
                touches_configured_file(&batch, &explicit_spec()),
                "{kind:?}"
            );
        }
    }
}

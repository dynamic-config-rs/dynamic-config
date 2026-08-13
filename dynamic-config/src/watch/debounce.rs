//! The background loop: block on a relevant event, wait out the flurry,
//! reload once.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use notify::Event;

use crate::error::Error;
#[cfg(not(feature = "tracing"))]
use crate::log::info;
use crate::log::warning;

use super::relevance::relevant_path;
use super::Watched;

/// Pause after the debounce window, before the files are read back.
///
/// An atomic save writes a temporary file and renames it into place. The rename
/// can be observed a hair before the new inode is visible, so a short grace
/// period avoids reading a file that is about to be replaced.
const ATOMIC_SAVE_GRACE: Duration = Duration::from_millis(25);

pub(super) fn run(
    name: &'static str,
    watched: &Watched,
    debounce: Duration,
    reload: impl Fn(&Path) -> Result<Option<String>, Error>,
    receiver: &mpsc::Receiver<notify::Result<Event>>,
) {
    loop {
        let trigger = match collect_relevant(receiver, name, debounce, watched) {
            Collected::Dirty(path) => path,
            Collected::Disconnected => {
                // The watcher was dropped, so no further events can arrive.
                return;
            }
        };

        thread::sleep(ATOMIC_SAVE_GRACE);

        // Under `tracing`, the reload is a span with its outcome and
        // duration as fields — enough to alert on "has not reloaded
        // cleanly in an hour" without parsing message strings. Without it,
        // the messages below carry the duration and stderr carries the
        // messages.
        #[cfg(feature = "tracing")]
        let _span = ::tracing::info_span!(target: "dynamic_config", "config_reload", config = name)
            .entered();

        let started = std::time::Instant::now();
        let outcome = reload(&trigger);
        let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

        // Under `tracing`, the outcome and duration are structured *fields*
        // — the alert the docs promise must not require parsing message
        // strings. Without it, the stderr lines carry both in text.
        #[cfg(feature = "tracing")]
        match &outcome {
            Ok(summary) => ::tracing::info!(
                target: "dynamic_config",
                config = name,
                outcome = "reloaded",
                duration_ms,
                summary = summary.as_deref().unwrap_or(""),
                "{name}: reloaded in {duration_ms}ms"
            ),
            Err(error) => ::tracing::warn!(
                target: "dynamic_config",
                config = name,
                outcome = "failed",
                duration_ms,
                error = %error,
                "{name}: reload failed in {duration_ms}ms, keeping the previous snapshot"
            ),
        }

        #[cfg(not(feature = "tracing"))]
        match outcome {
            Ok(Some(summary)) => info!("{name}: reloaded in {duration_ms}ms, {summary}"),
            Ok(None) => info!("{name}: reloaded in {duration_ms}ms"),
            Err(error) => warning!(
                "{name}: reload failed after {duration_ms}ms, keeping the previous snapshot: \
                 {error}"
            ),
        }
    }
}

/// What one round of event collection concluded.
enum Collected {
    /// A configured file changed; reload. Carries the path whose event
    /// opened the window — see [`collect_relevant`].
    Dirty(PathBuf),
    /// The channel closed; the watch is over.
    Disconnected,
}

/// Blocks until a *relevant* event arrives, then debounces — with a ceiling.
///
/// Three deliberate properties, each the fix for a shipped mistake:
///
/// - **Relevance is decided per event, before anything else.** The old code
///   batched first and filtered after, so a chatty neighbour in the config
///   directory — a log file, a state file — both filled a growing `Vec` and
///   kept pushing the quiet-period out. An irrelevant event now costs a
///   comparison and is gone.
/// - **No batch at all.** One dirty flag, and one path with it: whether a
///   configured file changed is one bit, and a bit cannot grow. The path is
///   the *first* relevant event's, kept so the reload can say what triggered
///   it; the later ones are deliberately dropped rather than collected,
///   because a Kubernetes remount writes a fresh timestamped directory name
///   every time and a set of those grows without bound.
/// - **`max_wait` bounds the debounce.** The quiet-period restarts on every
///   relevant event, which is the point of debouncing — but under a
///   sustained storm of writes it used to restart forever, and the reload
///   starved. From the first relevant event, at most `4 × debounce` passes
///   before the reload happens regardless.
fn collect_relevant(
    receiver: &mpsc::Receiver<notify::Result<Event>>,
    name: &'static str,
    debounce: Duration,
    watched: &Watched,
) -> Collected {
    // Phase 1: sleep until something we care about happens.
    let trigger = loop {
        match receiver.recv() {
            Ok(Ok(event)) => {
                if let Some(path) = relevant_path(&event, watched) {
                    break path.to_path_buf();
                }
            }
            Ok(Err(error)) => warning!("{name}: watcher error: {error}"),
            Err(mpsc::RecvError) => return Collected::Disconnected,
        }
    };

    // Phase 2: wait out the flurry an editor save produces, but not forever.
    let deadline = std::time::Instant::now() + debounce.saturating_mul(4);
    let mut quiet_until = std::time::Instant::now() + debounce;

    loop {
        let now = std::time::Instant::now();
        // The nearer of "one debounce of quiet elapsed" and the hard ceiling.
        let target = quiet_until.min(deadline);

        if now >= target {
            return Collected::Dirty(trigger);
        }

        match receiver.recv_timeout(target - now) {
            // A relevant event restarts the quiet period (up to the deadline);
            // an irrelevant one merely waits out the remainder — a neighbour's
            // churn must not delay our reload.
            Ok(Ok(event)) => {
                if relevant_path(&event, watched).is_some() {
                    quiet_until = std::time::Instant::now() + debounce;
                }
            }
            Ok(Err(error)) => warning!("{name}: watcher error: {error}"),
            Err(mpsc::RecvTimeoutError::Timeout) => return Collected::Dirty(trigger),
            Err(mpsc::RecvTimeoutError::Disconnected) => return Collected::Disconnected,
        }
    }
}

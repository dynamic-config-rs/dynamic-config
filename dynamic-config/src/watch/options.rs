//! The three numbers a file watch runs on.
//!
//! All three already governed every watcher; two of them were simply not
//! reachable. The debounce was an argument, the ceiling was `debounce × 4`
//! written into the loop, and the atomic-save grace was a process-wide
//! setter — which is the right default and the wrong granularity for a
//! program watching one configuration on a local disk and another on a
//! network mount.

use core::time::Duration;

/// How a file watch waits.
///
/// ```text
/// event ──▶ ┌─ debounce ─┐ quiet? ──▶ grace ──▶ read
///           └─ restarted by every further event ─┘
///                        └─ but never past the ceiling ─┘
/// ```
///
/// - **`debounce`** is the quiet period. An editor's save is three or four
///   filesystem events, and reloading on each of them reads a file that is
///   still being written.
/// - **`ceiling`** bounds it. The quiet period restarts on every event,
///   which is the point — but under a sustained storm of writes it would
///   restart forever and the reload would starve. Four times the debounce
///   unless it is set.
/// - **`atomic_save_grace`** is the pause after the window closes. An
///   atomic save writes a temporary file and renames it into place, and the
///   rename can be observed a hair before the new inode is visible.
///   [`set_atomic_save_grace`](crate::watch::set_atomic_save_grace)'s
///   process-wide value unless it is set here.
///
/// A bare [`Duration`] converts, so the one-argument form still reads the
/// way it always did:
///
/// ```no_run
/// # #[cfg(all(feature = "watch", feature = "json"))] {
/// # use core::time::Duration;
/// # use serde::Deserialize;
/// # #[dynamic_config::dynamic_config]
/// # #[derive(Debug, Deserialize)]
/// # struct Config { host: String }
/// use dynamic_config::WatchOptions;
///
/// // The common case, unchanged.
/// let handle = Config::builder("svc").watch(Duration::from_millis(100))?;
///
/// // And the same watch, with the numbers under it named.
/// let handle = Config::builder("svc").watch(
///     WatchOptions::new(Duration::from_millis(100))
///         .ceiling(Duration::from_secs(2))
///         .atomic_save_grace(Duration::from_millis(50)),
/// )?;
/// # }
/// # Ok::<(), std::io::Error>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchOptions {
    debounce: Duration,
    ceiling: Option<Duration>,
    grace: Option<Duration>,
}

impl WatchOptions {
    /// The quiet period, with the other two left at their defaults.
    #[must_use]
    pub const fn new(debounce: Duration) -> Self {
        Self {
            debounce,
            ceiling: None,
            grace: None,
        }
    }

    /// The longest the quiet period may keep restarting before the reload
    /// happens regardless.
    ///
    /// Four times the debounce unless this is called. Raise it where writes
    /// arrive in long bursts and reading mid-burst is worse than waiting;
    /// lower it where a reload must not be starved.
    #[must_use]
    pub const fn ceiling(mut self, ceiling: Duration) -> Self {
        self.ceiling = Some(ceiling);
        self
    }

    /// The pause between the window closing and the files being read.
    ///
    /// The process-wide value unless this is called — which is the right
    /// default, because the pause compensates for the *filesystem*. Set it
    /// per watch where one configuration lives on a local disk and another
    /// on a mount whose renames take longer to become visible.
    #[must_use]
    pub const fn atomic_save_grace(mut self, grace: Duration) -> Self {
        self.grace = Some(grace);
        self
    }

    /// The quiet period.
    #[must_use]
    pub const fn debounce_of(&self) -> Duration {
        self.debounce
    }

    /// The ceiling in force, defaulted if it was never set.
    #[must_use]
    pub(crate) fn ceiling_of(&self) -> Duration {
        self.ceiling
            .unwrap_or_else(|| self.debounce.saturating_mul(4))
    }

    /// The grace in force, falling back to the process-wide value.
    #[must_use]
    pub(crate) fn grace_of(&self) -> Duration {
        self.grace
            .unwrap_or_else(super::debounce::atomic_save_grace)
    }
}

impl From<Duration> for WatchOptions {
    fn from(debounce: Duration) -> Self {
        Self::new(debounce)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_duration_is_the_one_argument_form() {
        let options: WatchOptions = Duration::from_millis(100).into();

        assert_eq!(options.debounce_of(), Duration::from_millis(100));
    }

    /// The number that used to be written into the loop as `debounce * 4`.
    #[test]
    fn the_ceiling_defaults_to_four_debounces() {
        let options = WatchOptions::new(Duration::from_millis(100));

        assert_eq!(options.ceiling_of(), Duration::from_millis(400));
    }

    #[test]
    fn a_named_ceiling_wins() {
        let options = WatchOptions::new(Duration::from_millis(100)).ceiling(Duration::from_secs(2));

        assert_eq!(options.ceiling_of(), Duration::from_secs(2));
    }

    /// The grace falls back to the process-wide value, because it
    /// compensates for the filesystem — which every watcher in the process
    /// shares. Naming it per watch is for the program watching a local disk
    /// and a network mount at once.
    #[test]
    fn the_grace_falls_back_to_the_process_wide_value() {
        let options = WatchOptions::new(Duration::from_millis(100));

        assert_eq!(
            options.grace_of(),
            super::super::debounce::atomic_save_grace()
        );

        let named = options.atomic_save_grace(Duration::from_millis(70));

        assert_eq!(named.grace_of(), Duration::from_millis(70));
    }

    /// A ceiling below the debounce is not corrected: it means "reload as
    /// soon as the ceiling is up, whatever the quiet period thinks", which
    /// is a coherent thing to ask for and not this type's business to
    /// second-guess.
    #[test]
    fn a_ceiling_under_the_debounce_is_taken_at_its_word() {
        let options = WatchOptions::new(Duration::from_secs(1)).ceiling(Duration::from_millis(10));

        assert_eq!(options.ceiling_of(), Duration::from_millis(10));
    }
}

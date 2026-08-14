//! What is true of a remote source right now, for an operator asking.
//!
//! The fetch half of the picture `ConfigStatus` starts, in the same
//! vocabulary rather than a second one. What it does *not* carry is the
//! point: no document, no key, and no description of the store — a store
//! URL routinely embeds `user:password@host`.

use std::time::{Duration, Instant};

use crate::reload::FailureStatus;

/// What is true of a remote source right now, for an operator asking.
///
/// The fetch half of the picture [`ConfigStatus`](crate::ConfigStatus)
/// starts, and deliberately the *same* picture rather than a second one:
/// the same [`FailureStatus`] type, the same `consecutive_failures` meaning
/// zero-is-healthy, the same recorded-where-it-happens rule, and the same
/// rendering through [`telemetry::Exposition`](crate::telemetry::Exposition).
/// Two vocabularies for one question is how two surfaces come to disagree
/// after the first bug.
///
/// The two do not overlap, and the split is worth stating because it is the
/// distinction an operator is actually asking about:
///
/// | Question | Where it is answered |
/// |---|---|
/// | did the **store** answer | here |
/// | did the **document** install | [`ConfigStatus`](crate::ConfigStatus) |
///
/// A fetch that returned an unchanged document is a success here and is not
/// an install there, which is exactly the case neither surface could report
/// before this type existed.
///
/// # What it does not carry
///
/// **No document, no key, and no description of the store.** A store's
/// description is its URL, and a store URL routinely embeds
/// `user:password@host` — so nothing here is derived from
/// [`describe`](crate::Remote::describe), and the name a metric is labelled with
/// is the caller's own, exactly as it is for a `ConfigStatus`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct RemoteStatus {
    /// Documents this slot has received since the process started, whether
    /// pulled by [`refresh`](crate::Remote::refresh) or pushed through
    /// [`RemoteSink::apply`](crate::RemoteSink::apply).
    pub fetches: u64,
    /// When the last of them arrived. `None` before the first.
    pub last_fetch: Option<Instant>,
    /// How long the last *pulled* fetch took.
    ///
    /// `None` before the first pull, and `None` again after a document
    /// arrives by push: a watch loop's own round trip is timed by the store
    /// crate that made it, and reporting the previous pull's duration beside
    /// a push's timestamp would be a number that is not about the fetch it
    /// appears to describe.
    pub last_fetch_duration: Option<Duration>,
    /// The most recent fetch that returned nothing, if there has been one.
    /// Kept after a later success: it is history, and
    /// [`consecutive_failures`](Self::consecutive_failures) is the health.
    pub last_failure: Option<FailureStatus>,
    /// Fetches that returned nothing since one returned a document.
    /// **Zero means healthy.**
    pub consecutive_failures: u32,
}

impl RemoteStatus {
    /// Whether the store answered the last time it was asked.
    ///
    /// Three states rather than two, and the third is the point: `None`
    /// before anything has been asked of the store at all. A source that has
    /// been installed and never fetched is not *down* — reporting it as down
    /// is how a scrape at startup pages somebody — so the metric is absent
    /// rather than zero, exactly as `last_success_seconds` is.
    #[must_use]
    pub fn reachable(&self) -> Option<bool> {
        if self.fetches == 0 && self.consecutive_failures == 0 {
            return None;
        }

        Some(self.consecutive_failures == 0)
    }

    /// A status with nothing recorded yet.
    ///
    /// `const`, because [`Remote::new`] is: a `Remote` lives in a `static`.
    /// `Default` cannot be, which is the only reason this exists.
    pub(super) const fn empty() -> Self {
        Self {
            fetches: 0,
            last_fetch: None,
            last_fetch_duration: None,
            last_failure: None,
            consecutive_failures: 0,
        }
    }

    /// How long ago the last document arrived from the store.
    ///
    /// `None` before the first. Monotonic, for the reason
    /// [`ConfigStatus::stale_for`](crate::ConfigStatus::stale_for) is: a wall
    /// clock going backwards under NTP would make a fresh fetch look stale.
    #[must_use]
    pub fn stale_for(&self) -> Option<Duration> {
        self.last_fetch.map(|at| at.elapsed())
    }
}

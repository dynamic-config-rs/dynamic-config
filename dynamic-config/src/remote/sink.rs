//! The fenced door a watch loop pushes through.
//!
//! A sink remembers which source was installed when it was taken, and
//! refuses a delivery from a store that has since been replaced — by
//! construction rather than by asking a loop to please stop first.

use super::{Fetched, Remote, RemoteStatus};
use crate::error::Error;

/// A fenced door for a remote watch loop's pushes.
///
/// Created by the generated `remote_sink()` *after* the source is
/// installed, it remembers which source that was. [`apply`](Self::apply)
/// installs the document and reloads — unless the source has since been
/// replaced, in which case it refuses: a watch loop serving yesterday's
/// store cannot overwrite today's, by construction rather than by the old
/// documentation's request to please stop the loop first.
///
/// Cheap to clone; each wiring of a watch loop should take its own —
/// **once, where the loop starts**. A sink taken per delivery reads the
/// generation of that moment and fences nothing.
#[derive(Clone, Copy)]
pub struct RemoteSink {
    remote: &'static Remote,
    generation: u64,
    reload: fn() -> Result<(), Error>,
    name: &'static str,
}

impl RemoteSink {
    /// Not public API: called by the generated `remote_sink()`.
    #[doc(hidden)]
    #[must_use]
    pub fn new(
        remote: &'static Remote,
        reload: fn() -> Result<(), Error>,
        name: &'static str,
    ) -> Self {
        Self {
            remote,
            generation: remote.generation(),
            reload,
            name,
        }
    }

    /// How the fetches from the store behind this sink have gone.
    ///
    /// The door a `#[dynamic_config]` type has to its
    /// [`RemoteStatus`](crate::RemoteStatus): the slot itself is generated private, and a sink is
    /// the public handle on it — which is also where the question belongs,
    /// since a sink is what a watch loop holds.
    ///
    /// Taking a sink *only* to read this is fine and costs an atomic load:
    /// the generation a sink captures fences
    /// [`apply`](Self::apply) and nothing else. A loop that will deliver
    /// documents still takes its own, once, where it starts.
    ///
    /// ```no_run
    /// # struct DbConfig;
    /// # impl DbConfig {
    /// #     fn remote_sink() -> dynamic_config::RemoteSink { unimplemented!() }
    /// # }
    /// let status = DbConfig::remote_sink().status();
    ///
    /// if status.reachable() == Some(false) {
    ///     eprintln!("the store has stopped answering");
    /// }
    /// ```
    ///
    /// With the `telemetry` feature, `Exposition::add_remote` renders the
    /// same status as Prometheus text; see
    /// [the telemetry module](crate::telemetry). The example above stays
    /// feature-free on purpose, because this method is not.
    #[must_use]
    pub fn status(&self) -> RemoteStatus {
        self.remote.status()
    }

    /// Reports an attempt to reach the store that came back with nothing.
    ///
    /// A watch loop is the half of a store this crate cannot see.
    /// [`apply`](Self::apply) records a delivery, so a *working* watch keeps
    /// [`RemoteStatus`](crate::RemoteStatus) current — but a loop whose stream broke, whose
    /// blocking query is erroring or whose credential was refused delivers
    /// nothing, and would otherwise say nothing: `reachable` would report the
    /// last delivery rather than the last attempt, and a store that stopped
    /// answering an hour ago would look healthy until something called
    /// `refresh`.
    ///
    /// What it moves is deliberately narrow — the failure streak and the last
    /// failure, and nothing else. `fetches`, `last_fetch` and
    /// `last_fetch_duration` are left alone, so
    /// `dynamic_config_remote_last_fetch_seconds` keeps *ageing* while
    /// `dynamic_config_remote_up` goes to zero, which is the pair an alert
    /// wants. The stored document is untouched: a failed attempt is no reason
    /// to stop serving what the last good one produced.
    ///
    /// Fenced on the sink's generation exactly as [`apply`](Self::apply) is,
    /// so a loop still winding down after its source was replaced cannot
    /// charge its failures to the replacement. A stale report is dropped
    /// silently, and there is nothing to handle: a loop must never have to
    /// deal with a failure to report a failure.
    ///
    /// The error's kind and key path are recorded and nothing else — a
    /// store's address never enters a [`RemoteStatus`](crate::RemoteStatus), for the reason its
    /// own documentation gives.
    pub fn failed(&self, error: &Error) {
        // The fence is inside `record_fetch_failure`, under the same lock
        // that reads the generation: a check here and a write there would
        // leave a window for a replacement to land between them.
        self.remote.record_fetch_failure(error, self.generation);
    }

    /// Installs a document the watch pushed, and reloads.
    ///
    /// Everything a file change would do happens here too — validation,
    /// the reload hooks, the cache — because it is the same code path,
    /// reached with a document instead of a filesystem event. A failure
    /// leaves the previous snapshot serving.
    ///
    /// # Errors
    ///
    /// If the source has been replaced since this sink was created —
    /// checked before the reload *and again after it*, because a
    /// replacement can land while the reload runs — or if the resulting
    /// configuration does not load or validate.
    pub fn apply(&self, document: Fetched) -> Result<(), Error> {
        self.remote.install_if(self.generation, document)?;

        let outcome = (self.reload)();

        // The reload read the slot as it stood while it ran. If the source
        // was replaced mid-flight — after `install_if` said yes — what just
        // installed may derive from this sink's document even though the
        // fence now belongs to the replacement. Reload once more against
        // the slot as it stands, so the replacement's state has the last
        // word, then refuse like any other stale push.
        if self.remote.generation() != self.generation {
            let _ = (self.reload)();

            let error = Error::new(
                crate::ErrorKind::Backend,
                "the remote source this sink was created for was replaced \
                 while its delivery reloaded; the replacement's state was \
                 restored — stop the old watch loop and take a fresh sink \
                 from `remote_sink()`",
            );
            crate::__log_remote_failure(self.name, &error);

            return Err(error);
        }

        match outcome {
            Ok(()) => {
                crate::__log_remote_reload(self.name, None);

                Ok(())
            }
            Err(error) => {
                crate::__log_remote_failure(self.name, &error);

                Err(error)
            }
        }
    }
}

impl std::fmt::Debug for RemoteSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteSink")
            .field("config", &self.name)
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

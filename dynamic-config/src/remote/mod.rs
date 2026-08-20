//! Configuration served from somewhere other than this machine.
//!
//! etcd, Consul, NATS, Vault — a document fetched over a network and merged
//! like a file. The companion crates implement one of the two traits here;
//! this module is the part that never changes.
//!
//! ## Fetching is explicit
//!
//! A remote source is **not** read on every `load()`. Configuration is read on
//! nearly every request; a network round trip there would be indefensible, and
//! it is also what forces every async question to become a blocking one.
//!
//! ```text
//! refresh_remote()          →  fetch, keep the document
//! load()                    →  merge the kept document, no I/O
//! ```
//!
//! That one decision is what lets a blocking source and an async source sit
//! side by side without `block_on` anywhere, and without the crate caring which
//! runtime — if any — the program is built on.
//!
//! ## Where it sits
//!
//! ```text
//! defaults < files < remote < environment < flags < overrides
//! ```
//!
//! Above the files, because centrally distributed configuration should beat
//! what a package shipped. Below the environment, because a machine's own
//! settings should beat what a central store thinks it wants.
//!
//! ## Timeouts
//!
//! Every companion crate that takes a `with_timeout` means the same thing by
//! it: **the deadline for a single fetch attempt, excluding retries the
//! underlying client performs.** One sentence, seven stores, whatever each
//! client happens to call the knob underneath.
//!
//! The exclusion is the part that surprises. Where a client retries beneath us
//! — the AWS SDK does, by default — a fetch can take the timeout multiplied by
//! the attempt count, and that store's README says so rather than quietly
//! tuning the retries away.
//!
//! ## Two ways a fetch fails
//!
//! [`ErrorKind::Remote`] is the store being unreachable; [`ErrorKind::Auth`] is
//! a credential it refused. The difference is exactly what a watch loop needs:
//! the first may fix itself while the loop waits, the second will not. So a
//! store crate reaches for [`Error::auth`] only where the store's own answer
//! says so — a 401, a 403, a token that could not be replaced — and stays on
//! [`Error::remote`] wherever a proxy could have been the one talking.
//!
//! ## What a fetch reports about itself
//!
//! [`Remote`] records a [`RemoteStatus`](crate::RemoteStatus) — how many documents have arrived,
//! when the last one did, how long the last pull took, and how many fetches
//! have returned nothing since one returned a document. It is the fetch half
//! of the picture [`ConfigStatus`](crate::ConfigStatus) starts, in the same
//! vocabulary rather than a second one: *did the store answer* here, *did the
//! document install* there.
//!
//! Neither the document nor the store's description can reach it. A store's
//! description is its URL and a store URL routinely embeds
//! `user:password@host`, so nothing derived from
//! [`describe`](crate::Remote::describe) is recorded, spanned or labelled — the name
//! a metric carries is supplied by whoever renders it, exactly as it is for a
//! `ConfigStatus`.
//!
//! With the `tracing` feature a pull is also a `dynamic_config.fetch` span
//! around the round trip, with an event inside it carrying the outcome and,
//! on a failure, the [`ErrorKind`]. Nothing is on the read path: `load()`
//! reads [`Remote::document`], which none of this touches.
//!
//! ## Watching
//!
//! Polling a store on a timer works and is what [`Vault`] has to do, but three
//! of the four can tell you the moment a value moves — etcd has a watch stream,
//! NATS KV has one too, and Consul answers a blocking query. Each companion
//! crate owns that loop, because a watch is long-lived and protocol-shaped in a
//! way a single trait cannot honestly cover.
//!
//! What the loop pushes through is here: a document arrives, [`Remote::install`]
//! puts it in the slot, and a [`RemoteSink`](crate::RemoteSink)'s `apply` reloads exactly the way
//! a file change does — hooks, diffing, validation, the cache.
//!
//! The two halves are cancelled differently, and neither imposes a runtime:
//!
//! - **An async loop is a future.** Drop it and the watch stops. That is the
//!   whole cancellation story, and it works on any executor.
//! - **A blocking loop is a thread**, which cannot be dropped from outside, so
//!   it takes a [`Watching`] and checks it between requests. The caller holds
//!   the matching [`RemoteWatch`].
//!
//! [`Vault`]: https://docs.rs/dynamic-config-vault
//!
//! ## The files
//!
//! One concern each, and the split follows the vocabulary rather than the
//! line count: `Fetched` and the two traits are what a *store* implements
//! (`source`); `RemoteStatus` is what an operator reads (`status`);
//! `Remote` is the slot a configuration type owns (here); `RemoteSink` is
//! the door a watch loop pushes through (`sink`); and the blocking watch
//! handle is `watch`. Every name below is re-exported at the crate root
//! exactly where it was.

mod sink;
mod source;
mod status;
mod watch;

pub use sink::RemoteSink;
#[cfg(feature = "async")]
pub use source::AsyncRemoteSource;
pub use source::{Fetched, RemoteSource, WatchCapability};
pub use status::RemoteStatus;
pub use watch::{Pace, RemoteWatch, Watching};

use std::sync::Arc;

use crate::sync::Mutex;
use std::time::{Duration, Instant};

use crate::error::{Error, ErrorKind};
use crate::reload::FailureStatus;

/// The remote source for one configuration type, and its last document.
///
/// `Remote::new()` is `const`, so this lives in a `static` — which is how
/// `#[dynamic_config]` emits it.
///
/// # What it records about itself
///
/// Every fetch this type performs and every delivery it accepts is counted
/// into a [`RemoteStatus`](crate::RemoteStatus), on the same terms `ConfigCell` records a
/// [`ConfigStatus`](crate::ConfigStatus): recorded where it happens, read by
/// an atomic-cheap [`status`](Self::status), and never on the read path —
/// `load()` reads [`document`](Self::document), which this does not touch.
/// The cost is one `Instant::now()` per fetch, beside a network round trip.
#[derive(Default)]
pub struct Remote {
    /// One lock for the whole state, deliberately. Two separate locks — one
    /// for the source, one for the document — allowed an interleaving where
    /// a slow fetch from the *old* source committed its result after `set`
    /// had installed a new one: new source, old store's document. The
    /// generation counter is the fence that makes that impossible.
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    source: Option<Kind>,
    fetched: Option<Fetched>,
    /// Bumped on every source change. A fetch snapshots it before the network
    /// round trip and commits only if it has not moved — a result from a
    /// source that is no longer installed is discarded, never stored.
    ///
    /// It is *source identity*, and that is the whole of it: a
    /// [`RemoteSink`](crate::RemoteSink) holds one for the life of a watch loop, so anything
    /// that moves this number ends that loop.
    generation: u64,
    /// Bumped by [`clear`](crate::Remote::clear), and by nothing else.
    ///
    /// A counter of its own rather than a bump of `generation`, because the
    /// two questions differ: clearing drops the *document* and leaves the
    /// source installed. Folding it into `generation` made every live
    /// [`RemoteSink`](crate::RemoteSink) permanently stale — a watch loop whose store had not
    /// changed and whose stream was still delivering would have every later
    /// push refused for belonging to a source that had been "replaced". The
    /// in-flight fetch a `clear` must still discard is fenced on this.
    cleared: u64,
    /// How the fetches have gone. Under the same lock as everything else
    /// here, so a scrape cannot read a count that belongs to one source
    /// beside a document that belongs to another.
    status: RemoteStatus,
}

/// The state a fetch started under, in the two numbers that can invalidate
/// its result: the source it was fetching from, and the document epoch it
/// was fetching into.
///
/// Captured before the round trip and compared after it. Both halves are
/// needed and neither is enough: a replaced source must discard the result,
/// and so must a `clear` — but only the first ends a watch, which is why
/// they are counted apart.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Fence {
    generation: u64,
    cleared: u64,
}

impl Fence {
    fn of(state: &State) -> Self {
        Self {
            generation: state.generation,
            cleared: state.cleared,
        }
    }
}

/// `Arc` rather than `Box`: an async fetch borrows the source across an await
/// point, and cloning the handle out of the lock first is what keeps a `std`
/// mutex from being held across one.
#[derive(Clone)]
enum Kind {
    Blocking(Arc<dyn RemoteSource>),
    #[cfg(feature = "async")]
    Asynchronous(Arc<dyn AsyncRemoteSource>),
}

impl Remote {
    /// An empty slot: no source, no document.
    #[must_use]
    #[cfg(not(loom))]
    pub const fn new() -> Self {
        Self {
            state: Mutex::new(State {
                source: None,
                fetched: None,
                generation: 0,
                cleared: 0,
                status: RemoteStatus::empty(),
            }),
        }
    }

    /// The same, minus `const`: loom's constructors are not.
    #[must_use]
    #[cfg(loom)]
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State {
                source: None,
                fetched: None,
                generation: 0,
                cleared: 0,
                status: RemoteStatus::empty(),
            }),
        }
    }

    /// Installs a blocking source, replacing any previous one.
    ///
    /// The document already fetched, if any, is dropped with it — a new source
    /// answering with an old store's values would be a puzzle nobody needs.
    /// A fetch from the previous source that is still in flight is discarded
    /// when it lands, for the same reason.
    ///
    /// The recorded [`status`](Self::status) is dropped with the document:
    /// `remote_up` for the *previous* store says nothing about this one, and
    /// a stale `1` describing a store nobody is talking to any more is worse
    /// than no sample at all.
    pub fn set(&self, source: impl RemoteSource) {
        let mut state = self.state();
        state.source = Some(Kind::Blocking(Arc::new(source)));
        state.fetched = None;
        state.status = RemoteStatus::empty();
        state.generation = state.generation.wrapping_add(1);
    }

    /// Installs an async source, replacing any previous one.
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    pub fn set_async(&self, source: impl AsyncRemoteSource) {
        let mut state = self.state();
        state.source = Some(Kind::Asynchronous(Arc::new(source)));
        state.fetched = None;
        state.status = RemoteStatus::empty();
        state.generation = state.generation.wrapping_add(1);
    }

    /// Fetches, and keeps what came back.
    ///
    /// The network round trip happens with no lock held: a slow store cannot
    /// make `load()` — which reads this state for provenance — wait for it.
    /// If the source is replaced while the fetch is in flight, the result is
    /// discarded and `Ok` is returned: the fetch *did* succeed, and the new
    /// source's own refresh is the one that matters now.
    ///
    /// # Errors
    ///
    /// If no source is installed, if the installed one is async — use
    /// [`refresh_async`](Self::refresh_async) — or if the fetch fails.
    pub fn refresh(&self) -> Result<(), Error> {
        let (source, fence) = {
            let state = self.state();

            match state.source.as_ref() {
                Some(Kind::Blocking(source)) => (Arc::clone(source), Fence::of(&state)),

                #[cfg(feature = "async")]
                Some(Kind::Asynchronous(source)) => {
                    return Err(Error::new(
                        ErrorKind::Remote,
                        format!(
                            "`{}` is an async source; refresh it with `refresh_remote_async`",
                            source.describe()
                        ),
                    ))
                }

                None => return Err(none_installed()),
            }
        };

        // The span covers the round trip rather than following it, which is
        // the only arrangement that gives a trace a duration to draw. It
        // carries no name for the store: the one string a source has is its
        // description, and a store URL routinely embeds `user:password@host`.
        #[cfg(feature = "tracing")]
        let span = crate::telemetry::fetching();

        let started = Instant::now();

        match source.fetch() {
            Ok(fetched) => {
                let elapsed = started.elapsed();

                self.commit(fetched, fence);
                self.record_fetch(Some(elapsed), fence.generation);

                #[cfg(feature = "tracing")]
                crate::telemetry::fetched(&span, elapsed);

                Ok(())
            }
            Err(error) => {
                self.record_fetch_failure(&error, fence.generation);

                #[cfg(feature = "tracing")]
                crate::telemetry::fetch_failed(&span, &error);

                Err(error)
            }
        }
    }

    /// Fetches from an async source, and keeps what came back.
    ///
    /// A *blocking* source is not refused — swapping one implementation for
    /// the other must not be a breaking change for the caller — but it is not
    /// run on the executor either: it goes through
    /// [`off_thread`](crate::off_thread), so an async caller's worker thread
    /// never sits inside a blocking network call.
    ///
    /// The same replaced-mid-fetch rule as [`refresh`](Self::refresh)
    /// applies, and matters more here: the unlocked window spans an await.
    ///
    /// # Errors
    ///
    /// If no source is installed, or the fetch fails.
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    pub async fn refresh_async(&self) -> Result<(), Error> {
        // Cloned out of the lock before anything is awaited: holding a `std`
        // mutex across an await point is how an executor deadlocks itself.
        let (source, fence) = {
            let state = self.state();

            match state.source.as_ref() {
                Some(source) => (source.clone(), Fence::of(&state)),
                None => return Err(none_installed()),
            }
        };

        // Not entered: this span is held across an await, and an
        // `EnteredSpan` is `!Send`. `Span::in_scope` cannot wrap an await
        // either, so what a subscriber gets here is the span's own timing
        // and its fields rather than an ambient context — which is what a
        // fetch has to report anyway, since nothing else runs inside it.
        #[cfg(feature = "tracing")]
        let span = crate::telemetry::fetching_async();

        let started = Instant::now();

        let outcome = match source {
            Kind::Blocking(source) => crate::asynchronous::off_thread(move || source.fetch()).await,
            Kind::Asynchronous(source) => source.fetch().await,
        };

        match outcome {
            Ok(fetched) => {
                let elapsed = started.elapsed();

                self.commit(fetched, fence);
                self.record_fetch(Some(elapsed), fence.generation);

                #[cfg(feature = "tracing")]
                crate::telemetry::fetched(&span, elapsed);

                Ok(())
            }
            Err(error) => {
                self.record_fetch_failure(&error, fence.generation);

                #[cfg(feature = "tracing")]
                crate::telemetry::fetch_failed(&span, &error);

                Err(error)
            }
        }
    }

    /// The generation a sink created now would carry; see [`RemoteSink`](crate::RemoteSink).
    pub(crate) fn generation(&self) -> u64 {
        self.state().generation
    }

    /// Installs `document` if the source it came from is still the one
    /// installed — the push-side twin of the fetch fence.
    ///
    /// # Errors
    ///
    /// When the source has been replaced since `generation` was captured:
    /// the document belongs to a store nobody asked about any more, and
    /// installing it would hand a stale watcher the last word.
    pub(crate) fn install_if(&self, generation: u64, document: Fetched) -> Result<(), Error> {
        let mut state = self.state();

        if state.generation != generation {
            return Err(Error::new(
                crate::ErrorKind::Backend,
                "the remote source this sink was created for has been \
                 replaced; stop the old watch loop and take a fresh sink \
                 from `remote_sink()`",
            ));
        }

        state.fetched = Some(document);

        // A push is a fetch somebody else performed: the store answered, and
        // that is the whole question `RemoteStatus` reports on. Whether the
        // document then *installs* is `ConfigStatus`'s business, and
        // `RemoteSink::apply` records it there through the reload it runs.
        state.status.fetches = state.status.fetches.saturating_add(1);
        state.status.last_fetch = Some(Instant::now());
        state.status.last_fetch_duration = None;
        state.status.consecutive_failures = 0;

        Ok(())
    }

    /// Not public API: the loom suite's door to the fence internals.
    #[cfg(loom)]
    #[doc(hidden)]
    #[must_use]
    pub fn generation_for_loom(&self) -> u64 {
        self.generation()
    }

    /// Not public API: the loom suite's door to the fence internals.
    ///
    /// # Errors
    ///
    /// As `install_if`.
    #[cfg(loom)]
    #[doc(hidden)]
    pub fn install_if_for_loom(&self, generation: u64, document: Fetched) -> Result<(), Error> {
        self.install_if(generation, document)
    }

    /// How the installed store learns that its document changed.
    ///
    /// `None` when nothing is installed. What an agent reads to decide
    /// whether to run a watch at all, and what a report prints so an
    /// operator can see why a change took as long as it did.
    #[must_use]
    pub fn watch_capability(&self) -> Option<WatchCapability> {
        match self.state().source.as_ref()? {
            Kind::Blocking(source) => Some(source.watch_capability()),

            #[cfg(feature = "async")]
            Kind::Asynchronous(source) => Some(source.watch_capability()),
        }
    }

    /// Watches the installed store, keeping every document it delivers.
    ///
    /// The store's own mechanism where it has one — a blocking query, a
    /// stream, a subscription — and a jittered, backing-off poll where it
    /// does not. `interval` is the poll period, and a *resync* period for a
    /// store that pushes: a stream that has silently stalled looks exactly
    /// like a store where nothing has changed.
    ///
    /// Blocks until the watch is stopped, so it belongs on a thread of its
    /// own. Each document is stored the way a
    /// [`refresh`](Self::refresh) stores one, generation fence included —
    /// a document from a source that has since been replaced is discarded
    /// rather than installed.
    ///
    /// # Errors
    ///
    /// If no source is installed, if the installed one is async, or if the
    /// store's watch gives up. A *fetch* failing is not an error: a watch
    /// outlives an outage by design.
    pub fn watch(&self, watching: &Watching, interval: Duration) -> Result<(), Error> {
        let (source, fence) = {
            let state = self.state();

            match state.source.as_ref() {
                Some(Kind::Blocking(source)) => (Arc::clone(source), Fence::of(&state)),

                #[cfg(feature = "async")]
                Some(Kind::Asynchronous(source)) => {
                    return Err(Error::new(
                        ErrorKind::Remote,
                        format!(
                            "`{}` is an async source; watch it with `watch_async`",
                            source.describe()
                        ),
                    ))
                }
                None => {
                    return Err(Error::new(
                        ErrorKind::Remote,
                        "no remote source is configured",
                    ))
                }
            }
        };

        let mut deliver = |document| {
            self.commit(document, fence);
            self.record_fetch(None, fence.generation);

            Ok(())
        };

        if source.watch_capability() != WatchCapability::Native {
            return source.watch(watching, interval, &mut deliver);
        }

        // A store that pushes still gets read on the interval, because the
        // failure mode of a stream is silence: a connection that dropped
        // without an error, a subscription the broker forgot, a blocking
        // query answering an index that will never move again. All three
        // look exactly like a store where nothing has changed, and the only
        // way to tell them apart is to go and ask.
        //
        // Scoped threads rather than an executor: the resync is this call's
        // for as long as this call lasts, and nothing outlives it.
        std::thread::scope(|scope| {
            let source = Arc::clone(&source);
            let watcher = scope.spawn(move || source.watch(watching, interval, &mut deliver));
            let mut pace = Pace::new(interval);

            while watching.keep_going() && !watcher.is_finished() {
                // Sliced rather than slept whole, so the resync notices the
                // store's own watch returning instead of waiting out an
                // interval to find out.
                let mut left = pace.next_wait();

                while left > Duration::ZERO && watching.keep_going() && !watcher.is_finished() {
                    let slice = left.min(Duration::from_millis(250));

                    std::thread::sleep(slice);
                    left -= slice;
                }

                if watching.keep_going() && !watcher.is_finished() {
                    match self.refresh() {
                        Ok(()) => pace.succeeded(),
                        Err(_) => pace.failed(),
                    }
                }
            }

            watcher
                .join()
                .unwrap_or_else(|_| Err(Error::new(ErrorKind::Remote, "the watch panicked")))
        })
    }

    /// [`watch`](Self::watch), for a store that is read asynchronously.
    ///
    /// Cancellation is dropping the future.
    ///
    /// # Errors
    ///
    /// As [`watch`](Self::watch), with the sides swapped.
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    pub async fn watch_async(&self, watching: &Watching, interval: Duration) -> Result<(), Error> {
        let (source, fence) = {
            let state = self.state();

            match state.source.as_ref() {
                Some(Kind::Asynchronous(source)) => (Arc::clone(source), Fence::of(&state)),
                Some(Kind::Blocking(source)) => {
                    return Err(Error::new(
                        ErrorKind::Remote,
                        format!(
                            "`{}` is a blocking source; watch it with `watch` on a thread",
                            source.describe()
                        ),
                    ))
                }
                None => {
                    return Err(Error::new(
                        ErrorKind::Remote,
                        "no remote source is configured",
                    ))
                }
            }
        };

        let mut deliver = |document| {
            self.commit(document, fence);
            self.record_fetch(None, fence.generation);

            Ok(())
        };

        source.watch(watching, interval, &mut deliver).await
    }

    /// The document last fetched, if any.
    #[must_use]
    pub fn document(&self) -> Option<Fetched> {
        self.state().fetched.clone()
    }

    /// Whether a source is installed.
    #[must_use]
    pub fn is_configured(&self) -> bool {
        self.state().source.is_some()
    }

    /// How the fetches from this source have gone.
    ///
    /// One lock and a clone, no I/O and no network: an exporter may call it
    /// per scrape, which is the same contract
    /// [`ConfigCell::status`](crate::ConfigCell::status) makes.
    #[must_use]
    pub fn status(&self) -> RemoteStatus {
        self.state().status.clone()
    }

    /// Records a fetch that returned a document.
    ///
    /// Fenced on the source `generation` the fetch started under, and under
    /// the one lock that reads it: [`set`](Self::set) empties the status
    /// along with the document, so an old fetch landing afterwards would
    /// otherwise report the *replacement* as fetched and healthy — a store
    /// nothing has yet spoken to.
    fn record_fetch(&self, elapsed: Option<Duration>, generation: u64) {
        let mut state = self.state();

        if state.generation != generation {
            return;
        }

        state.status.fetches = state.status.fetches.saturating_add(1);
        state.status.last_fetch = Some(Instant::now());
        state.status.last_fetch_duration = elapsed;
        state.status.consecutive_failures = 0;
    }

    /// Records a fetch that returned nothing.
    ///
    /// The document is untouched: a store that stopped answering leaves the
    /// last one it did answer with in place, and the counter is what says
    /// so. Only the failure's category and key path are kept — the same
    /// [`FailureStatus`] a refused reload records, for the same reason.
    ///
    /// Fenced like [`record_fetch`](Self::record_fetch), and for the mirror
    /// reason: an old fetch's failure must not report a store that has just
    /// been installed as down.
    fn record_fetch_failure(&self, error: &Error, generation: u64) {
        let mut state = self.state();

        if state.generation != generation {
            return;
        }

        // Saturating rather than wrapping, as `ConfigCell` does: a counter
        // that rolls over to zero reads as "healthy" at the worst moment.
        state.status.consecutive_failures = state.status.consecutive_failures.saturating_add(1);
        state.status.last_failure = Some(FailureStatus::of(error));
    }

    /// How the installed source names itself.
    #[must_use]
    pub fn describe(&self) -> Option<String> {
        // The lock is held only for the clone: `describe()` on the source runs
        // unlocked, so a source whose description does real work cannot stall
        // readers.
        let source = self.state().source.clone()?;

        Some(match source {
            Kind::Blocking(source) => source.describe(),
            #[cfg(feature = "async")]
            Kind::Asynchronous(source) => source.describe(),
        })
    }

    /// Drops the document, so the next load sees no remote layer.
    ///
    /// A fetch that was already in flight is discarded when it lands, the
    /// same way [`set`](Self::set) discards one: clearing is a state change
    /// like any other, and a document a caller explicitly dropped must not
    /// come back from a round trip that started before they dropped it.
    ///
    /// The *source* is left alone, and so is every [`RemoteSink`](crate::RemoteSink) taken from
    /// it: a watch loop delivering from the same store keeps delivering, and
    /// its next push installs normally. Dropping the document is not
    /// replacing the store, and only replacing the store ends a watch.
    pub fn clear(&self) {
        let mut state = self.state();

        state.fetched = None;
        state.cleared = state.cleared.wrapping_add(1);
    }

    /// Stores a fetch result, unless the slot moved while it was in flight —
    /// the source was replaced, and the result belongs to a store nobody
    /// asked about any more, or the document was cleared and putting this
    /// one back would undo that.
    fn commit(&self, fetched: Fetched, fence: Fence) {
        let mut state = self.state();

        if Fence::of(&state) == fence {
            state.fetched = Some(fetched);
        }
    }

    fn state(&self) -> crate::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl std::fmt::Debug for Remote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Remote")
            .field("source", &self.describe())
            .field("fetched", &self.document().is_some())
            .finish()
    }
}

fn none_installed() -> Error {
    Error::new(
        ErrorKind::Remote,
        "no remote source is installed; call `set_remote` first",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Format;
    use crate::sync::atomic::Ordering;

    struct Fake(&'static str);

    impl RemoteSource for Fake {
        fn fetch(&self) -> Result<Fetched, Error> {
            Ok(Fetched::new(self.0, Format::Json))
        }

        fn describe(&self) -> String {
            "a fake store".to_owned()
        }
    }

    struct Broken;

    impl RemoteSource for Broken {
        fn fetch(&self) -> Result<Fetched, Error> {
            Err(Error::remote("the store is unreachable"))
        }

        fn describe(&self) -> String {
            "a broken store".to_owned()
        }
    }

    #[test]
    fn nothing_is_fetched_until_it_is_asked_for() {
        let remote = Remote::new();
        remote.set(Fake(r#"{"db": {"host": "a"}}"#));

        assert!(remote.is_configured());
        assert!(
            remote.document().is_none(),
            "installing a source must not reach the network"
        );

        remote.refresh().unwrap();
        assert!(remote.document().is_some());
    }

    /// Succeeds once, then fails — a store that answered and went away.
    struct Flaky(std::sync::atomic::AtomicBool);

    impl RemoteSource for Flaky {
        fn fetch(&self) -> Result<Fetched, Error> {
            if self.0.swap(true, Ordering::SeqCst) {
                return Err(Error::remote("the store went away"));
            }

            Fake(r#"{"db": {"host": "a"}}"#).fetch()
        }

        fn describe(&self) -> String {
            "a store that answers once".to_owned()
        }
    }

    #[test]
    fn a_failed_fetch_leaves_the_previous_document_alone() {
        let remote = Remote::new();
        remote.set(Flaky(std::sync::atomic::AtomicBool::new(false)));
        remote.refresh().unwrap();

        let before = remote.document();
        assert!(before.is_some(), "the first fetch succeeds");

        // The second fetch *fails*, and the failure must surface — while the
        // document from the fetch that worked stays where it was.
        let error = remote.refresh().unwrap_err();

        assert!(error.to_string().contains("went away"), "{error}");
        assert_eq!(remote.document(), before);
    }

    /// Blocks inside `fetch` on a pair of barriers, so a test can hold a
    /// fetch mid-flight while it does something else to the `Remote`.
    struct Parked {
        started: std::sync::Arc<std::sync::Barrier>,
        release: std::sync::Arc<std::sync::Barrier>,
    }

    impl RemoteSource for Parked {
        fn fetch(&self) -> Result<Fetched, Error> {
            self.started.wait();
            self.release.wait();

            Fake(r#"{"db": {"host": "stale"}}"#).fetch()
        }

        fn describe(&self) -> String {
            "a parked store".to_owned()
        }
    }

    /// The same, for the failing half of the fence: parked mid-fetch, and
    /// what it finally returns is an error.
    struct ParkedThenBroken {
        started: std::sync::Arc<std::sync::Barrier>,
        release: std::sync::Arc<std::sync::Barrier>,
    }

    impl RemoteSource for ParkedThenBroken {
        fn fetch(&self) -> Result<Fetched, Error> {
            self.started.wait();
            self.release.wait();

            Broken.fetch()
        }

        fn describe(&self) -> String {
            "a parked store that then breaks".to_owned()
        }
    }

    /// The race the generation fence exists for: a fetch from the *old*
    /// source lands after `set` installed a new one. Its result must be
    /// discarded — new source, old store's document is the state this
    /// module's docs promise cannot happen.
    #[test]
    fn a_fetch_from_a_replaced_source_is_discarded() {
        let started = std::sync::Arc::new(std::sync::Barrier::new(2));
        let release = std::sync::Arc::new(std::sync::Barrier::new(2));

        let remote = std::sync::Arc::new(Remote::new());
        remote.set(Parked {
            started: std::sync::Arc::clone(&started),
            release: std::sync::Arc::clone(&release),
        });

        let refresher = {
            let remote = std::sync::Arc::clone(&remote);
            std::thread::spawn(move || remote.refresh())
        };

        // The fetch is provably in flight...
        started.wait();

        // ...when the source is replaced.
        remote.set(Fake(r#"{"db": {"host": "fresh"}}"#));

        release.wait();
        refresher
            .join()
            .expect("the refresher must not panic")
            .expect("the fetch itself succeeded");

        assert_eq!(
            remote.document(),
            None,
            "the old source's document landed after the replacement and must \
             not be paired with the new source"
        );

        // And the new source works normally.
        remote.refresh().unwrap();
        assert!(remote.document().unwrap().text.contains("fresh"));
    }

    /// The same fence, from the other side: `clear()` is a state change too,
    /// so a fetch that was in flight when a caller cleared the slot must not
    /// put the document back. The barriers force the interleaving — the
    /// fetch is provably parked when `clear` runs — so this is a proof
    /// rather than a race the scheduler usually loses.
    #[test]
    fn a_fetch_in_flight_when_the_slot_is_cleared_is_discarded() {
        let started = std::sync::Arc::new(std::sync::Barrier::new(2));
        let release = std::sync::Arc::new(std::sync::Barrier::new(2));

        let remote = std::sync::Arc::new(Remote::new());
        remote.set(Parked {
            started: std::sync::Arc::clone(&started),
            release: std::sync::Arc::clone(&release),
        });

        let refresher = {
            let remote = std::sync::Arc::clone(&remote);
            std::thread::spawn(move || remote.refresh())
        };

        started.wait();

        remote.clear();

        release.wait();
        refresher
            .join()
            .expect("the refresher must not panic")
            .expect("the fetch itself succeeded");

        assert_eq!(
            remote.document(),
            None,
            "a document the caller cleared must not come back from a fetch \
             that started before they cleared it"
        );
    }

    /// Clearing the document must not end a watch. The source is untouched
    /// by `clear()`, so a loop that took its sink before the call is still
    /// serving the store it was created for, and its next delivery installs
    /// like any other. The first shape of this fence counted both events on
    /// one number and made every live sink permanently stale.
    #[test]
    fn clearing_the_document_leaves_a_watchs_sink_alive() {
        let remote = Remote::new();
        remote.set(Fake(r#"{"db": {"host": "a"}}"#));

        // What `remote_sink()` captures, once, where a loop starts.
        let generation = remote.generation();

        remote.clear();

        remote
            .install_if(generation, Fetched::new("{}", crate::Format::Json))
            .expect("clearing the document does not replace the source");
        assert!(remote.document().is_some());
    }

    /// The status fence, from the side `set` opens: an old fetch that
    /// succeeds after its source was replaced must not report the
    /// replacement — which nothing has yet spoken to — as fetched and
    /// healthy. `set` empties the status precisely so that it says nothing
    /// about a store that is no longer installed.
    #[test]
    fn a_late_fetch_does_not_report_the_replacement_as_healthy() {
        let started = std::sync::Arc::new(std::sync::Barrier::new(2));
        let release = std::sync::Arc::new(std::sync::Barrier::new(2));

        let remote = std::sync::Arc::new(Remote::new());
        remote.set(Parked {
            started: std::sync::Arc::clone(&started),
            release: std::sync::Arc::clone(&release),
        });

        let refresher = {
            let remote = std::sync::Arc::clone(&remote);
            std::thread::spawn(move || remote.refresh())
        };

        started.wait();
        remote.set(Fake(r#"{"db": {"host": "fresh"}}"#));
        release.wait();

        let _ = refresher.join().expect("the refresher must not panic");

        let status = remote.status();
        assert_eq!(
            status.fetches, 0,
            "the replacement has been fetched from nobody"
        );
        assert_eq!(status.last_fetch, None);
        assert_eq!(status.reachable(), None);
    }

    /// The same fence for a failure. A store that was replaced while its
    /// fetch was erroring must not leave the new one looking down.
    #[test]
    fn a_late_failure_does_not_report_the_replacement_as_down() {
        let started = std::sync::Arc::new(std::sync::Barrier::new(2));
        let release = std::sync::Arc::new(std::sync::Barrier::new(2));

        let remote = std::sync::Arc::new(Remote::new());
        remote.set(ParkedThenBroken {
            started: std::sync::Arc::clone(&started),
            release: std::sync::Arc::clone(&release),
        });

        let refresher = {
            let remote = std::sync::Arc::clone(&remote);
            std::thread::spawn(move || remote.refresh())
        };

        started.wait();
        remote.set(Fake(r#"{"db": {"host": "fresh"}}"#));
        release.wait();

        let _ = refresher.join().expect("the refresher must not panic");

        let status = remote.status();
        assert_eq!(status.consecutive_failures, 0);
        assert_eq!(
            status.reachable(),
            None,
            "nothing has yet asked the replacement anything"
        );
    }

    /// Readers must not wait for a slow store: `document()` and `describe()`
    /// are on the `load()` path, and `load()` promises to touch no network.
    #[test]
    fn readers_are_not_blocked_by_a_fetch_in_flight() {
        let started = std::sync::Arc::new(std::sync::Barrier::new(2));
        let release = std::sync::Arc::new(std::sync::Barrier::new(2));

        let remote = std::sync::Arc::new(Remote::new());
        remote.set(Parked {
            started: std::sync::Arc::clone(&started),
            release: std::sync::Arc::clone(&release),
        });

        let refresher = {
            let remote = std::sync::Arc::clone(&remote);
            std::thread::spawn(move || remote.refresh())
        };

        started.wait();

        // With the fetch parked, a reader thread must finish promptly. The
        // old two-lock design held the source lock across the fetch, so
        // `describe()` — and with it every `load()` — waited out the store's
        // full timeout.
        let (sender, receiver) = std::sync::mpsc::channel();
        {
            let remote = std::sync::Arc::clone(&remote);
            std::thread::spawn(move || {
                let described = remote.describe();
                let document = remote.document();
                let _ = sender.send((described, document));
            });
        }

        let (described, document) = receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("readers must not wait for the network");

        assert_eq!(described.as_deref(), Some("a parked store"));
        assert_eq!(document, None);

        release.wait();
        let _ = refresher.join();
    }

    /// `set` clears the document atomically with the source swap: no
    /// interleaving may observe the new source paired with any document.
    #[test]
    fn replacing_the_source_and_dropping_the_document_is_one_step() {
        let remote = Remote::new();
        remote.set(Fake(r#"{"db": {"host": "a"}}"#));
        remote.refresh().unwrap();
        let generation = remote.generation();
        remote
            .install_if(generation, Fetched::new("{}", crate::Format::Json))
            .expect("the source has not moved");

        remote.set(Fake(r#"{"db": {"host": "b"}}"#));

        assert_eq!(remote.document(), None);

        // And the push-side fence itself: the pre-swap generation is now
        // stale, so a late delivery bounces instead of landing.
        remote
            .install_if(generation, Fetched::new("{}", crate::Format::Json))
            .expect_err("a replaced source's generation must be refused");
        assert_eq!(remote.document(), None);
    }

    #[test]
    fn a_broken_store_reports_rather_than_pretending() {
        let remote = Remote::new();
        remote.set(Broken);

        let error = remote.refresh().unwrap_err();

        assert_eq!(error.kind(), ErrorKind::Remote);
        assert!(error.to_string().contains("unreachable"), "{error}");
    }

    #[test]
    fn refreshing_with_no_source_says_so() {
        let error = Remote::new().refresh().unwrap_err();

        assert!(error.to_string().contains("set_remote"), "{error}");
    }

    #[test]
    fn replacing_the_source_drops_the_old_document() {
        let remote = Remote::new();
        remote.set(Fake(r#"{"db": {"host": "a"}}"#));
        remote.refresh().unwrap();

        remote.set(Fake(r#"{"db": {"host": "b"}}"#));

        assert!(
            remote.document().is_none(),
            "a new source answering with the old store's values would be a puzzle"
        );
    }
}

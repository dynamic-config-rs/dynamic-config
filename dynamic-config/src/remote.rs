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
//! ## Watching
//!
//! Polling a store on a timer works and is what [`Vault`] has to do, but three
//! of the four can tell you the moment a value moves — etcd has a watch stream,
//! NATS KV has one too, and Consul answers a blocking query. Each companion
//! crate owns that loop, because a watch is long-lived and protocol-shaped in a
//! way a single trait cannot honestly cover.
//!
//! What the loop pushes through is here: a document arrives, [`Remote::install`]
//! puts it in the slot, and the generated `apply_remote` reloads exactly the way
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

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use crate::error::{Error, ErrorKind};
use crate::source::Format;

/// A document a remote store handed back.
#[derive(Clone, PartialEq, Eq)]
pub struct Fetched {
    /// The document text, in `format`.
    pub text: String,
    /// How to parse it.
    pub format: Format,
}

impl Fetched {
    /// A document and the format it is written in.
    #[must_use]
    pub fn new(text: impl Into<String>, format: Format) -> Self {
        Self {
            text: text.into(),
            format,
        }
    }
}

// The document is the one thing a `Debug` of this type must never print:
// a remote store's flagship use case is serving secrets, and `Fetched` is
// what every watch callback receives — one `tracing::debug!(?document)` away
// from a log. The length is enough to debug with.
impl std::fmt::Debug for Fetched {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fetched")
            .field("format", &self.format)
            .field("bytes", &self.text.len())
            .finish()
    }
}

/// A remote store that can be read without an async runtime.
///
/// The right trait for anything with a plain HTTP API — Consul and Vault both
/// are — because implementing it needs no runtime and using it needs no
/// runtime either. `fetch` may block; it is called from
/// `refresh_remote()`, never from `load()`.
pub trait RemoteSource: Send + Sync + 'static {
    /// Reads the current document.
    ///
    /// # Errors
    ///
    /// Whatever going wrong looks like for this store. Use
    /// [`Error::remote`](crate::Error::remote) so the failure is categorised
    /// consistently.
    fn fetch(&self) -> Result<Fetched, Error>;

    /// How to name this source in an error or a report.
    fn describe(&self) -> String;
}

/// A remote store that is read asynchronously.
///
/// The right trait for a client that is async to begin with — etcd speaks gRPC
/// and NATS is a streaming protocol, so both are. Used through
/// `refresh_remote_async().await`.
///
/// The lifetime-bound boxed future rather than `async fn`: this trait is
/// object-safe on purpose, so a configuration type can hold one without being
/// generic over it.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub trait AsyncRemoteSource: Send + Sync + 'static {
    /// Reads the current document.
    ///
    /// # Errors
    ///
    /// As [`RemoteSource::fetch`].
    fn fetch(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Fetched, Error>> + Send + '_>>;

    /// How to name this source in an error or a report.
    fn describe(&self) -> String;
}

/// The remote source for one configuration type, and its last document.
///
/// `Remote::new()` is `const`, so this lives in a `static` — which is how
/// `#[dynamic_config]` emits it.
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
    generation: u64,
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
    pub const fn new() -> Self {
        Self {
            state: Mutex::new(State {
                source: None,
                fetched: None,
                generation: 0,
            }),
        }
    }

    /// Installs a blocking source, replacing any previous one.
    ///
    /// The document already fetched, if any, is dropped with it — a new source
    /// answering with an old store's values would be a puzzle nobody needs.
    /// A fetch from the previous source that is still in flight is discarded
    /// when it lands, for the same reason.
    pub fn set(&self, source: impl RemoteSource) {
        let mut state = self.state();
        state.source = Some(Kind::Blocking(Arc::new(source)));
        state.fetched = None;
        state.generation = state.generation.wrapping_add(1);
    }

    /// Installs an async source, replacing any previous one.
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    pub fn set_async(&self, source: impl AsyncRemoteSource) {
        let mut state = self.state();
        state.source = Some(Kind::Asynchronous(Arc::new(source)));
        state.fetched = None;
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
        let (source, generation) = {
            let state = self.state();

            match state.source.as_ref() {
                Some(Kind::Blocking(source)) => (Arc::clone(source), state.generation),

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

        let fetched = source.fetch()?;

        self.commit(fetched, generation);

        Ok(())
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
        let (source, generation) = {
            let state = self.state();

            match state.source.as_ref() {
                Some(source) => (source.clone(), state.generation),
                None => return Err(none_installed()),
            }
        };

        let fetched = match source {
            Kind::Blocking(source) => {
                crate::asynchronous::off_thread(move || source.fetch()).await?
            }
            Kind::Asynchronous(source) => source.fetch().await?,
        };

        self.commit(fetched, generation);

        Ok(())
    }

    /// Puts a document in the slot without fetching one.
    ///
    /// What a watch loop calls: the document already arrived, pushed by the
    /// store, and re-fetching it to learn what it just said would be silly.
    ///
    /// No source need be installed for this to work — a program that only ever
    /// watches never has to configure one. A watch loop serving a source that
    /// has since been replaced should be stopped with its
    /// [`RemoteWatch`] — this call cannot tell one store's push from
    /// another's.
    pub fn install(&self, document: Fetched) {
        self.state().fetched = Some(document);
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
    pub fn clear(&self) {
        self.state().fetched = None;
    }

    /// Stores a fetch result, unless the source changed while it was in
    /// flight — then the result belongs to a store nobody asked about any
    /// more, and storing it would pair the new source with the old store's
    /// values.
    fn commit(&self, fetched: Fetched, generation: u64) {
        let mut state = self.state();

        if state.generation == generation {
            state.fetched = Some(fetched);
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
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

// ---------------------------------------------------------------------------
// Stopping a blocking watch
// ---------------------------------------------------------------------------

/// A running blocking watch, from the caller's side.
///
/// Dropping it stops the loop — the same contract the file watcher's
/// `WatchHandle` has, for the same reason: a watch nobody owns is a leak nobody
/// asked for. [`detach`](Self::detach) is the way to say *this one really should
/// run forever*.
///
/// Only blocking loops need this. An async watch is a future: drop it and it is
/// cancelled, on any executor.
///
/// ```no_run
/// # use dynamic_config::RemoteWatch;
/// # struct Consul;
/// # impl Consul {
/// #     fn watch(&self, _: dynamic_config::Watching, _: fn(dynamic_config::Fetched) -> Result<(), dynamic_config::Error>) -> Result<(), dynamic_config::Error> { Ok(()) }
/// # }
/// # fn example(consul: Consul) {
/// # fn apply(_: dynamic_config::Fetched) -> Result<(), dynamic_config::Error> { Ok(()) }
/// let watch = RemoteWatch::new();
/// let watching = watch.watching();
///
/// std::thread::spawn(move || consul.watch(watching, apply));
///
/// // ... and later, or by dropping `watch`:
/// watch.stop();
/// # }
/// ```
#[must_use = "dropping the handle stops the watch; bind it, or call `.detach()` \
              to watch for the rest of the process"]
#[derive(Debug)]
pub struct RemoteWatch {
    running: Arc<AtomicBool>,
}

impl RemoteWatch {
    /// A handle for a watch that has not been handed to a loop yet.
    pub fn new() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(true)),
        }
    }

    /// The loop's half of this handle.
    ///
    /// Hand it to the watch; keep the `RemoteWatch` yourself.
    #[must_use]
    pub fn watching(&self) -> Watching {
        Watching {
            running: Arc::downgrade(&self.running),
        }
    }

    /// Stops the loop at its next check.
    ///
    /// *At its next check* is the whole caveat, and it is not small: a loop
    /// parked in a blocking query does not return until the store answers or
    /// the wait expires, so the store's wait time is the worst-case delay. Each
    /// companion crate documents its own.
    pub fn stop(&self) {
        self.running.store(false, Ordering::Release);
    }

    /// Whether the loop has been told to stop.
    #[must_use]
    pub fn is_stopped(&self) -> bool {
        !self.running.load(Ordering::Acquire)
    }

    /// Watches for the remainder of the process.
    ///
    /// Leaks the handle on purpose, exactly as the file watcher's
    /// `WatchHandle::detach` does: a watch that must never stop has no owner to
    /// hold it, and pretending otherwise is how it ends up stopped at the end of
    /// `main`'s first statement.
    pub fn detach(self) {
        std::mem::forget(self);
    }
}

impl Default for RemoteWatch {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for RemoteWatch {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The loop's half of a [`RemoteWatch`].
///
/// A `Weak`, so a handle that is dropped without anyone remembering to call
/// `stop` still ends the loop: the upgrade fails and
/// [`keep_going`](Self::keep_going) answers `false`.
#[derive(Debug, Clone)]
pub struct Watching {
    running: Weak<AtomicBool>,
}

impl Watching {
    /// Whether the loop should go round again.
    ///
    /// `false` once the caller called [`RemoteWatch::stop`] or dropped the
    /// handle. Check it before every request, not only after one: a loop that
    /// checks only on the way out issues one more query than it was asked to.
    #[must_use]
    pub fn keep_going(&self) -> bool {
        self.running
            .upgrade()
            .is_some_and(|running| running.load(Ordering::Acquire))
    }

    /// Sleeps for `total`, waking early if the watch is stopped.
    ///
    /// The polling loop every blocking store crate writes: sleep a slice,
    /// check [`keep_going`](Self::keep_going), repeat — so a stopped watch
    /// ends within a quarter second instead of at the end of its interval.
    /// Here once, rather than once per store crate.
    pub fn sleep_for(&self, total: Duration) {
        const SLICE: Duration = Duration::from_millis(250);

        let mut slept = Duration::ZERO;

        while slept < total && self.keep_going() {
            std::thread::sleep(SLICE.min(total - slept));
            slept += SLICE;
        }
    }

    /// A token for a watch that should never stop.
    ///
    /// For a loop the caller genuinely wants to outlive everything, so there is
    /// no handle to hold. Prefer [`RemoteWatch::detach`], which says the same
    /// thing at the point where somebody decided it.
    #[must_use]
    pub fn forever() -> Self {
        // A `Weak` that can never upgrade would stop the loop immediately, so
        // this leaks one live flag — one allocation, once, for the life of the
        // process.
        let running = Box::leak(Box::new(Arc::new(AtomicBool::new(true))));

        Self {
            running: Arc::downgrade(running),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        remote.install(Fetched::new("{}", crate::Format::Json));

        remote.set(Fake(r#"{"db": {"host": "b"}}"#));

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

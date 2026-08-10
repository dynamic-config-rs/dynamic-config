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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    /// The document text, in `format`.
    pub text: String,
    /// How to parse it.
    pub format: Format,
}

impl Fetched {
    /// A document and the format it is written in.
    pub fn new(text: impl Into<String>, format: Format) -> Self {
        Self {
            text: text.into(),
            format,
        }
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
    source: Mutex<Option<Kind>>,
    fetched: Mutex<Option<Fetched>>,
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
            source: Mutex::new(None),
            fetched: Mutex::new(None),
        }
    }

    /// Installs a blocking source, replacing any previous one.
    ///
    /// The document already fetched, if any, is dropped with it — a new source
    /// answering with an old store's values would be a puzzle nobody needs.
    pub fn set(&self, source: impl RemoteSource) {
        *self.source_slot() = Some(Kind::Blocking(Arc::new(source)));
        *self.fetched_slot() = None;
    }

    /// Installs an async source, replacing any previous one.
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    pub fn set_async(&self, source: impl AsyncRemoteSource) {
        *self.source_slot() = Some(Kind::Asynchronous(Arc::new(source)));
        *self.fetched_slot() = None;
    }

    /// Fetches, and keeps what came back.
    ///
    /// # Errors
    ///
    /// If no source is installed, if the installed one is async — use
    /// [`refresh_async`](Self::refresh_async) — or if the fetch fails.
    pub fn refresh(&self) -> Result<(), Error> {
        let fetched = {
            let source = self.source_slot();

            match source.as_ref() {
                Some(Kind::Blocking(source)) => source.fetch()?,

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

        *self.fetched_slot() = Some(fetched);

        Ok(())
    }

    /// Fetches from an async source, and keeps what came back.
    ///
    /// # Errors
    ///
    /// If no source is installed, or the fetch fails. A *blocking* source is
    /// run inline here rather than refused: it is already allowed to block, and
    /// refusing would make swapping one implementation for the other a breaking
    /// change for the caller.
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    pub async fn refresh_async(&self) -> Result<(), Error> {
        // Cloned out of the lock before anything is awaited: holding a `std`
        // mutex across an await point is how an executor deadlocks itself.
        let source = self.source_slot().clone();

        let fetched = match source {
            Some(Kind::Blocking(source)) => source.fetch()?,
            Some(Kind::Asynchronous(source)) => source.fetch().await?,
            None => return Err(none_installed()),
        };

        *self.fetched_slot() = Some(fetched);

        Ok(())
    }

    /// Puts a document in the slot without fetching one.
    ///
    /// What a watch loop calls: the document already arrived, pushed by the
    /// store, and re-fetching it to learn what it just said would be silly.
    ///
    /// No source need be installed for this to work — a program that only ever
    /// watches never has to configure one.
    pub fn install(&self, document: Fetched) {
        *self.fetched_slot() = Some(document);
    }

    /// The document last fetched, if any.
    pub fn document(&self) -> Option<Fetched> {
        self.fetched_slot().clone()
    }

    /// Whether a source is installed.
    pub fn is_configured(&self) -> bool {
        self.source_slot().is_some()
    }

    /// How the installed source names itself.
    pub fn describe(&self) -> Option<String> {
        self.source_slot().as_ref().map(|source| match source {
            Kind::Blocking(source) => source.describe(),
            #[cfg(feature = "async")]
            Kind::Asynchronous(source) => source.describe(),
        })
    }

    /// Drops the document, so the next load sees no remote layer.
    pub fn clear(&self) {
        *self.fetched_slot() = None;
    }

    fn source_slot(&self) -> std::sync::MutexGuard<'_, Option<Kind>> {
        self.source
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn fetched_slot(&self) -> std::sync::MutexGuard<'_, Option<Fetched>> {
        self.fetched
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

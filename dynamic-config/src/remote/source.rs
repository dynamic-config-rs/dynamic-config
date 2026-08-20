//! What a store hands back, and the two traits a store implements.
//!
//! Two traits rather than one because a client is either async to begin
//! with or it is not, and making the wrong half pretend costs a `block_on`
//! in somebody's runtime. Both are object-safe: a configuration type holds
//! one without being generic over it.

use std::time::Duration;

use crate::error::Error;
use crate::source::Format;

use super::watch::{Pace, Watching};

/// How a store finds out that its document changed.
///
/// What a store answers here is a fact about the protocol, not a promise
/// about the implementation: it tells a caller what a watch is going to
/// cost, so an agent can decide whether to run one at all and an operator
/// can read why a change took as long as it did.
///
/// ```text
/// Native       the store says so         a blocking query, a stream, a subscription
/// Conditional  the store answers cheaply a version, an ETag, a revision — a header, not a document
/// Interval     nothing but re-reading    the whole document, on a timer
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum WatchCapability {
    /// The store pushes. A change is delivered as soon as it happens, and
    /// the interval is only a resync — a stream can stall without saying so.
    Native,
    /// The store answers "has it changed?" without sending the document.
    /// A poll costs a round trip and almost no bytes.
    Conditional,
    /// Nothing but re-reading the whole document on a timer.
    Interval,
}

impl std::fmt::Display for WatchCapability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Native => "native",
            Self::Conditional => "conditional",
            Self::Interval => "interval",
        })
    }
}

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
    /// consistently, or [`Error::auth`](crate::Error::auth) for a credential
    /// the store itself refused — that is the distinction a watch loop backs
    /// off on rather than stopping.
    fn fetch(&self) -> Result<Fetched, Error>;

    /// How to name this source in an error or a report.
    fn describe(&self) -> String;

    /// How this store learns that its document changed.
    ///
    /// [`Interval`](WatchCapability::Interval) unless a store says
    /// otherwise, which is the honest default: a store that has not been
    /// asked the question has no push to offer.
    fn watch_capability(&self) -> WatchCapability {
        WatchCapability::Interval
    }

    /// Watches until the handle is dropped, calling `on_change` with every
    /// document that differs from the last one delivered.
    ///
    /// **Override this** with the store's own mechanism — a blocking query,
    /// a stream, a subscription — and say so in
    /// [`watch_capability`](Self::watch_capability). An override may ignore
    /// `interval`: a store that reports
    /// [`Native`](WatchCapability::Native) gets its resync from
    /// [`Remote::watch`](crate::Remote::watch), which reads on the interval
    /// alongside the store's own watch. That is not belt and braces — the
    /// failure mode of a stream is *silence*, and a subscription the broker
    /// forgot looks exactly like a store where nothing has changed.
    ///
    /// The default polls: fetch, deliver anything new, wait, repeat. The
    /// waits are spread so a fleet does not poll in lockstep, and they grow
    /// after a failure so a store that is down is not hammered by everything
    /// that depends on it — [`Pace`] is that policy, and an implementation
    /// with its own loop should use it rather than sleep a flat interval.
    ///
    /// Called from a thread the caller owns. It returns when the watch is
    /// stopped, or when `on_change` refuses.
    ///
    /// # Errors
    ///
    /// If `on_change` refuses a document. A *fetch* failing is not an error
    /// here: a watch outlives an outage by design, and the failure is
    /// recorded and backed off from rather than returned.
    fn watch(
        &self,
        watching: &Watching,
        interval: Duration,
        on_change: &mut dyn FnMut(Fetched) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let mut pace = Pace::new(interval);
        let mut last: Option<Fetched> = None;

        while watching.keep_going() {
            match self.fetch() {
                Ok(fetched) => {
                    pace.succeeded();

                    if last.as_ref() != Some(&fetched) {
                        last = Some(fetched.clone());
                        on_change(fetched)?;
                    }
                }
                // Swallowed on purpose: a watch is what keeps a program
                // running through an outage, and a store that is down is a
                // reason to wait longer rather than to stop watching.
                Err(_) => pace.failed(),
            }

            pace.wait(watching);
        }

        Ok(())
    }
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

    /// How this store learns that its document changed.
    ///
    /// As [`RemoteSource::watch_capability`].
    fn watch_capability(&self) -> WatchCapability {
        WatchCapability::Interval
    }

    /// Watches until the future is dropped, calling `on_change` with every
    /// document that differs from the last one delivered.
    ///
    /// As [`RemoteSource::watch`], with two differences that matter.
    /// Cancellation is dropping the future, so a `Watching` is accepted but
    /// an async watch does not need one. And the resync a native store gets
    /// for free on the blocking side is the caller's here: an async caller
    /// has a runtime, and racing a timer against this future is a line of
    /// its own code rather than a thread this crate would have to spawn.
    ///
    /// **The default polls only with the `tokio` feature on.** This crate
    /// picks no runtime, and a poll needs a timer — so with the feature off
    /// the default refuses, naming the store and saying what to do about
    /// it. That is rarely the interesting case: a store is async because
    /// its protocol is, and a streaming protocol has a watch of its own to
    /// override this with.
    ///
    /// # Errors
    ///
    /// If `on_change` refuses a document, or if this build has no timer and
    /// the store did not override this.
    fn watch<'a>(
        &'a self,
        watching: &'a Watching,
        interval: Duration,
        on_change: &'a mut (dyn FnMut(Fetched) -> Result<(), Error> + Send),
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), Error>> + Send + 'a>> {
        Box::pin(async move {
            #[cfg(not(feature = "tokio"))]
            {
                let _ = (watching, interval, on_change);

                Err(Error::new(
                    crate::ErrorKind::Remote,
                    format!(
                        "`{}` has no watch of its own, and this build has no timer to poll it \
                         with; add features = [\"tokio\"] to your dynamic-config dependency, \
                         or call `refresh_remote_async` on a timer of your own",
                        self.describe()
                    ),
                ))
            }

            #[cfg(feature = "tokio")]
            {
                let mut pace = Pace::new(interval);
                let mut last: Option<Fetched> = None;

                while watching.keep_going() {
                    match self.fetch().await {
                        Ok(fetched) => {
                            pace.succeeded();

                            if last.as_ref() != Some(&fetched) {
                                last = Some(fetched.clone());
                                on_change(fetched)?;
                            }
                        }
                        // Swallowed on purpose, as in the blocking twin.
                        Err(_) => pace.failed(),
                    }

                    tokio::time::sleep(pace.next_wait()).await;
                }

                Ok(())
            }
        })
    }
}

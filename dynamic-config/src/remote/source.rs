//! What a store hands back, and the two traits a store implements.
//!
//! Two traits rather than one because a client is either async to begin
//! with or it is not, and making the wrong half pretend costs a `block_on`
//! in somebody's runtime. Both are object-safe: a configuration type holds
//! one without being generic over it.

use crate::error::Error;
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
    /// consistently, or [`Error::auth`](crate::Error::auth) for a credential
    /// the store itself refused — that is the distinction a watch loop backs
    /// off on rather than stopping.
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

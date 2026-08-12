//! Hot-reloadable, lock-free application configuration, built on
//! [figment](https://docs.rs/figment).
//!
//! Declare a struct, configure it with the builder, and read it from
//! anywhere:
//!
//! ```
//! # #[cfg(feature = "json")] {
//! use dynamic_config::dynamic_config;
//! use serde::Deserialize;
//!
//! #[dynamic_config]
//! #[derive(Debug, Deserialize)]
//! struct ServerConfig {
//!     #[serde(default = "default_host")]
//!     host: String,
//!     #[serde(default = "default_port")]
//!     port: u16,
//! }
//!
//! # fn default_host() -> String { "0.0.0.0".into() }
//! # fn default_port() -> u16 { 8080 }
//! // `config.json` does not exist here, so every field falls back to its
//! // default — a missing file is skipped, not an error.
//! ServerConfig::builder("server")
//!     .file("config.json")
//!     .env("APP_")
//!     .init()
//!     .expect("defaults cover every field");
//!
//! let config = ServerConfig::current();
//! println!("{}:{}", config.host, config.port);
//! # }
//! ```
//!
//! The attribute declares — *this type is a configuration* — and generates
//! its storage and accessors. The [`Builder`] configures: where the
//! sources are is runtime data, and it lives in runtime code.
//!
//! This page is the API reference. The guide — profiles, discovery, hot
//! reload, remote stores, encryption, testing — is
//! [**the book**](https://ctolon.github.io/dynamic-config/).
//!
//! # What the attribute generates
//!
//! The attribute declares; the builder configures. What gets generated is
//! the type-bound surface:
//!
//! | Method | Description |
//! |---|---|
//! | `builder(key) -> Builder<Self>` | Where everything starts: state the sources, `init()`. |
//! | `current() -> Arc<Self>` | The current snapshot. Panics before an install. |
//! | `try_current() -> Option<Arc<Self>>` | The current snapshot, or `None`. |
//! | `replace(Self)` | Atomically swap in a new snapshot. |
//! | `on_reload(f)` | Run a callback on every later reload, for the life of the process. |
//! | `on_reload_scoped(f) -> HookGuard` | The same, until the guard is dropped. |
//! | `set_default(path, value)` | A fallback used only when nothing else supplies the key. |
//! | `set_override(path, value)` | A value that wins over every file and variable. |
//! | `clear_defaults()` / `clear_overrides()` | Drop them again. |
//! | `changes()` | With `async`: a handle woken by every later reload. |
//!
//! Everything about *sources* lives on the [`Builder`] the generated
//! `builder(key)` returns: `.file(..)`, `.discover(name, paths)`,
//! `.env(prefix)`, `.strict_env()`, `.env_file(..)`, `.profile_env(..)`,
//! `.cache(path, mode)`, `.validate(f)` — then `.load()`, `.init()`,
//! `.watch(debounce)`, `.explain(path)`, `.check()`, and with `async`,
//! `.load_async()` / `.init_async()`. A successful `init` also *remembers*
//! the builder, so `source_of`, `is_set`, `snapshot`, `check`, `explain`,
//! `prepare` and the remote reload on the type answer for the running
//! configuration. The rest — remote stores, aliases, bindings, flags,
//! `bind_clap` — is in [the book's
//! reference](https://ctolon.github.io/dynamic-config/attribute-reference.html).
//!
//! # Precedence
//!
//! ```text
//! set_default < discovered < config.toml < secrets.json < remote < APP_DB_* < bind_env < set_flag < set_override
//!  (runtime)   (search path)   (first)      (last file)   (etcd…) (environment) (by name)  (CLI)     (runtime)
//! ```
//!
//! Files merge left to right and tables merge key by key, so a small
//! `secrets.json` can override two fields of a large `config.toml` without
//! restating it.
//!
//! The two runtime layers bracket the rest. Defaults cover a fallback the
//! program can compute but a file need not state; overrides are what make a
//! test or a `--set key=value` flag authoritative without touching disk. Both
//! take effect on the next `load()`.
//!
//! # Reading configuration is lock-free
//!
//! `current()` hands out an `Arc` cloned from an `ArcSwap`, so a reload never
//! blocks a request handler. A reader that already holds an `Arc` keeps its own
//! generation — call `current()` once per request and reuse it, or a reload
//! landing mid-request will show you two different configurations.
//!
//! # Reloading cannot take the process down
//!
//! A reload re-runs `load()`. If the new configuration is invalid, or a file is
//! caught half-written, the error is reported and the previous snapshot stays
//! in place. A bad edit degrades to "no change".
//!
//! # Environment variables
//!
//! `env = "APP_"` with `key = "db"` reads `APP_DB_*`. A single underscore is
//! part of a field name; a doubled one introduces nesting:
//!
//! | Variable | Sets |
//! |---|---|
//! | `APP_DB_HOST` | `host` |
//! | `APP_DB_MAX_SIZE` | `max_size` |
//! | `APP_DB_POOL__MAX_SIZE` | `pool.max_size` |
//!
//! Values are interpreted by figment, which reads them loosely: `8080` reaches
//! a `u16`, `true` reaches a `bool`, and `[a, b, c]` reaches a `Vec<String>`.
//! A value that cannot become the field's type is an error naming the field.
//!
//! # Units
//!
//! `timeout = 30` is ambiguous and `max_body = 67108864` is unreadable, so both
//! are usually written with a unit — which no stock `Deserialize` accepts:
//!
//! ```
//! use std::time::Duration;
//! use serde::Deserialize;
//!
//! #[derive(Deserialize)]
//! struct Limits {
//!     #[serde(with = "dynamic_config::duration")]
//!     timeout: Duration,          // "30s", "1h30m", "500ms", or a number of seconds
//!     #[serde(with = "dynamic_config::bytes")]
//!     max_body: u64,              // "64MiB", "1GB", or a number of bytes
//! }
//! ```
//!
//! # Async
//!
//! With the `async` feature, configuration loads without blocking the
//! executor, and tasks can await reloads instead of polling. No runtime is
//! named anywhere: `changes()` is a `Future`, so any executor drives it.
//!
//! ```ignore
//! #[dynamic_config]
//! #[derive(Debug, Deserialize)]
//! struct DbConfig { pool_size: u32 }
//!
//! let builder = DbConfig::builder("db").file("config.json");
//! builder.init_async().await?;
//! // Keep the handle: dropping it stops the watch.
//! let _watch = builder.watch(Duration::from_millis(250))?;
//!
//! let mut reloads = DbConfig::changes();
//!
//! spawn(async move {
//!     loop {
//!         let config = reloads.changed().await;
//!         pool.resize(config.pool_size);
//!     }
//! });
//! ```
//!
//! The watcher itself stays on a plain thread. `notify`'s channel is
//! synchronous, and keeping it off the runtime means file watching works
//! whether or not a runtime is running.
//!
//! # Features
//!
//! | Feature | Default | Effect |
//! |---|---|---|
//! | `json` | yes | `.json` sources |
//! | `toml` | no | `.toml` sources |
//! | `yaml` | no | `.yaml` / `.yml` sources |
//! | `watch` | no | `start_watch()` and the file watcher |
//! | `async` | no | `load_async`, `init_async`, `changes` — no runtime dependency |
//! | `tokio` | no | `async`, plus tokio's blocking pool instead of a thread per load |
//! | `clap` | no | `bind_clap`: named `clap` arguments as the flags layer |
//! | `schema` | no | `schema()`: a JSON Schema for the resolved configuration |
//! | `decrypt` | no | the [`Decryptor`]/[`Encryptor`] traits and `.age`-suffix handling |
//! | `age` | no | `decrypt`, plus the `age` module's implementation of it |
//! | `figment` | no | foreign figment providers as sources, via `Source::provider` |
//! | `dotenv` | no | `env_files = [".env"]`: `.env` files as the environment layer |
//! | `tracing` | no | Watcher diagnostics via `tracing` instead of stderr |
//! | `full` | no | all of the above |
//!
//! Using a format, `watch` or `async` whose feature is disabled is a compile
//! error naming the feature to add.
//!
//! # Without the macro
//!
//! [`load`], [`ConfigCell`] and [`LoadSpec`] are the whole engine and are
//! usable on their own:
//!
//! ```
//! # #[cfg(feature = "json")] {
//! use dynamic_config::{load, Format, LoadSpec, Source};
//! use serde::Deserialize;
//!
//! #[derive(Deserialize)]
//! struct Db { host: String }
//!
//! let sources = [Source::inline(r#"{"db": {"host": "localhost"}}"#, Format::Json)];
//! let db: Db = load(&LoadSpec::new("db", &sources))
//!     .expect("the inline document is well formed");
//!
//! assert_eq!(db.host, "localhost");
//! # }
//! ```

#![forbid(unsafe_code)]
#![deny(missing_docs)]
// A getter whose result is discarded is a mistake in a library like this one —
// `is_set`, `contains`, `document`, `describe` all answer a question and change
// nothing. Warned about rather than left to review, and CI denies warnings.
#![warn(clippy::must_use_candidate)]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(feature = "age")]
#[cfg_attr(docsrs, doc(cfg(feature = "age")))]
pub mod age;
mod aliases;
#[cfg(feature = "async")]
mod asynchronous;
mod bindings;
mod builder;
mod cache;
mod cell;
mod check;
#[cfg(feature = "decrypt")]
mod decrypt;
mod discovery;
#[cfg(feature = "dotenv")]
mod dotenv;
mod dynamic;
mod error;
mod explain;
mod group;
mod layer;
mod loader;
mod log;
mod redirects;
mod registry;
mod remote;
#[cfg(feature = "schema")]
#[cfg_attr(docsrs, doc(cfg(feature = "schema")))]
pub mod schema;
mod snapshot;
mod source;
pub(crate) mod sync;
mod units;
mod value;
mod write;

#[cfg(feature = "watch")]
#[cfg_attr(docsrs, doc(cfg(feature = "watch")))]
pub mod watch;

/// Not public API: the loom suite drives the wake protocol directly.
#[cfg(all(feature = "async", loom))]
#[doc(hidden)]
pub use asynchronous::Notify as LoomNotify;
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub use asynchronous::{set_blocking_executor, BlockingExecutor, Changes};
/// figment itself, re-exported.
///
/// So that writing a [`Source::provider`] needs no direct dependency, and no
/// second version of figment in the graph. This is the one place figment
/// appears in this crate's API, which is why it is behind a feature.
#[cfg(feature = "figment")]
#[cfg_attr(docsrs, doc(cfg(feature = "figment")))]
pub use figment;

pub use aliases::Aliases;
pub use bindings::EnvBindings;
pub use builder::Builder;
#[doc(hidden)]
pub use builder::Configured;
pub use cache::{CacheMode, Recovery};
pub use cell::{ConfigCell, HookGuard};
pub use check::{check, Report, Resolved, UnknownKey};
#[cfg(feature = "decrypt")]
#[cfg_attr(docsrs, doc(cfg(feature = "decrypt")))]
pub use decrypt::{has_decryptor, set_decryptor, Decryptor, Encryptor};
pub use discovery::Search;
pub use dynamic::Dynamic;
pub use error::{Error, ErrorKind, Origin};
pub use explain::{Contribution, Explanation};
pub use group::{Commit, ReloadGroup, Reloadable};
pub use layer::Layer;
pub use registry::Registry;
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub use remote::AsyncRemoteSource;
pub use remote::{Fetched, Remote, RemoteSink, RemoteSource, RemoteWatch, Watching};
pub use snapshot::{changed_paths, Change, ChangeKind, Snapshot};
pub use source::{Format, LoadSpec, Source, DEFAULT_NEST};
pub use units::{bytes, duration};
pub use value::Value;

/// This crate's version, for anything that embeds it and has to say so.
///
/// A language binding versions on its own schedule — its users read a
/// different changelog — so "which engine is inside this wheel" stops
/// being answerable from the outside. This answers it.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Whether `path` touches a secret, given the secret list a type or a
/// binding declared.
///
/// Three ways it can, and all three have to redact or the diagnostic
/// leaks: the path *is* a secret, the path is *under* one (every path
/// below a secret field is the secret's), or the path is an *ancestor* of
/// one — asking to explain `credentials` must not render the password
/// nested inside it. Secrets are named by a plain field for a
/// `#[config(secret)]` field, and by a dotted path when they live inside
/// a nested structure, which is what a language binding derives from a
/// nested model.
#[doc(hidden)]
#[must_use]
pub fn touches_secret(path: &str, secrets: &[impl AsRef<str>]) -> bool {
    secrets.iter().any(|secret| {
        let secret = secret.as_ref();

        secret == path
            || path
                .strip_prefix(secret)
                .is_some_and(|rest| rest.starts_with('.'))
            || secret
                .strip_prefix(path)
                .is_some_and(|rest| rest.starts_with('.'))
    })
}
#[cfg(feature = "decrypt")]
#[cfg_attr(docsrs, doc(cfg(feature = "decrypt")))]
pub use write::save_encrypted;
pub use write::{save, save_new};

/// Turns a struct into a hot-reloadable configuration snapshot.
///
/// See the [crate documentation](crate) for the full guide.
///
/// The attribute takes **no arguments**: it declares that the type *is* a
/// configuration, and generates its storage and surface. Where the
/// configuration comes from is stated on the [`Builder`] the generated
/// `builder(key)` returns — see the front page for the shape, and [the
/// book's
/// reference](https://ctolon.github.io/dynamic-config/attribute-reference.html)
/// for every method. An argument between the parentheses is a compile
/// error whose message maps each old argument to its builder method.
///
/// One field attribute: `#[config(secret)]` generates a `Debug` that prints
/// `***` for the marked fields, forbids `#[derive(Debug)]` alongside it,
/// keeps the field out of the redacted cache, and marks it `writeOnly` in
/// the schema.
///
/// # Requirements
///
/// The annotated struct must implement `serde::Deserialize` and be
/// `Send + Sync + 'static`. Type and const parameters are supported — those go
/// through a `TypeId` registry rather than a `static`, at a measured cost of
/// roughly 10 ns per read. A **lifetime** parameter is rejected at compile
/// time: the snapshot outlives every borrow that could name one.
pub use dynamic_config_macros::dynamic_config;

use serde::de::DeserializeOwned;

/// Reads and deserializes a configuration section.
///
/// This is what the generated `load()` calls. Missing files are skipped;
/// everything else — a parse failure, a missing required field, a value that
/// cannot become the requested type — is an [`Error`] naming the key path and
/// the source it came from.
///
/// # Errors
///
/// See [`ErrorKind`] for the categories.
///
/// # Example
///
/// ```
/// # #[cfg(feature = "json")] {
/// use dynamic_config::{load, Format, LoadSpec, Source};
/// use serde::Deserialize;
///
/// #[derive(Deserialize)]
/// struct Server { port: u16 }
///
/// let sources = [Source::inline(r#"{"server": {"port": 8080}}"#, Format::Json)];
/// let server: Server = load(&LoadSpec::new("server", &sources).with_env("APP_"))
///     .expect("the inline document is well formed");
///
/// assert_eq!(server.port, 8080);
/// # }
/// ```
pub fn load<T: DeserializeOwned>(spec: &LoadSpec<'_>) -> Result<T, Error> {
    loader::load(spec)
}

/// Resolves the section without deserializing it.
///
/// Two snapshots can be compared with [`Snapshot::diff`], which is how a reload
/// reports *which* keys changed rather than only that something did.
///
/// # Errors
///
/// If a source cannot be read or parsed — the same failures as [`load`].
pub fn snapshot(spec: &LoadSpec<'_>) -> Result<Snapshot, Error> {
    loader::snapshot(spec)
}

/// Where the value at `path` would come from, if anything supplies it.
///
/// This is the answer to the question every configuration bug starts with:
/// *which layer set this?* It re-reads the sources, so it reports what the
/// **next** load would see rather than what the current snapshot holds.
///
/// `path` is dotted and relative to the section, as in `"pool.max_size"`.
///
/// # Errors
///
/// If a source cannot be read or parsed — the same failures as [`load`].
///
/// # Example
///
/// ```
/// # #[cfg(feature = "json")] {
/// use dynamic_config::{source_of, Format, LoadSpec, Origin, Source};
///
/// let sources = [Source::inline(r#"{"db": {"host": "localhost"}}"#, Format::Json)];
/// let spec = LoadSpec::new("db", &sources);
///
/// assert_eq!(source_of(&spec, "host").unwrap(), Some(Origin::Inline));
/// assert_eq!(source_of(&spec, "port").unwrap(), None);
/// # }
/// ```
pub fn source_of(spec: &LoadSpec<'_>, path: &str) -> Result<Option<Origin>, Error> {
    loader::source_of(spec, path)
}

/// Explains `path`: every configured layer's answer, not just the winner's.
///
/// The rendered [`Explanation`] **contains values** — that is its point; you
/// asked. It is the one diagnostic in this crate that does, so treat its
/// output accordingly. A path the caller knows to be sensitive goes through
/// [`Explanation::redacted`]; the generated `explain()` does that for
/// `#[config(secret)]` fields automatically.
///
/// # Errors
///
/// Whatever reading the sources reports — the same failures a load would hit.
///
/// # Example
///
/// ```
/// # #[cfg(feature = "json")] {
/// use dynamic_config::{explain, Format, LoadSpec, Source};
///
/// let sources = [Source::inline(r#"{"db": {"port": 5432}}"#, Format::Json)];
/// let explanation = explain(&LoadSpec::new("db", &sources), "port")
///     .expect("the inline document is well formed");
///
/// assert_eq!(explanation.winner().unwrap().layer, "file");
/// println!("{explanation}");
/// # }
/// ```
pub fn explain(spec: &LoadSpec<'_>, path: &str) -> Result<Explanation, Error> {
    explain::explain(spec, path)
}

/// Whether anything supplies `path`.
///
/// Distinguishes "absent" from "present but falsy", which
/// `#[serde(default)]` cannot.
///
/// # Errors
///
/// If a source cannot be read or parsed — the same failures as [`load`].
pub fn is_set(spec: &LoadSpec<'_>, path: &str) -> Result<bool, Error> {
    loader::is_set(spec, path)
}

/// [`load`], moved off the async executor.
///
/// Reading configuration touches the filesystem, which would block the worker
/// it runs on. Where the work actually goes depends on what is available:
/// tokio's blocking pool with the `tokio` feature, an executor installed by
/// [`set_blocking_executor`], or a freshly spawned thread. A configuration load
/// happens at startup and on reload, so a thread per call is a real answer
/// rather than a placeholder.
///
/// `LoadSpec<'static>` is taken by value because the work outlives the call;
/// the spec the macro emits satisfies that for free.
///
/// # Errors
///
/// Same as [`load`], plus an [`ErrorKind::Backend`] error if the work never
/// produced a result — a panic inside it, or a runtime shutting down.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub async fn load_async<T>(spec: LoadSpec<'static>) -> Result<T, Error>
where
    T: DeserializeOwned + Send + 'static,
{
    off_thread(move || load(&spec)).await
}

/// Runs blocking configuration work without blocking the caller's executor.
///
/// See [`load_async`] for where the work goes.
///
/// # Errors
///
/// Whatever `work` returns, plus [`ErrorKind::Backend`] if it never produced a
/// result.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub async fn off_thread<T, F>(work: F) -> Result<T, Error>
where
    F: FnOnce() -> Result<T, Error> + Send + 'static,
    T: Send + 'static,
{
    asynchronous::off_thread(work).await
}

// ---------------------------------------------------------------------------
// Support items used by the generated code. The redirect *macros* — the
// feature-gated `__*!` wall — live in `redirects`; the functions stay here
// because they are reached by path, and a path names the module it lives in.
// ---------------------------------------------------------------------------
/// Not public API. Lets the generated code name `serde` without the caller
/// having to depend on it under that exact name.
#[doc(hidden)]
pub mod __private {
    #[cfg(feature = "clap")]
    pub use clap;
    #[cfg(feature = "schema")]
    pub use schemars;
    pub use serde;
    #[cfg(feature = "schema")]
    pub use serde_json;
}

/// Not public API.
///
/// A reload a remote watch caused. Worded to name the trigger, because a
/// program watching both files and a store wants its log to say which one
/// moved.
#[doc(hidden)]
pub fn __log_remote_reload(name: &str, summary: Option<&str>) {
    match summary {
        Some(summary) => crate::log::info!("{name}: reloaded from the remote store, {summary}"),
        None => crate::log::info!("{name}: reloaded from the remote store"),
    }
}

/// Not public API.
///
/// A document the store pushed that this program cannot use. Logged as well as
/// returned: the loop that called this has nobody to hand an error to either,
/// and a store quietly serving a configuration nothing accepts is worth a line.
#[doc(hidden)]
pub fn __log_remote_failure(name: &str, error: &Error) {
    crate::log::warning!(
        "{name}: the remote store's document did not apply, keeping the previous \
         snapshot: {error}"
    );
}

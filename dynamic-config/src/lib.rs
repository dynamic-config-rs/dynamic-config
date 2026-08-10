//! Hot-reloadable, lock-free application configuration, built on
//! [figment](https://docs.rs/figment).
//!
//! Annotate a struct, call `init()` once, and read it from anywhere:
//!
//! ```
//! # #[cfg(feature = "json")] {
//! use dynamic_config::dynamic_config;
//! use serde::Deserialize;
//!
//! #[dynamic_config(files = ["config.json"], key = "server", env = "APP_")]
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
//! ServerConfig::init().expect("defaults cover every field");
//!
//! let config = ServerConfig::current();
//! println!("{}:{}", config.host, config.port);
//! # }
//! ```
//!
//! This page is the API reference. The guide — profiles, discovery, hot
//! reload, remote stores, encryption, testing — is
//! [**the book**](https://ctolon.github.io/dynamic-config/).
//!
//! # What the attribute generates
//!
//! The everyday core:
//!
//! | Method | Description |
//! |---|---|
//! | `load() -> Result<Self, Error>` | Read the sources and deserialize. Does not touch the snapshot. |
//! | `init() -> Result<(), Error>` | `load()` plus install as the initial snapshot. Call once at startup. |
//! | `replace(Self)` | Atomically swap in a new snapshot. |
//! | `current() -> Arc<Self>` | The current snapshot. Panics before `init()`. |
//! | `try_current() -> Option<Arc<Self>>` | The current snapshot, or `None` before `init()`. |
//! | `start_watch() -> io::Result<WatchHandle>` | With `watch`: reload on file changes until the handle is dropped. A second watch while one runs is `AlreadyExists`. |
//! | `on_reload(f)` | Run a callback on every later reload, for the life of the process. |
//! | `on_reload_scoped(f) -> HookGuard` | The same, until the guard is dropped. |
//! | `set_default(path, value)` | A fallback used only when nothing else supplies the key. |
//! | `set_override(path, value)` | A value that wins over every file and variable. |
//! | `clear_defaults()` / `clear_overrides()` | Drop them again. |
//! | `load_async()` / `init_async()` | With `async`: the same, off the async executor. |
//! | `changes()` | With `async`: a handle woken by every later reload. |
//!
//! The rest of the surface — introspection (`snapshot`, `source_of`, `is_set`,
//! `check`), persistence (`save`, `save_new`, `save_encrypted`), remote stores
//! (`set_remote`, `refresh_remote`, `apply_remote`), aliases, environment
//! bindings, flags, `bind_clap`, `schema` — is in [the book's attribute
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
//! With the `async` feature and the `async` argument, configuration loads
//! without blocking the executor, and tasks can await reloads instead of
//! polling. No runtime is named anywhere: `changes()` is a `Future`, so any
//! executor drives it.
//!
//! ```ignore
//! #[dynamic_config(files = ["config.json"], key = "db", watch, async)]
//! #[derive(Debug, Deserialize)]
//! struct DbConfig { pool_size: u32 }
//!
//! DbConfig::init_async().await?;
//! // Keep the handle: dropping it stops the watch.
//! let _watch = DbConfig::start_watch()?;
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
mod cache;
mod cell;
mod check;
#[cfg(feature = "decrypt")]
mod decrypt;
mod discovery;
#[cfg(feature = "dotenv")]
mod dotenv;
mod error;
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
mod units;
mod write;

#[cfg(feature = "watch")]
#[cfg_attr(docsrs, doc(cfg(feature = "watch")))]
pub mod watch;

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
pub use cache::{CacheMode, Recovery};
pub use cell::{ConfigCell, HookGuard};
pub use check::{check, Report, Resolved, UnknownKey};
#[cfg(feature = "decrypt")]
#[cfg_attr(docsrs, doc(cfg(feature = "decrypt")))]
pub use decrypt::{has_decryptor, set_decryptor, Decryptor, Encryptor};
pub use discovery::Search;
pub use error::{Error, ErrorKind, Origin};
pub use group::{Commit, ReloadGroup, Reloadable};
pub use layer::Layer;
pub use registry::Registry;
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub use remote::AsyncRemoteSource;
pub use remote::{Fetched, Remote, RemoteSource, RemoteWatch, Watching};
pub use snapshot::{Change, ChangeKind, Snapshot};
pub use source::{Format, LoadSpec, Source, DEFAULT_NEST};
pub use units::{bytes, duration};
#[cfg(feature = "decrypt")]
#[cfg_attr(docsrs, doc(cfg(feature = "decrypt")))]
pub use write::save_encrypted;
pub use write::{save, save_new};

/// Turns a struct into a hot-reloadable configuration snapshot.
///
/// See the [crate documentation](crate) for the full guide.
///
/// # Arguments
///
/// | Argument | Form | Requires | Default |
/// |---|---|---|---|
/// | `files` | `files = ["a.toml"]` | one of `files` / `name`+`paths` | — |
/// | `name` | `name = "config"` | `paths` | — |
/// | `paths` | `paths = ["/etc/app", "."]` | `name` | — |
/// | `key` | `key = "db"` | always | — |
/// | `env` | `env = "APP_"` | | no environment layer |
/// | `nest` | `nest = "__"` | `env` | `"__"` |
/// | `allow_empty_env` | flag | `env` | off — `FOO=` is unset |
/// | `profile_env` | `profile_env = "APP_ENV"` | | no profile overlay |
/// | `watch` | flag | `watch` feature | off |
/// | `debounce` | `debounce = 250` | `watch` | 250 ms |
/// | `poll` / `poll_interval` | flag / `= 2000` | `watch` | native backend |
/// | `diff` | flag | | off |
/// | `validate` | flag | a `validate()` on the type | off |
/// | `save` | flag | `Self: Serialize` | off |
/// | `cache` | `cache = "last.json"` | | no cache — a bad start fails |
/// | `cache_mode` | `cache_mode = "redacted"` | `cache` | `"full"` |
/// | `env_files` | `env_files = [".env"]` | `dotenv` feature + `env` | none |
/// | `schema` | flag | `schema` feature + `Self: JsonSchema` | off |
/// | `async` | flag | `async` feature | off |
///
/// One field attribute: `#[config(secret)]` generates a `Debug` that prints
/// `***` for the marked fields, and forbids `#[derive(Debug)]` alongside it.
///
/// [The book's attribute reference](https://ctolon.github.io/dynamic-config/attribute-reference.html)
/// carries a section per argument, with an example and the reasoning behind
/// each default.
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
/// Writes the last configuration that worked, if one is configured. A failure
/// here is reported and swallowed: a cache that cannot be written is a worse
/// tomorrow, not a broken today.
#[doc(hidden)]
pub fn __write_cache(
    snapshot: &Snapshot,
    cache: Option<(&'static str, &'static str, &'static [&'static str])>,
) {
    let Some((path, mode, secrets)) = cache else {
        return;
    };

    let mode = CacheMode::parse(mode).unwrap_or_default();

    if let Err(error) = cache::write(snapshot, std::path::Path::new(path), mode, secrets) {
        crate::log::warning!("could not write the configuration cache to {path}: {error}");
    }
}

/// Not public API.
///
/// The last configuration that worked, when a cold start could not read the
/// real one.
///
/// # Errors
///
/// If the cache exists but cannot be read. A missing cache is `Ok(None)`.
#[doc(hidden)]
pub fn recover<T: DeserializeOwned>(
    name: &str,
    spec: &LoadSpec<'_>,
    cache: Option<(&'static str, &'static str, &'static [&'static str])>,
    failure: &Error,
) -> Result<Option<(T, Snapshot)>, Error> {
    let Some((path, mode, _)) = cache else {
        return Ok(None);
    };

    let mode = CacheMode::parse(mode).unwrap_or_default();
    let path = std::path::Path::new(path);

    // What the sources resolve to *now*, if they resolve at all — the drift
    // report needs it, and a parse failure means there is nothing to compare.
    let current = loader::snapshot(spec).ok();

    match cache::read(path, current.as_ref())? {
        Recovery::Absent => Ok(None),

        Recovery::Drift(moved) => {
            report(
                name,
                &format!(
                    "cannot start: {failure}. Since the last good configuration: {}",
                    match moved {
                        // The sources did not resolve, so there was nothing to
                        // compare against — said plainly, instead of the old
                        // claim of a value-level diff that never ran.
                        None => "could not compare — the sources do not resolve".to_owned(),
                        Some(moved) => moved.join(", "),
                    },
                ),
            );

            Ok(None)
        }

        Recovery::Usable(cached) if mode.recovers() => {
            let recovered = loader::recover::<T>(spec, &cached).map_err(|error| {
                Error::new(
                    ErrorKind::Backend,
                    format!("the cached configuration did not load either: {error}"),
                )
            })?;

            report(
                name,
                &format!("starting from the last configuration that worked, because: {failure}"),
            );

            Ok(Some(recovered))
        }

        Recovery::Usable(_) => Ok(None),
    }
}

fn report(name: &str, message: &str) {
    crate::log::warning!("{name}: {message}");
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

/// Not public API.
///
/// Renders the keys a reload changed, for the watcher to log. Returning a
/// string rather than logging keeps this out of the generated code's way and
/// leaves one log line per reload instead of two.
///
/// Always a string — "nothing to say" is not this function's case. The
/// caller's `Option` means "there was no previous snapshot to compare", and
/// that decision is made where the previous snapshot lives.
#[doc(hidden)]
#[must_use]
pub fn __summarize_changes(previous: &Snapshot, current: &Snapshot) -> String {
    let changes = previous.diff(current);

    if changes.is_empty() {
        return "no keys changed".to_owned();
    }

    changes
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

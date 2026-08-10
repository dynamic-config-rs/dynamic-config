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
//! # What the attribute generates
//!
//! | Method | Description |
//! |---|---|
//! | `load() -> Result<Self, Error>` | Read the sources and deserialize. Does not touch the snapshot. |
//! | `init() -> Result<(), Error>` | `load()` plus install as the initial snapshot. Call once at startup. |
//! | `replace(Self)` | Atomically swap in a new snapshot. |
//! | `current() -> Arc<Self>` | The current snapshot. Panics before `init()`. |
//! | `try_current() -> Option<Arc<Self>>` | The current snapshot, or `None` before `init()`. |
//! | `start_watch() -> io::Result<()>` | With `watch`: reload on file changes. Idempotent. |
//! | `load_async()` / `init_async()` | With `async`: the same, off the async executor. |
//! | `changes()` | With `async`: a handle woken by every later reload. |
//! | `set_default(path, value)` | A fallback used only when nothing else supplies the key. |
//! | `set_override(path, value)` | A value that wins over every file and variable. |
//! | `clear_defaults()` / `clear_overrides()` | Drop them again. |
//!
//! # Precedence
//!
//! ```text
//! set_default  <  config.toml  <  secrets.json  <  APP_DB_*  <  set_override
//!  (runtime)        (first)        (last file)    (environment)   (runtime)
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
//! DbConfig::start_watch()?;
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
//! | `clap` | no | `augment_command` / `from_matches`: flags as the top layer |
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
pub use cell::ConfigCell;
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
/// | `diff` | flag | `watch` | off |
/// | `validate` | flag | a `validate()` on the type | off |
/// | `save` | flag | `Self: Serialize` | off |
/// | `async` | flag | `async` feature | off |
///
/// One field attribute: `#[config(secret)]` generates a `Debug` that prints
/// `***` for the marked fields, and forbids `#[derive(Debug)]` alongside it.
///
/// The README carries a section per argument, with an example and the reasoning
/// behind each default.
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
// Redirects used by the generated code.
//
// A proc-macro cannot see which features this crate was built with, so it emits
// a call to one of these instead of naming `Format::Toml` directly. When the
// feature is off the redirect expands to a `compile_error!` that says exactly
// what to add — rather than "no variant named `Toml`", or worse, a runtime
// failure on a machine that only runs the code path in production.
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
/// Expands to `save_encrypted` when the `decrypt` feature is on, and to
/// nothing when it is not — nothing rather than a compile error, because
/// `save` alone is fully legitimate without decryption, and the method's very
/// signature names [`Encryptor`], which only exists with the feature.
///
/// It lives here rather than in the proc-macro because a `#[cfg]` emitted into
/// generated code is evaluated against the *user's* crate features; this macro
/// is expanded inside dynamic-config, where `decrypt` actually lives.
#[cfg(feature = "decrypt")]
#[macro_export]
#[doc(hidden)]
macro_rules! __save_encrypted_method {
    ($key:expr) => {
        /// Writes this configuration to `path`, encrypted.
        ///
        /// The counterpart to reading a `secrets.json.age`. The format comes
        /// from the extension *under* the `.age` suffix, so
        /// `secrets.json.age` is JSON.
        ///
        /// The encryptor is passed here rather than installed process-wide,
        /// because *who may read this file* is a decision about this write.
        ///
        /// # Errors
        ///
        /// If the name resolves to no supported format, the value cannot be
        /// serialized, encryption fails, or the file cannot be written.
        pub fn save_encrypted(
            &self,
            path: impl ::core::convert::AsRef<::std::path::Path>,
            encryptor: &dyn $crate::Encryptor,
        ) -> ::core::result::Result<(), $crate::Error> {
            let path = path.as_ref();
            let format = $crate::Format::from_path(path)?;

            $crate::save_encrypted(self, path, format, $key, encryptor)
        }
    };
}

/// Not public API.
#[cfg(not(feature = "decrypt"))]
#[macro_export]
#[doc(hidden)]
macro_rules! __save_encrypted_method {
    ($key:expr) => {};
}

/// Not public API.
///
/// Nothing when this build can read `.env` files, and a compile error naming
/// the feature when it cannot.
#[cfg(feature = "dotenv")]
#[macro_export]
#[doc(hidden)]
macro_rules! __require_dotenv {
    () => {};
}

/// Not public API.
#[cfg(not(feature = "dotenv"))]
#[macro_export]
#[doc(hidden)]
macro_rules! __require_dotenv {
    () => {
        ::core::compile_error!(
            "dynamic-config: `env_files` in #[dynamic_config(..)] requires the `dotenv` \
             feature; add features = [\"dotenv\"] to your dynamic-config dependency"
        );
    };
}

/// Not public API.
///
/// An encrypted source when this build can decrypt, and a compile error naming
/// the feature when it cannot — the same treatment a `.toml` file gets without
/// the `toml` feature, for the same reason: a silent runtime failure on the one
/// machine that has an encrypted file is worse than a build that will not start.
#[cfg(feature = "decrypt")]
#[macro_export]
#[doc(hidden)]
macro_rules! __source_encrypted {
    ($path:expr, $format:expr) => {
        $crate::Source::encrypted($path, $format)
    };
}

/// Not public API.
#[cfg(not(feature = "decrypt"))]
#[macro_export]
#[doc(hidden)]
macro_rules! __source_encrypted {
    ($path:expr, $format:expr) => {{
        ::core::compile_error!(
            "dynamic-config: a `.age` config file needs decryption support; \
             add features = [\"age\"] to your dynamic-config dependency"
        );

        $crate::Source::file($path, $format)
    }};
}

/// Not public API.
#[cfg(feature = "json")]
#[macro_export]
#[doc(hidden)]
macro_rules! __format_json {
    () => {
        $crate::Format::Json
    };
}

/// Not public API.
#[cfg(not(feature = "json"))]
#[macro_export]
#[doc(hidden)]
macro_rules! __format_json {
    () => {
        ::core::compile_error!(
            "dynamic-config: `.json` files require the `json` feature; \
             add features = [\"json\"] to your dynamic-config dependency"
        )
    };
}

/// Not public API.
#[cfg(feature = "toml")]
#[macro_export]
#[doc(hidden)]
macro_rules! __format_toml {
    () => {
        $crate::Format::Toml
    };
}

/// Not public API.
#[cfg(not(feature = "toml"))]
#[macro_export]
#[doc(hidden)]
macro_rules! __format_toml {
    () => {
        ::core::compile_error!(
            "dynamic-config: `.toml` files require the `toml` feature; \
             add features = [\"toml\"] to your dynamic-config dependency"
        )
    };
}

/// Not public API.
#[cfg(feature = "yaml")]
#[macro_export]
#[doc(hidden)]
macro_rules! __format_yaml {
    () => {
        $crate::Format::Yaml
    };
}

/// Not public API.
#[cfg(not(feature = "yaml"))]
#[macro_export]
#[doc(hidden)]
macro_rules! __format_yaml {
    () => {
        ::core::compile_error!(
            "dynamic-config: `.yaml` and `.yml` files require the `yaml` feature; \
             add features = [\"yaml\"] to your dynamic-config dependency"
        )
    };
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
) -> Result<Option<T>, Error> {
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
                    if moved.is_empty() {
                        "the same keys, with different values".to_owned()
                    } else {
                        moved.join(", ")
                    },
                ),
            );

            Ok(None)
        }

        Recovery::Usable(cached) if mode.recovers() => {
            let config = loader::recover::<T>(spec, &cached).map_err(|error| {
                Error::new(
                    ErrorKind::Backend,
                    format!("the cached configuration did not load either: {error}"),
                )
            })?;

            report(
                name,
                &format!("starting from the last configuration that worked, because: {failure}"),
            );

            Ok(Some(config))
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

/// Not public API.
#[cfg(feature = "watch")]
#[macro_export]
#[doc(hidden)]
macro_rules! __spawn_watch {
    ($($argument:tt)*) => {
        $crate::watch::spawn_with($($argument)*)
    };
}

/// Not public API.
#[cfg(not(feature = "watch"))]
#[macro_export]
#[doc(hidden)]
macro_rules! __spawn_watch {
    ($($argument:tt)*) => {
        ::core::compile_error!(
            "dynamic-config: `watch` in #[dynamic_config(..)] requires the `watch` feature; \
             add features = [\"watch\"] to your dynamic-config dependency"
        )
    };
}

/// Not public API.
///
/// Expands to `bind_clap` when the `clap` feature is on, and to nothing when it
/// is not. An item-level macro rather than an expression-level redirect,
/// because the signature names a clap type.
#[cfg(feature = "clap")]
#[macro_export]
#[doc(hidden)]
macro_rules! __clap_methods {
    () => {
        /// Copies clap arguments into the flags layer, by
        /// `(argument id, key path)`.
        ///
        /// Only arguments that came from the command line are taken: clap's own
        /// `default_value` is indistinguishable from a typed flag in
        /// `ArgMatches`, and letting one outrank a configuration file would
        /// invert the precedence order.
        ///
        /// # Errors
        ///
        /// If a key path is unusable, or an argument is not valid UTF-8.
        pub fn bind_clap(
            matches: &$crate::__private::clap::ArgMatches,
            bindings: &[(&str, &str)],
        ) -> ::core::result::Result<(), $crate::Error> {
            Self::dynamic_config_flags().bind_clap(matches, bindings)
        }
    };
}

/// Not public API.
#[cfg(not(feature = "clap"))]
#[macro_export]
#[doc(hidden)]
macro_rules! __clap_methods {
    () => {};
}

/// Not public API.
///
/// Expands to `schema` when the `schema` feature is on, and to nothing when it
/// is not. An item-level macro rather than an expression-level redirect,
/// because the `where` clause names a schemars trait.
#[cfg(feature = "schema")]
#[macro_export]
#[doc(hidden)]
macro_rules! __schema_methods {
    ($key:expr, $secrets:expr) => {
        /// A JSON Schema for the *file* this section lives in.
        ///
        /// Not the struct's schema: the struct is one section, and a config
        /// file is a map of them, so this is the struct's schema wrapped under
        /// its key. Fields marked `#[config(secret)]` carry `writeOnly`, which
        /// is how JSON Schema says *not for reading back*.
        ///
        /// Combine several with `dynamic_config::schema::merge` when more than
        /// one config type shares a file. See that module for what the schema
        /// deliberately leaves out, and for how each format wires one up.
        pub fn schema() -> ::dynamic_config::__private::serde_json::Value
        where
            Self: ::dynamic_config::__private::schemars::JsonSchema,
        {
            let generated = ::dynamic_config::__private::schemars::schema_for!(Self);

            ::dynamic_config::schema::section(
                $key,
                ::core::convert::Into::into(generated),
                $secrets,
            )
        }
    };
}

/// Not public API.
#[cfg(not(feature = "schema"))]
#[macro_export]
#[doc(hidden)]
macro_rules! __schema_methods {
    ($key:expr, $secrets:expr) => {
        ::core::compile_error!(
            "dynamic-config: `schema` in #[dynamic_config(..)] requires the `schema` feature; \
             add features = [\"schema\"] to your dynamic-config dependency"
        );
    };
}

/// Not public API.
///
/// Expands to the async half of the generated `impl`. It lives here rather than
/// in the proc-macro because these method *signatures* mention tokio types, and
/// a signature cannot be hidden behind an expression-level `compile_error!`.
#[cfg(feature = "async")]
#[macro_export]
#[doc(hidden)]
macro_rules! __async_methods {
    ($name:ident) => {
        /// Reads the configuration without blocking the async executor.
        ///
        /// # Errors
        ///
        /// Same as `load`, plus if the blocking task is cancelled.
        pub async fn load_async() -> ::core::result::Result<Self, $crate::Error> {
            $crate::load_async(Self::dynamic_config_spec()).await
        }

        /// Loads the configuration and installs it as the initial snapshot,
        /// without blocking the async executor.
        ///
        /// # Errors
        ///
        /// Same as `load_async`.
        pub async fn init_async() -> ::core::result::Result<(), $crate::Error> {
            $crate::off_thread(Self::dynamic_config_apply).await?;

            ::core::result::Result::Ok(())
        }

        /// A handle woken by every later reload.
        ///
        /// The snapshot current at this call counts as already seen, so the
        /// first `changed().await` waits for the *next* reload. Read the value
        /// you start from with `current()`.
        ///
        /// Runtime-agnostic: it is a `Future`, so tokio, async-std, smol and a
        /// hand-written executor all drive it the same way. Unlike `current()`
        /// it never panics — a handle taken before `init()` simply waits for
        /// the first snapshot.
        pub fn changes() -> $crate::Changes<Self> {
            Self::dynamic_config_cell().changes()
        }
    };
}

/// Not public API.
///
/// The async half of the remote API. Its own redirect rather than part of
/// `__async_methods!`, because a program can want an async *store* without
/// wanting the async *loading* surface — they are different axes.
#[cfg(feature = "async")]
#[macro_export]
#[doc(hidden)]
macro_rules! __async_remote_methods {
    () => {
        /// Installs an async remote store to read configuration from.
        ///
        /// Nothing is fetched here; call
        /// [`refresh_remote_async`](Self::refresh_remote_async) for that.
        /// Installing a source drops whatever the previous one had fetched.
        pub fn set_remote_async(source: impl $crate::AsyncRemoteSource) {
            Self::dynamic_config_remote().set_async(source);
        }

        /// Reads the remote store, and keeps what came back.
        ///
        /// Works with a blocking source too, so swapping one implementation for
        /// the other is not a breaking change for the caller.
        ///
        /// # Errors
        ///
        /// If no source is installed, or the fetch fails.
        pub async fn refresh_remote_async() -> ::core::result::Result<(), $crate::Error> {
            Self::dynamic_config_remote().refresh_async().await
        }
    };
}

/// Not public API.
#[cfg(not(feature = "async"))]
#[macro_export]
#[doc(hidden)]
macro_rules! __async_remote_methods {
    () => {};
}

/// Not public API.
#[cfg(not(feature = "async"))]
#[macro_export]
#[doc(hidden)]
macro_rules! __async_methods {
    ($name:ident) => {
        ::core::compile_error!(
            "dynamic-config: `async` in #[dynamic_config(..)] requires the `async` feature \
             (or `tokio`, which implies it); add features = [\"async\"] to your \
             dynamic-config dependency"
        );
    };
}

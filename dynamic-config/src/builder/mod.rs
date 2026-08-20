//! Configuring a load at runtime — the builder half of the attribute split.
//!
//! The attribute declares that a type *is* a configuration; the [`Builder`]
//! owns the "where" — chosen at runtime, not compile time — and funnels
//! into the same [`LoadSpec`] everything else reads, so the two surfaces
//! cannot drift apart on semantics.
//!
//! ```no_run
//! # #[cfg(feature = "json")] {
//! use dynamic_config::Builder;
//! use serde::Deserialize;
//!
//! #[derive(Debug, Deserialize)]
//! struct Db { host: String }
//!
//! let db: Db = Builder::new("db")
//!     .file("config.json")
//!     .env("APP_")
//!     .load()
//!     .expect("the sources read cleanly");
//! # }
//! ```
//!
//! On a `#[dynamic_config]` type, the generated `builder()` goes further:
//! its `init()` installs the result as the type's snapshot, so runtime-
//! chosen sources feed the same `current()` everything already reads.
//!
//! One concern per file: this module holds the struct, the fluent surface
//! and the one `with_spec` funnel; [`lifecycle`] loads, installs and
//! recovers; [`diagnostics`] answers questions without installing;
//! [`watching`] starts the file watcher; [`configured`] is the slot that
//! remembers a builder at `init` so the type can answer later.

mod configured;
mod diagnostics;
mod lifecycle;
#[cfg(feature = "watch")]
mod watching;

pub use configured::Configured;

use std::marker::PhantomData;
use std::path::Path;

use serde::de::DeserializeOwned;

use crate::cache::CacheMode;
use crate::error::Error;
use crate::source::{Format, LoadSpec, Source};

/// An application-level validation hook: deserialized, not yet installed.
///
/// A closure rather than a bare `fn`, because a validator that needs
/// *context* — a policy object, a schema, a foreign runtime's validator —
/// cannot be written as a function pointer, and that is the shape a
/// language binding needs. The `Arc` is what keeps `Builder` cloneable;
/// a plain `fn` still coerces, so every existing call site is unchanged.
type Validator<T> = std::sync::Arc<dyn Fn(&T) -> Result<(), Error> + Send + Sync>;

/// Where a load's outcome goes — the value on success, the news on failure.
///
/// Two known shapes rather than an `Arc<dyn Fn>`: the generated `builder()`
/// points at a `static` cell through plain `fn`s — no allocation — while a
/// [`Dynamic`](crate::Dynamic) instance owns its cell and shares it here.
///
/// The failure half is here rather than only in the caller because a failed
/// reload is a fact about the *cell*: `status()` answers "how many have
/// failed since one worked", and only the cell outlives the attempt. A
/// generated type reaches its static cell through a `fn` for the same
/// reason the install does.
pub(crate) enum Installer<T> {
    /// The generated path: `fn`s that reach the type's static cell.
    Static {
        /// Stores into the type's cell, stating why, and hands back what it
        /// stored — so `init_and_current` returns the snapshot *this* call
        /// installed rather than whatever a later reload made current.
        install: fn(T, crate::ReloadReason) -> std::sync::Arc<T>,
        /// Records a reload that installed nothing.
        record_failure: fn(&Error),
    },
    /// The instance path: this builder installs into a shared cell.
    Cell(std::sync::Arc<crate::cell::ConfigCell<T>>),
}

impl<T> Installer<T> {
    pub(super) fn install(&self, value: T, reason: crate::ReloadReason) -> std::sync::Arc<T> {
        match self {
            Self::Static { install, .. } => install(value, reason),
            Self::Cell(cell) => cell.store_with(value, reason),
        }
    }

    pub(super) fn record_failure(&self, error: &Error) {
        match self {
            Self::Static { record_failure, .. } => record_failure(error),
            Self::Cell(cell) => cell.record_failure(error),
        }
    }
}

impl<T> Clone for Installer<T> {
    fn clone(&self) -> Self {
        match self {
            Self::Static {
                install,
                record_failure,
            } => Self::Static {
                install: *install,
                record_failure: *record_failure,
            },
            Self::Cell(cell) => Self::Cell(std::sync::Arc::clone(cell)),
        }
    }
}

/// Runtime-chosen sources for one configuration section.
///
/// Methods take and return `self`, are infallible, and defer every check to
/// [`load`](Self::load) — a missing file or an unsupported extension is a
/// load-time answer, same as everywhere else in this crate.
///
/// What the builder configures in this stage is the source side: files,
/// the environment layer, `.env` files, profiles. The runtime layers
/// (`set_default`, `set_override`) and remote stores stay on the generated
/// type, whose statics they live in.
pub struct Builder<T> {
    key: String,
    files: Vec<(String, bool)>,
    env: Option<String>,
    nest: Option<String>,
    allow_empty_env: bool,
    strict_env: bool,
    whole_document: bool,
    engine: Option<&'static dyn crate::engine::Engine>,
    reader: Option<&'static dyn crate::reader::Reader>,
    env_files: Vec<String>,
    secrets_dir: Option<String>,
    allow_external_symlinks: bool,
    profile_env: Option<String>,
    search: Option<(String, Vec<String>)>,
    cache: Option<(String, CacheMode)>,
    /// `Some` routes the cache through this encryptor: written encrypted,
    /// recovered through the installed [`Decryptor`](crate::Decryptor).
    #[cfg(feature = "decrypt")]
    cache_encryptor: Option<std::sync::Arc<dyn crate::Encryptor>>,
    /// `Some` even when empty: knowing there are *no* secret fields is
    /// knowledge, and only the generated `builder()` has it.
    secrets: Option<Vec<String>>,
    validate: Option<Validator<T>>,
    fields: &'static [&'static str],
    install: Option<Installer<T>>,
    /// Remembers this builder as the type's configuration on a successful
    /// `init`, so `source_of`, `check`, `prepare` and friends can answer
    /// later without being handed the builder again.
    register: Option<fn(&Self)>,
    defaults: Option<&'static crate::Layer>,
    overrides: Option<&'static crate::Layer>,
    flags: Option<&'static crate::Layer>,
    bindings: Option<&'static crate::EnvBindings>,
    aliases: Option<&'static crate::Aliases>,
    remote: Option<&'static crate::Remote>,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Clone for Builder<T> {
    fn clone(&self) -> Self {
        Self {
            key: self.key.clone(),
            files: self.files.clone(),
            env: self.env.clone(),
            nest: self.nest.clone(),
            allow_empty_env: self.allow_empty_env,
            strict_env: self.strict_env,
            whole_document: self.whole_document,
            engine: self.engine,
            reader: self.reader,
            env_files: self.env_files.clone(),
            secrets_dir: self.secrets_dir.clone(),
            allow_external_symlinks: self.allow_external_symlinks,
            profile_env: self.profile_env.clone(),
            search: self.search.clone(),
            cache: self.cache.clone(),
            #[cfg(feature = "decrypt")]
            cache_encryptor: self.cache_encryptor.clone(),
            secrets: self.secrets.clone(),
            validate: self.validate.clone(),
            fields: self.fields,
            install: self.install.clone(),
            register: self.register,
            defaults: self.defaults,
            overrides: self.overrides,
            flags: self.flags,
            bindings: self.bindings,
            aliases: self.aliases,
            remote: self.remote,
            _marker: PhantomData,
        }
    }
}

impl<T: DeserializeOwned> Builder<T> {
    /// A builder for the section `key`, tied to no config type's storage.
    ///
    /// [`load`](Self::load) works; [`init`](Self::init) needs somewhere to
    /// install and is how the generated `builder()` differs from this.
    #[must_use]
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            files: Vec::new(),
            env: None,
            nest: None,
            allow_empty_env: false,
            strict_env: false,
            whole_document: false,
            engine: None,
            reader: None,
            env_files: Vec::new(),
            secrets_dir: None,
            allow_external_symlinks: false,
            profile_env: None,
            search: None,
            cache: None,
            #[cfg(feature = "decrypt")]
            cache_encryptor: None,
            secrets: None,
            validate: None,
            fields: &[],
            install: None,
            register: None,
            defaults: None,
            overrides: None,
            flags: None,
            bindings: None,
            aliases: None,
            remote: None,
            _marker: PhantomData,
        }
    }

    /// The generated `builder()`: everything installs into the type's cell,
    /// and every reload that installs nothing is recorded there too.
    #[doc(hidden)]
    #[must_use]
    pub fn with_installer(
        mut self,
        install: fn(T, crate::ReloadReason) -> std::sync::Arc<T>,
        record_failure: fn(&Error),
    ) -> Self {
        self.install = Some(Installer::Static {
            install,
            record_failure,
        });
        self
    }

    /// The instance path: this builder installs into `cell`. What
    /// [`Dynamic::new`](crate::Dynamic::new) wires; not public API.
    ///
    /// The registration callback is severed along with the installer: a
    /// generated builder's `register` points at the *type's* `Configured`
    /// slot, and an instance-owned builder landing there would cross-wire
    /// the type surface — `Config::reload()` installing into the
    /// `Dynamic`'s cell while `Config::current()` reads a static nothing
    /// writes.
    pub(crate) fn with_cell(mut self, cell: std::sync::Arc<crate::cell::ConfigCell<T>>) -> Self {
        self.install = Some(Installer::Cell(cell));
        self.register = None;
        self
    }

    /// The generated `builder()`: the type's `#[config(secret)]` fields, by
    /// their serde names — what a redacted cache needs to know.
    #[doc(hidden)]
    #[must_use]
    pub fn with_secrets(mut self, secrets: &[&str]) -> Self {
        self.secrets = Some(secrets.iter().map(|name| (*name).to_owned()).collect());
        self
    }

    /// Which paths hold secrets, stated by hand.
    ///
    /// `#[config(secret)]` is a *declaration*, and a configuration with no
    /// struct has nowhere to make one — so a schemaless configuration
    /// (`Builder::values`, or any bare [`Builder::new`]) starts with no
    /// secret list at all, and every surface that redacts one has nothing
    /// to redact. This is that list, supplied at the only place that knows
    /// it. It buys exactly what the attribute buys:
    ///
    /// - [`explain`](Self::explain) returns `***` for a path that is, sits
    ///   under, or contains one of these — the same three-way rule
    ///   `#[config(secret)]` gets;
    /// - [`CacheMode::Redacted`](crate::CacheMode::Redacted) and
    ///   [`Fingerprint`](crate::CacheMode::Fingerprint) become usable —
    ///   without a list they are **refused** at `init` rather than quietly
    ///   writing a cache with the secrets in it.
    ///
    /// Paths are dotted and relative to the section, as in
    /// `"credentials.password"`. Naming a table redacts everything below it.
    ///
    /// What it cannot buy is a redacting `Debug`: there is no type here to
    /// generate one for. [`Value`](crate::Value)'s own `Debug` prints shape
    /// and keys and never values, which is why that gap is a non-event.
    ///
    /// ```no_run
    /// # #[cfg(feature = "json")] {
    /// use dynamic_config::{Builder, CacheMode};
    ///
    /// let builder = Builder::values("db")
    ///     .file("config.json")
    ///     .secrets(&["password"])
    ///     .cache("last-known-good.json", CacheMode::Redacted);
    /// # let _ = builder;
    /// # }
    /// ```
    #[must_use]
    pub fn secrets(self, secrets: &[&str]) -> Self {
        self.with_secrets(secrets)
    }

    /// The generated `builder()`: the type's runtime layers and remote
    /// storage, which live in its statics.
    #[doc(hidden)]
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn with_type_statics(
        mut self,
        defaults: &'static crate::Layer,
        overrides: &'static crate::Layer,
        flags: &'static crate::Layer,
        bindings: &'static crate::EnvBindings,
        aliases: &'static crate::Aliases,
        remote: &'static crate::Remote,
        register: fn(&Self),
    ) -> Self {
        self.defaults = Some(defaults);
        self.overrides = Some(overrides);
        self.flags = Some(flags);
        self.bindings = Some(bindings);
        self.aliases = Some(aliases);
        self.remote = Some(remote);
        self.register = Some(register);
        self
    }

    /// The section key this builder reads.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Application-level validation, run after deserializing and before
    /// anything installs — on `init`, on every watch reload, and on a
    /// recovery from the cache. The reload path keeps the previous snapshot
    /// when this refuses, exactly like a parse failure.
    #[must_use]
    pub fn validate(
        mut self,
        check: impl Fn(&T) -> Result<(), Error> + Send + Sync + 'static,
    ) -> Self {
        self.validate = Some(std::sync::Arc::new(check));
        self
    }

    /// Adds a configuration file. Merged in call order; later files win.
    ///
    /// The format comes from the extension at load time. A missing file is
    /// skipped, which is what makes an optional `secrets.json` work.
    #[must_use]
    pub fn file(mut self, path: impl Into<String>) -> Self {
        self.files.push((path.into(), false));
        self
    }

    /// Adds an encrypted configuration file — `secrets.json.age`.
    ///
    /// The format comes from the extension *under* the suffix; the document
    /// decrypts through the installed [`Decryptor`](crate::Decryptor).
    #[cfg(feature = "decrypt")]
    #[cfg_attr(docsrs, doc(cfg(feature = "decrypt")))]
    #[must_use]
    pub fn encrypted_file(mut self, path: impl Into<String>) -> Self {
        self.files.push((path.into(), true));
        self
    }

    /// The environment layer: `prefix` plus the key, as in `env = "APP_"`.
    #[must_use]
    pub fn env(mut self, prefix: impl Into<String>) -> Self {
        self.env = Some(prefix.into());
        self
    }

    /// The nesting separator inside variable names; `"__"` unless said.
    #[must_use]
    pub fn nest(mut self, separator: impl Into<String>) -> Self {
        self.nest = Some(separator.into());
        self
    }

    /// Treats `FOO=` as set-to-empty rather than unset.
    #[must_use]
    pub fn allow_empty_env(mut self) -> Self {
        self.allow_empty_env = true;
        self
    }

    /// Refuses ambiguous environment spellings; see
    /// [`LoadSpec::with_strict_env`].
    #[must_use]
    pub fn strict_env(mut self) -> Self {
        self.strict_env = true;
        self
    }

    /// Folds this load's layers with `engine` rather than the installed one.
    ///
    /// Every engine that ships implements the same merge rule, so this says
    /// whose code does the folding and not what the configuration means —
    /// [`engine`](crate::engine) has the list and the two places they cannot
    /// agree.
    ///
    /// ```no_run
    /// # use serde::Deserialize;
    /// # #[derive(Deserialize)] struct Db { host: String }
    /// # #[cfg(feature = "figment")]
    /// # fn example() -> Result<(), dynamic_config::Error> {
    /// let config: Db = dynamic_config::Builder::new("db")
    ///     .file("config.toml")
    ///     .engine(dynamic_config::engine::figment())
    ///     .load()?;
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn engine(mut self, engine: &'static dyn crate::engine::Engine) -> Self {
        self.engine = Some(engine);
        self
    }

    /// Parses this load's documents with `reader`.
    ///
    /// Every reader is a different *dialect* where the formats overlap, so
    /// this is a choice with consequences — [`reader`](crate::reader) has
    /// the table and the divergences.
    #[must_use]
    pub fn reader(mut self, reader: &'static dyn crate::reader::Reader) -> Self {
        self.reader = Some(reader);
        self
    }

    /// Reads each document as this section's values, with no section header.
    ///
    /// The default is one file, several sections — every top-level key names
    /// one, and this builder's key says which is yours. That is what lets a
    /// `config.toml` carry `[db]` and `[server]` for two configuration types
    /// that know nothing about each other.
    ///
    /// Say this when the document is *only* this configuration:
    ///
    /// ```json
    /// { "host": "0.0.0.0", "port": 8000 }
    /// ```
    ///
    /// ```no_run
    /// # #[cfg(feature = "json")] {
    /// use dynamic_config::Builder;
    /// # use serde::Deserialize;
    /// # #[derive(Deserialize)]
    /// # struct Server { host: String, port: u16 }
    /// let server: Server = Builder::new("server")
    ///     .whole_document()
    ///     .file("server.json")
    ///     .env("APP_")
    ///     .load()
    ///     .expect("the sources read cleanly");
    /// # }
    /// ```
    ///
    /// The key keeps every other job it has: `APP_SERVER_PORT` still reaches
    /// `port`, the cache entry and the diagnostics are still named after it,
    /// and `""` is allowed for a configuration with nothing to call itself —
    /// the environment layer is then just the prefix, `APP_PORT`.
    ///
    /// Everything else is unchanged: profile variants
    /// (`server.production.json`), defaults, flags, overrides, aliases, the
    /// secrets directory and a remote store's document all behave exactly as
    /// they do for a sectioned load. It applies to **every** document this
    /// builder reads, because sources that disagreed about their own shape
    /// would be a configuration nobody could reason about.
    #[must_use]
    pub fn whole_document(mut self) -> Self {
        self.whole_document = true;
        self
    }

    /// A `.env` file read as the environment layer, below the real thing.
    #[must_use]
    pub fn env_file(mut self, path: impl Into<String>) -> Self {
        self.env_files.push(path.into());
        self
    }

    /// A directory of single-value files: one file per key, the filename is
    /// the key, the contents are the value.
    ///
    /// What Docker's `/run/secrets` and a Kubernetes secret volume look like.
    /// Nesting is spelled in the filename with the same separator
    /// [`nest`](Self::nest) sets, so `db__password` is `db.password`; one
    /// trailing newline is removed, because every tool that writes a secret
    /// writes one. The layer sits above the files and below `.env` and the
    /// environment — a mounted secret is a deployment fact, and a variable
    /// exported for this run is a more specific one.
    ///
    /// A directory that is not there is skipped, exactly like a missing
    /// file; one that cannot be read is a load-time error naming it.
    #[must_use]
    pub fn secrets_dir(mut self, path: impl Into<String>) -> Self {
        self.secrets_dir = Some(path.into());
        self
    }

    /// Lets a symlink in the secrets directory resolve outside it.
    ///
    /// Off by default since 0.7.1: an escaping link is refused with an
    /// error naming the entry, because a directory of mounted credentials
    /// that silently reads an arbitrary path through a planted link is a
    /// vulnerability, not a layout. Kubernetes' own `..data` indirection
    /// stays inside the mount and keeps working untouched. Turn this on
    /// only for a deliberate cross-mount arrangement — and say why in a
    /// comment, because the next reader will ask.
    #[must_use]
    pub fn allow_external_symlinks(mut self, allow: bool) -> Self {
        self.allow_external_symlinks = allow;
        self
    }

    /// The environment variable naming the active profile.
    #[must_use]
    pub fn profile_env(mut self, variable: impl Into<String>) -> Self {
        self.profile_env = Some(variable.into());
        self
    }

    /// Discovery: look for `{name}.{ext}` in each of `paths`, below any
    /// explicitly listed files — the same rule as the attribute's
    /// `name` + `paths`.
    #[must_use]
    pub fn discover(
        mut self,
        name: impl Into<String>,
        paths: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.search = Some((name.into(), paths.into_iter().map(Into::into).collect()));
        self
    }

    /// A last-known-good cache: written after every clean [`init`](Self::init)
    /// or watch reload, recovered from when the sources will not load.
    ///
    /// [`CacheMode::Redacted`] and [`CacheMode::Fingerprint`] need to know
    /// which fields are secret, which only the generated `builder()` on a
    /// `#[dynamic_config]` type carries — on a bare [`Builder::new`], those
    /// modes are refused at `init` rather than silently caching everything.
    #[must_use]
    pub fn cache(mut self, path: impl Into<String>, mode: CacheMode) -> Self {
        self.cache = Some((path.into(), mode));
        // Last writer wins outright: a plaintext cache asked for after an
        // encrypted one must not keep the encryptor and silently write a
        // full encrypted document where redaction was requested.
        #[cfg(feature = "decrypt")]
        {
            self.cache_encryptor = None;
        }
        self
    }

    /// A last-known-good cache, encrypted at rest.
    ///
    /// The fourth answer to the cache trade-off, and the one that collapses
    /// it: full fidelity — recovery needs nothing from the live environment
    /// — with nothing readable on disk. Written through `encryptor` after
    /// every clean [`init`](Self::init) or watch reload; recovered through
    /// the installed [`Decryptor`](crate::Decryptor), the same door
    /// [`encrypted_file`](Self::encrypted_file) reads through, so one
    /// `set_decryptor` covers both. The path carries the format under the
    /// encryption suffix — `last.json.age` — exactly like an encrypted
    /// source file.
    ///
    /// The recipient question that kept this out of the attribute era has
    /// the builder's answer: the recipients live in the `encryptor` the
    /// caller constructs, at the call site that owns them.
    #[cfg(feature = "decrypt")]
    #[cfg_attr(docsrs, doc(cfg(feature = "decrypt")))]
    #[must_use]
    pub fn cache_encrypted(
        mut self,
        path: impl Into<String>,
        encryptor: impl crate::Encryptor + 'static,
    ) -> Self {
        self.cache = Some((path.into(), CacheMode::Full));
        self.cache_encryptor = Some(std::sync::Arc::new(encryptor));
        self
    }

    /// The generated `builder()`: the struct's field names, for unknown-key
    /// detection in [`check`](Self::check).
    #[doc(hidden)]
    #[must_use]
    pub fn with_fields(mut self, fields: &'static [&'static str]) -> Self {
        self.fields = fields;
        self
    }

    /// Runs `operation` with the [`LoadSpec`] this builder describes.
    ///
    /// The one funnel: everything the builder does goes through the same
    /// spec the attribute generates, so the two surfaces cannot diverge.
    fn with_spec<R>(
        &self,
        operation: impl FnOnce(&LoadSpec<'_>) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let sources = self
            .files
            .iter()
            .map(|(file, encrypted)| {
                Format::from_path(Path::new(file)).map(|format| {
                    if *encrypted {
                        Source::encrypted(file, format)
                    } else {
                        Source::file(file, format)
                    }
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let env_files: Vec<&str> = self.env_files.iter().map(String::as_str).collect();

        let mut spec = LoadSpec::new(&self.key, &sources)
            .with_empty_env(self.allow_empty_env)
            .with_strict_env(self.strict_env)
            .with_whole_document(self.whole_document)
            .with_env_files(&env_files);

        if let Some(engine) = self.engine {
            spec = spec.with_engine(engine);
        }
        if let Some(reader) = self.reader {
            spec = spec.with_reader(reader);
        }
        if let Some(prefix) = &self.env {
            spec = spec.with_env(prefix);
        }
        if let Some(separator) = &self.nest {
            spec = spec.with_nest(separator);
        }
        if let Some(variable) = &self.profile_env {
            spec = spec.with_profile_env(variable);
        }
        if let Some(directory) = &self.secrets_dir {
            spec = spec.with_secrets_dir(directory);
            spec = spec.with_allow_external_symlinks(self.allow_external_symlinks);
        }

        let search_paths: Vec<&str>;
        if let Some((name, paths)) = &self.search {
            search_paths = paths.iter().map(String::as_str).collect();
            spec = spec.with_search(name, &search_paths);
        }

        if let Some(layer) = self.defaults {
            spec = spec.with_defaults(layer);
        }
        if let Some(layer) = self.overrides {
            spec = spec.with_overrides(layer);
        }
        if let Some(layer) = self.flags {
            spec = spec.with_flags(layer);
        }
        if let Some(bindings) = self.bindings {
            spec = spec.with_env_bindings(bindings);
        }
        if let Some(aliases) = self.aliases {
            spec = spec.with_aliases(aliases);
        }
        if let Some(remote) = self.remote {
            spec = spec.with_remote(remote);
        }

        operation(&spec)
    }
}

impl Builder<crate::Value> {
    /// A configuration with no struct: the resolved section as data.
    ///
    /// Sugar for `Builder::<Value>::new(key)`, and the entry point that
    /// makes the schemaless shape findable — a plugin host, a feature-flag
    /// table, a tool inspecting somebody else's configuration. Every source,
    /// layer and diagnostic on this builder behaves exactly as it does for a
    /// struct, because nothing in the engine ever needed one; what changes is
    /// the reading, which is by path.
    ///
    /// ```
    /// # #[cfg(feature = "json")] {
    /// use dynamic_config::{Builder, Dynamic};
    ///
    /// # std::fs::create_dir_all("target/doctest").unwrap();
    /// # std::fs::write("target/doctest/schemaless.json",
    /// #     r#"{"db": {"host": "localhost", "pool": {"max_size": 32}}}"#).unwrap();
    /// let config = Dynamic::new(
    ///     Builder::values("db").file("target/doctest/schemaless.json"),
    /// );
    /// let values = config.init_and_current()?;
    ///
    /// // One atomic load above; a walk of the tree here. No struct, and no
    /// // deserialize per read.
    /// assert_eq!(values.get("host").and_then(|v| v.as_str()), Some("localhost"));
    /// assert_eq!(values.get("pool.max_size").and_then(|v| v.as_i64()), Some(32));
    /// # }
    /// # Ok::<(), dynamic_config::Error>(())
    /// ```
    ///
    /// # What it does not get
    ///
    /// A struct is a *declaration*, and four things follow from it that
    /// nothing can reconstruct without one:
    ///
    /// | | With a struct | Here |
    /// |---|---|---|
    /// | Types | checked at the load | checked at each read |
    /// | Unknown keys | [`check`](Self::check) names them | reported as **not checked** |
    /// | Secrets | `#[config(secret)]` | [`secrets`](Self::secrets), by hand |
    /// | Missing required values | the load fails | absent is `None` |
    ///
    /// Everything else — layering, profiles, discovery, `.env`,
    /// `secrets_dir`, watching, the last-known-good cache, reload hooks,
    /// `source_of` and `explain` — is unchanged. The exception is not
    /// about schemas: remote stores and the runtime layers live in a
    /// `#[dynamic_config]` type's statics, so no builder made with
    /// [`new`](Self::new) or `values` reaches them, whatever `T` is.
    #[must_use]
    pub fn values(key: impl Into<String>) -> Self {
        Self::new(key)
    }
}

impl<T> std::fmt::Debug for Builder<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Builder")
            .field("key", &self.key)
            .field("files", &self.files)
            .field("env", &self.env)
            .field("env_files", &self.env_files)
            .field("strict_env", &self.strict_env)
            .field("whole_document", &self.whole_document)
            .field("installs", &self.install.is_some())
            .finish_non_exhaustive()
    }
}

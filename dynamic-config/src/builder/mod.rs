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
type Validator<T> = fn(&T) -> Result<(), Error>;

/// Where a successful load goes.
///
/// Two known shapes rather than an `Arc<dyn Fn>`: the generated `builder()`
/// points at a `static` cell through a plain `fn` — no allocation, and the
/// generated code keeps compiling unchanged — while a
/// [`Dynamic`](crate::Dynamic) instance owns its cell and shares it here.
pub(crate) enum Installer<T> {
    /// The generated path: a `fn` that stores into the type's static cell.
    Static(fn(T)),
    /// The instance path: this builder installs into a shared cell.
    Cell(std::sync::Arc<crate::cell::ConfigCell<T>>),
}

impl<T> Installer<T> {
    pub(super) fn install(&self, value: T) {
        match self {
            Self::Static(install) => install(value),
            Self::Cell(cell) => cell.store(value),
        }
    }
}

impl<T> Clone for Installer<T> {
    fn clone(&self) -> Self {
        match self {
            Self::Static(install) => Self::Static(*install),
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
    env_files: Vec<String>,
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
            env_files: self.env_files.clone(),
            profile_env: self.profile_env.clone(),
            search: self.search.clone(),
            cache: self.cache.clone(),
            #[cfg(feature = "decrypt")]
            cache_encryptor: self.cache_encryptor.clone(),
            secrets: self.secrets.clone(),
            validate: self.validate,
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
            env_files: Vec::new(),
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

    /// The generated `builder()`: everything installs into the type's cell.
    #[doc(hidden)]
    #[must_use]
    pub fn with_installer(mut self, install: fn(T)) -> Self {
        self.install = Some(Installer::Static(install));
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
    pub fn validate(mut self, check: Validator<T>) -> Self {
        self.validate = Some(check);
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

    /// A `.env` file read as the environment layer, below the real thing.
    #[must_use]
    pub fn env_file(mut self, path: impl Into<String>) -> Self {
        self.env_files.push(path.into());
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
            .with_env_files(&env_files);

        if let Some(prefix) = &self.env {
            spec = spec.with_env(prefix);
        }
        if let Some(separator) = &self.nest {
            spec = spec.with_nest(separator);
        }
        if let Some(variable) = &self.profile_env {
            spec = spec.with_profile_env(variable);
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

impl<T> std::fmt::Debug for Builder<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Builder")
            .field("key", &self.key)
            .field("files", &self.files)
            .field("env", &self.env)
            .field("env_files", &self.env_files)
            .field("strict_env", &self.strict_env)
            .field("installs", &self.install.is_some())
            .finish_non_exhaustive()
    }
}

//! Configuring a load at runtime — the builder half of the attribute split.
//!
//! The attribute declares that a type *is* a configuration; its arguments
//! also say where the configuration comes from. This module is the first
//! stage of separating those: a [`Builder`] owns the "where" — chosen at
//! runtime, not compile time — and funnels into the same [`LoadSpec`] the
//! attribute builds, so the two cannot drift apart on semantics.
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

use std::marker::PhantomData;
use std::path::Path;

use serde::de::DeserializeOwned;

use crate::cache::{CacheMode, Recovery};

/// An application-level validation hook: deserialized, not yet installed.
type Validator<T> = fn(&T) -> Result<(), Error>;
use crate::error::{Error, ErrorKind};
use crate::source::{Format, LoadSpec, Source};

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
    /// `Some` even when empty: knowing there are *no* secret fields is
    /// knowledge, and only the generated `builder()` has it.
    secrets: Option<Vec<String>>,
    validate: Option<Validator<T>>,
    fields: &'static [&'static str],
    install: Option<fn(T)>,
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
            secrets: self.secrets.clone(),
            validate: self.validate,
            fields: self.fields,
            install: self.install,
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
        self.install = Some(install);
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
        self
    }

    /// Reads the sources and deserializes, installing nothing.
    ///
    /// # Errors
    ///
    /// The same failures as any load: a file that will not parse, a missing
    /// required value, an unsupported extension.
    pub fn load(&self) -> Result<T, Error> {
        let value: T = self.with_spec(crate::loader::load)?;

        if let Some(check) = self.validate {
            check(&value)?;
        }

        Ok(value)
    }

    /// Loads and installs as the type's snapshot.
    ///
    /// # Errors
    ///
    /// Whatever [`load`](Self::load) reports — and, on a builder made with
    /// [`Builder::new`] rather than a generated `builder()`, the fact that
    /// there is no storage to install into.
    pub fn init(&self) -> Result<(), Error> {
        // Before the installer check: "your cache cannot redact" is the more
        // specific mistake, and the more dangerous one to leave unexplained.
        self.check_cache_mode()?;

        let Some(install) = self.install else {
            return Err(Error::new(
                ErrorKind::Backend,
                "this builder is tied to no config type, so there is nowhere \
                 to install; use `load()` here, or start from the generated \
                 `builder()` on a `#[dynamic_config]` type",
            ));
        };

        let outcome = match self.load() {
            Ok(value) => {
                install(value);
                self.write_cache();

                Ok(())
            }
            Err(failure) => {
                let recovered = self.recover(failure)?;

                if let Some(check) = self.validate {
                    check(&recovered)?;
                }

                install(recovered);
                crate::log::warning!(
                    "{}: started from the last known good configuration",
                    self.key
                );

                Ok(())
            }
        };

        if outcome.is_ok() {
            if let Some(register) = self.register {
                register(self);
            }
        }

        outcome
    }

    /// The first half of a grouped reload: load and validate now, install
    /// later — what [`ReloadGroup`](crate::ReloadGroup) drives.
    ///
    /// # Errors
    ///
    /// The same failures as [`load`](Self::load); a builder with no
    /// installer has nothing to commit into.
    pub fn prepare(&self) -> Result<crate::group::Commit, Error>
    where
        T: Send + 'static,
    {
        let Some(install) = self.install else {
            return Err(Error::new(
                ErrorKind::Backend,
                "this builder is tied to no config type, so a prepared \
                 commit would have nowhere to install",
            ));
        };

        let value = self.load()?;

        Ok(Box::new(move || install(value)))
    }

    /// Refuses a redaction-dependent cache mode on a builder that cannot
    /// know which fields are secret.
    fn check_cache_mode(&self) -> Result<(), Error> {
        if let Some((_, mode)) = &self.cache {
            if !matches!(mode, CacheMode::Full) && self.secrets.is_none() {
                return Err(Error::new(
                    ErrorKind::Backend,
                    "a redacted or fingerprint cache needs to know which \
                     fields are secret, and only the generated `builder()` on \
                     a `#[dynamic_config]` type knows; use that, or \
                     `CacheMode::Full`, spelled out",
                ));
            }
        }

        Ok(())
    }

    /// Best-effort, exactly like the attribute's cache: a cache that cannot
    /// be written is a worse tomorrow, not a broken today.
    fn write_cache(&self) {
        let Some((path, mode)) = &self.cache else {
            return;
        };

        // The same refusal `init` makes, as a structural belt: every path
        // that writes must hold it, not just the one that happens to run
        // the check first — a file *marked* redacted with nothing redacted
        // would be the quiet worst case.
        if !matches!(mode, CacheMode::Full) && self.secrets.is_none() {
            crate::log::warning!(
                "{}: not writing the cache at {path}: a redaction-dependent \
                 mode needs the generated builder's secret knowledge",
                self.key
            );

            return;
        }

        let secrets = self.secrets.clone().unwrap_or_default();
        let secret_refs: Vec<&str> = secrets.iter().map(String::as_str).collect();

        let written = self.with_spec(|spec| {
            let snapshot = crate::loader::snapshot(spec)?;

            crate::cache::write(&snapshot, Path::new(path), *mode, &secret_refs)
        });

        if let Err(error) = written {
            crate::log::warning!("could not write the configuration cache to {path}: {error}");
        }
    }

    /// The last known good configuration, when the sources will not load —
    /// or the original failure back, when there is nothing to recover from.
    fn recover(&self, failure: Error) -> Result<T, Error> {
        let Some((path, mode)) = &self.cache else {
            return Err(failure);
        };

        // The configured mode decides, not the file on disk: a value-bearing
        // cache left behind by an earlier deployment must not resurrect a
        // configuration the operator deliberately switched away from.
        // Fingerprint promises to diagnose and still fail.
        let may_recover = mode.recovers();

        // What the sources resolve to *now*, if they resolve at all — the
        // drift report needs it, and a parse failure means there is nothing
        // to compare.
        let current = self.with_spec(crate::loader::snapshot).ok();

        match crate::cache::read(Path::new(path), current.as_ref()) {
            // Through the loader, not a bare extract: the environment and
            // `.env` files layer over the cache exactly as they would over
            // the files, which is what lets a redacted cache work — the
            // values it dropped come back from wherever they were live.
            Ok(Recovery::Usable(snapshot)) if may_recover => self
                .with_spec(|spec| crate::loader::recover::<T>(spec, &snapshot))
                .map(|(value, _snapshot)| value),
            Ok(Recovery::Usable(_)) => {
                crate::log::warning!(
                    "{}: the cache at {path} holds values, but this builder \
                     is configured `Fingerprint`, which diagnoses and never \
                     recovers; refusing to start from it",
                    self.key
                );

                Err(failure)
            }
            // A fingerprint cannot rebuild a configuration, but it can still
            // say what moved since the last good state — the diagnosis that
            // makes the failure actionable at three in the morning.
            Ok(Recovery::Drift(moved)) => {
                crate::log::warning!(
                    "{}: cannot start: {failure}. Since the last good configuration: {}",
                    self.key,
                    match moved {
                        Some(paths) if paths.is_empty() => "nothing detectably moved".to_owned(),
                        Some(paths) => paths.join(", "),
                        None => "could not compare — the sources do not resolve".to_owned(),
                    }
                );

                Err(failure)
            }
            // A cache that will not read cures nothing: the original failure
            // is the honest answer (the cache's own trouble is logged by
            // `read` before this returns).
            Ok(Recovery::Absent) | Err(_) => Err(failure),
        }
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

    /// Explains `path` against this builder's sources; see [`crate::explain`].
    ///
    /// A builder that knows which fields are secret — every generated
    /// `builder()` does — hands back a path under one of them already
    /// redacted, the same as the type-level `explain`. A bare
    /// [`Builder::new`] knows no secrets and redacts nothing; pass the
    /// result through [`Explanation::redacted`](crate::Explanation::redacted)
    /// for a path you know to be sensitive.
    ///
    /// # Errors
    ///
    /// The same failures as [`load`](Self::load).
    pub fn explain(&self, path: &str) -> Result<crate::Explanation, Error> {
        let explanation = self.with_spec(|spec| crate::explain::explain(spec, path))?;

        // The same head-of-path check as the generated method: secrets are
        // field names, and every path under one is the secret's.
        if let Some(secrets) = &self.secrets {
            let head = path.split('.').next().unwrap_or(path);

            if secrets.iter().any(|secret| secret == head) {
                return Ok(explanation.redacted());
            }
        }

        Ok(explanation)
    }

    /// The generated `builder()`: the struct's field names, for unknown-key
    /// detection in [`check`](Self::check).
    #[doc(hidden)]
    #[must_use]
    pub fn with_fields(mut self, fields: &'static [&'static str]) -> Self {
        self.fields = fields;
        self
    }

    /// Where the value at `path` would come from, if anything supplies it.
    ///
    /// # Errors
    ///
    /// The same failures as [`load`](Self::load).
    pub fn source_of(&self, path: &str) -> Result<Option<crate::Origin>, Error> {
        self.with_spec(|spec| crate::loader::source_of(spec, path))
    }

    /// Whether anything supplies `path`.
    ///
    /// # Errors
    ///
    /// The same failures as [`load`](Self::load).
    pub fn is_set(&self, path: &str) -> Result<bool, Error> {
        self.with_spec(|spec| crate::loader::is_set(spec, path))
    }

    /// Resolves the section without deserializing it.
    ///
    /// # Errors
    ///
    /// The same failures as [`load`](Self::load).
    pub fn snapshot(&self) -> Result<crate::Snapshot, Error> {
        self.with_spec(crate::loader::snapshot)
    }

    /// What this configuration resolves to, and whether it would load —
    /// see [`check`](crate::check). Unknown-key detection uses the field
    /// names only the generated `builder()` carries; a bare builder reports
    /// none.
    ///
    /// # Errors
    ///
    /// Only if the sources cannot be read at all.
    pub fn check(&self) -> Result<crate::Report, Error> {
        self.with_spec(|spec| crate::check::<T>(spec, self.fields))
    }

    /// One reload: load, validate, install, rewrite the cache.
    ///
    /// What a watch iteration and `apply_remote` both do. A failure
    /// installs nothing — the previous snapshot keeps serving.
    ///
    /// # Errors
    ///
    /// The same failures as [`load`](Self::load); a builder with no
    /// installer has nothing to reload into.
    pub fn reload(&self) -> Result<(), Error> {
        let Some(install) = self.install else {
            return Err(Error::new(
                ErrorKind::Backend,
                "this builder is tied to no config type, so a reload would \
                 have nowhere to install",
            ));
        };

        install(self.load()?);
        self.write_cache();

        Ok(())
    }
}

/// The builder a type was configured with, remembered at `init`.
///
/// Generated code keeps one per type — a `static` or a registry slot, the
/// same split as every other slot — so `source_of`, `check`, `prepare` and
/// the remote reload can answer for "the configuration this process runs
/// on" without being handed the builder again.
#[doc(hidden)]
pub struct Configured<T> {
    builder: std::sync::Mutex<Option<Builder<T>>>,
}

// Manual, not derived: a derive would demand `T: Default`, and the generic
// path reaches this through `Registry::entry`'s `V: Default` bound — a
// config type should not have to be `Default` to be configurable.
impl<T> Default for Configured<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Configured<T> {
    /// An empty slot.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            builder: std::sync::Mutex::new(None),
        }
    }

    /// Remembers `builder` as the type's configuration.
    pub fn set(&self, builder: Builder<T>) {
        *self
            .builder
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(builder);
    }

    /// The remembered configuration, or an error that says how to get one.
    ///
    /// # Errors
    ///
    /// When nothing was configured yet.
    pub fn get(&self, name: &str) -> Result<Builder<T>, Error> {
        self.builder
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::Backend,
                    format!(
                        "`{name}` has not been configured yet; build and \
                         install one first: `{name}::builder(\"..\")\
                         .file(..).init()?`"
                    ),
                )
            })
    }
}

#[cfg(feature = "watch")]
#[cfg_attr(docsrs, doc(cfg(feature = "watch")))]
impl<T: DeserializeOwned + Send + Sync + 'static> Builder<T> {
    /// Reloads on file changes until the returned handle is dropped.
    ///
    /// The same watcher as the attribute's `watch` — same debounce, same
    /// registry: a type is watched once, whichever surface starts it, so a
    /// builder watch while `start_watch()` runs (or the reverse) is
    /// `AlreadyExists`. Each reload loads through this builder and installs
    /// into the type's snapshot, firing `on_reload` hooks and waking
    /// `changes()` exactly as any other install does; a configured
    /// [`cache`](Self::cache) is rewritten after each clean reload.
    ///
    /// # Errors
    ///
    /// As the generated `start_watch()`: no watchable directory, a backend
    /// that cannot start, or the type already being watched — plus a builder
    /// with no installer, which has nothing to reload *into*.
    pub fn watch(
        &self,
        debounce: core::time::Duration,
    ) -> std::io::Result<crate::watch::WatchHandle> {
        self.watch_with(debounce, crate::watch::WatchMode::Native)
    }

    /// [`watch`](Self::watch) with the detection strategy chosen explicitly
    /// — polling is what network and overlay filesystems need.
    ///
    /// # Errors
    ///
    /// As [`watch`](Self::watch).
    pub fn watch_with(
        &self,
        debounce: core::time::Duration,
        mode: crate::watch::WatchMode,
    ) -> std::io::Result<crate::watch::WatchHandle> {
        let Some(install) = self.install else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "this builder is tied to no config type, so a reload would \
                 have nowhere to install; start from the generated \
                 `builder()` on a `#[dynamic_config]` type",
            ));
        };

        let watched = self
            .with_spec(|spec| Ok(crate::watch::Watched::from_spec(spec)))
            .map_err(|error| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, error.to_string())
            })?;

        let reloader = self.clone();
        let name = watch_name::<T>(&self.key);

        let handle = crate::watch::spawn_with(
            std::any::TypeId::of::<T>(),
            name,
            watched,
            debounce,
            mode,
            move || {
                // `load` already validates, so a refused configuration keeps
                // the previous snapshot exactly like a parse failure.
                let value = reloader.load()?;
                install(value);
                reloader.write_cache();

                Ok(None)
            },
        )?;

        if let Some(register) = self.register {
            register(self);
        }

        Ok(handle)
    }
}

/// The registry wants a `&'static str`; leaking one per `watch()` call
/// would grow with every stop/start cycle, so the leak is memoized: one
/// name per type, ever.
#[cfg(feature = "watch")]
fn watch_name<T: 'static>(key: &str) -> &'static str {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    static NAMES: OnceLock<Mutex<HashMap<std::any::TypeId, &'static str>>> = OnceLock::new();

    let mut names = NAMES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    names
        .entry(std::any::TypeId::of::<T>())
        .or_insert_with(|| Box::leak(format!("builder:{key}").into_boxed_str()))
}

#[cfg(feature = "schema")]
#[cfg_attr(docsrs, doc(cfg(feature = "schema")))]
impl<T: DeserializeOwned> Builder<T> {
    /// A JSON Schema for the *file* this section lives in.
    ///
    /// The struct's schema wrapped under this builder's key, with
    /// `#[config(secret)]` fields carrying `writeOnly` — which the generated
    /// `builder()` knows and a bare one does not. Combine several with
    /// [`schema::merge`](crate::schema::merge) when more than one config
    /// type shares a file.
    #[must_use]
    pub fn schema(&self) -> serde_json::Value
    where
        T: schemars::JsonSchema,
    {
        let secrets = self.secrets.clone().unwrap_or_default();
        let secret_refs: Vec<&str> = secrets.iter().map(String::as_str).collect();

        crate::schema::section(&self.key, schemars::schema_for!(T).into(), &secret_refs)
    }
}

#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
impl<T: DeserializeOwned + Send + 'static> Builder<T> {
    /// [`load`](Self::load), off the async executor.
    ///
    /// # Errors
    ///
    /// The same failures as [`load`](Self::load).
    pub async fn load_async(&self) -> Result<T, Error> {
        let this = self.clone();

        crate::asynchronous::off_thread(move || this.load()).await
    }

    /// [`init`](Self::init), off the async executor.
    ///
    /// # Errors
    ///
    /// The same failures as [`init`](Self::init).
    pub async fn init_async(&self) -> Result<(), Error> {
        let this = self.clone();

        crate::asynchronous::off_thread(move || this.init()).await
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

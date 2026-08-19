//! Loading, installing, recovering: the half of the builder that commits.
//!
//! `load` stays pure — read, deserialize, validate, hand over. Everything
//! that *publishes* lives here too: `init` (and its recovery from the
//! last-known-good cache), `reload`, the grouped-commit `prepare`, and the
//! async variants that move the blocking read off the executor.

use serde::de::DeserializeOwned;

use std::path::Path;

use crate::cache::{CacheMode, Recovery};
use crate::error::{Error, ErrorKind};
use crate::reload::ReloadReason;

use super::Builder;

impl<T: DeserializeOwned> Builder<T> {
    /// Reads the sources and deserializes, installing nothing.
    ///
    /// # Errors
    ///
    /// The same failures as any load: a file that will not parse, a missing
    /// required value, an unsupported extension.
    pub fn load(&self) -> Result<T, Error> {
        let value: T = self.with_spec(crate::loader::load)?;

        if let Some(check) = &self.validate {
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
        self.install().map(|_| ())
    }

    /// [`init`](Self::init), handing back the snapshot it installed.
    ///
    /// The two calls always pair — install a configuration, then read it —
    /// and writing them apart means naming the type twice and reading the
    /// second line to learn that the first worked:
    ///
    /// ```no_run
    /// # #[cfg(feature = "json")] {
    /// # use serde::Deserialize;
    /// # #[dynamic_config::dynamic_config]
    /// # #[derive(Deserialize)]
    /// # struct ServerConfig { host: String }
    /// let config = ServerConfig::builder("server")
    ///     .file("config.json")
    ///     .init_and_current()?;
    ///
    /// println!("{}", config.host);
    /// # }
    /// # Ok::<(), dynamic_config::Error>(())
    /// ```
    ///
    /// The snapshot is the one *this* call installed. A reload landing
    /// between the install and the return would change what `current()`
    /// answers; it does not change what this returns, which is the
    /// configuration the program was started with.
    ///
    /// # Errors
    ///
    /// Exactly [`init`](Self::init)'s.
    pub fn init_and_current(&self) -> Result<std::sync::Arc<T>, Error> {
        self.install()
    }

    /// The whole of `init`, with the installed snapshot still in hand.
    fn install(&self) -> Result<std::sync::Arc<T>, Error> {
        // Before the installer check: "your cache cannot redact" is the more
        // specific mistake, and the more dangerous one to leave unexplained.
        self.check_cache_mode()?;

        let Some(install) = self.install.as_ref() else {
            return Err(Error::new(
                ErrorKind::Backend,
                "this builder is tied to no config type, so there is nowhere \
                 to install; use `load()` here, or start from the generated \
                 `builder()` on a `#[dynamic_config]` type",
            ));
        };

        let outcome = match self.load() {
            Ok(value) => {
                let installed = install.install(value, ReloadReason::Initial);
                self.write_cache();

                Ok(installed)
            }
            // Every exit from here that does not install has to say so: the
            // recovery is the one place a failed load can still succeed, so
            // "the load failed" is not yet the answer, and recording it here
            // would count a start that worked as a failure.
            Err(failure) => self
                .recover(failure)
                .and_then(|recovered| {
                    if let Some(check) = &self.validate {
                        check(&recovered)?;
                    }

                    let installed = install.install(recovered, ReloadReason::Recovered);
                    crate::log::warning!(
                        "{}: started from the last known good configuration",
                        self.key
                    );

                    Ok(installed)
                })
                .inspect_err(|error| {
                    install.record_failure(error);
                }),
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
        T: Send + Sync + 'static,
    {
        let Some(install) = self.install.as_ref() else {
            return Err(Error::new(
                ErrorKind::Backend,
                "this builder is tied to no config type, so a prepared \
                 commit would have nowhere to install",
            ));
        };

        let value = self.load().inspect_err(|error| {
            install.record_failure(error);
        })?;

        let install = install.clone();

        Ok(Box::new(move || {
            install.install(value, ReloadReason::Manual);
        }))
    }

    /// Refuses a redaction-dependent cache mode on a builder that cannot
    /// know which fields are secret.
    fn check_cache_mode(&self) -> Result<(), Error> {
        if let Some((_, mode)) = &self.cache {
            if !matches!(mode, CacheMode::Full) && self.secrets.is_none() {
                return Err(Error::new(
                    ErrorKind::Backend,
                    "a redacted or fingerprint cache needs to know which \
                     fields are secret, and nothing here has said. A \
                     `#[dynamic_config]` type's generated `builder()` says \
                     it from the declaration; a configuration with no \
                     declaration says it with `.secrets([..])` — the Python \
                     binding spells that `DynamicConfig(..., secrets=[..])`. \
                     Or ask for `CacheMode::Full`, which redacts nothing and \
                     says so — `cache(path, \"full\")` from Python",
                ));
            }
        }

        Ok(())
    }

    /// Best-effort, exactly like the attribute's cache: a cache that cannot
    /// be written is a worse tomorrow, not a broken today.
    pub(super) fn write_cache(&self) {
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
                 mode needs to know which fields are secret, and nothing \
                 has said — declare them, or pass them to `.secrets([..])`",
                self.key
            );

            return;
        }

        let secrets = self.secrets.clone().unwrap_or_default();
        let secret_refs: Vec<&str> = secrets.iter().map(String::as_str).collect();

        let written = self.with_spec(|spec| {
            let snapshot = crate::loader::snapshot(spec)?;

            #[cfg(feature = "decrypt")]
            if let Some(encryptor) = &self.cache_encryptor {
                return crate::cache::write_encrypted(
                    &snapshot,
                    Path::new(path),
                    encryptor.as_ref(),
                );
            }

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

        #[cfg(feature = "decrypt")]
        let recovered = if let Some(_encryptor) = &self.cache_encryptor {
            crate::cache::read_encrypted(Path::new(path), current.as_ref())
        } else {
            crate::cache::read(Path::new(path), current.as_ref())
        };
        #[cfg(not(feature = "decrypt"))]
        let recovered = crate::cache::read(Path::new(path), current.as_ref());

        match recovered {
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

    /// One reload: load, validate, install, rewrite the cache.
    ///
    /// What a watch iteration and a [`RemoteSink`](crate::RemoteSink)'s
    /// `apply` both do. A failure
    /// installs nothing — the previous snapshot keeps serving.
    ///
    /// # Errors
    ///
    /// The same failures as [`load`](Self::load); a builder with no
    /// installer has nothing to reload into.
    pub fn reload(&self) -> Result<(), Error> {
        self.reload_with(ReloadReason::Manual)
    }

    /// [`reload`](Self::reload), stating why.
    ///
    /// The reason reaches the reload hooks registered through
    /// `on_reload_with` and the `last_reason` in
    /// [`ConfigCell::status`](crate::ConfigCell::status). Everything else is
    /// identical — this is `reload` with the label it would otherwise have
    /// to guess. Programs that detect their own changes (a store this crate
    /// has no adapter for, a control plane pushing over a socket) have
    /// somewhere to say so; a plain `reload()` is
    /// [`Manual`](crate::ReloadReason::Manual).
    ///
    /// # Errors
    ///
    /// The same failures as [`reload`](Self::reload).
    pub fn reload_with(&self, reason: crate::ReloadReason) -> Result<(), Error> {
        let Some(install) = self.install.as_ref() else {
            return Err(Error::new(
                ErrorKind::Backend,
                "this builder is tied to no config type, so a reload would \
                 have nowhere to install",
            ));
        };

        let value = self.load().inspect_err(|error| {
            install.record_failure(error);
        })?;

        install.install(value, reason);
        self.write_cache();

        Ok(())
    }
}

#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
// `Sync` joined the bounds when the builder learned to carry a shared
// cell (`Installer::Cell` holds an `Arc<ConfigCell<T>>`, and moving the
// builder to the blocking worker moves the cell with it). A config type
// that is `Send` without `Sync` is a curiosity this crate does not chase.
impl<T: DeserializeOwned + Send + Sync + 'static> Builder<T> {
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

    /// [`init_and_current`](Self::init_and_current), off the async executor.
    ///
    /// The pair is what an async `main` writes at startup, and it is where
    /// splitting it costs most: `init_async().await?` on one line and the
    /// type named again on the next.
    ///
    /// # Errors
    ///
    /// The same failures as [`init`](Self::init).
    pub async fn init_and_current_async(&self) -> Result<std::sync::Arc<T>, Error> {
        let this = self.clone();

        crate::asynchronous::off_thread(move || this.init_and_current()).await
    }
}

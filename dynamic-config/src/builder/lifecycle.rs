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
                install.install(value);
                self.write_cache();

                Ok(())
            }
            Err(failure) => {
                let recovered = self.recover(failure)?;

                if let Some(check) = &self.validate {
                    check(&recovered)?;
                }

                install.install(recovered);
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
        T: Send + Sync + 'static,
    {
        let Some(install) = self.install.as_ref() else {
            return Err(Error::new(
                ErrorKind::Backend,
                "this builder is tied to no config type, so a prepared \
                 commit would have nowhere to install",
            ));
        };

        let value = self.load()?;

        let install = install.clone();

        Ok(Box::new(move || install.install(value)))
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
                 mode needs the generated builder's secret knowledge",
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
        let Some(install) = self.install.as_ref() else {
            return Err(Error::new(
                ErrorKind::Backend,
                "this builder is tied to no config type, so a reload would \
                 have nowhere to install",
            ));
        };

        install.install(self.load()?);
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
}

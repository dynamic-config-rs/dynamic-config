//! Starting the file watcher from a builder.
//!
//! The same watcher as the generated `start_watch()` — same debounce, same
//! one-watcher-per-type registry — with each reload loading through this
//! builder and installing into the type's snapshot.

use serde::de::DeserializeOwned;

use super::Builder;

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

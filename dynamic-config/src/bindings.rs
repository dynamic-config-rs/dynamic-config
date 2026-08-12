//! Binding one field to one environment variable, by name.
//!
//! The [`env`](crate::LoadSpec::env_prefix) layer covers the case where the
//! variable names are yours to choose: `APP_DB_POOL__MAX_SIZE` follows from the
//! prefix, the key and the field. It does not cover the case where they are
//! not.
//!
//! ```text
//! PORT                 the platform picked it — Heroku, Cloud Run, Fly
//! DATABASE_URL         a convention older than this program
//! REDIS_URL            an add-on wrote it into the environment
//! ```
//!
//! None of those can be renamed, and none of them fit a prefix. A binding says
//! *this field comes from that variable*, and nothing else changes:
//!
//! ```rust,no_run
//! # #[cfg(feature = "toml")] {
//! # use serde::Deserialize;
//! # #[dynamic_config::dynamic_config]
//! # #[derive(Deserialize)] struct ServerConfig { port: u16 }
//! ServerConfig::bind_env("port", "PORT")?;
//!
//! ServerConfig::builder("server")
//!     .file("config.toml")
//!     .env("APP_")
//!     .init()?;
//! # }
//! # Ok::<(), dynamic_config::Error>(())
//! ```
//!
//! # Where it sits
//!
//! ```text
//! defaults < files < remote < APP_SERVER_* < bindings < flags < overrides
//! ```
//!
//! Just above the prefixed environment layer, because a binding is the more
//! specific statement: somebody named this variable on purpose, and the
//! prefixed one is a convention.
//!
//! # Read at load time, not at binding time
//!
//! The variable is looked up during each `load()`, so a reload picks up a
//! change to it. Binding a variable that is not set contributes nothing — it is
//! not an error, because the whole point is that the platform may or may not
//! have set it.

use std::collections::BTreeMap;
use std::sync::Mutex;

use figment::value::{Dict, Value};
use figment::{Metadata, Profile, Provider};

use crate::error::Error;

/// The environment variables bound to fields of one configuration type.
///
/// `EnvBindings::new()` is `const`, so this lives in a `static` — which is how
/// `#[dynamic_config]` emits it.
#[derive(Debug, Default)]
pub struct EnvBindings {
    /// Key path → variable name.
    entries: Mutex<BTreeMap<String, String>>,
}

impl EnvBindings {
    /// No bindings.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Mutex::new(BTreeMap::new()),
        }
    }

    /// Binds the field at `path` to the environment variable `variable`.
    ///
    /// Takes effect on the next `load()`. Binding the same path twice replaces
    /// the first binding rather than layering it: two variables for one field
    /// would have no defensible order between them.
    ///
    /// # Errors
    ///
    /// If `path` is empty or has an empty segment — `"a..b"` names nothing.
    pub fn bind(&self, path: &str, variable: &str) -> Result<(), Error> {
        crate::layer::check_path(path)?;

        self.lock().insert(path.to_owned(), variable.to_owned());

        Ok(())
    }

    /// Drops every binding.
    pub fn clear(&self) {
        self.lock().clear();
    }

    /// Whether anything is bound.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    /// The variable bound to `path`, if any.
    #[must_use]
    pub fn variable(&self, path: &str) -> Option<String> {
        self.lock().get(path).cloned()
    }

    /// One provider per binding, so a value can be traced to the variable that
    /// supplied it rather than to "a binding" in general.
    ///
    /// figment attaches metadata per provider, not per key, so naming the
    /// variable means one provider each. There are as many as the program made
    /// bindings — a handful — and they are built once per load.
    pub(crate) fn providers(&self, key: &str, allow_empty: bool) -> Vec<BindingProvider> {
        self.lock()
            .iter()
            .map(|(path, variable)| BindingProvider {
                path: path.clone(),
                variable: variable.clone(),
                key: key.to_owned(),
                allow_empty,
            })
            .collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, String>> {
        // Recovered rather than propagated, as everywhere else in the crate:
        // the map has no invariant a panic could break.
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Prefixed onto the variable's name, so the loader can recognise this layer
/// and report the variable rather than a category.
pub(crate) const BINDING_PREFIX: &str = "the environment variable ";

/// One binding: one path, one variable.
pub(crate) struct BindingProvider {
    path: String,
    variable: String,
    key: String,
    allow_empty: bool,
}

impl Provider for BindingProvider {
    fn metadata(&self) -> Metadata {
        Metadata::named(format!("{BINDING_PREFIX}{}", self.variable))
    }

    fn data(&self) -> figment::Result<figment::value::Map<Profile, Dict>> {
        let mut values = Dict::new();

        if let Some(value) = resolve(&self.variable, self.allow_empty) {
            crate::layer::insert_path(&mut values, &self.path, value);
        }

        let mut map = figment::value::Map::new();
        map.insert(
            Profile::from(crate::loader::section_profile(&self.key)),
            values,
        );

        Ok(map)
    }
}

/// Reads one variable, or `None` if it does not usefully exist.
fn resolve(variable: &str, allow_empty: bool) -> Option<Value> {
    let text = std::env::var_os(variable)?;
    let text = text.to_str()?;

    // The same rule as the prefixed layer and `.env` files, `allow_empty_env`
    // included: an unset value rendered into a deployment template leaves
    // exactly `PORT=` (or a run of spaces), and letting that blank out a good
    // value is a bad afternoon — unless the program asked for exactly that.
    if text.trim().is_empty() && !allow_empty {
        return None;
    }

    // Parsed the way the environment layer parses: `8080` is a number, `[1,2]`
    // a list, and anything else a string.
    Some(
        text.parse::<Value>()
            .unwrap_or_else(|_| Value::from(text.to_owned())),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_with_an_empty_segment_is_refused() {
        let bindings = EnvBindings::new();

        assert!(bindings.bind("pool..max", "X").is_err());
        assert!(bindings.bind("", "X").is_err());
        assert!(bindings.bind("pool.max", "X").is_ok());
    }

    #[test]
    fn binding_the_same_path_twice_replaces_rather_than_layers() {
        let bindings = EnvBindings::new();

        bindings.bind("port", "OLD_PORT").unwrap();
        bindings.bind("port", "PORT").unwrap();

        assert_eq!(bindings.variable("port").as_deref(), Some("PORT"));
    }

    #[test]
    fn clearing_removes_everything() {
        let bindings = EnvBindings::new();

        bindings.bind("port", "PORT").unwrap();
        assert!(!bindings.is_empty());

        bindings.clear();
        assert!(bindings.is_empty());
    }
}

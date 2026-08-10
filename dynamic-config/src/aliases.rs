//! Old key paths that still work after a rename.
//!
//! `#[serde(alias = "..")]` covers a renamed *field*. It does not cover a
//! renamed *path*: a value that moved from `pool.size` to `pool.max_size`, or
//! out of one section into another, is a different key as far as the loader is
//! concerned.
//!
//! ```rust,no_run
//! # #[cfg(feature = "toml")] {
//! # use serde::Deserialize;
//! # #[dynamic_config::dynamic_config(files = ["config.toml"], key = "db")]
//! # #[derive(Deserialize)] struct DbConfig { pool: Pool }
//! # #[derive(Deserialize)] struct Pool { max_size: u16 }
//! // Files written before the rename keep working.
//! DbConfig::alias("pool.size", "pool.max_size")?;
//! # }
//! # Ok::<(), dynamic_config::Error>(())
//! ```
//!
//! # It fills a gap rather than overriding
//!
//! An alias supplies the new path **only when nothing else does**. A file that
//! has been updated wins over one that has not, whatever order they merge in,
//! and a deployment migrating one machine at a time does not get a surprise.
//!
//! # Where an aliased value traces back to
//!
//! `source_of` reports the **file that holds the old spelling**, not the alias.
//! figment attributes every path under a section to whichever provider supplied
//! the section, so the alias itself never surfaces — and that is the more useful
//! answer: it names the file to edit.
//!
//! # The old key stops being an unknown key
//!
//! Unknown-key detection exists to catch typos, and an alias that silenced it
//! would be worse than no alias: `pool.szie` would become a supported spelling.
//! So an aliased path is registered as *known* rather than ignored — `check()`
//! reports it as an alias, and anything else still shows up as a typo with a
//! suggestion.

use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::error::Error;

/// The old paths that still resolve, for one configuration type.
///
/// `Aliases::new()` is `const`, so this lives in a `static` — which is how
/// `#[dynamic_config]` emits it.
#[derive(Debug, Default)]
pub struct Aliases {
    /// Old path → current path.
    entries: Mutex<BTreeMap<String, String>>,
}

impl Aliases {
    /// No aliases.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Mutex::new(BTreeMap::new()),
        }
    }

    /// A value found at `from` also appears at `to`, if nothing supplies `to`.
    ///
    /// `from` is the old path — the one in files written before the rename —
    /// and `to` is where the field lives now.
    ///
    /// # Errors
    ///
    /// If either path names nothing, or they are the same path: an alias to
    /// itself is a loop that would never resolve to anything new.
    pub fn add(&self, from: &str, to: &str) -> Result<(), Error> {
        crate::layer::check_path(from)?;
        crate::layer::check_path(to)?;

        if from == to {
            return Err(Error::new(
                crate::ErrorKind::Type,
                format!("`{from}` cannot be an alias for itself"),
            ));
        }

        self.lock().insert(from.to_owned(), to.to_owned());

        Ok(())
    }

    /// Drops every alias.
    pub fn clear(&self) {
        self.lock().clear();
    }

    /// Whether anything is aliased.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    /// Every `(old, current)` pair, in path order.
    #[must_use]
    pub fn pairs(&self) -> Vec<(String, String)> {
        self.lock()
            .iter()
            .map(|(from, to)| (from.clone(), to.clone()))
            .collect()
    }

    /// The top-level keys an alias makes legitimate, so unknown-key detection
    /// does not report them as typos.
    #[must_use]
    pub fn known_keys(&self) -> Vec<String> {
        self.lock()
            .keys()
            .filter_map(|path| path.split('.').next().map(str::to_owned))
            .collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, String>> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_that_names_nothing_is_refused() {
        let aliases = Aliases::new();

        assert!(aliases.add("", "pool.max_size").is_err());
        assert!(aliases.add("pool..size", "pool.max_size").is_err());
        assert!(aliases.add("pool.size", "").is_err());
    }

    #[test]
    fn an_alias_to_itself_is_refused() {
        let aliases = Aliases::new();

        let error = aliases.add("pool.size", "pool.size").unwrap_err();

        assert!(error.to_string().contains("itself"), "{error}");
    }

    #[test]
    fn the_old_paths_top_level_key_counts_as_known() {
        let aliases = Aliases::new();

        aliases.add("legacy.size", "pool.max_size").unwrap();
        aliases.add("host", "hostname").unwrap();

        let known = aliases.known_keys();

        assert!(known.contains(&"legacy".to_owned()));
        assert!(known.contains(&"host".to_owned()));
    }

    #[test]
    fn aliasing_the_same_path_twice_replaces_rather_than_layers() {
        let aliases = Aliases::new();

        aliases.add("old", "first").unwrap();
        aliases.add("old", "second").unwrap();

        assert_eq!(
            aliases.pairs(),
            vec![("old".to_owned(), "second".to_owned())]
        );
    }
}

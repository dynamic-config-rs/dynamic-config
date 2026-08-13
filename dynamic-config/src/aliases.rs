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
//! # #[dynamic_config::dynamic_config]
//! # #[derive(Deserialize)] struct DbConfig { pool: Pool }
//! # #[derive(Deserialize)] struct Pool { max_size: u16 }
//! // Files written before the rename keep working.
//! DbConfig::alias("pool.size", "pool.max_size")?;
//! # }
//! # Ok::<(), dynamic_config::Error>(())
//! ```
//!
//! # A key that moved to another section
//!
//! `db::timeout` — a section, `::`, then a path inside it — is the old
//! spelling of a key that used to live in a *different* top-level section:
//!
//! ```rust,no_run
//! # #[cfg(feature = "toml")] {
//! # use serde::Deserialize;
//! # #[dynamic_config::dynamic_config]
//! # #[derive(Deserialize)] struct ServerConfig { timeout: u64 }
//! // `timeout` used to be `[db] timeout`; it is `[server] timeout` now.
//! ServerConfig::alias("db::timeout", "timeout")?;
//! # }
//! # Ok::<(), dynamic_config::Error>(())
//! ```
//!
//! **The type that owns the key today declares where it used to live**, and
//! never the other way round. A `DbConfig::moved_to("server::timeout")` would
//! be a claim on somebody else's section, resolved only if that call happened
//! to run before `ServerConfig::init()` — and in the migration this exists for,
//! the field has just been *deleted* from `DbConfig`, so there may be no type
//! left to make the claim. Declared on the destination, an alias is read from
//! the same `static` the load already consults, and reads at the call site as
//! what it is: this field used to be over there.
//!
//! Only the old path may name a section. The new path is always this
//! configuration's own, because that is the only section this type is loading.
//!
//! ## The old section is read from this configuration's own documents
//!
//! Every source this configuration lists is parsed whole — a top-level key
//! becomes a section — so the other section is already in hand and costs no
//! second read. What is *not* in hand is the other section's environment
//! layer, defaults, flags or overrides: those are built from this load's own
//! section key, and inventing a second set of them would be inventing a second
//! precedence order.
//!
//! So the boundary is a real one, and it is structural rather than
//! documentary: **two sections loaded by two builders from two file lists are
//! two configurations, not a rename.** An alias reaches into the documents
//! *this* load reads; a section that lives in a file this builder does not
//! list resolves to nothing, and the load carries on without it. The upside of
//! drawing the line there is that a watcher keeps working — the file the old
//! spelling sits in is a file this configuration already watches.
//!
//! The environment's old spelling has its own answer, and a better one:
//! `bind_env("APP_DB_TIMEOUT", "timeout")` names the variable exactly,
//! whatever section it was once built from.
//!
//! An alias that supplies nothing is **not** reported as a problem, here or in
//! the same-section case. The steady state of a *finished* migration is an
//! alias with nothing left to carry — every file has been rewritten — so a
//! report that flagged it would fire on precisely the deployments that did the
//! work, and be trained away in a week. What a live alias does is visible where
//! it matters: `check()` lists the key with the file that supplied it, and
//! `explain` names the old spelling.
//!
//! # It fills a gap rather than overriding
//!
//! An alias supplies the new path **only when nothing else does**. A file that
//! has been updated wins over one that has not, whatever order they merge in,
//! and a deployment migrating one machine at a time does not get a surprise.
//!
//! # Where an aliased value traces back to
//!
//! `source_of` reports the **file that holds the old spelling**, not the alias:
//! it names the file to edit. The alias layer carries the old path's own
//! provenance across, so the answer is the same whether the old spelling sat
//! next to the new one or in another section entirely.
//!
//! That leaves a second question a cross-section alias raises — *which*
//! spelling, over there — and `explain` is where it is answered: the alias row
//! names the old path next to the layer that supplied it, so
//! `alias db::timeout   in /etc/app.toml` says both hops.
//!
//! # The old key stops being an unknown key
//!
//! Unknown-key detection exists to catch typos, and an alias that silenced it
//! would be worse than no alias: `pool.szie` would become a supported spelling.
//! So an aliased path is registered as *known* rather than ignored — `check()`
//! reports it as an alias, and anything else still shows up as a typo with a
//! suggestion.
//!
//! A cross-section alias registers nothing, because the old key is not in this
//! section: `db` does not become a known key of `[server]`, and a stray `db`
//! table there is still a typo. The other side of it stands too — `[db]`'s own
//! `check()` goes on reporting the key left behind as unknown. That is not a
//! gap to be closed: the key really is no longer part of that section's schema,
//! and the report naming it is how a half-finished migration stays visible.
//! Silencing it would take a global side table keyed by section, consulted by a
//! type that may never have heard of the alias — action at a distance whose
//! failure mode is a typo nobody reports.

use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::error::Error;

/// What separates a section from a path inside it, in an alias's old path.
///
/// `::` and not `.`, because a dotted path already means "deeper in *this*
/// section" and one spelling cannot mean both. A reader can see at the call
/// site which aliases reach outside, and [`check_path`](crate::layer::check_path)
/// refuses `::` everywhere else so that a section qualifier in a path that
/// cannot honour one is an error rather than a key with a strange name.
pub(crate) const SECTION: &str = "::";

/// Splits `db::timeout` into `(Some("db"), "timeout")`; an unqualified path
/// into `(None, path)`.
pub(crate) fn split_section(path: &str) -> (Option<&str>, &str) {
    match path.split_once(SECTION) {
        Some((section, rest)) => (Some(section), rest),
        None => (None, path),
    }
}

/// Checks an alias's old path, which may carry one section qualifier.
///
/// A section is a *top-level* key: it has no dots and no second qualifier, so
/// `a::b::c` and `a.b::c` are refused rather than quietly meaning something.
fn check_old_path(from: &str) -> Result<(), Error> {
    let (section, path) = split_section(from);

    if let Some(section) = section {
        if section.is_empty() || section.contains('.') || path.contains(SECTION) {
            return Err(Error::new(
                crate::ErrorKind::Type,
                format!(
                    "`{from}` is not a usable old key path: `{SECTION}` names one \
                     top-level section, as in `db{SECTION}pool.size`"
                ),
            ));
        }
    }

    crate::layer::check_path(path)
}

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
    /// `from` may name another top-level section, `db::timeout`; `to` may not,
    /// because the only section this configuration loads is its own.
    ///
    /// # Errors
    ///
    /// If either path names nothing, if `to` names a section, or if they are
    /// the same path: an alias to itself is a loop that would never resolve to
    /// anything new.
    pub fn add(&self, from: &str, to: &str) -> Result<(), Error> {
        check_old_path(from)?;

        if to.contains(SECTION) {
            return Err(Error::new(
                crate::ErrorKind::Type,
                format!(
                    "`{to}` names another section, and an alias's new path is \
                     always in this configuration's own section; a section \
                     qualifier belongs on the old path, as in \
                     `alias(\"{to}\", ..)`"
                ),
            ));
        }

        crate::layer::check_path(to)?;

        if from == to {
            return Err(Error::new(
                crate::ErrorKind::Type,
                format!("`{from}` cannot be an alias for itself"),
            ));
        }

        {
            let mut entries = self.lock();

            // Chains resolve — `a → b` plus `b → c` carries a value from `a`
            // to `c`, in one deterministic pass — but a *cycle* would resolve
            // to whichever alias happened to fire first, silently. Walk the
            // chain the new edge would create; if it comes back around, the
            // rename is contradictory and the caller should hear so now.
            //
            // A section-qualified old path can only ever be the *head* of a
            // chain: `to` is never qualified, so no edge can point back at one.
            // That is what bounds a cross-section rename to a single hop —
            // by construction rather than by a depth counter.
            let mut cursor = to.to_owned();
            let mut hops = 0usize;

            while let Some(next) = entries.get(&cursor) {
                if next == from || hops > entries.len() {
                    return Err(Error::new(
                        crate::ErrorKind::Type,
                        format!(
                            "`{from}` -> `{to}` closes an alias cycle; renames \
                             must form a chain, not a loop"
                        ),
                    ));
                }

                cursor = next.clone();
                hops += 1;
            }

            entries.insert(from.to_owned(), to.to_owned());
        }

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
    ///
    /// A section-qualified old path contributes nothing: its key is in another
    /// section, so making `db` legitimate *here* would silence a genuine stray
    /// `db` table in this one.
    #[must_use]
    pub fn known_keys(&self) -> Vec<String> {
        self.lock()
            .keys()
            .filter(|path| !path.contains(SECTION))
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
    fn a_section_qualified_old_path_is_accepted() {
        let aliases = Aliases::new();

        aliases.add("db::timeout", "timeout").unwrap();

        assert_eq!(
            aliases.pairs(),
            vec![("db::timeout".to_owned(), "timeout".to_owned())]
        );
    }

    #[test]
    fn only_the_old_path_may_name_a_section() {
        let aliases = Aliases::new();

        let error = aliases.add("timeout", "server::timeout").unwrap_err();

        assert!(error.to_string().contains("own section"), "{error}");
    }

    #[test]
    fn a_qualifier_names_one_top_level_section() {
        let aliases = Aliases::new();

        assert!(aliases.add("::timeout", "timeout").is_err(), "no section");
        assert!(aliases.add("a::b::c", "timeout").is_err(), "two of them");
        assert!(aliases.add("a.b::c", "timeout").is_err(), "not top level");
        assert!(aliases.add("db::", "timeout").is_err(), "no path");
    }

    /// A qualified old path can only be the head of a chain, so the cycle walk
    /// — which follows unqualified targets — can never reach one. Pinned
    /// because it is what bounds a cross-section rename to one hop.
    #[test]
    fn a_cross_section_alias_cannot_be_chained_into() {
        let aliases = Aliases::new();

        aliases.add("db::timeout", "timeout").unwrap();
        aliases.add("timeout", "deadline").unwrap();

        let targets: Vec<String> = aliases.pairs().into_iter().map(|(_, to)| to).collect();

        assert!(
            targets.iter().all(|to| !to.contains(SECTION)),
            "nothing points at a section-qualified path: {targets:?}"
        );
    }

    #[test]
    fn a_foreign_sections_key_is_not_known_in_this_one() {
        let aliases = Aliases::new();

        aliases.add("db::timeout", "timeout").unwrap();

        assert!(
            aliases.known_keys().is_empty(),
            "`db` is another section's key, not a legitimate key here"
        );
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

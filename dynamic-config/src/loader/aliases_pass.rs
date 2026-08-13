//! The alias pass: filling gaps from old key paths after everything else has
//! merged.

use std::collections::BTreeSet;

use figment::value::Dict;
use figment::{Figment, Metadata};

use crate::layer::DEFAULTS_NAME;
use crate::source::LoadSpec;

/// Fills gaps from aliased paths.
///
/// Queried after everything else is merged, because that is the only point at
/// which "nothing supplies this" can be answered. An alias never overrides: a
/// file that has been updated wins over one that has not, whatever order they
/// merged in.
pub(super) fn apply_aliases(figment: Figment, spec: &LoadSpec<'_>) -> Figment {
    let Some(aliases) = spec.aliases else {
        return figment;
    };

    let mut figment = figment;
    let pairs = aliases.pairs();

    // Every path an alias has already filled. The loop's bound *was* "a filled
    // gap then counts as supplied" — which reads the destination's metadata,
    // and stopped being true once the alias layer began carrying its supplier's
    // metadata across: an alias fed from the defaults layer fills a path that
    // still looks like a default, is refilled, and the pass never ends. The
    // bound belongs here, where it depends on nothing but what this pass did.
    let mut filled: BTreeSet<&str> = BTreeSet::new();

    // Passes repeat until nothing new fills: a chain — `size → mid` plus
    // `mid → max_size` — resolves whatever the map's iteration order says,
    // instead of only when the alphabet happens to put the first hop first.
    // Bounded by `filled`: every productive pass adds a destination that is
    // never filled again, and there are finitely many. Cycles were refused at
    // `add` time on top of that.
    let mut progressed = true;

    while progressed {
        progressed = false;

        for (from, to) in &pairs {
            // One fill per destination: two aliases pointing at one path means
            // the first in path order wins, deterministically, as it did when
            // the second was stopped by finding the path already supplied.
            if filled.contains(to.as_str()) {
                continue;
            }

            let selected = figment.clone().select(super::section_profile(spec.key));

            // "Something supplies `to`" only counts when that something
            // outranks a runtime default. The defaults layer is the *bottom*
            // of the precedence order, and an alias exists to carry a real
            // value from a real source across a rename —
            // `set_default("pool.max_size", 8)` must not defeat the
            // not-yet-migrated file's `pool.size = 64`.
            //
            // The same rule governs a cross-section alias, deliberately: one
            // word, one meaning. A `[server] timeout` that has been written
            // beats a `[db] timeout` that has not been deleted.
            let supplied_above_defaults = selected
                .find_metadata(to)
                .is_some_and(|metadata| metadata.name != DEFAULTS_NAME);

            if supplied_above_defaults {
                continue;
            }

            // An old path may name the section it used to live in. Every
            // source this load reads is parsed whole and filed by top-level
            // key, so the other section is already here — no second
            // resolution, no second file list, and nothing to cache. What it
            // is *not* is the other section as its own type would load it:
            // the environment, defaults, flags and overrides are all built
            // from this load's key, and a second set of them would be a
            // second precedence order.
            let (section, from_path) = crate::aliases::split_section(from);
            let source = match section {
                Some(section) => figment.clone().select(super::section_profile(section)),
                None => selected,
            };

            let Ok(value) = source.find_value(from_path) else {
                continue;
            };

            let mut values = Dict::new();
            crate::layer::insert_path(&mut values, to, value);

            figment = figment.merge(Aliased {
                values,
                profile: figment::Profile::from(super::section_profile(spec.key)),
                from: from.clone(),
                // The old path's own provenance, carried across so that
                // `source_of` names the file to edit even when the alias is
                // what created the destination — a section written entirely
                // by an alias has no other provider to inherit from.
                supplier: source.find_metadata(from_path).cloned(),
            });

            filled.insert(to.as_str());
            progressed = true;
        }
    }

    figment
}

/// One aliased value, under the path the field actually has.
struct Aliased {
    values: Dict,
    profile: figment::Profile,
    from: String,
    /// The metadata of whatever supplied the old path, when it had any.
    supplier: Option<Metadata>,
}

impl figment::Provider for Aliased {
    fn metadata(&self) -> Metadata {
        self.supplier
            .clone()
            .unwrap_or_else(|| Metadata::named(format!("{ALIAS_PREFIX}{}", self.from)))
    }

    fn data(&self) -> figment::Result<figment::value::Map<figment::Profile, Dict>> {
        let mut map = figment::value::Map::new();
        map.insert(self.profile.clone(), self.values.clone());

        Ok(map)
    }
}

/// Prefixed onto the old path in this provider's metadata name.
///
/// The fallback, for the case where the old path's supplier described itself
/// with no metadata at all. Normally the alias layer answers with that
/// supplier's own metadata instead, so an aliased value traces back to the
/// *file that holds the old spelling* — the more useful answer, because it
/// names the file to edit, and the only workable one across sections, where
/// there may be no other provider under the destination to inherit from.
const ALIAS_PREFIX: &str = "an alias for ";

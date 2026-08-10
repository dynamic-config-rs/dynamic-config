//! The alias pass: filling gaps from old key paths after everything else has
//! merged.

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

    // Passes repeat until nothing new fills: a chain — `size → mid` plus
    // `mid → max_size` — resolves whatever the map's iteration order says,
    // instead of only when the alphabet happens to put the first hop first.
    // Bounded: every productive pass fills at least one gap that then counts
    // as supplied, and cycles were refused at `add` time.
    let mut progressed = true;

    while progressed {
        progressed = false;

        for (from, to) in &pairs {
            let selected = figment.clone().select(spec.key);

            // "Something supplies `to`" only counts when that something
            // outranks a runtime default. The defaults layer is the *bottom*
            // of the precedence order, and an alias exists to carry a real
            // value from a real source across a rename —
            // `set_default("pool.max_size", 8)` must not defeat the
            // not-yet-migrated file's `pool.size = 64`.
            let supplied_above_defaults = selected
                .find_metadata(to)
                .is_some_and(|metadata| metadata.name != DEFAULTS_NAME);

            if supplied_above_defaults {
                continue;
            }

            let Ok(value) = selected.find_value(from) else {
                continue;
            };

            let mut values = Dict::new();
            crate::layer::insert_path(&mut values, to, value);

            figment = figment.merge(Aliased {
                values,
                profile: figment::Profile::from(spec.key),
                from: from.clone(),
            });

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
}

impl figment::Provider for Aliased {
    fn metadata(&self) -> Metadata {
        Metadata::named(format!("{ALIAS_PREFIX}{}", self.from))
    }

    fn data(&self) -> figment::Result<figment::value::Map<figment::Profile, Dict>> {
        let mut map = figment::value::Map::new();
        map.insert(self.profile.clone(), self.values.clone());

        Ok(map)
    }
}

/// Prefixed onto the old path in this provider's metadata name.
///
/// Not something `source_of` ever reports: figment attributes every path under
/// a section to whichever provider supplied the section, so an aliased value
/// traces back to the *file that holds the old spelling*. That is the more
/// useful answer anyway — it names the file to edit — and the name here still
/// shows up in figment's own messages.
const ALIAS_PREFIX: &str = "an alias for ";

//! The alias pass: filling gaps from old key paths after everything else has
//! merged.

use std::collections::{BTreeMap, BTreeSet};

use crate::source::LoadSpec;

/// Fills gaps from aliased paths.
///
/// Queried after everything else is merged, because that is the only point at
/// which "nothing supplies this" can be answered. An alias never overrides: a
/// file that has been updated wins over one that has not, whatever order they
/// merged in.
///
/// # Errors
///
/// If the engine refuses a *sibling* section's layers. A cross-section alias
/// is the one thing here that folds anything, and a fold that fails is a load
/// that failed — not a gap left unfilled.
pub(super) fn apply_aliases(
    tree: &mut crate::resolve::Table,
    provenance: &mut BTreeMap<String, crate::Origin>,
    collected: &crate::resolve::Collected,
    spec: &LoadSpec<'_>,
) -> Result<(), crate::Error> {
    let Some(aliases) = spec.aliases else {
        return Ok(());
    };

    let pairs = aliases.pairs();

    // Every path an alias has already filled. The loop's bound *was* "a
    // filled gap then counts as supplied" — which reads the destination's
    // provenance, and stopped being true once an alias began carrying its
    // supplier's provenance across: an alias fed from the defaults layer
    // fills a path that still looks like a default, is refilled, and the
    // pass never ends. The bound belongs here, where it depends on nothing
    // but what this pass did.
    let mut filled: BTreeSet<&str> = BTreeSet::new();

    // Passes repeat until nothing new fills: a chain — `size → mid` plus
    // `mid → max_size` — resolves whatever the map's iteration order says,
    // instead of only when the alphabet happens to put the first hop first.
    // Bounded by `filled`: every productive pass adds a destination that is
    // never filled again, and there are finitely many. Cycles were refused
    // at `add` time on top of that.
    let mut progressed = true;
    let default = crate::Origin::Runtime("default");

    while progressed {
        progressed = false;

        for (from, to) in &pairs {
            // One fill per destination: two aliases pointing at one path
            // means the first in path order wins, deterministically, as it
            // did when the second was stopped by finding the path already
            // supplied.
            if filled.contains(to.as_str()) {
                continue;
            }

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
            if crate::resolve::supplied_beyond(provenance, to, &default) {
                continue;
            }

            // An old path may name the section it used to live in. Every
            // source this load reads is parsed whole, and its other sections
            // travelled with it — so the values are already here: no second
            // resolution, no second file list, and nothing to cache. What it
            // is *not* is the other section as its own type would load it:
            // the environment, defaults, flags and overrides are all built
            // from this load's key, and a second set of them would be a
            // second precedence order.
            let (section, from_path) = crate::aliases::split_section(from);

            let found = match section {
                // `?`, not `.ok()`: an engine refusing the sibling's layers
                // is a load failure, and reporting it as "the old path holds
                // nothing" would answer a question nobody asked.
                Some(section) => {
                    collected
                        .sibling(spec.engine(), section)?
                        .and_then(|(values, origins)| {
                            crate::resolve::at(&values, from_path)
                                .cloned()
                                .map(|value| (value, origins.get(from_path).cloned()))
                        })
                }
                None => crate::resolve::at(tree, from_path)
                    .cloned()
                    .map(|value| (value, provenance.get(from_path).cloned())),
            };

            let Some((value, supplier)) = found else {
                continue;
            };

            // The old path's own provenance, carried across so that
            // `source_of` names the file to edit even when the alias is what
            // created the destination — a section written entirely by an
            // alias has no other source to inherit from.
            crate::resolve::assign(
                tree,
                provenance,
                to,
                value,
                &supplier.unwrap_or(crate::Origin::Unknown),
            );

            filled.insert(to.as_str());
            progressed = true;
        }
    }

    Ok(())
}

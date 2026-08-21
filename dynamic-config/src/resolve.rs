//! Layers into one configuration, and the record of who supplied what.
//!
//! Every source produces the same thing — a tree of the values it has to
//! say about one section — and this module folds them in precedence order.
//! The fold is where provenance is *made*: a layer wins every leaf it
//! supplies, so the answer to "who set this?" is recorded as it happens
//! rather than reconstructed afterwards from whatever the merge left
//! behind.
//!
//! ```text
//! defaults   db.host = localhost   db.port = 5432
//! file       db.host = db.internal
//! env        db.port = 6543
//! ───────────────────────────────────────────────
//! result     db.host = db.internal   ← the file
//!            db.port = 6543          ← the environment
//! ```
//!
//! The merge rule is the one the crate documents everywhere else: tables
//! descend, everything else replaces, arrays included.

use std::collections::BTreeMap;

use crate::error::Origin;
use crate::value::Value;

/// A section's values: keys at the top, whatever they hold below.
pub(crate) type Table = BTreeMap<String, Value>;

/// A composed tree and the winner of every leaf in it.
pub(crate) type Resolved = (Table, BTreeMap<String, Origin>);

/// One layer's name, and what that layer alone resolves to — what an
/// explanation prints one row of.
pub(crate) type PerLayer = (&'static str, Table, BTreeMap<String, Origin>);

/// One layer's say: what it supplies, and where that came from.
///
/// A table rather than any value, because that is what a layer *is*: a
/// section is keys, and a source with nothing to say about this section
/// contributes an empty one rather than a nothing.
pub(crate) struct Contribution {
    /// The layer this came from, as the precedence order names it — what an
    /// explanation puts in its first column.
    pub(crate) layer: &'static str,
    pub(crate) origin: Origin,
    pub(crate) values: Table,
}

impl Contribution {
    pub(crate) fn new(layer: &'static str, origin: Origin, values: Table) -> Self {
        Self {
            layer,
            origin,
            values,
        }
    }
}

/// Everything one load's sources had to say.
///
/// The section being loaded, plus the other sections the same documents
/// carried — which a cross-section alias reads, and nothing else does.
#[derive(Default)]
pub(crate) struct Collected {
    pub(crate) layers: Vec<Contribution>,
    pub(crate) siblings: BTreeMap<String, Vec<Contribution>>,
}

impl Collected {
    /// One document's say: its section for us, its other sections kept
    /// beside it under the same origin.
    pub(crate) fn document(
        &mut self,
        layer: &'static str,
        origin: &Origin,
        section: Option<Table>,
        siblings: BTreeMap<String, Table>,
    ) {
        if let Some(values) = section {
            self.layers
                .push(Contribution::new(layer, origin.clone(), values));
        }

        for (name, values) in siblings {
            self.siblings
                .entry(name)
                .or_default()
                .push(Contribution::new(layer, origin.clone(), values));
        }
    }

    /// A layer that speaks only for the section being loaded.
    pub(crate) fn layer(&mut self, layer: &'static str, origin: Origin, values: Table) {
        self.layers.push(Contribution::new(layer, origin, values));
    }

    /// The layers, taken out for the fold — the siblings stay, because a
    /// cross-section alias reads them after it.
    pub(crate) fn take_layers(&mut self) -> Vec<Contribution> {
        std::mem::take(&mut self.layers)
    }

    /// Each configured layer's own say, composed and in precedence order.
    ///
    /// What an explanation walks: one row per layer that had something to
    /// say, lowest precedence first. A layer the spec does not configure
    /// contributes nothing and gets no row — a table of nine absences buries
    /// the answer.
    pub(crate) fn by_layer(
        &self,
        engine: &dyn crate::engine::Engine,
    ) -> Result<Vec<PerLayer>, crate::Error> {
        let mut grouped: Vec<(&'static str, Vec<Contribution>)> = Vec::new();

        for contribution in &self.layers {
            let restated = Contribution::new(
                contribution.layer,
                contribution.origin.clone(),
                contribution.values.clone(),
            );

            match grouped.last_mut() {
                Some((name, group)) if *name == contribution.layer => group.push(restated),
                _ => grouped.push((contribution.layer, vec![restated])),
            }
        }

        grouped
            .into_iter()
            .map(|(name, group)| {
                let (values, origins) = compose(engine, group)?;

                Ok((name, values, origins))
            })
            .collect()
    }

    /// Another section, composed — what a cross-section alias reads from,
    /// with the provenance that says which file to go and edit.
    ///
    /// `Ok(None)` is "no such section"; an engine refusing the section's
    /// layers is an error and says so. Swallowing it turned a backend
    /// failure into a missing value, and the alias pass then filled a
    /// default or reported a field nobody had failed to write — two
    /// diagnostics away from what actually happened.
    ///
    /// # Errors
    ///
    /// If the engine refuses the sibling's layers.
    pub(crate) fn sibling(
        &self,
        engine: &dyn crate::engine::Engine,
        name: &str,
    ) -> Result<Option<Resolved>, crate::Error> {
        let Some(contributions) = self.siblings.get(name) else {
            return Ok(None);
        };

        compose(
            engine,
            contributions
                .iter()
                .map(|c| Contribution::new(c.layer, c.origin.clone(), c.values.clone()))
                .collect(),
        )
        .map(Some)
    }
}

/// What the layers add up to, and who won each leaf.
pub(crate) fn compose(
    engine: &dyn crate::engine::Engine,
    contributions: Vec<Contribution>,
) -> Result<Resolved, crate::Error> {
    compose_with(engine, contributions, Fold::ShortCircuitOne)
}

/// Whether a single layer may skip the engine.
///
/// The shortcut is right for a load and wrong for the tests that *compare*
/// engines: with it, a one-layer case would compare two answers neither
/// engine produced. The corpora ask for [`Fold::Always`] so the comparison
/// stays a comparison.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Fold {
    ShortCircuitOne,
    Always,
}

/// [`compose`], with the shortcut a choice rather than a rule.
pub(crate) fn compose_with(
    engine: &dyn crate::engine::Engine,
    contributions: Vec<Contribution>,
    fold: Fold,
) -> Result<Resolved, crate::Error> {
    // The engine is handed trees and tags and nothing else, so the origins
    // stay here: a tag is this crate's index into them, and an engine that
    // reports one is naming a layer it was actually given.
    let (origins, trees): (Vec<Origin>, Vec<Value>) = contributions
        .into_iter()
        .map(|contribution| (contribution.origin, Value::Table(contribution.values)))
        .unzip();

    // **One layer folds to itself.** A fold merges layers and says who won
    // each leaf; with a single layer there is nothing to merge and the
    // winner of every leaf is that layer — an answer this crate can write
    // down without a backend, and the same answer any engine has to give,
    // which `tests/engines.rs` asserts over the whole corpus.
    //
    // Worth the branch because it is the common shape, not a corner: one
    // file and no environment, one document from a store, a test with an
    // inline source. It skips a conversion into the backend's tree, its
    // merge, and a conversion back — measured at just under half a load.
    if trees.len() == 1 && fold == Fold::ShortCircuitOne {
        let Some(Value::Table(values)) = trees.into_iter().next() else {
            return Err(not_a_table());
        };

        let mut provenance = BTreeMap::new();

        // Walked per top-level key rather than from the root, because the
        // root is not a leaf: `record` files an *empty* table as one, which
        // is right for `{"a": {}}` and would file an empty document under
        // the empty path. The engines walk it the same way, and the
        // property test beside this said so before this comment did.
        for (key, value) in &values {
            let mut path = vec![key.clone()];

            record(value, &origins[0], &mut path, &mut provenance);
        }

        return Ok((values, provenance));
    }

    let layers: Vec<crate::engine::Layer<'_>> = trees
        .iter()
        .enumerate()
        .map(|(tag, values)| crate::engine::Layer { tag, values })
        .collect();

    let folded = engine.fold(&layers)?;

    let Value::Table(values) = folded.values else {
        return Err(not_a_table());
    };

    let mut provenance: BTreeMap<String, Origin> = folded
        .tags
        .into_iter()
        .filter_map(|(path, tag)| origins.get(tag).map(|origin| (path, origin.clone())))
        .collect();

    backfill(&values, &trees, &origins, &mut provenance);

    Ok((values, provenance))
}

/// Answers for the leaves the engine did not.
///
/// An engine reports what it knows, and a backend can know less than this
/// crate promises — one of them loses the origin of an empty table while
/// merging it, and a third-party engine may report nothing at all. The gap
/// is filled from the same layers in the same order rather than left as
/// `Unknown`, and it cannot contradict the engine: under the merge rule
/// every engine implements, the winner of a leaf *is* the highest layer
/// that supplies that path, so this reads the answer off the input rather
/// than guessing at it.
fn backfill(
    values: &Table,
    trees: &[Value],
    origins: &[Origin],
    provenance: &mut BTreeMap<String, Origin>,
) {
    // Walked rather than cloned: this runs on every load, including every
    // reload, and the question is only which paths have no answer yet.
    let missing: std::collections::BTreeSet<String> = crate::value::leaf_paths_of(values)
        .into_iter()
        .filter(|leaf| !provenance.contains_key(leaf))
        .collect();

    // The ordinary case, and the one worth being quick: an engine that
    // reported every leaf leaves nothing to fill, and this crate's own
    // always does.
    if missing.is_empty() {
        return;
    }

    // Matched on the same dotted spelling the folded tree produced, rather
    // than by walking a path: a key may itself contain a dot, and a lookup
    // that split on one would miss exactly the leaf being asked about.
    //
    // Lowest layer first, so a higher one overwrites: the winner of a leaf
    // is the highest layer that supplies it.
    for (index, tree) in trees.iter().enumerate() {
        let Some(origin) = origins.get(index) else {
            continue;
        };

        let Value::Table(table) = tree else {
            continue;
        };

        for supplied in crate::value::leaf_paths_of(table) {
            if missing.contains(&supplied) {
                provenance.insert(supplied, origin.clone());
            }
        }
    }
}

/// The value at a dotted path, if anything is there.
pub(crate) fn at<'a>(tree: &'a Table, path: &str) -> Option<&'a Value> {
    let mut segments = path.split('.');
    let mut current = tree.get(segments.next()?)?;

    for segment in segments {
        let Value::Table(nested) = current else {
            return None;
        };

        current = nested.get(segment)?;
    }

    Some(current)
}

/// Whether anything at or under `path` was supplied by a layer other than
/// the one named.
///
/// The question an alias asks before it fills: a destination the defaults
/// layer reached is still a gap, because an alias exists to carry a real
/// value across a rename and a default is what it carries it *over*.
pub(crate) fn supplied_beyond(
    provenance: &BTreeMap<String, Origin>,
    path: &str,
    ignoring: &Origin,
) -> bool {
    let below = format!("{path}.");

    provenance
        .iter()
        .any(|(known, origin)| (known == path || known.starts_with(&below)) && origin != ignoring)
}

/// Puts `value` at `path` and records `origin` for every leaf it brings.
pub(crate) fn assign(
    tree: &mut Table,
    provenance: &mut BTreeMap<String, Origin>,
    path: &str,
    value: Value,
    origin: &Origin,
) {
    let mut walked: Vec<String> = path.split('.').map(str::to_owned).collect();

    forget(&walked, provenance);
    record(&value, origin, &mut walked, provenance);
    crate::layer::insert_path(tree, path, value);
}

/// What an engine answering with something other than a table is told.
fn not_a_table() -> crate::Error {
    crate::Error::new(
        crate::ErrorKind::Backend,
        "the resolution engine answered with something that is not a table",
    )
}

/// Records `origin` for every leaf inside `value`, at `path` and below.
fn record(
    value: &Value,
    origin: &Origin,
    path: &mut Vec<String>,
    provenance: &mut BTreeMap<String, Origin>,
) {
    match value {
        // A table with keys is a step on the way to a leaf, never a leaf
        // itself. An *empty* one is the exception and is recorded as a leaf,
        // because that is what it is to every other reader here: a path a
        // layer supplied, holding nothing.
        Value::Table(table) if !table.is_empty() => {
            for (key, nested) in table {
                path.push(key.clone());
                record(nested, origin, path, provenance);
                path.pop();
            }
        }
        _ => {
            provenance.insert(path.join("."), origin.clone());
        }
    }
}

/// Drops what was known about `path` and everything under it.
fn forget(path: &[String], provenance: &mut BTreeMap<String, Origin>) {
    if path.is_empty() {
        provenance.clear();

        return;
    }

    let here = path.join(".");
    let below = format!("{here}.");

    provenance.retain(|known, _| *known != here && !known.starts_with(&below));
}

#[cfg(test)]
mod tests {
    use super::{compose, compose_with, Contribution};
    use crate::error::Origin;
    use crate::value::Value;
    use proptest::prelude::*;

    fn table(pairs: &[(&str, Value)]) -> super::Table {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect()
    }

    /// The same, as a value — for the shapes nested inside a layer.
    fn nested(pairs: &[(&str, Value)]) -> Value {
        Value::Table(table(pairs))
    }

    fn origin(name: &str) -> Origin {
        Origin::Env(name.to_owned())
    }

    /// The native fold, for the cases that are about the fold's own rules
    /// rather than about which engine ran it.
    /// The default engine, for the cases that are about the fold's own
    /// rules rather than about which engine ran it.
    fn compose_in_test(contributions: Vec<Contribution>) -> super::Resolved {
        compose(crate::engine::default(), contributions).expect("the layers fold")
    }

    /// The shapes a resolved tree actually takes.
    fn trees() -> impl Strategy<Value = Value> {
        let leaf = prop_oneof![
            Just(Value::Null),
            any::<bool>().prop_map(Value::Bool),
            (0i64..8).prop_map(|number| Value::Integer(i128::from(number))),
            "[a-c]{1,3}".prop_map(Value::String),
            prop::collection::vec((0i64..4).prop_map(|n| Value::Integer(i128::from(n))), 0..3)
                .prop_map(Value::Array),
        ];

        leaf.prop_recursive(3, 12, 3, |inner| {
            prop::collection::btree_map("[a-c]", inner, 0..3).prop_map(Value::Table)
        })
    }

    /// A layer is a section's keys, which is what the fold is given.
    fn layers() -> impl Strategy<Value = Vec<(String, super::Table)>> {
        prop::collection::vec(
            (
                "layer[0-9]",
                prop::collection::btree_map("[a-c]", trees(), 0..3),
            ),
            1..5,
        )
    }

    proptest! {
        /// **What the one-layer shortcut owes.** `compose` answers a single
        /// layer itself rather than folding it, because a fold with nothing
        /// to merge is the layer and the winner of every leaf is the layer
        /// that supplied it. That is an *equivalence*, so it is asserted:
        /// the shortcut and every engine must agree on the tree and on the
        /// provenance, or the fast path means something the slow one does
        /// not.
        #[test]
        fn one_layer_answers_the_same_with_the_shortcut_and_with_an_engine(
            values in trees()
        ) {
            let table = match values {
                Value::Table(table) => table,
                other => super::Table::from([("a".to_owned(), other)]),
            };

            let short = compose_in_test(vec![Contribution::new(
                "test",
                origin("only"),
                table.clone(),
            )]);

            for engine in crate::engine::all() {
                let folded = compose_with(
                    engine,
                    vec![Contribution::new("test", origin("only"), table.clone())],
                    super::Fold::Always,
                )
                .expect("the layer folds");

                prop_assert_eq!(
                    &short.0, &folded.0,
                    "the shortcut and {} disagree on the tree", engine.name()
                );
                prop_assert_eq!(
                    &short.1, &folded.1,
                    "the shortcut and {} disagree on who won", engine.name()
                );
            }
        }

        /// **The property the engine seam rests on.** Same layers, same
        /// order, every engine: the tree *and* the winner of every leaf must
        /// come out the same, or which engine is installed is a question
        /// about what a configuration means — and this crate promises it is
        /// not.
        #[test]
        fn every_engine_folds_the_same_layers_the_same_way(layers in layers()) {
            let mut answers = Vec::new();

            for engine in crate::engine::all() {
                let contributions: Vec<_> = layers
                    .iter()
                    .map(|(layer, values)| {
                        Contribution::new("test", origin(layer), values.clone())
                    })
                    .collect();

                answers.push((
                    engine.name(),
                    compose_with(engine, contributions, super::Fold::Always)
                        .expect("the layers fold"),
                ));
            }

            let (reference_name, reference) = &answers[0];

            for (name, answer) in &answers[1..] {
                prop_assert_eq!(
                    &answer.0, &reference.0,
                    "{} and {} disagree on the tree", reference_name, name
                );
                prop_assert_eq!(
                    &answer.1, &reference.1,
                    "{} and {} disagree on who won", reference_name, name
                );
            }
        }
    }
}

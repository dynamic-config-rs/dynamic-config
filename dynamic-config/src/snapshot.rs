//! Comparing one resolved configuration against another.
//!
//! "Configuration reloaded" is a nearly useless log line during an incident:
//! the question is always *what* changed. A snapshot is the resolved section
//! before it becomes a struct, so two of them can be compared key by key.
//!
//! **Only paths are reported, never values.** That keeps a reload of
//! `db.password` from doing in the log exactly what `#[config(secret)]` exists
//! to prevent. Code that needs the values already has both sides in an
//! `on_reload` callback.

use std::collections::BTreeMap;
use std::fmt;

use figment::value::{Dict, Value};
use serde::de::DeserializeOwned;

use crate::error::{Error, ErrorKind, Origin};

/// What happened to one key between two snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ChangeKind {
    /// Nothing supplied it before.
    Added,
    /// Nothing supplies it any more.
    Removed,
    /// It has a different value.
    Modified,
}

impl fmt::Display for ChangeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Added => "added",
            Self::Removed => "removed",
            Self::Modified => "changed",
        })
    }
}

/// One difference between two snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// Dotted key path, relative to the section.
    pub path: String,
    /// What happened to it.
    pub kind: ChangeKind,
}

impl fmt::Display for Change {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.path, self.kind)
    }
}

/// A resolved configuration section, before it becomes a struct.
///
/// Obtained from [`snapshot`](crate::snapshot), compared with
/// [`diff`](Self::diff), and turned into a struct with
/// [`extract`](Self::extract).
#[derive(Clone, Default)]
pub struct Snapshot {
    values: Dict,
    /// Where each leaf came from, captured while the figment that knew was
    /// still alive. Empty for a snapshot that was not produced by a live
    /// resolution — one read back from the cache, for instance.
    provenance: BTreeMap<String, Origin>,
}

impl Snapshot {
    pub(crate) fn new(values: Dict) -> Self {
        Self {
            values,
            provenance: BTreeMap::new(),
        }
    }

    /// Attaches where each leaf came from, at resolution time.
    pub(crate) fn attach_provenance(&mut self, provenance: BTreeMap<String, Origin>) {
        self.provenance = provenance;
    }

    /// Where the value at `path` in **this snapshot** came from.
    ///
    /// This answers for the snapshot in hand — the values that were actually
    /// resolved together. The free-standing
    /// [`source_of`](crate::source_of) answers a different question: what the
    /// *next* load would see, re-reading the sources now.
    ///
    /// `None` when nothing supplies `path`, and for snapshots that did not
    /// come from a live resolution (a cache read, a [`sub`](Self::sub) of
    /// one of those): provenance is captured at resolution time and cannot
    /// be reconstructed later.
    #[must_use]
    pub fn source_of(&self, path: &str) -> Option<&Origin> {
        self.provenance.get(path)
    }

    /// The resolved section as an owned [`crate::Value`] tree.
    ///
    /// For the boundary that needs the configuration as *data* rather than
    /// a type — a language binding, an exporter. Built by walking the
    /// resolved tree directly, never through a serialized intermediate;
    /// like [`extract`](Self::extract), it hands over real values, secrets
    /// included — the paths-only rule governs what this crate *prints*.
    #[must_use]
    pub fn to_value(&self) -> crate::Value {
        crate::value::Value::Table(
            self.values
                .iter()
                .map(|(key, value)| (key.clone(), crate::value::from_figment(value)))
                .collect(),
        )
    }

    /// Deserializes the section into `T`.
    ///
    /// # Errors
    ///
    /// If a required value is missing or cannot become the field's type.
    ///
    /// Errors here carry the key path but **not** the originating file or
    /// variable: provenance lives in figment's metadata, which a snapshot has
    /// already left behind. Call [`load`](crate::load) when the error is going
    /// to be read by a person.
    pub fn extract<T: DeserializeOwned>(&self) -> Result<T, Error> {
        Value::from(self.values.clone())
            .deserialize()
            // Through the loader's translation, which strips the offending
            // value out of the backend's message. This used to render
            // `error.to_string()`, and figment renders a type mismatch as
            // ``found string "hunter2"`` — the leak the rest of the crate is
            // built to prevent, on the door that skips the loader.
            .map_err(|error: figment::Error| crate::loader::translate(&error))
    }

    /// Every key that differs between `self` and `other`, in path order.
    ///
    /// `self` is the earlier snapshot, so a key present only in `other` is
    /// [`Added`](ChangeKind::Added).
    #[must_use]
    pub fn diff(&self, other: &Self) -> Vec<Change> {
        let mut changes = Vec::new();

        compare(&self.values, &other.values, &mut Vec::new(), &mut changes);
        changes.sort_by(|left, right| left.path.cmp(&right.path));

        changes
    }

    /// Reads one value by dotted path, without a struct to hold it.
    ///
    /// For the shape a program does not know at compile time: a plugin's
    /// section, a user-defined table. Everything else should go through a
    /// struct, where a typo is a compile error rather than a runtime one.
    ///
    /// **This deserializes on every call**, which a `current()` on a struct
    /// does not — it is a diagnostic-grade read, not a request-path one. A
    /// schemaless configuration that wants the cheap door holds the resolved
    /// tree instead and walks it: `Dynamic<Value>` plus
    /// [`Value::get`](crate::Value::get). The book's schemaless chapter
    /// prints both numbers.
    ///
    /// # Errors
    ///
    /// If nothing supplies `path`, or the value cannot become `T`. The
    /// message names the path and the kind of thing that was there, never
    /// the value.
    pub fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, Error> {
        let value = self.at(path).ok_or_else(|| {
            Error::new(ErrorKind::Missing, "no value at this path").prepend_key(path)
        })?;

        // See `extract`: the translation is what keeps a mistyped secret out
        // of the message.
        value
            .deserialize()
            .map_err(|error: figment::Error| crate::loader::translate(&error).prepend_key(path))
    }

    /// This snapshot minus one top-level key — how the cache strips its own
    /// marker before the values are handed back as configuration.
    pub(crate) fn without_top_level(&self, key: &str) -> Self {
        let mut values = self.values().clone();
        values.remove(key);

        let prefix = format!("{key}.");
        let provenance = self
            .provenance
            .iter()
            .filter(|(path, _)| *path != key && !path.starts_with(&prefix))
            .map(|(path, origin)| (path.clone(), origin.clone()))
            .collect();

        Self { values, provenance }
    }

    /// Whether anything supplies `path`.
    #[must_use]
    pub fn contains(&self, path: &str) -> bool {
        self.at(path).is_some()
    }

    /// The table at `path`, as a snapshot of its own.
    ///
    /// Hands a subsystem the part of the configuration it owns and nothing
    /// else — the pool's settings without the credentials beside them.
    /// Provenance follows: the sub-snapshot's
    /// [`source_of`](Self::source_of) answers for its own, re-rooted paths.
    #[must_use]
    pub fn sub(&self, path: &str) -> Option<Self> {
        match self.at(path)? {
            Value::Dict(_, nested) => {
                let prefix = format!("{path}.");
                let provenance = self
                    .provenance
                    .iter()
                    .filter_map(|(leaf, origin)| {
                        leaf.strip_prefix(&prefix)
                            .map(|rest| (rest.to_owned(), origin.clone()))
                    })
                    .collect();

                Some(Self {
                    values: nested.clone(),
                    provenance,
                })
            }
            _ => None,
        }
    }

    fn at(&self, path: &str) -> Option<&Value> {
        let mut segments = path.split('.');
        let mut current = self.values.get(segments.next()?)?;

        for segment in segments {
            let Value::Dict(_, nested) = current else {
                return None;
            };

            current = nested.get(segment)?;
        }

        Some(current)
    }

    /// The dotted path of every leaf, in order.
    #[must_use]
    pub fn leaf_paths(&self) -> Vec<String> {
        let mut paths = Vec::new();

        collect_leaves(&self.values, &mut Vec::new(), &mut paths);

        paths
    }

    /// The section's immediate keys — the level a struct's fields map onto.
    #[must_use]
    pub fn top_level_keys(&self) -> Vec<String> {
        self.values.keys().cloned().collect()
    }

    /// The resolved tree, for the few places that need it whole.
    pub(crate) fn values(&self) -> &Dict {
        &self.values
    }

    /// Whether the section resolved to nothing at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

/// Keys and shape only, never values: a snapshot holds the *resolved*
/// configuration, secrets included, and `{:?}` in a log line is exactly how
/// resolved secrets leak. The dropped `#[derive(Debug)]` is the mistake
/// AGENTS.md warns about, made by this crate itself.
impl fmt::Debug for Snapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Snapshot")
            .field("keys", &self.top_level_keys())
            .field("leaves", &self.leaf_paths().len())
            .field("provenance", &self.provenance.len())
            .finish_non_exhaustive()
    }
}

/// The dotted paths that differ between two configuration values.
///
/// The audit half of a reload hook: `on_reload` hands over both structs, and
/// this names what moved — paths only, never values, same as every other
/// diagnostic here.
///
/// ```
/// # use serde::Serialize;
/// #[derive(Serialize)]
/// struct Db { host: String, port: u16 }
///
/// let before = Db { host: "a".into(), port: 1 };
/// let after = Db { host: "a".into(), port: 2 };
///
/// let changes = dynamic_config::changed_paths(&before, &after).unwrap();
/// assert_eq!(changes.len(), 1);
/// assert_eq!(changes[0].path, "port");
/// ```
///
/// # Errors
///
/// If either value does not serialize to a table — a bare scalar has no
/// paths to compare.
pub fn changed_paths<T: serde::Serialize>(previous: &T, current: &T) -> Result<Vec<Change>, Error> {
    let as_snapshot = |value: &T| -> Result<Snapshot, Error> {
        match Value::serialize(value) {
            Ok(Value::Dict(_, dict)) => Ok(Snapshot::new(dict)),
            Ok(_) => Err(Error::new(
                ErrorKind::Type,
                "only a table has paths to compare; this serializes to a scalar",
            )),
            Err(error) => Err(Error::new(ErrorKind::Type, error.to_string())),
        }
    };

    Ok(as_snapshot(previous)?.diff(&as_snapshot(current)?))
}

/// Records the dotted path of every leaf, treating an empty table as one.
fn collect_leaves(values: &Dict, path: &mut Vec<String>, paths: &mut Vec<String>) {
    for (key, value) in values {
        path.push(key.clone());

        match value {
            Value::Dict(_, nested) if !nested.is_empty() => {
                collect_leaves(nested, path, paths);
            }
            _ => paths.push(path.join(".")),
        }

        path.pop();
    }
}

/// Walks two tables in step, recording the leaves that differ.
fn compare(previous: &Dict, current: &Dict, path: &mut Vec<String>, changes: &mut Vec<Change>) {
    for (key, before) in previous {
        path.push(key.clone());

        match current.get(key) {
            Some(after) => compare_values(before, after, path, changes),
            None => changes.push(change(path, ChangeKind::Removed)),
        }

        path.pop();
    }

    for key in current.keys() {
        if previous.contains_key(key) {
            continue;
        }

        path.push(key.clone());
        changes.push(change(path, ChangeKind::Added));
        path.pop();
    }
}

fn compare_values(
    before: &Value,
    after: &Value,
    path: &mut Vec<String>,
    changes: &mut Vec<Change>,
) {
    match (before, after) {
        // Two tables are compared key by key, so a change deep inside one is
        // reported at the leaf that actually moved rather than at the table.
        (Value::Dict(_, before), Value::Dict(_, after)) => compare(before, after, path, changes),
        _ if values_equal(before, after) => {}
        _ => changes.push(change(path, ChangeKind::Modified)),
    }
}

/// Compared through this crate's own untagged [`Value`](crate::Value) — the
/// tree [`Snapshot::to_value`] already hands out — rather than through the
/// figment value in hand.
///
/// figment's value carries a provenance tag and a numeric *width*, neither of
/// which is part of the configuration: an integer that arrived as `u8` from
/// one provider and `u64` from another is the same setting. This used to
/// compare `Debug` renderings, which stripped the tag but kept the width, so
/// a reload could report every such key as changed — and which made
/// correctness rest on an upstream crate's rendering, where a future addition
/// to it would be silent in both directions.
fn values_equal(before: &Value, after: &Value) -> bool {
    crate::value::from_figment(before) == crate::value::from_figment(after)
}

fn change(path: &[String], kind: ChangeKind) -> Change {
    Change {
        path: path.join("."),
        kind,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict(entries: &[(&str, Value)]) -> Dict {
        entries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect()
    }

    fn snapshot(entries: &[(&str, Value)]) -> Snapshot {
        Snapshot::new(dict(entries))
    }

    #[test]
    fn identical_snapshots_have_no_changes() {
        let one = snapshot(&[("host", "a".into()), ("port", 1u16.into())]);
        let two = snapshot(&[("host", "a".into()), ("port", 1u16.into())]);

        assert!(one.diff(&two).is_empty());
    }

    /// A value as a provider hands it over — figment records where it came
    /// from in a tag, and two providers tag the same value differently.
    fn from_provider(key: &str, value: impl serde::Serialize) -> Value {
        figment::Figment::from((key, value))
            .find_value(key)
            .expect("the provider supplies exactly this key")
    }

    #[test]
    fn the_same_value_from_two_providers_is_not_a_change() {
        let (left, right) = (from_provider("host", "a"), from_provider("host", "a"));

        assert_ne!(
            left.tag(),
            right.tag(),
            "the point of this test is two differently tagged values"
        );

        assert!(snapshot(&[("host", left)])
            .diff(&snapshot(&[("host", right)]))
            .is_empty());
    }

    /// The width an integer arrived at is the provider's business, not the
    /// configuration's: `5432` from a `u8`-shaped default and `5432` from a
    /// JSON file are one setting. Comparing rendered figment values called
    /// that a change on every reload.
    #[test]
    fn the_same_number_at_two_widths_is_not_a_change() {
        let narrow = snapshot(&[("port", Value::from(1u8))]);
        let wide = snapshot(&[("port", Value::from(1u64))]);

        assert!(narrow.diff(&wide).is_empty());
    }

    /// And the line that stays drawn: a whole number is not the float that
    /// prints the same.
    #[test]
    fn an_integer_and_a_float_are_different_values() {
        let integer = snapshot(&[("ratio", Value::from(1u8))]);
        let float = snapshot(&[("ratio", Value::from(1.0f64))]);

        let changes = integer.diff(&float);

        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, "ratio");
        assert_eq!(changes[0].kind, ChangeKind::Modified);
    }

    #[test]
    fn a_modified_value_names_its_key_but_not_its_value() {
        let one = snapshot(&[("password", "hunter2".into())]);
        let two = snapshot(&[("password", "letmein".into())]);

        let changes = one.diff(&two);

        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, "password");
        assert_eq!(changes[0].kind, ChangeKind::Modified);

        let rendered = changes[0].to_string();
        assert_eq!(rendered, "password changed");
        assert!(!rendered.contains("hunter2"), "{rendered}");
        assert!(!rendered.contains("letmein"), "{rendered}");
    }

    #[test]
    fn additions_and_removals_are_told_apart() {
        let one = snapshot(&[("gone", 1u16.into())]);
        let two = snapshot(&[("fresh", 1u16.into())]);

        let changes = one.diff(&two);

        assert_eq!(
            changes,
            [
                Change {
                    path: "fresh".to_owned(),
                    kind: ChangeKind::Added,
                },
                Change {
                    path: "gone".to_owned(),
                    kind: ChangeKind::Removed,
                },
            ]
        );
    }

    #[test]
    fn a_change_inside_a_table_is_reported_at_the_leaf() {
        let one = snapshot(&[(
            "pool",
            Value::from(dict(&[("max", 1u16.into()), ("min", 1u16.into())])),
        )]);
        let two = snapshot(&[(
            "pool",
            Value::from(dict(&[("max", 2u16.into()), ("min", 1u16.into())])),
        )]);

        let changes = one.diff(&two);

        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, "pool.max", "not just `pool`");
    }

    #[test]
    fn a_table_replaced_by_a_scalar_is_one_change() {
        let one = snapshot(&[("pool", Value::from(dict(&[("max", 1u16.into())])))]);
        let two = snapshot(&[("pool", 1u16.into())]);

        let changes = one.diff(&two);

        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, "pool");
        assert_eq!(changes[0].kind, ChangeKind::Modified);
    }

    #[test]
    fn a_value_can_be_read_by_path_without_a_struct() {
        let snapshot = snapshot(&[
            ("host", "a".into()),
            ("pool", Value::from(dict(&[("max", 32u16.into())]))),
        ]);

        assert_eq!(snapshot.get::<String>("host").unwrap(), "a");
        assert_eq!(snapshot.get::<u16>("pool.max").unwrap(), 32);
        assert!(snapshot.contains("pool.max"));
        assert!(!snapshot.contains("pool.min"));
    }

    #[test]
    fn a_missing_path_and_a_wrong_type_are_told_apart() {
        let snapshot = snapshot(&[("host", "a".into())]);

        assert_eq!(
            snapshot.get::<String>("nowhere").unwrap_err().kind(),
            ErrorKind::Missing
        );
        assert_eq!(
            snapshot.get::<u16>("host").unwrap_err().kind(),
            ErrorKind::Type
        );
        // Walking through a scalar is a missing path, not a type error.
        assert_eq!(
            snapshot.get::<u16>("host.port").unwrap_err().kind(),
            ErrorKind::Missing
        );
    }

    #[test]
    fn a_sub_snapshot_carries_only_its_own_table() {
        let snapshot = snapshot(&[
            ("host", "a".into()),
            ("pool", Value::from(dict(&[("max", 32u16.into())]))),
        ]);

        let pool = snapshot.sub("pool").expect("`pool` is a table");

        assert_eq!(pool.get::<u16>("max").unwrap(), 32);
        assert!(!pool.contains("host"));

        assert!(snapshot.sub("host").is_none(), "a scalar is not a table");
    }

    #[test]
    fn leaf_paths_reach_into_nested_tables() {
        let snapshot = snapshot(&[
            ("host", "a".into()),
            ("pool", Value::from(dict(&[("max", 1u16.into())]))),
        ]);

        assert_eq!(snapshot.leaf_paths(), ["host", "pool.max"]);
        assert_eq!(snapshot.top_level_keys(), ["host", "pool"]);
    }

    #[test]
    fn extraction_reports_the_path_it_failed_at() {
        #[derive(serde::Deserialize, Debug)]
        #[allow(dead_code)]
        struct Target {
            port: u16,
        }

        let error = snapshot(&[("port", "not-a-number".into())])
            .extract::<Target>()
            .unwrap_err();

        assert_eq!(error.path(), "port");
    }

    mod properties {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(256))]

            /// A diff never panics and never reports a value, whatever the
            /// two trees hold — the security property, fuzzed.
            #[test]
            fn diff_reports_paths_never_values(
                a in prop::collection::btree_map("[a-z]{1,8}", "[a-zA-Z0-9]{4,16}", 0..8),
                b in prop::collection::btree_map("[a-z]{1,8}", "[a-zA-Z0-9]{4,16}", 0..8),
            ) {
                let left = Snapshot::new(
                    a.iter().map(|(k, v)| (k.clone(), Value::from(v.clone()))).collect(),
                );
                let right = Snapshot::new(
                    b.iter().map(|(k, v)| (k.clone(), Value::from(v.clone()))).collect(),
                );

                for change in left.diff(&right) {
                    let rendered = change.to_string();

                    for value in a.values().chain(b.values()) {
                        prop_assert!(
                            !rendered.contains(value.as_str()),
                            "a diff must name paths, never values: {}",
                            rendered
                        );
                    }
                }
            }
        }
    }
}

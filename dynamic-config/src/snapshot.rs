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

use std::fmt;

use figment::value::{Dict, Value};
use serde::de::DeserializeOwned;

use crate::error::{Error, ErrorKind};

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
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    values: Dict,
}

impl Snapshot {
    pub(crate) fn new(values: Dict) -> Self {
        Self { values }
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
            .map_err(|error: figment::Error| {
                let mut translated = Error::new(ErrorKind::Type, error.to_string());

                for segment in error.path.iter().rev() {
                    translated = translated.prepend_key(segment);
                }

                translated
            })
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
    /// # Errors
    ///
    /// If nothing supplies `path`, or the value cannot become `T`.
    pub fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, Error> {
        let value = self.at(path).ok_or_else(|| {
            Error::new(ErrorKind::Missing, "no value at this path").prepend_key(path)
        })?;

        value.deserialize().map_err(|error: figment::Error| {
            Error::new(ErrorKind::Type, error.to_string()).prepend_key(path)
        })
    }

    /// Whether anything supplies `path`.
    #[must_use]
    pub fn contains(&self, path: &str) -> bool {
        self.at(path).is_some()
    }

    /// The table at `path`, as a snapshot of its own.
    ///
    /// The analogue of Viper's `Sub`: hand a subsystem the part of the
    /// configuration it owns and nothing else.
    #[must_use]
    pub fn sub(&self, path: &str) -> Option<Self> {
        match self.at(path)? {
            Value::Dict(_, nested) => Some(Self::new(nested.clone())),
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

/// figment values carry a provenance tag that takes part in `PartialEq`, so two
/// identical values from different providers compare unequal. Rendering strips
/// the tag, which is the comparison anyone actually means here.
fn values_equal(before: &Value, after: &Value) -> bool {
    format!("{before:?}") == format!("{after:?}")
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
}

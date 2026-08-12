//! Answering "will this boot, and where is each value coming from?" without
//! booting.
//!
//! A deployment's configuration is assembled from files in three directories,
//! an environment, two runtime layers and possibly a profile. When it is wrong,
//! the useful question is not *what does the struct look like* — it is *which
//! layer set this, and did I typo a key*. That is a different report from a
//! successful load, and it has to work when the load **fails**.
//!
//! ## What it deliberately does not print
//!
//! Values. A report that showed them would be pasted into an issue tracker with
//! the database password in it, undoing `#[config(secret)]` exactly as a naive
//! reload diff would. Paths and origins say where to look; the file already
//! holds the value.
//!
//! ## What unknown-key detection can and cannot catch
//!
//! `Report` compares the resolved section's **top-level** keys against the
//! struct's field names, so `db.hsot` is caught and `db.pool.mx_size` is not:
//! a proc-macro sees the field's *type name*, not its fields, so nothing here
//! knows what lives inside `pool`.
//!
//! Detection is skipped entirely when a field carries `#[serde(flatten)]`,
//! because a flattened field legitimately absorbs keys the outer struct never
//! names — reporting those as typos would be worse than reporting nothing.

use std::fmt;

use crate::error::{Error, Origin};
use crate::snapshot::Snapshot;
use crate::source::LoadSpec;

/// A key the configuration supplies that the struct does not name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownKey {
    /// The key, relative to the section.
    pub path: String,
    /// The closest field name, when one is close enough to be a likely typo.
    pub suggestion: Option<String>,
}

impl fmt::Display for UnknownKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: unknown key", self.path)?;

        if let Some(suggestion) = &self.suggestion {
            write!(f, ", did you mean `{suggestion}`?")?;
        }

        Ok(())
    }
}

/// Where one key's value comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// The key, relative to the section.
    pub path: String,
    /// The layer that supplied it.
    pub origin: Origin,
}

impl fmt::Display for Resolved {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:<28} {}", self.path, self.origin)
    }
}

/// What a configuration resolves to, and whether it would load.
///
/// Built by [`check`], and by the `check()` the macro generates. Prints as a
/// short report; the fields are public for anything that wants to render it
/// differently.
#[derive(Debug, Clone)]
pub struct Report {
    /// The section this describes.
    pub key: String,
    /// Every key the sources supply, with the layer that won.
    pub resolved: Vec<Resolved>,
    /// Keys the struct does not name. Empty when detection was skipped.
    pub unknown: Vec<UnknownKey>,
    /// Why the configuration would fail to load, if it would.
    pub failure: Option<String>,
}

impl Report {
    /// Whether a load would succeed and no key looks like a typo.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.failure.is_none() && self.unknown.is_empty()
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "[{}]", self.key)?;

        if self.resolved.is_empty() {
            writeln!(f, "  (nothing supplies this section)")?;
        }

        for resolved in &self.resolved {
            writeln!(f, "  {resolved}")?;
        }

        if !self.unknown.is_empty() {
            writeln!(f)?;

            for unknown in &self.unknown {
                writeln!(f, "  {unknown}")?;
            }
        }

        match &self.failure {
            Some(failure) => write!(f, "\n  would not load: {failure}"),
            None => write!(f, "\n  would load"),
        }
    }
}

/// Builds a report for `spec`.
///
/// `fields` are the struct's field names, used for unknown-key detection; pass
/// an empty slice to skip it. The generated `check()` passes the real ones.
///
/// # Errors
///
/// Only if the sources cannot be read or parsed at all. A configuration that
/// parses but would not deserialize is a successful report with
/// [`failure`](Report::failure) set — that is the case worth reporting on.
pub fn check<T>(spec: &LoadSpec<'_>, fields: &[&str]) -> Result<Report, Error>
where
    T: serde::de::DeserializeOwned,
{
    // One build serves every question below. The old shape called
    // `source_of` per leaf key, and each call rebuilt the figment — re-reading
    // and re-parsing every source once per key.
    let (snapshot, figment) = crate::loader::resolved(spec)?;
    let paths = snapshot.leaf_paths();

    let resolved = paths
        .iter()
        .map(|path| Resolved {
            path: path.clone(),
            origin: crate::loader::origin_in(&figment, path, spec.nest),
        })
        .collect();

    Ok(Report {
        key: spec.key.to_owned(),
        resolved,
        // An aliased old path is a *known* key rather than an ignored one: the
        // detector exists to catch typos, and an alias that silenced it would
        // make `pool.szie` a supported spelling.
        unknown: unknown_keys(&snapshot, fields, &aliased_keys(spec)),
        // `load` rather than `Snapshot::extract`, so the message names the file
        // at fault — which is the whole point of running this.
        failure: crate::loader::load::<T>(spec)
            .err()
            .map(|error| error.to_string()),
    })
}

/// The top-level keys aliases make legitimate.
fn aliased_keys(spec: &LoadSpec<'_>) -> Vec<String> {
    spec.aliases
        .map(crate::Aliases::known_keys)
        .unwrap_or_default()
}

fn unknown_keys(snapshot: &Snapshot, fields: &[&str], aliased: &[String]) -> Vec<UnknownKey> {
    if fields.is_empty() {
        return Vec::new();
    }

    snapshot
        .top_level_keys()
        .into_iter()
        .filter(|key| !fields.contains(&key.as_str()))
        .filter(|key| !aliased.iter().any(|alias| alias == key))
        .map(|key| UnknownKey {
            suggestion: closest(&key, fields),
            path: key,
        })
        .collect()
}

/// The nearest field name, when it is near enough to be a typo rather than a
/// different word.
///
/// The threshold scales with the name: one edit in `id`, three in
/// `connection_timeout`. A fixed distance would either miss typos in long names
/// or propose nonsense for short ones.
fn closest(key: &str, fields: &[&str]) -> Option<String> {
    let budget = (key.len() / 4).max(1);

    fields
        .iter()
        .map(|field| (distance(key, field), *field))
        .filter(|(distance, _)| *distance <= budget)
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, field)| field.to_owned())
}

/// Optimal string alignment distance: Levenshtein, plus transposition.
///
/// Counting a swap as one edit rather than two is the whole reason to prefer it
/// here — `prot` for `port` is the single most common way to mistype a key, and
/// plain Levenshtein scores it the same as two unrelated substitutions.
fn distance(left: &str, right: &str) -> usize {
    let left: Vec<char> = left.chars().collect();
    let right: Vec<char> = right.chars().collect();

    // Three rows: the one before last is what makes a transposition visible.
    let mut before_last = vec![0; right.len() + 1];
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current = vec![0; right.len() + 1];

    for i in 0..left.len() {
        current[0] = i + 1;

        for j in 0..right.len() {
            let substitution = usize::from(left[i] != right[j]);

            current[j + 1] = (previous[j] + substitution)
                .min(previous[j + 1] + 1)
                .min(current[j] + 1);

            // The two characters are each other's, the other way round.
            if i > 0 && j > 0 && left[i] == right[j - 1] && left[i - 1] == right[j] {
                current[j + 1] = current[j + 1].min(before_last[j - 1] + 1);
            }
        }

        std::mem::swap(&mut before_last, &mut previous);
        std::mem::swap(&mut previous, &mut current);
    }

    previous[right.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_close_miss_is_suggested() {
        assert_eq!(closest("hsot", &["host", "port"]).as_deref(), Some("host"));
        assert_eq!(closest("prot", &["host", "port"]).as_deref(), Some("port"));
        assert_eq!(closest("hosts", &["host", "port"]).as_deref(), Some("host"));
    }

    #[test]
    fn an_unrelated_key_suggests_nothing() {
        assert_eq!(closest("elephant", &["host", "port"]), None);
    }

    #[test]
    fn the_budget_scales_with_the_name() {
        // One edit is always allowed, even in a two-letter name.
        assert_eq!(closest("od", &["id"]).as_deref(), Some("id"));
        // Three edits in a long name still reads as a typo.
        assert_eq!(
            closest("conection_timout", &["connection_timeout"]).as_deref(),
            Some("connection_timeout")
        );
        // Two edits in a short one does not.
        assert_eq!(closest("xy", &["id"]), None);
    }

    #[test]
    fn distance_counts_edits() {
        assert_eq!(distance("", ""), 0);
        assert_eq!(distance("host", "host"), 0);
        assert_eq!(distance("host", ""), 4);
        assert_eq!(distance("port", "sport"), 1, "one insertion");
        assert_eq!(distance("port", "pxrt"), 1, "one substitution");
    }

    #[test]
    fn a_transposition_is_one_edit_not_two() {
        // The point of the alignment variant: this is how keys get mistyped.
        assert_eq!(distance("port", "prot"), 1);
        assert_eq!(distance("host", "hsot"), 1);
    }

    #[test]
    fn an_unknown_key_renders_its_suggestion() {
        let unknown = UnknownKey {
            path: "hsot".to_owned(),
            suggestion: Some("host".to_owned()),
        };

        assert_eq!(
            unknown.to_string(),
            "hsot: unknown key, did you mean `host`?"
        );
    }
}

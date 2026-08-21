//! The environment, read as a configuration layer.
//!
//! A prefix decides which variables are configuration at all, a separator
//! decides how a flat name becomes a nested path, and
//! [`text_value`](crate::text_value) decides what the text means:
//!
//! ```text
//! APP_DATABASE__POOL__MAX=32   →   database.pool.max = 32
//! └┬┘ └───────┬──────┘ └┬┘
//!  │          │         └── read as a value, so this is a number
//!  │          └── split on the separator into a path
//!  └── the prefix, matched without regard to case and then dropped
//! ```
//!
//! The rules that look arbitrary are the ones a deployment depends on, so
//! they are written down rather than left to a reading: the prefix match
//! ignores case, names are lowercased after it, a name with an empty
//! segment is skipped rather than refused, and a variable holding nothing
//! is dropped unless the program asked to keep it — a template that renders
//! `PORT=` should not blank out a good value.

use std::collections::BTreeMap;

use crate::text_value;
use crate::value::Value;

/// Every prefixed variable, as one layer's tree.
///
/// `allow_empty` keeps variables whose value is blank; without it they are
/// dropped before they can overwrite anything.
pub(crate) fn tree(prefix: &str, nest: &str, allow_empty: bool) -> Value {
    let mut root = BTreeMap::new();

    for (path, text) in variables(prefix, nest) {
        if !allow_empty && text.trim().is_empty() {
            continue;
        }

        insert(&mut root, &path, text_value::from_text(&text));
    }

    Value::Table(root)
}

/// The prefixed variables, as `(path segments, value)` pairs.
///
/// Split out so the strict-mode check and the layer itself walk the
/// environment the same way — a variable either layer sees is a variable the
/// other sees.
pub(crate) fn variables(prefix: &str, nest: &str) -> Vec<(Vec<String>, String)> {
    let mut found = Vec::new();

    for (name, value) in std::env::vars_os() {
        if name.is_empty() {
            continue;
        }

        let name = name.to_string_lossy();
        let name = name.trim();

        // Case-insensitively, because a shell is not consistent about it and
        // the prefix is a namespace rather than a spelling.
        //
        // `get`, not a slice: `len` counts bytes, and a name whose first
        // characters are multi-byte — `€€` is six bytes against a prefix of
        // four — passes a length check and then slices mid-codepoint. That
        // is a panic on the load path of every program that reads the
        // environment, from a variable it was never going to accept.
        let Some(head) = name.get(..prefix.len()) else {
            continue;
        };

        if !head.eq_ignore_ascii_case(prefix) {
            continue;
        }

        let rest = name[prefix.len()..].trim();
        let segments: Vec<String> = rest
            .split(nest)
            .map(|segment| segment.trim().to_ascii_lowercase())
            .collect();

        // `APP__HOST` names nothing at all: the empty segment is a typo, and
        // guessing which of the two spellings was meant is worse than
        // passing it by.
        if segments.iter().any(String::is_empty) {
            continue;
        }

        found.push((segments, value.to_string_lossy().into_owned()));
    }

    found
}

/// Puts `value` at `path`, building tables on the way.
///
/// A segment that already holds a scalar becomes a table: two variables can
/// name `db` and `db__host`, and the deeper one is the more specific
/// statement.
fn insert(root: &mut BTreeMap<String, Value>, path: &[String], value: Value) {
    let Some((last, walk)) = path.split_last() else {
        return;
    };

    let mut here = root;

    for segment in walk {
        let slot = here
            .entry(segment.clone())
            .or_insert_with(|| Value::Table(BTreeMap::new()));

        if !matches!(slot, Value::Table(_)) {
            *slot = Value::Table(BTreeMap::new());
        }

        let Value::Table(table) = slot else {
            unreachable!("the slot was just made a table");
        };

        here = table;
    }

    match here.get_mut(last) {
        // Both spellings exist and the shallow one arrived first: the table
        // already there keeps its keys and gains nothing, because a scalar
        // cannot be merged into it.
        Some(Value::Table(_)) => {}
        _ => {
            here.insert(last.clone(), value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::tree;
    use crate::value::Value;

    /// The same variables, read by the implementation this one was ported
    /// from — through the conversion every provider's values go through, so
    /// the comparison is on meaning.
    fn original(prefix: &str, nest: &str) -> Value {
        use figment::Provider as _;

        let provider = figment::providers::Env::prefixed(prefix).split(nest);
        let data = provider.data().expect("the environment always reads");
        let dict = data.into_values().next().unwrap_or_default();

        Value::Table(
            dict.iter()
                .map(|(key, value)| (key.clone(), crate::backend::figment::from_figment(value)))
                .collect(),
        )
    }

    /// Variables are process-global, so the cases run in one test with one
    /// prefix nobody else uses.
    #[test]
    fn the_environment_reads_the_same_as_it_always_did() {
        let prefix = "DCENVPORT_";
        let cases = [
            ("HOST", "localhost"),
            ("PORT", "8080"),
            ("DEBUG", "true"),
            ("RATIO", "1.5"),
            ("TAGS", "[a, b, c]"),
            ("POOL__MAX", "32"),
            ("POOL__MIN", "1"),
            ("DEEP__A__B__C", "leaf"),
            ("QUOTED", "\"8080\""),
            ("STRUCTURED", "{key=10}"),
            ("EMPTY_SEGMENT____HERE", "dropped"),
            ("MiXeD", "case"),
        ];

        for (name, value) in cases {
            std::env::set_var(format!("{prefix}{name}"), value);
        }

        let ours = tree(prefix, "__", true);
        let theirs = original(prefix, "__");

        for (name, _) in cases {
            std::env::remove_var(format!("{prefix}{name}"));
        }

        assert_eq!(ours, theirs);
    }

    /// A variable whose name is not ASCII does not take the process down.
    ///
    /// The guard used to be `name.len() < prefix.len()` and then a *slice*
    /// at `prefix.len()`. Both are byte counts, so a name like `€€` — six
    /// bytes against a four-byte prefix — passed the check and sliced
    /// through the middle of a character. That is a panic on the load path
    /// of every program that reads the environment, triggered by a variable
    /// that was never going to match the prefix in the first place.
    #[test]
    fn a_variable_whose_name_is_not_ascii_is_passed_by_rather_than_panicked_on() {
        let prefix = "DCENVUTF8_";

        for name in ["€€", "ü", "日本語", "€€€€€€€€€€"] {
            std::env::set_var(name, "whatever");
        }

        std::env::set_var(format!("{prefix}HOST"), "db.internal");

        let tree = tree(prefix, "__", false);

        for name in ["€€", "ü", "日本語", "€€€€€€€€€€"] {
            std::env::remove_var(name);
        }

        std::env::remove_var(format!("{prefix}HOST"));

        assert!(matches!(tree, Value::Table(table) if table.contains_key("host")));
    }

    /// The one rule that is this crate's rather than the backend's.
    #[test]
    fn a_blank_variable_is_dropped_unless_it_was_asked_for() {
        let prefix = "DCENVBLANK_";
        std::env::set_var(format!("{prefix}HOST"), "");

        let dropped = tree(prefix, "__", false);
        let kept = tree(prefix, "__", true);

        std::env::remove_var(format!("{prefix}HOST"));

        assert_eq!(dropped, Value::Table(std::collections::BTreeMap::new()));
        assert!(matches!(kept, Value::Table(table) if table.contains_key("host")));
    }
}

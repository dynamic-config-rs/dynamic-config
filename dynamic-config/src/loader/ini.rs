//! INI, as a built-in format.
//!
//! The dialect, spelled out because "INI" names a family rather than a
//! standard:
//!
//! - `[section]` opens a table; `[a.b]` opens a nested one, the
//!   git-config convention. Keys before any header sit at the root.
//! - Whole-line comments start with `;` or `#`. There are no trailing
//!   comments: a `#` inside a value is part of the value, because
//!   stripping it would corrupt any value that legitimately contains one.
//! - Values are scalars widened by the same rule the environment layer
//!   applies — `true`/`false`, then integer, then float, then string —
//!   and a double-quoted value is a string, verbatim, no widening.
//! - No line continuations: no mainstream dialect has them.
//!
//! A line that is neither a header nor `key = value` is an error naming
//! the line *number* and never the line — the same redaction rule every
//! diagnostic here follows, because a mangled line is most often a pasted
//! secret.

// `insert` and `scalar` are `properties`' too, and both name `Dict` — so
// the map and the alias compile whenever either format does, while the INI
// parser itself stays behind `ini`.
use std::collections::BTreeMap;

#[cfg(feature = "ini")]
use crate::error::{Error, ErrorKind};
use crate::value::Value;

/// A document's keys.
type Dict = BTreeMap<String, Value>;

/// Reads INI text into a document.
///
/// # Errors
///
/// On a line that is neither a section header nor `key = value`, on an
/// empty section name or key, and on a key that would have to be both a
/// value and a table. Every message names the line and never the value.
#[cfg(feature = "ini")]
pub(crate) fn parse(text: &str) -> Result<Value, Error> {
    let mut root = Dict::new();
    let mut section: Vec<String> = Vec::new();

    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();

        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }

        if let Some(header) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = header
                .split('.')
                .map(|part| part.trim().to_owned())
                .collect();

            if section.iter().any(String::is_empty) {
                return Err(refused(index + 1, "an empty section name"));
            }

            continue;
        }

        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();

            if key.is_empty() {
                return Err(refused(index + 1, "`= value` with no key"));
            }

            let mut path: Vec<&str> = section.iter().map(String::as_str).collect();
            path.push(key);

            insert(&mut root, &path, scalar(value.trim()), index + 1)
                .map_err(|reason| Error::new(ErrorKind::Parse, reason))?;

            continue;
        }

        return Err(refused(
            index + 1,
            "neither a section header nor `key = value`",
        ));
    }

    Ok(Value::Table(root))
}

/// A refusal that names the line and nothing on it.
#[cfg(feature = "ini")]
fn refused(line: usize, reason: &str) -> Error {
    Error::new(ErrorKind::Parse, format!("line {line}: {reason}"))
}

/// Puts `value` at `path`, building tables on the way, refusing to turn a
/// scalar into a table or the reverse — a collision is a document saying
/// two contradictory things, and later-wins inside one file hides typos.
pub(super) fn insert(
    root: &mut Dict,
    path: &[&str],
    value: Value,
    line: usize,
) -> Result<(), String> {
    let (last, walk) = path.split_last().expect("a key is never empty");

    let mut here = root;

    for part in walk {
        let slot = here
            .entry((*part).to_owned())
            .or_insert_with(|| Value::Table(Dict::new()));

        match slot {
            Value::Table(dict) => here = dict,
            _ => {
                return Err(format!(
                    "line {line}: `{part}` is already a value, so it cannot also \
                     hold `{last}`"
                ));
            }
        }
    }

    if let Some(Value::Table(_)) = here.get(*last) {
        return Err(format!(
            "line {line}: `{last}` is already a table, so it cannot also be a value"
        ));
    }

    here.insert((*last).to_owned(), value);

    Ok(())
}

/// The environment layer's widening, applied to a format that has no
/// types of its own: `true`/`false`, integer, float, and string in that
/// order — and a double-quoted value is a string, no questions asked.
pub(super) fn scalar(text: &str) -> Value {
    if text.len() >= 2 && text.starts_with('"') && text.ends_with('"') {
        return Value::from(&text[1..text.len() - 1]);
    }

    if let Ok(flag) = text.parse::<bool>() {
        Value::Bool(flag)
    } else if let Ok(whole) = text.parse::<i64>() {
        Value::Integer(i128::from(whole))
    } else if let Ok(real) = text.parse::<f64>() {
        Value::Float(real)
    } else {
        Value::from(text)
    }
}

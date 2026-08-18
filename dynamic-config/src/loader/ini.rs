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

#[cfg(feature = "ini")]
use std::path::PathBuf;

#[cfg(feature = "ini")]
use figment::value::Map;
use figment::value::{Dict, Value};
#[cfg(feature = "ini")]
use figment::{Metadata, Profile, Provider};

/// The INI provider behind [`Format::Ini`](crate::Format::Ini).
#[cfg(feature = "ini")]
pub(crate) struct Ini {
    text: Result<String, String>,
    path: Option<PathBuf>,
}

#[cfg(feature = "ini")]
impl Ini {
    pub(crate) fn file(path: impl Into<PathBuf>) -> Self {
        let path = path.into();

        Self {
            text: std::fs::read_to_string(&path).map_err(|error| error.to_string()),
            path: Some(path),
        }
    }

    pub(crate) fn string(text: &str) -> Self {
        Self {
            text: Ok(text.to_owned()),
            path: None,
        }
    }
}

#[cfg(feature = "ini")]
impl Provider for Ini {
    fn metadata(&self) -> Metadata {
        match &self.path {
            Some(path) => Metadata::from("INI file", path.as_path()),
            None => Metadata::named("INI source"),
        }
    }

    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        let text = match &self.text {
            Ok(text) => text,
            Err(error) => return Err(figment::Error::from(error.clone())),
        };

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
                    return Err(figment::Error::from(format!(
                        "line {}: an empty section name",
                        index + 1
                    )));
                }

                continue;
            }

            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();

                if key.is_empty() {
                    return Err(figment::Error::from(format!(
                        "line {}: `= value` with no key",
                        index + 1
                    )));
                }

                let mut path: Vec<&str> = section.iter().map(String::as_str).collect();
                path.push(key);

                insert(&mut root, &path, scalar(value.trim()), index + 1)
                    .map_err(figment::Error::from)?;

                continue;
            }

            return Err(figment::Error::from(format!(
                "line {} is neither a section header nor `key = value`",
                index + 1
            )));
        }

        Ok(Map::from([(Profile::Default, root)]))
    }
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
            .or_insert_with(|| Value::from(Dict::new()));

        match slot {
            Value::Dict(_, dict) => here = dict,
            _ => {
                return Err(format!(
                    "line {line}: `{part}` is already a value, so it cannot also \
                     hold `{last}`"
                ));
            }
        }
    }

    if let Some(Value::Dict(..)) = here.get(*last) {
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
        Value::from(flag)
    } else if let Ok(whole) = text.parse::<i64>() {
        Value::from(whole)
    } else if let Ok(real) = text.parse::<f64>() {
        Value::from(real)
    } else {
        Value::from(text)
    }
}

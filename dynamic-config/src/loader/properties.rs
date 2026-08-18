//! Java-style `.properties`, as a built-in format.
//!
//! The dialect, and where it deliberately differs from
//! `java.util.Properties`:
//!
//! - **UTF-8, not ISO-8859-1.** Modern JDKs read UTF-8 properties too;
//!   an escape-only encoding is a legacy this crate does not inherit.
//! - Dotted keys nest: `db.pool.max = 8` is the document
//!   `{db: {pool: {max: 8}}}` — `.` is to properties what `__` is to the
//!   environment layer.
//! - `=` and `:` both separate, first unescaped one wins; a `\` at the
//!   end of a line continues onto the next, leading whitespace of the
//!   continuation trimmed; `\t` `\n` `\r` `\\` `\uXXXX` and escaped
//!   separators are honoured.
//! - Comments start with `#` or `!` at the start of a (trimmed) line.
//! - Values widen exactly as INI and the environment do.
//! - **A collision is an error, not last-wins**: `a = 1` and `a.b = 2`
//!   in one document contradict each other, and the error names both
//!   keys — and only the keys.

use std::path::PathBuf;

use figment::value::{Dict, Map};
use figment::Profile;
use figment::{Metadata, Provider};

use super::ini::{insert, scalar};

/// The provider behind [`Format::Properties`](crate::Format::Properties).
pub(crate) struct Properties {
    text: Result<String, String>,
    path: Option<PathBuf>,
}

impl Properties {
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

impl Provider for Properties {
    fn metadata(&self) -> Metadata {
        match &self.path {
            Some(path) => Metadata::from("properties file", path.as_path()),
            None => Metadata::named("properties source"),
        }
    }

    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        let text = match &self.text {
            Ok(text) => text,
            Err(error) => return Err(figment::Error::from(error.clone())),
        };

        let mut root = Dict::new();

        for (number, line) in logical_lines(text) {
            let trimmed = line.trim_start();

            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('!') {
                continue;
            }

            let (raw_key, raw_value) = split(trimmed).ok_or_else(|| {
                figment::Error::from(format!("line {number}: no `=` or `:` separator"))
            })?;

            let key = unescape(raw_key.trim(), number).map_err(figment::Error::from)?;
            let value = unescape(raw_value.trim_start(), number).map_err(figment::Error::from)?;

            if key.is_empty() {
                return Err(figment::Error::from(format!(
                    "line {number}: a separator with no key before it"
                )));
            }

            let path: Vec<&str> = key.split('.').collect();

            if path.iter().any(|part| part.is_empty()) {
                return Err(figment::Error::from(format!(
                    "line {number}: `{key}` has an empty segment"
                )));
            }

            insert(&mut root, &path, scalar(&value), number).map_err(figment::Error::from)?;
        }

        Ok(Map::from([(Profile::Default, root)]))
    }
}

/// Joins continuation lines: a line whose backslashes at the end number
/// odd continues onto the next, as `java.util.Properties` reads it.
/// Answers `(first physical line number, logical line)` pairs.
fn logical_lines(text: &str) -> Vec<(usize, String)> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut started_at = 0usize;

    for (index, raw) in text.lines().enumerate() {
        if current.is_empty() {
            started_at = index + 1;
        }

        let trailing = raw.chars().rev().take_while(|c| *c == '\\').count();

        if trailing % 2 == 1 {
            // Drop the continuation backslash, keep everything before it,
            // and trim the *next* line's leading whitespace when it lands.
            let mut kept = raw.to_owned();
            kept.pop();
            current.push_str(if current.is_empty() {
                &kept
            } else {
                kept.trim_start()
            });
        } else {
            current.push_str(if current.is_empty() {
                raw
            } else {
                raw.trim_start()
            });
            lines.push((started_at, std::mem::take(&mut current)));
        }
    }

    if !current.is_empty() {
        lines.push((started_at, current));
    }

    lines
}

/// The first unescaped `=` or `:` splits key from value.
fn split(line: &str) -> Option<(&str, &str)> {
    let mut escaped = false;

    for (index, character) in line.char_indices() {
        if escaped {
            escaped = false;

            continue;
        }

        match character {
            '\\' => escaped = true,
            '=' | ':' => return Some((&line[..index], &line[index + 1..])),
            _ => {}
        }
    }

    None
}

/// `\t` `\n` `\r` `\\` `\uXXXX`, and any other escaped character is
/// itself — which is what makes `\=` and `\:` and `\#` work.
fn unescape(text: &str, line: usize) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut characters = text.chars();

    while let Some(character) = characters.next() {
        if character != '\\' {
            out.push(character);

            continue;
        }

        match characters.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('u') => {
                let digits: String = characters.by_ref().take(4).collect();

                if digits.len() != 4 {
                    return Err(format!("line {line}: `\\u` needs four hex digits"));
                }

                let code = u32::from_str_radix(&digits, 16)
                    .map_err(|_| format!("line {line}: `\\u{digits}` is not four hex digits"))?;

                match char::from_u32(code) {
                    Some(decoded) => out.push(decoded),
                    None => {
                        return Err(format!("line {line}: `\\u{digits}` is not a character"));
                    }
                }
            }
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }

    Ok(out)
}

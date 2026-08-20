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

use std::collections::BTreeMap;

use super::ini::{insert, scalar};
use crate::error::{Error, ErrorKind};
use crate::value::Value;

/// Reads `.properties` text into a document.
///
/// # Errors
///
/// On a line with no separator, a key that is empty or has an empty
/// segment, an escape the format does not define, and a key that would have
/// to be both a value and a table. Every message names the line, never the
/// value.
pub(crate) fn parse(text: &str) -> Result<Value, Error> {
    let mut root: BTreeMap<String, Value> = BTreeMap::new();

    for (number, line) in logical_lines(text) {
        let trimmed = line.trim_start();

        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('!') {
            continue;
        }

        let (raw_key, raw_value) =
            split(trimmed).ok_or_else(|| refused(number, "no `=` or `:` separator"))?;

        let key = unescape(raw_key.trim(), number).map_err(parse_error)?;
        let value = unescape(raw_value.trim_start(), number).map_err(parse_error)?;

        if key.is_empty() {
            return Err(refused(number, "a separator with no key before it"));
        }

        let path: Vec<&str> = key.split('.').collect();

        if path.iter().any(|part| part.is_empty()) {
            return Err(refused(number, &format!("`{key}` has an empty segment")));
        }

        insert(&mut root, &path, scalar(&value), number).map_err(parse_error)?;
    }

    Ok(Value::Table(root))
}

/// A refusal that names the line and nothing on it.
fn refused(line: usize, reason: &str) -> Error {
    Error::new(ErrorKind::Parse, format!("line {line}: {reason}"))
}

fn parse_error(reason: String) -> Error {
    Error::new(ErrorKind::Parse, reason)
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

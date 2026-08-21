//! Reading a configuration document into this crate's tree.
//!
//! One function per direction — text in, [`Value`] out — with the format
//! deciding which parser runs and nothing else. Every reader lands in the
//! same tree, so everything above this line works on one shape whatever the
//! file extension was.
//!
//! **A message never carries a value.** A parse failure names where it
//! stopped, not what it found there: the line that failed to parse is
//! frequently the line holding the password, and an error is the one place
//! a configuration value has no business appearing. The redaction fuzz
//! target drives this module for exactly that reason.

use std::path::Path;

use crate::error::{Error, ErrorKind, Origin};
use crate::source::Format;
use crate::value::Value;

/// Parses `text` as `format`.
///
/// # Errors
///
/// If the text is not valid in its format, or the format's feature is off.
pub(crate) fn parse(text: &str, format: Format) -> Result<Value, Error> {
    parse_with(crate::reader::installed(), text, format)
}

/// [`parse`], with the reader named rather than installed — what a load
/// that chose its own uses.
///
/// # Errors
///
/// As [`parse`].
pub(crate) fn parse_with(
    reader: &'static dyn crate::reader::Reader,
    text: &str,
    format: Format,
) -> Result<Value, Error> {
    // Which reader answers is settled first, because "this build cannot
    // read RON at all" and "this RON file is empty" are different answers
    // and only one of them is about the document. An empty file otherwise
    // read as fine in a build that would have refused every other file
    // beside it.
    let Some(reader) = crate::reader::for_format(reader, format) else {
        return Err(crate::reader::unread(format));
    };

    // Nothing at all is an empty document rather than a failure: a file
    // somebody has not written yet says nothing, and saying nothing is
    // allowed — every layer is optional. Answered before the reader runs,
    // so it means the same thing whichever one is installed.
    if text.trim().is_empty() {
        return Ok(Value::Table(std::collections::BTreeMap::new()));
    }

    let parsed = reader.parse(text, format)?;

    // A document is keys: a bare scalar or a list at the top has no
    // section to be, and reading it as one would put the whole file under
    // a name it never chose. Checked here rather than in each reader, so
    // the refusal is one sentence whoever parsed.
    match parsed {
        table @ Value::Table(_) => Ok(table),
        _ => Err(Error::new(
            ErrorKind::Parse,
            "a configuration document is a table of keys; this one is not",
        )),
    }
}

/// This crate's own parsers, behind [`Reader`](crate::reader::Reader).
pub(crate) fn parse_natively(text: &str, format: Format) -> Result<Value, Error> {
    #[cfg(not(any(
        feature = "json",
        feature = "toml",
        feature = "yaml",
        feature = "ini",
        feature = "properties"
    )))]
    let _ = text;

    match format {
        #[cfg(feature = "json")]
        Format::Json => serde_json::from_str::<Value>(text).map_err(|error| failed("JSON", &error)),
        #[cfg(feature = "toml")]
        // Built from the parts rather than rendered: this parser's own
        // `Display` draws the offending line under a caret, which is a
        // picture of the document — and the line it draws is the line that
        // failed, which on a bad day is the password.
        Format::Toml => toml::from_str::<Value>(text).map(dates).map_err(|error| {
            let reason = crate::loader::redacted(error.message());

            match error.span().map(|span| at(text, span.start)) {
                Some((line, column)) => Error::new(
                    ErrorKind::Parse,
                    format!("TOML parse error at line {line}, column {column}: {reason}"),
                ),
                None => Error::new(ErrorKind::Parse, format!("TOML parse error: {reason}")),
            }
        }),
        #[cfg(feature = "yaml")]
        Format::Yaml => serde_yaml::from_str::<Value>(text).map_err(|error| failed("YAML", &error)),
        #[cfg(feature = "ini")]
        Format::Ini => crate::loader::ini::parse(text),
        #[cfg(feature = "properties")]
        Format::Properties => crate::loader::properties::parse(text),

        #[allow(unreachable_patterns)]
        format => Err(crate::reader::unread(format)),
    }
}

/// Reads and parses `path`, or `None` when there is no such file.
///
/// An absent file is not a failure — every file layer is optional, and a
/// deployment that ships two of three documents is ordinary.
///
/// # Errors
///
/// If the file exists but cannot be read, or does not parse.
pub(crate) fn read(
    reader: &'static dyn crate::reader::Reader,
    path: &Path,
    format: Format,
) -> Result<Option<Value>, Error> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(Error::new(ErrorKind::Io, error.to_string())
                .with_origin(Origin::File(path.to_owned())))
        }
    };

    parse_with(reader, &text, format)
        .map(Some)
        .map_err(|error| error.with_origin(Origin::File(path.to_owned())))
}

/// A parser said no.
///
/// **One line, never more.** A message that runs to a second line is
/// quoting the document — every parser here draws its snippets that way —
/// and the line it quotes is the line that failed to parse, which is
/// exactly the line a reader must not be shown.
#[cfg(any(feature = "json", feature = "yaml"))]
fn failed(format: &str, error: &dyn std::fmt::Display) -> Error {
    let rendered = error.to_string();
    let first = rendered.lines().next().unwrap_or_default();

    Error::new(
        ErrorKind::Parse,
        format!("{format}: {}", crate::loader::redacted(first)),
    )
}

/// TOML's datetimes, as the text they were written as.
///
/// The parser hands a datetime to serde as a one-key table under a
/// private name — `{"$__toml_private_datetime": "1979-05-27T07:32:00Z"}` —
/// which is its own encoding, not a shape anybody wrote. Left alone it
/// reaches a configuration as a table: a `String` field refuses it, a
/// `chrono::DateTime` field refuses it, and the message names a key from
/// inside a dependency.
///
/// So it becomes the string it was written as, which is what every
/// datetime library deserializes from and what the other readers here
/// already produce. The cost is one walk of a parsed document.
#[cfg(feature = "toml")]
pub(crate) fn dates(value: Value) -> Value {
    /// `toml`'s own name for the field it hides a datetime behind.
    const DATETIME: &str = "$__toml_private_datetime";

    match value {
        Value::Table(mut table) => {
            if table.len() == 1 {
                if let Some(Value::String(written)) = table.remove(DATETIME) {
                    return Value::String(written);
                }

                // Not a datetime after all: `remove` took the entry, so
                // whatever was there goes back.
                if let Some((key, value)) = table.into_iter().next() {
                    return Value::Table(std::collections::BTreeMap::from([(key, dates(value))]));
                }

                return Value::Table(std::collections::BTreeMap::new());
            }

            Value::Table(
                table
                    .into_iter()
                    .map(|(key, value)| (key, dates(value)))
                    .collect(),
            )
        }
        Value::Array(values) => Value::Array(values.into_iter().map(dates).collect()),
        other => other,
    }
}

/// The line and column a byte offset falls on, both one-based.
#[cfg(feature = "toml")]
fn at(text: &str, offset: usize) -> (usize, usize) {
    let before = &text[..offset.min(text.len())];
    let line = before.matches('\n').count() + 1;
    let column = before
        .rsplit_once('\n')
        .map_or(before.chars().count(), |(_, last)| last.chars().count())
        + 1;

    (line, column)
}

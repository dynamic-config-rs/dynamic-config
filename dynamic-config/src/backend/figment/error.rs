//! figment's errors, as this crate's.
//!
//! Reached from two places and no more: the interop `Source::provider`
//! seam, whose provider reports failures in figment's terms, and the
//! deserializer's oracle test, which compares this crate's error paths
//! against figment's — which is why the path reader and the message
//! scrubber are compiled for tests as well, and the translation itself
//! only behind the feature.
//!
//! **A message never carries a value.** figment renders a type error as
//! ``found string "hunter2"``, and a parse error as the document line it
//! stopped on — so a translation that passed either through would put a
//! configuration value in a diagnostic, which is the one place this crate
//! says one will never appear.

#[cfg(feature = "figment")]
use crate::error::{Error, ErrorKind};

#[cfg(feature = "figment")]
/// The message for a figment error, with any offending *value* left out.
///
/// figment renders a type mismatch as ``invalid type: found string "hunter2",
/// expected u16``, which is helpful right up until the value is a secret — a
/// password pasted into a numeric field lands in a log line, and every other
/// diagnostic in this crate goes to some length to make sure that cannot
/// happen. The key path, the kind of thing that was there, and the type that
/// was wanted are all kept; only the value goes.
fn message(error: &figment::Error) -> String {
    use figment::error::Kind;

    match &error.kind {
        Kind::InvalidType(actual, expected) => {
            format!(
                "invalid type: found {}, expected {expected}",
                kind_of(actual)
            )
        }
        Kind::InvalidValue(actual, expected) => {
            format!(
                "invalid value: found {}, expected {expected}",
                kind_of(actual)
            )
        }
        _ => crate::loader::redacted(&without_quoted_source(&error.to_string())),
    }
}

/// Drops the source excerpt a parser echoes back at a syntax error.
///
/// `toml` renders one as a gutter block:
///
/// ```text
/// TOML parse error at line 2, column 25
///   |
/// 2 | password = "hunter2
///   |                    ^
/// invalid basic string
/// ```
///
/// The offending line is the document, verbatim — which is the one way a
/// *value* reaches a diagnostic here without any of this crate's own code
/// putting it there, and an unterminated string is exactly the typo somebody
/// makes while pasting a password in. The position and the reason are what a
/// person needs and both survive; the quoted line goes.
///
/// A filter rather than a truncation at the first newline: the reason sits
/// *below* the block, so cutting at the first line would keep the position and
/// throw away what was wrong. JSON and YAML render one line and pass through
/// untouched.
fn without_quoted_source(message: &str) -> String {
    message
        .lines()
        .filter(|line| !is_gutter(line))
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_owned()
}

/// Whether a line belongs to a rendered excerpt: `  |`, `2 | text`, `  |   ^`.
fn is_gutter(line: &str) -> bool {
    line.trim_start()
        .trim_start_matches(|character: char| character.is_ascii_digit())
        .trim_start()
        .starts_with('|')
}

#[cfg(feature = "figment")]
/// Names what was there without saying what it was.
///
/// The variants that carry no payload keep figment's own wording; the ones that
/// do are reduced to their type. A length is not a secret, so
/// `InvalidLength` is left alone above.
fn kind_of(actual: &figment::error::Actual) -> &'static str {
    use figment::error::Actual;

    match actual {
        Actual::Bool(_) => "a boolean",
        Actual::Unsigned(_) => "an unsigned integer",
        Actual::Signed(_) => "a signed integer",
        Actual::Float(_) => "a float",
        Actual::Char(_) => "a character",
        Actual::Str(_) => "a string",
        Actual::Bytes(_) => "a byte string",
        Actual::Unit => "a unit",
        Actual::Option => "an option",
        Actual::NewtypeStruct => "a newtype struct",
        Actual::Seq => "a list",
        Actual::Map => "a table",
        Actual::Enum => "an enum",
        Actual::UnitVariant => "a unit variant",
        Actual::NewtypeVariant => "a newtype variant",
        Actual::TupleVariant => "a tuple variant",
        Actual::StructVariant => "a struct variant",
        // `Other` is a free-form description rather than a value, but it comes
        // from whatever produced the error, so it is not ours to vouch for.
        Actual::Other(_) => "something else",
    }
}

#[cfg(feature = "figment")]
/// Which of this crate's categories a figment failure belongs to.
fn kind_of_error(error: &figment::Error) -> ErrorKind {
    use figment::error::Kind;

    match &error.kind {
        Kind::MissingField(_) => ErrorKind::Missing,
        Kind::InvalidType(..)
        | Kind::InvalidValue(..)
        | Kind::InvalidLength(..)
        | Kind::ISizeOutOfRange(_)
        | Kind::USizeOutOfRange(_) => ErrorKind::Type,
        // figment has no dedicated parse variant: a provider that fails to read
        // its document reports a bare `Message` with no key path, whereas a
        // serde error raised during extraction always carries one.
        Kind::Message(_) if error.path.is_empty() => ErrorKind::Parse,
        Kind::Message(_) => ErrorKind::Type,
        _ => ErrorKind::Backend,
    }
}

#[cfg(any(feature = "figment", test))]
/// The key path a figment error happened at.
///
/// A missing field is named inside the kind rather than in the path, so it
/// has to be moved across for `Error::path()` to be useful.
pub(crate) fn error_path(error: &figment::Error) -> Vec<String> {
    use figment::error::Kind;

    let mut path = error.path.clone();

    if path.is_empty() {
        if let Kind::MissingField(field) = &error.kind {
            path.push(field.to_string());
        }
    }

    path
}

#[cfg(feature = "figment")]
/// A figment error as this crate's, value stripped, provenance not yet known.
///
/// **Every** road from figment to a caller goes through here, which is what
/// makes [`message`]'s stripping a property of the crate rather than of one
/// call site: a seam that parses on its own — [`Value::parse`](crate::Value::parse)
/// — has no layer to name and no `LoadSpec` to name it with, and still must not
/// be the one path that prints ``found string "hunter2"``.
///
/// The two schemaless read doors reach it the same way and for the same
/// reason. [`Snapshot::get`](crate::Snapshot::get),
/// [`Snapshot::extract`](crate::Snapshot::extract) and
/// [`Value::get_as`](crate::Value::get_as) deserialize a value already in
/// hand, so they have no spec either — and they used to render the backend's
/// message verbatim, which put the value of any mistyped key into the error.
/// Reading by path is precisely where a password lands in a numeric field.
pub(crate) fn translate(error: &figment::Error) -> Error {
    let mut translated = Error::new(kind_of_error(error), message(error));

    // Rebuilt outermost-last so `Error::path()` reads root-first.
    for segment in error_path(error).into_iter().rev() {
        translated = translated.prepend_key(segment);
    }

    translated
}

#[cfg(test)]
mod tests {
    use super::without_quoted_source;

    /// `toml`'s rendering, verbatim: the position and the reason are kept, the
    /// quoted document is not.
    #[test]
    fn a_quoted_source_line_is_dropped_and_the_reason_kept() {
        let rendered = "TOML parse error at line 2, column 25\n  \
                        |\n2 | password = \"hunter2\n  |                   ^\n\
                        invalid basic string\n";

        assert_eq!(
            without_quoted_source(rendered),
            "TOML parse error at line 2, column 25\ninvalid basic string"
        );
    }

    #[test]
    fn a_one_line_message_passes_through_untouched() {
        let rendered = "EOF while parsing an object at line 1 column 28";

        assert_eq!(without_quoted_source(rendered), rendered);
    }
}

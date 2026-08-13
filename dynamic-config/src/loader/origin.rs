//! Error translation and origin tracing: turning `figment::Error` into this
//! crate's [`Error`] with the value redacted, and naming the layer a value
//! came from.

use figment::Metadata;

use crate::error::{Error, ErrorKind, Origin};
use crate::layer::{DEFAULTS_NAME, FLAGS_NAME, OVERRIDES_NAME};

use super::{CACHED_NAME, REMOTE_PREFIX};

/// How figment names the metadata of a provider built from a string.
const INLINE_SUFFIX: &str = "source string";

/// How figment names the metadata of its environment provider.
const ENV_SUFFIX: &str = "environment variable(s)";

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
        _ => without_quoted_source(&error.to_string()),
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

/// The key path a figment error happened at.
///
/// A missing field is named inside the kind rather than in the path, so it
/// has to be moved across for `Error::path()` to be useful.
fn error_path(error: &figment::Error) -> Vec<String> {
    use figment::error::Kind;

    let mut path = error.path.clone();

    if path.is_empty() {
        if let Kind::MissingField(field) = &error.kind {
            path.push(field.to_string());
        }
    }

    path
}

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

/// Translates a figment error, preserving the key path and the source.
pub(super) fn convert(error: figment::Error, spec: &crate::source::LoadSpec<'_>) -> Error {
    let origin = refine_env(
        error.metadata.as_ref().map_or(Origin::Unknown, origin_of),
        error_path(&error).iter().map(String::as_str),
        spec.nest,
    );

    translate(&error).with_origin(origin)
}

/// Upgrades a prefix-grained environment origin to the exact variable.
///
/// figment attaches metadata per provider and the prefixed environment is
/// one provider, so the trail ends at `APP_DB_*`. But the crate holds
/// every ingredient the full name is made of — the prefix (in the origin
/// itself), the key path the question is about, and the nesting separator
/// — so the variable is *derived*: path segments uppercased and joined by
/// the separator, appended to the prefix. A naming convention rather than
/// a measurement, which is why `tests/loader.rs` pins it: if figment ever
/// changes the convention, the drift shows up there and not in a bug
/// report.
///
/// Derived, then *checked*: the composed name is only claimed when that
/// variable actually exists in the environment. An aliased value carries
/// the destination path while the variable that supplied it spells the
/// old one — deriving from the path would name a variable nobody set —
/// and the honest fallback for any composition the environment does not
/// confirm is the prefix wildcard the trail already ended at.
pub(super) fn refine_env<'a>(
    origin: Origin,
    path: impl Iterator<Item = &'a str>,
    nest: &str,
) -> Origin {
    let Origin::Env(prefix) = &origin else {
        return origin;
    };
    let Some(stem) = prefix.strip_suffix('*') else {
        return origin;
    };

    let segments: Vec<String> = path.map(str::to_ascii_uppercase).collect();

    if segments.is_empty() {
        return origin;
    }

    let variable = format!("{stem}{}", segments.join(&nest.to_ascii_uppercase()));

    if std::env::var_os(&variable).is_none() {
        return origin;
    }

    Origin::Env(variable)
}

/// Pulls the prefix out of ``"`APP_DB_` environment variable(s)"``.
///
/// figment names the environment provider after the prefix it was built with,
/// which is as specific as it gets: the provider knows the prefix, not which
/// variable under it failed.
fn env_prefix(name: &str) -> String {
    let prefix = name.trim_end_matches(ENV_SUFFIX).trim().trim_matches('`');

    if prefix.is_empty() {
        return "the environment".to_owned();
    }

    format!("{prefix}*")
}

pub(super) fn origin_of(metadata: &Metadata) -> Origin {
    // The runtime layers are named by us, so they are recognised by name rather
    // than by source — figment models both them and the environment as
    // `Source::Custom`.
    if metadata.name == DEFAULTS_NAME {
        return Origin::Runtime("default");
    }

    if metadata.name == OVERRIDES_NAME {
        return Origin::Runtime("override");
    }

    if metadata.name == FLAGS_NAME {
        return Origin::Runtime("command-line flag");
    }

    if metadata.name == CACHED_NAME {
        return Origin::Runtime("cached configuration");
    }

    // A binding names its own variable, which is the answer to the question
    // being asked — "where did this come from" is not usefully answered with
    // "a binding".
    if let Some(variable) = metadata.name.strip_prefix(crate::bindings::BINDING_PREFIX) {
        return Origin::Env(variable.to_owned());
    }

    #[cfg(feature = "dotenv")]
    if let Some(file) = metadata.name.strip_prefix(crate::dotenv::PREFIX) {
        return Origin::File(std::path::PathBuf::from(file));
    }

    if let Some(store) = metadata.name.strip_prefix(REMOTE_PREFIX) {
        return Origin::Remote(store.to_owned());
    }

    match metadata.source.as_ref() {
        Some(figment::Source::File(path)) => Origin::File(path.clone()),
        // figment names the env provider by its prefix rather than by the
        // variable that failed, so this is as specific as it gets.
        Some(figment::Source::Custom(name)) => Origin::Env(name.clone()),
        Some(_) => Origin::Inline,
        // Providers that read no file carry no source at all, only a name:
        // ``"`APP_DB_` environment variable(s)"``, `"JSON source string"`.
        // Recognising them by name is brittle by nature, which is why
        // `tests/loader.rs` asserts it rather than leaving it to be noticed in
        // a bug report.
        None if metadata.name.ends_with(ENV_SUFFIX) => Origin::Env(env_prefix(&metadata.name)),
        None if metadata.name.ends_with(INLINE_SUFFIX) => Origin::Inline,
        None => Origin::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_environment_prefix_is_recovered_from_figments_name() {
        assert_eq!(env_prefix("`APP_DB_` environment variable(s)"), "APP_DB_*");
        assert_eq!(env_prefix("environment variable(s)"), "the environment");
    }

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

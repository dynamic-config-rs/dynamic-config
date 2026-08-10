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
        _ => error.to_string(),
    }
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

/// Translates a figment error, preserving the key path and the source.
pub(super) fn convert(error: figment::Error) -> Error {
    use figment::error::Kind;

    let kind = match &error.kind {
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
    };

    // A missing field is named inside the kind rather than in the path, so it
    // has to be moved across for `Error::path()` to be useful.
    let mut path = error.path.clone();

    if path.is_empty() {
        if let Kind::MissingField(field) = &error.kind {
            path.push(field.to_string());
        }
    }

    let origin = error.metadata.as_ref().map_or(Origin::Unknown, origin_of);
    let mut translated = Error::new(kind, message(&error)).with_origin(origin);

    // Rebuilt outermost-last so `Error::path()` reads root-first.
    for segment in path.into_iter().rev() {
        translated = translated.prepend_key(segment);
    }

    translated
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
}

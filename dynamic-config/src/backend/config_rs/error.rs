//! The `config` crate's errors, as this crate's.
//!
//! Two of them, because this backend fills two seams and a failure means
//! a different thing in each: a document that will not parse is the
//! *document's* problem, and a fold that is refused is the backend's.
//!
//! **A message never carries document content.** The backend renders a
//! parse failure with the text it stopped on, and the line that failed to
//! parse is frequently the line holding the password — so only the first
//! line survives, and it goes through the same redaction every other
//! parser's message does.

use crate::error::{Error, ErrorKind};

/// A document would not parse.
pub(super) fn unparsed(error: &config_rs::ConfigError) -> Error {
    let rendered = error.to_string();
    let first = rendered.lines().next().unwrap_or_default();

    Error::new(ErrorKind::Parse, crate::loader::redacted(first))
}

/// The fold was refused.
///
/// Named by kind rather than rendered: the backend's own `Display` for a
/// type error quotes the value it found, and a layer's values are a
/// configuration.
pub(super) fn unfolded(error: &config_rs::ConfigError) -> Error {
    let reason = match error {
        config_rs::ConfigError::PathParse { .. } => "a key is not a path this engine can read",
        config_rs::ConfigError::Frozen => "the configuration is frozen",
        _ => "the resolution engine refused the layers it was given",
    };

    Error::new(ErrorKind::Backend, reason)
}

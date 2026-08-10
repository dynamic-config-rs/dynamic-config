//! The crate's single error type.
//!
//! No backend error type is ever exposed: a `figment::Error` or a
//! `serde_json::Error` is translated into an [`Error`] at the boundary, so
//! switching backends never changes a caller's signatures.

use std::fmt;
use std::path::PathBuf;

/// Broad category of a configuration failure.
///
/// Matching on this is enough to decide how to react; the human-readable
/// detail lives in the [`Display`](fmt::Display) output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    /// A configured file exists but could not be read.
    Io,
    /// A file was read but is not valid in its format.
    Parse,
    /// A required value was not supplied by any source.
    Missing,
    /// A value was supplied but cannot become the requested type.
    Type,
    /// An environment variable could not be interpreted.
    Env,
    /// Every value parsed, but the configuration as a whole was rejected.
    Invalid,
    /// A remote store could not be read.
    Remote,
    /// An encrypted file could not be decrypted.
    Decrypt,
    /// The active backend failed for a reason of its own.
    Backend,
}

impl ErrorKind {
    /// A short, stable label. Useful for metrics and log fields.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Io => "io",
            Self::Parse => "parse",
            Self::Missing => "missing",
            Self::Type => "type",
            Self::Env => "env",
            Self::Invalid => "invalid",
            Self::Remote => "remote",
            Self::Decrypt => "decrypt",
            Self::Backend => "backend",
        }
    }
}

/// Where a value came from.
///
/// Attached to errors so the first question of every configuration bug —
/// *which source set this?* — is answered by the message itself.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Origin {
    /// A file on disk.
    File(PathBuf),
    /// An environment variable, named in full.
    Env(String),
    /// An in-memory source supplied by the caller.
    Inline,
    /// A remote store, as it described itself.
    Remote(String),
    /// A value set from code: `"default"` or `"override"`.
    Runtime(&'static str),
    /// Provenance could not be determined.
    Unknown,
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File(path) => write!(f, "in {}", path.display()),
            Self::Env(name) => write!(f, "from {name}"),
            Self::Inline => f.write_str("in an inline source"),
            Self::Remote(store) => write!(f, "from {store}"),
            Self::Runtime(layer) => write!(f, "set as {layer}"),
            Self::Unknown => f.write_str("origin unknown"),
        }
    }
}

/// A configuration error.
///
/// Boxed internally so that `Result<T, Error>` stays small on the hot path —
/// `load` is called on every reload, and most calls succeed.
pub struct Error {
    inner: Box<Inner>,
}

struct Inner {
    kind: ErrorKind,
    /// Key path from the config root, outermost segment first.
    path: Vec<String>,
    origin: Origin,
    message: String,
}

impl Error {
    /// Builds an error with no path and no known origin.
    pub(crate) fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            inner: Box::new(Inner {
                kind,
                path: Vec::new(),
                origin: Origin::Unknown,
                message: message.into(),
            }),
        }
    }

    /// A configuration that parsed but failed its own validation.
    ///
    /// Called by the code `#[dynamic_config(.., validate)]` generates, from
    /// whatever `validate` resolves to at the call site.
    pub fn invalid<E: fmt::Display>(error: E) -> Self {
        Self::new(ErrorKind::Invalid, error.to_string())
    }

    /// Maps a validation result into a load failure.
    ///
    /// Takes the whole `Result` rather than the error, so the generated code is
    /// one expression regardless of what `validate` returns.
    ///
    /// # Errors
    ///
    /// If `outcome` is `Err`.
    pub fn ok_or_invalid<T, E: fmt::Display>(outcome: Result<T, E>) -> Result<(), Self> {
        outcome.map(|_| ()).map_err(Self::invalid)
    }

    /// A remote store that could not be read.
    ///
    /// For implementors of [`RemoteSource`](crate::RemoteSource), so a network
    /// failure is categorised the same way whichever store it came from.
    pub fn remote<E: fmt::Display>(error: E) -> Self {
        Self::new(ErrorKind::Remote, error.to_string())
    }

    /// A failure decrypting an encrypted config file.
    ///
    /// For [`Decryptor`](crate::Decryptor) implementations, so a scheme this
    /// crate has never heard of still reports through the same category.
    pub fn decrypt<E: fmt::Display>(error: E) -> Self {
        Self::new(ErrorKind::Decrypt, error.to_string())
    }

    /// A path whose extension names no format this build can write.
    #[must_use]
    pub fn unsupported(path: &std::path::Path) -> Self {
        Self::new(
            ErrorKind::Backend,
            "the extension names no supported format; expected `.json`, `.toml`, \
             `.yaml` or `.yml`",
        )
        .with_origin(Origin::File(path.to_owned()))
    }

    /// The error's category.
    #[must_use]
    pub fn kind(&self) -> ErrorKind {
        self.inner.kind
    }

    /// The dotted key path this error occurred at, empty at the root.
    #[must_use]
    pub fn path(&self) -> String {
        self.inner.path.join(".")
    }

    /// Where the offending value came from.
    #[must_use]
    pub fn origin(&self) -> &Origin {
        &self.inner.origin
    }

    /// The detail message, without the path or origin decoration.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.inner.message
    }

    /// Pushes a key onto the front of the path.
    ///
    /// Deserialization unwinds from the innermost field outwards, so each
    /// enclosing map prepends its own key and the path assembles itself in the
    /// right order without any of them knowing the full path.
    pub(crate) fn prepend_key(mut self, key: impl Into<String>) -> Self {
        self.inner.path.insert(0, key.into());
        self
    }

    /// Records provenance, unless something more specific is already known.
    pub(crate) fn with_origin(mut self, origin: Origin) -> Self {
        if self.inner.origin == Origin::Unknown {
            self.inner.origin = origin;
        }
        self
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.inner.path.is_empty() {
            f.write_str(&self.inner.message)?;
        } else {
            write!(f, "{}: {}", self.path(), self.inner.message)?;
        }

        if self.inner.origin != Origin::Unknown {
            write!(f, " ({})", self.inner.origin)?;
        }

        Ok(())
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Error")
            .field("kind", &self.inner.kind)
            .field("path", &self.path())
            .field("origin", &self.inner.origin)
            .field("message", &self.inner.message)
            .finish()
    }
}

impl std::error::Error for Error {}

impl serde::de::Error for Error {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Self::new(ErrorKind::Type, msg.to_string())
    }

    fn missing_field(field: &'static str) -> Self {
        Self::new(ErrorKind::Missing, "missing value").prepend_key(field)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::Error as _;

    #[test]
    fn prepend_key_assembles_the_path_outermost_first() {
        let error = Error::new(ErrorKind::Type, "boom")
            .prepend_key("max")
            .prepend_key("pool")
            .prepend_key("db");

        assert_eq!(error.path(), "db.pool.max");
    }

    #[test]
    fn display_includes_the_path_and_the_origin() {
        let error = Error::new(ErrorKind::Type, "invalid type")
            .prepend_key("port")
            .with_origin(Origin::Env("APP_DB_PORT".to_owned()));

        assert_eq!(error.to_string(), "port: invalid type (from APP_DB_PORT)");
    }

    #[test]
    fn display_omits_both_decorations_when_they_are_absent() {
        let error = Error::new(ErrorKind::Parse, "unexpected end of input");

        assert_eq!(error.to_string(), "unexpected end of input");
    }

    #[test]
    fn the_first_origin_recorded_wins() {
        let error = Error::new(ErrorKind::Type, "boom")
            .with_origin(Origin::Inline)
            .with_origin(Origin::Env("APP_X".to_owned()));

        assert_eq!(error.origin(), &Origin::Inline);
    }

    #[test]
    fn a_missing_field_carries_its_name_and_kind() {
        let error = Error::missing_field("host");

        assert_eq!(error.kind(), ErrorKind::Missing);
        assert_eq!(error.path(), "host");
    }
}

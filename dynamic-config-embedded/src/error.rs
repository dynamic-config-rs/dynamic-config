//! What can go wrong on a device, which is less than on a host.

use core::fmt;

/// Why a document could not become a configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The bytes are not valid in their format.
    Parse,
    /// They parsed, but do not fit the struct.
    Type,
    /// Every field is valid and the whole was rejected.
    Invalid,
    /// The format's feature is not enabled in this build.
    Unsupported,
}

impl ErrorKind {
    /// A short, stable label. Useful for a log line or a status register.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Parse => "parse",
            Self::Type => "type",
            Self::Invalid => "invalid",
            Self::Unsupported => "unsupported",
        }
    }
}

/// A configuration failure.
///
/// Carries a kind and a `&'static str`, and nothing else. There is no allocator
/// to build a message with, and a device's log is a line on a UART — the key
/// facts are *what kind* and *which rule*, both of which fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Error {
    kind: ErrorKind,
    message: &'static str,
}

impl Error {
    /// An error of `kind`, described by `message`.
    #[must_use]
    pub const fn new(kind: ErrorKind, message: &'static str) -> Self {
        Self { kind, message }
    }

    /// What went wrong, as a category.
    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// What went wrong, in words.
    #[must_use]
    pub const fn message(&self) -> &'static str {
        self.message
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind.as_str(), self.message)
    }
}

// `core::error::Error` rather than `std::error::Error`: stable since 1.81, and
// it means a device's error type composes with everything else the same way.
impl core::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_error_says_the_kind_and_the_rule() {
        let error = Error::new(ErrorKind::Invalid, "low_ms must be below high_ms");

        assert_eq!(error.kind(), ErrorKind::Invalid);
        assert_eq!(error.message(), "low_ms must be below high_ms");
    }

    #[test]
    fn it_is_copy_so_it_can_go_in_a_status_register() {
        let error = Error::new(ErrorKind::Parse, "bad");
        let copied = error;

        assert_eq!(error, copied);
    }
}

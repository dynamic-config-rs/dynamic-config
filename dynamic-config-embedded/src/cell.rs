//! Storage for one configuration, in a `static`, with no allocator.

use core::cell::RefCell;

use critical_section::Mutex;
use serde::de::DeserializeOwned;

use crate::error::{Error, ErrorKind};
use crate::{Format, Validate};

/// Process-wide storage for one configuration type.
///
/// Lives in a `static`, which is the only place a device has to put anything
/// that outlives a function:
///
/// ```
/// # use dynamic_config_embedded::ConfigCell;
/// # use serde::Deserialize;
/// # #[derive(Clone, Deserialize)] struct Settings { interval_ms: u32 }
/// static SETTINGS: ConfigCell<Settings> = ConfigCell::new();
/// ```
///
/// # Why a critical section
///
/// A reader has to see either the old configuration or the new one, never a
/// mixture. On a host that is an `ArcSwap`; on a device without an allocator it
/// is a few instructions with interrupts masked, which is what
/// [`critical_section`] provides and what every embedded HAL implements.
///
/// The section is held for a clone of the value and nothing else. Keep the
/// configuration struct small — which a device's configuration is — and that is
/// a memcpy with interrupts off, measured in microseconds.
pub struct ConfigCell<T, const WAITERS: usize = { crate::DEFAULT_WAITERS }> {
    inner: Mutex<RefCell<Option<T>>>,
    /// Bumped on every store. Zero means nothing has been stored yet.
    #[cfg(feature = "async")]
    notify: crate::asynchronous::Notify<WAITERS>,
}

impl<T, const WAITERS: usize> ConfigCell<T, WAITERS> {
    /// An empty cell.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            inner: Mutex::new(RefCell::new(None)),
            #[cfg(feature = "async")]
            notify: crate::asynchronous::Notify::new(),
        }
    }
}

impl<T: Clone, const WAITERS: usize> ConfigCell<T, WAITERS> {
    /// Installs `value`, replacing whatever was there.
    ///
    /// For compiled-in defaults at start-up, and for anything that builds a
    /// configuration without parsing one.
    pub fn store(&self, value: T) {
        critical_section::with(|token| {
            self.inner.borrow(token).replace(Some(value));
        });

        #[cfg(feature = "async")]
        self.notify.bump();
    }

    /// The current configuration, or `None` before anything is stored.
    ///
    /// Cloned out: there is no allocator, so there is no `Arc` to hand back.
    /// Call it once and reuse the value — two calls could straddle a store and
    /// let one piece of work observe two configurations.
    #[must_use]
    pub fn get(&self) -> Option<T> {
        critical_section::with(|token| self.inner.borrow(token).borrow().clone())
    }

    /// Whether anything has been stored.
    #[must_use]
    pub fn is_set(&self) -> bool {
        critical_section::with(|token| self.inner.borrow(token).borrow().is_some())
    }

    /// A handle that resolves each time the configuration is replaced.
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    #[must_use]
    pub fn changes(&'static self) -> crate::Changes<T, WAITERS> {
        crate::Changes::new(self)
    }

    #[cfg(feature = "async")]
    pub(crate) fn notify(&self) -> &crate::asynchronous::Notify<WAITERS> {
        &self.notify
    }
}

impl<T: Clone + DeserializeOwned + Validate, const WAITERS: usize> ConfigCell<T, WAITERS> {
    /// Parses `document` and installs it, if it is usable.
    ///
    /// Everything that can fail happens before anything is installed: a
    /// document that does not parse, does not fit, or does not validate leaves
    /// the previous configuration serving. That is the whole reason this is one
    /// call rather than parse-then-store.
    ///
    /// # Errors
    ///
    /// If the bytes are not valid in `format`, do not fit `T`, or `T` rejects
    /// itself.
    pub fn apply(&self, document: &[u8], format: Format) -> Result<(), Error> {
        let value = parse::<T>(document, format)?;

        value
            .validate()
            .map_err(|message| Error::new(ErrorKind::Invalid, message))?;

        self.store(value);

        Ok(())
    }
}

/// Parses a document without allocating.
fn parse<T: DeserializeOwned>(document: &[u8], format: Format) -> Result<T, Error> {
    // With no format feature on, `Format` has no variants and the match below
    // is empty — so nothing reads either argument.
    #[cfg(not(feature = "json"))]
    let _ = (document, format);

    match format {
        #[cfg(feature = "json")]
        Format::Json => serde_json_core::from_slice::<T>(document)
            .and_then(|(value, consumed)| {
                // The document must be *all* of the buffer. On a device the
                // bytes arrive over a link into a reused buffer, and a short
                // write leaving the tail of a longer previous document — or
                // two concatenated frames — parses cleanly as the first
                // object. Installing a configuration nobody sent is exactly
                // what "everything fallible happens before install" is for.
                if consumed == document.len() {
                    Ok(value)
                } else {
                    Err(serde_json_core::de::Error::TrailingCharacters)
                }
            })
            .map_err(|error| {
                // `serde-json-core` distinguishes a malformed document from one
                // that does not fit, and the difference is what a person
                // debugging this needs first.
                let kind = match error {
                    // A missing field or a value of the wrong shape reaches
                    // serde as a custom error; malformed bytes do not get that
                    // far.
                    serde_json_core::de::Error::InvalidType
                    | serde_json_core::de::Error::CustomError => ErrorKind::Type,
                    _ => ErrorKind::Parse,
                };

                Error::new(kind, "the document is not a configuration of this shape")
            }),

        #[allow(unreachable_patterns)]
        _ => Err(Error::new(
            ErrorKind::Unsupported,
            "the format's feature is not enabled in this build",
        )),
    }
}

impl<T, const WAITERS: usize> Default for ConfigCell<T, WAITERS> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> core::fmt::Debug for ConfigCell<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Never the value: a configuration holds whatever a device was told,
        // which on a device is as likely to be a key as anything else.
        f.debug_struct("ConfigCell").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[derive(Debug, Clone, PartialEq, Deserialize)]
    struct Settings {
        interval_ms: u32,
        verbose: bool,
    }

    impl Validate for Settings {
        fn validate(&self) -> Result<(), &'static str> {
            if self.interval_ms == 0 {
                return Err("interval_ms of zero would spin");
            }

            Ok(())
        }
    }

    #[test]
    fn an_empty_cell_answers_nothing() {
        let cell: ConfigCell<Settings> = ConfigCell::new();

        assert!(!cell.is_set());
        assert!(cell.get().is_none());
    }

    #[test]
    fn a_stored_value_comes_back() {
        let cell: ConfigCell<Settings> = ConfigCell::new();

        cell.store(Settings {
            interval_ms: 1000,
            verbose: false,
        });

        assert_eq!(cell.get().unwrap().interval_ms, 1000);
    }

    #[cfg(feature = "json")]
    #[test]
    fn a_document_replaces_the_configuration() {
        let cell: ConfigCell<Settings> = ConfigCell::new();

        cell.store(Settings {
            interval_ms: 1000,
            verbose: false,
        });

        cell.apply(br#"{"interval_ms": 250, "verbose": true}"#, Format::Json)
            .expect("the document fits");

        assert_eq!(
            cell.get().unwrap(),
            Settings {
                interval_ms: 250,
                verbose: true,
            }
        );
    }

    #[cfg(feature = "json")]
    #[test]
    fn a_document_that_does_not_parse_leaves_the_previous_one_serving() {
        let cell: ConfigCell<Settings> = ConfigCell::new();

        cell.store(Settings {
            interval_ms: 1000,
            verbose: false,
        });

        let error = cell
            .apply(b"{not json", Format::Json)
            .expect_err("that is not a document");

        assert_eq!(error.kind(), ErrorKind::Parse);
        assert_eq!(
            cell.get().unwrap().interval_ms,
            1000,
            "a bad document must not take the device's configuration with it"
        );
    }

    #[cfg(feature = "json")]
    #[test]
    fn a_document_that_fails_validation_is_refused_whole() {
        let cell: ConfigCell<Settings> = ConfigCell::new();

        cell.store(Settings {
            interval_ms: 1000,
            verbose: false,
        });

        let error = cell
            .apply(br#"{"interval_ms": 0, "verbose": true}"#, Format::Json)
            .expect_err("zero would spin");

        assert_eq!(error.kind(), ErrorKind::Invalid);
        assert_eq!(error.message(), "interval_ms of zero would spin");
        assert!(
            !cell.get().unwrap().verbose,
            "not even the fields that were fine"
        );
    }

    #[cfg(feature = "json")]
    #[test]
    fn a_document_missing_a_field_is_a_type_error_not_a_parse_error() {
        let cell: ConfigCell<Settings> = ConfigCell::new();

        let error = cell
            .apply(br#"{"interval_ms": 250}"#, Format::Json)
            .expect_err("`verbose` is missing");

        // The positive claim, not `assert_ne`: "anything but Invalid" would
        // also accept `Parse`, which is exactly the misclassification the
        // test's name promises against.
        assert_eq!(error.kind(), ErrorKind::Type);
    }

    /// `Debug` must not print the value, and checking that needs a formatter —
    /// which needs an allocator, which is why this one is host-only.
    #[cfg(feature = "std")]
    #[test]
    fn debug_never_prints_the_configuration() {
        extern crate std;

        let cell: ConfigCell<Settings> = ConfigCell::new();

        cell.store(Settings {
            interval_ms: 1234,
            verbose: true,
        });

        assert!(!std::format!("{cell:?}").contains("1234"));
    }
}

//! The slot that remembers a builder at `init`.

use crate::error::{Error, ErrorKind};

use super::Builder;

/// The builder a type was configured with, remembered at `init`.
///
/// Generated code keeps one per type — a `static` or a registry slot, the
/// same split as every other slot — so `source_of`, `check`, `prepare` and
/// the remote reload can answer for "the configuration this process runs
/// on" without being handed the builder again.
#[doc(hidden)]
pub struct Configured<T> {
    builder: std::sync::Mutex<Option<Builder<T>>>,
}

// Manual, not derived: a derive would demand `T: Default`, and the generic
// path reaches this through `Registry::entry`'s `V: Default` bound — a
// config type should not have to be `Default` to be configurable.
impl<T> Default for Configured<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Configured<T> {
    /// An empty slot.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            builder: std::sync::Mutex::new(None),
        }
    }

    /// Remembers `builder` as the type's configuration.
    pub fn set(&self, builder: Builder<T>) {
        *self
            .builder
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(builder);
    }

    /// The remembered configuration, or an error that says how to get one.
    ///
    /// # Errors
    ///
    /// When nothing was configured yet.
    pub fn get(&self, name: &str) -> Result<Builder<T>, Error> {
        self.builder
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::Backend,
                    format!(
                        "`{name}` has not been configured yet; build and \
                         install one first: `{name}::builder(\"..\")\
                         .file(..).init()?`"
                    ),
                )
            })
    }
}

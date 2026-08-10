//! Hot-reloadable configuration for `no_std` targets.
//!
//! [`dynamic-config`] reads files, searches directories and merges layers with
//! [figment]. A microcontroller has no files, no directories and no allocator,
//! and figment is `std` — so this is not that crate with a feature switched
//! off. It is the same *shape*, built from what a device actually has.
//!
//! ```rust
//! use dynamic_config_embedded::{ConfigCell, Format, Validate};
//! use serde::Deserialize;
//!
//! #[derive(Debug, Deserialize, Clone)]
//! struct Settings {
//!     interval_ms: u32,
//!     verbose: bool,
//! }
//!
//! // Accepting everything is the default; implement it to reject a
//! // configuration whose fields are individually fine and jointly wrong.
//! impl Validate for Settings {}
//!
//! static SETTINGS: ConfigCell<Settings> = ConfigCell::new();
//!
//! // Compiled-in defaults, so the device is configured before anything arrives.
//! SETTINGS.store(Settings { interval_ms: 1000, verbose: false });
//!
//! // A document from wherever this device gets one: a serial link, an MQTT
//! // message, a page of flash.
//! # #[cfg(feature = "json")]
//! SETTINGS.apply(br#"{"interval_ms": 250, "verbose": true}"#, Format::Json)?;
//!
//! # #[cfg(feature = "json")]
//! assert_eq!(SETTINGS.get().unwrap().interval_ms, 250);
//! # Ok::<(), dynamic_config_embedded::Error>(())
//! ```
//!
//! # What it keeps from the big crate
//!
//! - **A snapshot in a `static`**, replaced whole. A reader never sees a
//!   half-applied configuration.
//! - **A bad document cannot take the process down.** Parsing and validation
//!   happen before anything is installed; a failure leaves the previous
//!   configuration serving.
//! - **`changes()`**, a `Future` that resolves on the next configuration. The
//!   same generation-counter-and-wakers design as the `std` crate, which is why
//!   it drives on Embassy, RTIC, or a hand-written executor.
//! - **Validation**, through the same [`Validate`] shape.
//!
//! # What it cannot keep, and why
//!
//! | | |
//! |---|---|
//! | Files, directory search, profiles | there is no filesystem |
//! | Environment variables | there is no environment |
//! | Layered merging | figment is `std`, and merging needs a value tree that allocates |
//! | `Arc` snapshots | no allocator; readers get a `Copy` or a clone instead |
//! | Provenance (`source_of`) | there is one source, so the question does not arise |
//!
//! A device gets *one* document at a time and replaces the whole
//! configuration. That is not a reduced version of layering — it is what
//! configuring a device looks like.
//!
//! # No allocator, and no lock a reader can block on
//!
//! Storage is a `critical-section` around a plain slot: a handful of
//! instructions with interrupts masked, which is the primitive every embedded
//! HAL provides and the only one this crate needs. Readers clone the value out;
//! there is no `Arc` to hand back because there is no allocator to make one.
//!
//! That makes `T: Clone` the price of admission. For a configuration struct of
//! scalars — which is what a device's configuration is — the clone is a memcpy.
//!
//! [`dynamic-config`]: https://docs.rs/dynamic-config
//! [figment]: https://docs.rs/figment

#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(feature = "async")]
mod asynchronous;
mod cell;
mod error;

#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub use asynchronous::Changes;

/// How many tasks a [`ConfigCell`] can park by default.
///
/// Override it per cell with the second type parameter —
/// `ConfigCell<Settings, 8>` parks eight. Size it to the number of tasks
/// that genuinely await this configuration: beyond the limit, waiters evict
/// and wake each other in a churn that keeps the executor from idling.
pub const DEFAULT_WAITERS: usize = 4;
pub use cell::ConfigCell;
pub use error::{Error, ErrorKind};

/// How a document is written.
///
/// One variant today. It is an enum rather than an assumption so that a second
/// format — CBOR is the one that would earn its place on a device — does not
/// change every signature when it arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Format {
    /// JSON, via `serde-json-core`. Allocates nothing.
    #[cfg(feature = "json")]
    #[cfg_attr(docsrs, doc(cfg(feature = "json")))]
    Json,
}

/// A configuration that can reject itself.
///
/// The same idea as the `validate` argument in the `std` crate: every field can
/// be individually valid and the whole still wrong — a window that ends before
/// it starts, a buffer larger than the RAM on the part.
///
/// Implemented for every `T` by default, accepting everything, so implementing
/// it is opt-in:
///
/// ```
/// # use dynamic_config_embedded::Validate;
/// struct Settings {
///     low_ms: u32,
///     high_ms: u32,
/// }
///
/// impl Validate for Settings {
///     fn validate(&self) -> Result<(), &'static str> {
///         if self.low_ms >= self.high_ms {
///             return Err("low_ms must be below high_ms");
///         }
///
///         Ok(())
///     }
/// }
/// ```
///
/// The error is a `&'static str` rather than a `String`: there is no allocator,
/// and a fixed message is what a device can log anyway.
pub trait Validate {
    /// Rejects a configuration that is not usable.
    ///
    /// # Errors
    ///
    /// A short, static description of what is wrong with it.
    fn validate(&self) -> Result<(), &'static str> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_is_opt_in() {
        struct Anything;

        impl Validate for Anything {}

        assert!(Anything.validate().is_ok());
    }
}

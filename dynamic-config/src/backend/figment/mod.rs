//! The [`figment`](https://docs.rs/figment) crate, as a reader, a fold and
//! a source.
//!
//! This crate was built on figment through 0.8, and every piece that was
//! ported out of it — the value-string reader, the deserializer, the
//! serializer, the fold — is still proved against it. So figment is here
//! three times over: as an optional [engine](crate::engine) and
//! [reader](crate::reader), as the `Source::provider` interop seam, and as
//! a permanent dev-dependency the tests compare against.
//!
//! That last one is why this module is compiled under `test` as well as
//! under its feature: the oracles need the conversions even when the
//! feature is off.

pub(crate) mod error;
mod value;

#[cfg(feature = "figment")]
mod engine;
#[cfg(feature = "figment")]
mod reader;
#[cfg(feature = "figment")]
mod source;

#[cfg(feature = "figment")]
pub(crate) use engine::Figment as Engine;
#[cfg(feature = "figment")]
pub(crate) use reader::Figment as Reader;
#[cfg(feature = "figment")]
pub(crate) use source::section_of;

pub(crate) use value::{from_figment, to_figment};

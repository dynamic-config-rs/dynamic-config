//! Redirect macros used by the generated code.
//!
//! A proc-macro cannot see which features this crate was built with, so it
//! emits a call to one of these instead of naming the feature-gated item
//! directly. When the feature is off the redirect expands to nothing — or
//! to a `compile_error!` that says exactly what to add — rather than "no
//! such method", or worse, a runtime failure on a machine that only runs
//! the code path in production.
//!
//! The `#[cfg]` has to live *here*, in the facade: a `cfg` emitted into
//! generated code is evaluated against the user's crate features, not
//! dynamic-config's. Every macro is `#[macro_export]`, which exports at the
//! crate root regardless of module, and every path inside is `$crate::`-
//! absolute — so nothing here needs `pub`, and the split into files changes
//! nothing about how the macros are reached. One family per file:
//! [`clap`] the flag bindings, [`asynchronous`] the async loading and
//! async remote surfaces.

mod asynchronous;
mod clap;

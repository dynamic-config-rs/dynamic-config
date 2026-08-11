//! `std::sync`, or loom's mirror of it under `--cfg loom`.
//!
//! The modules whose interleavings the loom suite explores — the remote
//! fence, the wake protocol, group serialization — take their primitives
//! from here, so the model checker sees every permutation of the *real*
//! code rather than a copy that can drift. Everything else stays on `std`
//! directly: `ConfigCell`'s swap lives inside `arc-swap`, which loom cannot
//! instrument, and modelling around a black box would prove nothing.
//!
//! loom's constructors are not `const`, so the types built from these
//! primitives duplicate their `new` under `cfg(loom)` — same body, minus
//! the `const`.

#[cfg(loom)]
pub(crate) use loom::sync::atomic;
#[cfg(loom)]
pub(crate) use loom::sync::{Mutex, MutexGuard};

#[cfg(not(loom))]
pub(crate) use std::sync::atomic;
#[cfg(not(loom))]
pub(crate) use std::sync::{Mutex, MutexGuard};

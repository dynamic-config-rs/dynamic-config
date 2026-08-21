//! The crates this one can borrow machinery from, one directory each.
//!
//! Two steps of a load are pluggable — [reading](crate::reader) a document
//! and [folding](crate::engine) the layers — and a backend may fill either
//! or both. What lives here is everything about *one* backend: its value
//! conversions, its reader, its fold, and its errors.
//!
//! ```text
//! backend/
//!   config_rs/  value ─ reader ─ engine ─ error
//!   figment/    value ─ reader ─ engine ─ error ─ source
//! ```
//!
//! The shapes are the same except where a backend does more: figment has
//! a `source.rs` because it fills a third seam nothing else does —
//! `Source::provider`, where a foreign provider is one layer of a load.
//!
//! **Grouped by backend rather than by seam**, which is the axis these
//! actually change on: a backend's major release, or a decision to stop
//! carrying one, touches one directory. The seams themselves — the traits,
//! this crate's own implementations of them, and the registry a load picks
//! from — stay in `engine.rs` and `reader.rs`, beside the contract they
//! define.
//!
//! `figment` is behind the feature named after it. `config_rs` is not
//! optional: it carries the fold, and this crate has none of its own.
//! Either way, nothing outside this directory names a backend type.

pub(crate) mod config_rs;

#[cfg(any(feature = "figment", test))]
pub(crate) mod figment;

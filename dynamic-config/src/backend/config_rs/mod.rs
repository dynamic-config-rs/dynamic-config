//! The [`config`](https://docs.rs/config) crate, as a reader and a fold.
//!
//! The default engine and an opt-in reader. What it brings that this crate
//! does not have: a maintained YAML parser (`yaml-rust2`), RON and JSON5,
//! and a merge that was written by somebody else — which is what makes the
//! agreement tests worth running.
//!
//! Renamed to `config_rs` in the manifest, because the crate is called
//! `config` and this crate's attribute is `#[config(secret)]`: with the
//! plain name in scope, rustc appends "`config` is in scope, but it is a
//! crate, not an attribute" to a diagnostic that was clear without it.

mod engine;
mod error;
mod reader;
mod source;
mod value;

pub(crate) use engine::ConfigRs as Engine;
pub(crate) use reader::ConfigRs as Reader;
pub(crate) use source::layer;
pub(crate) use value::from_config_rs;

//! Procedural macro implementation for [`dynamic-config`](https://docs.rs/dynamic-config).
//!
//! Do not depend on this crate directly — it is an implementation detail of
//! `dynamic-config`, which re-exports everything here and provides the runtime
//! the generated code calls into. The two are released together and pinned to
//! the same version.
//!
//! A procedural macro can only be defined in a crate with `proc-macro = true`,
//! which is the sole reason this crate exists separately.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod args;
mod expand;

use proc_macro::TokenStream;
use syn::{parse_macro_input, ItemStruct};

/// Turns a struct into a hot-reloadable configuration snapshot.
///
/// Documented on the re-export in `dynamic_config`, which is where callers
/// should read it.
#[proc_macro_attribute]
pub fn dynamic_config(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as args::Args);
    let input = parse_macro_input!(item as ItemStruct);

    match expand::expand(args, input) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

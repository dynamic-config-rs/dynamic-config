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
use syn::ItemStruct;

/// Turns a struct into a hot-reloadable configuration snapshot.
///
/// Documented on the re-export in `dynamic_config`, which is where callers
/// should read it.
#[proc_macro_attribute]
pub fn dynamic_config(attr: TokenStream, item: TokenStream) -> TokenStream {
    // Errors are emitted *alongside* the original item, never instead of it.
    // Swallowing the struct turns one attribute mistake into a cascade of
    // "cannot find type" errors at every use site, burying the message that
    // actually matters.
    let fallback = item.clone();

    let with_original = |error: syn::Error| -> TokenStream {
        let mut output: TokenStream = error.into_compile_error().into();
        output.extend(fallback.clone());
        output
    };

    // A named, spanned message for a non-struct: syn's raw "expected `struct`"
    // does not say which attribute wanted one, or why.
    let input: ItemStruct = match syn::parse(item.clone()) {
        Ok(input) => input,
        Err(_) => {
            return with_original(syn::Error::new_spanned(
                proc_macro2::TokenStream::from(item),
                "#[dynamic_config] goes on a struct with named fields — the \
                 fields are what the configuration deserializes into; an enum \
                 or function has nowhere to put them",
            ))
        }
    };

    // The attribute takes no arguments; anything present gets the
    // migration map, next to the untouched struct.
    if let Err(error) = syn::parse::<args::Args>(attr) {
        return with_original(error);
    }

    match expand::expand(input) {
        Ok(tokens) => tokens.into(),
        Err(error) => with_original(error),
    }
}

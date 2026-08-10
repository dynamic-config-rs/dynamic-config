//! Async surface generation: `load_async`, `init_async` and `changes`, all
//! expanded through the facade's redirect.

use proc_macro2::TokenStream;
use quote::quote;
use syn::Ident;

/// The async methods, when `async` was asked for.
///
/// The async methods' signatures name `Changes`, so they cannot be hidden
/// behind an expression-level `compile_error!` the way a disabled format is.
/// The facade expands the whole block instead, and produces the actionable
/// error itself when the feature is off.
pub(super) fn async_methods(asynchronous: bool, name: &Ident) -> TokenStream {
    if asynchronous {
        quote!(::dynamic_config::__async_methods!(#name);)
    } else {
        quote! {}
    }
}

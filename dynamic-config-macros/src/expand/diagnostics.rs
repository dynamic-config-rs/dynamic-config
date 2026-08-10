//! Diagnostics generation: the redacted `Debug`, and the introspection
//! surface that reports where values come from and which keys are unknown.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Error, Fields, Ident, ItemStruct, Result};

/// Generates a `Debug` that prints `***` for the marked fields.
///
/// A secret that reaches a log is a secret that has leaked, and the usual way
/// it happens is a `#[derive(Debug)]` on a struct that grew a password field
/// later. Marking the field moves that from a review question to a compile-time
/// one.
pub(super) fn expand_redacted_debug(
    input: &ItemStruct,
    secrets: &[(Ident, String)],
) -> Result<TokenStream> {
    if secrets.is_empty() {
        return Ok(quote! {});
    }

    // Both impls would be generated, and the derived one would win the race to
    // confuse everybody. Better to say so.
    if let Some(attribute) = derives_debug(input) {
        return Err(Error::new_spanned(
            attribute,
            "`#[config(secret)]` generates a `Debug` that redacts the marked fields, so this \
             `#[derive(Debug)]` would conflict with it; remove `Debug` from the derive",
        ));
    }

    let name = &input.ident;

    let Fields::Named(fields) = &input.fields else {
        unreachable!("secrets are only collected from named fields")
    };

    let entries = fields.named.iter().map(|field| {
        let field_name = field
            .ident
            .as_ref()
            .expect("named fields always have an identifier");
        let rendered = field_name.to_string();

        if secrets.iter().any(|(ident, _)| ident == field_name) {
            quote!(.field(#rendered, &"***"))
        } else {
            quote!(.field(#rendered, &self.#field_name))
        }
    });

    let rendered_name = name.to_string();
    let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();

    Ok(quote! {
        impl #impl_generics ::core::fmt::Debug for #name #type_generics #where_clause {
            fn fmt(
                &self,
                formatter: &mut ::core::fmt::Formatter<'_>,
            ) -> ::core::fmt::Result {
                formatter
                    .debug_struct(#rendered_name)
                    #(#entries)*
                    .finish()
            }
        }
    })
}

/// The `#[derive(..)]` attribute that mentions `Debug`, if any.
fn derives_debug(input: &ItemStruct) -> Option<&syn::Attribute> {
    input.attrs.iter().find(|attribute| {
        if !attribute.path().is_ident("derive") {
            return false;
        }

        let mut found = false;

        let _ = attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("Debug") {
                found = true;
            }

            Ok(())
        });

        found
    })
}

/// `source_of`, `is_set` and `snapshot`: the read-only introspection surface.
pub(super) fn introspection_methods() -> TokenStream {
    quote! {
        /// Where the value at `path` would come from, if anything supplies it.
        ///
        /// Re-reads the sources, so it reports what the *next* load would
        /// see rather than what the current snapshot holds. `path` is
        /// dotted and relative to the section, as in `"pool.max_size"`.
        ///
        /// # Errors
        ///
        /// The same failures as [`load`](Self::load).
        pub fn source_of(
            path: &str,
        ) -> ::core::result::Result<
            ::core::option::Option<::dynamic_config::Origin>,
            ::dynamic_config::Error,
        > {
            ::dynamic_config::source_of(&Self::dynamic_config_spec(), path)
        }

        /// Whether anything supplies `path`.
        ///
        /// # Errors
        ///
        /// The same failures as [`load`](Self::load).
        pub fn is_set(path: &str) -> ::core::result::Result<bool, ::dynamic_config::Error> {
            ::dynamic_config::is_set(&Self::dynamic_config_spec(), path)
        }

        /// Resolves the section without deserializing it.
        ///
        /// For the shape a program does not know at compile time — a
        /// plugin's table, a user-defined section. Read from it with
        /// `get`, narrow it with `sub`. Everything with a known shape
        /// should go through the struct, where a typo is a compile error.
        ///
        /// # Errors
        ///
        /// The same failures as [`load`](Self::load).
        pub fn snapshot() -> ::core::result::Result<
            ::dynamic_config::Snapshot,
            ::dynamic_config::Error,
        > {
            ::dynamic_config::snapshot(&Self::dynamic_config_spec())
        }
    }
}

/// `check`: the unknown-key and would-it-load report.
pub(super) fn check_method() -> TokenStream {
    quote! {
        /// What this configuration resolves to, and whether it would load.
        ///
        /// Reports every key with the layer that supplied it, any key the
        /// struct does not name, and the reason a load would fail — without
        /// printing a single value, so the output is safe to paste.
        ///
        /// # Errors
        ///
        /// Only if the sources cannot be read or parsed at all. A
        /// configuration that would fail to deserialize is a successful
        /// report with `failure` set.
        pub fn check() -> ::core::result::Result<
            ::dynamic_config::Report,
            ::dynamic_config::Error,
        > {
            ::dynamic_config::check::<Self>(
                &Self::dynamic_config_spec(),
                Self::DYNAMIC_CONFIG_FIELDS,
            )
        }
    }
}

//! Code generation.
//!
//! The attribute declares — *this type is a configuration* — and the builder
//! configures. What gets generated here is therefore exactly the part a
//! runtime value cannot provide: the type's storage (its snapshot cell, its
//! runtime layers, its remembered configuration), the accessors over that
//! storage, and the diagnostics that follow from the *fields* (`#[config
//! (secret)]`, unknown-key detection). Everything about *sources* — files,
//! environment, caches, watching — lives on `Builder`, seeded here with the
//! statics only this expansion can name.
//!
//! The output is deliberately thin. Everything with real behaviour lives in
//! `dynamic-config` as ordinary functions that can be linted, stepped
//! through and unit tested. Generated code cannot be any of those things, so
//! there should be as little of it as possible.

mod accessors;
mod diagnostics;
mod remote;
mod schema;
mod watch;

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Error, ItemStruct, Result};

/// Builds the `impl` block that accompanies the annotated struct.
pub(crate) fn expand(mut input: ItemStruct) -> Result<TokenStream> {
    // Strips `#[config(..)]` before the struct is re-emitted: rustc knows
    // nothing about it, and leaving it in place is a hard error.
    let secrets = schema::take_field_options(&mut input)?;
    let redacted_debug = diagnostics::expand_redacted_debug(&input, &secrets)?;

    let name = &input.ident;

    // A lifetime cannot be `'static`, and the snapshot has to be: it outlives
    // every request that reads it. Type and const parameters are fine — they go
    // through the registry instead of a `static`.
    if let Some(lifetime) = input.generics.lifetimes().next() {
        return Err(Error::new_spanned(
            lifetime,
            "`#[dynamic_config]` does not support lifetime parameters, because the \
             configuration snapshot outlives every borrow that could name one",
        ));
    }

    let is_generic = !input.generics.params.is_empty();
    let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();

    // The registry keys on `TypeId`, which needs `'static`, and the slot lives
    // in a `static`, which needs `Send + Sync`. Stating both here puts the
    // unmet bound on the call site instead of somewhere inside the expansion.
    let where_clause = if is_generic {
        let mut clause = where_clause
            .cloned()
            .unwrap_or_else(|| syn::parse_quote!(where));

        clause
            .predicates
            .push(syn::parse_quote!(Self: 'static + ::core::marker::Send + ::core::marker::Sync));

        quote!(#clause)
    } else {
        quote!(#where_clause)
    };

    let known_fields = schema::field_names(&input).unwrap_or_default();
    // The *serde* names: this list reaches the cache redaction and the JSON
    // schema, both of which see the resolved tree — where a renamed field
    // lives under its rename.
    let secret_names: Vec<String> = secrets.iter().map(|(_, name)| name.clone()).collect();

    // Two shapes, chosen at compile time. A non-generic type keeps its `static`
    // and pays one atomic load per read; a generic one has no such option and
    // pays a registry lookup. Nobody pays for a feature they are not using.
    let cell_slot = accessors::cell_slot(is_generic, name, &type_generics);
    let configured_slot = accessors::configured_slot(is_generic, name, &type_generics);
    let defaults_slot = accessors::defaults_slot(is_generic);
    let overrides_slot = accessors::overrides_slot(is_generic);
    let remote_slot = accessors::remote_slot(is_generic);
    let aliases_slot = accessors::aliases_slot(is_generic);
    let bindings_slot = accessors::bindings_slot(is_generic);
    let flags_slot = accessors::flags_slot(is_generic);

    let layer_setters = accessors::layer_setters();
    let introspection_methods = diagnostics::introspection_methods(&secret_names);
    let hook_methods = watch::hook_methods();
    let defaults_and_flag_setters = accessors::defaults_and_flag_setters();
    let remote_methods = remote::remote_methods(name);
    let binding_methods = accessors::binding_methods();
    let clear_remote_method = remote::clear_remote_method();
    let clear_flags_method = accessors::clear_flags_method();
    let check_method = diagnostics::check_method();
    let clear_layer_methods = accessors::clear_layer_methods();

    Ok(quote! {
        // Re-emitted with only `#[config(..)]` removed: the attribute is
        // otherwise purely additive.
        #input

        #redacted_debug

        impl #impl_generics #name #type_generics #where_clause {
            #defaults_slot

            #remote_slot
            #aliases_slot
            #bindings_slot

            #flags_slot

            #overrides_slot

            /// Field names, for unknown-key detection.
            ///
            /// Empty when a `#[serde(flatten)]` field makes detection unsound.
            const DYNAMIC_CONFIG_FIELDS: &'static [&'static str] = &[#(#known_fields),*];

            /// Where this configuration comes from: a builder for `key`,
            /// wired to this type's storage.
            ///
            /// State the sources, then `init()` — which installs the result
            /// as the snapshot [`current`](Self::current) reads *and*
            /// remembers the builder, so `source_of`, `check`, `prepare`
            /// and the remote reload can answer for the running
            /// configuration later. Keep the builder around to
            /// [`watch`](::dynamic_config::Builder::watch) with it.
            #[must_use]
            pub fn builder(key: &str) -> ::dynamic_config::Builder<Self> {
                ::dynamic_config::Builder::new(key)
                    .with_installer(Self::replace)
                    .with_secrets(&[#(#secret_names),*])
                    .with_fields(Self::DYNAMIC_CONFIG_FIELDS)
                    .with_type_statics(
                        Self::dynamic_config_defaults(),
                        Self::dynamic_config_overrides(),
                        Self::dynamic_config_flags(),
                        Self::dynamic_config_env_bindings(),
                        Self::dynamic_config_aliases(),
                        Self::dynamic_config_remote(),
                        Self::dynamic_config_remember,
                    )
            }

            /// Remembers the builder that configured this type; see
            /// [`builder`](Self::builder).
            fn dynamic_config_remember(builder: &::dynamic_config::Builder<Self>) {
                Self::dynamic_config_configured().set(::core::clone::Clone::clone(builder));
            }

            /// The builder this type was configured with.
            ///
            /// # Errors
            ///
            /// When nothing was configured yet.
            fn dynamic_config_builder(
            ) -> ::core::result::Result<::dynamic_config::Builder<Self>, ::dynamic_config::Error>
            {
                Self::dynamic_config_configured().get(::core::stringify!(#name))
            }

            #layer_setters

            #introspection_methods

            #hook_methods

            #defaults_and_flag_setters

            #remote_methods

            #binding_methods

            #clear_remote_method

            #clear_flags_method

            #check_method

            #clear_layer_methods

            #cell_slot

            #configured_slot

            /// Loads and validates without installing, returning the swap.
            ///
            /// The fallible half of a reload, through the builder this type
            /// was configured with. A [`ReloadGroup`] runs this for every
            /// member before any of them commits, so a failure anywhere
            /// leaves every member on its previous snapshot.
            ///
            /// [`ReloadGroup`]: ::dynamic_config::ReloadGroup
            ///
            /// # Errors
            ///
            /// The same failures as a load — or the type not having been
            /// configured yet.
            pub fn prepare() -> ::core::result::Result<
                ::dynamic_config::Commit,
                ::dynamic_config::Error,
            > {
                Self::dynamic_config_builder()?.prepare()
            }

            /// Atomically swaps in a new snapshot.
            ///
            /// Readers already holding an `Arc` from an earlier
            /// [`current`](Self::current) keep their own generation.
            pub fn replace(config: Self) {
                Self::dynamic_config_cell().store(config);
            }

            /// The current snapshot.
            ///
            /// Cheap enough to call per request, but call it *once* per request
            /// and reuse the `Arc`: a reload landing between two calls would
            /// otherwise let one request observe two configurations.
            ///
            /// # Panics
            ///
            /// If nothing installed a snapshot yet — no `builder(..).init()`,
            /// no [`replace`](Self::replace). Use
            /// [`try_current`](Self::try_current) when that is a valid state.
            pub fn current() -> ::std::sync::Arc<Self> {
                Self::dynamic_config_cell().get_or_panic(::core::stringify!(#name))
            }

            /// The current snapshot, or `None` if none has been installed yet.
            pub fn try_current() -> ::core::option::Option<::std::sync::Arc<Self>> {
                Self::dynamic_config_cell().load()
            }

            // Expands to `bind_clap` when the facade has the `clap` feature,
            // and to nothing otherwise. It cannot be an expression-level guard
            // like the format redirects: the signature names a clap type.
            ::dynamic_config::__clap_methods!();

            // The async loading surface, whenever the facade has `async` —
            // there is no argument to opt in with any more, and an unused
            // `changes()` costs nothing.
            ::dynamic_config::__async_methods!(#name);

            // Emitted whenever the facade has `async`, with no argument to opt
            // in: wanting an async *store* is a different question from wanting
            // the async *loading* surface.
            ::dynamic_config::__async_remote_methods!();
        }

        impl #impl_generics ::dynamic_config::Reloadable for #name #type_generics
            #where_clause
        {
            fn prepare() -> ::core::result::Result<
                ::dynamic_config::Commit,
                ::dynamic_config::Error,
            > {
                Self::prepare()
            }

            fn name() -> &'static str {
                ::core::stringify!(#name)
            }
        }
    })
}

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

use proc_macro2::{Ident, Span, TokenStream};
use proc_macro_crate::{crate_name, FoundCrate};
use quote::quote;
use syn::{Error, ItemStruct, Result};

/// The path the generated code names the facade crate by.
///
/// `::dynamic_config` was hardcoded, which meant a renamed dependency —
/// `config = { package = "dynamic-config" }`, a perfectly ordinary thing to
/// write — expanded to a crate the user's namespace does not have.
/// `proc-macro-crate` reads the *consumer's* manifest and answers with the
/// name that is actually in scope there.
fn crate_path() -> TokenStream {
    match crate_name("dynamic-config") {
        // `Itself` means only that the manifest being compiled is the
        // facade's; it does not mean the facade's *library* is what is being
        // compiled. An example, a bin or a doctest of that same package links
        // the library as an ordinary extern crate, where `crate` is the
        // example — which is how this was caught: every example in the
        // repository stopped compiling.
        Ok(FoundCrate::Itself) if compiling_the_facade() => quote!(crate),
        Ok(FoundCrate::Itself) => quote!(::dynamic_config),
        Ok(FoundCrate::Name(name)) => {
            let ident = Ident::new(&name, Span::call_site());

            quote!(::#ident)
        }
        // Not in the dependency list at all, or a manifest that will not
        // parse. Today's behaviour stands, so what the user reads is rustc's
        // "unresolved crate `dynamic_config`" — pointing at the missing
        // dependency — rather than a panic from inside a proc macro.
        Err(_) => quote!(::dynamic_config),
    }
}

/// Whether this expansion is going into the facade's own library target.
///
/// Two sibling cases answer `FoundCrate::Itself` and must not be treated as
/// one. Cargo names each compilation unit in `CARGO_CRATE_NAME`, so an
/// example or a bin of the same package is told apart by that; a doctest is
/// not, because rustdoc runs under the library's own name — but it sets
/// `UNSTABLE_RUSTDOC_TEST_PATH`, which is the only marker there is. An
/// integration test needs neither check: `proc-macro-crate` already answers
/// `Name` for one.
///
/// Both are environment variables rather than anything a compiler
/// guarantees, so getting this wrong fails loudly at the use site — an
/// unresolved path — and never silently.
fn compiling_the_facade() -> bool {
    std::env::var_os("UNSTABLE_RUSTDOC_TEST_PATH").is_none()
        && std::env::var("CARGO_CRATE_NAME").is_ok_and(|name| name == "dynamic_config")
}

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
    // Resolved once and threaded through every helper: `crate_name` reads and
    // parses the consumer's `Cargo.toml`, and the answer cannot change
    // between two items of one expansion.
    let path = &crate_path();

    let cell_slot = accessors::cell_slot(path, is_generic, name, &type_generics);
    let configured_slot = accessors::configured_slot(path, is_generic, name, &type_generics);
    let defaults_slot = accessors::defaults_slot(path, is_generic);
    let overrides_slot = accessors::overrides_slot(path, is_generic);
    let remote_slot = accessors::remote_slot(path, is_generic);
    let aliases_slot = accessors::aliases_slot(path, is_generic);
    let bindings_slot = accessors::bindings_slot(path, is_generic);
    let flags_slot = accessors::flags_slot(path, is_generic);

    let layer_setters = accessors::layer_setters(path);
    let introspection_methods = diagnostics::introspection_methods(path, &secret_names);
    let hook_methods = watch::hook_methods(path);
    let defaults_and_flag_setters = accessors::defaults_and_flag_setters(path);
    let remote_methods = remote::remote_methods(path, name);
    let binding_methods = accessors::binding_methods(path);
    let clear_remote_method = remote::clear_remote_method();
    let clear_flags_method = accessors::clear_flags_method();
    let check_method = diagnostics::check_method(path);
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
            pub fn builder(key: &str) -> #path::Builder<Self> {
                #path::Builder::new(key)
                    .with_installer(
                        Self::dynamic_config_install,
                        Self::dynamic_config_record_failure,
                    )
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
            fn dynamic_config_remember(builder: &#path::Builder<Self>) {
                Self::dynamic_config_configured().set(::core::clone::Clone::clone(builder));
            }

            /// The builder this type was configured with.
            ///
            /// # Errors
            ///
            /// When nothing was configured yet.
            fn dynamic_config_builder(
            ) -> ::core::result::Result<#path::Builder<Self>, #path::Error>
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
                #path::Commit,
                #path::Error,
            > {
                Self::dynamic_config_builder()?.prepare()
            }

            /// Atomically swaps in a new snapshot.
            ///
            /// Readers already holding an `Arc` from an earlier
            /// [`current`](Self::current) keep their own generation. The
            /// install is recorded as `ReloadReason::Manual` — the program
            /// did it.
            pub fn replace(config: Self) {
                Self::dynamic_config_cell().store(config);
            }

            /// The builder's install door, carrying why and handing back
            /// what it installed — which is what `init_and_current`
            /// returns; see `builder`.
            fn dynamic_config_install(
                config: Self,
                reason: #path::ReloadReason,
            ) -> ::std::sync::Arc<Self> {
                Self::dynamic_config_cell().store_with(config, reason)
            }

            /// The builder's failure door: a reload that installed nothing
            /// is still a fact about this type, and `status()` reports it.
            fn dynamic_config_record_failure(error: &#path::Error) {
                Self::dynamic_config_cell().record_failure(error);
            }

            /// What is true of this configuration right now: which
            /// generation is live, when it landed, why, and how the reloads
            /// since have gone.
            ///
            /// A handful of atomic loads and **no I/O** — nothing is
            /// re-read — so an exporter can call it per scrape. It carries
            /// key paths, counts, timestamps and error kinds, and never a
            /// configured value.
            pub fn status() -> #path::ConfigStatus {
                Self::dynamic_config_cell().status()
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

            /// How many snapshots have been installed. Zero before the first.
            ///
            /// Monotonic, and the number to reach for when a hook needs a
            /// total order across concurrent reloads — the order callbacks
            /// are *called* in is not defined, this is.
            pub fn generation() -> u64 {
                Self::dynamic_config_cell().generation()
            }

            /// What is true of the installed snapshot: its generation, and
            /// when it was installed. `None` before the first install.
            ///
            /// For operators rather than for correctness. Reading it is a
            /// second load, so it can be one install away from a
            /// [`current`](Self::current) taken beside it; nothing on the
            /// read path consults it, which is why `current` is still one
            /// atomic load.
            pub fn meta() -> ::core::option::Option<#path::SnapshotMeta> {
                Self::dynamic_config_cell().meta()
            }

            // Expands to `bind_clap` when the facade has the `clap` feature,
            // and to nothing otherwise. It cannot be an expression-level guard
            // like the format redirects: the signature names a clap type.
            #path::__clap_methods!();

            // The async loading surface, whenever the facade has `async` —
            // there is no argument to opt in with any more, and an unused
            // `changes()` costs nothing.
            #path::__async_methods!(#name);

            // Emitted whenever the facade has `async`, with no argument to opt
            // in: wanting an async *store* is a different question from wanting
            // the async *loading* surface.
            #path::__async_remote_methods!();
        }

        impl #impl_generics #path::Reloadable for #name #type_generics
            #where_clause
        {
            fn prepare() -> ::core::result::Result<
                #path::Commit,
                #path::Error,
            > {
                Self::prepare()
            }

            fn name() -> &'static str {
                ::core::stringify!(#name)
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one branch the end-to-end fixture cannot reach.
    ///
    /// `tests/renamed_dependency.rs` in the facade covers `FoundCrate::Name`
    /// and the whole suite covers `Itself`; a consumer with no readable
    /// manifest at all is only reachable by pointing the resolver at one.
    /// What it must not do is panic — a proc macro that panics reports its
    /// own backtrace instead of the missing dependency.
    #[test]
    fn a_manifest_that_cannot_be_read_keeps_the_old_hardcoded_path() {
        let empty = std::env::temp_dir().join("dynamic-config-macros-no-manifest");
        std::fs::create_dir_all(&empty).expect("the scratch directory is writable");

        let restore = std::env::var_os("CARGO_MANIFEST_DIR");
        std::env::set_var("CARGO_MANIFEST_DIR", &empty);

        let path = crate_path().to_string();

        match restore {
            Some(value) => std::env::set_var("CARGO_MANIFEST_DIR", value),
            None => std::env::remove_var("CARGO_MANIFEST_DIR"),
        }

        assert_eq!(path, quote!(::dynamic_config).to_string());
    }
}

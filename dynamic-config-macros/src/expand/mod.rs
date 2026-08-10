//! Code generation.
//!
//! The output is deliberately thin. Everything with real behaviour —
//! loading, merging, storage, watching — lives in `dynamic-config` as ordinary
//! functions that can be linted, stepped through and unit tested. Generated
//! code cannot be any of those things, so there should be as little of it as
//! possible.
//!
//! The generation is split by concern — slots and accessors, persistence,
//! remote, watch, async, schema, diagnostics, sources — with [`expand`] as the
//! orchestrator that assembles the pieces in a fixed order.

mod accessors;
mod async_api;
mod diagnostics;
mod persistence;
mod remote;
mod schema;
mod source;
mod watch;

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Error, ItemStruct, Result};

use crate::args::Args;

/// Builds the `impl` block that accompanies the annotated struct.
pub(crate) fn expand(args: Args, mut input: ItemStruct) -> Result<TokenStream> {
    // Strips `#[config(..)]` before the struct is re-emitted: rustc knows
    // nothing about it, and leaving it in place is a hard error.
    let secrets = schema::take_field_options(&mut input)?;
    let redacted_debug = diagnostics::expand_redacted_debug(&input, &secrets)?;

    let name = &input.ident;
    let key = &args.key;

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

    let sources = args
        .files
        .iter()
        .map(source::source_expression)
        .collect::<Result<Vec<_>>>()?;

    // Built through the builder rather than a struct literal, so a new knob in
    // `LoadSpec` does not break every expansion at once.
    let env_call = match &args.env {
        Some(prefix) => quote!(.with_env(#prefix)),
        None => quote! {},
    };

    let search_call = match &args.search {
        Some((name, paths)) => quote! {
            .with_search(#name, &[#(#paths),*])
        },
        None => quote! {},
    };

    let profile_call = match &args.profile_env {
        Some(variable) => quote!(.with_profile_env(#variable)),
        None => quote! {},
    };

    let nest_call = match &args.nest {
        Some(separator) => quote!(.with_nest(#separator)),
        None => quote! {},
    };

    let save_impl = persistence::save_methods(args.save, key);

    let (env_files_call, env_files_guard) = if args.env_files.is_empty() {
        (quote! {}, quote! {})
    } else {
        let files = &args.env_files;

        (
            quote!(.with_env_files(&[#(#files),*])),
            quote!(::dynamic_config::__require_dotenv!();),
        )
    };

    let empty_env_call = if args.allow_empty_env {
        quote!(.with_empty_env(true))
    } else {
        quote! {}
    };

    // Two shapes, chosen at compile time. A non-generic type keeps its `static`
    // and pays one atomic load per read; a generic one has no such option and
    // pays a registry lookup. Nobody pays for a feature they are not using.
    let remote_slot = accessors::remote_slot(is_generic);
    let aliases_slot = accessors::aliases_slot(is_generic);
    let bindings_slot = accessors::bindings_slot(is_generic);
    let flags_slot = accessors::flags_slot(is_generic);

    let known_fields = schema::field_names(&input).unwrap_or_default();
    // The *serde* names: this list reaches the cache redaction and the JSON
    // schema, both of which see the resolved tree — where a renamed field
    // lives under its rename.
    let secret_names: Vec<String> = secrets.iter().map(|(_, name)| name.clone()).collect();

    let schema_impl = schema::schema_methods(args.schema, key, &secret_names);
    let cache_impl = persistence::cache_const(&args, &secret_names);

    let cell_slot = accessors::cell_slot(is_generic, name, &type_generics);
    let defaults_slot = accessors::defaults_slot(is_generic);
    let overrides_slot = accessors::overrides_slot(is_generic);

    let apply_body = persistence::apply_body(args.diff);
    let seed_recovered_previous = persistence::seed_recovered_previous(args.diff);
    let previous_slot = accessors::previous_slot(is_generic, args.diff);

    let validate_call = if args.validate {
        quote! {
            ::dynamic_config::Error::ok_or_invalid(config.validate())?;
        }
    } else {
        quote! {}
    };

    let watch_impl = if args.watch {
        watch::expand_watch(name, &args)
    } else {
        quote! {}
    };

    let async_impl = async_api::async_methods(args.asynchronous, name);

    // The method groups below are spliced into the impl block in exactly the
    // order they were written in when this was one function, so the emitted
    // token stream — and everything downstream of it, rustdoc order included —
    // is unchanged by the split.
    let layer_setters = accessors::layer_setters();
    let introspection_methods = diagnostics::introspection_methods();
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
            #previous_slot

            /// Layers this configuration is assembled from.
            const DYNAMIC_CONFIG_SOURCES: &'static [::dynamic_config::Source<'static>] =
                &[#(#sources),*];

            #defaults_slot

            #remote_slot
            #aliases_slot
            #bindings_slot

            #flags_slot

            #overrides_slot

            #cache_impl

            /// Field names, for unknown-key detection.
            ///
            /// Empty when a `#[serde(flatten)]` field makes detection unsound.
            const DYNAMIC_CONFIG_FIELDS: &'static [&'static str] = &[#(#known_fields),*];

            /// What to load, and from where.
            ///
            /// A function rather than a `const`, because it borrows the two
            /// runtime layers and a `const` may not refer to a `static`.
            fn dynamic_config_spec() -> ::dynamic_config::LoadSpec<'static> {
                ::dynamic_config::LoadSpec::new(#key, Self::DYNAMIC_CONFIG_SOURCES)
                    #search_call
                    #profile_call
                    #env_call
                    #nest_call
                    #env_files_call
                    #empty_env_call
                    .with_defaults(Self::dynamic_config_defaults())
                    .with_remote(Self::dynamic_config_remote())
                    .with_aliases(Self::dynamic_config_aliases())
                    .with_env_bindings(Self::dynamic_config_env_bindings())
                    .with_flags(Self::dynamic_config_flags())
                    .with_overrides(Self::dynamic_config_overrides())
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

            /// Reads the configured files and environment and deserializes them.
            ///
            /// A pure read: the stored snapshot is untouched. Use it to
            /// validate a configuration without publishing it.
            ///
            /// # Errors
            ///
            /// If a file cannot be parsed, a required value is missing, or a
            /// value cannot become the field's type. A file that does not exist
            /// is skipped rather than treated as an error.
            pub fn load() -> ::core::result::Result<Self, ::dynamic_config::Error> {
                let config: Self = ::dynamic_config::load(&Self::dynamic_config_spec())?;

                Self::dynamic_config_validate(config)
            }

            /// Runs the type's own `validate`, if the attribute asked for it.
            ///
            /// `validate` is resolved at the call site: an inherent method, or
            /// a trait such as `validator::Validate` that the caller has in
            /// scope. Nothing here depends on which — a crate that pinned a
            /// validation library would pin its version too.
            fn dynamic_config_validate(
                config: Self,
            ) -> ::core::result::Result<Self, ::dynamic_config::Error> {
                #validate_call

                ::core::result::Result::Ok(config)
            }

            /// Loads the configuration and installs it as the initial snapshot.
            ///
            /// Call once during startup, before anything calls
            /// [`current`](Self::current).
            ///
            /// # Errors
            ///
            /// Same as [`load`](Self::load).
            pub fn init() -> ::core::result::Result<(), ::dynamic_config::Error> {
                let failure = match Self::dynamic_config_apply() {
                    ::core::result::Result::Ok(_) => {
                        return ::core::result::Result::Ok(())
                    }
                    ::core::result::Result::Err(failure) => failure,
                };

                // Only a cold start consults the cache. A *reload* that fails
                // already has something better to fall back on: the snapshot
                // currently serving.
                let recovered = ::dynamic_config::recover::<Self>(
                    ::core::stringify!(#name),
                    &Self::dynamic_config_spec(),
                    Self::DYNAMIC_CONFIG_CACHE,
                    &failure,
                )?;

                match recovered {
                    ::core::option::Option::Some((config, snapshot)) => {
                        // Through the same gate every other path takes: a
                        // type that validates its configuration must not
                        // find its invariants suspended exactly when the
                        // configuration is stale and least trustworthy.
                        let config = Self::dynamic_config_validate(config)?;

                        #seed_recovered_previous

                        Self::replace(config);

                        ::core::result::Result::Ok(())
                    }
                    ::core::option::Option::None => ::core::result::Result::Err(failure),
                }
            }

            /// Loads and validates without installing, returning the swap.
            ///
            /// The fallible half of a reload. A [`ReloadGroup`] runs this for
            /// every member before any of them commits, so a failure anywhere
            /// leaves every member on its previous snapshot.
            ///
            /// [`ReloadGroup`]: ::dynamic_config::ReloadGroup
            ///
            /// # Errors
            ///
            /// The same failures as [`load`](Self::load).
            pub fn prepare() -> ::core::result::Result<
                ::dynamic_config::Commit,
                ::dynamic_config::Error,
            > {
                let config = Self::load()?;

                ::core::result::Result::Ok(::std::boxed::Box::new(move || {
                    Self::replace(config);
                }))
            }

            /// Loads and installs, reporting what changed when `diff` is on.
            ///
            /// One path for `init`, `init_async` and the watcher, so all three
            /// agree on validation and on seeding the diff baseline.
            fn dynamic_config_apply(
            ) -> ::core::result::Result<::core::option::Option<::std::string::String>, ::dynamic_config::Error>
            {
                #apply_body
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
            /// If neither [`init`](Self::init) nor [`replace`](Self::replace)
            /// has run. Use [`try_current`](Self::try_current) when that is a
            /// valid state.
            pub fn current() -> ::std::sync::Arc<Self> {
                Self::dynamic_config_cell().get_or_panic(::core::stringify!(#name))
            }

            /// The current snapshot, or `None` if none has been installed yet.
            pub fn try_current() -> ::core::option::Option<::std::sync::Arc<Self>> {
                Self::dynamic_config_cell().load()
            }

            #watch_impl

            #save_impl

            #async_impl

            // Expands to `bind_clap` when the facade has the `clap` feature,
            // and to nothing otherwise. It cannot be an expression-level guard
            // like the format redirects: the signature names a clap type.
            ::dynamic_config::__clap_methods!();

            // Emitted whenever the facade has `async`, with no argument to opt
            // in: wanting an async *store* is a different question from wanting
            // the async *loading* surface.
            ::dynamic_config::__async_remote_methods!();

            #env_files_guard

            #schema_impl
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

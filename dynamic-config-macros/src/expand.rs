//! Code generation.
//!
//! The output is deliberately thin. Everything with real behaviour —
//! loading, merging, storage, watching — lives in `dynamic-config` as ordinary
//! functions that can be linted, stepped through and unit tested. Generated
//! code cannot be any of those things, so there should be as little of it as
//! possible.

use std::path::Path;

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Error, Fields, Ident, ItemStruct, LitStr, Result};

use crate::args::Args;

/// Field attribute this macro consumes, e.g. `#[config(secret)]`.
const FIELD_ATTRIBUTE: &str = "config";

/// Builds the `impl` block that accompanies the annotated struct.
pub(crate) fn expand(args: Args, mut input: ItemStruct) -> Result<TokenStream> {
    // Strips `#[config(..)]` before the struct is re-emitted: rustc knows
    // nothing about it, and leaving it in place is a hard error.
    let secrets = take_field_options(&mut input)?;
    let redacted_debug = expand_redacted_debug(&input, &secrets)?;

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
        .map(source_expression)
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

    let save_impl = if args.save {
        quote! {
            /// Writes this configuration to `path`, atomically.
            ///
            /// The format comes from the extension, and the output is nested
            /// under this type's section key — so what comes out can be read
            /// straight back in.
            ///
            /// Written through a temporary file and renamed, because the
            /// watcher is very likely watching that directory and a partial
            /// file would look exactly like a broken edit.
            ///
            /// Secrets are written in the clear. `#[config(secret)]` keeps a
            /// value out of logs; it cannot keep it out of a file this was
            /// asked to write. On Unix the file is created `0600`.
            ///
            /// # Errors
            ///
            /// If the extension names no supported format, the value cannot be
            /// serialized, or the file cannot be written.
            pub fn save(
                &self,
                path: impl ::core::convert::AsRef<::std::path::Path>,
            ) -> ::core::result::Result<(), ::dynamic_config::Error> {
                let path = path.as_ref();
                let format = ::dynamic_config::Format::from_path(path)?;

                ::dynamic_config::save(self, path, format, #key)
            }

            /// Writes this configuration to `path`, refusing if it exists.
            ///
            /// For a setup wizard or an `--init` subcommand: replacing a
            /// configuration somebody wrote by hand, silently, is the one
            /// failure mode those have.
            ///
            /// # Errors
            ///
            /// If the file exists, the extension names no supported format,
            /// the value cannot be serialized, or the file cannot be written.
            pub fn save_new(
                &self,
                path: impl ::core::convert::AsRef<::std::path::Path>,
            ) -> ::core::result::Result<(), ::dynamic_config::Error> {
                let path = path.as_ref();
                let format = ::dynamic_config::Format::from_path(path)?;

                ::dynamic_config::save_new(self, path, format, #key)
            }

            // Not `#[cfg(feature = "decrypt")]` here: a `cfg` in generated
            // code is evaluated against the *user's* crate features, not
            // dynamic-config's — so the method would appear and disappear with
            // a feature the user happens to define, regardless of what the
            // facade was built with. The redirect macro is expanded inside
            // dynamic-config, where the feature actually lives.
            ::dynamic_config::__save_encrypted_method!(#key);
        }
    } else {
        quote! {}
    };

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
    let remote_slot = slot(
        is_generic,
        quote!(dynamic_config_remote),
        quote!(::dynamic_config::Remote),
        quote!(::dynamic_config::Remote::new()),
        quote!(::dynamic_config::Remote),
        "The remote store's document, layered over the files.",
    );

    let aliases_slot = slot(
        is_generic,
        quote!(dynamic_config_aliases),
        quote!(::dynamic_config::Aliases),
        quote!(::dynamic_config::Aliases::new()),
        quote!(::dynamic_config::Aliases),
        "Old key paths that still resolve.",
    );

    let bindings_slot = slot(
        is_generic,
        quote!(dynamic_config_env_bindings),
        quote!(::dynamic_config::EnvBindings),
        quote!(::dynamic_config::EnvBindings::new()),
        quote!(::dynamic_config::EnvBindings),
        "Fields bound to environment variables by name.",
    );

    let flags_slot = slot(
        is_generic,
        quote!(dynamic_config_flags),
        quote!(::dynamic_config::Layer),
        quote!(::dynamic_config::Layer::new()),
        quote!(::dynamic_config::Layer),
        "Values from the command line, layered over the environment.",
    );

    let known_fields = field_names(&input).unwrap_or_default();
    let secret_names: Vec<String> = secrets.iter().map(ToString::to_string).collect();

    // Opt-in like `save`, and for the same reason: the method needs a trait the
    // user has to derive. A `where Self: JsonSchema` clause cannot express that
    // — rustc rejects an inherent method whose bound a concrete `Self` does not
    // meet, at the definition rather than at the call.
    let schema_impl = if args.schema {
        quote! {
            ::dynamic_config::__schema_methods!(#key, &[#(#secret_names),*]);
        }
    } else {
        quote! {}
    };

    let cache_impl = match &args.cache {
        Some(path) => {
            let mode = args
                .cache_mode
                .as_ref()
                .map_or_else(|| "full".to_owned(), syn::LitStr::value);

            quote! {
                /// Where the last configuration that worked is kept.
                const DYNAMIC_CONFIG_CACHE: ::core::option::Option<(
                    &'static str,
                    &'static str,
                    &'static [&'static str],
                )> = ::core::option::Option::Some((
                    #path,
                    #mode,
                    &[#(#secret_names),*],
                ));
            }
        }
        None => quote! {
            /// No cache: a start that cannot read its configuration fails.
            const DYNAMIC_CONFIG_CACHE: ::core::option::Option<(
                &'static str,
                &'static str,
                &'static [&'static str],
            )> = ::core::option::Option::None;
        },
    };

    let cell_slot = slot(
        is_generic,
        quote!(dynamic_config_cell),
        quote!(::dynamic_config::ConfigCell<Self>),
        quote!(::dynamic_config::ConfigCell::<#name #type_generics>::new()),
        quote!(::dynamic_config::ConfigCell<#name #type_generics>),
        "Process-wide storage for the current snapshot.",
    );

    let defaults_slot = slot(
        is_generic,
        quote!(dynamic_config_defaults),
        quote!(::dynamic_config::Layer),
        quote!(::dynamic_config::Layer::new()),
        quote!(::dynamic_config::Layer),
        "Values consulted only when no file or variable supplies a key.",
    );

    let overrides_slot = slot(
        is_generic,
        quote!(dynamic_config_overrides),
        quote!(::dynamic_config::Layer),
        quote!(::dynamic_config::Layer::new()),
        quote!(::dynamic_config::Layer),
        "Values that win over the files and the environment alike.",
    );

    let apply_body = if args.diff {
        quote! {
            // One resolve, both deserialized and compared: reporting what
            // changed costs no extra file reads.
            let current = ::dynamic_config::snapshot(&Self::dynamic_config_spec())?;

            let config = match current.extract::<Self>() {
                ::core::result::Result::Ok(config) => Self::dynamic_config_validate(config)?,
                // A snapshot drops figment's provenance, so an error here is
                // re-raised through the full loader to name the file at fault.
                ::core::result::Result::Err(_) => Self::load()?,
            };

            // Poisoning is recovered from rather than propagated: the slot
            // holds one `Option<Snapshot>` with no invariant a panic could
            // break, and turning every later reload into a panic would be a
            // poor trade.
            let previous = Self::dynamic_config_previous()
                .lock()
                .unwrap_or_else(::std::sync::PoisonError::into_inner)
                .replace(current.clone());

            Self::replace(config);
            ::dynamic_config::__write_cache(&current, Self::DYNAMIC_CONFIG_CACHE);

            ::core::result::Result::Ok(
                // `None` means "first apply, nothing to compare against";
                // a comparison that happened always has something to say.
                previous.map(|previous| {
                    ::dynamic_config::__summarize_changes(&previous, &current)
                }),
            )
        }
    } else {
        quote! {
            Self::replace(Self::load()?);

            if Self::DYNAMIC_CONFIG_CACHE.is_some() {
                // Resolved a second time, because without `diff` the load did
                // not keep the tree around. Only when a cache is configured.
                if let ::core::result::Result::Ok(current) = Self::snapshot() {
                    ::dynamic_config::__write_cache(&current, Self::DYNAMIC_CONFIG_CACHE);
                }
            }

            ::core::result::Result::Ok(::core::option::Option::None)
        }
    };

    let previous_slot = if args.diff {
        slot(
            is_generic,
            quote!(dynamic_config_previous),
            quote!(::std::sync::Mutex<::core::option::Option<::dynamic_config::Snapshot>>),
            quote!(::std::sync::Mutex::new(::core::option::Option::None)),
            quote!(::std::sync::Mutex<::core::option::Option<::dynamic_config::Snapshot>>),
            "The last resolved section, kept so a reload can say what moved.",
        )
    } else {
        quote! {}
    };

    let validate_call = if args.validate {
        quote! {
            ::dynamic_config::Error::ok_or_invalid(config.validate())?;
        }
    } else {
        quote! {}
    };

    let watch_impl = if args.watch {
        expand_watch(name, &args)
    } else {
        quote! {}
    };

    // The async methods' signatures name `Changes`, so they cannot be hidden
    // behind an expression-level `compile_error!` the way a disabled format is.
    // The facade expands the whole block instead, and produces the actionable
    // error itself when the feature is off.
    let async_impl = if args.asynchronous {
        quote!(::dynamic_config::__async_methods!(#name);)
    } else {
        quote! {}
    };

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

            /// Sets a fallback for `path`, used only when nothing else supplies it.
            ///
            /// Dotted for nested fields, as in `"pool.max_size"`. Takes effect
            /// on the next `load()`.
            ///
            /// # Errors
            ///
            /// If `path` is unusable or `value` cannot be serialized.
            pub fn set_default<__DynamicConfigValue>(
                path: &str,
                value: __DynamicConfigValue,
            ) -> ::core::result::Result<(), ::dynamic_config::Error>
            where
                __DynamicConfigValue: ::dynamic_config::__private::serde::Serialize,
            {
                Self::dynamic_config_defaults().set(path, value)
            }

            /// Sets `path` above every file and variable.
            ///
            /// This is the layer for tests and for a `--set key=value` flag.
            /// Takes effect on the next `load()`.
            ///
            /// # Errors
            ///
            /// If `path` is unusable or `value` cannot be serialized.
            pub fn set_override<__DynamicConfigValue>(
                path: &str,
                value: __DynamicConfigValue,
            ) -> ::core::result::Result<(), ::dynamic_config::Error>
            where
                __DynamicConfigValue: ::dynamic_config::__private::serde::Serialize,
            {
                Self::dynamic_config_overrides().set(path, value)
            }

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

            /// Registers a callback for every later reload.
            ///
            /// The callback receives the outgoing and incoming snapshots, in
            /// that order, and runs on whichever thread performed the reload —
            /// the watcher thread, usually. Keep it short, and do not call
            /// [`replace`](Self::replace) from inside one.
            ///
            /// Installing the first snapshot is not a reload, so `init()` does
            /// not fire callbacks.
            pub fn on_reload<__DynamicConfigHook>(hook: __DynamicConfigHook)
            where
                __DynamicConfigHook: ::core::ops::Fn(
                        &::std::sync::Arc<Self>,
                        &::std::sync::Arc<Self>,
                    ) + ::core::marker::Send
                    + ::core::marker::Sync
                    + 'static,
            {
                Self::dynamic_config_cell().on_reload(hook);
            }

            /// Sets `path` from a command-line flag, above the environment.
            ///
            /// `None` is a no-op, so an absent flag leaves the files alone —
            /// which is what makes this safe to call unconditionally for every
            /// flag a program defines.
            ///
            /// # Errors
            ///
            /// If `path` is unusable or `value` cannot be serialized.
            pub fn set_flag<__DynamicConfigValue>(
                path: &str,
                value: ::core::option::Option<__DynamicConfigValue>,
            ) -> ::core::result::Result<(), ::dynamic_config::Error>
            where
                __DynamicConfigValue: ::dynamic_config::__private::serde::Serialize,
            {
                match value {
                    ::core::option::Option::Some(value) => {
                        Self::dynamic_config_flags().set(path, value)
                    }
                    ::core::option::Option::None => ::core::result::Result::Ok(()),
                }
            }

            /// Applies every `key=value` string, as from a `--set` flag.
            ///
            /// The values are read the way environment variables are, so
            /// `--set db.port=5432` and `APP_DB_PORT=5432` mean the same thing.
            ///
            /// # Errors
            ///
            /// If an assignment has no `=`, or its key is not a usable path.
            pub fn set_assignments<__DynamicConfigItems, __DynamicConfigItem>(
                assignments: __DynamicConfigItems,
            ) -> ::core::result::Result<(), ::dynamic_config::Error>
            where
                __DynamicConfigItems: ::core::iter::IntoIterator<Item = __DynamicConfigItem>,
                __DynamicConfigItem: ::core::convert::AsRef<str>,
            {
                Self::dynamic_config_flags().set_assignments(assignments)
            }

            /// Installs a remote store to read configuration from.
            ///
            /// Nothing is fetched here: call
            /// [`refresh_remote`](Self::refresh_remote) for that. Installing a
            /// source drops whatever the previous one had fetched.
            pub fn set_remote(source: impl ::dynamic_config::RemoteSource) {
                Self::dynamic_config_remote().set(source);
            }

            /// Reads the remote store, and keeps what came back.
            ///
            /// Takes effect on the next `load()`. Nothing here touches the
            /// network on any other call — a round trip on every configuration
            /// read would be indefensible.
            ///
            /// # Errors
            ///
            /// If no source is installed, if it is async — use
            /// `refresh_remote_async` — or if the fetch fails.
            pub fn refresh_remote() -> ::core::result::Result<(), ::dynamic_config::Error> {
                Self::dynamic_config_remote().refresh()
            }

            /// Installs a document a watch pushed, and reloads.
            ///
            /// The sink a remote watch loop calls. Everything a file change
            /// would do happens here too — validation, the reload hooks, the
            /// diff, the cache — because it is the same code path, reached with
            /// a document instead of a filesystem event.
            ///
            /// A failure leaves the previous snapshot serving, again exactly as
            /// a bad file edit does. The document itself stays installed: it is
            /// what the store currently says, and dropping it would hand the
            /// files back the decision without anyone asking.
            ///
            /// # Errors
            ///
            /// If the resulting configuration does not load or validate.
            pub fn apply_remote(
                document: ::dynamic_config::Fetched,
            ) -> ::core::result::Result<(), ::dynamic_config::Error> {
                Self::dynamic_config_remote().install(document);

                match Self::dynamic_config_apply() {
                    ::core::result::Result::Ok(summary) => {
                        ::dynamic_config::__log_remote_reload(
                            ::core::stringify!(#name),
                            summary.as_deref(),
                        );

                        ::core::result::Result::Ok(())
                    }
                    ::core::result::Result::Err(error) => {
                        ::dynamic_config::__log_remote_failure(
                            ::core::stringify!(#name),
                            &error,
                        );

                        ::core::result::Result::Err(error)
                    }
                }
            }

            /// Keeps an old key path working after a rename.
            ///
            /// `from` is the path in files written before the rename; `to` is
            /// where the field lives now. The alias fills a gap rather than
            /// overriding: a file that has been updated wins over one that has
            /// not, whatever order they merge in.
            ///
            /// The old path stops counting as an unknown key, so
            /// [`check`](Self::check) reports it as an alias while a genuine
            /// typo still shows up with a suggestion.
            ///
            /// # Errors
            ///
            /// If either path names nothing, or they are the same path.
            pub fn alias(
                from: &str,
                to: &str,
            ) -> ::core::result::Result<(), ::dynamic_config::Error> {
                Self::dynamic_config_aliases().add(from, to)
            }

            /// Drops every alias made by [`alias`](Self::alias).
            pub fn clear_aliases() {
                Self::dynamic_config_aliases().clear();
            }

            /// Binds a field to an environment variable by name.
            ///
            /// For the variables that are not yours to name — `PORT` from the
            /// platform, `DATABASE_URL` from a convention older than this
            /// program, `REDIS_URL` from an add-on. The prefixed environment
            /// layer covers the rest.
            ///
            /// The variable is read at every load, so a reload sees a change to
            /// it, and one that is not set contributes nothing. Binding the
            /// same path twice replaces the first binding.
            ///
            /// # Errors
            ///
            /// If `path` names nothing — empty, or with an empty segment.
            pub fn bind_env(
                path: &str,
                variable: &str,
            ) -> ::core::result::Result<(), ::dynamic_config::Error> {
                Self::dynamic_config_env_bindings().bind(path, variable)
            }

            /// Drops every binding made by [`bind_env`](Self::bind_env).
            pub fn clear_env_bindings() {
                Self::dynamic_config_env_bindings().clear();
            }

            /// Drops the fetched document, so the next load sees no remote layer.
            pub fn clear_remote() {
                Self::dynamic_config_remote().clear();
            }

            /// Drops every value set by [`set_flag`](Self::set_flag) or
            /// [`set_assignments`](Self::set_assignments).
            pub fn clear_flags() {
                Self::dynamic_config_flags().clear();
            }

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

            /// Drops every value set by [`set_default`](Self::set_default).
            pub fn clear_defaults() {
                Self::dynamic_config_defaults().clear();
            }

            /// Drops every value set by [`set_override`](Self::set_override).
            pub fn clear_overrides() {
                Self::dynamic_config_overrides().clear();
            }

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
                    ::core::option::Option::Some(config) => {
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

/// One process-wide slot, as a `static` or through the registry.
///
/// The two differ only in where the value lives; every caller sees the same
/// `&'static V` either way, so nothing downstream has to know which shape it
/// got.
fn slot(
    is_generic: bool,
    accessor: TokenStream,
    returned: TokenStream,
    initializer: TokenStream,
    stored: TokenStream,
    documentation: &str,
) -> TokenStream {
    if is_generic {
        return quote! {
            #[doc = #documentation]
            fn #accessor() -> &'static #returned {
                // Shared by every instantiation — a `static` in a function body
                // is not monomorphized — with `TypeId` telling them apart.
                static REGISTRY: ::dynamic_config::Registry = ::dynamic_config::Registry::new();

                REGISTRY.entry::<Self, #returned>()
            }
        };
    }

    quote! {
        #[doc = #documentation]
        fn #accessor() -> &'static #returned {
            static SLOT: #stored = #initializer;

            &SLOT
        }
    }
}

/// The field names a configuration section may legitimately carry.
///
/// Follows `#[serde(rename = "..")]`, since that is the name the file uses.
/// Returns `None` when any field is `#[serde(flatten)]`: a flattened field
/// absorbs keys the outer struct never names, so reporting those as typos would
/// be worse than reporting nothing.
fn field_names(input: &ItemStruct) -> Option<Vec<String>> {
    let Fields::Named(fields) = &input.fields else {
        return None;
    };

    let mut names = Vec::new();

    for field in &fields.named {
        let mut renamed = None;
        let mut aliases = Vec::new();
        let mut flattened = false;

        for attribute in &field.attrs {
            if !attribute.path().is_ident("serde") {
                continue;
            }

            // Best effort: serde's attribute grammar is larger than this, and
            // anything unrecognised simply leaves the field under its own name.
            let _ = attribute.parse_nested_meta(|meta| {
                if meta.path.is_ident("flatten") {
                    flattened = true;
                }

                if meta.path.is_ident("alias") {
                    // An alias is a name the file may legitimately use, so it
                    // must not be reported as an unknown key.
                    if let Ok(value) = meta.value() {
                        if let Ok(name) = value.parse::<LitStr>() {
                            aliases.push(name.value());
                        }
                    }
                }

                if meta.path.is_ident("rename") {
                    if let Ok(value) = meta.value() {
                        if let Ok(name) = value.parse::<LitStr>() {
                            renamed = Some(name.value());
                        }
                    }
                }

                Ok(())
            });
        }

        if flattened {
            return None;
        }

        names.push(renamed.unwrap_or_else(|| {
            field
                .ident
                .as_ref()
                .expect("named fields always have an identifier")
                .to_string()
        }));
        names.append(&mut aliases);
    }

    Some(names)
}

/// Reads and removes every `#[config(..)]` field attribute.
///
/// Returns the fields marked `secret`.
fn take_field_options(input: &mut ItemStruct) -> Result<Vec<Ident>> {
    let Fields::Named(fields) = &mut input.fields else {
        // Nothing to read, and nothing that could have been marked. A tuple or
        // unit struct is a perfectly good config type otherwise.
        return Ok(Vec::new());
    };

    let mut secrets = Vec::new();

    for field in &mut fields.named {
        let mut is_secret = false;
        let mut error = None;

        field.attrs.retain(|attribute| {
            if !attribute.path().is_ident(FIELD_ATTRIBUTE) {
                return true;
            }

            let parsed = attribute.parse_nested_meta(|meta| {
                if meta.path.is_ident("secret") {
                    is_secret = true;

                    return Ok(());
                }

                Err(meta.error("unknown option; the only one is `secret`"))
            });

            if let Err(parse_error) = parsed {
                error.get_or_insert(parse_error);
            }

            false
        });

        if let Some(error) = error {
            return Err(error);
        }

        if is_secret {
            let name = field
                .ident
                .clone()
                .expect("named fields always have an identifier");

            secrets.push(name);
        }
    }

    Ok(secrets)
}

/// Generates a `Debug` that prints `***` for the marked fields.
///
/// A secret that reaches a log is a secret that has leaked, and the usual way
/// it happens is a `#[derive(Debug)]` on a struct that grew a password field
/// later. Marking the field moves that from a review question to a compile-time
/// one.
fn expand_redacted_debug(input: &ItemStruct, secrets: &[Ident]) -> Result<TokenStream> {
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

        if secrets.contains(field_name) {
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

/// Builds one `Source` expression, choosing the format from the extension.
///
/// The format goes through a redirect macro in the facade rather than naming
/// `Format::Toml` directly: a proc-macro cannot see which features the facade
/// was built with, and the redirect turns a disabled feature into a compile
/// error that says which one to enable.
fn source_expression(file: &LitStr) -> Result<TokenStream> {
    let path = file.value();

    if path.is_empty() {
        return Err(Error::new(
            file.span(),
            "config file path must not be empty",
        ));
    }

    // `secrets.json.age` is JSON that happens to be encrypted, so the format
    // comes from the extension *under* the suffix and the source is a different
    // kind. A bare `secrets.age` names no format and is rejected below as an
    // unsupported `.age` extension, with a message that spells out the
    // `.json.age` shape it should have had.
    let encrypted = path
        .rsplit_once('.')
        .is_some_and(|(inner, suffix)| suffix.eq_ignore_ascii_case("age") && inner.contains('.'));

    let named = if encrypted {
        path.rsplit_once('.')
            .map_or(path.clone(), |(inner, _)| inner.to_owned())
    } else {
        path.clone()
    };

    let extension = Path::new(&named)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);

    let format = match extension.as_deref() {
        Some("json") => quote!(::dynamic_config::__format_json!()),
        Some("toml") => quote!(::dynamic_config::__format_toml!()),
        Some("yaml" | "yml") => quote!(::dynamic_config::__format_yaml!()),

        Some(extension) => {
            return Err(Error::new(
                file.span(),
                format!(
                    "unsupported config extension `.{extension}`; \
                     supported: `.json`, `.toml`, `.yaml`, `.yml`, \
                     each optionally with `.age` for an encrypted file"
                ),
            ));
        }

        None => {
            return Err(Error::new(
                file.span(),
                "config file has no extension; the format is inferred from it",
            ));
        }
    };

    if encrypted {
        return Ok(quote!(
            ::dynamic_config::__source_encrypted!(#file, #format)
        ));
    }

    Ok(quote!(::dynamic_config::Source::file(#file, #format)))
}

/// Generates `start_watch()`.
fn expand_watch(name: &Ident, args: &Args) -> TokenStream {
    let debounce = args.debounce;

    let mode = match args.poll_interval {
        Some(interval) => quote! {
            ::dynamic_config::watch::WatchMode::Poll {
                interval: ::core::time::Duration::from_millis(#interval),
            }
        },
        None => quote!(::dynamic_config::watch::WatchMode::Native),
    };

    quote! {
        /// Starts a background thread that reloads the snapshot when one of the
        /// configured files changes.
        ///
        /// **Dropping the returned handle stops the watcher.** Bind it for as
        /// long as the configuration should stay live, or call `.detach()` to
        /// watch for the rest of the process:
        ///
        /// ```ignore
        /// Config::start_watch()?.detach();          // a server
        /// let _watch = Config::start_watch()?;      // a test, a subcommand
        /// ```
        ///
        /// Calling this more than once is a no-op — later calls return a handle
        /// that owns nothing, so dropping it leaves the first watcher running.
        ///
        /// A reload that fails is reported and the previous snapshot is kept,
        /// so an invalid edit degrades to "no change" rather than a crash.
        ///
        /// # Errors
        ///
        /// If the notification backend cannot start, if no directory holding a
        /// configured file can be watched, or if the thread cannot be spawned.
        pub fn start_watch() -> ::std::io::Result<::dynamic_config::watch::WatchHandle> {
            ::dynamic_config::__spawn_watch!(
                ::core::stringify!(#name),
                Self::dynamic_config_spec(),
                ::core::time::Duration::from_millis(#debounce),
                #mode,
                Self::dynamic_config_apply,
            )
        }
    }
}

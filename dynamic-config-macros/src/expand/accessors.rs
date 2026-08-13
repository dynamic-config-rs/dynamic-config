//! Slot and accessor generation: the process-wide storage each expansion
//! carries, and the methods that fill or clear its runtime layers.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Ident, TypeGenerics};

/// One process-wide slot, as a `static` or through the registry.
///
/// The two differ only in where the value lives; every caller sees the same
/// `&'static V` either way, so nothing downstream has to know which shape it
/// got.
fn slot(
    path: &TokenStream,
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
                static REGISTRY: #path::Registry = #path::Registry::new();

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

/// The slot holding the remote store's document.
pub(super) fn remote_slot(path: &TokenStream, is_generic: bool) -> TokenStream {
    slot(
        path,
        is_generic,
        quote!(dynamic_config_remote),
        quote!(#path::Remote),
        quote!(#path::Remote::new()),
        quote!(#path::Remote),
        "The remote store's document, layered over the files.",
    )
}

/// The slot holding the key aliases.
pub(super) fn aliases_slot(path: &TokenStream, is_generic: bool) -> TokenStream {
    slot(
        path,
        is_generic,
        quote!(dynamic_config_aliases),
        quote!(#path::Aliases),
        quote!(#path::Aliases::new()),
        quote!(#path::Aliases),
        "Old key paths that still resolve.",
    )
}

/// The slot holding the by-name environment bindings.
pub(super) fn bindings_slot(path: &TokenStream, is_generic: bool) -> TokenStream {
    slot(
        path,
        is_generic,
        quote!(dynamic_config_env_bindings),
        quote!(#path::EnvBindings),
        quote!(#path::EnvBindings::new()),
        quote!(#path::EnvBindings),
        "Fields bound to environment variables by name.",
    )
}

/// The slot holding the command-line flag layer.
pub(super) fn flags_slot(path: &TokenStream, is_generic: bool) -> TokenStream {
    slot(
        path,
        is_generic,
        quote!(dynamic_config_flags),
        quote!(#path::Layer),
        quote!(#path::Layer::new()),
        quote!(#path::Layer),
        "Values from the command line, layered over the environment.",
    )
}

/// The slot holding the current snapshot.
pub(super) fn cell_slot(
    path: &TokenStream,
    is_generic: bool,
    name: &Ident,
    type_generics: &TypeGenerics<'_>,
) -> TokenStream {
    slot(
        path,
        is_generic,
        quote!(dynamic_config_cell),
        quote!(#path::ConfigCell<Self>),
        quote!(#path::ConfigCell::<#name #type_generics>::new()),
        quote!(#path::ConfigCell<#name #type_generics>),
        "Process-wide storage for the current snapshot.",
    )
}

/// The slot remembering the builder this type was configured with.
pub(super) fn configured_slot(
    path: &TokenStream,
    is_generic: bool,
    name: &Ident,
    type_generics: &TypeGenerics<'_>,
) -> TokenStream {
    slot(
        path,
        is_generic,
        quote!(dynamic_config_configured),
        quote!(#path::Configured<Self>),
        quote!(#path::Configured::<#name #type_generics>::new()),
        quote!(#path::Configured<#name #type_generics>),
        "The builder this type was configured with, remembered at `init`.",
    )
}

/// The slot holding the defaults layer.
pub(super) fn defaults_slot(path: &TokenStream, is_generic: bool) -> TokenStream {
    slot(
        path,
        is_generic,
        quote!(dynamic_config_defaults),
        quote!(#path::Layer),
        quote!(#path::Layer::new()),
        quote!(#path::Layer),
        "Values consulted only when no file or variable supplies a key.",
    )
}

/// The slot holding the overrides layer.
pub(super) fn overrides_slot(path: &TokenStream, is_generic: bool) -> TokenStream {
    slot(
        path,
        is_generic,
        quote!(dynamic_config_overrides),
        quote!(#path::Layer),
        quote!(#path::Layer::new()),
        quote!(#path::Layer),
        "Values that win over the files and the environment alike.",
    )
}

/// `set_default` and `set_override`: the two per-path layer setters.
pub(super) fn layer_setters(path: &TokenStream) -> TokenStream {
    quote! {
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
        ) -> ::core::result::Result<(), #path::Error>
        where
            __DynamicConfigValue: #path::__private::serde::Serialize,
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
        ) -> ::core::result::Result<(), #path::Error>
        where
            __DynamicConfigValue: #path::__private::serde::Serialize,
        {
            Self::dynamic_config_overrides().set(path, value)
        }
    }
}

/// `set_defaults`, `set_flag` and `set_assignments`: the bulk setters.
pub(super) fn defaults_and_flag_setters(path: &TokenStream) -> TokenStream {
    quote! {
        /// Seeds the defaults layer from a whole struct at once.
        ///
        /// The typed alternative to per-path `set_default` calls:
        ///
        /// ```ignore
        /// Config::set_defaults(&Config::default())?;
        /// ```
        ///
        /// Generic rather than bound to `Self`, so a configuration type
        /// that does not implement `Serialize` can still seed its
        /// defaults from any serializable map or mirror struct.
        ///
        /// # Errors
        ///
        /// If the value does not serialize, or serializes to something
        /// other than a map.
        pub fn set_defaults<__DynamicConfigDefaults>(
            value: &__DynamicConfigDefaults,
        ) -> ::core::result::Result<(), #path::Error>
        where
            __DynamicConfigDefaults: #path::__private::serde::Serialize,
        {
            Self::dynamic_config_defaults().set_struct(value)
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
        ) -> ::core::result::Result<(), #path::Error>
        where
            __DynamicConfigValue: #path::__private::serde::Serialize,
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
        ) -> ::core::result::Result<(), #path::Error>
        where
            __DynamicConfigItems: ::core::iter::IntoIterator<Item = __DynamicConfigItem>,
            __DynamicConfigItem: ::core::convert::AsRef<str>,
        {
            Self::dynamic_config_flags().set_assignments(assignments)
        }
    }
}

/// `alias`, `bind_env` and their clearers: the rename and by-name layers.
pub(super) fn binding_methods(path: &TokenStream) -> TokenStream {
    quote! {
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
        /// `from` may name the section the key moved out of —
        /// `alias("db::timeout", "timeout")` — read from this
        /// configuration's own documents, where every top-level key is
        /// already parsed. `to` may not: the only section this type loads is
        /// its own, which is also why the type that owns the key *today* is
        /// the one that declares where it used to live.
        ///
        /// # Errors
        ///
        /// If either path names nothing, if `to` names a section, or if they
        /// are the same path.
        pub fn alias(
            from: &str,
            to: &str,
        ) -> ::core::result::Result<(), #path::Error> {
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
        ) -> ::core::result::Result<(), #path::Error> {
            Self::dynamic_config_env_bindings().bind(path, variable)
        }

        /// Drops every binding made by [`bind_env`](Self::bind_env).
        pub fn clear_env_bindings() {
            Self::dynamic_config_env_bindings().clear();
        }
    }
}

/// `clear_flags`, on its own so it keeps its place in the emitted impl.
pub(super) fn clear_flags_method() -> TokenStream {
    quote! {
        /// Drops every value set by [`set_flag`](Self::set_flag) or
        /// [`set_assignments`](Self::set_assignments).
        pub fn clear_flags() {
            Self::dynamic_config_flags().clear();
        }
    }
}

/// `clear_defaults` and `clear_overrides`.
pub(super) fn clear_layer_methods() -> TokenStream {
    quote! {
        /// Drops every value set by [`set_default`](Self::set_default).
        pub fn clear_defaults() {
            Self::dynamic_config_defaults().clear();
        }

        /// Drops every value set by [`set_override`](Self::set_override).
        pub fn clear_overrides() {
            Self::dynamic_config_overrides().clear();
        }
    }
}

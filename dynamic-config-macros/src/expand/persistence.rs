//! Save and cache generation: writing a configuration back out, and keeping
//! the last one that worked.

use proc_macro2::TokenStream;
use quote::quote;
use syn::LitStr;

use crate::args::Args;

/// `save` and `save_new`, plus the encrypted variant behind its redirect.
pub(super) fn save_methods(save: bool, key: &LitStr) -> TokenStream {
    if save {
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
    }
}

/// The `DYNAMIC_CONFIG_CACHE` constant, configured or explicitly absent.
pub(super) fn cache_const(args: &Args, secret_names: &[String]) -> TokenStream {
    match &args.cache {
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
    }
}

/// The body of `dynamic_config_apply`, with or without the diff.
pub(super) fn apply_body(diff: bool) -> TokenStream {
    if diff {
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
    }
}

/// What `init` does with a recovered snapshot.
///
/// With `diff`, a recovery seeds the baseline so the first reload
/// afterwards reports what actually moved — recovering used to leave the
/// baseline empty and the first diff silent.
pub(super) fn seed_recovered_previous(diff: bool) -> TokenStream {
    if diff {
        quote! {
            *Self::dynamic_config_previous()
                .lock()
                .unwrap_or_else(::std::sync::PoisonError::into_inner) =
                ::core::option::Option::Some(snapshot);
        }
    } else {
        quote! {
            let _ = snapshot;
        }
    }
}

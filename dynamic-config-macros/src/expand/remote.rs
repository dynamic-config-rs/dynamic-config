//! Remote store generation: installing a source, refreshing it, and applying
//! a document a watch pushed.

use proc_macro2::TokenStream;
use quote::quote;
use syn::Ident;

/// `set_remote`, `refresh_remote` and `apply_remote`.
pub(super) fn remote_methods(name: &Ident) -> TokenStream {
    quote! {
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
    }
}

/// `clear_remote`, on its own so it keeps its place in the emitted impl.
pub(super) fn clear_remote_method() -> TokenStream {
    quote! {
        /// Drops the fetched document, so the next load sees no remote layer.
        pub fn clear_remote() {
            Self::dynamic_config_remote().clear();
        }
    }
}

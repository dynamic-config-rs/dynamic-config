//! Remote store generation: installing a source, refreshing it, and applying
//! a document a watch pushed.

use proc_macro2::TokenStream;
use quote::quote;
use syn::Ident;

/// `set_remote`, `refresh_remote` and `remote_sink`.
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
        /// Takes effect on the next reload. Nothing here touches the
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

        /// A fenced door for a remote watch loop's pushes.
        ///
        /// Take the sink *after* [`set_remote`](Self::set_remote) — it
        /// remembers which source was installed — and hand it to the watch
        /// loop's callback. [`apply`](::dynamic_config::RemoteSink::apply)
        /// installs the document and reloads through this type's
        /// configured builder; a sink whose source has since been replaced
        /// refuses, so a stale watcher cannot overwrite the store that
        /// followed it. Take it **once, where the loop starts** — a sink
        /// taken per delivery reads that moment's generation and fences
        /// nothing.
        ///
        /// No source need be installed first: a program that only ever
        /// watches never calls `set_remote`, and its sink works — the
        /// "after" above orders the two calls when both exist, it does not
        /// require the first.
        #[must_use]
        pub fn remote_sink() -> ::dynamic_config::RemoteSink {
            ::dynamic_config::RemoteSink::new(
                Self::dynamic_config_remote(),
                Self::dynamic_config_remote_reload,
                ::core::stringify!(#name),
            )
        }

        /// One reload through the remembered builder; see `remote_sink`.
        fn dynamic_config_remote_reload() -> ::core::result::Result<(), ::dynamic_config::Error>
        {
            Self::dynamic_config_builder()?.reload()
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

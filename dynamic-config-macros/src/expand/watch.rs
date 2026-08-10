//! Watch generation: the background reload thread and the hooks that observe
//! every reload it performs.

use proc_macro2::TokenStream;
use quote::quote;
use syn::Ident;

use crate::args::Args;

/// Generates `start_watch()`.
pub(super) fn expand_watch(name: &Ident, args: &Args) -> TokenStream {
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
        /// Calling this while a watcher for this type is already running —
        /// including one that was [`detach`]ed — is an error
        /// (`ErrorKind::AlreadyExists`): a second handle could only mislead,
        /// and the first watcher keeps running. Generic configurations are
        /// watched per instantiation: `Db<Postgres>` and `Db<Mysql>` each get
        /// their own watcher.
        ///
        /// [`detach`]: ::dynamic_config::watch::WatchHandle::detach
        ///
        /// A reload that fails is reported and the previous snapshot is kept,
        /// so an invalid edit degrades to "no change" rather than a crash.
        ///
        /// # Errors
        ///
        /// If a watcher for this type is already running, if the notification
        /// backend cannot start, if no directory holding a configured file can
        /// be watched, or if the thread cannot be spawned.
        pub fn start_watch() -> ::std::io::Result<::dynamic_config::watch::WatchHandle>
        where
            // Watchers are registered by `TypeId`, the identity that tells
            // `Db<Postgres>` apart from `Db<Mysql>` — a *name* keyed both as
            // "Db", and the second silently watched nothing. `TypeId` needs
            // `'static`, which a configuration type in a `static` already is;
            // the bound is spelled out so a violation reads as itself.
            Self: 'static,
        {
            ::dynamic_config::__spawn_watch!(
                ::core::any::TypeId::of::<Self>(),
                ::core::stringify!(#name),
                Self::dynamic_config_spec(),
                ::core::time::Duration::from_millis(#debounce),
                #mode,
                Self::dynamic_config_apply,
            )
        }
    }
}

/// `on_reload` and `on_reload_scoped`: the callbacks a reload fires.
pub(super) fn hook_methods() -> TokenStream {
    quote! {
        /// Registers a callback for every later reload, for the life of
        /// the process.
        ///
        /// The callback receives the outgoing and incoming snapshots, in
        /// that order, and runs on whichever thread performed the reload —
        /// the watcher thread, usually. Keep it short, and do not call
        /// [`replace`](Self::replace) from inside one.
        ///
        /// It cannot be removed — use
        /// [`on_reload_scoped`](Self::on_reload_scoped) for anything whose
        /// life is shorter than the process. A callback that panics is
        /// caught and reported; the remaining callbacks and the watcher
        /// survive.
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

        /// [`on_reload`](Self::on_reload), scoped: dropping the returned
        /// guard unregisters the callback.
        ///
        /// For tests, plugins, and anything else that is torn down while
        /// the process keeps running — the permanent variant would keep a
        /// dead subsystem's callback firing forever.
        #[must_use = "dropping the guard unregisters the hook; bind it for \
                      as long as the hook should fire"]
        pub fn on_reload_scoped<__DynamicConfigHook>(
            hook: __DynamicConfigHook,
        ) -> ::dynamic_config::HookGuard<Self>
        where
            __DynamicConfigHook: ::core::ops::Fn(
                    &::std::sync::Arc<Self>,
                    &::std::sync::Arc<Self>,
                ) + ::core::marker::Send
                + ::core::marker::Sync
                + 'static,
        {
            Self::dynamic_config_cell().on_reload_scoped(hook)
        }
    }
}

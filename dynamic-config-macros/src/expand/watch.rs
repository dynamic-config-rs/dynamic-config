//! Reload hook generation: `on_reload` and `on_reload_scoped`.
//!
//! Watching itself moved to `Builder::watch` — the hooks stay generated
//! because they hang off the type's storage, not its sources.

use proc_macro2::TokenStream;
use quote::quote;

/// `on_reload` and `on_reload_scoped`: the callbacks a reload fires.
pub(super) fn hook_methods(path: &TokenStream) -> TokenStream {
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
        ) -> #path::HookGuard<Self>
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

        /// [`on_reload`](Self::on_reload), told *why*.
        ///
        /// The callback receives a `ReloadEvent`: both snapshots, the
        /// `ReloadReason` — which file changed, a remote push, a manual
        /// reload, a recovery — and the install's `SnapshotMeta`. Same
        /// list, same registration order, same panic isolation as the pair
        /// form.
        ///
        /// One difference, and it is why this form exists: **the first
        /// install fires too**, with `previous: None`. `on_reload` has
        /// nowhere in its signature to say "there was none", so it stays
        /// silent for `init()`; this form says it.
        pub fn on_reload_with<__DynamicConfigHook>(hook: __DynamicConfigHook)
        where
            __DynamicConfigHook: ::core::ops::Fn(
                    &#path::ReloadEvent<Self>,
                ) + ::core::marker::Send
                + ::core::marker::Sync
                + 'static,
        {
            Self::dynamic_config_cell().on_reload_with(hook);
        }

        /// [`on_reload_with`](Self::on_reload_with), scoped: dropping the
        /// returned guard unregisters the callback.
        #[must_use = "dropping the guard unregisters the hook; bind it for \
                      as long as the hook should fire"]
        pub fn on_reload_with_scoped<__DynamicConfigHook>(
            hook: __DynamicConfigHook,
        ) -> #path::HookGuard<Self>
        where
            __DynamicConfigHook: ::core::ops::Fn(
                    &#path::ReloadEvent<Self>,
                ) + ::core::marker::Send
                + ::core::marker::Sync
                + 'static,
        {
            Self::dynamic_config_cell().on_reload_with_scoped(hook)
        }
    }
}

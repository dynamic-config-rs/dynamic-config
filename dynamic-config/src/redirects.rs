//! Redirect macros used by the generated code.
//!
//! A proc-macro cannot see which features this crate was built with, so it
//! emits a call to one of these instead of naming `Format::Toml` directly.
//! When the feature is off the redirect expands to a `compile_error!` that says
//! exactly what to add — rather than "no variant named `Toml`", or worse, a
//! runtime failure on a machine that only runs the code path in production.
//!
//! The `#[cfg]` has to live *here*, in the facade: a `cfg` emitted into
//! generated code is evaluated against the user's crate features, not
//! dynamic-config's. Every macro is `#[macro_export]`, which exports at the
//! crate root regardless of module, and every path inside is `$crate::`-
//! absolute — so this module needs no `pub` and changes nothing about how the
//! macros are reached.

/// Not public API.
///
/// Expands to `bind_clap` when the `clap` feature is on, and to nothing when it
/// is not. An item-level macro rather than an expression-level redirect,
/// because the signature names a clap type.
#[cfg(feature = "clap")]
#[macro_export]
#[doc(hidden)]
macro_rules! __clap_methods {
    () => {
        /// Copies clap arguments into the flags layer, by
        /// `(argument id, key path)`.
        ///
        /// Only arguments that came from the command line are taken: clap's own
        /// `default_value` is indistinguishable from a typed flag in
        /// `ArgMatches`, and letting one outrank a configuration file would
        /// invert the precedence order.
        ///
        /// # Errors
        ///
        /// If a key path is unusable, or an argument is not valid UTF-8.
        pub fn bind_clap(
            matches: &$crate::__private::clap::ArgMatches,
            bindings: &[(&str, &str)],
        ) -> ::core::result::Result<(), $crate::Error> {
            Self::dynamic_config_flags().bind_clap(matches, bindings)
        }
    };
}

/// Not public API.
#[cfg(not(feature = "clap"))]
#[macro_export]
#[doc(hidden)]
macro_rules! __clap_methods {
    () => {};
}

/// Not public API.
///
/// Expands to the async half of the generated `impl`. It lives here rather than
/// in the proc-macro because these method *signatures* mention tokio types, and
/// a signature cannot be hidden behind an expression-level `compile_error!`.
#[cfg(feature = "async")]
#[macro_export]
#[doc(hidden)]
macro_rules! __async_methods {
    ($name:ident) => {
        /// Reads the configuration without blocking the async executor.
        ///
        /// # Errors
        ///
        /// Same as `load`, plus if the blocking task is cancelled.
        pub async fn load_async() -> ::core::result::Result<Self, $crate::Error> {
            Self::dynamic_config_builder()?.load_async().await
        }

        /// Loads the configuration and installs it as the initial snapshot,
        /// without blocking the async executor.
        ///
        /// # Errors
        ///
        /// Same as `load_async`.
        pub async fn init_async() -> ::core::result::Result<(), $crate::Error> {
            Self::dynamic_config_builder()?.init_async().await
        }

        /// A handle woken by every later reload.
        ///
        /// The snapshot current at this call counts as already seen, so the
        /// first `changed().await` waits for the *next* reload. Read the value
        /// you start from with `current()`.
        ///
        /// Runtime-agnostic: it is a `Future`, so tokio, async-std, smol and a
        /// hand-written executor all drive it the same way. Unlike `current()`
        /// it never panics — a handle taken before `init()` simply waits for
        /// the first snapshot.
        pub fn changes() -> $crate::Changes<Self> {
            Self::dynamic_config_cell().changes()
        }
    };
}

/// Not public API.
///
/// The async half of the remote API. Its own redirect rather than part of
/// `__async_methods!`, because a program can want an async *store* without
/// wanting the async *loading* surface — they are different axes.
#[cfg(feature = "async")]
#[macro_export]
#[doc(hidden)]
macro_rules! __async_remote_methods {
    () => {
        /// Installs an async remote store to read configuration from.
        ///
        /// Nothing is fetched here; call
        /// [`refresh_remote_async`](Self::refresh_remote_async) for that.
        /// Installing a source drops whatever the previous one had fetched.
        pub fn set_remote_async(source: impl $crate::AsyncRemoteSource) {
            Self::dynamic_config_remote().set_async(source);
        }

        /// Reads the remote store, and keeps what came back.
        ///
        /// Works with a blocking source too, so swapping one implementation for
        /// the other is not a breaking change for the caller.
        ///
        /// # Errors
        ///
        /// If no source is installed, or the fetch fails.
        pub async fn refresh_remote_async() -> ::core::result::Result<(), $crate::Error> {
            Self::dynamic_config_remote().refresh_async().await
        }
    };
}

/// Not public API.
#[cfg(not(feature = "async"))]
#[macro_export]
#[doc(hidden)]
macro_rules! __async_remote_methods {
    () => {};
}

/// Not public API.
#[cfg(not(feature = "async"))]
#[macro_export]
#[doc(hidden)]
macro_rules! __async_methods {
    ($name:ident) => {};
}

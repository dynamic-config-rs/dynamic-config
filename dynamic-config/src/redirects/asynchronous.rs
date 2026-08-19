//! The async redirects: the loading surface and the remote surface,
//! separately — a program can want an async *store* without wanting the
//! async *loading* methods; they are different axes.

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

        /// `changes()` widened to refusals: a stream of `Event`s —
        /// installs *and* reloads that kept the previous snapshot.
        /// The push half of `status()`.
        pub fn events() -> $crate::Events<Self> {
            Self::dynamic_config_cell().events()
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

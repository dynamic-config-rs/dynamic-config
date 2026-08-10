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
/// Expands to `save_encrypted` when the `decrypt` feature is on, and to
/// nothing when it is not — nothing rather than a compile error, because
/// `save` alone is fully legitimate without decryption, and the method's very
/// signature names [`Encryptor`], which only exists with the feature.
///
/// It lives here rather than in the proc-macro because a `#[cfg]` emitted into
/// generated code is evaluated against the *user's* crate features; this macro
/// is expanded inside dynamic-config, where `decrypt` actually lives.
///
/// [`Encryptor`]: crate::Encryptor
#[cfg(feature = "decrypt")]
#[macro_export]
#[doc(hidden)]
macro_rules! __save_encrypted_method {
    ($key:expr) => {
        /// Writes this configuration to `path`, encrypted.
        ///
        /// The counterpart to reading a `secrets.json.age`. The format comes
        /// from the extension *under* the `.age` suffix, so
        /// `secrets.json.age` is JSON.
        ///
        /// The encryptor is passed here rather than installed process-wide,
        /// because *who may read this file* is a decision about this write.
        ///
        /// # Errors
        ///
        /// If the name resolves to no supported format, the value cannot be
        /// serialized, encryption fails, or the file cannot be written.
        pub fn save_encrypted(
            &self,
            path: impl ::core::convert::AsRef<::std::path::Path>,
            encryptor: &dyn $crate::Encryptor,
        ) -> ::core::result::Result<(), $crate::Error> {
            let path = path.as_ref();
            let format = $crate::Format::from_path(path)?;

            $crate::save_encrypted(self, path, format, $key, encryptor)
        }
    };
}

/// Not public API.
#[cfg(not(feature = "decrypt"))]
#[macro_export]
#[doc(hidden)]
macro_rules! __save_encrypted_method {
    ($key:expr) => {};
}

/// Not public API.
///
/// Nothing when this build can read `.env` files, and a compile error naming
/// the feature when it cannot.
#[cfg(feature = "dotenv")]
#[macro_export]
#[doc(hidden)]
macro_rules! __require_dotenv {
    () => {};
}

/// Not public API.
#[cfg(not(feature = "dotenv"))]
#[macro_export]
#[doc(hidden)]
macro_rules! __require_dotenv {
    () => {
        ::core::compile_error!(
            "dynamic-config: `env_files` in #[dynamic_config(..)] requires the `dotenv` \
             feature; add features = [\"dotenv\"] to your dynamic-config dependency"
        );
    };
}

/// Not public API.
///
/// An encrypted source when this build can decrypt, and a compile error naming
/// the feature when it cannot — the same treatment a `.toml` file gets without
/// the `toml` feature, for the same reason: a silent runtime failure on the one
/// machine that has an encrypted file is worse than a build that will not start.
#[cfg(feature = "decrypt")]
#[macro_export]
#[doc(hidden)]
macro_rules! __source_encrypted {
    ($path:expr, $format:expr) => {
        $crate::Source::encrypted($path, $format)
    };
}

/// Not public API.
#[cfg(not(feature = "decrypt"))]
#[macro_export]
#[doc(hidden)]
macro_rules! __source_encrypted {
    ($path:expr, $format:expr) => {{
        ::core::compile_error!(
            "dynamic-config: a `.age` config file needs decryption support; \
             add features = [\"age\"] to your dynamic-config dependency"
        );

        $crate::Source::file($path, $format)
    }};
}

/// Not public API.
#[cfg(feature = "json")]
#[macro_export]
#[doc(hidden)]
macro_rules! __format_json {
    () => {
        $crate::Format::Json
    };
}

/// Not public API.
#[cfg(not(feature = "json"))]
#[macro_export]
#[doc(hidden)]
macro_rules! __format_json {
    () => {
        ::core::compile_error!(
            "dynamic-config: `.json` files require the `json` feature; \
             add features = [\"json\"] to your dynamic-config dependency"
        )
    };
}

/// Not public API.
#[cfg(feature = "toml")]
#[macro_export]
#[doc(hidden)]
macro_rules! __format_toml {
    () => {
        $crate::Format::Toml
    };
}

/// Not public API.
#[cfg(not(feature = "toml"))]
#[macro_export]
#[doc(hidden)]
macro_rules! __format_toml {
    () => {
        ::core::compile_error!(
            "dynamic-config: `.toml` files require the `toml` feature; \
             add features = [\"toml\"] to your dynamic-config dependency"
        )
    };
}

/// Not public API.
#[cfg(feature = "yaml")]
#[macro_export]
#[doc(hidden)]
macro_rules! __format_yaml {
    () => {
        $crate::Format::Yaml
    };
}

/// Not public API.
#[cfg(not(feature = "yaml"))]
#[macro_export]
#[doc(hidden)]
macro_rules! __format_yaml {
    () => {
        ::core::compile_error!(
            "dynamic-config: `.yaml` and `.yml` files require the `yaml` feature; \
             add features = [\"yaml\"] to your dynamic-config dependency"
        )
    };
}

/// Not public API.
#[cfg(feature = "watch")]
#[macro_export]
#[doc(hidden)]
macro_rules! __spawn_watch {
    ($($argument:tt)*) => {
        $crate::watch::spawn_with($($argument)*)
    };
}

/// Not public API.
#[cfg(not(feature = "watch"))]
#[macro_export]
#[doc(hidden)]
macro_rules! __spawn_watch {
    ($($argument:tt)*) => {
        ::core::compile_error!(
            "dynamic-config: `watch` in #[dynamic_config(..)] requires the `watch` feature; \
             add features = [\"watch\"] to your dynamic-config dependency"
        )
    };
}

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
/// Expands to `schema` when the `schema` feature is on, and to nothing when it
/// is not. An item-level macro rather than an expression-level redirect,
/// because the `where` clause names a schemars trait.
#[cfg(feature = "schema")]
#[macro_export]
#[doc(hidden)]
macro_rules! __schema_methods {
    ($key:expr, $secrets:expr) => {
        /// A JSON Schema for the *file* this section lives in.
        ///
        /// Not the struct's schema: the struct is one section, and a config
        /// file is a map of them, so this is the struct's schema wrapped under
        /// its key. Fields marked `#[config(secret)]` carry `writeOnly`, which
        /// is how JSON Schema says *not for reading back*.
        ///
        /// Combine several with `dynamic_config::schema::merge` when more than
        /// one config type shares a file. See that module for what the schema
        /// deliberately leaves out, and for how each format wires one up.
        pub fn schema() -> ::dynamic_config::__private::serde_json::Value
        where
            Self: ::dynamic_config::__private::schemars::JsonSchema,
        {
            let generated = ::dynamic_config::__private::schemars::schema_for!(Self);

            ::dynamic_config::schema::section(
                $key,
                ::core::convert::Into::into(generated),
                $secrets,
            )
        }
    };
}

/// Not public API.
#[cfg(not(feature = "schema"))]
#[macro_export]
#[doc(hidden)]
macro_rules! __schema_methods {
    ($key:expr, $secrets:expr) => {
        ::core::compile_error!(
            "dynamic-config: `schema` in #[dynamic_config(..)] requires the `schema` feature; \
             add features = [\"schema\"] to your dynamic-config dependency"
        );
    };
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
            $crate::load_async(Self::dynamic_config_spec()).await
        }

        /// Loads the configuration and installs it as the initial snapshot,
        /// without blocking the async executor.
        ///
        /// # Errors
        ///
        /// Same as `load_async`.
        pub async fn init_async() -> ::core::result::Result<(), $crate::Error> {
            $crate::off_thread(Self::dynamic_config_apply).await?;

            ::core::result::Result::Ok(())
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
    ($name:ident) => {
        ::core::compile_error!(
            "dynamic-config: `async` in #[dynamic_config(..)] requires the `async` feature \
             (or `tokio`, which implies it); add features = [\"async\"] to your \
             dynamic-config dependency"
        );
    };
}

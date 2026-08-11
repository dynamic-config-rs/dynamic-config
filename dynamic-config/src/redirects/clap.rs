//! The clap redirect: `bind_clap`, only when the feature can back it.

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

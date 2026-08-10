//! Compile-fail tests.
//!
//! Every diagnostic the macro emits is locked down here, because an error
//! message is part of the API: a user meets it far more often than they read
//! the docs, and a refactor that degrades one is a regression like any other.
//!
//! Regenerate the expected output after an intentional change:
//!
//! ```text
//! TRYBUILD=overwrite cargo test -p dynamic-config --test ui --features json
//! ```
//!
//! The expected output is tied to a rustc version, so this suite runs on stable
//! only — the MSRV job in CI skips it.

#![cfg(feature = "json")]

#[test]
fn argument_diagnostics() {
    trybuild::TestCases::new().compile_fail("tests/ui/*.rs");
}

/// A diagnostic that only exists when a feature is *off* needs a build with it
/// off to be seen, so it lives in its own directory and its own test rather
/// than in the glob above.
///
/// ```text
/// cargo test -p dynamic-config --no-default-features --features json --test ui
/// ```
#[test]
#[cfg(not(feature = "decrypt"))]
fn feature_diagnostics() {
    trybuild::TestCases::new().compile_fail("tests/ui-no-decrypt/*.rs");
}

/// The generated `save_encrypted` must follow the *facade's* features, not the
/// caller's. The scratch crate trybuild compiles has no features at all, which
/// is every real user's situation — a `#[cfg]` in generated code would compile
/// the method away there.
#[test]
#[cfg(all(feature = "json", feature = "decrypt"))]
fn generated_methods_follow_the_facade_features() {
    trybuild::TestCases::new().pass("tests/ui-pass/*.rs");
}

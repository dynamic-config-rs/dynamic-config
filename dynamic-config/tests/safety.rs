//! Rejecting a bad configuration, and saying what a good one changed.

#![cfg(feature = "json")]

use dynamic_config::{dynamic_config, snapshot, ChangeKind, ErrorKind, Format, LoadSpec, Source};
use serde::Deserialize;

// ─────────────────────────────────────────────
// validate
// ─────────────────────────────────────────────

/// A configuration that parses but can still be nonsense: `max` below `min`.
///
/// One type per test: the override layer lives in a `static`, so sharing a type
/// would let these two race each other through it.
macro_rules! pool {
    ($name:ident) => {
        #[dynamic_config]
        #[derive(Debug, Deserialize)]
        struct $name {
            min_size: u16,
            max_size: u16,
        }

        impl $name {
            /// An inherent method here; `validator::Validate` would work the
            /// same way.
            fn validate(&self) -> Result<(), String> {
                if self.min_size > self.max_size {
                    return Err(format!(
                        "min_size ({}) is above max_size ({})",
                        self.min_size, self.max_size
                    ));
                }

                Ok(())
            }

            /// The `pool` section of the fixture, with the validation hook
            /// the attribute's `validate` used to wire in.
            fn sources() -> dynamic_config::Builder<$name> {
                $name::builder("pool")
                    .file("tests/fixtures/pool.json")
                    .validate(|pool| dynamic_config::Error::ok_or_invalid(pool.validate()))
            }
        }
    };
}

pool!(Pool);
pool!(HeldPool);

#[test]
fn a_configuration_that_parses_can_still_be_rejected() {
    // The fixture is coherent, so this is the baseline.
    let pool = Pool::sources().load().expect("the fixture is coherent");
    assert_eq!(pool.min_size, 2);

    // Every field still parses; only the relationship between them is wrong.
    Pool::set_override("min_size", 99u16).unwrap();

    let error = Pool::sources()
        .load()
        .expect_err("`min_size` is now above `max_size`");

    assert_eq!(error.kind(), ErrorKind::Invalid);
    assert!(error.to_string().contains("min_size (99)"), "{error}");

    Pool::clear_overrides();
}

#[test]
fn a_rejected_reload_leaves_the_snapshot_alone() {
    HeldPool::sources().init().expect("the fixture is coherent");
    assert_eq!(HeldPool::current().max_size, 8);

    HeldPool::set_override("min_size", 99u16).unwrap();
    assert!(HeldPool::sources().load().is_err());

    assert_eq!(
        HeldPool::current().max_size,
        8,
        "a load that fails validation must not have replaced anything"
    );

    HeldPool::clear_overrides();
}

// ─────────────────────────────────────────────
// Snapshot::diff
// ─────────────────────────────────────────────

fn spec<'a>(sources: &'a [Source<'a>]) -> LoadSpec<'a> {
    LoadSpec::new("db", sources)
}

#[test]
fn a_diff_names_the_keys_that_moved_and_nothing_else() {
    let before = [Source::inline(
        r#"{"db": {"host": "a", "port": 1, "pool": {"max": 1}}}"#,
        Format::Json,
    )];
    let after = [Source::inline(
        r#"{"db": {"host": "a", "port": 2, "pool": {"max": 1}, "tls": true}}"#,
        Format::Json,
    )];

    let before = snapshot(&spec(&before)).unwrap();
    let after = snapshot(&spec(&after)).unwrap();

    let changes = before.diff(&after);

    assert_eq!(changes.len(), 2, "{changes:?}");
    assert_eq!(changes[0].path, "port");
    assert_eq!(changes[0].kind, ChangeKind::Modified);
    assert_eq!(changes[1].path, "tls");
    assert_eq!(changes[1].kind, ChangeKind::Added);
}

#[test]
fn a_diff_never_carries_a_value() {
    let before = [Source::inline(
        r#"{"db": {"password": "hunter2"}}"#,
        Format::Json,
    )];
    let after = [Source::inline(
        r#"{"db": {"password": "letmein"}}"#,
        Format::Json,
    )];

    let changes = snapshot(&spec(&before))
        .unwrap()
        .diff(&snapshot(&spec(&after)).unwrap());

    let rendered = format!("{changes:?}");

    assert!(rendered.contains("password"), "{rendered}");
    assert!(!rendered.contains("hunter2"), "{rendered}");
    assert!(!rendered.contains("letmein"), "{rendered}");
}

// ─────────────────────────────────────────────
// profile_env
// ─────────────────────────────────────────────

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Profiled {
    host: String,
    port: u16,
}

/// The `db` section of the profiled fixture, with the profile chosen by
/// `DCSAFE_PROFILE`.
fn profiled_builder() -> dynamic_config::Builder<Profiled> {
    Profiled::builder("db")
        .file("tests/fixtures/profiled.json")
        .profile_env("DCSAFE_PROFILE")
}

/// The only test in this binary that touches the environment.
#[test]
fn a_profile_layers_a_sibling_file_over_the_base() {
    let base = profiled_builder()
        .load()
        .expect("the base fixture is complete");
    assert_eq!(base.host, "base-host");
    assert_eq!(base.port, 5432);

    std::env::set_var("DCSAFE_PROFILE", "production");

    let production = profiled_builder()
        .load()
        .expect("the production variant is a valid overlay");
    assert_eq!(
        production.port, 6432,
        "`profiled.production.json` overrides the port"
    );
    assert_eq!(
        production.host, "base-host",
        "a key the variant omits still comes from the base"
    );

    // A profile with no file of its own is not an error, just no overlay.
    std::env::set_var("DCSAFE_PROFILE", "staging");
    assert_eq!(profiled_builder().load().unwrap().port, 5432);

    std::env::remove_var("DCSAFE_PROFILE");
}

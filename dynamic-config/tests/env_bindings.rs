//! Fields bound to environment variables that are not ours to name.
//!
//! One test touches the environment per binary section, and each uses its own
//! variables: tests share a process and run in parallel.

#![cfg(feature = "json")]

use dynamic_config::{dynamic_config, Builder};
use serde::Deserialize;

/// A type per test, and variables per test. These write to the process
/// environment and the bindings live in a `static`, so two tests sharing either
/// would race — and would pass on their own, which is worse.
macro_rules! db_config {
    ($name:ident, $prefix:literal) => {
        #[dynamic_config]
        #[derive(Debug, Deserialize)]
        struct $name {
            #[allow(dead_code)]
            host: String,
            #[allow(dead_code)]
            port: u16,
        }

        impl $name {
            /// The base fixture plus this test's own environment prefix.
            fn sources() -> Builder<$name> {
                $name::builder("db")
                    .file("tests/fixtures/base.json")
                    .env($prefix)
            }
        }
    };
}

db_config!(Supplies, "DCB1_");
db_config!(Reread, "DCB2_");
db_config!(Empty, "DCB3_");
db_config!(Refused, "DCB4_");

#[test]
fn a_bound_variable_supplies_the_field() {
    Supplies::bind_env("port", "DCB1_LEGACY_PORT").unwrap();

    // Not set yet: a binding contributes nothing until the variable exists,
    // which is the whole point — the platform may or may not have set it.
    assert_eq!(
        Supplies::sources().load().unwrap().port,
        5432,
        "the file still wins"
    );

    std::env::set_var("DCB1_LEGACY_PORT", "9001");

    let config = Supplies::sources().load().expect("the binding resolves");

    assert_eq!(config.port, 9001);
    assert_eq!(config.host, "localhost", "and nothing else moved");

    std::env::remove_var("DCB1_LEGACY_PORT");
}

#[test]
fn a_binding_is_read_at_load_time_so_a_reload_sees_a_change() {
    Reread::bind_env("host", "DCB2_RELOAD_HOST").unwrap();

    std::env::set_var("DCB2_RELOAD_HOST", "first");
    assert_eq!(Reread::sources().load().unwrap().host, "first");

    std::env::set_var("DCB2_RELOAD_HOST", "second");
    assert_eq!(
        Reread::sources().load().unwrap().host,
        "second",
        "the variable is looked up per load, not captured when bound"
    );

    std::env::remove_var("DCB2_RELOAD_HOST");
}

#[test]
fn an_empty_variable_is_treated_as_unset() {
    Empty::bind_env("host", "DCB3_EMPTY_HOST").unwrap();

    std::env::set_var("DCB3_EMPTY_HOST", "");

    assert_eq!(
        Empty::sources().load().unwrap().host,
        "localhost",
        "a deployment template rendering `HOST=` must not blank out a good value"
    );

    std::env::remove_var("DCB3_EMPTY_HOST");
}

#[test]
fn a_path_that_names_nothing_is_refused() {
    assert!(Refused::bind_env("", "X").is_err());
    assert!(Refused::bind_env("pool..max", "X").is_err());
}

#[test]
fn clearing_removes_the_binding() {
    Refused::bind_env("host", "DCB4_CLEARED_HOST").unwrap();
    std::env::set_var("DCB4_CLEARED_HOST", "bound");

    assert_eq!(Refused::sources().load().unwrap().host, "bound");

    Refused::clear_env_bindings();

    assert_eq!(
        Refused::sources().load().unwrap().host,
        "localhost",
        "the file is back in charge"
    );

    std::env::remove_var("DCB4_CLEARED_HOST");
}

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Precedence {
    host: String,
    #[allow(dead_code)]
    port: u16,
}

/// The base fixture plus the prefix the precedence test competes against.
fn precedence_builder() -> Builder<Precedence> {
    Precedence::builder("db")
        .file("tests/fixtures/base.json")
        .env("DCBINDPREC_")
}

#[test]
fn a_binding_beats_the_prefixed_layer_and_loses_to_a_flag() {
    std::env::set_var("DCBINDPREC_DB_HOST", "from-the-prefix");
    std::env::set_var("DCBINDPREC_EXPLICIT_HOST", "from-the-binding");

    Precedence::bind_env("host", "DCBINDPREC_EXPLICIT_HOST").unwrap();

    assert_eq!(
        precedence_builder().load().unwrap().host,
        "from-the-binding",
        "a name somebody chose on purpose beats a convention"
    );

    Precedence::set_flag("host", Some("from-the-flag")).unwrap();

    assert_eq!(
        precedence_builder().load().unwrap().host,
        "from-the-flag",
        "and a flag typed for this one run beats both"
    );

    std::env::remove_var("DCBINDPREC_DB_HOST");
    std::env::remove_var("DCBINDPREC_EXPLICIT_HOST");
    Precedence::clear_flags();
}

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Traced {
    #[allow(dead_code)]
    host: String,
    #[allow(dead_code)]
    port: u16,
}

#[test]
fn a_bound_value_is_traced_to_the_binding_not_to_a_file() {
    Traced::bind_env("host", "DCBIND_TRACED_HOST").unwrap();

    std::env::set_var("DCBIND_TRACED_HOST", "bound");

    // No init has happened, so the question goes to the builder directly.
    let origin = Traced::builder("db")
        .file("tests/fixtures/base.json")
        .source_of("host")
        .unwrap()
        .expect("something supplies it");

    assert_eq!(
        format!("{origin}"),
        "from DCBIND_TRACED_HOST",
        "the variable that supplied it, not the category it belongs to"
    );

    std::env::remove_var("DCBIND_TRACED_HOST");
}

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Nested {
    #[allow(dead_code)]
    host: String,
    #[allow(dead_code)]
    port: u16,
    pool: Pool,
}

#[derive(Debug, Deserialize)]
struct Pool {
    max_size: u16,
}

#[test]
fn a_binding_can_reach_a_nested_field() {
    Nested::bind_env("pool.max_size", "DCBIND_POOL_MAX").unwrap();

    std::env::set_var("DCBIND_POOL_MAX", "64");

    assert_eq!(
        Nested::builder("db")
            .file("tests/fixtures/base.json")
            .load()
            .unwrap()
            .pool
            .max_size,
        64
    );

    std::env::remove_var("DCBIND_POOL_MAX");
}

/// One empty-value rule for all three env-shaped layers: whitespace counts
/// as empty, and `allow_empty_env` is honored by bindings too — it used to
/// be silently ignored there.
#[test]
fn bindings_honor_allow_empty_env_with_the_shared_rule() {
    use serde::Deserialize;

    #[dynamic_config::dynamic_config]
    #[derive(Debug, Deserialize)]
    struct EmptyAllowed {
        host: String,
    }

    EmptyAllowed::bind_env("host", "DCBINDEMPTY_HOST_SOURCE").unwrap();
    std::env::set_var("DCBINDEMPTY_HOST_SOURCE", "   ");

    let loaded = EmptyAllowed::builder("db")
        .file("tests/fixtures/base.json")
        .env("DCBINDEMPTY_")
        .allow_empty_env()
        .load();

    std::env::remove_var("DCBINDEMPTY_HOST_SOURCE");
    EmptyAllowed::clear_env_bindings();

    // With allow_empty_env, a whitespace-only bound variable IS a value —
    // it overrides the file's "localhost" with emptiness. (figment's value
    // parsing may normalise the whitespace itself; what matters is that the
    // binding supplied the value at all.)
    let host = loaded.expect("empty is allowed").host;
    assert!(host.trim().is_empty(), "the binding must win: {host:?}");
}

#[test]
fn a_whitespace_only_bound_variable_is_unset_by_default() {
    use serde::Deserialize;

    #[dynamic_config::dynamic_config]
    #[derive(Debug, Deserialize)]
    struct BlankDefault {
        host: String,
    }

    BlankDefault::bind_env("host", "DCBINDBLANK_HOST_SOURCE").unwrap();
    std::env::set_var("DCBINDBLANK_HOST_SOURCE", "   ");

    let loaded = BlankDefault::builder("db")
        .file("tests/fixtures/base.json")
        .env("DCBINDBLANK_")
        .load();

    std::env::remove_var("DCBINDBLANK_HOST_SOURCE");
    BlankDefault::clear_env_bindings();

    // Without the flag, whitespace is a template that rendered "nothing" —
    // the file's value must survive.
    assert_eq!(loaded.expect("the file supplies host").host, "localhost");
}

/// `nest` picks the separator that introduces nesting in a variable name —
/// an API that until now had only compile-fail coverage.
#[test]
fn a_custom_nest_separator_reaches_nested_fields() {
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    struct Pool {
        max_size: u16,
    }

    #[dynamic_config::dynamic_config]
    #[derive(Debug, Deserialize)]
    struct Nested {
        #[allow(dead_code)]
        host: String,
        pool: Pool,
    }

    std::env::set_var("DCNEST_DB_POOL__MAX_SIZE", "32");

    let loaded = Nested::builder("db")
        .file("tests/fixtures/base.json")
        .env("DCNEST_")
        .nest("__")
        .load();

    std::env::remove_var("DCNEST_DB_POOL__MAX_SIZE");

    assert_eq!(
        loaded.expect("the nested variable resolves").pool.max_size,
        32
    );
}

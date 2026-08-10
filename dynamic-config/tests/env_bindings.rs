//! Fields bound to environment variables that are not ours to name.
//!
//! One test touches the environment per binary section, and each uses its own
//! variables: tests share a process and run in parallel.

#![cfg(feature = "json")]

use dynamic_config::dynamic_config;
use serde::Deserialize;

/// A type per test, and variables per test. These write to the process
/// environment and the bindings live in a `static`, so two tests sharing either
/// would race — and would pass on their own, which is worse.
macro_rules! db_config {
    ($name:ident, $prefix:literal) => {
        #[dynamic_config(files = ["tests/fixtures/base.json"], key = "db", env = $prefix)]
        #[derive(Debug, Deserialize)]
        struct $name {
            #[allow(dead_code)]
            host: String,
            #[allow(dead_code)]
            port: u16,
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
    assert_eq!(Supplies::load().unwrap().port, 5432, "the file still wins");

    std::env::set_var("DCB1_LEGACY_PORT", "9001");

    let config = Supplies::load().expect("the binding resolves");

    assert_eq!(config.port, 9001);
    assert_eq!(config.host, "localhost", "and nothing else moved");

    std::env::remove_var("DCB1_LEGACY_PORT");
}

#[test]
fn a_binding_is_read_at_load_time_so_a_reload_sees_a_change() {
    Reread::bind_env("host", "DCB2_RELOAD_HOST").unwrap();

    std::env::set_var("DCB2_RELOAD_HOST", "first");
    assert_eq!(Reread::load().unwrap().host, "first");

    std::env::set_var("DCB2_RELOAD_HOST", "second");
    assert_eq!(
        Reread::load().unwrap().host,
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
        Empty::load().unwrap().host,
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

    assert_eq!(Refused::load().unwrap().host, "bound");

    Refused::clear_env_bindings();

    assert_eq!(
        Refused::load().unwrap().host,
        "localhost",
        "the file is back in charge"
    );

    std::env::remove_var("DCB4_CLEARED_HOST");
}

#[dynamic_config(files = ["tests/fixtures/base.json"], key = "db", env = "DCBINDPREC_")]
#[derive(Debug, Deserialize)]
struct Precedence {
    host: String,
    #[allow(dead_code)]
    port: u16,
}

#[test]
fn a_binding_beats_the_prefixed_layer_and_loses_to_a_flag() {
    std::env::set_var("DCBINDPREC_DB_HOST", "from-the-prefix");
    std::env::set_var("DCBINDPREC_EXPLICIT_HOST", "from-the-binding");

    Precedence::bind_env("host", "DCBINDPREC_EXPLICIT_HOST").unwrap();

    assert_eq!(
        Precedence::load().unwrap().host,
        "from-the-binding",
        "a name somebody chose on purpose beats a convention"
    );

    Precedence::set_flag("host", Some("from-the-flag")).unwrap();

    assert_eq!(
        Precedence::load().unwrap().host,
        "from-the-flag",
        "and a flag typed for this one run beats both"
    );

    std::env::remove_var("DCBINDPREC_DB_HOST");
    std::env::remove_var("DCBINDPREC_EXPLICIT_HOST");
    Precedence::clear_flags();
}

#[dynamic_config(files = ["tests/fixtures/base.json"], key = "db")]
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

    let origin = Traced::source_of("host")
        .unwrap()
        .expect("something supplies it");

    assert_eq!(
        format!("{origin}"),
        "from DCBIND_TRACED_HOST",
        "the variable that supplied it, not the category it belongs to"
    );

    std::env::remove_var("DCBIND_TRACED_HOST");
}

#[dynamic_config(files = ["tests/fixtures/base.json"], key = "db")]
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

    assert_eq!(Nested::load().unwrap().pool.max_size, 64);

    std::env::remove_var("DCBIND_POOL_MAX");
}

//! The two runtime layers, and where they sit relative to everything else.
//!
//! ```text
//! set_default  <  files  <  environment  <  set_override
//! ```
//!
//! Each test owns its own config type: the layers live in `static`s, so sharing
//! a type would let tests write over each other.

#![cfg(feature = "json")]

use dynamic_config::{dynamic_config, Builder, Origin};
use serde::Deserialize;

macro_rules! db_config {
    ($name:ident, $env:literal) => {
        #[dynamic_config]
        #[derive(Debug, Deserialize)]
        struct $name {
            host: String,
            // Not every test reads every field.
            #[allow(dead_code)]
            port: u16,
            // Not every test reads it; it is here to prove nested writes work.
            #[allow(dead_code)]
            pool: Pool,
        }

        impl $name {
            /// The base fixture plus this test's own environment prefix.
            fn sources() -> Builder<$name> {
                $name::builder("db")
                    .file("tests/fixtures/base.json")
                    .env($env)
            }
        }
    };
}

#[derive(Debug, Deserialize)]
struct Pool {
    max_size: u16,
}

db_config!(DefaultsOnly, "DCLAYER_A_");
db_config!(OverrideWins, "DCLAYER_B_");
db_config!(Nested, "DCLAYER_C_");
db_config!(Cleared, "DCLAYER_D_");

/// A field the file already supplies is untouched by a default.
#[test]
fn a_default_fills_only_what_nothing_else_supplies() {
    DefaultsOnly::set_default("host", "from-default").unwrap();
    DefaultsOnly::set_default("port", 1234u16).unwrap();

    let config = DefaultsOnly::sources()
        .load()
        .expect("the fixture plus defaults are complete");

    assert_eq!(
        config.host, "localhost",
        "the file supplies `host`, so the default stays out of the way"
    );
    assert_eq!(config.port, 5432, "same for `port`");
}

/// The fixture has no `db.timeout_secs`, so this type cannot load without one.
#[dynamic_config]
#[derive(Debug, Deserialize)]
struct WithGap {
    host: String,
    timeout_secs: u64,
}

/// Reads the `db` section of the base fixture, which lacks `timeout_secs`.
fn with_gap_builder() -> Builder<WithGap> {
    WithGap::builder("db").file("tests/fixtures/base.json")
}

#[test]
fn a_default_supplies_a_key_no_file_has() {
    assert!(
        with_gap_builder().load().is_err(),
        "nothing supplies `timeout_secs` yet"
    );

    WithGap::set_default("timeout_secs", 30u64).unwrap();

    let config = with_gap_builder()
        .load()
        .expect("the default closes the gap");
    assert_eq!(config.timeout_secs, 30);
    assert_eq!(config.host, "localhost", "the file still owns `host`");

    WithGap::clear_defaults();
}

#[test]
fn an_override_beats_the_file() {
    OverrideWins::set_override("host", "from-override").unwrap();

    let config = OverrideWins::sources()
        .load()
        .expect("the fixture is complete");

    assert_eq!(config.host, "from-override");
    assert_eq!(config.port, 5432, "untouched keys still come from the file");

    OverrideWins::clear_overrides();
}

#[test]
fn a_dotted_path_reaches_a_nested_field() {
    Nested::set_override("pool.max_size", 77u16).unwrap();

    let config = Nested::sources().load().expect("the fixture is complete");

    assert_eq!(config.pool.max_size, 77);
    assert_eq!(
        config.host, "localhost",
        "writing into `pool` must not disturb its siblings"
    );

    Nested::clear_overrides();
}

#[test]
fn clearing_puts_the_file_back_in_charge() {
    Cleared::set_override("host", "temporary").unwrap();
    assert_eq!(Cleared::sources().load().unwrap().host, "temporary");

    Cleared::clear_overrides();
    assert_eq!(Cleared::sources().load().unwrap().host, "localhost");

    Cleared::set_default("host", "fallback").unwrap();
    assert_eq!(
        Cleared::sources().load().unwrap().host,
        "localhost",
        "a default cannot displace the file either"
    );

    Cleared::clear_defaults();
}

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Attributed {
    #[allow(dead_code)]
    port: u16,
}

/// An error in a runtime layer says so, rather than blaming a file.
#[test]
fn a_bad_override_is_attributed_to_the_override_layer() {
    Attributed::set_override("port", "not-a-number").unwrap();

    // The one listed file does not exist, so only the override layer speaks.
    let error = Attributed::builder("db")
        .file("tests/fixtures/absent.json")
        .load()
        .expect_err("`port` cannot be a string");

    assert_eq!(error.origin(), &Origin::Runtime("override"));
    assert!(error.to_string().contains("set as override"), "{error}");

    Attributed::clear_overrides();
}

/// `set_defaults(&struct)` seeds the whole defaults layer at once — and
/// stays a *defaults* layer: a real file value still wins.
#[test]
fn a_defaults_struct_seeds_the_bottom_layer() {
    use serde::{Deserialize, Serialize};

    #[dynamic_config::dynamic_config]
    #[derive(Debug, Deserialize, Serialize)]
    struct StructDefaults {
        host: String,
        #[serde(default)]
        greeting: String,
    }

    StructDefaults::set_defaults(&StructDefaults {
        host: "fallback-host".to_owned(),
        greeting: "hello".to_owned(),
    })
    .unwrap();

    let loaded = StructDefaults::builder("db")
        .file("tests/fixtures/base.json")
        .load();
    StructDefaults::clear_defaults();

    let loaded = loaded.expect("defaults fill what the file lacks");
    assert_eq!(
        loaded.host, "localhost",
        "a real file value beats the defaults struct"
    );
    assert_eq!(
        loaded.greeting, "hello",
        "a field no file supplies comes from the struct"
    );
}

/// A bare value has no field to live under, and is refused with a message
/// that says so.
#[test]
fn a_non_map_defaults_value_is_refused() {
    use serde::Deserialize;

    #[dynamic_config::dynamic_config]
    #[derive(Debug, Deserialize)]
    struct BareDefaults {
        #[allow(dead_code)]
        host: String,
    }

    let error = BareDefaults::set_defaults(&42u16).expect_err("a bare number is not defaults");
    assert!(error.to_string().contains("struct or a map"), "{error}");
}

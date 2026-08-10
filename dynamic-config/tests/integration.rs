//! End-to-end behaviour of `#[dynamic_config]`.
//!
//! Cargo runs integration tests with the package root as the working directory,
//! so the fixture paths below resolve against `dynamic-config/`.
//!
//! Exactly one test touches the environment. Tests share a process and run in
//! parallel, and reading configuration enumerates the whole environment, so a
//! second env-touching test would race it.

#![cfg(feature = "json")]

use dynamic_config::{dynamic_config, ErrorKind};
use serde::Deserialize;

/// Two files: the second overrides one field of the first.
#[dynamic_config(
    files = ["tests/fixtures/base.json", "tests/fixtures/override.json"],
    key = "db"
)]
#[derive(Debug, Deserialize, PartialEq)]
struct DbConfig {
    host: String,
    port: u16,
    pool: PoolConfig,
}

#[derive(Debug, Deserialize, PartialEq)]
struct PoolConfig {
    max_size: u16,
}

/// Reads the `server` section, overridable through `DCTEST_SERVER_*`.
#[dynamic_config(files = ["tests/fixtures/base.json"], key = "server", env = "DCTEST_")]
#[derive(Debug, Deserialize)]
struct ServerConfig {
    host: String,
    port: u16,
}

/// Points only at a file that does not exist, so nothing supplies `value`.
#[dynamic_config(files = ["tests/fixtures/absent.json"], key = "missing")]
#[derive(Debug, Deserialize)]
struct MissingConfig {
    #[allow(dead_code)]
    value: u32,
}

#[test]
fn later_files_win_and_tables_merge_rather_than_replace() {
    let config = DbConfig::load().expect("fixtures should deserialize");

    assert_eq!(config.port, 6432, "the second file overrides the port");
    assert_eq!(
        config.host, "localhost",
        "a field the second file omits survives"
    );
    assert_eq!(
        config.pool.max_size, 10,
        "a nested table the second file omits survives entirely"
    );
}

#[test]
fn the_snapshot_appears_on_init_and_swaps_on_replace() {
    assert!(
        DbConfig::try_current().is_none(),
        "no snapshot should exist before init"
    );

    DbConfig::init().expect("init should succeed");

    let held = DbConfig::current();
    assert_eq!(held.port, 6432);

    DbConfig::replace(DbConfig {
        host: "replaced".to_owned(),
        port: 1,
        pool: PoolConfig { max_size: 1 },
    });

    assert_eq!(DbConfig::current().host, "replaced");
    assert_eq!(
        held.host, "localhost",
        "the Arc taken earlier keeps its own generation"
    );
}

#[test]
fn a_missing_file_is_skipped_rather_than_failing() {
    // The failure must be about the absent *field*, not the absent file: that
    // is what proves a missing file is skipped instead of raising an I/O error.
    let error = MissingConfig::load().expect_err("nothing supplies `value`");

    assert_eq!(error.kind(), ErrorKind::Missing);
    assert_eq!(error.path(), "value");
}

#[test]
fn a_wrong_type_names_the_path_and_the_variable_that_set_it() {
    // This is the only test in the binary that touches the environment, so no
    // other thread can be reading it concurrently.
    std::env::set_var("DCTEST_SERVER_PORT", "9999");
    std::env::set_var("DCTEST_SERVER_HOST", "10.0.0.1");

    let config = ServerConfig::load().expect("the environment should coerce cleanly");
    assert_eq!(config.port, 9999, "the environment wins over the file");
    assert_eq!(config.host, "10.0.0.1");

    std::env::set_var("DCTEST_SERVER_PORT", "not-a-number");

    let error = ServerConfig::load().expect_err("`not-a-number` is not a u16");
    assert_eq!(error.kind(), ErrorKind::Type);
    assert_eq!(error.path(), "port");
    // figment names the provider by its prefix rather than the exact variable,
    // which is still enough to say "this came from the environment, not a file".
    assert!(
        error.to_string().contains("DCTEST_SERVER_"),
        "the message should point at the environment: {error}"
    );

    std::env::remove_var("DCTEST_SERVER_PORT");
    std::env::remove_var("DCTEST_SERVER_HOST");
}

//! Writing configuration back out, and reading it without a struct.

#![cfg(feature = "json")]

use std::fs;
use std::path::PathBuf;

use dynamic_config::{dynamic_config, ErrorKind};
use serde::{Deserialize, Serialize};

/// A directory per test: they run in parallel and write real files.
fn scratch(test: &str) -> PathBuf {
    let directory = std::env::temp_dir()
        .join("dynamic-config-persistence")
        .join(test);

    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).expect("the scratch directory should be creatable");

    directory
}

#[dynamic_config(files = ["tests/fixtures/base.json"], key = "db", save)]
#[derive(Debug, Deserialize, Serialize, PartialEq)]
struct Db {
    host: String,
    port: u16,
    pool: Pool,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
struct Pool {
    max_size: u16,
}

#[test]
fn what_is_saved_is_shaped_the_way_the_loader_expects() {
    let path = scratch("round-trip").join("config.json");

    let original = Db::load().expect("the fixture is complete");
    original.save(&path).expect("the directory is writable");

    // Nested under the section key, so a loader pointed at this file finds it.
    let written: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();

    assert!(written.get("db").is_some(), "{written}");
    assert_eq!(written["db"]["port"], 5432);
}

#[test]
fn an_extension_that_names_no_format_is_refused() {
    let path = scratch("bad-extension").join("config.ini");

    let error = Db::load().unwrap().save(&path).unwrap_err();

    assert_eq!(error.kind(), ErrorKind::Backend);
    assert!(error.to_string().contains("`.json`"), "{error}");
}

// ─────────────────────────────────────────────
// Reading without a struct
// ─────────────────────────────────────────────

#[test]
fn a_value_can_be_read_by_path() {
    let snapshot = Db::snapshot().expect("the fixture is complete");

    assert_eq!(snapshot.get::<String>("host").unwrap(), "localhost");
    assert_eq!(snapshot.get::<u16>("pool.max_size").unwrap(), 10);
    assert!(snapshot.contains("pool.max_size"));
    assert!(!snapshot.contains("pool.min_size"));
}

#[test]
fn a_sub_snapshot_carries_only_its_own_table() {
    let pool = Db::snapshot()
        .unwrap()
        .sub("pool")
        .expect("`pool` is a table");

    assert_eq!(pool.get::<u16>("max_size").unwrap(), 10);
    assert!(!pool.contains("host"), "the parent's keys are not in scope");
}

// ─────────────────────────────────────────────
// Aliases
// ─────────────────────────────────────────────

/// The file uses the old name; the struct has renamed the field.
#[dynamic_config(files = ["tests/fixtures/aliased.json"], key = "db")]
#[derive(Debug, Deserialize)]
struct Renamed {
    #[serde(alias = "hostname")]
    host: String,
}

#[test]
fn an_alias_loads_and_is_not_reported_as_a_typo() {
    let config = Renamed::load().expect("`hostname` is an accepted name for `host`");
    assert_eq!(config.host, "localhost");

    let report = Renamed::check().expect("the fixture reads cleanly");
    assert!(
        report.unknown.is_empty(),
        "an alias is a name the file may use: {report}"
    );
}

//! Writing configuration back out, and reading it without a struct.

#![cfg(feature = "json")]

use std::fs;
use std::path::{Path, PathBuf};

use dynamic_config::{dynamic_config, Builder, ErrorKind, Format};
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

#[dynamic_config]
#[derive(Debug, Deserialize, Serialize, PartialEq)]
struct Db {
    host: String,
    port: u16,
    pool: Pool,
}

/// The `db` section of the base fixture.
fn db_builder() -> Builder<Db> {
    Db::builder("db").file("tests/fixtures/base.json")
}

/// Saves under the `db` key, taking the format from the extension the way the
/// old generated `save` method did — which is what lets an extension that
/// names no format be refused rather than guessed at.
fn save_db(value: &Db, path: &Path) -> Result<(), dynamic_config::Error> {
    dynamic_config::save(value, path, Format::from_path(path)?, "db")
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
struct Pool {
    max_size: u16,
}

#[test]
fn what_is_saved_is_shaped_the_way_the_loader_expects() {
    let path = scratch("round-trip").join("config.json");

    let original = db_builder().load().expect("the fixture is complete");
    save_db(&original, &path).expect("the directory is writable");

    // Nested under the section key, so a loader pointed at this file finds it.
    let written: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();

    assert!(written.get("db").is_some(), "{written}");
    assert_eq!(written["db"]["port"], 5432);
}

#[test]
fn an_extension_that_names_no_format_is_refused() {
    // `.conf` rather than `.ini`: since 0.7 INI *is* a format (and its
    // refusal below is about writing, not recognition).
    let path = scratch("bad-extension").join("config.conf");

    let error = save_db(&db_builder().load().unwrap(), &path).unwrap_err();

    assert_eq!(error.kind(), ErrorKind::Backend);
    assert!(error.to_string().contains("`.json`"), "{error}");
}

#[test]
#[cfg(feature = "ini")]
fn a_flat_format_is_recognised_and_still_refused_for_writing() {
    let path = scratch("flat-write").join("config.ini");

    let error = save_db(&db_builder().load().unwrap(), &path).unwrap_err();

    assert_eq!(error.kind(), ErrorKind::Backend);
    assert!(error.to_string().contains("cannot be written"), "{error}");
}

// ─────────────────────────────────────────────
// Reading without a struct
// ─────────────────────────────────────────────

#[test]
fn a_value_can_be_read_by_path() {
    let snapshot = db_builder().snapshot().expect("the fixture is complete");

    assert_eq!(snapshot.get::<String>("host").unwrap(), "localhost");
    assert_eq!(snapshot.get::<u16>("pool.max_size").unwrap(), 10);
    assert!(snapshot.contains("pool.max_size"));
    assert!(!snapshot.contains("pool.min_size"));
}

#[test]
fn a_sub_snapshot_carries_only_its_own_table() {
    let pool = db_builder()
        .snapshot()
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
#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Renamed {
    #[serde(alias = "hostname")]
    host: String,
}

/// The `db` section of the fixture that still spells it `hostname`.
fn renamed_builder() -> Builder<Renamed> {
    Renamed::builder("db").file("tests/fixtures/aliased.json")
}

#[test]
fn an_alias_loads_and_is_not_reported_as_a_typo() {
    let config = renamed_builder()
        .load()
        .expect("`hostname` is an accepted name for `host`");
    assert_eq!(config.host, "localhost");

    let report = renamed_builder()
        .check()
        .expect("the fixture reads cleanly");
    assert!(
        report.unknown.is_empty(),
        "an alias is a name the file may use: {report}"
    );
}

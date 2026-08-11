//! Finding configuration by name across a search path.
//!
//! The fixtures are committed so the test is deterministic: `tests/search/etc`
//! stands in for a package's `/etc/myapp`, `tests/search/home` for a user's
//! `~/.config/myapp`, and `tests/search/local` for the working directory.

#![cfg(feature = "json")]

use dynamic_config::{dynamic_config, load, Builder, LoadSpec, Search};
use serde::Deserialize;

#[derive(Debug, Deserialize, PartialEq)]
struct Db {
    host: String,
    port: u16,
}

#[dynamic_config]
#[derive(Debug, Deserialize, PartialEq)]
struct Layered {
    host: String,
    port: u16,
    tls: bool,
}

/// Three directories, layered in the order they are listed.
fn layered_builder() -> Builder<Layered> {
    Layered::builder("db").discover(
        "config",
        [
            "tests/search/etc",
            "tests/search/home",
            "tests/search/local",
        ],
    )
}

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct WithExplicit {
    #[allow(dead_code)]
    host: String,
    port: u16,
    #[allow(dead_code)]
    tls: bool,
}

/// Discovery and an explicit file together.
fn with_explicit_builder() -> Builder<WithExplicit> {
    WithExplicit::builder("db")
        .discover("config", ["tests/search/etc"])
        .file("tests/fixtures/override.json")
}

#[test]
fn later_directories_layer_over_earlier_ones() {
    let config = layered_builder()
        .load()
        .expect("the three fixtures together are complete");

    assert_eq!(
        config.host, "local-host",
        "the last directory in the list wins"
    );
    assert_eq!(
        config.port, 6432,
        "the middle directory overrides the first"
    );
    assert!(
        config.tls,
        "a key only the first directory sets still survives"
    );
}

#[test]
fn an_explicit_file_outranks_a_discovered_one() {
    let config = with_explicit_builder()
        .load()
        .expect("discovery plus the explicit file are complete");

    assert_eq!(
        config.port, 6432,
        "`.file(..)` is a deliberate statement; a search result is a guess"
    );
}

#[test]
fn a_directory_that_does_not_exist_is_simply_skipped() {
    let paths = ["tests/search/nowhere", "tests/search/etc"];
    let search = Search::new("config", &paths);

    let db: Db = load(&LoadSpec::new("db", &[]).with_search(search.name, search.paths))
        .expect("the one directory that exists should be enough");

    assert_eq!(db.host, "etc-host");
}

#[test]
fn a_search_that_finds_nothing_leaves_the_section_empty() {
    let paths = ["tests/search/nowhere"];

    let error = load::<Db>(&LoadSpec::new("db", &[]).with_search("config", &paths))
        .expect_err("nothing supplies `host`");

    assert_eq!(error.kind(), dynamic_config::ErrorKind::Missing);
}

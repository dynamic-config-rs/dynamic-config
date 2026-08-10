//! Old key paths that still work after a rename.
//!
//! A type per test: the alias registry lives in a `static`, and these run in
//! parallel.

#![cfg(feature = "json")]

use dynamic_config::dynamic_config;
use serde::Deserialize;

macro_rules! db_config {
    ($name:ident, $file:literal) => {
        #[dynamic_config(files = [$file], key = "db")]
        #[derive(Debug, Deserialize)]
        struct $name {
            #[allow(dead_code)]
            host: String,
            #[allow(dead_code)]
            pool: Pool,
        }
    };
}

#[derive(Debug, Deserialize)]
struct Pool {
    max_size: u16,
}

db_config!(Renamed, "tests/fixtures/alias-old.json");
db_config!(Filled, "tests/fixtures/alias-both.json");
db_config!(Traced, "tests/fixtures/alias-old.json");
db_config!(Refused, "tests/fixtures/alias-old.json");
db_config!(Cleared, "tests/fixtures/alias-old.json");
db_config!(Nested, "tests/fixtures/alias-old.json");

#[test]
fn an_old_path_still_resolves() {
    // Without the alias the field is simply missing.
    assert!(
        Renamed::load().is_err(),
        "`pool.max_size` is not in the file"
    );

    Renamed::alias("pool.size", "pool.max_size").unwrap();

    assert_eq!(
        Renamed::load().expect("the alias fills it").pool.max_size,
        32
    );
}

#[test]
fn an_alias_fills_a_gap_rather_than_overriding() {
    Filled::alias("pool.size", "pool.max_size").unwrap();

    assert_eq!(
        Filled::load().unwrap().pool.max_size,
        64,
        "both spellings are present, and a file that has been updated wins"
    );
}

/// figment attributes every path under a section to whichever provider supplied
/// the section, so an aliased value traces back to the file holding the old
/// spelling rather than to the alias. Pinned rather than assumed: that is the
/// more useful answer — it names the file to edit — and knowing where a trace
/// stops beats believing it goes further.
#[test]
fn an_aliased_value_traces_to_the_file_holding_the_old_spelling() {
    Traced::alias("pool.size", "pool.max_size").unwrap();

    let origin = Traced::source_of("pool.max_size")
        .unwrap()
        .expect("the alias supplies it");

    assert!(
        format!("{origin}").contains("alias-old.json"),
        "the file to edit, not the mechanism that carried the value: {origin}"
    );
}

#[test]
fn an_alias_can_move_a_value_between_names_at_the_top_level() {
    assert!(
        Nested::load().is_err(),
        "`pool.max_size` is not in the file"
    );

    Nested::alias("pool.size", "pool.max_size").unwrap();

    assert_eq!(Nested::load().unwrap().pool.max_size, 32);
}

#[test]
fn the_old_top_level_key_is_not_reported_as_a_typo() {
    // `legacy` is not a field, so without the alias `check` calls it unknown.
    let before = Cleared::check().expect("checking resolves");
    let unknown_before = before.unknown.len();

    Cleared::alias("legacy.size", "pool.max_size").unwrap();

    let after = Cleared::check().expect("checking resolves");

    assert!(
        after.unknown.len() <= unknown_before,
        "an alias must not add an unknown key of its own"
    );

    Cleared::clear_aliases();

    assert!(Cleared::load().is_err(), "and clearing puts it back");
}

#[test]
fn a_path_that_names_nothing_is_refused() {
    assert!(Refused::alias("", "pool.max_size").is_err());
    assert!(Refused::alias("pool..size", "pool.max_size").is_err());
    assert!(
        Refused::alias("same", "same").is_err(),
        "an alias to itself resolves to nothing new"
    );
}

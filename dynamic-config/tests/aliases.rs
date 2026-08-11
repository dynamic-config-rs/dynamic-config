//! Old key paths that still work after a rename.
//!
//! A type per test: the alias registry lives in a `static`, and these run in
//! parallel.

#![cfg(feature = "json")]

use dynamic_config::{dynamic_config, Builder};
use serde::Deserialize;

macro_rules! db_config {
    ($name:ident, $file:literal) => {
        #[dynamic_config]
        #[derive(Debug, Deserialize)]
        struct $name {
            #[allow(dead_code)]
            host: String,
            #[allow(dead_code)]
            pool: Pool,
        }

        impl $name {
            /// The `db` section of this test's own alias fixture.
            // Not every generated type loads; refusing an alias needs no file.
            #[allow(dead_code)]
            fn sources() -> Builder<$name> {
                $name::builder("db").file($file)
            }
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
        Renamed::sources().load().is_err(),
        "`pool.max_size` is not in the file"
    );

    Renamed::alias("pool.size", "pool.max_size").unwrap();

    assert_eq!(
        Renamed::sources()
            .load()
            .expect("the alias fills it")
            .pool
            .max_size,
        32
    );
}

#[test]
fn an_alias_fills_a_gap_rather_than_overriding() {
    Filled::alias("pool.size", "pool.max_size").unwrap();

    assert_eq!(
        Filled::sources().load().unwrap().pool.max_size,
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

    let origin = Traced::sources()
        .source_of("pool.max_size")
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
        Nested::sources().load().is_err(),
        "`pool.max_size` is not in the file"
    );

    Nested::alias("pool.size", "pool.max_size").unwrap();

    assert_eq!(Nested::sources().load().unwrap().pool.max_size, 32);
}

#[test]
fn the_old_top_level_key_is_not_reported_as_a_typo() {
    // `legacy` is not a field, so without the alias `check` calls it unknown.
    let before = Cleared::sources().check().expect("checking resolves");
    let unknown_before = before.unknown.len();

    Cleared::alias("legacy.size", "pool.max_size").unwrap();

    let after = Cleared::sources().check().expect("checking resolves");

    assert!(
        after.unknown.len() <= unknown_before,
        "an alias must not add an unknown key of its own"
    );

    Cleared::clear_aliases();

    assert!(
        Cleared::sources().load().is_err(),
        "and clearing puts it back"
    );
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

/// The inversion this pins: a runtime default supplying the new path used to
/// stop the alias from firing, so `set_default` beat a real file's value —
/// the exact thing the precedence table forbids.
#[test]
fn a_runtime_default_does_not_defeat_an_alias() {
    use serde::Deserialize;

    #[dynamic_config::dynamic_config]
    #[derive(Debug, Deserialize)]
    struct Defaulted {
        max_size: u16,
    }

    // The file still spells it `size`; the alias carries it to `max_size`.
    Defaulted::alias("size", "max_size").unwrap();
    // A fallback for machines with nothing at all — it must stay a fallback.
    Defaulted::set_default("max_size", 8u16).unwrap();

    let loaded = Defaulted::builder("db")
        .file("tests/fixtures/alias-defaults.json")
        .load();

    Defaulted::clear_aliases();
    Defaulted::clear_defaults();

    assert_eq!(
        loaded.expect("the alias fills the gap").max_size,
        64,
        "the file's value through the alias must beat the runtime default"
    );
}

/// Chains resolve deterministically; cycles are refused at `add` time.
#[test]
fn chains_resolve_and_cycles_are_refused() {
    use serde::Deserialize;

    #[dynamic_config::dynamic_config]
    #[derive(Debug, Deserialize)]
    struct Chained {
        renamed_twice: u16,
    }

    // size → mid → renamed_twice: two renames, one migration.
    Chained::alias("size", "mid").unwrap();
    Chained::alias("mid", "renamed_twice").unwrap();

    let loaded = Chained::builder("db")
        .file("tests/fixtures/alias-chain.json")
        .load();
    Chained::clear_aliases();

    assert_eq!(
        loaded.expect("the chain carries the value").renamed_twice,
        64
    );

    // A loop is a contradictory rename, refused loudly.
    Chained::alias("a", "b").unwrap();
    let error = Chained::alias("b", "a").expect_err("a cycle must be refused");
    assert!(error.to_string().contains("cycle"), "{error}");
    Chained::clear_aliases();
}

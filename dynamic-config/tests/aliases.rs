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
    assert!(
        Refused::alias("pool.size", "other::max_size").is_err(),
        "the new path is always in this configuration's own section"
    );
    assert!(
        Refused::set_default("other::max_size", 8u16).is_err(),
        "a section qualifier is meaningless in an ordinary path"
    );
}

/// A section-qualified old path: the key moved out of `[db]` into `[server]`.
///
/// One type per test, as everywhere here; the fixtures are read-only and
/// shared.
macro_rules! server_config {
    ($name:ident) => {
        #[dynamic_config]
        #[derive(Debug, Deserialize)]
        struct $name {
            #[allow(dead_code)]
            port: u16,
            #[allow(dead_code)]
            timeout_secs: u64,
        }
    };
}

server_config!(Moved);
server_config!(Overtaken);
server_config!(TracedAcross);
server_config!(Explained);
server_config!(Strayed);
server_config!(Chained2);

#[test]
fn a_key_that_moved_to_another_section_still_resolves() {
    let builder = || Moved::builder("server").file("tests/fixtures/alias-moved.json");

    assert!(
        builder().load().is_err(),
        "`timeout_secs` is not in the `server` section"
    );

    Moved::alias("db::timeout_secs", "timeout_secs").unwrap();

    assert_eq!(
        builder()
            .load()
            .expect("the alias reaches the section the key moved out of")
            .timeout_secs,
        30
    );
}

#[test]
fn a_cross_section_alias_fills_a_gap_rather_than_overriding() {
    Overtaken::alias("db::timeout_secs", "timeout_secs").unwrap();

    assert_eq!(
        Overtaken::builder("server")
            .file("tests/fixtures/alias-moved-both.json")
            .load()
            .unwrap()
            .timeout_secs,
        5,
        "a section that has been migrated wins over one that has not"
    );
}

/// The same promise the in-section alias makes: the trace names the file to
/// edit, not the mechanism that carried the value — even though here the alias
/// is what created the destination and there is no other provider under it.
#[test]
fn a_cross_section_alias_traces_to_the_file_holding_the_old_spelling() {
    TracedAcross::alias("db::timeout_secs", "timeout_secs").unwrap();

    let origin = TracedAcross::builder("server")
        .file("tests/fixtures/alias-moved.json")
        .source_of("timeout_secs")
        .unwrap()
        .expect("the alias supplies it");

    assert!(
        format!("{origin}").contains("alias-moved.json"),
        "the file to edit: {origin}"
    );
}

/// The second dimension a cross-section alias adds is *which* spelling, in
/// which section — so the alias row names the old path next to the file.
#[test]
fn explaining_a_cross_section_alias_names_both_hops() {
    Explained::alias("db::timeout_secs", "timeout_secs").unwrap();

    let explanation = Explained::builder("server")
        .file("tests/fixtures/alias-moved.json")
        .explain("timeout_secs")
        .unwrap();

    let winner = explanation.winner().expect("the alias wins");

    assert_eq!(winner.layer, "alias", "{explanation}");
    assert_eq!(
        winner.aliased_from.as_deref(),
        Some("db::timeout_secs"),
        "{explanation}"
    );

    let rendered = explanation.to_string();

    assert!(rendered.contains("db::timeout_secs"), "{rendered}");
    assert!(rendered.contains("alias-moved.json"), "{rendered}");
}

/// The old key is in another section, so it is not a key this one may have.
#[test]
fn the_other_sections_name_is_not_a_known_key_here() {
    Strayed::alias("db::timeout_secs", "timeout_secs").unwrap();

    let report = Strayed::builder("server")
        .file("tests/fixtures/alias-moved-stray.json")
        .check()
        .expect("checking resolves");

    assert!(
        report.unknown.iter().any(|unknown| unknown.path == "db"),
        "a stray `db` table in `[server]` is still a typo: {report}"
    );
}

/// A qualified old path can head a chain — `db::timeout_secs` to `mid`, `mid`
/// to the field — and nothing can point back at it, which is what keeps a
/// cross-section rename to one hop without a depth counter.
#[test]
fn a_cross_section_alias_can_head_a_chain() {
    Chained2::alias("db::timeout_secs", "mid").unwrap();
    Chained2::alias("mid", "timeout_secs").unwrap();

    assert_eq!(
        Chained2::builder("server")
            .file("tests/fixtures/alias-moved.json")
            .load()
            .expect("the chain carries the value")
            .timeout_secs,
        30
    );
}

/// The boundary, stated as a test: an alias reaches into the documents *this*
/// load reads. A section that lives somewhere this builder never looks is not
/// a rename away — it is a second configuration — and the load carries on
/// without it rather than failing.
#[test]
fn a_section_this_load_never_reads_supplies_nothing() {
    #[dynamic_config]
    #[derive(Debug, Deserialize)]
    struct Elsewhere {
        #[allow(dead_code)]
        port: u16,
        timeout_secs: Option<u64>,
    }

    Elsewhere::alias("absent::timeout_secs", "timeout_secs").unwrap();

    let loaded = Elsewhere::builder("server")
        .file("tests/fixtures/alias-moved.json")
        .load()
        .expect("a dead alias is not a failure");

    assert_eq!(loaded.timeout_secs, None);
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

/// The alias layer carries its supplier's provenance, so a path filled from
/// the *defaults* layer still looks like a default afterwards — and the pass's
/// old "a filled gap counts as supplied" bound then never terminated. The
/// bound is now what the pass itself filled.
#[test]
fn an_alias_fed_by_a_runtime_default_terminates() {
    #[dynamic_config]
    #[derive(Debug, Deserialize)]
    struct DefaultedSource {
        max_size: u16,
    }

    // The old spelling is all the defaults layer knows.
    DefaultedSource::set_default("size", 8u16).unwrap();
    DefaultedSource::alias("size", "max_size").unwrap();

    let loaded = DefaultedSource::builder("db").load();

    DefaultedSource::clear_aliases();
    DefaultedSource::clear_defaults();

    assert_eq!(loaded.expect("the alias carries the default").max_size, 8);
}

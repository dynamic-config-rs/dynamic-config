//! The loader's contract, exercised through the public API.
//!
//! These are the behaviours the crate promises regardless of how figment
//! happens to be wired underneath: precedence, missing files, error categories,
//! and how environment values are interpreted. If a figment upgrade changes one
//! of them, this suite is where it shows up.
//!
//! Exactly one test touches the environment. Tests share a process and run in
//! parallel, and reading configuration enumerates the whole environment, so a
//! second env-touching test would race it.

#![cfg(feature = "json")]

use dynamic_config::{load, ErrorKind, Format, LoadSpec, Source};
use serde::Deserialize;

#[derive(Debug, Deserialize, PartialEq)]
struct Db {
    host: String,
    port: u16,
}

fn json(text: &'static str) -> Source<'static> {
    Source::inline(text, Format::Json)
}

fn spec<'a>(sources: &'a [Source<'a>], env_prefix: Option<&'a str>) -> LoadSpec<'a> {
    let spec = LoadSpec::new("db", sources);

    match env_prefix {
        Some(prefix) => spec.with_env(prefix),
        None => spec,
    }
}

#[test]
fn later_sources_win_and_tables_merge() {
    let sources = [
        json(r#"{"db": {"host": "localhost", "port": 1}}"#),
        json(r#"{"db": {"port": 2}}"#),
    ];

    let db: Db = load(&spec(&sources, None)).expect("both layers should merge");

    assert_eq!(
        db,
        Db {
            host: "localhost".to_owned(),
            port: 2,
        }
    );
}

#[test]
fn a_missing_file_is_skipped() {
    let sources = [
        Source::file("tests/fixtures/absent.json", Format::Json),
        json(r#"{"db": {"host": "a", "port": 1}}"#),
    ];

    let db: Db = load(&spec(&sources, None)).expect("an absent file should not fail the load");

    assert_eq!(db.host, "a");
}

#[test]
fn each_struct_sees_only_its_own_section() {
    #[derive(Deserialize)]
    struct Server {
        port: u16,
    }

    let sources = [json(
        r#"{"db": {"host": "a", "port": 1}, "server": {"port": 8080}}"#,
    )];

    let server: Server = load(&LoadSpec::new("server", &sources))
        .expect("the server section is complete on its own");

    assert_eq!(server.port, 8080);
}

#[test]
fn a_required_value_that_no_source_supplies_is_a_missing_error() {
    let sources = [json(r#"{"db": {"host": "a"}}"#)];

    let error = load::<Db>(&spec(&sources, None)).expect_err("`port` is unset");

    assert_eq!(error.kind(), ErrorKind::Missing);
    assert_eq!(error.path(), "port");
}

#[test]
fn a_malformed_document_is_a_parse_error() {
    let sources = [json("{ not json")];

    let error = load::<Db>(&spec(&sources, None)).expect_err("the document is malformed");

    assert_eq!(error.kind(), ErrorKind::Parse);
}

#[test]
fn a_value_of_the_wrong_type_names_the_field() {
    let sources = [json(r#"{"db": {"host": "a", "port": "not-a-number"}}"#)];

    let error = load::<Db>(&spec(&sources, None)).expect_err("`port` is not a u16");

    assert_eq!(error.kind(), ErrorKind::Type);
    assert_eq!(error.path(), "port");
}

/// Provenance is reported for each kind of source, not only for files.
#[test]
fn every_kind_of_source_is_attributed() {
    use dynamic_config::{source_of, Origin};

    let inline = [json(r#"{"db": {"host": "a", "port": 1}}"#)];
    assert_eq!(
        source_of(&spec(&inline, None), "host").unwrap(),
        Some(Origin::Inline),
        "figment gives a string provider no source, only a name"
    );

    let file = [Source::file("tests/fixtures/base.json", Format::Json)];
    let origin = source_of(&spec(&file, None), "host").unwrap();
    assert!(
        matches!(origin, Some(Origin::File(ref path)) if path.ends_with("base.json")),
        "{origin:?}"
    );

    assert_eq!(
        source_of(&spec(&inline, None), "nothing_supplies_this").unwrap(),
        None
    );
}

/// A value the environment supplied is attributed to it, not left unknown.
#[test]
fn an_environment_value_names_its_prefix() {
    use dynamic_config::{source_of, Origin};

    std::env::set_var("DCORIGIN_DB_PORT", "7777");

    let sources = [json(r#"{"db": {"host": "a", "port": 1}}"#)];
    let spec = spec(&sources, Some("DCORIGIN_"));

    assert_eq!(
        source_of(&spec, "port").unwrap(),
        Some(Origin::Env("DCORIGIN_DB_*".to_owned()))
    );
    assert_eq!(
        source_of(&spec, "host").unwrap(),
        Some(Origin::Inline),
        "a key the environment does not set still points at the file"
    );

    std::env::remove_var("DCORIGIN_DB_PORT");
}

/// Everything else the environment layer promises.
///
/// Both env-touching tests in this file use distinct prefixes and distinct
/// variables, so they do not collide even though they run in parallel.
#[test]
fn the_environment_layers_over_the_files() {
    #[derive(Debug, Deserialize)]
    struct Wide {
        host: String,
        port: u16,
        enabled: bool,
        tags: Vec<String>,
        pool: Pool,
    }

    #[derive(Debug, Deserialize)]
    struct Pool {
        max_size: u16,
    }

    std::env::set_var("DCLOAD_DB_PORT", "7777");
    std::env::set_var("DCLOAD_DB_ENABLED", "true");
    std::env::set_var("DCLOAD_DB_TAGS", "[a, b, c]");
    // A doubled underscore introduces nesting; the single one inside
    // `max_size` stays part of the field name.
    std::env::set_var("DCLOAD_DB_POOL__MAX_SIZE", "42");

    let sources = [json(
        r#"{"db": {"host": "from-file", "port": 1, "enabled": false,
                   "tags": [], "pool": {"max_size": 1}}}"#,
    )];

    let db: Wide = load(&spec(&sources, Some("DCLOAD_"))).expect("the environment should coerce");

    assert_eq!(db.host, "from-file", "untouched fields come from the file");
    assert_eq!(db.port, 7777, "a numeric string reaches a u16");
    assert!(db.enabled, "a boolean string reaches a bool");
    assert_eq!(db.tags, ["a", "b", "c"], "a list literal reaches a Vec");
    assert_eq!(db.pool.max_size, 42, "`__` addresses a nested field");

    // An empty variable is unset by default, so the file value survives...
    std::env::set_var("DCLOAD_DB_HOST", "");

    let db: Wide = load(&spec(&sources, Some("DCLOAD_")))
        .expect("an empty variable should not blank out the file");
    assert_eq!(db.host, "from-file");

    // ...unless the caller says empty is a value it wants to be able to send.
    let allowing = spec(&sources, Some("DCLOAD_")).with_empty_env(true);
    let db: Wide = load(&allowing).expect("an empty string is a valid host");
    assert_eq!(db.host, "");

    for name in [
        "DCLOAD_DB_PORT",
        "DCLOAD_DB_ENABLED",
        "DCLOAD_DB_TAGS",
        "DCLOAD_DB_POOL__MAX_SIZE",
        "DCLOAD_DB_HOST",
    ] {
        std::env::remove_var(name);
    }
}

// ---------------------------------------------------------------------------
// Top-level keys are sections
// ---------------------------------------------------------------------------

/// The one top-level key that is not a section.
///
/// An editor finds a JSON file's schema through `$schema`, so refusing it would
/// mean the schema this crate emits could not be wired up in the file it
/// describes.
#[test]
fn a_schema_key_is_tolerated_at_the_top_level() {
    #[derive(Debug, Deserialize)]
    struct Db {
        host: String,
    }

    let text = r#"{"$schema": "https://example.invalid/config.json", "db": {"host": "localhost"}}"#;
    let sources = [Source::inline(text, Format::Json)];

    let db: Db = load(&LoadSpec::new("db", &sources)).expect("`$schema` is not a section");

    assert_eq!(db.host, "localhost");
}

/// Any *other* top-level scalar is still an error — and the error says why,
/// rather than leaving the reader to infer that top-level keys are sections.
#[test]
fn another_top_level_scalar_says_what_the_rule_is() {
    #[derive(Debug, Deserialize)]
    struct Db {
        #[allow(dead_code)]
        host: String,
    }

    let text = r#"{"_comment": "a note to myself", "db": {"host": "localhost"}}"#;
    let sources = [Source::inline(text, Format::Json)];

    let error = load::<Db>(&LoadSpec::new("db", &sources)).expect_err("a note is not a section");

    assert!(error.to_string().contains("_comment"), "{error}");
    assert!(error.to_string().contains("section"), "{error}");
}

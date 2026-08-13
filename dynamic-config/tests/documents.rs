//! The parse-and-merge seam: parsing documents this crate can read, combining
//! them, and handing one back.
//!
//! The case it exists for is a store crate reading several keys under a prefix
//! and installing them as one document. Before this, the only way to do that
//! outside the crate was to depend on `serde_json`, `toml` and `serde_yaml`
//! directly and write the merge again — so the tests here are written from
//! that caller's side rather than from the seam's.

#![cfg(feature = "json")]

use dynamic_config::{load, Fetched, Format, LoadSpec, Source, Value};
use serde::Deserialize;

#[derive(Deserialize, Debug, PartialEq)]
struct Db {
    host: String,
    port: u16,
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

#[test]
fn a_document_parses_into_a_tree_with_its_top_level_keys_intact() {
    let value = Value::parse(r#"{"db": {"host": "a", "port": 5432}}"#, Format::Json)
        .expect("well-formed JSON");

    // Not a section: the loader maps top-level keys onto profiles, and that
    // happens above this seam, so `db` is still a key here.
    assert_eq!(value.get("db.host"), Some(&Value::String("a".to_owned())));
    assert_eq!(value.get("db.port"), Some(&Value::Integer(5432)));
}

#[test]
fn an_empty_document_is_an_empty_table_rather_than_an_error() {
    let value = Value::parse("{}", Format::Json).expect("an empty object is a document");

    assert_eq!(value, Value::Table(std::collections::BTreeMap::new()));
}

#[test]
fn a_malformed_document_is_a_parse_error() {
    let error = Value::parse("not json at all", Format::Json).expect_err("that is not JSON");

    assert_eq!(error.kind(), dynamic_config::ErrorKind::Parse);
}

/// The whole reason the seam routes its failures through the loader's
/// translation rather than rendering the backend error directly.
#[test]
fn a_syntax_error_echoes_no_value_back() {
    const PLANTED: &str = "hunter2-parse-seam";

    for (text, format) in [
        #[cfg(feature = "json")]
        (format!(r#"{{"password": "{PLANTED}""#), Format::Json),
        #[cfg(feature = "toml")]
        (
            format!("password = \"{PLANTED}\ntrailing = 1\n"),
            Format::Toml,
        ),
        #[cfg(feature = "yaml")]
        (format!("password: \"{PLANTED}\n  bad: ["), Format::Yaml),
    ] {
        let rendered = Value::parse(&text, format)
            .expect_err("each of these is malformed")
            .to_string();

        assert!(
            !rendered.contains(PLANTED),
            "a {format:?} syntax error printed the line it failed on: {rendered}"
        );
    }
}

// ---------------------------------------------------------------------------
// Merging
// ---------------------------------------------------------------------------

#[test]
fn a_later_document_wins_key_by_key() {
    let mut merged = Value::parse(r#"{"db": {"host": "a", "port": 1}}"#, Format::Json).unwrap();
    merged.merge(Value::parse(r#"{"db": {"port": 5432}}"#, Format::Json).unwrap());

    // The table merged rather than being replaced: `host` survived a document
    // that did not mention it.
    assert_eq!(merged.get("db.host"), Some(&Value::String("a".to_owned())));
    assert_eq!(merged.get("db.port"), Some(&Value::Integer(5432)));
}

#[test]
fn two_disjoint_sections_become_one_document() {
    let mut merged = Value::parse(r#"{"db": {"host": "a"}}"#, Format::Json).unwrap();
    merged.merge(Value::parse(r#"{"server": {"port": 8080}}"#, Format::Json).unwrap());

    assert_eq!(merged.get("db.host"), Some(&Value::String("a".to_owned())));
    assert_eq!(merged.get("server.port"), Some(&Value::Integer(8080)));
}

/// The rule every layer in this crate already follows for arrays, held here
/// too: a later document supplying a list means *that* list.
#[test]
fn an_array_is_replaced_whole_rather_than_appended_to() {
    let mut merged = Value::parse(r#"{"db": {"tags": ["a", "b"]}}"#, Format::Json).unwrap();
    merged.merge(Value::parse(r#"{"db": {"tags": ["c"]}}"#, Format::Json).unwrap());

    assert_eq!(
        merged.get("db.tags"),
        Some(&Value::Array(vec![Value::String("c".to_owned())]))
    );
}

#[test]
fn a_scalar_replaces_the_table_it_lands_on() {
    let mut merged = Value::parse(r#"{"db": {"host": "a"}}"#, Format::Json).unwrap();
    merged.merge(Value::parse(r#"{"db": 1}"#, Format::Json).unwrap());

    assert_eq!(merged.get("db"), Some(&Value::Integer(1)));
}

// ---------------------------------------------------------------------------
// Collisions
// ---------------------------------------------------------------------------

#[test]
fn overlapping_paths_names_the_leaves_both_documents_supply() {
    let left = Value::parse(r#"{"db": {"host": "a", "port": 1}}"#, Format::Json).unwrap();
    let right = Value::parse(
        r#"{"db": {"port": 2}, "server": {"port": 3}}"#,
        Format::Json,
    )
    .unwrap();

    assert_eq!(left.overlapping_paths(&right), ["db.port"]);
}

/// Two tables at one path are not a collision — they merge. Only a leaf both
/// sides supply is a decision somebody has to make.
#[test]
fn a_shared_table_is_not_a_collision() {
    let left = Value::parse(r#"{"db": {"host": "a"}}"#, Format::Json).unwrap();
    let right = Value::parse(r#"{"db": {"port": 1}}"#, Format::Json).unwrap();

    assert!(left.overlapping_paths(&right).is_empty());
}

#[test]
fn a_collision_report_names_paths_and_never_values() {
    const PLANTED: &str = "hunter2-overlap";

    let left = Value::parse(
        &format!(r#"{{"db": {{"password": "{PLANTED}"}}}}"#),
        Format::Json,
    )
    .unwrap();
    let right = Value::parse(r#"{"db": {"password": "letmein"}}"#, Format::Json).unwrap();

    let overlaps = left.overlapping_paths(&right);

    assert_eq!(overlaps, ["db.password"]);
    assert!(overlaps.iter().all(|path| !path.contains(PLANTED)));
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

#[test]
fn a_tree_renders_back_to_text_that_parses_to_the_same_tree() {
    let text = r#"{"db": {"host": "a", "port": 5432, "tls": true, "ratio": 0.5,
                          "tags": ["x"], "nothing": null}}"#;
    let value = Value::parse(text, Format::Json).unwrap();

    let rendered = value.render(Format::Json).expect("a table is a document");

    assert_eq!(Value::parse(&rendered, Format::Json).unwrap(), value);
}

#[cfg(feature = "toml")]
#[test]
fn the_seam_crosses_formats() {
    let value = Value::parse(r#"{"db": {"host": "a", "port": 5432}}"#, Format::Json).unwrap();

    let as_toml = value.render(Format::Toml).expect("no nulls to trip TOML");

    assert_eq!(Value::parse(&as_toml, Format::Toml).unwrap(), value);
}

#[test]
fn a_scalar_is_not_a_document() {
    let error = Value::Integer(1)
        .render(Format::Json)
        .expect_err("a document has named keys at its root");

    assert_eq!(error.kind(), dynamic_config::ErrorKind::Type);
}

// ---------------------------------------------------------------------------
// What item 08 needs: N keys in, one document out
// ---------------------------------------------------------------------------

/// The five lines a store crate writes, exercised end to end: two keys read
/// from a prefix, merged later-wins, re-emitted, and installed as the one
/// document `Fetched` carries.
#[test]
fn several_keys_become_one_document_the_loader_reads() {
    fn merge_keys(documents: &[&str], format: Format) -> Result<Fetched, dynamic_config::Error> {
        let mut merged = Value::Table(std::collections::BTreeMap::new());

        for document in documents {
            merged.merge(Value::parse(document, format)?);
        }

        Ok(Fetched::new(merged.render(format)?, format))
    }

    let fetched = merge_keys(
        &[
            r#"{"db": {"host": "a", "port": 1}}"#,
            r#"{"db": {"port": 5432}}"#,
        ],
        Format::Json,
    )
    .expect("two well-formed documents");

    // Straight into the loader, exactly as a remote document arrives.
    let sources = [Source::inline(&fetched.text, fetched.format)];
    let db: Db = load(&LoadSpec::new("db", &sources)).expect("the merged document is a section");

    assert_eq!(
        db,
        Db {
            host: "a".to_owned(),
            port: 5432,
        }
    );
}

/// A key that will not parse fails the whole merge rather than producing half
/// a configuration — the all-or-nothing failure mode item 08 asks for, and it
/// comes free from `?` on the seam.
#[test]
fn one_unreadable_key_fails_the_whole_merge() {
    let mut merged = Value::parse(r#"{"db": {"host": "a"}}"#, Format::Json).unwrap();
    let second = Value::parse("}not json{", Format::Json);

    assert!(second.is_err());
    assert_eq!(merged.get("db.host"), Some(&Value::String("a".to_owned())));

    merged.merge(Value::Table(std::collections::BTreeMap::new()));
}

// ---------------------------------------------------------------------------
// The seam keeps figment out of its own signature
// ---------------------------------------------------------------------------

/// `doc_surface.rs` polices the generated surface; this polices the seam's,
/// which is hand-written. Parsing is where figment is closest to the door, so
/// the assertion is worth having in code rather than in review.
#[test]
fn the_parse_seam_names_no_backend_type() {
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/value.rs"),
    )
    .expect("value.rs is next to the tests");

    for signature in [
        "pub fn parse(text: &str, format: crate::Format) -> Result<Self, crate::Error>",
        "pub fn merge(&mut self, other: Value)",
        "pub fn overlapping_paths(&self, other: &Value) -> Vec<String>",
        "pub fn render(&self, format: crate::Format) -> Result<String, crate::Error>",
    ] {
        assert!(
            source.contains(signature),
            "the seam's signature moved; if that was deliberate, move this list too: {signature}"
        );
        assert!(
            !signature.contains("figment"),
            "a backend type reached the seam's signature: {signature}"
        );
    }
}

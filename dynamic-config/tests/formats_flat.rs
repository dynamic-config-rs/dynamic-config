//! The two flat formats — INI and properties — against their documented
//! dialects, and against each other.
//!
//! The one test that matters most is the last: the same logical document
//! written in TOML, INI and properties resolves to the same values. That
//! is the claim the Formats chapter makes, and it is the seed of the
//! cross-language conformance fixtures.

#![cfg(all(feature = "toml", feature = "ini", feature = "properties"))]

use dynamic_config::{load, Format, LoadSpec, Source};
use serde::Deserialize;

#[derive(Debug, Deserialize, PartialEq)]
struct Database {
    host: String,
    port: u16,
    #[serde(default)]
    tls: bool,
    pool: Pool,
}

#[derive(Debug, Deserialize, PartialEq)]
struct Pool {
    max: u32,
}

fn read<T: for<'de> Deserialize<'de>>(
    text: &str,
    format: Format,
) -> Result<T, dynamic_config::Error> {
    let sources = [Source::inline(text, format)];

    load::<T>(&LoadSpec::new("db", &sources))
}

// ── INI ────────────────────────────────────────────────────────────────

#[test]
fn ini_reads_sections_comments_and_quotes() {
    let document = r#"
; a comment
# another
[db]
host = "db.internal"
port = 5432
tls = true

[db.pool]
max = 8
"#;

    let database: Database = read(document, Format::Ini).expect("loads");

    assert_eq!(database.host, "db.internal");
    assert_eq!(database.port, 5432);
    assert!(database.tls);
    assert_eq!(database.pool.max, 8);
}

#[test]
fn ini_quoted_values_stay_strings_and_hashes_stay_in_values() {
    #[derive(Debug, Deserialize)]
    struct Section {
        version: String,
        secret: String,
    }

    let document = r#"
[db]
version = "1.10"
secret = a#b
"#;

    let section: Section = read(document, Format::Ini).expect("loads");

    // Unquoted `1.10` would widen to a float; the quotes refuse that.
    assert_eq!(section.version, "1.10");
    // A trailing-comment rule would have eaten `#b`.
    assert_eq!(section.secret, "a#b");
}

#[test]
fn ini_names_the_line_and_never_the_content() {
    let document = "[db]\npassword hunter2 with no equals\n";

    let error = read::<Database>(document, Format::Ini).expect_err("refused");
    let text = error.to_string();

    assert!(text.contains("line 2"), "{text}");
    assert!(
        !text.contains("hunter2"),
        "a value reached an error: {text}"
    );
}

#[test]
fn ini_a_scalar_cannot_also_be_a_table() {
    let document = "[db]\nhost = a\n[db.host]\nx = 1\n";

    let error = read::<Database>(document, Format::Ini).expect_err("refused");

    assert!(error.to_string().contains("host"), "{error}");
}

// ── properties ─────────────────────────────────────────────────────────

#[test]
fn properties_reads_dots_continuations_and_escapes() {
    #[derive(Debug, Deserialize)]
    struct Wide {
        host: String,
        motto: String,
        emoji: String,
        colon: String,
    }

    let document = "db.host = db.internal\n\
                    db.motto = one \\\n   two\n\
                    db.emoji = \\u00e9\n\
                    db.colon = a\\:b\n";

    let wide: Wide = read(document, Format::Properties).expect("loads");

    assert_eq!(wide.host, "db.internal");
    assert_eq!(wide.motto, "one two");
    assert_eq!(wide.emoji, "é");
    assert_eq!(wide.colon, "a:b");
}

#[test]
fn properties_colon_separates_too() {
    #[derive(Debug, Deserialize)]
    struct One {
        host: String,
    }

    let one: One = read("db.host: elsewhere\n", Format::Properties).expect("loads");

    assert_eq!(one.host, "elsewhere");
}

#[test]
fn properties_a_collision_is_an_error_naming_both_keys() {
    let document = "db.pool = 111\ndb.pool.max = 222\n";

    let error = read::<Database>(document, Format::Properties).expect_err("refused");
    let text = error.to_string();

    assert!(text.contains("pool"), "{text}");
    assert!(
        !text.contains("111") && !text.contains("222"),
        "a value leaked: {text}"
    );
}

// ── the claim ──────────────────────────────────────────────────────────

#[test]
fn one_document_three_formats_one_resolution() {
    let as_toml = r#"
[db]
host = "db.internal"
port = 5432
tls = true

[db.pool]
max = 8
"#;

    let as_ini = r#"
[db]
host = db.internal
port = 5432
tls = true

[db.pool]
max = 8
"#;

    let as_properties = "\
db.host = db.internal\n\
db.port = 5432\n\
db.tls = true\n\
db.pool.max = 8\n";

    let from_toml: Database = read(as_toml, Format::Toml).expect("toml loads");
    let from_ini: Database = read(as_ini, Format::Ini).expect("ini loads");
    let from_properties: Database =
        read(as_properties, Format::Properties).expect("properties loads");

    assert_eq!(from_toml, from_ini);
    assert_eq!(from_toml, from_properties);
}

#[test]
fn neither_flat_format_can_be_written() {
    use std::collections::BTreeMap;

    let document: BTreeMap<String, u32> = BTreeMap::new();
    let directory = tempfile::tempdir().expect("a directory");

    // RON and JSON5 are here for the same reason and not the same one:
    // nothing in this crate writes them and neither backend does either, so
    // a refusal that blamed a missing feature was wrong when the feature was
    // off (turning it on adds a reader, not a writer) and false when it was
    // on.
    for (name, format) in [
        ("out.ini", Format::Ini),
        ("out.properties", Format::Properties),
        ("out.ron", Format::Ron),
        ("out.json5", Format::Json5),
    ] {
        let error = dynamic_config::save(&document, directory.path().join(name), format, "db")
            .expect_err("refused");
        let text = error.to_string();

        assert!(
            text.contains("cannot be written"),
            "the refusal did not say why: {text}"
        );

        assert!(
            !text.contains("feature"),
            "the refusal blames a feature that would not help: {text}"
        );
    }
}

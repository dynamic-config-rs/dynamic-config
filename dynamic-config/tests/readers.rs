//! Which parser reads a document, and where the parsers disagree.
//!
//! Unlike the engines, readers are **not** interchangeable down to the
//! corner: a fold is one rule with three implementations, and a parser is
//! a dialect. So this file does two things. It holds the readers to
//! agreement on the shapes documents actually take — which is the part a
//! deployment depends on — and it *records* the places they diverge, so a
//! divergence is a decision somebody made rather than a surprise somebody
//! gets.

#![cfg(all(
    feature = "json",
    feature = "toml",
    feature = "yaml",
    feature = "ini",
    feature = "properties",
    feature = "figment"
))]

use dynamic_config::reader::{self, Reader};
use dynamic_config::{Format, Value};

/// Every reader that reads `format`, by the name it answers to.
fn readers(format: Format) -> Vec<(&'static str, &'static dyn Reader)> {
    reader::all()
        .into_iter()
        .filter(|reader| reader.reads(format))
        .map(|reader| (reader.name(), reader))
        .collect()
}

/// The same document, through every reader that can read it.
#[track_caller]
fn agree(format: Format, text: &str, what: &str) {
    let answers: Vec<(&str, String)> = readers(format)
        .into_iter()
        .map(|(name, reader)| {
            let parsed = reader.parse(text, format).map_or_else(
                |error| format!("error: {error}"),
                |value| format!("{value:?}"),
            );

            (name, parsed)
        })
        .collect();

    assert!(answers.len() > 1, "only one reader for {format:?}");

    let (reference_name, reference) = &answers[0];

    for (name, answer) in &answers[1..] {
        assert_eq!(
            answer, reference,
            "the {reference_name} and {name} readers disagree on {what}"
        );
    }
}

#[test]
fn the_shapes_a_document_takes_read_the_same() {
    agree(
        Format::Json,
        r#"{"db": {"host": "h", "port": 5432, "tls": true, "tags": ["a", "b"], "ratio": 1.5}}"#,
        "a JSON document",
    );
    agree(
        Format::Toml,
        "[db]\nhost = 'h'\nport = 5432\ntls = true\ntags = ['a', 'b']\nratio = 1.5\n",
        "a TOML document",
    );
    agree(
        Format::Yaml,
        "db:\n  host: h\n  port: 5432\n  tls: true\n  tags: [a, b]\n  ratio: 1.5\n",
        "a YAML document",
    );
}

/// Nesting, which is where a section lives.
/// A TOML datetime reaches a configuration as the text it was written
/// as, whoever read it.
///
/// The `toml` parser hands one to serde as a one-key table under a
/// private name of its own, and two of the three readers here go through
/// that parser — so left alone, a datetime arrived as a table with a key
/// from inside a dependency in it. A `String` field refused it, a
/// `chrono::DateTime` field refused it, and the error named
/// `$__toml_private_datetime`.
#[test]
fn a_toml_datetime_is_the_text_it_was_written_as() {
    for (what, text, written) in [
        (
            "an offset datetime",
            "a = 1979-05-27T07:32:00Z\n",
            "1979-05-27T07:32:00Z",
        ),
        ("a local date", "a = 1979-05-27\n", "1979-05-27"),
        ("a local time", "a = 07:32:00\n", "07:32:00"),
    ] {
        for (name, reader) in readers(Format::Toml) {
            let parsed = reader.parse(text, Format::Toml).expect("valid TOML");

            assert_eq!(
                parsed.get("a"),
                Some(&Value::String(written.to_owned())),
                "the {name} reader on {what}"
            );
        }

        agree(Format::Toml, text, what);
    }
}

#[test]
fn nesting_reads_the_same() {
    agree(
        Format::Json,
        r#"{"db": {"pool": {"max": 32, "min": 1}}, "other": {"x": 1}}"#,
        "a nested JSON document",
    );
    agree(
        Format::Toml,
        "[db.pool]\nmax = 32\nmin = 1\n\n[other]\nx = 1\n",
        "a nested TOML document",
    );
    agree(
        Format::Yaml,
        "db:\n  pool:\n    max: 32\n    min: 1\nother:\n  x: 1\n",
        "a nested YAML document",
    );
}

/// An empty document is an empty document, whoever reads it.
#[test]
fn an_empty_document_reads_the_same() {
    agree(Format::Json, "{}", "an empty JSON object");
    agree(Format::Toml, "", "an empty TOML document");
}

/// A parse failure never carries the document.
///
/// The line that failed to parse is, on a bad day, the line holding the
/// password — so this holds every reader to the rule, not only the one
/// this crate wrote.
#[test]
fn no_reader_echoes_the_document_back() {
    let secret = "hunter2";
    let broken = format!("[db]\npassword = \"{secret}\n");

    for (name, reader) in readers(Format::Toml) {
        let error = reader
            .parse(&broken, Format::Toml)
            .expect_err("an unterminated string is not TOML")
            .to_string();

        assert!(
            !error.contains(secret),
            "the {name} reader put the value in its message: {error}"
        );
    }
}

// ---------------------------------------------------------------------------
// Where they diverge, recorded
// ---------------------------------------------------------------------------

/// **`.properties` has one parser.** Neither backend crate ships one —
/// `config` reads six formats and figment three, and `.properties` is in
/// neither list — so exactly one reader here answers `reads` for it.
///
/// What that does *not* mean is in
/// [`a_properties_document_reads_whichever_reader_was_chosen`].
#[test]
fn properties_is_this_crates_alone() {
    let reading: Vec<&str> = readers(Format::Properties)
        .into_iter()
        .map(|(name, _)| name)
        .collect();

    assert_eq!(reading, ["native"], "{reading:?}");
}

/// **Choosing a backend's reader does not cost you `.properties`.** A
/// format the chosen reader cannot read is handed to one that can, so a
/// load that asks for `config-rs` — for its YAML, say — still reads the
/// properties file beside it, and reads it the same way.
///
/// This is the guarantee the row above is easy to misread as denying: one
/// parser, reachable from every reader.
#[test]
fn a_properties_document_reads_whichever_reader_was_chosen() {
    use dynamic_config::{load, LoadSpec, Source};

    #[derive(Debug, serde::Deserialize, PartialEq)]
    struct Db {
        host: String,
        port: u16,
        pool: Pool,
    }

    #[derive(Debug, serde::Deserialize, PartialEq)]
    struct Pool {
        max: u32,
    }

    let text = "db.host = db.internal\ndb.port = 5432\ndb.pool.max = 32\n";
    let sources = [Source::inline(text, Format::Properties)];

    let answers: Vec<(&str, Db)> = reader::all()
        .into_iter()
        .map(|reader| {
            let loaded: Db = load(&LoadSpec::new("db", &sources).with_reader(reader))
                .unwrap_or_else(|error| panic!("the {} reader: {error}", reader.name()));

            (reader.name(), loaded)
        })
        .collect();

    assert!(answers.len() > 1, "only one reader is installed");

    for (name, answer) in &answers[1..] {
        assert_eq!(
            answer, &answers[0].1,
            "the {} and {name} readers disagree on a properties document",
            answers[0].0
        );
    }
}

/// **INI is two dialects.** This crate's own is the one the book
/// specifies — `[a.b]` nests, quotes are stripped, a `#` inside a value
/// stays in it. `rust-ini`, which the backend uses, answers differently,
/// and a load that asks for the backend's reader gets the backend's INI.
///
/// Asserted rather than assumed, because "they differ" is the reason the
/// native reader is the default and the reason this is written down.
#[test]
fn the_two_ini_dialects_are_not_the_same() {
    let text = "[a.b]\nhost = \"x\"\n";

    let native = reader::native()
        .parse(text, Format::Ini)
        .expect("this crate's INI reads it");
    let backend = reader::config_rs()
        .parse(text, Format::Ini)
        .expect("the backend's INI reads it");

    assert!(
        native.get("a.b").is_some(),
        "this crate's INI nests `[a.b]`: {native:?}"
    );
    assert_ne!(
        format!("{native:?}"),
        format!("{backend:?}"),
        "if these ever agree, the note in the book and the default reader \
         both want revisiting"
    );
}

/// **The backend reads two formats nothing here parses.** RON and JSON5
/// have no reader in this crate, and a load asking for one gets the
/// backend's without having to install a reader by hand.
#[test]
fn the_backend_brings_formats_this_crate_has_no_parser_for() {
    assert!(!reader::native().reads(Format::Ron));
    assert!(reader::config_rs().reads(Format::Ron));

    let parsed = dynamic_config::Value::parse("(db: (port: 5432))", Format::Ron)
        .expect("RON falls through to the reader that reads it");

    assert_eq!(parsed.get("db.port"), Some(&Value::Integer(5432)));
}

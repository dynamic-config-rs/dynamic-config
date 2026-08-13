//! What the loader does with the shape of a document, and with keys that
//! are on one side of the deal and not the other.
//!
//! Four questions, each of which somebody has to be able to answer from a
//! test rather than from a guess:
//!
//! 1. Must a file be sectioned? (No — `whole_document` reads a bare
//!    `{"host": …, "port": …}`, and a sectioned load refuses one with a
//!    message that names the fix.)
//! 2. A key the file supplies and the struct does not name: ignored, or an
//!    error? (Ignored — and reported by `check`, or refused outright by
//!    `#[serde(deny_unknown_fields)]`.)
//! 3. Two files, half the struct in each: does that work? (Yes; later
//!    files win where they overlap.)
//! 4. A field no source supplies: what happens? (`ErrorKind::Missing`,
//!    naming the field — unless a default or an `Option` covers it.)
//!
//! The book says all of this in prose (`document-shape.md`); this is the
//! copy that fails when it stops being true.
#![cfg(feature = "json")]

use dynamic_config::{Builder, ErrorKind};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
// Read through `Debug` and through the assertions, which dead-code
// analysis does not count.
#[allow(dead_code)]
struct Server {
    host: String,
    port: u16,
}

/// Somewhere to write fixtures. Under the crate, never the system
/// temporary directory: `tests/scratch/` is where this suite's files go.
fn write(name: &str, text: &str) -> String {
    let directory = std::path::Path::new("tests/scratch/document-shape");
    std::fs::create_dir_all(directory).expect("the scratch directory is writable");

    let path = directory.join(name);
    std::fs::write(&path, text).expect("the scratch directory is writable");

    path.display().to_string()
}

// ---------------------------------------------------------------------------
// 1. A document with no section header
// ---------------------------------------------------------------------------

/// The shape a container image or another tool's file arrives in: no
/// header, the whole document is the configuration.
#[test]
fn a_whole_document_needs_no_section_header() {
    let file = write("bare.json", r#"{"host": "0.0.0.0", "port": 8000}"#);

    let server: Server = Builder::new("server")
        .whole_document()
        .file(&file)
        .load()
        .expect("the document is the section");

    assert_eq!(server.host, "0.0.0.0");
    assert_eq!(server.port, 8000);
}

/// Without saying so, the same file is refused — and the refusal names the
/// method that reads it, because "top-level key `host` is not a table" is
/// only obvious to somebody who already knows this crate's layout.
#[test]
fn a_sectioned_load_refuses_a_bare_document_and_says_what_to_do() {
    let file = write("bare-refused.json", r#"{"host": "0.0.0.0", "port": 8000}"#);

    let error = Builder::<Server>::new("server")
        .file(&file)
        .load()
        .expect_err("a bare document has no section to select");

    let message = error.to_string();
    assert!(message.contains("is not a table"), "{message}");
    assert!(message.contains("whole_document"), "{message}");
}

/// The key is not consumed by the document, so it keeps every other job it
/// has: the environment prefix is still `{prefix}{KEY}_`.
#[test]
fn the_environment_still_layers_over_a_whole_document() {
    let file = write("bare-env.json", r#"{"host": "0.0.0.0", "port": 8000}"#);

    // A variable of this test's own, as every environment test here does:
    // the suite runs in parallel in one process.
    std::env::set_var("DOCSHAPEENV_SERVER_PORT", "9999");

    let server: Server = Builder::new("server")
        .whole_document()
        .file(&file)
        .env("DOCSHAPEENV_")
        .load()
        .expect("the environment reads over the document");

    std::env::remove_var("DOCSHAPEENV_SERVER_PORT");

    assert_eq!(server.port, 9999, "the environment wins, as it always does");
}

/// A configuration with nothing to call itself. The empty key contributes
/// nothing to the prefix rather than an extra underscore — `APP__PORT` is a
/// variable nobody would guess they had to set.
#[test]
fn an_empty_key_names_nothing_and_the_prefix_stands_alone() {
    let file = write("bare-nameless.json", r#"{"host": "0.0.0.0", "port": 8000}"#);

    std::env::set_var("DOCSHAPENAMELESS_PORT", "7777");

    let server: Server = Builder::new("")
        .whole_document()
        .file(&file)
        .env("DOCSHAPENAMELESS_")
        .load()
        .expect("a configuration may have no name");

    std::env::remove_var("DOCSHAPENAMELESS_PORT");

    assert_eq!(server.host, "0.0.0.0");
    assert_eq!(server.port, 7777);
}

/// Profile variants are a rule about *file names*, so they are untouched by
/// what is inside the file.
#[test]
fn a_profile_variant_overlays_a_whole_document_too() {
    let base = write("profiled.json", r#"{"host": "0.0.0.0", "port": 8000}"#);
    write("profiled.production.json", r#"{"port": 443}"#);

    std::env::set_var("DOCSHAPEPROFILE_ENV", "production");

    let server: Server = Builder::new("server")
        .whole_document()
        .file(&base)
        .profile_env("DOCSHAPEPROFILE_ENV")
        .load()
        .expect("the variant is a sibling file like any other");

    std::env::remove_var("DOCSHAPEPROFILE_ENV");

    assert_eq!(server.port, 443);
    assert_eq!(server.host, "0.0.0.0", "the base file still supplies this");
}

/// `check` answers about a whole document exactly as it does about a
/// section: the paths are relative to the configuration either way.
#[test]
fn check_reports_on_a_whole_document_like_any_other() {
    let file = write("bare-checked.json", r#"{"host": "0.0.0.0", "port": 8000}"#);

    let report = Builder::<Server>::new("server")
        .whole_document()
        .file(&file)
        .check()
        .expect("the report is produced whether or not the load would work");

    assert!(report.is_clean(), "{report}");
    assert!(report.resolved.iter().any(|it| it.path == "host"));
    assert!(report.resolved.iter().any(|it| it.path == "port"));
}

// ---------------------------------------------------------------------------
// 2. A key the file has and the struct does not
// ---------------------------------------------------------------------------

/// Ignored, deliberately: a file this crate reads may be shared with
/// another configuration type, another tool, or a later version of this
/// program. Refusing what one struct does not name would make a shared
/// file impossible.
#[test]
fn a_key_the_struct_does_not_name_is_ignored_by_the_load() {
    let file = write(
        "extra.json",
        r#"{"server": {"host": "0.0.0.0", "port": 8000, "hsot": "typo", "extra": 1}}"#,
    );

    let server: Server = Builder::new("server")
        .file(&file)
        .load()
        .expect("an unknown key is not a reason to refuse to boot");

    assert_eq!(server.host, "0.0.0.0");
}

/// Ignored is not unnoticed. `check` compares the section's top-level keys
/// against the field list the generated `builder()` carries, and a near
/// miss gets a suggestion — which is the answer to "why is my typo silently
/// doing nothing".
#[test]
fn check_names_the_key_the_struct_does_not_have_and_guesses_the_typo() {
    let file = write(
        "extra-checked.json",
        r#"{"server": {"host": "0.0.0.0", "port": 8000, "hsot": "typo"}}"#,
    );

    let report = Builder::<Server>::new("server")
        .file(&file)
        .with_fields(&["host", "port"])
        .check()
        .expect("the report is produced");

    assert!(report.unknown_checked);
    assert!(
        !report.is_clean(),
        "an unknown key makes the report unclean"
    );

    let unknown = report
        .unknown
        .iter()
        .find(|it| it.path == "hsot")
        .expect("the typo is reported");

    assert_eq!(unknown.suggestion.as_deref(), Some("host"));
}

/// And a struct that wants the strict reading has serde's own switch. The
/// engine does not second-guess it: `deny_unknown_fields` is a statement
/// about the type, and it turns the same file into a load-time error.
#[test]
fn deny_unknown_fields_turns_the_same_file_into_a_refusal() {
    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    #[allow(dead_code)]
    struct Strict {
        host: String,
    }

    let file = write(
        "strict.json",
        r#"{"server": {"host": "0.0.0.0", "port": 1}}"#,
    );

    let error = Builder::<Strict>::new("server")
        .file(&file)
        .load()
        .expect_err("`port` is a field this struct does not have");

    assert!(error.to_string().contains("unknown field"), "{error}");
    assert_eq!(
        error.path(),
        "port",
        "the error names the key, not the struct"
    );
}

// ---------------------------------------------------------------------------
// 3. One struct, two files, half of it in each
// ---------------------------------------------------------------------------

/// The layering rule applied to the ordinary case: files are merged key by
/// key, so no single file has to be complete.
#[test]
fn two_files_each_holding_half_the_struct_make_one_configuration() {
    let host = write("half-host.json", r#"{"server": {"host": "0.0.0.0"}}"#);
    let port = write("half-port.json", r#"{"server": {"port": 8000}}"#);

    let server: Server = Builder::new("server")
        .file(&host)
        .file(&port)
        .load()
        .expect("between them the two files say everything");

    assert_eq!(server.host, "0.0.0.0");
    assert_eq!(server.port, 8000);
}

/// The same, for a whole-document load: the shape of the file does not
/// change how files layer.
#[test]
fn two_whole_documents_layer_the_same_way() {
    let host = write("whole-host.json", r#"{"host": "0.0.0.0"}"#);
    let port = write("whole-port.json", r#"{"port": 8000}"#);

    let server: Server = Builder::new("server")
        .whole_document()
        .file(&host)
        .file(&port)
        .load()
        .expect("between them the two documents say everything");

    assert_eq!(server.port, 8000);
}

/// Where they overlap, the later file wins — call order is the precedence,
/// which is what makes a small `secrets.json` after a large `config.toml`
/// work.
#[test]
fn where_two_files_overlap_the_later_one_wins() {
    let first = write("over-first.json", r#"{"server": {"host": "a", "port": 1}}"#);
    let second = write("over-second.json", r#"{"server": {"port": 2}}"#);

    let server: Server = Builder::new("server")
        .file(&first)
        .file(&second)
        .load()
        .expect("both files read");

    assert_eq!(server.host, "a", "the second file said nothing about this");
    assert_eq!(server.port, 2);
}

/// A file that is not there is skipped rather than refused, which is the
/// property an optional `secrets.json` rests on.
#[test]
fn a_file_that_is_not_there_is_skipped() {
    let present = write("present.json", r#"{"server": {"host": "a", "port": 1}}"#);

    let server: Server = Builder::new("server")
        .file(&present)
        .file("tests/scratch/document-shape/absent.json")
        .load()
        .expect("a missing file is an empty layer");

    assert_eq!(server.port, 1);
}

// ---------------------------------------------------------------------------
// 4. A field no source supplies
// ---------------------------------------------------------------------------

/// The load fails, and the error is precise about which field and why —
/// `ErrorKind::Missing` is the kind a caller matches on to tell "you have
/// not configured this yet" from "the store is down".
#[test]
fn a_field_no_source_supplies_fails_the_load_and_names_itself() {
    let file = write("incomplete.json", r#"{"server": {"host": "0.0.0.0"}}"#);

    let error = Builder::<Server>::new("server")
        .file(&file)
        .load()
        .expect_err("`port` has no value anywhere");

    assert_eq!(error.kind(), ErrorKind::Missing);
    assert_eq!(error.path(), "port");
    assert!(error.to_string().contains("missing field"), "{error}");
}

/// The same file loads once the type says what to do without a value.
/// `#[serde(default)]` and `Option<T>` are the type's answer; the engine
/// has nothing to add.
#[test]
fn a_default_or_an_option_covers_what_no_source_supplies() {
    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    struct Tolerant {
        host: String,
        #[serde(default = "eight_thousand")]
        port: u16,
        tags: Option<Vec<String>>,
    }

    fn eight_thousand() -> u16 {
        8000
    }

    let file = write("incomplete-ok.json", r#"{"server": {"host": "0.0.0.0"}}"#);

    let config: Tolerant = Builder::new("server")
        .file(&file)
        .load()
        .expect("the type says what to do without a value");

    assert_eq!(config.port, 8000);
    assert!(config.tags.is_none());
}

/// A section no file mentions is not a special case — it is a section with
/// no values, which is the missing-field error again. There is no "unknown
/// section" to report: a section exists exactly where a value does.
#[test]
fn a_section_no_file_mentions_reads_as_missing_fields() {
    let file = write("elsewhere.json", r#"{"database": {"url": "postgres://"}}"#);

    let error = Builder::<Server>::new("server")
        .file(&file)
        .load()
        .expect_err("nothing in this file is the server's");

    assert_eq!(error.kind(), ErrorKind::Missing);
    assert_eq!(error.path(), "host", "the first field it looks for");
}

/// And a failing load still reports: `check` answers when `load` cannot,
/// which is the whole reason it is a separate call. The failure is in the
/// report rather than in place of it.
#[test]
fn check_still_reports_when_the_load_would_fail() {
    let file = write(
        "incomplete-checked.json",
        r#"{"server": {"host": "0.0.0.0"}}"#,
    );

    let report = Builder::<Server>::new("server")
        .file(&file)
        .with_fields(&["host", "port"])
        .check()
        .expect("a report is produced even though the load fails");

    assert!(!report.is_clean());
    assert!(
        report
            .failure
            .as_deref()
            .is_some_and(|it| it.contains("port")),
        "{report}"
    );
    assert!(
        report.resolved.iter().any(|it| it.path == "host"),
        "what *is* configured is still reported: {report}"
    );
}

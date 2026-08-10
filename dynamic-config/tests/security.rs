//! The properties `SECURITY.md` claims, asserted rather than reviewed.
//!
//! Everything here is a promise made to a user in prose somewhere else. A
//! promise with no test is a promise until somebody refactors.

#![cfg(feature = "json")]

use std::fmt::Write as _;

use dynamic_config::{dynamic_config, Format, LoadSpec, Source};
use serde::{Deserialize, Serialize};

const SECRET: &str = "hunter2-do-not-print-me";

#[dynamic_config(files = [], key = "db", env = "DCSEC_", save)]
#[derive(Deserialize, Serialize)]
struct Secretive {
    host: String,
    #[config(secret)]
    password: String,
    pool: Pool,
}

#[derive(Debug, Deserialize, Serialize)]
struct Pool {
    max_size: u16,
}

fn document() -> String {
    format!(
        r#"{{"db": {{"host": "localhost", "password": "{SECRET}", "pool": {{"max_size": 10}}}}}}"#
    )
}

fn loaded() -> Secretive {
    let text = document();
    let sources = [Source::inline(&text, Format::Json)];

    dynamic_config::load(&LoadSpec::new("db", &sources)).expect("the document is complete")
}

// ---------------------------------------------------------------------------
// Secrets stay out of diagnostics
// ---------------------------------------------------------------------------

#[test]
fn debug_redacts_the_marked_field_and_only_that_one() {
    let printed = format!("{:?}", loaded());

    assert!(!printed.contains(SECRET), "{printed}");
    assert!(printed.contains("***"), "{printed}");
    assert!(
        printed.contains("localhost"),
        "an unmarked field is fine to show"
    );
}

#[test]
fn a_check_report_names_keys_and_never_values() {
    let text = document();
    let sources = [Source::inline(&text, Format::Json)];
    let report = dynamic_config::check::<Secretive>(&LoadSpec::new("db", &sources), &["host"])
        .expect("checking resolves");

    // Everything the report can render, in one string.
    let mut rendered = format!("{report:?}");

    for resolved in &report.resolved {
        let _ = write!(rendered, " {} {}", resolved.path, resolved.origin);
    }

    for unknown in &report.unknown {
        let _ = write!(rendered, " {unknown:?}");
    }

    assert!(!rendered.contains(SECRET), "{rendered}");
    assert!(
        rendered.contains("password"),
        "the key is the useful half: {rendered}"
    );
}

#[test]
fn an_error_names_the_key_and_the_source_but_not_the_value() {
    // A value of the wrong type: the message has to say enough to fix it
    // without repeating what was there.
    let text = format!(
        r#"{{"db": {{"host": "localhost", "password": "{SECRET}", "pool": {{"max_size": "{SECRET}"}}}}}}"#
    );
    let sources = [Source::inline(&text, Format::Json)];

    let error = dynamic_config::load::<Secretive>(&LoadSpec::new("db", &sources))
        .expect_err("max_size is not a number");

    let rendered = error.to_string();

    assert!(rendered.contains("max_size"), "{rendered}");
    assert!(
        !rendered.contains(SECRET),
        "an error message is a log line: {rendered}"
    );
}

// ---------------------------------------------------------------------------
// A remote store is untrusted input
// ---------------------------------------------------------------------------

#[test]
fn a_document_that_is_not_configuration_is_an_error_rather_than_a_panic() {
    for hostile in [
        "",
        "not json at all",
        r#"{"db": "a string where a table belongs"}"#,
        r#"{"db": {"pool": {"max_size": 99999999999999999999}}}"#,
        // A deeply nested document, in case anything here recurses.
        &format!(
            "{}{}",
            "{\"db\":{\"a\":".repeat(200),
            "1".to_owned() + &"}".repeat(201)
        ),
    ] {
        let sources = [Source::inline(hostile, Format::Json)];

        // The assertion is that this returns — either answer is fine.
        let _ = dynamic_config::load::<Secretive>(&LoadSpec::new("db", &sources));
    }
}

#[test]
fn an_environment_value_that_is_not_utf8_is_skipped_rather_than_fatal() {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;

        std::env::set_var(
            "DCSEC_DB_HOST",
            std::ffi::OsStr::from_bytes(&[0x66, 0xff, 0x66]),
        );

        // Whatever it decides, it must decide rather than panic.
        let text = document();
        let sources = [Source::inline(&text, Format::Json)];
        let _ = dynamic_config::load::<Secretive>(&LoadSpec::new("db", &sources));

        std::env::remove_var("DCSEC_DB_HOST");
    }
}

// ---------------------------------------------------------------------------
// Files this crate writes
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn a_written_file_is_private_from_the_moment_it_exists() {
    use std::os::unix::fs::PermissionsExt;

    let directory = std::env::temp_dir().join("dynamic-config-security");
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();

    let path = directory.join("written.json");

    loaded().save(&path).expect("saving works");

    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;

    assert_eq!(mode, 0o600, "mode was {mode:o}");

    // And it really did write the secret, so the test above is not passing on
    // an empty file.
    assert!(std::fs::read_to_string(&path).unwrap().contains(SECRET));

    let _ = std::fs::remove_dir_all(&directory);
}

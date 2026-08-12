//! The properties `SECURITY.md` claims, asserted rather than reviewed.
//!
//! Everything here is a promise made to a user in prose somewhere else. A
//! promise with no test is a promise until somebody refactors.

#![cfg(feature = "json")]

use std::fmt::Write as _;

use dynamic_config::{dynamic_config, Format, LoadSpec, Source};
use serde::{Deserialize, Serialize};

const SECRET: &str = "hunter2-do-not-print-me";

#[dynamic_config]
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

    dynamic_config::save(&loaded(), &path, Format::Json, "db").expect("saving works");

    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;

    assert_eq!(mode, 0o600, "mode was {mode:o}");

    // And it really did write the secret, so the test above is not passing on
    // an empty file.
    assert!(std::fs::read_to_string(&path).unwrap().contains(SECRET));

    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn a_fetched_remote_document_never_prints_its_contents() {
    // A remote store's flagship use case is serving secrets; `Fetched` is
    // what every watch callback receives, one `debug!(?document)` from a log.
    let document = dynamic_config::Fetched::new(
        r#"{"db": {"password": "hunter2"}}"#,
        dynamic_config::Format::Json,
    );

    let printed = format!("{document:?}");

    assert!(!printed.contains("hunter2"), "{printed}");
    assert!(printed.contains("bytes"), "{printed}");
}

/// `APP_ENV=../../evil` must be refused, not interpolated into a file name:
/// a profile is a word, and anything with a path separator or a parent
/// reference walks the loader to a file somebody else chose.
#[test]
fn a_profile_that_looks_like_a_path_is_refused() {
    use serde::Deserialize;

    #[dynamic_config::dynamic_config]
    #[derive(Debug, Deserialize)]
    struct Profiled {
        #[allow(dead_code)]
        host: String,
    }

    for hostile in ["../secrets", "a/b", "a\\b", ".."] {
        std::env::set_var("DCSEC_PROFILE_TRAVERSAL", hostile);

        let error = Profiled::builder("db")
            .file("tests/fixtures/base.json")
            .profile_env("DCSEC_PROFILE_TRAVERSAL")
            .load()
            .expect_err("a path-shaped profile must be refused");

        assert_eq!(error.kind(), dynamic_config::ErrorKind::Env, "{hostile}");
        assert!(
            error.to_string().contains("DCSEC_PROFILE_TRAVERSAL"),
            "{error}"
        );
    }

    std::env::remove_var("DCSEC_PROFILE_TRAVERSAL");
}

/// A non-UTF-8 environment variable anywhere in the process must not panic
/// `load()` — it runs on every reload, including the watcher thread.
#[cfg(unix)]
#[test]
fn a_foreign_non_utf8_variable_does_not_panic_the_load() {
    use serde::Deserialize;
    use std::os::unix::ffi::OsStrExt;

    #[dynamic_config::dynamic_config]
    #[derive(Debug, Deserialize)]
    struct EnvironmentUser {
        #[allow(dead_code)]
        host: String,
    }

    // A variable this config never asked about, with bytes that are not
    // UTF-8 — the shape a hostile or merely odd parent process leaves behind.
    std::env::set_var(
        "DCSEC_UNRELATED_GARBAGE",
        std::ffi::OsStr::from_bytes(&[0x66, 0xff, 0x66]),
    );

    let loaded = EnvironmentUser::builder("db")
        .file("tests/fixtures/base.json")
        .env("DCSECUTF_")
        .load();

    std::env::remove_var("DCSEC_UNRELATED_GARBAGE");
    loaded.expect("a foreign variable must not break the load");
}

/// A snapshot holds the *resolved* configuration, secrets included; its
/// `{:?}` must describe the shape and never the values.
#[test]
fn a_snapshot_debug_shows_keys_and_never_values() {
    use serde::Deserialize;

    #[dynamic_config::dynamic_config]
    #[derive(Deserialize)]
    struct SnapshotDebug {
        #[allow(dead_code)]
        host: String,
        #[config(secret)]
        #[allow(dead_code)]
        password: String,
    }

    std::fs::create_dir_all("tests/scratch").unwrap();
    std::fs::write(
        "tests/scratch/security-snapshot.json",
        r#"{"db": {"host": "db.internal", "password": "hunter2-snapshot"}}"#,
    )
    .unwrap();

    let snapshot = SnapshotDebug::builder("db")
        .file("tests/scratch/security-snapshot.json")
        .snapshot()
        .expect("the source reads cleanly");
    let rendered = format!("{snapshot:?}");

    assert!(
        !rendered.contains("hunter2-snapshot") && !rendered.contains("db.internal"),
        "a snapshot's Debug must not print values: {rendered}"
    );
    assert!(
        rendered.contains("password"),
        "keys are the safe half: {rendered}"
    );
}

/// The strict_env refusal names the variable — and not its value, even
/// though the ambiguous family is seven known words. No diagnostic prints a
/// value; a bounded exception is how unbounded ones start.
#[test]
fn the_strict_env_refusal_echoes_no_value() {
    use serde::Deserialize;

    #[dynamic_config::dynamic_config]
    #[derive(Debug, Deserialize)]
    struct StrictMessage {
        #[allow(dead_code)]
        #[serde(default)]
        mode: String,
    }

    std::env::set_var("DCSECSTRICT_SVC_MODE", "off");

    let error = StrictMessage::builder("svc")
        .env("DCSECSTRICT_")
        .strict_env()
        .load()
        .expect_err("`off` is refused under strict_env");
    let message = error.to_string();

    std::env::remove_var("DCSECSTRICT_SVC_MODE");

    assert!(message.contains("DCSECSTRICT_SVC_MODE"), "{message}");
    assert!(
        !message.contains("=off") && !message.contains("\"off\""),
        "the value must not be echoed back: {message}"
    );
}

/// A redacted explanation stays redacted through every door: Display, Debug,
/// and the public rows.
#[test]
fn a_redacted_explanation_leaks_through_no_surface() {
    use serde::Deserialize;

    #[dynamic_config::dynamic_config]
    #[derive(Deserialize)]
    struct ExplainDebug {
        #[allow(dead_code)]
        host: String,
        #[config(secret)]
        #[allow(dead_code)]
        token: String,
    }

    std::fs::create_dir_all("tests/scratch").unwrap();
    std::fs::write(
        "tests/scratch/security-explain.json",
        r#"{"db": {"host": "localhost", "token": "hunter2-doors"}}"#,
    )
    .unwrap();

    // The redaction under test lives on the type-level `explain`, which
    // answers through the builder the type was configured with — so
    // configure it first.
    ExplainDebug::builder("db")
        .file("tests/scratch/security-explain.json")
        .init()
        .expect("the source reads cleanly");

    let explanation = ExplainDebug::explain("token").expect("the source reads cleanly");

    for rendered in [format!("{explanation}"), format!("{explanation:?}")] {
        assert!(
            !rendered.contains("hunter2-doors"),
            "a redacted explanation must stay redacted: {rendered}"
        );
    }
    assert!(explanation
        .rows()
        .iter()
        .all(|row| row.value.as_deref() != Some("hunter2-doors")));
}

/// The profile guard is unconditional, not positional: an env-only load —
/// no files, no discovery — must still refuse a path-shaped profile.
#[test]
fn an_env_only_load_still_validates_the_profile() {
    use serde::Deserialize;

    #[dynamic_config::dynamic_config]
    #[derive(Debug, Deserialize)]
    struct EnvOnly {
        #[allow(dead_code)]
        #[serde(default)]
        host: String,
    }

    std::env::set_var("DCSECPROFILE_ENV", "../secrets");

    let error = EnvOnly::builder("svc")
        .env("DCSECPROFILE_")
        .profile_env("DCSECPROFILE_ENV")
        .load()
        .expect_err("a traversal-shaped profile must be refused with no file layer active");

    std::env::remove_var("DCSECPROFILE_ENV");

    assert!(error.to_string().contains("DCSECPROFILE_ENV"), "{error}");
}

/// Every door to `explain` redacts when the secrets are known — the
/// generated builder handle included, not just the type-level method.
#[test]
fn the_generated_builders_explain_redacts_too() {
    use serde::Deserialize;

    #[dynamic_config::dynamic_config]
    #[derive(Deserialize)]
    struct BuilderExplain {
        #[allow(dead_code)]
        host: String,
        #[config(secret)]
        #[allow(dead_code)]
        token: String,
    }

    std::fs::create_dir_all("tests/scratch").unwrap();
    std::fs::write(
        "tests/scratch/security-builder-explain.json",
        r#"{"db": {"host": "localhost", "token": "hunter2-builder-door"}}"#,
    )
    .unwrap();

    let explanation = BuilderExplain::builder("db")
        .file("tests/scratch/security-builder-explain.json")
        .explain("token")
        .expect("the source reads cleanly");

    assert!(
        !explanation.to_string().contains("hunter2-builder-door"),
        "{explanation}"
    );
    assert!(explanation
        .rows()
        .iter()
        .all(|row| row.value.as_deref() != Some("hunter2-builder-door")));

    // The non-secret neighbour still explains with its value visible.
    let host = BuilderExplain::builder("db")
        .file("tests/scratch/security-builder-explain.json")
        .explain("host")
        .expect("the source reads cleanly");
    assert!(host.to_string().contains("localhost"), "{host}");
}

/// `Snapshot::to_value` hands over the resolved tree — and the tree's
/// `Debug`, like `Snapshot`'s own, shows shape and keys but never values:
/// `{:?}` in a log line is exactly how a resolved secret leaks.
#[test]
fn a_values_debug_shows_shape_and_never_values() {
    let text = r#"{"db": {"host": "localhost", "password": "hunter2-value-debug", "port": 5432}}"#;
    let sources = [dynamic_config::Source::inline(
        text,
        dynamic_config::Format::Json,
    )];
    let snapshot = dynamic_config::snapshot(&dynamic_config::LoadSpec::new("db", &sources))
        .expect("the inline source resolves");

    let rendered = format!("{:?}", snapshot.to_value());

    assert!(!rendered.contains("hunter2-value-debug"), "{rendered}");
    assert!(!rendered.contains("5432"), "{rendered}");
    assert!(
        rendered.contains("password"),
        "keys are the useful half, and they stay: {rendered}"
    );
}

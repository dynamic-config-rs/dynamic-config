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

/// A reload event and a status are exactly what somebody `{:?}`s into a
/// log, so neither may hold a configured value — not through the reason,
/// not through the failure record, and not through the snapshots the event
/// carries.
#[test]
fn a_reload_event_and_a_status_print_no_values() {
    use serde::Deserialize;
    use std::sync::{Arc, Mutex};

    #[dynamic_config::dynamic_config]
    #[derive(Deserialize)]
    struct Operated {
        #[allow(dead_code)]
        host: String,
        #[config(secret)]
        #[allow(dead_code)]
        password: String,
    }

    const PLANTED: &str = "hunter2-operations";

    std::fs::create_dir_all("tests/scratch").unwrap();
    std::fs::write(
        "tests/scratch/security-operations.json",
        format!(r#"{{"db": {{"host": "db.internal", "password": "{PLANTED}"}}}}"#),
    )
    .unwrap();

    let rendered = Arc::new(Mutex::new(Vec::new()));
    {
        let recorder = Arc::clone(&rendered);
        Operated::on_reload_with(move |event| {
            recorder.lock().unwrap().push(format!("{event:?}"));
        });
    }

    let builder = Operated::builder("db").file("tests/scratch/security-operations.json");
    builder.init().expect("the source reads cleanly");
    builder.reload().expect("and reloads");

    // And a failure, so `last_failure` is populated for the status below.
    std::fs::write(
        "tests/scratch/security-operations.json",
        format!(r#"{{"db": {{"host": {{"nested": "{PLANTED}"}}, "password": "x"}}}}"#),
    )
    .unwrap();
    builder
        .reload()
        .expect_err("`host` is a table, not a string");

    let mut printed = format!("{:?}", Operated::status());

    for event in rendered.lock().unwrap().iter() {
        printed.push(' ');
        printed.push_str(event);
    }

    assert!(
        !printed.contains(PLANTED),
        "a reload event and a status are log lines: {printed}"
    );
    assert!(
        !printed.contains("db.internal"),
        "no value, secret or not: {printed}"
    );
    assert!(
        printed.contains("host"),
        "the key path is the actionable half, and it stays: {printed}"
    );
    assert!(printed.contains("Manual"), "so is the reason: {printed}");
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

// ---------------------------------------------------------------------------
// A configuration with no struct: the same line, with nothing declaring it
// ---------------------------------------------------------------------------

/// Reading by path is *where* a password lands in a numeric field, and the
/// three schemaless read doors all deserialize a value already in hand — so
/// none of them met the loader's stripping. figment renders a mismatch as
/// ``invalid type: found string "hunter2", expected u16``; two of these used
/// to hand that back verbatim.
#[test]
fn a_schemaless_type_error_names_the_path_and_not_the_value() {
    let text = format!(r#"{{"db": {{"port": "{SECRET}"}}}}"#);
    let sources = [Source::inline(&text, Format::Json)];
    let spec = LoadSpec::new("db", &sources);

    let snapshot = dynamic_config::snapshot(&spec).expect("the document resolves");
    let values = snapshot.to_value();

    #[derive(Deserialize, Debug)]
    #[allow(dead_code)]
    struct Ported {
        port: u16,
    }

    let doors = [
        values.get_as::<u16>("port").unwrap_err(),
        snapshot.get::<u16>("port").unwrap_err(),
        snapshot.extract::<Ported>().unwrap_err(),
    ];

    for error in doors {
        let rendered = error.to_string();

        assert!(!rendered.contains(SECRET), "{rendered}");
        assert!(
            rendered.contains("port"),
            "the path is the actionable half: {rendered}"
        );
        assert!(
            rendered.contains("a string"),
            "and so is the kind of thing that was there: {rendered}"
        );
        assert_eq!(error.kind(), dynamic_config::ErrorKind::Type);
    }
}

/// A schemaless configuration has no `#[config(secret)]` to derive a list
/// from — so the list is supplied by hand, and `explain` redacts against it
/// exactly as it does for a declared one.
#[test]
fn a_hand_supplied_secret_list_redacts_a_schemaless_explanation() {
    const PLANTED: &str = "hunter2-schemaless-explain";
    const FIXTURE: &str = "tests/scratch/security-schemaless.json";

    std::fs::create_dir_all("tests/scratch").unwrap();
    std::fs::write(
        FIXTURE,
        format!(r#"{{"db": {{"host": "localhost", "credentials": {{"password": "{PLANTED}"}}}}}}"#),
    )
    .unwrap();

    let guarded = dynamic_config::Builder::values("db")
        .file(FIXTURE)
        .secrets(&["credentials"]);

    // The three-way rule, against a list nothing declared: the path *is* a
    // secret, and the path is *under* one.
    for path in ["credentials", "credentials.password"] {
        let explanation = guarded.explain(path).expect("the source reads cleanly");

        assert!(
            !explanation.to_string().contains(PLANTED),
            "{path}: {explanation}"
        );
        assert!(explanation
            .rows()
            .iter()
            .all(|row| row.value.as_deref() != Some(PLANTED)));
    }

    // The neighbour is not a secret and still explains with its value.
    assert!(guarded
        .explain("host")
        .expect("the source reads cleanly")
        .to_string()
        .contains("localhost"));

    // With no list, `explain` has nothing to redact against — it is the one
    // diagnostic that prints values, and the book says so. The tree itself
    // still never prints, declared or not: `Value`'s `Debug` is shape-only,
    // which is what keeps a `{:?}` of a schemaless snapshot from being the
    // leak a struct's generated `Debug` prevents.
    let values = dynamic_config::Builder::values("db")
        .file(FIXTURE)
        .load()
        .expect("the source reads cleanly");

    let printed = format!("{values:?}");

    assert!(!printed.contains(PLANTED), "{printed}");
    assert!(!printed.contains("localhost"), "{printed}");
    assert!(
        printed.contains("credentials"),
        "keys are the safe half: {printed}"
    );
}

/// A *syntax* error is the one diagnostic whose text this crate does not
/// write: the parser does, and `toml` writes it by quoting the line that
/// failed.
///
/// An unterminated string is exactly the typo somebody makes while pasting a
/// password into a config file, and the quoted line is then the password. The
/// position and the reason are what a person needs; both survive
/// `loader::origin::without_quoted_source`, and the document does not.
#[cfg(feature = "toml")]
#[test]
fn a_syntax_error_does_not_quote_the_line_it_failed_on() {
    let text = format!("[db]\nhost = \"localhost\"\npassword = \"{SECRET}\n");
    let sources = [dynamic_config::Source::inline(
        &text,
        dynamic_config::Format::Toml,
    )];

    let error = dynamic_config::load::<Secretive>(&dynamic_config::LoadSpec::new("db", &sources))
        .expect_err("the string is never closed");

    let rendered = error.to_string();

    assert!(!rendered.contains(SECRET), "{rendered}");
    assert!(
        rendered.contains("line 3"),
        "the position is the actionable half, and it stays: {rendered}"
    );
    assert!(
        rendered.contains("basic string"),
        "so is the reason, which `toml` prints below the quoted line: {rendered}"
    );
}

/// The same line, held on the parse seam a store crate reaches directly.
///
/// Two doors onto one road: `Value::parse` never touches a `LoadSpec` and has
/// no layer to name, so it would have been the easy place to render the
/// backend error raw.
#[cfg(feature = "toml")]
#[test]
fn the_parse_seam_strips_what_the_loader_strips() {
    let text = format!("[db]\npassword = \"{SECRET}\n");

    let error = dynamic_config::Value::parse(&text, dynamic_config::Format::Toml)
        .expect_err("the string is never closed");

    assert!(!error.to_string().contains(SECRET), "{error}");
}

/// The unit adapters parse a *value*, and a value that is not a duration is
/// exactly the shape of a password pasted into the wrong field. The message
/// says what was expected, never what was there — including the *unit* arm,
/// which used to quote back whatever followed the digits.
#[test]
fn a_unit_that_does_not_parse_echoes_no_value() {
    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    struct Timed {
        #[serde(with = "dynamic_config::duration")]
        timeout: std::time::Duration,
        #[serde(with = "dynamic_config::bytes")]
        max_body: u64,
    }

    for document in [
        format!(r#"{{"db": {{"timeout": "{SECRET}", "max_body": 1}}}}"#),
        format!(r#"{{"db": {{"timeout": "12{SECRET}", "max_body": 1}}}}"#),
        format!(r#"{{"db": {{"timeout": "1s", "max_body": "12{SECRET}"}}}}"#),
    ] {
        let sources = [Source::inline(&document, Format::Json)];

        let error = dynamic_config::load::<Timed>(&LoadSpec::new("db", &sources))
            .expect_err("the secret is not a duration");

        assert!(!error.to_string().contains(SECRET), "{error}");
    }
}

/// Writing refuses a configuration that is not a table by naming its *shape*
/// rather than its contents: a newtype over a token serializes to exactly one
/// string, and this is the path that would otherwise have written it to disk.
#[test]
fn a_section_that_is_not_a_table_is_refused_without_quoting_it() {
    let path = std::env::temp_dir().join("dynamic-config-not-a-table.json");

    let error = dynamic_config::save(&SECRET.to_owned(), &path, Format::Json, "db")
        .expect_err("a bare string is not a section");

    assert!(!error.to_string().contains(SECRET), "{error}");
    assert!(error.to_string().contains("table"), "{error}");
    assert!(!path.exists(), "and nothing was written");
}

/// An `Error` has no `source()`, and that is a decision rather than an
/// oversight.
///
/// Returning the backend failure would make `anyhow`/`eyre` chains and
/// `{:#}` render usefully — and would render the message this crate went to
/// the trouble of stripping: figment says ``invalid type: found string
/// "hunter2", expected u16``, and `loader::origin::message` reduces it to
/// the kind. A source is that stripped message's way back into every log
/// line, so there is none, and the chain a caller prints is the redacted
/// `Display` and nothing else.
#[test]
fn an_error_chain_has_nothing_behind_it_to_leak() {
    use std::error::Error as _;

    let text = format!(r#"{{"db": {{"pool": {{"max_size": "{SECRET}"}}}}}}"#);
    let sources = [Source::inline(&text, Format::Json)];

    let error = dynamic_config::load::<Secretive>(&LoadSpec::new("db", &sources))
        .expect_err("max_size is not a number");

    assert!(
        error.source().is_none(),
        "a source would carry the backend's unredacted message"
    );

    // What `anyhow` prints: the whole chain, walked the way it walks it.
    let mut chain = error.to_string();
    let mut link: Option<&(dyn std::error::Error + 'static)> = error.source();

    while let Some(cause) = link {
        write!(chain, ": {cause}").unwrap();
        link = cause.source();
    }

    assert!(!chain.contains(SECRET), "{chain}");
    assert!(chain.contains("max_size"), "{chain}");
}

// ---------------------------------------------------------------------------
// Telemetry: a metric label and a span field are diagnostics like any other,
// and the scrape endpoint is usually the least guarded surface a process has.
// ---------------------------------------------------------------------------

/// A metric label may not carry a configured value — and, unlike a log
/// line, may not carry a **key path** either: a path is a disclosure *and*
/// an unbounded label set, which is how one counter becomes a million
/// series.
#[test]
#[cfg(feature = "telemetry")]
fn no_metric_label_carries_a_value_a_path_or_a_file_name() {
    #[dynamic_config]
    #[derive(Deserialize)]
    struct Exported {
        #[allow(dead_code)]
        host: String,
        #[config(secret)]
        #[allow(dead_code)]
        password: String,
    }

    const PLANTED: &str = "hunter2-exported";
    const FIXTURE: &str = "tests/scratch/security-telemetry.json";

    std::fs::create_dir_all("tests/scratch").unwrap();
    std::fs::write(
        FIXTURE,
        format!(r#"{{"db": {{"host": "db.internal", "password": "{PLANTED}"}}}}"#),
    )
    .unwrap();

    let builder = Exported::builder("db").file(FIXTURE);
    builder.init().expect("the source reads cleanly");

    // A refusal too, so `last_failure` — the half that knows a key path — is
    // populated when the exposition is rendered.
    std::fs::write(
        FIXTURE,
        format!(r#"{{"db": {{"host": {{"nested": "{PLANTED}"}}, "password": "x"}}}}"#),
    )
    .unwrap();
    builder
        .reload()
        .expect_err("`host` is a table, not a string");

    let mut exposition = dynamic_config::telemetry::Exposition::new();
    exposition.add("db", &Exported::status());
    let rendered = exposition.render();

    assert!(
        !rendered.contains(PLANTED),
        "a value reached a label: {rendered}"
    );
    assert!(
        !rendered.contains("db.internal"),
        "no value, secret or not: {rendered}"
    );
    assert!(
        !rendered.contains("host"),
        "a key path is unbounded cardinality as well as a disclosure: {rendered}"
    );
    assert!(
        !rendered.contains("security-telemetry.json"),
        "and so is a file name: {rendered}"
    );
    assert!(
        rendered.contains(r#"kind="type""#),
        "the category is what a label may carry: {rendered}"
    );
}

#[cfg(feature = "tracing")]
mod capture;

/// A span field may name a key path — that is what makes a failed reload
/// actionable — and may never hold what was at it.
#[test]
#[cfg(feature = "tracing")]
fn no_span_or_event_field_carries_a_value() {
    #[dynamic_config]
    #[derive(Deserialize)]
    struct Traced {
        #[allow(dead_code)]
        host: String,
        #[config(secret)]
        #[allow(dead_code)]
        password: String,
    }

    const PLANTED: &str = "hunter2-traced";
    const FIXTURE: &str = "tests/scratch/security-telemetry-events.json";

    std::fs::create_dir_all("tests/scratch").unwrap();
    std::fs::write(
        FIXTURE,
        format!(r#"{{"db": {{"host": "db.internal", "password": "{PLANTED}"}}}}"#),
    )
    .unwrap();

    let captured = capture::global();

    let builder = Traced::builder("db").file(FIXTURE);
    builder.init().expect("the source reads cleanly");

    std::fs::write(
        FIXTURE,
        format!(r#"{{"db": {{"host": {{"nested": "{PLANTED}"}}, "password": "x"}}}}"#),
    )
    .unwrap();
    builder
        .reload()
        .expect_err("`host` is a table, not a string");

    // This test's own records, and not its neighbours': the subscriber is
    // the binary's, and every record names the configuration type it
    // belongs to.
    let lines = captured.about("no_span_or_event_field_carries_a_value::Traced");

    assert!(!lines.contains(PLANTED), "a value reached a field: {lines}");
    assert!(
        !lines.contains("db.internal"),
        "no value, secret or not: {lines}"
    );
    assert!(
        lines.contains(r#"error.kind="type""#),
        "the category is the useful half, and it stays: {lines}"
    );
    assert!(
        lines.contains(r#"error.path="host""#),
        "so is the key path: {lines}"
    );
}

// ---------------------------------------------------------------------------
// A remote store's URL is a credential
// ---------------------------------------------------------------------------

/// The one string a `Remote` can produce about itself is its source's
/// description, and a store URL routinely embeds `user:password@host`. The
/// fetch half of telemetry therefore names no store at all: the series is
/// labelled by whoever renders it.
#[cfg(feature = "telemetry")]
#[test]
fn a_store_url_never_becomes_a_metric_label() {
    use dynamic_config::{Error, Fetched, Remote, RemoteSource};

    const URL: &str = "https://vault:hunter2-in-a-url@store.internal:8200/v1/secret/db";
    const DOCUMENT: &str = r#"{"db": {"password": "hunter2-in-a-document"}}"#;

    struct Store(bool);

    impl RemoteSource for Store {
        fn fetch(&self) -> Result<Fetched, Error> {
            if self.0 {
                Ok(Fetched::new(DOCUMENT, Format::Json))
            } else {
                Err(Error::remote("the store is unreachable"))
            }
        }

        fn describe(&self) -> String {
            URL.to_owned()
        }
    }

    let remote = Remote::new();

    remote.set(Store(true));
    remote.refresh().expect("the store answers");
    remote.set(Store(false));
    remote.refresh().expect_err("the store is unreachable");

    let mut exposition = dynamic_config::telemetry::Exposition::new();
    exposition.add_remote("db", &remote.status());
    let rendered = exposition.render();

    assert!(
        !rendered.contains("hunter2-in-a-url"),
        "a credential reached a label: {rendered}"
    );
    assert!(
        !rendered.contains("store.internal"),
        "and neither does the host it was in: {rendered}"
    );
    assert!(
        !rendered.contains("hunter2-in-a-document"),
        "nor anything the store answered with: {rendered}"
    );
    assert!(!rendered.contains("password"), "nor a key path: {rendered}");
    assert!(
        rendered.contains(r#"dynamic_config_remote_up{config="db"} 0"#),
        "what survives is the outcome, under the caller's own name: {rendered}"
    );
    assert!(
        rendered.contains(r#"kind="remote""#),
        "and the category: {rendered}"
    );
}

/// The same rule one notch looser and still tight: a fetch span may carry an
/// outcome and an `ErrorKind`, and may not carry the URL it fetched from or
/// the document it fetched.
#[cfg(all(feature = "tracing", feature = "telemetry"))]
#[test]
fn a_store_url_never_becomes_a_span_or_event_field() {
    use dynamic_config::{Error, Fetched, Remote, RemoteSource};

    const URL: &str = "https://consul:hunter2-in-a-span@store.internal:8500/v1/kv/db";
    const DOCUMENT: &str = r#"{"db": {"password": "hunter2-in-a-fetched-document"}}"#;

    struct Store;

    impl RemoteSource for Store {
        fn fetch(&self) -> Result<Fetched, Error> {
            Ok(Fetched::new(DOCUMENT, Format::Json))
        }

        fn describe(&self) -> String {
            URL.to_owned()
        }
    }

    let captured = capture::global();

    let remote = Remote::new();
    remote.set(Store);
    remote.refresh().expect("the store answers");

    // Every record in the binary, not this test's own slice: a fetch record
    // carries no configuration type to filter by — deliberately, since the
    // only name a `Remote` has is the URL this test is asserting about.
    let lines = captured.joined();

    assert!(
        !lines.contains("hunter2-in-a-span"),
        "a credential reached a field: {lines}"
    );
    assert!(
        !lines.contains("store.internal"),
        "and neither does the host: {lines}"
    );
    assert!(
        !lines.contains("hunter2-in-a-fetched-document"),
        "nor the document: {lines}"
    );
    assert!(
        lines.contains(r#"outcome="fetched""#),
        "what survives is the outcome: {lines}"
    );
}

/// Found by the `lkg_serves_previous` fuzz target on its first corpus
/// run: a serde error reaching figment as plain text — a whole-document
/// extract failing on a wrong-typed root — rendered the offending value
/// in backticks. A password mistyped into a numeric field is exactly
/// that value.
#[cfg(feature = "json")]
#[test]
fn a_serde_type_error_does_not_quote_the_value() {
    let text = "\"hunter2-pasted-where-a-number-goes\"";
    let sources = [dynamic_config::Source::inline(
        text,
        dynamic_config::Format::Json,
    )];
    let spec = dynamic_config::LoadSpec::new("doc", &sources).with_whole_document(true);

    let error = dynamic_config::load::<u32>(&spec).expect_err("a string is not a u32");
    let rendered = format!("{error}");

    assert!(
        !rendered.contains("hunter2"),
        "the refused value reached a diagnostic: {rendered}"
    );
    assert!(
        rendered.contains("<redacted>") || rendered.contains("invalid type"),
        "the shape of the failure must survive the scrub: {rendered}"
    );
}

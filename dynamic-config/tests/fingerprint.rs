//! The digest a fleet compares instead of comparing documents.
//!
//! The unit tests beside the implementation pin the encoding; these pin the
//! two properties a caller actually relies on — that the same configuration
//! fingerprints the same however it was *written*, and that the string is
//! safe to print.

#![cfg(all(feature = "json", feature = "toml"))]

use dynamic_config::{dynamic_config, Dynamic};
use serde::Deserialize;

#[dynamic_config]
#[derive(Deserialize)]
struct Service {
    // Read through the fingerprint rather than by name: these tests are
    // about the digest of the resolved tree, not about the struct that
    // receives it.
    #[allow(dead_code)]
    host: String,
    #[allow(dead_code)]
    port: u16,
    #[config(secret)]
    #[allow(dead_code)]
    password: String,
}

fn written(directory: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
    let path = directory.join(name);
    std::fs::write(&path, body).expect("the fixture writes");

    path
}

fn fingerprint_of(file: &std::path::Path) -> String {
    let config =
        Dynamic::<Service>::new(Service::builder("svc").file(file.to_string_lossy().into_owned()));

    config.init().expect("the document is complete");

    config
        .fingerprint()
        .map(|fingerprint| (*fingerprint).clone())
        .expect("an installed configuration has a fingerprint")
}

/// The same values, written two ways. A fingerprint that disagreed here
/// would report drift between two hosts that are configured identically and
/// merely chose different file formats.
#[test]
fn the_same_configuration_fingerprints_the_same_in_any_format() {
    let directory = tempfile::tempdir().expect("a temporary directory");

    let json = written(
        directory.path(),
        "config.json",
        r#"{"svc": {"host": "db.internal", "port": 5432, "password": "hunter2"}}"#,
    );

    // Keys in a different order, a different syntax, the same configuration.
    let toml = written(
        directory.path(),
        "config.toml",
        "[svc]\nport = 5432\npassword = \"hunter2\"\nhost = \"db.internal\"\n",
    );

    assert_eq!(fingerprint_of(&json), fingerprint_of(&toml));
}

/// The property that makes it printable: rotating a secret does not move it.
///
/// A digest that changed when a password changed would let anyone holding a
/// candidate password confirm it against a logged fingerprint.
#[test]
fn rotating_a_secret_does_not_move_the_fingerprint() {
    let directory = tempfile::tempdir().expect("a temporary directory");

    let before = written(
        directory.path(),
        "before.json",
        r#"{"svc": {"host": "db.internal", "port": 5432, "password": "hunter2"}}"#,
    );
    let after = written(
        directory.path(),
        "after.json",
        r#"{"svc": {"host": "db.internal", "port": 5432, "password": "correct-horse"}}"#,
    );

    assert_eq!(fingerprint_of(&before), fingerprint_of(&after));
}

/// And the property that makes it useful: everything else does move it.
#[test]
fn changing_anything_else_moves_the_fingerprint() {
    let directory = tempfile::tempdir().expect("a temporary directory");

    let before = written(
        directory.path(),
        "before.json",
        r#"{"svc": {"host": "db.internal", "port": 5432, "password": "hunter2"}}"#,
    );
    let after = written(
        directory.path(),
        "after.json",
        r#"{"svc": {"host": "db.internal", "port": 6432, "password": "hunter2"}}"#,
    );

    assert_ne!(fingerprint_of(&before), fingerprint_of(&after));
}

/// It is rendered to be read, and it names its algorithm.
#[test]
fn it_is_prefixed_and_hexadecimal() {
    let directory = tempfile::tempdir().expect("a temporary directory");

    let file = written(
        directory.path(),
        "config.json",
        r#"{"svc": {"host": "db.internal", "port": 5432, "password": "hunter2"}}"#,
    );

    let fingerprint = fingerprint_of(&file);
    let digest = fingerprint
        .strip_prefix("sha256:")
        .expect("a fingerprint names its algorithm");

    assert_eq!(digest.len(), 64);
    assert!(digest
        .chars()
        .all(|character| character.is_ascii_hexdigit()));

    assert!(
        !fingerprint.contains("hunter2"),
        "the one thing a printable digest must never contain"
    );
}

/// Nothing is installed, so there is nothing to describe. `None` rather
/// than the digest of an empty document, which would compare equal across
/// every process that had not started yet.
#[test]
fn there_is_no_fingerprint_before_the_first_install() {
    let directory = tempfile::tempdir().expect("a temporary directory");

    let file = written(
        directory.path(),
        "config.json",
        r#"{"svc": {"host": "db.internal", "port": 5432, "password": "hunter2"}}"#,
    );

    let config =
        Dynamic::<Service>::new(Service::builder("svc").file(file.to_string_lossy().into_owned()));

    assert!(config.fingerprint().is_none());

    config.init().expect("the document is complete");

    assert!(config.fingerprint().is_some());
}

/// A reload that installs a different document installs a different
/// fingerprint with it — the value is a property of what is serving, not of
/// the process.
#[test]
fn a_reload_moves_it() {
    let directory = tempfile::tempdir().expect("a temporary directory");

    let file = written(
        directory.path(),
        "config.json",
        r#"{"svc": {"host": "db.internal", "port": 5432, "password": "hunter2"}}"#,
    );

    let config =
        Dynamic::<Service>::new(Service::builder("svc").file(file.to_string_lossy().into_owned()));
    config.init().expect("the document is complete");

    let first = config.fingerprint().expect("installed");

    std::fs::write(
        &file,
        r#"{"svc": {"host": "db.internal", "port": 6432, "password": "hunter2"}}"#,
    )
    .expect("the fixture rewrites");

    config.reload().expect("the new document is complete");

    let second = config.fingerprint().expect("installed");

    assert_ne!(*first, *second);
}

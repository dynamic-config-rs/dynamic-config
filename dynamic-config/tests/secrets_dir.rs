//! A directory of single-value files, the way Docker and Kubernetes mount
//! secrets.
//!
//! One type, one directory and one environment variable per test: the layer
//! reads whatever is on disk at load time, so two tests sharing a directory
//! would race and pass alone.

#![cfg(feature = "json")]

use std::path::{Path, PathBuf};

use dynamic_config::{dynamic_config, CacheMode, Origin};
// Only the `cfg(unix)` tests classify an error: the two that do are about
// permissions and symlinks. Unconditionally imported, this is an unused
// import on Windows, which `-D warnings` refuses.
#[cfg(unix)]
use dynamic_config::ErrorKind;
use serde::Deserialize;

/// This test's own mount, emptied first so a previous run cannot supply a key
/// this one never wrote.
fn mount(test: &str) -> PathBuf {
    let directory = std::env::temp_dir()
        .join("dynamic-config-secrets-dir")
        .join(test);

    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("the scratch directory is writable");

    directory
}

fn put(directory: &Path, name: &str, contents: &str) {
    std::fs::write(directory.join(name), contents).expect("the scratch file is writable");
}

// ---------------------------------------------------------------------------
// The shape itself
// ---------------------------------------------------------------------------

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Basic {
    host: String,
    password: String,
    pool: Pool,
}

#[derive(Debug, Deserialize)]
struct Pool {
    max_size: String,
}

#[test]
fn one_file_per_key_with_the_filename_as_the_key() {
    let directory = mount("basic");
    put(&directory, "host", "mounted-host\n");
    put(&directory, "password", "hunter2\n");
    put(&directory, "pool__max_size", "64\n");

    let config = Basic::builder("db")
        .secrets_dir(directory.to_str().expect("a UTF-8 scratch path"))
        .load()
        .expect("the directory reads cleanly");

    assert_eq!(config.host, "mounted-host");
    assert_eq!(config.password, "hunter2");
    assert_eq!(
        config.pool.max_size, "64",
        "the nesting separator applies to the filename, as it does to a \
         variable name"
    );
}

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Newlines {
    once: String,
    twice: String,
    never: String,
    windows: String,
}

#[test]
fn exactly_one_trailing_newline_is_removed() {
    let directory = mount("newlines");
    put(&directory, "once", "value\n");
    put(&directory, "twice", "value\n\n");
    put(&directory, "never", "value");
    put(&directory, "windows", "value\r\n");

    let config = Newlines::builder("db")
        .secrets_dir(directory.to_str().expect("a UTF-8 scratch path"))
        .load()
        .expect("the directory reads cleanly");

    assert_eq!(config.once, "value");
    assert_eq!(
        config.twice, "value\n",
        "one newline is the tool that wrote the file; the second is content"
    );
    assert_eq!(config.never, "value");
    assert_eq!(config.windows, "value", "a CRLF is one newline, not two");
}

// ---------------------------------------------------------------------------
// Precedence
// ---------------------------------------------------------------------------

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct OverFile {
    host: String,
    port: u16,
}

#[test]
fn the_directory_wins_over_a_file() {
    let directory = mount("over-file");
    put(&directory, "host", "from-the-mount\n");

    let config = OverFile::builder("db")
        .file("tests/fixtures/base.json")
        .secrets_dir(directory.to_str().expect("a UTF-8 scratch path"))
        .load()
        .expect("the sources read cleanly");

    assert_eq!(config.host, "from-the-mount");
    assert_eq!(
        config.port, 5432,
        "a key the mount does not supply still comes from the file"
    );
}

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct UnderEnv {
    host: String,
    #[allow(dead_code)]
    port: u16,
}

#[test]
fn the_environment_wins_over_the_directory() {
    let directory = mount("under-env");
    put(&directory, "host", "from-the-mount\n");
    std::env::set_var("DCSECRETS_DB_HOST", "from-the-environment");

    let config = UnderEnv::builder("db")
        .file("tests/fixtures/base.json")
        .secrets_dir(directory.to_str().expect("a UTF-8 scratch path"))
        .env("DCSECRETS_")
        .load()
        .expect("the sources read cleanly");

    assert_eq!(
        config.host, "from-the-environment",
        "a variable exported for this run is more specific than a mount"
    );
}

// ---------------------------------------------------------------------------
// Provenance
// ---------------------------------------------------------------------------

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Traced {
    #[allow(dead_code)]
    host: String,
}

#[test]
fn the_origin_names_the_individual_file() {
    let directory = mount("traced");
    put(&directory, "host", "mounted-host\n");

    Traced::builder("db")
        .secrets_dir(directory.to_str().expect("a UTF-8 scratch path"))
        .init()
        .expect("the directory reads cleanly");

    let origin = Traced::source_of("host")
        .expect("the sources read cleanly")
        .expect("something supplies it");

    assert_eq!(
        origin,
        Origin::File(directory.join("host")),
        "not the directory — the file, which is the answer to the question"
    );
}

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Explained {
    #[allow(dead_code)]
    host: String,
}

#[test]
fn explain_lists_the_secrets_layer_between_the_files_and_the_environment() {
    let directory = mount("explained");
    put(&directory, "host", "mounted-host\n");

    Explained::builder("db")
        .file("tests/fixtures/base.json")
        .secrets_dir(directory.to_str().expect("a UTF-8 scratch path"))
        .env("DCSECRETSX_")
        .init()
        .expect("the sources read cleanly");

    let explanation = Explained::explain("host").expect("the sources read cleanly");
    let layers: Vec<&str> = explanation.rows().iter().map(|row| row.layer).collect();

    assert_eq!(
        layers,
        vec!["file", "secrets", "environment"],
        "lowest precedence first"
    );
    assert_eq!(
        explanation.winner().and_then(|row| row.value.as_deref()),
        Some("mounted-host"),
        "with nothing in the environment, the mount has the last word"
    );
}

// ---------------------------------------------------------------------------
// Missing, unreadable, and the Kubernetes shape
// ---------------------------------------------------------------------------

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Absent {
    host: String,
    #[allow(dead_code)]
    port: u16,
}

#[test]
fn a_missing_directory_is_skipped_like_a_missing_file() {
    let config = Absent::builder("db")
        .file("tests/fixtures/base.json")
        .secrets_dir("/no/such/mount/for/this/test")
        .load()
        .expect("a container that mounts nothing still starts");

    assert_eq!(config.host, "localhost");
}

// Only the `cfg(unix)` test below constructs this: permissions and symlinks
// are what it is about, and on Windows the type would be dead code that
// `-D warnings` refuses.
#[cfg(unix)]
#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Unreadable {
    #[allow(dead_code)]
    host: String,
}

#[cfg(unix)]
#[test]
fn an_unreadable_directory_is_an_io_error_naming_the_path_and_no_contents() {
    use std::os::unix::fs::PermissionsExt as _;

    const SECRET: &str = "hunter2-do-not-print-me";

    let directory = mount("unreadable");
    put(&directory, "host", SECRET);
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o000))
        .expect("the scratch directory's mode is ours to set");

    let outcome = Unreadable::builder("db")
        .secrets_dir(directory.to_str().expect("a UTF-8 scratch path"))
        .load();

    // Restored before the assertions, so a failure still leaves a directory
    // the next run can delete.
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
        .expect("the scratch directory's mode is ours to set");

    let error = outcome.expect_err("a mount that refuses to be read is a bug, not a silence");

    assert_eq!(error.kind(), ErrorKind::Io);

    let rendered = error.to_string();

    assert!(
        rendered.contains(directory.to_str().expect("a UTF-8 scratch path")),
        "the error names the path it could not read: {rendered}"
    );
    assert!(
        !rendered.contains(SECRET),
        "and never what was inside it: {rendered}"
    );
}

// Only the `cfg(unix)` test below constructs this: permissions and symlinks
// are what it is about, and on Windows the type would be dead code that
// `-D warnings` refuses.
#[cfg(unix)]
#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Mounted {
    host: String,
    password: String,
}

/// The shape a real Kubernetes secret volume has: a `..data` symlink to a
/// timestamped directory, and every key a symlink into it. Following the key
/// links is required; descending into the directories must not happen, or
/// `..data` and its target would each contribute the whole set again.
#[cfg(unix)]
#[test]
fn a_kubernetes_style_mount_resolves_through_its_symlinks() {
    use std::os::unix::fs::symlink;

    let directory = mount("kubernetes");
    let versioned = directory.join("..2026_08_12_00_00_00.1234567890");

    std::fs::create_dir_all(&versioned).expect("the scratch directory is writable");
    put(&versioned, "host", "mounted-host\n");
    put(&versioned, "password", "hunter2\n");

    symlink(&versioned, directory.join("..data")).expect("symlinks are ours to make");
    symlink(Path::new("..data/host"), directory.join("host")).expect("symlinks are ours to make");
    symlink(Path::new("..data/password"), directory.join("password"))
        .expect("symlinks are ours to make");

    let config = Mounted::builder("db")
        .secrets_dir(directory.to_str().expect("a UTF-8 scratch path"))
        .load()
        .expect("the mount reads cleanly");

    assert_eq!(config.host, "mounted-host");
    assert_eq!(config.password, "hunter2");
}

// ---------------------------------------------------------------------------
// A mounted secret is a secret
// ---------------------------------------------------------------------------

#[dynamic_config]
#[derive(Deserialize)]
struct Planted {
    host: String,
    #[config(secret)]
    #[allow(dead_code)]
    password: String,
}

const PLANTED: &str = "hunter2-do-not-print-me";

#[test]
fn a_mounted_secret_is_redacted_in_explain_and_left_out_of_a_redacted_cache() {
    let directory = mount("planted");
    put(&directory, "host", "mounted-host\n");
    put(&directory, "password", PLANTED);

    let cache = directory.join("last-good.json");

    Planted::builder("db")
        .secrets_dir(directory.to_str().expect("a UTF-8 scratch path"))
        .cache(
            cache.to_str().expect("a UTF-8 scratch path"),
            CacheMode::Redacted,
        )
        .init()
        .expect("the directory reads cleanly");

    let explanation = Planted::explain("password").expect("the sources read cleanly");
    let rendered = explanation.to_string();

    assert!(
        !rendered.contains(PLANTED),
        "redaction follows the field's path, whichever layer supplied it: \
         {rendered}"
    );
    assert!(rendered.contains("***"), "{rendered}");
    assert!(
        rendered.contains(&directory.join("password").display().to_string()),
        "the origin stays — where a secret comes from is the useful half: \
         {rendered}"
    );

    let written = std::fs::read_to_string(&cache).expect("the cache was written");

    assert!(
        !written.contains(PLANTED),
        "a redacted cache leaves a mounted secret off disk too: {written}"
    );
    assert!(
        written.contains("mounted-host"),
        "while everything unmarked is still recoverable: {written}"
    );
}

// ---------------------------------------------------------------------------
// What it deliberately does not do
// ---------------------------------------------------------------------------

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Shallow {
    host: String,
    #[allow(dead_code)]
    port: u16,
}

#[test]
fn a_subdirectory_is_not_descended_into() {
    let directory = mount("shallow");
    let nested = directory.join("pool");

    std::fs::create_dir_all(&nested).expect("the scratch directory is writable");
    put(&nested, "max_size", "64\n");
    put(&directory, "host", "mounted-host\n");

    let config = Shallow::builder("db")
        .file("tests/fixtures/base.json")
        .secrets_dir(directory.to_str().expect("a UTF-8 scratch path"))
        .load()
        .expect("a subdirectory is skipped, not an error");

    assert_eq!(config.host, "mounted-host");
}

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Textual {
    password: String,
}

#[test]
fn a_value_is_always_a_string_so_a_numeric_password_stays_one() {
    let directory = mount("textual");
    put(&directory, "password", "12345\n");

    let config = Textual::builder("db")
        .secrets_dir(directory.to_str().expect("a UTF-8 scratch path"))
        .load()
        .expect("the directory reads cleanly");

    assert_eq!(
        config.password, "12345",
        "the environment layer would have parsed this into an integer and \
         failed the field; a mounted credential is text and stays text"
    );
}

// ---------------------------------------------------------------------------
// Containment: a symlink may not leave the mount (0.7.1)
// ---------------------------------------------------------------------------
//
// The vulnerability shape Pydantic Settings shipped a CVE for in June 2026:
// a planted link inside the secrets directory that resolves to a file
// outside it. Since 0.7.1 the escape is refused with the entry's name —
// and the error must never carry what the target held.

#[cfg(unix)]
#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Contained {
    host: String,
    #[allow(dead_code)]
    password: String,
}

/// Somewhere outside every mount, holding bytes that must never load.
#[cfg(unix)]
fn outside_file(test: &str) -> PathBuf {
    let path = std::env::temp_dir()
        .join("dynamic-config-secrets-outside")
        .join(test);

    std::fs::create_dir_all(path.parent().expect("a parent")).expect("writable scratch");
    std::fs::write(&path, "stolen-bytes\n").expect("writable scratch");

    path
}

#[cfg(unix)]
#[test]
fn a_symlink_escaping_the_mount_is_refused_by_name() {
    use std::os::unix::fs::symlink;

    let directory = mount("escape-absolute");
    let target = outside_file("escape-absolute");

    put(&directory, "host", "fine\n");
    symlink(&target, directory.join("password")).expect("symlinks are ours to make");

    let error = Contained::builder("db")
        .secrets_dir(directory.to_str().expect("a UTF-8 scratch path"))
        .load()
        .expect_err("the escape is refused");

    let rendered = format!("{error}");
    assert!(
        rendered.contains("resolves outside"),
        "the refusal names the shape: {rendered}"
    );
    assert!(
        rendered.contains("password"),
        "the refusal names the entry: {rendered}"
    );
    assert!(
        !rendered.contains("stolen-bytes"),
        "and never what the target held: {rendered}"
    );
}

#[cfg(unix)]
#[test]
fn a_dot_dot_relative_escape_is_refused_too() {
    use std::os::unix::fs::symlink;

    let directory = mount("escape-relative");
    let target = outside_file("escape-relative");

    // `../../dynamic-config-secrets-outside/escape-relative`, spelled
    // relative so canonicalization has `..` segments to resolve.
    let relative = Path::new("..").join("..").join(
        target
            .strip_prefix(std::env::temp_dir())
            .expect("under the temp dir"),
    );

    put(&directory, "host", "fine\n");
    symlink(&relative, directory.join("password")).expect("symlinks are ours to make");

    let error = Contained::builder("db")
        .secrets_dir(directory.to_str().expect("a UTF-8 scratch path"))
        .load()
        .expect_err("the relative escape is refused");

    assert!(format!("{error}").contains("resolves outside"), "{error}");
}

#[cfg(unix)]
#[test]
fn a_chain_whose_final_target_escapes_is_refused() {
    use std::os::unix::fs::symlink;

    let directory = mount("escape-chain");
    let target = outside_file("escape-chain");

    // key -> inside-link -> outside: the first hop is inside the mount,
    // and only full resolution sees the escape.
    symlink(&target, directory.join("inner")).expect("symlinks are ours to make");
    symlink(Path::new("inner"), directory.join("password")).expect("symlinks are ours to make");
    put(&directory, "host", "fine\n");

    let error = Contained::builder("db")
        .secrets_dir(directory.to_str().expect("a UTF-8 scratch path"))
        .load()
        .expect_err("the chained escape is refused");

    assert!(format!("{error}").contains("resolves outside"), "{error}");
}

#[cfg(unix)]
#[test]
fn the_opt_out_restores_the_old_behaviour_deliberately() {
    use std::os::unix::fs::symlink;

    let directory = mount("escape-opted-out");
    let target = outside_file("escape-opted-out");

    put(&directory, "host", "fine\n");
    symlink(&target, directory.join("password")).expect("symlinks are ours to make");

    let config = Contained::builder("db")
        .secrets_dir(directory.to_str().expect("a UTF-8 scratch path"))
        .allow_external_symlinks(true)
        .load()
        .expect("the opt-out follows the link");

    assert_eq!(config.host, "fine");
}

//! Starting from the last configuration that worked.

#![cfg(feature = "json")]

use std::fs;
use std::path::PathBuf;

use dynamic_config::dynamic_config;
use serde::Deserialize;

/// A directory per test: these write real files and run in parallel.
fn scratch(test: &str) -> PathBuf {
    let directory = std::env::temp_dir()
        .join("dynamic-config-cache-tests")
        .join(test);

    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).expect("the scratch directory should be creatable");

    directory
}

// The three modes each point at a *fixed* path, because the macro takes a
// literal. Each test owns its own.
#[dynamic_config(
    files = ["tests/scratch/cache-full.json"],
    key = "db",
    cache = "tests/scratch/last-good-full.json",
)]
#[derive(Debug, Deserialize)]
struct Full {
    host: String,
    port: u16,
}

#[dynamic_config(
    files = ["tests/scratch/cache-redacted.json"],
    key = "db",
    env = "DCCACHE_",
    cache = "tests/scratch/last-good-redacted.json",
    cache_mode = "redacted",
)]
#[derive(Deserialize)]
struct Redacted {
    host: String,
    #[config(secret)]
    password: String,
}

/// Its own type and its own files: this one deliberately starts from a broken
/// file, and sharing `Full`'s would race the test that starts from a good one.
#[dynamic_config(
    files = ["tests/scratch/cache-first-start.json"],
    key = "db",
    cache = "tests/scratch/last-good-first-start.json",
)]
#[derive(Debug, Deserialize)]
struct FirstStart {
    #[allow(dead_code)]
    host: String,
}

#[dynamic_config(
    files = ["tests/scratch/cache-fingerprint.json"],
    key = "db",
    cache = "tests/scratch/last-good-fingerprint.json",
    cache_mode = "fingerprint",
)]
#[derive(Debug, Deserialize)]
struct Fingerprint {
    #[allow(dead_code)]
    host: String,
}

fn prepare(name: &str, contents: &str) {
    fs::create_dir_all("tests/scratch").unwrap();
    let _ = fs::remove_file(format!("tests/scratch/last-good-{name}.json"));
    fs::write(format!("tests/scratch/cache-{name}.json"), contents).unwrap();
}

fn break_file(name: &str) {
    fs::write(format!("tests/scratch/cache-{name}.json"), "{ not json").unwrap();
}

#[test]
fn a_full_cache_carries_a_cold_start_over_a_broken_file() {
    prepare("full", r#"{"db": {"host": "localhost", "port": 5432}}"#);

    Full::init().expect("the file is fine to begin with");
    assert_eq!(Full::current().port, 5432);

    // Somebody saves a broken file, and the machine reboots.
    break_file("full");

    // A fresh load fails, as it should...
    assert!(Full::load().is_err());

    // ...but a start recovers, loudly, rather than refusing to come up.
    Full::init().expect("the cache stands in for the broken file");
    assert_eq!(Full::current().host, "localhost");
    assert_eq!(Full::current().port, 5432);
}

#[test]
fn a_redacted_cache_keeps_the_secret_off_disk_and_takes_it_from_the_environment() {
    prepare(
        "redacted",
        r#"{"db": {"host": "localhost", "password": "hunter2"}}"#,
    );

    Redacted::init().expect("the file is complete");

    let written = fs::read_to_string("tests/scratch/last-good-redacted.json").unwrap();
    assert!(written.contains("localhost"), "{written}");
    assert!(
        !written.contains("hunter2"),
        "the secret must not be on disk: {written}"
    );

    break_file("redacted");

    // Without the secret from somewhere live, recovery cannot complete.
    assert!(
        Redacted::init().is_err(),
        "a redacted cache alone is not a whole configuration"
    );

    // With it, the cache supplies the rest.
    std::env::set_var("DCCACHE_DB_PASSWORD", "from-the-environment");

    Redacted::init().expect("the environment closes the gap the cache left");
    assert_eq!(Redacted::current().host, "localhost");
    assert_eq!(Redacted::current().password, "from-the-environment");

    std::env::remove_var("DCCACHE_DB_PASSWORD");
}

#[test]
fn a_fingerprint_cache_refuses_to_recover_and_says_what_moved() {
    prepare("fingerprint", r#"{"db": {"host": "localhost"}}"#);

    Fingerprint::init().expect("the file is complete");

    let written = fs::read_to_string("tests/scratch/last-good-fingerprint.json").unwrap();
    assert!(
        !written.contains("localhost"),
        "a fingerprint holds no value at all: {written}"
    );

    // A configuration that parses but no longer matches the struct.
    fs::write(
        "tests/scratch/cache-fingerprint.json",
        r#"{"db": {"hsot": "localhost"}}"#,
    )
    .unwrap();

    assert!(
        Fingerprint::init().is_err(),
        "a fingerprint cache does not pretend it can recover"
    );
}

#[test]
fn a_first_start_with_no_cache_still_fails_on_a_broken_file() {
    let directory = scratch("first-start");

    // Nothing has ever been written here, which is the state of every first
    // start. Recovery must not turn that into a false success.
    assert!(!directory.join("last-good.json").exists());

    prepare("first-start", "{ not json");

    assert!(FirstStart::load().is_err(), "the file does not parse");
    assert!(
        FirstStart::init().is_err(),
        "and there is no cache to fall back on"
    );
}

/// Recovery must keep `build()`'s precedence: the real environment above the
/// `.env` file. The inversion this pins had the repository's `.env` beating
/// the variable a human exported to steer the recovery.
#[cfg(feature = "dotenv")]
#[test]
fn recovery_keeps_the_environment_above_env_files() {
    use serde::Deserialize;

    #[dynamic_config::dynamic_config(
        files = ["tests/scratch/cache-envorder.json"],
        key = "db",
        env = "DCCACHEENV_",
        env_files = ["tests/scratch/cache-envorder.env"],
        cache = "tests/scratch/cache-envorder-cache.json",
    )]
    #[derive(Debug, Deserialize)]
    struct Ordered {
        host: String,
    }

    std::fs::create_dir_all("tests/scratch").unwrap();
    std::fs::write(
        "tests/scratch/cache-envorder.json",
        r#"{"db": {"host": "from-the-file"}}"#,
    )
    .unwrap();
    std::fs::write(
        "tests/scratch/cache-envorder.env",
        "DCCACHEENV_DB_HOST=from-dotenv\n",
    )
    .unwrap();

    // A clean start writes the cache.
    Ordered::init().expect("the first load succeeds");

    // The file breaks; the exported variable is the human's override.
    std::fs::write("tests/scratch/cache-envorder.json", "{ not json").unwrap();
    std::env::set_var("DCCACHEENV_DB_HOST", "from-the-real-environment");

    // `init()` walks the recovery path when the sources fail.
    let initialized = Ordered::init();

    std::env::remove_var("DCCACHEENV_DB_HOST");
    initialized.expect("the cache recovers");

    // The real environment wins over the .env file during recovery, exactly
    // as it does during a normal load.
    assert_eq!(Ordered::current().host, "from-the-real-environment");
}

/// Recovery goes through `validate` like every other path: a cache holding a
/// configuration the type would reject must not be installed just because
/// the sources are broken — stale AND invalid is the worst of both.
#[test]
fn recovery_respects_validate() {
    use serde::Deserialize;

    #[dynamic_config::dynamic_config(
        files = ["tests/scratch/cache-validated.json"],
        key = "db",
        cache = "tests/scratch/cache-validated-cache.json",
        validate,
    )]
    #[derive(Debug, Deserialize)]
    struct Validated {
        connections: u16,
    }

    impl Validated {
        fn validate(&self) -> Result<(), String> {
            if self.connections == 0 {
                return Err("connections of zero would serve nobody".to_owned());
            }

            Ok(())
        }
    }

    std::fs::create_dir_all("tests/scratch").unwrap();

    // A configuration that passes validation, cached on a clean start.
    std::fs::write(
        "tests/scratch/cache-validated.json",
        r#"{"db": {"connections": 4}}"#,
    )
    .unwrap();
    Validated::init().expect("the clean start succeeds");

    // The cache is then tampered into something the type rejects, and the
    // source breaks — the recovery must NOT install the invalid cache.
    std::fs::write(
        "tests/scratch/cache-validated-cache.json",
        r#"{"cached": {"connections": 0}}"#,
    )
    .unwrap();
    std::fs::write("tests/scratch/cache-validated.json", "{ not json").unwrap();

    let error = Validated::init().expect_err("an invalid cache must not be installed");

    assert!(
        error.to_string().contains("serve nobody"),
        "the validation error names the rule: {error}"
    );
}

/// A renamed secret is redacted under its *serde* name — redacting by the
/// Rust ident used to write a "redacted" cache containing the secret in the
/// clear under its renamed key.
#[test]
fn a_renamed_secret_stays_out_of_the_redacted_cache() {
    use serde::Deserialize;

    #[dynamic_config::dynamic_config(
        files = ["tests/scratch/cache-renamed.json"],
        key = "db",
        cache = "tests/scratch/cache-renamed-cache.json",
        cache_mode = "redacted",
    )]
    #[derive(Deserialize)]
    struct Renamed {
        #[allow(dead_code)]
        host: String,
        #[serde(rename = "pass")]
        #[config(secret)]
        #[allow(dead_code)]
        password: String,
    }

    std::fs::create_dir_all("tests/scratch").unwrap();
    std::fs::write(
        "tests/scratch/cache-renamed.json",
        r#"{"db": {"host": "localhost", "pass": "hunter2-renamed"}}"#,
    )
    .unwrap();

    Renamed::init().expect("the load succeeds");

    let written = std::fs::read_to_string("tests/scratch/cache-renamed-cache.json")
        .expect("the cache was written");

    assert!(
        !written.contains("hunter2-renamed"),
        "the redacted cache must not contain the renamed secret: {written}"
    );
    assert!(written.contains("localhost"), "{written}");
}

/// `rename_all` on the container is honoured too.
#[test]
fn a_rename_all_secret_stays_out_of_the_redacted_cache() {
    use serde::Deserialize;

    #[dynamic_config::dynamic_config(
        files = ["tests/scratch/cache-renameall.json"],
        key = "db",
        cache = "tests/scratch/cache-renameall-cache.json",
        cache_mode = "redacted",
    )]
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct CamelCased {
        #[allow(dead_code)]
        host_name: String,
        #[config(secret)]
        #[allow(dead_code)]
        api_token: String,
    }

    std::fs::create_dir_all("tests/scratch").unwrap();
    std::fs::write(
        "tests/scratch/cache-renameall.json",
        r#"{"db": {"hostName": "localhost", "apiToken": "hunter2-camel"}}"#,
    )
    .unwrap();

    CamelCased::init().expect("the load succeeds");

    let written = std::fs::read_to_string("tests/scratch/cache-renameall-cache.json")
        .expect("the cache was written");

    assert!(
        !written.contains("hunter2-camel"),
        "the redacted cache must not contain the camelCased secret: {written}"
    );
}

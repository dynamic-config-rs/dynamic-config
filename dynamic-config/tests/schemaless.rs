//! A configuration with no struct behind it.
//!
//! The engine's only bound on a configuration type is `DeserializeOwned`,
//! and [`Value`] satisfies it — so `Dynamic<Value>` is the same engine with
//! the declaration removed. What these tests pin is that *removing the
//! declaration removes nothing else*: the layers still layer, the watcher
//! still watches, the cache still recovers, and a reload is still a whole
//! new tree rather than a mutated one.
//!
//! What it *does* remove is the subject of the last group: unknown-key
//! detection has nothing to compare against, and it has to say so rather
//! than report an empty list that reads like an all-clear.
//!
//! One type, one fixture, one variable per test — the snapshots and watcher
//! identities behind these are per-instance, but the *files* are not.

#![cfg(feature = "json")]

use std::fs;
use std::sync::Arc;

use dynamic_config::{Builder, CacheMode, Dynamic, ErrorKind, Value};

fn write(path: &str, document: &str) -> String {
    fs::create_dir_all("tests/scratch").unwrap();
    fs::write(path, document).unwrap();

    path.to_owned()
}

// ---------------------------------------------------------------------------
// Reading by path
// ---------------------------------------------------------------------------

/// Every shape a source can express, read back through the accessors, with
/// no type named anywhere.
#[test]
fn a_configuration_loads_with_no_type_and_reads_every_shape_by_path() {
    let file = write(
        "tests/scratch/schemaless-shapes.json",
        r#"{"db": {
            "host": "localhost",
            "port": 5432,
            "ratio": 0.25,
            "tls": true,
            "nothing": null,
            "tags": ["a", "b"],
            "pool": {"max_size": 32}
        }}"#,
    );

    let values: Value = Builder::values("db").file(&file).load().unwrap();

    assert_eq!(
        values.get("host").and_then(Value::as_str),
        Some("localhost")
    );
    assert_eq!(values.get("port").and_then(Value::as_i64), Some(5432));
    assert_eq!(values.get("port").and_then(Value::as_u64), Some(5432));
    assert_eq!(values.get("port").and_then(Value::as_integer), Some(5432));
    assert_eq!(values.get("ratio").and_then(Value::as_float), Some(0.25));
    assert_eq!(values.get("tls").and_then(Value::as_bool), Some(true));
    assert_eq!(values.get("nothing"), Some(&Value::Null));
    assert_eq!(
        values.get("pool.max_size").and_then(Value::as_i64),
        Some(32),
        "a nested path is one dotted read"
    );

    let tags = values.get("tags").and_then(Value::as_array).unwrap();
    assert_eq!(tags.len(), 2);
    assert_eq!(tags[0].as_str(), Some("a"));

    let pool = values.get("pool").and_then(Value::as_table).unwrap();
    assert!(pool.contains_key("max_size"), "a sub-tree is a table");
}

/// An accessor answers for the shape that is there and `None` for every
/// other — including the two a numeric type would be tempted to blur.
#[test]
fn an_accessor_never_converts_between_shapes() {
    let integer = Value::Integer(1);
    let float = Value::Float(1.0);

    assert_eq!(integer.as_float(), None, "an integer is not a float");
    assert_eq!(float.as_integer(), None, "and a float is not an integer");

    // Narrowing rather than saturating: what does not fit is absent.
    assert_eq!(Value::Integer(i128::MAX).as_i64(), None);
    assert_eq!(Value::Integer(-1).as_u64(), None);
    assert_eq!(Value::Integer(-1).as_i64(), Some(-1));

    assert_eq!(Value::String("8080".into()).as_i64(), None);
    assert_eq!(Value::Bool(true).as_str(), None);
}

#[test]
fn a_missing_path_and_a_wrong_type_are_told_apart() {
    let file = write(
        "tests/scratch/schemaless-errors.json",
        r#"{"db": {"host": "localhost", "pool": {"max_size": 32}}}"#,
    );

    let values: Value = Builder::values("db").file(&file).load().unwrap();

    assert_eq!(values.get("nowhere"), None);
    assert_eq!(
        values.get("host.deeper"),
        None,
        "a walk through a scalar ends, it does not error"
    );

    assert_eq!(
        values.get_as::<u16>("nowhere").unwrap_err().kind(),
        ErrorKind::Missing
    );
    assert_eq!(
        values.get_as::<u16>("host").unwrap_err().kind(),
        ErrorKind::Type
    );
    assert_eq!(values.get_as::<u16>("pool.max_size").unwrap(), 32);

    let error = values.get_as::<u16>("host").unwrap_err();
    assert_eq!(error.path(), "host", "the path is the actionable half");
}

/// The keys a program did not declare, learned at runtime — the schemaless
/// replacement for a field list.
#[test]
fn leaf_paths_names_the_keys_that_are_actually_there() {
    let file = write(
        "tests/scratch/schemaless-paths.json",
        r#"{"db": {"host": "h", "pool": {"max": 1, "min": 2}, "tags": ["a"], "empty": {}}}"#,
    );

    let values: Value = Builder::values("db").file(&file).load().unwrap();

    assert_eq!(
        values.leaf_paths(),
        ["empty", "host", "pool.max", "pool.min", "tags"],
        "an array is a leaf, and so is an empty table"
    );
    assert!(
        Value::Integer(1).leaf_paths().is_empty(),
        "a tree that is not a table has no paths"
    );
}

// ---------------------------------------------------------------------------
// The engine, unchanged
// ---------------------------------------------------------------------------

/// The layers do not know what `T` is, and this is the proof: a file, an
/// environment variable above it, and a runtime override above that, all
/// landing in a tree with no fields to land in.
#[test]
fn the_layers_still_layer_without_a_struct() {
    let file = write(
        "tests/scratch/schemaless-layers.json",
        r#"{"svc": {"host": "from-file", "port": 1}}"#,
    );

    std::env::set_var("DCSCHEMALESS_SVC_PORT", "2");

    let values: Value = Builder::values("svc")
        .file(&file)
        .env("DCSCHEMALESS_")
        .load()
        .unwrap();

    std::env::remove_var("DCSCHEMALESS_SVC_PORT");

    assert_eq!(
        values.get("host").and_then(Value::as_str),
        Some("from-file")
    );
    assert_eq!(
        values.get("port").and_then(Value::as_i64),
        Some(2),
        "the environment layer wins, exactly as it does for a struct"
    );
}

/// Provenance is a property of the load, not of the type: the answer must
/// be the same one a struct gets from the same file.
#[test]
fn provenance_matches_what_a_typed_configuration_reports() {
    #[derive(serde::Deserialize)]
    struct Typed {
        #[allow(dead_code)]
        host: String,
    }

    let file = write(
        "tests/scratch/schemaless-provenance.json",
        r#"{"db": {"host": "localhost"}}"#,
    );

    let schemaless = Builder::values("db").file(&file);
    let typed = Builder::<Typed>::new("db").file(&file);

    assert_eq!(
        schemaless.source_of("host").unwrap(),
        typed.source_of("host").unwrap()
    );
    assert!(matches!(
        schemaless.source_of("host").unwrap(),
        Some(dynamic_config::Origin::File(_))
    ));
    assert!(schemaless.is_set("host").unwrap());
    assert!(!schemaless.is_set("port").unwrap());

    // And through the snapshot, per leaf.
    let snapshot = schemaless.snapshot().unwrap();
    assert!(snapshot.source_of("host").is_some());
}

/// The read shape, whole: one atomic load for the tree, a walk for the path,
/// and an `Arc` that keeps answering for the generation it was taken in.
#[test]
fn a_reload_is_a_new_tree_and_an_old_handle_keeps_the_old_one() {
    let file = write(
        "tests/scratch/schemaless-reload.json",
        r#"{"flags": {"beta": false}}"#,
    );

    let config = Dynamic::new(Builder::values("flags").file(&file));
    let before: Arc<Value> = config.init_and_current().unwrap();

    assert_eq!(before.get("beta").and_then(Value::as_bool), Some(false));

    write(
        "tests/scratch/schemaless-reload.json",
        r#"{"flags": {"beta": true, "gamma": 1}}"#,
    );
    config.reload().unwrap();

    let after = config.current().unwrap();

    assert_eq!(after.get("beta").and_then(Value::as_bool), Some(true));
    assert_eq!(
        after.get("gamma").and_then(Value::as_i64),
        Some(1),
        "a key that did not exist before is simply there — no schema to bar it"
    );
    assert_eq!(
        before.get("beta").and_then(Value::as_bool),
        Some(false),
        "the handle taken before the reload still reads its own generation"
    );
    assert!(before.get("gamma").is_none());
    assert_eq!(config.generation(), 2);
}

/// A reload hook on a schemaless configuration gets both trees, and
/// `changed_paths` turns them into the paths-only report a struct's hook
/// gets — which is the only reason `Value` serializes at all.
#[test]
fn a_reload_hook_can_diff_two_trees_by_path() {
    use std::sync::Mutex;

    let file = write(
        "tests/scratch/schemaless-hook.json",
        r#"{"hooked": {"password": "before", "port": 1}}"#,
    );

    let config = Dynamic::new(Builder::values("hooked").file(&file));
    let seen = Arc::new(Mutex::new(Vec::new()));

    {
        let recorder = Arc::clone(&seen);
        config.on_reload(move |previous, current| {
            let changes = dynamic_config::changed_paths(&**previous, &**current).unwrap();

            recorder
                .lock()
                .unwrap()
                .extend(changes.iter().map(ToString::to_string));
        });
    }

    config.init().unwrap();
    write(
        "tests/scratch/schemaless-hook.json",
        r#"{"hooked": {"password": "after", "port": 1}}"#,
    );
    config.reload().unwrap();

    let seen = seen.lock().unwrap();

    assert_eq!(seen.as_slice(), ["password changed"]);
    assert!(
        !seen
            .iter()
            .any(|line| line.contains("before") || line.contains("after")),
        "a diff names paths, never values: {seen:?}"
    );
}

/// The last-known-good cache does not know what `T` is either: a broken
/// source and a `Full` cache still start the process.
#[test]
fn the_last_known_good_cache_recovers_a_schemaless_configuration() {
    let file = write(
        "tests/scratch/schemaless-cached.json",
        r#"{"cached": {"host": "good"}}"#,
    );
    let cache = "tests/scratch/schemaless-cache-store.json";
    let _ = fs::remove_file(cache);

    let good = Dynamic::new(
        Builder::values("cached")
            .file(&file)
            .cache(cache, CacheMode::Full),
    );
    good.init().unwrap();

    write("tests/scratch/schemaless-cached.json", "{ not json at all");

    let recovered = Dynamic::new(
        Builder::values("cached")
            .file(&file)
            .cache(cache, CacheMode::Full),
    );
    let values = recovered.init_and_current().unwrap();

    assert_eq!(values.get("host").and_then(Value::as_str), Some("good"));
}

/// Validation is a closure over `&T`, and `T` being a tree rather than a
/// struct changes nothing about when it runs: before anything installs.
#[test]
fn validation_runs_against_the_tree() {
    let file = write(
        "tests/scratch/schemaless-validated.json",
        r#"{"validated": {"port": 0}}"#,
    );

    let config = Dynamic::new(Builder::values("validated").file(&file).validate(|values| {
        match values.get("port").and_then(Value::as_i64) {
            Some(0) => Err(dynamic_config::Error::invalid("port must not be zero")),
            _ => Ok(()),
        }
    }));

    let error = config.init().unwrap_err();

    assert!(error.to_string().contains("must not be zero"), "{error}");
    assert!(
        config.current().is_none(),
        "a refused configuration installs nothing"
    );
}

// ---------------------------------------------------------------------------
// What it does not get, said out loud
// ---------------------------------------------------------------------------

/// The risk this item was warned about: an unknown-key pass with no field
/// list must not return an empty vector that reads as "everything is fine".
#[test]
fn check_reports_resolution_and_says_unknown_keys_were_not_checked() {
    let file = write(
        "tests/scratch/schemaless-check.json",
        r#"{"checked": {"host": "h", "hsot": "typo"}}"#,
    );

    let report = Builder::values("checked").file(&file).check().unwrap();

    assert!(report.failure.is_none(), "any tree is a valid Value");
    assert_eq!(report.resolved.len(), 2, "resolution still answers");
    assert!(report.unknown.is_empty());
    assert!(
        !report.unknown_checked,
        "with no field list, nobody looked — and the report has to say so"
    );
    assert!(
        report.to_string().contains("not checked"),
        "the rendering says it too: {report}"
    );

    // The same document with a field list: the typo is caught, and the flag
    // says the question was asked.
    let typed = Builder::values("checked")
        .file(&file)
        .with_fields(&["host"])
        .check()
        .unwrap();

    assert!(typed.unknown_checked);
    assert_eq!(typed.unknown.len(), 1);
    assert_eq!(typed.unknown[0].path, "hsot");
}

/// A redaction-dependent cache with no secret list is **refused**, not
/// quietly written: a schemaless configuration cannot derive the list, so
/// the failure has to be loud at `init` rather than a file on disk with the
/// password in it.
#[test]
fn a_redacted_cache_is_refused_until_the_secrets_are_named() {
    let file = write(
        "tests/scratch/schemaless-redacted.json",
        r#"{"guarded": {"password": "hunter2-schemaless-cache", "host": "h"}}"#,
    );
    let cache = "tests/scratch/schemaless-redacted-cache.json";
    let _ = fs::remove_file(cache);

    let refused = Dynamic::new(
        Builder::values("guarded")
            .file(&file)
            .cache(cache, CacheMode::Redacted),
    );
    let error = refused.init().unwrap_err();

    assert_eq!(error.kind(), ErrorKind::Backend);
    assert!(
        error.to_string().contains("which fields are secret"),
        "{error}"
    );
    assert!(
        !std::path::Path::new(cache).exists(),
        "and nothing was written"
    );

    // Named by hand, the same cache mode works and drops the secret.
    let allowed = Dynamic::new(
        Builder::values("guarded")
            .file(&file)
            .secrets(&["password"])
            .cache(cache, CacheMode::Redacted),
    );
    allowed.init().unwrap();

    let written = fs::read_to_string(cache).unwrap();

    assert!(!written.contains("hunter2-schemaless-cache"), "{written}");
    assert!(written.contains("host"), "the rest is still cached");
}

#[cfg(feature = "watch")]
mod watching {
    use super::*;
    use dynamic_config::watch::WatchMode;
    use std::time::{Duration, Instant};

    /// A schemaless configuration that could not hot reload would not be
    /// this crate's feature.
    #[test]
    fn a_watched_schemaless_configuration_reloads_on_a_file_change() {
        let file = write(
            "tests/scratch/schemaless-watched.json",
            r#"{"watched": {"level": 1}}"#,
        );

        let config = Dynamic::new(Builder::values("watched").file(&file));
        config.init().unwrap();

        let _handle = config
            .watch_with(
                Duration::from_millis(25),
                WatchMode::Poll {
                    interval: Duration::from_millis(50),
                },
            )
            .unwrap();

        // Rewritten until seen: a poll watcher takes its baseline on its
        // first tick, so one write can land inside it and never look like a
        // change.
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut seen = false;

        while Instant::now() < deadline && !seen {
            write(
                "tests/scratch/schemaless-watched.json",
                r#"{"watched": {"level": 2}}"#,
            );
            std::thread::sleep(Duration::from_millis(200));

            seen = config
                .current()
                .and_then(|values| values.get("level").and_then(Value::as_i64))
                == Some(2);
        }

        assert!(seen, "the watcher never reloaded the tree");
    }
}

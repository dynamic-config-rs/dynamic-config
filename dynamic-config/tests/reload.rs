//! The watcher, exercised against a real file on a real filesystem.
//!
//! Everything here is timing-dependent by nature, so the waits are generous and
//! poll rather than sleep a fixed amount. One test owns the file for the whole
//! binary; splitting it would mean two tests racing over the same path.

#![cfg(all(feature = "watch", feature = "json"))]

use std::fs;
use std::thread;
use std::time::{Duration, Instant};

use dynamic_config::dynamic_config;
use serde::Deserialize;

const PATH: &str = "tests/scratch/reload.json";

#[dynamic_config(files = ["tests/scratch/reload.json"], key = "app", watch, debounce = 50, diff)]
#[derive(Debug, Deserialize)]
struct AppConfig {
    value: u32,
}

fn write(contents: &str) {
    fs::write(PATH, contents).expect("the scratch file should be writable");
}

/// Polls until the snapshot reaches `expected`, or gives up.
///
/// The budget is far longer than any reload needs; it exists so a wedged
/// watcher fails the test instead of hanging CI.
fn settles_on(expected: u32) -> bool {
    let deadline = Instant::now() + Duration::from_secs(15);

    while Instant::now() < deadline {
        if AppConfig::current().value == expected {
            return true;
        }

        thread::sleep(Duration::from_millis(25));
    }

    false
}

#[test]
fn the_watcher_reloads_on_edit_and_survives_a_broken_file() {
    fs::create_dir_all("tests/scratch").expect("the scratch directory should be creatable");
    write(r#"{"app": {"value": 1}}"#);

    AppConfig::init().expect("the initial load should succeed");
    assert_eq!(AppConfig::current().value, 1);

    let watch = AppConfig::start_watch().expect("the watcher thread should spawn");

    // A duplicate is refused loudly — and the refusal must not take down
    // the watcher that is actually running.
    let error = AppConfig::start_watch().expect_err("a second watcher is an error");
    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);

    write(r#"{"app": {"value": 2}}"#);
    assert!(settles_on(2), "an edit should reach the snapshot");

    // A half-written or invalid file must not take the process down, and must
    // not blank out a configuration that is still perfectly good.
    write("{ this is not json");
    thread::sleep(Duration::from_millis(750));
    assert_eq!(
        AppConfig::current().value,
        2,
        "a broken file should leave the previous snapshot in place"
    );

    // ...and recovery needs no intervention.
    write(r#"{"app": {"value": 3}}"#);
    assert!(settles_on(3), "a repaired file should reload on its own");

    // Dropping the handle stops the watcher, so later edits go unnoticed.
    drop(watch);
    thread::sleep(Duration::from_millis(250));

    write(r#"{"app": {"value": 4}}"#);
    thread::sleep(Duration::from_millis(750));

    assert_eq!(
        AppConfig::current().value,
        3,
        "a dropped handle should have stopped the watcher"
    );
}

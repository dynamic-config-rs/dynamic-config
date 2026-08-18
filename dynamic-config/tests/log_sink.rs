//! The diagnostic sink and level, from the outside.
//!
//! One test function on purpose: the level and the sink are process-wide
//! (that is their contract — a wheel installs one for the whole process),
//! so parallel test functions would race each other's installs. One
//! function, sequential scenes, and every scene restores the default it
//! found.
//!
//! Only meaningful without `tracing`: a compiled-in subscriber takes the
//! lines before this layer exists.

#![cfg(all(feature = "watch", feature = "json", not(feature = "tracing")))]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use dynamic_config::{clear_log_sink, dynamic_config, set_log_level, set_log_sink, LogLevel};
use serde::Deserialize;

#[dynamic_config]
#[derive(Deserialize)]
struct SinkProbe {
    n: u32,
}

fn reload_out_of(directory: &tempfile::TempDir, n: u32) {
    let path = directory.path().join("config.json");
    std::fs::write(&path, format!(r#"{{"probe": {{"n": {n}}}}}"#)).expect("written");

    let file = path.to_str().expect("utf-8");

    if n == 1 {
        SinkProbe::builder("probe")
            .file(file)
            .init()
            .expect("loads");
    } else {
        SinkProbe::builder("probe")
            .file(file)
            .reload()
            .expect("reloads");
    }
}

#[test]
fn the_sink_and_the_level_govern_every_line() {
    let directory = tempfile::tempdir().expect("a directory");
    let lines: Arc<Mutex<Vec<(LogLevel, String)>>> = Arc::default();

    // ── an installed sink receives what the engine would have printed ──
    let seen = Arc::clone(&lines);
    set_log_sink(move |level, line| {
        seen.lock()
            .expect("not poisoned")
            .push((level, line.to_string()));
    });

    reload_out_of(&directory, 1);

    // A watcher is what makes reloads emit; drive one reload through it.
    let handle = SinkProbe::builder("probe")
        .file(
            directory
                .path()
                .join("config.json")
                .to_str()
                .expect("utf-8"),
        )
        .watch(Duration::from_millis(20))
        .expect("watches");

    reload_out_of(&directory, 2);

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while lines.lock().expect("not poisoned").is_empty() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }

    drop(handle);

    {
        let taken = lines.lock().expect("not poisoned");

        assert!(
            taken
                .iter()
                .any(|(level, line)| { *level == LogLevel::Info && line.contains("reloaded") }),
            "the sink never saw the reload line: {taken:?}"
        );
        assert!(
            taken
                .iter()
                .all(|(_, line)| !line.contains("[dynamic-config]")),
            "the prefix belongs to stderr, not to a sink"
        );
    }

    // ── the level gates the sink too ──
    lines.lock().expect("not poisoned").clear();
    set_log_level(LogLevel::Off);

    let handle = SinkProbe::builder("probe")
        .file(
            directory
                .path()
                .join("config.json")
                .to_str()
                .expect("utf-8"),
        )
        .watch(Duration::from_millis(20))
        .expect("watches");

    reload_out_of(&directory, 3);
    std::thread::sleep(Duration::from_millis(300));
    drop(handle);

    assert!(
        lines.lock().expect("not poisoned").is_empty(),
        "Off still let a line through"
    );

    // ── restore what everything else expects ──
    set_log_level(LogLevel::Info);
    clear_log_sink();
}

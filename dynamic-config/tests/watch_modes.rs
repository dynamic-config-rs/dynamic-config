//! Watching under conditions the native backend does not cover.
//!
//! Two of them matter in practice:
//!
//! - **Polling.** inotify and its equivalents do not fire on many network and
//!   overlay filesystems. The failure is silent — the watch registers and then
//!   never delivers — so polling has to be chosen deliberately.
//! - **Kubernetes ConfigMaps.** An update is not a write: the kubelet builds a
//!   new timestamped directory and swings a `..data` symlink at it, so the
//!   config file's own inode never changes. Watching the *directory* is what
//!   makes this visible at all, and "should" is not "is tested".

#![cfg(all(feature = "watch", feature = "json"))]

use std::fs;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use dynamic_config::dynamic_config;
use dynamic_config::watch::WatchMode;
use serde::Deserialize;

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Polled {
    value: u32,
}

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Twice {
    value: u32,
}

// The ConfigMap machinery below is unix-only, like the symlinks it imitates,
// so everything only *it* uses is gated with it — Windows builds with
// `-D warnings`, and dead code there is an error, not a footnote.
#[cfg(unix)]
#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Mounted {
    value: u32,
}

/// Polls until `read` reports `expected`, or gives up well after it should have.
#[cfg(unix)]
fn settles_on(read: impl Fn() -> u32, expected: u32) -> bool {
    let deadline = Instant::now() + Duration::from_secs(15);

    while Instant::now() < deadline {
        if read() == expected {
            return true;
        }

        thread::sleep(Duration::from_millis(25));
    }

    false
}

#[test]
fn the_poll_backend_notices_an_edit() {
    let path = "tests/scratch/polled.json";
    fs::create_dir_all("tests/scratch").unwrap();
    fs::write(path, r#"{"app": {"value": 1}}"#).unwrap();

    let builder = Polled::builder("app").file(path);
    builder.init().expect("the initial load should succeed");

    let _watch = builder
        .watch_with(
            Duration::from_millis(50),
            WatchMode::Poll {
                interval: Duration::from_millis(100),
            },
        )
        .expect("the poll watcher should start");

    // **One** write, immediately after the watcher started — the case that
    // used to be lost. The poll backend compares mtimes in whole seconds, so
    // this edit and the scan before it share a timestamp and look identical;
    // it is noticed because the contents are hashed as well.
    //
    // Before that, a caller had to keep rewriting the file until an edit
    // happened to cross a second boundary — which is not a guarantee anybody
    // could build on, and this test used to be written that way.
    fs::write(path, r#"{"app": {"value": 2}}"#).unwrap();

    let deadline = Instant::now() + Duration::from_secs(15);

    while Instant::now() < deadline {
        if Polled::current().value == 2 {
            return;
        }

        thread::sleep(Duration::from_millis(25));
    }

    panic!("polling should pick a single edit up without being written to again");
}

#[test]
fn the_poll_backend_notices_two_edits_inside_one_second() {
    let path = "tests/scratch/polled-twice.json";
    fs::create_dir_all("tests/scratch").unwrap();
    fs::write(path, r#"{"app": {"value": 1}}"#).unwrap();

    let builder = Twice::builder("app").file(path);
    builder.init().expect("the initial load should succeed");

    let _watch = builder
        .watch_with(
            Duration::from_millis(20),
            WatchMode::Poll {
                interval: Duration::from_millis(50),
            },
        )
        .expect("the poll watcher should start");

    // Two writes, milliseconds apart. With second-granularity timestamps the
    // pair is indistinguishable from no write at all; the hash is what makes
    // the *last* of them the one that serves.
    fs::write(path, r#"{"app": {"value": 2}}"#).unwrap();
    thread::sleep(Duration::from_millis(10));
    fs::write(path, r#"{"app": {"value": 3}}"#).unwrap();

    let deadline = Instant::now() + Duration::from_secs(15);

    while Instant::now() < deadline {
        if Twice::current().value == 3 {
            return;
        }

        thread::sleep(Duration::from_millis(25));
    }

    panic!("polling should settle on the last of two edits in one second");
}

/// Rebuilds the `..data` symlink the way the kubelet does.
///
/// ```text
/// config.json -> ..data/config.json
/// ..data      -> ..2026_08_09_00_00_00
/// ```
///
/// Gated like its only caller: symlinks are the mechanism under test, and
/// `std::os::unix` does not exist on Windows.
#[cfg(unix)]
fn write_configmap(root: &Path, generation: &str, value: u32) {
    let payload = root.join(generation);
    fs::create_dir_all(&payload).unwrap();
    fs::write(
        payload.join("config.json"),
        format!(r#"{{"app": {{"value": {value}}}}}"#),
    )
    .unwrap();

    // The swap is atomic in the kubelet; a remove-then-create is close enough
    // to produce the same event shape.
    let data = root.join("..data");
    let _ = fs::remove_file(&data);
    std::os::unix::fs::symlink(generation, &data).unwrap();

    let visible = root.join("config.json");
    if !visible.exists() {
        std::os::unix::fs::symlink("..data/config.json", &visible).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn a_configmap_symlink_swap_is_seen_as_a_change() {
    let root = Path::new("tests/scratch/k8s");
    let _ = fs::remove_dir_all(root);
    fs::create_dir_all(root).unwrap();

    write_configmap(root, "..2026_08_09_00_00_00", 1);

    let builder = Mounted::builder("app").file("tests/scratch/k8s/config.json");
    builder
        .init()
        .expect("the mounted file should load through the symlinks");
    assert_eq!(Mounted::current().value, 1);

    let _watch = builder
        .watch(Duration::from_millis(50))
        .expect("the watcher should start");

    // The file's own inode never changes; only the directory does. A file-level
    // watch would see nothing at all here.
    write_configmap(root, "..2026_08_09_00_00_01", 2);

    assert!(
        settles_on(|| Mounted::current().value, 2),
        "watching the directory should surface a ConfigMap update"
    );
}

/// A chatty neighbour in the config directory must not starve the reload:
/// irrelevant events used to extend the debounce window indefinitely.
#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Stormed {
    value: u32,
}

#[test]
fn a_storm_of_unrelated_events_does_not_starve_the_reload() {
    let root = Path::new("tests/scratch/storm");
    fs::create_dir_all(root).unwrap();
    fs::write(root.join("config.json"), r#"{"app": {"value": 1}}"#).unwrap();

    let builder = Stormed::builder("app").file("tests/scratch/storm/config.json");
    builder.init().expect("the initial load succeeds");
    let _watch = builder
        .watch_with(
            Duration::from_millis(200),
            WatchMode::Poll {
                interval: Duration::from_millis(100),
            },
        )
        .expect("the watcher starts");

    // A neighbour that never shuts up: an unrelated file rewritten every
    // 50ms — well inside the 200ms debounce window, forever.
    let storming = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let storm = {
        let storming = std::sync::Arc::clone(&storming);
        let noise = root.join("neighbour.log");
        std::thread::spawn(move || {
            let mut i = 0u64;
            while storming.load(std::sync::atomic::Ordering::Relaxed) {
                let _ = fs::write(&noise, i.to_le_bytes());
                i += 1;
                thread::sleep(Duration::from_millis(50));
            }
        })
    };

    // Edit the config repeatedly rather than once, and here that is the
    // point rather than a workaround: the old code starved even a stream of
    // real edits, because the unfiltered storm kept the quiet period from
    // ever elapsing.
    //
    // Old behaviour: timeout. New behaviour: irrelevant events do not extend
    // the window at all, and even relevant churn is bounded by max_wait
    // (4 × debounce). The deadline is deliberately generous — this is a
    // correctness test, not a latency benchmark.
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let mut reloaded = false;
    let mut edit = 2u32;

    while std::time::Instant::now() < deadline {
        fs::write(
            root.join("config.json"),
            format!(r#"{{"app": {{"value": {edit}}}}}"#),
        )
        .unwrap();
        edit += 1;

        thread::sleep(Duration::from_millis(500));

        if Stormed::current().value >= 2 {
            reloaded = true;
            break;
        }
    }

    storming.store(false, std::sync::atomic::Ordering::Relaxed);
    let _ = storm.join();

    assert!(reloaded, "the reload starved behind unrelated events");
}

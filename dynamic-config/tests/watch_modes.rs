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
use serde::Deserialize;

#[dynamic_config(
    files = ["tests/scratch/polled.json"],
    key = "app",
    watch,
    debounce = 50,
    poll_interval = 100
)]
#[derive(Debug, Deserialize)]
struct Polled {
    value: u32,
}

#[dynamic_config(files = ["tests/scratch/k8s/config.json"], key = "app", watch, debounce = 50)]
#[derive(Debug, Deserialize)]
struct Mounted {
    value: u32,
}

/// Polls until `read` reports `expected`, or gives up well after it should have.
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

    Polled::init().expect("the initial load should succeed");

    let _watch = Polled::start_watch().expect("the poll watcher should start");

    // A poll watcher takes its baseline on its first tick, not when `watch()`
    // returns, so a single write can land inside the baseline and never look
    // like a change. Rewriting until it is noticed is what a caller would have
    // to do too, and is the honest shape of the guarantee: polling detects
    // changes *eventually*, not by a deadline.
    let deadline = Instant::now() + Duration::from_secs(15);

    while Instant::now() < deadline {
        fs::write(path, r#"{"app": {"value": 2}}"#).unwrap();
        thread::sleep(Duration::from_millis(200));

        if Polled::current().value == 2 {
            return;
        }
    }

    panic!("polling should pick the edit up within its interval");
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

    Mounted::init().expect("the mounted file should load through the symlinks");
    assert_eq!(Mounted::current().value, 1);

    let _watch = Mounted::start_watch().expect("the watcher should start");

    // The file's own inode never changes; only the directory does. A file-level
    // watch would see nothing at all here.
    write_configmap(root, "..2026_08_09_00_00_01", 2);

    assert!(
        settles_on(|| Mounted::current().value, 2),
        "watching the directory should surface a ConfigMap update"
    );
}

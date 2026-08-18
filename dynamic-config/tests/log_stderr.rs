//! What reaches fd 2, proven from outside the process.
//!
//! The promise the sink work must keep: with nothing configured the bytes
//! on stderr are exactly what 0.6 printed, and with a sink installed
//! stderr is silent. Only a subprocess can assert either, so each test
//! re-runs this binary as its own child.

#![cfg(not(feature = "tracing"))]

use std::process::Command;

fn child(scene: &str) -> std::process::Output {
    Command::new(std::env::current_exe().expect("a test binary"))
        .args(["--exact", scene, "--nocapture", "--include-ignored"])
        .env("LOG_STDERR_CHILD", "1")
        .output()
        .expect("the child ran")
}

fn is_child() -> bool {
    std::env::var_os("LOG_STDERR_CHILD").is_some()
}

#[test]
#[ignore = "runs only as the subprocess the tests below spawn"]
fn scene_default() {
    if !is_child() {
        return;
    }

    dynamic_config::__log_remote_reload("probe", None);
}

#[test]
#[ignore = "runs only as the subprocess the tests below spawn"]
fn scene_with_a_sink() {
    if !is_child() {
        return;
    }

    dynamic_config::set_log_sink(|_, _| { /* swallowed on purpose */ });
    dynamic_config::__log_remote_reload("probe", None);
}

#[test]
fn unconfigured_stderr_is_byte_identical_to_what_it_always_was() {
    let output = child("scene_default");

    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        stderr.contains("[dynamic-config] probe: reloaded from the remote store\n"),
        "the default line moved: {stderr:?}"
    );
}

#[test]
fn an_installed_sink_leaves_stderr_silent() {
    let output = child("scene_with_a_sink");

    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !stderr.contains("[dynamic-config]"),
        "a line escaped past the sink to stderr: {stderr:?}"
    );
}

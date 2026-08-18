//! Graceful shutdown, as its own subject.
//!
//! ```text
//! cargo run -p dynamic-config --example graceful_shutdown --features json,watch
//! ```
//!
//! The engine's answer is deliberately boring: there is nothing to flush.
//! Every install already happened atomically, the last-known-good cache
//! (when configured) was written at install time, and a watcher is a
//! thread behind an RAII handle — dropping the handle stops it. So a
//! graceful shutdown is: stop taking work, drop the handles, exit. This
//! example makes each of those visible.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use dynamic_config::dynamic_config;
use serde::Deserialize;

// Read through `Debug`, which dead-code analysis does not count — the
// same note every example here carries.
#[allow(dead_code)]
#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Server {
    port: u16,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("config.json");
    std::fs::write(&path, r#"{"server": {"port": 8080}}"#)?;
    let file = path.to_str().expect("utf-8");

    Server::builder("server").file(file).init()?;

    // The handle is the watcher's lifetime. Not detached: detaching is
    // for processes that never stop; a service that shuts down cleanly
    // keeps the handle where shutdown can drop it.
    let watcher = Server::builder("server")
        .file(file)
        .watch(Duration::from_millis(50))?;

    let stop = Arc::new(AtomicBool::new(false));

    // Stand-in for a signal handler: in a real service this is ctrl_c()
    // or SIGTERM from the platform.
    let trigger = Arc::clone(&stop);
    let worker = std::thread::spawn(move || {
        while !trigger.load(Ordering::Relaxed) {
            let _config = Server::current(); // work reads config per task
            std::thread::sleep(Duration::from_millis(20));
        }
    });

    std::thread::sleep(Duration::from_millis(200));

    println!("shutting down:");

    // 1. Stop taking work.
    stop.store(true, Ordering::Relaxed);
    worker.join().expect("the worker exits");
    println!("  workers drained");

    // 2. Drop the watcher. The thread ends; no more installs can land.
    drop(watcher);
    println!("  watcher stopped");

    // 3. Exit. Nothing to flush: installs were atomic when they happened.
    println!("  nothing to flush — that is the design, not an omission");

    Ok(())
}

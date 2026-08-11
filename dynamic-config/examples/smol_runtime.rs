//! The async surface on `smol`, with no tokio anywhere in the build.
//!
//! ```text
//! cargo run -p dynamic-config --example smol_runtime --features async,watch,json
//! ```
//!
//! `changes()` is a hand-written `Future` over a generation counter and a list
//! of wakers — `std`, and nothing else — so any executor drives it. This example
//! is the claim being checked rather than asserted: `cargo tree` for it contains
//! no `tokio`.
//!
//! The one genuinely runtime-specific thing is *where blocking work runs*, and
//! that is pluggable: `set_blocking_executor` hands the crate smol's
//! `unblock`, so `load_async` does its file reading off the executor's threads.

use std::path::{Path, PathBuf};
use std::time::Duration;

use dynamic_config::{dynamic_config, BlockingExecutor};
use serde::Deserialize;

const DIRECTORY: &str = "/tmp/dynamic-config-smol";

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct ServerConfig {
    greeting: String,
    workers: usize,
}

/// smol's blocking pool, handed to the crate.
///
/// Without one it spawns a thread per load, which is correct everywhere and
/// cheap enough for work that happens at startup and on reload — but a program
/// that already has a pool should not grow a second habit.
struct Smol;

impl BlockingExecutor for Smol {
    fn execute(&self, work: Box<dyn FnOnce() + Send + 'static>) {
        smol::spawn(smol::unblock(work)).detach();
    }
}

fn write(directory: &Path, greeting: &str, workers: usize) -> std::io::Result<()> {
    std::fs::write(
        directory.join("config.json"),
        format!(r#"{{"server": {{"greeting": "{greeting}", "workers": {workers}}}}}"#),
    )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = PathBuf::from(DIRECTORY);
    std::fs::create_dir_all(&directory)?;
    write(&directory, "hello from smol", 4)?;

    let _ = dynamic_config::set_blocking_executor(Smol);

    smol::block_on(async {
        let sources = ServerConfig::builder("server")
            .file("/tmp/dynamic-config-smol/config.json")
            .env("APP_");

        // The load runs on smol's blocking pool rather than on the executor
        // thread, so a slow disk does not stall every other task.
        sources.init_async().await?;

        println!("loaded: {}", ServerConfig::current().greeting);
        println!("workers: {}\n", ServerConfig::current().workers);

        // A watcher, and a task awaiting what it installs. Neither knows which
        // runtime it is on.
        let handle = sources.watch(Duration::from_millis(250))?;
        let mut changes = ServerConfig::changes();

        let editor = smol::spawn({
            let directory = directory.clone();

            async move {
                for (greeting, workers) in [("edited once", 8), ("edited twice", 16)] {
                    smol::Timer::after(Duration::from_millis(400)).await;

                    let _ = write(&directory, greeting, workers);
                }
            }
        });

        for _ in 0..2 {
            // Resolves when the watcher installs a new snapshot. No polling, no
            // channel type from somebody else's runtime.
            let config = changes.changed().await;

            println!("woke up: {} ({} workers)", config.greeting, config.workers);
        }

        editor.await;
        drop(handle);

        Ok::<(), Box<dyn std::error::Error>>(())
    })?;

    let _ = std::fs::remove_dir_all(&directory);

    Ok(())
}

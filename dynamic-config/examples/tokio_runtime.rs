//! The async surface on tokio, with its blocking pool doing the file reading.
//!
//! ```text
//! cargo run -p dynamic-config --example tokio_runtime --features tokio,watch,json
//! ```
//!
//! The companion to [`smol_runtime`] and [`embassy_runtime`]: the same code
//! driven by a third executor. What differs on tokio is only where blocking
//! work goes — the `tokio` feature wires `spawn_blocking` in for you, so
//! `load_async` reads files off the executor's threads without being asked.
//!
//! [`smol_runtime`]: https://github.com/ctolon/dynamic-config/blob/main/dynamic-config/examples/smol_runtime.rs
//! [`embassy_runtime`]: https://github.com/ctolon/dynamic-config/blob/main/dynamic-config/examples/embassy_runtime.rs

use std::path::{Path, PathBuf};
use std::time::Duration;

use dynamic_config::dynamic_config;
use serde::Deserialize;

const DIRECTORY: &str = "/tmp/dynamic-config-tokio";

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct ServerConfig {
    greeting: String,
    workers: usize,
}

fn write(directory: &Path, greeting: &str, workers: usize) -> std::io::Result<()> {
    std::fs::write(
        directory.join("config.json"),
        format!(r#"{{"server": {{"greeting": "{greeting}", "workers": {workers}}}}}"#),
    )
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = PathBuf::from(DIRECTORY);
    std::fs::create_dir_all(&directory)?;
    write(&directory, "hello from tokio", 4)?;

    let sources = ServerConfig::builder("server")
        .file("/tmp/dynamic-config-tokio/config.json")
        .env("APP_");

    // With the `tokio` feature there is no `set_blocking_executor` call: the
    // crate uses `spawn_blocking`, because a tokio program already has a pool
    // and growing a second habit would be a waste of threads.
    sources.init_async().await?;

    println!("loaded: {}", ServerConfig::current().greeting);
    println!("workers: {}\n", ServerConfig::current().workers);

    let handle = sources.watch(Duration::from_millis(250))?;

    // Two readers, to show the shape that matters: `changes()` is per-handle,
    // so every task wakes on its own rather than sharing a receiver.
    let mut first = ServerConfig::changes();
    let mut second = ServerConfig::changes();

    let readers = tokio::spawn(async move {
        for _ in 0..2 {
            let config = first.changed().await;

            println!(
                "  reader A: {} ({} workers)",
                config.greeting, config.workers
            );
        }
    });

    let other = tokio::spawn(async move {
        for _ in 0..2 {
            let config = second.changed().await;

            println!(
                "  reader B: {} ({} workers)",
                config.greeting, config.workers
            );
        }
    });

    let editor = tokio::spawn({
        let directory = directory.clone();

        async move {
            for (greeting, workers) in [("edited once", 8), ("edited twice", 16)] {
                tokio::time::sleep(Duration::from_millis(400)).await;

                let _ = write(&directory, greeting, workers);
            }
        }
    });

    let _ = tokio::join!(readers, other, editor);

    // Dropping the handle stops the watcher; `.detach()` is for a server that
    // wants it to outlive `main`'s body.
    drop(handle);

    println!("\nfinal: {}", ServerConfig::current().greeting);
    println!("\nThe same `changes()` future drives on smol and on Embassy —");
    println!("see the other two runtime examples. Only the blocking pool is");
    println!("runtime-specific, and it is pluggable.");

    let _ = std::fs::remove_dir_all(&directory);

    Ok(())
}

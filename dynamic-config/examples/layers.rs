//! Every layer, and which one wins.
//!
//! ```text
//! set_default < discovered < files < environment < set_flag < set_override
//! ```
//!
//! Run from the workspace root, and watch one key climb the stack:
//!
//! ```text
//! cargo run -p dynamic-config --example layers --features json
//! ```

use dynamic_config::{dynamic_config, Origin};
use serde::Deserialize;

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct ServerConfig {
    #[allow(dead_code)]
    host: String,
    port: u16,
    /// Nothing in the file supplies this, so it starts life as a default.
    #[serde(default)]
    workers: u16,
}

fn sources() -> dynamic_config::Builder<ServerConfig> {
    ServerConfig::builder("server")
        .file("dynamic-config/examples/config.json")
        .env("APP_")
}

fn show(stage: &str) -> Result<(), dynamic_config::Error> {
    let sources = sources();
    let config = sources.load()?;
    let origin = sources.source_of("port")?.unwrap_or(Origin::Unknown);

    println!(
        "{stage:<24} port = {:<6} workers = {:<4} ({origin})",
        config.port, config.workers
    );

    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    show("file only")?;

    // Below the files: only fills what nothing else supplies. `port` is in the
    // file, so this default is invisible; `workers` is not, so it appears.
    ServerConfig::set_default("port", 1u16)?;
    ServerConfig::set_default("workers", 4u16)?;
    show("+ defaults")?;

    // The environment beats every file.
    std::env::set_var("APP_SERVER_PORT", "3000");
    show("+ environment")?;

    // A flag beats the environment: a person typed it for this one run.
    ServerConfig::set_flag("port", Some(4000u16))?;
    show("+ flag")?;

    // An override beats everything. This is the layer for tests.
    ServerConfig::set_override("port", 5000u16)?;
    show("+ override")?;

    // Peeling them back off restores whatever was underneath.
    ServerConfig::clear_overrides();
    ServerConfig::clear_flags();
    std::env::remove_var("APP_SERVER_PORT");
    show("back to the file")?;

    Ok(())
}

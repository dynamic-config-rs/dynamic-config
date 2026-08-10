//! Load a configuration section once and read it.
//!
//! Run from the workspace root:
//!
//! ```text
//! cargo run -p dynamic-config --example basic
//! APP_SERVER_PORT=9000 cargo run -p dynamic-config --example basic
//! ```

use dynamic_config::dynamic_config;
use serde::Deserialize;

#[dynamic_config(
    files = ["dynamic-config/examples/config.json"],
    key = "server",
    env = "APP_"
)]
#[derive(Debug, Deserialize)]
struct ServerConfig {
    host: String,
    port: u16,
    tags: Vec<String>,
}

fn main() -> Result<(), dynamic_config::Error> {
    ServerConfig::init()?;

    // Take the snapshot once; every later read is this same generation.
    let config = ServerConfig::current();

    println!("listening on {}:{}", config.host, config.port);
    println!("tags: {:?}", config.tags);

    Ok(())
}

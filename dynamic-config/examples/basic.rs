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

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct ServerConfig {
    host: String,
    port: u16,
    tags: Vec<String>,
}

fn main() -> Result<(), dynamic_config::Error> {
    // The attribute declares the type; where the values come from is chosen
    // here, at runtime. `init_and_current` installs and hands back what it
    // installed — everywhere else in the program reads the same snapshot with
    // `ServerConfig::current()`, which needs no builder and no arguments.
    let config = ServerConfig::builder("server")
        .file("dynamic-config/examples/config.json")
        .env("APP_")
        .init_and_current()?;

    println!("listening on {}:{}", config.host, config.port);
    println!("tags: {:?}", config.tags);

    Ok(())
}

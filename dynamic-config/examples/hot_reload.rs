//! Watch a file and reload the snapshot when it changes.
//!
//! Run from the workspace root, then edit `dynamic-config/examples/config.toml`
//! in another terminal and watch the printed values follow:
//!
//! ```text
//! cargo run -p dynamic-config --example hot_reload --features watch,toml
//! ```
//!
//! Try saving an invalid file too: the error is reported and the previous
//! snapshot keeps serving.

use std::thread;
use std::time::Duration;

use dynamic_config::dynamic_config;
use serde::Deserialize;

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct ServerConfig {
    host: String,
    port: u16,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sources = ServerConfig::builder("server")
        .file("dynamic-config/examples/config.toml")
        .env("APP_");

    sources.init()?;
    // A server watches for as long as it runs, so nothing here owns the handle.
    sources.watch(Duration::from_millis(250))?.detach();

    println!("edit dynamic-config/examples/config.toml — ctrl-c to stop");

    loop {
        // A real server takes the snapshot once per request; this loop stands
        // in for that, so each iteration sees whatever is current.
        let config = ServerConfig::current();

        println!("{}:{}", config.host, config.port);

        thread::sleep(Duration::from_secs(2));
    }
}

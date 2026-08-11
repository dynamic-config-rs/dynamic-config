//! Finding configuration by name, and layering a profile over it.
//!
//! `examples/deployment/` stands in for a real machine: `etc/` is what a
//! package ships, `home/` is what a user overrides.
//!
//! ```text
//! cargo run -p dynamic-config --example discovery --features json
//! APP_ENV=production cargo run -p dynamic-config --example discovery --features json
//! ```

use dynamic_config::dynamic_config;
use serde::Deserialize;

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct ServerConfig {
    host: String,
    port: u16,
    tls: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sources = ServerConfig::builder("server")
        .discover(
            "config",
            [
                "dynamic-config/examples/deployment/etc",
                "dynamic-config/examples/deployment/home",
            ],
        )
        .profile_env("APP_ENV");

    let config = sources.load()?;

    println!(
        "host = {}  port = {}  tls = {}",
        config.host, config.port, config.tls
    );

    // Every directory that had a match contributed one file, so the report
    // shows different origins for different keys.
    println!("\n{}", sources.check()?);

    println!(
        "\n`host` comes from etc/, `port` from home/. With APP_ENV=production, \
         home/config.production.json layers over home/config.json."
    );

    println!(
        "\nA profile variant sits directly on top of *its own* base, so a later \
         directory's plain file still wins over an earlier directory's variant. \
         The search order is the layering order; the profile is not a layer of \
         its own above it."
    );

    Ok(())
}

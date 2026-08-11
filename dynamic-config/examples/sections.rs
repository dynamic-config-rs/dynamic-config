//! Several configuration types over one set of files.
//!
//! ```text
//! cargo run -p dynamic-config --example sections --features json,watch
//! ```
//!
//! Each struct claims a top-level key and sees nothing else. They can list
//! different files, use different environment prefixes, and watch or not
//! independently — which is the point: a subsystem's configuration is its own.

use std::time::Duration;

use dynamic_config::dynamic_config;
use serde::Deserialize;

#[dynamic_config]
#[derive(Debug, Deserialize)]
// Read through `Debug`, which dead-code analysis does not count.
#[allow(dead_code)]
struct ServerConfig {
    host: String,
    port: u16,
}

#[dynamic_config]
#[derive(Deserialize)]
// Read through `Debug`, which dead-code analysis does not count.
#[allow(dead_code)]
struct DatabaseConfig {
    host: String,
    #[config(secret)]
    password: String,
}

#[dynamic_config]
#[derive(Debug, Deserialize)]
// Read through `Debug`, which dead-code analysis does not count.
#[allow(dead_code)]
struct FeatureFlags {
    metrics: bool,
    tracing: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerConfig::builder("server")
        .file("dynamic-config/examples/app.json")
        .env("APP_");

    // Two files: the second is optional and holds the credentials.
    let database = DatabaseConfig::builder("db")
        .file("dynamic-config/examples/app.json")
        .file("dynamic-config/examples/app.secrets.json")
        .env("APP_");

    // No watching for this one: feature flags here are read once at startup
    // on purpose.
    let features = FeatureFlags::builder("features")
        .file("dynamic-config/examples/app.json")
        .env("APP_");

    server.init()?;
    database.init()?;
    features.init()?;

    // Each watcher is separate, so a single edit swaps them one at a time
    // rather than as one transaction. Fine here — no code reads two sections
    // and requires them to be the same generation.
    let debounce = Duration::from_millis(250);
    let _watchers = (server.watch(debounce)?, database.watch(debounce)?);

    println!("server:   {:?}", ServerConfig::current());
    println!("database: {:?}", DatabaseConfig::current());
    println!("features: {:?}", FeatureFlags::current());

    println!("\nEach type sees only its own key:");
    println!(
        "  ServerConfig::is_set(\"password\") = {}",
        ServerConfig::is_set("password")?
    );
    println!(
        "  DatabaseConfig::is_set(\"password\") = {}",
        DatabaseConfig::is_set("password")?
    );

    Ok(())
}

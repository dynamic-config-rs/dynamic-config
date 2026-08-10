//! Several configuration types over one set of files.
//!
//! ```text
//! cargo run -p dynamic-config --example sections --features json,watch
//! ```
//!
//! Each struct claims a top-level key and sees nothing else. They can list
//! different files, use different environment prefixes, and watch or not
//! independently — which is the point: a subsystem's configuration is its own.

use dynamic_config::dynamic_config;
use serde::Deserialize;

#[dynamic_config(files = ["dynamic-config/examples/app.json"], key = "server", env = "APP_", watch)]
#[derive(Debug, Deserialize)]
// Read through `Debug`, which dead-code analysis does not count.
#[allow(dead_code)]
struct ServerConfig {
    host: String,
    port: u16,
}

// Two files: the second is optional and holds the credentials.
#[dynamic_config(
    files = ["dynamic-config/examples/app.json", "dynamic-config/examples/app.secrets.json"],
    key = "db",
    env = "APP_",
    watch,
)]
#[derive(Deserialize)]
// Read through `Debug`, which dead-code analysis does not count.
#[allow(dead_code)]
struct DatabaseConfig {
    host: String,
    #[config(secret)]
    password: String,
}

// No watching: feature flags here are read once at startup on purpose.
#[dynamic_config(files = ["dynamic-config/examples/app.json"], key = "features", env = "APP_")]
#[derive(Debug, Deserialize)]
// Read through `Debug`, which dead-code analysis does not count.
#[allow(dead_code)]
struct FeatureFlags {
    metrics: bool,
    tracing: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    ServerConfig::init()?;
    DatabaseConfig::init()?;
    FeatureFlags::init()?;

    // Each watcher is separate, so a single edit swaps them one at a time
    // rather than as one transaction. Fine here — no code reads two sections
    // and requires them to be the same generation.
    let _watchers = (ServerConfig::start_watch()?, DatabaseConfig::start_watch()?);

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

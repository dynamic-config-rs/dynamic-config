//! Reloading two configuration types as one step.
//!
//! ```text
//! cargo run -p dynamic-config --example group --features json
//! ```
//!
//! Two structs over one file normally reload independently, and for a moment
//! after an edit one is new while the other is old. Usually nobody notices.
//! When it matters — a certificate path and the port it is served on — a
//! half-applied configuration is worse than a late one.

use dynamic_config::{dynamic_config, ReloadGroup};
use serde::Deserialize;

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct ServerConfig {
    #[allow(dead_code)]
    host: String,
    port: u16,
}

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct FeatureFlags {
    metrics: bool,
    #[allow(dead_code)]
    tracing: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // A group reloads through each member's remembered configuration, so
    // every member is initialized through its builder first.
    ServerConfig::builder("server")
        .file("dynamic-config/examples/app.json")
        .init()?;
    FeatureFlags::builder("features")
        .file("dynamic-config/examples/app.json")
        .init()?;

    let group = ReloadGroup::new()
        .with::<ServerConfig>()
        .with::<FeatureFlags>();

    println!("group: {group:?}");

    group.reload()?;
    println!(
        "\nloaded together: port={} metrics={}",
        ServerConfig::current().port,
        FeatureFlags::current().metrics
    );

    // Change one member and break the other, the way a single bad edit to a
    // shared file would.
    ServerConfig::set_override("port", 9090u16)?;
    FeatureFlags::set_override("metrics", "not-a-boolean")?;

    match group.reload() {
        Ok(()) => println!("unexpectedly reloaded"),
        Err(error) => println!("\nrejected: {error}"),
    }

    println!(
        "still serving: port={} metrics={}",
        ServerConfig::current().port,
        FeatureFlags::current().metrics
    );
    println!("  ^ the port change was valid and was still not applied — that is");
    println!("    the whole point: all of it, or none of it.");

    FeatureFlags::clear_overrides();
    group.reload()?;

    println!(
        "\nafter repairing the break: port={} metrics={}",
        ServerConfig::current().port,
        FeatureFlags::current().metrics
    );

    ServerConfig::clear_overrides();

    Ok(())
}

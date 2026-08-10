//! Writing configuration back out, and reading keys with no field.
//!
//! ```text
//! cargo run -p dynamic-config --example persistence --features json
//! ```

use dynamic_config::dynamic_config;
use serde::{Deserialize, Serialize};

#[dynamic_config(files = ["dynamic-config/examples/app.json"], key = "server", save)]
#[derive(Debug, Deserialize, Serialize)]
struct ServerConfig {
    host: String,
    port: u16,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = ServerConfig::load()?;
    println!("loaded: {config:?}");

    // A CLI persisting a preference. The write is atomic — temporary file and
    // rename — because a watcher is very likely watching that directory, and a
    // partial file would look exactly like a broken edit.
    config.port = 9090;

    let path = std::env::temp_dir().join("dynamic-config-example.json");
    config.save(&path)?;

    println!("\nwrote {}:", path.display());
    println!("{}", std::fs::read_to_string(&path)?);
    println!("Nested under the section key, so it reads straight back in.");

    // Reading keys the struct does not name: a plugin's table, a user-defined
    // section, anything whose shape is not known at compile time.
    let snapshot = ServerConfig::snapshot()?;

    println!("\nreading without a struct:");
    println!("  host        = {}", snapshot.get::<String>("host")?);
    println!("  contains(x) = {}", snapshot.contains("nothing_here"));

    std::fs::remove_file(&path)?;

    Ok(())
}

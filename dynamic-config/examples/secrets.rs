//! Keeping credentials out of logs — and what that does not cover.
//!
//! ```text
//! cargo run -p dynamic-config --example secrets --features json
//! ```

use dynamic_config::dynamic_config;
use serde::Deserialize;

#[dynamic_config(files = ["dynamic-config/examples/secrets.json"], key = "db", env = "APP_")]
// No `Debug` in the derive: `#[config(secret)]` writes one, and having both
// would be a compile error rather than a race between two impls.
#[derive(Deserialize)]
struct DatabaseConfig {
    host: String,
    username: String,
    #[config(secret)]
    password: String,
    // Read only through the redacted `Debug` below, which dead-code analysis
    // does not count.
    #[allow(dead_code)]
    #[config(secret)]
    api_token: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = DatabaseConfig::load()?;

    // The values loaded perfectly well — redaction is about printing.
    println!("connecting as {} to {}", config.username, config.host);
    println!("password length: {}", config.password.len());

    // ...but the whole struct is safe to log, which is the one place a
    // credential usually escapes from.
    println!("\n{config:?}");

    println!("\nWhat this covers:");
    println!("  • `{{:?}}` anywhere — a log line, a panic message, a tracing field");
    println!("  • a reload diff, which reports paths and never values");
    println!("  • `check()`, which reports origins and never values");

    println!("\nWhat it does not:");
    println!("  • `println!(\"{{}}\", config.password)` — an explicit read is yours to make");
    println!("  • `save()`, which writes what it was asked to write (0600 on Unix)");
    println!("  • the config file itself, which holds the value in the clear");

    Ok(())
}

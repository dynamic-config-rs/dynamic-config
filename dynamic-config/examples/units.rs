//! Durations and sizes, written the way people write them.
//!
//! ```text
//! cargo run -p dynamic-config --example units --features json
//! ```
//!
//! `timeout = 30` is ambiguous — seconds? milliseconds? — and
//! `max_body = 67108864` is unreadable. No stock `Deserialize` accepts `"30s"`
//! or `"64MiB"`; these adapters do, while still taking a bare number so an
//! existing file keeps working.

use std::time::Duration;

use dynamic_config::dynamic_config;
use serde::Deserialize;

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Limits {
    /// `"30s"`, `"1h30m"`, `"500ms"` — or a bare number of seconds.
    #[serde(with = "dynamic_config::duration")]
    request_timeout: Duration,

    /// The same, for a value that need not be set at all.
    #[serde(default, with = "dynamic_config::duration::option")]
    shutdown_grace: Option<Duration>,

    /// `"64MiB"`, `"1GB"` — or a bare number of bytes.
    #[serde(with = "dynamic_config::bytes")]
    max_body: u64,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sources = Limits::builder("limits")
        .file("dynamic-config/examples/limits.json")
        .env("APP_");

    let limits = sources.load()?;

    println!("request_timeout = {:?}", limits.request_timeout);
    println!("shutdown_grace  = {:?}", limits.shutdown_grace);
    println!("max_body        = {} bytes", limits.max_body);

    println!("\nThe same values through the environment, since a variable is");
    println!("text and these adapters read text:");

    // SAFETY-adjacent note: this is a single-threaded example. A real program
    // sets its environment before it starts threads.
    std::env::set_var("APP_LIMITS_REQUEST_TIMEOUT", "1h30m");
    std::env::set_var("APP_LIMITS_MAX_BODY", "1GB");

    let limits = sources.load()?;
    println!("  request_timeout = {:?}", limits.request_timeout);
    println!("  max_body        = {} bytes", limits.max_body);

    println!("\nBinary and decimal units are not the same thing:");
    println!("  1KiB = {} bytes", dynamic_config::bytes::parse("1KiB")?);
    println!("  1KB  = {} bytes", dynamic_config::bytes::parse("1KB")?);

    println!("\nAnd nonsense is an error listing what was expected:");
    match dynamic_config::duration::parse("30 fortnights") {
        Ok(parsed) => println!("  unexpectedly parsed {parsed:?}"),
        Err(error) => println!("  {error}"),
    }

    std::env::remove_var("APP_LIMITS_REQUEST_TIMEOUT");
    std::env::remove_var("APP_LIMITS_MAX_BODY");

    Ok(())
}

//! Configuring a program under test, without touching a file.
//!
//! ```text
//! cargo run -p dynamic-config --example testing --features json
//! cargo test  -p dynamic-config --example testing --features json
//! ```
//!
//! The override layer exists for this: it wins over every file and every
//! variable, so a test states exactly the configuration it needs and nothing on
//! the machine can change the answer.

use dynamic_config::dynamic_config;
use serde::Deserialize;

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct ServerConfig {
    host: String,
    port: u16,
}

/// The sources, in one place — `main` and the test install from the same ones.
fn sources() -> dynamic_config::Builder<ServerConfig> {
    ServerConfig::builder("server")
        .file("dynamic-config/examples/app.json")
        .env("APP_")
}

/// The code under test: it reads configuration, as production code does.
fn describe_binding() -> Result<String, dynamic_config::Error> {
    let config = ServerConfig::current();

    Ok(format!("{}:{}", config.host, config.port))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    sources().init()?;
    println!("as deployed: {}", describe_binding()?);

    // A test pins what it needs and leaves the rest alone.
    ServerConfig::set_override("port", 1u16)?;
    sources().init()?;

    println!("under test:  {}", describe_binding()?);

    ServerConfig::clear_overrides();
    sources().init()?;
    println!("restored:    {}", describe_binding()?);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The snapshot lives in a `static`, so tests sharing a config type share
    /// its layers. Either give each test its own type, or run them on one
    /// thread — a `static` is process-wide however it is reached.
    #[test]
    fn an_override_decides_what_the_code_under_test_sees() {
        ServerConfig::set_override("host", "test-host").unwrap();
        ServerConfig::set_override("port", 1234u16).unwrap();
        sources().init().unwrap();

        assert_eq!(describe_binding().unwrap(), "test-host:1234");

        ServerConfig::clear_overrides();
    }
}

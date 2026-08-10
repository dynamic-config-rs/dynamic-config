//! The engine without the attribute.
//!
//! ```text
//! cargo run -p dynamic-config --example no_macro --features json
//! ```
//!
//! `load`, `LoadSpec`, `Layer` and `ConfigCell` are the whole of it; the macro
//! writes about twenty-five lines of glue over them. Reach for this when the
//! sources are decided at run time, or when a config type is built by another
//! macro that will not compose with this one.

use dynamic_config::{load, ConfigCell, Format, Layer, LoadSpec, Source};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
// Read through `Debug`, which dead-code analysis does not count.
#[allow(dead_code)]
struct ServerConfig {
    host: String,
    port: u16,
}

// The same storage the macro emits: `const`, so it lives in a `static`.
static SERVER: ConfigCell<ServerConfig> = ConfigCell::new();

// And the same runtime layers.
static OVERRIDES: Layer = Layer::new();

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Sources can be anything by now — a path computed from an argument, a
    // document just read off a socket. `Source` borrows, so nothing leaks.
    let fetched = String::from(r#"{"server": {"host": "0.0.0.0"}}"#);

    let sources = [
        Source::file("dynamic-config/examples/config.json", Format::Json),
        Source::inline(&fetched, Format::Json),
    ];

    let spec = LoadSpec::new("server", &sources)
        .with_env("APP_")
        .with_overrides(&OVERRIDES);

    SERVER.store(load::<ServerConfig>(&spec)?);
    println!("{:?}", SERVER.load().unwrap());

    // The same lock-free read the macro's `current()` performs.
    let held = SERVER.load().expect("just stored");

    OVERRIDES.set("port", 9999u16)?;
    SERVER.store(load::<ServerConfig>(&spec)?);

    println!("{:?}", SERVER.load().unwrap());
    println!(
        "\nthe Arc taken before the swap still reads {} — a reader keeps its \
         own generation",
        held.port
    );

    Ok(())
}

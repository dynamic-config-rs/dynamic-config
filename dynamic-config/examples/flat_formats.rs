//! INI and properties, as first-class sources.
//!
//! ```text
//! cargo run -p dynamic-config --example flat_formats --features ini,properties
//! ```
//!
//! The same document three ways, resolving to the same values — and the
//! layering works across formats, because a layer is a layer: here a
//! properties file overrides an INI base.

use dynamic_config::{load, Format, LoadSpec, Source};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Database {
    host: String,
    port: u16,
    pool: Pool,
}

#[derive(Debug, Deserialize)]
struct Pool {
    max: u32,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let base = r#"
; the base, maintained by the team that owns the service
[db]
host = db.internal
port = 5432

[db.pool]
max = 8
"#;

    // The override, in the format the Java-side tooling emits. Dotted
    // keys nest; later wins, per layer order.
    let overrides = "db.pool.max = 32\n";

    let sources = [
        Source::inline(base, Format::Ini),
        Source::inline(overrides, Format::Properties),
    ];

    let database: Database = load(&LoadSpec::new("db", &sources))?;

    println!("host        {}", database.host);
    println!("port        {}", database.port);
    println!(
        "pool.max    {}   (the properties layer decided)",
        database.pool.max
    );

    assert_eq!(database.pool.max, 32);

    Ok(())
}

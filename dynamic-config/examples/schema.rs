//! A JSON Schema for the config files, so an editor completes and checks them.
//!
//! ```text
//! cargo run -p dynamic-config --example schema --features schema,json
//! ```
//!
//! The point is what the schema describes: not a struct, but a *file*. Two
//! config types share `app.json` here, and one emitted schema covers both.

use dynamic_config::dynamic_config;
use schemars::JsonSchema;
use serde::Deserialize;

#[dynamic_config(files = ["dynamic-config/examples/app.json"], key = "db", schema)]
#[derive(Deserialize, JsonSchema)]
struct DbConfig {
    /// Where the database lives.
    #[allow(dead_code)]
    host: String,
    /// Kept out of logs, and marked `writeOnly` in the schema.
    #[config(secret)]
    #[serde(default)]
    #[allow(dead_code)]
    password: String,
}

#[dynamic_config(files = ["dynamic-config/examples/app.json"], key = "server", schema)]
#[derive(Deserialize, JsonSchema)]
struct ServerConfig {
    host: String,
    /// Doc comments become schema descriptions, which is what an editor shows
    /// on hover — so they are worth writing on config types especially.
    port: u16,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Loaded as well as described, so the example proves the schema belongs to
    // a file this crate can actually read.
    let server = ServerConfig::load()?;
    println!("loaded: {}:{}\n", server.host, server.port);

    // ---------------------------------------------------------------------
    // One type: its schema describes the file it sits in, not the struct.
    // ---------------------------------------------------------------------
    let db = DbConfig::schema();

    println!("DbConfig::schema() describes a file:");
    println!("  top-level type      = {}", db["type"]);
    println!("  its only section    = db");
    println!(
        "  password writeOnly  = {}",
        db["properties"]["db"]["properties"]["password"]["writeOnly"]
    );
    println!(
        "  host writeOnly      = {}  (only the marked fields)",
        db["properties"]["db"]["properties"]["host"]
            .get("writeOnly")
            .map_or("absent", |_| "true")
    );

    // ---------------------------------------------------------------------
    // Two types over one file: one schema for the file they share.
    // ---------------------------------------------------------------------
    let whole_file = dynamic_config::schema::merge([DbConfig::schema(), ServerConfig::schema()]);

    println!("\nmerged, both sections are there:");
    for section in whole_file["properties"]
        .as_object()
        .expect("the merged schema has properties")
        .keys()
    {
        println!("  {section}");
    }

    // ---------------------------------------------------------------------
    // Nothing is required, and that is deliberate.
    // ---------------------------------------------------------------------
    println!(
        "\nrequired inside the db section  = {:?}",
        whole_file["properties"]["db"].get("required")
    );
    println!(
        "required inside db.pool        = {:?}",
        whole_file["properties"]["db"]["properties"]
            .get("pool")
            .and_then(|pool| pool.get("required"))
    );
    println!(
        "A field the file omits may still be supplied by the environment, a\n\
         flag or a computed default — none of which an editor can see. Marking\n\
         fields required would light up every 12-factor config file in red."
    );

    // ---------------------------------------------------------------------
    // Writing it out, and wiring it up.
    // ---------------------------------------------------------------------
    let path = std::env::temp_dir().join("dynamic-config-example.schema.json");
    std::fs::write(&path, serde_json::to_string_pretty(&whole_file)?)?;

    println!("\nwrote {}", path.display());
    println!(
        "\nWire it up per format:\n\
         \x20 JSON  \"$schema\": \"./app.schema.json\"          (a top-level key)\n\
         \x20 YAML  # yaml-language-server: $schema=./app.schema.json\n\
         \x20 TOML  #:schema ./app.schema.json"
    );
    println!(
        "\n`$schema` is the one top-level key this crate does not read as a\n\
         section — otherwise wiring the schema into the file it describes would\n\
         stop the file from loading."
    );

    std::fs::remove_file(&path)?;

    Ok(())
}

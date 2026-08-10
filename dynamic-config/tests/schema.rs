//! A JSON Schema that describes the file, not the struct.

#![cfg(all(feature = "schema", feature = "json"))]

use dynamic_config::dynamic_config;
use schemars::JsonSchema;
use serde::Deserialize;

#[dynamic_config(files = ["tests/fixtures/base.json"], key = "db", schema)]
#[derive(Deserialize, JsonSchema)]
struct Described {
    /// Where the database lives.
    #[allow(dead_code)]
    host: String,
    #[allow(dead_code)]
    port: u16,
    #[config(secret)]
    #[serde(default)]
    #[allow(dead_code)]
    password: String,
}

#[dynamic_config(files = ["tests/fixtures/base.json"], key = "server", schema)]
#[derive(Debug, Deserialize, JsonSchema)]
struct AlsoDescribed {
    #[allow(dead_code)]
    host: String,
    #[allow(dead_code)]
    port: u16,
}

#[test]
fn the_schema_describes_a_file_rather_than_a_struct() {
    let schema = Described::schema();

    assert_eq!(schema["type"], "object");
    assert!(
        schema["properties"]["db"]["properties"]["host"].is_object(),
        "the struct belongs under its section key: {schema}"
    );
}

#[test]
fn a_secret_is_marked_write_only() {
    let schema = Described::schema();
    let section = &schema["properties"]["db"];

    assert_eq!(section["properties"]["password"]["writeOnly"], true);
    assert!(
        section["properties"]["host"].get("writeOnly").is_none(),
        "only the fields `#[config(secret)]` marks"
    );
}

#[test]
fn doc_comments_become_descriptions() {
    let schema = Described::schema();

    assert_eq!(
        schema["properties"]["db"]["properties"]["host"]["description"],
        "Where the database lives.",
        "what an editor shows on hover"
    );
}

#[test]
fn nothing_is_required_because_a_file_is_only_one_layer() {
    let schema = Described::schema();

    assert!(
        schema["properties"]["db"].get("required").is_none(),
        "`port` has no default, but the environment can still supply it"
    );
}

#[test]
fn several_types_over_one_file_merge_into_one_schema() {
    let whole = dynamic_config::schema::merge([Described::schema(), AlsoDescribed::schema()]);

    assert!(whole["properties"]["db"].is_object());
    assert!(whole["properties"]["server"].is_object());
    assert_eq!(
        whole["properties"]["db"]["properties"]["password"]["writeOnly"], true,
        "merging must not undo the marking"
    );
}

/// The schema the crate emits must describe a file the crate can read — which
/// is only true because `$schema` is exempt from the section rule.
#[test]
fn a_file_carrying_its_own_schema_still_loads() {
    let schema = Described::schema();
    let document = serde_json::json!({
        "$schema": schema,
        "db": { "host": "localhost", "port": 5432 },
    })
    .to_string();

    let sources = [dynamic_config::Source::inline(
        &document,
        dynamic_config::Format::Json,
    )];

    let config: Described = dynamic_config::load(&dynamic_config::LoadSpec::new("db", &sources))
        .expect("a file may carry the schema that describes it");

    assert_eq!(config.host, "localhost");
}

//! A JSON Schema for the files this program reads.
//!
//! `schemars` describes a *struct*. What an editor needs is a description of the
//! *file*, and the two are not the same thing here: a config file is a map of
//! sections, and a struct is one section. This module is the difference —
//! wrapping a struct's schema under its section key, marking the fields
//! `#[config(secret)]` covers, and merging several structs into the one file
//! they share.
//!
//! Opt in with the `schema` argument, which is what emits `schema()`. Like
//! `save`, it is a separate argument rather than something every type gets,
//! because the method needs a trait — `JsonSchema` — that the user has to
//! derive.
//!
//! ```
//! # #[cfg(feature = "schema")] {
//! use schemars::JsonSchema;
//! use serde::Deserialize;
//!
//! #[dynamic_config::dynamic_config(files = ["config.json"], key = "db", schema)]
//! #[derive(Deserialize, JsonSchema)]
//! struct DbConfig {
//!     host: String,
//!     #[config(secret)]
//!     password: String,
//! }
//!
//! let schema = DbConfig::schema();
//!
//! // A file-level schema: `{"db": {...}}`, not `{...}`.
//! assert!(schema["properties"]["db"].is_object());
//! // And the secret is marked as one.
//! assert_eq!(
//!     schema["properties"]["db"]["properties"]["password"]["writeOnly"],
//!     true
//! );
//! # }
//! ```
//!
//! # Wiring it up
//!
//! | Format | How the editor finds it |
//! |---|---|
//! | JSON | `"$schema": "./config.schema.json"` as a top-level key in the file |
//! | YAML | `# yaml-language-server: $schema=./config.schema.json` as the first line |
//! | TOML | `#:schema ./config.schema.json` as the first line |
//!
//! The JSON case is the reason `$schema` is the one top-level key this crate
//! does not treat as a section. The other two are comments, and were never a
//! problem.
//!
//! # What is deliberately *not* in the schema
//!
//! **Nothing is `required`, at any depth.** schemars marks every field that is
//! neither `Option` nor `#[serde(default)]` as required, which is right for a
//! struct and wrong for a config file: the environment, a flag, an override or a
//! computed default can all supply a value, and none of them are visible to an
//! editor. Left in place it would light up every 12-factor deployment's config
//! file in red for values that are perfectly well supplied. `check()` answers
//! the question a schema cannot — *does this actually resolve* — with every
//! layer in view.
//!
//! **The top level accepts unknown keys.** Other sections belong to other
//! structs, and one struct's schema has no business calling them a mistake. Use
//! [`merge`] to describe the whole file, and unknown-key detection —
//! `check()` — for the part a schema cannot see.

use serde_json::{Map, Value};

/// The draft every emitted schema declares.
const DRAFT: &str = "https://json-schema.org/draft/2020-12/schema";

/// Wraps one struct's schema as the section it occupies in a file.
///
/// `secrets` are the field names to mark `writeOnly`, which is JSON Schema's
/// way of saying *this value is not for reading back* — editors stop echoing
/// them in completions and hovers.
///
/// Called by the generated `schema()`; public because the macro-free API is
/// public too.
#[must_use]
pub fn section(key: &str, schema: Value, secrets: &[&str]) -> Value {
    let mut schema = schema;

    drop_required(&mut schema);
    mark_secrets(&mut schema, secrets);

    let mut properties = Map::new();
    properties.insert(key.to_owned(), schema);

    let mut file = Map::new();
    file.insert("$schema".to_owned(), Value::from(DRAFT));
    file.insert("type".to_owned(), Value::from("object"));
    file.insert("properties".to_owned(), Value::Object(properties));

    Value::Object(file)
}

/// Combines several section schemas into one schema for the file they share.
///
/// The usual shape once a program has more than one config type over one file:
///
/// ```
/// # #[cfg(feature = "schema")] {
/// # use serde::Deserialize;
/// # use schemars::JsonSchema;
/// # #[dynamic_config::dynamic_config(files = ["app.json"], key = "db", schema)]
/// # #[derive(Deserialize, JsonSchema)] struct DbConfig { host: String }
/// # #[dynamic_config::dynamic_config(files = ["app.json"], key = "server", schema)]
/// # #[derive(Deserialize, JsonSchema)] struct ServerConfig { port: u16 }
/// let whole_file = dynamic_config::schema::merge([
///     DbConfig::schema(),
///     ServerConfig::schema(),
/// ]);
///
/// assert!(whole_file["properties"]["db"].is_object());
/// assert!(whole_file["properties"]["server"].is_object());
/// # }
/// ```
///
/// Later sections win over earlier ones on a key collision, which cannot happen
/// unless two config types claim the same section — and if they do, the schema
/// is not where that will hurt.
#[must_use]
pub fn merge(schemas: impl IntoIterator<Item = Value>) -> Value {
    let mut properties = Map::new();

    for schema in schemas {
        let Value::Object(mut schema) = schema else {
            continue;
        };

        if let Some(Value::Object(section)) = schema.remove("properties") {
            properties.extend(section);
        }
    }

    let mut file = Map::new();
    file.insert("$schema".to_owned(), Value::from(DRAFT));
    file.insert("type".to_owned(), Value::from("object"));
    file.insert("properties".to_owned(), Value::Object(properties));

    Value::Object(file)
}

/// Removes every `required` list, however deep.
///
/// Depth matters: a nested table the file leaves out entirely can still be
/// supplied by `APP_DB_POOL__MAX`, so the argument that makes `required` wrong
/// at the top makes it wrong all the way down.
fn drop_required(schema: &mut Value) {
    match schema {
        Value::Object(fields) => {
            fields.remove("required");

            for value in fields.values_mut() {
                drop_required(value);
            }
        }
        // `anyOf`, `oneOf`, `prefixItems`: schemars uses arrays of schemas for
        // enums and tuples, and a required list inside one is just as wrong.
        Value::Array(items) => {
            for item in items {
                drop_required(item);
            }
        }
        _ => {}
    }
}

/// Sets `writeOnly` on the named properties.
///
/// Only the top level of the section: `#[config(secret)]` marks a field, and a
/// field is a top-level property of its own struct. A secret nested inside
/// another struct is that struct's business, and marking it here would mean
/// guessing which of several nested types the name belonged to.
fn mark_secrets(schema: &mut Value, secrets: &[&str]) {
    if secrets.is_empty() {
        return;
    }

    let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) else {
        return;
    };

    for secret in secrets {
        // A rename makes the schema's property name differ from the field name,
        // and the attribute only ever saw the field name. Silently doing
        // nothing is the honest outcome: the alternative is marking whichever
        // property happens to sort nearby.
        if let Some(Value::Object(property)) = properties.get_mut(*secret) {
            property.insert("writeOnly".to_owned(), Value::Bool(true));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn struct_schema() -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "host": { "type": "string" },
                "password": { "type": "string" },
            }
        })
    }

    #[test]
    fn a_struct_becomes_a_section_of_a_file() {
        let schema = section("db", struct_schema(), &[]);

        assert_eq!(schema["$schema"], DRAFT);
        assert_eq!(schema["type"], "object");
        assert_eq!(
            schema["properties"]["db"]["properties"]["host"]["type"],
            "string"
        );
    }

    #[test]
    fn secrets_are_marked_write_only() {
        let schema = section("db", struct_schema(), &["password"]);
        let section = &schema["properties"]["db"];

        assert_eq!(section["properties"]["password"]["writeOnly"], true);
        assert!(
            section["properties"]["host"].get("writeOnly").is_none(),
            "only the marked fields"
        );
    }

    #[test]
    fn a_secret_that_was_renamed_is_left_alone_rather_than_guessed_at() {
        let schema = section("db", struct_schema(), &["not_a_property"]);

        assert_eq!(
            schema["properties"]["db"]["properties"],
            struct_schema()["properties"],
            "a name the schema does not have must change nothing"
        );
    }

    #[test]
    fn nothing_is_required_at_any_depth() {
        let with_required = serde_json::json!({
            "type": "object",
            "required": ["host"],
            "properties": {
                "host": { "type": "string" },
                "pool": {
                    "type": "object",
                    "required": ["max_size"],
                    "properties": { "max_size": { "type": "integer" } }
                }
            }
        });

        let schema = section("db", with_required, &[]);
        let section = &schema["properties"]["db"];

        assert!(
            section.get("required").is_none(),
            "the environment can supply anything the file does not"
        );
        assert!(
            section["properties"]["pool"].get("required").is_none(),
            "and it can supply a nested table the file leaves out entirely"
        );
    }

    #[test]
    fn a_required_list_inside_a_variant_goes_too() {
        let tagged = serde_json::json!({
            "anyOf": [
                { "type": "object", "required": ["kind"], "properties": {} },
                { "type": "null" }
            ]
        });

        let schema = section("db", tagged, &[]);

        assert!(
            schema["properties"]["db"]["anyOf"][0]
                .get("required")
                .is_none(),
            "schemars uses arrays of schemas for enums, and the rule is the same there"
        );
    }

    #[test]
    fn several_sections_become_one_file() {
        let merged = merge([
            section("db", struct_schema(), &["password"]),
            section("server", struct_schema(), &[]),
        ]);

        assert_eq!(merged["$schema"], DRAFT);
        assert!(merged["properties"]["db"].is_object());
        assert!(merged["properties"]["server"].is_object());
        assert_eq!(
            merged["properties"]["db"]["properties"]["password"]["writeOnly"], true,
            "merging must not undo the marking"
        );
    }

    #[test]
    fn merging_nothing_is_an_empty_file_schema() {
        let merged = merge([]);

        assert_eq!(merged["type"], "object");
        assert_eq!(merged["properties"], serde_json::json!({}));
    }
}

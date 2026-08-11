# Schema

## The `schema` method

```rust
let schema = DbConfig::builder("db").schema();
```

With the `schema` feature, the builder has `schema()`. It lives on the builder
rather than on the type for two reasons: the schema wraps the struct under the
builder's *key*, which the type alone does not know, and the method needs
`T: JsonSchema` — a bound a generated inherent method cannot carry (rustc
rejects an inherent method whose bound a concrete `Self` does not meet, at the
definition rather than at the call; the builder's generic `impl` block can
state it).

## A schema for the config files

With the `schema` feature, every config type can describe the file it reads, so
an editor completes and validates it:

```rust
#[dynamic_config]
#[derive(Deserialize, JsonSchema)]
struct DbConfig {
    /// Where the database lives.        <- becomes the hover text
    host: String,
    #[config(secret)]                    <- becomes `writeOnly: true`
    password: String,
}

let schema = DbConfig::builder("db").schema();

// Several types over one file describe that one file together.
let whole = dynamic_config::schema::merge([
    DbConfig::builder("db").schema(),
    ServerConfig::builder("server").schema(),
]);
```

What comes out describes the **file**, not the struct — the struct is one
section, and a config file is a map of them, so the schema is the struct's
wrapped under its key.

| Format | How the editor finds it |
|---|---|
| JSON | `"$schema": "./config.schema.json"` as a top-level key |
| YAML | `# yaml-language-server: $schema=./config.schema.json` |
| TOML | `#:schema ./config.schema.json` |

The JSON row is why `$schema` is the one top-level key this crate does not read
as a section: otherwise wiring the schema into the file it describes would stop
the file from loading.

## Nothing is marked required, and that is the point

`schemars` marks every field that is neither `Option` nor `#[serde(default)]` as
required. That is right for a struct and wrong for a config file: the
environment, a flag, an override or a computed default can all supply a value,
and an editor sees none of them. Left in place it would light up every 12-factor
config file in red for values that are perfectly well supplied — so the emitted
schema drops `required` at every depth.

The question a schema cannot answer — *does this actually resolve* — is what
[`check()`](validation-diagnostics.md#checking-without-booting) is for, with
every layer in view.

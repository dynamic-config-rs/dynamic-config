//! Choosing the engine and the reader — the two seams a load can swap.
//!
//! ```text
//! cargo run -p dynamic-config --example engines --features json,yaml
//! cargo run -p dynamic-config --example engines --features json,yaml,figment
//! ```
//!
//! The **engine** folds the collected layers into one configuration; the
//! **reader** turns a document's text into a tree. Everything else — which
//! files are looked for, what the environment contributes, precedence,
//! provenance, the snapshot, the reload — is this crate's either way.
//!
//! Run it with and without `--features figment`: the same numbers come out,
//! which is the point. The engines are held to one merge rule leaf by leaf
//! by the crate's own tests; the readers are *not* interchangeable down to
//! the corner, and this prints the one case where that shows.

use dynamic_config::{engine, load, reader, Format, LoadSpec, Source};
use serde::Deserialize;

#[derive(Debug, Deserialize, PartialEq)]
struct Database {
    host: String,
    port: u16,
    pool: Pool,
}

#[derive(Debug, Deserialize, PartialEq)]
struct Pool {
    max: u32,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Three layers, in precedence order: later wins, tables descend into
    // each other, everything else replaces.
    let sources = [
        Source::inline(
            r#"{"db": {"host": "localhost", "port": 5432, "pool": {"max": 8}}}"#,
            Format::Json,
        ),
        Source::inline(r#"{"db": {"host": "db.internal"}}"#, Format::Json),
        Source::inline(r#"{"db": {"pool": {"max": 32}}}"#, Format::Json),
    ];

    // ---------------------------------------------------------------------
    // The engine. `engine::all()` is the registry — one entry per engine
    // compiled in, which is `config-rs` alone until the `figment` feature
    // turns its adapter on.
    // ---------------------------------------------------------------------
    let mut answers = Vec::new();

    for engine in engine::all() {
        let database: Database = load(&LoadSpec::new("db", &sources).with_engine(engine))?;

        println!("{:<10} {:?}", engine.name(), database);
        answers.push(database);
    }

    // Not a hope: the crate's own agreement tests assert exactly this over
    // three corpora, which is why the choice is a dependency question
    // rather than a semantic one.
    assert!(answers.windows(2).all(|pair| pair[0] == pair[1]));

    // A load that names no engine gets the default, `config-rs`. Naming one
    // per load is a `LoadSpec` field here and `Builder::engine` with the
    // macro; `engine::set_engine` decides it once for the process.

    // ---------------------------------------------------------------------
    // The reader. Unlike the engines, these are dialects — so the crate
    // defaults to its own and the difference is documented rather than
    // smoothed over.
    // ---------------------------------------------------------------------
    let yaml = "db:\n  host: db.internal\n  port: 5432\n  pool:\n    max: 32\n";

    for reader in reader::all() {
        // figment's reader does not read `.properties`, this crate's own
        // does not read RON — a reader that cannot read a format hands it
        // on rather than failing the load.
        let database: Database =
            load(&LoadSpec::new("db", &[Source::inline(yaml, Format::Yaml)]).with_reader(reader))?;

        println!("{:<10} {:?}", reader.name(), database);
    }

    // A format the chosen reader has no parser for is handed to one that
    // has. `.properties` is the case that matters: neither backend crate
    // ships a parser for it, and choosing one of them still reads it —
    // through this crate's parser, so the answer is the same too.
    let properties = "db.host = db.internal\ndb.port = 5432\ndb.pool.max = 32\n";

    for reader in reader::all() {
        let database: Database = load(
            &LoadSpec::new("db", &[Source::inline(properties, Format::Properties)])
                .with_reader(reader),
        )?;

        println!("{:<10} .properties  {:?}", reader.name(), database);
    }

    // The reason to reach for a backend's reader is usually YAML: this
    // crate's own is `serde_yaml`, which its author archived, and
    // `config-rs` brings `yaml-rust2`, which is maintained.
    let maintained: Database = load(
        &LoadSpec::new("db", &[Source::inline(yaml, Format::Yaml)])
            .with_reader(reader::config_rs()),
    )?;

    assert_eq!(maintained.pool.max, 32);

    // Where they differ, and why the default is this crate's own: a
    // top-level key with a dot in it is a *key* here, and a *path* to one
    // of the backends — the same three bytes, two documents.
    let dotted = r#"{"db": {"my.module": "debug"}}"#;

    #[derive(Deserialize)]
    struct Logging {
        #[serde(rename = "my.module")]
        module: Option<String>,
    }

    let native: Logging = load(
        &LoadSpec::new("db", &[Source::inline(dotted, Format::Json)]).with_reader(reader::native()),
    )?;

    println!(
        "\ndotted key, this crate's reader: {:?}",
        native.module.as_deref()
    );

    Ok(())
}

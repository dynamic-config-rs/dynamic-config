//! Switching engines, and the promise that switching changes nothing.
//!
//! The engine is the step that folds one tree per layer into one
//! configuration. Three ship, and any type implementing `Engine` is a
//! fourth. What this file pins is the contract around that: the same load
//! through every engine gives the same values *and* the same account of
//! which layer supplied each of them, a load can name its own engine, and
//! an engine of your own is reached by the same door.

#![cfg(all(feature = "json", feature = "figment"))]

use std::collections::BTreeMap;

use dynamic_config::engine::{self, Engine, Folded, Layer};
use dynamic_config::{Builder, Value};
use serde::Deserialize;

#[derive(Debug, Deserialize, PartialEq)]
struct Server {
    host: String,
    port: u16,
    tags: Vec<String>,
}

fn fixture(name: &str, contents: &str) -> String {
    let directory = std::env::temp_dir().join("dynamic-config-engines");
    std::fs::create_dir_all(&directory).expect("the scratch directory is creatable");

    let path = directory.join(name);
    std::fs::write(&path, contents).expect("the fixture is writable");

    path.display().to_string()
}

/// Every engine this build ships, by the name it answers to.
///
/// Read off `engine::all()` rather than written out here, so an engine
/// added to the crate is compared by every test in this file without one of
/// them being edited.
fn engines() -> Vec<(&'static str, &'static dyn Engine)> {
    engine::all()
        .into_iter()
        .map(|engine| (engine.name(), engine))
        .collect()
}

#[test]
fn every_engine_resolves_the_same_load_the_same_way() {
    let base = fixture(
        "layered-base.json",
        r#"{"server": {"host": "from-the-file", "port": 8080, "tags": ["a"]}}"#,
    );
    let over = fixture(
        "layered-over.json",
        r#"{"server": {"port": 9090, "tags": ["b", "c"]}}"#,
    );

    let mut answers = Vec::new();

    for (name, engine) in engines() {
        let builder = Builder::<Server>::new("server")
            .file(base.clone())
            .file(over.clone())
            .engine(engine);

        let config = builder.clone().load().expect("the layers resolve");
        let sources: Vec<Option<String>> = ["host", "port", "tags"]
            .iter()
            .map(|path| {
                builder
                    .source_of(path)
                    .expect("the sources read")
                    .map(|origin| origin.to_string())
            })
            .collect();

        answers.push((name, config, sources));
    }

    let (reference_name, reference_config, reference_sources) = &answers[0];

    for (name, config, sources) in &answers[1..] {
        assert_eq!(
            config, reference_config,
            "the {reference_name} and {name} engines resolved different values"
        );
        assert_eq!(
            sources, reference_sources,
            "the {reference_name} and {name} engines disagree on who supplied what"
        );
    }

    // And the answer is the one the merge rule says it is: a later layer
    // replaces a list rather than appending to it.
    assert_eq!(
        reference_config,
        &Server {
            host: "from-the-file".to_owned(),
            port: 9090,
            tags: vec!["b".to_owned(), "c".to_owned()],
        }
    );
}

/// The engine is a choice at the call site, not a property of the process.
#[test]
fn two_loads_in_one_process_may_use_different_engines() {
    let file = fixture(
        "per-load.json",
        r#"{"server": {"host": "h", "port": 1, "tags": []}}"#,
    );

    let one: Server = Builder::new("server")
        .file(file.clone())
        .engine(engine::figment())
        .load()
        .expect("figment's fold resolves");
    let other: Server = Builder::new("server")
        .file(file)
        .engine(engine::config_rs())
        .load()
        .expect("the default engine resolves");

    assert_eq!(one, other);
}

/// An engine of your own is reached by the same door as the three that ship.
#[test]
fn an_engine_of_your_own_is_an_ordinary_engine() {
    /// Folds by the same rule, and answers "I do not know" for every leaf —
    /// which the crate is allowed to fill in from the layers it was given.
    #[derive(Debug)]
    struct Silent;

    impl Engine for Silent {
        fn name(&self) -> &str {
            "silent"
        }

        fn fold(&self, layers: &[Layer<'_>]) -> Result<Folded, dynamic_config::Error> {
            let mut values = Value::Table(BTreeMap::new());

            for layer in layers {
                values.merge(layer.values.clone());
            }

            Ok(Folded {
                values,
                tags: BTreeMap::new(),
            })
        }
    }

    static SILENT: Silent = Silent;

    let base = fixture(
        "own-base.json",
        r#"{"server": {"host": "h", "port": 1, "tags": []}}"#,
    );
    let over = fixture("own-over.json", r#"{"server": {"port": 2}}"#);

    let builder = Builder::<Server>::new("server")
        .file(base)
        .file(over)
        .engine(&SILENT);

    let config = builder.clone().load().expect("a custom engine resolves");

    assert_eq!(config.port, 2, "the fold is still the fold");

    // Reporting no tags costs nothing a reader can see: the winner is read
    // off the same layers, in the same order, so provenance survives an
    // engine that does not track it.
    let source = builder
        .source_of("port")
        .expect("the sources read")
        .expect("something supplied the port");

    assert!(
        source.to_string().contains("own-over.json"),
        "the higher file should still be named: {source}"
    );
}

/// The shapes where a backend's own idea of a key could show through.
///
/// A key is a *name* here, never a path: `{"my.module": "debug"}` is one key
/// with a dot in it, which is how half the logging configuration in the
/// world is written. One of the engines reads a top-level key as a path
/// expression and would otherwise answer `{"my": {"module": ...}}`, so this
/// is the case that would break a deployment by changing which engine ran.
#[test]
fn a_key_is_a_name_in_every_engine() {
    fn table(pairs: &[(&str, Value)]) -> Value {
        Value::Table(
            pairs
                .iter()
                .map(|(key, value)| ((*key).to_owned(), value.clone()))
                .collect(),
        )
    }

    let cases: Vec<(&str, Vec<Value>)> = vec![
        (
            "a dotted key",
            vec![table(&[("my.module", Value::String("debug".to_owned()))])],
        ),
        (
            "a dotted key below the top",
            vec![table(&[(
                "log",
                table(&[("my.module", Value::Integer(1))]),
            )])],
        ),
        (
            "a key that looks like an index",
            vec![table(&[("hosts[0]", Value::Integer(1))])],
        ),
        ("an empty key", vec![table(&[("", Value::Integer(1))])]),
        (
            "a key with a space",
            vec![table(&[("a b", Value::Integer(1))])],
        ),
        (
            "a null over a value",
            vec![
                table(&[("a", Value::Integer(1))]),
                table(&[("a", Value::Null)]),
            ],
        ),
        (
            "a value over a null",
            vec![
                table(&[("a", Value::Null)]),
                table(&[("a", Value::Integer(1))]),
            ],
        ),
        (
            "a list replaced rather than appended to",
            vec![
                table(&[("a", Value::Array(vec![Value::Integer(1)]))]),
                table(&[(
                    "a",
                    Value::Array(vec![Value::Integer(2), Value::Integer(3)]),
                )]),
            ],
        ),
        (
            "a table over a scalar",
            vec![
                table(&[("a", Value::Integer(1))]),
                table(&[("a", table(&[("b", Value::Integer(2))]))]),
            ],
        ),
        (
            "a scalar over a table",
            vec![
                table(&[("a", table(&[("b", Value::Integer(2))]))]),
                table(&[("a", Value::Integer(1))]),
            ],
        ),
        (
            "an empty table as a leaf",
            vec![table(&[("a", table(&[]))])],
        ),
        (
            "an integer no backend has one width for",
            vec![table(&[("a", Value::Integer(i128::MAX))])],
        ),
    ];

    for (what, trees) in cases {
        let answers: Vec<(&str, String)> = engines()
            .into_iter()
            .map(|(name, engine)| {
                let answer = dynamic_config::__fuzz::fold_through(engine, &trees).map_or_else(
                    |error| format!("error: {error}"),
                    |(values, provenance)| format!("{values:?} | {provenance:?}"),
                );

                (name, answer)
            })
            .collect();

        let (reference_name, reference) = &answers[0];

        for (name, answer) in &answers[1..] {
            assert_eq!(
                answer, reference,
                "the {reference_name} and {name} engines disagree on {what}"
            );
        }
    }
}

/// A `config` source is a layer, the way a figment provider is.
///
/// The two interop doors are symmetric on purpose: whichever backend a
/// deployment already has something written against, that thing is one
/// `Source` away from being a layer of a load — at the same precedence
/// slot, under the same rules.
#[test]
fn a_config_source_is_one_layer_like_any_other() {
    use dynamic_config::{Format, LoadSpec, Source};

    // The backend's own reader, over a document this crate never sees.
    let foreign = config_rs::File::from_str(
        r#"{"host": "from-the-foreign-source", "port": 5432}"#,
        config_rs::FileFormat::Json,
    );

    let base = fixture(
        "config-source-base.json",
        r#"{"db": {"host": "from-the-file", "port": 1, "tags": []}}"#,
    );
    let sources = [
        Source::file(&base, Format::Json),
        Source::config_source(&foreign),
    ];
    let spec = LoadSpec::new("db", &sources);

    let config: Server = dynamic_config::load(&spec).expect("both layers resolve");

    assert_eq!(
        config.host, "from-the-foreign-source",
        "a later layer wins, whoever wrote it"
    );
    assert_eq!(config.port, 5432);

    // And it is a *layer*, so provenance names it rather than shrugging.
    let origin = dynamic_config::source_of(&spec, "host")
        .expect("the sources read")
        .expect("something supplied it");

    assert_eq!(origin.to_string(), "in an inline source");
}

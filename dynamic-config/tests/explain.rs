//! `explain()`: every layer's answer, and the snapshot's own provenance.
//!
//! One struct per test: the generated statics are per *type*, and two tests
//! sharing one would race on the runtime layers in parallel.

#![cfg(feature = "json")]

use serde::Deserialize;

#[test]
fn every_layer_answers_and_the_highest_wins() {
    #[dynamic_config::dynamic_config]
    #[derive(Debug, Deserialize)]
    struct Layered {
        #[allow(dead_code)]
        port: u16,
    }

    std::fs::create_dir_all("tests/scratch").unwrap();
    std::fs::write(
        "tests/scratch/explain-layers.json",
        r#"{"svc": {"port": 8080}}"#,
    )
    .unwrap();

    Layered::set_default("port", 3000).expect("a plain default");
    Layered::set_override("port", 9999).expect("a plain override");

    // Nothing has been initialised, so the diagnostics run on the builder.
    let builder = Layered::builder("svc")
        .file("tests/scratch/explain-layers.json")
        .env("EXPLAINLAYERS_");

    let explanation = builder.explain("port").expect("the sources read cleanly");

    let layers: Vec<&str> = explanation.rows().iter().map(|row| row.layer).collect();
    assert_eq!(
        layers,
        ["default", "file", "environment", "override"],
        "one row per layer with something to say, lowest precedence first"
    );

    let values: Vec<Option<&str>> = explanation
        .rows()
        .iter()
        .map(|row| row.value.as_deref())
        .collect();
    assert_eq!(values[0], Some("3000"));
    assert_eq!(values[1], Some("8080"));
    assert_eq!(values[2], None, "no EXPLAINLAYERS_SVC_PORT is exported");
    assert_eq!(values[3], Some("9999"));

    let winner = explanation.winner().expect("something supplies it");
    assert_eq!(winner.layer, "override");

    // The drift check: the winner the per-layer walk found must be the same
    // source the composed load reports — the two must never disagree.
    let composed = builder
        .source_of("port")
        .expect("the sources read cleanly")
        .expect("something supplies it");
    assert_eq!(winner.origin.as_ref(), Some(&composed));
}

#[test]
fn an_unsupplied_path_says_so() {
    #[dynamic_config::dynamic_config]
    #[derive(Debug, Deserialize)]
    struct Absent {
        #[allow(dead_code)]
        #[serde(default)]
        port: u16,
    }

    std::fs::create_dir_all("tests/scratch").unwrap();
    std::fs::write(
        "tests/scratch/explain-absent.json",
        r#"{"svc": {"port": 1}}"#,
    )
    .unwrap();

    let explanation = Absent::builder("svc")
        .file("tests/scratch/explain-absent.json")
        .explain("no_such_key")
        .expect("the sources read cleanly");

    assert!(explanation.winner().is_none());
    assert!(
        explanation.to_string().contains("nothing supplies it"),
        "{explanation}"
    );
}

#[test]
fn the_snapshot_answers_for_itself() {
    #[dynamic_config::dynamic_config]
    #[derive(Debug, Deserialize)]
    struct Snapshotted {
        #[allow(dead_code)]
        port: u16,
    }

    std::fs::create_dir_all("tests/scratch").unwrap();
    std::fs::write(
        "tests/scratch/explain-snapshot.json",
        r#"{"svc": {"port": 8080, "nested": {"inner": true}}}"#,
    )
    .unwrap();

    let snapshot = Snapshotted::builder("svc")
        .file("tests/scratch/explain-snapshot.json")
        .snapshot()
        .expect("the sources read cleanly");

    let origin = snapshot
        .source_of("port")
        .expect("a live resolution carries provenance");
    assert!(
        matches!(origin, dynamic_config::Origin::File(path) if path.ends_with("explain-snapshot.json")),
        "{origin:?}"
    );
    assert!(snapshot.source_of("no_such_key").is_none());

    // A sub-snapshot answers for its own, re-rooted paths.
    let nested = snapshot.sub("nested").expect("the table exists");
    assert!(nested.source_of("inner").is_some());
    assert!(nested.source_of("nested.inner").is_none());
}

#[test]
fn a_secret_path_comes_back_redacted() {
    #[dynamic_config::dynamic_config]
    #[derive(Deserialize)]
    struct WithSecret {
        #[allow(dead_code)]
        host: String,
        #[config(secret)]
        #[allow(dead_code)]
        password: String,
    }

    std::fs::create_dir_all("tests/scratch").unwrap();
    std::fs::write(
        "tests/scratch/explain-secret.json",
        r#"{"db": {"host": "localhost", "password": "hunter2-explain"}}"#,
    )
    .unwrap();

    // Redaction is the *type's* job — the generated `explain` knows which
    // fields are secret — so this test initialises and asks the type.
    WithSecret::builder("db")
        .file("tests/scratch/explain-secret.json")
        .init()
        .expect("the fixture is complete");

    // The one diagnostic that shows values must still not show *these*.
    let explanation = WithSecret::explain("password").expect("the sources read cleanly");
    let rendered = explanation.to_string();

    assert!(!rendered.contains("hunter2-explain"), "{rendered}");
    assert!(rendered.contains("***"), "{rendered}");
    assert!(
        explanation.winner().unwrap().origin.is_some(),
        "the origin is the safe half, and stays"
    );

    // The non-secret neighbour still explains with its value visible.
    let host = WithSecret::explain("host").expect("the sources read cleanly");
    assert!(host.to_string().contains("localhost"), "{host}");
}

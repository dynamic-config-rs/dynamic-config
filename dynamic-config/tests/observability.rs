//! Knowing what the configuration is doing: where a value came from, when it
//! changed, and what must never be printed.

#![cfg(feature = "json")]

use std::sync::{Arc, Mutex};

use dynamic_config::{dynamic_config, Origin};
use serde::Deserialize;

// ─────────────────────────────────────────────
// source_of / is_set
// ─────────────────────────────────────────────

#[dynamic_config(files = ["tests/fixtures/base.json"], key = "db", env = "DCOBS_")]
#[derive(Debug, Deserialize)]
struct Traced {
    #[allow(dead_code)]
    host: String,
    #[allow(dead_code)]
    port: u16,
}

#[test]
fn a_value_from_a_file_names_the_file() {
    let origin = Traced::source_of("host")
        .expect("the fixture reads cleanly")
        .expect("`host` is set by the fixture");

    let Origin::File(path) = origin else {
        panic!("expected a file origin, got {origin:?}");
    };

    assert!(path.ends_with("base.json"), "{}", path.display());
}

#[test]
fn a_value_from_a_runtime_layer_says_so() {
    Traced::set_override("port", 9999u16).unwrap();

    assert_eq!(
        Traced::source_of("port").unwrap(),
        Some(Origin::Runtime("override")),
        "the override outranks the file, so it is what the next load would use"
    );

    Traced::clear_overrides();
}

#[test]
fn is_set_distinguishes_absent_from_falsy() {
    assert!(Traced::is_set("host").unwrap());
    assert!(!Traced::is_set("nothing_supplies_this").unwrap());
    assert_eq!(Traced::source_of("nothing_supplies_this").unwrap(), None);
}

// ─────────────────────────────────────────────
// on_reload
// ─────────────────────────────────────────────

#[dynamic_config(files = ["tests/fixtures/base.json"], key = "db")]
#[derive(Debug, Deserialize)]
struct Hooked {
    host: String,
    #[allow(dead_code)]
    port: u16,
}

#[test]
fn a_callback_sees_both_sides_of_a_reload_but_not_the_first_load() {
    let seen: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(Vec::new()));

    let recorder = Arc::clone(&seen);
    Hooked::on_reload(move |previous, current| {
        recorder
            .lock()
            .unwrap()
            .push((previous.host.clone(), current.host.clone()));
    });

    Hooked::init().expect("the fixture reads cleanly");
    assert!(
        seen.lock().unwrap().is_empty(),
        "installing the first snapshot is not a reload"
    );

    Hooked::replace(Hooked {
        host: "second".to_owned(),
        port: 1,
    });

    let recorded = seen.lock().unwrap();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0], ("localhost".to_owned(), "second".to_owned()));
}

// ─────────────────────────────────────────────
// #[config(secret)]
// ─────────────────────────────────────────────

/// No `#[derive(Debug)]`: `#[config(secret)]` writes one that redacts.
#[dynamic_config(files = ["tests/fixtures/secrets.json"], key = "db")]
#[derive(Deserialize)]
struct Guarded {
    host: String,
    #[config(secret)]
    password: String,
    #[config(secret)]
    token: String,
}

#[test]
fn a_marked_field_never_reaches_a_debug_log() {
    let config = Guarded::load().expect("the fixture is complete");

    // The values really did load — redaction is about printing, not parsing.
    assert_eq!(config.password, "hunter2");
    assert_eq!(config.token, "t0ken");

    let rendered = format!("{config:?}");

    assert!(rendered.contains("localhost"), "{rendered}");
    assert!(!rendered.contains("hunter2"), "{rendered}");
    assert!(!rendered.contains("t0ken"), "{rendered}");
    assert_eq!(rendered.matches("***").count(), 2, "{rendered}");
}

#[test]
fn an_unmarked_field_is_still_printed() {
    let rendered = format!("{:?}", Guarded::load().unwrap());

    assert!(rendered.contains("host: \"localhost\""), "{rendered}");
}

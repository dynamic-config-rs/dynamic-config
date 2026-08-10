//! Every way a load can fail, and what to do about each.
//!
//! ```text
//! cargo run -p dynamic-config --example errors --features json
//! ```
//!
//! The point of `ErrorKind` is that the categories call for different
//! *reactions*, not different messages: a `Parse` is somebody's bad edit, a
//! `Missing` is an incomplete deployment, an `Io` is a permissions problem.

use dynamic_config::{load, Error, ErrorKind, Format, LoadSpec, Origin, Source};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Db {
    #[allow(dead_code)]
    host: String,
    #[allow(dead_code)]
    port: u16,
}

fn attempt(label: &str, sources: &[Source<'_>]) {
    println!("── {label}");

    match load::<Db>(&LoadSpec::new("db", sources)) {
        Ok(config) => println!("   loaded: {config:?}\n"),
        Err(error) => {
            describe(&error);
            println!();
        }
    }
}

/// The shape a real program's error handling takes.
fn describe(error: &Error) {
    // The category decides what to *do*.
    let advice = match error.kind() {
        ErrorKind::Io => "check the file's permissions",
        ErrorKind::Parse => "somebody saved a broken file",
        ErrorKind::Missing => "the deployment is incomplete",
        ErrorKind::Type => "the value is there but the wrong shape",
        ErrorKind::Env => "an environment variable could not be read",
        ErrorKind::Invalid => "it parsed, but the configuration is nonsense",
        ErrorKind::Backend => "the loader itself could not proceed",
        // `ErrorKind` is `#[non_exhaustive]`, so a match must stay open.
        _ => "unclassified",
    };

    println!("   kind:   {:?} — {advice}", error.kind());

    // The path says *which* key, even several levels down.
    if !error.path().is_empty() {
        println!("   path:   {}", error.path());
    }

    // The origin says which layer to go and edit.
    match error.origin() {
        Origin::File(path) => println!("   origin: the file {}", path.display()),
        Origin::Env(prefix) => println!("   origin: the environment, under {prefix}"),
        Origin::Runtime(layer) => println!("   origin: set from code as a {layer}"),
        Origin::Inline => println!("   origin: a source compiled into the binary"),
        _ => println!("   origin: unknown"),
    }

    // And `Display` puts all three together for a log line.
    println!("   log:    {error}");
}

fn main() {
    attempt(
        "a complete configuration",
        &[Source::inline(
            r#"{"db": {"host": "localhost", "port": 5432}}"#,
            Format::Json,
        )],
    );

    attempt(
        "a file nobody can parse",
        &[Source::inline("{ not json", Format::Json)],
    );

    attempt(
        "a key nothing supplies",
        &[Source::inline(r#"{"db": {"host": "a"}}"#, Format::Json)],
    );

    attempt(
        "a value of the wrong shape",
        &[Source::inline(
            r#"{"db": {"host": "a", "port": "not-a-number"}}"#,
            Format::Json,
        )],
    );

    // A file that is not there is *not* an error: that is what makes an
    // optional `secrets.json` work.
    attempt(
        "a missing file, plus one that is there",
        &[
            Source::file("does/not/exist.json", Format::Json),
            Source::inline(r#"{"db": {"host": "a", "port": 1}}"#, Format::Json),
        ],
    );

    println!("note: a missing file is skipped, not an error — which is what");
    println!("      makes an optional `secrets.json` work.");
}

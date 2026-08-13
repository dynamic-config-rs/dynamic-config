//! The generated code compiles under a renamed dependency.
//!
//! ```toml
//! config = { package = "dynamic-config", version = "0.6" }
//! ```
//!
//! is an ordinary thing to write, and until the expansion asked
//! `proc-macro-crate` for the facade's real name it produced code naming
//! `::dynamic_config` — a crate that consumer's namespace does not have. Only
//! a *separate* crate can prove this: every other test in this suite depends
//! on the facade under its own name, so the rename is invisible to them.
//!
//! The fixture is written into a temporary directory rather than checked in,
//! so the test dirties neither the tree nor the workspace's member list. It
//! shares the workspace's target directory, so the second run reuses
//! everything the first one built.

#![cfg(feature = "json")]

use std::path::{Path, PathBuf};
use std::process::Command;

/// What the fixture consumer declares: the facade, under a name that is not
/// its own, with the macro reached only through the re-export.
///
/// The path goes in with forward slashes, which cargo accepts on every
/// platform. A Windows path spelled with backslashes lands in a TOML *basic*
/// string, where `\a` is an escape sequence and `D:\a\dynamic-config` is a
/// parse error rather than a path.
fn manifest(facade: &Path) -> String {
    let facade = facade.display().to_string().replace('\\', "/");

    format!(
        r#"[package]
name = "renamed-consumer"
version = "0.0.0"
edition = "2021"
publish = false

# Its own workspace root, so the outer workspace neither adopts it nor has to
# list it.
[workspace]

[dependencies]
config = {{ package = "dynamic-config", path = "{}", default-features = false, features = ["json"] }}
serde = {{ version = "1", features = ["derive"] }}
"#,
        facade
    )
}

/// One load, from an inline document, through the renamed facade.
const MAIN: &str = r##"use config::{dynamic_config, Format, LoadSpec, Source};
use serde::Deserialize;

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Db {
    host: String,
}

fn main() {
    const DOCUMENT: &str = r#"{"db": {"host": "localhost"}}"#;

    let sources = [Source::inline(DOCUMENT, Format::Json)];
    let db: Db = config::load(&LoadSpec::new("db", &sources)).expect("the document loads");

    assert_eq!(db.host, "localhost");

    // The generated storage surface, which is where every hardcoded path
    // lived: the cell, the runtime layers, the remembered builder.
    Db::replace(db);
    assert_eq!(Db::current().host, "localhost");

    Db::set_override("host", "overridden").expect("a string serializes");
    Db::clear_overrides();

    println!("loaded through a renamed dependency");
}
"##;

fn facade() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn the_expansion_resolves_the_facade_under_whatever_name_the_consumer_gave_it() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let root = directory.path();

    std::fs::create_dir_all(root.join("src")).expect("the fixture directory is writable");
    std::fs::write(root.join("Cargo.toml"), manifest(&facade()))
        .expect("the fixture manifest is writable");
    std::fs::write(root.join("src/main.rs"), MAIN).expect("the fixture source is writable");

    let mut cargo = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));

    cargo
        .arg("run")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(root.join("Cargo.toml"))
        // Shared with the workspace, so this costs a compile of the fixture
        // and of the facade's feature set, not of the whole dependency tree.
        .arg("--target-dir")
        .arg(facade().join("../target/renamed-dependency"));

    // Cargo sets these per compilation unit and the child would inherit this
    // test binary's. `CARGO_TARGET_TMPDIR` in particular is how
    // `proc-macro-crate` recognises an integration test, and the fixture is
    // not one.
    for inherited in [
        "CARGO_MANIFEST_DIR",
        "CARGO_CRATE_NAME",
        "CARGO_PKG_NAME",
        "CARGO_TARGET_TMPDIR",
        "CARGO_TARGET_DIR",
        "RUSTFLAGS",
    ] {
        cargo.env_remove(inherited);
    }

    let output = cargo.output().expect("cargo runs");

    assert!(
        output.status.success(),
        "the fixture must compile and run:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("loaded through a renamed dependency"),
        "it has to have actually run"
    );
}

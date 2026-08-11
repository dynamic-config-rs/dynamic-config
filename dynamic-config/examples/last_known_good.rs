//! Starting from the last configuration that worked, and the three answers to
//! "what may be written down".
//!
//! ```text
//! cargo run -p dynamic-config --example last_known_good --features json
//! ```
//!
//! The example plays out the scenario the feature exists for: a service starts
//! cleanly, the machine reboots, and the file that was fine yesterday is
//! truncated. Once with each `CacheMode`.

use std::path::{Path, PathBuf};

use dynamic_config::{dynamic_config, CacheMode};
use serde::Deserialize;

/// Cached with `CacheMode::Full` — everything, secrets included.
#[dynamic_config]
#[derive(Deserialize)]
struct Full {
    host: String,
    #[config(secret)]
    password: String,
}

/// Cached with `CacheMode::Redacted` — the secret is not written, so recovery
/// needs it from somewhere live.
#[dynamic_config]
#[derive(Deserialize)]
struct Redacted {
    host: String,
    #[config(secret)]
    password: String,
}

/// Cached with `CacheMode::Fingerprint` — no value is written, so nothing
/// recovers. What survives is the diagnosis.
#[dynamic_config]
#[derive(Deserialize)]
struct Fingerprint {
    host: String,
    #[config(secret)]
    password: String,
}

const DIRECTORY: &str = "/tmp/dynamic-config-lkg";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = PathBuf::from(DIRECTORY);
    let config = directory.join("config.json");

    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory)?;

    // One set of sources, three caching policies. The mode is spelled out at
    // the one place a cache is asked for.
    let full = Full::builder("db")
        .file("/tmp/dynamic-config-lkg/config.json")
        .env("FULL_")
        .cache("/tmp/dynamic-config-lkg/full.json", CacheMode::Full);
    let redacted = Redacted::builder("db")
        .file("/tmp/dynamic-config-lkg/config.json")
        .env("REDACTED_")
        .cache("/tmp/dynamic-config-lkg/redacted.json", CacheMode::Redacted);
    let fingerprint = Fingerprint::builder("db")
        .file("/tmp/dynamic-config-lkg/config.json")
        .env("FINGERPRINT_")
        .cache(
            "/tmp/dynamic-config-lkg/fingerprint.json",
            CacheMode::Fingerprint,
        );

    // -----------------------------------------------------------------------
    // A clean start. Every load writes the cache, whichever mode it is in.
    // -----------------------------------------------------------------------
    std::fs::write(
        &config,
        r#"{"db": {"host": "db.internal", "password": "hunter2"}}"#,
    )?;

    full.init()?;
    redacted.init()?;
    fingerprint.init()?;

    println!("a clean start: {}\n", Full::current().host);

    show("full", &directory.join("full.json"))?;
    show("redacted", &directory.join("redacted.json"))?;
    show("fingerprint", &directory.join("fingerprint.json"))?;

    // -----------------------------------------------------------------------
    // The reboot. Something truncated the file mid-write.
    // -----------------------------------------------------------------------
    std::fs::write(&config, r#"{"db": {"host": "db.int"#)?;

    println!("\n--- the file is now truncated ---\n");

    // Recovery lives in `init()`, not `load()`. `load()` is the pure one — it
    // reads the sources and hands back a value — and a function that quietly
    // returned yesterday's answer instead would be a poor thing to build on.

    // `full` recovers on its own: the secret was on disk too.
    match full.init() {
        Ok(()) => {
            let config = Full::current();

            println!(
                "full:        recovered — host = {}, password = {}",
                config.host,
                if config.password == "hunter2" {
                    "the one that worked"
                } else {
                    "something else"
                }
            );
        }
        Err(error) => println!("full:        failed — {error}"),
    }

    // `redacted` needs the secret from somewhere live. Without it the field is
    // simply missing, which is a clearer failure than a silently empty string.
    match redacted.init() {
        Ok(()) => println!(
            "redacted:    recovered — host = {}",
            Redacted::current().host
        ),
        Err(error) => println!("redacted:    failed — {error}"),
    }

    std::env::set_var("REDACTED_DB_PASSWORD", "hunter2");

    match redacted.init() {
        Ok(()) => {
            let config = Redacted::current();

            println!(
                "redacted:    recovered with REDACTED_DB_PASSWORD set — host = {}, \
                 password = {}",
                config.host,
                if config.password == "hunter2" {
                    "from the environment"
                } else {
                    "something else"
                }
            );
        }
        Err(error) => println!("redacted:    still failed — {error}"),
    }

    // `fingerprint` never recovers, and does not pretend to. What it adds is
    // the diagnosis attached to the failure.
    match fingerprint.init() {
        Ok(()) => println!(
            "fingerprint: recovered — host = {}, password = {}",
            Fingerprint::current().host,
            Fingerprint::current().password
        ),
        Err(error) => println!("fingerprint: failed, as designed — {error}"),
    }

    println!(
        "\nRecovery reads no files, only the cache plus the environment: the files\n\
         are what broke, and a malformed one fails to parse whatever sits under it."
    );

    let _ = std::fs::remove_dir_all(&directory);

    Ok(())
}

/// Prints a cache file, so the trade-off is visible rather than described.
fn show(mode: &str, path: &Path) -> std::io::Result<()> {
    let written = std::fs::read_to_string(path)?;
    let one_line: String = written.split_whitespace().collect::<Vec<_>>().join(" ");

    println!("{mode:12} {one_line}");

    Ok(())
}

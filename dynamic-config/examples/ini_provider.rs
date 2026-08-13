//! A format this crate does not read, plugged in as a source.
//!
//! ```text
//! cargo run -p dynamic-config --example ini_provider --features json,figment
//! ```
//!
//! There is no `Parser` trait to implement, and deliberately so: `Source::provider`
//! already is one, with three methods instead of two. Anything that can turn text
//! into `Map<Profile, Dict>` is a source here — a format nobody has written a
//! figment provider for, an `Env` with a filter this crate does not model, a test
//! double. `Ini` below is the whole plug-in; everything after it is the ordinary
//! layer stack, unchanged.
//!
//! Two things are the plug-in author's to get right, and both are visible here:
//! an INI `[section]` becomes a figment **profile**, because a profile is what
//! this crate reads a section from; and the provider's `Metadata` is where its
//! provenance comes from — a provider that carries a name but no source resolves
//! to `origin unknown`, which is a diagnostic nobody can act on.

use std::path::{Path, PathBuf};

use dynamic_config::figment::value::{Dict, Map, Value};
use dynamic_config::figment::{self, Metadata, Profile, Provider};
use dynamic_config::{load, source_of, Format, LoadSpec, Origin, Source};
use serde::Deserialize;

// Read through `Debug`, which dead-code analysis does not count.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct Database {
    host: String,
    port: u16,
    pool: u32,
    #[serde(default)]
    tls: bool,
}

// ── The parser ─────────────────────────────────────────────────────────

/// An INI file, as a figment provider.
struct Ini {
    path: PathBuf,
    text: String,
}

impl Ini {
    fn read(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        Ok(Self {
            text: std::fs::read_to_string(&path)?,
            path,
        })
    }
}

impl Provider for Ini {
    /// The name is what a diagnostic calls this source; the *source* is what
    /// `source_of` resolves to. Supplying only the first leaves every value
    /// this provider contributes with `Origin::Unknown`.
    fn metadata(&self) -> Metadata {
        Metadata::from("INI file", self.path.as_path())
    }

    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        let mut sections: Map<Profile, Dict> = Map::new();
        // Keys before the first `[header]` land in figment's default profile,
        // which this crate reads as belonging to every section.
        let mut section = Profile::Default;

        for (index, line) in self.text.lines().enumerate() {
            let line = line.trim();
            // A comment is a *whole* line. Trimming from the first `#` instead
            // would silently truncate a password at its first `#`, which is a
            // character a password is allowed to contain.
            if line.is_empty() || line.starts_with(['#', ';']) {
                continue;
            }

            if let Some(header) = line
                .strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
            {
                section = Profile::from(header.trim());
            } else if let Some((key, value)) = line.split_once('=') {
                sections
                    .entry(section.clone())
                    .or_default()
                    .insert(key.trim().to_owned(), scalar(value.trim()));
            } else {
                // The position and the reason — never the line. A line that is
                // not `key = value` is most often an unterminated quoted value,
                // and quoting it back is how a pasted secret reaches a log.
                return Err(figment::Error::from(format!(
                    "line {} is neither a section header nor `key = value`",
                    index + 1
                )));
            }
        }

        Ok(sections)
    }
}

/// INI has no types, so the provider chooses them. Being explicit here means
/// `port = "5432"` and `port = 5432` cannot quietly come to mean different
/// things depending on which layer supplied them.
fn scalar(text: &str) -> Value {
    let text = text.trim_matches('"');
    if let Ok(flag) = text.parse::<bool>() {
        Value::from(flag)
    } else if let Ok(whole) = text.parse::<i64>() {
        Value::from(whole)
    } else if let Ok(real) = text.parse::<f64>() {
        Value::from(real)
    } else {
        Value::from(text)
    }
}

// ── Using it ───────────────────────────────────────────────────────────

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The base, in a format this crate does read.
    let base = r#"{"db": {"host": "localhost", "port": 5432, "pool": 8}}"#;
    let ini = Ini::read("dynamic-config/examples/database.ini")?;

    // A provider is an ordinary member of the source list, and the list is
    // still later-wins: the INI is read after the JSON, so it decides.
    let sources = [Source::inline(base, Format::Json), Source::provider(&ini)];
    let spec = LoadSpec::new("db", &sources);

    println!("{:?}\n", load::<Database>(&spec)?);

    // `host` is in the JSON only; `port` was overridden by the INI; `tls` exists
    // in no format this crate knows about.
    for key in ["host", "port", "tls"] {
        let origin = source_of(&spec, key)?.unwrap_or(Origin::Unknown);
        println!("{key:<5} {origin}");
    }

    // The provider's failures are this crate's failures: same `Error`, same
    // `ErrorKind::Parse`, same origin. Note what is *not* in it — the line that
    // failed. figment renders its own metadata into the message and this crate
    // appends the origin, so the file is named twice; that is cosmetic, and the
    // property worth keeping is that neither rendering quotes the text.
    let broken = Ini {
        path: PathBuf::from("broken.ini"),
        text: String::from("[db]\nport\n"),
    };
    let sources = [Source::provider(&broken)];
    let error = load::<Database>(&LoadSpec::new("db", &sources)).unwrap_err();
    println!("\n{error}");

    Ok(())
}

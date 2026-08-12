//! Starting from the last configuration that worked.
//!
//! A process that cannot read its configuration should normally refuse to
//! start — that is the point of failing loudly. But there is one case where
//! refusing is worse: a machine reboots, something on disk is half-written or
//! a mount has not appeared yet, and a service that would otherwise have come
//! up sits dead until a person notices.
//!
//! Opting into a cache says: prefer running on yesterday's configuration to not
//! running at all. It is deliberately opt-in, and deliberately loud — recovery
//! logs a warning every time, because a service quietly running on a stale
//! configuration is its own kind of outage.
//!
//! ## What ends up on disk
//!
//! A resolved configuration holds every value, including the ones
//! `#[config(secret)]` exists to keep out of logs. There is no way to make that
//! not a trade-off, so it is a choice with three answers rather than a default
//! nobody was told about:
//!
//! | Mode | On disk | Recovers |
//! |---|---|---|
//! | [`Full`](CacheMode::Full) | everything, secrets included | completely |
//! | [`Redacted`](CacheMode::Redacted) *(the attribute's default)* | everything except `#[config(secret)]` fields | only if the secrets come from somewhere live |
//! | [`Fingerprint`](CacheMode::Fingerprint) | a hash and the key names | never — it reports what changed and still fails |
//!
//! On Unix the file is written `0600`. That is the most that can be done
//! without refusing the request.
//!
//! ## Recovery reads no files
//!
//! The files are what broke. Recovery loads from the cache plus the
//! environment and the runtime layers — never from the sources whose failure
//! caused it, because a malformed file fails to parse whatever sits underneath
//! it.

#[cfg(all(test, feature = "json"))]
use std::collections::BTreeMap;
use std::fmt;
// `std::collections::hash_map::DefaultHasher` rather than `std::hash::`: the
// latter is the same type re-exported, but only since 1.76, and the core
// crate's floor is 1.71.
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;

use figment::value::{Dict, Value};

use crate::error::{Error, ErrorKind, Origin};
use crate::snapshot::Snapshot;
use crate::source::Format;

/// The key a cache document is written under, so it reads back like any file.
const CACHED: &str = "cached";

/// The marker every cache document carries, naming what it is.
///
/// The reader used to *sniff*: "has a top-level `fingerprint` key" meant
/// "is a fingerprint document" — and a real configuration with a
/// `fingerprint` section (a TLS pin, an image digest) was misread as one,
/// turning a perfectly good full cache into a refusal to start. A document
/// that says what it is cannot be misread. `version` is there so a future
/// format change can tell old files from new ones.
const MARKER: &str = "__dynamic_config_cache";

/// Where the fingerprint lives inside a `Fingerprint` document.
const FINGERPRINT: &str = "fingerprint";

/// Where the key list lives inside a `Fingerprint` document.
const KEYS: &str = "keys";

/// How much of a configuration to keep on disk.
///
/// A resolved configuration holds every value, including the ones
/// `#[config(secret)]` exists to keep out of logs. There is no way to make that
/// not a trade-off, so it is a choice with three answers rather than a default
/// nobody was told about:
///
/// | Mode | On disk | Recovers |
/// |---|---|---|
/// | [`Full`](Self::Full) | everything, secrets included | completely |
/// | [`Redacted`](Self::Redacted) *(the attribute's default)* | everything except `#[config(secret)]` fields | only if the secrets come from somewhere live |
/// | [`Fingerprint`](Self::Fingerprint) | a hash and the key names | never — it reports what changed and still fails |
///
/// On Unix the file is written `0600`. That is the most that can be done
/// without refusing the request.
///
/// Recovery reads no files: the files are what broke, so it loads from the
/// cache plus the environment and the runtime layers, never from the sources
/// whose failure caused it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum CacheMode {
    /// Everything, secrets included. Recovers completely.
    ///
    /// The default, because a cache that cannot recover is a cache that will
    /// disappoint somebody at three in the morning. The file is `0600`; the
    /// rest is documented rather than solved.
    #[default]
    Full,
    /// Everything except the fields marked `#[config(secret)]`.
    ///
    /// Recovery then depends on those values arriving from somewhere live —
    /// the environment, usually. That is arguably the right deployment shape
    /// anyway, and useless for anyone whose secrets live in a file.
    Redacted,
    /// A hash and the key names. No values at all.
    ///
    /// Cannot recover, and does not pretend to: a failed start still fails.
    /// What it buys is the diagnosis — *which keys have moved since the last
    /// time this worked* — which is usually the first thing anyone wants.
    Fingerprint,
}

impl fmt::Display for CacheMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Full => "full",
            Self::Redacted => "redacted",
            Self::Fingerprint => "fingerprint",
        })
    }
}

impl CacheMode {
    /// Whether a cache in this mode can stand in for the real thing.
    #[must_use]
    pub fn recovers(self) -> bool {
        !matches!(self, Self::Fingerprint)
    }
}

/// What a cache file turned out to hold.
#[derive(Debug)]
pub enum Recovery {
    /// A configuration to start from.
    Usable(Snapshot),
    /// Only a fingerprint: what differs from the last good state.
    ///
    /// `Some` lists the key paths that differ — or one explanatory sentence
    /// when the keys match and only values moved. `None` means the comparison
    /// itself was impossible: the sources do not resolve, so there is nothing
    /// to compare against.
    Drift(Option<Vec<String>>),
    /// No cache on disk yet.
    Absent,
}

/// Writes `snapshot` to `path` in `mode`.
///
/// `secrets` are the field names to drop in [`CacheMode::Redacted`]; ignored
/// otherwise.
///
/// # Errors
///
/// If the path names no supported format, or the file cannot be written.
pub(crate) fn write(
    snapshot: &Snapshot,
    path: &Path,
    mode: CacheMode,
    secrets: &[&str],
) -> Result<(), Error> {
    let format = format_of(path)?;

    let mut document = match mode {
        CacheMode::Full => snapshot.values().clone(),
        CacheMode::Redacted => without(snapshot.values(), secrets),
        CacheMode::Fingerprint => fingerprint_document(snapshot),
    };

    let mut marker = Dict::new();
    marker.insert("version".to_owned(), Value::from(1));
    marker.insert(
        "mode".to_owned(),
        Value::from(match mode {
            CacheMode::Full => "full",
            CacheMode::Redacted => "redacted",
            CacheMode::Fingerprint => "fingerprint",
        }),
    );
    document.insert(MARKER.to_owned(), Value::from(marker));

    crate::write::save_dict(&document, path, format, CACHED)
}

/// [`write`], full fidelity, through an [`Encryptor`](crate::Encryptor).
///
/// Always the whole document — encryption at rest is what collapses the
/// full/redacted trade-off, which is the mode's whole reason to exist —
/// with the same marker a `full` cache carries, so the read side after
/// decryption is the ordinary read side.
#[cfg(feature = "decrypt")]
pub(crate) fn write_encrypted(
    snapshot: &Snapshot,
    path: &Path,
    encryptor: &dyn crate::Encryptor,
) -> Result<(), Error> {
    let format = encrypted_format_of(path)?;

    let mut document = snapshot.values().clone();
    let mut marker = Dict::new();
    marker.insert("version".to_owned(), Value::from(1));
    marker.insert("mode".to_owned(), Value::from("full"));
    document.insert(MARKER.to_owned(), Value::from(marker));

    crate::write::save_dict_encrypted(&document, path, format, CACHED, encryptor)
}

/// [`read`], decrypting through the installed
/// [`Decryptor`](crate::Decryptor) first — the same door
/// `encrypted_file(..)` reads through, so one installation covers both.
#[cfg(feature = "decrypt")]
pub(crate) fn read_encrypted(path: &Path, current: Option<&Snapshot>) -> Result<Recovery, Error> {
    let format = encrypted_format_of(path)?;

    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Recovery::Absent),
        Err(error) => {
            return Err(Error::new(ErrorKind::Io, error.to_string())
                .with_origin(Origin::File(path.to_owned())))
        }
    };

    // `decrypt` hands back a zeroizing `Plaintext` and has already refused
    // non-UTF-8, naming the path.
    let plaintext = crate::decrypt::decrypt(&bytes, &path.display().to_string())?;

    parse_cache(plaintext.text(), format, path, current)
}

/// The format under the encryption suffix: `last.json.age` is JSON.
#[cfg(feature = "decrypt")]
fn encrypted_format_of(path: &Path) -> Result<crate::Format, Error> {
    let name = path
        .to_str()
        .ok_or_else(|| Error::new(ErrorKind::Io, "the cache path is not valid UTF-8"))?;

    let Some((inner, _suffix)) = crate::source::inner_name(name) else {
        return Err(Error::new(
            ErrorKind::Backend,
            format!(
                "an encrypted cache path carries the format under the \
                 encryption suffix — `last.json.{}`, not `{name}`",
                crate::source::ENCRYPTED_SUFFIX
            ),
        ));
    };

    format_of(Path::new(inner))
}

/// Reads whatever `path` holds.
///
/// # Errors
///
/// If the file exists but cannot be read or parsed. A file that is not there is
/// [`Recovery::Absent`], not a failure — the first start has no cache.
pub(crate) fn read(path: &Path, current: Option<&Snapshot>) -> Result<Recovery, Error> {
    let format = format_of(path)?;

    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Recovery::Absent),
        Err(error) => {
            return Err(Error::new(ErrorKind::Io, error.to_string())
                .with_origin(Origin::File(path.to_owned())))
        }
    };

    parse_cache(&text, format, path, current)
}

/// The shared tail of [`read`] and [`read_encrypted`]: text to `Recovery`.
fn parse_cache(
    text: &str,
    format: crate::Format,
    path: &Path,
    current: Option<&Snapshot>,
) -> Result<Recovery, Error> {
    let _ = path;
    let sources = [crate::Source::inline(text, format)];
    let cached = crate::loader::snapshot(&crate::LoadSpec::new(CACHED, &sources))?;

    // The marker says what the document is. Files written before the marker
    // existed (0.0.1) fall back to the old heuristic for one release —
    // documented in the changelog, removed after it.
    let is_fingerprint = match cached.get::<String>(&format!("{MARKER}.mode")) {
        Ok(mode) => mode == "fingerprint",
        Err(_) => cached.contains(FINGERPRINT),
    };

    if !is_fingerprint {
        return Ok(Recovery::Usable(cached.without_top_level(MARKER)));
    }

    Ok(Recovery::Drift(drift(&cached, current)))
}

/// Which keys have appeared or vanished since the cache was written.
///
/// `None` means "could not compare": the sources did not resolve — which is
/// the ordinary case during recovery, since a broken source is *why*
/// recovery is running. The caller must say so rather than claim a
/// comparison that never happened.
fn drift(cached: &Snapshot, current: Option<&Snapshot>) -> Option<Vec<String>> {
    let current = current?;

    let before: Vec<String> = cached.get(KEYS).unwrap_or_default();
    let after = current.leaf_paths();

    let mut moved: Vec<String> = before
        .iter()
        .filter(|key| !after.contains(key))
        .map(|key| format!("{key} is gone"))
        .chain(
            after
                .iter()
                .filter(|key| !before.contains(key))
                .map(|key| format!("{key} is new")),
        )
        .collect();

    moved.sort();

    // The keys all match — that is what the stored hash is FOR: telling
    // "identical" apart from "same keys, different values". It used to be
    // written and never read, and the report asserted the comparison anyway.
    if moved.is_empty() {
        let stored: Option<String> = cached.get(FINGERPRINT).ok();
        let current_hash = fingerprint_of(current);

        if stored.as_deref() == Some(current_hash.as_str()) {
            return Some(vec!["nothing moved — the sources match the last good \
                              configuration exactly"
                .to_owned()]);
        }

        return Some(vec!["the same keys, with different values".to_owned()]);
    }

    Some(moved)
}

/// The hash of a snapshot's values, as `fingerprint_document` computes it.
fn fingerprint_of(snapshot: &Snapshot) -> String {
    let mut hasher = DefaultHasher::new();
    format!("{:?}", snapshot.values()).hash(&mut hasher);

    format!("{:016x}", hasher.finish())
}

/// The whole tree minus the top-level keys named in `secrets`.
///
/// Top-level is not a limitation here but a property of the source:
/// `#[config(secret)]` marks fields of the annotated struct, and those fields
/// ARE the section's top-level keys. The names arrive serde-resolved — a
/// `#[serde(rename = "pass")]` secret is redacted under `pass`, the key the
/// resolved tree actually uses.
fn without(values: &Dict, secrets: &[&str]) -> Dict {
    values
        .iter()
        .filter(|(key, _)| !secrets.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

/// A hash of the values, plus the key names — and no value anywhere.
fn fingerprint_document(snapshot: &Snapshot) -> Dict {
    let keys = snapshot.leaf_paths();

    // `Debug` rather than `Hash` (inside `fingerprint_of`): figment's values
    // carry a provenance tag that takes part in equality, and two identical
    // values from different providers must fingerprint the same.
    let mut document = Dict::new();
    document.insert(
        FINGERPRINT.to_owned(),
        Value::from(fingerprint_of(snapshot)),
    );
    document.insert(
        KEYS.to_owned(),
        Value::from(keys.into_iter().map(Value::from).collect::<Vec<_>>()),
    );

    document
}

fn format_of(path: &Path) -> Result<Format, Error> {
    // Named before the generic "unsupported" because the mistake is specific:
    // the cache writes plaintext, and a `.age` name would promise otherwise.
    // Failing loudly here beats a file that says "encrypted" and is not.
    if path.extension().is_some_and(|extension| extension == "age") {
        return Err(Error::new(
            ErrorKind::Backend,
            format!(
                "{} ends in `.age`, but the last-known-good cache is written in plaintext; give the cache an unencrypted name",
                path.display()
            ),
        ));
    }

    path.extension()
        .and_then(|extension| extension.to_str())
        .and_then(Format::from_extension)
        .ok_or_else(|| Error::unsupported(path))
}

/// A map, for the tests below.
#[cfg(all(test, feature = "json"))]
fn dict_of(entries: &[(&str, Value)]) -> BTreeMap<String, Value> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

#[cfg(all(test, feature = "json"))]
mod tests {
    use super::*;

    fn scratch(test: &str) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join("dynamic-config-cache").join(test);

        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("the scratch directory should be creatable");

        directory.join("cache.json")
    }

    fn snapshot() -> Snapshot {
        Snapshot::new(dict_of(&[
            ("host", "localhost".into()),
            ("password", "hunter2".into()),
            ("pool", Value::from(dict_of(&[("max", 10u16.into())]))),
        ]))
    }

    #[test]
    fn full_keeps_everything_and_recovers() {
        let path = scratch("full");

        write(&snapshot(), &path, CacheMode::Full, &["password"]).unwrap();

        let Recovery::Usable(recovered) = read(&path, None).unwrap() else {
            panic!("a full cache must be usable");
        };

        assert_eq!(recovered.get::<String>("host").unwrap(), "localhost");
        assert_eq!(recovered.get::<String>("password").unwrap(), "hunter2");
        assert_eq!(recovered.get::<u16>("pool.max").unwrap(), 10);
    }

    #[test]
    fn redacted_drops_the_marked_fields_and_nothing_else() {
        let path = scratch("redacted");

        write(&snapshot(), &path, CacheMode::Redacted, &["password"]).unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        assert!(!written.contains("hunter2"), "{written}");

        let Recovery::Usable(recovered) = read(&path, None).unwrap() else {
            panic!("a redacted cache is still usable");
        };

        assert_eq!(recovered.get::<String>("host").unwrap(), "localhost");
        assert!(
            !recovered.contains("password"),
            "the secret must not have survived"
        );
    }

    #[test]
    fn fingerprint_writes_no_value_at_all() {
        let path = scratch("fingerprint");

        write(&snapshot(), &path, CacheMode::Fingerprint, &[]).unwrap();

        let written = std::fs::read_to_string(&path).unwrap();

        assert!(!written.contains("hunter2"), "{written}");
        assert!(!written.contains("localhost"), "{written}");
        // The key names are there; the values are not.
        assert!(written.contains("host"), "{written}");
    }

    #[test]
    fn fingerprint_cannot_recover_but_reports_what_moved() {
        let path = scratch("drift");

        write(&snapshot(), &path, CacheMode::Fingerprint, &[]).unwrap();

        let now = Snapshot::new(dict_of(&[
            ("host", "localhost".into()),
            ("hsot", "typo".into()),
        ]));

        let Recovery::Drift(Some(moved)) = read(&path, Some(&now)).unwrap() else {
            panic!("a fingerprint cache cannot be usable");
        };

        assert!(moved.contains(&"hsot is new".to_owned()), "{moved:?}");
        assert!(moved.contains(&"password is gone".to_owned()), "{moved:?}");
    }

    #[test]
    fn an_age_cache_path_is_refused_because_the_cache_is_plaintext() {
        let error = write(
            &snapshot(),
            Path::new("cache.json.age"),
            CacheMode::Full,
            &[],
        )
        .unwrap_err();

        assert!(error.to_string().contains("plaintext"), "{error}");
    }

    #[test]
    fn a_first_start_has_no_cache_and_that_is_not_a_failure() {
        let path = scratch("absent").with_file_name("nothing.json");

        assert!(matches!(read(&path, None).unwrap(), Recovery::Absent));
    }

    #[test]
    fn only_fingerprint_refuses_to_recover() {
        assert!(CacheMode::Full.recovers());
        assert!(CacheMode::Redacted.recovers());
        assert!(!CacheMode::Fingerprint.recovers());
    }
}

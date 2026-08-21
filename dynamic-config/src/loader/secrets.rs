//! A directory of single-value files: one file per key, the filename is the
//! key, the contents are the value.
//!
//! How Docker and Kubernetes hand a container its credentials, and the shape
//! pydantic-settings calls `secrets_dir`. It is not another document format —
//! the same bytes in a `.json` would mean nothing — so it reads like the
//! environment layer does: flat names, coercion left to the struct.
//!
//! # Three decisions
//!
//! **Nesting is in the name, not in subdirectories.** `db__password` is
//! `db.password`, through the same [`nest`](crate::LoadSpec::nest) separator
//! the environment layer uses, so one setting governs both. A Kubernetes
//! secret is one flat directory of keys; recursion would buy a prettier
//! spelling nobody's mount produces. Subdirectories can be added later
//! without changing what a flat directory means today.
//!
//! **A missing directory is skipped, an unreadable one is an error.** A
//! container that mounts secrets in production and not in a test must still
//! start; a directory that is there but refuses to be read is a permissions
//! bug, and silence about it would be worse than a failed load.
//!
//! **Provenance is per file.** Each key is its own provider naming its own
//! path, so `explain` and `source_of` answer with `/run/secrets/db__password`
//! rather than with "the secrets directory" — the exact file, which is the
//! useful answer when two mounts disagree.

use std::path::{Path, PathBuf};

use crate::error::{Error, ErrorKind, Origin};
use crate::source::LoadSpec;

/// One mounted secret.
///
/// Per file rather than one contribution for the whole directory: provenance
/// is recorded per contribution, so a single one would trace every key back
/// to the directory and no further. Ten files is ten contributions, and ten
/// is the order of magnitude a mount actually has.
struct Secret {
    path: PathBuf,
    /// Dotted key path within the section, from the filename.
    key: String,
    value: String,
}

/// Every secret file's contribution: one per file, so provenance names the
/// file to edit rather than the directory it sits in.
pub(super) fn collect(
    into: &mut crate::resolve::Collected,
    spec: &LoadSpec<'_>,
) -> Result<(), Error> {
    let Some(directory) = spec.secrets_dir else {
        return Ok(());
    };

    for secret in read(Path::new(directory), spec)? {
        let mut values = std::collections::BTreeMap::new();
        crate::layer::insert_path(&mut values, &secret.key, crate::Value::String(secret.value));

        into.layer("secrets", crate::Origin::File(secret.path), values);
    }

    Ok(())
}

/// Reads one directory level into one [`Secret`] per key file.
fn read(directory: &Path, spec: &LoadSpec<'_>) -> Result<Vec<Secret>, Error> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        // Skipped exactly like a missing config file: the same image runs in
        // a test that mounts nothing.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(io(directory, &error)),
    };

    let mut paths = Vec::new();

    for entry in entries {
        paths.push(entry.map_err(|error| io(directory, &error))?.path());
    }

    // `read_dir` yields in whatever order the filesystem likes. Sorted so a
    // reload merges the same layers in the same order as the load before it —
    // the keys are distinct, so this changes no outcome, only the reading of
    // a diagnostic.
    paths.sort();

    // The containment root, resolved once: every followed link must land
    // under it unless the spec opted out. `read_dir` above succeeded, so
    // the directory exists and canonicalization cannot race its absence.
    let root = if spec.allow_external_symlinks {
        None
    } else {
        Some(
            directory
                .canonicalize()
                .map_err(|error| io(directory, &error))?,
        )
    };

    let mut secrets = Vec::new();

    for path in paths {
        let Some(key) = key_of(&path, spec.nest) else {
            continue;
        };

        // `metadata` follows symlinks where `DirEntry::file_type` does not,
        // and following is the whole point: Kubernetes mounts every key as a
        // symlink into a timestamped directory behind `..data`, so a layer
        // that refused to follow one would read an empty directory. Following
        // is not descending — a link that lands on a directory (`..data`
        // itself) is skipped like any other thing that is not a file.
        let metadata = match std::fs::metadata(&path) {
            Ok(metadata) => metadata,
            // A dangling link is the instant Kubernetes swaps a mount; the
            // next reload sees the new one. Anything else — a permission
            // denied on the link's target — is the bug this layer refuses to
            // be quiet about.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(io(&path, &error)),
        };

        if !metadata.is_file() {
            continue;
        }

        // Containment: the entry's fully-resolved target must live under
        // the directory's own resolved root. Kubernetes' `..data` links
        // pass by construction — their targets are inside the mount — and
        // a link planted to `/etc/shadow` (or climbing out through `..`)
        // is refused with the entry's name, never its contents. Reading
        // happens through the *verified* target below, so the check and
        // the read cannot be split by a swap of the link itself; swapping
        // the target file afterwards is the same trust boundary as any
        // configuration file. This is the shape Pydantic Settings shipped
        // a CVE for; `LoadSpec::allow_external_symlinks` is the opt-out
        // for a deliberate cross-mount layout.
        let read_from = match &root {
            None => path.clone(),
            Some(root) => {
                let target = match path.canonicalize() {
                    Ok(target) => target,
                    // The link went dangling between `metadata` and here:
                    // the same mount-swap instant handled above.
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(io(&path, &error)),
                };

                if !target.starts_with(root) {
                    return Err(Error::invalid(format!(
                        "secrets directory entry `{}` resolves outside `{}`; \
                         a symlink escaping the mount is refused (see \
                         `allow_external_symlinks`)",
                        path.display(),
                        directory.display(),
                    )));
                }

                target
            }
        };

        // A file whose bytes are not UTF-8 fails here rather than arriving
        // lossily converted: a mangled credential that loads is worse than
        // one that does not.
        let text = std::fs::read_to_string(&read_from).map_err(|error| io(&path, &error))?;

        secrets.push(Secret {
            path,
            key,
            value: trim_one_newline(&text).to_owned(),
        });
    }

    Ok(secrets)
}

/// The dotted key path a filename spells, or `None` if it spells nothing.
///
/// The name is the key *verbatim* — no case folding, unlike the environment
/// layer. A variable name is shouted by convention and has to be quietened on
/// the way in; a filename is written in whatever case the field uses, and
/// lowercasing would put a `#[serde(rename = "apiKey")]` field out of reach.
fn key_of(path: &Path, nest: &str) -> Option<String> {
    let key = path.file_name()?.to_str()?.replace(nest, ".");

    if key.is_empty() || key.split('.').any(str::is_empty) {
        return None;
    }

    Some(key)
}

/// Removes one trailing newline and no more.
///
/// Every tool that writes a secret to a file writes one, and nobody means it
/// as part of the password. Two of them are a value that ends in a blank
/// line, and that is the caller's business.
fn trim_one_newline(text: &str) -> &str {
    text.strip_suffix('\n').map_or(text, |trimmed| {
        // A CRLF is one newline, not two characters of value.
        trimmed.strip_suffix('\r').unwrap_or(trimmed)
    })
}

/// An I/O failure naming the path — and never the contents, which for this
/// layer are a credential by construction.
fn io(path: &Path, error: &std::io::Error) -> Error {
    Error::new(ErrorKind::Io, error.to_string()).with_origin(Origin::File(path.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exactly_one_trailing_newline_goes() {
        assert_eq!(trim_one_newline("hunter2\n"), "hunter2");
        assert_eq!(trim_one_newline("hunter2\r\n"), "hunter2");
        assert_eq!(trim_one_newline("hunter2\n\n"), "hunter2\n");
        assert_eq!(trim_one_newline("hunter2"), "hunter2");
        assert_eq!(trim_one_newline("  spaced  "), "  spaced  ");
    }

    #[test]
    fn the_filename_nests_through_the_separator() {
        assert_eq!(
            key_of(Path::new("/run/secrets/db__password"), "__").as_deref(),
            Some("db.password")
        );
        assert_eq!(
            key_of(Path::new("/run/secrets/apiKey"), "__").as_deref(),
            Some("apiKey"),
            "a filename is not shouted the way a variable name is, so it is \
             not quietened either"
        );
        assert_eq!(key_of(Path::new("/run/secrets/db__"), "__"), None);
    }

    #[test]
    fn a_missing_directory_contributes_nothing() {
        let spec = LoadSpec::new("db", &[]);

        assert!(read(Path::new("/no/such/secrets"), &spec)
            .unwrap()
            .is_empty());
    }
}

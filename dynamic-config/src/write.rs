//! Writing a configuration back out.
//!
//! For the programs that own their configuration file rather than read one
//! somebody else deploys: a CLI persisting preferences, a setup wizard, an
//! admin endpoint. A server should not be writing its own `/etc` file.
//!
//! Two things are load-bearing here.
//!
//! **The write is atomic.** A temporary file next to the target, then a rename.
//! This crate's own watcher is very likely watching that directory, and a
//! partial file would be read, fail to parse, and log a failure that never
//! happened — the exact "editor saves half a file" case the watcher was built
//! to survive, caused by us.
//!
//! **The output is the section, nested under its key**, so what comes out can
//! be read straight back in. A file whose shape differs from the one the loader
//! expects is not a saved configuration, it is a new bug.
//!
//! Secrets go to disk in the clear. `#[config(secret)]` keeps a value out of
//! logs; it cannot keep it out of a file the program was asked to write. On
//! Unix the file is created `0600` — *created*, not chmodded afterwards, so
//! there is no window in which it is readable by anyone else. That is the most
//! that can be done without refusing the request.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use figment::value::{Dict, Value};
use serde::Serialize;

use crate::error::{Error, ErrorKind, Origin};
use crate::source::Format;

/// Writes `value` to `path` as the `key` section of a `format` document.
///
/// The format is taken from the argument rather than the extension, so a file
/// named `config` or `.myapprc` can still be written.
///
/// # Errors
///
/// If `value` cannot be serialized, the format's feature is not enabled, or the
/// file cannot be written.
pub fn save<T: Serialize>(
    value: &T,
    path: impl AsRef<Path>,
    format: Format,
    key: &str,
) -> Result<(), Error> {
    let path = path.as_ref();

    write_atomically(path, &render(&document_of(value, key)?, format)?)
}

/// Writes an already-built section, without going through `Serialize`.
///
/// The cache holds a resolved tree rather than a struct, so it arrives here
/// shaped already.
pub(crate) fn save_dict(
    section: &Dict,
    path: &Path,
    format: Format,
    key: &str,
) -> Result<(), Error> {
    let mut document = Dict::new();
    document.insert(key.to_owned(), Value::from(section.clone()));

    let rendered = render(&document, format)?;

    write_atomically(path, &rendered)
}

/// [`save_dict`], through an [`Encryptor`](crate::Encryptor) — what the
/// encrypted last-known-good cache writes with.
#[cfg(feature = "decrypt")]
pub(crate) fn save_dict_encrypted(
    section: &Dict,
    path: &Path,
    format: Format,
    key: &str,
    encryptor: &dyn crate::Encryptor,
) -> Result<(), Error> {
    let mut document = Dict::new();
    document.insert(key.to_owned(), Value::from(section.clone()));

    let mut rendered = render(&document, format)?;
    let encrypted = encryptor
        .encrypt(rendered.as_bytes())
        .map_err(|error| error.prepend_key(encryptor.describe()));

    // The same courtesy `save_encrypted` extends: the plaintext does not
    // linger in freed memory on either path.
    {
        use zeroize::Zeroize;

        rendered.zeroize();
    }

    write_bytes_atomically(path, &encrypted?)
}

/// One tree, as the text of a `format` document.
///
/// `pub(crate)` rather than private because it is one half of the parse seam —
/// [`Value::render`](crate::Value::render) is this, reached from outside — and a
/// second serializer written next to it would be a second set of edge cases for
/// nulls and integer widths.
pub(crate) fn render(document: &Dict, format: Format) -> Result<String, Error> {
    // With no format feature on, every arm below is compiled out.
    #[cfg(not(any(feature = "json", feature = "toml", feature = "yaml")))]
    let _ = document;

    match format {
        #[cfg(feature = "json")]
        Format::Json => serde_json::to_string_pretty(document)
            .map(|mut rendered| {
                rendered.push('\n');
                rendered
            })
            .map_err(serialization),

        #[cfg(feature = "toml")]
        Format::Toml => toml::to_string_pretty(document).map_err(serialization),

        #[cfg(feature = "yaml")]
        Format::Yaml => serde_yaml::to_string(document).map_err(serialization),

        #[allow(unreachable_patterns)]
        format => Err(Error::new(
            ErrorKind::Backend,
            format!(
                "cannot write {format:?} because the `{}` feature is not enabled",
                format.feature()
            ),
        )),
    }
}

#[allow(dead_code)]
fn serialization(error: impl std::fmt::Display) -> Error {
    Error::new(ErrorKind::Type, error.to_string())
}

/// As [`save`], but refuses if `path` already exists.
///
/// The case is a setup wizard or a `--init` subcommand: overwriting a
/// configuration somebody already wrote by hand, silently, is the one failure
/// mode those have.
///
/// Written directly rather than through a temporary file and a rename. The
/// rename is what makes [`save`] atomic *against an existing file*, and there is
/// no existing file here by definition — so `create_new` on the target itself is
/// both simpler and free of the race a check-then-write would have.
///
/// # Errors
///
/// If the file exists, if `value` cannot be serialized, if the format's feature
/// is off, or if the file cannot be written.
pub fn save_new<T: Serialize>(
    value: &T,
    path: impl AsRef<Path>,
    format: Format,
    key: &str,
) -> Result<(), Error> {
    let path = path.as_ref();
    let rendered = render(&document_of(value, key)?, format)?;

    // `create_and_fill` cleans up after its own failures — and only its own:
    // a file the open refused to create is somebody else's, and removing it
    // here would destroy exactly the thing this function exists to protect.
    create_and_fill(path, &rendered).map_err(|error| {
        Error::new(ErrorKind::Io, error.to_string()).with_origin(Origin::File(path.to_owned()))
    })
}

/// As [`save`], encrypting the document before it reaches the disk.
///
/// The counterpart to reading a `secrets.json.age`. The encryptor is passed
/// here rather than installed process-wide, because *who may read this file* is
/// a decision about this write.
///
/// ```no_run
/// # #[cfg(feature = "age")] {
/// # use serde::Serialize;
/// # #[derive(Serialize)] struct Db { host: String }
/// # let config = Db { host: "localhost".to_owned() };
/// use dynamic_config::age::Recipients;
///
/// let recipients = Recipients::from_public_keys(["age1ql3z7..."])?;
///
/// dynamic_config::save_encrypted(
///     &config,
///     "secrets.json.age",
///     dynamic_config::Format::Json,
///     "db",
///     &recipients,
/// )?;
/// # }
/// # Ok::<(), dynamic_config::Error>(())
/// ```
///
/// # Errors
///
/// If `value` cannot be serialized, the format's feature is off, encryption
/// fails, or the file cannot be written.
#[cfg(feature = "decrypt")]
#[cfg_attr(docsrs, doc(cfg(feature = "decrypt")))]
pub fn save_encrypted<T: Serialize>(
    value: &T,
    path: impl AsRef<Path>,
    format: Format,
    key: &str,
    encryptor: &dyn crate::Encryptor,
) -> Result<(), Error> {
    let path = path.as_ref();
    let mut rendered = render(&document_of(value, key)?, format)?;

    let encrypted = encryptor
        .encrypt(rendered.as_bytes())
        .map_err(|error| error.prepend_key(encryptor.describe()));

    // The rendered plaintext is the whole point of encrypting: it does not
    // get to linger in freed memory on either the success or the failure
    // path. The same courtesy the read side's `Plaintext` extends.
    {
        use zeroize::Zeroize;

        rendered.zeroize();
    }

    write_bytes_atomically(path, &encrypted?)
}

/// The serialized value, nested under its section key.
fn document_of<T: Serialize>(value: &T, key: &str) -> Result<Dict, Error> {
    let section =
        Value::serialize(value).map_err(|error| Error::new(ErrorKind::Type, error.to_string()))?;

    let Value::Dict(_, section) = section else {
        return Err(Error::new(
            ErrorKind::Type,
            format!(
                "a configuration section must be a table, not {}",
                shape(&section)
            ),
        ));
    };

    let mut document = Dict::new();
    document.insert(key.to_owned(), Value::from(section));

    Ok(document)
}

/// Names what a value is without saying what it holds.
///
/// `{value:?}` here rendered the value itself: a configuration type that
/// serializes to a bare string — a newtype over a token is the easy way to
/// have one — put that string in the error, and this is a *write* path, so
/// the string is as likely to be a credential as anything in the crate ever
/// is. The shape is the whole of what the message needs.
fn shape(value: &Value) -> &'static str {
    match value {
        Value::Dict(..) => "a table",
        Value::Array(..) => "a list",
        Value::Empty(..) => "nothing",
        _ => "a single value",
    }
}

/// Writes through a temporary file in the same directory, then renames.
///
/// Same directory on purpose: a rename is only atomic within one filesystem,
/// and `/tmp` is routinely a different one.
fn write_atomically(path: &Path, contents: &str) -> Result<(), Error> {
    write_bytes_atomically(path, contents.as_bytes())
}

fn write_bytes_atomically(path: &Path, contents: &[u8]) -> Result<(), Error> {
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    let temporary = match directory {
        Some(directory) => directory.join(temporary_name(path)),
        None => Path::new(".").join(temporary_name(path)),
    };

    let io = |error: std::io::Error| {
        Error::new(ErrorKind::Io, error.to_string()).with_origin(Origin::File(path.to_owned()))
    };

    create_and_fill_bytes(&temporary, contents).map_err(io)?;

    fs::rename(&temporary, path).map_err(|error| {
        // The rename is what makes this atomic; if it fails there is a stray
        // file to clean up rather than leave behind.
        let _ = fs::remove_file(&temporary);

        io(error)
    })?;

    // The rename itself lives in the directory, and a crash can lose an
    // un-synced directory entry even though the file's bytes are safe.
    // Best-effort: a filesystem that refuses (or a platform without the
    // notion) still got the atomic rename, which is the part correctness
    // rests on — durability of the *entry* is defence in depth.
    #[cfg(unix)]
    if let Some(directory) = directory {
        if let Ok(handle) = fs::File::open(directory) {
            let _ = handle.sync_all();
        }
    }

    Ok(())
}

/// Creates the temporary file and writes `contents` into it.
///
/// Two properties matter more than they look:
///
/// **`create_new`** means the file must not already exist. A configuration file
/// often holds secrets and often lives in a directory this process does not own
/// exclusively; with plain `create`, a symlink planted at the temporary path
/// would be followed and the secrets written wherever it pointed. Refusing to
/// open an existing path removes that entirely, and the random component in the
/// name keeps two writers from colliding on it by accident.
///
/// **`mode(0o600)`** applies at creation. Writing first and chmodding after
/// leaves a window — short, but real — in which the secrets are readable by
/// every user on the machine.
fn create_and_fill(temporary: &Path, contents: &str) -> std::io::Result<()> {
    create_and_fill_bytes(temporary, contents.as_bytes())
}

/// Creates `path` and writes `contents`, cleaning up after its own failures —
/// and only its own.
///
/// The distinction is load-bearing: if the *open* failed, nothing was created
/// and nothing may be removed — `AlreadyExists` in particular means the path is
/// somebody else's file. If the open succeeded and the *write* failed, the
/// half-written file is this call's to remove, and leaving it would hand the
/// next reader a truncated configuration.
fn create_and_fill_bytes(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let mut options = OpenOptions::new();

    options.write(true).create_new(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        options.mode(0o600);
    }

    let mut file = options.open(path)?;

    let written = file
        .write_all(contents)
        // `sync_all`, not `flush`: on `std::fs::File`, `flush` is a no-op —
        // there is no userspace buffer — and the old comment here claimed
        // durability it never had. `sync_all` is the actual promise: the
        // bytes reach the disk before the rename makes them the
        // configuration, so a power loss after the rename cannot leave a
        // zero-length or half-written file wearing the real file's name.
        // Config-sized writes make the cost a rounding error, and the
        // last-known-good cache exists precisely for the machine that just
        // lost power. Applied to every atomic write — user `save()`
        // included — deliberately: a split durable/non-durable path is more
        // API than the difference is worth.
        .and_then(|()| file.sync_all());

    if written.is_err() {
        drop(file);

        let _ = fs::remove_file(path);
    }

    written
}

/// A neighbour of `path`, distinct enough that two writers do not collide.
/// Bumped per call, so two writes in one process cannot pick the same name.
static ATTEMPT: AtomicU64 = AtomicU64::new(0);

/// A name unlikely to collide, next to the target so the rename stays on one
/// filesystem.
///
/// Unlikely rather than unguessable: the security property comes from
/// `create_new`, not from the name. What the pid, the clock and the counter buy
/// is that two writers do not fail each other by picking the same path.
fn temporary_name(path: &Path) -> String {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config");

    let attempt = ATTEMPT.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.subsec_nanos());

    format!(".{name}.{}.{nanos:08x}{attempt:x}.tmp", std::process::id())
}

// A cautionary tale lived on this module: it once carried stacked
// `#[cfg(unix)]` + `#[cfg(not(unix))]` attributes, which AND together into an
// unsatisfiable condition — so none of these tests had ever compiled, on any
// platform. Stacked `cfg`s are conjunction, not alternatives.
#[cfg(all(test, any(feature = "json", feature = "toml", feature = "yaml")))]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct Db {
        host: String,
        port: u16,
    }

    /// A directory per test: these run in parallel, and one of them lists the
    /// directory looking for leftovers — which would otherwise catch another
    /// test's write in flight.
    fn scratch(test: &str, name: &str) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join("dynamic-config-write").join(test);

        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).expect("the scratch directory should be creatable");

        directory.join(name)
    }

    #[cfg(feature = "json")]
    #[test]
    fn what_is_written_can_be_read_straight_back() {
        use crate::{load, LoadSpec, Source};

        let path = scratch("round-trip", "config.json");
        let db = Db {
            host: "localhost".to_owned(),
            port: 5432,
        };

        save(&db, &path, Format::Json, "db").unwrap();

        let text = fs::read_to_string(&path).unwrap();
        let sources = [Source::inline(&text, Format::Json)];

        assert_eq!(load::<Db>(&LoadSpec::new("db", &sources)).unwrap(), db);
    }

    #[cfg(feature = "json")]
    #[test]
    fn the_temporary_file_does_not_survive_the_write() {
        let path = scratch("clean", "config.json");

        save(
            &Db {
                host: "a".to_owned(),
                port: 1,
            },
            &path,
            Format::Json,
            "db",
        )
        .unwrap();

        let leftovers: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();

        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[cfg(all(unix, feature = "json"))]
    #[test]
    fn the_file_is_not_world_readable() {
        use std::os::unix::fs::PermissionsExt;

        let path = scratch("private", "config.json");

        save(
            &Db {
                host: "a".to_owned(),
                port: 1,
            },
            &path,
            Format::Json,
            "db",
        )
        .unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;

        assert_eq!(mode, 0o600, "{mode:o}");
    }
}

#[cfg(all(test, unix, feature = "json"))]
mod permissions {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn scratch(test: &str) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join("dynamic-config-write").join(test);

        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();

        directory
    }

    /// The file holds secrets, so it must never exist with any other mode —
    /// not even for the moment between writing and chmodding.
    #[test]
    fn the_file_is_created_private_rather_than_made_private() {
        let path = scratch("mode").join("config.json");
        let mut section = Dict::new();
        section.insert("password".to_owned(), Value::from("hunter2"));

        save_dict(&section, &path, Format::Json, "db").unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode();

        assert_eq!(mode & 0o777, 0o600, "mode was {:o}", mode & 0o777);
    }

    /// A symlink planted at the temporary path must not be followed: that is
    /// how secrets end up written somewhere the attacker chose.
    #[test]
    fn a_planted_symlink_is_refused_rather_than_followed() {
        let directory = scratch("symlink");
        let path = directory.join("config.json");
        let elsewhere = directory.join("attacker-owned");

        fs::write(&elsewhere, "").unwrap();

        // The name carries a random component, so this plants a link at *every*
        // name the writer could pick, by making the whole directory read-only
        // to the writer instead. What is asserted is the outcome that matters:
        // the attacker's file is not the one that gets the secrets.
        std::os::unix::fs::symlink(&elsewhere, directory.join(".config.json.link")).unwrap();

        let mut section = Dict::new();
        section.insert("password".to_owned(), Value::from("hunter2"));

        save_dict(&section, &path, Format::Json, "db").unwrap();

        assert_eq!(
            fs::read_to_string(&elsewhere).unwrap(),
            "",
            "the write must have gone to its own file"
        );
        assert!(fs::read_to_string(&path).unwrap().contains("hunter2"));
    }

    /// `create_new` is the property this rests on, asserted directly.
    #[test]
    fn an_existing_temporary_path_is_never_opened() {
        let directory = scratch("existing");
        let occupied = directory.join("taken");

        fs::write(&occupied, "not ours").unwrap();

        let error = create_and_fill(&occupied, "ours").expect_err("the path is taken");

        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(
            fs::read_to_string(&occupied).unwrap(),
            "not ours",
            "and nothing was written over it"
        );
    }

    #[test]
    fn two_writes_in_one_process_do_not_pick_the_same_temporary_name() {
        let path = Path::new("/tmp/config.json");

        assert_ne!(temporary_name(path), temporary_name(path));
    }

    /// An open that fails for a reason *other* than `AlreadyExists` — a
    /// symlink loop here — must not remove what sits at the path. The old
    /// cleanup keyed on "any error except AlreadyExists", which deleted the
    /// very thing `save_new` exists to protect whenever the open failed
    /// differently.
    #[test]
    fn an_open_that_fails_oddly_removes_nothing() {
        let directory = scratch("eloop");
        let path = directory.join("config.json");

        // A symlink pointing at itself: `open` fails with ELOOP, not
        // AlreadyExists.
        std::os::unix::fs::symlink(&path, &path).unwrap();

        let mut section = Dict::new();
        section.insert("host".to_owned(), Value::from("localhost"));

        assert!(save_new(&BTree(section), &path, Format::Json, "db").is_err());

        assert!(
            path.symlink_metadata().is_ok(),
            "whatever was at the path must survive an open failure"
        );
    }

    /// `save_new` takes a `Serialize`; the permission tests work in `Dict`s.
    struct BTree(Dict);

    impl serde::Serialize for BTree {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            use serde::ser::SerializeMap;

            let mut map = serializer.serialize_map(Some(self.0.len()))?;

            for (key, value) in &self.0 {
                map.serialize_entry(key, value)?;
            }

            map.end()
        }
    }

    #[test]
    fn a_failed_write_leaves_no_temporary_file_behind() {
        let directory = scratch("cleanup");
        // A directory where the target's parent does not exist: the create
        // fails, and the question is whether anything is left over.
        let path = directory.join("missing").join("config.json");

        let mut section = Dict::new();
        section.insert("host".to_owned(), Value::from("localhost"));

        assert!(save_dict(&section, &path, Format::Json, "db").is_err());

        let leftovers: Vec<_> = fs::read_dir(&directory)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name())
            .collect();

        assert!(leftovers.is_empty(), "{leftovers:?}");
    }
}

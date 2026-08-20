//! Values set from code rather than read from a file.
//!
//! Two of these bracket the file and environment layers:
//!
//! ```text
//! defaults  <  files  <  environment  <  overrides
//! ```
//!
//! **Defaults** are for values a program can compute but a file need not state —
//! a pool size derived from the core count, say. `#[serde(default)]` covers the
//! constant case; this covers the case where the fallback is only known at run
//! time, and it is *observable*: the value came from somewhere, rather than from
//! a `Deserialize` impl nobody can see.
//!
//! **Overrides** win over everything, which is what makes them useful in tests
//! and behind a `--set key=value` flag. They are also the one layer that can be
//! changed while the process runs, so they take effect on the next `load()`.

use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::value::Value;

/// The layer's own shape: a path to a value, flat until it is asked for.
type Tree = BTreeMap<String, Value>;
use serde::Serialize;

use crate::error::{Error, ErrorKind};

/// A set of values addressed by dotted path.
///
/// `Layer::new()` is `const`, so this lives in a `static` — which is how
/// `#[dynamic_config]` emits it. Every method takes `&self`, so a layer can be
/// written to from anywhere without threading a handle around.
///
/// # Example
///
/// ```
/// use dynamic_config::Layer;
///
/// static OVERRIDES: Layer = Layer::new();
///
/// assert!(OVERRIDES.is_empty());
///
/// OVERRIDES.set("pool.max_size", 32u16).unwrap();
/// assert!(!OVERRIDES.is_empty());
///
/// OVERRIDES.clear();
/// ```
#[derive(Default)]
pub struct Layer {
    entries: Mutex<Tree>,
}

impl Layer {
    /// An empty layer.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Mutex::new(BTreeMap::new()),
        }
    }

    /// Sets `path` — dotted for nested fields, as in `"pool.max_size"`.
    ///
    /// Setting the same path again replaces it. The value is converted once,
    /// here, so a type that cannot be represented fails at the call site rather
    /// than at the next reload.
    ///
    /// # Errors
    ///
    /// If `path` is empty or has a blank segment, or if `value` cannot be
    /// serialized.
    ///
    pub fn set<T: Serialize>(&self, path: &str, value: T) -> Result<(), Error> {
        check_path(path)?;

        let value = crate::ser::to_value(&value)
            .map_err(|error| Error::new(ErrorKind::Type, error.to_string()).prepend_key(path))?;

        self.lock().insert(path.to_owned(), value);

        Ok(())
    }

    /// Sets every top-level field of a serializable value at once.
    ///
    /// The typed way to seed the defaults layer: instead of one
    /// `set_default(path, value)` per field, hand over a whole struct —
    /// usually `&Config::default()` — and every field lands in the layer in
    /// one step, in the same shape the configuration itself has.
    ///
    /// # Errors
    ///
    /// If `value` does not serialize, or serializes to something other than
    /// a map — defaults have named fields by definition.
    pub fn set_struct<T: serde::Serialize>(&self, value: &T) -> Result<(), Error> {
        let serialized = crate::ser::to_value(value).map_err(|error| {
            Error::new(
                crate::ErrorKind::Type,
                format!("the defaults struct did not serialize: {error}"),
            )
        })?;

        let Value::Table(entries) = serialized else {
            return Err(Error::new(
                crate::ErrorKind::Type,
                "defaults must be a struct or a map; a bare value has no field name to live under",
            ));
        };

        let mut layer = self.lock();

        for (path, value) in entries {
            layer.insert(path, value);
        }

        Ok(())
    }

    /// Sets `path` from text, read the way an environment variable is.
    ///
    /// `"8080"` becomes a number, `"true"` a boolean, `"[a, b]"` a list — the
    /// same loose reading the environment layer gets, so a value means the same
    /// thing whether it arrives as `APP_DB_PORT=8080` or `--set db.port=8080`.
    ///
    /// # Errors
    ///
    /// If `path` is empty or has a blank segment.
    ///
    pub fn set_text(&self, path: &str, text: &str) -> Result<(), Error> {
        check_path(path)?;

        // Parsed the way the environment layer parses: one grammar for every
        // surface that receives configuration as text, so `--set port=8080`
        // and `APP_PORT=8080` cannot come to different conclusions.
        let value = crate::text_value::from_text(text);

        self.lock().insert(path.to_owned(), value);

        Ok(())
    }

    /// Sets every `key=value` in `assignments`.
    ///
    /// The shape behind a `--set key=value` flag. A pair with no `=` is an
    /// error naming the offending argument rather than a silently ignored one.
    ///
    /// # Errors
    ///
    /// If an assignment has no `=`, or its key is not a usable path.
    ///
    pub fn set_assignments<I, S>(&self, assignments: I) -> Result<(), Error>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for assignment in assignments {
            let assignment = assignment.as_ref();

            let Some((path, value)) = assignment.split_once('=') else {
                return Err(Error::new(
                    ErrorKind::Type,
                    format!("`{assignment}` is not a `key=value` assignment"),
                ));
            };

            self.set_text(path.trim(), value)?;
        }

        Ok(())
    }

    /// Copies clap arguments into this layer, by `(argument id, key path)`.
    ///
    /// Only arguments that came from the **command line** are taken. clap's own
    /// `default_value` looks identical to a typed flag in `ArgMatches`, and
    /// letting a clap default outrank a configuration file would invert the
    /// whole precedence order — the file would be ignored in favour of a
    /// fallback nobody asked for.
    ///
    /// Values are read the way environment variables are, so `--port 8080` and
    /// `APP_DB_PORT=8080` mean the same thing whatever type the argument was
    /// declared with.
    ///
    /// ```no_run
    /// # use dynamic_config::Layer;
    /// # fn example(matches: &clap::ArgMatches, flags: &Layer) -> Result<(), dynamic_config::Error> {
    /// flags.bind_clap(matches, &[("db-host", "host"), ("db-port", "port")])?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// If a key path is unusable, or an argument's value is not valid UTF-8.
    ///
    #[cfg(feature = "clap")]
    #[cfg_attr(docsrs, doc(cfg(feature = "clap")))]
    pub fn bind_clap(
        &self,
        matches: &clap::ArgMatches,
        bindings: &[(&str, &str)],
    ) -> Result<(), Error> {
        for (argument, path) in bindings {
            if matches.value_source(argument) != Some(clap::parser::ValueSource::CommandLine) {
                continue;
            }

            let Some(mut values) = matches.get_raw(argument) else {
                continue;
            };

            // A repeated argument is a list; a single one is a scalar, so that
            // `--port 8080` fills a `u16` rather than a one-element `Vec`.
            let raw: Vec<&std::ffi::OsStr> = values.by_ref().collect();

            let text = match raw.as_slice() {
                [] => continue,
                [single] => utf8(single, argument)?.to_owned(),
                many => {
                    let mut rendered = String::from("[");

                    for (index, value) in many.iter().enumerate() {
                        if index > 0 {
                            rendered.push(',');
                        }

                        rendered.push_str(utf8(value, argument)?);
                    }

                    rendered.push(']');
                    rendered
                }
            };

            self.set_text(path, &text)?;
        }

        Ok(())
    }

    /// Removes `path`, reporting whether anything was there.
    #[must_use = "the return says whether anything was removed; ignore it \
                  deliberately with `let _ =` if you do not care"]
    pub fn unset(&self, path: &str) -> bool {
        self.lock().remove(path).is_some()
    }

    /// Removes everything.
    ///
    pub fn clear(&self) {
        self.lock().clear();
    }

    /// Whether the layer would contribute anything.
    ///
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    /// Recovers from poisoning rather than propagating it.
    ///
    /// The map behind this lock has no invariant a panic could break — it is a
    /// `BTreeMap` of paths to values, written one entry at a time. Propagating
    /// the poison would mean one panicked caller turns every later `load()`
    /// into a panic, which is a poor trade for a configuration library. The
    /// same choice is made everywhere else in the crate.
    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Value>> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Expands the dotted paths into the nested shape the loader wants.
    pub(crate) fn tree(&self) -> Tree {
        let mut root = Tree::new();

        for (path, value) in self.lock().iter() {
            insert_path(&mut root, path, value.clone());
        }

        root
    }
}

/// Configuration is text, and an argument that is not valid UTF-8 cannot be a
/// configuration value whatever its bytes mean elsewhere.
#[cfg(feature = "clap")]
fn utf8<'a>(value: &'a std::ffi::OsStr, argument: &str) -> Result<&'a str, Error> {
    value.to_str().ok_or_else(|| {
        Error::new(
            ErrorKind::Type,
            format!("`--{argument}` is not valid UTF-8"),
        )
    })
}

/// Writes `value` at a dotted path, creating intermediate tables.
///
/// A non-table sitting where a table is needed is replaced: the caller asked
/// for a nested key, so the scalar that was there cannot also be honoured.
/// Rejects a path that names nothing: empty, or with an empty segment.
///
/// Shared with the environment bindings, which have exactly the same rule for
/// exactly the same reason.
pub(crate) fn check_path(path: &str) -> Result<(), Error> {
    if path.is_empty() || path.split('.').any(str::is_empty) {
        return Err(Error::new(
            ErrorKind::Type,
            format!("`{path}` is not a usable key path"),
        ));
    }

    // `db::timeout` means "in another section", and exactly one place can
    // honour that: the old path of an alias, which splits the qualifier off
    // before it gets here. Everywhere else a path is relative to the section
    // being loaded, so accepting one would silently create a key with a colon
    // in its name that looks, in every report, like it worked.
    if path.contains(crate::aliases::SECTION) {
        return Err(Error::new(
            ErrorKind::Type,
            format!(
                "`{path}` names another section, and this path is relative to \
                 the section being loaded; `{}` is only meaningful in the old \
                 path of an alias",
                crate::aliases::SECTION
            ),
        ));
    }

    Ok(())
}

pub(crate) fn insert_path(root: &mut Tree, path: &str, value: Value) {
    let mut segments = path.split('.').peekable();
    let mut current = root;

    while let Some(segment) = segments.next() {
        if segments.peek().is_none() {
            current.insert(segment.to_owned(), value);
            return;
        }

        let entry = current
            .entry(segment.to_owned())
            .or_insert_with(|| Value::Table(Tree::new()));

        if !matches!(entry, Value::Table(_)) {
            *entry = Value::Table(Tree::new());
        }

        let Value::Table(nested) = entry else {
            unreachable!("just replaced with a table")
        };

        current = nested;
    }
}

// Key names and a count, never the values: an override layer is exactly
// where a secret set at runtime lives, and `{:?}` reaching a log is an
// ordinary accident.
impl std::fmt::Debug for Layer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let entries = self.lock();

        f.debug_struct("Layer")
            .field("keys", &entries.keys().collect::<Vec<_>>())
            .field("len", &entries.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_layer_contributes_nothing() {
        assert!(Layer::new().is_empty());
    }

    #[test]
    fn setting_the_same_path_replaces_it() {
        let layer = Layer::new();
        layer.set("port", 1u16).unwrap();
        layer.set("port", 2u16).unwrap();

        let dict = layer.tree();
        assert_eq!(dict.get("port"), Some(&Value::from(2u16)));
    }

    #[test]
    fn a_dotted_path_becomes_a_nested_table() {
        let layer = Layer::new();
        layer.set("pool.max_size", 32u16).unwrap();

        let dict = layer.tree();
        let Some(Value::Table(pool)) = dict.get("pool") else {
            panic!("expected a nested dict, got {dict:?}");
        };

        assert_eq!(pool.get("max_size"), Some(&Value::from(32u16)));
    }

    #[test]
    fn siblings_under_one_parent_do_not_clobber_each_other() {
        let layer = Layer::new();
        layer.set("pool.max_size", 32u16).unwrap();
        layer.set("pool.min_size", 4u16).unwrap();

        let dict = layer.tree();
        let Some(Value::Table(pool)) = dict.get("pool") else {
            panic!("expected a nested dict");
        };

        assert_eq!(pool.len(), 2);
    }

    #[test]
    fn a_scalar_standing_where_a_table_is_needed_is_replaced() {
        let layer = Layer::new();
        layer.set("pool", 1u16).unwrap();
        layer.set("pool.max_size", 32u16).unwrap();

        let dict = layer.tree();
        assert!(matches!(dict.get("pool"), Some(Value::Table(_))));
    }

    #[test]
    fn unset_and_clear_both_report_honestly() {
        let layer = Layer::new();
        layer.set("a", 1u16).unwrap();

        assert!(layer.unset("a"));
        assert!(!layer.unset("a"));
        assert!(layer.is_empty());

        layer.set("b", 1u16).unwrap();
        layer.clear();
        assert!(layer.is_empty());
    }

    #[test]
    fn text_is_read_the_way_an_environment_variable_is() {
        let layer = Layer::new();
        layer.set_text("port", "8080").unwrap();
        layer.set_text("enabled", "true").unwrap();
        layer.set_text("host", "localhost").unwrap();

        let dict = layer.tree();

        // Compared through the crate's own erasure: which integer width
        // carried 8080 is not a reading, and `values_equal` says so
        // everywhere else too.
        let read = |key: &str| dict.get(key).cloned();

        assert_eq!(read("port"), Some(Value::Integer(8080)));
        assert_eq!(read("enabled"), Some(Value::Bool(true)));
        assert_eq!(read("host"), Some(Value::String("localhost".to_owned())));
    }

    /// A `--set` value that used to take the process down.
    ///
    /// The reading was delegated to a parser that indexed a byte range with
    /// a character position while resolving escapes, so any quoted string
    /// holding a non-ASCII character before an escape aborted the load
    /// rather than returning a value.
    #[test]
    fn a_quoted_escape_after_a_non_ascii_character_is_read_not_fatal() {
        let layer = Layer::new();

        layer
            .set_text("greeting", "\"é\\nthere\"")
            .expect("the value is read");

        assert_eq!(
            layer.tree().get("greeting").cloned(),
            Some(Value::String("é\nthere".to_owned()))
        );
    }

    #[test]
    fn assignments_are_split_on_the_first_equals() {
        let layer = Layer::new();
        layer
            .set_assignments(["db.host=post=gres", "db.port=5432"])
            .unwrap();

        let dict = layer.tree();
        let Some(Value::Table(db)) = dict.get("db") else {
            panic!("expected a nested dict");
        };

        let read = |key: &str| db.get(key).cloned();

        assert_eq!(read("host"), Some(Value::String("post=gres".to_owned())));
        assert_eq!(read("port"), Some(Value::Integer(5432)));
    }

    #[test]
    fn an_assignment_without_an_equals_names_itself() {
        let error = Layer::new().set_assignments(["nonsense"]).unwrap_err();

        assert!(error.to_string().contains("`nonsense`"), "{error}");
    }

    #[test]
    fn an_unusable_path_is_rejected_at_the_call_site() {
        let layer = Layer::new();

        assert!(layer.set("", 1u16).is_err());
        assert!(layer.set("a..b", 1u16).is_err());
        assert!(layer.set(".a", 1u16).is_err());
    }

    #[test]
    fn structured_values_survive_the_round_trip() {
        #[derive(serde::Serialize)]
        struct Pool {
            max_size: u16,
        }

        let layer = Layer::new();
        layer.set("pool", Pool { max_size: 7 }).unwrap();

        let dict = layer.tree();
        let Some(Value::Table(pool)) = dict.get("pool") else {
            panic!("expected a nested dict");
        };

        assert_eq!(pool.get("max_size"), Some(&Value::from(7u16)));
    }
}

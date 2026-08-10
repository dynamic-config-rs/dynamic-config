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

use figment::value::{Dict, Map, Value};
use figment::{Metadata, Profile, Provider};
use serde::Serialize;

use crate::error::{Error, ErrorKind};

/// Metadata name for the defaults layer. Matched when attributing an error.
pub(crate) const DEFAULTS_NAME: &str = "values set as defaults";

/// Metadata name for the overrides layer.
pub(crate) const OVERRIDES_NAME: &str = "values set as overrides";

/// Metadata name for the command-line layer.
pub(crate) const FLAGS_NAME: &str = "values set from the command line";

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
    entries: Mutex<BTreeMap<String, Value>>,
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

        let value = Value::serialize(value)
            .map_err(|error| Error::new(ErrorKind::Type, error.to_string()).prepend_key(path))?;

        self.lock().insert(path.to_owned(), value);

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

        // Parsed the way the environment layer parses, fallback included:
        // `figment`'s parser does not fail today, but "cannot fail" is its
        // implementation detail, not this crate's to promise on.
        let value = text
            .parse::<Value>()
            .unwrap_or_else(|_| Value::from(text.to_owned()));

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
    ///
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
    fn dict(&self) -> Dict {
        let mut root = Dict::new();

        for (path, value) in self.lock().iter() {
            insert_path(&mut root, path, value.clone());
        }

        root
    }

    /// A figment provider emitting this layer under `profile`.
    pub(crate) fn provider<'a>(&'a self, profile: &str, name: &'static str) -> LayerProvider<'a> {
        LayerProvider {
            layer: self,
            profile: Profile::from(profile),
            name,
        }
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

    Ok(())
}

pub(crate) fn insert_path(root: &mut Dict, path: &str, value: Value) {
    let mut segments = path.split('.').peekable();
    let mut current = root;

    while let Some(segment) = segments.next() {
        if segments.peek().is_none() {
            current.insert(segment.to_owned(), value);
            return;
        }

        let entry = current
            .entry(segment.to_owned())
            .or_insert_with(|| Value::from(Dict::new()));

        if !matches!(entry, Value::Dict(..)) {
            *entry = Value::from(Dict::new());
        }

        let Value::Dict(_, nested) = entry else {
            unreachable!("just replaced with a dict")
        };

        current = nested;
    }
}

pub(crate) struct LayerProvider<'a> {
    layer: &'a Layer,
    profile: Profile,
    name: &'static str,
}

impl Provider for LayerProvider<'_> {
    fn metadata(&self) -> Metadata {
        Metadata::named(self.name)
    }

    fn data(&self) -> figment::Result<Map<Profile, Dict>> {
        let mut map = Map::new();
        map.insert(self.profile.clone(), self.layer.dict());

        Ok(map)
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

        let dict = layer.dict();
        assert_eq!(dict.get("port"), Some(&Value::from(2u16)));
    }

    #[test]
    fn a_dotted_path_becomes_a_nested_table() {
        let layer = Layer::new();
        layer.set("pool.max_size", 32u16).unwrap();

        let dict = layer.dict();
        let Some(Value::Dict(_, pool)) = dict.get("pool") else {
            panic!("expected a nested dict, got {dict:?}");
        };

        assert_eq!(pool.get("max_size"), Some(&Value::from(32u16)));
    }

    #[test]
    fn siblings_under_one_parent_do_not_clobber_each_other() {
        let layer = Layer::new();
        layer.set("pool.max_size", 32u16).unwrap();
        layer.set("pool.min_size", 4u16).unwrap();

        let dict = layer.dict();
        let Some(Value::Dict(_, pool)) = dict.get("pool") else {
            panic!("expected a nested dict");
        };

        assert_eq!(pool.len(), 2);
    }

    #[test]
    fn a_scalar_standing_where_a_table_is_needed_is_replaced() {
        let layer = Layer::new();
        layer.set("pool", 1u16).unwrap();
        layer.set("pool.max_size", 32u16).unwrap();

        let dict = layer.dict();
        assert!(matches!(dict.get("pool"), Some(Value::Dict(..))));
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

        let dict = layer.dict();

        assert_eq!(dict.get("port"), Some(&Value::from(8080u64)));
        assert_eq!(dict.get("enabled"), Some(&Value::from(true)));
        assert_eq!(dict.get("host"), Some(&Value::from("localhost")));
    }

    #[test]
    fn assignments_are_split_on_the_first_equals() {
        let layer = Layer::new();
        layer
            .set_assignments(["db.host=post=gres", "db.port=5432"])
            .unwrap();

        let dict = layer.dict();
        let Some(Value::Dict(_, db)) = dict.get("db") else {
            panic!("expected a nested dict");
        };

        assert_eq!(db.get("host"), Some(&Value::from("post=gres")));
        assert_eq!(db.get("port"), Some(&Value::from(5432u64)));
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

        let dict = layer.dict();
        let Some(Value::Dict(_, pool)) = dict.get("pool") else {
            panic!("expected a nested dict");
        };

        assert_eq!(pool.get("max_size"), Some(&Value::from(7u16)));
    }
}
